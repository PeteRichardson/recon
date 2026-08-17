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

const DIM_STYLE: Style = Style::new()
    .fg(Color::Indexed(DIM_GREY))
    .add_modifier(Modifier::DIM);

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

#[derive(Debug, Default)]
pub struct FilterSet {
    filters: Vec<Filter>,
    /// Enabled flags captured by `disable_all_remembering`, awaiting a restore.
    ///
    /// Held separately from the filters so that a filter removed in the
    /// meantime simply drops out of the restore rather than resurrecting.
    remembered: Option<Vec<bool>>,
}

impl FilterSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

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
        Ok(())
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
        if let Some(remembered) = self.remembered.as_mut() {
            if index < remembered.len() {
                remembered.remove(index);
            }
        }
        true
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
        for filter in &mut self.filters {
            filter.enabled = false;
        }
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
    }

    pub fn has_remembered(&self) -> bool {
        self.remembered.is_some()
    }

    pub fn any_enabled(&self) -> bool {
        self.filters.iter().any(|filter| filter.enabled)
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

        self.filters
            .iter()
            .enumerate()
            .find(|(_, filter)| {
                filter.enabled
                    && filter.sense == Sense::Include
                    && filter.pattern.is_match(line)
            })
            .map_or(Verdict::Unmatched, |(index, _)| Verdict::Included(index))
    }

    /// Whether any enabled filter selects lines, as opposed to removing them.
    ///
    /// Dimming means "this line matched no include filter". With only
    /// excluding filters there is nothing to dim against, so the file reads
    /// normally minus the removed lines.
    fn any_including(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense == Sense::Include)
    }

    /// The style to render a line with, or `None` to leave it alone.
    ///
    /// `Unmatched` dims only when some including filter is actually active —
    /// otherwise every line of an unfiltered file would be dimmed.
    pub fn style_for(&self, verdict: Verdict) -> Option<Style> {
        match verdict {
            Verdict::Included(index) => self.filters.get(index).map(|f| f.style),
            Verdict::Unmatched if self.any_including() => Some(DIM_STYLE),
            Verdict::Unmatched | Verdict::Excluded => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    /// With no filters at all, nothing is dimmed — a plain file reads normally.
    #[test]
    fn an_empty_set_leaves_every_line_unmatched() {
        let set = FilterSet::new();

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
        let mut set = FilterSet::new();

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
        let mut set = FilterSet::new();
        set.add("a").expect("valid");
        set.add("b").expect("valid");

        assert_ne!(
            set.filters()[0].style,
            set.filters()[1].style,
            "two filters would be indistinguishable"
        );
    }

    fn set_excluding(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
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
        let mut set = FilterSet::new();

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
        assert!(!set.filters()[1].enabled, "a filter the user had off came back on");
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
        let mut set = FilterSet::new();

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

    /// Adding a filter while a restore is pending is not part of the `!`
    /// workflow, but it must not corrupt state either. `add` always creates
    /// an enabled filter, and the captured flags are shorter than the
    /// filters by one, so `zip` in `restore_remembered` pairs the capture
    /// only with the filters that existed when it was taken — the new filter
    /// has nothing to restore it to, so it is left exactly as added:
    /// enabled, regardless of every other filter being forced off around it.
    #[test]
    fn adding_while_a_restore_is_pending_leaves_the_new_filter_enabled() {
        let mut set = set_with(&["foo"]);
        set.disable_all_remembering();

        set.add("bar").expect("valid pattern");
        assert!(
            set.filters()[1].enabled,
            "new filters are always added enabled"
        );

        set.restore_remembered();

        assert!(set.filters()[0].enabled, "restored to what it was before");
        assert!(
            set.filters()[1].enabled,
            "nothing was captured for it, so it is left as added"
        );
    }
}
