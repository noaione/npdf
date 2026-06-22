"""Optional Pillow integration for tiny-poppler.

This module is only available when Pillow is installed. It converts raw buffers
returned by the core extension into PIL ``Image`` objects.
"""

from __future__ import annotations

import io
from typing import TYPE_CHECKING

import numpy as np

from tiny_poppler import (
    ColorMode,
    ExportedImage,
    ImageExportExtension,
    RenderedImage,
)

try:
    from PIL import Image
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "Pillow is required for tiny_poppler.pillow. "
        "Install it with: pip install 'tiny-poppler[pillow]'"
    ) from exc

if TYPE_CHECKING:
    from PIL.Image import Image as PILImage


def _mode_for_color_mode(color_mode: ColorMode, components: int) -> str:
    if color_mode == ColorMode.Mono1:
        return "1"
    if color_mode == ColorMode.Mono8:
        return "L"
    if color_mode == ColorMode.Rgb8:
        return "RGB"
    if color_mode == ColorMode.Bgr8:
        return "BGR"
    if color_mode == ColorMode.Xbgr8:
        return "RGBA" if components == 4 else "RGB"
    if color_mode == ColorMode.Cmyk8:
        return "CMYK"
    if color_mode == ColorMode.DeviceN8:
        # PIL does not support DeviceN directly. If it happens to have 4
        # components we interpret it as CMYK; otherwise we bail.
        if components == 4:
            return "CMYK"
        raise NotImplementedError(
            f"DeviceN images with {components} components are not supported by Pillow"
        )
    raise ValueError(f"Unsupported color mode: {color_mode}")


