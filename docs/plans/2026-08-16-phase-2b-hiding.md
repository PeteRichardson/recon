# Phase 2b — hiding — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lines can now disappear. `F` adds an excluding filter that removes
matching lines outright, and `Ctrl-H`/`H` toggles between dimming non-matching
lines and hiding them — returning you to the exact line you were on.

**Architecture:** `Document` gains a `visible: Vec<usize>` — the source line
indices currently on screen — and becomes the source of truth for the file's
lines, because in hidden mode the textarea holds only a subset. `App` rebuilds
the textarea from `visible` and supplies each row's *source* number via
`FileView::set_line_numbers`, the second capability the Phase 1 fork added. The
cursor is stored as a **source line index**, never a position in the visible
list, which is what makes the round trip exact.

**Tech Stack:** Rust 2021, ratatui 0.30.2, vendored tui-textarea-2 0.12.1,
crossterm 0.29, regex 1.

**Spec:** `docs/specs/2026-08-15-filter-based-viewing-design.md` (Phase 2b)

## Global Constraints

- **The cursor is a source line index.** Never store a position in the visible
  list and derive the source index from it. Step 4 of the user's workflow —
  toggle back and land on the exact line — is exact by construction only if
  this holds. Any code that round-trips through a visible position is a defect.
- **Excluded lines are hidden in *both* modes.** An excluding filter removes
  lines whether or not `Ctrl-H` is on; the toggle governs `Unmatched` lines only.
- **Filters persist across file loads.** Already true; hiding must not break it.
- 123 tests pass today; all must still pass. Any pre-existing test that appears
  to need editing is a signal something is wrong — stop and report.
- **Test output must be pristine.** Verify with `cargo test 2>&1 | grep -ci "^warning"`
  printing `0`. Never verify with `grep -E "^test result"`, which filters
  warnings out by construction and has hidden a real warning on this project
  twice.
- **Guard every new global key with `key.modifiers.is_empty()`** (except where
  the binding *is* a modified key). An unguarded `KeyCode::Char` arm swallowed
  `Ctrl-f` in the previous phase and broke page-down.
- Tests must not depend on the live repo-root directory listing or on build
  artefacts. Use fixture directories under `target/`. Repo-listing coupling has
  broken this suite three times.
- TDD throughout: failing test first, observe it fail, then implement.

## File Structure

| File | Responsibility |
|---|---|
| `src/filter.rs` | Excluding filters: `add_excluding`, and `Verdict::Excluded` actually produced |
| `src/document.rs` | `visible`, the source↔visible mapping, and the mode that decides it |
| `src/lib.rs` | `F`, the `Ctrl-H`/`H` toggle, rebuilding the buffer, cursor preservation, the funnel |
| `README.md` | The two new keys and what hiding means |

No new modules: `Document` is the right home for the mapping, because it already
owns the verdicts the mapping derives from.

---

### Task 1: Excluding filters

Evaluation only — nothing is hidden yet, and no key creates one. Splitting this
out means the evaluation order can be tested exhaustively before any rendering
depends on it.

**Files:**
- Modify: `src/filter.rs`

