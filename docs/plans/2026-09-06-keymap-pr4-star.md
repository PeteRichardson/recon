# Keymap PR 4: `*` searches for the word under the cursor

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the file view, `*` sets the live search to the word under the cursor, literally, and steps to its next occurrence; `* p` then promotes it to a numbered filter. A pasted newline never lands in a prompt pattern.

**Architecture:** One query on the viewport, `word_under_cursor() -> Option<String>`, reads the cursor's source line and column and returns the maximal `[A-Za-z0-9_]` run around the column. One arm in `App::dispatch_event`, scoped to the file view like `n`, feeds it through `regex::escape` into the existing `apply_search`, which already does "set it, then step as `n` does". Outside the view the key hints at `t *`, matching PR 3's hint style. A one-line guard in `handle_search_key` drops `\n`/`\r` characters from a prompt.

**Tech Stack:** Rust, ratatui 0.30, crossterm, regex, vendored tui-textarea (`vendor/tui-textarea-2`; `TextArea::cursor()` returns `(row, col)` with `col` a character index). Tests are `#[cfg(test)]` in `src/lib.rs`, driven by `key`, `typed`, `app_over_file`, `cursor_source`, `status`.

**Spec:** `docs/specs/2026-09-05-keymap-reconciliation-design.md` — §13 and the Testing section's `*` and paste bullets. This plan is "Landing" item 4. Base: `main` at a2fe745 or later (PR 3 merged).

## Global Constraints

- `*` is Shift-8 on most layouts and crossterm attaches SHIFT to it, so the arm's guard is `!key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)` (the `n`/`N` and `?` pattern), never `modifiers.is_empty()`.
- A word is a maximal run of ASCII `[A-Za-z0-9_]` containing the cursor's column. The cursor may sit on any character of the word. On whitespace, punctuation, or past the end of the line, there is no word.
- The search pattern is the word passed through `regex::escape` (a no-op for this character class, kept so the contract is "literal" whatever the class becomes).
- Every `KeyCode::Char('*')` arm needs a `KEYMAP` row (`every_bound_key_is_documented`); a README File view row too. The two help-overlay layout tests at width 150 must pass without widening.
- Nothing goes silent: no word → `no word under the cursor`; `*` outside the view → `* searches the word under the cursor · t *`.
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` pass. `cargo test` takes one filter at a time. Fixture dir names unique, no `filters` substring.

---

## File map

| File | Change |
|---|---|
| `src/viewport.rs` | `pub(crate) fn word_under_cursor(&self) -> Option<String>` on `impl App`, plus a free `fn word_around(line: &str, col: usize) -> Option<&str>` it delegates to (unit-testable without an `App`). |
| `src/lib.rs` | `*` arm in `dispatch_event`; `\n`/`\r` guard in `handle_search_key`; tests. |
| `src/help.rs` | File view row for `*`. |
| `README.md` | File view table row; one paragraph in the search/filters prose. |

---

### Task 1: The word under the cursor

**Files:**
- Modify: `src/viewport.rs` — beside `cursor_source` (~line 267)
- Test: `src/viewport.rs` gets a `#[cfg(test)] mod tests` if it has none (check first; if `src/viewport.rs` already has one, add to it), for `word_around`; `src/lib.rs` tests for `word_under_cursor`

**Interfaces:**
- Produces: `pub(crate) fn word_around(line: &str, col: usize) -> Option<&str>` (free function in `src/viewport.rs`); `pub(crate) fn word_under_cursor(&self) -> Option<String>` on `App`.

- [ ] **Step 1: Write the failing tests**

In `src/viewport.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::word_around;

    #[test]
    fn a_word_is_a_run_of_identifier_characters_around_the_column() {
        assert_eq!(word_around("foo::bar(x)", 0), Some("foo"));
        assert_eq!(word_around("foo::bar(x)", 2), Some("foo"), "last char of the word");
        assert_eq!(word_around("foo::bar(x)", 5), Some("bar"));
        assert_eq!(word_around("foo::bar(x)", 6), Some("bar"), "middle of the word");
        assert_eq!(word_around("foo::bar(x)", 9), Some("x"));
    }

    #[test]
    fn a_mangled_name_stays_whole_and_stops_at_punctuation() {
        let line = "_ZN4core3fmt9Formatter3pad17hE::call(a.b)";
        assert_eq!(word_around(line, 10), Some("_ZN4core3fmt9Formatter3pad17hE"));
        assert_eq!(word_around(line, 38), Some("a"), "stops at the dot");
    }

    #[test]
    fn whitespace_punctuation_and_past_the_end_have_no_word() {
        assert_eq!(word_around("foo::bar", 3), None, "on a colon");
        assert_eq!(word_around("a  b", 1), None, "on a space");
        assert_eq!(word_around("abc", 3), None, "past the end");
        assert_eq!(word_around("", 0), None);
    }

    #[test]
    fn the_column_counts_characters_not_bytes() {
        // Two multi-byte chars before the word: byte offsets would miss it.
        assert_eq!(word_around("éé foo", 3), Some("foo"));
        assert_eq!(word_around("éé foo", 0), None, "é is not an ASCII word char");
    }
}
```

