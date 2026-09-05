# Keymap PR 2: `u`, `Shift-Tab`, shared list motions, `Esc` layering, `/` from the filter pane, digit toggles

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the six small, independent key changes that make the keymap's layer model hold: hide mode on an unshifted key, an opposite for `Tab`, the same list motions in every list pane, `Esc` clearing the focused pane's own search first, `/` working from the filter pane, and digits toggling numbered filters.

**Architecture:** Every item is a new match arm plus a row in `KEYMAP` and the README. The navigator and the filter pane each gain a `last_height` recorded at render time, so page motions know their page; the two panes get parallel `select_first` / `select_last` / `move_by` methods. `Esc` and `/` change only in `App::dispatch_event` and `App::run_search`. Digits reuse the numbering `FilterList::texts` already draws, through a shared `numbered` walk so key and label cannot disagree. Nothing here touches the cross-file stepping from PR 1.

**Tech Stack:** Rust, ratatui 0.30, crossterm. Tests are `#[cfg(test)]` modules in the same files, driven by `key(&mut app, KeyCode::…)` over fixture directories under `target/test-appdirs` (`src/lib.rs`), `nav_over(name, files)` (`src/widgets/filenav.rs`), and `FilterList::default()` with `rows(&filters)` (`src/widgets/filterlist.rs`).

**Spec:** `docs/specs/2026-09-05-keymap-reconciliation-design.md` — sections 5, 6, 7, 11, 12 and 14. This plan is "Landing" item 2. Base: `main` at b6eda5f or later (PR 1 merged).

## Global Constraints

- Every `KeyCode::Char('x')` arm in `src/lib.rs`, `src/viewport.rs`, `src/widgets/filenav.rs`, `src/widgets/filterlist.rs` or `src/widgets/fileview.rs` needs a row in `KEYMAP` (`src/help.rs`) whose `Binding::codes` yields that character, or `every_bound_key_is_documented` fails the build. The scanner reads every `'x'` literal between `Char(` and the next `)`, so `Char(c @ '1'..='9')` binds `'1'` and `'9'` as far as the test is concerned.
- Every new key also needs a row in the README's *Keybindings* section (hand-maintained).
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` must pass; CI runs both.
- Nothing goes silent: every key that does not move or change anything reports on the status row via `self.report(text, false)`.
- A shared motion means the same thing in every pane it is bound in: `g`/`Home` first, `G`/`End` last, `Ctrl-d`/`Ctrl-u` half a page, `PageDown`/`PageUp` a page. Page = the pane's inner height at the last render; before any render, assume 20 rows.
- Fixture directory names passed to `app_over`/`app_over_file`/`nav_over` must be unique across their test module and must not contain `filters` (see memory: a path containing "filters" broke a status-line test) or differ from another only by case (APFS is case-insensitive).
- Run focused tests with `cargo test --lib <name>`; the whole suite with `cargo test`. If `title_shows_the_current_directory` fails, check the worktree path length first; it is a known flake, not a regression.

---

## File map

| File | Change |
|---|---|
| `src/widgets/mod.rs` | `Focus::prev()`. |
| `src/lib.rs` | Arms for `u`, `BackTab`, digits; `focus_prev`; `Esc` layered; `/` guard removed; `run_search` treats the filter pane like the view; tests. |
| `src/widgets/filenav.rs` | `last_height`, `select_first`/`select_last`/`move_by`, `page_rows`, `clear_search`, `has_search`; arms for `g`/`G`/`Home`/`End`/`Ctrl-d`/`Ctrl-u`/`PageUp`/`PageDown`; tests. |
| `src/widgets/filterlist.rs` | `last_height`, `select_first`/`select_last`/`move_by`, `page_rows`; the same arms in `handle_key`; `pub(crate) fn numbered(&ActiveFilters) -> Vec<usize>` shared with `texts`; tests. |
| `src/help.rs` | `Binding::codes` understands a `"1-9"` range label; rows for every new key. |
| `README.md` | Rows for every new key; `Ctrl-H` references become `u`; one reasoning paragraph. |

---

### Task 1: `u` toggles hide mode

**Files:**
- Modify: `src/lib.rs` — beside the `H` / `Ctrl-h` arms (search `KeyCode::Char('H')` in `dispatch_event`)
- Modify: `src/help.rs` — the Global row `keys: &["Ctrl-h", "H"]`
- Modify: `README.md` — Global table row for `Ctrl-H` / `H` (~line 311); Quick Start row (~179); feature bullets that name `Ctrl-H` as the hide key (~17, 57, 61, 67, 74, 77)
- Test: `src/lib.rs` tests module, after the last `peek_*` test

**Interfaces:**
- Consumes: `App::toggle_hiding()` (exists).

- [ ] **Step 1: Write the failing test**

```rust
    /// #120 §10: hide mode is toggled often and lived behind Shift. `u`
    /// ("unmatched") is the primary key now; `H` and `Ctrl-H` stay as aliases.
    #[test]
    fn u_toggles_hiding_like_ctrl_h() {
        let mut app = app_over_file("u_hides", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("beta").expect("valid pattern");
        app.refresh_view();
        assert_eq!(app.document.mode(), Mode::Dimmed, "sanity");

        key(&mut app, KeyCode::Char('u'));
        assert_eq!(app.document.mode(), Mode::FilteredOnly, "u did not hide");

        key(&mut app, KeyCode::Char('u'));
        assert_eq!(app.document.mode(), Mode::Dimmed, "u did not restore");

        // The aliases still work, and share the state.
        key(&mut app, KeyCode::Char('H'));
        assert_eq!(app.document.mode(), Mode::FilteredOnly);
        key(&mut app, KeyCode::Char('u'));
        assert_eq!(app.document.mode(), Mode::Dimmed);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib u_toggles_hiding`
Expected: FAIL on `u did not hide` (the key falls through to the view, which ignores it).

- [ ] **Step 3: Add the arm**

Directly above the `KeyCode::Char('H')` arm in `dispatch_event`:

```rust
                // The primary hide key (#120 §10). `H` needs Shift, and many
                // terminals deliver `Ctrl-H` as Backspace; both stay as
                // aliases, but the key pressed most often in the review loop
                // should not be the one that breaks the flow. `u` as in
                // "toggle unmatched lines", which is what the mode does; vim's
                // `u` is undo and recon has no undo, so no habit collides.
                KeyCode::Char('u') if key.modifiers.is_empty() => {
                    self.toggle_hiding();
                    return;
                }
```

- [ ] **Step 4: Document the key**

`src/help.rs`, the Global row:

```rust
            Binding {
                keys: &["u", "Ctrl-h", "H"],
                action: "Dim unmatched lines, or hide them",
            },
```

`README.md` Global table row (~311):

```markdown
| `u` / `Ctrl-H` / `H` | Toggle between dimming unmatched lines and hiding them — `u` for **u**nmatched; the other two are aliases for terminals and habits that already use them |
```

Quick Start row (~179): change `` `Ctrl-H` `` to `` `u` ``. In the feature bullets (~17, 57, 61, 67, 74, 77) replace `` `Ctrl-H` `` with `` `u` `` where the text means "the hide key". Leave the paragraph around 564–572 (which explains `Ctrl-H`-as-Backspace and `H`) as is, but add one sentence at its start: "`u` is the primary key; the two below are aliases."

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib u_toggles_hiding every_bound_key hiding`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/help.rs README.md
git commit -m "feat(keys): u toggles hide mode; H and Ctrl-H stay as aliases (#120)"
```

---

### Task 2: `Shift-Tab` focuses the previous pane

**Files:**
- Modify: `src/widgets/mod.rs` — after `Focus::next` (~line 71)
- Modify: `src/lib.rs` — after `focus_next` (~line 2118); the `KeyCode::Tab` arm (~line 933)
- Modify: `src/help.rs` — the Global `Tab` row
- Modify: `README.md` — Global table `Tab` row (~301)
- Test: `src/lib.rs` tests module, after `tab_reaches_the_filter_pane_while_it_is_empty`

**Interfaces:**
- Produces: `Focus::prev(self) -> Self`; `App::focus_prev(&mut self)`.

- [ ] **Step 1: Write the failing tests**

```rust
    /// `Tab` finally has its opposite (#120 §1). crossterm reports Shift-Tab
    /// as `KeyCode::BackTab`.
    #[test]
    fn shift_tab_reverses_tab() {
        let mut app = app_over_file("backtab_cycle", "alpha\n");
        draw(&mut app);
        assert_eq!(app.focus, Focus::Nav, "sanity: starts on the navigator");

        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Filters, "did not wrap to the filter pane");
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::View);
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Nav);

        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Nav, "Tab then Shift-Tab is not a no-op");
    }

    /// The zoomed pane is always the focused pane; `focus_prev` keeps that
    /// invariant the way `focus_next` does.
    #[test]
    fn shift_tab_moves_the_zoom_with_the_focus() {
        let mut app = app_over_file("backtab_zoom", "alpha\n");
        draw(&mut app);
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('z'));
        assert_eq!(app.zoom, Some(Focus::View), "sanity: view zoomed");

        key(&mut app, KeyCode::BackTab);

        assert_eq!(app.focus, Focus::Nav);
        assert_eq!(app.zoom, Some(Focus::Nav), "zoom stayed on an unfocused pane");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib shift_tab`
Expected: FAIL on `did not wrap to the filter pane`.

- [ ] **Step 3: Implement**

`src/widgets/mod.rs`, after `next`:

```rust
    /// The other way round. Written out like `next`, and for the same
    /// reason.
    pub(crate) fn prev(self) -> Self {
        match self {
            Self::Nav => Self::Filters,
            Self::View => Self::Nav,
            Self::Filters => Self::View,
        }
    }
