# Changelog

## Unreleased

### Changes
- `tiny-poppler`: fix issues with extraction not working as intended for softmask/mask/stencil
- `tiny-poppler`: re-prefix all function with `ntsplash_` to avoid name conflicts
- `tiny-poppler`: add page colorspace information to `PageInfo`
- `sjpegli`: avoid using `strncpy` which may not null-terminate the string properly
- `sjpegli`: re-prefix all enum values with `SJ_` to avoid name conflicts
- `npdf`: adjust the text when rendering pages
- `npdf`: fix export page numbering still wrong when in reversed order mode
- `npdf`: fix colorspace fixing command to not messed up color on `SCN` operator
- `npdf`: use `color-eyre`'s for error handling with better context and backtrace support
- third-party: update `poppler` to `956c68d0261969a79654a191de7ba3b13713706d` (v26.01.99)

### Build
- Change `opt-level` to `s` instead of `z` for better performance with *similar* binary size

## [0.1.0] (2025-12-27)

Initial release version of `npdf`