In `src/lib.rs` tests, after the last PR 3 hint test:

```rust
    // ---- `*` (#120 §13) ----------------------------------------------------

    #[test]
    fn word_under_cursor_follows_the_view_cursor() {
        let mut app = app_over_file("word_cursor", "foo bar\nbaz\n");
        key(&mut app, KeyCode::Char('t'));
        assert_eq!(app.word_under_cursor().as_deref(), Some("foo"));

        key(&mut app, KeyCode::Char('w'));
        assert_eq!(app.word_under_cursor().as_deref(), Some("bar"));

        key(&mut app, KeyCode::Char('j'));
        assert_eq!(app.word_under_cursor().as_deref(), Some("baz"));
    }
```

`w` is the view's word motion; if it lands one column past where expected, use `l` presses instead and say so.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib word_around`; then `cargo test --lib word_under_cursor`
Expected: compile errors, no such functions.

- [ ] **Step 3: Implement**

In `src/viewport.rs`, after `cursor_source`:

```rust
    /// The identifier-shaped word under the cursor, for `*` (#120 §13).
    /// `None` on whitespace, punctuation, or past the end of the line.
    pub(crate) fn word_under_cursor(&self) -> Option<String> {
        let line = self.document.lines().get(self.cursor_source())?;
        let col = self.view.textarea().cursor().1;
        word_around(line, col).map(str::to_owned)
    }
```

And as a free function in the same file (below `impl App`):

