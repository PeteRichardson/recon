# Fork tui-textarea-2 for per-line styling — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Vendor `tui-textarea-2` as a local fork and add two public setters —
per-line styles and per-line number overrides — without changing any observable
behaviour of `recon`.

**Architecture:** The crate already styles individual lines internally
(`LineHighlighter::set_line_style`, used for the cursor line) and already
derives gutter numbers from the source row. Both are unreachable from outside.
The fork exposes them: `set_line_styles` feeds the existing per-line style hook
from user data, and `set_line_numbers` overrides the number passed to the
gutter. Nothing touches cursor movement, the screen map, or wrap logic. The
crate is vendored at `vendor/tui-textarea-2` and wired in with
`[patch.crates-io]`, so `recon`'s dependency line is unchanged.

**Tech Stack:** Rust 2021, ratatui 0.30.2, tui-textarea-2 0.12.1 (MIT),
crossterm 0.29, regex 1.

**Spec:** `docs/specs/2026-08-15-filter-based-viewing-design.md` (Phase 1)

## Global Constraints

- Rust edition 2021; the vendored crate declares `rust-version = 1.88.0`.
- ratatui pinned at `0.30.2`; the vendored crate must keep depending on the
  same `ratatui-core` / `ratatui-widgets` so only one copy is compiled.
- The vendored crate keeps `name = "tui-textarea-2"` and `version = "0.12.1"`
  verbatim — `[patch.crates-io]` matches on both.
- The `search` feature stays enabled for `recon`.
- Upstream is MIT licensed; the licence file (`LICENSE`, no extension) must be preserved unmodified.
- **Phase 1 changes no behaviour of `recon`.** All **82** existing tests
  (73 lib + 9 integration) must keep passing untouched at every commit. If a
  test needs editing, something has gone wrong — stop and re-read the spec.
- The patch stays limited to per-line *presentation*. Needing to touch cursor
  movement, `screen_map.rs`, or `wrap.rs` is the signal to stop and revisit
  the from-scratch viewer with the user.

## File Structure

| File | Responsibility |
|---|---|
| `vendor/tui-textarea-2/` | Vendored upstream 0.12.1 source, patched |
| `vendor/tui-textarea-2/src/textarea.rs` | New fields, setters, render hooks |
| `vendor/tui-textarea-2/src/widget.rs` | Gutter width accounting for overrides |
| `vendor/tui-textarea-2/tests/line_presentation.rs` | Fork's own tests (upstreamable) |
| `vendor/tui-textarea-2/PATCH.md` | Provenance, the diff, how to rebase |
| `Cargo.toml` | `[patch.crates-io]` entry |
| `src/widgets/fileview.rs` | Thin wrappers exposing both setters |
| `README.md` | Note that a patched dependency is vendored |

---

### Task 1: Vendor the crate behind an unchanged dependency line

Copies upstream verbatim and routes the build through it. No source edits, so
this task proves the plumbing alone: if `recon`'s tests still pass, the fork is
wired in correctly and every later task starts from a known-good base.

**Files:**
- Create: `vendor/tui-textarea-2/` (copied from the cargo registry cache)
- Create: `vendor/tui-textarea-2/PATCH.md`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: a buildable path crate named `tui-textarea-2` version `0.12.1`,
  reached by `recon` through `[patch.crates-io]`. Later tasks edit its source.

- [ ] **Step 1: Copy the exact published source out of the registry cache**

The registry copy is the precise 0.12.1 source and is stored read-only, so it
needs making writable. `.cargo-ok` is a cargo bookkeeping marker and must not
be committed.

```bash
mkdir -p vendor
cp -R "$(ls -d ~/.cargo/registry/src/*/tui-textarea-2-0.12.1)" vendor/tui-textarea-2
chmod -R u+w vendor/tui-textarea-2
rm -f vendor/tui-textarea-2/.cargo-ok
```

