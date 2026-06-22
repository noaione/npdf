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
from tiny_poppler.pillow import exported_to_pil, sink_exported_image

FLOAT_TOLERANCE = 1e-5


def assert_close(actual: float, expected: float, label: str) -> None:
    delta = abs(actual - expected)
    assert delta <= FLOAT_TOLERANCE, f"{label} expected {expected} but got {actual} (delta={delta})"


def test_extract_from_rgb8():
    doc = Document.open(str(sample("image_rgb8.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.page == 1
    assert info.width == 200
    assert info.height == 200
    assert info.image_type == ImageType.Image
    assert info.components == 3
    assert info.bits_per_component == 8
    assert_close(info.dpi()[0], 72.0, "metadata width DPI")
    assert_close(info.dpi()[1], 72.0, "metadata height DPI")
    assert info.xref() == (4, 0)

    exported = doc.export_image(0, xref_object=4, xref_generation=0)
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
    assert exported.data

    pil_img = exported_to_pil(exported)
    assert pil_img.mode == "RGB"
    assert pil_img.size == (200, 200)

    png_bytes = sink_exported_image(exported)
    assert png_bytes[:8] == b"\x89PNG\r\n\x1a\n"


def test_extract_from_jbig2_with_globals():
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

    xref_meta = info.xref()
    assert xref_meta is not None

    obj, generation = xref_meta
    assert obj == 799
    assert generation == 0

    exported = doc.export_image(0, xref_object=obj, xref_generation=generation)
    assert exported.width == 1747
    assert exported.height == 2554
    assert exported.stride == 0
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Jbig2
    assert math.isclose(exported.width_dpi, 72.0)
    assert math.isclose(exported.height_dpi, 72.0)
    assert exported.ccitt_params is None
    assert exported.data
    assert len(exported.data) == 2577

    assert exported.jbig2_globals is not None
    assert len(exported.jbig2_globals) == 80537


def test_extract_from_ccitt_group3():
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

    xref_meta = info.xref()
    assert xref_meta is not None

    obj, generation = xref_meta
    assert obj == 8
    assert generation == 0

    exported = doc.export_image(
        0,
        xref_object=obj,
        xref_generation=generation,
        target_type=ImageExportType.Stencil,
    )
    assert exported.width == 1451
    assert exported.height == 2528
    assert exported.stride == 0
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Ccitt
    assert math.isclose(exported.width_dpi, 72.0)
    assert math.isclose(exported.height_dpi, 72.0)
    assert exported.jbig2_globals is None
    assert exported.data
    assert len(exported.data) == 27619

    params = exported.ccitt_params
    assert params is not None
    assert params.encoding == -1  # Group 3 1D
    assert params.columns == 1451
    assert params.rows == 2528
    assert params.damaged_rows_before_error == 0
    assert not params.end_of_line
    assert not params.byte_align
    assert params.end_of_block
    assert not params.black_is_one


def test_extract_from_rgba8_with_softmask():
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
    assert soft_mask.page == 1
    assert soft_mask.width == 200
    assert soft_mask.height == 200
    assert soft_mask.components == 1
    assert soft_mask.bits_per_component == 8
    assert soft_mask.image_type == ImageType.SoftMask
    assert soft_mask.colorspace == ImageColorSpace.DeviceGray

    exported = doc.export_image(0, xref_object=5, xref_generation=0)
    assert exported.width == 200
    assert exported.height == 200
    assert exported.stride == 600
    assert exported.components == 3
    assert exported.bits_per_component == 8
    assert exported.format == ImageExportFormat.Rgb
    assert exported.extension == ImageExportExtension.Png
    assert math.isclose(exported.width_dpi, 72.0)
    assert math.isclose(exported.height_dpi, 72.0)
    assert len(exported.data) == 120000
    assert exported.jbig2_globals is None
    assert exported.ccitt_params is None

    mask_export = doc.export_image(
        0,
        xref_object=5,
        xref_generation=0,
        target_type=ImageExportType.SoftMask,
    )
    assert mask_export.width == 200
    assert mask_export.height == 200
    assert mask_export.stride == 200
    assert mask_export.components == 1
    assert mask_export.bits_per_component == 8
    assert mask_export.format == ImageExportFormat.Gray
    assert mask_export.extension == ImageExportExtension.Png
    assert math.isclose(mask_export.width_dpi, 72.0)
    assert math.isclose(mask_export.height_dpi, 72.0)
    assert len(mask_export.data) == 40000
    assert mask_export.jbig2_globals is None
    assert mask_export.ccitt_params is None


def test_extract_from_rgba16_with_softmask():
    doc = Document.open(str(sample("image_rgba16.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 2

    image = collection.images[0]
    assert image.width == 200
    assert image.height == 200
    assert image.components == 3
    assert image.bits_per_component == 16
    assert image.image_type == ImageType.Image
    assert image.xref() == (5, 0)

    soft_mask = collection.images[1]
    assert soft_mask.image_type == ImageType.SoftMask
    assert soft_mask.components == 1
    assert soft_mask.bits_per_component == 16

    exported = doc.export_image(0, xref_object=5, xref_generation=0)
    assert exported.width == 200
    assert exported.height == 200
    assert exported.stride == 1200
    assert exported.components == 3
    assert exported.bits_per_component == 16
    assert exported.format == ImageExportFormat.Rgb48
    assert exported.extension == ImageExportExtension.Png
    assert len(exported.data) == 240000
    assert math.isclose(exported.width_dpi, 72.0)
    assert math.isclose(exported.height_dpi, 72.0)

    mask_export = doc.export_image(
        0,
        xref_object=5,
        xref_generation=0,
        target_type=ImageExportType.SoftMask,
    )
    assert mask_export.width == 200
    assert mask_export.height == 200
    assert mask_export.stride == 200
    assert mask_export.components == 1
    assert mask_export.bits_per_component == 8
    assert mask_export.format == ImageExportFormat.Gray
    assert mask_export.extension == ImageExportExtension.Png
    assert math.isclose(mask_export.width_dpi, 72.0)
    assert math.isclose(mask_export.height_dpi, 72.0)
    assert len(mask_export.data) == 40000
    assert mask_export.jbig2_globals is None
    assert mask_export.ccitt_params is None


def test_extract_from_rgb16():
    doc = Document.open(str(sample("image_rgb16.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 200
    assert info.height == 200
    assert info.components == 3
    assert info.bits_per_component == 16
    assert info.image_type == ImageType.Image
    assert info.xref() == (4, 0)

    exported = doc.export_image(0, xref_object=4, xref_generation=0)
    assert exported.width == 200
    assert exported.height == 200
    assert exported.components == 3
    assert exported.bits_per_component == 16
    assert exported.stride == 1200
    assert exported.format == ImageExportFormat.Rgb48
    assert exported.extension == ImageExportExtension.Png
    assert len(exported.data) == 240000


def test_extract_from_cmyk_jpeg():
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

    exported = doc.export_image(0, xref_object=4, xref_generation=0)
    assert exported.width == 200
    assert exported.height == 200
    assert exported.components == 4
    assert exported.bits_per_component == 8
    assert exported.stride == 0
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Jpeg
    assert len(exported.data) == 8001


def test_extract_from_luma8():
    doc = Document.open(str(sample("image_luma8.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 200
    assert info.height == 200
    assert info.components == 1
    assert info.bits_per_component == 8
    assert info.colorspace == ImageColorSpace.DeviceGray

    exported = doc.export_image(0, xref_object=4, xref_generation=0)
    assert exported.width == 200
    assert exported.height == 200
    assert exported.components == 1
    assert exported.bits_per_component == 8
    assert exported.stride == 200
    assert exported.format == ImageExportFormat.Gray
    assert exported.extension == ImageExportExtension.Png
    assert len(exported.data) == 40000


def test_extract_from_luma16():
    doc = Document.open(str(sample("image_luma16.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 200
    assert info.height == 200
    assert info.components == 1
    assert info.bits_per_component == 16

    exported = doc.export_image(0, xref_object=4, xref_generation=0)
    assert exported.width == 200
    assert exported.height == 200
    assert exported.components == 1
    assert exported.bits_per_component == 8
    assert exported.stride == 200
    assert exported.format == ImageExportFormat.Gray
    assert exported.extension == ImageExportExtension.Png
    assert len(exported.data) == 40000


def test_extract_from_one_bit_gray():
    doc = Document.open(str(sample("image_1_bit_per_component.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 256
    assert info.height == 256
    assert info.components == 1
    assert info.bits_per_component == 1
    assert info.colorspace == ImageColorSpace.DeviceGray
    assert_close(info.dpi()[0], 71.99100112485938, "metadata width dpi")
    assert_close(info.dpi()[1], 71.99100112485938, "metadata height dpi")

    exported = doc.export_image(0, xref_object=6, xref_generation=0)
    assert exported.width == 256
    assert exported.height == 256
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.stride == 32
    assert exported.format == ImageExportFormat.Monochrome
    assert exported.extension == ImageExportExtension.Png
    assert len(exported.data) == 8192


def test_extract_from_inline_ccitt():
    doc = Document.open(str(sample("image_inline_2.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 138
    assert info.height == 130
    assert info.components == 1
    assert info.bits_per_component == 1
    assert info.colorspace == ImageColorSpace.DeviceGray
    assert info.xref() is None

    exported = doc.export_image(0, occurrence_index=0)
    assert exported.width == 138
    assert exported.height == 130
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.stride == 0
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Ccitt
    assert len(exported.data) == 1585

    params = exported.ccitt_params
    assert params is not None
    assert params.encoding == -1
    assert params.columns == 138
    assert params.rows == 130
    assert params.end_of_block
    assert not params.end_of_line
    assert not params.byte_align
    assert not params.black_is_one


def test_extract_from_ccitt_group1():
    doc = Document.open(str(sample("image_ccit_1.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 415
    assert info.height == 314
    assert info.components == 1
    assert info.bits_per_component == 1

    exported = doc.export_image(0, xref_object=8, xref_generation=0)
    assert exported.width == 415
    assert exported.height == 314
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.stride == 0
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Ccitt
    assert len(exported.data) == 911

    params = exported.ccitt_params
    assert params is not None
    assert params.encoding == -1  # Group 1
    assert params.columns == 415
    assert params.rows == 314
    assert params.damaged_rows_before_error == 0
    assert not params.end_of_line
    assert not params.byte_align
    assert params.end_of_block
    assert not params.black_is_one


def test_extract_from_ccitt_group4():
    doc = Document.open(str(sample("image_ccit_4.pdf")))
    collection = doc.collect_images()
    assert len(collection.images) == 1

    info = collection.images[0]
    assert info.width == 2336
    assert info.height == 2857
    assert info.components == 1
    assert info.bits_per_component == 1

    exported = doc.export_image(
        0,
        xref_object=8,
        xref_generation=0,
        target_type=ImageExportType.Stencil,
    )
    assert exported.width == 2336
    assert exported.height == 2857
    assert exported.components == 1
    assert exported.bits_per_component == 1
    assert exported.stride == 0
    assert exported.format == ImageExportFormat.Unknown
    assert exported.extension == ImageExportExtension.Ccitt
    assert len(exported.data) == 31187

    params = exported.ccitt_params
    assert params is not None
    assert params.encoding == -1  # Group 4
    assert params.columns == 2336
    assert params.rows == 2857
    assert params.damaged_rows_before_error == 0
    assert not params.end_of_line
    assert not params.byte_align
    assert params.end_of_block
    assert not params.black_is_one