```rust
/// The maximal run of `[A-Za-z0-9_]` that contains character `col` of
/// `line`. This is vim's default `iskeyword` narrowed to ASCII: it keeps a
/// mangled `_ZN4core3fmt9Formatter3pad17hE` whole and stops at `::`, `.`
/// and `(`. `col` is a character index, matching what the textarea's
/// cursor reports, not a byte offset.
pub(crate) fn word_around(line: &str, col: usize) -> Option<&str> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let &(_, at) = chars.get(col)?;
    if !is_word(at) {
        return None;
    }
    let start = chars[..col]
        .iter()
        .rposition(|&(_, c)| !is_word(c))
        .map_or(0, |i| i + 1);
    let end = chars[col..]
        .iter()
        .position(|&(_, c)| !is_word(c))
        .map_or(chars.len(), |i| col + i);
    let byte_start = chars[start].0;
    let byte_end = chars.get(end).map_or(line.len(), |&(b, _)| b);
    Some(&line[byte_start..byte_end])
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib word_around`; `cargo test --lib word_under_cursor`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
git add src/viewport.rs src/lib.rs
git commit -m "feat(viewport): word_under_cursor — the identifier run around the cursor column (#120)"
```

---

### Task 2: `*` sets the live search to that word and steps

**Files:**
- Modify: `src/lib.rs` — a new arm directly above the `n`/`N` arm in `dispatch_event`
- Modify: `src/help.rs` — File view section, after the `#` row
- Modify: `README.md` — File view table, after the `#` row (~676)
- Test: `src/lib.rs` tests, after `word_under_cursor_follows_the_view_cursor`

**Interfaces:**
- Consumes: `word_under_cursor` (Task 1), `apply_search` (exists), `report` (exists).

- [ ] **Step 1: Write the failing tests**

```rust
    /// `*` is vim's two-key version of #67's first use case: a long,
    /// possibly mangled symbol under the cursor — where else does it appear?
    #[test]
    fn star_searches_for_the_word_under_the_cursor_and_steps() {
        let mut app = app_over_file("star_basic", "foo bar\nbar\nfoo\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('*'));

        let search = app.filters.search().expect("a live search was set");
        assert_eq!(search.predicate.display(), "foo");
        assert_eq!(cursor_source(&app), 2, "did not step to the next occurrence");

        key(&mut app, KeyCode::Char('k'));
        key(&mut app, KeyCode::Char('k'));
        key(&mut app, KeyCode::Char('w'));
        key(&mut app, KeyCode::Char('*'));
        assert_eq!(app.filters.search().unwrap().predicate.display(), "bar");
        assert_eq!(cursor_source(&app), 1);
    }

    #[test]
    fn star_keeps_a_mangled_name_whole() {
        let body = "_ZN4core3fmt9Formatter3pad17hE::x\nplain\n_ZN4core3fmt9Formatter3pad17hE\n";
        let mut app = app_over_file("star_mangled", body);
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('l'));
        key(&mut app, KeyCode::Char('l'));

        key(&mut app, KeyCode::Char('*'));

        assert_eq!(
            app.filters.search().unwrap().predicate.display(),
            "_ZN4core3fmt9Formatter3pad17hE"
        );
        assert_eq!(cursor_source(&app), 2);
    }

    #[test]
    fn star_on_whitespace_says_so() {
        let mut app = app_over_file("star_space", "a  b\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('l'));

        key(&mut app, KeyCode::Char('*'));

        assert!(app.filters.search().is_none(), "a search was set from whitespace");
        assert_eq!(status(&app), Some("no word under the cursor"));
    }

    /// `* p`: the whole "symbol to filter" flow without a selection (#67).
    #[test]
    fn star_then_p_promotes_the_literal_word() {
        let mut app = app_over_file("star_promote", "foo bar\nfoo\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('*'));
        key(&mut app, KeyCode::Char('p'));

        assert!(app.filters.search().is_none(), "p did not consume the search");
        assert_eq!(app.filters.len(), 1);
        let numbered = widgets::filterlist::numbered(&app.filters);
        assert_eq!(app.filters.filters()[numbered[0]].predicate.display(), "foo");
    }

    /// Shift arrives with `*` on most layouts; the arm must not be guarded
    /// on an empty modifier set (the `?`/`N` trap).
    #[test]
    fn star_works_with_shift_reported() {
        let mut app = app_over_file("star_shift", "foo\nfoo\n");
        key(&mut app, KeyCode::Char('t'));

        app.handle_event(event::Event::Key(event::KeyEvent::new(
            KeyCode::Char('*'),
            KeyModifiers::SHIFT,
        )));

        assert!(app.filters.search().is_some());
    }

    #[test]
    fn star_outside_the_view_hints_at_t_star() {
        let mut app = app_over_file("star_hint", "foo\n");
        key(&mut app, KeyCode::Char('e'));

        key(&mut app, KeyCode::Char('*'));

        assert!(app.filters.search().is_none());
        assert_eq!(status(&app), Some("* searches the word under the cursor · t *"));
        assert_eq!(app.focus, Focus::Nav);
    }
```

`status` is the PR 3 test helper (`fn status<'a>(app: &'a App<'a>) -> Option<&'a str>`). `KeyModifiers` is imported at the top of `src/lib.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib star_`
Expected: FAIL — no search set (the key falls through).

- [ ] **Step 3: Implement the arm**

Directly above the `n`/`N` arm in `dispatch_event`:

```rust
                // `*` — the live search becomes the word under the cursor,
                // literally, then step as `/` does (#120 §13). Vim's two-key
                // answer to "where else does this symbol appear?", which is
                // #67's first use case, without a selection. `regex::escape`
                // keeps the contract literal whatever the word class becomes.
                // Scoped to the file view, where a cursor column exists; the
                // other panes get a hint (#120 §9). Not guarded on an empty
                // modifier set: `*` is Shift-8 and crossterm reports the
                // Shift, the same trap `?` and `N` document.
                KeyCode::Char('*')
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if self.focus != Focus::View {
                        self.report("* searches the word under the cursor · t *", false);
                        return;
                    }
                    match self.word_under_cursor() {
                        Some(word) => {
                            // An escaped literal always compiles; a failure
                            // here would be a regex-crate bug, not user input.
                            if self.apply_search(&regex::escape(&word)).is_err() {
                                self.report("could not search for that word", true);
                            }
                        }
                        None => self.report("no word under the cursor", false),
                    }
                    return;
                }
```

- [ ] **Step 4: Document the key**

`src/help.rs`, File view section, after the `#` row:

```rust
            Binding {
                keys: &["*"],
                action: "Search for the word under the cursor",
            },
```

`README.md`, File view table, after the `#` row:

```markdown
| `*` | Set the live search to the word under the cursor — a run of letters, digits and `_`, so a mangled symbol stays whole — and move to its next occurrence. `* p` makes it a numbered filter |
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib star_`; `cargo test --lib every_bound_key`; `cargo test --lib help`
Expected: all PASS. If a layout test fails, shorten the action to `"Search the word under the cursor"`.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
git add src/lib.rs src/help.rs README.md
git commit -m "feat(keys): * searches for the word under the cursor, literally, and steps (#120)"
```

---

### Task 3: A pasted newline never enters a prompt; docs; verification

**Files:**
- Modify: `src/lib.rs` — the `KeyCode::Char(c)` arm of `handle_search_key` (~571)
- Modify: `README.md` — one paragraph after the File view table's prose about `/` and `n` (search for the paragraph that explains `?` used to search backward, ~line 320; put the new paragraph in the section that documents the live search, "Search as a filter" or the nearest heading that describes `/`)
- Test: `src/lib.rs` tests

- [ ] **Step 1: Write the failing test**

```rust
    /// A terminal paste arrives as individual `Char` events; a newline in
    /// it must not become part of a single-line pattern (#120 §13). This
    /// guards the `*`/paste interplay if bracketed paste is ever enabled
    /// for #67.
    #[test]
    fn a_pasted_newline_is_dropped_from_the_prompt() {
        let mut app = app_over_file("paste_newline", "alpha\n");
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "al");
        key(&mut app, KeyCode::Char('\n'));
        key(&mut app, KeyCode::Char('\r'));
        typed(&mut app, "pha");

        assert_eq!(prompt_line(&mut app).trim_end(), "/alpha");
    }
