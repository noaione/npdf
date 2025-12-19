# npdf

an opinionated pdf-to-png exporter/renderer using poppler splash output renderer 

## why?

poppler/pdftoppm itself is already solid CLI tools but lacks some features that I need:
- automatic color mode selection based on images (`--color auto`)
- automatically chose DPI based on the possible page size (`--auto-dpi vertical`)
- "native" multi-threaded exporting (see info below)
- no need to install poppler/pdftoppm separately, it's already bundled in the binary

this version of exporter/renderer is custom-made for people that want to export their comic and manga PDF bought from
somewhere into a collection of PNGs.

as for why I use the `SplashOutput` renderer? mainly because it works much better on grayscale/mono images compared to the cairo backend.<br />
this is why I don't use any pre-existing glib/cairo backend available in crates.io.

## download

this project has built-in nightly CI that build for multiple platforms:
- [nightly.link](https://nightly.link/noaione/npdf/workflows/build/master?preview)
- [latest CI build](https://github.com/noaione/npdf/actions/workflows/build.yml)

until I find this project "ready" there would be no official releases/tags.

## requirements

when building from source, you'll need the following tools installed:
- Rust toolchain (obviously)
- cmake
- pkg-config
- ninja
- C++ compiler that support C++23 standard

and the following libraries installed and available for linking:
- zlib
- freetype
    - bzip2
    - brotli
    - lzma/xz
- fontconfig
    - expat
- turbojpeg
- openjpeg
- libtiff
- lcms2
- libpng
- iconv (macOS)
- nss3 + nspr4 (for encrypted PDFs)

most of the dependencies in CI has been statically linked into the final binary, with exception:
- `libiconv` on macOS (dynamically linked against system version)
- `zlib` on Linux/macOS (dynamically linked against system version)
- `nss3` and `nspr4` on Linux/macOS
- `lcms2` on Linux

windows version is fully statically linked, so you don't need to install anything extra (except maybe VC++ runtime).

## usage

### list
```
npdf list <pdf_file>
```

list all available images in the PDF files, can be used to help determine what your DPI would be.

### export
```
npdf export <pdf_file> <output_dir>
```

export the PDF into `<output_dir>` as PNG images. Use `-h`/`--help` to see options (DPI, color mode, page ranges, etc.).

by default `npdf export` spawns multiple workers (one per logical CPU).<br />
pass `--threads 1` to run single-threaded or `--threads N` to clamp the worker count.

currently, the way this is implemented is that the main thread will parse the PDF once and then spawn worker threads that each open their own isolated `Document` instance from a shared `DocumentFactory`.<br />
since i don't think poppler's `PDFDoc` is thread-safe *yet* to allow sharing between threads.

when using `--force-cmyk` option, the exported file would use JPEG format instead as PNG does not support CMYK color mode.<br />
this is powered with [`jpegli`/`libjxl`](https://github.com/libjxl/libjxl) via [`simple-jpegli-enc`](https://github.com/noaione/npdf/tree/master/simple-jpegli-enc) crate.

you can also use the `--cmyk-png` option to use CMYK as rendering color mode but still export as PNG (with color conversion to RGB/Gray).

### extract
```
npdf extract <pdf_file> <output_dir>
```

extract all images embedded in the PDF file and save them into `<output_dir>`.<br />
the images will be saved using their original format (png/jpg/tiff/etc.) if possible

this command should be similar to `pdfimages` from poppler/xpdf.

### unwatermark
```
npdf unwatermark <pdf_file> <output_file>
```

remove watermark images from the PDF file and save the cleaned PDF into `<output_file>`.<br />
this command works by scanning all images in the PDF and hashing them, then asking the user to select which images to remove based on their hashes.<br />
note that this command may not be able to remove all watermarks, especially if the watermark is drawn using vector graphics or text.

this use [`lopdf`](https://crates.io/crates/lopdf) crate to manipulate the PDF file directly.

### recrop
```
npdf recrop --cropbox <crop_mode> <pdf_file> <output_file>
```

recrop all pages in the PDF file using the specified crop mode and save the recropped PDF into `<output_file>`.

`<crop_mode>` can be one of the following:
- `crop`: use the existing CropBox (or revert from previous `OriginalCropBox` if present)
- `media`: use the MediaBox
- `bleed`: use the BleedBox
- `trim`: use the TrimBox
- `art`: use the ArtBox

this use [`qpdf-rs`](https://crates.io/crates/qpdf-rs) crate to manipulate the PDF file directly.

## other tools

this repository also contains some small tools to work with PDF files using `pikepdf` (which use `qpdf` as backend).

see the [tools/README.md](./tools/README.md) for more info.

## disclaimer

this project is not affiliated with or endorsed by the poppler/xpdf project or its maintainers. use at your own risk.

## license

GPL-3.0-or-later as the poppler/xpdf library is licensed in GPL

## acknowledgements

- thanks to the [poppler](https://poppler.freedesktop.org/)/xpdf project for their great rendering library for PDF files.
- the [libjxl](https://github.com/libjxl/libjxl) project for the great JPEG compression library (especially the jpegli bridge).
- the [qpdf](https://github.com/qpdf/qpdf) (and the [rust-binding](https://github.com/ancwrd1/qpdf-rs)) project for their PDF manipulation library.
