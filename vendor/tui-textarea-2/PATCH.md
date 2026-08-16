# Vendored fork of tui-textarea-2

**Upstream:** https://github.com/srothgan/tui-textarea
**Vendored version:** 0.12.1, copied verbatim from crates.io
**Licence:** MIT (see LICENSE, unmodified)

## Why this fork exists

`recon` dims lines that do not match a filter and colours those that do. The
public API exposes only whole-area, cursor-line, search and line-number
styles — there is no way to style an individual line. The machinery already
exists internally (`LineHighlighter::set_line_style`, used for the cursor
line); this fork only makes it reachable.

## Changes from upstream

### 1. Per-line styles

- `src/textarea.rs`, `struct TextArea`: added field `line_styles: Vec<Option<Style>>`.
- `src/textarea.rs`, `TextArea::new`: initialise it to `Vec::new()`.
- `src/textarea.rs`: added `set_line_styles`, `line_styles`, `clear_line_styles`
  after `line_number_style`.
- `src/textarea.rs`, `line_spans_segment`: apply the entry for `wrapped.row`
  immediately *before* the cursor-line block, so the cursor line still wins.
- `tests/line_presentation.rs`: new.
- `Cargo.toml`: added a `[[test]]` entry for `line_presentation`. This
  generated Cargo.toml sets `autotests = false` with explicit test entries
  (unlike `Cargo.toml.orig`, which relies on auto-discovery), so a new test
  file needs an entry here or cargo won't find it.

### 2. Gutter number overrides

- `src/textarea.rs`, `struct TextArea`: added field `line_numbers: Vec<usize>`.
- `src/textarea.rs`, `TextArea::new`: initialise it to `Vec::new()`.
- `src/textarea.rs`: added `set_line_numbers`, `line_numbers`,
  `clear_line_numbers`, and the crate-internal `display_row` /
  `widest_display_row`.
- `src/textarea.rs`, `line_spans_segment`: pass `display_row(wrapped.row)` to
  `hl.line_number` instead of `wrapped.row`.
- `src/widget.rs`, `text_widget` and `scroll_top_col`: size the gutter from
  `widest_display_row() + 1` rather than `lines().len()`. Required, not
  cosmetic: `LineHighlighter::line_number` pads with an unsigned subtraction
  that underflows if a number is wider than the gutter.
- `src/widget.rs`, `rendered_position_in`: same substitution, for
  consistency. This one can't underflow (it's a saturating addition, not the
  unsigned subtraction that motivates the other two sites) but without it,
  the reported cursor column would be offset by the wrong gutter width
  whenever line numbers are overridden to wider values.

### Local-only: removed `[profile.bench]`

- `Cargo.toml`: dropped `[profile.bench] lto = "thin"`.

Not for upstream. Cargo ignores profiles declared by a non-root workspace
member and warns about them on every build; here the crate is a member of
`recon`'s workspace, so the setting was dead config producing noise. In
upstream's own tree it is the root package and the profile is valid.

## Rebasing onto a new upstream release

1. Copy the new version over this directory.
2. Re-apply the entries listed above (each names its file and anchor).
3. Run `cargo test` in the repo root; all tests must pass.

The patch is deliberately confined to per-line *presentation*. If it ever
needs to touch cursor movement, `screen_map.rs` or `wrap.rs`, stop: that is
the signal to reconsider a purpose-built viewer instead (see the spec).
