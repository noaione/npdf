from __future__ import annotations

import math

from conftest import sample
from tiny_poppler import (
    Document,
    ImageColorSpace,
    ImageExportExtension,
    ImageExportFormat,
    ImageExportType,
    ImageType,
)

FLOAT_TOLERANCE = 1e-5


def assert_close(actual: float, expected: float, label: str) -> None:
    delta = abs(actual - expected)
    assert delta <= FLOAT_TOLERANCE, f"{label} expected {expected} but got {actual} (delta={delta})"


def test_describe_from_rgb8():
    doc = Document.open(str(sample("image_rgb8.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.page == 1
    assert info.width == 200
    assert info.height == 200
    assert info.xref() == (4, 0)
    assert info.image_type == ImageType.Image

    exported = doc.export_image(
        0,
        xref_object=4,
        xref_generation=0,
        describe_only=True,
    )
    assert exported.data == b""
    assert exported.width == 200
    assert exported.height == 200
    assert exported.stride == 600
    assert exported.components == 3
    assert exported.bits_per_component == 8
    assert exported.format == ImageExportFormat.Rgb
    assert exported.extension == ImageExportExtension.Png
    assert math.isclose(exported.width_dpi, 72.0)
    assert math.isclose(exported.height_dpi, 72.0)
    assert exported.jbig2_globals is None
    assert exported.ccitt_params is None


def test_describe_from_jbig2_with_globals():
    doc = Document.open(str(sample("image_jbig2_withglobals.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.page == 1
    assert info.width == 1747
    assert info.height == 2554
    assert info.image_type == ImageType.Image
    assert info.components == 1
    assert info.bits_per_component == 1
    assert info.colorspace == ImageColorSpace.DeviceGray
    assert_close(info.dpi()[0], 300.0, "metadata width DPI")
    assert_close(info.dpi()[1], 300.0, "metadata height DPI")
    assert info.xref() == (799, 0)

    exported = doc.export_image(
        0,
        xref_object=799,
        xref_generation=0,
        describe_only=True,
    )
    assert exported.data == b""
    assert exported.width == 1747
    assert exported.height == 2554
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Jbig2
    assert exported.jbig2_globals is not None
    assert exported.ccitt_params is None


def test_describe_from_ccitt_group3():
    doc = Document.open(str(sample("image_ccit_3.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.page == 1
    assert info.width == 1451
    assert info.height == 2528
    assert info.image_type == ImageType.Stencil
    assert info.components == 1
    assert info.bits_per_component == 1
    assert info.colorspace == ImageColorSpace.Unknown
    assert_close(info.dpi()[0], 300.0, "metadata width DPI")
    assert_close(info.dpi()[1], 300.0, "metadata height DPI")
    assert info.xref() == (8, 0)

    exported = doc.export_image(
        0,
        xref_object=8,
        xref_generation=0,
        target_type=ImageExportType.Stencil,
        describe_only=True,
    )
    assert exported.data == b""
    assert exported.width == 1451
    assert exported.height == 2528
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Ccitt
    assert exported.ccitt_params is not None

    params = exported.ccitt_params
    assert params.encoding == -1
    assert params.columns == 1451
    assert params.rows == 2528
    assert params.damaged_rows_before_error == 0
    assert not params.end_of_line
    assert not params.byte_align
    assert params.end_of_block
    assert not params.black_is_one


def test_describe_from_rgba8_with_softmask():
    doc = Document.open(str(sample("image_rgba8.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 2

    image = collection.images[0]
    assert image.page == 1
    assert image.width == 200
    assert image.height == 200
    assert image.components == 3
    assert image.bits_per_component == 8
    assert image.image_type == ImageType.Image
    assert image.colorspace == ImageColorSpace.DeviceRgb
    assert image.xref() == (5, 0)

    soft_mask = collection.images[1]
    assert soft_mask.image_type == ImageType.SoftMask
    assert soft_mask.components == 1
    assert soft_mask.bits_per_component == 8
    assert soft_mask.colorspace == ImageColorSpace.DeviceGray

    exported = doc.export_image(
        0,
        xref_object=5,
        xref_generation=0,
        describe_only=True,
    )
    assert exported.data == b""
    assert exported.width == 200
    assert exported.height == 200
    assert exported.stride == 600
    assert exported.components == 3
    assert exported.bits_per_component == 8
    assert exported.format == ImageExportFormat.Rgb
    assert exported.extension == ImageExportExtension.Png

    mask_export = doc.export_image(
        0,
        xref_object=5,
        xref_generation=0,
        target_type=ImageExportType.SoftMask,
        describe_only=True,
    )
    assert mask_export.data == b""
    assert mask_export.width == 200
    assert mask_export.height == 200
    assert mask_export.stride == 200
    assert mask_export.components == 1
    assert mask_export.bits_per_component == 8
    assert mask_export.format == ImageExportFormat.Gray
    assert mask_export.extension == ImageExportExtension.Png


def test_describe_from_cmyk_jpeg():
    doc = Document.open(str(sample("image_cmyk_jpg.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 200
    assert info.height == 200
    assert info.components == 4
    assert info.bits_per_component == 8
    assert info.image_type == ImageType.Image
    assert info.colorspace == ImageColorSpace.DeviceCmyk
    assert info.xref() == (4, 0)

    exported = doc.export_image(
        0,
        xref_object=4,
        xref_generation=0,
        describe_only=True,
    )
    assert exported.data == b""
    assert exported.width == 200
    assert exported.height == 200
    assert exported.components == 4
    assert exported.bits_per_component == 8
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Jpeg
