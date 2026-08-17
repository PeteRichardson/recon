# Phase 2a — include filters and dimming — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define include filters at a prompt; lines matching one render in that
filter's colour, lines matching none render dimmed. Nothing is ever hidden.

**Architecture:** A `FilterSet` on `App` outlives any one file, because a filter
set describes a log format rather than a document. `Document` owns the loaded
lines and a cached per-line `Verdict`, recomputed only when the lines or the
filters change. `App` turns verdicts into a `Vec<Option<Style>>` and hands it to
`FileView::set_line_styles` — the API Phase 1 added to the vendored fork. The
buffer is never rebuilt, so no line is hidden and no line number is overridden;
that is Phase 2b's job.

**Tech Stack:** Rust 2021, ratatui 0.30.2, vendored tui-textarea-2 0.12.1,
crossterm 0.29, regex 1.

**Spec:** `docs/specs/2026-08-15-filter-based-viewing-design.md` (Phase 2a)

## Global Constraints

- **The buffer is never rebuilt in this phase.** Only `set_line_styles` is used.
  Reaching for `set_line_numbers`, a `visible` mapping, or anything that removes
  a line from the textarea means the work has crossed into Phase 2b — stop.
- **Excluding filters are out of scope.** `Sense` is modelled so 2b can add them
  without reshaping anything, but only `Include` is constructed or evaluated
  here. An exclude filter cannot be created from the prompt in this phase.
- Filters are **regular expressions**, matching how search already works, so
  `^foo` anchors. An invalid pattern is reported, never silently ignored.
- Filters **persist across file loads**. `load` and `preview` rebuild the
  `TextArea` and clear its line styles, so styles must be re-applied after every
  load — this is already pinned by `loading_a_file_clears_line_styles_and_numbers`.
- **The cursor line must not escape dimming.** `render` sets the cursor-line
  style to that line's own verdict style patched with the focus decoration.
- 85 tests pass today; all must still pass. Any pre-existing test that needs
  editing is a signal something is wrong — stop and report.
- **Test output must be pristine.** Verify with `cargo test 2>&1 | grep -ci "^warning"`
  printing `0`. Never verify with `grep -E "^test result"`, which filters
  warnings out by construction and has hidden a real warning on this project before.
- Tests must not depend on the live repo-root directory listing. Use fixture
  directories under `target/`. This has broken the suite three times.
- TDD throughout: failing test first, observe it fail, then implement.

## File Structure

| File | Responsibility |
|---|---|
| `src/filter.rs` (new) | `Filter`, `Sense`, `FilterSet`, `Verdict` — matching and evaluation, no UI |
| `src/document.rs` (new) | `Document` — lines plus the cached verdict vector |
| `src/lib.rs` | Owns the `FilterSet`, the `f` prompt, `!`, and pushing styles into the view |
| `src/widgets/fileview.rs` | Cursor-line style derived from the verdict style |

`filter.rs` and `document.rs` are new top-level modules rather than additions to
`widgets/`, because neither draws anything — keeping them widget-free is what
lets them be tested without a `Buffer`.

---

### Task 1: `Filter` and `FilterSet`

The matching model, with no rendering and no `App` involvement, so its evaluation
order can be tested exhaustively in isolation.

