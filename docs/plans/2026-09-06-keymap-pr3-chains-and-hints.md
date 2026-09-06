# Keymap PR 3: Chains return focus, wrong-pane hints

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `f i … Enter`, `f x … Enter` and `f c … Enter` from another pane return focus to that pane and then act as `n`; a filter-pane verb pressed in another pane, or a navigator verb pressed in the filter pane, says so on the status row instead of doing nothing.

**Architecture:** One new `App` field, `chain_origin: Option<Focus>`, set by the `f` arm when focus actually moves and cleared by every other focus change, by a prompt cancel, and by a second `f`. The prompt-commit path in `handle_search_key` consumes it for the three filter prompts and then dispatches a synthetic `n` so each pane's own "next interesting" meaning applies. Hints are one global arm for seven letters guarded on "not the filter pane" (those letters are unbound in the other two panes), plus two arms in `handle_filter_key` for `h`/`l`. Both use the existing one-keypress `status_message`.

**Tech Stack:** Rust, ratatui 0.30, crossterm. Tests are `#[cfg(test)]` in `src/lib.rs`, driven by `key(&mut app, KeyCode::…)`, `typed(&mut app, "…")`, `app_over_file`, `app_over_matching_logs`, `cursor_source`, `shown`.

**Spec:** `docs/specs/2026-09-05-keymap-reconciliation-design.md` — §8 (return focus, decision (a)) and §9 (hints). This plan is "Landing" item 3. Base: `main` at ce0467c or later (PR 2 merged).

## Global Constraints