def _mono1_to_packed_white_is_one(
    arr: np.ndarray, width: int, height: int
) -> bytes:
    """Unpack Splash Mono1 buffer (1=black) into packed PIL bits (1=white)."""
    out = bytearray((width + 7) // 8 * height)
    for y in range(height):
        dst_row = y * ((width + 7) // 8)
        for x in range(width):
            byte = int(arr[y, x // 8])
            bit = 7 - (x % 8)
            val = (byte >> bit) & 1
            # Invert: Splash 1=black, PIL 1=white.
            if not val:
                out[dst_row + x // 8] |= 1 << (7 - (x % 8))
    return bytes(out)


def rendered_to_pil(image: RenderedImage) -> PILImage:
    """Convert a :class:`RenderedImage` to a Pillow ``Image``.

    Args:
        image: The rendered page image.

    Returns:
        A Pillow ``Image`` in a mode matching the rendered color mode.
    """
    if image.data is None:
        raise ValueError("RenderedImage has no pixel data")

    arr = image.data
    mode = _mode_for_color_mode(image.color_mode, image.components)

    if mode == "1":
        packed = _mono1_to_packed_white_is_one(arr, image.width, image.height)
        return Image.frombytes("1", (image.width, image.height), packed)

    if mode == "BGR":
        # Swap BGR -> RGB.
        rgb = arr[..., ::-1].copy()
        return Image.fromarray(rgb, mode="RGB")

    if mode == "RGBA":
        # arr is XBGR; reorder to RGBA and keep the X/A channel as alpha.
        rgba = arr[..., [2, 1, 0, 3]].copy()
        return Image.fromarray(rgba, mode="RGBA")

    if image.color_mode == ColorMode.Xbgr8:
        # Drop the X channel and reorder BGR -> RGB.
        rgb = arr[..., :3][..., ::-1].copy()
        return Image.fromarray(rgb, mode="RGB")

    return Image.fromarray(arr, mode=mode)


# Keep a convenient alias matching the plan.
to_pil = rendered_to_pil


def exported_to_pil(image: ExportedImage) -> PILImage:
    """Convert an :class:`ExportedImage` to a Pillow ``Image``.

    Encoded streams that Pillow understands (JPEG, JPEG 2000, TIFF, PNM) are
    opened directly.  Raw raster buffers produced by ``writeImageFile`` are
    reconstructed from ``image.format``.

    Args:
        image: The exported embedded image.

    Returns:
        A Pillow ``Image``.
    """
    if not image.data:
        raise ValueError("ExportedImage has no data (describe_only?)")

    from tiny_poppler import ImageExportFormat

    fmt = image.format
    ext = image.extension

    # Encoded streams: let Pillow decode them.
    if fmt == ImageExportFormat.Unknown:
        if ext in (ImageExportExtension.Ccitt, ImageExportExtension.Jbig2):
            raise NotImplementedError(
                f"Extension {ext.name} cannot be opened directly with Pillow"
            )
        return Image.open(io.BytesIO(image.data))

    # Raw raster buffers produced by writeImageFile.
    width = image.width
    height = image.height
    data = bytes(image.data)

    if fmt == ImageExportFormat.Rgb:
        expected = width * height * 3
        if len(data) != expected:
            raise ValueError(f"RGB buffer size mismatch: {len(data)} != {expected}")
        return Image.frombytes("RGB", (width, height), data)

    if fmt == ImageExportFormat.Rgb48:
        arr = np.frombuffer(data, dtype=np.uint16).reshape((height, width, 3))
        # Pillow supports 16-bit RGB via Image.fromarray with mode "RGB" only
        # after downcasting.  Return the raw 16-bit array as I;16 for now.
        return Image.fromarray(arr, mode="RGB")

    if fmt == ImageExportFormat.Gray:
        expected = width * height
        if len(data) != expected:
            raise ValueError(f"Gray buffer size mismatch: {len(data)} != {expected}")
        return Image.frombytes("L", (width, height), data)

    if fmt == ImageExportFormat.Monochrome:
        stride = (width + 7) // 8
        expected = stride * height
        if len(data) != expected:
            raise ValueError(f"Monochrome buffer size mismatch: {len(data)} != {expected}")
        # Splash packs bits with 1=black; PIL mode "1" uses 1=white.
        inverted = bytearray(data)
        for i in range(len(inverted)):
            inverted[i] ^= 0xFF
        return Image.frombytes("1", (width, height), bytes(inverted))

    if fmt == ImageExportFormat.Cmyk:
        expected = width * height * 4
        if len(data) != expected:
            raise ValueError(f"CMYK buffer size mismatch: {len(data)} != {expected}")
        return Image.frombytes("CMYK", (width, height), data)

    raise NotImplementedError(f"Unsupported export format: {fmt}")


def sink_exported_image(image: ExportedImage, *, format: str | None = None) -> bytes:
    """Encode an :class:`ExportedImage` to a bytes buffer.

    This mirrors Rust's ``sink_exported_image``: raw raster buffers are
    encoded with Pillow, while already-encoded streams are returned as-is.

    Args:
        image: The exported embedded image.
        format: Target Pillow format.  If ``None``, the format is chosen from
            ``image.extension``.

    Returns:
        The encoded image as ``bytes``.
    """
    from tiny_poppler import ImageExportFormat

    if not image.data:
        raise ValueError("ExportedImage has no data (describe_only?)")

    fmt = image.format

    # Already encoded stream (JPEG, JPEG 2000, CCITT, JBIG2, ...): pass through.
    if fmt == ImageExportFormat.Unknown:
        return bytes(image.data)

    if format is None:
        if image.extension == ImageExportExtension.Tiff:
            format = "TIFF"
        elif image.extension == ImageExportExtension.Jpeg:
            format = "JPEG"
        else:
            format = "PNG"

    pil_img = exported_to_pil(image)
    buffer = io.BytesIO()
    pil_img.save(buffer, format=format)
    return buffer.getvalue()


def image_info_to_pil(
    image: RenderedImage,
    *,
    format: str | None = None,
    **kwargs: object,
) -> bytes:
    """Encode a :class:`RenderedImage` to a bytes buffer using Pillow.

    Args:
        image: The rendered page image.
        format: Pillow format (e.g. ``"PNG"``, ``"JPEG"``). If ``None``,
            PNG is used unless ``image.color_mode`` is CMYK/DeviceN, in which
            case TIFF is used because JPEG/PNG do not support CMYK well.
        **kwargs: Extra arguments passed to ``PIL.Image.save``.

    Returns:
        The encoded image as ``bytes``.
    """
    pil_img = rendered_to_pil(image)

    if format is None:
        if image.color_mode in (ColorMode.Cmyk8, ColorMode.DeviceN8):
            format = "TIFF"
        else:
            format = "PNG"

    buffer = io.BytesIO()
    pil_img.save(buffer, format=format, **kwargs)
    return buffer.getvalue()