- [ ] **Step 2: Confirm the copy is intact**

```bash
grep -E '^(name|version) = ' vendor/tui-textarea-2/Cargo.toml | head -2
ls vendor/tui-textarea-2/LICENSE
```

Expected: `name = "tui-textarea-2"`, `version = "0.12.1"`, and the licence file
(`LICENSE`, no extension) present. If the name or version differs, `[patch.crates-io]` will not apply.

- [ ] **Step 3: Point the build at the vendored copy**

Append to `Cargo.toml`. The `[dependencies]` line stays exactly as it is —
`patch` redirects it without changing the version requirement or features.

```toml
[patch.crates-io]
tui-textarea-2 = { path = "vendor/tui-textarea-2" }
```

- [ ] **Step 4: Verify the build resolves to the vendored path**

```bash
cargo tree | grep tui-textarea
```

Expected: a line containing `tui-textarea-2 v0.12.1 (/Users/pete/projects/rust/recon/vendor/tui-textarea-2)`.
If it shows no path, the patch did not apply — recheck name and version.

- [ ] **Step 5: Verify no behaviour changed**

```bash
cargo test 2>&1 | grep -E "^test result"
```

Expected: 4 result lines totalling **82 passed, 0 failed**. Also confirm only
one copy of the crate and of ratatui-core is compiled:

```bash
cargo tree -d | grep -E "tui-textarea|ratatui-core" || echo "no duplicates"
```

Expected: `no duplicates`.

- [ ] **Step 6: Record provenance**

Create `vendor/tui-textarea-2/PATCH.md`:

```markdown
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

None yet — this commit is a verbatim copy, so that the wiring can be verified
independently of any patch.

## Rebasing onto a new upstream release

1. Copy the new version over this directory.
2. Re-apply the entries listed above (each names its file and anchor).
3. Run `cargo test` in the repo root; all tests must pass.

The patch is deliberately confined to per-line *presentation*. If it ever
needs to touch cursor movement, `screen_map.rs` or `wrap.rs`, stop: that is
the signal to reconsider a purpose-built viewer instead (see the spec).
```

- [ ] **Step 7: Confirm the vendored tree is not ignored**

`.gitignore` contains only `/target`, so `vendor/` is tracked. Verify the
files are actually staged rather than silently skipped:

```bash
git add vendor Cargo.toml Cargo.lock
git status --short | grep -c "^A.*vendor/tui-textarea-2/src"
```

Expected: a non-zero count (roughly 15 source files).

- [ ] **Step 8: Commit**

```bash
git commit -m "build: vendor tui-textarea-2 0.12.1 as a local fork

Verbatim copy, wired in with [patch.crates-io] so the dependency line
is unchanged. No source edits yet: this commit isolates the plumbing so
that the 82 existing tests verify the fork is correctly wired before any
patch is applied."
```

---

### Task 2: `set_line_styles` — per-line styling

Exposes the per-line style hook. This is what dimming and filter colours will
ride on.

**Files:**
- Create: `vendor/tui-textarea-2/tests/line_presentation.rs`
- Modify: `vendor/tui-textarea-2/src/textarea.rs` (struct fields ~line 201,
  `TextArea::new` ~line 316, `line_spans_segment` ~line 2046)
- Modify: `vendor/tui-textarea-2/PATCH.md`

**Interfaces:**
- Consumes: the vendored crate from Task 1.
- Produces, on `TextArea<'a>`:
  - `pub fn set_line_styles(&mut self, styles: Vec<Option<Style>>)`
  - `pub fn line_styles(&self) -> &[Option<Style>]`
  - `pub fn clear_line_styles(&mut self)`

- [ ] **Step 1: Write the failing tests**

Create `vendor/tui-textarea-2/tests/line_presentation.rs`. The cursor style and
cursor-line style are neutralised so that assertions test the new feature and
not the cursor decoration that lands on row 0 by default.