```

`src/lib.rs`, after `focus_next`:

```rust
    /// `Shift-Tab`. Same zoom rule as `focus_next`, kept inside the method
    /// for the same reason.
    fn focus_prev(&mut self) {
        self.focus = self.focus.prev();
        if self.zoom.is_some() {
            self.zoom = Some(self.focus);
        }
    }
```

The `Tab` arm gains a sibling directly below it:

```rust
                KeyCode::BackTab => {
                    self.focus_prev();
                    return;
                }
```

- [ ] **Step 4: Document the key**

`src/help.rs` Global row:

```rust
            Binding {
                keys: &["Tab", "Shift-Tab"],
                action: "Focus the next / previous pane",
            },
```

(`Binding::codes` ignores multi-character labels, so `Shift-Tab` documents nothing the scanner checks; that is fine, `BackTab` is not a `Char` arm.)

`README.md` Global row (~301):

```markdown
| `Tab` / `Shift-Tab` | Move focus to the next / previous of the three panes — navigator, file view, filter pane. All three are always on screen, so the cycle never skips one |
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib shift_tab tab_ every_bound_key`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/widgets/mod.rs src/lib.rs src/help.rs README.md
git commit -m "feat(keys): Shift-Tab focuses the previous pane (#120)"
```

---

### Task 3: Shared list motions in the navigator

**Files:**
- Modify: `src/widgets/filenav.rs` — struct fields (~line 200–235), `render` (~807), `handle_events` (~411), beside `select_next` (~668)
- Modify: `src/help.rs` — Navigator section
- Modify: `README.md` — Navigator table (~700)
- Test: `src/widgets/filenav.rs` tests module, after `select_previous_clamps_at_first_entry`

**Interfaces:**
- Produces on `FileNav`: `fn select_first(&mut self)`, `fn select_last(&mut self)`, `fn move_by(&mut self, delta: isize)`, `fn page_rows(&self) -> usize`, field `last_height: Option<u16>`.
- Constant: `const ASSUMED_PAGE: usize = 20;` in `filenav.rs` (and again in `filterlist.rs`, Task 4; two panes, two files, same number).

