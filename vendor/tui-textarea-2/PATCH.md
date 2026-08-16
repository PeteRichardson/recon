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
- `src/widget.rs`, `text_widget`: size the gutter from
  `widest_display_row() + 1` rather than `lines().len()`. Required, not
  cosmetic: `LineHighlighter::line_number` pads with an unsigned subtraction
  that underflows if a number is wider than the gutter.
- `src/widget.rs`, `scroll_top_col` and `rendered_position_in`: same
  substitution, for consistency and a correct offset. Neither can underflow
  — both do `u16`/saturating arithmetic rather than the unsigned subtraction
  that motivates `text_widget` — but without the substitution, the reported
  scroll column and cursor column would be offset by the wrong gutter width
  whenever line numbers are overridden to wider values.

### Limitation: gutter overrides need `WrapMode::None`

`screen_map.rs` (`screen_map_load`, ~line 89) and `measure_content_rows` in
`textarea.rs` (~line 2950) both still size the gutter reservation from
`self.lines.len()` rather than the widest *displayed* number, and both feed
wrap layout. With `wrap_mode != WrapMode::None` and overrides wider than the
buffer's natural numbering, they under-reserve and content clips on the
right.

This is not fixed, and should not be: `screen_map.rs` is the design's
declared tripwire below — touching it is the signal to stop forking and
build a purpose-built viewer instead. `recon` never calls `set_wrap_mode`,
so the default `WrapMode::None` short-circuits both sites and nothing can
hit this today. Gutter overrides are therefore supported only under
`WrapMode::None`.

### Local-only: removed `[profile.bench]`

- `Cargo.toml`: dropped `[profile.bench] lto = "thin"`.

Not for upstream. Cargo ignores profiles declared by a non-root workspace
member and warns about them on every build; here the crate is a member of
`recon`'s workspace, so the setting was dead config producing noise. In
upstream's own tree it is the root package and the profile is valid.

## Rebasing onto a new upstream release

1. Copy the new version over this directory.
2. Re-apply the entries listed above (each names its file and anchor).
3. Run `cargo test --workspace` in the repo root; all tests must pass. Plain
   `cargo test` is not enough: the workspace's default members are `recon`
   alone, so it runs `recon`'s tests and skips this crate entirely, including
   `tests/line_presentation.rs`, which pins this fork's whole contract.

The patch is deliberately confined to per-line *presentation*. If it ever
needs to touch cursor movement, `screen_map.rs` or `wrap.rs`, stop: that is
the signal to reconsider a purpose-built viewer instead (see the spec).

## Inert publish artefacts

The vendored tree also carries `Cargo.lock`, `Cargo.toml.orig` and
`.cargo_vcs_info.json`, left over from the published package. They are
inert: workspace members use the workspace's own `Cargo.lock`, so the
vendored one is never consulted. `Cargo.toml.orig` is **not** a stale copy
of `Cargo.toml` to ignore — it is the pre-publish manifest and differs
materially (no `[[test]]` entries, no `autotests` key, and it still has
`[profile.bench]`). Patching the wrong one gives confusing results; the
entries above all target `Cargo.toml`.
