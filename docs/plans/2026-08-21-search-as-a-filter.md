# Search as a Filter — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `recon`'s search a filter, so that `n`/`N` step between lines matched by *either* an enabled filter or the live search, and `Ctrl-H` hides lines matched by neither.

**Architecture:** The search stops being widget state and becomes a dedicated slot on `FilterSet`, reported by a new `Verdict::Searched`. `/` sets that slot, `Esc` clears it, `p` promotes it into the numbered set. Because `FilterSet` already sees every line of the file, this needs no new evaluation machinery — hiding, exclusion, `!` and file-survival all come free. `n`/`N` move up from `FileView` to `App`, which is the only place that can see both the verdicts and the cursor.

**Tech Stack:** Rust (edition 2024, let-chains in use), `ratatui`, `crossterm`, `regex` 1.x, and a vendored fork of `tui-textarea-2` at `vendor/tui-textarea-2`.

**Spec:** `docs/specs/2026-08-21-search-as-a-filter-design.md`

## Global Constraints

- **`recompute_visible` must never run a regex.** It is what makes `Ctrl-H` O(lines) and instant on a large log. Any predicate it needs is cached on the `Document` at `evaluate` time.
- **`Verdict::Included(usize)` is a position in `FilterSet::filters`.** The live search must never occupy a slot in that vector, or `/` and `Esc` would renumber the user's filters.
- **Rust's `regex` crate rejects lookaround.** Every pattern in this codebase is a plain linear-time regex. Do not reach for `fancy-regex`.
- **Every fork change is recorded in `vendor/tui-textarea-2/PATCH.md`.** A patch that is not written down there is a patch that gets lost at the next upstream bump.
- **Fixture directory names must be unique per process.** `claim_fixture_dir` panics on reuse. Pick a fresh name for every new test that builds one.
- Run the full suite with `cargo test`. Lint with `cargo clippy --all-targets -- -D warnings` and format with `cargo fmt` before each commit.

---

### Task 1: `Verdict::Searched` and the search slot

**Files:**
- Modify: `src/filter.rs` (the `Verdict` enum, `FilterSet`, `verdict`, `style_for`, `disable_all_remembering`, `restore_remembered`, `any_enabled`)
- Modify: `src/document.rs:62` (`match_count`) and `src/document.rs:80` (`recompute_visible`) — exhaustiveness
- Test: `src/filter.rs` `mod tests`, `src/document.rs` `mod tests`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `Verdict::Searched` — a new variant
  - `pub(crate) const SEARCH_STYLE: Style`
  - `FilterSet::set_search(&mut self, pattern: &str) -> Result<(), regex::Error>`
  - `FilterSet::clear_search(&mut self)`
  - `FilterSet::search(&self) -> Option<&Filter>`
  - `FilterSet::row_count(&self) -> usize`

- [ ] **Step 1: Write the failing tests**

Add to `src/filter.rs` `mod tests`:

```rust
#[test]
fn a_search_matches_like_an_including_filter() {
    let mut set = FilterSet::new();
    set.set_search("timeout").expect("valid pattern");

    assert_eq!(set.verdict("conn timeout"), Verdict::Searched);
    assert_eq!(set.verdict("all fine"), Verdict::Unmatched);
}

/// The user's attention is on the pattern they just typed, so it wins the
/// colour on a line a numbered filter also matches.
#[test]
fn the_search_outranks_a_numbered_filter() {
    let mut set = set_with(&["ERROR"]);
    set.set_search("timeout").expect("valid pattern");

    assert_eq!(set.verdict("ERROR timeout on socket"), Verdict::Searched);
    assert_eq!(set.verdict("ERROR disk full"), Verdict::Included(0));
}

/// Exclusion runs first and beats everything, so search inherits the rule
/// rather than needing one of its own.
#[test]
fn exclusion_beats_the_search() {
    let mut set = FilterSet::new();
    set.add_excluding("heartbeat").expect("valid pattern");
    set.set_search("timeout").expect("valid pattern");

    assert_eq!(set.verdict("heartbeat timeout"), Verdict::Excluded);
}

/// One search at a time, like vim's search register: a second `/` replaces
/// the first rather than stacking another filter.
#[test]
fn setting_a_search_replaces_the_previous_one() {
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");
    set.set_search("bar").expect("valid pattern");

    assert_eq!(set.verdict("bar line"), Verdict::Searched);
    assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
}

/// The whole point of the separate slot: `/` and `Esc` must never renumber
/// the filters the user built, because `Verdict::Included` is a position.
#[test]
fn the_search_does_not_occupy_a_numbered_slot() {
    let mut set = set_with(&["alpha", "beta"]);
    set.set_search("gamma").expect("valid pattern");

    assert_eq!(set.len(), 2, "the search must not join the numbered set");
    assert_eq!(set.verdict("beta line"), Verdict::Included(1));

    set.clear_search();
    assert_eq!(set.verdict("beta line"), Verdict::Included(1));
}

#[test]
fn clearing_a_search_removes_it() {
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");
    set.clear_search();

    assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
    assert!(set.search().is_none());
}

#[test]
fn an_invalid_search_pattern_is_reported_and_changes_nothing() {
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");

    assert!(set.set_search("[").is_err());
    assert_eq!(set.verdict("foo line"), Verdict::Searched, "the old search was lost");
}

#[test]
fn a_disabled_search_matches_nothing() {
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");
    set.set_all_enabled(false);

    assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
}

/// The search carries a colour of its own, outside `PALETTE`, so it never
/// shifts as filters are added and removed.
#[test]
fn the_search_style_is_reserved_rather_than_drawn_from_the_palette() {
    assert!(
        !PALETTE.iter().any(|colour| SEARCH_STYLE.fg == Some(*colour)),
        "the search colour would move as the palette rotates"
    );
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");
    assert_eq!(set.style_for(Verdict::Searched), Some(SEARCH_STYLE));
}

/// `!` must round-trip the search slot too, or it stops meaning "back to an
/// unfiltered view".
#[test]
fn disabling_all_remembers_the_search_slot() {
    let mut set = set_with(&["foo"]);
    set.set_search("bar").expect("valid pattern");

    set.disable_all_remembering();
    assert!(!set.any_enabled(), "the search kept the set enabled");
    assert_eq!(set.verdict("bar line"), Verdict::Unmatched);

    set.restore_remembered();
    assert_eq!(set.verdict("bar line"), Verdict::Searched);
}

/// A search that the user had deliberately toggled off must not come back on.
#[test]
fn restoring_does_not_switch_a_disabled_search_back_on() {
    let mut set = FilterSet::new();
    set.set_search("bar").expect("valid pattern");
    set.search_set_enabled(false);

    set.disable_all_remembering();
    set.restore_remembered();

    assert_eq!(set.verdict("bar line"), Verdict::Unmatched);
}

/// A capture describes a set that no longer exists once the search changes,
/// exactly as it does when a filter is added — see `add`'s comment.
#[test]
fn changing_the_search_drops_a_pending_capture() {
    let mut set = set_with(&["foo"]);
    set.disable_all_remembering();

    set.set_search("bar").expect("valid pattern");
    assert!(!set.has_remembered());

    set.clear_search();
    assert!(!set.has_remembered());
}

#[test]
fn row_count_includes_the_search_row() {
    let mut set = set_with(&["foo", "bar"]);
    assert_eq!(set.row_count(), 2);

    set.set_search("baz").expect("valid pattern");
    assert_eq!(set.row_count(), 3);
}
```

Add to `src/document.rs` `mod tests`:

```rust
fn set_searching(pattern: &str) -> FilterSet {
    let mut set = FilterSet::new();
    set.set_search(pattern).expect("valid pattern");
    set
}

#[test]
fn a_searched_line_is_visible_when_unmatched_lines_are_hidden() {
    let mut document = doc(&["alpha", "beta", "gamma"]);
    document.set_mode(Mode::FilteredOnly);
    document.evaluate(&set_searching("beta"));

    assert_eq!(document.visible(), &[1]);
}

#[test]
fn match_count_includes_searched_lines() {
    let mut document = doc(&["foo a", "bar", "foo b"]);
    document.evaluate(&set_searching("foo"));

    assert_eq!(document.match_count(), 2);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filter:: 2>&1 | tail -20`
Expected: FAIL to compile — `no method named 'set_search' found`, `no variant named 'Searched'`, `cannot find value 'SEARCH_STYLE'`.

- [ ] **Step 3: Add the reserved style and the verdict variant**

In `src/filter.rs`, after the `DIM_STYLE` definition:

```rust
/// The colour reserved for the live search.
///
/// Deliberately outside `PALETTE`: drawing from it would make the search's
/// colour depend on how many filters happen to exist, so it would shift as
/// filters come and go. A fixed colour gives the user one rule — white means
/// what you just typed.
///
/// White *and* bold, for the reason `pane_block` gives about focus: a single
/// visual channel fails on a theme with weak contrast and in a terminal with
/// no colour at all.
pub(crate) const SEARCH_STYLE: Style = Style::new()
    .fg(Color::White)
    .add_modifier(Modifier::BOLD);
```

