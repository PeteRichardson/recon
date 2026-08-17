# Keymap — zoom, hide, and thumb-and-pinky paging — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scan long and wide files quickly. `b` hides the left column to give
the file its full width, `e` brings it back and focuses it, `z` maximises
whichever pane has focus, and `space`/`Enter` page up and down under thumb and
pinky.

**Architecture:** Hiding the left column and maximising the file view are the
*same thing*, so they are one piece of state, not two: `App::zoom:
Option<usize>` names the single widget filling the screen, or `None` for the
normal split. `b` zooms the file view, `z` zooms whatever has focus, and `e`
clears the zoom and focuses the navigator. Modelling "left column hidden" and
"a pane is zoomed" separately would let them disagree.

**Tech Stack:** Rust 2021, ratatui 0.30.2, vendored tui-textarea-2 0.12.1,
crossterm 0.29.

**Spec:** none — this is an ergonomics change, not part of the filtering design.
It lands before Phase 2c-i Task 3 so that the layout work later in that plan
accounts for zoom from the start rather than retrofitting it.

## Global Constraints

- **The zoomed pane is always the focused pane.** That single invariant is what
  stops focus being stranded on a pane that is not on screen — the same class
  of bug as focusing a collapsed pane. Any operation that zooms must also
  focus, and any operation that moves focus while zoomed must move the zoom
  with it.
- **`b` and `e` become global, displacing the file view's vim `b` (word back)
  and `e` (word end).** That was decided deliberately; `w` still moves forward
  by word. Do not try to preserve the motions by scoping the global keys — that
  was considered and rejected, because returning to the navigator *from* the
  file view is `e`'s main use.
- **Guard every new global key with `key.modifiers.is_empty()`.** `b`, `e` and
  `z` are unshifted. An unguarded `KeyCode::Char` arm swallowed `Ctrl-f` in an
  earlier phase and broke page-down; `Ctrl-b`, `Ctrl-e` and `Ctrl-z` must all
  still reach the file view.
- 179 tests pass today; all must still pass except where a task says otherwise.
  Any *other* pre-existing test that appears to need editing is a signal
  something is wrong — stop and report.
- **Test output must be pristine.** Verify with `cargo test 2>&1 | grep -ci "^warning"`
  printing `0`. Never verify with `grep -E "^test result"`, which filters
  warnings out by construction and has hidden a real warning on this project
  twice.
- **Fixture directory names must be unique**; the helpers panic on a duplicate.
- The filter pane does not exist yet. Zoom is indexed by widget, so it will
  cover that pane automatically when Phase 2c-i adds it — do not special-case
  panes by name.
- TDD throughout: failing test first, observe it fail, then implement.

## File Structure

| File | Responsibility |
|---|---|
| `src/lib.rs` | The `zoom` state, the `b`/`e`/`z` keys, and a layout that honours it |
| `src/widgets/fileview.rs` | `space` pages up instead of duplicating `Enter` |
| `README.md` | The four keys, and the two vim motions that are gone |

---

