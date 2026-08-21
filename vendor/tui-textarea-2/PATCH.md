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

### 3. Viewport reporting

- `src/textarea.rs`, beside `cursor()`: added `scroll_top`, reporting the
  existing `pub(crate)` `Viewport::scroll_top()` value. `scroll` and `cursor`
  were already public; only the viewport's own top-left position had no way
  out of the crate. Used by `recon` to hold the cursor's screen row steady
  across a buffer rebuild (`set_lines` resets the viewport), rather than
  letting it re-anchor to wherever the reset viewport happens to land it.

### 4. Minimum gutter width

- `src/textarea.rs`, `struct TextArea`: added field `min_line_number_width: u8`.
- `src/textarea.rs`, `TextArea::new`: initialise it to `0`.
- `src/textarea.rs`: added `set_min_line_number_width`, `min_line_number_width`,
  and the crate-internal `line_number_width`, before `widest_display_row`.
- `src/widget.rs`, `text_widget`, `scroll_top_col` and `rendered_position_in`:
  all three now call `line_number_width()` instead of computing
  `num_digits(widest_display_row() + 1)` for themselves. The `use
  crate::util::num_digits` import went with them.
- `tests/line_presentation.rs`: four new tests.

`recon` shows a bounded preview of a large file before loading the rest. Sized
from the preview alone, the gutter fits ~500 lines and then widens the instant
the full file arrives, shifting every line of text sideways on a pane the user
is reading. `recon` estimates the file's line count from the preview's own
bytes-per-line and reserves the width up front.

**Routing all three `widget.rs` sites through one accessor is the point, not a
tidy-up.** The width feeds the rendered text, the horizontal scroll offset and
the cursor's screen column; sizing only the text would leave the cursor drawn
`min - natural` columns left of the character it is on. The three had already
drifted apart once, in change 2 above, for the same reason.

The minimum only ever raises the width, never lowers it, so a stale
reservation cannot truncate a gutter that has outgrown it — which is what
makes a wrong estimate cost at most the single redraw it was trying to avoid.

Same `WrapMode::None` caveat as change 2: `screen_map.rs` and
`measure_content_rows` still reserve gutter space from `self.lines.len()`, so
a minimum wider than the buffer's natural numbering would under-reserve under
a wrapping mode. `recon` never calls `set_wrap_mode`. Not fixed, for the
reason given above — that file is the declared tripwire.

### 5. Cursor positioning without `u16` truncation

- `src/textarea.rs`: added `set_cursor_position` after `set_lines`.
- `tests/cursor.rs`: two tests appended.

`recon`'s `n`/`N` land the cursor on an arbitrary source line.
`CursorMove::Jump` takes `u16` and widens with `as usize` in its handler, so
a caller past 65,535 lines truncates. `set_lines` clamps in `usize` but
replaces the buffer, clears history and resets the viewport, which a plain
cursor move must not do. This exposes the existing private
`clamp_cursor_to_buffer` as a cursor-only operation.

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

The patch is deliberately confined to per-line *presentation*, viewport
*reporting*, and — as of change 5 — positioning the cursor through the
crate's own existing clamp. It does not touch how movement itself works. If
it ever needs to change `cursor.rs`'s movement logic, `screen_map.rs` or
`wrap.rs`, stop: that is the signal to reconsider a purpose-built viewer
instead (see the spec). `set_cursor_position` (change 5) stays on the
allowed side of that line because it only calls the crate's existing private
`clamp_cursor_to_buffer` and touches nothing outside `textarea.rs`; writing
a new cursor-movement algorithm, or reaching into `screen_map.rs` or
`wrap.rs`, would not.

## This file is the record

There is no `upstream.patch` in this directory. There used to be — a
`diff -ruN` against pristine 0.12.1 — and it was deleted, because it could
only stay correct if every change above remembered to regenerate it, and that
was missed the first time it mattered (entry 4). It was never submittable
as-is either: see the README's "Vendored dependency" section.

So this file is the sole record of what the fork changes, and the entries
above have to carry their own weight — each one names its file and its
anchor, because that is what a rebase and any future upstream submission
actually work from. **A new change here is not done until it has an entry.**

An equivalent diff is one command away whenever it is wanted; the README
gives it. Two older plan documents under `docs/plans/` still describe
regenerating the file as a step. They are historical records of work already
done and are marked superseded at those points.

## Inert publish artefacts

The vendored tree also carries `Cargo.lock`, `Cargo.toml.orig` and
`.cargo_vcs_info.json`, left over from the published package. They are
inert: workspace members use the workspace's own `Cargo.lock`, so the
vendored one is never consulted. `Cargo.toml.orig` is **not** a stale copy
of `Cargo.toml` to ignore — it is the pre-publish manifest and differs
materially (no `[[test]]` entries, no `autotests` key, and it still has
`[profile.bench]`). Patching the wrong one gives confusing results; the
entries above all target `Cargo.toml`.
