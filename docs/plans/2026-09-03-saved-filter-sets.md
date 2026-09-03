# Saved Filter Sets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Named groups of filters loaded from `~/.config/recon/filters.toml`, toggled as units from a two-level filter pane, with profiles, solo, reset, and a one-key save of the scratch set.

**Architecture:** `ActiveFilters` keeps its single flat `Vec<Filter>` (the *known list*) so `Verdict::Included(index)`, `Matcher` and the scan cache are untouched; each `Filter` gains a `set` index and a `name`, and a parallel `Vec<FilterSet>` carries per-set state. "Effective" (`filter.enabled && set.enabled`) replaces `filter.enabled` at every decision point. A new `src/filtersets.rs` owns the file schema, validation and path; the pane builds a `Vec<Row>` from the model and every key and every label goes through it.

**Tech Stack:** Rust 2024, `serde` + `toml` (already dependencies, `display` feature stays off), `toml_edit` (new, Part 5 only), ratatui 0.30, `regex`.

**Spec:** `docs/specs/2026-09-03-saved-filter-sets-design.md` — read it first; every task argues from it. Tracks #8; parts map to #128, #129, #130, #132, #131 in that order.

## Global Constraints

- **Prerequisites merged before Part 1 starts:** #39 (global AND mode) and #123 (`Predicate` enum). Task 1.0 reconciles names against what actually merged.
- CI runs, in this order: `cargo fmt -p recon --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --release`. Run all four in that order before every commit.
- `[lints.clippy] pedantic = warn` plus `-D warnings`: every pedantic lint is an error. `#![warn(unreachable_pub)]` is on: anything `pub` inside the private `widgets` module must be `pub(crate)`.
- **No environment variables in tests** (spec, *Testing*). Path resolution takes `xdg`/`home` as arguments. File fixtures go under `target/test-config/` and claim a unique name through the existing `CONFIG_FIXTURE_NAMES` mutex pattern.
- **`toml`'s `display` feature stays off.** Part 5 adds `toml_edit`; nothing ever calls `toml::to_string`.
- **A user with no `filters.toml` sees exactly today's behaviour.** Every existing test in `src/filter.rs`, `src/widgets/filterlist.rs` and `src/lib.rs` must pass unchanged unless a task says otherwise and says why.
- The known list is contiguous by set: scratch filters first, then each named set's filters in the set list's order. Every mutation preserves this.
- Each part is one PR on its own worktree, branched from `origin/main` after the previous part merged: `git worktree add .worktrees/<branch> -b <branch> origin/main`. **Known flake:** `title_shows_the_current_directory` fails on a long worktree path — keep branch names short.
- Commit messages: imperative subject with a `type(scope):` prefix, body explains *why*. Match `git log`.

---

## File structure

| File | Responsibility |
|---|---|
| `src/filter.rs` (modify) | `Origin`, `FilterSet`, `Filter.name` / `Filter.set`, effective-enabled, set toggling, profiles, solo, reset, `with_sets`, `adopt_scratch_as` |
| `src/filtersets.rs` (**create**, `pub mod`) | The `filters.toml` schema, `parse`, validation, `Error`, `path_from`, `load_file`; Part 5 adds `append_set` |
| `src/config.rs` (modify) | `config_home_from` split out of `config_path_from`; `parse_colour` split out of `non_empty_palette`; `Config.filter_sets` |
| `src/main.rs` (modify) | Load `filters.toml` after `config.toml`, before the terminal |
| `src/lib.rs` (modify) | `App::new` builds from sets; `handle_filter_key` arms; picker state and keys; `S` prompt; `save_scratch_as` |
| `src/widgets/mod.rs` (modify) | `FilterCommand` variants |
| `src/widgets/filterlist.rs` (modify) | `Row`, `rows()`, two-level rendering, keys `a`/`s`/`R` |
| `src/widgets/picker.rs` (**create**) | `ProfilePicker`: state, keys, overlay rendering |
| `src/help.rs` (modify) | `KEYMAP` rows for `a`, `s`, `R`, `S`, and the `Enter` row's text |
| `README.md` (modify) | *Saved filter sets* section; keybinding rows; the 64-pattern note |
| `Cargo.toml` (modify, Part 5) | `toml_edit` |

---

# Part 1 — #128: the model and the loader

Branch: `Fix-I128-filter-sets-model`. Deliverable: write `filters.toml` with `autoload = true`, get its filters at startup. The pane stays flat.

### Task 1.0: Reconcile with what #123 and #39 merged

**Files:** read only.

**Interfaces assumed by every later task** (verify each exists under this name; if #123 chose another, substitute it consistently in this plan before starting):
- `filter::Predicate` with variants `Regex(regex::Regex)` and `Definition(..)`.
- `Predicate::display(&self) -> String` — the pattern source for a regex.
- `Predicate::as_regex(&self) -> Option<&Regex>`.
- `Filter { predicate: Predicate, sense: Sense, enabled: bool, style: Style }`.
- `ActiveFilters::verdict` decides per line with an AND/OR mode from #39; the places it reads `filter.enabled` are what Task 1.3 changes.

- [ ] **Step 1: Read `src/filter.rs` top to bottom.** Note every occurrence of `filter.enabled` and `.enabled &&` outside tests; list them in a scratch file with line numbers. Task 1.3 replaces each one.
- [ ] **Step 2: Confirm the names above** with `grep -n 'enum Predicate\|fn display\|fn as_regex\|pub struct Filter' src/filter.rs`. If a name differs, edit this plan's code blocks now.
- [ ] **Step 3: Run the suite** to get a green baseline: `cargo test --workspace`. Expected: all pass.

### Task 1.1: `Origin`, `FilterSet`, and the scratch set that already exists

**Files:**
- Modify: `src/filter.rs` (types after `Filter`; `ActiveFilters` fields; `Default`; `new`; `with_palette`)

**Interfaces:**
- Produces:
  ```rust
  pub enum Origin { Scratch, File(PathBuf) }
  pub struct FilterSet { pub name: String, pub origin: Origin, pub priority: i32, pub autoload: bool, pub enabled: bool, pub profiles: BTreeMap<String, Vec<String>> }
  impl ActiveFilters { pub fn sets(&self) -> &[FilterSet]; }
  ```
  `Filter` gains `pub name: Option<String>` and `pub set: usize`. `Filter::display_name(&self) -> String`.

- [ ] **Step 1: Write the failing tests** in `src/filter.rs`'s `mod tests`, in a new section:

```rust
    // ---- sets ----------------------------------------------------------

    /// A fresh `ActiveFilters` already has one set: the scratch set, so that
    /// every filter lives in a set and nothing needs a "loose filter" case.
    #[test]
    fn a_new_set_has_only_the_scratch_set() {
        let set = ActiveFilters::new();
        assert_eq!(set.sets().len(), 1);
        assert_eq!(set.sets()[0].origin, Origin::Scratch);
        assert!(set.sets()[0].enabled);
        assert_eq!(set.sets()[0].name, "");
    }

    /// Typed filters land in the scratch set.
    #[test]
    fn added_filters_belong_to_the_scratch_set() {
        let set = set_with(&["foo", "bar"]);
        assert!(set.filters().iter().all(|filter| filter.set == 0));
    }

    /// With no name from a file, a filter is called by its pattern.
    #[test]
    fn a_filter_without_a_name_is_named_by_its_pattern() {
        let set = set_with(&["foo"]);
        assert_eq!(set.filters()[0].display_name(), "foo");
    }
```

- [ ] **Step 2: Run them** — `cargo test -p recon filter::tests::a_new_set_has_only_the_scratch_set` — Expected: compile error, `Origin` not found.

- [ ] **Step 3: Add the types and fields.** After `pub struct Filter { .. }`:

```rust
impl Filter {
    /// What the pane calls this filter: the file's `name`, else the pattern.
    ///
    /// Profiles refer to filters by this string, so it is also the filter's
    /// handle — see `filtersets::parse`, which rejects two filters in one set
    /// that would answer to the same one.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| self.predicate.display())
    }
}

/// Where a set came from, which decides what may be done to it.
///
/// `File` carries its path from day one so that "unload this file" (#46) is a
/// filter over origins later, not a new field then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The unnamed set typed filters land in. Always index 0, never in a file.
    Scratch,
    File(PathBuf),
}

/// A named group of filters, toggled as a unit.
///
/// The filters themselves are not here: they are in `ActiveFilters::filters`,
/// the flat known list, each carrying the index of its set. Keeping the list
/// flat is what leaves `Verdict::Included(index)`, `Matcher` and the scan
/// cache untouched by this feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterSet {
    pub name: String,
    pub origin: Origin,
    /// Pane position, lower first. The scratch set ignores it and is always first.
    pub priority: i32,
    /// Enabled at startup, and what `reset` returns the flag to.
    pub autoload: bool,
    pub enabled: bool,
    /// Named subsets of this set's filters, by `display_name`.
    pub profiles: BTreeMap<String, Vec<String>>,
}

impl FilterSet {
    fn scratch() -> Self {
        Self {
            name: String::new(),
            origin: Origin::Scratch,
            priority: i32::MIN,
            autoload: true,
            enabled: true,
            profiles: BTreeMap::new(),
        }
    }
}
```

Add `pub name: Option<String>` and `pub set: usize` to `Filter`. Add `use std::collections::BTreeMap; use std::path::PathBuf;` at the top. Add `sets: Vec<FilterSet>` to `ActiveFilters`; since `ActiveFilters` derives `Default` and the scratch set must exist, replace the derive with a hand-written `Default` that sets `sets: vec![FilterSet::scratch()]` and every other field to its default. Add:

```rust
    /// Every set, scratch first, then in pane order.
    #[must_use]
    pub fn sets(&self) -> &[FilterSet] {
        &self.sets
    }
```

Every place that constructs a `Filter` (`add`, `add_excluding`, `set_search`, and any in tests) gets `name: None, set: 0`.

- [ ] **Step 4: Run** `cargo test -p recon filter::` — Expected: PASS, including every pre-existing test.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter): Origin, FilterSet, and the scratch set every filter already lives in"`.

