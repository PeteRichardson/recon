# Keymap PR 5: The keymap reads as the model

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `KEYMAP` and the README's *Keybindings* section are grouped by layer — Global · Chains · Shared motions · Navigator · File view · Filter pane · While a prompt is open — so a shared motion appears once and the `?` overlay teaches the model. No key changes.

**Architecture:** Pure documentation restructuring. `KEYMAP` in `src/help.rs` gains two sections (Chains, Shared motions) and loses the duplicated motion rows from the three pane sections; a new test pins the section order and the once-only rule for shared motions. The overlay's two layout tests get their width re-derived from the new table (the doc comment on them says how). The README gets a short layer paragraph, a Chains table, a Shared motions table, and pane tables trimmed to their own verbs; its prose is kept, with two sentences reworded where a row moved out from under them.

**Tech Stack:** Rust; the overlay layout in `src/help.rs`; Markdown.

**Spec:** `docs/specs/2026-09-05-keymap-reconciliation-design.md` §15. Base: `main` at 8c07042 or later (PR 4 merged).

## Global Constraints

- No behaviour changes: no file but `src/help.rs` and `README.md` is touched, and no key arm moves.
- `every_bound_key_is_documented` must keep passing: every character bound in a `Char(..)` arm must still be documented by some row, in some section. Multi-character chain labels (`f i`) document nothing, which is fine — they name commands, not new keys.
- The two overlay layout tests (`a_normal_terminal_shows_the_whole_keymap`, `a_column_is_sized_to_its_own_widest_row`) must pass at a width that sits strictly between the correct per-column total and the buggy whole-table total for the *new* table. Recompute both totals (temporarily print `total_width` from a scratch test, or reason from the widest key/action per column), pick a width with a little slack above the correct total and below the buggy one, and update the tests' width and their doc-comment numbers. Never widen past the buggy total.
- The overlay must still fit one screen at the test's height (42 inner rows): the new table has about 73 rows, so two columns; keep action strings short enough that two columns fit the chosen width.
- The README's "three places a key can be bound" paragraph stays word for word.
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` pass; `cargo test` takes one filter at a time.

---

## File map

| File | Change |
|---|---|
| `src/help.rs` | `KEYMAP` regrouped into seven sections; new test `sections_follow_the_layer_model`; layout tests' width and comment re-derived. |
| `README.md` | Layer paragraph; Chains and Shared motions tables; pane tables trimmed; two sentences reworded. |

---

### Task 1: `KEYMAP` regrouped by layer

**Files:**
- Modify: `src/help.rs` — `KEYMAP` (~lines 83–330), the two layout tests and their doc comment (~770–800)
- Test: `src/help.rs` tests module

**Interfaces:**
- Produces: `KEYMAP` with section titles, in order: `"Global"`, `"Chains"`, `"Shared motions"`, `"Navigator"`, `"File view"`, `"Filter pane"`, `"While a prompt is open"`.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `src/help.rs`:

```rust
    /// The overlay is the model (#120 §15): one section per layer, in the
    /// order the layers are checked, and a shared motion documented once
    /// rather than once per pane.
    #[test]
    fn sections_follow_the_layer_model() {
        let titles: Vec<&str> = KEYMAP.iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            [
                "Global",
                "Chains",
                "Shared motions",
                "Navigator",
                "File view",
                "Filter pane",
                "While a prompt is open",
            ]
        );

        // A shared motion appears in exactly one section outside Global and
        // the prompt: the shared one. Counting sections, not rows, so a key
        // that legitimately has two rows in one section is not a failure.
        let shared = [
            "j", "k", "g", "G", "Home", "End", "Ctrl-d", "Ctrl-u", "PageDown", "PageUp", "n", "N",
            "Enter",
        ];
        for key in shared {
            let sections: Vec<&str> = KEYMAP
                .iter()
                .filter(|s| s.title != "Global" && s.title != "While a prompt is open")
                .filter(|s| s.bindings.iter().any(|b| b.keys.contains(&key)))
                .map(|s| s.title)
                .collect();
            assert_eq!(sections, ["Shared motions"], "{key} is documented in {sections:?}");
        }

        // Pane verbs stay in their pane.
        let pane_only = [("h", "Navigator"), ("l", "Navigator"), ("i", "Filter pane"), ("*", "File view")];
        for (key, pane) in pane_only {
            let sections: Vec<&str> = KEYMAP
                .iter()
                .filter(|s| s.bindings.iter().any(|b| b.keys.contains(&key)))
                .map(|s| s.title)
                .collect();
            assert!(sections.contains(&pane), "{key} is not documented in {pane}: {sections:?}");
        }
    }
