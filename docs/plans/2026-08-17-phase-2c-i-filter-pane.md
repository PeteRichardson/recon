# Phase 2c-i — the filter pane — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A visible list of the filters you have defined, in the lower half of
the left-hand column, where you can select, toggle and delete them — and where
toggling one no longer makes the file view lurch.

**Architecture:** A third `AppWidget` variant, `FilterList`, renders the filter
set and owns its selection. The left column splits horizontally: file navigator
on top, filters beneath, the filter pane sized to its contents and collapsing to
nothing when no filters exist. `Tab` cycles the three panes, skipping the filter
pane while it is collapsed. Because toggling a filter changes which lines are
visible, `refresh_view` gains scroll preservation: the cursor's screen row is
captured before the rebuild and restored after, so lines appear and disappear
around a fixed point instead of the view re-anchoring.

**Tech Stack:** Rust 2021, ratatui 0.30.2, vendored tui-textarea-2 0.12.1,
crossterm 0.29, regex 1.

**Spec:** `docs/specs/2026-08-15-filter-based-viewing-design.md` (Phase 2c-i)

## Global Constraints

- **Editing, reordering and recolouring are out of scope.** They are 2c-ii.
  This phase selects, toggles and deletes. A filter's pattern cannot be changed
  here, and the order is the order they were added.
- **Deleting invalidates every cached verdict.** `Verdict::Included(index)`
  stores a *positional* index into the filter list, so removing a filter shifts
  every later filter's index and any surviving verdict would colour lines with
  the wrong filter's style. Every mutation of the set must be followed by a full
  re-evaluation — never a partial patch of the verdict cache.
- **`!` must restore the enabled state it captured**, not enable everything.
  Today `set_all_enabled(true)` turns every filter on; that is indistinguishable
  from correct only because nothing can disable a filter individually. This
  phase makes that possible, so the bug becomes reachable.
- 162 tests pass today; all must still pass. Any pre-existing test that appears
  to need editing is a signal something is wrong — stop and report.
- **Test output must be pristine.** Verify with `cargo test 2>&1 | grep -ci "^warning"`
  printing `0`, in debug *and* `--release`. Never verify with
  `grep -E "^test result"`, which filters warnings out by construction and has
  hidden a real warning on this project twice.
- **Guard every new global key against modifiers.** Unshifted keys use
  `key.modifiers.is_empty()`; a shifted letter requires only that CONTROL and
  ALT are absent, never `== SHIFT`, because not every terminal reports SHIFT.
  An unguarded `KeyCode::Char` arm swallowed `Ctrl-f` in an earlier phase.
- **Fixture directory names must be unique.** The helpers panic on a duplicate;
  do not defeat that guard. A collision previously produced a release-only flake.
- TDD throughout: failing test first, observe it fail, then implement.

## File Structure

| File | Responsibility |
|---|---|
| `src/filter.rs` | `remove`, `set_enabled`, and `!` capturing/restoring per-filter state |
| `src/widgets/filterlist.rs` (new) | The pane: renders the filter set, owns its selection |
| `src/widgets/mod.rs` | `AppWidget::FilterList` and its dispatch |
| `src/lib.rs` | Stacked layout, three-way focus, scroll preservation, key routing |
| `README.md` | The pane and its keys |

`filterlist.rs` sits beside `filenav.rs` and `fileview.rs` because it is a
widget with the same shape: it renders, it owns a selection, and it returns
actions for `App` to perform.

---

### Task 1: Mutating the filter set

The model changes, with no pane and no rendering, so the verdict-invalidation
contract can be tested without a terminal.

**Files:**
- Modify: `src/filter.rs`

