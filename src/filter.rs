//! Filters decide how each line of the viewed file is presented.
//!
//! A filter set describes a *log format* rather than a document, so it outlives
//! any one file. Matching is by regular expression, the same as search, so
//! `^foo` anchors to the start of a line.

use ratatui::style::{Color, Modifier, Style};
use regex::Regex;

/// Colours assigned to successive filters, so two filters are never
/// indistinguishable. Wraps once exhausted.
const PALETTE: [Color; 6] = [
    Color::Yellow,
    Color::Cyan,
    Color::Green,
    Color::Magenta,
    Color::Blue,
    Color::Red,
];

/// How lines matching no including filter are rendered.
///
/// `Modifier::DIM` alone is not enough: it emits the terminal's "faint"
/// attribute, which many terminals ignore outright, leaving dimmed lines
/// indistinguishable from matched ones. An explicit grey is what actually
/// produces the contrast; the modifier is kept for terminals that do honour it.
///
/// The colour is the 256-colour greyscale ramp rather than `DarkGray`, so the
/// shade does not depend on the terminal's theme. **Lower is darker** — adjust
/// `DIM_GREY` to taste: 244 is subtle, 240 clear, 236 heavy. The ramp runs from
/// 232 (near-black) to 255 (near-white).
const DIM_GREY: u8 = 240;

pub(crate) const DIM_STYLE: Style = Style::new()
    .fg(Color::Indexed(DIM_GREY))
    .add_modifier(Modifier::DIM);

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
pub(crate) const SEARCH_STYLE: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);

/// Whether a filter selects lines or removes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sense {
    Include,
    Exclude,
}

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

#[derive(Debug)]
pub struct Filter {
    pub pattern: Regex,
    pub sense: Sense,
    pub enabled: bool,
    pub style: Style,
}

/// Every enabled flag in an [`ActiveFilters`], captured so it can be restored.
///
/// Opaque on purpose: it is a token to hand back to
/// [`ActiveFilters::apply_enabled_flags`], not a structure to read or build.
/// Positions in it are meaningless without the set it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledFlags {
    filters: Vec<bool>,
    search: Option<bool>,
}

#[derive(Debug, Default)]
pub struct ActiveFilters {
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

impl ActiveFilters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether there are any *numbered* filters. The live search is not one
    /// of these — see `row_count`, which counts it and is what the pane
    /// sizes itself against.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// How many *numbered* filters there are. The live search is not one of
    /// these — see `row_count`, which counts it and is what the pane sizes
    /// itself against.
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    /// The colour the next filter added will take.
    fn next_style(&self) -> Style {
        Style::default().fg(PALETTE[self.filters.len() % PALETTE.len()])
    }

    /// Add an including filter, colouring it distinctly from its
    /// predecessors. A pattern that will not compile is rejected and the set
    /// left untouched.
    pub fn add(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let compiled = Regex::new(pattern)?;
        let style = self.next_style();
        self.filters.push(Filter {
            pattern: compiled,
            sense: Sense::Include,
            enabled: true,
            style,
        });
        // A pending capture describes a set that no longer exists. Keeping it
        // would strand it: `!` would see an enabled filter, try to capture,
        // find one already pending, and do nothing at all — forever.
        self.forget_capture();
        Ok(())
    }

    /// Add an excluding filter: its matches are removed from view entirely,
    /// in both display modes.
    ///
    /// Excluding filters carry no colour, since a line they match is never
    /// rendered.
    pub fn add_excluding(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.filters.push(Filter {
            pattern,
            sense: Sense::Exclude,
            enabled: true,
            style: Style::default(),
        });
        // A pending capture describes a set that no longer exists. Keeping it
        // would strand it: `!` would see an enabled filter, try to capture,
        // find one already pending, and do nothing at all — forever.
        self.forget_capture();
        Ok(())
    }

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
    ///
    /// Reports whether there was a search to drop, the same shape as
    /// `promote_search`: a caller can skip the `refresh_view` it would
    /// otherwise pay for on every press — see the `Esc` binding.
    pub fn clear_search(&mut self) -> bool {
        let had_search = self.search.take().is_some();
        self.forget_capture();
        had_search
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

    /// Whether any enabled filter removes lines.
    pub fn any_excluding(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense == Sense::Exclude)
    }