- Every `KeyCode::Char('x')` arm needs its character documented somewhere in `KEYMAP` (`src/help.rs`) or `every_bound_key_is_documented` fails. The hint letters (`i x c d m a s h l`) are already documented by the pane rows, so no new `KEYMAP` rows are needed for them; the test checks the character, not the section.
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` must pass.
- Nothing goes silent: every key that does nothing reports on the status row via `self.report(text, false)`.
- Chains that return: exactly `PromptKind::Filter`, `PromptKind::Exclude`, `PromptKind::Edit` commits. `Search`, `EditSearch` and `SaveSet` never return. `f d`, `f Enter`, `f m`, `f a`, `f s`, `f R` and plain `f` never return.
- The origin is cleared by any focus change that is not the `f` arm (`Tab`, `Shift-Tab`, `e`, `t`, `b`, `z`, mouse), by a prompt cancel (`Esc`, or backspacing past the start), and by pressing `f` while the pane already has focus (`f f` is the sticky gesture, per the user's decision on §4).
- After a return, the app behaves as if `n` had been pressed in the origin pane: from the view, the next interesting line (crossing files, PR 1); from the navigator, the navigator's own `n`. Implement by dispatching a synthetic `n` key event rather than duplicating either.
- `cargo test` takes one filter; run named filters one at a time. Fixture dir names unique, no `filters` substring, no case-only variants.
- **Test migration rule (Task 1):** existing tests that press `f`, commit `i`/`x`/`c`, and then press more filter-pane keys start with focus on the navigator (that is `app_over_file`'s default) and will now return there. Fix each by inserting `key(&mut app, KeyCode::Char('f'));` immediately after the committing `Enter`, which is exactly the keystroke a user makes under the new rule. Never fix one by removing or weakening an assertion, and never by starting the test in the filter pane unless the test is about the pane's own initial state. List every migrated test in the report.

---

## File map

| File | Change |
|---|---|
| `src/lib.rs` | `chain_origin` field; `f` arm sets/clears it; `reveal_and_focus`, `focus_next`, `focus_prev`, `zoom_file_view` clear it; `handle_search_key` clears it on cancel and consumes it on the three filter commits; `return_to_chain_origin()`; hint arm for `i x c d m a s`; `h`/`l` hint arms in `handle_filter_key`; tests. |
| `src/help.rs` | Global `f` row action text. |
| `README.md` | Chain paragraph rewritten; hints paragraph; `f` row. |

---

### Task 1: Chains that commit a prompt return focus, then step

**Files:**
- Modify: `src/lib.rs` — `App` struct (beside `swallow_next_enter`, ~line 279) and its initialiser (~471); the `f` arm (~1028); `reveal_and_focus` (~2213), `focus_next` (~2147), `focus_prev` (~2160), `zoom_file_view` (~2200); `handle_search_key` (the `Esc`, `Backspace` and `Enter` arms, ~466–540); new method next to `reveal_and_focus`
- Test: `src/lib.rs` tests module, after `slash_from_the_filter_pane_sets_a_live_search`; plus the migration of existing tests

**Interfaces:**
- Produces: `chain_origin: Option<Focus>` on `App`; `fn return_to_chain_origin(&mut self)`.

- [ ] **Step 1: Write the failing tests**

```rust
    // ---- chains return focus (#120 §8, decision (a)) ---------------------

    /// `f i fn Enter` from the file view: the filter is added, focus comes
    /// back, and the cursor lands on the first `fn` as if `n` were pressed.
    #[test]
    fn f_i_enter_from_the_view_returns_and_steps() {
        let mut app = app_over_file("chain_fi_view", "plain\nfn one\nplain\nfn two\n");
        key(&mut app, KeyCode::Char('t'));
        assert_eq!(cursor_source(&app), 0, "sanity");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "fn");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.filters.filters().len(), 1, "the filter was not added");
        assert_eq!(app.focus, Focus::View, "focus did not return");
        assert_eq!(cursor_source(&app), 1, "did not step to the first hit");
        assert!(app.chain_origin.is_none(), "origin not consumed");
    }

    /// From the navigator, the return acts as the navigator's `n`. A
    /// filename search is the observable form: adding a filter changes the
    /// scan cache key, so match marks cannot be pre-sent in a test, but the
    /// navigator's search-repeat needs no scan at all.
    #[test]
    fn f_i_enter_from_the_navigator_returns_and_repeats_its_n() {
        let mut app = app_over("chain_fi_nav", &["a.log", "b.log", "c.log"]);
        key(&mut app, KeyCode::Char('e'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "log");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.nav.selected_name().as_deref(), Some("a.log"), "sanity");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "x");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.focus, Focus::Nav, "focus did not return");
        assert_eq!(
            app.nav.selected_name().as_deref(),
            Some("b.log"),
            "the return did not act as the navigator's n"
        );
    }

    /// `f c … Enter` (edit) returns too; `f d`, `f Enter` and plain `f` do not.
    #[test]
    fn f_c_returns_but_f_d_and_f_enter_stay() {
        let mut app = app_over_file("chain_fc", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        app.filters.add("beta").expect("valid pattern");
        app.refresh_view();

        key(&mut app, KeyCode::Char('f'));
        // Filters added directly leave the pane with no selection; `j`
        // selects the first row, which is what a user would do before `c`.
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('c'));
        typed(&mut app, "x");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::View, "f c … Enter did not return");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Filters, "f Enter returned");

        key(&mut app, KeyCode::Char('d'));
        assert_eq!(app.focus, Focus::Filters, "f d returned");
        assert_eq!(app.filters.filters().len(), 1, "d did not delete");
    }

    /// A focus change inside the chain ends it: `f Tab i … Enter` stays put.
    #[test]
    fn a_focus_change_after_f_ends_the_chain() {
        let mut app = app_over_file("chain_tab", "alpha\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Filters, "sanity: back on the pane");
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.focus, Focus::Filters, "returned after a Tab broke the chain");
    }

    /// `f f` is the sticky gesture: a second `f` keeps focus in the pane.
    #[test]
    fn f_f_makes_the_pane_sticky() {
        let mut app = app_over_file("chain_ff", "alpha\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.focus, Focus::Filters);
    }

    /// A cancelled prompt ends the chain: `f i … Esc`, then `i … Enter`, stays.
    #[test]
    fn a_cancelled_prompt_ends_the_chain() {
        let mut app = app_over_file("chain_cancel", "alpha\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "al");
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.focus, Focus::Filters);
    }

    /// Search prompts are not chains: `f / … Enter` and `S … Enter` stay.
    #[test]
    fn search_and_save_prompts_do_not_return() {
        let mut app = app_over_file("chain_search", "alpha\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.focus, Focus::Filters, "a search commit returned");
    }

    /// The `Enter` that commits is still swallowed once after the return,
    /// so it cannot also open the navigator's selection.
    #[test]
    fn the_committing_enter_is_swallowed_after_a_return() {
        let mut app = app_over("chain_swallow", &["a.log", "b.log"]);
        key(&mut app, KeyCode::Char('e'));
        let before = shown(&app);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "x");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Nav, "sanity: returned");
        key(&mut app, KeyCode::Enter);

        assert_eq!(shown(&app), before, "the doubled Enter opened an entry");
    }