- [ ] **Step 1: Write the failing tests**

```rust
    // ---- shared list motions (#120 §3) ----------------------------------

    fn render_at_height(nav: &mut FileNav<'_>, height: u16) {
        let area = Rect::new(0, 0, 40, height);
        let mut buf = Buffer::empty(area);
        (&mut *nav).render(area, &mut buf);
    }

    fn press(nav: &mut FileNav<'_>, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        nav.handle_events(Event::Key(KeyEvent::new(code, modifiers)))
    }

    #[test]
    fn g_and_capital_g_select_the_first_and_last_entry() {
        let files: Vec<String> = (0..30).map(|i| format!("f{i:02}.log")).collect();
        let names: Vec<&str> = files.iter().map(String::as_str).collect();
        let mut nav = nav_over("motions_ends", &names);
        nav.select_entry(nav.files()[5].0);

        let action = press(&mut nav, KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(selected_name(&nav), "f29.log", "G did not reach the end");
        assert!(matches!(action, Some(Action::Preview(_))), "G did not preview");

        press(&mut nav, KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(selected_name(&nav), PARENT, "g did not reach the top");

        press(&mut nav, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(selected_name(&nav), "f29.log");
        press(&mut nav, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(selected_name(&nav), PARENT);
    }

    #[test]
    fn page_motions_use_the_rendered_height_and_clamp() {
        let files: Vec<String> = (0..30).map(|i| format!("f{i:02}.log")).collect();
        let names: Vec<&str> = files.iter().map(String::as_str).collect();
        let mut nav = nav_over("motions_page", &names);
        // 12 rows tall, 10 inside the border: a page is 10, half is 5.
        render_at_height(&mut nav, 12);
        nav.select_entry(nav.files()[0].0); // row 1, under `..`

        press(&mut nav, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(selected_name(&nav), "f05.log", "Ctrl-d is not half a page");
        press(&mut nav, KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(selected_name(&nav), "f15.log", "PageDown is not a page");
        press(&mut nav, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(selected_name(&nav), "f10.log", "Ctrl-u is not half a page up");

        press(&mut nav, KeyCode::PageUp, KeyModifiers::NONE);
        press(&mut nav, KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(selected_name(&nav), PARENT, "PageUp did not clamp at the top");
        for _ in 0..5 {
            press(&mut nav, KeyCode::PageDown, KeyModifiers::NONE);
        }
        assert_eq!(selected_name(&nav), "f29.log", "PageDown did not clamp at the end");
    }

    /// Before the first render, a page is the assumed height rather than
    /// zero — a zero-row page would make the keys silent no-ops.
    #[test]
    fn page_motions_assume_a_height_before_the_first_render() {
        let files: Vec<String> = (0..30).map(|i| format!("f{i:02}.log")).collect();
        let names: Vec<&str> = files.iter().map(String::as_str).collect();
        let mut nav = nav_over("motions_unrendered", &names);
        nav.select_entry(nav.files()[0].0);

        press(&mut nav, KeyCode::PageDown, KeyModifiers::NONE);

        assert_eq!(selected_name(&nav), "f20.log");
    }
```

`Rect`, `Buffer`, `KeyModifiers`, `Event`, `KeyEvent`, `Widget` may already be imported in the test module; add whichever `use` lines the compiler asks for, keeping them inside `mod tests`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib motions_`
Expected: FAIL — `G did not reach the end` (the keys are unbound).

- [ ] **Step 3: Implement**

Add near the top of `src/widgets/filenav.rs` (after the `use` block):

```rust
/// A page, in rows, before the pane has been drawn once. `App::run` renders
/// before it reads a key, so this only ever matters to a test — but a zero
/// page would make `PageDown` a silent no-op, and #120 forbids silent keys.
const ASSUMED_PAGE: usize = 20;
```

Add the field to `FileNav`, next to `active`:

```rust
    /// Inner height at the last render, so page motions know their page.
    /// `None` until then; see `ASSUMED_PAGE`.
    last_height: Option<u16>,
```

Initialise it as `last_height: None,` wherever the struct is built (`FileNav::new`).

In `render`, after `let inner = block.inner(area);`:

```rust
        self.last_height = Some(inner.height);
```

Beside `select_next`:

```rust
    fn select_first(&mut self) {
        if !self.visible.is_empty() {
            self.state.select(Some(0));
        }
    }

    fn select_last(&mut self) {
        if let Some(last) = self.visible.len().checked_sub(1) {
            self.state.select(Some(last));
        }
    }

    /// Move the selection by `delta` rows, clamping at both ends. Positive is
    /// down. Shared by the page motions; `j`/`k` keep their own one-row
    /// methods, which predate this and are the ones tests already name.
    fn move_by(&mut self, delta: isize) {
        let Some(last) = self.visible.len().checked_sub(1) else {
            return;
        };
        let from = self.state.selected().unwrap_or(0);
        let to = from.saturating_add_signed(delta).min(last);
        self.state.select(Some(to));
    }

    /// Rows in a page: the pane's inner height at the last render.
    fn page_rows(&self) -> usize {
        self.last_height
            .map_or(ASSUMED_PAGE, usize::from)
            .max(1)
    }
```

In `handle_events`, add arms beside the `Up`/`Down` ones. The existing arms match on `key.code` alone; the two Ctrl arms need the modifier, so match on the pair:

```rust
                // Shared list motions (#120 §3): the same keys, with the same
                // meaning, as the file view. `g`/`G` and the Ctrl pair are
                // deliberately not intercepted by `App` for this pane — only
                // the file view holds a *window* of its document; the
                // navigator holds all its rows, so it can answer itself.
                KeyCode::Char('g') | KeyCode::Home => {
                    self.select_first();
                    return self.preview_selection();
                }
                KeyCode::Char('G') | KeyCode::End => {
                    self.select_last();
                    return self.preview_selection();
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let half = (self.page_rows() / 2).max(1);
                    self.move_by(half as isize);
                    return self.preview_selection();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let half = (self.page_rows() / 2).max(1);
                    self.move_by(-(half as isize));
                    return self.preview_selection();
                }
                KeyCode::PageDown => {
                    self.move_by(self.page_rows() as isize);
                    return self.preview_selection();
                }
                KeyCode::PageUp => {
                    self.move_by(-(self.page_rows() as isize));
                    return self.preview_selection();
                }