    /// Enable or disable every filter at once, for the `!` toggle.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            filter.enabled = enabled;
        }
        if let Some(search) = self.search.as_mut() {
            search.enabled = enabled;
        }
    }

    /// Remove the filter at `index`, reporting whether it existed.
    ///
    /// Indices are positional, so this renumbers every later filter. Any
    /// cached `Verdict::Included` is invalid afterwards — callers must
    /// re-evaluate rather than patch.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.filters.len() {
            return false;
        }
        self.filters.remove(index);
        if let Some(remembered) = self.remembered.as_mut()
            && index < remembered.len()
        {
            remembered.remove(index);
        }
        true
    }

    /// Replace one filter's pattern, keeping everything else about it —
    /// reporting whether it existed.
    ///
    /// The filter stays at `index`, which is the point: `verdict` returns the
    /// *first* match, so a position is a precedence, and `style_for` looks a
    /// colour up by the same number. Deleting and re-adding — the only way to
    /// change a pattern before this existed — put the replacement at the end
    /// and silently reordered the set.
    ///
    /// Compiles before it mutates, the same discipline `add` follows: a
    /// pattern that will not compile leaves the old one in place, so the
    /// prompt has something intact to stay open over.
    ///
    /// Deliberately does **not** `forget_capture`. Every other mutator here
    /// drops a pending `!` capture because it changes the set's *shape* —
    /// `remembered` is a `Vec<bool>` aligned to `filters` by position, so an
    /// add or a remove invalidates it. An edit changes neither the length nor
    /// any enabled flag, so the capture still describes this set exactly and
    /// dropping it would strand a restore for nothing.
    ///
    /// Callers must re-evaluate: the verdicts cached against the old pattern
    /// are stale. Unlike `remove`, only *this* filter's verdicts can have
    /// changed — the numbering is untouched — but `Document::evaluate` is the
    /// only thing that recomputes them, so a full pass is what a caller owes.
    pub fn set_pattern(&mut self, index: usize, pattern: &str) -> Result<bool, regex::Error> {
        let compiled = Regex::new(pattern)?;
        match self.filters.get_mut(index) {
            Some(filter) => {
                filter.pattern = compiled;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Enable or disable one filter, reporting whether it existed.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool {
        match self.filters.get_mut(index) {
            Some(filter) => {
                filter.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Flip one filter, returning its new state, or `None` if there is no
    /// such filter.
    ///
    /// Distinguishing the two matters: a caller cannot otherwise tell "turned
    /// off" from "that row is gone", and the pane's selection can lag a
    /// deletion by a frame.
    pub fn toggle_enabled(&mut self, index: usize) -> Option<bool> {
        let filter = self.filters.get_mut(index)?;
        filter.enabled = !filter.enabled;
        Some(filter.enabled)
    }

    /// Disable every filter, recording which were enabled.
    ///
    /// A second call before a restore is ignored: the flags at that point are
    /// the ones this method just cleared, so capturing them again would
    /// overwrite the real state with all-disabled and lose it for good.
    pub fn disable_all_remembering(&mut self) {
        if self.remembered.is_some() {
            return;
        }
        self.remembered = Some(self.filters.iter().map(|f| f.enabled).collect());
        self.remembered_search = self.search.as_ref().map(|search| search.enabled);
        self.set_all_enabled(false);
    }

    /// Put back exactly the state `disable_all_remembering` captured.
    ///
    /// Enabling everything instead would silently switch on filters the user
    /// had deliberately turned off.
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

    pub fn has_remembered(&self) -> bool {
        self.remembered.is_some()
    }

    /// Capture every enabled flag for a caller to hold and hand back later.
    ///
    /// Deliberately *not* `disable_all_remembering`, even though the peek `App`
    /// uses this for (#48) also turns everything off. That method owns a single
    /// internal slot which `!` already uses; a peek writing to it would
    /// overwrite a capture `!` was still holding, and `!` would then restore
    /// all-disabled for the rest of the session. Two independent undo stacks
    /// need two independent captures, so this one lives with its caller.
    pub fn enabled_flags(&self) -> EnabledFlags {
        EnabledFlags {
            filters: self.filters.iter().map(|filter| filter.enabled).collect(),
            search: self.search.as_ref().map(|search| search.enabled),
        }
    }

    /// Put back what [`enabled_flags`](Self::enabled_flags) captured.
    ///
    /// `zip` rather than an index, and the same tolerance `restore_remembered`
    /// has: a filter deleted since the capture simply drops out of the restore
    /// rather than resurrecting, and one added since keeps whatever it has now.
    /// A snapshot is a convenience, not a transaction.
    pub fn apply_enabled_flags(&mut self, flags: &EnabledFlags) {
        for (filter, was_enabled) in self.filters.iter_mut().zip(&flags.filters) {
            filter.enabled = *was_enabled;
        }
        if let (Some(search), Some(was_enabled)) = (self.search.as_mut(), flags.search) {
            search.enabled = was_enabled;
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.filters.iter().any(|filter| filter.enabled)
            || self.search.as_ref().is_some_and(|search| search.enabled)
    }

    /// Decide how `line` should be presented.
    ///
    /// A set with no enabled including filters leaves every line `Unmatched`,
    /// so an empty or fully disabled set renders an ordinary, undimmed file
    /// rather than a wholly dimmed one. The first matching filter wins, which
    /// is what makes the set's order meaningful.
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

    /// Whether anything at all is marking lines — a numbered including filter,
    /// or the live search.
    ///
    /// Drives hiding (including the `Ctrl-H` guard in `Document`) and `n`/`N`.
    /// Public because `Document` caches it at `evaluate` time.
    pub fn any_including(&self) -> bool {
        self.any_numbered_including() || self.search.as_ref().is_some_and(|search| search.enabled)
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

    /// The style to render a line with, or `None` to leave it alone.
    ///
    /// `Unmatched` dims only when a *numbered* including filter is active. The
    /// live search does not trigger dimming on its own; see `any_numbered_including`
    /// for why.
    pub fn style_for(&self, verdict: Verdict) -> Option<Style> {
        match verdict {
            Verdict::Included(index) => self.filters.get(index).map(|f| f.style),
            Verdict::Searched => Some(SEARCH_STYLE),
            Verdict::Unmatched if self.any_numbered_including() => Some(DIM_STYLE),
            Verdict::Unmatched | Verdict::Excluded => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    /// With no filters at all, nothing is dimmed — a plain file reads normally.
    #[test]
    fn an_empty_set_leaves_every_line_unmatched() {
        let set = ActiveFilters::new();

        assert_eq!(set.verdict("anything"), Verdict::Unmatched);
        assert!(set.is_empty());
    }

    #[test]
    fn a_matching_line_is_included_with_its_filter_index() {
        let set = set_with(&["foo", "bar"]);

        assert_eq!(set.verdict("a bar line"), Verdict::Included(1));
    }

    #[test]
    fn a_non_matching_line_is_unmatched() {
        let set = set_with(&["foo"]);

        assert_eq!(set.verdict("nothing here"), Verdict::Unmatched);
    }

    /// Order in the set decides the colour, so the first match wins.
    #[test]
    fn the_first_matching_filter_wins() {
        let set = set_with(&["foo", "foo.*bar"]);

        assert_eq!(set.verdict("foo and bar"), Verdict::Included(0));
    }

    #[test]
    fn patterns_are_regular_expressions() {
        let set = set_with(&[r"^\d+ms$"]);

        assert_eq!(set.verdict("250ms"), Verdict::Included(0));
        assert_eq!(set.verdict("took 250ms"), Verdict::Unmatched);
    }

    #[test]
    fn an_invalid_pattern_is_reported() {
        let mut set = ActiveFilters::new();

        assert!(set.add("[").is_err());
        assert!(set.is_empty(), "a rejected pattern must not be added");
    }

    #[test]
    fn a_disabled_filter_does_not_match() {
        let mut set = set_with(&["foo"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
        assert!(!set.any_enabled());
    }

    /// `!` disables every filter, then a second press re-enables them all —
    /// not a restore of whatever per-filter state existed before, since
    /// nothing here can disable a filter individually yet.
    #[test]
    fn disabling_and_restoring_round_trips() {
        let mut set = set_with(&["foo"]);
        assert!(set.any_enabled());

        set.set_all_enabled(false);
        set.set_all_enabled(true);

        assert_eq!(set.verdict("foo"), Verdict::Included(0));
    }

    /// A set whose filters are all disabled behaves like an empty one: an
    /// undimmed file, not a fully dimmed one.
    #[test]
    fn a_fully_disabled_set_leaves_lines_unmatched() {
        let mut set = set_with(&["foo"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("bar"), Verdict::Unmatched);
    }

    #[test]
    fn successive_filters_get_distinct_colours() {
        let mut set = ActiveFilters::new();
        set.add("a").expect("valid");
        set.add("b").expect("valid");

        assert_ne!(
            set.filters()[0].style,
            set.filters()[1].style,
            "two filters would be indistinguishable"
        );
    }

    fn set_excluding(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    #[test]
    fn an_excluding_filter_excludes_its_matches() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.verdict("a heartbeat line"), Verdict::Excluded);
    }

    /// Excluding filters run after including ones, so exclusion wins even on a
    /// line an including filter selected.
    #[test]
    fn exclusion_beats_inclusion_on_the_same_line() {
        let mut set = set_with(&["foo"]);
        set.add_excluding("noisy").expect("valid pattern");

        assert_eq!(set.verdict("foo but noisy"), Verdict::Excluded);
        assert_eq!(set.verdict("foo alone"), Verdict::Included(0));
    }

    /// With only excluding filters, unmatched lines stay ordinary — there is
    /// nothing to dim against.
    #[test]
    fn excluding_filters_alone_do_not_dim() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.verdict("something else"), Verdict::Unmatched);
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    #[test]
    fn a_disabled_excluding_filter_excludes_nothing() {
        let mut set = set_excluding(&["heartbeat"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("a heartbeat line"), Verdict::Unmatched);
    }

    #[test]
    fn an_invalid_excluding_pattern_is_reported() {
        let mut set = ActiveFilters::new();

        assert!(set.add_excluding("[").is_err());
        assert!(set.is_empty(), "a rejected pattern must not be added");
    }

    #[test]
    fn any_excluding_reports_whether_one_is_enabled() {
        let mut set = set_with(&["foo"]);
        assert!(!set.any_excluding());

        set.add_excluding("bar").expect("valid pattern");
        assert!(set.any_excluding());

        set.set_all_enabled(false);
        assert!(!set.any_excluding(), "a disabled filter does not count");
    }

    /// Dimming must set a foreground colour, not just the DIM attribute: many
    /// terminals ignore the attribute entirely, and on those a "dimmed" line
    /// would be indistinguishable from a matched one.
    #[test]
    fn dimming_sets_a_colour_rather_than_only_an_attribute() {
        let set = set_with(&["foo"]);

        let style = set.style_for(Verdict::Unmatched).expect("unmatched dims");

        assert!(
            style.fg.is_some(),
            "dimming relies on the DIM attribute alone, which many terminals ignore"
        );
        assert!(style.add_modifier.contains(Modifier::DIM));
    }

    /// An excluded line is never rendered, so it has no style.
    #[test]
    fn an_excluded_line_has_no_style() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.style_for(Verdict::Excluded), None);
    }

    #[test]
    fn removing_a_filter_drops_it() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.remove(0));

        assert_eq!(set.len(), 1);
        assert_eq!(set.verdict("bar line"), Verdict::Included(0));
    }

    /// Indices are positional, so removing a filter renumbers the ones after
    /// it. Any verdict cached against the old numbering is now wrong, which is
    /// why callers must re-evaluate rather than patch.
    #[test]
    fn removing_a_filter_renumbers_the_rest() {
        let mut set = set_with(&["foo", "bar"]);
        assert_eq!(set.verdict("bar line"), Verdict::Included(1));

        set.remove(0);

        assert_eq!(set.verdict("bar line"), Verdict::Included(0));
    }

    #[test]
    fn removing_out_of_range_reports_failure_and_changes_nothing() {
        let mut set = set_with(&["foo"]);

        assert!(!set.remove(5));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn a_single_filter_can_be_disabled() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.set_enabled(0, false));

        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
        assert_eq!(set.verdict("bar line"), Verdict::Included(1));
    }

    #[test]
    fn toggle_flips_one_filter_and_reports_its_new_state() {
        let mut set = set_with(&["foo"]);

        assert_eq!(
            set.toggle_enabled(0),
            Some(false),
            "was enabled, so is now disabled"
        );
        assert_eq!(set.toggle_enabled(0), Some(true), "and back on");
    }

    /// `toggle_enabled` must distinguish "turned off" from "no such filter" —
    /// both would otherwise report `false`, and the pane's selection can lag
    /// a deletion by a frame.
    #[test]
    fn toggling_a_missing_index_reports_none_and_changes_nothing() {
        let mut set = set_with(&["foo"]);

        assert_eq!(set.toggle_enabled(5), None);
        assert!(set.filters()[0].enabled, "nothing should have changed");
    }

    /// `!` must restore what was enabled before, not enable everything —
    /// otherwise it silently switches on filters the user turned off.
    #[test]
    fn disabling_all_remembers_the_previous_state() {
        let mut set = set_with(&["foo", "bar", "baz"]);
        set.set_enabled(1, false);

        set.disable_all_remembering();
        assert!(!set.any_enabled());

        set.restore_remembered();

        assert!(set.filters()[0].enabled);
        assert!(
            !set.filters()[1].enabled,
            "a filter the user had off came back on"
        );
        assert!(set.filters()[2].enabled);
    }

    #[test]
    fn has_remembered_reports_whether_a_restore_is_pending() {
        let mut set = set_with(&["foo"]);
        assert!(!set.has_remembered());

        set.disable_all_remembering();
        assert!(set.has_remembered());

        set.restore_remembered();
        assert!(!set.has_remembered());
    }

    /// Removing a filter while a restore is pending must not resurrect it or
    /// misapply the remembered flags to the wrong filters.
    #[test]
    fn removing_while_disabled_does_not_corrupt_the_restore() {
        let mut set = set_with(&["foo", "bar"]);
        set.disable_all_remembering();

        set.remove(0);
        set.restore_remembered();

        assert_eq!(set.len(), 1);
        assert!(set.filters()[0].enabled);
    }

    /// A second `disable_all_remembering` before a restore must not overwrite
    /// the capture: by then every filter reads disabled (this method just
    /// disabled them), so capturing again would replace the real prior state
    /// with all-false and lose it for good — the exact bug this task exists
    /// to prevent, reached from the other direction.
    #[test]
    fn disabling_all_twice_does_not_overwrite_the_capture() {
        let mut set = set_with(&["foo", "bar"]);
        set.set_enabled(1, false);

        set.disable_all_remembering();
        set.disable_all_remembering();

        set.restore_remembered();

        assert!(set.filters()[0].enabled);
        assert!(
            !set.filters()[1].enabled,
            "a filter the user had off came back on"
        );
    }

    #[test]
    fn removing_down_to_an_empty_set_leaves_it_empty() {
        let mut set = set_with(&["foo"]);

        assert!(set.remove(0));

        assert!(set.is_empty());
        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
    }

    #[test]
    fn removing_from_an_already_empty_set_reports_failure() {
        let mut set = ActiveFilters::new();

        assert!(!set.remove(0));
        assert!(set.is_empty());
    }

    /// With nothing captured, a restore is a no-op — the `else { return; }`
    /// path in `restore_remembered` has no other coverage, and a future
    /// refactor to `.unwrap()` should fail this test rather than panic
    /// unnoticed in production.
    #[test]
    fn restoring_with_nothing_captured_does_nothing() {
        let mut set = set_with(&["foo"]);
        set.set_enabled(0, false);

        set.restore_remembered();

        assert!(
            !set.filters()[0].enabled,
            "nothing was captured, so nothing should change"
        );
        assert!(!set.has_remembered());
    }

    /// Adding a filter while a restore is pending describes a set that no
    /// longer exists, so `add` drops the capture rather than let it strand:
    /// a later `restore_remembered` is a no-op, and every filter — the ones
    /// captured and the one just added — is left exactly as it stood right
    /// after the add.
    #[test]
    fn adding_while_a_restore_is_pending_drops_the_capture() {
        let mut set = set_with(&["foo"]);
        set.disable_all_remembering();

        set.add("bar").expect("valid pattern");
        assert!(
            set.filters()[1].enabled,
            "new filters are always added enabled"
        );
        assert!(
            !set.has_remembered(),
            "adding a filter should drop the now-stale capture"
        );

        set.restore_remembered();

        assert!(
            !set.filters()[0].enabled,
            "nothing was captured any more, so restore is a no-op"
        );
        assert!(set.filters()[1].enabled, "still enabled, exactly as added");
    }

    #[test]
    fn a_search_matches_like_an_including_filter() {
        let mut set = ActiveFilters::new();
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
        let mut set = ActiveFilters::new();
        set.add_excluding("heartbeat").expect("valid pattern");
        set.set_search("timeout").expect("valid pattern");

        assert_eq!(set.verdict("heartbeat timeout"), Verdict::Excluded);
    }

    /// One search at a time, like vim's search register: a second `/` replaces
    /// the first rather than stacking another filter.
    #[test]
    fn setting_a_search_replaces_the_previous_one() {
        let mut set = ActiveFilters::new();
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
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        set.clear_search();

        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
        assert!(set.search().is_none());
    }

    #[test]
    fn an_invalid_search_pattern_is_reported_and_changes_nothing() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");

        assert!(set.set_search("[").is_err());
        assert_eq!(
            set.verdict("foo line"),
            Verdict::Searched,
            "the old search was lost"
        );
    }

    #[test]
    fn a_disabled_search_matches_nothing() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        set.set_all_enabled(false);

        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
    }

    /// The search carries a colour of its own, outside `PALETTE`, so it never
    /// shifts as filters are added and removed.
    #[test]
    fn the_search_style_is_reserved_rather_than_drawn_from_the_palette() {
        assert!(
            !PALETTE
                .iter()
                .any(|colour| SEARCH_STYLE.fg == Some(*colour)),
            "the search colour would move as the palette rotates"
        );
        let mut set = ActiveFilters::new();
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
        let mut set = ActiveFilters::new();
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

    /// Dimming is a contrast mechanism, and a search on its own is one thing to
    /// see: its hits already carry the span highlight, so greying the rest of the
    /// file buys nothing and costs the readability of the context the user
    /// searched in order to reach.
    #[test]
    fn a_search_alone_does_not_dim() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");

        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    /// But it still counts as something to hide against, so `Ctrl-H` works after
    /// a bare search. This is the asymmetry the two predicates exist for.
    #[test]
    fn a_search_alone_still_counts_for_hiding() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");

        assert!(
            set.any_including(),
            "Ctrl-H would have nothing to hide against"
        );
    }

    /// Add a numbered filter and dimming switches on, because now there really
    /// are two things to tell apart. If we then disable the numbered filter,
    /// leaving only the search, dimming must stop — that's the asymmetry.
    /// This pins the difference between the two predicates directly rather than
    /// testing a case where they agree.
    #[test]
    fn a_numbered_filter_alongside_a_search_dims() {
        let mut set = set_with(&["ERROR"]);
        set.set_search("foo").expect("valid pattern");

        assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE));

        set.set_enabled(0, false);
        assert_eq!(
            set.style_for(Verdict::Unmatched),
            None,
            "search alone must not dim, even when one was present"
        );
    }

    #[test]
    fn a_disabled_search_counts_for_neither() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        set.search_set_enabled(false);

        assert!(!set.any_including());
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    /// The probe-and-keep loop: `/` to try a pattern, `p` to keep it, `/` again.
    /// Nothing is retyped.
    #[test]
    fn promoting_moves_the_search_into_the_numbered_set() {
        let mut set = set_with(&["alpha"]);
        set.set_search("beta").expect("valid pattern");

        assert!(set.promote_search());

        assert_eq!(set.len(), 2);
        assert!(
            set.search().is_none(),
            "the slot should be free for the next probe"
        );
        assert_eq!(set.verdict("beta line"), Verdict::Included(1));
    }

    /// `next_style` reads `self.filters.len()`, so it must be called before the
    /// push that grows `filters`, not after — call it after and it reads one
    /// too many and returns `PALETTE[2]` instead of `PALETTE[1]`. A mere
    /// inequality would not catch that swap: `PALETTE[2]` is still unequal to
    /// both filter 0's colour and to `SEARCH_STYLE`. Pinning the exact colour
    /// is the only assertion that sees the off-by-one.
    #[test]
    fn a_promoted_search_takes_the_next_palette_colour() {
        let mut set = set_with(&["alpha"]);
        set.set_search("beta").expect("valid pattern");
        set.promote_search();

        assert_eq!(
            set.filters()[1].style.fg,
            Some(PALETTE[1]),
            "index 1 is where the promoted filter actually lands"
        );
        assert_ne!(
            set.filters()[1].style,
            SEARCH_STYLE,
            "a promoted filter is a keeper, not the live probe"
        );
    }

    /// Promoting must not silently switch on a search the user had toggled off.
    #[test]
    fn promoting_preserves_the_enabled_state() {
        let mut set = ActiveFilters::new();
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
        let mut set = ActiveFilters::new();
        set.set_search("beta").expect("valid pattern");
        set.disable_all_remembering();

        set.promote_search();

        assert!(!set.has_remembered());
    }

    /// The whole point of editing in place rather than deleting and retyping:
    /// the filter keeps its position, so it keeps its colour and its
    /// precedence in `verdict`. Retyping put the replacement at the end and
    /// silently reordered the set.
    #[test]
    fn editing_a_pattern_keeps_the_filter_at_its_index() {
        let mut set = set_with(&["alpha", "beta"]);
        let colour = set.filters()[0].style;

        assert!(set.set_pattern(0, "gamma").expect("valid pattern"));

        assert_eq!(set.len(), 2, "editing must not grow the set");
        assert_eq!(set.verdict("gamma line"), Verdict::Included(0));
        assert_eq!(set.verdict("beta line"), Verdict::Included(1));
        assert_eq!(set.filters()[0].style, colour, "the colour moved with it");
    }

    #[test]
    fn editing_a_pattern_replaces_the_old_one() {
        let mut set = set_with(&["alpha"]);

        set.set_pattern(0, "gamma").expect("valid pattern");

        assert_eq!(
            set.verdict("alpha line"),
            Verdict::Unmatched,
            "the old pattern still matches"
        );
    }

    /// An edit changes the pattern and nothing else — a filter the user had
    /// toggled off must not come back on, and an excluding filter must not
    /// quietly become an including one.
    #[test]
    fn editing_preserves_the_sense_and_the_enabled_state() {
        let mut set = set_excluding(&["heartbeat"]);
        set.set_enabled(0, false);

        set.set_pattern(0, "keepalive").expect("valid pattern");

        assert!(!set.filters()[0].enabled, "a disabled filter came back on");
        assert_eq!(set.filters()[0].sense, Sense::Exclude);

        set.set_enabled(0, true);
        assert_eq!(set.verdict("a keepalive line"), Verdict::Excluded);
    }

    /// The same discipline `add` follows: compile first, mutate second, so a
    /// rejected pattern leaves the previous one intact and the prompt has
    /// something to stay open over.
    #[test]
    fn an_invalid_edit_is_reported_and_leaves_the_filter_untouched() {
        let mut set = set_with(&["alpha"]);

        assert!(set.set_pattern(0, "[").is_err());

        assert_eq!(set.verdict("alpha line"), Verdict::Included(0));
    }

    #[test]
    fn editing_out_of_range_reports_failure_and_changes_nothing() {
        let mut set = set_with(&["alpha"]);

        assert!(!set.set_pattern(5, "gamma").expect("valid pattern"));
        assert_eq!(set.len(), 1);
        assert_eq!(set.verdict("alpha line"), Verdict::Included(0));
    }

    /// Unlike `add` and `remove`, an edit leaves the set's *shape* alone — same
    /// length, same enabled flags — so a pending `!` capture still describes it
    /// exactly and must survive. Dropping it here would strand the restore for
    /// no reason.
    #[test]
    fn editing_keeps_a_pending_capture_valid() {
        let mut set = set_with(&["alpha", "beta"]);
        set.set_enabled(1, false);
        set.disable_all_remembering();

        set.set_pattern(0, "gamma").expect("valid pattern");
        assert!(set.has_remembered(), "the capture is still accurate");

        set.restore_remembered();

        assert!(set.filters()[0].enabled);
        assert!(
            !set.filters()[1].enabled,
            "a filter the user had off came back on"
        );
    }
}
