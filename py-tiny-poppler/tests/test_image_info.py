from __future__ import annotations

from conftest import sample
from tiny_poppler import Document, ImageColorSpace


def test_single_colorspace():
    doc = Document.open(str(sample("colorspaces_single.pdf")))
    collection = doc.collect_images()
    assert len(collection.pages) == 1

    page = collection.pages[0]
    assert "CS0" in page.colorspaces

    cs = page.colorspaces["CS0"]
    assert cs.mode == ImageColorSpace.Separation
    assert cs.separation_name == "All"
    assert cs.alternate is not None
    assert cs.alternate.mode == ImageColorSpace.DeviceGray
