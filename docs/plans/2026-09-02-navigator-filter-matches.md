# Navigator Filter Matches Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The navigator marks which files would show at least one line under the active filters, scanning in the background and re-answering filter toggles from a per-line bitset cache with no I/O.

**Architecture:** A pure `scan()` core over any `BufRead` records which patterns hit each line as a `u64` bitset and stops at the first line that selects the file; a thin `Scanner` thread drives it per file and streams `Scanned` results over `mpsc`. `App` owns the cache and the "is a scan needed" decision, drains results on the existing render tick, and hands `FileNav` ready-made `Match` answers. A third `Sense::Context` lets a filter show lines without selecting files.

**Tech Stack:** Rust 2024, `regex::RegexSet` (already a dependency), `std::thread` + `std::sync::mpsc` (the editor-reaper pattern already in `src/editor.rs`), ratatui 0.30. **No new crates.**

**Spec:** `docs/specs/2026-09-02-navigator-filter-matches-design.md` — read it first; every task below argues from it. Tracks issue #119.

## Global Constraints

- CI runs, in this order: `cargo fmt -p recon --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --release`. Run all four in that order before every commit — `fmt` can reshape code into a clippy warning.
- `[lints.clippy] pedantic = warn` is on, and `-D warnings` makes every lint an error. `#![warn(unreachable_pub)]` is on: anything `pub` inside the private `widgets` module must be `pub(crate)`.
- Bitsets are `u64`: 63 numbered patterns plus the search. Above 64 patterns the feature is **off**, never wrong.
- The match rule is `Verdict::Included(i)` where `filters[i].sense == Sense::Include`, or `Verdict::Searched`, and not `Excluded`. `Context` never selects a file.
- `FileNav` never sees a filter, a file's contents, a stamp, or a thread. It is told answers.
- Work on a worktree branched from `origin/main`: `git worktree add .worktrees/Fix-I119-navigator-filter-matches -b Fix-I119-navigator-filter-matches origin/main`. Every command below runs inside it. **Known flake:** `widgets::filenav::tests::title_shows_the_current_directory` fails when the worktree path is long — it is path length, not your change (see the memory note); the branch name above is short enough.
- Test fixture directories must have unique names across the whole crate (`claim_fixture_dir` panics on reuse, and APFS is case-insensitive — `Foo` and `foo` collide).
- Commit messages: imperative subject with a `type(scope):` prefix, body explains *why*. Match `git log`.

---

## File structure

| File | Responsibility |
|---|---|
| `src/filter.rs` (modify) | `Sense::Context`; `toggle_context`; `Matcher` + `Owner` snapshot; `pattern_key` |
| `src/scan.rs` (**create**, `pub mod`) | `Progress`, `Record`, `Stamp`, `stamp()`, the pure `scan()` core, `Request`/`Scanned`, the `Scan` trait, the `Scanner` thread driver, `double::RecordingScanner` |
| `src/widgets/mod.rs` (modify) | `FilterCommand::ToggleContext(usize)` |
| `src/widgets/filterlist.rs` (modify) | `m` key; `ctx` glyph; style arm |
| `src/widgets/filenav.rs` (modify) | `Match`; `Entry.matched`; `Mode` + `visible`; `set_answer` / `set_mode` / `rebuild_visible`; selection through `visible`; `n`/`N` fallback |
| `src/lib.rs` (modify) | `mod scan`; `App` fields; `refresh_scan`, `drain_scan_results`, `poll_stamps`, `reload_active_file`; `r`; the badge; `set_mode` helper; `sigil` arm; `handle_filter_key` arm |
| `src/help.rs` (modify) | `KEYMAP` rows for `r`, `m`, navigator `n`/`N` |
| `README.md` (modify) | Keybinding rows; a paragraph on the three senses |
| `tests/scan_thread.rs` (**create**) | One integration test: a real `Scanner` thread over a fixture directory |

---

### Task 1: `Sense::Context` — a filter that shows lines and selects nothing

**Files:**
- Modify: `src/filter.rs` (`enum Sense`, `verdict`, `verdict_by_scanning`, `any_numbered_including`, tests)
- Modify: `src/lib.rs` (`SearchPrompt::sigil`, ~line 92)

**Interfaces:**
- Produces: `Sense::Context` variant; `ActiveFilters::toggle_context(&mut self, index: usize) -> bool`.

- [ ] **Step 1: Write the failing tests** in `src/filter.rs`'s `mod tests`, after `the_first_matching_filter_wins`:

```rust
    // ---- the third sense ------------------------------------------------

    /// A context filter shows its lines exactly as an include filter does.
    #[test]
    fn a_context_filter_includes_its_lines() {
        let mut set = set_with(&["foo"]);
        assert!(set.toggle_context(0));

        assert_eq!(set.filters()[0].sense, Sense::Context);
        assert_eq!(set.verdict("foo"), Verdict::Included(0));
        assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE), "context dims the rest");
    }

    #[test]
    fn toggle_context_round_trips_without_touching_the_pattern() {
        let mut set = set_with(&["foo", "bar"]);
        let before = set.filters()[1].style;

        assert!(set.toggle_context(1));
        assert!(set.toggle_context(1));

        assert_eq!(set.filters()[1].sense, Sense::Include);
        assert_eq!(set.filters()[1].pattern.as_str(), "bar");
        assert_eq!(set.filters()[1].style, before);
        assert_eq!(set.verdict("bar"), Verdict::Included(1));
    }

    /// An exclude filter is never context, and an index off the end is not a filter.
    #[test]
    fn toggle_context_leaves_excludes_and_missing_indices_alone() {
        let mut set = set_with(&["foo"]);
        set.add_excluding("noise").expect("valid pattern");

        assert!(!set.toggle_context(1));
        assert_eq!(set.filters()[1].sense, Sense::Exclude);
        assert!(!set.toggle_context(7));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib filter::tests::a_context_filter_includes_its_lines`
Expected: compile error — `no variant named Context`, `no method named toggle_context`.

- [ ] **Step 3: Add the variant and the toggle** in `src/filter.rs`:

```rust
/// Whether a filter selects lines, removes them, or shows them without
/// counting them.
///
/// `Context` is the third kind (#119). A realistic set for a folder of logs
/// holds patterns that *discriminate* — part of a bug's signature — and
/// patterns that pick out metadata every log carries: the commit, the host.
/// The second kind is wanted in the view and useless for choosing files. A
/// `Context` filter is an `Include` for every purpose except one: it never
/// selects a file in the navigator.
///
/// A variant rather than a flag on `Include`: an `Exclude` already never
/// selects a file, so "selects?" is not orthogonal to sense but one more value
/// of it — and the compiler then finds every `match` that needs to know.
///
/// Sense is the user's choice, per filter, in this set. It is not a property
/// of the pattern: `^host: production-.*` is `Include` when the question is
/// "which production logs have errors" and `Context` when it is "which logs
/// have bug 57, and where did they run". Nothing derives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sense {
    Include,
    Context,
    Exclude,
}
```

Then, in `impl ActiveFilters`, directly after `set_pattern`:

```rust
    /// Flip a filter between `Include` and `Context`, reporting whether it
    /// changed. An `Exclude` filter is left alone: it already selects nothing.
    ///
    /// No `recompile` — the pattern is untouched, so the compiled set is still
    /// right — and no `forget_capture`, for the same reason `set_pattern` gives:
    /// the set's shape is unchanged, so a pending `!` capture still describes it.
    pub fn toggle_context(&mut self, index: usize) -> bool {
        let Some(filter) = self.filters.get_mut(index) else {
            return false;
        };
        filter.sense = match filter.sense {
            Sense::Include => Sense::Context,
            Sense::Context => Sense::Include,
            Sense::Exclude => return false,
        };
        true
    }
```

Then make `verdict` and `verdict_by_scanning` treat `Context` as `Include` for a line. In **both** functions, change the include `find`:

```rust
            .find(|&(index, filter)| {
                filter.enabled && filter.sense != Sense::Exclude && matched(index)
            })
```

(in `verdict_by_scanning` the predicate is `filter.pattern.is_match(line)` instead of `matched(index)` — same `!= Sense::Exclude` change). And `any_numbered_including`:

```rust
    fn any_numbered_including(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense != Sense::Exclude)
    }
```

- [ ] **Step 4: Fix the exhaustive match in `src/lib.rs`** — `SearchPrompt::sigil` (~line 92) matches on `PromptKind::Edit { sense, .. }`. Editing a context filter keeps its sense, and the prompt label is the same as for an include:

```rust
            PromptKind::Filter
            | PromptKind::Edit {
                sense: filter::Sense::Include | filter::Sense::Context,
                ..
            } => "filter: ",
```

`src/widgets/filterlist.rs` also matches on `Sense` twice (`row_text`, `render`); Task 2 fixes those. To keep this task compiling on its own, add a temporary arm to each now: `Sense::Context => "inc",` in `row_text` and `Sense::Include | Sense::Context => filter.style,` in `render`. Task 2 replaces the first.

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib filter::`
Expected: all pass, including the three new ones.

- [ ] **Step 6: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/filter.rs src/lib.rs src/widgets/filterlist.rs
git commit -m "feat(filter): a third sense, Context, that shows lines and selects nothing

A realistic filter set for a folder of logs mixes patterns that
discriminate (a bug's signature) with patterns that pick out metadata
every log carries (the commit, the host). The second kind is wanted in
the view and useless for choosing files. Context is an Include for
every purpose except selecting a file (#119).

A variant rather than a flag: an Exclude already never selects, so
\"selects?\" is one more value of sense, and the compiler finds every
match that needs to know."
```

---

### Task 2: `m` in the filter pane toggles Context

**Files:**
- Modify: `src/widgets/mod.rs` (`enum FilterCommand`)
- Modify: `src/widgets/filterlist.rs` (`handle_key`, `row_text`, `render`, tests)
- Modify: `src/lib.rs` (`handle_filter_key`, tests)

**Interfaces:**
- Consumes: `ActiveFilters::toggle_context` (Task 1).
- Produces: `FilterCommand::ToggleContext(usize)`.

- [ ] **Step 1: Write the failing tests.** In `src/widgets/filterlist.rs`'s `mod tests`:

```rust
    #[test]
    fn m_reports_a_context_toggle_for_the_selected_filter() {
        let mut list = FilterList::default();
        list.select_next(2);
        list.select_next(2);

        let command = list.handle_key(KeyEvent::from(KeyCode::Char('m')), 2, false);

        assert_eq!(command, Some(FilterCommand::ToggleContext(1)));
    }

    /// The search cannot be context: it always selects. `m` on its row is nothing.
    #[test]
    fn m_on_the_search_row_does_nothing() {
        let mut list = FilterList::default();
        list.select_next(1);

        assert_eq!(list.handle_key(KeyEvent::from(KeyCode::Char('m')), 1, true), None);
    }

    #[test]
    fn a_context_filter_reads_ctx_in_its_row() {
        let mut filters = set_of(&["foo"], &[]);
        filters.toggle_context(0);

        assert_eq!(FilterList::row_text(&filters, 0), "1[x] ctx foo");
    }
```

And in `src/lib.rs`'s `mod tests`, near `app_with_two_filters`:

```rust
    #[test]
    fn m_in_the_filter_pane_flips_the_selected_filter_to_context_and_back() {
        let mut app = app_with_two_filters("ctx_toggle");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('j'));

        key(&mut app, KeyCode::Char('m'));
        assert_eq!(app.filters.filters()[0].sense, filter::Sense::Context);

        key(&mut app, KeyCode::Char('m'));
        assert_eq!(app.filters.filters()[0].sense, filter::Sense::Include);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib m_`
Expected: compile error — `no variant ToggleContext`; the row-text test fails with `inc` where `ctx` is expected.

- [ ] **Step 3: Add the command** in `src/widgets/mod.rs`, inside `enum FilterCommand` after `Edit(usize)`:

```rust
    /// Flip the selected numbered filter between `Include` and `Context`. No
    /// search variant: the search always selects, so it has no context form.
    ToggleContext(usize),
```

In `src/widgets/filterlist.rs`, `handle_key`, after the `c` arm:

```rust
            // `m` as in *metadata*: the filter keeps showing its lines but
            // stops choosing files in the navigator (#119). `target` is `None`
            // on the search row, and the search has no context form.
            KeyCode::Char('m') => target(self.selected()?).map(FilterCommand::ToggleContext),
```

Replace the temporary arm from Task 1 in `row_text`:

```rust
        let sense = match filter.sense {
            Sense::Include => "inc",
            Sense::Context => "ctx",
            Sense::Exclude => "exc",
        };
```

The `render` arm from Task 1 (`Sense::Include | Sense::Context => filter.style`) stays — a context filter wears its colour like an include.

In `src/lib.rs`, `handle_filter_key`, add an arm after `FilterCommand::Delete(index)`:

```rust
            FilterCommand::ToggleContext(index) => {
                self.filters.toggle_context(index);
            }
```

It falls through to the `refresh_view` at the bottom, like `Toggle` does.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib m_ && cargo test --lib filterlist::`
Expected: all pass.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/widgets/mod.rs src/widgets/filterlist.rs src/lib.rs
git commit -m "feat(filter pane): m toggles the selected filter between include and context

Reported as FilterCommand::ToggleContext, the same route Toggle, Delete
and Edit take: the pane only borrows the set, so App applies it. The
row reads ctx beside inc and exc. Nothing on the search row — the
search always selects, so it has no context form (#119)."
```

---

### Task 3: `Matcher` — the `Send` snapshot a scan thread holds

**Files:**
- Modify: `src/filter.rs` (new types after `EnabledFlags`; new methods on `ActiveFilters`; tests)

**Interfaces:**
- Consumes: `ActiveFilters::compiled`, `in_step` (existing).
- Produces:
  - `pub struct Matcher` (`Debug + Clone + Send`), with `bits(&self, line: &str) -> u64`, `selects(&self, bits: u64) -> bool`, `owner(&self, bits: u64) -> Option<Owner>`, `masks(&self) -> (u64, u64)`.
  - `pub enum Owner { Search, Filter(usize) }` (`Debug + Clone + Copy + PartialEq + Eq`), and `Owner::rank(self) -> usize`.
  - `ActiveFilters::matcher(&self) -> Option<Matcher>`; `ActiveFilters::pattern_key(&self) -> Vec<String>`.