**Interfaces:**
- Consumes: the existing `FilterSet`, `Filter`, `Verdict`.
- Produces:
  - `pub fn remove(&mut self, index: usize) -> bool`
  - `pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool`
  - `pub fn toggle_enabled(&mut self, index: usize) -> Option<bool>`
  - `pub fn disable_all_remembering(&mut self)` and
    `pub fn restore_remembered(&mut self)`, replacing the `!` behaviour
  - `pub fn has_remembered(&self) -> bool`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/filter.rs`. `set_with` and `set_excluding` already
exist — reuse them.

```rust
    #[test]
    fn removing_a_filter_drops_it() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.remove(0));

        assert_eq!(set.len(), 1);
        assert_eq!(set.verdict("bar line"), Verdict::Included(0));
    }

    /// Indices are positional, so removing a filter renumbers the ones after
    /// it. Any verdict cached against the old numbering is now wrong, which is
    /// why callers must re-evaluate rather than patch.
    #[test]
    fn removing_a_filter_renumbers_the_rest() {
        let mut set = set_with(&["foo", "bar"]);
        assert_eq!(set.verdict("bar line"), Verdict::Included(1));

        set.remove(0);

        assert_eq!(set.verdict("bar line"), Verdict::Included(0));
    }

    #[test]
    fn removing_out_of_range_reports_failure_and_changes_nothing() {
        let mut set = set_with(&["foo"]);

        assert!(!set.remove(5));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn a_single_filter_can_be_disabled() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.set_enabled(0, false));

        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
        assert_eq!(set.verdict("bar line"), Verdict::Included(1));
    }

    #[test]
    fn toggle_flips_one_filter_and_reports_its_new_state() {
        let mut set = set_with(&["foo"]);

        assert_eq!(set.toggle_enabled(0), Some(false), "was enabled, so is now disabled");
        assert_eq!(set.toggle_enabled(0), Some(true), "and back on");
        assert_eq!(set.toggle_enabled(9), None, "no such filter");
    }

    /// `!` must restore what was enabled before, not enable everything —
    /// otherwise it silently switches on filters the user turned off.
    #[test]
    fn disabling_all_remembers_the_previous_state() {
        let mut set = set_with(&["foo", "bar", "baz"]);
        set.set_enabled(1, false);

        set.disable_all_remembering();
        assert!(!set.any_enabled());

        set.restore_remembered();

        assert!(set.filters()[0].enabled);
        assert!(!set.filters()[1].enabled, "a filter the user had off came back on");
        assert!(set.filters()[2].enabled);
    }

    #[test]
    fn has_remembered_reports_whether_a_restore_is_pending() {
        let mut set = set_with(&["foo"]);
        assert!(!set.has_remembered());

        set.disable_all_remembering();
        assert!(set.has_remembered());

        set.restore_remembered();
        assert!(!set.has_remembered());
    }

    /// Removing a filter while a restore is pending must not resurrect it or
    /// misapply the remembered flags to the wrong filters.
    #[test]
    fn removing_while_disabled_does_not_corrupt_the_restore() {
        let mut set = set_with(&["foo", "bar"]);
        set.disable_all_remembering();

        set.remove(0);
        set.restore_remembered();

        assert_eq!(set.len(), 1);
        assert!(set.filters()[0].enabled);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filter::`
Expected: FAIL to compile — `no method named 'remove' found`.

- [ ] **Step 3: Add the mutators**

In `src/filter.rs`, add a field to `FilterSet` recording the state captured by
`!`:

```rust
    /// Enabled flags captured by `disable_all_remembering`, awaiting a restore.
    ///
    /// Held separately from the filters so that a filter removed in the
    /// meantime simply drops out of the restore rather than resurrecting.
    remembered: Option<Vec<bool>>,
```

and beside `set_all_enabled`:

```rust
    /// Remove the filter at `index`, reporting whether it existed.
    ///
    /// Indices are positional, so this renumbers every later filter. Any
    /// cached `Verdict::Included` is invalid afterwards — callers must
    /// re-evaluate rather than patch.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.filters.len() {
            return false;
        }
        self.filters.remove(index);
        if let Some(remembered) = self.remembered.as_mut() {
            if index < remembered.len() {
                remembered.remove(index);
            }
        }
        true
    }

    /// Enable or disable one filter, reporting whether it existed.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool {
        match self.filters.get_mut(index) {
            Some(filter) => {
                filter.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Flip one filter, returning its new state, or `None` if there is no
    /// such filter.
    ///
    /// Distinguishing the two matters: a caller cannot otherwise tell "turned
    /// off" from "that row is gone", and the pane's selection can lag a
    /// deletion by a frame.
    pub fn toggle_enabled(&mut self, index: usize) -> Option<bool> {
        let filter = self.filters.get_mut(index)?;
        filter.enabled = !filter.enabled;
        Some(filter.enabled)
    }

    /// Disable every filter, recording which were enabled.
    ///
    /// A second call before a restore is ignored: the flags at that point are
    /// the ones this method just cleared, so capturing them again would
    /// overwrite the real state with all-disabled and lose it for good.
    pub fn disable_all_remembering(&mut self) {
        if self.remembered.is_some() {
            return;
        }
        self.remembered = Some(self.filters.iter().map(|f| f.enabled).collect());
        for filter in &mut self.filters {
            filter.enabled = false;
        }
    }

    /// Put back exactly the state `disable_all_remembering` captured.
    ///
    /// Enabling everything instead would silently switch on filters the user
    /// had deliberately turned off.
    pub fn restore_remembered(&mut self) {
        let Some(remembered) = self.remembered.take() else {
            return;
        };
        for (filter, was_enabled) in self.filters.iter_mut().zip(remembered) {
            filter.enabled = was_enabled;
        }
    }

    pub fn has_remembered(&self) -> bool {
        self.remembered.is_some()
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib filter::`
Expected: PASS, 8 new tests.

- [ ] **Step 5: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 170 passing, `0` warnings.

- [ ] **Step 6: Commit**

```bash
git add src/filter.rs
git commit -m "feat(filter): remove and individually toggle filters

Adds what a filter pane needs to mutate the set, and replaces the
all-or-nothing ! behaviour with one that captures the per-filter enabled
state and restores exactly it. Enabling everything was indistinguishable
from correct only while nothing could disable a filter individually.

The remembered flags are held separately from the filters, so a filter
removed while a restore is pending drops out of it rather than coming
back to life."
```

---

### Task 2: Route `!` through the remembering pair

Small, and separated from Task 1 so the behaviour change is visible on its own
rather than buried in a pane.

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `disable_all_remembering`, `restore_remembered`, `has_remembered`.
- Produces: no new API; the `!` arm changes behaviour.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/lib.rs`. Helpers `app_over_file`, `key`, `typed`
already exist.

```rust
    /// `!` must put back what the user had, not switch everything on.
    #[test]
    fn bang_restores_the_per_filter_state_it_captured() {
        let mut app = app_over_file("bang_restore", "alpha\nbeta\n");
        for pattern in ["alpha", "beta"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        app.filters.set_enabled(1, false);

        key(&mut app, KeyCode::Char('!'));
        assert!(!app.filters.any_enabled());

        key(&mut app, KeyCode::Char('!'));

        assert!(app.filters.filters()[0].enabled);
        assert!(
            !app.filters.filters()[1].enabled,
            "a filter the user had disabled was switched back on"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib bang_restores_the_per_filter_state_it_captured`
Expected: FAIL — the second filter comes back enabled, because `set_all_enabled(true)` enables everything.

- [ ] **Step 3: Change the `!` arm**

Replace the body of the `KeyCode::Char('!')` arm's toggle with:

```rust
                    // Restore what was captured rather than enabling
                    // everything, so filters the user turned off individually
                    // stay off.
                    if self.filters.has_remembered() {
                        self.filters.restore_remembered();
                    } else {
                        self.filters.disable_all_remembering();
                    }
                    self.refresh_view();
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS. The pre-existing `bang_disables_every_filter_and_restores_them`
must still pass — it enables everything before pressing `!`, so a faithful
restore and an enable-all are indistinguishable for it.

- [ ] **Step 5: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 171 passing, `0` warnings.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs
git commit -m "fix(app): make ! restore the state it captured

Enabling everything was indistinguishable from correct only while no
filter could be disabled individually. The pane makes that possible, so
! now puts back exactly what it turned off."
```

---

### Task 3: Preserve the scroll across a rebuild

Done before the pane exists, so the pane's toggles inherit correct behaviour
rather than needing it retrofitted.

**Files:**
- Modify: `vendor/tui-textarea-2/src/textarea.rs` (one accessor)
- Modify: `vendor/tui-textarea-2/PATCH.md`
- Modify: `vendor/tui-textarea-2/upstream.patch` (regenerate)
- Modify: `src/widgets/fileview.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `TextArea::scroll`, which is already public.
- Produces:
  - `TextArea::scroll_top(&self) -> (u16, u16)` — a new public accessor on the
    vendored fork
  - `FileView::cursor_screen_row(&self) -> u16`
  - `FileView::scroll_cursor_to_row(&mut self, row: u16)`

**A fork change is authorised for this task, and only this one.** The fork
already tracks the viewport, but the field is `pub(crate)`, so there is no way
to read the scroll offset from outside the crate. Everything else needed
(`scroll`, `cursor`) is public. Working around the gap inside `recon` would mean
inferring the offset from render-time state, which is fragile and would have to
be undone later. Exposing the existing value is the same shape of minimal
presentation accessor the fork already carries, and it belongs beside them.

The patch stays confined to per-line presentation and viewport *reporting*. If
this turns into changing how scrolling works, stop and report — that is the
tripwire the fork was set up with.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`:

```rust
    /// Toggling a filter changes the visible set, so the buffer is rebuilt —
    /// but the line under the cursor must stay on the same screen row rather
    /// than the view re-anchoring beneath it.
    #[test]
    fn toggling_a_filter_leaves_the_cursor_on_the_same_screen_row() {
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("scroll_hold", &body);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "line 1[0-9][0-9]");
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        // Put the cursor well down the file, then note where it sits on screen.
        for _ in 0..120 {
            key(&mut app, KeyCode::Tab);
            key(&mut app, KeyCode::Char('j'));
            key(&mut app, KeyCode::Tab);
        }
        draw(&mut app);
        let before_row = cursor_screen_row(&app);
        let before_source = cursor_source(&app);

        key(&mut app, KeyCode::Char('!'));
        draw(&mut app);

        assert_eq!(
            cursor_screen_row(&app),
            before_row,
            "the view re-anchored instead of holding the line in place"
        );
        assert_eq!(cursor_source(&app), before_source, "the cursor changed line");
    }
```

and the helper it needs, beside `cursor_source`:

```rust
    fn cursor_screen_row(app: &App) -> u16 {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.cursor_screen_row()),
                _ => None,
            })
            .expect("no file view")
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib toggling_a_filter_leaves_the_cursor_on_the_same_screen_row`
Expected: FAIL — the rows differ, because `set_lines` resets the viewport and
the cursor is re-anchored to the pane's last row.

- [ ] **Step 3: Expose the scroll offset on the fork**

In `vendor/tui-textarea-2/src/textarea.rs`, beside the other public accessors:

```rust
    /// The viewport's top-left position: the first visible row and column.
    ///
    /// Reported so a caller can tell where the cursor sits *on screen*, not
    /// just in the buffer — needed to hold a line in place across a
    /// `set_lines`, which resets the viewport.
    pub fn scroll_top(&self) -> (u16, u16) {
        self.viewport.scroll_top()
    }
```

Add a section to `vendor/tui-textarea-2/PATCH.md` recording it alongside the
existing changes, then regenerate `upstream.patch` exactly as that file's own
instructions describe. Confirm the regenerated patch still touches only the
files it did before, plus this one hunk.

- [ ] **Step 4: Expose the screen row on `FileView`**

In `src/widgets/fileview.rs`:

```rust
    /// Which row of the pane the cursor is currently drawn on.
    ///
    /// Used to hold a line in place across a rebuild: `set_lines` resets the
    /// viewport, so without this the cursor re-anchors to the pane's last row
    /// and the view lurches whenever a filter changes.
    pub fn cursor_screen_row(&self) -> u16 {
        let (top, _) = self.textarea.scroll_top();
        self.textarea.cursor().0.saturating_sub(top as usize) as u16
    }

    /// Scroll so the cursor sits on `row` of the pane, as far as the buffer
    /// allows near its start or end.
    pub fn scroll_cursor_to_row(&mut self, row: u16) {
        let cursor = self.textarea.cursor().0;
        let desired_top = cursor.saturating_sub(row as usize);
        let (current_top, _) = self.textarea.scroll_top();
        let delta = desired_top as i64 - current_top as i64;
        if delta != 0 {
            self.textarea
                .scroll((delta.clamp(i16::MIN as i64, i16::MAX as i64) as i16, 0));
        }
    }
```

- [ ] **Step 5: Capture and restore the row in `refresh_view`**

In `App::refresh_view`, alongside the existing cursor-source capture:

```rust
        let screen_row = self.file_view_screen_row();
```

and after the rebuild, once the cursor has been placed:

```rust
        self.restore_screen_row(screen_row);
```

with:

```rust
    fn file_view_screen_row(&self) -> u16 {
        self.widgets
            .iter()
            .find_map(|widget| match widget {
                AppWidget::FileView(view) => Some(view.cursor_screen_row()),
                AppWidget::FileNav(_) => None,
            })
            .unwrap_or(0)
    }

    /// Put the cursor back on the screen row it occupied before the rebuild,
    /// so lines appear and disappear around a fixed point.
    fn restore_screen_row(&mut self, row: u16) {
        for widget in &mut self.widgets {
            if let AppWidget::FileView(view) = widget {
                view.scroll_cursor_to_row(row);
            }
        }
    }
```

Note the match arms will need a `FilterList` arm once Task 4 adds it — use a
wildcard only if the file already does so; otherwise extend it in Task 4.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including the new test and every pre-existing one.

- [ ] **Step 7: Verify the whole suite in both profiles**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test --release 2>&1 | grep -E "^test result"
cargo test -p tui-textarea-2 --test line_presentation 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 172 passing in both profiles, the fork's own tests still passing, `0` warnings.

- [ ] **Step 8: Commit**

```bash
git add src/lib.rs src/widgets/fileview.rs vendor/tui-textarea-2
git commit -m "fix(app): hold the cursor's screen row across a rebuild

Rebuilding the buffer resets the viewport, so every filter change
re-anchored the view and dropped the cursor line to the pane's last row.
With a filter pane that is the dominant interaction, so the row is now
captured before the rebuild and restored after: lines appear and
disappear around a fixed point."
```

---

### Task 4: The filter pane widget

Renders the set and owns its selection. No layout or key routing yet, so it can
be tested against a `Buffer` in isolation.

**Files:**
- Create: `src/widgets/filterlist.rs`
- Modify: `src/widgets/mod.rs` (declare the module)

**Interfaces:**
- Consumes: `FilterSet`, `Filter`, `Sense`.
- Produces:
  - `pub struct FilterList { pub state: ListState, pub active: bool }`
  - `pub fn selected(&self) -> Option<usize>`
  - `pub fn select_next(&mut self, len: usize)` / `select_previous(&mut self, len: usize)`
  - `pub fn clamp_selection(&mut self, len: usize)`
  - `pub fn preferred_height(&self, len: usize) -> u16`
  - `pub fn preferred_width(&self, filters: &FilterSet) -> u16`
  - `pub fn render(&mut self, filters: &FilterSet, area: Rect, buf: &mut Buffer)`

It renders from a borrowed `FilterSet` rather than owning one, because `App`
owns the set and the pane must never hold a stale copy.

- [ ] **Step 1: Write the failing tests**

Create `src/widgets/filterlist.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn set_of(includes: &[&str], excludes: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in includes {
            set.add(pattern).expect("valid pattern");
        }
        for pattern in excludes {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    fn rendered(list: &mut FilterList, filters: &FilterSet, width: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, 8);
        let mut buf = Buffer::empty(area);
        list.render(filters, area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn each_filter_gets_a_row_showing_its_pattern() {
        let filters = set_of(&["foo", "bar"], &[]);
        let mut list = FilterList::default();

        let rows = rendered(&mut list, &filters, 30).join("\n");

        assert!(rows.contains("foo"), "pattern missing:\n{rows}");
        assert!(rows.contains("bar"), "pattern missing:\n{rows}");
    }

    /// A disabled filter must be distinguishable at a glance from an enabled
    /// one, since that is the pane's main job.
    #[test]
    fn enabled_and_disabled_filters_are_marked_differently() {
        let mut filters = set_of(&["foo", "bar"], &[]);
        filters.set_enabled(1, false);
        let mut list = FilterList::default();

        let rows = rendered(&mut list, &filters, 30);
        let foo = rows.iter().find(|r| r.contains("foo")).expect("foo row");
        let bar = rows.iter().find(|r| r.contains("bar")).expect("bar row");

        assert_ne!(
            foo.replace("foo", ""),
            bar.replace("bar", ""),
            "enabled and disabled rows are indistinguishable"
        );
    }

    /// Excluding filters carry no colour, so the pane must say what they are
    /// some other way.
    #[test]
    fn excluding_filters_are_marked_as_excluding() {
        let filters = set_of(&["foo"], &["noise"]);
        let mut list = FilterList::default();

        let rows = rendered(&mut list, &filters, 30);
        let noise = rows.iter().find(|r| r.contains("noise")).expect("noise row");
        let foo = rows.iter().find(|r| r.contains("foo")).expect("foo row");

        assert_ne!(
            noise.replace("noise", ""),
            foo.replace("foo", ""),
            "an excluding filter looks like an including one"
        );
    }

    #[test]
    fn an_including_filter_shows_its_colour() {
        let filters = set_of(&["foo"], &[]);
        let mut list = FilterList::default();
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        let expected = filters.filters()[0].style.fg;
        assert!(
            (0..area.width).any(|x| buf[(x, 1)].style().fg == expected),
            "the filter's colour is not shown anywhere on its row"
        );
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut list = FilterList::default();

        list.select_next(2);
        assert_eq!(list.selected(), Some(1));
        list.select_next(2);
        assert_eq!(list.selected(), Some(1), "selection ran past the end");
        list.select_previous(2);
        assert_eq!(list.selected(), Some(0));
    }

    /// Deleting the last filter must not leave the selection pointing past the
    /// end of the list.
    #[test]
    fn clamping_pulls_the_selection_back_into_range() {
        let mut list = FilterList::default();
        list.select_next(3);
        list.select_next(3);
        assert_eq!(list.selected(), Some(2));

        list.clamp_selection(1);

        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn an_empty_set_has_no_selection_and_no_height() {
        let mut list = FilterList::default();

        list.clamp_selection(0);

        assert_eq!(list.selected(), None);
        assert_eq!(list.preferred_height(0), 0, "an empty pane must take no rows");
    }

    #[test]
    fn the_pane_grows_with_the_number_of_filters() {
        let list = FilterList::default();

        assert!(list.preferred_height(3) > list.preferred_height(1));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filterlist::`
Expected: FAIL to compile — `cannot find type FilterList in this scope`.

- [ ] **Step 3: Implement the widget**

Put this above the test module in `src/widgets/filterlist.rs`:

```rust
//! The pane listing the filters that have been defined.
//!
//! It renders from a borrowed `FilterSet` rather than owning one: `App` owns
//! the set, and a copy here could go stale the moment a filter changed.

use crate::filter::{FilterSet, Sense};
use crossterm::event::KeyCode;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, StatefulWidget};

/// Marks the row the cursor is on, matching the navigator pane.
const SELECTION: &str = ">>";

/// Rows of chrome the pane needs on top of one row per filter.
const BORDERS: u16 = 2;

#[derive(Debug, Default)]
pub struct FilterList {
    pub state: ListState,
    pub active: bool,
}

impl FilterList {
    pub fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub fn select_next(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let next = self.state.selected().map_or(0, |i| (i + 1).min(len - 1));
        self.state.select(Some(next));
    }

    pub fn select_previous(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let previous = self.state.selected().map_or(0, |i| i.saturating_sub(1));
        self.state.select(Some(previous));
    }

    /// Pull the selection back into range after the set has shrunk, and drop
    /// it entirely when nothing is left.
    pub fn clamp_selection(&mut self, len: usize) {
        match len {
            0 => self.state.select(None),
            _ => {
                let index = self.state.selected().unwrap_or(0).min(len - 1);
                self.state.select(Some(index));
            }
        }
    }

    /// Rows this pane wants: one per filter plus its borders, or none at all
    /// when there are no filters, so it costs nothing to a user who never
    /// defines one.
    pub fn preferred_height(&self, len: usize) -> u16 {
        match len {
            0 => 0,
            n => u16::try_from(n).unwrap_or(u16::MAX).saturating_add(BORDERS),
        }
    }

    /// Columns needed for the widest row.
    pub fn preferred_width(&self, filters: &FilterSet) -> u16 {
        let longest = (0..filters.len())
            .map(|index| Self::row_text(filters, index).chars().count())
            .max()
            .unwrap_or(0);
        u16::try_from(longest + SELECTION.len() + BORDERS as usize).unwrap_or(u16::MAX)
    }

    /// One filter's row: its number, whether it is on, which way it filters,
    /// and its pattern.
    ///
    /// The sense is spelled out because excluding filters carry no colour —
    /// nothing else on the row would distinguish them.
    fn row_text(filters: &FilterSet, index: usize) -> String {
        let Some(filter) = filters.filters().get(index) else {
            return String::new();
        };
        let mark = if filter.enabled { 'x' } else { ' ' };
        let sense = match filter.sense {
            Sense::Include => "inc",
            Sense::Exclude => "exc",
        };
        format!("{}[{}] {} {}", index + 1, mark, sense, filter.pattern.as_str())
    }

    pub fn render(&mut self, filters: &FilterSet, area: Rect, buf: &mut Buffer) {
        let items: Vec<ListItem> = (0..filters.len())
            .map(|index| {
                let filter = &filters.filters()[index];
                let mut style = match filter.sense {
                    // An including filter wears its own colour, so the pane
                    // and the file view agree at a glance.
                    Sense::Include => filter.style,
                    Sense::Exclude => Style::default().fg(Color::DarkGray),
                };
                if !filter.enabled {
                    style = style.add_modifier(Modifier::DIM);
                }
                ListItem::new(Self::row_text(filters, index)).style(style)
            })
            .collect();

        let mut highlight = Style::new().add_modifier(Modifier::REVERSED);
        if self.active {
            highlight = highlight.fg(Color::Green);
        }

        let list = List::new(items)
            .block(Block::bordered().title("Filters"))
            .highlight_style(highlight)
            .highlight_symbol(SELECTION);
        StatefulWidget::render(&list, area, buf, &mut self.state);
    }
}
```

Declare it in `src/widgets/mod.rs` with `pub mod filterlist;`, beside the
existing `pub mod filenav;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib filterlist::`
Expected: PASS, 8 tests.

- [ ] **Step 5: Verify the whole suite**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 180 passing, `0` warnings.

- [ ] **Step 6: Commit**

```bash
git add src/widgets/filterlist.rs src/widgets/mod.rs
git commit -m "feat(filterlist): add the filter pane widget

Renders from a borrowed FilterSet rather than owning one, so it cannot
hold a stale copy of a set App owns.

Each row spells out the sense as well as showing the filter's colour,
because excluding filters carry no colour and would otherwise be
indistinguishable from including ones."
```

---

### Task 5: Stack the pane and cycle focus

Where it appears on screen.

**Files:**
- Modify: `src/widgets/mod.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `FilterList` from Task 4.
- Produces: `AppWidget::FilterList(FilterList)`, and a left column split
  horizontally between the navigator and the filter pane.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`:

```rust
    /// The pane costs nothing until a filter exists.
    #[test]
    fn the_filter_pane_is_absent_until_a_filter_is_defined() {
        let mut app = app_over_file("pane_absent", "alpha\n");

        assert!(!draw_to_string(&mut app).contains("Filters"));

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        assert!(draw_to_string(&mut app).contains("Filters"));
    }

    #[test]
    fn the_filter_pane_lists_the_patterns() {
        let mut app = app_over_file("pane_lists", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        assert!(draw_to_string(&mut app).contains("alpha"));
    }

    /// Tab reaches the filter pane once it exists, and skips it before then.
    #[test]
    fn tab_skips_the_filter_pane_while_it_is_collapsed() {
        let mut app = app_over_file("pane_focus", "alpha\n");
        draw(&mut app);

        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);

        assert_eq!(app.active_widget, 0, "focus did not return to the navigator");
    }

    #[test]
    fn tab_reaches_the_filter_pane_once_a_filter_exists() {
        let mut app = app_over_file("pane_focus_on", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        let mut seen = vec![app.active_widget];
        for _ in 0..3 {
            key(&mut app, KeyCode::Tab);
            seen.push(app.active_widget);
        }

        assert!(
            seen.contains(&2),
            "the filter pane never took focus: {seen:?}"
        );
    }
```

and the helper:

```rust
    fn draw_to_string(app: &mut App) -> String {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        (0..AREA.height)
            .map(|y| {
                (0..AREA.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib the_filter_pane_lists_the_patterns`
Expected: FAIL — nothing renders a pane titled "Filters".

- [ ] **Step 3: Add the variant**

In `src/widgets/mod.rs`, add `FilterList(filterlist::FilterList)` to
`AppWidget`, and extend `set_active`, `handle_events` and the `Widget` impl.
`handle_events` returns `Ok(None)` for it in this task — key routing is Task 6.

The `Widget` impl needs the filter set to render, which `AppWidget` does not
hold. Rather than give it one, have `App::render` special-case this variant and
call `FilterList::render` directly with the set it owns; leave the `AppWidget`
arm rendering nothing. Note this in a comment so the asymmetry is deliberate
rather than puzzling.

- [ ] **Step 4: Split the left column**

In `App::new`, push a third widget: `AppWidget::FilterList(FilterList::default())`.
The `assert!(self.widgets.len() == 2)` in `render` becomes `== 3`.

In `App::render`, split the left column vertically:

```rust
        use Constraint::{Length, Min};
        let filter_height = self.filter_pane_height();
        let [nav_area, filter_area] =
            Layout::vertical([Min(0), Length(filter_height)]).areas(left);
```

with:

```rust
    /// Rows the filter pane wants, which is none while no filter exists — so
    /// it costs nothing to a user who never defines one.
    fn filter_pane_height(&self) -> u16 {
        self.filter_list()
            .map(|list| list.preferred_height(self.filters.len()))
            .unwrap_or(0)
    }
```

and an accessor mirroring `nav()`. The column's width should take the wider of
the two panes' preferred widths, still capped at `MAX_NAV_WIDTH`.

- [ ] **Step 5: Skip the pane in the focus cycle while collapsed**

`Tab` currently does `(self.active_widget + 1) % self.widgets.len()`. Replace
with a step that skips the filter pane when `self.filters.is_empty()`:

```rust
    /// Move focus to the next pane, skipping the filter pane while it is
    /// collapsed — focusing a pane that is not on screen would strand the user.
    fn focus_next(&mut self) {
        let count = self.widgets.len();
        for step in 1..=count {
            let candidate = (self.active_widget + step) % count;
            let collapsed = matches!(self.widgets[candidate], AppWidget::FilterList(_))
                && self.filters.is_empty();
            if !collapsed {
                self.active_widget = candidate;
                return;
            }
        }
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS. Several pre-existing tests assert on the left column's contents
or width; they should still pass because the pane is collapsed unless a filter
exists. **If any pre-existing test fails, stop and report** rather than editing
it — it means the layout changed for a case that should have been unaffected.

- [ ] **Step 7: Verify the whole suite in both profiles**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test --release 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 184 passing in both, `0` warnings.

- [ ] **Step 8: Commit**

```bash
git add src/lib.rs src/widgets/mod.rs
git commit -m "feat(app): stack the filter pane under the navigator

The left column now splits between the file navigator and the filters,
with the filter pane sized to its contents and collapsing to nothing
when none are defined - so it costs no rows to anyone who never defines
a filter.

Tab skips it while collapsed, since focusing a pane that is not on
screen would strand the user with no visible cursor."
```

---

### Task 6: Toggle and delete from the pane

What the pane is for.

**Files:**
- Modify: `src/widgets/mod.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `FilterSet::toggle_enabled`, `remove`; `FilterList` selection.
- Produces: `Action::FiltersChanged`, returned by the filter pane so `App`
  re-evaluates and rebuilds.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`:

```rust
    fn focus_filter_pane(app: &mut App) {
        draw(app);
        for _ in 0..3 {
            if matches!(app.widgets[app.active_widget], AppWidget::FilterList(_)) {
                return;
            }
            key(app, KeyCode::Tab);
        }
        panic!("could not focus the filter pane");
    }

    fn app_with_two_filters(name: &str) -> App<'static> {
        let mut app = app_over_file(name, "alpha\nbeta\ngamma\n");
        for pattern in ["alpha", "beta"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        app
    }

    #[test]
    fn space_toggles_the_selected_filter() {
        let mut app = app_with_two_filters("pane_toggle");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char(' '));

        assert!(!app.filters.filters()[0].enabled);
    }

    /// Toggling must re-evaluate: the view is what the pane is controlling.
    #[test]
    fn toggling_a_filter_restyles_the_view() {
        let mut app = app_with_two_filters("pane_toggle_view");
        focus_filter_pane(&mut app);
        let before = view_line_styles(&app);

        key(&mut app, KeyCode::Char(' '));

        assert_ne!(before, view_line_styles(&app), "the view did not follow");
    }

    #[test]
    fn d_deletes_the_selected_filter() {
        let mut app = app_with_two_filters("pane_delete");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('d'));

        assert_eq!(app.filters.len(), 1);
    }

    /// Deleting renumbers the filters, so every cached verdict is stale.
    #[test]
    fn deleting_a_filter_re_evaluates_rather_than_patching() {
        let mut app = app_with_two_filters("pane_delete_verdicts");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('d')); // removes "alpha", "beta" becomes 0

        let styles = view_line_styles(&app);
        let beta = styles[1].expect("beta still matches a filter");
        assert_eq!(
            beta.fg,
            app.filters.filters()[0].style.fg,
            "the line is coloured with the wrong filter's style"
        );
    }

    #[test]
    fn deleting_the_last_filter_collapses_the_pane_and_moves_focus() {
        let mut app = app_over_file("pane_delete_last", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('d'));
        draw(&mut app);

        assert!(app.filters.is_empty());
        assert!(!draw_to_string(&mut app).contains("Filters"));
        assert!(
            !matches!(app.widgets[app.active_widget], AppWidget::FilterList(_)),
            "focus was left on a pane that is no longer on screen"
        );
    }

    #[test]
    fn j_and_k_move_the_filter_selection() {
        let mut app = app_with_two_filters("pane_select");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char(' '));

        assert!(app.filters.filters()[0].enabled, "toggled the wrong filter");
        assert!(!app.filters.filters()[1].enabled);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib space_toggles_the_selected_filter`
Expected: FAIL — the filter is still enabled, because the pane handles no keys.

- [ ] **Step 3: Add the action**

In `src/widgets/mod.rs`, add to `Action`:

```rust
    /// The filter set changed, so every cached verdict is stale and the view
    /// must be re-evaluated and rebuilt.
    FiltersChanged,