```

`h` and `l` are in both the Navigator and File view sections on purpose (different meanings, documented trade); the test only checks each is present in its pane.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib sections_follow_the_layer_model`
Expected: FAIL on the titles assertion.

- [ ] **Step 3: Rewrite `KEYMAP`**

Replace the whole `KEYMAP` constant with the following. The Global section keeps its current rows and order exactly; only the sections after it change.

```rust
pub const KEYMAP: &[Section] = &[
    Section {
        title: "Global",
        bindings: &[
            // … the existing Global rows, unchanged …
        ],
    },
    Section {
        title: "Chains",
        bindings: &[
            Binding {
                keys: &["f i", "f x"],
                action: "Add an including / excluding filter — returns on commit",
            },
            Binding {
                keys: &["f c"],
                action: "Change the selected filter — returns on commit",
            },
            Binding {
                keys: &["f d", "f Enter"],
                action: "Delete / toggle the selected filter — focus stays",
            },
            Binding {
                keys: &["f f"],
                action: "Stay in the filter pane",
            },
            Binding {
                keys: &["e n"],
                action: "Next file the filters match, from anywhere",
            },
            Binding {
                keys: &["t *"],
                action: "Search the word under the view's cursor, from the navigator",
            },
        ],
    },
    Section {
        title: "Shared motions",
        bindings: &[
            Binding {
                keys: &["j", "k", "Down", "Up"],
                action: "Down / up one row — line, entry or filter",
            },
            Binding {
                keys: &["g", "G", "Home", "End"],
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
            Binding {
                keys: &["n", "N"],
                action: "Next / previous interesting line, crossing files — or matching file, in the navigator",
            },
            Binding {
                keys: &["Enter"],
                action: "Open the entry; toggle the filter or set; nothing in the view",
            },
        ],
    },
    Section {
        title: "Navigator",
        bindings: &[
            Binding {
                keys: &["h", "Left"],
                action: "Up to the parent directory",
            },
            Binding {
                keys: &["l", "Right"],
                action: "Open the entry",
            },
        ],
    },
    Section {
        title: "File view",
        bindings: &[
            Binding {
                keys: &["h", "Left"],
                action: "Cursor back",
            },
            Binding {
                keys: &["l", "Right"],
                action: "Cursor forward",
            },
            Binding {
                keys: &["w"],
                action: "Next word",
            },
            Binding {
                keys: &["0", "^"],
                action: "Start of the line",
            },
            Binding {
                keys: &["$"],
                action: "End of the line",
            },
            Binding {
                keys: &["{", "}"],
                action: "Previous / next paragraph",
            },
            Binding {
                keys: &["#"],
                action: "Toggle the line-number gutter",
            },
            Binding {
                keys: &["*"],
                action: "Search for the word under the cursor",
            },
            Binding {
                keys: &["Ctrl-e", "Ctrl-y"],
                action: "Scroll one line down / up",
            },
            Binding {
                keys: &["Ctrl-f", "Ctrl-b"],
                action: "A page down / up (aliases)",
            },
        ],
    },
    Section {
        title: "Filter pane",
        bindings: &[
            Binding {
                keys: &["i"],
                action: "Add an including filter",
            },
            Binding {
                keys: &["x"],
                action: "Add an excluding filter",
            },
            Binding {
                keys: &["c"],
                action: "Change the selected filter's pattern",
            },
            Binding {
                keys: &["d"],
                action: "Delete the selected filter",
            },
            Binding {
                keys: &["m"],
                action: "Toggle the selected filter between include and context",
            },
            Binding {
                keys: &["a"],
                action: "Pick a profile for the selected set",
            },
            Binding {
                keys: &["s"],
                action: "Solo the selected set — or un-solo it",
            },
            Binding {
                keys: &["R"],
                action: "Reset every set to its startup state",
            },
            Binding {
                keys: &["S"],
                action: "Save the scratch filters as a named set",
            },
        ],
    },
    Section {
        title: "While a prompt is open",
        bindings: &[
            // … the existing prompt rows, unchanged …
        ],
    },
];
```