### Task 1.2: Loaded sets go into the known list, coloured and disabled

**Files:**
- Modify: `src/filter.rs` (`ActiveFilters::with_sets`; a private `insert_scratch`)

**Interfaces:**
- Consumes: `LoadedSet` / `LoadedFilter` — defined in Task 1.5; define them in `filter.rs` now, exactly as below, and Task 1.5 uses them. (They are the loader's *output* and the model's *input*, so the model owns them.)
- Produces:
  ```rust
  pub struct LoadedFilter { pub name: String, pub predicate: Predicate, pub sense: Sense, pub colour: Option<Color> }
  pub struct LoadedSet { pub name: String, pub path: PathBuf, pub priority: i32, pub autoload: bool, pub profiles: BTreeMap<String, Vec<String>>, pub filters: Vec<LoadedFilter> }
  impl ActiveFilters { pub fn with_sets(palette: Option<Vec<Color>>, sets: &[LoadedSet]) -> Self; }
  ```

- [ ] **Step 1: Write the failing tests**:

```rust
    fn loaded(name: &str, priority: i32, autoload: bool, patterns: &[&str]) -> LoadedSet {
        LoadedSet {
            name: name.to_string(),
            path: PathBuf::from("test/filters.toml"),
            priority,
            autoload,
            profiles: BTreeMap::new(),
            filters: patterns
                .iter()
                .map(|p| LoadedFilter {
                    name: (*p).to_string(),
                    predicate: Predicate::Regex(Regex::new(p).expect("valid")),
                    sense: Sense::Include,
                    colour: None,
                })
                .collect(),
        }
    }

    /// Loaded filters follow the scratch set in the known list, contiguous
    /// by set, and start disabled.
    #[test]
    fn loaded_sets_follow_scratch_in_the_known_list() {
        let set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);
        assert_eq!(set.sets().len(), 2);
        assert_eq!(set.sets()[1].name, "a");
        assert_eq!(set.filters().len(), 2);
        assert!(set.filters().iter().all(|f| f.set == 1 && !f.enabled));
    }

    /// A loaded filter's colour is its known-list position in the palette —
    /// the same rule `add` uses — unless the file named one.
    #[test]
    fn loaded_filters_are_coloured_by_position_or_by_the_file() {
        let mut one = loaded("a", 50, false, &["x", "y"]);
        one.filters[1].colour = Some(Color::Red);
        let set = ActiveFilters::with_sets(None, &[one]);
        assert_eq!(set.filters()[0].style, Style::default().fg(DEFAULT_PALETTE[0]));
        assert_eq!(set.filters()[1].style, Style::default().fg(Color::Red));
    }

    /// The set with `autoload` starts enabled; the other does not.
    #[test]
    fn autoload_sets_start_enabled() {
        let set = ActiveFilters::with_sets(
            None,
            &[loaded("a", 50, true, &["x"]), loaded("b", 50, false, &["y"])],
        );
        assert!(set.sets()[1].enabled);
        assert!(!set.sets()[2].enabled);
    }

    /// A filter typed after loading is inserted at the end of the scratch
    /// range, ahead of every file filter, and takes the next colour after
    /// every known filter so it does not repeat a file filter's colour.
    #[test]
    fn a_typed_filter_lands_after_scratch_and_before_file_filters() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x", "y"])]);
        set.add("typed").expect("valid");
        assert_eq!(set.filters()[0].display_name(), "typed");
        assert_eq!(set.filters()[0].set, 0);
        assert_eq!(set.filters()[1].set, 1);
        assert_eq!(set.filters()[0].style, Style::default().fg(DEFAULT_PALETTE[2]));
    }
```

- [ ] **Step 2: Run** `cargo test -p recon filter::tests::loaded_sets_follow_scratch_in_the_known_list` — Expected: compile error, `LoadedSet` not found.

- [ ] **Step 3: Implement.** Add the two `Loaded*` structs (both `#[derive(Debug, Clone)]`) beside `FilterSet`. Then:

```rust
impl ActiveFilters {
    /// Build the startup set: the scratch set, then `sets` in the order given
    /// (the loader has already sorted them by priority and name), every file
    /// filter disabled, then each `autoload` set enabled — which applies its
    /// `default` profile, if it has one.
    #[must_use]
    pub fn with_sets(palette: Option<Vec<Color>>, sets: &[LoadedSet]) -> Self {
        let mut this = match palette {
            Some(palette) => Self::with_palette(palette),
            None => Self::new(),
        };
        for loaded in sets {
            let index = this.sets.len();
            this.sets.push(FilterSet {
                name: loaded.name.clone(),
                origin: Origin::File(loaded.path.clone()),
                priority: loaded.priority,
                autoload: loaded.autoload,
                enabled: false,
                profiles: loaded.profiles.clone(),
            });
            for filter in &loaded.filters {
                let style = match filter.colour {
                    Some(colour) => Style::default().fg(colour),
                    None => this.next_style(),
                };
                this.filters.push(Filter {
                    predicate: filter.predicate.clone(),
                    sense: filter.sense,
                    enabled: false,
                    style,
                    name: Some(filter.name.clone()),
                    set: index,
                });
            }
        }
        this.recompile();
        for index in 1..this.sets.len() {
            if this.sets[index].autoload {
                this.set_enabled_set(index, true);
            }
        }
        this
    }

    /// Where the scratch set's filters end: the first filter not in set 0.
    fn scratch_end(&self) -> usize {
        self.filters.iter().position(|f| f.set != 0).unwrap_or(self.filters.len())
    }

    /// Put a typed filter at the end of the scratch range.
    ///
    /// An insert, not a push: file filters follow the scratch set in the
    /// known list, and the list must stay contiguous by set. Every cached
    /// `Verdict::Included` is a position, so callers re-evaluate — which they
    /// already do, since a push had the same effect on the compiled set.
    fn insert_scratch(&mut self, filter: Filter) {
        let at = self.scratch_end();
        self.filters.insert(at, filter);
        self.recompile();
        self.forget_capture();
    }
}
```

`set_enabled_set` does not exist yet; for this task write it as `self.sets[index].enabled = enabled;` — Task 1.4 finishes it. Change `add`, `add_excluding` and `promote_search` to build the `Filter` and call `self.insert_scratch(filter)` instead of `self.filters.push(..); self.recompile(); self.forget_capture();`. `next_style` keeps using `self.filters.len()` — it is the known-list count, which is the spec's rule.

- [ ] **Step 4: Run** `cargo test -p recon filter::` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter): with_sets loads file sets behind the scratch set, coloured by position or by the file"`.

### Task 1.3: Effective enabled — a filter counts only when its set does

**Files:**
- Modify: `src/filter.rs` (`verdict`, `verdict_by_scanning`, `matcher`, `any_excluding`, `any_numbered_including`, plus whatever #39 added that reads `filter.enabled` to decide a line)

**Interfaces:**
- Produces: `fn effective(&self, index: usize) -> bool` (private).

- [ ] **Step 1: Write the failing tests**:

```rust
    /// The rule: a filter takes effect when it is enabled *and* its set is.
    #[test]
    fn a_filter_in_a_disabled_set_matches_nothing() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["foo"])]);
        set.set_enabled(0, true);
        assert_eq!(set.verdict("foo"), Verdict::Included(0));
        set.sets[1].enabled = false;
        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
        assert!(set.filters()[0].enabled, "the filter's own flag is untouched");
    }

    /// The navigator's masks follow the same rule.
    #[test]
    fn the_matcher_ignores_filters_in_disabled_sets() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["foo"])]);
        set.set_enabled(0, true);
        assert!(set.matcher().is_some());
        set.sets[1].enabled = false;
        assert!(set.matcher().is_none(), "nothing selects, so there is no scan to run");
    }

    /// Dimming follows it too: an enabled filter in a disabled set does not
    /// grey the rest of the file.
    #[test]
    fn dimming_needs_an_effective_including_filter() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["foo"])]);
        set.set_enabled(0, true);
        set.sets[1].enabled = false;
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }
```

- [ ] **Step 2: Run** `cargo test -p recon filter::tests::a_filter_in_a_disabled_set_matches_nothing` — Expected: FAIL, `Included(0)` where `Unmatched` expected.

- [ ] **Step 3: Implement.** Add:

```rust
    /// Whether the filter at `index` currently takes effect: on, and in a set
    /// that is on. This is the one place the two flags meet; everything that
    /// decides a line, a file, or a dim reads this and never `enabled` alone.
    fn effective(&self, index: usize) -> bool {
        let filter = &self.filters[index];
        filter.enabled && self.sets[filter.set].enabled
    }
```

Then, in each function from Task 1.0's list that decides a line, file or style — `verdict`, `verdict_by_scanning`, `matcher`, `any_excluding`, `any_numbered_including`, and #39's AND-mode branch — replace `filter.enabled` with `self.effective(index)` (enumerate where the loop does not already). Leave `set_all_enabled`, `disable_all_remembering`, `restore_remembered`, `enabled_flags`, `apply_enabled_flags`, `any_enabled`, `toggle_enabled`, `set_enabled` alone: they are the flag-level operations the spec keeps flag-level. The search's own `enabled` is unchanged; it is not in a set.

- [ ] **Step 4: Run** `cargo test -p recon` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter): a filter takes effect only when its set does"`.

### Task 1.4: Set toggling and profiles

**Files:**
- Modify: `src/filter.rs`

**Interfaces:**
- Produces:
  ```rust
  pub fn set_enabled_set(&mut self, set: usize, enabled: bool) -> bool   // false if no such set or set 0
  pub fn toggle_set(&mut self, set: usize) -> Option<bool>
  pub fn apply_profile(&mut self, set: usize, profile: &str) -> bool
  pub fn filters_in(&self, set: usize) -> impl Iterator<Item = (usize, &Filter)>
  ```

- [ ] **Step 1: Write the failing tests**:

```rust
    fn with_default_profile() -> ActiveFilters {
        let mut a = loaded("a", 50, false, &["x", "y", "z"]);
        a.profiles.insert("default".into(), vec!["x".into(), "z".into()]);
        a.profiles.insert("loud".into(), vec!["x".into(), "y".into(), "z".into()]);
        ActiveFilters::with_sets(None, &[a])
    }

    fn flags(set: &ActiveFilters, of: usize) -> Vec<bool> {
        set.filters_in(of).map(|(_, f)| f.enabled).collect()
    }

    /// Enabling a set with a `default` profile applies it.
    #[test]
    fn enabling_a_set_applies_its_default_profile() {
        let mut set = with_default_profile();
        assert!(set.set_enabled_set(1, true));
        assert_eq!(flags(&set, 1), vec![true, false, true]);
    }

    /// Without `default`, enabling keeps whatever flags the filters had.
    #[test]
    fn enabling_a_set_without_default_keeps_the_flags() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);
        set.set_enabled(1, true);
        set.set_enabled_set(1, true);
        assert_eq!(flags(&set, 1), vec![false, true]);
        set.set_enabled_set(1, false);
        assert_eq!(flags(&set, 1), vec![false, true], "disabling touches no flag");
    }

    /// A profile enables exactly its members and disables the rest of the set.
    #[test]
    fn a_profile_is_exact() {
        let mut set = with_default_profile();
        set.set_enabled_set(1, true);
        assert!(set.apply_profile(1, "loud"));
        assert_eq!(flags(&set, 1), vec![true, true, true]);
        assert!(set.apply_profile(1, "default"));
        assert_eq!(flags(&set, 1), vec![true, false, true]);
        assert!(!set.apply_profile(1, "nope"));
    }

    /// The scratch set cannot be toggled through this path.
    #[test]
    fn the_scratch_set_is_not_toggleable() {
        let mut set = set_with(&["foo"]);
        assert!(!set.set_enabled_set(0, false));
        assert!(set.sets()[0].enabled);
    }
```

- [ ] **Step 2: Run** — Expected: compile error, `filters_in` not found.

- [ ] **Step 3: Implement**:

```rust
    /// The filters in `set`, with their known-list indices.
    pub fn filters_in(&self, set: usize) -> impl Iterator<Item = (usize, &Filter)> {
        self.filters
            .iter()
            .enumerate()
            .filter(move |(_, filter)| filter.set == set)
    }

    /// Enable or disable a named set. Enabling applies the set's `default`
    /// profile if it has one; otherwise, and on disable, no filter flag moves.
    ///
    /// Returns `false` for the scratch set, which is never toggled by hand,
    /// and for an index that names no set.
    pub fn set_enabled_set(&mut self, set: usize, enabled: bool) -> bool {
        if set == 0 || set >= self.sets.len() {
            return false;
        }
        self.sets[set].enabled = enabled;
        if enabled && self.sets[set].profiles.contains_key("default") {
            self.apply_profile(set, "default");
        }
        true
    }

    pub fn toggle_set(&mut self, set: usize) -> Option<bool> {
        let now = !self.sets.get(set)?.enabled;
        self.set_enabled_set(set, now).then_some(now)
    }

    /// Enable exactly the profile's members within `set` and disable the
    /// set's other filters. An action, not a binding: nothing remembers that
    /// a profile was applied.
    pub fn apply_profile(&mut self, set: usize, profile: &str) -> bool {
        let Some(members) = self.sets.get(set).and_then(|s| s.profiles.get(profile)).cloned() else {
            return false;
        };
        for filter in self.filters.iter_mut().filter(|f| f.set == set) {
            filter.enabled = members.contains(&filter.display_name());
        }
        true
    }
```

Replace Task 1.2's stub body of `set_enabled_set` with this one.

- [ ] **Step 4: Run** `cargo test -p recon filter::` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter): sets toggle as a unit; profiles set a set's flags exactly"`.

### Task 1.5: `filtersets::parse` — the schema, and every way a file is wrong

**Files:**
- Create: `src/filtersets.rs`
- Modify: `src/lib.rs` (`pub mod filtersets;` beside `pub mod filter;`)
- Modify: `src/config.rs` (extract `pub(crate) fn parse_colour(spelling: &str) -> Result<Color, String>` from `non_empty_palette`; `non_empty_palette` calls it)

**Interfaces:**
- Produces:
  ```rust
  pub enum Error { Read { path, source: io::Error }, Parse { path, source: toml::de::Error }, Invalid { path, set: String, filter: Option<String>, message: String } }
  pub fn parse(text: &str, path: &Path) -> Result<Vec<LoadedSet>, Error>
  ```
  Output sorted by `(priority, name)`.

- [ ] **Step 1: Extract `parse_colour`** in `config.rs`. Move the `Color::from_str(..).map_err(|_| format!("{spelling:?} is not a colour. Use a name ({COLOUR_NAMES}), a hex triple (#RRGGBB), or a 256-colour index as a string (\"0-255\", e.g. \"220\")"))` body into `pub(crate) fn parse_colour(spelling: &str) -> Result<Color, String>`, and have `non_empty_palette` map over it with `D::Error::custom`. Run `cargo test -p recon config::` — Expected: PASS, unchanged.

- [ ] **Step 2: Write the failing tests** in the new file:

```rust
//! `filters.toml`: the schema, its validation, and where it lives.
//!
//! See docs/specs/2026-09-03-saved-filter-sets-design.md, *The file*.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Sense;

    fn parsed(text: &str) -> Vec<LoadedSet> {
        parse(text, Path::new("t/filters.toml")).expect("valid file")
    }

    fn rejected(text: &str) -> String {
        parse(text, Path::new("t/filters.toml"))
            .expect_err("invalid file")
            .to_string()
    }

    const MINIMAL: &str = r#"
[sets.a]
[[sets.a.filters]]
pattern = 'foo'
"#;

    #[test]
    fn an_empty_file_has_no_sets() {
        assert!(parsed("").is_empty());
    }

    #[test]
    fn a_minimal_set_takes_every_default() {
        let sets = parsed(MINIMAL);
        assert_eq!(sets.len(), 1);
        let a = &sets[0];
        assert_eq!(a.name, "a");
        assert_eq!(a.priority, 50);
        assert!(!a.autoload);
        assert!(a.profiles.is_empty());
        assert_eq!(a.filters[0].name, "foo", "name falls back to the pattern");
        assert_eq!(a.filters[0].sense, Sense::Include);
        assert_eq!(a.filters[0].colour, None);
    }

    #[test]
    fn every_key_is_read() {
        let sets = parsed(
            r#"
[sets.w]
priority = 10
autoload = true
mode = "or"
[sets.w.profiles]
default = ["assoc"]
[[sets.w.filters]]
name = "assoc"
pattern = 'associated'
colour = "red"
[[sets.w.filters]]
pattern = 'retry'
sense = "exclude"
"#,
        );
        let w = &sets[0];
        assert_eq!((w.priority, w.autoload), (10, true));
        assert_eq!(w.profiles["default"], vec!["assoc".to_string()]);
        assert_eq!(w.filters[0].colour, Some(ratatui::style::Color::Red));
        assert_eq!(w.filters[1].sense, Sense::Exclude);
        assert_eq!(w.filters[1].name, "retry");
    }

    #[test]
    fn sets_sort_by_priority_then_name() {
        let sets = parsed(
            r#"
[sets.zebra]
priority = 10
[[sets.zebra.filters]]
pattern = 'z'
[sets.beta]
[[sets.beta.filters]]
pattern = 'b'
[sets.alpha]
[[sets.alpha.filters]]
pattern = 'a'
"#,
        );
        let names: Vec<&str> = sets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["zebra", "alpha", "beta"]);
    }

    #[test]
    fn a_bad_pattern_names_the_file_set_and_filter() {
        let message = rejected("[sets.a]\n[[sets.a.filters]]\npattern = '('\n");
        assert!(message.contains("t/filters.toml"), "{message}");
        assert!(message.contains("[sets.a]"), "{message}");
        assert!(message.contains("'('"), "{message}");
    }

    #[test]
    fn a_bad_colour_explains_the_forms() {
        let message = rejected("[sets.a]\n[[sets.a.filters]]\npattern = 'x'\ncolour = 'reddish'\n");
        assert!(message.contains("hex triple"), "{message}");
    }

    #[test]
    fn duplicate_names_are_rejected_after_the_fallback() {
        let message = rejected(
            "[sets.a]\n[[sets.a.filters]]\npattern = 'x'\n[[sets.a.filters]]\nname = 'x'\npattern = 'y'\n",
        );
        assert!(message.contains("two filters named \"x\""), "{message}");
    }

    #[test]
    fn a_profile_must_name_real_filters() {
        let message = rejected(
            "[sets.a]\n[sets.a.profiles]\ndefault = ['nope']\n[[sets.a.filters]]\npattern = 'x'\n",
        );
        assert!(message.contains("profile \"default\""), "{message}");
        assert!(message.contains("\"nope\""), "{message}");
    }

    #[test]
    fn a_set_needs_a_filter() {
        assert!(rejected("[sets.a]\n").contains("no filters"));
    }

    #[test]
    fn mode_accepts_only_or() {
        assert!(parse("[sets.a]\nmode = 'or'\n[[sets.a.filters]]\npattern = 'x'\n", Path::new("t")).is_ok());
        assert!(rejected("[sets.a]\nmode = 'and'\n[[sets.a.filters]]\npattern = 'x'\n").contains("mode"));
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        assert!(rejected("[sets.a]\ncolor = 'red'\n[[sets.a.filters]]\npattern = 'x'\n").contains("color"));
    }

    #[test]
    fn an_empty_set_name_is_rejected() {
        assert!(rejected("[sets.\"\"]\n[[sets.\"\".filters]]\npattern = 'x'\n").contains("empty"));
    }
}
```

- [ ] **Step 3: Run** `cargo test -p recon filtersets::` — Expected: compile error, `parse` not found.

- [ ] **Step 4: Implement** above the tests:

```rust
use crate::config::parse_colour;
use crate::filter::{LoadedFilter, LoadedSet, Predicate, Sense};
use ratatui::style::Color;
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Position in the pane when the file does not say.
pub const DEFAULT_PRIORITY: i32 = 50;

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
struct FileSchema {
    #[serde(default)]
    sets: BTreeMap<String, SetSchema>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
struct SetSchema {
    priority: Option<i32>,
    autoload: Option<bool>,
    /// Reserved for #40. Only `"or"` is accepted, and it means nothing.
    mode: Option<String>,
    #[serde(default)]
    profiles: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    filters: Vec<FilterSchema>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct FilterSchema {
    pattern: String,
    name: Option<String>,
    sense: Option<SenseSchema>,
    colour: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum SenseSchema {
    Include,
    Context,
    Exclude,
}

impl From<SenseSchema> for Sense {
    fn from(sense: SenseSchema) -> Self {
        match sense {
            SenseSchema::Include => Self::Include,
            SenseSchema::Context => Self::Context,
            SenseSchema::Exclude => Self::Exclude,
        }
    }
}

/// Why `filters.toml` could not be loaded. Every variant carries the path:
/// with `$XDG_CONFIG_HOME` in play, *which* file is the first question.
#[derive(Debug)]
pub enum Error {
    Read { path: PathBuf, source: std::io::Error },
    Parse { path: PathBuf, source: toml::de::Error },
    /// Parsed, but says something recon cannot use. `filter` is the pattern
    /// or name of the offending filter when there is one.
    Invalid { path: PathBuf, set: String, filter: Option<String>, message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "could not read filter sets file {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(f, "invalid filter sets file {}\n{source}", path.display())
            }
            Self::Invalid { path, set, filter, message } => {
                write!(f, "invalid filter sets file {}: [sets.{set}]", path.display())?;
                if let Some(filter) = filter {
                    write!(f, " filter '{filter}'")?;
                }
                write!(f, ": {message}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Parse and validate one file's text. Pure: `path` is only for messages.
///
/// The result is sorted by `(priority, name)`, which is the pane's order.
pub fn parse(text: &str, path: &Path) -> Result<Vec<LoadedSet>, Error> {
    let file: FileSchema = toml::from_str(text).map_err(|source| Error::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let invalid = |set: &str, filter: Option<&str>, message: String| Error::Invalid {
        path: path.to_path_buf(),
        set: set.to_string(),
        filter: filter.map(str::to_string),
        message,
    };

    let mut sets = Vec::with_capacity(file.sets.len());
    for (name, schema) in file.sets {
        if name.is_empty() {
            return Err(invalid(&name, None, "a set's name cannot be empty".into()));
        }
        if let Some(mode) = &schema.mode
            && mode != "or"
        {
            return Err(invalid(&name, None, format!("mode {mode:?} is not supported; only \"or\" is")));
        }
        if schema.filters.is_empty() {
            return Err(invalid(&name, None, "a set with no filters; add at least one [[sets.<name>.filters]]".into()));
        }

        let mut filters = Vec::with_capacity(schema.filters.len());
        for entry in schema.filters {
            let regex = Regex::new(&entry.pattern)
                .map_err(|err| invalid(&name, Some(&entry.pattern), err.to_string()))?;
            let colour = entry
                .colour
                .as_deref()
                .map(parse_colour)
                .transpose()
                .map_err(|message| invalid(&name, Some(&entry.pattern), message))?;
            let display = entry.name.unwrap_or_else(|| entry.pattern.clone());
            if filters.iter().any(|f: &LoadedFilter| f.name == display) {
                return Err(invalid(&name, Some(&display), format!("two filters named {display:?}; give one a distinct `name`")));
            }
            filters.push(LoadedFilter {
                name: display,
                predicate: Predicate::Regex(regex),
                sense: entry.sense.map_or(Sense::Include, Into::into),
                colour,
            });
        }

        for (profile, members) in &schema.profiles {
            if let Some(missing) = members.iter().find(|m| !filters.iter().any(|f| &f.name == *m)) {
                return Err(invalid(&name, None, format!("profile {profile:?} names {missing:?}, which is not a filter in this set")));
            }
        }

        sets.push(LoadedSet {
            name,
            path: path.to_path_buf(),
            priority: schema.priority.unwrap_or(DEFAULT_PRIORITY),
            autoload: schema.autoload.unwrap_or(false),
            profiles: schema.profiles,
            filters,
        });
    }
    sets.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.name.cmp(&b.name)));
    Ok(sets)
}
```

`LoadedFilter` needs `PartialEq` on `Sense` (it has it) and the test compares `colour`; `Predicate` must be `Clone` (from #123; add the derive if it is not).

- [ ] **Step 5: Run** `cargo test -p recon filtersets::` — Expected: PASS. Then `cargo clippy --workspace --all-targets -- -D warnings` — fix anything pedantic (likely `needless_pass_by_value` on the closure args; use references).
- [ ] **Step 6: Commit** — `git commit -am "feat(filtersets): parse and validate filters.toml"`.

### Task 1.6: Where the file lives, and loading it before the terminal

**Files:**
- Modify: `src/config.rs` (`config_path_from` → `pub(crate) fn config_home_from(xdg, home) -> Option<PathBuf>` plus a thin `config_path_from` over it; `Config.filter_sets`)
- Modify: `src/filtersets.rs` (`path_from`, `path`, `load_from`, `load_file`)
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `filtersets::path_from(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf>`; `filtersets::load_file() -> Result<Vec<LoadedSet>, Error>`; `Config.filter_sets: Vec<LoadedSet>` (`#[arg(skip)]`).

- [ ] **Step 1: Write the failing tests** in `filtersets.rs`:

```rust
    #[test]
    fn the_path_sits_beside_config_toml() {
        assert_eq!(
            path_from(Some("/x"), Some("/h")),
            Some(PathBuf::from("/x/recon/filters.toml"))
        );
        assert_eq!(
            path_from(None, Some("/h")),
            Some(PathBuf::from("/h/.config/recon/filters.toml"))
        );
        assert_eq!(path_from(Some("relative"), None), None);
    }

    #[test]
    fn a_missing_file_is_no_sets() {
        let sets = load_from(Path::new("target/test-config/no-such-filters.toml")).expect("not an error");
        assert!(sets.is_empty());
    }

    #[test]
    fn a_directory_in_the_files_place_is_an_error() {
        let dir = Path::new("target/test-config/filters-as-a-dir.toml");
        std::fs::create_dir_all(dir).expect("mkdir");
        assert!(matches!(load_from(dir), Err(Error::Read { .. })));
    }
```

- [ ] **Step 2: Run** — Expected: compile error, `path_from` not found.

- [ ] **Step 3: Implement.** In `config.rs`, split:

```rust
/// The directory recon's files live in, from the two variables that decide
/// it — or `None` when neither names a home. Shared with `filtersets`.
pub(crate) fn config_home_from(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let config_home = /* the existing body of config_path_from up to the `?` */;
    Some(config_home.join(CONFIG_DIR))
}

fn config_path_from(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    Some(config_home_from(xdg_config_home, home)?.join(CONFIG_FILE))
}
```

Add to `Config`:

```rust
    /// Sets read from `filters.toml`, in pane order. Filled by `main` after
    /// `load`; `#[arg(skip)]` because no flag names the file (#46 would).
    #[arg(skip)]
    pub filter_sets: Vec<crate::filter::LoadedSet>,
```

and `filter_sets: Vec::new()` in `impl Default for Config`. In `filtersets.rs`:

```rust
const FILE: &str = "filters.toml";

/// Where `filters.toml` lives: beside `config.toml`, by the same rules.
#[must_use]
pub fn path_from(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    Some(crate::config::config_home_from(xdg_config_home, home)?.join(FILE))
}

#[must_use]
pub fn path() -> Option<PathBuf> {
    path_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Read and parse one file. A file that is not there is no sets.
fn load_from(path: &Path) -> Result<Vec<LoadedSet>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::Read { path: path.to_path_buf(), source }),
    };
    parse(&text, path)
}

/// The file layer. Call before the terminal is initialised: an error here
/// refuses to start, for the reason `Config::load` gives.
pub fn load_file() -> Result<Vec<LoadedSet>, Error> {
    let Some(path) = path() else {
        log::debug!("no config home; no filters.toml read");
        return Ok(Vec::new());
    };
    if path.exists() {
        log::debug!("reading filter sets from {}", path.display());
    } else {
        log::debug!("no filter sets file at {}", path.display());
    }
    load_from(&path)
}
```

In `main.rs`, directly after `let config = Config::load()?;`:

```rust
    let mut config = config;
    config.filter_sets = recon::filtersets::load_file()?;
```

(or make the original binding `mut`). In `App::new`, replace the `filters:` initialiser with `ActiveFilters::with_sets(config.filter_palette.clone(), &config.filter_sets)`.

- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS. Then run the binary against a real file to see it start: write `target/test-config/manual.toml` with the spec's example, and `XDG_CONFIG_HOME=$PWD/target/manual-home cargo run -- src` after copying it to `target/manual-home/recon/filters.toml`. Expected: the pane lists the set's filters, prefixed (Task 1.7 adds the prefix; for now they show by name).
- [ ] **Step 5: Commit** — `git commit -am "feat(app): load filters.toml beside config.toml, before the terminal"`.

### Task 1.7: Startup through `App::new`, and the flat pane prefix

**Files:**
- Modify: `src/lib.rs` (tests)
- Modify: `src/widgets/filterlist.rs` (`row_text`)

- [ ] **Step 1: Write the failing tests.** In `src/lib.rs`'s tests, near the other `App::new(&Config::default())` tests:

```rust
    fn config_with_sets(sets: Vec<filter::LoadedSet>) -> Config {
        Config {
            filter_sets: sets,
            ..Config::default()
        }
    }

    /// An `autoload` set's `default` profile is what the app starts with.
    #[test]
    fn an_autoload_set_is_live_at_startup() {
        let mut set = filter::LoadedSet {
            name: "a".into(),
            path: "t".into(),
            priority: 50,
            autoload: true,
            profiles: std::collections::BTreeMap::new(),
            filters: vec![filter::LoadedFilter {
                name: "x".into(),
                predicate: filter::Predicate::Regex(regex::Regex::new("x").unwrap()),
                sense: filter::Sense::Include,
                colour: None,
            }],
        };
        set.profiles.insert("default".into(), vec!["x".into()]);
        let app = App::new(&config_with_sets(vec![set]));
        assert!(app.filters.sets()[1].enabled);
        assert!(app.filters.filters()[0].enabled);
        assert!(app.filters.matcher().is_some(), "the navigator has something to scan for");
    }