```

- [ ] **Step 4: Handle the pane's keys**

`FilterList` cannot mutate the set it borrows, so give it a method that reports
what the user asked for, and let `App` carry it out:

```rust
/// What a keypress in the filter pane asks `App` to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCommand {
    Toggle(usize),
    Delete(usize),
}

impl FilterList {
    /// Handle a key, reporting any change `App` must make to the filter set.
    ///
    /// Selection movement is handled here because it is the pane's own state;
    /// mutations are reported because the set belongs to `App`.
    pub fn handle_key(&mut self, code: KeyCode, len: usize) -> Option<FilterCommand> {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next(len);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous(len);
                None
            }
            KeyCode::Char(' ') => self.selected().map(FilterCommand::Toggle),
            KeyCode::Char('d') => self.selected().map(FilterCommand::Delete),
            _ => None,
        }
    }
}
```

Route it from `AppWidget::handle_events`, returning `Action::FiltersChanged`
when a command was carried out. Because the pane cannot reach the set, have
`AppWidget::handle_events` return the command and let `App` apply it — or give
`App` a small `handle_filter_key` that borrows both. Either is fine; choose the
one that keeps `App::handle_event` readable, and say which you chose and why in
your report.

Whichever you choose, applying a command must:
1. mutate the set (`toggle_enabled` or `remove`),
2. `clamp_selection` so a deletion cannot leave the selection past the end,
3. move focus off the filter pane if the set is now empty,
4. `refresh_view()` — a full re-evaluation, never a patch of the verdict cache.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including all six new tests.

- [ ] **Step 6: Verify the whole suite in both profiles**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test --release 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
cargo clippy --lib 2>&1 | grep -c "^warning:"
```
Expected: 190 passing in both, `0` test warnings, and clippy reporting only the
pre-existing `AppWidget` variant-size warning. Note that adding a third variant
may change that warning's wording — if a *new* clippy warning appears, fix it.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/widgets/mod.rs src/widgets/filterlist.rs
git commit -m "feat(app): toggle and delete filters from the pane

