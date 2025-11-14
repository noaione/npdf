# npdf

an opinionated pdf-to-png exporter using poppler splash output renderer 

## why?

poppler/pdftoppm itself is already solid CLI tools but lacks some features that I need:
- automatic color mode selection based on images (`--color auto`)
- automatically chose DPI based on the possible page size (`--auto-dpi vertical`)

this version of exporter is custom-made for people that want to export their comic and manga PDF bought from
somewhere into a collection of PNGs.

as for why I use the SplashOutput renderer? mainly because it works much better on grayscale/mono images compared to the cairo backend.<br />
this is why I don't use any pre-existing glib/cairo backend available in crates.io.

## usage

```
npdf list <pdf_file>
```

List all available images in the PDF files, can be used to help determine what your DPI would be.

```
npdf export <pdf_file> <output_dir>
```

Export the PDF into `<output_dir>` as PNG images. Use `-h`/`--help` to see options (DPI, color mode, page ranges, etc.).

## license

GPL-3.0-or-later as the poppler/xpdf library is licensed in GPL
