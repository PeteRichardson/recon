# Keymap PR 1: Cross-file `n`, file-stepping keys, global paging, peek-then-move

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the review loop run from the file view without changing focus: `n`/`N` cross file boundaries, `.`/`,` skip files from any pane, `[`/`]` page the view from any pane, and a jump out of a peeked file restores the peek first.

**Architecture:** Three small query additions to the viewport (`impl App` in `src/viewport.rs`), one new navigator method that steps to the next `Match::Yes` entry, and one new `App` method (`step_interesting`) that composes them: in-file step first, cross-file step second, in-file wrap as the fallback. The crossing is reported three ways (status row, centred notice, accent-coloured view title), all cleared by the next keypress via the existing `status_message` lifetime. The `n` arm in `App::dispatch_event` becomes a call to the new method; two new global arms (`.`/`,` and `[`/`]`) sit beside it.

**Tech Stack:** Rust, ratatui, crossterm, tui-textarea (vendored). Tests are `#[cfg(test)]` modules in the same files, driven by `key(&mut app, KeyCode::…)` over fixture directories under `target/test-appdirs`.

**Spec:** `docs/specs/2026-09-05-keymap-reconciliation-design.md` — sections 1 through 4 and the "Implementation notes" and "Testing" sections. This plan is "Landing" item 1.

## Global Constraints

- Every `KeyCode::Char('x')` arm added to `src/lib.rs` needs a row in `KEYMAP` (`src/help.rs`) or `every_bound_key_is_documented` fails the build. Add the row in the same task as the arm.
- Every new key also needs a row in the README's *Keybindings* section (hand-maintained; the test does not check it).
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` must pass; CI runs both.
- Cross-file stepping uses `Match::Yes` only. It never consults the navigator's filename search. `Match::Unknown` and `Match::No` are skipped.
- An interesting line is `Verdict::Included | Verdict::Searched`. A line only a context filter matched has its own verdict, `Verdict::Context`, and is not a stop for `n` (added after the branch's review, matching the navigator's scan mask).
- Nothing goes silent: every path that does not move reports on the status row.
- Fixture directory names passed to `app_over`/`claim_fixture_dir` must be unique across the test module and must not contain `filters` (see memory note on status-line path fragility) or differ only by case from another (APFS is case-insensitive).
- Run tests with `cargo test --lib <name>`; the whole suite with `cargo test`.

---

## File map

| File | Change |
|---|---|
| `src/viewport.rs` | Add `next_interesting_strict`, `first_interesting`, `land_on` to `impl App`. Refactor `step_to_interesting` to use `land_on`. |
| `src/widgets/filenav.rs` | Add `pub(crate) fn step_to_match(&mut self, reverse: bool) -> Option<Action>` and `pub(crate) fn selected_name(&self) -> Option<String>`. |
| `src/widgets/fileview.rs` | Add `title_accent: bool` field, `set_title_accent`, and use it in `render` when building the block title. |
| `src/lib.rs` | Add `Crossing` struct and `crossing: Option<Crossing>` field; `step_interesting`, `cross_file`, `skip_file`, `forward_to_view`; new arms for `.`/`,`/`[`/`]`; `n`/`N` arm rewritten; render the notice; tests. |
| `src/help.rs` | `KEYMAP`: new Global rows for `.`/`,` and `[`/`]`; the file view's `[`/`]` row loses those two keys; `n`/`N` row text updated. |
| `README.md` | Keybindings tables and a reasoning paragraph. |

---

### Task 1: Viewport queries — strict step, first/last interesting, land

**Files:**
- Modify: `src/viewport.rs:280-330` (beside `next_interesting` and `step_to_interesting`)
- Test: `src/lib.rs` (tests module, near `n_wraps_at_the_end_of_the_file` around line 7100)

**Interfaces:**
- Produces:
  - `pub(crate) fn next_interesting_strict(&self, backwards: bool) -> Option<usize>` — source index of the next interesting line strictly after (or before) the cursor, **no wrap**.
  - `pub(crate) fn first_interesting(&self, from_end: bool) -> Option<usize>` — source index of the first interesting line in the file, or the last when `from_end`.
  - `pub(crate) fn land_on(&mut self, target: usize)` — put the cursor on source line `target`, bringing the window with it. Returns nothing; a hidden target is a no-op.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/lib.rs`, after `n_wraps_at_the_end_of_the_file`:

```rust
    /// The strict step is what lets `n` know it has run out of hits in this
    /// file: unlike `next_interesting` it refuses to wrap.
    #[test]
    fn next_interesting_strict_does_not_wrap() {
        let mut app = app_over_file("strict_no_wrap", "hit\nplain\nhit\nplain\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.set_search("hit").expect("valid pattern");
        app.refresh_view();

        assert_eq!(app.next_interesting_strict(false), Some(2), "forward from 0");
        assert_eq!(app.next_interesting_strict(true), None, "nothing before 0");

        app.land_on(2);
        assert_eq!(cursor_source(&app), 2, "land_on moved the cursor");
        assert_eq!(app.next_interesting_strict(false), None, "nothing after 2");
        assert_eq!(app.next_interesting_strict(true), Some(0), "backward from 2");
    }

    #[test]
    fn first_interesting_finds_either_end() {
        let mut app = app_over_file("first_either_end", "plain\nhit\nplain\nhit\nplain\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.set_search("hit").expect("valid pattern");
        app.refresh_view();

        assert_eq!(app.first_interesting(false), Some(1));
        assert_eq!(app.first_interesting(true), Some(3));
    }

    #[test]
    fn first_interesting_is_none_without_hits() {
        let mut app = app_over_file("first_none", "plain\nplain\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.set_search("hit").expect("valid pattern");
        app.refresh_view();

        assert_eq!(app.first_interesting(false), None);
        assert_eq!(app.next_interesting_strict(false), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib next_interesting_strict first_interesting`