### Task 1: Zoom, hide and focus

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Produces on `App`:
  - `zoom: Option<usize>` — the widget filling the screen, or `None`
  - `fn zoom_focused(&mut self)` — `z`
  - `fn zoom_file_view(&mut self)` — `b`
  - `fn reveal_and_focus_nav(&mut self)` — `e`
  - `fn file_view_index(&self) -> usize`, `fn nav_index(&self) -> usize`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/lib.rs`. `app_over_file`, `key`, `draw` and
`draw_to_string` may or may not all exist yet — `app_over_file`, `key` and
`draw` do. Add this helper if it is not already present:

```rust
    fn rendered(app: &mut App) -> String {
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

Then:

```rust
    /// `b` gives the file its full width by hiding the left column.
    #[test]
    fn b_hides_the_left_column() {
        let mut app = app_over_file("zoom_b", "alpha\n");
        assert!(rendered(&mut app).contains("alpha"));
        let before = rendered(&mut app);

        key(&mut app, KeyCode::Char('b'));

        let after = rendered(&mut app);
        assert_ne!(before, after, "the layout did not change");
        assert!(after.contains("alpha"), "the file view went missing");
        assert!(
            !after.contains(">>"),
            "the navigator's selection marker is still on screen"
        );
    }

    #[test]
    fn b_toggles_back() {
        let mut app = app_over_file("zoom_b_back", "alpha\n");
        let before = rendered(&mut app);

        key(&mut app, KeyCode::Char('b'));
        key(&mut app, KeyCode::Char('b'));

        assert_eq!(rendered(&mut app), before, "b did not restore the split");
    }

    /// Hiding the column the cursor is in must move focus somewhere visible,
    /// or the user is left typing into a pane that is not on screen.
    #[test]
    fn b_moves_focus_out_of_the_hidden_column() {
        let mut app = app_over_file("zoom_b_focus", "alpha\n");
        assert_eq!(app.active_widget, app.nav_index(), "starts in the navigator");

        key(&mut app, KeyCode::Char('b'));

        assert_eq!(app.active_widget, app.file_view_index());
    }

    /// `e` is how you get back, so it must work from a hidden state.
    #[test]
    fn e_reveals_the_left_column_and_focuses_it() {
        let mut app = app_over_file("zoom_e", "alpha\n");
        key(&mut app, KeyCode::Char('b'));

        key(&mut app, KeyCode::Char('e'));

        assert_eq!(app.zoom, None, "the left column is still hidden");
        assert_eq!(app.active_widget, app.nav_index());
    }

    #[test]
    fn e_focuses_the_navigator_even_when_nothing_is_hidden() {
        let mut app = app_over_file("zoom_e_visible", "alpha\n");
        key(&mut app, KeyCode::Tab);
        assert_ne!(app.active_widget, app.nav_index());

        key(&mut app, KeyCode::Char('e'));

        assert_eq!(app.active_widget, app.nav_index());
    }

    /// `z` maximises whatever has focus — including the navigator, for long
    /// filenames.
    #[test]
    fn z_zooms_the_navigator_when_it_has_focus() {
        let mut app = app_over_file("zoom_z_nav", "alpha\n");

        key(&mut app, KeyCode::Char('z'));

        let after = rendered(&mut app);
        assert!(after.contains(">>"), "the navigator is not on screen");
        assert!(!after.contains("alpha"), "the file view is still showing");
    }

    /// With focus in the file view, `z` and `b` do the same thing.
    ///
    /// Both apps must be built over the *same* file: the view's title is its
    /// full path, so two fixture directories would differ on screen no matter
    /// how the zoom behaved.
    #[test]
    fn z_in_the_file_view_matches_b() {
        let path = fixture_path("zoom_same", "alpha\n");
        let config = Config { file: path.clone() };

        let mut with_z = App::new(&config);
        key(&mut with_z, KeyCode::Tab);
        key(&mut with_z, KeyCode::Char('z'));

        let mut with_b = App::new(&Config { file: path });
        key(&mut with_b, KeyCode::Char('b'));

        assert_eq!(rendered(&mut with_z), rendered(&mut with_b));
    }

    #[test]
    fn z_toggles_back() {
        let mut app = app_over_file("zoom_z_back", "alpha\n");
        let before = rendered(&mut app);

        key(&mut app, KeyCode::Char('z'));
        key(&mut app, KeyCode::Char('z'));

        assert_eq!(rendered(&mut app), before);
    }

    /// Tab while zoomed must not leave the cursor on an invisible pane: the
    /// zoom follows the focus.
    #[test]
    fn tab_while_zoomed_moves_the_zoom_with_the_focus() {
        let mut app = app_over_file("zoom_tab", "alpha\n");
        key(&mut app, KeyCode::Char('z'));

        key(&mut app, KeyCode::Tab);

        assert_eq!(
            app.zoom,
            Some(app.active_widget),
            "focus moved off the zoomed pane"
        );
        assert!(rendered(&mut app).contains("alpha"), "the focused pane is not visible");
    }

    /// The modifier guard: an earlier phase shipped a global key that swallowed
    /// a Ctrl- binding the file view needed.
    #[test]
    fn ctrl_modified_letters_still_reach_the_file_view() {
        let mut app = app_over_file("zoom_ctrl", "alpha\nbeta\n");
        key(&mut app, KeyCode::Tab);

        for code in [KeyCode::Char('b'), KeyCode::Char('e'), KeyCode::Char('z')] {
            app.handle_event(event::Event::Key(event::KeyEvent::new(
                code,
                KeyModifiers::CONTROL,
            )))
            .unwrap();
            assert_eq!(app.zoom, None, "a Ctrl- key was taken as a zoom command");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib b_hides_the_left_column`
Expected: FAIL to compile — `no field 'zoom' on type 'App'`.

- [ ] **Step 3: Add the state and the index helpers**

Add to `App`:

```rust
    /// The single widget filling the screen, or `None` for the normal split.
    ///
    /// Hiding the left column and maximising the file view are the same thing,
    /// so they share this one field. Two separate flags could disagree.
    zoom: Option<usize>,
```

initialised to `None` in `App::new`, and:

```rust
    fn nav_index(&self) -> usize {
        self.index_of(|widget| matches!(widget, AppWidget::FileNav(_)))
    }

    fn file_view_index(&self) -> usize {
        self.index_of(|widget| matches!(widget, AppWidget::FileView(_)))
    }

    fn index_of(&self, predicate: impl Fn(&AppWidget<'_>) -> bool) -> usize {
        self.widgets.iter().position(predicate).unwrap_or(0)
    }
```

- [ ] **Step 4: Add the three commands**

```rust
    /// Maximise the focused pane, or restore the split if it already is.
    fn zoom_focused(&mut self) {
        self.zoom = match self.zoom {
            Some(index) if index == self.active_widget => None,
            _ => Some(self.active_widget),
        };
    }

    /// Give the file its full width. Focus follows, because the pane the
    /// cursor was in may no longer be on screen.
    fn zoom_file_view(&mut self) {
        let view = self.file_view_index();
        self.zoom = match self.zoom {
            Some(index) if index == view => None,
            _ => Some(view),
        };
        if self.zoom == Some(view) {
            self.active_widget = view;
        }
    }

    /// Bring the left column back and put the cursor in it.
    fn reveal_and_focus_nav(&mut self) {
        self.zoom = None;
        self.active_widget = self.nav_index();
    }
```

- [ ] **Step 5: Bind the keys**

In `handle_event`'s app-wide match, beside the existing global keys:

```rust
                KeyCode::Char('b') if key.modifiers.is_empty() => {
                    self.zoom_file_view();
                    return Ok(());
                }
                KeyCode::Char('e') if key.modifiers.is_empty() => {
                    self.reveal_and_focus_nav();
                    return Ok(());
                }
                KeyCode::Char('z') if key.modifiers.is_empty() => {
                    self.zoom_focused();
                    return Ok(());
                }
```

- [ ] **Step 6: Keep the zoom with the focus**

At the end of whatever `Tab` calls to move focus, add:

```rust
        // The zoomed pane is always the focused pane, so the cursor is never
        // on a pane that is not on screen.
        if self.zoom.is_some() {
            self.zoom = Some(self.active_widget);
        }
```

- [ ] **Step 7: Honour the zoom when rendering**

In `App::render`, replace the pane split with a branch. The status/prompt row
is split off *above* this point and drawn *below* it, so **do not `return`
early** — that would skip the status line whenever a pane is zoomed:

```rust
        // A zoomed pane takes the whole pane area; the others are not drawn.
        // This deliberately falls through to the status/prompt drawing below
        // rather than returning, so the status line survives a zoom.
        if let Some(index) = self.zoom {
            for (i, widget) in self.widgets.iter_mut().enumerate() {
                widget.set_active(i == self.active_widget);
            }
            if let Some(widget) = self.widgets.get_mut(index) {
                widget.render(area, buf);
            }
        } else {
            // ... the existing nav_width / Layout::horizontal split, unchanged
        }
```

Keep `self.divider` meaningful: while zoomed there is no divider to drag, so
set it somewhere a mouse cannot hit it rather than leaving a stale value from
the last unzoomed frame. Say what you chose and why in your report.

Add a test that the status line still renders while zoomed — define a filter
first so there is status text to look for.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, 11 new tests, and **nothing pre-existing should fail**.

Note carefully: the file view's `b`/`e` word-motion tests drive `FileView`
directly rather than going through `App::handle_event`, so they keep passing
even though the motions are now unreachable in the running app. That is the
point Task 2 acts on — the arms are dead from the user's side, and both the
arms and their tests go there. If something else fails, stop and report.

- [ ] **Step 9: Commit**

```bash
git add src/lib.rs
git commit -m "feat(app): zoom a pane, hide the left column, and come back

b gives the file its full width, e brings the left column back and
focuses it, z maximises whichever pane has focus - useful in the
navigator too, when filenames are long.

Hiding the left column and maximising the file view are one state
rather than two, because two flags could disagree about whether the
column is on screen. The zoomed pane is always the focused pane, so the
cursor can never be left on a pane that is not drawn."
```

---

### Task 2: `space` pages up, and the displaced motions go

**Files:**
- Modify: `src/widgets/fileview.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: no new API; `space` changes meaning and two motions are removed.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/widgets/fileview.rs`:

```rust
    /// `space` and `Enter` page in opposite directions, so a thumb on the
    /// space bar and a pinky on Enter can scan a file in both directions.
    #[test]
    fn space_pages_up_and_enter_pages_down() {
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut view = view_of("space_pages.txt", &body);
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        (&mut view).render(area, &mut buf);

        send(&mut view, Key::Enter);
        (&mut view).render(area, &mut buf);
        let after_enter = view.textarea.cursor().0;
        assert!(after_enter > 0, "Enter did not page down");

        send(&mut view, Key::Char(' '));
        (&mut view).render(area, &mut buf);

        assert!(
            view.textarea.cursor().0 < after_enter,
            "space did not page back up"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib space_pages_up_and_enter_pages_down`
Expected: FAIL — `space` currently pages *down* alongside `Enter`, so the
cursor does not move back.

- [ ] **Step 3: Split `space` from `Enter`**

In `handle_events`, replace the combined arm:

```rust
            Input {
                key: Key::Char(' '),
                ..
            }
            | Input {
                key: Key::Enter, ..
            } => self.textarea.scroll(Scrolling::PageDown),
```

with:

```rust
            // Paired deliberately: Enter under the pinky pages down, space
            // under the thumb pages back up.
            Input {
                key: Key::Enter, ..
            } => self.textarea.scroll(Scrolling::PageDown),
            Input {
                key: Key::Char(' '),
                ..
            } => self.textarea.scroll(Scrolling::PageUp),
```

Delete the commented-out `Shift`-space block just below it, which sketched this
same idea and is now done.

- [ ] **Step 4: Remove the displaced motions and their tests**

`b` (word back) and `e` (word end) can no longer reach the file view, so their
arms are dead code and their tests assert behaviour that cannot happen.

Remove the `Key::Char('b'), ctrl: false` and `Key::Char('e'), ctrl: false` arms
from `handle_events`. **Leave the `ctrl: true` arms alone** — `Ctrl-b` is page
up and `Ctrl-e` scrolls a line, and both still reach the view.

Remove the tests that exercised them, naming each one you removed in your
report. Do not weaken any other test to compensate.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, with the removed tests gone and the new one passing.

- [ ] **Step 6: Verify the whole suite in both profiles**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test --release 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
cargo clippy --lib 2>&1 | grep -c "^warning:"
```
Both profiles must agree, warnings must be `0`, and clippy must show only the
pre-existing `AppWidget` variant-size warning. State the totals in your report.

- [ ] **Step 7: Commit**

```bash
git add src/widgets/fileview.rs
git commit -m "feat(fileview): page up with space, down with Enter

Space duplicated Enter; pairing them in opposite directions lets a thumb
and a pinky scan a file without moving the hand.

Also drops the vim word-back and word-end motions, whose keys are now
the global hide and reveal commands. w still moves forward by word."
```

---

### Task 3: Documentation

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document the new keys**

Add to the global keybindings table, verifying each against
`App::handle_event` before writing it — a documented key that does not exist is
worse than an undocumented one, and this project has already shipped one wrong
key description.

```markdown
| `b` | Hide the left column, giving the file its full width — press again to restore |
| `e` | Bring the left column back and focus it |
| `z` | Maximise the focused pane, or restore the split — works in the navigator too, for long filenames |
```

- [ ] **Step 2: Correct the file view's table**

`space` now pages **up**, `Enter` pages down, and `b`/`e` are gone from that
pane. Update the rows accordingly, and remove `b` and `e` from the file view's
motion list. Check `w` is still listed — it survives.

- [ ] **Step 3: Say what was traded**

Somewhere near the file view's keys:

```markdown
`b` and `e` are global window commands rather than vim word motions: the
trade was deliberate, since returning to the navigator from a maximised file
view is exactly when you need `e`. `w` still moves forward by word.
```

- [ ] **Step 4: Verify**

```bash
cargo test 2>&1 | grep -E "^test result"
cargo test 2>&1 | grep -ci "^warning"
```

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: document the zoom, hide and paging keys"
```

---

## Completion criteria

- `cargo test` passes in debug *and* `--release`, with `0` warnings and no new
  clippy warnings.
- `b` hides the left column and moves focus to the file view; `b` again
  restores it.
- `e` reveals and focuses the navigator, from any state.
- `z` maximises the focused pane, in the navigator as well as the file view,
  and `z` in the file view is indistinguishable from `b`.
- `Tab` while zoomed keeps the zoom on the focused pane.
- `Ctrl-b`, `Ctrl-e` and `Ctrl-z` still reach the file view.
- `space` pages up, `Enter` pages down.
- Manual check in a real terminal: `cargo run -- Cargo.lock`, `b` for full
  width, thumb and pinky on space and Enter to scan, `e` to get back.

Phase 2c-i Task 3 resumes once these hold.
