# npdf

an opinionated pdf-to-png exporter using poppler splash output renderer 

## why?

poppler/pdftoppm itself is already solid CLI tools but lacks some features that I need:
- automatic color mode selection based on images (`--color auto`)
- automatically chose DPI based on the possible page size (`--auto-dpi vertical`)
- "native" multi-threaded exporting (see info below)

this version of exporter is custom-made for people that want to export their comic and manga PDF bought from
somewhere into a collection of PNGs.

as for why I use the `SplashOutput` renderer? mainly because it works much better on grayscale/mono images compared to the cairo backend.<br />
this is why I don't use any pre-existing glib/cairo backend available in crates.io.

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
pass `--threads 1` to run single-threaded or `--threads N` to clamp the worker count.<br />
currently, the way this is implemented is that the main thread will parse the PDF once and then spawn worker threads that each open their own isolated `Document` instance from a shared `DocumentFactory`.<br />
since i don't think poppler's `PDFDoc` is thread-safe *yet* to allow sharing between threads.

## disclaimer

this project is not affiliated with or endorsed by the poppler/xpdf project or its maintainers. use at your own risk.

i also haven't ran `miri` yet on this since i need some example PDFs first that could help test some stuff.

## license

GPL-3.0-or-later as the poppler/xpdf library is licensed in GPL
