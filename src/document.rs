//! The loaded file and what the filters made of it.

use crate::filter::{FilterSet, Verdict};
use ratatui::style::Style;

/// A loaded file, with a cached verdict per line.
///
/// Evaluating a filter set is O(lines × filters), which is not free on a large
/// log, so verdicts are computed when the lines or the filters change rather
/// than once per frame. `match_count` is cached alongside the verdicts for the
/// same reason: `render` reads it every frame (via the status line), and
/// rescanning the whole verdict vector at redraw rate would scale with the
/// file rather than with how often the filters actually change.
#[derive(Debug, Default)]
pub struct Document {
    lines: Vec<String>,
    verdicts: Vec<Verdict>,
    match_count: usize,
}

impl Document {
    pub fn new(lines: Vec<String>) -> Self {
        let verdicts = vec![Verdict::Unmatched; lines.len()];
        Self {
            lines,
            verdicts,
            match_count: 0,
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn verdicts(&self) -> &[Verdict] {
        &self.verdicts
    }

    /// Recompute every line's verdict. Call when the lines or the filters change.
    pub fn evaluate(&mut self, filters: &FilterSet) {
        self.verdicts = self
            .lines
            .iter()
            .map(|line| filters.verdict(line))
            .collect();
        self.match_count = self
            .verdicts
            .iter()
            .filter(|verdict| matches!(verdict, Verdict::Included(_)))
            .count();
    }

    /// How many lines an including filter selected.
    pub fn match_count(&self) -> usize {
        self.match_count
    }

    /// One style slot per line, for `FileView::set_line_styles`.
    ///
    /// Always covers every line, so a shorter vector can never leave trailing
    /// lines wearing styles computed for a previously loaded file.
    pub fn line_styles(&self, filters: &FilterSet) -> Vec<Option<Style>> {
        self.verdicts
            .iter()
            .map(|verdict| filters.style_for(*verdict))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    fn doc(lines: &[&str]) -> Document {
        Document::new(lines.iter().map(|l| l.to_string()).collect())
    }

    fn set_with(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    #[test]
    fn a_new_document_has_a_verdict_for_every_line() {
        let document = doc(&["one", "two", "three"]);

        assert_eq!(document.verdicts().len(), document.lines().len());
    }

    #[test]
    fn evaluating_records_each_line_s_verdict() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);

        document.evaluate(&filters);

        assert_eq!(
            document.verdicts(),
            &[Verdict::Unmatched, Verdict::Included(0), Verdict::Unmatched]
        );
    }

    #[test]
    fn re_evaluating_replaces_the_previous_verdicts() {
        let mut document = doc(&["alpha", "beta"]);
        document.evaluate(&set_with(&["beta"]));

        document.evaluate(&set_with(&["alpha"]));

        assert_eq!(
            document.verdicts(),
            &[Verdict::Included(0), Verdict::Unmatched]
        );
    }

    #[test]
    fn match_count_reports_included_lines_only() {
        let mut document = doc(&["foo a", "bar", "foo b"]);
        document.evaluate(&set_with(&["foo"]));

        assert_eq!(document.match_count(), 2);
    }

    /// The vector handed to the textarea has one entry per line, so no line is
    /// left to fall through to whatever the previous file's styles were.
    #[test]
    fn line_styles_covers_every_line() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_eq!(styles.len(), 2);
    }

    #[test]
    fn matching_lines_take_their_filter_s_colour_and_others_dim() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_eq!(styles[1].expect("beta styled").fg, filters.filters()[0].style.fg);
        assert!(
            styles[0]
                .expect("alpha styled")
                .add_modifier
                .contains(Modifier::DIM),
            "unmatched line not dimmed"
        );
    }

    /// Without filters nothing is dimmed, so an ordinary file looks ordinary.
    #[test]
    fn an_unfiltered_document_styles_nothing() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = FilterSet::new();
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert!(styles.iter().all(Option::is_none));
    }

    #[test]
    fn two_filters_colour_their_lines_differently() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["alpha", "beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_ne!(styles[0].unwrap().fg, styles[1].unwrap().fg);
    }
}