```

`shown` is a PR 1 helper in the same module. `app.search` and `app.chain_origin` are private fields readable from the tests module.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib chain_`
Expected: compile error on `app.chain_origin`, then failures on `focus did not return`.

- [ ] **Step 3: Implement the bookkeeping**

Field, beside `swallow_next_enter`:

```rust
    /// The pane that had focus when `f` moved it to the filter pane, so a
    /// chain that commits a prompt — `f i … Enter`, `f x … Enter`,
    /// `f c … Enter` — can put focus back and step as `n` would (#120 §8,
    /// decision (a)). `None` once any other focus change, a prompt cancel,
    /// or a second `f` ends the chain; a toggle or delete inside the pane
    /// does not end it, because those are often one of several.
    chain_origin: Option<Focus>,
```

Initialise `chain_origin: None,` in `App::new`.

The `f` arm:

```rust
                KeyCode::Char('f') if key.modifiers.is_empty() => {
                    // A second `f` while the pane has focus is the sticky
                    // gesture: the user is staying, so no chain to return.
                    let origin = (self.focus != Focus::Filters).then_some(self.focus);
                    self.reveal_and_focus(Focus::Filters);
                    self.chain_origin = origin;
                    return;
                }
```

Add `self.chain_origin = None;` as the first line of `reveal_and_focus`, `focus_next`, `focus_prev`, and inside `zoom_file_view`'s `if` (beside `self.focus = Focus::View;`).

In `handle_search_key`: the `Esc` arm becomes

```rust
            KeyCode::Esc => {
                self.search = None;
                self.chain_origin = None;
            }
```

and in the `Backspace` arm, where `self.search = None;` runs on backspacing past the start, add `self.chain_origin = None;` beside it.

In the `Enter` arm, after `self.swallow_next_enter = true;` in the `outcome.is_ok()` branch:

```rust
                    if matches!(
                        kind,
                        PromptKind::Filter | PromptKind::Exclude | PromptKind::Edit { .. }
                    ) {
                        self.return_to_chain_origin();
                    }
```

The method, next to `reveal_and_focus`:

```rust
    /// End a chain that just committed: focus goes back to where `f` was
    /// pressed, and the app behaves as if `n` were pressed there — the
    /// first `fn` after `f i fn Enter` from the view, the next matching
    /// file from the navigator. Dispatching a real `n` rather than calling
    /// either step directly is what keeps "as if `n`" true per pane.
    ///
    /// Nothing to do when `f` was not what brought focus here.
    fn return_to_chain_origin(&mut self) {
        let Some(origin) = self.chain_origin.take() else {
            return;
        };
        self.reveal_and_focus(origin);
        self.dispatch_event(event::Event::Key(event::KeyEvent::from(KeyCode::Char('n'))));
        // The synthetic `n` spent the bounce guard; re-arm it, since the
        // `Enter` that committed is still the last key the user pressed.
        self.swallow_next_enter = true;
    }
```

`dispatch_event` clears `status_message` and `crossing` on a key, which is right: the status row should describe the step, not the commit.

- [ ] **Step 4: Run the new tests, then the whole suite, and migrate**

Run: `cargo test --lib chain_` — expected PASS.