**Interfaces:**
- Consumes: the existing `FilterSet`, `Sense`, `Verdict`.
- Produces:
  - `pub fn add_excluding(&mut self, pattern: &str) -> Result<(), regex::Error>`
  - `verdict` now returns `Verdict::Excluded` for a line matching any enabled
    excluding filter
  - `pub fn any_excluding(&self) -> bool`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/filter.rs`. The helper `set_with` already exists —
reuse it.

```rust
    fn set_excluding(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in patterns {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    #[test]
    fn an_excluding_filter_excludes_its_matches() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.verdict("a heartbeat line"), Verdict::Excluded);
    }

    /// Excluding filters run after including ones, so exclusion wins even on a
    /// line an including filter selected.
    #[test]
    fn exclusion_beats_inclusion_on_the_same_line() {
        let mut set = set_with(&["foo"]);
        set.add_excluding("noisy").expect("valid pattern");

        assert_eq!(set.verdict("foo but noisy"), Verdict::Excluded);
        assert_eq!(set.verdict("foo alone"), Verdict::Included(0));
    }

    /// With only excluding filters, unmatched lines stay ordinary — there is
    /// nothing to dim against.
    #[test]
    fn excluding_filters_alone_do_not_dim() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.verdict("something else"), Verdict::Unmatched);
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    #[test]
    fn a_disabled_excluding_filter_excludes_nothing() {
        let mut set = set_excluding(&["heartbeat"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("a heartbeat line"), Verdict::Unmatched);
    }

    #[test]
    fn an_invalid_excluding_pattern_is_reported() {
        let mut set = FilterSet::new();

        assert!(set.add_excluding("[").is_err());
        assert!(set.is_empty(), "a rejected pattern must not be added");
    }

    #[test]
    fn any_excluding_reports_whether_one_is_enabled() {
        let mut set = set_with(&["foo"]);
        assert!(!set.any_excluding());

        set.add_excluding("bar").expect("valid pattern");
        assert!(set.any_excluding());

        set.set_all_enabled(false);
        assert!(!set.any_excluding(), "a disabled filter does not count");
    }

    /// An excluded line is never rendered, so it has no style.
    #[test]
    fn an_excluded_line_has_no_style() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.style_for(Verdict::Excluded), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filter::`
Expected: FAIL to compile — `no method named 'add_excluding' found`.

- [ ] **Step 3: Add the constructor and the predicate**

In `src/filter.rs`, beside `add`:

```rust
    /// Add an excluding filter: its matches are removed from view entirely,
    /// in both display modes.
    ///
    /// Excluding filters carry no colour, since a line they match is never
    /// rendered.
    pub fn add_excluding(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.filters.push(Filter {
            pattern,
            sense: Sense::Exclude,
            enabled: true,
            style: Style::default(),
        });
        Ok(())
    }

    /// Whether any enabled filter removes lines.
    pub fn any_excluding(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense == Sense::Exclude)
    }
```

- [ ] **Step 4: Apply exclusion after inclusion**

Replace the body of `verdict` with:

```rust
    pub fn verdict(&self, line: &str) -> Verdict {
        // Exclusion is applied after inclusion and overrides it, so a line an
        // including filter selected is still removed if an excluding filter
        // also matches it.
        if self
            .filters
            .iter()
            .any(|filter| {
                filter.enabled && filter.sense == Sense::Exclude && filter.pattern.is_match(line)
            })
        {
            return Verdict::Excluded;
        }

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
```

- [ ] **Step 5: Keep `style_for` correct for the new case**

`style_for` already returns `None` for `Verdict::Excluded`. But its
`Unmatched` arm dims whenever `any_enabled()` is true, which would now dim a
whole file when the only filters are excluding ones — there is nothing to dim
*against*. Change that guard to ask whether any *including* filter is enabled:

```rust
    /// Whether any enabled filter selects lines, as opposed to removing them.
    ///
    /// Dimming means "this line matched no include filter". With only
    /// excluding filters there is nothing to dim against, so the file reads
    /// normally minus the removed lines.
    fn any_including(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense == Sense::Include)
    }
```

and in `style_for`, replace `Verdict::Unmatched if self.any_enabled()` with
`Verdict::Unmatched if self.any_including()`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib filter::`
Expected: PASS. The existing filter tests must pass unchanged — in particular
`a_fully_disabled_set_leaves_lines_unmatched` and
`an_empty_set_leaves_every_line_unmatched`.

- [ ] **Step 7: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 130 passing, `0` warnings.

- [ ] **Step 8: Commit**

```bash
git add src/filter.rs
git commit -m "feat(filter): add excluding filters

Exclusion is applied after inclusion and overrides it, so a line an
include filter selected is still removed when an exclude filter matches.

Dimming now keys off whether any *including* filter is enabled rather
than any filter at all: with only excluding filters there is nothing to
dim against, and the file should read normally minus the removed lines."
```

---

### Task 2: The visible mapping

`Document` learns which lines are on screen. Still no rendering change — this is
the model the next two tasks consume.

**Files:**
- Modify: `src/document.rs`