Extend `Verdict`:

```rust
/// What the filter set decided about one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Matched an including filter; carries its index, for colouring.
    Included(usize),
    /// Matched the live search rather than a numbered filter.
    ///
    /// Carries no index: the search lives in its own slot, precisely so that
    /// setting and clearing it cannot renumber the filters the user built.
    Searched,
    /// Matched no including filter.
    Unmatched,
    /// Removed by an excluding filter.
    Excluded,
}
```

- [ ] **Step 4: Add the slot and its methods**

Extend `FilterSet`:

```rust
#[derive(Debug, Default)]
pub struct FilterSet {
    filters: Vec<Filter>,
    /// The live search: at most one, replaced by each `/`, and never an
    /// element of `filters`.
    ///
    /// `Verdict::Included(usize)` is a *position* in `filters` — see
    /// `remove`'s doc comment. Were the search stored there, every `/` and
    /// every `Esc` would renumber the user's filters as a side effect of
    /// typing a search.
    search: Option<Filter>,
    /// Enabled flags captured by `disable_all_remembering`, awaiting a restore.
    ///
    /// Held separately from the filters so that a filter removed in the
    /// meantime simply drops out of the restore rather than resurrecting.
    remembered: Option<Vec<bool>>,
    /// The search slot's enabled flag, captured alongside `remembered`.
    remembered_search: Option<bool>,
}
```

Add these methods to `impl FilterSet`:

```rust
    /// Set the live search, replacing any previous one.
    ///
    /// One search at a time, like vim's search register: a second `/` is a
    /// new question, not another filter.
    pub fn set_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.search = Some(Filter {
            pattern,
            sense: Sense::Include,
            enabled: true,
            style: SEARCH_STYLE,
        });
        self.forget_capture();
        Ok(())
    }

    /// Drop the live search. A no-op when there is none.
    pub fn clear_search(&mut self) {
        self.search = None;
        self.forget_capture();
    }

    pub fn search(&self) -> Option<&Filter> {
        self.search.as_ref()
    }

    /// Enable or disable the search, reporting whether there was one.
    pub fn search_set_enabled(&mut self, enabled: bool) -> bool {
        match self.search.as_mut() {
            Some(search) => {
                search.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Rows the filter pane draws: one per numbered filter, plus the search
    /// row when a search exists.
    ///
    /// Distinct from `len`, which counts only the numbered filters and is
    /// what `Verdict::Included` indexes into. The pane needs the larger
    /// number; nothing else does.
    pub fn row_count(&self) -> usize {
        self.filters.len() + usize::from(self.search.is_some())
    }

    /// Drop a pending `!` capture, both halves together.
    ///
    /// A capture describes a set that no longer exists once the set changes.
    /// Keeping it would strand it — see `add`. Both fields go, always:
    /// dropping only one leaves the capture half-valid, which is worse than
    /// dropping neither.
    fn forget_capture(&mut self) {
        self.remembered = None;
        self.remembered_search = None;
    }
```

Replace the two existing `self.remembered = None;` statements in `add` and `add_excluding` with `self.forget_capture();`, keeping their surrounding comments.

- [ ] **Step 5: Teach the existing methods about the slot**

`verdict` gains one step, between exclusion and inclusion:

```rust
    pub fn verdict(&self, line: &str) -> Verdict {
        // Exclusion is applied after inclusion and overrides it, so a line an
        // including filter selected is still removed if an excluding filter
        // also matches it.
        if self.filters.iter().any(|filter| {
            filter.enabled && filter.sense == Sense::Exclude && filter.pattern.is_match(line)
        }) {
            return Verdict::Excluded;
        }

        // The live search outranks the numbered filters: the user's attention
        // is on the pattern they just typed, so it wins the colour on a line
        // that several things match.
        if let Some(search) = &self.search
            && search.enabled
            && search.pattern.is_match(line)
        {
            return Verdict::Searched;
        }

        self.filters
            .iter()
            .enumerate()
            .find(|(_, filter)| {
                filter.enabled && filter.sense == Sense::Include && filter.pattern.is_match(line)
            })
            .map_or(Verdict::Unmatched, |(index, _)| Verdict::Included(index))
    }
```

`style_for` gains an arm:

```rust
    pub fn style_for(&self, verdict: Verdict) -> Option<Style> {
        match verdict {
            Verdict::Included(index) => self.filters.get(index).map(|f| f.style),
            Verdict::Searched => Some(SEARCH_STYLE),
            Verdict::Unmatched if self.any_including() => Some(DIM_STYLE),
            Verdict::Unmatched | Verdict::Excluded => None,
        }
    }
```

`set_all_enabled`, `any_enabled`, `disable_all_remembering` and `restore_remembered` all take the slot along:

```rust
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            filter.enabled = enabled;
        }
        if let Some(search) = self.search.as_mut() {
            search.enabled = enabled;
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.filters.iter().any(|filter| filter.enabled)
            || self.search.as_ref().is_some_and(|search| search.enabled)
    }

    pub fn disable_all_remembering(&mut self) {
        if self.remembered.is_some() {
            return;
        }
        self.remembered = Some(self.filters.iter().map(|f| f.enabled).collect());
        self.remembered_search = self.search.as_ref().map(|search| search.enabled);
        self.set_all_enabled(false);
    }

    pub fn restore_remembered(&mut self) {
        let Some(remembered) = self.remembered.take() else {
            return;
        };
        for (filter, was_enabled) in self.filters.iter_mut().zip(remembered) {
            filter.enabled = was_enabled;
        }
        // Taken unconditionally, so a capture made while no search existed
        // does not linger and get applied to an unrelated later search.
        let remembered_search = self.remembered_search.take();
        if let (Some(search), Some(was_enabled)) = (self.search.as_mut(), remembered_search) {
            search.enabled = was_enabled;
        }
    }
```

- [ ] **Step 6: Make `document.rs` exhaustive**

`match_count`, inside `evaluate`:

```rust
        self.match_count = self
            .verdicts
            .iter()
            .filter(|verdict| matches!(verdict, Verdict::Included(_) | Verdict::Searched))
            .count();
```

`recompute_visible`'s match arms:

```rust
            .filter(|(_, verdict)| match (self.mode, verdict) {
                // Excluded lines are gone in both modes; the toggle governs
                // unmatched lines only.
                (_, Verdict::Excluded) => false,
                (Mode::Dimmed, _) => true,
                (Mode::FilteredOnly, Verdict::Included(_) | Verdict::Searched) => true,
                (Mode::FilteredOnly, Verdict::Unmatched) => false,
            })
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: PASS, all tests.

- [ ] **Step 8: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/filter.rs src/document.rs
git commit -m "feat(filter): give the live search its own slot and verdict

The search becomes a filter, held in a dedicated slot rather than in the
numbered vector: Verdict::Included is a position, so storing it there
would renumber the user's filters on every / and every Esc.

It outranks the numbered filters and is outranked by exclusion, so a line
several things match takes the colour of the pattern just typed."
```

---

### Task 2: Dimming takes a narrower predicate than hiding

**Files:**
- Modify: `src/filter.rs` (`any_including`, new `any_numbered_including`, `style_for`)
- Test: `src/filter.rs` `mod tests`

**Interfaces:**
- Consumes: `Verdict::Searched`, `FilterSet::set_search` from Task 1.
- Produces:
  - `FilterSet::any_including(&self) -> bool` — now `pub`, counts the search
  - `FilterSet::any_numbered_including(&self) -> bool` — private, drives dimming alone

This is the subtlest pair in the design. Two predicates that differ in exactly one input, used in two places that look interchangeable and are not.

- [ ] **Step 1: Write the failing tests**

Add to `src/filter.rs` `mod tests`:

```rust
/// Dimming is a contrast mechanism, and a search on its own is one thing to
/// see: its hits already carry the span highlight, so greying the rest of the
/// file buys nothing and costs the readability of the context the user
/// searched in order to reach.
#[test]
fn a_search_alone_does_not_dim() {
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");

    assert_eq!(set.style_for(Verdict::Unmatched), None);
}

/// But it still counts as something to hide against, so `Ctrl-H` works after
/// a bare search. This is the asymmetry the two predicates exist for.
#[test]
fn a_search_alone_still_counts_for_hiding() {
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");

    assert!(set.any_including(), "Ctrl-H would have nothing to hide against");
}

/// Add a numbered filter and dimming switches on, because now there really
/// are two things to tell apart.
#[test]
fn a_numbered_filter_alongside_a_search_dims() {
    let mut set = set_with(&["ERROR"]);
    set.set_search("foo").expect("valid pattern");

    assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE));
}

#[test]
fn a_disabled_search_counts_for_neither() {
    let mut set = FilterSet::new();
    set.set_search("foo").expect("valid pattern");
    set.search_set_enabled(false);

    assert!(!set.any_including());
    assert_eq!(set.style_for(Verdict::Unmatched), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filter::tests::a_search_alone_does_not_dim -- --exact 2>&1 | tail -20`
