# Vendored alacritty_terminal 0.26.0 (Koral patch)

Provenance:

- Upstream crate: `alacritty_terminal` 0.26.0, Apache-2.0.
- Source: the published crates.io package, built from alacritty/alacritty commit
  `94e7c8874e526b1e67b349d9ba30ddf81669119e` (recorded in `.cargo_vcs_info.json`).
- Vendored files: `src/`, `Cargo.toml`, `README.md`, `LICENSE-APACHE`,
  `.cargo_vcs_info.json`, plus `koral.patch` and this note. The crate's 46 MB
  `tests/ref` fixtures are not vendored; `Cargo.toml` sets `autotests = false`.
- No other file differs from the published crate. `koral.patch` is the complete
  difference against the registry source.

## Patch

`Term::write_at_cursor` replaces the glyph, colours, flags and metadata of the
cell it writes. Koral draws a picture from the cell it rides on, whose anchor and
foot are zerowidth marks in the top half of plane 16 (`src/view.rs`:
`PICTURE_BASE = 0x10_8000`, `picture_foot() = 0x10_FFFF`). Writing text over
that cell must not delete the picture, so the patch copies only those marks from
the old cell into the replacement cell. Every other zerowidth mark, hyperlink,
underline colour and attribute is replaced exactly as before.

The common path (a cell with no Koral picture) checks `zerowidth()` once, builds
an empty `Vec` that never allocates, and behaves as upstream.

Erase and reset paths are deliberately untouched: `ED 2`, reset and the
alternate-screen reset still clear pictures, while `ED 3` only drops the
scrollback and leaves the visible cells (and their pictures) alone. A wide glyph
overwrites its two cells through `write_at_cursor`, so an anchor on either of
them survives; clearing half of a wide character via `clear_wide` remains an
erase and is not preserved.

To update the crate, replace this directory with the next published version,
reapply the two KORAL PATCH hunks, and regenerate `koral.patch`.