The pane reports what the user asked for rather than mutating the set,
which App owns - so the pane can never hold a stale copy.

Every mutation triggers a full re-evaluation. Verdicts store a
positional index into the filter list, so deleting one renumbers the
rest and any surviving verdict would colour lines with the wrong
filter's style."
```

---

### Task 7: Documentation

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document the pane and its keys**

Add a filter-pane section to the keybindings, and verify each key against
`FilterList::handle_key` and `App::handle_event` before writing it — a
documented key that does not exist is worse than an undocumented one, and this
project has already shipped one wrong key description.

```markdown
Filter pane (`src/widgets/filterlist.rs`), reached with `Tab` once a filter
exists:

| Key(s) | Action |
| --- | --- |
| `k` / `Up` | Select the previous filter |
| `j` / `Down` | Select the next filter |
| `space` | Enable or disable the selected filter |
| `d` | Delete the selected filter |
```

- [ ] **Step 2: Explain the pane**

```markdown
Each row shows the filter's number, whether it is enabled, whether it includes
or excludes, and its pattern. Including filters are drawn in their own colour so
the pane and the file view agree at a glance; excluding filters have no colour,
which is why the sense is spelled out. The pane appears only once a filter
exists and takes no space before that.

`!` disables every filter at once and puts back exactly what was enabled when
you press it again — filters you turned off individually stay off.
```

- [ ] **Step 3: Verify**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```
Expected: 190 passing, `0` warnings.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document the filter pane"
```

---

## Phase 2c-i completion criteria

- `cargo test` reports **190 passing**, 0 failed, in debug *and* `--release`;
  the 162 pre-existing tests are unmodified.
- `cargo test 2>&1 | grep -ci "^warning"` prints `0`.
- `cargo clippy --lib` reports no *new* warnings.
- Toggling a filter does not move the line under the cursor off its screen row.
- Deleting a filter re-evaluates rather than patching, so no line is coloured
  with a renumbered filter's style.
- `!` restores individually-disabled filters to their disabled state.
- Manual check in a real terminal: `cargo run -- Cargo.lock`, add two filters
  with `f`, `Tab` to the pane, `space` one off and watch the view follow without
  the text jumping, then `d` to delete and confirm the pane collapses when the
  last one goes.

Phase 2c-ii (editing, reordering, recolouring) begins only once all of the
above hold.