```

`prompt_line` exists (~line 3141 of the tests module). If the prompt row carries a badge or other text so `trim_end` is not enough, assert `.contains("/alpha")` and that it does not contain a control character.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib a_pasted_newline`
Expected: FAIL — the pattern contains the newline.

- [ ] **Step 3: Implement**

In `handle_search_key`, the `Char` arm becomes:

```rust
            // A paste arrives as one `Char` per character. A newline in it
            // is dropped rather than typed: the pattern is single-line, and
            // a stray `\n` would silently make it match nothing (#120 §13).
            KeyCode::Char('\n' | '\r') => {}
            KeyCode::Char(c) => {
                if let Some(prompt) = self.search.as_mut() {
                    prompt.error = None;
                    prompt.pattern.push(c);
                }
            }
```

- [ ] **Step 4: README paragraph**

After the File view table's prose (or in the live-search section, whichever the file has nearest the `/` explanation), add:

```markdown
`*` is the two-key version of "where else does this symbol appear?": the word
under the cursor — letters, digits and `_`, so a mangled `_ZN…E` stays whole
and `foo::bar` stops at the colons — becomes the live search, literally, and
the cursor moves to its next occurrence exactly as `/` would. `* p` then keeps
it as a numbered filter. There is no backward twin: `#` is the gutter, and `N`
covers the direction. A paste into any prompt drops newlines rather than typing
them, so a copied line never becomes a pattern that matches nothing.
```

- [ ] **Step 5: Run everything**

```bash
cargo test --lib a_pasted_newline
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs README.md
git commit -m "fix(prompt): pasted newlines are dropped; docs for * (#120)"
```

---

## Self-review

**Spec coverage:** §13 — view key, live search set to the word, `regex::escape`, word class `[A-Za-z0-9_]`, steps as `/` does, `* p` promotes, no backward twin, status row on no word — Tasks 1 and 2. The Testing section's `*` bullets (`foo::bar(` → `bar`; `_ZN4core3fmt` whole; whitespace says so; escaping) — Tasks 1 and 2; escaping is unobservable for this word class, so the test asserts the `.`-stop instead and the escape call stays for the contract. The paste bullet — Task 3. Hint outside the view — PR 3's §9 pattern, Task 2.

**Not in this PR:** the `KEYMAP` regroup by layer (§15 — PR 5).

**Placeholder scan:** none. **Type consistency:** `word_around(&str, usize) -> Option<&str>`; `word_under_cursor(&self) -> Option<String>`; `apply_search(&str) -> Result<(), regex::Error>`.