Expected: compile error, `no method named next_interesting_strict`.

- [ ] **Step 3: Implement the three methods and refactor `step_to_interesting`**

In `src/viewport.rs`, replace `step_to_interesting` and add the new methods directly after `next_interesting`:

```rust
    /// `next_interesting` without the wrap: `None` once the cursor is past the
    /// last interesting line (or before the first, going backwards). This is
    /// how `n` learns it has finished the file and should move to the next
    /// one rather than circle back.
    pub(crate) fn next_interesting_strict(&self, backwards: bool) -> Option<usize> {
        let verdicts = self.document.verdicts();
        let from = self.cursor_source();
        let interesting =
            |index: &usize| matches!(verdicts[*index], Verdict::Included(_) | Verdict::Searched);
        if backwards {
            (0..from).rev().find(interesting)
        } else {
            (from + 1..verdicts.len()).find(interesting)
        }
    }

    /// The first interesting line of the file — or the last, when
    /// `from_end`. Where a cross-file step lands.
    pub(crate) fn first_interesting(&self, from_end: bool) -> Option<usize> {
        let verdicts = self.document.verdicts();
        let interesting =
            |index: &usize| matches!(verdicts[*index], Verdict::Included(_) | Verdict::Searched);
        if from_end {
            (0..verdicts.len()).rev().find(interesting)
        } else {
            (0..verdicts.len()).find(interesting)
        }
    }

    /// Put the cursor on source line `target`, bringing the window with it.
    /// Quiet when the line is not visible in the current mode.
    pub(crate) fn land_on(&mut self, target: usize) {
        let Some(row) = self.document.visible_position(target) else {
            return;
        };
        self.place_cursor_on_visible_row(row);
    }

    /// Move the file view's cursor to the next interesting line, wrapping, if
    /// there is one. Quiet when there is not.
    pub(crate) fn step_to_interesting(&mut self, backwards: bool) {
        if let Some(target) = self.next_interesting(backwards) {
            self.land_on(target);
        }
    }
```