Expected: FAIL — `a_search_alone_does_not_dim` asserts `None` but gets `Some(DIM_STYLE)`, because Task 1's `style_for` still guards on `any_including`.

- [ ] **Step 3: Split the predicate in two**

Replace the private `any_including` in `src/filter.rs` with this pair:

```rust
    /// Whether anything at all is marking lines — a numbered including filter,
    /// or the live search.
    ///
    /// Drives hiding (including the `Ctrl-H` guard in `Document`) and `n`/`N`.
    /// Public because `Document` caches it at `evaluate` time.
    pub fn any_including(&self) -> bool {
        self.any_numbered_including()
            || self.search.as_ref().is_some_and(|search| search.enabled)
    }

    /// Whether a *numbered* including filter is enabled. Drives dimming alone.
    ///
    /// Dimming is a contrast mechanism: unmatched lines recede so that
    /// coloured matches stand out, and its value scales with how many things
    /// are being told apart. A search on its own is one thing, and its hits
    /// already carry the span highlight — so dimming the rest of the file buys
    /// nothing and costs the readability of the context the search was run in
    /// order to reach.
    ///
    /// The consequence is deliberate and is the one place dimming stops being
    /// a strict preview of hiding: after a bare `/foo`, nothing is grey and
    /// `Ctrl-H` still hides plenty. A user pressing a key that means "hide
    /// unmatched" is not surprised to get it.
    fn any_numbered_including(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense == Sense::Include)
    }
```

Point `style_for`'s `Unmatched` arm at the narrower one:

```rust
            Verdict::Unmatched if self.any_numbered_including() => Some(DIM_STYLE),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: PASS, all tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/filter.rs
git commit -m "feat(filter): dim on numbered filters, hide on anything including

A search on its own is one thing to see and its hits already carry the
span highlight, so dimming the rest of the file buys nothing and costs
the readability of the context the search was run to reach.

/foo therefore reads as it always has, while Ctrl-H still collapses the
file to the matching lines."
```

---

### Task 3: `promote_search`

**Files:**
- Modify: `src/filter.rs`
- Test: `src/filter.rs` `mod tests`

**Interfaces:**
- Consumes: the search slot from Task 1.
- Produces: `FilterSet::promote_search(&mut self) -> bool`

- [ ] **Step 1: Write the failing tests**

```rust
/// The probe-and-keep loop: `/` to try a pattern, `p` to keep it, `/` again.
/// Nothing is retyped.
#[test]
fn promoting_moves_the_search_into_the_numbered_set() {
    let mut set = set_with(&["alpha"]);
    set.set_search("beta").expect("valid pattern");

    assert!(set.promote_search());

    assert_eq!(set.len(), 2);
    assert!(set.search().is_none(), "the slot should be free for the next probe");
    assert_eq!(set.verdict("beta line"), Verdict::Included(1));
}

#[test]
fn a_promoted_search_takes_the_next_palette_colour() {
    let mut set = set_with(&["alpha"]);
    set.set_search("beta").expect("valid pattern");
    set.promote_search();

    assert_ne!(set.filters()[0].style, set.filters()[1].style);
    assert_ne!(
        set.filters()[1].style, SEARCH_STYLE,
        "a promoted filter is a keeper, not the live probe"
    );
}

/// Promoting must not silently switch on a search the user had toggled off.
#[test]
fn promoting_preserves_the_enabled_state() {
    let mut set = FilterSet::new();
    set.set_search("beta").expect("valid pattern");
    set.search_set_enabled(false);

    set.promote_search();

    assert!(!set.filters()[0].enabled);
}

#[test]
fn promoting_without_a_search_reports_failure_and_changes_nothing() {
    let mut set = set_with(&["alpha"]);

    assert!(!set.promote_search());
    assert_eq!(set.len(), 1);
}

#[test]
fn promoting_drops_a_pending_capture() {
    let mut set = FilterSet::new();
    set.set_search("beta").expect("valid pattern");
    set.disable_all_remembering();

    set.promote_search();

    assert!(!set.has_remembered());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filter::tests::promoting 2>&1 | tail -20`
Expected: FAIL to compile — `no method named 'promote_search' found`.

- [ ] **Step 3: Implement it**

```rust
    /// Move the live search into the numbered set and free the slot.
    ///
    /// This is the probe-and-keep loop `p` exists for: `/` a pattern, look at
    /// what it catches, `p` to keep it, `/` again — building a set worth
    /// saving without retyping anything.
    ///
    /// The enabled state is carried across rather than forced to `true`, so a
    /// search the user had toggled off is not silently switched back on.
    /// Reports whether there was a search to promote.
    pub fn promote_search(&mut self) -> bool {
        let Some(mut search) = self.search.take() else {
            return false;
        };
        search.style = self.next_style();
        self.filters.push(search);
        self.forget_capture();
        true
    }
```

`next_style` is already defined and already reads `self.filters.len()`, which at this point is the index the promoted filter is about to take.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: PASS, all tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/filter.rs
git commit -m "feat(filter): promote the live search into the numbered set

Probe with /, keep with p, probe again - a filter set assembled without
retyping a regex that was hard to get right. The enabled state carries
across so a search toggled off is not silently switched back on."
```

---

### Task 4: The `Ctrl-H` guard — issue #36

**Files:**
- Modify: `src/document.rs` (`Document` struct, `evaluate`, `recompute_visible`)
- Test: `src/document.rs` `mod tests`

**Interfaces:**
- Consumes: `FilterSet::any_including` (public as of Task 2).
- Produces: no new public API. `Document::recompute_visible` keeps its existing no-argument signature.

Issue #36: `Ctrl-H` with no filters blanks the pane. The cause is that dimming guards itself with `any_including` and hiding never asks. This design makes it far easier to hit — `Esc` and `!` both blank the pane instantly — so the fix ships here.

- [ ] **Step 1: Write the failing tests**

```rust
/// Issue #36. With nothing including, there is nothing to hide against, so
/// hiding shows the file rather than blanking the pane. Dimming has always
/// had this guard (`style_for`); hiding never did.
#[test]
fn hiding_with_no_filters_shows_the_whole_file() {
    let mut document = doc(&["alpha", "beta", "gamma"]);
    document.set_mode(Mode::FilteredOnly);
    document.evaluate(&FilterSet::new());

    assert_eq!(document.visible(), &[0, 1, 2]);
}

/// The same bug through a second door, unreported until #36 was investigated:
/// with only excluding filters there is nothing to hide unmatched lines
/// *against* — the user wants the file minus the noise, not an empty pane.
#[test]
fn hiding_with_only_excluding_filters_shows_the_rest_of_the_file() {
    let mut document = doc(&["alpha", "noise here", "gamma"]);
    document.set_mode(Mode::FilteredOnly);
    document.evaluate(&set_excluding(&["noise"]));

    assert_eq!(document.visible(), &[0, 2]);
}

/// A bare search counts as something to hide against, which is what makes
/// `/foo` followed by `Ctrl-H` an instant grep.
#[test]
fn hiding_with_only_a_search_collapses_to_its_matches() {
    let mut document = doc(&["alpha", "beta", "gamma"]);
    document.set_mode(Mode::FilteredOnly);
    document.evaluate(&set_searching("beta"));

    assert_eq!(document.visible(), &[1]);
}

/// The guard must not soften a real filter set: a file with no hits still
/// renders blank, which is exactly what the directory-skim feature needs
/// "blank" to mean.
#[test]
fn a_file_with_no_hits_is_still_blank_when_hiding() {
    let mut document = doc(&["alpha", "beta"]);
    document.set_mode(Mode::FilteredOnly);
    document.evaluate(&set_with(&["ERROR"]));

    assert!(document.visible().is_empty());
}

