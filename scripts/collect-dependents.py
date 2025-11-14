"""Collect the DLLs required by npdf.exe from a vcpkg installation."""

from __future__ import annotations

import argparse
import os
import pprint
import shutil
import struct
import sys
from pathlib import Path
from typing import Dict, Iterable, List, NamedTuple, Sequence

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_EXE = REPO_ROOT / "target" / "release" / "npdf.exe"
IMPORT_DIRECTORY_INDEX = 1  # IMAGE_DIRECTORY_ENTRY_IMPORT


class Section(NamedTuple):
    virtual_address: int
    virtual_size: int
    raw_size: int
    raw_pointer: int


class PEFormatError(RuntimeError):
    """Raised when the PE file layout is not what we expect."""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Collect dependent DLLs for npdf.exe by inspecting its import table "
            "and copying matches from the vcpkg bin directory."
        )
    )
    parser.add_argument(
        "--exe",
        type=Path,
        default=DEFAULT_EXE,
        help="Path to the PE executable to inspect (default: target/release/npdf.exe)",
    )
    parser.add_argument(
        "--dest",
        type=Path,
        default=None,
        help="Destination directory for the copied DLLs (default: alongside the exe)",
    )
    parser.add_argument(
        "--vcpkg-root",
        type=Path,
        default=None,
        help="Override VCPKG_ROOT (falls back to the env variable)",
    )
    parser.add_argument(
        "--triplet",
        default=None,
        help="Override VCPKG_DEFAULT_TRIPLET (falls back to the env variable)",
    )
    parser.add_argument(
        "--bin-dir",
        type=Path,
        default=None,
        help="Skip env lookups by providing the vcpkg bin directory explicitly",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the copy plan without modifying the filesystem",
    )
    return parser.parse_args()


def resolve_path(path: Path, *, default_base: Path = REPO_ROOT) -> Path:
    if path.is_absolute():
        return path
    return (default_base / path).resolve()


def read_imported_dlls(exe_path: Path) -> List[str]:
    data = memoryview(exe_path.read_bytes())
    if data[:2].tobytes() != b"MZ":
        raise PEFormatError("Missing MZ header")

    header_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[header_offset : header_offset + 4].tobytes() != b"PE\0\0":
        raise PEFormatError("Missing PE signature")

    number_of_sections = struct.unpack_from("<H", data, header_offset + 6)[0]
    optional_header_size = struct.unpack_from("<H", data, header_offset + 20)[0]
    optional_header_offset = header_offset + 24

    magic = struct.unpack_from("<H", data, optional_header_offset)[0]
    if magic == 0x10B:
        data_directory_start = optional_header_offset + 96
    elif magic == 0x20B:
        data_directory_start = optional_header_offset + 112
    else:
        raise PEFormatError(f"Unsupported PE magic 0x{magic:X}")

    number_of_rvas = struct.unpack_from(
        "<I",
        data,
        data_directory_start - 4,  # NumberOfRvaAndSizes field
    )[0]
    if number_of_rvas <= IMPORT_DIRECTORY_INDEX:
        return []

    import_directory_rva, _ = struct.unpack_from("<II", data, data_directory_start + IMPORT_DIRECTORY_INDEX * 8)
    if import_directory_rva == 0:
        return []

    sections_offset = optional_header_offset + optional_header_size
    sections = _read_sections(data, sections_offset, number_of_sections)

    import_offset = rva_to_offset(import_directory_rva, sections)
    if import_offset is None:
        raise PEFormatError("Could not map import directory RVA to file offset")

    dlls: List[str] = []
    cursor = import_offset
    while True:
        try:
            (
                original_first_thunk,
                time_date_stamp,
                forwarder_chain,
                name_rva,
                first_thunk,
            ) = struct.unpack_from("<IIIII", data, cursor)
        except struct.error as exc:  # pragma: no cover - truncated binaries
            raise PEFormatError("Truncated import descriptor table") from exc

        if not any((
            original_first_thunk,
            time_date_stamp,
            forwarder_chain,
            name_rva,
            first_thunk,
        )):
            break

        name_offset = rva_to_offset(name_rva, sections)
        if name_offset is None:
            raise PEFormatError(f"Could not map import descriptor name RVA 0x{name_rva:X}")

        dll_name = _read_c_string(data, name_offset)
        if dll_name:
            dlls.append(dll_name)

        cursor += 20

    seen = set()
    ordered = []
    for dll in dlls:
        dll_lower = dll.lower()
        if dll_lower in seen:
            continue
        seen.add(dll_lower)
        ordered.append(dll)
    return ordered


