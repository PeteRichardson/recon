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

/// Whether a filter selects lines or removes them.
///
/// Only `Include` is constructed in this phase; `Exclude` exists so that adding
/// it later does not reshape the model.
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
    /// Removed by an excluding filter. Never produced in this phase.
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

    /// Enable or disable every filter at once, for the `!` toggle.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            filter.enabled = enabled;
        }
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

    /// The style to render a line with, or `None` to leave it alone.
    ///
    /// `Unmatched` dims only when some including filter is actually active —
    /// otherwise every line of an unfiltered file would be dimmed.
    pub fn style_for(&self, verdict: Verdict) -> Option<Style> {
        match verdict {
            Verdict::Included(index) => self.filters.get(index).map(|f| f.style),
            Verdict::Unmatched if self.any_enabled() => {
                Some(Style::default().add_modifier(Modifier::DIM))
            }
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
}