```rust
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use tui_textarea::{CursorRenderMode, TextArea};

fn plain(lines: &[&str]) -> TextArea<'static> {
    let mut textarea = TextArea::new(lines.iter().map(|s| s.to_string()).collect());
    // The cursor is drawn as a styled cell over column 0 of its line, which
    // would mask the styles under test. Hide it and neutralise the cursor
    // line so assertions isolate the feature.
    textarea.set_cursor_render_mode(CursorRenderMode::Hidden);
    textarea.set_cursor_line_style(Style::default());
    textarea
}

fn render(textarea: &TextArea<'_>, width: u16, height: u16) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    Widget::render(textarea, area, &mut buf);
    buf
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn line_styles_apply_to_the_named_line_only() {
    let mut textarea = plain(&["alpha", "beta", "gamma"]);
    textarea.set_line_styles(vec![None, Some(Style::default().fg(Color::Yellow)), None]);

    let buf = render(&textarea, 10, 3);

    assert_eq!(buf[(0, 1)].style().fg, Some(Color::Yellow), "beta not styled");
    assert_ne!(buf[(0, 0)].style().fg, Some(Color::Yellow), "alpha wrongly styled");
    assert_ne!(buf[(0, 2)].style().fg, Some(Color::Yellow), "gamma wrongly styled");
}

#[test]
fn lines_past_the_end_of_the_styles_are_unstyled() {
    let mut textarea = plain(&["alpha", "beta"]);
    // Deliberately shorter than the buffer: must not panic or misapply.
    textarea.set_line_styles(vec![Some(Style::default().fg(Color::Yellow))]);

    let buf = render(&textarea, 10, 2);

    assert_eq!(buf[(0, 0)].style().fg, Some(Color::Yellow));
    assert_ne!(buf[(0, 1)].style().fg, Some(Color::Yellow));
}

#[test]
fn line_styles_are_empty_by_default() {
    let textarea = plain(&["alpha"]);
    assert!(textarea.line_styles().is_empty());
}

#[test]
fn clear_line_styles_restores_the_default_look() {
    let mut textarea = plain(&["alpha"]);
    textarea.set_line_styles(vec![Some(Style::default().fg(Color::Yellow))]);
    textarea.clear_line_styles();

    let buf = render(&textarea, 10, 1);

    assert!(textarea.line_styles().is_empty());
    assert_ne!(buf[(0, 0)].style().fg, Some(Color::Yellow));
}

#[test]
fn the_cursor_line_style_still_wins() {
    let mut textarea = plain(&["alpha", "beta"]);
    textarea.set_cursor_line_style(Style::default().fg(Color::Green));
    textarea.set_line_styles(vec![Some(Style::default().fg(Color::Yellow)); 2]);

    let buf = render(&textarea, 10, 2);

    // The cursor sits on row 0, which must keep the cursor-line colour.
    assert_eq!(buf[(0, 0)].style().fg, Some(Color::Green), "cursor line lost its style");
    assert_eq!(buf[(0, 1)].style().fg, Some(Color::Yellow), "other line lost its style");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p tui-textarea-2 --test line_presentation
```

Expected: FAIL to compile — `no method named 'set_line_styles' found`. The
first run also builds the crate's dev-dependencies (termion, termwiz,
serde_json), which takes a minute; this is normal and only happens once.

- [ ] **Step 3: Add the field**

In `vendor/tui-textarea-2/src/textarea.rs`, in `pub struct TextArea<'a>`, add
immediately after the `line_number_style: Option<Style>,` line:

```rust
    line_styles: Vec<Option<Style>>,
```

- [ ] **Step 4: Initialise the field**

In `TextArea::new`, in the `let textarea = Self { ... }` literal, add
immediately after `line_number_style: None,`:

```rust
            line_styles: Vec::new(),
```

- [ ] **Step 5: Add the public setters**

