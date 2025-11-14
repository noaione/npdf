"""Quick packaging tools for npdf."""

import os
import sys
import tarfile
import zipfile
from pathlib import Path

MATRIX_NAME = os.getenv("MATRIX_NAME", "unknown").lower()
ROOT_DIR = Path(__file__).resolve().parent.parent
TARGET_DIR = ROOT_DIR / "target" / "release"
IS_WINDOWS = sys.platform.startswith("win")

if IS_WINDOWS:
    BINARY_NAME = "npdf.exe"
    ARCHIVE_NAME = f"npdf-{MATRIX_NAME}.zip"
else:
    BINARY_NAME = "npdf"
    ARCHIVE_NAME = f"npdf-{MATRIX_NAME}.tar.gz"


def create_archive() -> int:
    """Create an archive containing the built binary."""
    binary_path = TARGET_DIR / BINARY_NAME
    if not binary_path.exists():
        print(f"Built binary not found: {binary_path}", file=sys.stderr)
        return 1

    archive_path = ROOT_DIR / ARCHIVE_NAME
    if IS_WINDOWS:
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED, compresslevel=7) as zipf:
            zipf.write(binary_path, arcname=BINARY_NAME)
            # Loop and get DLLs in the same directory
            for item in TARGET_DIR.glob("*.dll"):
                zipf.write(item, arcname=item.name)
    else:
        with tarfile.open(archive_path, "w:gz") as tarf:
            tarf.add(binary_path, arcname=BINARY_NAME)

    print(f"Created archive: {archive_path}")
    return 0


if __name__ == "__main__":
    sys.exit(create_archive())