**Files:**
- Create: `src/filter.rs`
- Modify: `src/lib.rs` (add `pub mod filter;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub struct Filter { pub pattern: Regex, pub sense: Sense, pub enabled: bool, pub style: Style }`
  - `pub enum Sense { Include, Exclude }`
  - `pub enum Verdict { Included(usize), Unmatched, Excluded }` — the `usize` is
    the index of the matching filter in the set, used to pick its colour
  - `pub struct FilterSet { filters: Vec<Filter> }` with
    `new()`, `is_empty()`, `len()`, `add(pattern: &str, style: Style) -> Result<(), regex::Error>`,
    `verdict(line: &str) -> Verdict`, `set_all_enabled(bool)`, `any_enabled() -> bool`,
    `filters() -> &[Filter]`, `next_style() -> Style`,
    `style_for(verdict: Verdict) -> Option<Style>` — the colour for a matched
    line, `DIM` for an unmatched one while any filter is enabled, `None` otherwise

- [ ] **Step 1: Write the failing tests**

Create `src/filter.rs` with only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn yellow() -> Style {
        Style::default().fg(Color::Yellow)
    }

    fn set_with(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in patterns {
            set.add(pattern, yellow()).expect("valid pattern");
        }
        set
    }

    /// With no filters at all, nothing is dimmed — a plain file reads normally.
    #[test]
    fn an_empty_set_leaves_every_line_unmatched() {
        let set = FilterSet::new();

        assert_eq!(set.verdict("anything"), Verdict::Unmatched);
        assert!(set.is_empty());
    }

    #[test]
    fn a_matching_line_is_included_with_its_filter_index() {
        let set = set_with(&["foo", "bar"]);

        assert_eq!(set.verdict("a bar line"), Verdict::Included(1));
    }

    #[test]
    fn a_non_matching_line_is_unmatched() {
        let set = set_with(&["foo"]);

        assert_eq!(set.verdict("nothing here"), Verdict::Unmatched);
    }

    /// Order in the set decides the colour, so the first match wins.
    #[test]
    fn the_first_matching_filter_wins() {
        let set = set_with(&["foo", "foo.*bar"]);

        assert_eq!(set.verdict("foo and bar"), Verdict::Included(0));
    }

    #[test]
    fn patterns_are_regular_expressions() {
        let set = set_with(&[r"^\d+ms$"]);

        assert_eq!(set.verdict("250ms"), Verdict::Included(0));
        assert_eq!(set.verdict("took 250ms"), Verdict::Unmatched);
    }

    #[test]
    fn an_invalid_pattern_is_reported() {
        let mut set = FilterSet::new();

        assert!(set.add("[", yellow()).is_err());
        assert!(set.is_empty(), "a rejected pattern must not be added");
    }

    #[test]
    fn a_disabled_filter_does_not_match() {
        let mut set = set_with(&["foo"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
        assert!(!set.any_enabled());
    }

    /// `!` disables everything and restores exactly what was enabled before.
    #[test]
    fn disabling_and_restoring_round_trips() {
        let mut set = set_with(&["foo"]);
        assert!(set.any_enabled());

        set.set_all_enabled(false);
        set.set_all_enabled(true);

        assert_eq!(set.verdict("foo"), Verdict::Included(0));
    }

    /// A set whose filters are all disabled behaves like an empty one: an
    /// undimmed file, not a fully dimmed one.
    #[test]
    fn a_fully_disabled_set_leaves_lines_unmatched() {
        let mut set = set_with(&["foo"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("bar"), Verdict::Unmatched);
    }

    #[test]
    fn successive_filters_get_distinct_colours() {
        let mut set = FilterSet::new();
        let first = set.next_style();
        set.add("a", first).expect("valid");
        let second = set.next_style();

        assert_ne!(first, second, "two filters would be indistinguishable");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filter::`
Expected: FAIL to compile — `cannot find type FilterSet in this scope`.

- [ ] **Step 3: Implement the module**

Put this above the test module in `src/filter.rs`:

```rust
//! Filters decide how each line of the viewed file is presented.
//!
//! A filter set describes a *log format* rather than a document, so it outlives
//! any one file. Matching is by regular expression, the same as search, so
//! `^foo` anchors to the start of a line.

use ratatui::style::{Color, Modifier, Style};
use regex::Regex;

/// Colours assigned to successive filters, so two filters are never
/// indistinguishable. Wraps once exhausted.
const PALETTE: [Color; 6] = [
    Color::Yellow,
    Color::Cyan,
    Color::Green,
    Color::Magenta,
    Color::Blue,
    Color::Red,
];

/// Whether a filter selects lines or removes them.
///
/// Only `Include` is constructed in this phase; `Exclude` exists so that adding
/// it later does not reshape the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sense {
    Include,
    Exclude,
}

/// What the filter set decided about one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Matched an including filter; carries its index, for colouring.
    Included(usize),
    /// Matched no including filter.
    Unmatched,
    /// Removed by an excluding filter. Never produced in this phase.
    Excluded,
}

#[derive(Debug)]
pub struct Filter {
    pub pattern: Regex,
    pub sense: Sense,
    pub enabled: bool,
    pub style: Style,
}

#[derive(Debug, Default)]
pub struct FilterSet {
    filters: Vec<Filter>,
}

impl FilterSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    pub fn len(&self) -> usize {
        self.filters.len()
    }

    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    /// The colour the next filter added should take.
    pub fn next_style(&self) -> Style {
        Style::default().fg(PALETTE[self.filters.len() % PALETTE.len()])
    }

    /// Add an including filter. A pattern that will not compile is rejected and
    /// the set left untouched.
    pub fn add(&mut self, pattern: &str, style: Style) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.filters.push(Filter {
            pattern,
            sense: Sense::Include,
            enabled: true,
            style,
        });
        Ok(())
    }

    /// Enable or disable every filter at once, for the `!` toggle.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            filter.enabled = enabled;
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.filters.iter().any(|filter| filter.enabled)
    }

    /// Decide how `line` should be presented.
    ///
    /// A set with no enabled including filters leaves every line `Unmatched`,
    /// so an empty or fully disabled set renders an ordinary, undimmed file
    /// rather than a wholly dimmed one. The first matching filter wins, which
    /// is what makes the set's order meaningful.
    pub fn verdict(&self, line: &str) -> Verdict {
        self.filters
            .iter()
            .enumerate()
            .find(|(_, filter)| {
                filter.enabled
                    && filter.sense == Sense::Include
                    && filter.pattern.is_match(line)
            })
            .map_or(Verdict::Unmatched, |(index, _)| Verdict::Included(index))
    }

    /// The style to render a line with, or `None` to leave it alone.
    ///
    /// `Unmatched` dims only when some including filter is actually active —
    /// otherwise every line of an unfiltered file would be dimmed.
    pub fn style_for(&self, verdict: Verdict) -> Option<Style> {
        match verdict {
            Verdict::Included(index) => self.filters.get(index).map(|f| f.style),
            Verdict::Unmatched if self.any_enabled() => {
                Some(Style::default().add_modifier(Modifier::DIM))
            }
            Verdict::Unmatched | Verdict::Excluded => None,
        }
    }
}
```

Register the module by adding `pub mod filter;` near the top of `src/lib.rs`,
beside `mod widgets;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib filter::`
Expected: PASS, 10 tests.

- [ ] **Step 5: Verify nothing else moved**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 95 passing total (85 existing + 10 new), and `0` warnings.

- [ ] **Step 6: Commit**

```bash
git add src/filter.rs src/lib.rs
git commit -m "feat(filter): add the filter matching model