def _read_sections(data: memoryview, start: int, count: int) -> List[Section]:
    sections: List[Section] = []
    for idx in range(count):
        offset = start + idx * 40
        header = struct.unpack_from("<8sIIIIIIHHI", data, offset)
        virtual_size = header[1]
        virtual_address = header[2]
        raw_size = header[3]
        raw_pointer = header[4]
        sections.append(Section(virtual_address, virtual_size, raw_size, raw_pointer))
    return sections


def rva_to_offset(rva: int, sections: Sequence[Section]) -> int | None:
    for section in sections:
        size = max(section.virtual_size, section.raw_size)
        start = section.virtual_address
        end = start + size
        if start <= rva < end:
            return section.raw_pointer + (rva - start)
    return None


def _read_c_string(data: memoryview, offset: int) -> str:
    end = offset
    length = len(data)
    while end < length and data[end] != 0:
        end += 1
    return data[offset:end].tobytes().decode("ascii", errors="ignore")


def build_bin_map(bin_dir: Path) -> Dict[str, Path]:
    return {path.name.lower(): path for path in bin_dir.glob("*.dll")}


def copy_dlls(
    dlls: Iterable[str],
    bin_map: Dict[str, Path],
    dest_dir: Path,
    *,
    dry_run: bool,
) -> None:
    dest_dir.mkdir(parents=True, exist_ok=True)
    copied: List[str] = []
    missing: List[str] = []

    for dll in dlls:
        source = bin_map.get(dll.lower())
        if not source:
            missing.append(dll)
            continue

        target = dest_dir / source.name
        if dry_run:
            action = "would overwrite" if target.exists() else "would copy"
            print(f"[dry-run] {action} {source} -> {target}")
            copied.append(source.name)
            continue

        shutil.copy2(source, target)
        print(f"Copied {source.name} -> {target}")
        copied.append(source.name)

    print()
    print(f"DLLs copied: {len(copied)}")
    if copied:
        print("  " + ", ".join(copied))
    if missing:
        print(f"DLLs missing in vcpkg bin: {len(missing)}")
        print("  " + ", ".join(missing))
    else:
        print("All requested DLLs were located in the vcpkg bin directory.")


def main() -> int:
    args = parse_args()

    exe_path = resolve_path(args.exe)
    if not exe_path.exists():
        print(f"Executable not found: {exe_path}", file=sys.stderr)
        return 1

    dest_dir = resolve_path(args.dest, default_base=exe_path.parent) if args.dest else exe_path.parent

    if args.bin_dir is not None:
        bin_dir = resolve_path(args.bin_dir)
    else:
        vcpkg_root = args.vcpkg_root or os.environ.get("VCPKG_ROOT")
        triplet = args.triplet or os.environ.get("VCPKG_DEFAULT_TRIPLET")
        if not vcpkg_root or not triplet:
            print(
                "Provide --bin-dir or ensure VCPKG_ROOT and VCPKG_DEFAULT_TRIPLET are set.",
                file=sys.stderr,
            )
            return 2
        bin_dir = Path(vcpkg_root).expanduser().resolve() / "installed" / triplet / "bin"

    if not bin_dir.exists():
        print(f"vcpkg bin directory not found: {bin_dir}", file=sys.stderr)
        return 3

    print(f"Inspecting imports from {exe_path}")
    dlls = read_imported_dlls(exe_path)
    pprint.pprint(dlls)
    if not dlls:
        print("No DLL imports discovered; nothing to copy.")
        return 0

    print(f"Found {len(dlls)} imported DLLs")
    bin_map = build_bin_map(bin_dir)
    if not bin_map:
        print(f"No DLLs present in {bin_dir}", file=sys.stderr)
        return 4

    copy_dlls(dlls, bin_map, dest_dir, dry_run=args.dry_run)
    return 0


if __name__ == "__main__":
    sys.exit(main())