- [ ] **Step 1: Write the failing tests** in `src/filter.rs`'s `mod tests`:

```rust
    // ---- the matcher snapshot --------------------------------------------

    /// The invariant the navigator rests on: `selects` agrees with `verdict`
    /// about which lines pick a file, across all three senses and the search.
    #[test]
    fn the_matcher_agrees_with_verdict_on_what_selects_a_file() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        set.add_excluding("noise").expect("valid pattern");
        set.set_search("gamma").expect("valid pattern");
        let matcher = set.matcher().expect("something selects");

        for line in [
            "alpha", "beta", "delta", "alpha noise", "beta delta", "gamma", "gamma noise",
            "beta gamma", "nothing here", "alpha beta",
        ] {
            let expected = match set.verdict(line) {
                Verdict::Searched => true,
                Verdict::Included(i) => set.filters()[i].sense == Sense::Include,
                Verdict::Unmatched | Verdict::Excluded => false,
            };
            assert_eq!(
                matcher.selects(matcher.bits(line)),
                expected,
                "matcher and verdict disagree on {line:?}"
            );
        }
    }

    /// The owner is the lowest *selecting* filter — which is not always the
    /// filter `verdict` colours the line with. A line hit by context filter 1
    /// and include filter 2 is drawn in filter 1's colour (first wins) but the
    /// *file* is owned by filter 2: it is the one that selected it.
    #[test]
    fn the_owner_is_the_lowest_selecting_filter_and_search_outranks_them() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        set.set_search("gamma").expect("valid pattern");
        let matcher = set.matcher().expect("something selects");

        assert_eq!(matcher.owner(matcher.bits("beta")), None);
        assert_eq!(matcher.owner(matcher.bits("beta delta")), Some(Owner::Filter(2)));
        assert_eq!(set.verdict("beta delta"), Verdict::Included(1), "the line is still beta's");
        assert_eq!(matcher.owner(matcher.bits("alpha delta")), Some(Owner::Filter(0)));
        assert_eq!(matcher.owner(matcher.bits("alpha gamma")), Some(Owner::Search));
    }

    #[test]
    fn a_disabled_filter_neither_selects_nor_excludes() {
        let mut set = set_with(&["alpha"]);
        set.add_excluding("noise").expect("valid pattern");
        set.set_enabled(1, false);
        let matcher = set.matcher().expect("alpha selects");

        assert!(matcher.selects(matcher.bits("alpha noise")));
    }

    /// Nothing selecting means nothing to match against — the #36 guard.
    #[test]
    fn no_matcher_without_a_selecting_filter() {
        assert!(ActiveFilters::new().matcher().is_none(), "empty set");

        let mut context_only = set_with(&["alpha"]);
        context_only.toggle_context(0);
        assert!(context_only.matcher().is_none(), "context only");

        let mut exclude_only = ActiveFilters::new();
        exclude_only.add_excluding("noise").expect("valid pattern");
        assert!(exclude_only.matcher().is_none(), "exclude only");

        let mut disabled = set_with(&["alpha"]);
        disabled.set_enabled(0, false);
        assert!(disabled.matcher().is_none(), "disabled");

        let mut search_only = ActiveFilters::new();
        search_only.set_search("gamma").expect("valid pattern");
        assert!(search_only.matcher().is_some(), "the search selects on its own");
    }

    /// 64 is the width of the bitset; the 65th pattern switches the feature off
    /// rather than wrapping a shift.
    #[test]
    fn no_matcher_past_sixty_four_patterns() {
        let mut set = ActiveFilters::new();
        for i in 0..64 {
            set.add(&format!("p{i}")).expect("valid pattern");
        }
        assert!(set.matcher().is_some());

        set.add("p64").expect("valid pattern");
        assert!(set.matcher().is_none());
    }

    #[test]
    fn the_pattern_key_lists_every_pattern_with_the_search_last() {
        let mut set = set_with(&["alpha", "beta"]);
        set.set_search("gamma").expect("valid pattern");

        assert_eq!(set.pattern_key(), vec!["alpha", "beta", "gamma"]);

        set.toggle_context(0);
        assert_eq!(set.pattern_key(), vec!["alpha", "beta", "gamma"], "sense is not part of the key");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib the_matcher_agrees`
Expected: compile error — `no method named matcher`.

- [ ] **Step 3: Add the types and methods.** In `src/filter.rs`, after `pub struct EnabledFlags { .. }`:

```rust
/// The bitset width. 63 numbered patterns plus the search; past this the
/// navigator's file matching switches off rather than shifting out of range.
const MAX_PATTERNS: usize = 64;

/// Which filter selected a file, for its colour in the navigator.
///
/// `Search` outranks every numbered filter, as it does per line in `verdict`.
/// Among numbered filters the lowest index wins — the view's "first matching
/// filter wins", applied per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Search,
    Filter(usize),
}

impl Owner {
    /// Lower is higher precedence. `Search` first, then filter order.
    #[must_use]
    pub fn rank(self) -> usize {
        match self {
            Self::Search => 0,
            Self::Filter(index) => index + 1,
        }
    }
}

/// A `Send` snapshot of the filter set, for a scan thread to match with (#119).
///
/// `ActiveFilters` is neither `Send` nor `Clone`. This is the three things a
/// scan needs from it: every pattern compiled into one `RegexSet` (cloning one
/// is an `Arc` bump), and which positions currently select or exclude. The set
/// covers every pattern whether or not it is enabled — a deliberate choice in
/// #86 — which is what makes a line's `bits` independent of the enabled mask,
/// and so reusable across toggles.
///
/// `Context` filters are in neither mask: they neither select nor exclude, so
/// a line only a context filter hit contributes nothing to a file's answer.
#[derive(Debug, Clone)]
pub struct Matcher {
    set: RegexSet,
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Include`; plus the
    /// search's bit when it is present and enabled.
    selects: u64,
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Exclude`.
    exclude: u64,
    /// The search's bit alone, or zero — so `owner` can rank it first.
    search: u64,
}

impl Matcher {
    /// Which patterns hit `line`, enabled or not.
    #[must_use]
    pub fn bits(&self, line: &str) -> u64 {
        self.set
            .matches(line)
            .iter()
            .fold(0, |bits, index| bits | (1 << index))
    }

    /// Whether a line with these hits selects its file. The hide-mode rule as
    /// a bit test, minus the context sense.
    #[must_use]
    pub fn selects(&self, bits: u64) -> bool {
        bits & self.selects != 0 && bits & self.exclude == 0
    }

    /// Which filter selected a line with these hits, if any.
    #[must_use]
    pub fn owner(&self, bits: u64) -> Option<Owner> {
        if !self.selects(bits) {
            return None;
        }
        if bits & self.search != 0 {
            return Some(Owner::Search);
        }
        Some(Owner::Filter((bits & self.selects).trailing_zeros() as usize))
    }

    /// `(selects, exclude)`, for the caller that wants to know whether a
    /// toggle changed anything a scan cares about.
    #[must_use]
    pub fn masks(&self) -> (u64, u64) {
        (self.selects, self.exclude)
    }
}
```

In `impl ActiveFilters`, after `verdict_by_scanning`:

```rust
    /// The snapshot a scan thread matches with, or `None` when there is no
    /// scan to run.
    ///
    /// `None` when nothing selects — no enabled `Include` and no enabled
    /// search, which is the same "nothing to match against" guard `Document`
    /// applies for #36 — and when the pattern count exceeds the bitset width.
    #[must_use]
    pub fn matcher(&self) -> Option<Matcher> {
        let set = self.compiled.as_ref().filter(|set| self.in_step(set))?;
        if set.len() > MAX_PATTERNS {
            return None;
        }
        let mut selects = 0u64;
        let mut exclude = 0u64;
        for (index, filter) in self.filters.iter().enumerate() {
            if !filter.enabled {
                continue;
            }
            match filter.sense {
                Sense::Include => selects |= 1 << index,
                Sense::Exclude => exclude |= 1 << index,
                Sense::Context => {}
            }
        }
        let mut search = 0u64;
        if self.search.as_ref().is_some_and(|search| search.enabled) {
            search = 1 << self.filters.len();
            selects |= search;
        }
        if selects == 0 {
            return None;
        }
        Some(Matcher {
            set: set.clone(),
            selects,
            exclude,
            search,
        })
    }

    /// Every pattern's source, in compiled order, search last. What a scan
    /// cache is keyed on: a change here shifts bit positions, so cached
    /// bitsets mean something else. Sense and enabled are deliberately not
    /// part of it — they are masks over the same bits.
    #[must_use]
    pub fn pattern_key(&self) -> Vec<String> {
        self.filters
            .iter()
            .chain(self.search.as_ref())
            .map(|filter| filter.pattern.as_str().to_string())
            .collect()
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib filter::`
Expected: all pass.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/filter.rs
git commit -m "feat(filter): Matcher, a Send snapshot of what selects and excludes

