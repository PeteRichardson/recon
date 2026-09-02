//! The loaded file and what the filters made of it.

use crate::filter::{ActiveFilters, Verdict};
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
/// Evaluating a filter set is not free on a large log, so verdicts are computed
/// when the lines or the filters change rather than once per frame.
///
/// There is deliberately no cached `match_count`. One used to sit here,
/// documented as read by the status line every frame — it was not: the status
/// row reports lines *shown*, counted from `visible`, and the two comments in
/// `lib.rs` that mention `match_count` both say why it is the wrong number
/// (it counts `Included` and `Searched` verdicts, so it read "0 matched" with
/// only excluding filters active). Nothing outside this file ever called the
/// getter (#77).
#[derive(Debug, Default)]
pub struct Document {
    lines: Vec<String>,
    verdicts: Vec<Verdict>,
    /// Whether anything was marking lines at the last `evaluate` — a numbered
    /// including filter, or the live search.
    ///
    /// Cached rather than asked of the `ActiveFilters` inside
    /// `recompute_visible`, so that method keeps taking no arguments and stays
    /// independent of the filter set. That independence is what makes the
    /// `Ctrl-H` path cheap: the toggle re-derives `visible` from the verdicts
    /// alone, with no borrow and no regex.
    anything_including: bool,
    mode: Mode,
    visible: Vec<usize>,
}

impl Document {
    #[must_use]
    pub fn new(lines: Vec<String>) -> Self {
        let verdicts = vec![Verdict::Unmatched; lines.len()];
        Self {
            lines,
            verdicts,
            anything_including: false,
            mode: Mode::default(),
            visible: Vec::new(),
        }
    }

    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    #[must_use]
    pub fn verdicts(&self) -> &[Verdict] {
        &self.verdicts
    }

