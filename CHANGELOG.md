# Changelog

## Unreleased

### Changes
- `npdf`: raise the DPI back for text-bearing pages whose imagery would soften the text (`--skip-text-dpi-floor` to opt out)
- `tiny-poppler`: report `has_text` on `PageInfo`, from the collector's text callback
- `npdf`: add the ML colour-page check for `--color auto`, via `mangacolor` (opt out with `--skip-ml-color-check`)
- `npdf`: add `--output-dpi` to `export`, supersampling pages above that resolution and averaging them down
- `npdf`: auto-DPI is now set by a page's dominant imagery (sharpest-first coverage accumulation) instead of its weakest image
- `npdf`: add `--min-image-coverage`, `--min-image-dim` and `--color-export-dpi` to `export`
- `npdf`: fix auto-DPI taking the multi-image path for a single image that carries a soft mask
- docs: add `docs/plans/` with implementation plans for porting the `sandbox/` prototypes
- `tiny-poppler`: fix issues with extraction not working as intended for softmask/mask/stencil
- `tiny-poppler`: re-prefix all function with `ntsplash_` to avoid name conflicts
- `tiny-poppler`: add page colorspace information to `PageInfo`
- `sjpegli`: avoid using `strncpy` which may not null-terminate the string properly
- `sjpegli`: re-prefix all enum values with `SJ_` to avoid name conflicts
- `npdf`: adjust the text when rendering pages
- `npdf`: fix export page numbering still wrong when in reversed order mode
- `npdf`: fix colorspace fixing command to not messed up color on `SCN` operator
- `npdf`: use `color-eyre`'s for error handling with better context and backtrace support
- `npdf`: make sure we check page colorspace information when doing auto color render/export
- third-party: update `poppler` to `956c68d0261969a79654a191de7ba3b13713706d` (v26.1.99 or 26.1.0)

### Build
- Change `opt-level` to `s` instead of `z` for better performance with *similar* binary size

## [0.1.0] (2025-12-27)

Initial release version of `npdf`
