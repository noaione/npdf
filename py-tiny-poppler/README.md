# tiny-poppler

Python bindings for the tiny-poppler C/C++ bridge, linking directly against
Poppler's Splash backend. No Rust intermediary.

## Features

- Render PDF pages to raw numpy arrays (RGB8, CMYK8, Mono8, etc.)
- Collect image metadata across pages
- Extract embedded images in their native formats (PNG, JPEG, JBIG2, CCITT, etc.)
- Optional Pillow integration for encoding/convenience

## Installation

```bash
pip install tiny-poppler
```

For Pillow support:

```bash
pip install 'tiny-poppler[pillow]'
```

## Quick start

```python
import tiny_poppler
from tiny_poppler import Document, ColorMode

doc = Document.open("document.pdf")
print(doc.page_count)

image = doc.render_page(0, dpi=150.0, color_mode=ColorMode.Rgb8)
print(image.data.shape)  # (height, width, 3)

collection = doc.collect_images()
for info in collection.images:
    print(info.width, info.height, info.xref())

exported = doc.export_image(0, xref_object=4, xref_generation=0)
print(len(exported.data))
```

## License

GPL-3.0-or-later