```

If clippy complains about `as isize` casts, use `isize::try_from(n).unwrap_or(isize::MAX)`. Confirm `KeyModifiers` is imported at the top of the file (it is used by `App`; add `use crossterm::event::KeyModifiers;` if not).

`Ctrl-u` here is not the global `u` from Task 1: that arm is guarded `modifiers.is_empty()`, so a Ctrl-u falls through to the focused pane.

- [ ] **Step 4: Document the keys**

`src/help.rs`, Navigator section, after the `n`/`N` row:

```rust
            Binding {
                keys: &["g", "Home"],
                action: "First entry",
            },
            Binding {
                keys: &["G", "End"],
                action: "Last entry",
            },
            Binding {
                keys: &["Ctrl-d", "Ctrl-u"],
                action: "Half a page down / up",
            },
            Binding {
                keys: &["PageDown", "PageUp"],
                action: "A page down / up",
            },
```

`README.md`, Navigator table (~700), after the `n`/`N` row:

```markdown
| `g` / `Home` | Select the first entry |
| `G` / `End` | Select the last entry |
| `Ctrl-d` / `Ctrl-u` | Move half a page down / up |
| `PageDown` / `PageUp` | Move a page down / up |
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib motions_ select_ every_bound_key`
Expected: all PASS. If the help overlay's layout test fails on width, shorten the action strings, not the keys.

- [ ] **Step 6: Commit**

```bash
git add src/widgets/filenav.rs src/help.rs README.md
git commit -m "feat(filenav): g/G, Home/End, Ctrl-d/Ctrl-u, PageUp/PageDown in the navigator (#120)"
```

---

### Task 4: Shared list motions in the filter pane

**Files:**
- Modify: `src/widgets/filterlist.rs` — struct (~line 113), `handle_key` (~164), `render` (~the `pub(crate) fn render`), beside `select_next` (~124)
- Modify: `src/help.rs` — Filter pane section
- Modify: `README.md` — Filter pane table (~783)
- Test: `src/widgets/filterlist.rs` tests module, after `a_on_a_header_opens_the_picker_and_on_a_filter_does_nothing`

**Interfaces:**
- Produces on `FilterList`: `pub(crate) fn select_first(&mut self, len: usize)`, `pub(crate) fn select_last(&mut self, len: usize)`, `pub(crate) fn move_by(&mut self, delta: isize, len: usize)`, `fn page_rows(&self) -> usize`, field `last_height: Option<u16>`.

- [ ] **Step 1: Write the failing tests**

```rust
    // ---- shared list motions (#120 §3) ----------------------------------

    /// Twelve scratch filters: enough rows for a page to be smaller than the
    /// list on a short pane.
    fn twelve() -> ActiveFilters {
        let patterns: Vec<String> = (0..12).map(|i| format!("p{i:02}")).collect();
        let refs: Vec<&str> = patterns.iter().map(String::as_str).collect();
        set_of(&refs, &[])
    }

    fn press(list: &mut FilterList, code: KeyCode, modifiers: KeyModifiers, rows: &[Row]) {
        let command = list.handle_key(KeyEvent::new(code, modifiers), rows);
        assert_eq!(command, None, "a motion is not a command");
    }

    #[test]
    fn g_and_capital_g_select_the_first_and_last_row() {
        let filters = twelve();
        let rows = rows(&filters);
        let mut list = FilterList::default();
        list.state.select(Some(4));

        press(&mut list, KeyCode::Char('G'), KeyModifiers::SHIFT, &rows);
        assert_eq!(list.selected(), Some(11));
        press(&mut list, KeyCode::Char('g'), KeyModifiers::NONE, &rows);
        assert_eq!(list.selected(), Some(0));
        press(&mut list, KeyCode::End, KeyModifiers::NONE, &rows);
        assert_eq!(list.selected(), Some(11));
        press(&mut list, KeyCode::Home, KeyModifiers::NONE, &rows);
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn page_motions_use_the_rendered_height_and_clamp() {
        let filters = twelve();
        let rows = rows(&filters);
        let mut list = FilterList::default();
        // 8 rows tall, 6 inside the border: a page is 6, half is 3.
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        list.render(&filters, area, &mut buf);
        list.state.select(Some(0));

        press(&mut list, KeyCode::Char('d'), KeyModifiers::CONTROL, &rows);
        assert_eq!(list.selected(), Some(3), "Ctrl-d is not half a page");
        press(&mut list, KeyCode::PageDown, KeyModifiers::NONE, &rows);
        assert_eq!(list.selected(), Some(9), "PageDown is not a page");
        press(&mut list, KeyCode::PageDown, KeyModifiers::NONE, &rows);
        assert_eq!(list.selected(), Some(11), "PageDown did not clamp");
        press(&mut list, KeyCode::Char('u'), KeyModifiers::CONTROL, &rows);
        assert_eq!(list.selected(), Some(8), "Ctrl-u is not half a page up");
        press(&mut list, KeyCode::PageUp, KeyModifiers::NONE, &rows);
        press(&mut list, KeyCode::PageUp, KeyModifiers::NONE, &rows);
        assert_eq!(list.selected(), Some(0), "PageUp did not clamp");
    }

    /// `Ctrl-d` was already guarded from reading as `d` (delete). It now
    /// moves instead of being dropped, and still never deletes.
    #[test]
    fn ctrl_d_moves_and_never_deletes() {
        let filters = twelve();
        let rows = rows(&filters);
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &rows,
        );

        assert_eq!(command, None);
        assert_ne!(list.selected(), Some(0), "Ctrl-d did not move");
    }
