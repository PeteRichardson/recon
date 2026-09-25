//! Filter sets: the set, solo, reset and adopt state machine over
//! [`ActiveFilters`], and the types the loader hands it.

use super::{ActiveFilters, Filter, Predicate, Sense};
use crate::syntax::Kind;
use ratatui::style::{Color, Style};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Where a set came from, which decides what may be done to it (#128).
///
/// `File` carries its path from day one so that "unload this file" (#46) is
/// a filter over origins later, not a new field then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The unnamed set typed filters land in. Always index 0, never in a file.
    Scratch,
    File(PathBuf),
    /// A set recon ships (#127): always present, its filters never in the
    /// file. A `[sets.<name>]` table may set its `priority`, `autoload`,
    /// `listed`, `description` and `profiles` and nothing else. Its filters take no number and no palette colour.
    BuiltIn,
}

/// The name of the one built-in set: definition filters — functions,
/// classes, structs, enums — answered by the grammar pass in
/// `syntax::definitions` (#127).
pub const DEFINITIONS_SET: &str = "definitions";

/// What the set picker says about the built-in set when the file's
/// `[sets.definitions]` table gives no `description` of its own (#284).
pub const DEFINITIONS_DESCRIPTION: &str =
    "Functions, types and other definitions, found by the syntax grammar";

/// Whether `name` is a set recon ships, which the file may position and
/// switch but not fill.
#[must_use]
pub fn is_builtin_name(name: &str) -> bool {
    name == DEFINITIONS_SET
}

/// A named group of filters, toggled as a unit (#128).
///
/// The filters themselves are not here: they are in `ActiveFilters::filters`,
/// the flat known list, each carrying the index of its set. Keeping the list
/// flat is what leaves `Verdict::Included(index)`, `Matcher` and the scan
/// cache untouched by sets. A filter takes effect only when it is enabled
/// *and* its set is — see `ActiveFilters::effective`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterSet {
    pub name: String,
    pub origin: Origin,
    /// Pane position, lower first. The scratch set ignores it and is always first.
    pub priority: i32,
    /// Enabled at startup, and what a reset returns the flag to — for a
    /// set that is listed.
    pub autoload: bool,
    /// Has a row in the pane (#282). An unlisted set has none and is never
    /// enabled: `enabled` implies `listed`, and the methods that set either
    /// flag keep it so. The file's `listed` is the startup value only (ADR
    /// 0002); `set_listed` changes it for the session.
    pub listed: bool,
    pub enabled: bool,
    /// One line for the set picker (#284), or `None` for a blank.
    pub description: Option<String>,
    /// Named subsets of this set's filters, by `Filter::display_name`.
    pub profiles: BTreeMap<String, Vec<String>>,
}

impl FilterSet {
    pub(super) fn scratch() -> Self {
        Self {
            name: String::new(),
            origin: Origin::Scratch,
            priority: i32::MIN,
            autoload: true,
            listed: true,
            enabled: true,
            description: None,
            profiles: BTreeMap::new(),
        }
    }
}

/// A solo in force: which set, and what every set's flag was before it.
///
/// From audio mixers. The snapshot is aligned to `ActiveFilters::sets` by
/// index and is taken once, on the first `s`; moving the solo to another set
/// keeps it, so un-soloing returns to the world before the *first* `s` and
/// not to an intermediate one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Solo {
    set: usize,
    snapshot: Vec<bool>,
}

/// One filter as read from `filters.toml`, before it has a colour or a
/// position. The loader's output and the model's input; the model owns the
/// type because the model decides what a set *is*.
#[derive(Debug, Clone)]
pub struct LoadedFilter {
    pub name: String,
    pub predicate: Predicate,
    pub sense: Sense,
    /// The file's `colour`, or `None` for the next palette colour.
    pub colour: Option<Color>,
}

/// One set as read from `filters.toml`.
#[derive(Debug, Clone)]
pub struct LoadedSet {
    pub name: String,
    pub path: PathBuf,
    pub priority: i32,
    pub autoload: bool,
    /// The file's `listed`, `true` when absent. Wins over `autoload`.
    pub listed: bool,
    /// The file's `description`, `None` when absent (#284).
    pub description: Option<String>,
    pub profiles: BTreeMap<String, Vec<String>>,
    pub filters: Vec<LoadedFilter>,
    /// A `[sets.<name>]` table naming a built-in set: `priority`,
    /// `autoload` and `profiles` are the file's, the filters are recon's,
    /// and `filters` above is empty.
    pub builtin: bool,
}

impl LoadedSet {
    /// The built-in definitions set as it is when `filters.toml` has no
    /// table for it: collapsed, at the default priority, no profiles.
    ///
    /// The loader appends this to every file that does not name the set,
    /// so the list `Config::check_sets` validates `--set` against and the
    /// list `with_sets` builds from are the same list (#220). `with_sets`
    /// falls back to it too, for a caller that never ran the loader.
    #[must_use]
    pub fn builtin_default() -> Self {
        Self {
            name: DEFINITIONS_SET.to_string(),
            path: PathBuf::new(),
            priority: crate::filtersets::DEFAULT_PRIORITY,
            autoload: false,
            listed: true,
            description: None,
            profiles: BTreeMap::new(),
            filters: Vec::new(),
            builtin: true,
        }
    }
}

/// Why [`ActiveFilters::enable_named`] could not apply a `--set` (#143).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnableError {
    /// No set of that name — or the scratch set, which has none.
    UnknownSet(String),
    /// The set exists but defines no such profile.
    UnknownProfile { set: String, profile: String },
}

impl std::fmt::Display for EnableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSet(name) => write!(f, "unknown set {name:?}"),
            Self::UnknownProfile { set, profile } => {
                write!(f, "unknown profile {profile:?} for set {set:?}")
            }
        }
    }
}

impl std::error::Error for EnableError {}