/// The guard is cached at `evaluate` time precisely so that the mode toggle
/// stays O(lines) and runs no regex — see `recompute_visible`.
#[test]
fn the_guard_survives_a_mode_toggle_without_re_evaluating() {
    let mut document = doc(&["alpha", "beta"]);
    document.evaluate(&FilterSet::new());

    document.set_mode(Mode::FilteredOnly);
    document.recompute_visible();

    assert_eq!(document.visible(), &[0, 1]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib document::tests::hiding_with 2>&1 | tail -20`
Expected: FAIL — `hiding_with_no_filters_shows_the_whole_file` asserts `[0, 1, 2]` and gets `[]`.

- [ ] **Step 3: Cache the predicate on the document**

Add the field to `Document`:

```rust
    /// Whether anything was marking lines at the last `evaluate` — a numbered
    /// including filter, or the live search.
    ///
    /// Cached rather than asked of the `FilterSet` inside
    /// `recompute_visible`, so that method keeps taking no arguments and stays
    /// independent of the filter set. That independence is what makes the
    /// `Ctrl-H` path cheap: the toggle re-derives `visible` from the verdicts
    /// alone, with no borrow and no regex.
    anything_including: bool,
```

Initialise it in `Document::new` alongside `match_count`:

```rust
            anything_including: false,
```

Set it in `evaluate`, before `recompute_visible` runs:

```rust
        self.anything_including = filters.any_including();
        self.recompute_visible();
```

- [ ] **Step 4: Use it in `recompute_visible`**

```rust
            .filter(|(_, verdict)| match (self.mode, verdict) {
                // Excluded lines are gone in both modes; the toggle governs
                // unmatched lines only.
                (_, Verdict::Excluded) => false,
                (Mode::Dimmed, _) => true,
                (Mode::FilteredOnly, Verdict::Included(_) | Verdict::Searched) => true,
                // Issue #36: with nothing including, there is nothing to hide
                // *against*, so hiding shows the file rather than blanking the
                // pane. Dimming has always had this guard in `style_for`;
                // hiding never did, which made `Ctrl-H` with no filters — and
                // with only excluding filters — produce an empty view that read
                // as "this file is empty".
                (Mode::FilteredOnly, Verdict::Unmatched) => !self.anything_including,
            })
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. If `lib.rs`'s `status_text` tests fail on the "nothing to show — no filters" string, leave them for now — Task 12 revisits that message.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/document.rs
git commit -m "fix(view): stop Ctrl-H blanking the pane with nothing to hide against

Closes half of #36. Dimming has always guarded itself with any_including;
hiding never did, so Ctrl-H with no filters - and, unreported until now,
with only excluding filters - emptied the view entirely.

Cached at evaluate time so recompute_visible keeps taking no arguments
and the mode toggle stays free of both borrows and regex.

Blank now means exactly one thing: this file has no hits."
```

---

### Task 5: `set_cursor_position` in the vendored fork

**Files:**
- Modify: `vendor/tui-textarea-2/src/textarea.rs`
- Modify: `vendor/tui-textarea-2/PATCH.md`
- Test: `vendor/tui-textarea-2/tests/cursor.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `TextArea::set_cursor_position(&mut self, cursor: (usize, usize))`

`n`/`N` need to land the cursor on an arbitrary row. `CursorMove::Jump` takes `u16` and its handler does `*row as usize`, so a caller past 65,535 lines truncates and lands 65,536 rows from the target. `set_lines` clamps in `usize` but replaces the buffer, clears history and resets the viewport — far too heavy for a cursor move. This adds the missing primitive.

- [ ] **Step 1: Write the failing test**

Append to `vendor/tui-textarea-2/tests/cursor.rs`:

```rust
#[test]
fn set_cursor_position_clamps_in_usize() {
    let lines: Vec<String> = (0..70_000).map(|i| format!("line {i}")).collect();
    let mut textarea = TextArea::new(lines);

    textarea.set_cursor_position((70_000 - 1, 0));

    assert_eq!(
        textarea.cursor().0,
        69_999,
        "CursorMove::Jump would have truncated this row through u16"
    );
}

#[test]
fn set_cursor_position_clamps_past_the_end() {
    let mut textarea = TextArea::from(["alpha", "beta"]);

    textarea.set_cursor_position((99, 99));

    assert_eq!(textarea.cursor(), (1, 4));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p tui-textarea-2 --test cursor set_cursor_position 2>&1 | tail -20`
Expected: FAIL to compile — `no method named 'set_cursor_position' found`.

- [ ] **Step 3: Add the method**

In `vendor/tui-textarea-2/src/textarea.rs`, immediately after `set_lines`:

```rust
    /// Move the cursor to a position in the buffer, clamped to it, without
    /// touching the text.
    ///
    /// [`CursorMove::Jump`] takes `u16` and its handler widens with `as
    /// usize`, so a caller past 65,535 lines truncates and lands 65,536 rows
    /// from its target. This clamps in `usize` instead, the same way
    /// [`TextArea::set_lines`] does — but without replacing the buffer,
    /// clearing the history or resetting the viewport, none of which a plain
    /// cursor move should do.
    ///
    /// The viewport is not scrolled here. Rendering brings the cursor into
    /// view, exactly as it does after [`TextArea::move_cursor`].
    pub fn set_cursor_position(&mut self, cursor: (usize, usize)) {
        self.cancel_selection();
        self.cursor = self.clamp_cursor_to_buffer(cursor);
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p tui-textarea-2 --test cursor 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Record the patch**

Append to the "Changes from upstream" section of `vendor/tui-textarea-2/PATCH.md`:

```markdown
### 4. Cursor positioning without `u16` truncation

- `src/textarea.rs`: added `set_cursor_position` after `set_lines`.
- `tests/cursor.rs`: two tests appended.

`recon`'s `n`/`N` land the cursor on an arbitrary source line.
`CursorMove::Jump` takes `u16` and widens with `as usize` in its handler, so
a caller past 65,535 lines truncates. `set_lines` clamps in `usize` but
replaces the buffer, clears history and resets the viewport, which a plain
cursor move must not do. This exposes the existing private
`clamp_cursor_to_buffer` as a cursor-only operation.
```

Renumber if the section number 4 is already taken.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo test
git add vendor/tui-textarea-2/src/textarea.rs vendor/tui-textarea-2/PATCH.md vendor/tui-textarea-2/tests/cursor.rs
git commit -m "feat(vendor): add set_cursor_position, clamped in usize

CursorMove::Jump takes u16 and widens with 'as usize' in its handler, so
landing the cursor past 65,535 lines truncates. set_lines clamps properly
but replaces the buffer and resets the viewport, which a cursor move must
not do. n/N need the missing primitive."
```

---

### Task 6: `n` and `N` step between interesting lines

**Files:**
- Modify: `src/widgets/fileview.rs:374-387` (remove the `n`/`N` arms), and add `set_cursor_row`
- Modify: `src/lib.rs` (`handle_event`, new `next_interesting` and `step_to_interesting`)
- Test: `src/lib.rs` `mod tests`

**Interfaces:**
- Consumes: `Verdict::Searched` (Task 1), `TextArea::set_cursor_position` (Task 5), `Document::verdicts`, `Document::visible_position`, `App::cursor_source`.
- Produces:
  - `FileView::set_cursor_row(&mut self, row: usize)`
  - `App::step_to_interesting(&mut self, backwards: bool)` — used by Task 7's `/`

`n`/`N` move up to `App` because they need `Document::verdicts()`, which `FileView` cannot see. They must stay *scoped* to the file view: pressed in the navigator or the filter pane they belong to that pane, not to the view's cursor.

- [ ] **Step 1: Write the failing tests**

Add to `src/lib.rs` `mod tests`:

```rust
/// `n` walks the union of filter hits and search hits, in source order. This
/// is the whole point of the design: one notion of an interesting line.
#[test]
fn n_steps_between_filter_and_search_matches_alike() {
    let mut app = app_over_file("n_union", "alpha\nERROR one\nbeta\ntimeout two\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("ERROR").expect("valid pattern");
    app.filters.set_search("timeout").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.cursor_source(), 1, "did not reach the filter match");

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.cursor_source(), 3, "did not reach the search match");
}

/// Line-oriented, not span-oriented: three hits on one line is one stop.
/// `recon` is a line-focused tool, and the alternative cannot be explained
/// without explaining the implementation.
#[test]
fn n_stops_once_on_a_line_with_several_matches() {
    let mut app = app_over_file("n_once", "foo foo foo\nbar\nfoo\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.set_search("foo").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.cursor_source(), 2, "stopped more than once on line 0");
}

#[test]
fn n_wraps_at_the_end_of_the_file() {
    let mut app = app_over_file("n_wrap", "hit\nplain\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.set_search("hit").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.cursor_source(), 0, "did not wrap");
}

#[test]
fn capital_n_walks_backwards() {
    let mut app = app_over_file("n_back", "hit a\nplain\nhit b\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.set_search("hit").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('N'));
    assert_eq!(app.cursor_source(), 2, "N did not walk upwards and wrap");
}

/// Quiet, not a panic and not a jump to line 0.
#[test]
fn n_with_nothing_interesting_does_nothing() {
    let mut app = app_over_file("n_empty", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    let before = app.cursor_source();

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(app.cursor_source(), before);
}

/// `n` belongs to the file view. Hoisting it into `App` must not make it
/// global — in the navigator it is still the navigator's key.
#[test]
fn n_in_the_navigator_does_not_move_the_file_view_cursor() {
    let mut app = app_over("n_nav", &["alpha.log", "beta.log"]);
    app.filters.set_search("x").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('e'));
    let before = app.cursor_source();

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(app.cursor_source(), before, "n leaked out of the file view");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tests::n_ 2>&1 | tail -30`
Expected: FAIL — `n_steps_between_filter_and_search_matches_alike` finds the cursor still at 0, because `FileView`'s `n` arm calls the old `repeat_search`, which knows nothing about filters.

- [ ] **Step 3: Give `FileView` a cursor setter and drop its search**

In `src/widgets/fileview.rs`, delete the `search_reverse` field, the `search`, `repeat_search` and `step_search` methods, and the `n` / `N` arms of `handle_events` (lines 374-387). Add:

```rust
    /// Put the cursor on `row` of the current buffer, clamped to it.
    ///
    /// Used by `n`/`N`, which decide *which* line to land on in `App` — the
    /// only place that can see both the verdicts and the cursor — and then
    /// ask the view to go there.
    pub fn set_cursor_row(&mut self, row: usize) {
        self.textarea.set_cursor_position((row, 0));
    }
```

Delete the now-unreachable `FileView` search tests: `search_jumps_to_the_first_match`, `search_supports_regex`, `search_repeats_forwards_and_backwards`, `search_wraps_around_the_buffer`, `a_backward_search_walks_upwards`, and the invalid-pattern one at `fileview.rs:950`. Their behaviour is now covered by the `App`-level tests above and by Task 7.

- [ ] **Step 4: Find and move to the next interesting line**

Add to `impl App` in `src/lib.rs`:

```rust
    /// The next source line matched by an enabled including filter or by the
    /// live search, walking from the cursor and wrapping once.
    ///
    /// Line-oriented rather than span-oriented: a line with three matches is
    /// one stop. `recon` is a line-focused tool, and the alternative — three
    /// stops on a search hit but one on a filter hit — is a distinction that
    /// cannot be explained without explaining the implementation.
    ///
    /// An interesting line is always visible in both modes: `Excluded` is the
    /// only verdict that hides a line in `Dimmed`, and it is never
    /// interesting. So the caller can map through `visible_position` without
    /// a fallback for "the target is hidden".
    fn next_interesting(&self, backwards: bool) -> Option<usize> {
        let verdicts = self.document.verdicts();
        let len = verdicts.len();
        if len == 0 {
            return None;
        }
        let from = self.cursor_source();
        // 1..=len, so the line the cursor is on is considered last: `n` moves
        // off it if anything else matches, and stays put if it is the only
        // interesting line in the file.
        (1..=len)
            .map(|step| {
                if backwards {
                    (from + len - step) % len
                } else {
                    (from + step) % len
                }
            })
            .find(|&index| {
                matches!(verdicts[index], Verdict::Included(_) | Verdict::Searched)
            })
    }

    /// Move the file view's cursor to the next interesting line, if there is
    /// one. Quiet when there is not.
    fn step_to_interesting(&mut self, backwards: bool) {
        let Some(target) = self.next_interesting(backwards) else {
            return;
        };
        let Some(row) = self.document.visible_position(target) else {
            return;
        };
        if let Some(AppWidget::FileView(view)) = self
            .widgets
            .iter_mut()
            .find(|w| matches!(w, AppWidget::FileView(_)))
        {
            view.set_cursor_row(row);
        }
    }
```

Import the variant at the top of `src/lib.rs`, alongside the existing `use filter::FilterSet;`:

```rust
use filter::{FilterSet, Verdict};
```

- [ ] **Step 5: Bind the keys, scoped to the file view**

In `handle_event`'s global `match key.code`, after the `'!'` arm:

```rust
                // Scoped to the file view rather than global: `n` in the
                // navigator is the navigator's key, and hoisting the binding
                // up here to reach the verdicts must not change that.
                KeyCode::Char(c @ ('n' | 'N'))
                    if key.modifiers.is_empty()
                        && matches!(
                            self.widgets[self.active_widget],
                            AppWidget::FileView(_)
                        ) =>
                {
                    self.step_to_interesting(c == 'N');
                    return Ok(());
                }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS, all tests.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/lib.rs src/widgets/fileview.rs
git commit -m "feat(view): n and N step between interesting lines

n now walks the union of filter hits and search hits, which is the point
of treating search as a filter. Line-oriented: a line with three matches
is one stop, because recon is a line-focused tool and the alternative -
three stops on a search hit but one on a filter hit - cannot be explained
without explaining the implementation.

Handled in App because only App sees both the verdicts and the cursor,
but guarded to the file view so n in the navigator stays the navigator's."
```

---

### Task 7: `/` sets the search filter, and `?` is retired

**Files:**
- Modify: `src/lib.rs` (`SearchPrompt`, `PromptKind`, `handle_search_key`, `run_search`, `handle_event`'s `/` arm)
- Test: `src/lib.rs` `mod tests`

**Interfaces:**
- Consumes: `FilterSet::set_search` (Task 1), `App::step_to_interesting` (Task 6).
- Produces: `App::apply_search(&mut self, pattern: &str) -> Result<(), regex::Error>`

`/` is defined as "set the search, then do exactly what `n` does", so there is one movement path rather than two — and so the buffer rebuild that adding a filter triggers in `Mode::FilteredOnly` is completed by `refresh_view` before anything tries to move a cursor through it.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn slash_sets_the_search_filter_and_moves_to_its_first_hit() {
    let mut app = app_over_file("slash_filter", "alpha\nbeta\ngamma\nbeta again\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert!(app.filters.search().is_some(), "the search did not become a filter");
    assert_eq!(app.cursor_source(), 1);
}

/// A search survives loading another file, exactly as the numbered filters
/// do — it is one of them now.
#[test]
fn the_search_filter_survives_a_file_load() {
    let mut app = app_over("slash_survives", &["alpha.log", "beta.log"]);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "x");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('j'));

    assert!(app.filters.search().is_some(), "the search did not outlive the load");
}

/// With hiding on, a bare search is an instant grep — the capability the
/// merge unlocks.
#[test]
fn a_search_with_hiding_on_collapses_the_file_to_its_matches() {
    let mut app = app_over_file("slash_grep", "alpha\nbeta\ngamma\nbeta again\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('H'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.document.visible(), &[1, 3]);
}

#[test]
fn an_invalid_search_pattern_leaves_the_prompt_open() {
    let mut app = app_over_file("slash_bad", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "[");
    key(&mut app, KeyCode::Enter);

    assert!(app.search.is_some(), "prompt closed on an invalid pattern");
    assert!(app.filters.search().is_none(), "a rejected pattern became a filter");
}

/// `?` is reserved for the help view (#25). With n/N covering both
/// directions there is nothing left for it to do.
#[test]
fn question_mark_no_longer_opens_a_prompt() {
    let mut app = app_over_file("question_inert", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('?'));

    assert!(app.search.is_none(), "? still opens a prompt");
}

/// `/` in the navigator still searches filenames — that pane has its own
/// search and is untouched by this work.
#[test]
fn slash_in_the_navigator_still_searches_filenames() {
    let mut app = app_over("slash_nav", &["alpha.log", "zebra.log"]);
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zebra");
    key(&mut app, KeyCode::Enter);

    assert!(app.filters.search().is_none(), "a nav search became a filter");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tests::slash_ 2>&1 | tail -30`
Expected: FAIL — `slash_sets_the_search_filter_and_moves_to_its_first_hit` finds `app.filters.search()` is `None`, because `run_search` still calls the deleted `FileView::search`.

- [ ] **Step 3: Retire `?` from the prompt**

Delete the `reverse` field from `SearchPrompt` and simplify `line`:

```rust
/// A search pattern being typed at the bottom of the screen.
#[derive(Debug, Default)]
struct SearchPrompt {
    pattern: String,
    error: Option<String>,
    kind: PromptKind,
}

impl SearchPrompt {
    /// What the bottom line shows: the error if the pattern was rejected,
    /// otherwise the pattern being typed behind its sigil.
    fn line(&self) -> String {
        match (&self.error, self.kind) {
            (Some(error), _) => error.clone(),
            (None, PromptKind::Filter) => format!("filter: {}", self.pattern),
            (None, PromptKind::Exclude) => format!("exclude: {}", self.pattern),
            (None, PromptKind::Search) => format!("/{}", self.pattern),
        }
    }
}
```

In `handle_event`, narrow the sigil arm to `/` alone:

```rust
                // The filter pane has nothing to search over, so the prompt
                // is not opened at all while it has focus — opening it and
                // then having `Enter` silently do nothing looked like the
                // keystroke was simply swallowed.
                //
                // `?` used to open a backward search here. `n`/`N` cover both
                // directions now, so it is unbound and reserved for the help
                // view (#25).
                KeyCode::Char('/')
                    if key.modifiers.is_empty()
                        && !matches!(
                            self.widgets[self.active_widget],
                            AppWidget::FilterList(_)
                        ) =>
                {
                    self.search = Some(SearchPrompt::default());
                    return Ok(());
                }
```

In `handle_search_key`'s `Enter` branch, drop the `reverse` read so the call reads `self.run_search(&pattern)`. Update the `PromptKind::Search` arm to match the new `run_search` signature below.

- [ ] **Step 4: Route `/` to the filter set**

Replace `run_search` and add `apply_search`:

```rust
    /// Run a committed `/` pattern against whichever pane has focus.
    ///
    /// The navigator has its own search over filenames and keeps it. In the
    /// file view, `/` now sets the live search *filter* — the pane has no
    /// search of its own any more.
    fn run_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let mut view_search = false;
        let action = match &mut self.widgets[self.active_widget] {
            AppWidget::FileNav(nav) => nav.search(pattern, false)?,
            AppWidget::FileView(_) => {
                // Deferred rather than done here: setting the filter needs
                // `&mut self` for `refresh_view`, and this borrow of
                // `self.widgets` is still live.
                view_search = true;
                None
            }
            // Unreachable in practice: `/` is not opened at all while the
            // filter pane has focus (see its guard in `handle_event`), and the
            // prompt swallows every key including `Tab` while it is open. Kept
            // so this match stays exhaustive against a fourth `AppWidget`.
            AppWidget::FilterList(_) => None,
        };

        if let Some(action) = action {
            self.perform(action);
        }
        if view_search {
            self.apply_search(pattern)?;
        }
        Ok(())
    }

    /// Set the live search filter and move to its first hit.
    ///
    /// Defined as "set it, then do exactly what `n` does", so there is one
    /// movement path rather than two — and so the buffer rebuild that adding
    /// a filter triggers in `Mode::FilteredOnly` is completed by
    /// `refresh_view` before anything moves a cursor through it.
    ///
    /// A pattern that will not compile is reported and changes nothing, so the
    /// prompt can stay open over an intact previous search.
    fn apply_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.set_search(pattern)?;
        self.refresh_view();
        self.step_to_interesting(false);
        Ok(())
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. The existing `slash_opens_a_search_prompt` test should still pass; `question_mark_opens_a_backward_search` at `lib.rs:1544` must be deleted — `question_mark_no_longer_opens_a_prompt` replaces it.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/lib.rs
git commit -m "feat(search): / sets the live search filter, and ? is retired

/ in the file view now sets a filter rather than the textarea's own
search, so it sees hidden lines, survives a file load, honours exclusion
and answers to '!' - all for free, because it is a filter now.

Defined as 'set it, then do what n does', so there is one movement path.
? is unbound and reserved for the help view (#25); n/N cover both
directions."
```

---

### Task 8: `Esc` clears the search

**Files:**
- Modify: `src/lib.rs` (`handle_event`)
- Test: `src/lib.rs` `mod tests`

**Interfaces:**
- Consumes: `FilterSet::clear_search` (Task 1).
- Produces: no new API.

Without this the feature leaks: a search typed ten minutes ago keeps changing what is on screen with no way to stop it. Filters have `space`, `d` and `!`; the search needs its own off switch.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn escape_clears_the_search_filter() {
    let mut app = app_over_file("esc_clears", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert!(app.filters.search().is_some(), "sanity: search set");

    key(&mut app, KeyCode::Esc);

    assert!(app.filters.search().is_none());
}

/// Issue #36's guard is what makes this safe: clearing the last thing that
/// was including must not leave a blank pane behind.
#[test]
fn escape_while_hiding_restores_the_file_rather_than_blanking_it() {
    let mut app = app_over_file("esc_hiding", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('H'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.document.visible(), &[1], "sanity: grepped down");

    key(&mut app, KeyCode::Esc);

    assert_eq!(app.document.visible(), &[0, 1, 2], "the pane went blank");
}

/// An open prompt still wins: Esc there cancels the prompt, as it always has,
/// rather than reaching past it to delete an established search.
#[test]
fn escape_in_an_open_prompt_still_cancels_the_prompt() {
    let mut app = app_over_file("esc_prompt", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "gamma");

    key(&mut app, KeyCode::Esc);

    assert!(app.search.is_none(), "the prompt did not close");
    assert!(app.filters.search().is_some(), "Esc reached past the prompt");
}

#[test]
fn escape_with_no_search_does_nothing() {
    let mut app = app_over_file("esc_noop", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Esc);

    assert_eq!(app.filters.len(), 1, "Esc touched the numbered filters");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tests::escape_ 2>&1 | tail -30`
Expected: FAIL — `escape_clears_the_search_filter` finds the search still set.

- [ ] **Step 3: Bind it**

In `handle_event`'s global `match key.code`, after the `'/'` arm. It sits *after* the open-prompt early return at the top of the function, so a prompt already wins without any extra guard:

```rust
                // Global, like `!`: the search is app state, not the file
                // view's, and having to focus a particular pane to switch it
                // off would make it easy to leave one running by accident.
                //
                // This departs from vim, where Esc leaves the pattern alone.
                // Here the search is a filter, and a filter that cannot be
                // turned off is a leak — one typed ten minutes ago would keep
                // changing what is on screen with nothing to stop it.
                //
                // An open prompt already took this key: `handle_event` returns
                // early while `self.search` is `Some`, so Esc there cancels the
                // prompt rather than reaching past it.
                KeyCode::Esc if key.modifiers.is_empty() => {
                    self.filters.clear_search();
                    self.refresh_view();
                    return Ok(());
                }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS, all tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/lib.rs
git commit -m "feat(search): Esc clears the live search

A filter that cannot be turned off is a leak: a search typed ten minutes
ago would keep changing what is on screen with nothing to stop it.

Departs from vim, where Esc leaves the pattern alone. An open prompt
still wins, since handle_event returns early while one is up."
```

---

### Task 9: `p` promotes the search into the numbered set

**Files:**
- Modify: `src/lib.rs` (`handle_event`)
- Test: `src/lib.rs` `mod tests`

**Interfaces:**
- Consumes: `FilterSet::promote_search` (Task 3).
- Produces: no new API.

- [ ] **Step 1: Write the failing tests**

```rust
/// Probe, keep, probe again — a filter set assembled without retyping a
/// regex that was hard to get right. Feeds #8.
#[test]
fn p_promotes_the_search_and_frees_the_slot() {
    let mut app = app_over_file("p_promote", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('p'));

    assert_eq!(app.filters.len(), 1, "the search did not become a filter");
    assert!(app.filters.search().is_none(), "the slot is not free for the next probe");
}

#[test]
fn two_probes_promote_into_two_filters() {
    let mut app = app_over_file("p_twice", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    for pattern in ["beta", "gamma"] {
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('p'));
    }

    assert_eq!(app.filters.len(), 2);
    assert_eq!(app.document.match_count(), 2);
}

#[test]
fn p_with_no_search_does_nothing() {
    let mut app = app_over_file("p_noop", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('p'));

    assert!(app.filters.is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tests::p_ 2>&1 | tail -30`
Expected: FAIL — `p_promotes_the_search_and_frees_the_slot` finds `filters.len()` is 0.

- [ ] **Step 3: Bind it**

In `handle_event`'s global `match key.code`, after the `Esc` arm:

```rust
                // Global rather than pane-scoped: the user has just searched
                // and should not have to go and find the filter pane to keep
                // the result.
                KeyCode::Char('p') if key.modifiers.is_empty() => {
                    if self.filters.promote_search() {
                        self.refresh_view();
                    }
                    return Ok(());
                }
```

Guarded on the return value so `p` over an empty slot does not pay for a full `refresh_view` — `evaluate` is O(lines × filters), and this key will be pressed speculatively.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS, all tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/lib.rs
git commit -m "feat(filter): p promotes the live search into the numbered set

Probe with /, keep with p, probe again. Global rather than pane-scoped:
the user has just searched and should not have to go find the filter
pane to keep the result."
```

---

### Task 10: The `/` row in the filter pane

**Files:**
- Modify: `src/widgets/filterlist.rs` (`row_text`, `render`, `preferred_width`, `handle_key`, `clamp_selection` call sites)
- Modify: `src/lib.rs` (`refresh_view`'s `clamp_selection`, `handle_filter_key`, `filter_pane_height`)
- Modify: `src/widgets/mod.rs` (`FilterCommand`)
- Test: `src/widgets/filterlist.rs` `mod tests`, `src/lib.rs` `mod tests`

**Interfaces:**
- Consumes: `FilterSet::search`, `FilterSet::row_count`, `FilterSet::search_set_enabled`, `FilterSet::clear_search`.
- Produces: `FilterCommand::ToggleSearch` and `FilterCommand::DeleteSearch` variants.

The pane's selection now addresses a list with one row that is not an element of `filters()`. That offset is where bugs will hide. The search row is **row 0**, matching its precedence in `verdict`.

- [ ] **Step 1: Write the failing tests**

Add to `src/widgets/filterlist.rs` `mod tests`:

```rust
#[test]
fn the_search_row_is_drawn_first_and_carries_a_slash() {
    let mut set = FilterSet::new();
    set.add("ERROR").expect("valid pattern");
    set.set_search("timeout").expect("valid pattern");

    assert_eq!(FilterList::row_text(&set, 0), "/[x] inc timeout");
    assert_eq!(FilterList::row_text(&set, 1), "1[x] inc ERROR");
}

#[test]
fn without_a_search_the_numbered_filters_start_at_row_zero() {
    let mut set = FilterSet::new();
    set.add("ERROR").expect("valid pattern");

    assert_eq!(FilterList::row_text(&set, 0), "1[x] inc ERROR");
}

/// The offset is the whole risk in this task: `space` on row 1 must toggle
/// filter 0, not filter 1.
#[test]
fn space_below_the_search_row_toggles_the_right_filter() {
    let mut set = FilterSet::new();
    set.add("ERROR").expect("valid pattern");
    set.set_search("timeout").expect("valid pattern");
    let mut list = FilterList::default();
    list.state.select(Some(1));

    let command = list.handle_key(KeyEvent::from(KeyCode::Char(' ')), set.row_count(), true);

    assert_eq!(command, Some(FilterCommand::Toggle(0)));
}

#[test]
fn space_on_the_search_row_toggles_the_search() {
    let mut set = FilterSet::new();
    set.set_search("timeout").expect("valid pattern");
    let mut list = FilterList::default();
    list.state.select(Some(0));

    let command = list.handle_key(KeyEvent::from(KeyCode::Char(' ')), set.row_count(), true);

    assert_eq!(command, Some(FilterCommand::ToggleSearch));
}

#[test]
fn d_on_the_search_row_deletes_the_search() {
    let mut list = FilterList::default();
    list.state.select(Some(0));

    let command = list.handle_key(KeyEvent::from(KeyCode::Char('d')), 1, true);

    assert_eq!(command, Some(FilterCommand::DeleteSearch));
}
```

Add to `src/lib.rs` `mod tests`:

```rust
/// End to end through the real key path: the pane's `d` on the search row
/// must remove the search and nothing else.
#[test]
fn deleting_the_search_row_leaves_the_numbered_filters_alone() {
    let mut app = app_over_file("pane_del_search", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('d'));

    assert!(app.filters.search().is_none());
    assert_eq!(app.filters.len(), 1, "a numbered filter was deleted instead");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib filterlist:: 2>&1 | tail -30`
Expected: FAIL to compile — `row_text` takes no third argument concept yet, `FilterCommand::ToggleSearch` does not exist, `handle_key` takes two arguments.

- [ ] **Step 3: Extend `FilterCommand`**

In `src/widgets/mod.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCommand {
    Toggle(usize),
    Delete(usize),
    /// The search row, which carries no index: the live search lives in its
    /// own slot on the `FilterSet`, not at a position in `filters`.
    ToggleSearch,
    DeleteSearch,
}
```

- [ ] **Step 4: Teach the pane about the offset**

In `src/widgets/filterlist.rs`, `row_text` becomes row-indexed rather than filter-indexed:

```rust
    /// One row of the pane: its number, whether it is on, which way it
    /// filters, and its pattern.
    ///
    /// Row 0 is the live search when one exists, marked `/` rather than a
    /// number — it has no number, because it does not occupy a position in
    /// `filters`. Its precedence here matches its precedence in `verdict`.
    ///
    /// The sense is spelled out because excluding filters carry no colour —
    /// nothing else on the row would distinguish them.
    fn row_text(filters: &FilterSet, row: usize) -> String {
        let (label, filter) = match (filters.search(), row) {
            (Some(search), 0) => ("/".to_string(), Some(search)),
            (search, row) => {
                let index = row - usize::from(search.is_some());
                ((index + 1).to_string(), filters.filters().get(index))
            }
        };
        let Some(filter) = filter else {
            return String::new();
        };
        let mark = if filter.enabled { 'x' } else { ' ' };
        let sense = match filter.sense {
            Sense::Include => "inc",
            Sense::Exclude => "exc",
        };
        format!("{}[{}] {} {}", label, mark, sense, filter.pattern.as_str())
    }
```

`handle_key` takes a flag saying whether row 0 is the search, and translates:

```rust
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        rows: usize,
        has_search: bool,
    ) -> Option<FilterCommand> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        // Row 0 is the live search when one exists, so every row below it
        // addresses a filter one lower. Doing this translation here, once,
        // keeps the offset out of `App` entirely.
        let target = |row: usize| -> Option<usize> {
            match (has_search, row) {
                (true, 0) => None,
                (true, row) => Some(row - 1),
                (false, row) => Some(row),
            }
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next(rows);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous(rows);
                None
            }
            KeyCode::Char(' ') => Some(match target(self.selected()?) {
                Some(index) => FilterCommand::Toggle(index),
                None => FilterCommand::ToggleSearch,
            }),
            KeyCode::Char('d') => Some(match target(self.selected()?) {
                Some(index) => FilterCommand::Delete(index),
                None => FilterCommand::DeleteSearch,
            }),
            _ => None,
        }
    }
```

`preferred_width` and `render` iterate `0..filters.row_count()` instead of `0..filters.len()`. `render`'s empty-set early return becomes `if filters.row_count() == 0`. In `render`'s per-row style lookup, replace the direct `&filters.filters()[index]` with a helper mirroring `row_text`'s destructuring, so the disabled-row dimming lands on the right row:

```rust
    /// The filter a pane row refers to, search row included.
    fn row_filter(filters: &FilterSet, row: usize) -> Option<&Filter> {
        match (filters.search(), row) {
            (Some(search), 0) => Some(search),
            (search, row) => filters.filters().get(row - usize::from(search.is_some())),
        }
    }
```

Add `Filter` to the `use crate::filter::{...}` list at the top of the file.

- [ ] **Step 5: Update the call sites in `lib.rs`**

`refresh_view`'s clamp and `filter_pane_height` both switch to `row_count`:

```rust
        let rows = self.filters.row_count();
        for widget in &mut self.widgets {
            if let AppWidget::FilterList(list) = widget {
                list.clamp_selection(rows);
            }
        }
```

```rust
    fn filter_pane_height(&self) -> u16 {
        self.filter_list()
            .map(|list| list.preferred_height(self.filters.row_count()))
            .unwrap_or(0)
    }
```

`handle_filter_key`'s dispatch:

```rust
        let rows = self.filters.row_count();
        let has_search = self.filters.search().is_some();
        let Some(AppWidget::FilterList(list)) = self.widgets.get_mut(self.active_widget) else {
            return;
        };
        let Some(command) = list.handle_key(key, rows, has_search) else {
            return;
        };
        match command {
            FilterCommand::Toggle(index) => {
                self.filters.toggle_enabled(index);
            }
            FilterCommand::Delete(index) => {
                self.filters.remove(index);
            }
            FilterCommand::ToggleSearch => {
                let enabled = self.filters.search().is_some_and(|search| search.enabled);
                self.filters.search_set_enabled(!enabled);
            }
            FilterCommand::DeleteSearch => self.filters.clear_search(),
        }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS, all tests.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/widgets/filterlist.rs src/widgets/mod.rs src/lib.rs
git commit -m "feat(filters): show the live search as a / row in the pane

Row 0, marked / rather than a number, because it holds no position in
the filters vector. space toggles it and d deletes it, the same as any
other row.

The row-to-filter translation lives in FilterList::handle_key so the
offset stays out of App entirely."
```

---

### Task 11: The span highlight follows the search's enabled flag

**Files:**
- Modify: `src/widgets/fileview.rs` (new `set_highlight`)
- Modify: `src/lib.rs` (`apply_view`)
- Test: `src/lib.rs` `mod tests`

**Interfaces:**
- Consumes: `FilterSet::search`.
- Produces: `FileView::set_highlight(&mut self, pattern: Option<&str>) -> Result<(), regex::Error>`

The `TextArea` still paints matched spans black-on-yellow. That painting must track the filter's enabled flag, or `!` leaves highlights glowing on a view where nothing is meant to be active — breaking the "one keystroke back to an unfiltered view" the README promises.

- [ ] **Step 1: Write the failing tests**

```rust
/// The buffer is rebuilt whenever the visible set changes, which clears the
/// textarea's search pattern. `apply_view` must put it back, or the
/// highlight vanishes the first time a filter changes.
#[test]
fn the_span_highlight_survives_a_rebuild() {
    let mut app = app_over_file("hl_rebuild", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('H'));

    assert!(app.file_view_highlight().is_some(), "the highlight was lost");
}

/// `!` promises one keystroke back to an unfiltered view. Yellow left glowing
/// on an inert view breaks that promise.
#[test]
fn disabling_everything_clears_the_span_highlight() {
    let mut app = app_over_file("hl_bang", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('!'));
    assert!(app.file_view_highlight().is_none(), "highlights outlived '!'");

    key(&mut app, KeyCode::Char('!'));
    assert!(app.file_view_highlight().is_some(), "the highlight did not come back");
}

#[test]
fn clearing_the_search_clears_the_span_highlight() {
    let mut app = app_over_file("hl_esc", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Esc);

    assert!(app.file_view_highlight().is_none());
}
```

Add this test-only accessor inside `mod tests` in `src/lib.rs`:

```rust
    impl App<'_> {
        /// The pattern the file view is currently highlighting, for tests.
        fn file_view_highlight(&self) -> Option<String> {
            self.widgets.iter().find_map(|widget| match widget {
                AppWidget::FileView(view) => view.highlight(),
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tests::the_span_highlight 2>&1 | tail -30`
Expected: FAIL to compile — `no method named 'highlight' found for FileView`.

- [ ] **Step 3: Give `FileView` the setter and a reader**

In `src/widgets/fileview.rs`:

```rust
    /// Set or clear the pattern whose spans the pane highlights.
    ///
    /// The search *filter* owns the pattern; this is only the painting of it.
    /// Passing `None` clears the highlight — `set_search_pattern` treats an
    /// empty query as "no pattern", so nothing is compiled on that path.
    ///
    /// Rebuilding the buffer clears this, so `App::apply_view` re-applies it
    /// on every pass rather than only when the pattern changes.
    pub fn set_highlight(&mut self, pattern: Option<&str>) -> Result<(), regex::Error> {
        self.textarea.set_search_pattern(pattern.unwrap_or(""))
    }

    /// The pattern currently highlighted, if any.
    pub fn highlight(&self) -> Option<String> {
        self.textarea
            .search_pattern()
            .map(|pattern| pattern.as_str().to_string())
    }
```

If `TextArea::search_pattern` is not public in the fork, add it next to `set_search_pattern` in `vendor/tui-textarea-2/src/textarea.rs` returning `Option<&Regex>` from `self.search.pat.as_ref()`, and record it in `PATCH.md` under the section added in Task 5.

- [ ] **Step 4: Apply it from `apply_view`**

In `src/lib.rs`'s `apply_view`, alongside the existing `view.set_line_numbers` / `set_line_styles` calls:

```rust
        // Re-applied on every pass, not only when the pattern changes: a
        // rebuild clears the textarea's copy, and the pattern tracks the
        // filter's *enabled* flag, so `!` and `space` both have to reach it.
        let highlight = self
            .filters
            .search()
            .filter(|search| search.enabled)
            .map(|search| search.pattern.as_str().to_string());
```

Compute this before the `iter_mut` borrow of `self.widgets`, then inside the block:

```rust
        // The pattern came from a `Regex` that already compiled, so this
        // cannot fail; ignored rather than propagated so `apply_view` keeps
        // its infallible signature.
        let _ = view.set_highlight(highlight.as_deref());
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS, all tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/lib.rs src/widgets/fileview.rs vendor/tui-textarea-2
git commit -m "fix(view): tie the span highlight to the search's enabled flag

'!' promises one keystroke back to an unfiltered view; yellow left
glowing on an inert view broke that. Re-applied on every apply_view
pass, since a rebuild clears the textarea's copy and the pattern has to
track space and '!' as well as Esc."
```

---

### Task 12: Documentation

**Files:**
- Modify: `README.md` (Features, Keybindings, Known Limitations)
- Modify: `src/lib.rs` (`status_text`'s no-filters message)
- Modify: `docs/specs/2026-08-21-search-as-a-filter-design.md` (status line)
- Test: `src/lib.rs` `mod tests`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing.

- [ ] **Step 1: Write the failing test**

The `Ctrl-H` guard from Task 4 makes `status_text`'s "nothing to show — no filters" unreachable and wrong: with nothing including, hiding now shows the file.

```rust
/// Task 4's guard made this state impossible: with no filters, hiding shows
/// the whole file, so the status line must not claim otherwise.
#[test]
fn hiding_with_no_filters_does_not_claim_the_file_is_empty() {
    let mut app = app_over_file("status_no_filters", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('H'));

    assert!(
        !app.status_text().contains("nothing to show"),
        "the status line still reports a blank pane that no longer happens"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib tests::hiding_with_no_filters_does_not_claim 2>&1 | tail -20`
Expected: FAIL — the status text still reads `▼ nothing to show — no filters`.

- [ ] **Step 3: Fix the status message**

In `status_text`, replace the `is_empty` early return:

```rust
        if self.filters.is_empty() && self.filters.search().is_none() {
            // With nothing including, hiding shows the file rather than
            // blanking it (see `Document::recompute_visible`), so there is
            // nothing to explain — the funnel alone would be a lie about a
            // state that no longer exists.
            return String::new();
        }
```

Delete any existing test asserting the old message.

- [ ] **Step 4: Update the README**

In **Features**, rewrite the "Dim or hide, on one keystroke" bullet, which currently says dimmed lines flip "from dimmed to gone":

```markdown
- **Dim or hide, on one keystroke** — `Ctrl-H` toggles unmatched lines between
  dimmed-but-present and removed. Toggling back returns you to the exact line
  you were on. Dimming marks unmatched lines whenever a numbered filter is
  enabled; a search on its own doesn't grey the file, since its hits already
  carry a highlight — but `Ctrl-H` still collapses to them.
```

Add a Features bullet for the merged model:

```markdown
- **Search is just a filter** — `/` defines one in a keystroke, `Esc` throws it
  away, and `p` keeps it: it joins the numbered set with its own colour and
  frees `/` for the next probe, so a filter set gets built by trying patterns
  rather than by retyping them. In between it behaves like any other filter —
  it survives loading another file, answers to `!`, loses to an exclude, and
  feeds `Ctrl-H`. `n` and `N` step between *interesting* lines, whether the
  filters or the search made them so.
```

In **Keybindings**, add `p` ("promote the live search to a numbered filter") and `Esc` ("clear the live search"), and remove `?`. Change the `n`/`N` entry to "next / previous interesting line".

In **Known Limitations**, add:

```markdown
- `n` and `N` are line-oriented: a line matching the search three times is one
  stop, not three. `recon` is a line-focused tool, and one rule for filter hits
  and search hits alike beats two.
- Only the live search highlights the matched text within a line. Numbered
  filters colour the whole line — the vendored `TextArea` holds one search
  pattern, so extending spans to every filter needs more work in the fork.
```

- [ ] **Step 5: Mark the spec accepted**

In `docs/specs/2026-08-21-search-as-a-filter-design.md`, change `**Status:** proposed` to `**Status:** implemented`.

- [ ] **Step 6: Run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test 2>&1 | tail -20`
Expected: PASS, all tests, no warnings.

- [ ] **Step 7: Commit**

```bash
git add README.md src/lib.rs docs/specs/2026-08-21-search-as-a-filter-design.md
git commit -m "docs: describe search as a filter, and stop over-claiming Ctrl-H

The README said dimmed lines flip 'from dimmed to gone', which no longer
holds when a search is the only thing active: nothing is dimmed and
Ctrl-H still hides plenty.

status_text's 'nothing to show - no filters' described a blank pane that
the #36 guard made impossible."
```

---

## Self-Review

**Spec coverage.** Every section of the spec maps to a task: the model and `Verdict::Searched` (1), the two predicates (2), promotion (3 and 9), the #36 guard (4), the keys `/` `n` `N` `Esc` `p` and retiring `?` (6–9), the filter pane row (10), the span highlight following the enabled flag (11), and the migration notes (12). The `set_cursor_position` fork change (5) is not named in the spec — it surfaced while checking that `CursorMove::Jump` could land `n`/`N` correctly, and is recorded in `PATCH.md`.

**Type consistency.** `set_search`/`clear_search`/`search`/`search_set_enabled`/`promote_search`/`row_count`/`any_including`/`any_numbered_including` are defined in Tasks 1–3 and used with those exact names and signatures in Tasks 4 and 6–11. `FilterList::handle_key` gains its third parameter in Task 10 and every call site changes in the same task. `FileView::set_cursor_row` (Task 6) wraps `TextArea::set_cursor_position` (Task 5).

**Known risks.**
- Task 10 carries the most risk: the pane's row-to-filter offset touches selection, `space`, `d`, width and height at once. Its tests target the offset directly rather than the rendering.
- Task 11 may need a second small fork addition (`search_pattern`) — Step 3 says so explicitly and points at `PATCH.md`.
- Task 6 deletes six `FileView` search tests. Their behaviour moves to `App`-level tests in Tasks 6 and 7; check nothing is left uncovered before committing.