In `vendor/tui-textarea-2/src/textarea.rs`, directly after the existing
`set_line_number_style` / `line_number_style` pair:

```rust
    /// Set a style for each line, indexed by line number.
    ///
    /// Lines whose entry is `None`, and lines past the end of `styles`, are
    /// left at the textarea's own style. This is intended for showing the
    /// state of a line — dimming lines that do not match a filter, or
    /// colouring those that do — rather than for syntax highlighting.
    ///
    /// The cursor line style still takes precedence over the entry for the
    /// line the cursor is on.
    ///
    /// ```
    /// use ratatui::style::{Color, Style};
    /// use tui_textarea::TextArea;
    ///
    /// let mut textarea = TextArea::new(vec!["a".to_string(), "b".to_string()]);
    /// textarea.set_line_styles(vec![None, Some(Style::default().fg(Color::DarkGray))]);
    /// ```
    pub fn set_line_styles(&mut self, styles: Vec<Option<Style>>) {
        self.line_styles = styles;
    }

    /// The per-line styles set by [`TextArea::set_line_styles`]. Empty when
    /// none have been set.
    pub fn line_styles(&self) -> &[Option<Style>] {
        &self.line_styles
    }

    /// Remove all per-line styles, returning every line to the textarea's own
    /// style.
    pub fn clear_line_styles(&mut self) {
        self.line_styles.clear();
    }