impl ActiveFilters {
    /// Build the startup set (#128): the scratch set, then `sets` in the
    /// order given — the loader has already sorted them by priority and
    /// name — every file filter disabled, then each `autoload` set that is
    /// also listed enabled, which applies its `default` profile if it has one.
    ///
    /// A file filter's colour is its position in the known list, the same
    /// rule `add` uses, unless the file named one. Assigned here, once, and
    /// stored on the filter: toggling a set later changes what is shown and
    /// what matches, never a colour.
    #[must_use]
    pub fn with_sets(palette: Option<Vec<Color>>, sets: &[LoadedSet]) -> Self {
        let mut this = Self::bare(palette);
        // The built-in set is always present. The loader supplies it — a
        // `[sets.definitions]` table passed through with its `priority`,
        // `autoload` and `profiles` and no filters, or the default when the
        // file has no such table — so a caller that ran the loader never
        // reaches the fallback. It is here for the ones that did not:
        // `new`, `with_palette` and the tests.
        let mut ordered: Vec<&LoadedSet> = sets.iter().collect();
        let default_builtin = LoadedSet::builtin_default();
        if !ordered.iter().any(|set| set.builtin) {
            ordered.push(&default_builtin);
        }
        ordered.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.name.cmp(&b.name))
        });

        for loaded in ordered {
            let index = this.sets.len();
            if loaded.builtin {
                // The profiles are the file's, over the kinds' plural
                // names; the loader has checked every member is one.
                this.sets.push(FilterSet {
                    name: loaded.name.clone(),
                    origin: Origin::BuiltIn,
                    priority: loaded.priority,
                    autoload: loaded.autoload,
                    listed: loaded.listed,
                    enabled: false,
                    // Recon describes its own set; the table can override.
                    description: Some(
                        loaded
                            .description
                            .clone()
                            .unwrap_or_else(|| DEFINITIONS_DESCRIPTION.to_string()),
                    ),
                    profiles: loaded.profiles.clone(),
                });
                // No palette colour: a built-in filter wears the terminal's
                // default, so the pane's colours stay the user's own.
                for kind in Kind::ALL {
                    this.filters.push(Filter {
                        predicate: Predicate::Definition(kind),
                        sense: Sense::Include,
                        enabled: false,
                        style: Style::default(),
                        name: Some(kind.plural().to_string()),
                        set: index,
                    });
                }
                continue;
            }
            this.sets.push(FilterSet {
                name: loaded.name.clone(),
                origin: Origin::File(loaded.path.clone()),
                priority: loaded.priority,
                autoload: loaded.autoload,
                listed: loaded.listed,
                enabled: false,
                description: loaded.description.clone(),
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
        // `listed = false` wins over `autoload = true` (ADR 0002).
        for index in 1..this.sets.len() {
            if this.sets[index].autoload && this.sets[index].listed {
                this.set_enabled_set(index, true);
            }
        }
        this
    }

    /// Every set, scratch first, then in pane order.
    #[must_use]
    pub fn sets(&self) -> &[FilterSet] {
        &self.sets
    }

    /// The filters in `set`, with their known-list indices.
    pub fn filters_in(&self, set: usize) -> impl Iterator<Item = (usize, &Filter)> {
        self.filters
            .iter()
            .enumerate()
            .filter(move |(_, filter)| filter.set == set)
    }

    /// Enable or disable a named set. Enabling applies the set's `default`
    /// profile if it has one; otherwise, and on disable, no filter flag
    /// moves — a set re-enabled without a `default` shows the toggles it
    /// had.
    ///
    /// Returns `false` for the scratch set, which is never toggled by hand,
    /// for an index that names no set, and for enabling an unlisted set —
    /// enabled implies listed (#282), so `set_listed` comes first.
    pub fn set_enabled_set(&mut self, set: usize, enabled: bool) -> bool {
        if set == 0 || set >= self.sets.len() || (enabled && !self.sets[set].listed) {
            return false;
        }
        self.sets[set].enabled = enabled;
        if enabled && self.sets[set].profiles.contains_key("default") {
            self.apply_profile(set, "default");
        }
        true
    }

    /// Flip a named set, reporting its new state — or `None` for the
    /// scratch set, for an index that names no set, and for an unlisted set.
    pub fn toggle_set(&mut self, set: usize) -> Option<bool> {
        let now = !self.sets.get(set)?.enabled;
        self.set_enabled_set(set, now).then_some(now)
    }

    /// List or unlist a named set for this session (#282).
    ///
    /// Unlisting also disables the set, so it stops deciding lines at once;
    /// its filter flags are kept. Unlisting the soloed set ends the solo.
    /// Listing gives a disabled set with the flags it had, in its `priority`
    /// position, which it never left: `autoload` is a startup value and does
    /// not apply here.
    ///
    /// Returns `false`, changing nothing, for the scratch set — which is
    /// always listed — and for an index that names no set.
    pub fn set_listed(&mut self, set: usize, listed: bool) -> bool {
        if set == 0 || set >= self.sets.len() {
            return false;
        }
        self.sets[set].listed = listed;
        if !listed {
            self.sets[set].enabled = false;
            if let Some(solo) = self.solo.take_if(|solo| solo.set == set) {
                self.restore(solo.snapshot);
            }
        }
        true
    }

    /// Put every set's flag back from a solo's snapshot, except that an
    /// unlisted set stays disabled: a snapshot does not bring a set back
    /// (ADR 0002).
    fn restore(&mut self, snapshot: Vec<bool>) {
        for (meta, was) in self.sets.iter_mut().zip(snapshot) {
            meta.enabled = was && meta.listed;
        }
    }

    /// Enable exactly the profile's members within `set` and disable the
    /// set's other filters. An action, not a binding: nothing remembers that
    /// a profile was applied, and later toggles are the user's to make.
    pub fn apply_profile(&mut self, set: usize, profile: &str) -> bool {
        let Some(members) = self
            .sets
            .get(set)
            .and_then(|meta| meta.profiles.get(profile))
            .cloned()
        else {
            return false;
        };
        for filter in self.filters.iter_mut().filter(|f| f.set == set) {
            filter.enabled = members.contains(&filter.display_name());
        }
        true
    }

    /// List and enable the set called `set` — `default` profile and all,
    /// exactly as `set_enabled_set` does — then apply `profile` when one is named
    /// (#143). Both names are checked before anything moves, so a refused
    /// call changes nothing. `Config::check_sets` refuses the same names in
    /// `main` before the terminal comes up; this is the same lookup, so a
    /// name that passed there cannot fail here.
    pub fn enable_named(&mut self, set: &str, profile: Option<&str>) -> Result<(), EnableError> {
        let index = self.named(set)?;
        if let Some(profile) = profile
            && !self.sets[index].profiles.contains_key(profile)
        {
            return Err(EnableError::UnknownProfile {
                set: set.to_string(),
                profile: profile.to_string(),
            });
        }
        self.set_listed(index, true);
        self.set_enabled_set(index, true);
        if let Some(profile) = profile {
            self.apply_profile(index, profile);
        }
        Ok(())
    }

    /// Unlist the set called `set`, as `set_listed` does (#283). The same
    /// lookup as `enable_named`, so `Config::check_sets` refuses in `main`
    /// every name that would fail here.
    pub fn unlist_named(&mut self, set: &str) -> Result<(), EnableError> {
        let index = self.named(set)?;
        self.set_listed(index, false);
        Ok(())
    }

    /// The index of the named set called `set`. The scratch set has no name
    /// a flag can give, so it is never found.
    fn named(&self, set: &str) -> Result<usize, EnableError> {
        self.sets
            .iter()
            .position(|meta| meta.name == set)
            .filter(|&index| index != 0)
            .ok_or_else(|| EnableError::UnknownSet(set.to_string()))
    }

    /// Solo `set` (#132): snapshot every set's flag — the scratch set's
    /// included — and enable only `set`. On the soloed set, restore the
    /// snapshot instead. On another set while soloed, move the solo there
    /// and keep the original snapshot. Returns whether a solo is now on.
    ///
    /// Filter flags are untouched throughout: the soloed set shows exactly
    /// the toggles it had. A set that was off comes on the way `Enter`
    /// brings it on, `default` profile and all. Toggling a set by hand while
    /// soloed is drift, as toggling a filter during `!` is; un-solo restores
    /// the snapshot regardless.
    pub fn solo(&mut self, set: usize) -> bool {
        if set == 0 || set >= self.sets.len() || !self.sets[set].listed {
            return self.solo.is_some();
        }
        if let Some(current) = self.solo.take() {
            if current.set == set {
                self.restore(current.snapshot);
                return false;
            }
            self.solo = Some(Solo {
                set,
                snapshot: current.snapshot,
            });
        } else {
            self.solo = Some(Solo {
                set,
                snapshot: self.sets.iter().map(|meta| meta.enabled).collect(),
            });
        }
        let was_enabled = self.sets[set].enabled;
        for (index, meta) in self.sets.iter_mut().enumerate() {
            meta.enabled = index == set;
        }
        if !was_enabled && self.sets[set].profiles.contains_key("default") {
            self.apply_profile(set, "default");
        }
        true
    }

    /// The soloed set, if any.
    #[must_use]
    pub fn soloed(&self) -> Option<usize> {
        self.solo.as_ref().map(|solo| solo.set)
    }

    /// Every set back to its startup state (#132): enabled iff `autoload`
    /// and listed now — the listed state itself is left alone (#282) —
    /// each file filter from its set's `default` profile if there is one and
    /// off otherwise, no solo, no pending `!` capture.
    ///
    /// Scratch filters are not deleted and their flags are left alone: a
    /// reset must never destroy something the user typed. Flags only, and
    /// one key to redo from any state, so it asks no confirmation.
    pub fn reset(&mut self) {
        self.solo = None;
        self.forget_capture();
        self.sets[0].enabled = true;
        for set in 1..self.sets.len() {
            for filter in self.filters.iter_mut().filter(|f| f.set == set) {
                filter.enabled = false;
            }
            self.sets[set].enabled = false;
            if self.sets[set].autoload && self.sets[set].listed {
                self.set_enabled_set(set, true);
            }
        }
    }

    /// Turn the scratch set into a named, enabled set (#131), in memory.
    ///
    /// The new set takes the default priority and a `default` profile of
    /// the scratch filters that are on right now, so it opens the way it
    /// is. Its filters keep their colours — colour is a property of the
    /// filter — and their flags. Other sets keep whatever state they are
    /// in: this is not a reload. Returns `false`, changing nothing, when
    /// the scratch set is empty or a set of that name already exists.
    pub fn adopt_scratch_as(&mut self, name: &str, path: PathBuf) -> bool {
        let count = self.scratch_end();
        if count == 0 || self.sets.iter().any(|meta| meta.name == name) {
            return false;
        }
        let priority = crate::filtersets::DEFAULT_PRIORITY;
        let default: Vec<String> = self.filters[..count]
            .iter()
            .filter(|filter| filter.enabled)
            .map(Filter::display_name)
            .collect();
        let mut profiles = BTreeMap::new();
        if !default.is_empty() {
            profiles.insert("default".to_string(), default);
        }
        // Its place among the named sets: after every set that sorts before
        // it by (priority, name), which is the loader's order.
        let at = self
            .sets
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, meta)| (meta.priority, meta.name.as_str()) > (priority, name))
            .map_or(self.sets.len(), |(index, _)| index);
        self.sets.insert(
            at,
            FilterSet {
                name: name.to_string(),
                origin: Origin::File(path),
                priority,
                autoload: false,
                listed: true,
                enabled: true,
                // `S` writes no description (#284), so the session has none.
                description: None,
                profiles,
            },
        );
        if let Some(solo) = self.solo.as_mut() {
            solo.snapshot.insert(at, false);
            if solo.set >= at {
                solo.set += 1;
            }
        }
        // Renumber: every filter in a set at or past the insertion point
        // moves up one; the scratch filters take the new index and move to
        // the end of the sets before it, keeping the known list contiguous.
        for filter in &mut self.filters {
            if filter.set >= at {
                filter.set += 1;
            }
        }
        let mut adopted: Vec<Filter> = self.filters.drain(..count).collect();
        for filter in &mut adopted {
            filter.set = at;
        }
        let splice_at = self
            .filters
            .iter()
            .position(|filter| filter.set > at)
            .unwrap_or(self.filters.len());
        self.filters.splice(splice_at..splice_at, adopted);
        self.recompile();
        self.forget_capture();
        true
    }

    /// Where the scratch set's filters end: the first filter not in set 0.
    fn scratch_end(&self) -> usize {
        self.filters
            .iter()
            .position(|filter| filter.set != 0)
            .unwrap_or(self.filters.len())
    }

    /// Put a typed filter at the end of the scratch range.
    ///
    /// An insert, not a push: file filters follow the scratch set in the
    /// known list, and the list must stay contiguous by set. Every cached
    /// `Verdict::Included` is a position, so callers re-evaluate — which
    /// they already did, since a push had the same effect on the compiled
    /// set. A pending `!` capture describes a set that no longer exists and
    /// is dropped for the same reason it always was.
    pub(super) fn insert_scratch(&mut self, filter: Filter) {
        let at = self.scratch_end();
        self.filters.insert(at, filter);
        self.recompile();
        self.forget_capture();
    }

    /// Rows the filter pane numbers: the user-authored filters. The status
    /// row's "N filters" is about what the user built, and a built-in row
    /// nobody turned on is not that.
    ///
    /// Distinct from `len`, which counts every filter and is what
    /// `Verdict::Included` indexes into.
    ///
    /// An unlisted set's filters are not counted: the pane has no row for
    /// them (#282).
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.filters
            .iter()
            .filter(|filter| {
                let set = &self.sets[filter.set];
                set.origin != Origin::BuiltIn && set.listed
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{builtin_override, builtin_with_profiles, loaded};
    use super::super::tests::set_with;
    use super::super::*;
    use std::path::PathBuf;

    // ---- sets (#128) -------------------------------------------------------

    fn flags(set: &ActiveFilters, of: usize) -> Vec<bool> {
        set.filters_in(of).map(|(_, f)| f.enabled).collect()
    }

    fn with_default_profile() -> ActiveFilters {
        let mut a = loaded("a", 50, false, &["x", "y", "z"]);
        a.profiles
            .insert("default".into(), vec!["x".into(), "z".into()]);
        a.profiles
            .insert("loud".into(), vec!["x".into(), "y".into(), "z".into()]);
        ActiveFilters::with_sets(None, &[a])
    }

    /// A fresh `ActiveFilters` already has one set: the scratch set, so that
    /// every filter lives in a set and nothing needs a "loose filter" case.
    #[test]
    fn a_new_set_has_the_scratch_set_and_the_built_in_one() {
        let set = ActiveFilters::new();
        assert_eq!(
            set.sets().len(),
            2,
            "scratch, and the built-in definitions set"
        );
        assert_eq!(set.sets()[0].origin, Origin::Scratch);
        assert!(set.sets()[0].enabled);
        assert_eq!(set.sets()[0].name, "");
    }

    #[test]
    fn typed_filters_belong_to_the_scratch_set_and_are_named_by_their_pattern() {
        let set = set_with(&["foo", "bar"]);
        assert_eq!(set.filters_in(0).count(), 2);
        assert_eq!(set.filters()[0].display_name(), "foo");
    }

    /// Loaded filters follow the scratch set in the known list, contiguous
    /// by set, and start disabled.
    #[test]
    fn loaded_sets_follow_scratch_in_the_known_list() {
        let set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);
        assert_eq!(set.sets().len(), 3, "scratch, a, definitions");
        assert_eq!(set.sets()[1].name, "a");
        assert_eq!(
            set.sets()[1].origin,
            Origin::File(PathBuf::from("test/filters.toml"))
        );
        assert_eq!(set.len(), 2);
        assert!(set.filters_in(1).all(|(_, f)| !f.enabled));
        assert_eq!(set.filters()[0].display_name(), "x");
    }

    /// A loaded filter's colour is its known-list position in the palette —
    /// the same rule `add` uses — unless the file named one.
    #[test]
    fn loaded_filters_are_coloured_by_position_or_by_the_file() {
        let mut one = loaded("a", 50, false, &["x", "y"]);
        one.filters[1].colour = Some(Color::Red);
        let set = ActiveFilters::with_sets(None, &[one]);
        assert_eq!(
            set.filters()[0].style,
            Style::default().fg(DEFAULT_PALETTE[0])
        );
        assert_eq!(set.filters()[1].style, Style::default().fg(Color::Red));
    }

    // ---- enable_named (#143) ----------------------------------------------

    /// One file set `a` with filters `x`, `y`, `z`; `default` = `x`,
    /// `p` = `y`, `z`.
    fn with_profiles() -> ActiveFilters {
        let mut set = loaded("a", 50, false, &["x", "y", "z"]);
        set.profiles
            .insert("default".to_string(), vec!["x".to_string()]);
        set.profiles
            .insert("p".to_string(), vec!["y".to_string(), "z".to_string()]);
        ActiveFilters::with_sets(None, &[set])
    }

    fn enabled_names(set: &ActiveFilters) -> Vec<String> {
        set.filters_in(1)
            .filter(|(_, filter)| filter.enabled)
            .map(|(_, filter)| filter.display_name())
            .collect()
    }

    #[test]
    fn enable_named_turns_the_set_on_and_applies_default() {
        let mut set = with_profiles();

        set.enable_named("a", None).expect("known set");

        assert!(set.sets()[1].enabled);
        assert_eq!(enabled_names(&set), ["x"]);
    }

    #[test]
    fn enable_named_without_a_default_moves_no_flag() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);

        set.enable_named("a", None).expect("known set");

        assert!(set.sets()[1].enabled);
        assert!(
            enabled_names(&set).is_empty(),
            "no default profile: the set comes on with the toggles it had"
        );
    }

    #[test]
    fn enable_named_applies_the_named_profile_instead_of_default() {
        let mut set = with_profiles();

        set.enable_named("a", Some("p"))
            .expect("known set and profile");

        assert!(set.sets()[1].enabled);
        assert_eq!(enabled_names(&set), ["y", "z"]);
    }

    #[test]
    fn enable_named_refuses_an_unknown_name_and_changes_nothing() {
        let mut set = with_profiles();

        assert_eq!(
            set.enable_named("b", None),
            Err(EnableError::UnknownSet("b".to_string()))
        );
        assert_eq!(
            set.enable_named("a", Some("nope")),
            Err(EnableError::UnknownProfile {
                set: "a".to_string(),
                profile: "nope".to_string(),
            })
        );
        assert_eq!(
            set.enable_named("", None),
            Err(EnableError::UnknownSet(String::new())),
            "the scratch set has no name and is never enabled this way"
        );
        assert!(!set.sets()[1].enabled, "a refused call enables nothing");
        assert!(enabled_names(&set).is_empty());
    }

    #[test]
    fn autoload_sets_start_enabled() {
        let set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 50, true, &["x"]),
                loaded("b", 50, false, &["y"]),
            ],
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
        set.add_excluding("second").expect("valid");
        assert_eq!(set.filters()[0].display_name(), "typed");
        assert_eq!(set.filters()[1].display_name(), "second");
        assert_eq!(set.filters()[0].set, 0);
        assert_eq!(set.filters()[2].set, 1);
        assert_eq!(
            set.filters()[0].style,
            Style::default().fg(DEFAULT_PALETTE[2])
        );
        // The compiled set follows the new order: `typed` is index 0.
        set.set_enabled(0, true);
        assert_eq!(set.verdict("typed", KindSet::EMPTY), Verdict::Included(0));
    }

    /// The rule: a filter takes effect when it is enabled *and* its set is.
    #[test]
    fn a_filter_in_a_disabled_set_matches_nothing() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["foo"])]);
        set.set_enabled(0, true);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Included(0));
        set.set_enabled_set(1, false);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);
        assert_eq!(
            set.verdict_by_scanning("foo", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert!(
            set.filters()[0].enabled,
            "the filter's own flag is untouched"
        );
    }

    /// Exclusion follows the rule too: an exclude in a disabled set removes
    /// nothing.
    #[test]
    fn an_exclude_in_a_disabled_set_removes_nothing() {
        let mut a = loaded("a", 50, true, &["foo", "skip"]);
        a.filters[1].sense = Sense::Exclude;
        let mut set = ActiveFilters::with_sets(None, &[a]);
        set.set_all_enabled(true);
        assert_eq!(set.verdict("foo skip", KindSet::EMPTY), Verdict::Excluded);
        set.set_enabled_set(1, false);
        assert_eq!(set.verdict("foo skip", KindSet::EMPTY), Verdict::Unmatched);
        assert!(!set.any_excluding());
    }

    /// The navigator's masks and dimming follow the same rule.
    #[test]
    fn the_matcher_and_dimming_ignore_filters_in_disabled_sets() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["foo"])]);
        set.set_enabled(0, true);
        assert!(set.matcher().is_some());
        assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE));
        set.set_enabled_set(1, false);
        assert!(
            set.matcher().is_none(),
            "nothing selects, so no scan to run"
        );
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    /// `!` is flag-level: it disables and restores across every known filter
    /// and never touches a set's flag.
    #[test]
    fn bang_acts_on_filter_flags_across_sets_and_leaves_set_flags_alone() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x"])]);
        set.add("typed").expect("valid");
        set.set_enabled(1, true); // x
        set.disable_all_remembering();
        assert_eq!(flags(&set, 0), vec![false]);
        assert_eq!(flags(&set, 1), vec![false]);
        assert!(set.sets()[1].enabled);
        set.restore_remembered();
        assert_eq!(flags(&set, 0), vec![true]);
        assert_eq!(flags(&set, 1), vec![true]);
    }

    /// Enabling a set with a `default` profile applies it.
    #[test]
    fn enabling_a_set_applies_its_default_profile() {
        let mut set = with_default_profile();
        assert!(set.set_enabled_set(1, true));
        assert_eq!(flags(&set, 1), vec![true, false, true]);
    }

    /// `autoload` with no `default` starts the set enabled and every filter
    /// off: `autoload` names the sets that are live at startup, `default`
    /// names their filters, and a set without one names none. Decided on
    /// #161 and documented in the README's `autoload` and `Reset`
    /// paragraphs; pinned here so the alternative that issue offered —
    /// switching every filter on — cannot arrive by accident.
    #[test]
    fn autoload_without_a_default_starts_enabled_with_every_filter_off() {
        let set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x", "y"])]);
        assert!(set.sets()[1].enabled, "the set is enabled");
        assert_eq!(
            flags(&set, 1),
            vec![false, false],
            "and its filters are off"
        );
        assert!(!set.any_enabled(), "nothing filters until a row is toggled");
    }

    /// `autoload` goes through the same path, so `default` applies at startup.
    #[test]
    fn autoload_applies_default() {
        let mut a = loaded("a", 50, true, &["x", "y"]);
        a.profiles.insert("default".into(), vec!["y".into()]);
        let set = ActiveFilters::with_sets(None, &[a]);
        assert_eq!(flags(&set, 1), vec![false, true]);
    }

    /// Without `default`, enabling keeps whatever flags the filters had, and
    /// disabling touches none.
    #[test]
    fn enabling_a_set_without_default_keeps_the_flags() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);
        set.set_enabled(1, true);
        set.set_enabled_set(1, true);
        assert_eq!(flags(&set, 1), vec![false, true]);
        set.set_enabled_set(1, false);
        assert_eq!(flags(&set, 1), vec![false, true]);
        assert_eq!(set.toggle_set(1), Some(true));
        assert_eq!(flags(&set, 1), vec![false, true]);
    }

    /// A profile enables exactly its members and disables the rest of the set.
    #[test]
    fn a_profile_is_exact_and_touches_one_set() {
        let mut set = with_default_profile();
        set.add("typed").expect("valid");
        set.set_enabled_set(1, true);
        assert!(set.apply_profile(1, "loud"));
        assert_eq!(flags(&set, 1), vec![true, true, true]);
        assert!(set.apply_profile(1, "default"));
        assert_eq!(flags(&set, 1), vec![true, false, true]);
        assert!(!set.apply_profile(1, "nope"));
        assert_eq!(flags(&set, 0), vec![true], "the scratch set is untouched");
    }

    /// The scratch set cannot be toggled through this path.
    #[test]
    fn the_scratch_set_is_not_toggleable() {
        let mut set = set_with(&["foo"]);
        assert!(!set.set_enabled_set(0, false));
        assert_eq!(set.toggle_set(0), None);
        assert_eq!(set.toggle_set(7), None);
        assert!(set.sets()[0].enabled);
    }

    // ---- solo and reset (#132) ---------------------------------------------

    fn three_sets() -> ActiveFilters {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 10, true, &["x"]),
                loaded("b", 20, true, &["y"]),
                loaded("c", 30, false, &["z"]),
            ],
        );
        set.add("scratch").expect("valid");
        set
    }

    fn set_flags(set: &ActiveFilters) -> Vec<bool> {
        set.sets().iter().map(|meta| meta.enabled).collect()
    }

    #[test]
    fn solo_enables_one_set_and_suspends_the_rest_including_scratch() {
        let mut set = three_sets();
        assert!(set.solo(2));
        assert_eq!(set_flags(&set), vec![false, false, true, false, false]);
        assert_eq!(set.soloed(), Some(2));
        assert!(
            set.filters().iter().all(|f| f.enabled || f.set != 0),
            "scratch filter flags are untouched"
        );
        assert_eq!(
            set.verdict("scratch", KindSet::EMPTY),
            Verdict::Unmatched,
            "the scratch set is suspended, not just hidden"
        );
    }

    #[test]
    fn solo_again_restores_the_snapshot() {
        let mut set = three_sets();
        set.solo(2);
        assert!(!set.solo(2));
        assert_eq!(set_flags(&set), vec![true, true, true, false, false]);
        assert_eq!(set.soloed(), None);
    }

    #[test]
    fn moving_the_solo_keeps_the_first_snapshot() {
        let mut set = three_sets();
        set.solo(2);
        assert!(set.solo(3));
        assert_eq!(set_flags(&set), vec![false, false, false, true, false]);
        assert!(!set.solo(3));
        assert_eq!(set_flags(&set), vec![true, true, true, false, false]);
    }

    #[test]
    fn soloing_a_disabled_set_applies_its_default() {
        let mut c = loaded("c", 30, false, &["z", "w"]);
        c.profiles.insert("default".into(), vec!["w".into()]);
        let mut set = ActiveFilters::with_sets(None, &[c]);
        set.solo(1);
        assert_eq!(flags(&set, 1), vec![false, true]);
    }

    #[test]
    fn solo_refuses_the_scratch_set_and_bad_indices() {
        let mut set = three_sets();
        assert!(!set.solo(0));
        assert!(!set.solo(9));
        assert_eq!(set_flags(&set), vec![true, true, true, false, false]);
        set.solo(1);
        assert!(set.solo(0), "still soloed after a refused press");
    }

    /// `!` and solo are independent: `!` inside a solo acts on filter flags,
    /// and restores them, while the set flags stay soloed.
    #[test]
    fn bang_inside_a_solo_acts_on_filter_flags_only() {
        let mut set = three_sets();
        set.set_enabled(1, true); // x
        set.solo(1);
        set.disable_all_remembering();
        assert_eq!(flags(&set, 1), vec![false]);
        assert_eq!(set.soloed(), Some(1));
        set.restore_remembered();
        assert_eq!(flags(&set, 1), vec![true]);
        assert_eq!(set_flags(&set), vec![false, true, false, false, false]);
    }

    #[test]
    fn reset_returns_every_set_to_startup_and_leaves_scratch_alone() {
        let mut a = loaded("a", 10, true, &["x", "y"]);
        a.profiles.insert("default".into(), vec!["x".into()]);
        let mut set = ActiveFilters::with_sets(None, &[a, loaded("b", 20, false, &["z"])]);
        set.add("scratch").expect("valid");
        set.set_enabled(0, false); // scratch off, by hand
        set.solo(2);
        set.set_enabled(3, true); // z
        set.set_enabled(1, false); // x, contrary to default
        set.disable_all_remembering();
        set.reset();
        assert_eq!(set_flags(&set), vec![true, true, false, false]);
        assert_eq!(flags(&set, 1), vec![true, false]);
        assert_eq!(flags(&set, 2), vec![false]);
        assert!(!set.filters()[0].enabled, "scratch flag untouched");
        assert_eq!(set.filters_in(0).count(), 1, "scratch filter not deleted");
        assert_eq!(set.soloed(), None);
        assert!(!set.has_remembered());
    }

    // ---- adopting the scratch set (#131) -----------------------------------

    #[test]
    fn adopt_moves_scratch_into_a_new_enabled_set_with_default_from_the_flags() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("m", 10, true, &["q"]),
                loaded("z", 90, false, &["r"]),
            ],
        );
        set.add("a").expect("valid");
        set.add("b").expect("valid");
        set.set_enabled(1, false); // b off
        let a_style = set.filters()[0].style;
        assert!(set.adopt_scratch_as("new", PathBuf::from("t")));
        assert_eq!(set.filters_in(0).count(), 0);
        let names: Vec<&str> = set.sets().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["", "m", "definitions", "new", "z"],
            "priority 50 sorts between 10 and 90"
        );
        let new = 3;
        assert!(set.sets()[new].enabled);
        assert_eq!(set.sets()[new].priority, 50);
        assert_eq!(set.sets()[new].profiles["default"], vec!["a".to_string()]);
        assert_eq!(flags(&set, new), vec![true, false]);
        // Known list stays contiguous by set: m's q, then a, b, then z's r.
        // The built-in set's rows sit between m's and new's; count them
        // rather than name them, since the kinds may grow.
        let order: Vec<(usize, String)> = set
            .filters()
            .iter()
            .filter(|f| set.sets()[f.set].origin != Origin::BuiltIn)
            .map(|f| (f.set, f.display_name()))
            .collect();
        assert_eq!(set.filters_in(2).count(), Kind::ALL.len());
        assert_eq!(
            order,
            vec![
                (1, "q".into()),
                (3, "a".into()),
                (3, "b".into()),
                (4, "r".into())
            ]
        );
        assert_eq!(
            set.filters()[1 + Kind::ALL.len()].style,
            a_style,
            "colour travels with the filter"
        );
        assert_eq!(
            set.verdict("a", KindSet::EMPTY),
            Verdict::Included(1 + Kind::ALL.len())
        );
        assert!(
            !set.adopt_scratch_as("new", PathBuf::from("t")),
            "name taken"
        );
        assert!(
            !set.adopt_scratch_as("other", PathBuf::from("t")),
            "scratch is empty now"
        );
    }

    /// A live solo's snapshot stays aligned with the set list.
    #[test]
    fn adopt_keeps_a_live_solo_aligned() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("z", 90, true, &["r"])]);
        set.add("a").expect("valid");
        set.solo(2); // z: the sets are "", definitions, z
        assert!(set.adopt_scratch_as("new", PathBuf::from("t")));
        assert_eq!(set.soloed(), Some(3), "z moved from 2 to 3");
        set.solo(3);
        let flags: Vec<bool> = set.sets().iter().map(|s| s.enabled).collect();
        assert_eq!(
            flags,
            vec![true, false, false, true],
            "the snapshot restored z as it was and new as off"
        );
    }

    // ---- the built-in definitions set (#127) -------------------------------

    fn builtin_index(set: &ActiveFilters) -> usize {
        set.sets()
            .iter()
            .position(|meta| meta.origin == Origin::BuiltIn)
            .expect("the built-in set is always present")
    }

    #[test]
    fn the_definitions_set_is_always_present_and_collapsed_by_default() {
        let set = ActiveFilters::new();
        assert_eq!(set.sets().len(), 2, "scratch and definitions");
        let index = builtin_index(&set);
        let meta = &set.sets()[index];
        assert_eq!(meta.name, DEFINITIONS_SET);
        assert!(!meta.enabled);
        assert!(!meta.autoload);
        assert_eq!(meta.priority, crate::filtersets::DEFAULT_PRIORITY);
        let kinds: Vec<String> = set
            .filters_in(index)
            .map(|(_, f)| f.display_name())
            .collect();
        let expected: Vec<&str> = Kind::ALL.iter().map(|kind| kind.plural()).collect();
        assert_eq!(kinds, expected);
        assert_eq!(kinds[..4], ["functions", "classes", "structs", "enums"]);
        assert!(set.filters_in(index).all(|(_, f)| !f.enabled));
        assert!(
            set.filters_in(index)
                .all(|(_, f)| matches!(f.predicate, Predicate::Definition(_)))
        );
    }

    /// Built-in filters take no palette colour and do not move the next one.
    #[test]
    fn builtin_filters_do_not_consume_the_palette() {
        let mut set = ActiveFilters::new();
        set.add("typed").expect("valid");
        assert_eq!(
            set.filters()[0].style,
            Style::default().fg(DEFAULT_PALETTE[0])
        );
        let index = builtin_index(&set);
        assert!(
            set.filters_in(index)
                .all(|(_, f)| f.style == Style::default())
        );
        assert!(set.is_user_authored(0));
        assert!(!set.is_user_authored(1));
    }

    /// The built-in set sorts among file sets by priority and name.
    #[test]
    fn the_definitions_set_sorts_among_file_sets() {
        let set = ActiveFilters::with_sets(
            None,
            &[
                loaded("alpha", 50, false, &["a"]),
                loaded("zeta", 50, false, &["z"]),
            ],
        );
        let names: Vec<&str> = set.sets().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["", "alpha", "definitions", "zeta"]);
    }

    /// A `[sets.definitions]` table positions and switches it, and nothing
    /// else: its filters are still recon's four.
    #[test]
    fn a_file_override_positions_and_switches_the_definitions_set() {
        let set = ActiveFilters::with_sets(
            None,
            &[
                loaded("alpha", 50, false, &["a"]),
                builtin_override(10, true),
            ],
        );
        let index = builtin_index(&set);
        assert_eq!(index, 1, "priority 10 sorts before alpha");
        assert!(set.sets()[index].enabled, "autoload");
        assert_eq!(set.filters_in(index).count(), Kind::ALL.len());
        assert_eq!(set.sets().len(), 3, "no second definitions set");
    }

    /// A `[sets.definitions]` table's profiles are the set's (#220): the
    /// `default` one applies on autoload, another applies by name, and the
    /// members are the kinds' plural names.
    #[test]
    fn a_file_override_gives_the_definitions_set_profiles() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[builtin_with_profiles(
                true,
                &[
                    ("default", &["functions"]),
                    ("types", &["types", "structs", "enums"]),
                ],
            )],
        );
        let index = builtin_index(&set);
        assert_eq!(set.sets()[index].profiles.len(), 2);
        let on = |set: &ActiveFilters| -> Vec<String> {
            set.filters_in(index)
                .filter(|(_, f)| f.enabled)
                .map(|(_, f)| f.display_name())
                .collect()
        };
        assert_eq!(on(&set), ["functions"], "autoload applied `default`");
        assert!(set.apply_profile(index, "types"));
        assert_eq!(
            on(&set),
            ["structs", "enums", "types"],
            "in Kind::ALL order"
        );
        assert_eq!(
            set.enable_named(DEFINITIONS_SET, Some("nope")),
            Err(EnableError::UnknownProfile {
                set: DEFINITIONS_SET.to_string(),
                profile: "nope".to_string(),
            })
        );
    }

    /// The grammar pass is paid only once a definition filter takes effect.
    #[test]
    fn needs_kinds_is_false_until_a_definition_filter_is_effective() {
        let mut set = ActiveFilters::new();
        assert!(!set.needs_kinds(), "present is not effective");
        let index = builtin_index(&set);
        set.set_enabled(0, true); // functions, but the set is collapsed
        assert!(!set.needs_kinds());
        set.set_enabled_set(index, true);
        assert!(set.needs_kinds());
    }

    /// The regex pass is paid only once a regex filter of any sense takes
    /// effect. The built-in set alone never asks for it.
    #[test]
    fn needs_regex_is_false_until_a_regex_filter_is_effective() {
        let mut set = ActiveFilters::new();
        assert!(!set.needs_regex(), "the built-in set is all definitions");

        set.add("foo").expect("valid pattern");
        assert!(set.needs_regex());
        set.set_all_enabled(false);
        assert!(!set.needs_regex(), "present is not effective");

        // An excluding filter selects nothing, and `matcher` says so with
        // `None`; it still runs over every line, so it counts here.
        set.add_excluding("noise").expect("valid pattern");
        assert!(set.needs_regex());
        set.set_all_enabled(false);
        assert!(!set.needs_regex());
    }

    /// With no regex to run, a definition filter is still answered from the
    /// kinds, and the definition filters the user left off still say
    /// nothing — the guard changes what is asked, not what is answered.
    #[test]
    fn a_definition_filter_alone_is_answered_without_the_regex_pass() {
        let mut set = ActiveFilters::new();
        let index = builtin_index(&set);
        set.set_enabled(0, true); // functions
        set.set_enabled_set(index, true);
        assert!(!set.needs_regex());

        let functions = KindSet::of(&[Kind::Function]);
        assert_eq!(
            set.verdict("anything at all", functions),
            Verdict::Included(0)
        );
        let structs = KindSet::of(&[Kind::Struct]);
        assert_eq!(set.verdict("a struct line", structs), Verdict::Unmatched);
        assert_eq!(
            set.verdict("plain text", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    /// Solo, reset and `!` treat it as any set.
    #[test]
    fn the_definitions_set_is_an_ordinary_set_to_solo_and_reset() {
        let mut set = ActiveFilters::new();
        set.add("typed").expect("valid");
        let index = builtin_index(&set);
        assert!(set.solo(index));
        assert!(set.sets()[index].enabled);
        assert!(!set.sets()[0].enabled);
        set.reset();
        assert!(!set.sets()[index].enabled, "autoload is off");
        assert!(set.sets()[0].enabled);
    }

    // ---- listed and unlisted sets (#282) -----------------------------------

    fn unlisted(mut set: LoadedSet) -> LoadedSet {
        set.listed = false;
        set
    }

    #[test]
    fn a_set_is_listed_unless_the_file_says_otherwise() {
        let set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x"])]);
        assert!(
            set.sets().iter().all(|meta| meta.listed),
            "scratch, a, definitions"
        );
        assert!(set.sets()[1].enabled);
    }

    /// `listed = false` wins over `autoload = true` (ADR 0002): the set is
    /// known, and decides nothing.
    #[test]
    fn an_unlisted_autoload_set_starts_unlisted_and_disabled() {
        let set = ActiveFilters::with_sets(None, &[unlisted(loaded("a", 50, true, &["x"]))]);
        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(set.matcher().is_none(), "its filter selects nothing");
        assert_eq!(set.row_count(), 0, "and has no row");
    }

    #[test]
    fn unlisting_an_enabled_set_disables_it_and_keeps_its_flags() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 10, true, &["x", "y"]),
                loaded("b", 20, false, &["z"]),
            ],
        );
        set.set_enabled(0, true);
        set.set_enabled(1, true);
        assert!(set.needs_regex());

        assert!(set.set_listed(1, false));
        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(!set.needs_regex(), "the visible lines change at once");
        assert_eq!(flags(&set, 1), vec![true, true], "flags kept");

        assert!(set.set_listed(1, true));
        assert_eq!(set.sets()[1].name, "a", "back in its priority position");
        assert!(set.sets()[1].listed);
        assert!(!set.sets()[1].enabled, "listing again gives a disabled set");
        assert_eq!(flags(&set, 1), vec![true, true], "with the flags it had");
    }

    #[test]
    fn an_unlisted_set_cannot_be_enabled_toggled_or_soloed() {
        let mut set = ActiveFilters::with_sets(None, &[unlisted(loaded("a", 50, false, &["x"]))]);
        assert!(!set.set_enabled_set(1, true));
        assert_eq!(set.toggle_set(1), None);
        assert!(!set.solo(1));
        assert_eq!(set.soloed(), None);
        assert!(!set.sets()[1].enabled);
        assert!(set.set_enabled_set(1, false), "disabling is always allowed");
    }

    #[test]
    fn the_scratch_set_cannot_be_unlisted() {
        let mut set = ActiveFilters::new();
        assert!(!set.set_listed(0, false));
        assert!(set.sets()[0].listed);
        assert!(!set.set_listed(99, false), "no such set");
    }

    /// `--set NAME` lists the set and enables it, `default` profile and all,
    /// whatever the file says.
    #[test]
    fn enable_named_lists_an_unlisted_set() {
        let mut a = unlisted(loaded("a", 50, false, &["x", "y"]));
        a.profiles.insert("default".into(), vec!["x".into()]);
        let mut set = ActiveFilters::with_sets(None, &[a]);

        set.enable_named("a", None).expect("known set");

        assert!(set.sets()[1].listed);
        assert!(set.sets()[1].enabled);
        assert_eq!(enabled_names(&set), ["x"]);
    }

    /// `--unlist NAME` (#283): the set has no row and decides nothing, even
    /// with `autoload`.
    #[test]
    fn unlist_named_unlists_an_autoload_set() {
        let mut a = loaded("a", 50, true, &["x"]);
        a.profiles.insert("default".into(), vec!["x".into()]);
        let mut set = ActiveFilters::with_sets(None, &[a]);
        assert!(set.matcher().is_some(), "autoload turned `x` on");

        set.unlist_named("a").expect("known set");

        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(set.matcher().is_none(), "its filter selects nothing");
        assert_eq!(set.row_count(), 0, "and has no row");
    }

    #[test]
    fn unlist_named_refuses_an_unknown_name_and_the_scratch_set() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x"])]);
        let scratch = set.sets()[0].name.clone();

        assert_eq!(
            set.unlist_named("b"),
            Err(EnableError::UnknownSet("b".to_string()))
        );
        assert_eq!(
            set.unlist_named(&scratch),
            Err(EnableError::UnknownSet(scratch.clone()))
        );
        assert!(set.sets().iter().all(|meta| meta.listed), "nothing moved");
    }

    #[test]
    fn un_solo_restores_only_sets_that_are_still_listed() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[loaded("a", 10, true, &["x"]), loaded("b", 20, true, &["y"])],
        );
        set.solo(1);
        assert!(set.set_listed(2, false));
        assert!(!set.solo(1), "un-solo");
        assert!(set.sets()[1].enabled);
        assert!(!set.sets()[2].listed, "the snapshot does not list it");
        assert!(!set.sets()[2].enabled, "and so does not enable it");
    }

    #[test]
    fn unlisting_the_soloed_set_ends_the_solo() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[loaded("a", 10, true, &["x"]), loaded("b", 20, true, &["y"])],
        );
        set.solo(1);
        assert!(set.set_listed(1, false));
        assert_eq!(set.soloed(), None);
        assert!(!set.sets()[1].enabled);
        assert!(
            set.sets()[2].enabled,
            "the rest come back from the snapshot"
        );
    }

    /// `R` does not change the listed state, and enables an autoload set
    /// only if it is listed now.
    #[test]
    fn reset_keeps_the_listed_state_and_autoloads_only_listed_sets() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 10, true, &["x"]),
                unlisted(loaded("b", 20, true, &["y"])),
            ],
        );
        set.set_listed(1, false);
        set.set_listed(2, true);
        set.reset();
        assert!(!set.sets()[1].listed, "still unlisted");
        assert!(!set.sets()[1].enabled, "so autoload does not apply");
        assert!(set.sets()[2].listed, "still listed");
        assert!(set.sets()[2].enabled, "listed now, so autoload applies");
    }

    #[test]
    fn the_definitions_set_can_be_unlisted() {
        let set = ActiveFilters::with_sets(
            None,
            &[unlisted(builtin_override(
                crate::filtersets::DEFAULT_PRIORITY,
                true,
            ))],
        );
        assert_eq!(set.sets()[1].origin, Origin::BuiltIn);
        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(!set.needs_kinds());

        let mut set = ActiveFilters::new();
        assert!(set.set_listed(1, false), "and unlisted in a session");
        assert!(!set.sets()[1].listed);
    }

    // ---- descriptions (#284) -----------------------------------------------

    #[test]
    fn the_definitions_set_has_a_builtin_description_the_table_can_override() {
        let set = ActiveFilters::new();
        assert_eq!(
            set.sets()[1].description.as_deref(),
            Some(DEFINITIONS_DESCRIPTION)
        );

        let mut table = builtin_override(crate::filtersets::DEFAULT_PRIORITY, false);
        table.description = Some("mine".into());
        let set = ActiveFilters::with_sets(None, &[table]);
        assert_eq!(set.sets()[1].description.as_deref(), Some("mine"));
    }

    #[test]
    fn a_file_set_carries_its_description() {
        let mut a = loaded("a", 50, false, &["x"]);
        a.description = Some("logs".into());
        let set = ActiveFilters::with_sets(None, &[a, loaded("b", 60, false, &["y"])]);
        let described = |name: &str| {
            let meta = set.sets().iter().find(|meta| meta.name == name);
            meta.expect("known").description.clone()
        };
        assert_eq!(described("a").as_deref(), Some("logs"));
        assert_eq!(described("b"), None);
    }
}