**Interfaces:**
- Consumes: `Verdict` from Task 1.
- Produces, on `Document`:
  - `pub enum Mode { Dimmed, FilteredOnly }` (re-exported from `document`)
  - `pub fn set_mode(&mut self, mode: Mode)` and `pub fn mode(&self) -> Mode`
  - `pub fn visible(&self) -> &[usize]` — source indices currently on screen
  - `pub fn visible_lines(&self) -> Vec<String>` — the text for those indices
  - `pub fn visible_styles(&self, filters: &FilterSet) -> Vec<Option<Style>>`
  - `pub fn visible_position(&self, source: usize) -> Option<usize>`
  - `pub fn nearest_visible(&self, source: usize) -> Option<usize>` — the source
    index of the first visible line at or after `source`, else the last before it
  - `pub fn source_at(&self, visible_row: usize) -> Option<usize>`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/document.rs`. `doc` and `set_with` already exist.

```rust
    fn set_excluding(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in patterns {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    #[test]
    fn dimmed_mode_shows_every_line_that_is_not_excluded() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[0, 1, 2]);
    }

    /// Excluded lines are gone in both modes — the toggle governs unmatched
    /// lines only.
    #[test]
    fn excluded_lines_are_hidden_even_when_dimmed() {
        let mut document = doc(&["alpha", "noise", "gamma"]);
        document.evaluate(&set_excluding(&["noise"]));

        assert_eq!(document.mode(), Mode::Dimmed);
        assert_eq!(document.visible(), &[0, 2]);
    }

    #[test]
    fn filtered_only_mode_shows_matches_alone() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[1, 3]);
    }

    #[test]
    fn visible_lines_are_the_text_of_the_visible_indices() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_lines(), vec!["beta".to_string()]);
    }

    #[test]
    fn visible_styles_line_up_with_visible_lines() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        assert_eq!(
            document.visible_styles(&filters).len(),
            document.visible().len()
        );
    }

    #[test]
    fn source_and_visible_positions_map_both_ways() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_position(3), Some(1));
        assert_eq!(document.source_at(1), Some(3));
        assert_eq!(document.visible_position(0), None, "line 0 is hidden");
    }

    /// Toggling into filtered mode from a hidden line snaps forward to the
    /// next match, which is what the user was navigating towards.
    #[test]
    fn nearest_visible_snaps_forward() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.nearest_visible(0), Some(1));
        assert_eq!(document.nearest_visible(2), Some(3));
    }

    /// With no match after it, fall back to the one before rather than losing
    /// the cursor entirely.
    #[test]
    fn nearest_visible_falls_back_to_the_previous_match() {
        let mut document = doc(&["beta", "alpha", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.nearest_visible(2), Some(0));
    }

    #[test]
    fn nearest_visible_is_none_when_nothing_is_visible() {
        let mut document = doc(&["alpha", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["zzz"]));

        assert!(document.visible().is_empty());
        assert_eq!(document.nearest_visible(0), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib document::`
Expected: FAIL to compile — `cannot find type Mode in this scope`.

- [ ] **Step 3: Add the mode and the mapping**

In `src/document.rs`, above `Document`:

```rust
/// Which lines the file view shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Every line that no excluding filter removed; unmatched lines are dimmed.
    #[default]
    Dimmed,
    /// Only lines an including filter selected.
    FilteredOnly,
}
```

Add `mode: Mode,` and `visible: Vec<usize>,` to `Document`, initialised to
`Mode::default()` and an empty `Vec` in `new`. Then:

```rust
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Change which lines are shown. The caller must re-`evaluate` afterwards.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Source line indices currently on screen, in order.
    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// The text of the visible lines, for rebuilding the view's buffer.
    pub fn visible_lines(&self) -> Vec<String> {
        self.visible
            .iter()
            .map(|&source| self.lines[source].clone())
            .collect()
    }

    /// One style slot per *visible* line, aligned with `visible_lines`.
    pub fn visible_styles(&self, filters: &FilterSet) -> Vec<Option<Style>> {
        self.visible
            .iter()
            .map(|&source| filters.style_for(self.verdicts[source]))
            .collect()
    }

    /// Where a source line sits in the visible list, if it is shown at all.
    pub fn visible_position(&self, source: usize) -> Option<usize> {
        self.visible.binary_search(&source).ok()
    }

    /// The source index of the visible row at `visible_row`.
    pub fn source_at(&self, visible_row: usize) -> Option<usize> {
        self.visible.get(visible_row).copied()
    }

    /// The nearest visible source line at or after `source`, falling back to
    /// the last one before it.
    ///
    /// Used when a mode change hides the line the cursor was on: snapping
    /// forward lands on the match the user was navigating towards, and the
    /// backward fallback stops the cursor being lost when nothing follows.
    pub fn nearest_visible(&self, source: usize) -> Option<usize> {
        match self.visible.binary_search(&source) {
            Ok(_) => Some(source),
            Err(index) => self
                .visible
                .get(index)
                .copied()
                .or_else(|| self.visible.last().copied()),
        }
    }
```

`visible` is built in ascending order, so `binary_search` is valid — do not
replace it with a linear scan.

- [ ] **Step 4: Rebuild `visible` when evaluating**

At the end of `evaluate`, after the verdicts and `match_count` are set:

```rust
        self.visible = self
            .verdicts
            .iter()
            .enumerate()
            .filter(|(_, verdict)| match (self.mode, verdict) {
                // Excluded lines are gone in both modes; the toggle governs
                // unmatched lines only.
                (_, Verdict::Excluded) => false,
                (Mode::Dimmed, _) => true,
                (Mode::FilteredOnly, Verdict::Included(_)) => true,
                (Mode::FilteredOnly, Verdict::Unmatched) => false,
            })
            .map(|(index, _)| index)
            .collect();
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib document::`
Expected: PASS, 9 new tests.

- [ ] **Step 6: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 139 passing, `0` warnings.

- [ ] **Step 7: Commit**

```bash
git add src/document.rs
git commit -m "feat(document): track which lines are visible

Adds the source-index mapping that hiding needs: which lines are on
screen, where a source line sits among them, and the nearest visible
line to one that has been hidden.

Excluded lines are absent in both modes - the toggle governs unmatched
lines only - which is why the filter is written as a match on the pair
rather than a mode check alone."
```

---

### Task 3: Rebuild the view from the visible lines

The rendering change. After this the view shows exactly `visible`, with source
line numbers in the gutter.

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `Document::visible_lines`, `visible_styles`, `visible`,
  `FileView::set_line_numbers` (from the Phase 1 fork).
- Produces: `App::restyle` replaced by `App::refresh_view`, which rebuilds the
  buffer as well as the styles.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`. `app_over_file`, `key`, `typed`,
`view_line_styles` and `AREA` already exist.

```rust
    /// The text the file view is currently showing, one entry per row.
    fn view_lines(app: &App) -> Vec<String> {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.textarea.lines().to_vec()),
                AppWidget::FileNav(_) => None,
            })
            .expect("no file view")
    }

    fn view_line_numbers(app: &App) -> Vec<usize> {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.textarea.line_numbers().to_vec()),
                AppWidget::FileNav(_) => None,
            })
            .expect("no file view")
    }

    #[test]
    fn an_excluding_filter_removes_its_lines_from_the_view() {
        let mut app = app_over_file("exclude_view", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_lines(&app), vec!["alpha".to_string(), "gamma".to_string()]);
    }

    /// The gutter keeps the original numbering, so a hidden line leaves a gap.
    #[test]
    fn the_gutter_shows_source_line_numbers_when_lines_are_hidden() {
        let mut app = app_over_file("exclude_gutter", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        // 0-based source indices: rows 0 and 2 render as 1 and 3.
        assert_eq!(view_line_numbers(&app), vec![0, 2]);
    }

    #[test]
    fn styles_still_line_up_with_the_rebuilt_buffer() {
        let mut app = app_over_file("exclude_styles", "alpha\nnoise\nbeta\n");
        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_line_styles(&app).len(), view_lines(&app).len());
    }

    /// With nothing excluded the buffer is the whole file and the gutter is
    /// left to number itself.
    #[test]
    fn without_hiding_the_gutter_is_not_overridden() {
        let mut app = app_over_file("no_hiding", "alpha\nbeta\n");

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_lines(&app).len(), 2);
        assert!(
            view_line_numbers(&app).is_empty(),
            "the gutter was overridden when nothing is hidden"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib an_excluding_filter_removes_its_lines_from_the_view`
Expected: FAIL — the buffer still holds all three lines, because nothing
rebuilds it.

- [ ] **Step 3: Add the imports this task needs**

`src/lib.rs` imports `KeyModifiers` only inside `mod tests`, and does not import
`Mode` at all. Both are needed by production code from here on:

```rust
use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use document::Mode;
```

- [ ] **Step 4: Add `F` for excluding filters**

In `handle_event`'s app-wide match, beside the `f` arm — note the modifier
guard, which is not optional:

```rust
                KeyCode::Char('F')
                    if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.search = Some(SearchPrompt {
                        kind: PromptKind::Exclude,
                        ..SearchPrompt::default()
                    });
                    return Ok(());
                }
```

`F` is a shifted letter, so `is_empty()` is the wrong guard — but requiring
`SHIFT` exactly is also wrong, because not every terminal reports it for a
shifted character. Require only that no `CONTROL` or `ALT` is held, which is
what actually distinguishes this from `Ctrl-F`. Add `Exclude` to `PromptKind`, give it a `exclude: ` sigil in
`SearchPrompt::line`, and route it in the Enter arm to a new
`App::add_excluding_filter`, mirroring `add_filter`:

```rust
    /// Add an excluding filter: its matches leave the view entirely.
    fn add_excluding_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.add_excluding(pattern)?;
        self.refresh_view();
        Ok(())
    }
```

- [ ] **Step 5: Rebuild the buffer, not just the styles**

Replace `restyle` with:

```rust
    /// Re-evaluate the filters and rebuild what the view shows.
    ///
    /// When nothing is hidden the buffer is the whole document and the gutter
    /// numbers itself. As soon as a line is hidden the buffer holds only the
    /// visible lines, so the gutter must be told each row's *source* number or
    /// it would renumber 1..N and the line numbers would be lies.
    ///
    /// Rebuilding costs a clone of the visible lines. That is bounded but not
    /// free on a large log; if it bites, the fix is to keep both buffers and
    /// swap between them rather than to rebuild.
    fn refresh_view(&mut self) {
        self.document.evaluate(&self.filters);

        let hiding = self.document.visible().len() < self.document.lines().len();
        let lines = self.document.visible_lines();
        let styles = self.document.visible_styles(&self.filters);
        let numbers: Vec<usize> = if hiding {
            self.document.visible().to_vec()
        } else {
            Vec::new()
        };

        for widget in &mut self.widgets {
            if let AppWidget::FileView(view) = widget {
                if hiding {
                    view.show_lines(lines.clone());
                }
                view.set_line_numbers(numbers.clone());
                view.set_line_styles(styles.clone());
            }
        }
    }
```

Rename every existing call of `restyle` to `refresh_view`.

- [ ] **Step 6: Give `FileView` a way to replace its contents**

`load` and `preview` read from disk; this replaces the buffer from memory. In
`src/widgets/fileview.rs`, beside `set_line_styles`:

```rust
    /// Replace the buffer's contents without touching the file.
    ///
    /// Used when filtering hides lines: the view then holds a subset of the
    /// document rather than the file as read. The filename is left alone, since
    /// it still describes where these lines came from.
    pub fn show_lines(&mut self, lines: Vec<String>) {
        self.textarea = TextArea::new(lines);
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including the four new tests.

- [ ] **Step 8: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 143 passing, `0` warnings.

- [ ] **Step 9: Commit**

```bash
git add src/lib.rs src/widgets/fileview.rs
git commit -m "feat(app): hide excluded lines and keep the gutter honest

F adds an excluding filter, whose matches leave the view entirely. The
buffer is then a subset of the file, so each row's source number is
supplied to the gutter - otherwise it would renumber 1..N and every line
number on screen would be wrong.

The gutter is only overridden while something is actually hidden, so an
unfiltered file still numbers itself."
```

---

### Task 4: The Ctrl-H toggle and the cursor round trip

The workflow this whole design exists for.

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `Document::set_mode`, `nearest_visible`, `visible_position`,
  `source_at`, `Mode`.
- Produces: `App::toggle_hiding`, and cursor preservation across a rebuild.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`:

```rust
    /// The cursor's source line, derived from where it sits in the view.
    fn cursor_source(app: &App) -> usize {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => {
                    let row = view.textarea.cursor().0;
                    Some(app.document.source_at(row).unwrap_or(row))
                }
                AppWidget::FileNav(_) => None,
            })
            .expect("no file view")
    }

    fn move_cursor_to_visible_row(app: &mut App, row: usize) {
        for widget in &mut app.widgets {
            if let AppWidget::FileView(view) = widget {
                view.textarea.move_cursor(CursorMove::Jump(row as u16, 0));
            }
        }
    }

    #[test]
    fn h_hides_lines_that_match_no_filter() {
        let mut app = app_over_file("toggle_hide", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert_eq!(view_lines(&app).len(), 3, "nothing hidden yet");

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(view_lines(&app), vec!["beta".to_string()]);
    }

    #[test]
    fn ctrl_h_toggles_the_same_way() {
        let mut app = app_over_file("toggle_ctrl_h", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        app.handle_event(event::Event::Key(event::KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();

        assert_eq!(view_lines(&app), vec!["beta".to_string()]);
    }

    /// The workflow: filter, hide, scroll to a match, show everything again,
    /// and land on that exact line with its context around it.
    #[test]
    fn the_round_trip_returns_to_the_chosen_line() {
        let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("round_trip", &body);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "line 1[0-9]");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));
        // Visible rows are now source lines 10..=19; pick the third of them.
        move_cursor_to_visible_row(&mut app, 2);
        assert_eq!(cursor_source(&app), 12);

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(cursor_source(&app), 12, "did not return to the same line");
        assert_eq!(view_lines(&app).len(), 20, "context did not come back");
    }

    /// Toggling into hidden mode from a line that is not a match snaps forward
    /// to the next one, and toggling back lands on that.
    #[test]
    fn hiding_from_an_unmatched_line_snaps_to_the_next_match() {
        let mut app = app_over_file("snap", "alpha\nbeta\ngamma\nbeta two\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        move_cursor_to_visible_row(&mut app, 2); // gamma, unmatched
        assert_eq!(cursor_source(&app), 2);

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(cursor_source(&app), 3, "did not snap to the next match");
    }

    /// Hiding with nothing to show must not panic or lose the cursor.
    #[test]
    fn hiding_with_no_matches_is_survivable() {
        let mut app = app_over_file("no_matches", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "zzz");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));
        assert!(view_lines(&app).iter().all(String::is_empty) || view_lines(&app).is_empty());

        key(&mut app, KeyCode::Char('H'));
        assert_eq!(view_lines(&app).len(), 2, "did not come back");
    }

    #[test]
    fn the_status_line_shows_a_funnel_while_hiding() {
        let mut app = app_over_file("funnel", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert!(!status_line(&mut app).contains('▼'));

        key(&mut app, KeyCode::Char('H'));

        assert!(
            status_line(&mut app).contains('▼'),
            "no indication that lines are hidden: {}",
            status_line(&mut app)
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib h_hides_lines_that_match_no_filter`
Expected: FAIL — nothing is hidden, because `H` is unbound.

- [ ] **Step 3: Preserve the cursor across a rebuild**

The cursor must be a *source* index across the rebuild. In `refresh_view`,
capture it before rebuilding and restore it after:

```rust
    fn refresh_view(&mut self) {
        // The cursor is a source line index for the duration of the rebuild:
        // its row in the view is only meaningful against the old visible list.
        let cursor_source = self.cursor_source();
        self.document.evaluate(&self.filters);
        let cursor_source = self
            .document
            .nearest_visible(cursor_source)
            .unwrap_or(cursor_source);

        // ... existing rebuild ...

        self.restore_cursor(cursor_source);
    }

    /// The source line the cursor is on, mapped through the *current* visible
    /// list before it is rebuilt.
    fn cursor_source(&self) -> usize {
        self.widgets
            .iter()
            .find_map(|widget| match widget {
                AppWidget::FileView(view) => {
                    let row = view.textarea.cursor().0;
                    Some(self.document.source_at(row).unwrap_or(row))
                }
                AppWidget::FileNav(_) => None,
            })
            .unwrap_or(0)
    }

    /// Put the cursor back on `source`, wherever that line now sits.
    fn restore_cursor(&mut self, source: usize) {
        let Some(row) = self.document.visible_position(source) else {
            return;
        };
        for widget in &mut self.widgets {
            if let AppWidget::FileView(view) = widget {
                view.textarea
                    .move_cursor(CursorMove::Jump(row as u16, 0));
            }
        }
    }
```

Import `CursorMove` from `tui_textarea` in `src/lib.rs`.

- [ ] **Step 4: Bind the toggle**

In `handle_event`'s app-wide match:

```rust
                KeyCode::Char('H')
                    if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.toggle_hiding();
                    return Ok(());
                }
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.toggle_hiding();
                    return Ok(());
                }
```

Both are needed: terminals that fold Ctrl-H into Backspace never deliver the
second, and the prompt already uses Backspace, so it cannot be repurposed.

```rust
    /// Flip between dimming unmatched lines and hiding them.
    fn toggle_hiding(&mut self) {
        let mode = match self.document.mode() {
            Mode::Dimmed => Mode::FilteredOnly,
            Mode::FilteredOnly => Mode::Dimmed,
        };
        self.document.set_mode(mode);
        self.refresh_view();
    }
```

- [ ] **Step 5: Add the funnel to the status line**

In `status_text`, prefix the summary when hiding:

```rust
        let funnel = if self.document.mode() == Mode::FilteredOnly {
            "▼ "
        } else {
            ""
        };
```

and include it in both the disabled and enabled branches, so the mode is
visible either way.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including all six new tests.

- [ ] **Step 7: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
cargo clippy --lib 2>&1 | grep -c "^warning:"
```
Expected: 149 passing, `0` test warnings, and clippy reporting only the
pre-existing `AppWidget` variant-size warning.

- [ ] **Step 8: Commit**

```bash
git add src/lib.rs
git commit -m "feat(app): toggle hiding with Ctrl-H or H

The workflow this design exists for: filter, hide, find the line, show
everything again and land on that exact line with its context.

It is exact by construction because the cursor is carried across the
rebuild as a source line index, never as a row in the visible list.
Toggling into hidden mode from a line that is not a match snaps forward
to the next one, which is what the user was navigating towards.

Both keys are bound: terminals that fold Ctrl-H into Backspace never
deliver it, and Backspace is already the prompt's delete key."
```

---

### Task 5: Documentation

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document the keys**

Add to the global keybindings table, after the `f` row:

```markdown
| `F` | Add an exclude filter — its matches leave the view entirely |
| `Ctrl-H` / `H` | Toggle between dimming unmatched lines and hiding them |
```

Verify each against `App::handle_event` before writing it — a documented key
that does not exist is worse than an undocumented one, and this project has
already shipped one wrong key description.

- [ ] **Step 2: Explain what hiding means**

Extend the filters paragraph:

```markdown
Excluding filters (`F`) are different: their matches are removed from view
outright, in both modes. `Ctrl-H` (or `H`, for terminals that fold Ctrl-H into
Backspace) toggles the remaining lines between dimmed and hidden. The gutter
keeps the original line numbers either way, so a gap in the numbering is how
you tell something was left out. Toggling back returns you to the exact line
you were on, which is the point: the hidden view is for finding a line, not
for living in.
```

- [ ] **Step 3: Verify**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 149 passing, `0` warnings.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document exclude filters and the hide toggle"
```

---

## Phase 2b completion criteria

- `cargo test` reports **149 passing**, 0 failed, in debug *and* `--release`;
  the 123 pre-existing tests are unmodified.
- `cargo test 2>&1 | grep -ci "^warning"` prints `0`.
- `cargo clippy --lib` reports only the pre-existing `AppWidget` warning.
- The round trip is exact: a test drives filter → hide → move → show and
  asserts the cursor is on the same source line with full context restored.
- Manual check in a real terminal: `cargo run -- Cargo.lock`, `f`, `name`,
  Enter, then `H`. Only matching lines remain, the gutter shows their original
  numbers with gaps, and `H` again restores the file with the cursor where you
  left it.

Phase 2c (the filter pane) begins only once all of the above hold.