```

- [ ] **Step 6: Apply the style while rendering**

In `line_spans_segment`, insert this **immediately before** the
`if wrapped.row == self.cursor.0 {` block. Order matters: the cursor-line block
calls `hl.set_line_style` too, and must be able to override this one.

```rust
        if let Some(Some(style)) = self.line_styles.get(wrapped.row) {
            hl.set_line_style(*style);
        }
```

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cargo test -p tui-textarea-2 --test line_presentation
```

Expected: PASS, 5 tests.

- [ ] **Step 8: Verify `recon` is unaffected**

```bash
cargo test 2>&1 | grep -E "^test result"
```

Expected: still **82 passed, 0 failed**. Nothing in `recon` calls the new API
yet, so any change here means the render hook altered default behaviour.

- [ ] **Step 9: Record the change**

Replace the "Changes from upstream" section of
`vendor/tui-textarea-2/PATCH.md` with:

```markdown
## Changes from upstream

### 1. Per-line styles

- `src/textarea.rs`, `struct TextArea`: added field `line_styles: Vec<Option<Style>>`.
- `src/textarea.rs`, `TextArea::new`: initialise it to `Vec::new()`.
- `src/textarea.rs`: added `set_line_styles`, `line_styles`, `clear_line_styles`
  after `line_number_style`.
- `src/textarea.rs`, `line_spans_segment`: apply the entry for `wrapped.row`
  immediately *before* the cursor-line block, so the cursor line still wins.
- `tests/line_presentation.rs`: new.
```

- [ ] **Step 10: Commit**

```bash
git add vendor/tui-textarea-2 && git commit -m "feat(vendor): add per-line styles to the textarea fork

Exposes LineHighlighter::set_line_style, which the crate already uses
internally for the cursor line, as a public per-line API. This is what
filter dimming and filter colours will be drawn with.

Applied before the cursor-line block so the cursor line keeps priority."
```

---

### Task 3: `set_line_numbers` — gutter number overrides

Lets the gutter show source line numbers when the buffer holds only a filtered
subset, instead of renumbering 1..N.

**Files:**
- Modify: `vendor/tui-textarea-2/tests/line_presentation.rs`
- Modify: `vendor/tui-textarea-2/src/textarea.rs`
- Modify: `vendor/tui-textarea-2/src/widget.rs` (`text_widget` ~line 101,
  `scroll_top_col` ~line 131)
- Modify: `vendor/tui-textarea-2/PATCH.md`

**Interfaces:**
- Consumes: the vendored crate from Tasks 1–2.
- Produces, on `TextArea<'a>`:
  - `pub fn set_line_numbers(&mut self, numbers: Vec<usize>)` — 0-based source
    indices; the gutter renders each `+ 1`
  - `pub fn line_numbers(&self) -> &[usize]`
  - `pub fn clear_line_numbers(&mut self)`
  - `pub(crate) fn display_row(&self, row: usize) -> usize`
  - `pub(crate) fn widest_display_row(&self) -> usize`

- [ ] **Step 1: Write the failing tests**

Append to `vendor/tui-textarea-2/tests/line_presentation.rs`:

```rust
#[test]
fn line_numbers_can_be_overridden() {
    let mut textarea = plain(&["beta", "delta"]);
    textarea.set_line_number_style(Style::default());
    // 0-based source rows 1 and 3 render as 2 and 4.
    textarea.set_line_numbers(vec![1, 3]);

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_start().starts_with("2 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("4 "), "got {:?}", row_text(&buf, 1));
}

/// The gutter pads with `lnum_len - num_digits(row + 1)`, an unsigned
/// subtraction. An override wider than the buffer's own line count would
/// underflow and panic unless the gutter is sized from the overrides.
#[test]
fn the_gutter_widens_for_overridden_numbers() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_line_numbers(vec![9997, 9998]);

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_start().starts_with("9998 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("9999 "), "got {:?}", row_text(&buf, 1));
}

#[test]
fn without_overrides_numbering_is_unchanged() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_start().starts_with("1 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("2 "), "got {:?}", row_text(&buf, 1));
}

#[test]
fn rows_without_an_override_fall_back_to_their_position() {
    let mut textarea = plain(&["a", "b", "c"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_line_numbers(vec![41]);

    let buf = render(&textarea, 20, 3);

    assert!(row_text(&buf, 0).trim_start().starts_with("42 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("2 "), "got {:?}", row_text(&buf, 1));
}

#[test]
fn clear_line_numbers_restores_natural_numbering() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_line_numbers(vec![100, 101]);
    textarea.clear_line_numbers();

    let buf = render(&textarea, 20, 2);

    assert!(textarea.line_numbers().is_empty());
    assert!(row_text(&buf, 0).trim_start().starts_with("1 "), "got {:?}", row_text(&buf, 0));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p tui-textarea-2 --test line_presentation
```

Expected: FAIL to compile — `no method named 'set_line_numbers' found`.

- [ ] **Step 3: Add the field and initialiser**

In `struct TextArea<'a>`, immediately after `line_styles: Vec<Option<Style>>,`:

```rust
    line_numbers: Vec<usize>,
```

In `TextArea::new`, immediately after `line_styles: Vec::new(),`:

```rust
            line_numbers: Vec::new(),
```

- [ ] **Step 4: Add the public setters and internal helpers**

In `vendor/tui-textarea-2/src/textarea.rs`, directly after the
`clear_line_styles` method added in Task 2:

```rust
    /// Override the number shown in the gutter for each line.
    ///
    /// Values are 0-based source indices, matching the crate's internal row
    /// numbering; the gutter renders each one `+ 1`. Rows past the end of
    /// `numbers` fall back to their position in the buffer.
    ///
    /// This exists so a buffer holding a filtered subset of a file can show
    /// the original line numbers — 2, 4, 9 — rather than renumbering 1, 2, 3.
    /// The gutter widens automatically to fit the largest number given.
    ///
    /// ```
    /// use tui_textarea::TextArea;
    ///
    /// let mut textarea = TextArea::new(vec!["second".to_string()]);
    /// textarea.set_line_numbers(vec![1]); // renders as line 2
    /// ```
    pub fn set_line_numbers(&mut self, numbers: Vec<usize>) {
        self.line_numbers = numbers;
    }

    /// The overrides set by [`TextArea::set_line_numbers`]. Empty when none
    /// have been set.
    pub fn line_numbers(&self) -> &[usize] {
        &self.line_numbers
    }

    /// Remove all overrides, returning the gutter to natural numbering.
    pub fn clear_line_numbers(&mut self) {
        self.line_numbers.clear();
    }

    /// The 0-based number to show in the gutter for `row`.
    pub(crate) fn display_row(&self, row: usize) -> usize {
        self.line_numbers.get(row).copied().unwrap_or(row)
    }

    /// The largest 0-based number the gutter will have to show.
    ///
    /// The gutter computes its padding as `lnum_len - num_digits(row + 1)` on
    /// unsigned integers, so sizing the gutter from `lines.len()` alone would
    /// underflow and panic as soon as an override is wider than the buffer.
    pub(crate) fn widest_display_row(&self) -> usize {
        let natural = self.lines.len().saturating_sub(1);
        self.line_numbers
            .iter()
            .copied()
            .max()
            .unwrap_or(natural)
            .max(natural)
    }
```

- [ ] **Step 5: Use the override when rendering the gutter**

In `line_spans_segment`, replace:

```rust
                hl.line_number(wrapped.row, lnum_len, style);
```

with:

```rust
                hl.line_number(self.display_row(wrapped.row), lnum_len, style);
```

- [ ] **Step 6: Size the gutter from the overrides**

In `vendor/tui-textarea-2/src/widget.rs`, in `text_widget`, replace:

```rust
        let lnum_len = num_digits(self.lines().len());
```

with:

```rust
        let lnum_len = num_digits(self.widest_display_row() + 1);
```

And in `scroll_top_col`, so horizontal scrolling accounts for the same width,
replace:

```rust
            let lnum = num_digits(self.lines().len()) as u16 + 2; // `+ 2` for margins
```

with:

```rust
            let lnum = num_digits(self.widest_display_row() + 1) as u16 + 2; // `+ 2` for margins
```

Both are equivalent to the original when no overrides are set: for a buffer of
`n` lines, `widest_display_row() + 1 == n`.

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cargo test -p tui-textarea-2 --test line_presentation
```

Expected: PASS, 10 tests.

- [ ] **Step 8: Run the vendored crate's full suite**

The gutter width change touches shared rendering, so the crate's own tests are
the safety net:

```bash
cargo test -p tui-textarea-2
```

Expected: all pass. A failure here means the width calculation changed
behaviour for the un-overridden case — recheck Step 6.

- [ ] **Step 9: Verify `recon` is unaffected**

```bash
cargo test 2>&1 | grep -E "^test result"
```

Expected: still **82 passed, 0 failed**.

- [ ] **Step 10: Record the change**

Append to the "Changes from upstream" section of `vendor/tui-textarea-2/PATCH.md`:

```markdown
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
```

- [ ] **Step 11: Commit**

```bash
git add vendor/tui-textarea-2 && git commit -m "feat(vendor): allow overriding gutter line numbers

A buffer holding a filtered subset of a file needs to show the original
line numbers rather than renumbering 1..N.

The gutter is now sized from the widest number actually shown. That is
required rather than cosmetic: line_number pads with an unsigned
subtraction that underflows when a number exceeds the gutter width."
```

---

### Task 4: Expose both setters through `FileView`

Gives `recon` access to the new API and proves it works end-to-end, so Phase 2
starts from something demonstrated rather than assumed.

**Files:**
- Modify: `src/widgets/fileview.rs`

**Interfaces:**
- Consumes: `TextArea::set_line_styles`, `TextArea::set_line_numbers` from
  Tasks 2–3.
- Produces, on `FileView<'a>`:
  - `pub fn set_line_styles(&mut self, styles: Vec<Option<Style>>)`
  - `pub fn set_line_numbers(&mut self, numbers: Vec<usize>)`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `src/widgets/fileview.rs`. `view_of`,
`rendered` and `send` already exist there from earlier work.

```rust
    /// Whether any cell in row `y` carries `colour` as its foreground.
    fn row_has_fg(buf: &Buffer, y: u16, colour: Color) -> bool {
        (0..buf.area.width).any(|x| buf[(x, y)].style().fg == Some(colour))
    }

    /// The row containing `needle`. The view draws a bordered block, so text
    /// does not begin at row 0 and row indices cannot be assumed.
    fn row_of(buf: &Buffer, needle: &str) -> u16 {
        (0..buf.area.height)
            .find(|&y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .contains(needle)
            })
            .unwrap_or_else(|| panic!("no row containing {needle:?}"))
    }

    #[test]
    fn line_styles_reach_the_rendered_view() {
        let mut view = view_of("line_styles.txt", "alpha\nbeta\n");
        view.set_line_styles(vec![None, Some(Style::default().fg(Color::Yellow))]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        let beta = row_of(&buf, "beta");
        assert!(row_has_fg(&buf, beta, Color::Yellow), "beta not styled");
        assert!(!row_has_fg(&buf, alpha, Color::Yellow), "alpha wrongly styled");
    }

    #[test]
    fn overridden_line_numbers_reach_the_gutter() {
        let mut view = view_of("line_numbers.txt", "beta\ndelta\n");
        view.set_line_numbers(vec![1, 3]);

        let text = rendered(&mut view);

        assert!(text.contains("2 beta"), "gutter not overridden:\n{text}");
        assert!(text.contains("4 delta"), "gutter not overridden:\n{text}");
    }

    /// Loading a file rebuilds the TextArea, which drops both. Phase 2 must
    /// re-apply them after every load; this pins the behaviour so that is not
    /// discovered by surprise.
    #[test]
    fn loading_a_file_clears_line_styles_and_numbers() {
        let path = fixture("reload.txt", "alpha\nbeta\n");
        let mut view = view_of("reload_start.txt", "x\n");
        view.set_line_styles(vec![Some(Style::default().fg(Color::Yellow))]);
        view.set_line_numbers(vec![41]);

        view.load(&path);

        assert!(view.textarea.line_styles().is_empty());
        assert!(view.textarea.line_numbers().is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test --lib line_styles_reach_the_rendered_view overridden_line_numbers_reach_the_gutter
```

Expected: FAIL to compile — `no method named 'set_line_styles' found for struct FileView`.

- [ ] **Step 3: Add the wrappers**

In `src/widgets/fileview.rs`, in `impl FileView<'_>`, after `preview`:

```rust
    /// Style individual lines, indexed by line number.
    ///
    /// Filtering uses this to dim lines that match no filter and colour those
    /// that do. Rebuilding the textarea — which `load` and `preview` both do —
    /// clears these, so they must be re-applied after either.
    pub fn set_line_styles(&mut self, styles: Vec<Option<Style>>) {
        self.textarea.set_line_styles(styles);
    }

    /// Show these 0-based source line numbers in the gutter instead of
    /// numbering the buffer 1..N.
    ///
    /// Used when the buffer holds only the lines matching a filter, so the
    /// gutter still reads as positions in the original file. Cleared by
    /// `load` and `preview`, as above.
    pub fn set_line_numbers(&mut self, numbers: Vec<usize>) {
        self.textarea.set_line_numbers(numbers);
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test --lib
```

Expected: PASS — 76 lib tests (73 existing + 3 new), 0 failed.

- [ ] **Step 5: Run everything**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo build 2>&1 | grep -E "warning|error" || echo "build clean"
```

Expected: **85 passed, 0 failed** (76 lib + 9 integration), and `build clean`.

- [ ] **Step 6: Commit**

```bash
git add src/widgets/fileview.rs && git commit -m "feat(fileview): expose per-line styles and gutter overrides

Thin wrappers over the forked textarea, with tests proving both reach
the rendered buffer. Also pins the fact that load/preview rebuild the
TextArea and therefore clear both, which filtering will have to
re-apply."
```

---

### Task 5: Prepare the upstream patch and document the fork

The fork is only acceptable if it is easy to drop. This produces the artefact
needed to offer it upstream and tells a reader of the repo why `vendor/` exists.

**Files:**
- Create: `vendor/tui-textarea-2/upstream.patch`
- Modify: `README.md`

**Interfaces:**
- Consumes: the completed fork from Tasks 1–4.
- Produces: no code. `upstream.patch` is the diff to offer upstream.

- [ ] **Step 1: Generate the diff against pristine upstream**

Compares the patched tree with a fresh copy of the published source, so the
diff contains exactly the fork's changes:

```bash
rm -rf /tmp/tui-pristine
cp -R "$(ls -d ~/.cargo/registry/src/*/tui-textarea-2-0.12.1)" /tmp/tui-pristine
chmod -R u+w /tmp/tui-pristine
rm -f /tmp/tui-pristine/.cargo-ok
diff -ruN \
  --exclude=PATCH.md --exclude=upstream.patch --exclude=target \
  /tmp/tui-pristine vendor/tui-textarea-2 \
  > vendor/tui-textarea-2/upstream.patch || true
```

- [ ] **Step 2: Check the diff is confined to the intended files**

```bash
grep "^diff " vendor/tui-textarea-2/upstream.patch
```

Expected: exactly three files — `src/textarea.rs`, `src/widget.rs`, and
`tests/line_presentation.rs`. **Anything touching `screen_map.rs`, `wrap.rs`
or `cursor.rs` violates the spec's constraint** — stop and raise it rather
than committing.

- [ ] **Step 3: Note the fork in the README**

Add this section to `README.md`:

```markdown
## Vendored dependency

`vendor/tui-textarea-2` is a patched copy of
[tui-textarea-2](https://github.com/srothgan/tui-textarea) 0.12.1, wired in
via `[patch.crates-io]`. The patch adds two public setters — per-line styles
and gutter number overrides — which the file view needs in order to dim lines
that do not match a filter and to show original line numbers while filtered.

See `vendor/tui-textarea-2/PATCH.md` for the exact changes and how to rebase
onto a new upstream release, and `upstream.patch` for the diff as offered
upstream. If the patch is accepted, this directory and the `[patch.crates-io]`
entry can both be deleted.
```

- [ ] **Step 4: Verify nothing broke**

```bash
cargo test 2>&1 | grep -E "^test result"
```

Expected: **85 passed, 0 failed**.

- [ ] **Step 5: Commit**

```bash
git add vendor/tui-textarea-2/upstream.patch README.md
git commit -m "docs: record the vendored fork and prepare the upstream patch

upstream.patch is the diff against pristine 0.12.1, confined to
textarea.rs, widget.rs and the new test file. If upstream accepts it,
vendor/ and the [patch.crates-io] entry can both be removed."
```

- [ ] **Step 6: Open the upstream pull request (manual, outside the repo)**

Fork <https://github.com/srothgan/tui-textarea>, apply `upstream.patch` on a
branch, and open a PR describing the use case: read-only viewers that need to
show per-line state (filter matches, diff status, log severity) currently have
no way to style a line, though the crate already does so internally for the
cursor line. Record the PR URL at the top of `PATCH.md` when it exists.

---

## Phase 1 completion criteria

- `cargo test` reports **85 passed, 0 failed**; the 82 pre-existing tests are
  unmodified.
- `cargo build` and `cargo clippy --lib` produce no new warnings.
- `cargo tree -d` shows no duplicate `tui-textarea-2` or `ratatui-core`.
- `upstream.patch` touches only `src/textarea.rs`, `src/widget.rs` and
  `tests/line_presentation.rs`.
- No observable behaviour of `recon` has changed.

Phase 2 (`Document`, filters, Ctrl+H) begins only once all of the above hold.
