from __future__ import annotations

import math

from conftest import sample
from tiny_poppler import (
    Document,
    ImageExportExtension,
    ImageExportFormat,
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
