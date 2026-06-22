from __future__ import annotations

from conftest import sample
from tiny_poppler import ColorMode, Document
from tiny_poppler.pillow import image_info_to_pil, rendered_to_pil


def test_render_cid_one():
    doc = Document.open(str(sample("font_cid_1.pdf")))
    assert doc.page_count == 1

    image = doc.render_page(0, dpi=150.0)
    assert image.width > 0
    assert image.height > 0
    assert image.color_mode == ColorMode.Rgb8
    assert image.components == 3
    assert image.bits_per_component == 8

    png_bytes = image_info_to_pil(image, format="PNG")
    assert png_bytes[:8] == b"\x89PNG\r\n\x1a\n"

    pil_img = rendered_to_pil(image)
    assert pil_img.mode == "RGB"


def test_render_cmyk():
    doc = Document.open(str(sample("image_cmyk_jpg.pdf")))
    assert doc.page_count == 1

    image = doc.render_page(0, dpi=150.0, color_mode=ColorMode.Cmyk8)
    assert image.color_mode == ColorMode.Cmyk8
    assert image.components == 4

    tiff_bytes = image_info_to_pil(image, format="TIFF")
    assert len(tiff_bytes) > 0
    assert tiff_bytes[:4] in (b"II*\x00", b"MM\x00*")


def test_render_grayscale():
    doc = Document.open(str(sample("font_cid_1.pdf")))
    image = doc.render_page(0, dpi=72.0, color_mode=ColorMode.Mono8)
    assert image.color_mode == ColorMode.Mono8
    assert image.components == 1
    assert image.bits_per_component == 8

    pil_img = rendered_to_pil(image)
    assert pil_img.mode == "L"
