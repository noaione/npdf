# tiny-poppler

A highly opinioned PDF rendering library based on Poppler C++ bindings.

This does not use Cairo backend but rather the built-in poppler backend.

## Threaded rendering

For multi-threaded exporters (like the `npdf` CLI) prefer using `DocumentFactory`:

1. Call `DocumentFactory::with_images(path)` once on the main thread to parse the file, cache the page count, and snapshot image metadata for heuristics.
2. Clone the factory for each worker thread and call `factory.open()` to obtain an isolated `Document` bound to that thread.
3. Never share a `Document` between threads; Poppler expects each `PDFDoc` instance to be used from a single thread at a time.

This pattern amortizes the expensive parse step while keeping each renderer thread-safe.

## Sanitizer builds

The crate exposes optional build features that wire common compiler sanitizers through the
Poppler build and the C++ bridge. Enable exactly one of `asan`, `lsan`, `tsan`, or `ubsan` when
invoking Cargo to instrument native code:

```bash
cargo test -p tiny-poppler --features asan
```

When enabled, the build script also surfaces `tiny_poppler_has_sanitizer` and
`tiny_poppler_sanitizer="<name>"` configuration flags at compile time so downstream crates can
adjust behaviour if needed.