Keep the existing doc comments above `KEYMAP` and inside the Global section. Add one sentence to the `KEYMAP` doc comment: sections are the layers of #120, in the order they are checked, and a shared motion is documented once.

- [ ] **Step 4: Re-derive the layout tests' width**

Run `cargo test --lib help`. If the two layout tests fail, or even if they pass, recompute the two totals for the new table: add a temporary `#[test] fn print_totals()` that lays out `rows()` at `inner(400, 42)` and prints `total_width` for the correct layout, and a patched `pack` (every column sized to the whole table's widest key and action) for the buggy total — or reason from the widest key/action per column. Choose a width strictly between them with a few columns of slack above the correct total, set it in both tests, and rewrite the two numbers and the sentence in their doc comment. Delete the temporary test. Record both totals in your report.

If no width discriminates (correct ≥ buggy), shorten the longest action strings in whichever column they land until one does; the `n`/`N` shared row is the likely culprit.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib sections_follow`; `cargo test --lib every_bound_key`; `cargo test --lib help`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
git add src/help.rs
git commit -m "docs(help): KEYMAP grouped by layer — Global, Chains, Shared motions, then the panes (#120)"
```

---

### Task 2: README Keybindings regrouped the same way

**Files:**
- Modify: `README.md` — the *Keybindings* section (~271–830)

- [ ] **Step 1: Add the layer paragraph**

Directly after the paragraph ending "so a new key needs a row here too." (~line 293) and before "Global (`src/lib.rs`), handled before the focused pane sees the key:", insert:

```markdown
Every key lives in one of four layers, checked in this order, and the tables
below follow them. A **prompt**, while open, takes every key. **Global** keys
mean one thing everywhere and are handled before any pane sees them.
**Chains** are a focus key followed by a pane key — `f i` adds a filter from
anywhere, and costs nothing extra from inside the pane, where `f` is a no-op.
**Pane** keys are either *shared motions*, bound in every pane where the idea
exists with the same meaning, or *pane verbs* that only make sense in one place
and are reached from elsewhere by a chain. Two rules hold across all of it: a
key with a direction has a partner, and where vim has an opinion recon follows
it unless a paragraph here says why not.
```

- [ ] **Step 2: Add the Chains and Shared motions tables**

Directly after the two paragraphs that follow the Global table (the `?`-history paragraph and the "The overlay is a centred panel" paragraph, ending around line 340 with "…the bottom border says how many rows were cut."), insert:

```markdown
Chains — a focus key, then a pane key. Documented as commands because they
are how a pane verb is reached from anywhere:

| Keys | Action |
| --- | --- |
| `f i` / `f x` | Add an including / excluding filter. When the prompt commits, focus returns to where you were and `n` runs there |
| `f c` | Change the selected filter's pattern; returns on commit the same way |
| `f d` / `f Enter` | Delete / toggle the selected filter; focus stays in the pane, since one delete is often the first of several |
| `f f` | Stay in the filter pane — the second `f` ends the chain |
| `e n` | Move the navigator to the next file the filters match |
| `t *` | Search for the word under the file view's cursor, from the navigator |

Shared motions — the same key, the same meaning, in every pane where the idea
exists:

| Key(s) | Navigator | File view | Filter pane |
| --- | --- | --- | --- |
| `j` / `k`, `Down` / `Up` | next / previous entry | cursor down / up | next / previous row |
| `g` / `G`, `Home` / `End` | first / last entry | top / bottom of the file | first / last row |
| `Ctrl-d` / `Ctrl-u` | half a page | half a page | half a page |
| `PageDown` / `PageUp` | a page | a page (also `Ctrl-f` / `Ctrl-b`) | a page |
| `n` / `N` | next / previous filename-search match, or file the filters match | next / previous interesting line, crossing files | acts on the file view |
| `Enter` | open the entry | — | toggle the filter, or the set |

`[` and `]` page the file view from every pane and so live in the Global table.
```

- [ ] **Step 3: Trim the pane tables**

- File view table (~665–682): remove the rows for `j` / `Down`, `k` / `Up`, `g` / `Home`, `G` / `End`, `n` / `N`, `Ctrl-d` / `Ctrl-u`; change the two paging rows to one: `` `Ctrl-f` / `Ctrl-b` | Page down / up — aliases for `PageDown` / `PageUp` ``. Keep `h`, `l`, `w`, `0`/`^`, `$`, `{`/`}`, `#`, `*`, `Ctrl-e`/`Ctrl-y`.
- Navigator table (~719–727): remove `k`/`Up`, `j`/`Down`, `n`/`N`, `g`/`Home`, `G`/`End`, `Ctrl-d`/`Ctrl-u`, `PageDown`/`PageUp`; change `` `l` / `Right` / `Enter` `` to `` `l` / `Right` `` with "Open the selected entry — descend into a directory, or load a file (`Enter` does the same)". Keep `h`.
- Filter pane table (~804–819): remove `k`/`Up`, `j`/`Down`, `g`/`Home`, `G`/`End`, `Ctrl-d`/`Ctrl-u`, `PageDown`/`PageUp`, and the `Enter` row. Keep `i`, `x`, `d`, `c`, `m`, `a`, `s`, `R`, `S`.

Do not remove or reorder any prose paragraph. Each table intro line ("File view pane (`src/widgets/fileview.rs`):" etc.) stays.

- [ ] **Step 4: Reword two sentences**

- Under the file view table, the paragraph beginning "`n` and `N` are handled globally, the same as `u`": change "they reach this table from the filter pane as well as the file view" to "they act on the file view from the filter pane as well" (the row is now in the Shared motions table).
- In the filter-pane prose, the sentence "`Enter` took the toggle over from `space`" (or whichever sentence names `Enter` as this table's toggle): leave it; it describes behaviour, and the Shared motions table now carries the row. If a sentence says "the table above lists `Enter`", reword to point at the Shared motions table.

- [ ] **Step 5: Verify and commit**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all green (README changes touch no test). Then:

```bash
git add README.md
git commit -m "docs(readme): keybindings grouped by layer — chains and shared motions tabled once (#120)"
```

---

## Self-review

**Spec coverage (§15):** `KEYMAP` regrouped by layer, shared motion once with per-pane meaning in the action — Task 1, pinned by a test. README regrouped the same way, "three places" paragraph untouched, reasoning paragraphs already in place from PRs 1–3 — Task 2. `every_bound_key_is_documented` still gates — Task 1.

**Decisions made here:** `Enter` sits in Shared motions with its per-pane meaning ("act on the selected thing"), since the spec lists it there and the view's "nothing" is itself a documented decision (§6). `h`/`l` stay in both pane sections; the model's rule is "same meaning where bound in more than one pane", and this pair is the documented exception. `[`/`]` stay Global, where PR 1 put them.

**Placeholder scan:** the two "… existing rows, unchanged …" comments in the `KEYMAP` listing are instructions to keep the current rows, not placeholders for new content.