```

In `filterlist.rs` tests:

```rust
    /// A file filter's row carries its set's name; a scratch filter's does not.
    #[test]
    fn file_filters_are_prefixed_by_their_set() {
        let loaded = crate::filter::LoadedSet {
            name: "w".into(),
            path: "t".into(),
            priority: 50,
            autoload: true,
            profiles: std::collections::BTreeMap::new(),
            filters: vec![crate::filter::LoadedFilter {
                name: "assoc".into(),
                predicate: crate::filter::Predicate::Regex(regex::Regex::new("associated").unwrap()),
                sense: Sense::Include,
                colour: None,
            }],
        };
        let filters = ActiveFilters::with_sets(None, &[loaded]);
        let mut list = FilterList::default();
        let rows = rendered(&mut list, &filters, 30);
        assert!(rows[1].contains("w/assoc"), "{rows:?}");
    }
```

- [ ] **Step 2: Run** — Expected: the `lib.rs` test passes already (Task 1.6 wired it); the pane test FAILS, no prefix.

- [ ] **Step 3: Implement** in `row_text`: replace `filter.pattern.as_str()` (or `filter.predicate.display()` after #123) with

```rust
        let text = match filter.set {
            0 => filter.display_name(),
            set => format!("{}/{}", filters.sets()[set].name, filter.display_name()),
        };
```

`preferred_width` measures `row_text`, so it follows.

- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter pane): show file filters by name, prefixed by their set"`.

### Task 1.8: README and the spec pointer

**Files:**
- Modify: `README.md` (new `### Saved filter sets` under the filter section, before `#### The filter palette`; the 64-pattern note in *Known Limitations*)

- [ ] **Step 1: Write the section.** Content: the spec's file example verbatim; one paragraph each on *what a set is*, *enabling makes filters available*, *profiles*, *`autoload`*, *`priority`*, *`colour`*; the location line; a sentence that recon never writes this file (Part 5 revises it). Add to *Known Limitations*: "The navigator's file matching covers at most 64 patterns across every loaded set; above that it switches off."
- [ ] **Step 2: Run** `cargo test -p recon config::tests::readme_usage_block_matches_the_real_help` — Expected: PASS (no flag changed).
- [ ] **Step 3: Commit** — `git commit -am "docs: saved filter sets — the file, sets, profiles, priority"`.
- [ ] **Step 4: Open the PR** per `/work-issue`, referencing #128.

---

# Part 2 — #129: the two-level pane

Branch: `Fix-I129-filter-pane-sets`.

### Task 2.1: `Row` and `rows()` — one description of the pane

**Files:**
- Modify: `src/widgets/filterlist.rs` (replace `filter_index_for_row`; add `Row`, `rows`)
- Modify: `src/filter.rs` (delete `row_count`)
- Modify: `src/lib.rs` (every `row_count()` call → `filterlist::rows(&self.filters).len()`)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub(crate) enum Row { Search, Header(usize), Filter(usize) }
  pub(crate) fn rows(filters: &ActiveFilters) -> Vec<Row>
  ```

- [ ] **Step 1: Write the failing tests** in `filterlist.rs`:

```rust
    fn two_sets(a_enabled: bool, b_enabled: bool) -> ActiveFilters {
        let a = loaded("a", 10, a_enabled, &["x", "y"]);
        let b = loaded("b", 20, b_enabled, &["z"]);
        let mut filters = ActiveFilters::with_sets(None, &[a, b]);
        filters.add("scratch").expect("valid");
        filters
    }

    /// Search, scratch (no header), then each set: header, and rows only
    /// while enabled.
    #[test]
    fn rows_follow_the_spec_order() {
        let mut filters = two_sets(true, false);
        filters.set_search("s").expect("valid");
        assert_eq!(
            rows(&filters),
            vec![Row::Search, Row::Filter(0), Row::Header(1), Row::Filter(1), Row::Filter(2), Row::Header(2)]
        );
    }

    #[test]
    fn with_no_sets_rows_are_exactly_todays() {
        let filters = set_of(&["a", "b"], &[]);
        assert_eq!(rows(&filters), vec![Row::Filter(0), Row::Filter(1)]);
    }
```

(`loaded` is the helper from Part 1's `filter.rs` tests; move it to a `#[cfg(test)] pub(crate) mod test_support` in `filter.rs` so both modules use one copy.)

- [ ] **Step 2: Run** — Expected: compile error, `Row` not found.

- [ ] **Step 3: Implement**:

```rust
/// One row of the pane. The pane is *only* this list: labels, styles, keys and
/// height all derive from it, so they cannot disagree about what a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Search,
    /// A named set's header, by set index. Never set 0: the scratch set has no header.
    Header(usize),
    /// A filter, by known-list index.
    Filter(usize),
}

pub(crate) fn rows(filters: &ActiveFilters) -> Vec<Row> {
    let mut out = Vec::new();
    if filters.search().is_some() {
        out.push(Row::Search);
    }
    out.extend(filters.filters_in(0).map(|(i, _)| Row::Filter(i)));
    for (set, meta) in filters.sets().iter().enumerate().skip(1) {
        out.push(Row::Header(set));
        if meta.enabled {
            out.extend(filters.filters_in(set).map(|(i, _)| Row::Filter(i)));
        }
    }
    out
}
```

Delete `filter_index_for_row` and `ActiveFilters::row_count`; fix every caller to use `rows(..)` (the next tasks rewrite `handle_key`, `resolve_row`, `row_text`, `render`; for this commit make them compile by computing `rows(filters)` and matching `Row::Search => .. , Row::Filter(i) => .., Row::Header(_) => <treat as no-op / empty label>`). In `lib.rs`, `self.filters.row_count()` becomes `widgets::filterlist::rows(&self.filters).len()`.

- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "refactor(filter pane): rows() is the one description of the pane"`.

### Task 2.2: Rendering the two levels

**Files:**
- Modify: `src/widgets/filterlist.rs` (`resolve_row` → `label_for`, `row_text`, `render`, `preferred_width`)

- [ ] **Step 1: Write the failing tests**:

```rust
    #[test]
    fn an_enabled_set_shows_a_header_and_indented_rows() {
        let filters = two_sets(true, false);
        let mut list = FilterList::default();
        let rows = rendered(&mut list, &filters, 30);
        assert_eq!(rows[1].trim_start_matches('│').trim_end_matches('│').trim_end(), " 1[x] inc scratch");
        assert!(rows[2].contains("[x] a"), "{rows:?}");
        assert!(rows[3].contains("   2[ ] inc x"), "{rows:?}");
        assert!(rows[4].contains("   3[ ] inc y"), "{rows:?}");
        assert!(rows[5].contains("[ ] b"), "{rows:?}");
        assert!(!rows[6].contains('z'), "a disabled set shows no filters");
    }

    #[test]
    fn numbers_run_over_what_is_shown() {
        let mut filters = two_sets(false, true);
        let mut list = FilterList::default();
        let rows = rendered(&mut list, &filters, 30);
        assert!(rows[4].contains("2[ ] inc z"), "{rows:?}");
        filters.set_enabled_set(1, true);
        let rows = rendered(&mut list, &filters, 30);
        assert!(rows[6].contains("4[ ] inc z"), "enabling a set above renumbers below: {rows:?}");
    }

    #[test]
    fn a_disabled_header_is_dimmed() {
        let filters = two_sets(true, false);
        let mut list = FilterList::default();
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);
        list.render(&filters, area, &mut buf);
        assert_eq!(buf[(1, 5)].style().fg, DIM_STYLE.fg);
    }
```

- [ ] **Step 2: Run** — Expected: FAIL on row text.

- [ ] **Step 3: Implement.** Replace `resolve_row`/`row_text` with:

```rust
    /// The text of one row. `number` is the running count of filter rows
    /// above it, so numbers follow what is shown.
    fn row_text(filters: &ActiveFilters, row: Row, number: usize) -> String {
        match row {
            Row::Search => {
                let search = filters.search().expect("Row::Search only when a search exists");
                format!("/[{}] inc {}", mark(search.enabled), search.predicate.display())
            }
            Row::Header(set) => {
                let meta = &filters.sets()[set];
                let star = if meta.profiles.is_empty() { "" } else { " *" };
                format!("[{}] {}{star}", mark(meta.enabled), meta.name)
            }
            Row::Filter(index) => {
                let filter = &filters.filters()[index];
                let indent = if filter.set == 0 { "" } else { "  " };
                format!("{indent}{number}[{}] {} {}", mark(filter.enabled), sense_word(filter.sense), filter.display_name())
            }
        }
    }

fn mark(enabled: bool) -> char { if enabled { 'x' } else { ' ' } }
fn sense_word(sense: Sense) -> &'static str { match sense { Sense::Include => "inc", Sense::Context => "ctx", Sense::Exclude => "exc" } }

    /// Every row's text, numbered.
    fn texts(filters: &ActiveFilters) -> Vec<(Row, String)> {
        let mut number = 0;
        rows(filters)
            .into_iter()
            .map(|row| {
                if matches!(row, Row::Filter(_)) { number += 1; }
                (row, Self::row_text(filters, row, number))
            })
            .collect()
    }

    fn row_style(filters: &ActiveFilters, row: Row) -> Style {
        match row {
            Row::Header(set) if filters.sets()[set].enabled => Style::default(),
            Row::Header(_) => DIM_STYLE,
            Row::Search => if filters.search().is_some_and(|s| s.enabled) { SEARCH_STYLE } else { DIM_STYLE },
            Row::Filter(index) => {
                let filter = &filters.filters()[index];
                if !filter.enabled { return DIM_STYLE; }
                match filter.sense {
                    Sense::Include | Sense::Context => filter.style,
                    Sense::Exclude => Style::default().fg(Color::DarkGray),
                }
            }
        }
    }