Run: `cargo test` — expect failures in existing tests that pressed `f`, committed, and continued in the pane. For each, apply the migration rule from Global Constraints (insert `key(&mut app, KeyCode::Char('f'));` after the committing `Enter`). Re-run until green. Record every migrated test name in the report. If a failing test is about something else (a status-row assertion now showing the step's text, a cursor that moved because of the synthetic `n`), fix it to the new behaviour only if the new behaviour is what the spec says; otherwise stop and report BLOCKED with the test name and output.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
git add src/lib.rs
git commit -m "feat(keys): f i / f x / f c … Enter return focus to the origin pane and step as n (#120)"
```

---

### Task 2: A pane verb in the wrong pane says so

**Files:**
- Modify: `src/lib.rs` — a global arm in `dispatch_event` directly above the `n`/`N` arm; two arms at the top of `handle_filter_key`'s key match (~1918, beside `i`/`x`)
- Test: `src/lib.rs` tests module, after the Task 1 tests

**Interfaces:**
- Consumes: `self.report(text, false)`.

- [ ] **Step 1: Write the failing tests**

```rust
    // ---- wrong-pane hints (#120 §9) ---------------------------------------

    fn status(app: &App) -> Option<&str> {
        app.status_message.as_ref().map(|m| m.text.as_str())
    }

    #[test]
    fn a_filter_verb_in_the_view_hints_at_the_chain() {
        let mut app = app_over_file("hint_view", "alpha\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('i'));

        assert_eq!(status(&app), Some("i adds a filter · f i"));
        assert!(app.search.is_none(), "a prompt opened");
        assert_eq!(app.focus, Focus::View, "focus moved");
    }

    #[test]
    fn every_filter_verb_hints_in_the_navigator() {
        let mut app = app_over("hint_nav", &["a.log"]);
        key(&mut app, KeyCode::Char('e'));
        let expected = [
            ('i', "i adds a filter · f i"),
            ('x', "x adds an excluding filter · f x"),
            ('c', "c changes the selected filter · f c"),
            ('d', "d deletes the selected filter · f d"),
            ('m', "m toggles include and context · f m"),
            ('a', "a picks a profile for the set · f a"),
            ('s', "s solos the set · f s"),
        ];
        for (c, text) in expected {
            key(&mut app, KeyCode::Char(c));
            assert_eq!(status(&app), Some(text), "hint for {c}");
            assert_eq!(app.focus, Focus::Nav, "{c} moved focus");
        }
        assert!(app.filters.is_empty(), "a verb acted outside its pane");
    }

    #[test]
    fn a_hint_lasts_one_keypress() {
        let mut app = app_over_file("hint_gone", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('i'));
        assert!(status(&app).is_some(), "sanity");

        key(&mut app, KeyCode::Char('j'));

        assert_eq!(status(&app), None);
    }

    #[test]
    fn h_and_l_in_the_filter_pane_hint_at_the_navigator() {
        let mut app = app_over("hint_pane", &["a.log"]);
        key(&mut app, KeyCode::Char('f'));

        key(&mut app, KeyCode::Char('h'));
        assert_eq!(status(&app), Some("h goes up a directory · e h"));
        key(&mut app, KeyCode::Char('l'));
        assert_eq!(status(&app), Some("l opens the entry · e l"));
        assert_eq!(app.focus, Focus::Filters);
    }

    /// The hint does not fire where the verb is real.
    #[test]
    fn a_filter_verb_in_the_filter_pane_is_not_a_hint() {
        let mut app = app_over_file("hint_real", "alpha\n");
        key(&mut app, KeyCode::Char('f'));

        key(&mut app, KeyCode::Char('i'));

        assert!(app.search.is_some(), "i did not open the prompt");
        assert_eq!(status(&app), None);
    }
```

If a `status` helper already exists in the module under that name, call this one `status_text`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib hint_`
Expected: FAIL — `status(&app)` is `None` (the keys fall through silently).

- [ ] **Step 3: Implement**

Global arm, directly above the `n`/`N` arm in `dispatch_event`:

```rust
                // A filter-pane verb pressed anywhere else says so, for one
                // keypress, instead of doing nothing (#120 §9). Not a
                // redirect: making `i` global would collapse `f i` and `i`,
                // and `x`-not-`e` for exclude exists because `e` is a focus
                // key. The chain stays the answer; the hint teaches it.
                // These seven letters are unbound in the navigator and the
                // file view, so this arm shadows nothing.
                KeyCode::Char(c @ ('i' | 'x' | 'c' | 'd' | 'm' | 'a' | 's'))
                    if key.modifiers.is_empty() && self.focus != Focus::Filters =>
                {
                    let verb = match c {
                        'i' => "adds a filter",
                        'x' => "adds an excluding filter",
                        'c' => "changes the selected filter",
                        'd' => "deletes the selected filter",
                        'm' => "toggles include and context",
                        'a' => "picks a profile for the set",
                        _ => "solos the set",
                    };
                    self.report(&format!("{c} {verb} · f {c}"), false);
                    return;
                }
```

In `handle_filter_key`, inside the `if key.modifiers.is_empty()` block before the `kind` match:

```rust
            // The navigator's `h`/`l` in this pane: a hint, not a redirect,
            // for the same reason as the filter verbs elsewhere (#120 §9).
            match key.code {
                KeyCode::Char('h') => {
                    self.report("h goes up a directory · e h", false);
                    return;
                }
                KeyCode::Char('l') => {
                    self.report("l opens the entry · e l", false);
                    return;
                }
                _ => {}
            }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib hint_`; then `cargo test --lib every_bound_key`; then `cargo test`.
Expected: all PASS. If an existing test pressed one of these letters in the wrong pane and asserted silence, update it to the hint and name it in the report.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
git add src/lib.rs
git commit -m "feat(keys): a pane verb in the wrong pane hints at the chain that reaches it (#120)"
```

---

### Task 3: Docs and full verification

**Files:**
- Modify: `src/help.rs` — Global `f` row
- Modify: `README.md` — Global `f` row (~line 306); the chain paragraph in the filter-pane section (~880, "`i` and `x` work only while this pane has focus…"); a new hints paragraph after it

- [ ] **Step 1: `KEYMAP`**

```rust
            Binding {
                keys: &["f"],
                action: "Focus the filter pane; f i / f x / f c return on commit",
            },
```

If the width-150 layout tests object, use `"Focus the filter pane (f i, f x, f c chain)"`.

- [ ] **Step 2: README**

Global `f` row:

```markdown
| `f` | Focus the filter pane. `f i`, `f x` and `f c` are chains: the pair works from anywhere, and when the prompt commits, focus returns to where you were and the app steps as if you had pressed `n`. `f f` stays in the pane |
```

Replace the paragraph beginning "`i` and `x` work only while this pane has focus" with:

```markdown
`i`, `x` and `c` work only while this pane has focus, which is what `f` is for —
`f i`, `f x` and `f c` reach them from anywhere, and `f` is a no-op when the pane
already has focus, so the pair is always correct. They are deliberately not
global: bound app-wide they would swallow a keystroke from every other pane,
which is exactly what `f` and `F` used to do and the reason they moved.

A chain that commits a prompt returns. `f i fn Enter` from the file view adds
the filter, puts focus back in the view, and lands on the first `fn`, as if you
had pressed `n`; from the navigator it lands on the first matching file. Only a
commit returns — `f d`, `f Enter`, `f m`, `f a`, `f s` and a plain `f` leave focus
in the pane, because a toggle or a delete is often one of several. `f f` is the
way to say "I am staying": the second `f` ends the chain. So does any other
focus key, `Tab`, or cancelling the prompt.

A pane's verb pressed in the wrong pane is not silent. `i`, `x`, `c`, `d`, `m`,
`a` or `s` in the navigator or the file view puts a one-line hint on the status
row — `i adds a filter · f i` — for one keypress; `h` and `l` in the filter pane
do the same for the navigator. It is a hint rather than a redirect on purpose:
making `i` global would make `f i` and `i` the same key, and the chain is the
thing worth learning.
```

- [ ] **Step 3: Run everything**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: fmt makes no changes, clippy clean, all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/help.rs README.md
git commit -m "docs: chains return on commit, f f stays, wrong-pane hints (#120)"
```

---

## Self-review

**Spec coverage:** §8 return-focus for `f i`/`f x`/`f c` commits, origin recorded when `f` moves focus, cleared on any other focus change — Task 1. The spec's "then behave as if `n` had been pressed" is the synthetic dispatch. `p` keeps focus (no change). §9 hints for `i x c d m a s` outside the pane and `h`/`l` inside, one keypress, not a redirect — Task 2. Docs — Task 3.

**Decisions made here, beyond the spec's text:** a prompt cancel ends the chain (the spec is silent; a chain that survives its own cancel would make an unrelated later commit jump). `f f` ends the chain (the user's stated sticky gesture). The bounce guard is re-armed after the synthetic `n` so a doubled `Enter` cannot open an entry in the navigator.

**Not in this PR:** `*` (§13 — PR 4), the `KEYMAP` regroup (§15 — PR 5).

**Placeholder scan:** none. **Type consistency:** `chain_origin: Option<Focus>` is read by `return_to_chain_origin` with `take()`; `report(&str, bool)` matches the existing signature.
