# Significant feature changes and additions

This project follows Rust semantic versioning.

<!-- see keepachangelog.com for format ideas -->

## 0.1.0 (2026-03-22)

### Additions

- Page-based updates (like curses)
- Tile handling, with automatic updates

### Breaking changes

- `TermOut` becomes `Output`
- `Share<TermOut>` becomes `TermShare`
- Some methods on `Output` are renamed (e.g. `attr_reset` to
  `attr_99`, `*_cursor` to `cursor_*`, etc)
- `Terminal::flush` is no longer available, but it is now called
  automatically on `Output::flush`
- `Terminal::init` requires a new "sizer" argument (pass
  `SimpleSizer::new()` to start with, then read docs)

<!-- Local Variables: -->
<!-- mode: markdown -->
<!-- End: -->