```

`Rect` and `Buffer` come from `ratatui::prelude`; `KeyModifiers`, `KeyEvent` from `crossterm::event`. Add `use` lines inside `mod tests` as the compiler asks.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filterlist::tests::g_and_capital filterlist::tests::page_motions filterlist::tests::ctrl_d_moves`
Expected: FAIL — `G` leaves the selection at 4; `Ctrl-d` does not move.

- [ ] **Step 3: Implement**

Near the top of `src/widgets/filterlist.rs`, after `const BORDERS`:

```rust
/// A page, in rows, before the pane has been drawn once — see the same
/// constant in `filenav.rs`.
const ASSUMED_PAGE: usize = 20;
```

Add to `FilterList`:

```rust
    /// Inner height at the last render, so page motions know their page.
    last_height: Option<u16>,
```

(`FilterList` derives `Default`, so no initialiser is needed.)

In `render`, before `StatefulWidget::render(&list, area, buf, &mut self.state);`:

```rust
        self.last_height = Some(area.height.saturating_sub(BORDERS));
```

Beside `select_previous`:

```rust
    pub(crate) fn select_first(&mut self, len: usize) {
        if len > 0 {
            self.state.select(Some(0));
        }
    }

    pub(crate) fn select_last(&mut self, len: usize) {
        if let Some(last) = len.checked_sub(1) {
            self.state.select(Some(last));
        }
    }

    /// Move by `delta` rows, clamping at both ends. Positive is down.
    pub(crate) fn move_by(&mut self, delta: isize, len: usize) {
        let Some(last) = len.checked_sub(1) else {
            return;
        };
        let from = self.state.selected().unwrap_or(0);
        self.state.select(Some(from.saturating_add_signed(delta).min(last)));
    }

    fn page_rows(&self) -> usize {
        self.last_height
            .map_or(ASSUMED_PAGE, usize::from)
            .max(1)
    }
```

In `handle_key`, replace the opening modifier guard and the `j`/`k` match with:

```rust
        // The two Ctrl motions are the only modified keys this pane answers.
        // Every other modified key is dropped here — `Ctrl-D` used to read as
        // `d` and delete the selected filter, which is the whole reason this
        // guard exists.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            let half = isize::try_from((self.page_rows() / 2).max(1)).unwrap_or(isize::MAX);
            match key.code {
                KeyCode::Char('d') => self.move_by(half, rows.len()),
                KeyCode::Char('u') => self.move_by(-half, rows.len()),
                _ => {}
            }
            return None;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return None;
        }
        let page = isize::try_from(self.page_rows()).unwrap_or(isize::MAX);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next(rows.len());
                return None;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous(rows.len());
                return None;
            }
            // Shared list motions (#120 §3), same meaning as the other panes.
            KeyCode::Char('g') | KeyCode::Home => {
                self.select_first(rows.len());
                return None;
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.select_last(rows.len());
                return None;
            }
            KeyCode::PageDown => {
                self.move_by(page, rows.len());
                return None;
            }
            KeyCode::PageUp => {
                self.move_by(-page, rows.len());
                return None;
            }
            _ => {}
        }
```

Keep the doc comment above `handle_key` that explains the `Ctrl-D` history; amend its last sentence to say Ctrl-d now pages instead of being dropped.

- [ ] **Step 4: Document the keys**

`src/help.rs`, Filter pane section, after the `["j", "k"]` row:

```rust
            Binding {
                keys: &["g", "G"],
                action: "First / last row",
            },
            Binding {
                keys: &["Ctrl-d", "Ctrl-u"],
                action: "Half a page down / up",
            },
            Binding {
                keys: &["PageDown", "PageUp"],
                action: "A page down / up",
            },
```

`README.md`, Filter pane table, after the `j` / `Down` row:

```markdown
| `g` / `Home` | Select the first row |
| `G` / `End` | Select the last row |
| `Ctrl-d` / `Ctrl-u` | Move half a page down / up |
| `PageDown` / `PageUp` | Move a page down / up |
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib filterlist every_bound_key`
Expected: all PASS, including the existing `handle_key` tests.

- [ ] **Step 6: Commit**

```bash
git add src/widgets/filterlist.rs src/help.rs README.md
git commit -m "feat(filterlist): g/G, Home/End, Ctrl-d/Ctrl-u, PageUp/PageDown in the filter pane (#120)"
```

---

### Task 5: `Esc` clears the focused pane's search first

**Files:**
- Modify: `src/widgets/filenav.rs` — beside `search` (~361)
- Modify: `src/lib.rs` — the `KeyCode::Esc` arm (~987)
- Modify: `src/help.rs` — the Global `Esc` row
- Modify: `README.md` — Global `Esc` row (~304)
- Test: `src/lib.rs` tests module, after `slash_in_the_navigator_still_searches_filenames`

**Interfaces:**
- Produces on `FileNav`: `pub(crate) fn clear_search(&mut self) -> bool` (true if there was one), `pub(crate) fn has_search(&self) -> bool`.

- [ ] **Step 1: Write the failing test**

```rust
    /// #120 §8: `Esc` clears whichever search the focused pane owns first,
    /// then the live search. One key, one meaning, layered.
    #[test]
    fn esc_clears_the_navigator_search_before_the_live_search() {
        let mut app = app_over_files(
            "esc_layers",
            &[("alpha.log", "hit\n"), ("zebra.log", "hit\n")],
        );
        open_file(&mut app, 0);
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "hit");
        key(&mut app, KeyCode::Enter);
        assert!(app.filters.search().is_some(), "sanity: live search set");

        key(&mut app, KeyCode::Char('e'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "zebra");
        key(&mut app, KeyCode::Enter);
        assert!(app.nav.has_search(), "sanity: navigator search set");

        key(&mut app, KeyCode::Esc);
        assert!(!app.nav.has_search(), "Esc did not clear the navigator search");
        assert!(
            app.filters.search().is_some(),
            "Esc cleared the live search on the same press"
        );

        key(&mut app, KeyCode::Esc);
        assert!(app.filters.search().is_none(), "second Esc did not clear the live search");
    }

    /// From the file view, `Esc` does not reach into the navigator.
    #[test]
    fn esc_in_the_view_leaves_the_navigator_search_alone() {
        let mut app = app_over("esc_view_only", &["alpha.log", "zebra.log"]);
        key(&mut app, KeyCode::Char('e'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "zebra");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Esc);

        assert!(app.nav.has_search());
    }
```