ActiveFilters is neither Send nor Clone, so a scan thread cannot hold
one. Matcher is the three things it needs: the RegexSet over every
pattern (an Arc bump to clone), and the selects/exclude masks. Because
the set covers patterns whether or not they are enabled (#86), a line's
bitset is mask-independent and a toggle never needs a rescan (#119).

The invariant test pins selects() to verdict() across all three senses.
The owner is the lowest selecting filter, which is not always the one
verdict colours the line with — a context filter can precede it — and
the test says so."
```

---

### Task 4: The pure scan core

**Files:**
- Create: `src/scan.rs`
- Modify: `src/lib.rs` (add `pub mod scan;` after `pub mod help;`)

**Interfaces:**
- Consumes: `filter::Matcher` (Task 3).
- Produces: `pub struct Progress { pub seen: Vec<u64>, pub scanned_to: u64, pub eof: bool }` (`Debug + Clone + Default + PartialEq + Eq`); `pub fn scan<R: BufRead>(reader: R, matcher: &Matcher, from: Progress, cancel: &AtomicBool) -> Progress`.

- [ ] **Step 1: Create `src/scan.rs` with the module doc and the tests only**

```rust
//! Which files would show a line under the active filters — answered in the
//! background, one file at a time, without loading any of them (#119).
//!
//! Two layers, kept apart so the interesting behaviour is testable without a
//! thread. [`scan`] is a pure core over any `BufRead`: it records which
//! patterns hit each line as a bitset and stops at the first line that selects
//! the file. [`Scanner`] is the thread that drives it per file and streams
//! [`Scanned`] results over a channel.
//!
//! The bitsets are the point. A file matches under a mask iff some line's
//! bitset has a selecting bit and no excluding bit — a few `u64` ops — so a
//! filter toggle re-answers a whole folder with no I/O. See the design at
//! `docs/specs/2026-09-02-navigator-filter-matches-design.md`.

use crate::filter::Matcher;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::ActiveFilters;
    use std::io::Cursor;

    fn matcher(includes: &[&str], excludes: &[&str]) -> Matcher {
        let mut set = ActiveFilters::new();
        for pattern in includes {
            set.add(pattern).expect("valid pattern");
        }
        for pattern in excludes {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set.matcher().expect("something selects")
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn stops_at_the_first_selecting_line() {
        let text = "one\ntwo\nalpha here\nthree\n";
        let progress = scan(Cursor::new(text), &matcher(&["alpha"], &[]), Progress::default(), &never());

        assert!(!progress.eof, "kept reading past the first match");
        assert_eq!(progress.scanned_to, "one\ntwo\nalpha here\n".len() as u64);
        assert!(progress.seen.contains(&0b1), "the matching line's bitset was not recorded");
    }

    #[test]
    fn reads_to_eof_when_nothing_selects() {
        let text = "one\ntwo\nthree";
        let progress = scan(Cursor::new(text), &matcher(&["alpha"], &[]), Progress::default(), &never());

        assert!(progress.eof);
        assert_eq!(progress.scanned_to, text.len() as u64, "a last line without a newline still counts");
        assert_eq!(progress.seen, vec![0]);
    }

    #[test]
    fn resumes_from_where_it_stopped_and_reads_nothing_twice() {
        let text = "alpha\nbeta\n";
        let m = matcher(&["alpha", "beta"], &[]);
        let first = scan(Cursor::new(text), &m, Progress::default(), &never());
        assert_eq!(first.scanned_to, 6);

        // The driver seeks; the core is handed a reader already positioned.
        let mut rest = Cursor::new(text);
        rest.set_position(first.scanned_to);
        let second = scan(rest, &m, first.clone(), &never());

        assert_eq!(second.scanned_to, text.len() as u64);
        assert!(second.seen.contains(&0b10));
    }

    #[test]
    fn distinct_bitsets_are_recorded_once_each() {
        let text = "x\nx\nx\nbeta\nbeta\n";
        // `beta` is a context-only hit: it must be recorded but must not stop the scan.
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add("beta").expect("valid pattern");
        set.toggle_context(1);
        let progress = scan(Cursor::new(text), &set.matcher().expect("alpha selects"), Progress::default(), &never());

        assert!(progress.eof);
        assert_eq!(progress.seen, vec![0, 0b10]);
    }

    #[test]
    fn an_excluded_line_does_not_select_but_is_still_recorded() {
        let text = "alpha noise\nalpha\n";
        let progress = scan(Cursor::new(text), &matcher(&["alpha"], &["noise"]), Progress::default(), &never());

        assert_eq!(progress.scanned_to, text.len() as u64, "stopped on the excluded line");
        assert_eq!(progress.seen, vec![0b11, 0b01]);
    }

    #[test]
    fn cancel_returns_what_it_had_so_far() {
        let text = "one\ntwo\n";
        let cancel = AtomicBool::new(true);
        let progress = scan(Cursor::new(text), &matcher(&["alpha"], &[]), Progress::default(), &cancel);

        assert_eq!(progress, Progress::default(), "read a line after being told to stop");
    }

    #[test]
    fn a_line_that_is_not_utf8_does_not_abort_the_file() {
        let bytes = b"one\n\xff\xfe bad\nalpha\n";
        let progress = scan(Cursor::new(&bytes[..]), &matcher(&["alpha"], &[]), Progress::default(), &never());

        assert_eq!(progress.scanned_to, bytes.len() as u64);
        assert!(progress.seen.contains(&0b1));
    }

    /// Patterns anchored at the end must see the line without its newline,
    /// the way `Document` lines have none.
    #[test]
    fn the_newline_is_not_part_of_the_line() {
        let progress = scan(Cursor::new("alpha\r\n"), &matcher(&["alpha$"], &[]), Progress::default(), &never());

        assert!(progress.seen.contains(&0b1));
    }
}
```

- [ ] **Step 2: Add `pub mod scan;` to `src/lib.rs`** after `pub mod help;`, and run to verify the tests fail

Run: `cargo test --lib scan::`
Expected: compile error — `Progress` and `scan` not found.

- [ ] **Step 3: Write the core** above the `mod tests` in `src/scan.rs`:

```rust
/// How far one file has been read, and what its lines matched.
///
/// `seen` holds every *distinct* per-line bitset met so far, deduplicated. It
/// stays tiny — a real log has single-digit distinct match combinations — so a
/// `Vec` with a linear `contains` beats a hash set. `scanned_to` is a byte
/// offset at a line boundary, which is what lets a later scan resume rather
/// than restart. `eof` says whether `seen` is complete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub seen: Vec<u64>,
    pub scanned_to: u64,
    pub eof: bool,
}

/// Read lines from `reader` — already positioned at `from.scanned_to` — and
/// record what each one matched, stopping at the first line that selects the
/// file, at EOF, or when `cancel` is set.
///
/// Early exit is why a matching file is free: a 2 GB log that matches on line
/// three costs three lines. The price is that `seen` is only complete at
/// `eof`, which `Record::answer` accounts for.
///
/// Bytes, not `str`: a log with one bad byte on line 40,000 must still get an
/// answer. `from_utf8_lossy` is a `Cow` that allocates only on an invalid line,
/// the same tolerance `read_lines` got in 7d6e587. The newline is stripped so
/// `foo$` matches the way it does against a `Document` line.
///
/// `cancel` is checked per line — an atomic load, not a syscall — and a
/// cancelled scan returns what it has. Nothing read is ever thrown away.
pub fn scan<R: BufRead>(
    mut reader: R,
    matcher: &Matcher,
    mut progress: Progress,
    cancel: &AtomicBool,
) -> Progress {
    let mut buf = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return progress;
        }
        buf.clear();
        let read = match reader.read_until(b'\n', &mut buf) {
            Ok(read) => read,
            Err(err) => {
                log::warn!("scan stopped early: {err}");
                progress.eof = true;
                return progress;
            }
        };
        if read == 0 {
            progress.eof = true;
            return progress;
        }
        progress.scanned_to += read as u64;
        let line = String::from_utf8_lossy(&buf);
        let bits = matcher.bits(line.trim_end_matches(['\n', '\r']));
        if !progress.seen.contains(&bits) {
            progress.seen.push(bits);
        }
        if matcher.selects(bits) {
            return progress;
        }
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib scan::`
Expected: all eight pass.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/scan.rs src/lib.rs
git commit -m "feat(scan): a pure core that records what each line matched

Reads lines from a BufRead, ORs each line's RegexSet bitset into a
deduplicated set, and stops at the first line that selects the file,
at EOF, or on cancel. Takes a reader and returns a value: every
interesting behaviour is tested over a Cursor with no thread in sight.

The bitsets are why a filter toggle will not need a rescan; the early
exit is why a matching file is free; scanned_to is why nothing is ever
read twice (#119)."
```

---

### Task 5: `Record` — a file's cached answer, and when it can be given

**Files:**
- Modify: `src/scan.rs` (types + tests)

**Interfaces:**
- Consumes: `Progress` (Task 4), `filter::{Matcher, Owner}` (Task 3).
- Produces: `pub type Stamp = (SystemTime, u64)`; `pub fn stamp(path: &Path) -> io::Result<Stamp>`; `pub struct Record { pub stamp: Option<Stamp>, pub progress: Progress }` (`Debug + Clone`) with `answer(&self, m: &Matcher) -> Option<bool>` and `owner(&self, m: &Matcher) -> Option<Owner>`.

- [ ] **Step 1: Write the failing tests** in `src/scan.rs`'s `mod tests`:

```rust
    // ---- records ---------------------------------------------------------

    fn record(seen: &[u64], eof: bool) -> Record {
        Record {
            stamp: None,
            progress: Progress {
                seen: seen.to_vec(),
                scanned_to: 0,
                eof,
            },
        }
    }

    #[test]
    fn a_seen_selecting_bitset_answers_yes_without_reading() {
        let m = matcher(&["alpha"], &[]);
        assert_eq!(record(&[0, 0b1], false).answer(&m), Some(true));
    }

    #[test]
    fn eof_with_no_selecting_bitset_answers_no() {
        let m = matcher(&["alpha"], &[]);
        assert_eq!(record(&[0], true).answer(&m), Some(false));
    }

    #[test]
    fn partial_with_no_selecting_bitset_needs_a_resume() {
        let m = matcher(&["alpha"], &[]);
        assert_eq!(record(&[0], false).answer(&m), None);
    }

    /// The same bitsets, a different mask: this is the toggle that costs no I/O.
    #[test]
    fn the_answer_follows_the_mask_not_the_scan() {
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add_excluding("noise").expect("valid pattern");
        let rec = record(&[0b11], true); // every alpha line also had noise

        assert_eq!(rec.answer(&set.matcher().expect("selects")), Some(false));
        set.set_enabled(1, false);
        assert_eq!(rec.answer(&set.matcher().expect("selects")), Some(true));
    }

    #[test]
    fn the_owner_is_the_highest_ranked_across_every_seen_bitset() {
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add("beta").expect("valid pattern");
        set.set_search("gamma").expect("valid pattern");
        let m = set.matcher().expect("selects");

        assert_eq!(record(&[0b010, 0b001], false).owner(&m), Some(Owner::Filter(0)));
        assert_eq!(record(&[0b010, 0b100], false).owner(&m), Some(Owner::Search));
        assert_eq!(record(&[0], true).owner(&m), None);
    }

    #[test]
    fn stamp_reads_mtime_and_length() {
        let dir = std::path::Path::new("target/test-scan");
        std::fs::create_dir_all(dir).expect("fixture dir");
        let path = dir.join("stamp.txt");
        std::fs::write(&path, "hello").expect("write");

        let (_, len) = stamp(&path).expect("stat");
        assert_eq!(len, 5);
        assert!(stamp(&dir.join("missing.txt")).is_err());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib scan::tests::a_seen`
Expected: compile error — `Record` not found.

- [ ] **Step 3: Add the types** in `src/scan.rs` after `Progress`:

```rust
use crate::filter::Owner;
use std::path::Path;
use std::time::SystemTime;

/// `(mtime, len)` of a file when it was scanned. A mismatch on re-stat means
/// the record is for a file that no longer exists in that form.
pub type Stamp = (SystemTime, u64);

/// Read a file's [`Stamp`].
///
/// # Errors
/// Whatever `fs::metadata` reports — a missing file, no permission.
pub fn stamp(path: &Path) -> std::io::Result<Stamp> {
    let meta = std::fs::metadata(path)?;
    Ok((meta.modified()?, meta.len()))
}

/// One file's scan state, held in `App`'s cache.
///
/// `stamp` is `None` when the file could not be stat'd; two `None`s compare
/// equal, so an unreadable file is not re-tried on every poll.
#[derive(Debug, Clone)]
pub struct Record {
    pub stamp: Option<Stamp>,
    pub progress: Progress,
}

impl Record {
    /// Whether the file matches under `m`, if that can be known from what has
    /// been read. `None` means resume the scan from `progress.scanned_to`.
    ///
    /// Three outcomes, and the middle one is what makes early exit and the
    /// cache coexist: a selecting bitset answers yes at once; `eof` with none
    /// answers no; a partial read with none is the only case that costs I/O,
    /// and only for the unread remainder.
    #[must_use]
    pub fn answer(&self, m: &Matcher) -> Option<bool> {
        if self.progress.seen.iter().any(|&bits| m.selects(bits)) {
            return Some(true);
        }
        if self.progress.eof {
            return Some(false);
        }
        None
    }

    /// Which filter selected the file, for its colour: the highest-ranked
    /// owner across every seen bitset.
    #[must_use]
    pub fn owner(&self, m: &Matcher) -> Option<Owner> {
        self.progress
            .seen
            .iter()
            .filter_map(|&bits| m.owner(bits))
            .min_by_key(|owner| owner.rank())
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib scan::`
Expected: all pass.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/scan.rs
git commit -m "feat(scan): Record answers a file from its bitsets, or asks for a resume

Three outcomes. A seen selecting bitset is yes with no I/O; eof with
none is no with no I/O; a partial read with none is the only case that
reads, and only the unread remainder. That is how early exit and a
toggle-proof cache coexist (#119)."
```

---

### Task 6: The `Scanner` thread, the `Scan` trait, and its recording double

**Files:**
- Modify: `src/scan.rs`
- Create: `tests/scan_thread.rs`

**Interfaces:**
- Consumes: `scan`, `Progress`, `Record`, `Stamp`, `stamp` (Tasks 4–5); `filter::Matcher`.
- Produces:
  - `pub struct Request { pub cache_id: u64, pub matcher: Matcher, pub files: Vec<(usize, PathBuf, Progress)> }` (`Debug + Clone`)
  - `pub struct Scanned { pub cache_id: u64, pub index: usize, pub path: PathBuf, pub stamp: Option<Stamp>, pub progress: Progress }` (`Debug + Clone`)
  - `pub trait Scan { fn start(&self, request: Request); fn cancel(&self); }`
  - `pub struct Scanner` with `Scanner::new(tx: Sender<Scanned>) -> Self`, implementing `Scan`
  - `impl Default for Box<dyn Scan>` (a no-op scanner, so `App` can keep deriving `Default`)
  - `pub(crate) mod double { pub(crate) struct RecordingScanner { pub requests: Mutex<Vec<Request>>, pub cancels: Mutex<usize> } }` implementing `Scan` for itself and for `Rc<RecordingScanner>`

- [ ] **Step 1: Write the failing integration test** at `tests/scan_thread.rs`:

```rust
//! The one test that runs a real `Scanner` thread. Everything else about
//! scanning is tested over a `Cursor` in `src/scan.rs`.

use recon::filter::ActiveFilters;
use recon::scan::{Progress, Request, Scan, Scanned, Scanner};
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn collect(rx: &mpsc::Receiver<Scanned>, want: usize) -> Vec<Scanned> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while got.len() < want {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(scanned) => got.push(scanned),
            Err(_) => panic!("timed out with {} of {want} results: {got:?}", got.len()),
        }
    }
    got
}

#[test]
fn a_scanner_thread_answers_every_file_it_is_given() {
    let dir = Path::new("target/test-scan-thread");
    fs::create_dir_all(dir).expect("fixture dir");
    let hit = dir.join("hit.log");
    let miss = dir.join("miss.log");
    let gone = dir.join("gone.log");
    fs::write(&hit, "one\nERROR deploy failed\nthree\n").expect("write");
    fs::write(&miss, "quiet\nquieter\n").expect("write");
    fs::remove_file(&gone).ok();

    let mut set = ActiveFilters::new();
    set.add("ERROR").expect("valid pattern");
    let (tx, rx) = mpsc::channel();
    let scanner = Scanner::new(tx);

    scanner.start(Request {
        cache_id: 7,
        matcher: set.matcher().expect("selects"),
        files: vec![
            (1, hit.clone(), Progress::default()),
            (2, miss.clone(), Progress::default()),
            (3, gone.clone(), Progress::default()),
        ],
    });

    let mut results = collect(&rx, 3);
    results.sort_by_key(|scanned| scanned.index);
    let matcher = set.matcher().expect("selects");

    assert!(results.iter().all(|scanned| scanned.cache_id == 7));
    let hit = &results[0];
    assert!(!hit.progress.eof, "kept reading past the match");
    assert!(hit.progress.seen.iter().any(|&bits| matcher.selects(bits)));
    assert!(hit.stamp.is_some());

    let miss = &results[1];
    assert!(miss.progress.eof);
    assert!(!miss.progress.seen.iter().any(|&bits| matcher.selects(bits)));

    let gone = &results[2];
    assert!(gone.progress.eof, "an unreadable file must answer, not hang");
    assert!(gone.stamp.is_none());
}

#[test]
fn a_new_request_cancels_the_old_one() {
    let dir = Path::new("target/test-scan-thread-cancel");
    fs::create_dir_all(dir).expect("fixture dir");
    let big = dir.join("big.log");
    let mut text = String::new();
    for i in 0..200_000 {
        text.push_str(&format!("line {i}\n"));
    }
    fs::write(&big, &text).expect("write");

    let mut set = ActiveFilters::new();
    set.add("never matches").expect("valid pattern");
    let (tx, rx) = mpsc::channel();
    let scanner = Scanner::new(tx);
    let request = |cache_id| Request {
        cache_id,
        matcher: set.matcher().expect("selects"),
        files: vec![(0, big.clone(), Progress::default())],
    };

    scanner.start(request(1));
    scanner.start(request(2));

    // Both workers report — the cancelled one with whatever it had.
    let results = collect(&rx, 2);
    assert!(results.iter().any(|scanned| scanned.cache_id == 2 && scanned.progress.eof));
    assert!(results.iter().any(|scanned| scanned.cache_id == 1));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test scan_thread`
Expected: compile error — `Request`, `Scan`, `Scanner` not found.

- [ ] **Step 3: Write the driver** in `src/scan.rs`, after `Record`:

```rust
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};

/// One scan: which files, from where, matched with what.
///
/// `cache_id` is echoed on every [`Scanned`] so the receiver can drop results
/// from a cache that has since been replaced. `files` carries each file's
/// existing [`Progress`] so the worker resumes rather than restarts.
#[derive(Debug, Clone)]
pub struct Request {
    pub cache_id: u64,
    pub matcher: Matcher,
    pub files: Vec<(usize, PathBuf, Progress)>,
}

/// One file's result. `index` is the navigator row the request named; the
/// receiver checks it still names `path` before using it.
#[derive(Debug, Clone)]
pub struct Scanned {
    pub cache_id: u64,
    pub index: usize,
    pub path: PathBuf,
    pub stamp: Option<Stamp>,
    pub progress: Progress,
}

/// Something that runs scan requests. `&self`, like `editor::Launcher`, so a
/// test can hold an `Rc` to a recording double while `App` owns the box.
pub trait Scan {
    /// Start scanning. An in-flight scan is cancelled first; its partial
    /// results still arrive.
    fn start(&self, request: Request);
    /// Stop the in-flight scan, if any.
    fn cancel(&self);
}

/// The real thing: at most one worker thread, results over an `mpsc` channel.
///
/// Holds no cache and no state between requests — that is `App`'s. Its one
/// piece of state is the cancel flag of the current worker.
pub struct Scanner {
    tx: Sender<Scanned>,
    cancel: Mutex<Arc<AtomicBool>>,
}

impl Scanner {
    #[must_use]
    pub fn new(tx: Sender<Scanned>) -> Self {
        Self {
            tx,
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
        }
    }
}

impl Scan for Scanner {
    fn start(&self, request: Request) {
        self.cancel();
        let flag = Arc::new(AtomicBool::new(false));
        *self
            .cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Arc::clone(&flag);
        let tx = self.tx.clone();
        std::thread::spawn(move || worker(request, &tx, &flag));
    }

    fn cancel(&self) {
        self.cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store(true, Ordering::Relaxed);
    }
}

/// The thread body. One file at a time; a cancel between files stops the
/// walk, a cancel inside one returns that file's partial progress — and it is
/// still sent, so nothing read is thrown away.
fn worker(request: Request, tx: &Sender<Scanned>, cancel: &AtomicBool) {
    let Request {
        cache_id,
        matcher,
        files,
    } = request;
    for (index, path, progress) in files {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let stamp = stamp(&path).ok();
        let progress = match File::open(&path) {
            Ok(mut file) => {
                if let Err(err) = file.seek(SeekFrom::Start(progress.scanned_to)) {
                    log::warn!("{}: cannot resume at {}: {err}", path.display(), progress.scanned_to);
                }
                scan(BufReader::new(file), &matcher, progress, cancel)
            }
            // Unreadable answers "no", complete: it will show nothing. Not
            // retried until its stamp changes.
            Err(err) => {
                log::warn!("{}: {err}", path.display());
                Progress {
                    eof: true,
                    ..progress
                }
            }
        };
        let sent = tx.send(Scanned {
            cache_id,
            index,
            path,
            stamp,
            progress,
        });
        if sent.is_err() {
            return;
        }
    }
}

/// Runs nothing. So `App` can hold a `Box<dyn Scan>` in a `#[derive(Default)]`
/// struct without the field becoming an `Option` — the same reason
/// `editor::Launcher` has one. `App::new` replaces it with a real `Scanner`.
struct NoScanner;

impl Scan for NoScanner {
    fn start(&self, _: Request) {}
    fn cancel(&self) {}
}

impl Default for Box<dyn Scan> {
    fn default() -> Self {
        Box::new(NoScanner)
    }
}

/// Test doubles. `pub(crate)` so `lib.rs`'s tests can install one.
#[cfg(test)]
pub(crate) mod double {
    use super::{Request, Scan};
    use std::sync::{Mutex, PoisonError};

    /// Records every request and cancel, runs nothing.
    #[derive(Default)]
    pub(crate) struct RecordingScanner {
        pub requests: Mutex<Vec<Request>>,
        pub cancels: Mutex<usize>,
    }

    impl RecordingScanner {
        pub(crate) fn requests(&self) -> Vec<Request> {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl Scan for RecordingScanner {
        fn start(&self, request: Request) {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
        }

        fn cancel(&self) {
            *self.cancels.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        }
    }

    impl Scan for std::rc::Rc<RecordingScanner> {
        fn start(&self, request: Request) {
            (**self).start(request);
        }

        fn cancel(&self) {
            (**self).cancel();
        }
    }
}
```

Reorganise the `use` lines at the top of the file into one block (rustfmt will order them).

- [ ] **Step 4: Run the tests**

Run: `cargo test --test scan_thread && cargo test --lib scan::`
Expected: all pass. The cancel test may take a second.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/scan.rs tests/scan_thread.rs
git commit -m "feat(scan): a Scanner thread that drives the core per file over mpsc

At most one worker; a new request cancels the old one, whose partial
progress is still sent — nothing read is thrown away. Unreadable files
answer no, complete. The Scan trait takes &self like editor::Launcher
so a test can hold an Rc to the recording double while App owns the
box (#119)."
```

---

### Task 7: `FileNav` learns an answer per entry

**Files:**
- Modify: `src/widgets/filenav.rs` (`Entry`, `read_dir_entries`/`sorted_entries` constructors, `rebuild_list`, new methods, tests)
- Modify: `src/widgets/fileview.rs` (the `Entry { .. }` literal in `the_listing_name_column_aligns_across_wide_glyphs` gains `matched: Match::Unknown`)

**Interfaces:**
- Produces: `pub(crate) enum Match { Unknown, No, Yes(Style) }` (`Debug + Clone + Copy + PartialEq + Eq + Default`, default `Unknown`); `Entry.matched: Match`; `FileNav::files(&self) -> Vec<(usize, PathBuf)>`; `FileNav::path_at(&self, index: usize) -> Option<PathBuf>`; `FileNav::set_answer(&mut self, index: usize, matched: Match) -> bool`; `FileNav::restyle(&mut self)`.

- [ ] **Step 1: Write the failing tests** in `src/widgets/filenav.rs`'s `mod tests`:

```rust
    // ---- filter matches ---------------------------------------------------

    fn row_style_of(nav: &mut FileNav<'_>, name: &str) -> Style {
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        nav.render(area, &mut buf);
        let y = nav.entries().iter().position(|e| e.name == name).expect("listed") as u16 + 1;
        buf[(4, y)].style()
    }

    #[test]
    fn files_lists_every_non_directory_with_its_index_and_absolute_path() {
        let nav = nav_over("match_files", &["a.log", "b.log"]);
        std::fs::create_dir_all(nav.dir().join("sub")).expect("subdir");
        let nav = FileNav::new(nav.dir().join("placeholder").display().to_string());

        let files = nav.files();
        let names: Vec<_> = files.iter().map(|(_, p)| p.file_name().unwrap().to_owned()).collect();
        assert_eq!(names, ["a.log", "b.log"], "directories and .. must not be listed");
        assert!(files.iter().all(|(i, p)| nav.path_at(*i).as_ref() == Some(p)));
        assert!(files.iter().all(|(_, p)| p.is_absolute()));
    }

    #[test]
    fn a_yes_answer_draws_the_name_in_the_style_it_was_given() {
        let mut nav = nav_over("match_yes", &["a.log"]);
        let index = nav.files()[0].0;
        let style = Style::default().fg(Color::Magenta);

        assert!(nav.set_answer(index, Match::Yes(style)));
        nav.restyle();

        assert_eq!(row_style_of(&mut nav, "a.log").fg, Some(Color::Magenta));
    }

    #[test]
    fn a_no_answer_dims_the_name_and_unknown_leaves_it_alone() {
        let mut nav = nav_over("match_no", &["a.log", "b.log"]);
        let (a, b) = (nav.files()[0].0, nav.files()[1].0);
        let plain = row_style_of(&mut nav, "b.log");

        nav.set_answer(a, Match::No);
        nav.set_answer(b, Match::Unknown);
        nav.restyle();

        assert_eq!(row_style_of(&mut nav, "a.log"), DIM_STYLE);
        assert_eq!(row_style_of(&mut nav, "b.log"), plain);
    }

    #[test]
    fn set_answer_reports_whether_anything_changed() {
        let mut nav = nav_over("match_changed", &["a.log"]);
        let index = nav.files()[0].0;

        assert!(!nav.set_answer(index, Match::Unknown), "unknown to unknown");
        assert!(nav.set_answer(index, Match::No));
        assert!(!nav.set_answer(index, Match::No));
        assert!(!nav.set_answer(99, Match::No), "no such row");
    }

    /// The filename search asked for that name by name; it keeps its highlight.
    #[test]
    fn a_search_hit_stays_yellow_whatever_its_answer() {
        let mut nav = nav_over("match_search", &["a.log"]);
        let index = nav.files()[0].0;
        nav.search("a", false).expect("valid pattern");
        nav.set_answer(index, Match::No);
        nav.restyle();

        assert_eq!(row_style_of(&mut nav, "a.log"), MATCH_STYLE);
    }

    #[test]
    fn a_new_listing_forgets_every_answer() {
        let mut nav = nav_over("match_reset", &["a.log"]);
        nav.set_answer(nav.files()[0].0, Match::No);
        std::fs::create_dir_all(nav.dir().join("sub")).expect("subdir");

        nav.set_dir(nav.dir().join("sub"), Select::First);
        nav.go_to_parent();

        assert!(nav.entries().iter().all(|e| e.matched == Match::Unknown));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib filenav::tests::a_yes_answer`
Expected: compile error — `Match` not found, `files`/`set_answer`/`restyle` not found.

- [ ] **Step 3: Add `Match` and the field.** In `src/widgets/filenav.rs`, before `pub(crate) struct Entry`:

```rust
/// Whether a file would show a line under the active filters — the answer the
/// navigator is *told*, never one it works out (#119).
///
/// `Yes` carries a ready style: the colour of the filter that selected the
/// file, decided by `App`, which is the only thing that can see the filters.
/// The navigator needs to know nothing about filters to draw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Match {
    /// Not yet scanned, or the feature is off. Drawn plain, never hidden:
    /// you do not hide what you have not read.
    #[default]
    Unknown,
    No,
    Yes(Style),
}
```

Add to `Entry`, after `modified`:

```rust
    /// See [`Match`]. Always `Unknown` for a directory and for `..`.
    pub matched: Match,
```

Every `Entry { .. }` literal gains `matched: Match::Unknown` — in `read_dir_entries`, `sorted_entries` (or wherever the constructor lives; `grep -n "Entry {" src/widgets/filenav.rs src/widgets/fileview.rs`), and the test literal in `src/widgets/fileview.rs`. If the constructor uses `..Default::default()` nothing is needed there.

- [ ] **Step 4: Add the methods.** In `impl FileNav`, after `selected_path`:

```rust
    /// Every file in the listing, as `(entries index, absolute path)`.
    /// Directories and `..` are not files and are never scanned.
    pub(crate) fn files(&self) -> Vec<(usize, PathBuf)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !matches!(entry.kind, Kind::Dir | Kind::Parent))
            .map(|(index, entry)| (index, self.dir.join(&entry.name)))
            .collect()
    }

    /// The absolute path at `index`, so a result can be checked against the
    /// row it was requested for before being applied to it.
    pub(crate) fn path_at(&self, index: usize) -> Option<PathBuf> {
        self.entries.get(index).map(|entry| self.dir.join(&entry.name))
    }

    /// Record an answer for one row, reporting whether it changed. Does not
    /// redraw — a burst of answers arrives together, so the caller calls
    /// `restyle` once afterwards rather than paying a list rebuild per row.
    pub(crate) fn set_answer(&mut self, index: usize, matched: Match) -> bool {
        match self.entries.get_mut(index) {
            Some(entry) if entry.matched != matched => {
                entry.matched = matched;
                true
            }
            _ => false,
        }
    }

    /// Rebuild the drawn rows from the current answers.
    pub(crate) fn restyle(&mut self) {
        self.rebuild_list();
    }
```

In `rebuild_list`, replace the style computation:

```rust
                let style = match matcher {
                    // Asked for by name: the search highlight outranks the answer.
                    Some(pattern) if pattern.is_match(&entry.matchable()) => MATCH_STYLE,
                    _ => match entry.matched {
                        Match::Yes(style) => style,
                        Match::No => DIM_STYLE,
                        Match::Unknown => entry.style(),
                    },
                };
```

`set_dir` rebuilds `entries` from disk, so every answer is `Unknown` again by construction — the last test pins that.

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib filenav::`
Expected: all pass.

- [ ] **Step 6: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/widgets/filenav.rs src/widgets/fileview.rs
git commit -m "feat(filenav): each entry carries an answer it is told, and draws it

Match::{Unknown, No, Yes(Style)}. Yes carries the colour of the filter
that selected the file, decided by App; the navigator knows nothing
about filters. No dims, Unknown is plain, and a filename-search hit
keeps its yellow over any of them — you asked for that name by name.
restyle() is separate from set_answer() so a burst of answers costs one
list rebuild, not one per row (#119)."
```

---

### Task 8: Hide mode in the navigator, selection through `visible`

**Files:**
- Modify: `src/widgets/filenav.rs` (struct fields; `set_dir`; `select_next`; `selected_path`; `activate_selection`; `step_search`; new methods; tests)

**Interfaces:**
- Consumes: `document::Mode`, `Match` (Task 7).
- Produces: `FileNav::set_mode(&mut self, mode: Mode)`; `FileNav::rebuild_visible(&mut self)` (replaces `restyle` as the thing `App` calls after a batch of answers — keep `restyle` as an alias that calls it); `FileNav::selected_entry(&self) -> Option<usize>`; `FileNav::select_entry(&mut self, index: usize)`.

- [ ] **Step 1: Write the failing tests** in `src/widgets/filenav.rs`'s `mod tests`:

```rust
    // ---- hide mode --------------------------------------------------------

    #[test]
    fn hide_mode_removes_no_answers_and_keeps_unknown_and_directories() {
        let mut nav = nav_over("hide_basic", &["no.log", "unk.log", "yes.log"]);
        std::fs::create_dir_all(nav.dir().join("sub")).expect("subdir");
        let mut nav = FileNav::new(nav.dir().join("placeholder").display().to_string());
        let idx = |nav: &FileNav<'_>, name: &str| nav.entries().iter().position(|e| e.name == name).unwrap();
        nav.set_answer(idx(&nav, "no.log"), Match::No);
        nav.set_answer(idx(&nav, "yes.log"), Match::Yes(Style::default()));

        nav.set_mode(Mode::FilteredOnly);
        assert_eq!(names(&nav), vec![PARENT, "sub", "unk.log", "yes.log"]);

        nav.set_mode(Mode::Dimmed);
        assert_eq!(names(&nav), vec![PARENT, "no.log", "sub", "unk.log", "yes.log"]);
    }

    #[test]
    fn the_selection_follows_its_entry_across_a_rebuild() {
        let mut nav = nav_over("hide_follow", &["no.log", "yes.log"]);
        let (no, yes) = (nav.files()[0].0, nav.files()[1].0);
        nav.set_answer(no, Match::No);
        nav.set_answer(yes, Match::Yes(Style::default()));
        nav.select_entry(yes);

        nav.set_mode(Mode::FilteredOnly);

        assert_eq!(nav.selected_entry(), Some(yes));
        assert_eq!(nav.selected_path().unwrap().file_name().unwrap(), "yes.log");
    }

    #[test]
    fn a_selection_whose_row_vanishes_clamps_to_a_neighbour_never_to_none() {
        let mut nav = nav_over("hide_clamp", &["a.log", "no.log"]);
        let no = nav.files()[1].0;
        nav.set_answer(no, Match::No);
        nav.select_entry(no);

        nav.set_mode(Mode::FilteredOnly);

        assert!(nav.selected_entry().is_some());
        assert_ne!(nav.selected_entry(), Some(no));
    }

    /// `j`, `Enter` and the filename search all walk the *visible* rows.
    #[test]
    fn movement_and_activation_see_only_visible_rows() {
        let mut nav = nav_over("hide_walk", &["a.log", "no.log", "z.log"]);
        let no = nav.files()[1].0;
        nav.set_answer(no, Match::No);
        nav.set_mode(Mode::FilteredOnly);
        nav.select_entry(nav.files()[0].0);

        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('j'))));
        assert_eq!(nav.selected_path().unwrap().file_name().unwrap(), "z.log", "j landed on a hidden row");

        nav.search("no", false).expect("valid pattern");
        assert_eq!(nav.selected_path().unwrap().file_name().unwrap(), "z.log", "search found a hidden row");
    }

    #[test]
    fn a_new_listing_shows_everything_again() {
        let mut nav = nav_over("hide_reset", &["no.log"]);
        nav.set_answer(nav.files()[0].0, Match::No);
        nav.set_mode(Mode::FilteredOnly);
        assert_eq!(names(&nav), vec![PARENT]);

        nav.go_to_parent();
        assert!(names(&nav).len() > 1);
    }
```

(`names(&nav)` already exists in this test module and reads the drawn rows; if it reads `entries` directly, change it to read `visible` — the point is what is *listed*.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib filenav::tests::hide_mode`
Expected: compile error — `set_mode`, `select_entry`, `selected_entry` not found.

- [ ] **Step 3: Add the fields and the visible model.** In `pub struct FileNav`, after `widest`:

```rust
    /// Whether rows answered `Match::No` are dimmed or removed. Pushed in by
    /// `App` so it is always the file view's mode too: one key, one meaning,
    /// both panes (#119).
    mode: Mode,
    /// Indices into `entries` of the rows currently listed, in order — the
    /// model `Document` uses for lines. `visible[0]` is always `..`, and
    /// `state.selected()` indexes *this*, not `entries`.
    visible: Vec<usize>,
```

Add `use crate::document::Mode;` to the imports.

Replace `set_dir`:

```rust
    fn set_dir(&mut self, dir: PathBuf, select: Select) {
        self.dir = crate::path::lexical_absolute(&dir);
        self.entries = read_dir_entries(&self.dir);
        // Fresh entries are all `Unknown`, so every row is visible whatever
        // the mode; `rebuild_visible` also rebuilds the drawn list.
        self.rebuild_visible();
        self.state = ListState::default();
        self.select_entry(self.index_of(&select));
    }
```

Add, after `restyle` (and change `restyle` to call `rebuild_visible`):

```rust
    /// Rebuild the drawn rows from the current answers. A `No` answer can hide
    /// a row in `FilteredOnly` mode, so this is `rebuild_visible`.
    pub(crate) fn restyle(&mut self) {
        self.rebuild_visible();
    }

    /// Dim or hide rows answered `No`. The same `Mode` the file view uses.
    pub(crate) fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.rebuild_visible();
    }

    /// Recompute which rows are listed, redraw them, and keep the selection on
    /// the entry it was on — or on a neighbour if that entry vanished.
    ///
    /// Directories and `..` are always listed: they have no answer. `Unknown`
    /// is always listed: you do not hide what you have not read. Only `No` is
    /// removed, and only in `FilteredOnly`.
    pub(crate) fn rebuild_visible(&mut self) {
        let keep = self.selected_entry();
        let row_before = self.state.selected();
        let mode = self.mode;
        let visible: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches!(entry.kind, Kind::Dir | Kind::Parent)
                    || mode == Mode::Dimmed
                    || entry.matched != Match::No
            })
            .map(|(index, _)| index)
            .collect();
        self.visible = visible;
        self.rebuild_list();
        // The same rule `FilterList::clamp_selection` follows: never `None`
        // while there is a row to be on, and `..` is always a row.
        let row = keep
            .and_then(|index| self.visible.iter().position(|&v| v == index))
            .or_else(|| row_before.map(|row| row.min(self.visible.len().saturating_sub(1))))
            .unwrap_or(0);
        self.state.select((!self.visible.is_empty()).then_some(row));
    }

    /// The `entries` index of the selected row.
    pub(crate) fn selected_entry(&self) -> Option<usize> {
        self.visible.get(self.state.selected()?).copied()
    }

    /// Select by `entries` index. A hidden entry cannot be selected; the
    /// selection is left where it was.
    pub(crate) fn select_entry(&mut self, index: usize) {
        if let Some(row) = self.visible.iter().position(|&v| v == index) {
            self.state.select(Some(row));
        }
    }
```

Then route everything that touched `entries` by selection through `visible`:

```rust
    fn select_next(&mut self) {
        let last = self.visible.len().saturating_sub(1);
        let next = self
            .state
            .selected()
            .map_or(0, |index| (index + 1).min(last));
        self.state.select(Some(next));
    }

    pub(crate) fn selected_path(&self) -> Option<PathBuf> {
        let index = self.selected_entry()?;
        Some(self.dir.join(&self.entries.get(index)?.name))
    }
```

In `activate_selection`, replace `self.entries.get(self.state.selected()?)?.kind` with `self.entries.get(self.selected_entry()?)?.kind`.

In `rebuild_list`, iterate the visible rows instead of every entry:

```rust
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .map(|&index| &self.entries[index])
            .map(|entry| {
```

(the closure body is unchanged.) `widest` is measured over listed rows, which is what the pane draws.

In `step_search`, walk visible rows:

```rust
    fn step_search(&mut self, reverse: bool) -> Option<Action> {
        let matcher = self.matcher.as_ref()?;
        let count = self.visible.len();
        if count == 0 {
            return None;
        }
        let start = self.state.selected().unwrap_or(0);

        let found = (1..=count)
            .map(|offset| {
                if reverse {
                    (start + count - offset) % count
                } else {
                    (start + offset) % count
                }
            })
            .find(|&row| matcher.is_match(&self.entries[self.visible[row]].matchable()))?;

        self.state.select(Some(found));
        self.preview_selection()
    }
```

`select_previous` (`state.select_previous()`) needs no change — it works on rows. `index_of` returns an `entries` index and is only used through `select_entry` now.

- [ ] **Step 4: Run all the navigator tests**

Run: `cargo test --lib filenav::`
Expected: all pass — including every pre-existing test, which is the real check that the `visible` indirection is transparent when nothing is hidden.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/widgets/filenav.rs
git commit -m "feat(filenav): hide mode removes non-matching files; selection walks visible rows

The model Document already uses for lines: a visible index over
entries, rebuilt from mode and each row's answer. Only No is removed,
and only in FilteredOnly — directories, .. and Unknown are always
listed. The selection follows its entry across a rebuild and clamps to
a neighbour when that entry vanishes, never to None (#119)."
```

---

### Task 9: `n`/`N` step between matching files when no search is active

**Files:**
- Modify: `src/widgets/filenav.rs` (`repeat_search`, `step_search`, new `step_to`, tests)

**Interfaces:**
- Consumes: `Match`, `visible` (Tasks 7–8).

- [ ] **Step 1: Write the failing tests** in `src/widgets/filenav.rs`'s `mod tests`:

```rust
    // ---- n / N over matches ----------------------------------------------

    #[test]
    fn n_steps_to_the_next_matching_file_when_no_search_is_active() {
        let mut nav = nav_over("n_match", &["a.log", "b.log", "c.log"]);
        let (a, c) = (nav.files()[0].0, nav.files()[2].0);
        nav.set_answer(a, Match::Yes(Style::default()));
        nav.set_answer(c, Match::Yes(Style::default()));
        nav.restyle();
        nav.select_entry(a);

        let action = nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('n'))));

        assert_eq!(nav.selected_entry(), Some(c));
        assert!(matches!(action, Some(Action::Preview(_))), "the step previews, like a search step");

        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('n'))));
        assert_eq!(nav.selected_entry(), Some(a), "wraps");

        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('N'))));
        assert_eq!(nav.selected_entry(), Some(c), "N reverses");
    }

    /// A filename search, once started, owns `n`/`N` — exactly as before.
    #[test]
    fn n_repeats_the_search_when_one_is_active() {
        let mut nav = nav_over("n_search", &["a.log", "b.log", "c.log"]);
        let c = nav.files()[2].0;
        nav.set_answer(c, Match::Yes(Style::default()));
        nav.search("b", false).expect("valid pattern");
        nav.select_entry(nav.files()[0].0);

        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('n'))));

        assert_eq!(nav.selected_path().unwrap().file_name().unwrap(), "b.log");
    }

    #[test]
    fn n_with_nothing_matching_and_no_search_goes_nowhere() {
        let mut nav = nav_over("n_nothing", &["a.log"]);
        let a = nav.files()[0].0;
        nav.select_entry(a);

        assert!(nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('n')))).is_none());
        assert_eq!(nav.selected_entry(), Some(a));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib filenav::tests::n_steps`
Expected: FAIL — `n` with no search does nothing today, so the selection stays on `a`.

- [ ] **Step 3: Factor the ring walk out of `step_search` and add the fallback.** Replace `repeat_search` and `step_search`:

```rust
    /// `n`/`N`: the next filename-search match if a search is active, else
    /// the next file the filters selected. The same "next interesting row"
    /// the file view gives these keys (#119).
    fn repeat_search(&mut self, opposite: bool) -> Option<Action> {
        if self.matcher.is_some() {
            return self.step_search(self.search_reverse != opposite);
        }
        self.step_to(opposite, |entry| matches!(entry.matched, Match::Yes(_)))
    }

    fn step_search(&mut self, reverse: bool) -> Option<Action> {
        let matcher = self.matcher.clone()?;
        self.step_to(reverse, |entry| matcher.is_match(&entry.matchable()))
    }

    /// Walk the visible rows from the selection, wrapping, to the first that
    /// `wanted` accepts; select it and ask for a preview.
    fn step_to(&mut self, reverse: bool, wanted: impl Fn(&Entry) -> bool) -> Option<Action> {
        let count = self.visible.len();
        if count == 0 {
            return None;
        }
        let start = self.state.selected().unwrap_or(0);

        let found = (1..=count)
            .map(|offset| {
                if reverse {
                    (start + count - offset) % count
                } else {
                    (start + offset) % count
                }
            })
            .find(|&row| wanted(&self.entries[self.visible[row]]))?;

        self.state.select(Some(found));
        self.preview_selection()
    }
```

(`Regex` is `Clone` — an `Arc` bump — which is what lets `step_search` release the borrow on `self.matcher` before calling a `&mut self` method.)

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib filenav::`
Expected: all pass.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/widgets/filenav.rs
git commit -m "feat(filenav): n/N step between matching files when no search is active

The same meaning the file view gives them — next interesting row — with
the navigator's own notion of interesting. A filename search, once
started, still owns the keys; the ring walk both share is one function
now (#119)."
```

---

### Task 10: `App` decides when to scan

**Files:**
- Modify: `src/lib.rs` (`App` fields; `App::new`; `handle_event` → `dispatch_event` split; new `refresh_scan`, `match_style`, `answer_to_match`; test seam; tests)

**Interfaces:**
- Consumes: `scan::{Scan, Scanner, Request, Scanned, Record, stamp}`, `scan::double::RecordingScanner`; `filter::{Matcher, Owner}`; `FileNav::{files, set_answer, restyle}`.
- Produces: `App::refresh_scan(&mut self, force: bool)`; `#[cfg(test)] fn record_scans(app: &mut App) -> (Rc<RecordingScanner>, Sender<Scanned>)`; private `struct ScanCache`, `struct ScanState`.

- [ ] **Step 1: Write the failing tests** in `src/lib.rs`'s `mod tests`, in a new section after the editor tests:

```rust
    // ---- navigator filter matches (#119) ----------------------------------

    use scan::double::RecordingScanner;
    use std::sync::mpsc::Sender;

    /// Swap in the recording double and a channel the test controls.
    fn record_scans(app: &mut App) -> (Rc<RecordingScanner>, Sender<scan::Scanned>) {
        let scanner = Rc::new(RecordingScanner::default());
        app.scanner = Box::new(Rc::clone(&scanner));
        let (tx, rx) = std::sync::mpsc::channel();
        app.scan_results = Some(rx);
        (scanner, tx)
    }

    fn app_over_logs(name: &str) -> App<'static> {
        app_over(name, &["a.log", "b.log"])
    }

    #[test]
    fn adding_a_filter_starts_a_scan_of_every_file() {
        let mut app = app_over_logs("scan_start");
        let (scanner, _tx) = record_scans(&mut app);

        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);

        let requests = scanner.requests();
        assert_eq!(requests.len(), 1);
        let names: Vec<_> = requests[0].files.iter().map(|(_, p, _)| p.file_name().unwrap().to_owned()).collect();
        assert_eq!(names, ["a.log", "b.log"]);
    }

    #[test]
    fn nothing_selecting_means_no_scan_and_no_answers() {
        let mut app = app_over_logs("scan_off");
        let (scanner, _tx) = record_scans(&mut app);

        app.refresh_scan(false);
        assert!(scanner.requests().is_empty(), "empty set");

        app.filters.add_excluding("noise").expect("valid pattern");
        app.refresh_scan(false);
        assert!(scanner.requests().is_empty(), "exclude only");
    }

    /// The guard: an unchanged state is free. `j` in the file view must not
    /// stat the folder.
    #[test]
    fn an_unchanged_state_issues_nothing() {
        let mut app = app_over_logs("scan_guard");
        let (scanner, _tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        assert_eq!(scanner.requests().len(), 1);

        app.refresh_scan(false);
        app.refresh_scan(false);

        assert_eq!(scanner.requests().len(), 1);
    }

    #[test]
    fn force_bypasses_the_guard() {
        let mut app = app_over_logs("scan_force");
        let (scanner, _tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);

        app.refresh_scan(true);

        assert_eq!(scanner.requests().len(), 2);
    }

    /// The point of the bitset cache: with every answer known, a toggle issues
    /// no request and touches no thread.
    #[test]
    fn a_toggle_with_every_answer_cached_issues_no_scan() {
        let mut app = app_over_logs("scan_cached");
        let (scanner, _tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.add_filter("beta").expect("valid pattern");
        app.refresh_scan(false);
        let request = &scanner.requests()[0];
        // Pretend the scan finished: every file read to EOF, one matched beta.
        for (i, (index, path, _)) in request.files.iter().enumerate() {
            let seen = if i == 0 { vec![0, 0b10] } else { vec![0] };
            app.scan_cache.records.insert(
                path.clone(),
                scan::Record { stamp: scan::stamp(path).ok(), progress: scan::Progress { seen, scanned_to: 1, eof: true } },
            );
            let _ = index;
        }

        app.filters.set_enabled(1, false);
        app.refresh_scan(false);
        app.filters.set_enabled(1, true);
        app.refresh_scan(false);
        app.filters.toggle_context(0);
        app.refresh_scan(false);

        assert_eq!(scanner.requests().len(), 1, "a toggle re-scanned");
        assert!(matches!(app.nav.entries()[app.nav.files()[0].0].matched, widgets::filenav::Match::Yes(_)));
        assert_eq!(app.nav.entries()[app.nav.files()[1].0].matched, widgets::filenav::Match::No);
    }

    #[test]
    fn a_pattern_change_drops_the_cache_and_bumps_its_id() {
        let mut app = app_over_logs("scan_key");
        let (scanner, _tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        let first = scanner.requests()[0].cache_id;

        app.add_filter("beta").expect("valid pattern");
        app.refresh_scan(false);

        assert_ne!(scanner.requests()[1].cache_id, first);
    }

    /// Peek drops every filter and puts them back; the round trip is free.
    #[test]
    fn a_peek_round_trip_issues_no_scan() {
        let mut app = app_over_logs("scan_peek");
        let (scanner, _tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        for (_, path, _) in &scanner.requests()[0].files {
            app.scan_cache.records.insert(
                path.clone(),
                scan::Record { stamp: scan::stamp(path).ok(), progress: scan::Progress { seen: vec![0], scanned_to: 1, eof: true } },
            );
        }
        app.refresh_scan(true);
        let before = scanner.requests().len();

        key(&mut app, KeyCode::Char(' '));
        assert!(app.nav.entries().iter().all(|e| e.matched == widgets::filenav::Match::Unknown), "peek must un-dim");
        key(&mut app, KeyCode::Char(' '));

        assert_eq!(scanner.requests().len(), before);
        assert_eq!(app.nav.entries()[app.nav.files()[0].0].matched, widgets::filenav::Match::No);
    }

    #[test]
    fn a_matched_file_takes_the_colour_of_the_filter_that_selected_it() {
        let mut app = app_over_logs("scan_colour");
        let (scanner, _tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.add_filter("beta").expect("valid pattern");
        app.refresh_scan(false);
        let (_, path, _) = scanner.requests()[0].files[0].clone();
        app.scan_cache.records.insert(
            path,
            scan::Record { stamp: None, progress: scan::Progress { seen: vec![0b10], scanned_to: 1, eof: true } },
        );

        app.refresh_scan(true);

        let expected = app.filters.filters()[1].style;
        assert_eq!(app.nav.entries()[app.nav.files()[0].0].matched, widgets::filenav::Match::Yes(expected));
    }
```

(`app.add_filter` exists at `src/lib.rs:479`. `app.nav.entries()` is the `#[cfg(test)]` accessor from #116. If `widgets::filenav::Match` needs a `use`, add `use widgets::filenav::Match;` to the test module instead of the paths above.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib adding_a_filter_starts_a_scan`
Expected: compile error — `scanner`, `scan_results`, `scan_cache`, `refresh_scan` not found.

- [ ] **Step 3: Add the fields and the cache types.** In `src/lib.rs`, in `pub struct App` after `help: bool`:

```rust
    /// Runs the navigator's file scans (#119). A `Box<dyn Scan>` for the same
    /// reason `launcher` is: tests swap in a recording double.
    scanner: Box<dyn scan::Scan>,
    /// Where the scanner's results arrive. Drained on the render tick.
    scan_results: Option<std::sync::mpsc::Receiver<scan::Scanned>>,
    /// Every file's bitsets, keyed on the pattern list they were read for.
    scan_cache: ScanCache,
    /// What the last `refresh_scan` saw. Unchanged means nothing to do — the
    /// guard that keeps `j` in the file view from stat-ing the folder.
    last_scan: Option<ScanState>,
    /// When `poll_stamps` last re-stat'd the listing.
    last_poll: Option<Instant>,
    /// The active file's stamp moved since it was loaded. Shown as a badge,
    /// cleared by `r`.
    view_stale: bool,
```

After `struct PeekState` (or near `StatusMessage`):

```rust
/// The scan cache: one [`scan::Record`] per file, valid for exactly one
/// pattern list in one directory.
///
/// `key` changing shifts bit positions, so every record means something
/// else; the whole cache is dropped and `id` bumped so in-flight results from
/// the old one are ignored on arrival. `dir` changing means different files.
/// A single file's record is dropped alone when its stamp moves.
#[derive(Debug, Default)]
struct ScanCache {
    id: u64,
    key: Vec<String>,
    dir: std::path::PathBuf,
    records: std::collections::HashMap<std::path::PathBuf, scan::Record>,
}

impl ScanCache {
    fn fresh(id: u64, key: Vec<String>, dir: std::path::PathBuf) -> Self {
        Self {
            id,
            key,
            dir,
            records: std::collections::HashMap::new(),
        }
    }
}

/// Everything `refresh_scan` depends on. Equal to last time ⇒ nothing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScanState {
    key: Vec<String>,
    selects: u64,
    exclude: u64,
    dir: std::path::PathBuf,
}
```

In `App::new`, alongside the editor channel:

```rust
        let (scan_tx, scan_rx) = std::sync::mpsc::channel();
```

and in the struct literal:

```rust
            scanner: Box::new(scan::Scanner::new(scan_tx)),
            scan_results: Some(scan_rx),
            scan_cache: ScanCache::default(),
            last_scan: None,
            last_poll: None,
            view_stale: false,
```

- [ ] **Step 4: Split `handle_event` so the guard runs after every early return.** Rename the existing `pub fn handle_event(&mut self, event: event::Event)` to `fn dispatch_event(&mut self, event: event::Event)` and add, directly above it:

```rust
    /// Dispatch a single event: app-wide keys first, then the focused widget.
    ///
    /// Split out from the polling loop so that it can be driven directly.
    /// Returns nothing — see `dispatch_event`, which is the body and cannot
    /// fail (#80). The one thing added around it is `refresh_scan`: the
    /// dispatch has two dozen early returns, and the scan guard has to run
    /// after every one of them.
    pub fn handle_event(&mut self, event: event::Event) {
        self.dispatch_event(event);
        self.refresh_scan(false);
    }
```

Move the old doc comment's `#80` paragraph onto `dispatch_event`.

- [ ] **Step 5: Write `refresh_scan` and its helpers.** In `impl App`, after `drain_editor_outcomes`:

```rust
    /// Decide whether the navigator's answers need work, and start it (#119).
    ///
    /// Cheap-idempotent unless `force`: it compares the pattern list, the
    /// masks and the directory to what it saw last time and returns at once
    /// if nothing moved. Runs after every event, so that guard is what keeps a
    /// keystroke in the file view from stat-ing two hundred files.
    ///
    /// When it proceeds, every file the navigator lists is answered from the
    /// cache if it can be — `Record::answer` — and put on a request if it
    /// cannot. A toggle whose every answer is cached issues no request and
    /// touches no thread; that is the whole point of caching bitsets rather
    /// than answers.
    fn refresh_scan(&mut self, force: bool) {
        let matcher = self.filters.matcher();
        let dir = self.nav.dir().to_path_buf();
        let state = matcher.as_ref().map(|m| {
            let (selects, exclude) = m.masks();
            ScanState {
                key: self.filters.pattern_key(),
                selects,
                exclude,
                dir: dir.clone(),
            }
        });
        if !force && state == self.last_scan {
            return;
        }
        self.last_scan = state;

        let Some(matcher) = matcher else {
            // Nothing selects: the feature is off, not "nothing matches".
            for (index, _) in self.nav.files() {
                self.nav.set_answer(index, Match::Unknown);
            }
            self.scanner.cancel();
            self.nav.restyle();
            return;
        };

        let key = self.filters.pattern_key();
        if self.scan_cache.key != key || self.scan_cache.dir != dir {
            self.scan_cache = ScanCache::fresh(self.scan_cache.id + 1, key, dir);
        }

        let mut pending = Vec::new();
        for (index, path) in self.nav.files() {
            let stamp = scan::stamp(&path).ok();
            if self
                .scan_cache
                .records
                .get(&path)
                .is_some_and(|record| record.stamp != stamp)
            {
                self.scan_cache.records.remove(&path);
            }
            let answer = self
                .scan_cache
                .records
                .get(&path)
                .map(|record| self.answer_to_match(record, &matcher));
            let matched = match answer {
                Some(matched @ (Match::Yes(_) | Match::No)) => matched,
                _ => {
                    let progress = self
                        .scan_cache
                        .records
                        .get(&path)
                        .map(|record| record.progress.clone())
                        .unwrap_or_default();
                    pending.push((index, path, progress));
                    Match::Unknown
                }
            };
            self.nav.set_answer(index, matched);
        }

        if pending.is_empty() {
            self.scanner.cancel();
        } else {
            self.scanner.start(scan::Request {
                cache_id: self.scan_cache.id,
                matcher,
                files: pending,
            });
        }
        self.nav.restyle();
    }

    /// A record's answer as the navigator's `Match`, with the owning filter's
    /// colour on a yes.
    fn answer_to_match(&self, record: &scan::Record, matcher: &filter::Matcher) -> Match {
        match record.answer(matcher) {
            Some(true) => Match::Yes(self.match_style(record.owner(matcher))),
            Some(false) => Match::No,
            None => Match::Unknown,
        }
    }

    /// The style the view would draw a line selected by `owner` with. The
    /// navigator draws the file's name in it, so the two panes agree at a
    /// glance and the colour says *which* filter picked the file.
    fn match_style(&self, owner: Option<filter::Owner>) -> Style {
        match owner {
            Some(filter::Owner::Search) => filter::SEARCH_STYLE,
            Some(filter::Owner::Filter(index)) => self
                .filters
                .style_for(filter::Verdict::Included(index))
                .unwrap_or_default(),
            None => Style::default(),
        }
    }
```

Add `use widgets::filenav::Match;` near the other `use widgets::...` lines.

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib scan_ && cargo test --lib`
Expected: the eight new tests pass; the rest of the suite still passes (every existing test now runs `refresh_scan` after each event, with no filters — the `None` branch, which is cheap).

- [ ] **Step 7: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/lib.rs
git commit -m "feat(app): decide when the navigator needs a scan, and answer from the cache first

refresh_scan runs after every event behind a guard on (pattern list,
masks, directory), so a keystroke in the file view costs one
comparison. When something moved, every listed file is answered from
its cached bitsets if it can be and put on one request if it cannot; a
toggle with every answer known issues nothing. handle_event is split
so the guard runs after all two dozen early returns (#119)."
```

---

### Task 11: Draining results on the tick

**Files:**
- Modify: `src/lib.rs` (`handle_events`; new `drain_scan_results`; tests)

**Interfaces:**
- Consumes: `scan_results`, `scan_cache`, `answer_to_match` (Task 10); `FileNav::{path_at, set_answer, restyle}`.
- Produces: `App::drain_scan_results(&mut self) -> bool`.

- [ ] **Step 1: Write the failing tests** in the `#119` test section:

```rust
    fn scanned(app: &App, row: usize, seen: Vec<u64>, eof: bool) -> scan::Scanned {
        let (index, path) = app.nav.files()[row].clone();
        scan::Scanned {
            cache_id: app.scan_cache.id,
            index,
            stamp: scan::stamp(&path).ok(),
            path,
            progress: scan::Progress { seen, scanned_to: 1, eof },
        }
    }

    #[test]
    fn a_result_answers_its_row_and_asks_for_a_redraw() {
        let mut app = app_over_logs("drain_basic");
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);

        tx.send(scanned(&app, 0, vec![0b1], false)).expect("send");
        tx.send(scanned(&app, 1, vec![0], true)).expect("send");

        assert!(app.drain_scan_results(), "answers arrived, the frame is stale");
        assert!(matches!(app.nav.entries()[app.nav.files()[0].0].matched, Match::Yes(_)));
        assert_eq!(app.nav.entries()[app.nav.files()[1].0].matched, Match::No);
        assert!(!app.drain_scan_results(), "nothing new");
    }

    #[test]
    fn a_result_from_a_replaced_cache_is_dropped() {
        let mut app = app_over_logs("drain_stale");
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        let mut stale = scanned(&app, 0, vec![0b1], false);
        stale.cache_id += 1;

        tx.send(stale).expect("send");

        assert!(!app.drain_scan_results());
        assert_eq!(app.nav.entries()[app.nav.files()[0].0].matched, Match::Unknown);
    }

    /// A result that reaches further than the record held replaces it; one
    /// that does not — a cancelled worker's partial — is ignored.
    #[test]
    fn a_result_is_kept_only_if_it_read_further() {
        let mut app = app_over_logs("drain_further");
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        let mut far = scanned(&app, 0, vec![0], true);
        far.progress.scanned_to = 50;
        let mut near = scanned(&app, 0, vec![0b1], false);
        near.progress.scanned_to = 10;

        tx.send(far).expect("send");
        tx.send(near).expect("send");
        app.drain_scan_results();

        let (_, path) = &app.nav.files()[0];
        assert_eq!(app.scan_cache.records[path].progress.scanned_to, 50);
        assert_eq!(app.nav.entries()[app.nav.files()[0].0].matched, Match::No);
    }

    /// The row a result names may no longer be the file it was for.
    #[test]
    fn a_result_for_a_row_that_now_holds_another_file_updates_the_cache_only() {
        let mut app = app_over_logs("drain_moved");
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        let mut moved = scanned(&app, 0, vec![0b1], false);
        moved.index = app.nav.files()[1].0;

        tx.send(moved.clone()).expect("send");
        app.drain_scan_results();

        assert!(app.scan_cache.records.contains_key(&moved.path));
        assert_eq!(app.nav.entries()[moved.index].matched, Match::Unknown, "applied to the wrong row");
    }

    #[test]
    fn a_disconnected_scanner_is_survived() {
        let mut app = app_over_logs("drain_gone");
        let (_scanner, tx) = record_scans(&mut app);
        drop(tx);

        assert!(!app.drain_scan_results());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib drain_`
Expected: compile error — `drain_scan_results` not found.

- [ ] **Step 3: Write the drain and wire it into the tick.** In `impl App`, after `refresh_scan`:

```rust
    /// Move scan results into the cache and the navigator, reporting whether
    /// anything on screen changed.
    ///
    /// A result is dropped if its cache id is stale — the pattern list changed
    /// while it was in flight, so its bitsets mean something else. Otherwise
    /// it replaces the held record only if it read further; a cancelled
    /// worker's partial can arrive after the fresh worker's complete. The row
    /// it names is checked against the path it is for before the navigator is
    /// told anything: the listing may have changed under it.
    fn drain_scan_results(&mut self) -> bool {
        let Some(results) = self.scan_results.as_ref() else {
            return false;
        };
        let matcher = self.filters.matcher();
        let mut changed = false;
        loop {
            let scanned = match results.try_recv() {
                Ok(scanned) => scanned,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::warn!("the scan worker is gone; answers stay unknown until the next change");
                    break;
                }
            };
            if scanned.cache_id != self.scan_cache.id {
                continue;
            }
            let further = self
                .scan_cache
                .records
                .get(&scanned.path)
                .is_none_or(|held| {
                    scanned.progress.scanned_to > held.progress.scanned_to
                        || (scanned.progress.eof && !held.progress.eof)
                });
            if !further {
                continue;
            }
            let record = scan::Record {
                stamp: scanned.stamp,
                progress: scanned.progress,
            };
            let matched = matcher
                .as_ref()
                .map_or(Match::Unknown, |m| self.answer_to_match(&record, m));
            self.scan_cache.records.insert(scanned.path.clone(), record);
            if self.nav.path_at(scanned.index).as_ref() == Some(&scanned.path) {
                changed |= self.nav.set_answer(scanned.index, matched);
            }
        }
        if changed {
            self.nav.restyle();
        }
        changed
    }
```

`Option::is_none_or` is stable since Rust 1.82; if the toolchain rejects it, use `.map_or(true, |held| ..)`.

Then in `handle_events`, replace the first line:

```rust
        let drained = self.drain_editor_outcomes() | self.drain_scan_results();
```

(`|`, not `||` — both drains must run every tick.)

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib drain_ && cargo test --lib`
Expected: all pass.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/lib.rs
git commit -m "feat(app): drain scan results on the tick and redraw when one lands

Beside drain_editor_outcomes, ORed into the same did-anything-change
the conditional redraw (#85) branches on. Stale cache ids are dropped,
a result replaces its record only if it read further, and the row it
names is checked against its path before the navigator is told (#119)."
```

---

### Task 12: One hide mode for both panes

**Files:**
- Modify: `src/lib.rs` (new `set_mode` helper; `toggle_hiding`, `toggle_peek`, `sync_document`; tests)

**Interfaces:**
- Consumes: `FileNav::set_mode` (Task 8).
- Produces: `App::set_mode(&mut self, mode: Mode)`.

- [ ] **Step 1: Write the failing test** in the `#119` section:

```rust
    #[test]
    fn hide_mode_hides_non_matching_files_in_the_navigator_too() {
        let mut app = app_over_logs("hide_both");
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        tx.send(scanned(&app, 0, vec![0], true)).expect("send");
        tx.send(scanned(&app, 1, vec![0b1], false)).expect("send");
        app.drain_scan_results();

        ctrl(&mut app, KeyCode::Char('h'));
        assert_eq!(app.nav.files().len(), 1, "a.log should be hidden");

        ctrl(&mut app, KeyCode::Char('h'));
        assert_eq!(app.nav.files().len(), 2);
    }

    #[test]
    fn peek_shows_every_file_and_restoring_hides_them_again() {
        let mut app = app_over_logs("hide_peek");
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        tx.send(scanned(&app, 0, vec![0], true)).expect("send");
        app.drain_scan_results();
        ctrl(&mut app, KeyCode::Char('h'));
        assert_eq!(app.nav.files().len(), 1);

        key(&mut app, KeyCode::Char(' '));
        assert_eq!(app.nav.files().len(), 2, "peek must show the plain listing");

        key(&mut app, KeyCode::Char(' '));
        assert_eq!(app.nav.files().len(), 1);
    }
```

(`FileNav::files()` walks `entries`, not `visible` — so for these tests to see hiding, change `files()` in Task 7 to walk `visible` **or** assert on `names(&app.nav)`-style rendered output instead. Prefer the second: add a `nav_names(app: &mut App) -> Vec<String>` helper in the test module that renders the navigator and reads its rows, and assert `nav_names(&mut app).contains("a.log")`. `files()` must stay over `entries` — a hidden file still needs scanning.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib hide_mode_hides_non_matching_files_in_the_navigator_too`
Expected: FAIL — `a.log` is still listed after `Ctrl-H`.

- [ ] **Step 3: Add the helper and route every mode write through it.** In `impl App`:

```rust
    /// The one place the mode is set. `Ctrl-H`/`H` is one key with one meaning
    /// in both panes: non-matching *lines* dim or hide in the view, and
    /// non-matching *files* dim or hide in the navigator (#119).
    fn set_mode(&mut self, mode: Mode) {
        self.document.set_mode(mode);
        self.nav.set_mode(mode);
    }
```

Then replace each `self.document.set_mode(..)` in `toggle_hiding`, `toggle_peek` (both calls) and `sync_document` with `self.set_mode(..)`. In `sync_document` the `Document` is being rebuilt, so the call order is: build the document, then `self.set_mode(mode)`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib hide_ && cargo test --lib`
Expected: all pass.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/lib.rs
git commit -m "feat(app): Ctrl-H dims or hides non-matching files as well as lines

One helper sets the mode on both the document and the navigator, and
every write goes through it — the toggle, peek in both directions, and
the rebuild on load. One key, one meaning, both panes (#119)."
```

---

### Task 13: Re-stat on the tick, and a badge when the active file moved

**Files:**
- Modify: `src/lib.rs` (`STALE_BADGE_TEXT`; `handle_events`; new `poll_stamps` + `check_stamps`; the badge painting in `render`; tests)

**Interfaces:**
- Consumes: `scan::stamp`, `scan_cache`, `scanner` (Task 10); `FileView::filename` (#116).
- Produces: `App::poll_stamps(&mut self) -> bool`; `App::check_stamps(&mut self) -> bool`; `const STALE_BADGE_TEXT`.

- [ ] **Step 1: Write the failing tests** in the `#119` section:

```rust
    #[test]
    fn a_file_that_changed_on_disk_is_rescanned() {
        let mut app = app_over_logs("poll_changed");
        let (scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        tx.send(scanned(&app, 0, vec![0], true)).expect("send");
        app.drain_scan_results();
        assert_eq!(app.nav.entries()[app.nav.files()[0].0].matched, Match::No);
        let (_, path) = app.nav.files()[0].clone();
        fs::write(&path, "now alpha is here\nand more\n").expect("rewrite");

        assert!(app.check_stamps());

        assert_eq!(app.nav.entries()[app.nav.files()[0].0].matched, Match::Unknown);
        let last = scanner.requests().last().expect("a rescan").clone();
        assert_eq!(last.files.len(), 1);
        assert_eq!(last.files[0].1, path);
    }

    #[test]
    fn an_unchanged_listing_is_free() {
        let mut app = app_over_logs("poll_same");
        let (scanner, tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        tx.send(scanned(&app, 0, vec![0], true)).expect("send");
        tx.send(scanned(&app, 1, vec![0], true)).expect("send");
        app.drain_scan_results();
        let before = scanner.requests().len();

        assert!(!app.check_stamps());
        assert_eq!(scanner.requests().len(), before);
    }

    #[test]
    fn the_active_file_changing_raises_the_badge() {
        let mut app = app_over_logs("poll_badge");
        let (_scanner, tx) = record_scans(&mut app);
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Enter);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        tx.send(scanned(&app, 0, vec![0], true)).expect("send");
        app.drain_scan_results();
        assert!(!status_line(&mut app).contains("changed on disk"));
        fs::write(app.view.filename(), "rewritten\n").expect("rewrite");

        app.check_stamps();

        assert!(status_line(&mut app).contains("changed on disk"), "{}", status_line(&mut app));
    }

    #[test]
    fn polling_is_rate_limited() {
        let mut app = app_over_logs("poll_rate");
        let _seam = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);

        app.poll_stamps();
        let first = app.last_poll.expect("stamped");
        app.poll_stamps();

        assert_eq!(app.last_poll, Some(first), "polled again inside the interval");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib poll_`
Expected: compile error — `check_stamps`, `poll_stamps` not found.

- [ ] **Step 3: Add the constant, the poll, and the badge.** After `HIDE_BADGE_STYLE` at the top of `src/lib.rs`:

```rust
/// The badge saying the file on screen is not the file on disk (#119).
///
/// Raised by `poll_stamps` when the *active* file's stamp moves, cleared by
/// `r`. The navigator's answer for that file updates on its own; the view does
/// not reload on its own. That is a real inconsistency between the panes, and
/// the badge exists so it is never a silent one: one key resolves it.
const STALE_BADGE_TEXT: &str = " changed on disk · r ";

/// How often `poll_stamps` re-stats the listing while the feature is on.
const POLL_INTERVAL: Duration = Duration::from_secs(2);
```

In `impl App`, after `drain_scan_results`:

```rust
    /// Re-stat the listing every `POLL_INTERVAL` while the feature is on.
    /// Returns whether anything changed.
    ///
    /// On the tick the render loop already wakes on, not a thread and not a
    /// file-watching API: a few hundred `stat` calls every two seconds is
    /// nothing, and it covers every listed file rather than only the active
    /// one.
    fn poll_stamps(&mut self) -> bool {
        if self.filters.matcher().is_none() {
            return false;
        }
        let now = Instant::now();
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < POLL_INTERVAL)
        {
            return false;
        }
        self.last_poll = Some(now);
        self.check_stamps()
    }

    /// Compare every recorded file's stamp to the disk; drop, forget and
    /// rescan the ones that moved. The active file moving also raises the
    /// badge. Split from `poll_stamps` so tests can call it without waiting.
    fn check_stamps(&mut self) -> bool {
        let Some(matcher) = self.filters.matcher() else {
            return false;
        };
        let active = self.view.filename().to_path_buf();
        let mut pending = Vec::new();
        for (index, path) in self.nav.files() {
            let Some(held) = self.scan_cache.records.get(&path) else {
                continue;
            };
            if held.stamp == scan::stamp(&path).ok() {
                continue;
            }
            self.scan_cache.records.remove(&path);
            self.nav.set_answer(index, Match::Unknown);
            if path == active {
                self.view_stale = true;
            }
            pending.push((index, path, scan::Progress::default()));
        }
        if pending.is_empty() {
            return false;
        }
        self.scanner.start(scan::Request {
            cache_id: self.scan_cache.id,
            matcher,
            files: pending,
        });
        self.nav.restyle();
        true
    }
```

In `handle_events`:

```rust
        let drained =
            self.drain_editor_outcomes() | self.drain_scan_results() | self.poll_stamps();
```

Then the badge. In `render`, replace the single-badge computation and painting. Where it reads

```rust
        let badge = (self.document.mode() == Mode::FilteredOnly).then_some(HIDE_BADGE_TEXT);
        let badge_width = badge.map_or(0, |text| text.chars().count() + 1);
```

write

```rust
        let badges: Vec<&str> = [
            (self.document.mode() == Mode::FilteredOnly).then_some(HIDE_BADGE_TEXT),
            self.view_stale.then_some(STALE_BADGE_TEXT),
        ]
        .into_iter()
        .flatten()
        .collect();
        let badge_width: usize = badges.iter().map(|text| text.chars().count() + 1).sum();
```

and where it paints (`if let Some(badge) = badge { buf.set_stringn(..) }`) write

```rust
        let mut x = prompt_area.x;
        for badge in &badges {
            buf.set_stringn(x, prompt_area.y, badge, prompt_area.width as usize, HIDE_BADGE_STYLE);
            x += u16::try_from(badge.chars().count() + 1).unwrap_or(u16::MAX);
        }
```

The existing `prompt_area.x + badge_width as u16` for the status text keeps working since `badge_width` is now the sum.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib poll_ && cargo test --lib badge && cargo test --lib`
Expected: all pass, including the pre-existing `the_badge_*` tests.

- [ ] **Step 5: CI steps, then commit**

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/lib.rs
git commit -m "feat(app): re-stat the listing every two seconds, and say when the open file moved

On the tick the loop already wakes on — no thread, no watcher API. A
file whose stamp moved is forgotten and rescanned; if it is the active
file, a badge beside HIDE says the view is not the disk, and r is the
key that fixes it. The navigator updating while the view does not is a
real inconsistency; this is what keeps it from being a silent one (#119)."
```

---

### Task 14: `r` — refresh from disk

**Files:**
- Modify: `src/lib.rs` (the global key match in `dispatch_event`; new `reload_active_file`; tests)

**Interfaces:**
- Consumes: `refresh_scan(true)` (Task 10); `cursor_source`, `place_cursor_on_visible_row` (`src/viewport.rs`); `Document::{nearest_visible, visible_position}`; `FileView::{filename, load}`.
- Produces: `App::reload_active_file(&mut self)`.

- [ ] **Step 1: Write the failing tests** in the `#119` section:

```rust
    #[test]
    fn r_reloads_the_file_and_keeps_the_cursor_on_its_line() {
        let mut app = app_over_file("r_reload", "one\ntwo\nthree\nfour\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        assert_eq!(cursor_source(&app), 2);
        fs::write(app.view.filename(), "one\ntwo\nthree\nfour\nfive\n").expect("rewrite");

        key(&mut app, KeyCode::Char('r'));

        assert_eq!(app.document.lines().len(), 5, "not reloaded");
        assert_eq!(cursor_source(&app), 2, "the reader lost their place");
    }

    #[test]
    fn r_clears_the_badge_and_forces_a_rescan() {
        let mut app = app_over_logs("r_rescan");
        let (scanner, _tx) = record_scans(&mut app);
        app.add_filter("alpha").expect("valid pattern");
        app.refresh_scan(false);
        let before = scanner.requests().len();
        app.view_stale = true;

        key(&mut app, KeyCode::Char('r'));

        assert!(!app.view_stale);
        assert_eq!(scanner.requests().len(), before + 1);
    }

    #[test]
    fn r_on_a_truncated_file_clamps_rather_than_losing_the_cursor() {
        let mut app = app_over_file("r_shrunk", "one\ntwo\nthree\nfour\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('G'));
        fs::write(app.view.filename(), "one\n").expect("truncate");

        key(&mut app, KeyCode::Char('r'));

        assert_eq!(app.document.lines().len(), 1);
        assert_eq!(cursor_source(&app), 0);
    }
```

(`app_over_file` and `cursor_source(&App)` are existing test helpers; `fs` is imported in the test module.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib r_reloads`
Expected: FAIL — `r` does nothing today, so the document still has 4 lines.

- [ ] **Step 3: Bind the key and write the reload.** In `dispatch_event`'s global key match — the block that handles `q`, `Tab`, `e`, `t`, `f` and so on, with empty modifiers — add an arm:

```rust
                // Refresh from disk: rescan the listing and reload the file,
                // keeping the reader's place. The one key that resolves the
                // navigator and the view disagreeing about a file that
                // changed underneath them (#119).
                KeyCode::Char('r') => {
                    self.refresh_scan(true);
                    self.reload_active_file();
                    return;
                }
```

In `impl App`, after `promote_file_view`:

```rust
    /// Re-read the active file, and put the cursor back on the line it was on.
    ///
    /// `load` rebuilds the buffer from the top, so this remembers the cursor's
    /// *source* line first and re-places it afterwards — the machinery a
    /// filter change already uses to rebuild without losing the reader's
    /// place. A file that shrank underneath the cursor (logrotate) simply
    /// clamps to what is left.
    fn reload_active_file(&mut self) {
        let path = self.view.filename().to_path_buf();
        if path.as_os_str().is_empty() {
            return;
        }
        let source = self.cursor_source();
        self.view.load(&path);
        self.sync_document();
        self.document.evaluate(&self.filters);
        let row = self
            .document
            .nearest_visible(source)
            .and_then(|nearest| self.document.visible_position(nearest))
            .unwrap_or(0);
        self.place_cursor_on_visible_row(row);
        self.view_stale = false;
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib r_ && cargo test --lib`
Expected: all pass. If `every_bound_key_is_documented` in `src/help.rs` now fails naming `'r'` — it should — that is Task 15's job; do Task 15 before committing, or commit with `cargo test --lib -- --skip every_bound_key_is_documented` and note it. **Prefer doing Task 15 first and committing both together.**

- [ ] **Step 5: CI steps, then commit** (after Task 15 if the keymap test fired)

```bash
cargo fmt -p recon && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/lib.rs
git commit -m "feat(app): r refreshes from disk — rescan the listing, reload the file, keep the cursor

Two things one key: refresh_scan(true) bypasses the guard and re-stats
everything, and the active file is re-read with the cursor put back on
its source line through the same path a filter change uses. A file that
shrank clamps. Clears the changed-on-disk badge (#119)."
```

---

### Task 15: Keymap, README, spec corrections

**Files:**
- Modify: `src/help.rs` (`KEYMAP`)
- Modify: `README.md` (Global, Navigator, Filter pane tables; a paragraph on senses)
- Modify: `docs/specs/2026-09-02-navigator-filter-matches-design.md` (two corrections found while planning)
- The spec file is committed here — the project's convention is that a spec lands with its feature, never alone.

**Interfaces:** none.

- [ ] **Step 1: Run the drift test to see what it wants**

Run: `cargo test --lib every_bound_key_is_documented`
Expected: FAIL naming `'r' bound in src/lib.rs` and `'m' bound in src/widgets/filterlist.rs`.

- [ ] **Step 2: Add the `KEYMAP` rows** in `src/help.rs`. In the `Global` section after the `O` binding:

```rust
            Binding {
                keys: &["r"],
                action: "Refresh from disk — rescan the listing, reload the file",
            },
```

In `Navigator`, replace the `n`/`N` row:

```rust
            Binding {
                keys: &["n", "N"],
                action: "Next / previous search match, or matching file",
            },
```

In `Filter pane`, after the `c` binding:

```rust
            Binding {
                keys: &["m"],
                action: "Toggle the selected filter between include and context",
            },
```

- [ ] **Step 3: Run the drift test**

Run: `cargo test --lib help::`
Expected: all pass.

- [ ] **Step 4: Update the README.** In the Global table, after the `O` row:

```markdown
| `r` | **Refresh from disk** — re-stat and rescan the navigator's listing, and reload the file in the view with the cursor kept on its line. The status row shows `changed on disk · r` when the open file's size or mtime has moved |
```

In the Navigator table, replace the `n` / `N` row:

```markdown
| `n` / `N` | Repeat the last filename search, forward / reversed — or, with no search active, move to the next / previous file the filters match |
```

In the Filter pane table, after the `c` row:

```markdown
| `m` | Toggle the selected filter between *include* and *context* — see below |
```

After the paragraph beginning "`c` edits in place", add:

```markdown
### Three senses, and which files the navigator marks

Every numbered filter has a sense, shown in its row as `inc`, `ctx` or `exc`:

| Sense | In the view | Marks the file in the navigator? |
| --- | --- | --- |
| `inc` — include | shows the line, in the filter's colour | **yes** |
| `ctx` — context | shows the line, in the filter's colour | no |
| `exc` — exclude | removes the line | no |

With at least one include filter (or a live search) enabled, the navigator marks
each file: a name drawn in a filter's colour has at least one line that filter
selected; a dimmed name has none; a plain name has not been scanned yet. In hide
mode (`Ctrl-H`), non-matching files leave the listing the way non-matching lines
leave the view. The scan runs in the background and stops each file at its first
matching line, and toggling a filter on or off usually re-answers the whole
folder without reading anything.

*Context* is for the patterns every log carries — the build's commit, the host —
that you want to see in the view but that say nothing about which logs are
interesting. Which sense a pattern has is your call, per set: `^host:
production-.*` is an include when the question is "which production logs have
errors" and context when it is "which logs have bug 57, and where did they run".
New filters are include; `m` flips the selected one.
```

- [ ] **Step 5: Correct the spec.** In `docs/specs/2026-09-02-navigator-filter-matches-design.md`:

In the *Three senses* table, the `Pane glyph` column reads `i` / `m` / `x`; make it `inc` / `ctx` / `exc`, which is how the pane already shows `inc` and `exc`.

In *Testing*, the invariant paragraph is wrong in two places and both were found in execution (see the ledger's Task 3 ruling). Replace the whole paragraph beginning "**The invariant that makes the whole thing correct:**" with:

```markdown
**The invariant that makes the whole thing correct:** for a grid of lines × filter sets
mixing all three senses, `matcher.selects(matcher.bits(line))` equals the spec's own
definition stated directly — an enabled `Include` filter or the search hits the line, and
no enabled `Exclude` does. Deliberately *not* derived from `verdict`'s index: that is a
colouring rule (first match wins), and a context filter can win the colour of a line an
include filter also hit. Selecting and colouring are different questions. The one relation
to `verdict` that does hold, and is asserted: a selected line is always a shown line.
```

In *Data flow › drain* and *Error handling*, the claim "`Disconnected` on the receiver means the worker panicked" is wrong: `Scanner` holds a `Sender` for its whole life, so a panicked worker only stops sending — the receiver sees `Empty`, not `Disconnected`, and the affected entries stay `Unknown` until the next trigger (which is the designed recovery). `Disconnected` can only mean the `Scanner` itself was dropped. Reword both places to say that.

Then, still in *Testing*, the sentence "And `matcher.owner(bits)` names the same filter `verdict` would colour the line with." is wrong. Replace with:

```markdown
And `matcher.owner(bits)` names the lowest *selecting* filter with a hit — which is not
always the filter `verdict` colours the line with. A line hit by context filter 1 and
include filter 2 is drawn in filter 1's colour (first wins) but the *file* is owned by
filter 2, the one that selected it. The test says so explicitly.
```

- [ ] **Step 6: Full CI, then commit everything from Task 14 (if held) and this task**

```bash
cargo fmt -p recon --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release
git add src/help.rs README.md docs/specs/2026-09-02-navigator-filter-matches-design.md docs/plans/2026-09-02-navigator-filter-matches.md src/lib.rs
git commit -m "docs: keymap rows for r and m, the three senses, and the design

KEYMAP gains r and m and the navigator's n/N row says what it does with
no search active; every_bound_key_is_documented enforces the first two.
The README's tables follow by hand, plus a section on the three senses
and what the navigator marks. The spec and plan land with the feature,
as every spec in docs/specs/ has (#119)."
```

- [ ] **Step 7: Push and open the PR**

```bash
git push -u origin Fix-I119-navigator-filter-matches
gh pr create --title "Mark which files in the navigator match the active filters" --body "Fixes #119

Design: docs/specs/2026-09-02-navigator-filter-matches-design.md
Plan:   docs/plans/2026-09-02-navigator-filter-matches.md

See the spec's *What it does* for the behaviour and *Three senses* for the new Sense::Context. Tests: every scan behaviour over a Cursor in src/scan.rs, the selects/verdict invariant in src/filter.rs, hide mode and selection in src/widgets/filenav.rs, App wiring with a recording Scanner double in src/lib.rs, and one real-thread integration test in tests/scan_thread.rs."
```

---

## Self-review

**Spec coverage.** Every section of the spec maps to a task: *Three senses* → 1–3; *Components: scan.rs* → 4–6; *Matcher* → 3; *filenav display* → 7–9; *filterlist* → 2; *App* → 10–14; *Data model* → 4–5, 10; *The cache* → 5, 10–11; *refresh_scan* → 10; *drain* → 11; *poll_stamps* → 13; *Refresh from disk* → 13–14; *Peek* → 10 (test), 12; *Navigator rendering, hide mode, selection* → 7–8; *Keys* → 2, 9, 14, 15; *Error handling* → 4 (bad bytes, read error), 6 (unreadable file, disconnected sender), 3 (>64), 11 (disconnected receiver), 14 (deleted/shrunk active file); *Testing* → each task; *Out of scope* — untouched by design. The spec's "Listing a different directory ⇒ cache dropped" is implemented in Task 10 via `scan_cache.dir`.

**Placeholder scan.** No TBD/TODO. Every code step shows code. Task 12's note about `files()` vs `visible` is a resolved instruction (use a rendered-rows helper), not a placeholder.

**Type consistency.** `Match::{Unknown, No, Yes(Style)}` in 7–13; `set_answer(index, Match) -> bool` and `restyle()` in 7, 8, 10, 11, 13; `files() -> Vec<(usize, PathBuf)>` and `path_at` in 7, 10, 11, 13; `Matcher::{bits, selects, owner, masks}` in 3–5, 10; `Record::{answer, owner}` in 5, 10, 11; `Request { cache_id, matcher, files }` and `Scanned { cache_id, index, path, stamp, progress }` in 6, 10, 11, 13; `Scan::{start, cancel}` in 6, 10, 13; `ScanCache::{id, key, dir, records}` in 10, 11, 13; `refresh_scan(force)` in 10, 14; `set_mode` on both `FileNav` (8) and `App` (12). `Owner::rank` defined in 3, used in 5.

One known compile-order note: Task 1 leaves temporary arms in `filterlist.rs` that Task 2 replaces; Task 14's `r` binding trips the keymap drift test until Task 15 adds the row — the plan says to do 15 before committing 14.