A filter set describes a log format rather than a document, so it is
modelled independently of any file or widget and can be evaluated
without a Buffer.

An empty or fully disabled set leaves every line Unmatched, so an
unfiltered file renders normally rather than wholly dimmed. The first
matching filter wins, which is what gives the set's order meaning."
```

---

### Task 2: `Document` and the verdict cache

Holds the lines with their verdicts, so evaluation happens when something
changes rather than once per frame.

**Files:**
- Create: `src/document.rs`
- Modify: `src/lib.rs` (add `pub mod document;`)

**Interfaces:**
- Consumes: `FilterSet`, `Verdict` from Task 1.
- Produces:
  - `pub struct Document { path: PathBuf, lines: Vec<String>, verdicts: Vec<Verdict> }`
  - `pub fn new(path: PathBuf, lines: Vec<String>) -> Self`
  - `pub fn path(&self) -> &PathBuf`
  - `pub fn lines(&self) -> &[String]`
  - `pub fn verdicts(&self) -> &[Verdict]`
  - `pub fn evaluate(&mut self, filters: &FilterSet)`
  - `pub fn line_styles(&self, filters: &FilterSet) -> Vec<Option<Style>>`
  - `pub fn match_count(&self) -> usize`

- [ ] **Step 1: Write the failing tests**

Create `src/document.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn doc(lines: &[&str]) -> Document {
        Document::new(
            PathBuf::from("fixture.log"),
            lines.iter().map(|l| l.to_string()).collect(),
        )
    }

    fn set_with(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in patterns {
            let style = set.next_style();
            set.add(pattern, style).expect("valid pattern");
        }
        set
    }

    #[test]
    fn a_new_document_has_a_verdict_for_every_line() {
        let document = doc(&["one", "two", "three"]);

        assert_eq!(document.verdicts().len(), document.lines().len());
    }

    #[test]
    fn evaluating_records_each_line_s_verdict() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);

        document.evaluate(&filters);

        assert_eq!(
            document.verdicts(),
            &[Verdict::Unmatched, Verdict::Included(0), Verdict::Unmatched]
        );
    }

    #[test]
    fn re_evaluating_replaces_the_previous_verdicts() {
        let mut document = doc(&["alpha", "beta"]);
        document.evaluate(&set_with(&["beta"]));

        document.evaluate(&set_with(&["alpha"]));

        assert_eq!(
            document.verdicts(),
            &[Verdict::Included(0), Verdict::Unmatched]
        );
    }

    #[test]
    fn match_count_reports_included_lines_only() {
        let mut document = doc(&["foo a", "bar", "foo b"]);
        document.evaluate(&set_with(&["foo"]));

        assert_eq!(document.match_count(), 2);
    }

    /// The vector handed to the textarea has one entry per line, so no line is
    /// left to fall through to whatever the previous file's styles were.
    #[test]
    fn line_styles_covers_every_line() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_eq!(styles.len(), 2);
    }

    #[test]
    fn matching_lines_take_their_filter_s_colour_and_others_dim() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_eq!(styles[1].expect("beta styled").fg, filters.filters()[0].style.fg);
        assert!(
            styles[0]
                .expect("alpha styled")
                .add_modifier
                .contains(Modifier::DIM),
            "unmatched line not dimmed"
        );
    }

    /// Without filters nothing is dimmed, so an ordinary file looks ordinary.
    #[test]
    fn an_unfiltered_document_styles_nothing() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = FilterSet::new();
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert!(styles.iter().all(Option::is_none));
    }

    #[test]
    fn two_filters_colour_their_lines_differently() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["alpha", "beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_ne!(styles[0].unwrap().fg, styles[1].unwrap().fg);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib document::`
Expected: FAIL to compile — `cannot find type Document in this scope`.

- [ ] **Step 3: Implement the module**

Put this above the test module in `src/document.rs`:

```rust
//! The loaded file and what the filters made of it.

use crate::filter::{FilterSet, Verdict};
use ratatui::style::Style;
use std::path::PathBuf;

/// A loaded file, with a cached verdict per line.
///
/// Evaluating a filter set is O(lines × filters), which is not free on a large
/// log, so verdicts are computed when the lines or the filters change rather
/// than once per frame.
#[derive(Debug, Default)]
pub struct Document {
    path: PathBuf,
    lines: Vec<String>,
    verdicts: Vec<Verdict>,
}

impl Document {
    pub fn new(path: PathBuf, lines: Vec<String>) -> Self {
        let verdicts = vec![Verdict::Unmatched; lines.len()];
        Self {
            path,
            lines,
            verdicts,
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn verdicts(&self) -> &[Verdict] {
        &self.verdicts
    }

    /// Recompute every line's verdict. Call when the lines or the filters change.
    pub fn evaluate(&mut self, filters: &FilterSet) {
        self.verdicts = self
            .lines
            .iter()
            .map(|line| filters.verdict(line))
            .collect();
    }

    /// How many lines an including filter selected.
    pub fn match_count(&self) -> usize {
        self.verdicts
            .iter()
            .filter(|verdict| matches!(verdict, Verdict::Included(_)))
            .count()
    }

    /// One style slot per line, for `FileView::set_line_styles`.
    ///
    /// Always covers every line, so a shorter vector can never leave trailing
    /// lines wearing styles computed for a previously loaded file.
    pub fn line_styles(&self, filters: &FilterSet) -> Vec<Option<Style>> {
        self.verdicts
            .iter()
            .map(|verdict| filters.style_for(*verdict))
            .collect()
    }
}
```

Register it with `pub mod document;` in `src/lib.rs`, beside `pub mod filter;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib document::`
Expected: PASS, 8 tests.

- [ ] **Step 5: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 103 passing, `0` warnings.

- [ ] **Step 6: Commit**

```bash
git add src/document.rs src/lib.rs
git commit -m "feat(document): cache a verdict per line

Evaluating filters is O(lines x filters), which is not free on a large
log, so verdicts are cached and recomputed when the lines or filters
change rather than once per frame.

line_styles always covers every line, so a short vector can never leave
trailing lines wearing a previous file's styles."
```

---

### Task 3: The cursor line must not escape dimming

Closes the gap the Phase 1 review found, before anything depends on dimming
being uniform.

**Files:**
- Modify: `src/widgets/fileview.rs`

**Interfaces:**
- Consumes: `TextArea::line_styles()` (a Phase 1 getter).
- Produces: no new public API. `FileView::render` now derives its cursor-line
  style from the cursor row's entry in the style vector.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/widgets/fileview.rs`. `row_of` and `row_has_fg`
already exist there from Phase 1 — reuse them, do not redefine them.

```rust
    /// The cursor line must not escape dimming: the textarea replaces rather
    /// than merges line styles, so `render` has to fold the line's own style
    /// into the cursor-line style.
    #[test]
    fn the_cursor_line_keeps_its_own_line_style() {
        let mut view = view_of("cursor_dim.txt", "alpha\nbeta\n");
        // The cursor starts on row 0.
        view.set_line_styles(vec![
            Some(Style::default().fg(Color::Yellow)),
            Some(Style::default().fg(Color::Yellow)),
        ]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        assert!(
            row_has_fg(&buf, alpha, Color::Yellow),
            "the cursor's line lost its style"
        );
    }

    /// With focus, the cursor line is still distinguishable from its neighbours.
    #[test]
    fn an_active_view_still_marks_the_cursor_line() {
        let mut view = view_of("cursor_active.txt", "alpha\nbeta\n");
        view.active = true;
        view.set_line_styles(vec![Some(Style::default().fg(Color::Yellow)); 2]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        let beta = row_of(&buf, "beta");
        let cursor_cell = (0..area.width)
            .map(|x| buf[(x, alpha)].style())
            .find(|s| s.add_modifier.contains(Modifier::REVERSED));
        assert!(cursor_cell.is_some(), "cursor line not marked when active");
        assert!(
            row_has_fg(&buf, beta, Color::Yellow),
            "the other line lost its style"
        );
    }

    /// Without line styles, the old behaviour is unchanged.
    #[test]
    fn without_line_styles_the_cursor_line_is_unchanged() {
        let mut view = view_of("cursor_plain.txt", "alpha\nbeta\n");
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        assert!(!row_has_fg(&buf, alpha, Color::Yellow));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib the_cursor_line_keeps_its_own_line_style`
Expected: FAIL — the assertion `the cursor's line lost its style` fires, because
`render` currently overwrites the cursor row's style with `Style::default()`.

- [ ] **Step 3: Derive the cursor-line style from the line's own style**

In `FileView::render`, replace:

```rust
        let mut style = Style::default();
        if self.active {
            style = style.fg(Color::Green).add_modifier(Modifier::REVERSED);
        }
        self.textarea.set_cursor_line_style(style);
```

with:

```rust
        // The textarea replaces rather than merges a line's style, so the
        // cursor line would otherwise discard whatever the filters gave it and
        // read as unfiltered. Start from that line's own style and add the
        // focus decoration on top.
        let cursor_row = self.textarea.cursor().0;
        let mut style = self
            .textarea
            .line_styles()
            .get(cursor_row)
            .copied()
            .flatten()
            .unwrap_or_default();
        if self.active {
            style = style.fg(Color::Green).add_modifier(Modifier::REVERSED);
        }
        self.textarea.set_cursor_line_style(style);
```

Note the borrow: `line_styles()` borrows the textarea immutably and
`set_cursor_line_style` borrows it mutably, so the `.copied()` must land before
the setter is called. Writing it as above (a `let` binding that ends the borrow)
is what makes it compile.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS. 106 lib+ tests overall; in particular
`the_cursor_line_keeps_its_own_line_style`, `an_active_view_still_marks_the_cursor_line`
and `without_line_styles_the_cursor_line_is_unchanged` all pass, and the Phase 1
test `line_styles_reach_the_rendered_view` still passes.

- [ ] **Step 5: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 106 passing, `0` warnings.

- [ ] **Step 6: Update the doc comment that described the old behaviour**

`FileView::set_line_styles`' doc currently warns that the cursor line discards
its style. That is no longer true. Replace that paragraph with:

```rust
    /// The line the cursor is on keeps its style too: `render` folds it into
    /// the cursor-line style, because the textarea replaces rather than merges.
```

- [ ] **Step 7: Commit**

```bash
git add src/widgets/fileview.rs
git commit -m "fix(fileview): stop the cursor line escaping its line style

The textarea replaces rather than merges a line's style, and render set
a cursor-line style unconditionally, so the cursor's line discarded
whatever it had been given - and rendered Reset while the pane was
inactive, making one line of a dimmed view read as a match.

render now starts from that line's own style and adds the focus
decoration on top."
```

---

### Task 4: Add a filter at a prompt

Reuses the search prompt's shape, which the user already knows.

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `FilterSet::add`, `FilterSet::next_style` from Task 1.
- Produces: `App` gains `filters: FilterSet`; the existing `SearchPrompt` gains
  a `kind: PromptKind` field with `enum PromptKind { Search, Filter }`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`. The helpers `app_over`, `key`, `typed`,
`prompt_line` and `draw` already exist there — reuse them.

```rust
    #[test]
    fn f_opens_a_filter_prompt() {
        let mut app = app_over("filter_prompt", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");

        assert_eq!(prompt_line(&mut app), "filter: foo");
    }

    #[test]
    fn committing_a_filter_adds_it() {
        let mut app = app_over("filter_add", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_none(), "prompt stayed open");
        assert_eq!(app.filters.len(), 1);
    }

    #[test]
    fn an_invalid_filter_pattern_keeps_the_prompt_open() {
        let mut app = app_over("filter_bad", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "[");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_some(), "prompt closed on an invalid pattern");
        assert!(prompt_line(&mut app).contains("E486"));
        assert_eq!(app.filters.len(), 0, "a rejected pattern must not be added");
    }

    #[test]
    fn esc_cancels_a_filter_prompt_without_adding() {
        let mut app = app_over("filter_esc", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");
        key(&mut app, KeyCode::Esc);

        assert!(app.search.is_none());
        assert_eq!(app.filters.len(), 0);
    }

    /// The prompt swallows keys, so `q` types rather than quits — as for search.
    #[test]
    fn q_while_filtering_is_typed_not_quit() {
        let mut app = app_over("filter_q", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "q");

        assert!(app.is_running());
        assert_eq!(prompt_line(&mut app), "filter: q");
    }

    #[test]
    fn successive_filters_take_different_colours() {
        let mut app = app_over("filter_colours", &["a.rs"]);

        for pattern in ["foo", "bar"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }

        let styles: Vec<_> = app.filters.filters().iter().map(|f| f.style.fg).collect();
        assert_ne!(styles[0], styles[1]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib f_opens_a_filter_prompt`
Expected: FAIL — `no field 'filters' on type 'App'`.

- [ ] **Step 3: Add the prompt kind**

In `src/lib.rs`, beside `SearchPrompt`:

```rust
/// What an open prompt will do with the pattern being typed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    #[default]
    Search,
    Filter,
}
```

Add `kind: PromptKind` to `SearchPrompt`, and change its `line()` so a filter
prompt is distinguishable from a search:

```rust
    fn line(&self) -> String {
        match (&self.error, self.kind) {
            (Some(error), _) => error.clone(),
            (None, PromptKind::Filter) => format!("filter: {}", self.pattern),
            (None, PromptKind::Search) => format!(
                "{}{}",
                if self.reverse { '?' } else { '/' },
                self.pattern
            ),
        }
    }
```

- [ ] **Step 4: Hold a filter set on `App`**

Add `filters: FilterSet,` to `struct App`, and `filters: FilterSet::new(),` to
the literal in `App::new`. Import with `use crate::filter::FilterSet;` — or
`use filter::FilterSet;` to match the file's existing style.

- [ ] **Step 5: Open the prompt on `f`**

In `handle_event`'s app-wide key match, beside the `/` and `?` arm:

```rust
                KeyCode::Char('f') => {
                    self.search = Some(SearchPrompt {
                        kind: PromptKind::Filter,
                        ..SearchPrompt::default()
                    });
                    return Ok(());
                }
```

- [ ] **Step 6: Commit the pattern on Enter**

In `handle_search_key`'s `Enter` arm, branch on the prompt's kind. Where it
currently calls `self.run_search(&pattern, reverse)`, use:

```rust
                let outcome = match prompt.kind {
                    PromptKind::Search => self.run_search(&pattern, reverse),
                    PromptKind::Filter => self.add_filter(&pattern),
                };
                if outcome.is_ok() {
                    self.search = None;
                } else if let Some(prompt) = self.search.as_mut() {
                    prompt.error = Some(INVALID_PATTERN.to_string());
                }
```

reading `prompt.kind` out alongside `pattern` and `reverse` before the mutable
borrow, exactly as those two already are. Add:

```rust
    /// Add an including filter, colouring it distinctly from its predecessors.
    fn add_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let style = self.filters.next_style();
        self.filters.add(pattern, style)
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including all six new tests.

- [ ] **Step 8: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 112 passing, `0` warnings.

- [ ] **Step 9: Commit**

```bash
git add src/lib.rs
git commit -m "feat(app): add filters at a prompt with f

Reuses the search prompt's shape, which the user already knows, rather
than inventing a second input idiom. The prompt now carries a kind, so
its sigil says which of the two it is.

An invalid pattern reports E486 and leaves the prompt open to correct,
matching how search already behaves."
```

---

### Task 5: Apply the styles to the view

Where the model finally reaches the screen.

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `Document`, `FilterSet`, `FileView::set_line_styles`.
- Produces: `App` gains `document: Document`, and a private
  `fn restyle(&mut self)` that re-evaluates and pushes styles into the view.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`:

```rust
    /// Returns the styles the file view is currently rendering with.
    fn view_line_styles(app: &App) -> Vec<Option<Style>> {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.textarea.line_styles().to_vec()),
                AppWidget::FileNav(_) => None,
            })
            .expect("no file view")
    }

    fn app_over_file(name: &str, body: &str) -> App<'static> {
        let dir = std::path::Path::new("target/test-appdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        let file = dir.join("log.txt");
        fs::write(&file, body).expect("write fixture");
        App::new(&Config {
            file: file.display().to_string(),
        })
    }

    #[test]
    fn committing_a_filter_styles_the_view() {
        let mut app = app_over_file("restyle", "alpha\nbeta\ngamma\n");

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let styles = view_line_styles(&app);
        assert_eq!(styles.len(), 3, "a style slot per line");
        assert!(styles[1].is_some(), "matching line unstyled");
        assert!(
            styles[0]
                .expect("unmatched line unstyled")
                .add_modifier
                .contains(Modifier::DIM),
            "unmatched line not dimmed"
        );
    }

    #[test]
    fn an_unfiltered_view_has_no_styles() {
        let app = app_over_file("restyle_none", "alpha\nbeta\n");

        assert!(view_line_styles(&app).iter().all(Option::is_none));
    }

    /// Filters describe a log format, so they outlive the file they were
    /// defined against — and must be re-applied after a load clears them.
    #[test]
    fn filters_survive_loading_another_file() {
        let mut app = app_over_file("restyle_reload", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let dir = std::path::Path::new("target/test-appdirs/restyle_reload");
        fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
        app.perform(Action::Load(dir.join("other.txt")));

        let styles = view_line_styles(&app);
        assert_eq!(styles.len(), 2, "styles not re-applied to the new file");
        assert!(styles[0].is_some(), "match in the new file unstyled");
    }

    #[test]
    fn bang_disables_every_filter_and_restores_them() {
        let mut app = app_over_file("restyle_bang", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));
        assert!(
            view_line_styles(&app).iter().all(Option::is_none),
            "! did not clear the styling"
        );

        key(&mut app, KeyCode::Char('!'));
        assert!(
            view_line_styles(&app)[1].is_some(),
            "! did not restore the filters"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib committing_a_filter_styles_the_view`
Expected: FAIL — the styles vector is empty, because nothing pushes it yet.

- [ ] **Step 3: Hold a document on `App`**

`src/lib.rs` does not currently import either of these — add them:

```rust
use ratatui::style::Modifier;   // the tests assert on Modifier::DIM
use std::path::PathBuf;         // sync_document builds one
```

Add `document: Document,` to `struct App` and `document: Document::default(),`
to `App::new`'s literal. Then populate it wherever the view's contents change.
`App::perform` is the one place that loads or previews:

```rust
    fn perform(&mut self, action: Action) {
        for widget in &mut self.widgets {
            if let AppWidget::FileView(view) = widget {
                match &action {
                    Action::Load(path) => view.load(path),
                    Action::Preview(path) => view.preview(path),
                }
            }
        }
        self.sync_document();
        self.restyle();
    }

    /// Take the view's current contents as the document to filter.
    ///
    /// The view owns the reading — including its preview truncation and its
    /// error messages — so the document follows it rather than re-reading.
    fn sync_document(&mut self) {
        let Some((path, lines)) = self.widgets.iter().find_map(|w| match w {
            AppWidget::FileView(view) => Some((
                PathBuf::from(&view.filename),
                view.textarea.lines().to_vec(),
            )),
            AppWidget::FileNav(_) => None,
        }) else {
            return;
        };
        self.document = Document::new(path, lines);
    }

    /// Re-evaluate the filters and push the resulting styles into the view.
    ///
    /// Loading or previewing rebuilds the textarea, which clears its line
    /// styles, so this must run after any change to the contents as well as
    /// after any change to the filters.
    fn restyle(&mut self) {
        self.document.evaluate(&self.filters);
        let styles = self.document.line_styles(&self.filters);
        for widget in &mut self.widgets {
            if let AppWidget::FileView(view) = widget {
                view.set_line_styles(styles.clone());
            }
        }
    }
```

Call `self.sync_document(); self.restyle();` at the end of `App::new` too, so
the file named on the command line is filtered from the first frame.

- [ ] **Step 4: Restyle after a filter is added**

In `add_filter`, after a successful `add`, call `self.restyle()`:

```rust
    fn add_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let style = self.filters.next_style();
        self.filters.add(pattern, style)?;
        self.restyle();
        Ok(())
    }
```

- [ ] **Step 5: Add the `!` toggle**

In `handle_event`'s app-wide match:

```rust
                KeyCode::Char('!') => {
                    // Toggle the whole set, so an unfiltered view is one
                    // keystroke away without losing the filters themselves.
                    let enable = !self.filters.any_enabled();
                    self.filters.set_all_enabled(enable);
                    self.restyle();
                    return Ok(());
                }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including all four new tests.

- [ ] **Step 7: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 116 passing, `0` warnings.

- [ ] **Step 8: Commit**

```bash
git add src/lib.rs
git commit -m "feat(app): dim lines that match no filter

Loading or previewing rebuilds the textarea and clears its line styles,
so restyle runs after any change to the contents as well as after any
change to the filters - which is what lets a filter set outlive the file
it was defined against.

! toggles the whole set off and back on, so an unfiltered view is one
keystroke away without discarding the filters."
```

---

### Task 6: Status line and documentation

Without this there is no indication a filter is active beyond the dimming
itself, and no record of the two new keys.

**Files:**
- Modify: `src/lib.rs`
- Modify: `README.md`

**Interfaces:**
- Consumes: `FilterSet::len`, `FilterSet::any_enabled`, `Document::match_count`.
- Produces: no new public API.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`:

```rust
    /// The bottom row when no prompt is open.
    fn status_line(app: &mut App) -> String {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        let y = AREA.height - 1;
        (0..AREA.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn the_status_line_is_empty_without_filters() {
        let mut app = app_over_file("status_none", "alpha\n");

        assert_eq!(status_line(&mut app), "");
    }

    #[test]
    fn the_status_line_reports_filters_and_matches() {
        let mut app = app_over_file("status_some", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let status = status_line(&mut app);

        assert!(status.contains('1'), "filter count missing: {status}");
        assert!(status.contains("1/3"), "match count missing: {status}");
    }

    #[test]
    fn the_status_line_says_when_filters_are_disabled() {
        let mut app = app_over_file("status_off", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));

        assert!(
            status_line(&mut app).contains("disabled"),
            "no indication the filters are off: {}",
            status_line(&mut app)
        );
    }

    /// An open prompt takes the row, as it already does.
    #[test]
    fn a_prompt_still_takes_the_bottom_row() {
        let mut app = app_over_file("status_prompt", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");

        assert_eq!(status_line(&mut app), "filter: foo");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib the_status_line_reports_filters_and_matches`
Expected: FAIL — the bottom row is blank, because nothing writes a status line yet.

- [ ] **Step 3: Render the status line**

`render` currently reserves the bottom row only while `self.search.is_some()`.
Reserve it whenever there is something to show:

```rust
        let status = self.status_text();
        let (area, prompt_area) = if self.search.is_some() || !status.is_empty() {
            let [panes, prompt] = Layout::vertical([Min(0), Length(1)]).areas(area);
            (panes, Some(prompt))
        } else {
            (area, None)
        };
```

and where it currently writes the prompt, fall back to the status text:

```rust
        if let Some(prompt_area) = prompt_area {
            let (text, style) = match self.search.as_ref() {
                Some(prompt) if prompt.error.is_some() => {
                    (prompt.line(), Style::default().fg(Color::Red))
                }
                Some(prompt) => (prompt.line(), Style::default()),
                None => (status, Style::default().fg(Color::DarkGray)),
            };
            buf.set_stringn(
                prompt_area.x,
                prompt_area.y,
                text,
                prompt_area.width as usize,
                style,
            );
        }
```

with:

```rust
    /// A one-line summary of the filter state, empty when no filters exist.
    ///
    /// Dimming alone does not say *why* lines are dim, or that a filter is
    /// defined but currently disabled — the pane would just look ordinary.
    fn status_text(&self) -> String {
        if self.filters.is_empty() {
            return String::new();
        }
        let filters = self.filters.len();
        if !self.filters.any_enabled() {
            return format!("{filters} filters (disabled)");
        }
        format!(
            "{filters} filters   {}/{} lines match",
            self.document.match_count(),
            self.document.lines().len()
        )
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including the four new tests.

- [ ] **Step 5: Document the two new keys**

In `README.md`'s global keybindings table, after the `?` row:

```markdown
| `f` | Add an include filter to the focused pane's view |
| `!` | Disable every filter, or restore them |
```

And after the table's surrounding prose, a short paragraph:

```markdown
Filters colour the lines they match and dim the rest; they are regular
expressions, like search. A filter set describes a log format rather than one
file, so it survives loading another file — `!` is the single keystroke back to
an unfiltered view without discarding the set. Nothing is hidden: filters only
change how lines are presented.
```

- [ ] **Step 6: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
cargo clippy --lib 2>&1 | grep -c "^warning:"
```
Expected: 120 passing, `0` test warnings. Clippy should report only the
pre-existing `AppWidget` variant-size warning in `src/widgets/mod.rs` — a file
this phase does not touch. Any *new* clippy warning is a defect to fix.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs README.md
git commit -m "feat(app): report the filter state on a status line

Dimming alone does not say why lines are dim, and a filter set that is
defined but disabled is invisible - the pane just looks ordinary. The
bottom row now reports the filter count and how many lines match, and
says so when the set is disabled.

The row is still surrendered to a prompt while one is open."
```

---

## Phase 2a completion criteria

- `cargo test` reports **120 passing**, 0 failed; the 85 pre-existing tests are
  unmodified.
- `cargo test 2>&1 | grep -ci "^warning"` prints `0`.
- `cargo clippy --lib` reports only the pre-existing `AppWidget` warning.
- No line is ever hidden: `set_line_numbers` is not called anywhere, and the
  textarea is rebuilt only by `load`/`preview` as it already was.
- Manual check in a real terminal: `cargo run -- Cargo.toml`, press `f`, type
  `version`, Enter. Matching lines are coloured, the rest dimmed, and the line
  under the cursor is dimmed like its neighbours rather than standing out.

Phase 2b (excluding filters, Ctrl+H, the visible↔source mapping and the cursor
round trip) begins only once all of the above hold.