`app_over_files` and `open_file` exist in the module (from PR 1).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib esc_`
Expected: compile error, `no method named has_search`.

- [ ] **Step 3: Implement**

`src/widgets/filenav.rs`, after `search`:

```rust
    /// Drop the filename search, restoring the plain listing styles.
    /// Reports whether there was one, so `Esc` can fall through to the live
    /// search when there was not.
    pub(crate) fn clear_search(&mut self) -> bool {
        if self.matcher.take().is_none() {
            return false;
        }
        self.rebuild_list();
        true
    }

    pub(crate) fn has_search(&self) -> bool {
        self.matcher.is_some()
    }
```

`src/lib.rs`, the `Esc` arm becomes:

```rust
                KeyCode::Esc if key.modifiers.is_empty() => {
                    // Layered (#120 §8): the focused pane's own search first,
                    // then the live search. The navigator's filename search
                    // is separate state, and until now nothing but a new
                    // search replaced it — `n` kept repeating a search the
                    // user thought they had dismissed.
                    if self.focus == Focus::Nav && self.nav.clear_search() {
                        return;
                    }
                    // `clear_search` reports whether there was one to drop, the
                    // same shape `p`'s `promote_search` guard uses just below:
                    // `refresh_view` is not free — `evaluate` is
                    // O(lines × filters) — and Esc is a key people tap out of
                    // habit, so it should not pay for a re-evaluate when there
                    // was nothing to clear.
                    if self.filters.clear_search() {
                        self.refresh_view();
                    }
                    return;
                }
```

- [ ] **Step 4: Document the key**

`src/help.rs` Global `Esc` row action: `"Clear the navigator's filename search, else the live search"` (shorten to `"Clear the pane's search, else the live search"` if the overlay layout test objects).

`README.md` Global `Esc` row (~304):

```markdown
| `Esc` | In the navigator with a filename search active, clear it; otherwise clear the live search. An open prompt takes this key first and just cancels the prompt |
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib esc_ slash_ n_repeats every_bound_key`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/widgets/filenav.rs src/lib.rs src/help.rs README.md
git commit -m "feat(keys): Esc clears the navigator's filename search before the live search (#120)"
```

---

### Task 6: `/` from the filter pane sets a live search

**Files:**
- Modify: `src/lib.rs` — the `/` arm (~962), `run_search` (~565–590), the comment in `handle_filter_key` that mentions how `/` is guarded (~"Guarding the global arm the way `/` is guarded")
- Modify: `README.md` — Global `/` row (~302)
- Test: `src/lib.rs` tests module, after `esc_in_the_view_leaves_the_navigator_search_alone`

- [ ] **Step 1: Write the failing test**

```rust
    /// #120 §7 decision (b): the filter pane forwards `/` to the view — a
    /// "new search", where `c` on the search row is "edit search". Focus
    /// stays in the pane (return-focus is PR 3), and `n` works from there.
    #[test]
    fn slash_from_the_filter_pane_sets_a_live_search() {
        let mut app = app_over_file("slash_from_pane", "plain\nhit\nplain\nhit\n");
        key(&mut app, KeyCode::Char('f'));

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "hit");
        key(&mut app, KeyCode::Enter);

        assert!(app.filters.search().is_some(), "no live search was set");
        assert_eq!(app.focus, Focus::Filters, "focus moved");
        assert_eq!(cursor_source(&app), 1, "did not move to the first hit");

        key(&mut app, KeyCode::Char('n'));
        assert_eq!(cursor_source(&app), 3);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib slash_from_the_filter_pane`
Expected: FAIL on `no live search was set` (the `/` arm is guarded off in the filter pane).

- [ ] **Step 3: Implement**

The `/` arm loses its focus guard:

```rust
                // Global (#120 §7): from the filter pane too. The pane used to
                // refuse `/` so that a swallowed `Enter` would not look like
                // a toggle; the prompt now takes every key while open, and
                // `swallow_next_enter` already guards the commit, so the
                // reason is gone. `/` here is "new search"; `c` on the search
                // row is "edit search".
                KeyCode::Char('/') if key.modifiers.is_empty() => {
                    self.search = Some(SearchPrompt::default());
                    return;
                }
```

In `run_search`, merge the filter pane into the view's arm and drop the "unreachable" comment:

```rust
        let action = match self.focus {
            Focus::Nav => self.nav.search(pattern, false)?,
            // The filter pane forwards view-shaped keys to the view (#120):
            // a search started there is the same live search. Deferred
            // rather than done here: setting the filter needs `&mut self`
            // for `refresh_view`, and the borrow taken to reach the pane is
            // still live.
            Focus::View | Focus::Filters => {
                view_search = true;
                None
            }
        };
```

In `handle_filter_key`, the comment beginning "`e` would read better than `x`" refers to how `/` is guarded; change "Guarding the global arm the way `/` is guarded" to "Guarding the global arm on focus" so it no longer cites a guard that no longer exists.

- [ ] **Step 4: Document the key**

`README.md` Global `/` row (~302):

```markdown
| `/` | In the navigator, search filenames. In the file view or the filter pane, set a live search — a filter of its own, which moves you to its next hit from the cursor exactly as `n` would |
```

`src/help.rs` `/` row action already says "Search — filenames, or file contents"; leave it.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib slash_ search every_bound_key`
Expected: all PASS. If a test asserted that `/` in the filter pane does nothing, replace its assertion with the new behaviour and say so in the commit body.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs README.md
git commit -m "feat(keys): / from the filter pane sets the live search (#120)"
```

---