```

`render` iterates `texts(filters)` building `ListItem::new(text).style(Self::row_style(filters, row))`; the empty-hint branch tests `rows(filters).is_empty()`. `preferred_width` maxes over `texts(..)`. Drop the star for now if `profiles` is not shown until Part 3 — no: the spec's header shows `*`, and the data is already there, so render it here and Part 3 only adds the key.

- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS. Existing tests that asserted exact row strings for scratch filters still pass because scratch rows are unchanged.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter pane): headers for named sets, rows only while enabled"`.

### Task 2.3: Keys on a header row

**Files:**
- Modify: `src/widgets/mod.rs` (`FilterCommand::ToggleSet(usize)`, `FilterCommand::SetIsReadOnly`)
- Modify: `src/widgets/filterlist.rs` (`handle_key(&mut self, key, rows: &[Row])`)
- Modify: `src/lib.rs` (`handle_filter_key` arms; the `report` message)

- [ ] **Step 1: Write the failing tests** in `filterlist.rs`:

```rust
    fn key(c: char) -> KeyEvent { KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE) }
    fn enter() -> KeyEvent { KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE) }

    #[test]
    fn enter_on_a_header_toggles_the_set() {
        let filters = two_sets(true, false);
        let rows = rows(&filters);
        let mut list = FilterList::default();
        list.state.select(Some(1)); // Header(1)
        assert_eq!(list.handle_key(enter(), &rows), Some(FilterCommand::ToggleSet(1)));
    }

    #[test]
    fn d_c_m_on_a_header_report_read_only() {
        let filters = two_sets(true, false);
        let rows = rows(&filters);
        let mut list = FilterList::default();
        list.state.select(Some(1));
        for c in ['d', 'c', 'm'] {
            assert_eq!(list.handle_key(key(c), &rows), Some(FilterCommand::SetIsReadOnly), "{c}");
        }
    }
```

Add `#[derive(PartialEq, Eq)]` to `FilterCommand` if it lacks it.

- [ ] **Step 2: Run** — Expected: compile error, `ToggleSet` not found.

- [ ] **Step 3: Implement.** `handle_key`'s selection moves use `rows.len()`; the command arms become:

```rust
        let row = self.selected().and_then(|i| rows.get(i).copied());
        match (key.code, row) {
            (KeyCode::Enter, Some(Row::Filter(i))) => Some(FilterCommand::Toggle(i)),
            (KeyCode::Enter, Some(Row::Search)) => Some(FilterCommand::ToggleSearch),
            (KeyCode::Enter, Some(Row::Header(s))) => Some(FilterCommand::ToggleSet(s)),
            (KeyCode::Char('d'), Some(Row::Filter(i))) => Some(FilterCommand::Delete(i)),
            (KeyCode::Char('d'), Some(Row::Search)) => Some(FilterCommand::DeleteSearch),
            (KeyCode::Char('c'), Some(Row::Filter(i))) => Some(FilterCommand::Edit(i)),
            (KeyCode::Char('c'), Some(Row::Search)) => Some(FilterCommand::EditSearch),
            (KeyCode::Char('m'), Some(Row::Filter(i))) => Some(FilterCommand::ToggleContext(i)),
            (KeyCode::Char('d' | 'c' | 'm'), Some(Row::Header(_))) => Some(FilterCommand::SetIsReadOnly),
            _ => None,
        }
```

In `lib.rs`'s `handle_filter_key`: compute `let rows = widgets::filterlist::rows(&self.filters);`, pass `&rows`, and add arms:

```rust
            FilterCommand::ToggleSet(set) => {
                self.filters.toggle_set(set);
            }
            FilterCommand::SetIsReadOnly => {
                self.report("sets are defined in filters.toml; edit the file to change one", false);
                return;
            }
```

`ToggleSet` falls through to `refresh_view`, which re-evaluates and — via `refresh_scan` on the next tick — re-answers the navigator.

- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter pane): Enter toggles a set; d, c, m on a set say why they do nothing"`.

### Task 2.4: Docs

- [ ] **Step 1:** `src/help.rs`: change the `Enter` row's action to `"Enable or disable the selected filter or set"`. Run `cargo test -p recon help::` — Expected: PASS.
- [ ] **Step 2:** README: the pane diagram from the spec under *Saved filter sets*; the `Enter` keybinding row; a paragraph on *enabling keeps the flags*.
- [ ] **Step 3: Commit** — `git commit -am "docs: the two-level filter pane"`. Open the PR for #129.

---

# Part 3 — #130: the profile picker

Branch: `Fix-I130-profile-picker`.

### Task 3.1: `ProfilePicker` — state and keys, no rendering

**Files:**
- Create: `src/widgets/picker.rs`
- Modify: `src/widgets/mod.rs` (`mod picker;`, `FilterCommand::PickProfile(usize)`)

**Interfaces:**
- Produces:
  ```rust
  pub(crate) struct ProfilePicker { pub set: usize, names: Vec<String>, selected: usize }
  impl ProfilePicker {
      pub(crate) fn new(set: usize, names: Vec<String>) -> Self
      pub(crate) fn handle_key(&mut self, key: KeyEvent) -> PickerOutcome
      pub(crate) fn render(&self, area: Rect, buf: &mut Buffer)
  }
  pub(crate) enum PickerOutcome { Open, Closed, Chosen(String) }
  ```

- [ ] **Step 1: Write the failing tests** in `picker.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn picker() -> ProfilePicker { ProfilePicker::new(1, vec!["default".into(), "loud".into()]) }
    fn key(code: KeyCode) -> KeyEvent { KeyEvent::new(code, KeyModifiers::NONE) }

    #[test]
    fn j_and_k_move_and_clamp() {
        let mut p = picker();
        assert_eq!(p.handle_key(key(KeyCode::Char('k'))), PickerOutcome::Open);
        assert_eq!(p.selected, 0);
        p.handle_key(key(KeyCode::Char('j')));
        p.handle_key(key(KeyCode::Char('j')));
        assert_eq!(p.selected, 1);
    }

    #[test]
    fn enter_chooses_and_esc_closes() {
        let mut p = picker();
        p.handle_key(key(KeyCode::Char('j')));
        assert_eq!(p.handle_key(key(KeyCode::Enter)), PickerOutcome::Chosen("loud".into()));
        assert_eq!(picker().handle_key(key(KeyCode::Esc)), PickerOutcome::Closed);
    }

    #[test]
    fn other_keys_are_swallowed() {
        assert_eq!(picker().handle_key(key(KeyCode::Char('q'))), PickerOutcome::Open);
    }
}
```

- [ ] **Step 2: Run** — Expected: compile error.
- [ ] **Step 3: Implement** the struct, `new`, `handle_key` (`j`/`Down`, `k`/`Up`, `Enter`, `Esc`, everything else `Open`), and a `render` that: computes a centred `Rect` of width `names.iter().map(len).max() + 4` and height `names.len() + 2`, clamped to `area`; `Clear.render`; `Block::bordered().title(" Profiles ")`; one line per name, the selected one `Modifier::REVERSED`. Model it on `help::render`'s `Clear` + `Block` use.
- [ ] **Step 4: Run** `cargo test -p recon widgets::picker` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(picker): a profile picker overlay — state, keys, rendering"`.

### Task 3.2: Wiring `a`, the overlay, and applying the choice

**Files:**
- Modify: `src/widgets/filterlist.rs` (`a` on a header → `PickProfile(set)`)
- Modify: `src/lib.rs` (`picker: Option<ProfilePicker>` on `App`; route keys; render; apply)
- Modify: `src/help.rs` (`a` row in *Filter pane*)

- [ ] **Step 1: Write the failing tests** in `lib.rs`:

```rust
    #[test]
    fn a_on_a_set_with_profiles_opens_the_picker_and_enter_applies() {
        let mut set = loaded("a", 50, true, &["x", "y"]);
        set.profiles.insert("only-y".into(), vec!["y".into()]);
        let mut app = App::new(&config_with_sets(vec![set]));
        app.focus = Focus::Filters;
        app.filters_pane.state.select(Some(0)); // Header(1): no scratch, no search
        app.handle_event(press('a'));
        assert!(app.picker.is_some());
        app.handle_event(press('q'));
        assert!(app.picker.is_some(), "the picker takes every key");
        app.handle_event(key(KeyCode::Enter));
        assert!(app.picker.is_none());
        let flags: Vec<bool> = app.filters.filters_in(1).map(|(_, f)| f.enabled).collect();
        assert_eq!(flags, vec![false, true]);
    }

    #[test]
    fn a_on_a_set_without_profiles_reports() {
        let mut app = App::new(&config_with_sets(vec![loaded("a", 50, true, &["x"])]));
        app.focus = Focus::Filters;
        app.filters_pane.state.select(Some(0));
        app.handle_event(press('a'));
        assert!(app.picker.is_none());
        assert!(app.status_message.as_ref().is_some_and(|m| m.text.contains("no profiles")));
    }
```

(`press`/`key` are whatever the existing `lib.rs` tests use to synthesise a key event — reuse them.)