Keep the existing doc comment on `step_to_interesting` if it says more than the one above; only the body changes.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib next_interesting_strict first_interesting n_`
Expected: all PASS, including the existing `n_*` tests.

- [ ] **Step 5: Commit**

```bash
git add src/viewport.rs src/lib.rs
git commit -m "feat(viewport): strict and first/last interesting-line queries, land_on"
```

---

### Task 2: Navigator — step to the next `Match::Yes` entry

**Files:**
- Modify: `src/widgets/filenav.rs:370-410` (beside `repeat_search` / `step_to`)
- Test: `src/widgets/filenav.rs` tests module, after `n_steps_to_the_next_matching_file_when_no_search_is_active` (around line 1780)

**Interfaces:**
- Produces:
  - `pub(crate) fn step_to_match(&mut self, reverse: bool) -> Option<Action>` — select the next (previous) entry whose `matched` is `Match::Yes(_)`, wrapping, ignoring any filename search; returns `Action::Preview(path)` for it, or `None` when no entry matches. If the only match is the already-selected entry, it is selected again and `Preview` is still returned (the caller compares `selected_entry()` before and after).
  - `pub(crate) fn selected_name(&self) -> Option<String>` — the selected entry's file name, lossy.

- [ ] **Step 1: Write the failing tests**

```rust
    /// The cross-file step the file view's `n` uses: filter matches only,
    /// even when a filename search is active and would pick differently.
    #[test]
    fn step_to_match_ignores_the_filename_search() {
        let mut nav = nav_over("step_match", &["a.log", "b.log", "c.log"]);
        let (a, b, c) = (nav.files()[0].0, nav.files()[1].0, nav.files()[2].0);
        nav.set_answer(a, Match::Yes(Style::default()));
        nav.set_answer(b, Match::Yes(Style::default()));
        nav.set_answer(c, Match::No);
        nav.restyle();
        nav.search("c", false).expect("valid pattern");
        nav.select_entry(a);

        let action = nav.step_to_match(false);

        assert_eq!(nav.selected_entry(), Some(b), "went to the filter match, not the search hit");
        assert!(matches!(action, Some(Action::Preview(_))));
        assert_eq!(nav.selected_name().as_deref(), Some("b.log"));

        nav.step_to_match(false);
        assert_eq!(nav.selected_entry(), Some(a), "wraps past the unmatched c.log");

        nav.step_to_match(true);
        assert_eq!(nav.selected_entry(), Some(b), "reverse");
    }

    #[test]
    fn step_to_match_is_none_when_nothing_matches() {
        let mut nav = nav_over("step_match_none", &["a.log", "b.log"]);
        let a = nav.files()[0].0;
        nav.set_answer(a, Match::No);
        nav.restyle();
        nav.select_entry(a);

        assert!(nav.step_to_match(false).is_none());
        assert_eq!(nav.selected_entry(), Some(a), "selection untouched");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib step_to_match`
Expected: compile error, `no method named step_to_match`.

- [ ] **Step 3: Implement**

In `src/widgets/filenav.rs`, directly after `repeat_search`:

```rust
    /// The next (or previous) entry the filters selected, wrapping, whatever
    /// the filename search says. This is the step the file view's `n` takes
    /// when it has run out of interesting lines in the current file, and the
    /// step `.`/`,` take unconditionally: the loop those keys drive is about
    /// content, so a filename hit with no interesting lines is not a stop.
    pub(crate) fn step_to_match(&mut self, reverse: bool) -> Option<Action> {
        self.step_to(reverse, |entry| matches!(entry.matched, Match::Yes(_)))
    }

    /// The selected entry's file name, for reporting.
    pub(crate) fn selected_name(&self) -> Option<String> {
        let path = self.selected_path()?;
        Some(path.file_name()?.to_string_lossy().into_owned())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib step_to_match`
Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/widgets/filenav.rs
git commit -m "feat(filenav): step_to_match and selected_name for cross-file stepping"
```

---

### Task 3: `n`/`N` cross file boundaries; `n` from the filter pane

**Files:**
- Modify: `src/lib.rs` — `App` struct (around line 267, beside `peek`), `App::new` initialiser, the `n`/`N` arm (around line 1020), and new methods next to `promote_truncated_preview` (around line 1227)
- Modify: `src/help.rs:205-207` (`n`/`N` row text)
- Modify: `README.md:670` (`n`/`N` row in the file view table)
- Test: `src/lib.rs` tests module

**Interfaces:**
- Consumes: `next_interesting_strict`, `first_interesting`, `land_on` (Task 1); `FileNav::step_to_match`, `FileNav::selected_name` (Task 2).
- Produces:
  - `fn step_interesting(&mut self, backwards: bool)` on `App` — the whole `n`/`N` behaviour.
  - `fn cross_file(&mut self, backwards: bool) -> bool` on `App` — select, load and land in the next matching file; `false` when there is no *other* matching file. Sets `self.crossing` and a status message on success.
  - `struct Crossing { backwards: bool, name: String }` and field `crossing: Option<Crossing>` on `App`, cleared wherever `status_message` is cleared.

- [ ] **Step 1: Add a test fixture helper**

In the `tests` module of `src/lib.rs`, next to `app_over` (line 2357):

```rust
    /// `app_over`, with real contents. The app starts on a placeholder path
    /// that does not exist, so nothing is loaded until `open_file`.
    fn app_over_files(name: &str, files: &[(&str, &str)]) -> App<'static> {
        claim_fixture_dir(name);
        let dir = std::path::Path::new("target/test-appdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        for (file, body) in files {
            fs::write(dir.join(file), body).expect("write fixture");
        }
        App::new(&Config {
            path: dir.join("placeholder").display().to_string(),
            ..Config::default()
        })
    }

    /// Select the `row`th file in the navigator and load it into the view,
    /// the way `Enter` in the navigator would.
    fn open_file(app: &mut App, row: usize) {
        let (index, path) = app.nav.files()[row].clone();
        app.nav.select_entry(index);
        app.perform(Action::Load(path));
    }

    /// Mark the `row`th file as matching (`yes`) or not, through the scan
    /// result channel — the path the real scanner uses.
    fn mark(app: &mut App, tx: &Sender<scan::Scanned>, row: usize, yes: bool) {
        let seen = if yes { vec![0b1] } else { vec![0] };
        tx.send(scanned(app, row, seen, true)).expect("send");
        app.drain_scan_results();
    }

    /// The file the view is showing, by name.
    fn shown(app: &App) -> String {
        app.view
            .filename()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Two matching logs, one unmatched between them, scan answers in place.
    /// Returns the app with `a.log` loaded and the view focused.
    fn app_over_matching_logs(name: &str) -> (App<'static>, Sender<scan::Scanned>) {
        let mut app = app_over_files(
            name,
            &[
                ("a.log", "plain\nhit a1\nhit a2\n"),
                ("b.log", "plain\nplain\n"),
                ("c.log", "hit c1\nplain\nhit c2\n"),
            ],
        );
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("hit").expect("valid pattern");
        app.refresh_scan(false);
        mark(&mut app, &tx, 0, true);
        mark(&mut app, &tx, 1, false);
        mark(&mut app, &tx, 2, true);
        open_file(&mut app, 0);
        key(&mut app, KeyCode::Char('t'));
        (app, tx)
    }
```

`Sender` and `scan` are already in scope in this module (see `record_scans` at line 8919). If `scanned` is defined below this point, that is fine: Rust functions in a module are order-independent.

- [ ] **Step 2: Write the failing tests**

```rust
    /// The loop-collapsing change (#120 §1): `n` past the last hit in a file
    /// goes to the first hit of the next file the filters selected, skipping
    /// files the scan said no to.
    #[test]
    fn n_at_the_last_hit_crosses_to_the_next_matching_file() {
        let (mut app, _tx) = app_over_matching_logs("cross_next");
        key(&mut app, KeyCode::Char('n'));
        key(&mut app, KeyCode::Char('n'));
        assert_eq!(cursor_source(&app), 2, "sanity: on the last hit of a.log");

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(shown(&app), "c.log", "did not cross, or stopped on the unmatched b.log");
        assert_eq!(cursor_source(&app), 0, "did not land on the first hit");
        assert_eq!(
            app.nav.selected_name().as_deref(),
            Some("c.log"),
            "the navigator's selection did not follow"
        );
        assert_eq!(
            app.status_message.as_ref().map(|m| m.text.as_str()),
            Some("next file · c.log")
        );
        assert!(app.crossing.is_some());
    }

    #[test]
    fn capital_n_at_the_first_hit_crosses_to_the_previous_files_last_hit() {
        let (mut app, _tx) = app_over_matching_logs("cross_prev");
        open_file(&mut app, 2);
        assert_eq!(cursor_source(&app), 0, "sanity: on c.log's first hit");

        key(&mut app, KeyCode::Char('N'));

        assert_eq!(shown(&app), "a.log");
        assert_eq!(cursor_source(&app), 2, "did not land on the last hit");
        assert_eq!(
            app.status_message.as_ref().map(|m| m.text.as_str()),
            Some("previous file · a.log")
        );
    }

    /// With no other matching file, `n` wraps within the file as it always
    /// has, and nothing claims a crossing happened.
    #[test]
    fn n_wraps_within_the_file_when_no_other_file_matches() {
        let (mut app, tx) = app_over_matching_logs("cross_alone");
        mark(&mut app, &tx, 2, false);
        key(&mut app, KeyCode::Char('n'));
        key(&mut app, KeyCode::Char('n'));
        assert_eq!(cursor_source(&app), 2);

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(shown(&app), "a.log");
        assert_eq!(cursor_source(&app), 1, "did not wrap to the first hit");
        assert!(app.crossing.is_none());
    }

    /// A filename search in the navigator does not redirect the content loop.
    #[test]
    fn n_crossing_ignores_the_navigator_filename_search() {
        let (mut app, _tx) = app_over_matching_logs("cross_ignores_search");
        app.nav.search("b", false).expect("valid pattern");
        open_file(&mut app, 0);
        key(&mut app, KeyCode::Char('n'));
        key(&mut app, KeyCode::Char('n'));

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(shown(&app), "c.log", "followed the filename search to b.log");
    }

    /// The in-file step still comes first: `n` with hits remaining in this
    /// file must not cross.
    #[test]
    fn n_prefers_the_next_hit_in_this_file() {
        let (mut app, _tx) = app_over_matching_logs("cross_prefers_local");

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(shown(&app), "a.log");
        assert_eq!(cursor_source(&app), 1);
        assert!(app.crossing.is_none());
    }

    /// #120 §2 decision (b): the filter pane forwards `n` to the view.
    #[test]
    fn n_from_the_filter_pane_acts_on_the_file_view() {
        let (mut app, _tx) = app_over_matching_logs("cross_from_filters");
        key(&mut app, KeyCode::Char('f'));

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(cursor_source(&app), 1, "n was swallowed by the filter pane");
        assert_eq!(app.focus, Focus::Filters, "focus moved");
    }

    /// The notice and the status line are cleared by the next keypress, like
    /// every other status message.
    #[test]
    fn a_crossing_is_forgotten_on_the_next_key() {
        let (mut app, _tx) = app_over_matching_logs("cross_forgotten");
        key(&mut app, KeyCode::Char('n'));
        key(&mut app, KeyCode::Char('n'));
        key(&mut app, KeyCode::Char('n'));
        assert!(app.crossing.is_some(), "sanity");

        key(&mut app, KeyCode::Char('j'));

        assert!(app.crossing.is_none());
        assert!(app.status_message.is_none());
    }
```

The existing test `n_in_the_navigator_does_not_move_the_file_view_cursor` must keep passing: the navigator's own `n` is untouched.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib cross_ n_from_the_filter_pane`
Expected: compile error on `app.crossing` / `selected_name`, or the crossing tests fail with `shown == "a.log"`.

- [ ] **Step 4: Add the `Crossing` type and field**

In `src/lib.rs`, after `struct PeekState` (line 318):

```rust
/// A cross-file step that just happened, for the notice over the file view
/// and the accent on its title. Lives exactly as long as a `StatusMessage`:
/// until the next keypress.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Crossing {
    backwards: bool,
    name: String,
}
```

In the `App` struct, after `peek: Option<PeekState>,`:

```rust
    /// The cross-file step `n`, `N`, `.` or `,` just made, if any (#120).
    crossing: Option<Crossing>,
```

In `App::new`, wherever `peek: None,` is written, add `crossing: None,` beside it.

In `dispatch_event` (line 798), extend the keypress clear:

```rust
        if matches!(event, event::Event::Key(_)) {
            self.status_message = None;
            self.crossing = None;
        }
```

- [ ] **Step 5: Add `step_interesting` and `cross_file`**

In `src/lib.rs`, directly after `promote_truncated_preview` (line 1227):

```rust
    /// `n`/`N` in the file view: the next interesting line in this file, else
    /// the first interesting line of the next file the filters selected, else
    /// (when this is the only such file) wrap within it as `n` always has.
    ///
    /// The in-file step comes first so the loop the key drives — every hit in
    /// every file — never skips a hit. The cross-file step is what makes it a
    /// single loop rather than one per file (#120 §1).
    fn step_interesting(&mut self, backwards: bool) {
        // `n`/`N` bypass the widget's own `handle_events`, which is where a
        // truncated preview normally promotes itself on first interaction —
        // see `promote_truncated_preview`, which `apply_search` also calls
        // for the same reason.
        self.promote_truncated_preview();
        if let Some(target) = self.next_interesting_strict(backwards) {
            self.land_on(target);
            return;
        }
        if !self.cross_file(backwards) {
            self.step_to_interesting(backwards);
        }
    }

    /// Select, load and land in the next (previous) file the filters
    /// selected. `false` when there is no *other* such file — the navigator
    /// wraps, so "the only match is the one we are in" comes back as an
    /// unchanged selection rather than `None`.
    ///
    /// Reports the crossing three ways, all gone by the next keypress: the
    /// status row, the notice `render` paints over the file view, and the
    /// accent on the view's title. Log files look alike, and a step that
    /// silently changed which one is on screen would be worse than no step.
    fn cross_file(&mut self, backwards: bool) -> bool {
        let before = self.nav.selected_entry();
        let Some(action) = self.nav.step_to_match(backwards) else {
            return false;
        };
        if self.nav.selected_entry() == before {
            return false;
        }
        self.perform(action);
        self.promote_truncated_preview();
        if let Some(target) = self.first_interesting(backwards) {
            self.land_on(target);
        }
        let name = self.nav.selected_name().unwrap_or_default();
        let direction = if backwards { "previous file" } else { "next file" };
        self.report(&format!("{direction} · {name}"), false);
        self.crossing = Some(Crossing { backwards, name });
        true
    }
```

- [ ] **Step 6: Rewrite the `n`/`N` arm**

Replace the arm at `src/lib.rs` around line 1020 (the one guarded by `self.focus == Focus::View`) with:

```rust
                // Scoped away from the navigator rather than global: `n` in
                // the navigator is the navigator's key (next filename-search
                // hit, else next matching file) and stays that way. The
                // filter pane forwards it to the view — the pane has no
                // "next" of its own, and the user wants to see the effect of
                // the filter they just touched (#120 §2).
                //
                // Not guarded with `.is_empty()`, unlike `/` above: crossterm
                // attaches SHIFT to every uppercase character a real terminal
                // sends, so an `is_empty()` guard would make `N` unreachable
                // outside a test harness that never sets it. CONTROL/ALT is
                // the same tolerance `H` uses just below, for the same reason.
                KeyCode::Char(c @ ('n' | 'N'))
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && self.focus != Focus::Nav =>
                {
                    self.step_interesting(c == 'N');
                    return;
                }
```

Keep whichever of the original comment lines still apply; the block above is complete on its own.

- [ ] **Step 7: Update the `n`/`N` rows in `KEYMAP` and the README**

`src/help.rs` line 205-207:

```rust
            Binding {
                keys: &["n", "N"],
                action: "Next / previous interesting line, crossing into the next matching file",
            },
```

If the overlay layout test complains about width, shorten to `"Next / previous interesting line — crosses files"`.

`README.md` line 670:

```markdown
| `n` / `N` | Move to the next / previous *interesting* line. Past the last one in this file, move to the first interesting line of the next file the filters match (the last, for `N`), skipping files that don't. Also works from the filter pane |
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --lib cross_ n_ every_bound_key`
Expected: all PASS.

- [ ] **Step 9: Commit**

```bash
git add src/lib.rs src/help.rs README.md
git commit -m "feat(keys): n/N cross file boundaries; n from the filter pane acts on the view (#120)"
```

---

### Task 4: A jump out of a peeked file restores the peek first

**Files:**
- Modify: `src/lib.rs` — `step_interesting` (Task 3)
- Test: `src/lib.rs` tests module

**Interfaces:**
- Consumes: `self.peek`, `toggle_peek` (existing), `step_interesting` (Task 3).

- [ ] **Step 1: Write the failing tests**

```rust
    /// #120 §4: with every filter disabled by the peek, a step would find no
    /// interesting line and cross files at once. Put the filters back first.
    #[test]
    fn n_while_peeked_restores_the_peek_before_moving() {
        let (mut app, _tx) = app_over_matching_logs("peek_then_n");
        key(&mut app, KeyCode::Char(' '));
        assert!(app.peek.is_some(), "sanity: peeking");

        key(&mut app, KeyCode::Char('n'));

        assert!(app.peek.is_none(), "still peeking");
        assert_eq!(shown(&app), "a.log", "crossed files instead of stepping");
        assert_eq!(cursor_source(&app), 1);
    }

    /// In-file motions leave the peek alone: that is what peeking is for.
    #[test]
    fn j_while_peeked_keeps_the_peek() {
        let (mut app, _tx) = app_over_matching_logs("peek_then_j");
        key(&mut app, KeyCode::Char(' '));

        key(&mut app, KeyCode::Char('j'));

        assert!(app.peek.is_some());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib peek_then`
Expected: `n_while_peeked_restores_the_peek_before_moving` FAILS on `still peeking`; `j_while_peeked_keeps_the_peek` passes already.

- [ ] **Step 3: Implement**

At the top of `step_interesting` in `src/lib.rs`, before `promote_truncated_preview`:

```rust
        // A jump that leaves the peeked context has nothing to come back to,
        // and with every filter disabled by the peek the step would find no
        // interesting line and cross files at once. Restore first (#120 §4).
        if self.peek.is_some() {
            self.toggle_peek();
        }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib peek`
Expected: all PASS, including the existing `peek_*` tests.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat(keys): n/N restore a peek before moving (#120)"
```

---

### Task 5: `.` and `,` skip to the next / previous matching file from any pane

**Files:**
- Modify: `src/lib.rs` — new global arm beside the `n`/`N` arm; new method beside `cross_file`
- Modify: `src/help.rs` — new row in the Global section (after the `space` row, line 124-127)
- Modify: `README.md` — new row in the Global table (after `space`, line 308)
- Test: `src/lib.rs` tests module

**Interfaces:**
- Consumes: `cross_file` (Task 3).
- Produces: `fn skip_file(&mut self, backwards: bool)` on `App`.

- [ ] **Step 1: Write the failing tests**

```rust
    /// `.`/`,` are global: the same step from all three panes, with focus
    /// left where it was.
    #[test]
    fn dot_and_comma_step_files_from_every_pane() {
        let (mut app, _tx) = app_over_matching_logs("skip_every_pane");

        key(&mut app, KeyCode::Char('.'));
        assert_eq!(shown(&app), "c.log", "from the view");
        assert_eq!(cursor_source(&app), 0);

        key(&mut app, KeyCode::Char('e'));
        key(&mut app, KeyCode::Char('.'));
        assert_eq!(shown(&app), "a.log", "from the navigator (wrapped)");
        assert_eq!(app.focus, Focus::Nav);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char(','));
        assert_eq!(shown(&app), "c.log", "from the filter pane, backwards");
        assert_eq!(cursor_source(&app), 2, "`,` lands on the last hit");
        assert_eq!(app.focus, Focus::Filters);
    }

    /// `.` does not wait for the current file to be exhausted.
    #[test]
    fn dot_skips_the_rest_of_the_current_file() {
        let (mut app, _tx) = app_over_matching_logs("skip_rest");
        assert_eq!(cursor_source(&app), 0, "sanity: hits remain below");

        key(&mut app, KeyCode::Char('.'));

        assert_eq!(shown(&app), "c.log");
        assert!(app.crossing.is_some());
    }

    #[test]
    fn dot_with_no_other_matching_file_says_so() {
        let (mut app, tx) = app_over_matching_logs("skip_alone");
        mark(&mut app, &tx, 2, false);

        key(&mut app, KeyCode::Char('.'));

        assert_eq!(shown(&app), "a.log");
        assert_eq!(
            app.status_message.as_ref().map(|m| m.text.as_str()),
            Some("no other file matches")
        );
    }

    #[test]
    fn dot_while_peeked_restores_the_peek_first() {
        let (mut app, _tx) = app_over_matching_logs("skip_peeked");
        key(&mut app, KeyCode::Char(' '));

        key(&mut app, KeyCode::Char('.'));

        assert!(app.peek.is_none());
        assert_eq!(shown(&app), "c.log");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib dot_`
Expected: FAIL — `shown == "a.log"` (the keys fall through to the panes and do nothing).

- [ ] **Step 3: Implement `skip_file` and the arm**

After `cross_file` in `src/lib.rs`:

```rust
    /// `.`/`,`: the cross-file half of `n`/`N`, without first exhausting the
    /// current file. Global, so the loop can skip a file from any pane.
    fn skip_file(&mut self, backwards: bool) {
        if self.peek.is_some() {
            self.toggle_peek();
        }
        if !self.cross_file(backwards) {
            self.report("no other file matches", false);
        }
    }
```

Add a global arm directly before the `n`/`N` arm in `dispatch_event`:

```rust
                // Global, unlike `n`: skipping a file is the outer loop of
                // the review workflow, and it should not matter which pane
                // the inner loop left focus in. The keycaps carry the
                // mnemonic — `<` and `>` (#120 §2).
                KeyCode::Char(c @ ('.' | ',')) if key.modifiers.is_empty() => {
                    self.skip_file(c == ',');
                    return;
                }
```

- [ ] **Step 4: Document the keys**

`src/help.rs`, in the Global section after the `space` binding:

```rust
            Binding {
                keys: &[".", ","],
                action: "Next / previous file the filters match",
            },
```

`README.md`, Global table, after the `space` row:

```markdown
| `.` / `,` | Skip to the next / previous file the filters match, landing on its first / last interesting line. Works from every pane; focus stays put. The keycaps say `>` and `<` |
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib dot_ every_bound_key`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/help.rs README.md
git commit -m "feat(keys): . and , skip to the next / previous matching file from any pane (#120)"
```

---

### Task 6: The crossing is visible — centred notice and accented title

**Files:**
- Modify: `src/widgets/fileview.rs` — struct fields (around line 300), `new`, `render` (line 1274)
- Modify: `src/lib.rs` — `render` (the `impl Widget for &mut App` block, around line 2085-2210)
- Test: `src/lib.rs` tests module

**Interfaces:**
- Consumes: `self.crossing` (Task 3).
- Produces: `pub(crate) fn set_title_accent(&mut self, on: bool)` on `FileView`.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Log files look alike. A crossing paints a notice over the view and
    /// accents its title; the next key clears both (#120 §1).
    #[test]
    fn a_crossing_paints_a_notice_over_the_file_view() {
        let (mut app, _tx) = app_over_matching_logs("notice_paint");
        key(&mut app, KeyCode::Char('.'));

        let screen = rendered(&mut app);
        assert!(
            screen.contains("▼ next file · c.log"),
            "no notice on screen:\n{screen}"
        );

        key(&mut app, KeyCode::Char('j'));
        let screen = rendered(&mut app);
        assert!(!screen.contains("next file"), "notice survived a keypress:\n{screen}");
    }

    #[test]
    fn a_backwards_crossing_points_up() {
        let (mut app, _tx) = app_over_matching_logs("notice_up");
        key(&mut app, KeyCode::Char(','));

        let screen = rendered(&mut app);
        assert!(screen.contains("▲ previous file · c.log"), "{screen}");
    }

    #[test]
    fn a_crossing_accents_the_view_title() {
        let (mut app, _tx) = app_over_matching_logs("notice_title");
        key(&mut app, KeyCode::Char('.'));
        let mut buf = Buffer::empty(AREA);
        (&mut app).render(AREA, &mut buf);

        // The title sits on the view's top border; find the first cell of
        // the file name and read its style.
        let title_cell = (0..AREA.width)
            .map(|x| buf[(x, 0)].clone())
            .find(|cell| cell.symbol() == "c")
            .expect("the title is drawn on the top border");
        assert_eq!(title_cell.fg, Color::Yellow, "title not accented");

        key(&mut app, KeyCode::Char('j'));
        let mut buf = Buffer::empty(AREA);
        (&mut app).render(AREA, &mut buf);
        let title_cell = (0..AREA.width)
            .map(|x| buf[(x, 0)].clone())
            .find(|cell| cell.symbol() == "c")
            .expect("title");
        assert_ne!(title_cell.fg, Color::Yellow, "accent survived a keypress");
    }
```

`rendered` (line 5766) renders at `AREA` (120×10) and joins the rows. The notice is one bordered row, three rows tall, centred in the file view's area; it fits.

If the `c` search in the title test collides with a `c` elsewhere on row 0 (the navigator's border title is the directory name, which contains `notice_title`), search from the divider column instead: `(app.divider..AREA.width)`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib notice_`
Expected: FAIL — no notice text on screen; title not yellow.

- [ ] **Step 3: Add the title accent to `FileView`**

In `src/widgets/fileview.rs`, add a field after `active: bool,`:

```rust
    /// Draw the title in the accent colour for this frame. Set by `App` for
    /// the one keypress after a cross-file step, so the eye catches that the
    /// file changed even when the notice is dismissed by the same key that
    /// raised it (#120).
    title_accent: bool,
```

Initialise `title_accent: false,` in `FileView::new` beside `active: false,`.

Add the setter next to `set_active` (line 388):

```rust
    pub(crate) fn set_title_accent(&mut self, on: bool) {
        self.title_accent = on;
    }
```

In `render`, replace the `set_block` call:

```rust
        // The one place the path is rendered, and the one place a lossy
        // conversion is both correct and harmless — see the `filename` field.
        let title = self.filename.display().to_string();
        let title = if self.title_accent {
            ratatui::text::Line::from(ratatui::text::Span::styled(
                title,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            ratatui::text::Line::from(title)
        };
        self.textarea
            .set_block(crate::widgets::pane_block(title, self.active));
```

`pane_block` already takes `impl Into<Line>`, so both branches fit.

- [ ] **Step 4: Paint the notice in `App::render`**

In `src/lib.rs`, in `impl Widget for &mut App`, the two branches that call `render_pane(Focus::View, …)` need the view's area afterwards. Change the zoomed branch to:

```rust
            self.set_active_pane();
            self.view.set_title_accent(self.crossing.is_some());
            self.render_pane(zoomed, area, buf);
            if zoomed == Focus::View {
                self.render_crossing(area, buf);
            }
```

and the split branch to:

```rust
            self.set_active_pane();
            self.view.set_title_accent(self.crossing.is_some());
            self.render_pane(Focus::Nav, nav_area, buf);
            self.render_pane(Focus::View, right, buf);
            self.render_pane(Focus::Filters, filter_area, buf);
            self.render_crossing(right, buf);
```

Add the method to `impl App` next to `render_pane` (line 2041):

```rust
    /// The one-line notice a cross-file step leaves over the file view until
    /// the next keypress. Centred, bordered, and cleared underneath so it
    /// reads over any text. Nothing is drawn when there was no crossing.
    fn render_crossing(&self, view_area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Block, Clear, Paragraph};
        let Some(crossing) = &self.crossing else {
            return;
        };
        let text = format!(
            "{} {} · {}",
            if crossing.backwards { "▲" } else { "▼" },
            if crossing.backwards { "previous file" } else { "next file" },
            crossing.name
        );
        let width = u16::try_from(UnicodeWidthStr::width(text.as_str()) + 4)
            .unwrap_or(u16::MAX)
            .min(view_area.width);
        if width < 5 || view_area.height < 3 {
            return;
        }
        let x = view_area.x + (view_area.width - width) / 2;
        let y = view_area.y + (view_area.height - 3) / 2;
        let area = Rect { x, y, width, height: 3 };
        Clear.render(area, buf);
        Paragraph::new(text)
            .centered()
            .block(
                Block::bordered().border_style(Style::default().fg(Color::Yellow)),
            )
            .render(area, buf);
    }
```

`UnicodeWidthStr` is already imported in `src/lib.rs` (used by `elide_left`). If `Paragraph::centered` is not available in the vendored ratatui version, use `.alignment(ratatui::layout::Alignment::Center)`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib notice_ cross_ dot_`
Expected: all PASS. If an existing render test asserts an exact screen after an `n` that now crosses files, adjust that test's fixture so it does not (only one matching file) rather than weakening the assertion.

- [ ] **Step 6: Commit**

```bash
git add src/widgets/fileview.rs src/lib.rs
git commit -m "feat(view): centred notice and accented title for one keypress after a cross-file step (#120)"
```

---

### Task 7: `[` and `]` page the file view from any pane

**Files:**
- Modify: `src/lib.rs` — pane dispatch (around line 1148-1175) and a new global arm
- Modify: `src/help.rs` — Global section gains a `[`/`]` row; the file view's two rows at lines 221-226 drop `[` and `]`
- Modify: `README.md` — Global table gains a row; file view rows at lines 673-674 drop `[` and `]`
- Test: `src/lib.rs` tests module

**Interfaces:**
- Produces: `fn forward_to_view(&mut self, event: event::Event)` on `App` — hand one event to the file view with the truncation resync and window check the focused-dispatch path already does.

- [ ] **Step 1: Write the failing test**

```rust
    /// `[`/`]` are global so a peeked file can be paged without leaving the
    /// pane the loop is being driven from (#120 §3).
    #[test]
    fn brackets_page_the_file_view_from_the_navigator_and_filter_pane() {
        let body = numbered_lines(400);
        let mut app = app_over_file("brackets_global", &body);
        let mut buf = Buffer::empty(AREA);
        (&mut app).render(AREA, &mut buf);
        key(&mut app, KeyCode::Char('e'));
        assert_eq!(cursor_source(&app), 0, "sanity");

        key(&mut app, KeyCode::Char(']'));
        (&mut app).render(AREA, &mut buf);
        let after_page = cursor_source(&app);
        assert!(after_page > 0, "] from the navigator did not page the view");
        assert_eq!(app.focus, Focus::Nav, "focus moved");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('['));
        (&mut app).render(AREA, &mut buf);
        assert!(cursor_source(&app) < after_page, "[ from the filter pane did not page up");
        assert_eq!(app.focus, Focus::Filters);
    }
```

`numbered_lines` exists in this module (used by `page_up_keeps_paging_a_full_screen_after_the_window_is_rebuilt`, line 3126). The renders between presses are what apply the pending scroll, exactly as that test does.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib brackets_global`
Expected: FAIL on `] from the navigator did not page the view`.

- [ ] **Step 3: Extract `forward_to_view` and add the arm**

In `dispatch_event`'s tail (around line 1148-1175), the `Focus::View` arm currently reads `self.view.handle_events(event.into()); None` inside a `match self.focus` that also computes `was_truncated` before and resyncs after. Extract the view half into a method next to `promote_truncated_preview`:

```rust
    /// Hand one event to the file view, whichever pane has focus, with the
    /// same after-care the focused dispatch gives it: a truncated preview
    /// that promoted itself on this keypress is resynced without re-reading
    /// the file, and the window is checked after a page at its edge.
    fn forward_to_view(&mut self, event: event::Event) {
        let was_truncated = self.file_view_truncated();
        self.view.handle_events(event.into());
        if was_truncated && !self.file_view_truncated() {
            self.sync_document();
            self.refresh_view();
        }
        self.ensure_window();
    }
```

Then make the `Focus::View` dispatch arm call it. The existing block becomes:

```rust
        let was_truncated = self.file_view_truncated();
        let action = match self.focus {
            Focus::Nav => self.nav.handle_events(event),
            Focus::View => {
                self.forward_to_view(event);
                return;
            }
            Focus::Filters => None,
        };
        if let Some(action) = action {
            self.perform(action);
        } else if was_truncated && !self.file_view_truncated() {
            self.sync_document();
            self.refresh_view();
        }
        self.ensure_window();
```

(The `was_truncated`/resync after the match now only serves the navigator arm; leave it, since `perform` can change truncation too.)

Add a global arm directly before the `.`/`,` arm:

```rust
                // Global: the file view is what gets read during the review
                // loop, and paging it should not require focusing it. The
                // view's own `[`/`]` arms stay; this reaches them from the
                // other two panes (#120 §3).
                KeyCode::Char('[' | ']') if key.modifiers.is_empty() && self.focus != Focus::View => {
                    self.forward_to_view(event);
                    return;
                }
```

`event` must still be in scope at that point; the arms above are inside `if let event::Event::Key(key) = event` — check whether `event` was moved. If it was, bind `let event = event::Event::Key(key);` inside the arm instead.

- [ ] **Step 4: Document the move**

`src/help.rs`, Global section, after the `.`/`,` row:

```rust
            Binding {
                keys: &["[", "]"],
                action: "Page the file view up / down, from any pane",
            },
```

and change the file view's two rows to `keys: &["Ctrl-b", "PageUp"]` and `keys: &["Ctrl-f", "PageDown"]`.

`README.md`, Global table after the `.`/`,` row:

```markdown
| `[` / `]` | Page the file view up / down, whichever pane has focus — so a peeked file can be skimmed from the navigator |
```

and the file view rows at 673-674 become `` `Ctrl-b` / `PageUp` `` and `` `Ctrl-f` / `PageDown` ``.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib brackets page_up every_bound_key`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/help.rs README.md
git commit -m "feat(keys): [ and ] page the file view from any pane (#120)"
```

---

### Task 8: README reasoning, full verification

**Files:**
- Modify: `README.md` — the reasoning paragraphs after the Keybindings tables (the section that explains `space` and `h`/`l`; search for "deliberate trade" near line 700-760)

- [ ] **Step 1: Add the reasoning paragraph**

After the existing paragraph that explains why `space` is global, add:

```markdown
`n` crosses file boundaries because recon's central workflow is a loop over
every interesting line in every interesting file, and running it as two loops
cost two focus keys per file (`e n t`). With the crossing, the whole loop is
`u`, then `n n n …`, with `space` to peek — focus never leaves the file view.
This is vim's quickfix model (`:cnext`) and the shape of `grep -n` output. `j`/`k`
stay bounded to the file, so "walk this file's hits" and "walk every hit
everywhere" are both available. The crossing uses the filters' answer for each
file and ignores the navigator's filename search: a filename hit with no
interesting lines has nowhere to land.

`space` stays the peek rather than becoming the filter pane's toggle: peek is
the key interleaved most often with `n`, `j` and `k`, and the thumb is the only
key that alternates hands against a right-hand vim vocabulary without leaving
the home position. Toggling a filter is a setup action, and `Enter` already
does it.
```

- [ ] **Step 2: Run everything**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: fmt makes no changes (or only in files this plan touched), clippy clean, all tests pass. If `title_shows_the_current_directory` fails, check the worktree path length first (see memory: it fails on long branch names and is not a regression).

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs(readme): why n crosses files and why space stays the peek (#120)"
```

---

## Self-review

**Spec coverage** (spec sections 1–4):

- §1 in-file step first, cross-file second, wrap fallback — Task 3. Interesting-file = `Match::Yes` only, filename search ignored — Task 2 + test in Task 3. Unknown skipped — follows from the `Match::Yes(_)` predicate. Wrapping via the navigator — Task 3 `cross_file`. Crossing cues (notice, title, status row) — Tasks 3 and 6. Navigator selection follows — Task 3. First/last interesting line — Task 1. Navigator `n` unchanged — the arm's `!= Focus::Nav` guard, existing test. Filter pane forwards `n` — Task 3.
- §2 `.`/`,` — Task 5.
- §3 `[`/`]` global — Task 7.
- §4 peek stays; peek-then-move for `n`/`N`/`.`/`,`, in-file motions leave it — Tasks 4 and 5. The `space`-to-`a` reversal needs no code.
- Implementation notes: cross-file step in `App` (Task 3); truncation promoted before landing (Task 3); one transient slot shared with the status message (Task 3 clears `crossing` with `status_message`); `/` from the filter pane is PR 2, not here.

**Not in this PR** (spec "Landing" items 2–5): `u`, `Shift-Tab`, shared list motions, `Esc` layering, `/` from the filter pane, digit toggles, chain return-focus, hints, `*`, the `KEYMAP` regroup.

**Type consistency:** `step_to_match(reverse: bool)` (Task 2) is called as `step_to_match(backwards)` (Task 3) — same meaning, `true` = previous. `first_interesting(from_end)` is called with `backwards` — `N` lands on the last line, `n` on the first, which is what the spec says. `Crossing { backwards, name }` fields are read by `render_crossing` (Task 6) under those names.