    /// Recompute every line's verdict. Call when the lines or the filters change.
    pub fn evaluate(&mut self, filters: &ActiveFilters) {
        self.verdicts = self
            .lines
            .iter()
            .map(|line| filters.verdict(line))
            .collect();
        self.anything_including = filters.any_including();
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
                (Mode::FilteredOnly, Verdict::Included(_) | Verdict::Searched) => true,
                // Issue #36: with nothing including, there is nothing to hide
                // *against*, so hiding shows the file rather than blanking the
                // pane. Dimming has always had this guard in `style_for`;
                // hiding never did, which made `Ctrl-H` with no filters — and
                // with only excluding filters — produce an empty view that read
                // as "this file is empty".
                (Mode::FilteredOnly, Verdict::Unmatched) => !self.anything_including,
            })
            .map(|(index, _)| index)
            .collect();
    }

    /// One style slot per line, for `FileView::set_line_styles`.
    ///
    /// Always covers every line, so a shorter vector can never leave trailing
    /// lines wearing styles computed for a previously loaded file.
    #[cfg(test)]
    #[must_use]
    pub fn line_styles(&self, filters: &ActiveFilters) -> Vec<Option<Style>> {
        self.verdicts
            .iter()
            .map(|verdict| filters.style_for(*verdict))
            .collect()
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Change which lines are shown. The caller must re-`evaluate` (or, if
    /// the verdicts have not changed, just `recompute_visible`) afterwards.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Source line indices currently on screen, in order.
    #[must_use]
    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// The text of the visible lines, for rebuilding the view's buffer.
    #[cfg(test)]
    #[must_use]
    pub fn visible_lines(&self) -> Vec<String> {
        self.visible_lines_range(0, self.visible.len())
    }

    /// The text of visible rows `start..end`, for rebuilding the view's buffer
    /// from a window rather than the whole visible set.
    ///
    /// The range is clamped rather than panicking on a stale bound: the window
    /// is computed from a pane height captured on the previous frame, and a
    /// filter change between frames can shorten the visible set under it. A
    /// short buffer for one frame is a cosmetic glitch; an index panic takes
    /// the whole TUI down.
    #[must_use]
    pub fn visible_lines_range(&self, start: usize, end: usize) -> Vec<String> {
        self.window(start, end)
            .iter()
            .map(|&source| self.lines[source].clone())
            .collect()
    }

    /// One style slot per *visible* line, aligned with `visible_lines`.
    #[cfg(test)]
    #[must_use]
    pub fn visible_styles(&self, filters: &ActiveFilters) -> Vec<Option<Style>> {
        self.visible_styles_range(filters, 0, self.visible.len())
    }

    /// One style slot per row of visible `start..end`, aligned with
    /// `visible_lines_range` over the same bounds.
    ///
    /// Windowing this matters more than it looks: unlike the gutter numbers,
    /// the style vector was never gated on hiding, so an unfiltered million-line
    /// file rebuilt a million-entry vector on every navigator arrow key.
    #[must_use]
    pub fn visible_styles_range(
        &self,
        filters: &ActiveFilters,
        start: usize,
        end: usize,
    ) -> Vec<Option<Style>> {
        self.window(start, end)
            .iter()
            .map(|&source| filters.style_for(self.verdicts[source]))
            .collect()
    }

    /// `visible[start..end]`, with both bounds clamped to the visible set.
    fn window(&self, start: usize, end: usize) -> &[usize] {
        let end = end.min(self.visible.len());
        let start = start.min(end);
        &self.visible[start..end]
    }

    /// One flag per *visible* line, aligned with `visible_lines`: whether the
    /// source line after it is hidden.
    ///
    /// Hiding unmatched lines collapses a file into groups of consecutive
    /// matches with nothing on screen to say how much was skipped between
    /// them, so ten matched lines either side of a hundred hidden ones read as
    /// twenty consecutive lines (issue #2). A set flag is where the view draws
    /// the boundary.
    ///
    /// "The next source line is hidden" rather than "another group follows",
    /// so a group running into trailing hidden lines is marked like any other
    /// — the file really does continue below it. The last line of the file has
    /// no next line and is never marked, which is what keeps an unfiltered
    /// document unmarked throughout.
    #[cfg(test)]
    #[must_use]
    pub fn visible_group_ends(&self) -> Vec<bool> {
        self.visible_group_ends_range(0, self.visible.len())
    }

    /// `visible_group_ends` over visible rows `start..end` only.
    ///
    /// **Peeks one row past `end`**, which is the whole reason this is not a
    /// slice of the full vector. A row's mark asks "is the next source line
    /// hidden?", and for the final *visible* row it means "does the file
    /// continue below?". Sliced naively, the window's last row would be
    /// mistaken for the document's last row and marked wrong — a gap marker
    /// appearing or vanishing purely because of where the window happens to
    /// stop. Reading `visible[end]` when it exists keeps every mark
    /// independent of the window.
    #[must_use]
    pub fn visible_group_ends_range(&self, start: usize, end: usize) -> Vec<bool> {
        let end = end.min(self.visible.len());
        let start = start.min(end);
        (start..end)
            .map(|row| {
                let source = self.visible[row];
                match self.visible.get(row + 1) {
                    Some(&next) => next != source + 1,
                    // Nothing visible after it: a gap only if the file continues.
                    None => source + 1 < self.lines.len(),
                }
            })
            .collect()
    }

    /// Where a source line sits in the visible list, if it is shown at all.
    #[must_use]
    pub fn visible_position(&self, source: usize) -> Option<usize> {
        self.visible.binary_search(&source).ok()
    }

    /// The source index of the visible row at `visible_row`.
    #[must_use]
    pub fn source_at(&self, visible_row: usize) -> Option<usize> {
        self.visible.get(visible_row).copied()
    }

    /// The nearest visible source line at or after `source`, falling back to
    /// the last one before it.
    ///
    /// Used when a mode change hides the line the cursor was on: snapping
    /// forward lands on the match the user was navigating towards, and the
    /// backward fallback stops the cursor being lost when nothing follows.
    #[must_use]
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
        Document::new(lines.iter().map(std::string::ToString::to_string).collect())
    }

    fn set_with(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    fn set_searching(pattern: &str) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        set.set_search(pattern).expect("valid pattern");
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

    // ---- windowed accessors (#7) ---------------------------------------

    /// Everything visible, which is what `visible_lines_range` and friends are
    /// windowing over. `Document::new` leaves `visible` empty until something
    /// evaluates, so an unfiltered pass is the "no filters yet" baseline.
    fn shown(lines: &[&str]) -> Document {
        let mut document = doc(lines);
        document.evaluate(&ActiveFilters::new());
        document
    }

    #[test]
    fn visible_lines_range_returns_only_the_window() {
        let document = shown(&["a", "b", "c", "d", "e"]);

        assert_eq!(
            document.visible_lines_range(1, 4),
            vec!["b".to_string(), "c".to_string(), "d".to_string()]
        );
    }

    /// The window is computed from a pane height captured on the previous
    /// frame, so a filter change can shorten the visible set under it. Clamping
    /// costs a short buffer for one frame; panicking takes the TUI down.
    #[test]
    fn visible_lines_range_clamps_a_stale_end() {
        let document = shown(&["a", "b"]);

        assert_eq!(document.visible_lines_range(1, 999), vec!["b".to_string()]);
    }

    #[test]
    fn visible_lines_range_clamps_a_start_past_the_end() {
        let document = shown(&["a", "b"]);

        assert!(document.visible_lines_range(9, 999).is_empty());
    }

    #[test]
    fn visible_styles_range_lines_up_with_its_window() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let windowed = document.visible_styles_range(&filters, 1, 3);

        assert_eq!(windowed.len(), 2);
        assert_eq!(windowed, document.visible_styles(&filters)[1..3].to_vec());
    }

    /// The regression this range method exists to prevent. Row 1 is followed by
    /// a *hidden* line, so it is a group end — and must stay one when the
    /// window stops right after it. Slicing a whole-set vector would give the
    /// same answer; computing the range in isolation, without peeking at
    /// `visible[end]`, would treat row 1 as the document's last row and ask the
    /// wrong question.
    #[test]
    fn visible_group_ends_range_peeks_past_the_window() {
        let mut document = doc(&["keep", "keep", "drop", "keep"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["keep"]));

        // Visible rows are sources 0, 1, 3 — source 2 is hidden.
        assert_eq!(document.visible(), &[0, 1, 3]);

        let windowed = document.visible_group_ends_range(0, 2);

        assert_eq!(
            windowed,
            vec![false, true],
            "row 1 is followed by a hidden line and must be marked a group end \
             even though the window stops there"
        );
        assert_eq!(windowed, document.visible_group_ends()[0..2].to_vec());
    }

    #[test]
    fn visible_group_ends_range_clamps_a_stale_end() {
        let document = shown(&["a", "b"]);

        assert_eq!(document.visible_group_ends_range(0, 999).len(), 2);
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

        assert_eq!(
            styles[1].expect("beta styled").fg,
            filters.filters()[0].style.fg
        );
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
        let filters = ActiveFilters::new();
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

    fn set_excluding(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
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

    /// Issue #36. With nothing including, there is nothing to hide against, so
    /// hiding shows the file rather than blanking the pane. Dimming has always
    /// had this guard (`style_for`); hiding never did.
    #[test]
    fn hiding_with_no_filters_shows_the_whole_file() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&ActiveFilters::new());

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
        document.evaluate(&ActiveFilters::new());

        document.set_mode(Mode::FilteredOnly);
        document.recompute_visible();

        assert_eq!(document.visible(), &[0, 1]);
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

    /// Issue #2. Hiding collapses a file into groups of consecutive matches
    /// separated by invisible gaps; this is what marks where a group stops.
    #[test]
    fn a_group_ends_where_the_next_source_line_is_hidden() {
        let mut document = doc(&["beta", "beta", "alpha", "beta", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[0, 1, 3, 4]);
        assert_eq!(
            document.visible_group_ends(),
            vec![false, true, false, false]
        );
    }

    /// The mark means "the next source line is not shown", so a group running
    /// to the end of the file has nothing after it to mark.
    #[test]
    fn the_last_line_of_the_file_never_ends_a_group() {
        let mut document = doc(&["alpha", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_group_ends(), vec![false]);
    }

    /// Trailing hidden lines are a gap like any other — the group really does
    /// stop there, and the rest of the file is below it.
    #[test]
    fn a_group_ends_where_the_rest_of_the_file_is_hidden() {
        let mut document = doc(&["beta", "alpha"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_group_ends(), vec![true]);
    }

    #[test]
    fn nothing_ends_a_group_when_every_line_is_visible() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_group_ends(), vec![false; 3]);
    }

    #[test]
    fn group_ends_line_up_with_visible_lines() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(
            document.visible_group_ends().len(),
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

        assert_eq!(
            document.mode(),
            Mode::FilteredOnly,
            "the mode was not recorded"
        );
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

        assert_eq!(
            document.visible(),
            &[1],
            "visible did not pick up the new mode"
        );
        assert_eq!(
            document.verdicts(),
            verdicts_before.as_slice(),
            "recompute_visible must not touch the verdicts"
        );
    }
}