- [ ] **Step 2: Run** — Expected: compile error, `picker` field.
- [ ] **Step 3: Implement.** `filterlist.rs`: `(KeyCode::Char('a'), Some(Row::Header(s))) => Some(FilterCommand::PickProfile(s))`. `lib.rs`: at the top of `handle_event`'s key branch, before the prompt check, `if let Some(picker) = self.picker.as_mut() { match picker.handle_key(key) { Open => {}, Closed => self.picker = None, Chosen(name) => { let set = picker.set; self.picker = None; self.filters.apply_profile(set, &name); self.refresh_view(); } } return; }`. In `handle_filter_key`: `FilterCommand::PickProfile(set) => { let names: Vec<String> = self.filters.sets()[set].profiles.keys().cloned().collect(); if names.is_empty() { self.report("no profiles in this set", false); } else { self.picker = Some(ProfilePicker::new(set, names)); } return; }`. In `render`, after the help overlay: `if let Some(picker) = &self.picker { picker.render(area, buf); }`. `help.rs`: `Binding { keys: &["a"], action: "Apply a profile to the selected set" }`.
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS, including `every_bound_key_is_documented`.
- [ ] **Step 5: Commit** — `git commit -am "feat(app): a opens a profile picker on a set; Enter applies it"`.
- [ ] **Step 6:** README: one paragraph and the `a` row. Commit `docs: the profile picker`. Open the PR for #130.

---

# Part 4 — #132: solo and reset

Branch: `Fix-I132-solo-reset`.

### Task 4.1: `solo` and `soloed` on the model

**Files:**
- Modify: `src/filter.rs`

**Interfaces:**
- Produces: `pub fn solo(&mut self, set: usize) -> bool`, `pub fn soloed(&self) -> Option<usize>`.

- [ ] **Step 1: Write the failing tests**:

```rust
    fn three_sets() -> ActiveFilters {
        let mut set = ActiveFilters::with_sets(None, &[
            loaded("a", 10, true, &["x"]), loaded("b", 20, true, &["y"]), loaded("c", 30, false, &["z"]),
        ]);
        set.add("scratch").expect("valid");
        set
    }
    fn set_flags(set: &ActiveFilters) -> Vec<bool> { set.sets().iter().map(|s| s.enabled).collect() }

    #[test]
    fn solo_enables_one_set_and_suspends_the_rest_including_scratch() {
        let mut set = three_sets();
        assert!(set.solo(2));
        assert_eq!(set_flags(&set), vec![false, false, true, false]);
        assert_eq!(set.soloed(), Some(2));
    }

    #[test]
    fn solo_again_restores_the_snapshot() {
        let mut set = three_sets();
        set.solo(2);
        assert!(!set.solo(2));
        assert_eq!(set_flags(&set), vec![true, true, true, false]);
        assert_eq!(set.soloed(), None);
    }

    #[test]
    fn moving_the_solo_keeps_the_first_snapshot() {
        let mut set = three_sets();
        set.solo(2);
        set.solo(3);
        assert_eq!(set_flags(&set), vec![false, false, false, true]);
        set.solo(3);
        assert_eq!(set_flags(&set), vec![true, true, true, false]);
    }

    #[test]
    fn soloing_a_disabled_set_applies_its_default() {
        let mut c = loaded("c", 30, false, &["z", "w"]);
        c.profiles.insert("default".into(), vec!["w".into()]);
        let mut set = ActiveFilters::with_sets(None, &[c]);
        set.solo(1);
        assert_eq!(flags(&set, 1), vec![false, true]);
    }
```

- [ ] **Step 2: Run** — Expected: compile error.
- [ ] **Step 3: Implement.** Add `solo: Option<Solo>` to `ActiveFilters` (`struct Solo { set: usize, snapshot: Vec<bool> }`, `Default` gives `None`):

```rust
    /// Solo `set`: snapshot every set flag (scratch included), enable only
    /// `set`. On the soloed set, restore the snapshot instead. On another set
    /// while soloed, move the solo and keep the original snapshot.
    /// Returns whether a solo is now in force.
    pub fn solo(&mut self, set: usize) -> bool {
        if set == 0 || set >= self.sets.len() {
            return self.solo.is_some();
        }
        if let Some(current) = self.solo.take() {
            if current.set == set {
                for (meta, was) in self.sets.iter_mut().zip(current.snapshot) {
                    meta.enabled = was;
                }
                return false;
            }
            self.solo = Some(Solo { set, snapshot: current.snapshot });
        } else {
            self.solo = Some(Solo { set, snapshot: self.sets.iter().map(|s| s.enabled).collect() });
        }
        let was_enabled = self.sets[set].enabled;
        for (index, meta) in self.sets.iter_mut().enumerate() {
            meta.enabled = index == set;
        }
        // The same thing enabling by hand does: a set that was off comes on
        // with its `default` profile applied, if it has one.
        if !was_enabled && self.sets[set].profiles.contains_key("default") {
            self.apply_profile(set, "default");
        }
        true
    }

    #[must_use]
    pub fn soloed(&self) -> Option<usize> {
        self.solo.as_ref().map(|s| s.set)
    }
```

The snapshot is aligned to `sets`, which never shrinks in-session (Part 5's `adopt_scratch_as` inserts, so it must also insert `false` into a live snapshot at the same index — add that there), so no other realignment is needed.

- [ ] **Step 4: Run** `cargo test -p recon filter::` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter): solo isolates one set and restores the rest on a second press"`.

### Task 4.2: `reset` on the model

- [ ] **Step 1: Write the failing test**:

```rust
    #[test]
    fn reset_returns_every_set_to_startup_and_leaves_scratch_alone() {
        let mut a = loaded("a", 10, true, &["x", "y"]);
        a.profiles.insert("default".into(), vec!["x".into()]);
        let mut set = ActiveFilters::with_sets(None, &[a, loaded("b", 20, false, &["z"])]);
        set.add("scratch").expect("valid");
        set.set_enabled(0, false);
        set.solo(2);
        set.set_enabled(3, true); // z
        set.disable_all_remembering();
        set.reset();
        assert_eq!(set_flags(&set), vec![true, true, false]);
        assert_eq!(flags(&set, 1), vec![true, false]);
        assert_eq!(flags(&set, 2), vec![false]);
        assert!(!set.filters()[0].enabled, "scratch flag untouched");
        assert_eq!(set.soloed(), None);
        assert!(!set.has_remembered());
    }
```

- [ ] **Step 2: Run** — Expected: compile error.
- [ ] **Step 3: Implement**:

```rust
    /// Every set back to its startup state; scratch filters untouched.
    pub fn reset(&mut self) {
        self.solo = None;
        self.forget_capture();
        self.sets[0].enabled = true;
        for set in 1..self.sets.len() {
            for filter in self.filters.iter_mut().filter(|f| f.set == set) {
                filter.enabled = false;
            }
            self.sets[set].enabled = false;
            if self.sets[set].autoload {
                self.set_enabled_set(set, true);
            }
        }
    }
```

- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter): reset returns every set to its startup state"`.

### Task 4.3: `s`, `R`, the `solo` marker, and rows while soloed

**Files:**
- Modify: `src/widgets/mod.rs` (`FilterCommand::Solo(usize)`, `FilterCommand::Reset`)
- Modify: `src/widgets/filterlist.rs` (`rows` while soloed; header text; keys)
- Modify: `src/lib.rs`, `src/help.rs`, `README.md`

- [ ] **Step 1: Write the failing tests** in `filterlist.rs`:

```rust
    #[test]
    fn while_soloed_rows_are_the_search_and_the_soloed_set_only() {
        let mut filters = two_sets(true, true);
        filters.set_search("s").expect("valid");
        filters.solo(2);
        assert_eq!(rows(&filters), vec![Row::Search, Row::Header(2), Row::Filter(3)]);
    }

    #[test]
    fn the_soloed_header_says_so() {
        let mut filters = two_sets(true, true);
        filters.solo(2);
        let mut list = FilterList::default();
        let rows = rendered(&mut list, &filters, 30);
        assert!(rows[1].contains("[x] b solo"), "{rows:?}");
    }

    #[test]
    fn s_on_a_header_and_big_r_anywhere() {
        let filters = two_sets(true, false);
        let rows = rows(&filters);
        let mut list = FilterList::default();
        list.state.select(Some(1));
        assert_eq!(list.handle_key(key('s'), &rows), Some(FilterCommand::Solo(1)));
        list.state.select(Some(0));
        assert_eq!(list.handle_key(key('s'), &rows), None);
        assert_eq!(list.handle_key(key('R'), &rows), Some(FilterCommand::Reset));
    }
```

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement.** `rows()`: if `filters.soloed()` is `Some(set)`, return search row (if any), `Header(set)`, and its filters — nothing else. `row_text` header arm: append `" solo"` when `filters.soloed() == Some(set)`. Keys: `(KeyCode::Char('s'), Some(Row::Header(s))) => Some(FilterCommand::Solo(s))`, `(KeyCode::Char('R'), _) => Some(FilterCommand::Reset)` (uppercase `R` arrives with `SHIFT` in the modifiers on some terminals; the guard at the top of `handle_key` only rejects CONTROL and ALT, so this is fine). `lib.rs`: `FilterCommand::Solo(set) => { self.filters.solo(set); }`, `FilterCommand::Reset => { self.filters.reset(); }` — both fall through to `refresh_view`. `help.rs`: rows for `s` ("Solo the selected set — or un-solo it") and `R` ("Reset every set to its startup state").
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter pane): s solos a set, R resets every set"`. README paragraphs and rows; commit `docs: solo and reset`. Open the PR for #132.

---

# Part 5 — #131: `S` saves the scratch set

Branch: `Fix-I131-save-scratch-set`.

### Task 5.1: `filtersets::append_set` — pure over the document text

**Files:**
- Modify: `Cargo.toml` (`toml_edit = "0.24"` — check the current version on crates.io; pin the exact one)
- Modify: `src/filtersets.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct SetToSave<'a> { pub name: &'a str, pub filters: Vec<(String /* pattern */, Sense)>, pub default: Vec<String> /* patterns enabled now */ }
  pub fn append_set(text: &str, set: &SetToSave<'_>) -> Result<String, toml_edit::TomlError>
  ```

- [ ] **Step 1: Write the failing tests**:

```rust
    #[test]
    fn append_set_preserves_comments_and_other_sets() {
        let before = "# my sets\n[sets.a]\n# keep me\n[[sets.a.filters]]\npattern = 'x'\n";
        let after = append_set(before, &SetToSave {
            name: "bug 57",
            filters: vec![("ERROR".into(), Sense::Include), ("DEBUG".into(), Sense::Exclude)],
            default: vec!["ERROR".into()],
        }).expect("edits");
        assert!(after.starts_with(before), "existing text is untouched:\n{after}");
        assert!(after.contains("[sets.\"bug 57\"]"), "{after}");
        assert!(after.contains("pattern = 'ERROR'"), "single-quoted literal:\n{after}");
        assert!(after.contains("sense = \"exclude\""), "{after}");
        assert!(after.contains("default = [\"ERROR\"]"), "{after}");
        assert!(!after.contains("autoload"), "{after}");
        let sets = parse(&after, Path::new("t")).expect("round-trips");
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[1].name, "bug 57");
    }

    #[test]
    fn append_set_starts_an_empty_file() {
        let after = append_set("", &SetToSave { name: "n", filters: vec![("x".into(), Sense::Include)], default: vec![] }).expect("edits");
        assert!(parse(&after, Path::new("t")).is_ok());
    }
```

- [ ] **Step 2: Run** — Expected: compile error.
- [ ] **Step 3: Implement** with `toml_edit::DocumentMut`: parse `text`; get or create the `sets` table (`implicit(true)` so no bare `[sets]` header is emitted); insert a new `Table` under `set.name` (a `Key` built from the string — `toml_edit` quotes it when needed); under it a `profiles` table with `default` as an `Array` of strings when non-empty, and a `filters` `ArrayOfTables` with one `Table` per filter holding `pattern` as a **literal string** (`toml_edit::Value::from(toml_edit::Formatted::new(..))` then `.decor` — or simplest: `Value::String(Formatted::new(pattern))` followed by setting the repr to `'..'` via `Repr::new_unchecked(format!("'{pattern}'"))` when the pattern contains no `'`; otherwise leave the default double-quoted, escaped form, which is still correct) and `sense` only when not `Include`. Return `doc.to_string()`.
- [ ] **Step 4: Run** `cargo test -p recon filtersets::` — Expected: PASS. Add a Cargo.toml comment on `toml_edit` saying it exists for `filters.toml` alone and why `toml`'s `display` stays off (quote the config spec's tripwire).
- [ ] **Step 5: Commit** — `git commit -am "feat(filtersets): append_set writes a set with toml_edit, preserving the file"`.

### Task 5.2: `adopt_scratch_as` on the model

**Files:**
- Modify: `src/filter.rs`

**Interfaces:**
- Produces: `pub fn adopt_scratch_as(&mut self, name: &str, path: PathBuf) -> bool` (false if the scratch set is empty or the name exists).

- [ ] **Step 1: Write the failing test**:

```rust
    #[test]
    fn adopt_moves_scratch_into_a_new_enabled_set_with_default_from_the_flags() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("m", 10, true, &["q"])]);
        set.add("a").unwrap();
        set.add("b").unwrap();
        set.set_enabled(1, false); // b off
        assert!(set.adopt_scratch_as("new", PathBuf::from("t")));
        assert_eq!(set.filters_in(0).count(), 0);
        let new = set.sets().iter().position(|s| s.name == "new").expect("exists");
        assert!(set.sets()[new].enabled);
        assert_eq!(set.sets()[new].priority, 50);
        assert_eq!(set.sets()[new].profiles["default"], vec!["a".to_string()]);
        assert_eq!(flags(&set, new), vec![true, false]);
        assert!(!set.adopt_scratch_as("new", PathBuf::from("t")), "name taken");
        assert!(!set.adopt_scratch_as("other", PathBuf::from("t")), "scratch is empty now");
    }
```

- [ ] **Step 2: Run** — Expected: compile error.
- [ ] **Step 3: Implement.** Build the `FilterSet` (`origin: File(path)`, `priority: DEFAULT_PRIORITY` — move that const to `filter.rs` or import it — `autoload: false`, `enabled: true`, `profiles: {default: enabled scratch names}`); compute its position in `sets` by `(priority, name)` among indices ≥ 1; insert it there and bump `filter.set` by one for every filter whose set index is ≥ the insertion point; then take the scratch filters (`drain(..scratch_end())`), set their `set` to the new index, and splice them into the known list at the end of the sets before it (the first index whose `set` is greater than the new index, or `len`). `recompile()`, `forget_capture()`. Colours are kept — they are properties of the filter.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(filter): adopt_scratch_as turns the scratch set into a named, enabled set"`.

### Task 5.3: `S`, the prompt, and the write

**Files:**
- Modify: `src/lib.rs` (`PromptKind::SaveSet`; sigil `"save as: "`; `S` in `handle_filter_key`; `save_scratch_as`)
- Modify: `src/help.rs`, `README.md`, the configuration spec's "If a write path ever lands" (one sentence: it landed, for `filters.toml` only, via `toml_edit`)

- [ ] **Step 1: Write the failing tests** in `lib.rs` (fixture under `target/test-config/`, claimed by name; inject the path — add `save_path: Option<PathBuf>` to `App`, defaulting to `filtersets::path()`, and set it in the test):

```rust
    #[test]
    fn big_s_saves_the_scratch_set_and_adopts_it() {
        let path = fixture_path("save-scratch.toml"); // claims the name; file absent
        let mut app = App::new(&Config::default());
        app.save_path = Some(path.clone());
        app.add_filter("ERROR").unwrap();
        app.focus = Focus::Filters;
        app.handle_event(press('S'));
        assert!(app.search.is_some(), "the prompt is open");
        for c in "bug".chars() { app.handle_event(press(c)); }
        app.handle_event(key(KeyCode::Enter));
        assert!(app.search.is_none());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[sets.bug]"), "{text}");
        assert_eq!(app.filters.filters_in(0).count(), 0);
        assert_eq!(app.filters.sets()[1].name, "bug");
    }

    #[test]
    fn big_s_with_an_empty_scratch_set_reports_and_opens_nothing() {
        let mut app = App::new(&Config::default());
        app.focus = Focus::Filters;
        app.handle_event(press('S'));
        assert!(app.search.is_none());
        assert!(app.status_message.as_ref().is_some_and(|m| m.text.contains("nothing to save")));
    }
```

- [ ] **Step 2: Run** — Expected: compile error.
- [ ] **Step 3: Implement.** In `handle_filter_key`, beside `i`/`x`: `KeyCode::Char('S') => { if self.filters.filters_in(0).next().is_none() { self.report("nothing to save: the scratch set is empty", false); return; } self.search = Some(SearchPrompt { kind: PromptKind::SaveSet, ..Default::default() }); return; }`. In `handle_search_key`'s `Enter` match: `PromptKind::SaveSet => self.save_scratch_as(&pattern)`. The outcome type of that match is `Result<(), regex::Error>`; give `save_scratch_as` its own error and map: on failure `self.report(&err, true)` and keep the prompt open by returning `Err`. Then:

```rust
    /// `S`: write the scratch set to `filters.toml` as `name`, then adopt it
    /// in memory so the pane shows what a restart would show — without
    /// discarding any other set's current state.
    fn save_scratch_as(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a set needs a name".into());
        }
        if self.filters.sets().iter().any(|s| s.name == name) {
            return Err(format!("a set named {name:?} already exists; edit filters.toml to change it"));
        }
        let Some(path) = self.save_path.clone() else {
            return Err("no config home ($XDG_CONFIG_HOME, $HOME unset); nowhere to save".into());
        };
        let to_save = filtersets::SetToSave {
            name,
            filters: self.filters.filters_in(0).map(|(_, f)| (f.predicate.display(), f.sense)).collect(),
            default: self.filters.filters_in(0).filter(|(_, f)| f.enabled).map(|(_, f)| f.display_name()).collect(),
        };
        let before = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("could not read {}: {e}", path.display())),
        };
        let after = filtersets::append_set(&before, &to_save).map_err(|e| e.to_string())?;
        // Prove a restart would load it before touching the disk or the model.
        filtersets::parse(&after, &path).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        std::fs::write(&path, after).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        self.filters.adopt_scratch_as(name, path);
        self.refresh_view();
        self.report(&format!("saved set {name:?}"), false);
        Ok(())
    }
```

The prompt's error line shows `INVALID_PATTERN` today on `Err`; for `SaveSet` show the message itself (set `prompt.error = Some(message)`).

- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Docs.** `help.rs`: `S` row ("Save the scratch filters as a named set"). README: the save flow; revise "recon never writes this file" to "recon writes this file only on `S`, and only appends". Configuration spec: one sentence under *If a write path ever lands*. Commit `feat(app): S saves the scratch set to filters.toml` and `docs: saving a set`. Open the PR for #131.

---

## Self-review against the spec

- *The model* → Tasks 1.1–1.4. *Scratch set* → 1.1, 1.2 (`insert_scratch`), 4.1 (suspended by solo). *Order and priority* → 1.5 sort, 2.1 rows. *Numbers and colours* → 1.2 (colour by position or file), 2.2 (running numbers). *Enabling a set* → 1.4. *`autoload`* → 1.2. *Solo* → 4.1, 4.3. *Reset* → 4.2, 4.3. *`!`, peek* → untouched by design; 1.3 leaves the flag-level operations alone. *The file* → 1.5, 1.6. *The pane* → 2.1–2.3, 3.2, 4.3. *The navigator* → 1.3 (`matcher` effective). *Built-in sets* → not this plan (#127); the `Origin` enum and "a table naming a built-in set" seam are left to #127, which adds the variant and the name check in `parse`. *Saving* → 5.1–5.3. *Testing* rules → every task's tests are pure or fixture-based; no env vars.
- One spec sentence is refined by 5.3: the saved set is adopted in memory rather than the whole model rebuilt from a re-read, so other sets keep their state; the written text is re-parsed to prove a restart would load it. The spec's *Saving* step 3 is updated to say so.
- Known edge, unchanged from today: adding a filter during a peek misaligns the peek's captured flags. With `insert_scratch` the misalignment now shifts file filters too. Spec calls this drift; not fixed here.
