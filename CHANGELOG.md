# Changelog

## Unreleased

### Changes
- `tiny-poppler`: fix issues with extraction not working as intended for softmask/mask/stencil
- `tiny-poppler`: re-prefix all function with `ntsplash_` to avoid name conflicts
- `npdf`: adjust the text when rendering
- `sjpegli`: avoid using `strncpy` which may not null-terminate the string properly
- `sjpegli`: re-prefix all enum values with `SJ_` to avoid name conflicts

## [0.1.0] (2025-12-27)

Initial release version of `npdf`
