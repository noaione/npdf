"""Tiny-Poppler: fast Python bindings to Poppler's Splash backend."""

from __future__ import annotations

from tiny_poppler._core import (  # pyright: ignore[reportMissingModuleSource]
    ColorMode,
    CropMode,
    Document,
    ExportedImage,
    ImageCollection,
    ImageColorSpace,
    ImageExportExtension,
    ImageExportFormat,
    ImageExportType,
    ImageInfo,
    ImageType,
    PageInfo,
    RenderedImage,
    ZeroWidthLineMode,
    get_version,
)

__all__ = [
    "ColorMode",
    "CropMode",
    "Document",
    "ExportedImage",
    "ImageCollection",
    "ImageColorSpace",
    "ImageExportExtension",
    "ImageExportFormat",
    "ImageExportType",
    "ImageInfo",
    "ImageType",
    "PageInfo",
    "RenderedImage",
    "ZeroWidthLineMode",
    "get_version",
]
