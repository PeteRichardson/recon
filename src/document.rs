//! The loaded file and what the filters made of it.

use crate::filter::{FilterSet, Verdict};
use ratatui::style::Style;

/// Which lines the file view shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Every line that no excluding filter removed; unmatched lines are dimmed.
    #[default]
    Dimmed,
    /// Only lines an including filter selected.
    FilteredOnly,
}

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
    mode: Mode,
    visible: Vec<usize>,
}

impl Document {
    pub fn new(lines: Vec<String>) -> Self {
        let verdicts = vec![Verdict::Unmatched; lines.len()];
        Self {
            lines,
            verdicts,
            match_count: 0,
            mode: Mode::default(),
            visible: Vec::new(),
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
        self.recompute_visible();
    }

    /// Recompute `visible` from the existing verdicts and the current mode,
    /// without re-running the filters.
    ///
    /// A verdict depends on the lines and the filters; `visible` depends only
    /// on the verdicts and the mode. So toggling the mode (`H` / `Ctrl-H`)
    /// only needs this, not a full `evaluate` — which matters, since
    /// `evaluate` is O(lines × filters) and this is O(lines).
    pub fn recompute_visible(&mut self) {
        self.visible = self
            .verdicts
            .iter()
            .enumerate()
            .filter(|(_, verdict)| match (self.mode, verdict) {
                // Excluded lines are gone in both modes; the toggle governs
                // unmatched lines only.
                (_, Verdict::Excluded) => false,
                (Mode::Dimmed, _) => true,
                (Mode::FilteredOnly, Verdict::Included(_)) => true,
                (Mode::FilteredOnly, Verdict::Unmatched) => false,
            })
            .map(|(index, _)| index)
            .collect();
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

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Change which lines are shown. The caller must re-`evaluate` (or, if
    /// the verdicts have not changed, just `recompute_visible`) afterwards.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Source line indices currently on screen, in order.
    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// The text of the visible lines, for rebuilding the view's buffer.
    pub fn visible_lines(&self) -> Vec<String> {
        self.visible
            .iter()
            .map(|&source| self.lines[source].clone())
            .collect()
    }

    /// One style slot per *visible* line, aligned with `visible_lines`.
    pub fn visible_styles(&self, filters: &FilterSet) -> Vec<Option<Style>> {
        self.visible
            .iter()
            .map(|&source| filters.style_for(self.verdicts[source]))
            .collect()
    }

    /// Where a source line sits in the visible list, if it is shown at all.
    pub fn visible_position(&self, source: usize) -> Option<usize> {
        self.visible.binary_search(&source).ok()
    }

    /// The source index of the visible row at `visible_row`.
    pub fn source_at(&self, visible_row: usize) -> Option<usize> {
        self.visible.get(visible_row).copied()
    }

    /// The nearest visible source line at or after `source`, falling back to
    /// the last one before it.
    ///
    /// Used when a mode change hides the line the cursor was on: snapping
    /// forward lands on the match the user was navigating towards, and the
    /// backward fallback stops the cursor being lost when nothing follows.
    pub fn nearest_visible(&self, source: usize) -> Option<usize> {
        match self.visible.binary_search(&source) {
            Ok(_) => Some(source),
            Err(index) => self
                .visible
                .get(index)
                .copied()
                .or_else(|| self.visible.last().copied()),
        }
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

    fn set_excluding(patterns: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in patterns {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    #[test]
    fn dimmed_mode_shows_every_line_that_is_not_excluded() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[0, 1, 2]);
    }

    /// Excluded lines are gone in both modes — the toggle governs unmatched
    /// lines only.
    #[test]
    fn excluded_lines_are_hidden_even_when_dimmed() {
        let mut document = doc(&["alpha", "noise", "gamma"]);
        document.evaluate(&set_excluding(&["noise"]));

        assert_eq!(document.mode(), Mode::Dimmed);
        assert_eq!(document.visible(), &[0, 2]);
    }

    #[test]
    fn filtered_only_mode_shows_matches_alone() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[1, 3]);
    }

    #[test]
    fn visible_lines_are_the_text_of_the_visible_indices() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_lines(), vec!["beta".to_string()]);
    }

    #[test]
    fn visible_styles_line_up_with_visible_lines() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        assert_eq!(
            document.visible_styles(&filters).len(),
            document.visible().len()
        );
    }

    #[test]
    fn source_and_visible_positions_map_both_ways() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_position(3), Some(1));
        assert_eq!(document.source_at(1), Some(3));
        assert_eq!(document.visible_position(0), None, "line 0 is hidden");
    }

    /// Toggling into filtered mode from a hidden line snaps forward to the
    /// next match, which is what the user was navigating towards.
    #[test]
    fn nearest_visible_snaps_forward() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.nearest_visible(0), Some(1));
        assert_eq!(document.nearest_visible(2), Some(3));
    }

    /// With no match after it, fall back to the one before rather than losing
    /// the cursor entirely.
    #[test]
    fn nearest_visible_falls_back_to_the_previous_match() {
        let mut document = doc(&["beta", "alpha", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.nearest_visible(2), Some(0));
    }

    #[test]
    fn nearest_visible_is_none_when_nothing_is_visible() {
        let mut document = doc(&["alpha", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["zzz"]));

        assert!(document.visible().is_empty());
        assert_eq!(document.nearest_visible(0), None);
    }

    /// `set_mode` records the mode but does not recompute anything: `visible`
    /// catches up on the next `evaluate`. The task that toggles modes relies on
    /// that ordering, because it captures the cursor's source line against the
    /// *old* mapping before rebuilding.
    #[test]
    fn set_mode_alone_does_not_change_what_is_visible() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);
        assert_eq!(document.visible(), &[0, 1, 2]);

        document.set_mode(Mode::FilteredOnly);

        assert_eq!(document.mode(), Mode::FilteredOnly, "the mode was not recorded");
        assert_eq!(
            document.visible(),
            &[0, 1, 2],
            "visible changed before evaluate was called"
        );

        document.evaluate(&filters);
        assert_eq!(document.visible(), &[1], "visible did not catch up");
    }

    /// The point of splitting `recompute_visible` out of `evaluate`: the mode
    /// toggle can refresh `visible` alone, without redoing the verdict pass
    /// (the expensive part on a large document).
    #[test]
    fn recompute_visible_updates_visible_without_rerunning_the_filters() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);
        let verdicts_before = document.verdicts().to_vec();

        document.set_mode(Mode::FilteredOnly);
        document.recompute_visible();

        assert_eq!(document.visible(), &[1], "visible did not pick up the new mode");
        assert_eq!(
            document.verdicts(),
            verdicts_before.as_slice(),
            "recompute_visible must not touch the verdicts"
        );
    }
}
