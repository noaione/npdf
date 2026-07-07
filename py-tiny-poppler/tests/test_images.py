from __future__ import annotations

from conftest import sample
from tiny_poppler import Document


def test_load_cid_one():
    doc = Document.open(str(sample("font_cid_1.pdf")))
    assert doc.page_count == 1

    collection = doc.collect_images()
    assert len(collection.images) == 0
    assert len(collection.pages) == 1

    page = collection.pages[0]
    assert page.page_number == 1
    assert page.image_count == 0
    assert page.object_count == 18