### Task 7: `1`–`9` toggle a filter by its pane number

**Files:**
- Modify: `src/widgets/filterlist.rs` — beside `texts` (~271)
- Modify: `src/help.rs` — `Binding::codes` (~57) and a new Global row
- Modify: `src/lib.rs` — a global arm beside the `.`/`,` arm
- Modify: `README.md` — Global table, after the `[`/`]` row
- Test: `src/widgets/filterlist.rs` and `src/lib.rs` tests modules

**Interfaces:**
- Produces: `pub(crate) fn numbered(filters: &ActiveFilters) -> Vec<usize>` in `filterlist.rs` — known-list indices of the user-authored filters in pane order; position `n-1` is the filter the pane labels `n`.
- `Binding::codes` yields every char in a `"a-z"`-shaped label (two single chars around a `-`), so one `"1-9"` label documents nine keys.

- [ ] **Step 1: Write the failing tests**

`src/widgets/filterlist.rs`, after the motion tests from Task 4:

```rust
    /// The digit keys and the gutter labels are one walk, so they cannot
    /// drift: label `n` is `numbered()[n - 1]`.
    #[test]
    fn numbered_agrees_with_the_pane_labels() {
        let mut filters = two_sets(true, true);
        filters.set_search("s").expect("valid");
        let numbered = numbered(&filters);

        let labelled: Vec<(String, Row)> = FilterList::texts(&filters)
            .into_iter()
            .filter_map(|(row, text)| {
                let label = text.split_whitespace().next()?.to_string();
                label.parse::<usize>().ok().map(|_| (label, row))
            })
            .collect();

        assert!(!labelled.is_empty(), "sanity: the pane numbers something");
        for (label, row) in labelled {
            let n: usize = label.parse().expect("digit label");
            assert_eq!(Some(row), numbered.get(n - 1).map(|&i| Row::Filter(i)), "label {n}");
        }
        assert!(
            numbered.iter().all(|&i| filters.is_user_authored(i)),
            "a built-in filter got a number"
        );
    }
```

`src/help.rs` tests module:

```rust
    #[test]
    fn a_range_label_documents_every_key_in_it() {
        let binding = Binding {
            keys: &["1-9"],
            action: "",
        };
        let codes: Vec<char> = binding.codes().collect();
        assert_eq!(codes, ('1'..='9').collect::<Vec<_>>());

        // `Ctrl-d` is not a range: one key, `d`.
        let binding = Binding {
            keys: &["Ctrl-d"],
            action: "",
        };
        assert_eq!(binding.codes().collect::<Vec<_>>(), vec!['d']);
    }
```

`src/lib.rs`, after `slash_from_the_filter_pane_sets_a_live_search`:

```rust
    /// #120 §14: `3` toggles the filter the pane labels `3`. Global, so the
    /// loop can switch a filter without leaving the view.
    #[test]
    fn digits_toggle_filters_by_their_pane_number() {
        let mut app = app_over_file("digit_toggle", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        app.filters.add("beta").expect("valid pattern");
        app.refresh_view();
        assert!(app.filters.filters()[1].enabled, "sanity");

        key(&mut app, KeyCode::Char('2'));
        assert!(!app.filters.filters()[1].enabled, "2 did not toggle filter 2");
        assert!(app.filters.filters()[0].enabled, "2 touched filter 1");
        assert_eq!(app.focus, Focus::View, "focus moved");

        key(&mut app, KeyCode::Char('2'));
        assert!(app.filters.filters()[1].enabled, "2 did not toggle back");
    }

    #[test]
    fn a_digit_with_no_filter_behind_it_says_so() {
        let mut app = app_over_file("digit_missing", "alpha\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        app.refresh_view();

        key(&mut app, KeyCode::Char('9'));

        assert!(app.filters.filters()[0].enabled, "9 toggled something");
        assert_eq!(
            app.status_message.as_ref().map(|m| m.text.as_str()),
            Some("no filter 9")
        );
    }

    /// The digit toggles what the *pane* numbers: with a set soloed, the
    /// numbering restarts inside it, and so does the key.
    #[test]
    fn digits_follow_the_pane_numbering_under_a_solo() {
        let a = filter::test_support::loaded("a", 10, true, &["alpha"]);
        let b = filter::test_support::loaded("b", 20, true, &["beta"]);
        let mut app = app_over_file("digit_solo", "alpha\nbeta\nneither\n");
        app.filters = ActiveFilters::with_sets(None, &[a, b]);
        app.refresh_view();
        key(&mut app, KeyCode::Char('t'));
        let before = widgets::filterlist::numbered(&app.filters);
        assert_eq!(before.len(), 2, "sanity: alpha is 1, beta is 2");
        let beta = before[1];

        // Solo set 2 (`b`): the pane now labels beta `1`, and so does the key.
        app.filters.solo(2);
        assert_eq!(widgets::filterlist::numbered(&app.filters), vec![beta], "sanity");
        assert!(app.filters.filters()[beta].enabled, "sanity");

        key(&mut app, KeyCode::Char('1'));

        assert!(
            !app.filters.filters()[beta].enabled,
            "1 did not follow the solo numbering"
        );
        assert!(
            app.filters.filters()[before[0]].enabled,
            "1 reached the soloed-out set"
        );
    }
```

`filter::test_support::loaded(name, order, enabled, patterns)` and `ActiveFilters::with_sets` are what `app_with_two_sets` (~line 5402) uses; copy its import style. The set index passed to `solo` is the set's position in the pane (`1` = `a`, `2` = `b`); confirm against `app_with_two_sets`'s neighbours if `solo(2)` does not select `b`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib numbered_agrees a_range_label digit`
Expected: compile errors — no `numbered`, and `codes` returns one char for `"1-9"`.

- [ ] **Step 3: Implement `numbered` and use it in `texts`**

In `src/widgets/filterlist.rs`, above `texts`:

```rust
/// The user-authored filters in pane order: `numbered()[n - 1]` is the
/// known-list index of the filter the pane labels `n`, and the one the `n`
/// key toggles (#120 §14). One walk for both, so the label and the key
/// cannot disagree. Built-in filters are unnumbered and absent.
pub(crate) fn numbered(filters: &ActiveFilters) -> Vec<usize> {
    rows(filters)
        .into_iter()
        .filter_map(|row| match row {
            Row::Filter(index) => Some(index),
            _ => None,
        })
        .collect()
}
```

Then make `texts` number rows through it, replacing its `let mut number = 0; … number += 1` with a lookup:

```rust
    fn texts(filters: &ActiveFilters) -> Vec<(Row, String)> {
        let numbered = numbered(filters);
        rows(filters)
            .into_iter()
            .map(|row| {
                // Built-in filters (#127) take no number: numbering, like the
                // palette, runs over what the user wrote.
                let label = match row {
                    Row::Filter(index) => numbered
                        .iter()
                        .position(|&i| i == index)
                        .map_or_else(|| " ".to_string(), |n| (n + 1).to_string()),
                    _ => " ".to_string(),
                };
                (row, Self::row_text(filters, row, &label))
            })
            .collect()
    }
```

- [ ] **Step 4: Teach `Binding::codes` the range label**

In `src/help.rs`:

```rust
    fn codes(&self) -> impl Iterator<Item = char> + '_ {
        self.keys.iter().flat_map(|label| {
            if *label == "space" {
                return vec![' '];
            }
            let bare = label.strip_prefix("Ctrl-").unwrap_or(label);
            let chars: Vec<char> = bare.chars().collect();
            match chars.as_slice() {
                [c] => vec![*c],
                // `1-9`: one label, nine keys. Only for a bare range — a
                // `Ctrl-` prefix was stripped above, so `Ctrl-d` is `d`.
                [a, '-', b] if a < b => (*a..=*b).collect(),
                _ => Vec::new(),
            }
        })
    }
```

- [ ] **Step 5: Add the global arm**

In `src/lib.rs` `dispatch_event`, directly above the `.`/`,` arm:

```rust
                // Toggle a numbered filter from anywhere (#120 §14). The
                // number is the one the pane draws in its gutter, and
                // `numbered` is the walk that draws it, so key and label
                // cannot disagree. Set headers, the search row and built-in
                // filters have no number and no key: `f Enter` covers them.
                KeyCode::Char(c @ '1'..='9') if key.modifiers.is_empty() => {
                    let n = usize::from(c as u8 - b'0');
                    match widgets::filterlist::numbered(&self.filters).get(n - 1) {
                        Some(&index) => {
                            self.filters.toggle_enabled(index);
                            self.refresh_view();
                        }
                        None => self.report(&format!("no filter {n}"), false),
                    }
                    return;
                }
```

- [ ] **Step 6: Document the keys**

`src/help.rs`, Global section, after the `[`/`]` row:

```rust
            Binding {
                keys: &["1-9"],
                action: "Toggle the filter with that number",
            },
```

`README.md`, Global table, after the `[`/`]` row:

```markdown
| `1` – `9` | Toggle the filter the pane numbers `1` to `9`, from any pane. Built-in filters, set headers and the search row have no number; `Enter` in the filter pane toggles those |
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib numbered_agrees a_range_label digit every_bound_key filterlist`
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
git add src/widgets/filterlist.rs src/help.rs src/lib.rs README.md
git commit -m "feat(keys): 1-9 toggle a filter by its pane number, from any pane (#120)"
```

---

### Task 8: README reasoning, full verification

**Files:**
- Modify: `README.md` — the reasoning paragraphs in the Keybindings section (after the "`space` stays the peek" paragraph in the filter-pane block)

- [ ] **Step 1: Add the reasoning paragraph**

```markdown
Hide mode moved to `u` because it is toggled constantly during a review and lived
behind Shift (`H`) or behind a key many terminals deliver as Backspace (`Ctrl-H`).
`u` is "toggle **u**nmatched lines", which is what the mode does; vim's `u` is undo,
and recon has no undo, so no habit collides. The two old keys stay as aliases. The
list motions (`g`/`G`, `Ctrl-d`/`Ctrl-u`, `PageUp`/`PageDown`) are the same in every
list pane for the same reason `n` is: a key with one meaning is one you stop
thinking about. `Esc` clears the focused pane's own search before the live one, so a
navigator search you thought you had dismissed cannot keep driving `n`. And the
digits toggle the filter the pane numbers, from anywhere, because switching a filter
off to see what it was hiding is a loop action, not a setup one.
```

- [ ] **Step 2: Run everything**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: fmt makes no changes, clippy clean, all tests pass.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs(readme): why u, shared motions, layered Esc and digit toggles (#120)"
```

---

## Self-review

**Spec coverage:** §5 `u` — Task 1. §6 `Shift-Tab` — Task 2. §7 shared list motions in both list panes, with the `Ctrl-d`-not-`d` guard kept — Tasks 3 and 4. §12 `Esc` layering — Task 5. §11 `/` from the filter pane — Task 6. §14 digits by pane number (as amended) — Task 7. Every new `Char` arm has a `KEYMAP` row: `u` (T1), `g`/`G` in two files (T3, T4; rows in each pane's section), `Ctrl-d`/`Ctrl-u` in two files (T3, T4), `'1'`/`'9'` (T7, via the range label).

**Not in this PR:** chain return-focus and wrong-pane hints (§8, §9 — PR 3), `*` (§13 — PR 4), the `KEYMAP` regroup (§15 — PR 5).

**Type consistency:** `FileNav::move_by(delta: isize)` vs `FilterList::move_by(delta: isize, len: usize)` — different signatures on purpose: the navigator owns its row count, the filter pane is handed `rows`. `numbered()` returns known-list indices, which is what `toggle_enabled(index)` and `is_user_authored(index)` take. `Binding::codes` still returns `impl Iterator<Item = char>`; the body changed from `filter_map` to `flat_map` over `Vec<char>`.

**Placeholder scan:** none.
