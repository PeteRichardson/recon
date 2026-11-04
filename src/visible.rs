//! The lines a document shows, as runs of consecutive lines (#572).

/// The source lines on screen, in order, kept as runs of consecutive lines
/// rather than one index for each line.
///
/// A filter that hides few lines, or most of them, gives few runs, so a file
/// of a hundred million lines costs a few runs here, not 800 MB. The worst
/// case, every other line shown, costs 8 bytes a line: what a `Vec<usize>`
/// always cost.
///
/// Two runs never touch: `push` grows the last run when it can. So two sets
/// of the same lines have the same runs, and `==` compares the runs.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct Visible {
    runs: Vec<Run>,
}

/// Lines `start..` of the file, which end at visible row `rows_end`: the
/// run's length is the gap from the run before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    start: usize,
    rows_end: usize,
}

impl Visible {
    /// Add `source` after the last line. It must come after it.
    pub(crate) fn push(&mut self, source: usize) {
        let last = self.runs.len().checked_sub(1);
        if let Some(last) = last {
            let end = self.end(last);
            debug_assert!(source >= end, "{source} is not after {end}");
            if source == end {
                self.runs[last].rows_end += 1;
                return;
            }
        }
        let rows_end = self.len() + 1;
        self.runs.push(Run {
            start: source,
            rows_end,
        });
    }

    /// How many lines are visible.
    #[must_use]
    pub fn len(&self) -> usize {
        self.runs.last().map_or(0, |run| run.rows_end)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// The last shown line.
    #[must_use]
    pub fn last(&self) -> Option<usize> {
        let last = self.runs.len().checked_sub(1)?;
        Some(self.end(last) - 1)
    }

    /// The source line at visible row `row`.
    #[must_use]
    pub fn get(&self, row: usize) -> Option<usize> {
        let index = self.runs.partition_point(|run| run.rows_end <= row);
        let run = self.runs.get(index)?;
        Some(run.start + row - self.rows_start(index))
    }

    /// The visible row of source line `source`, if it is shown.
    #[must_use]
    pub fn position(&self, source: usize) -> Option<usize> {
        let row = self.rows_before(source);
        (self.get(row) == Some(source)).then_some(row)
    }

    /// How many shown lines come before source line `source`: the row it
    /// has, or would have if it were shown.
    #[must_use]
    pub fn rows_before(&self, source: usize) -> usize {
        let Some(index) = self
            .runs
            .partition_point(|run| run.start < source)
            .checked_sub(1)
        else {
            return 0;
        };
        let rows_start = self.rows_start(index);
        rows_start + (source - self.runs[index].start).min(self.runs[index].rows_end - rows_start)
    }

    /// The shown lines, in order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.iter_from(0)
    }

    /// The shown lines from visible row `row` on.
    pub fn iter_from(&self, row: usize) -> impl Iterator<Item = usize> + '_ {
        let first = self.runs.partition_point(|run| run.rows_end <= row);
        (first..self.runs.len()).flat_map(move |index| {
            let rows_start = self.rows_start(index);
            let start = self.runs[index].start;
            start + row.saturating_sub(rows_start)..self.end(index)
        })
    }

    /// The shown lines of visible rows `start..end`, with both bounds
    /// clamped to the set.
    pub fn rows(&self, start: usize, end: usize) -> impl Iterator<Item = usize> + '_ {
        let end = end.min(self.len());
        let start = start.min(end);
        self.iter_from(start).take(end - start)
    }

    /// The shown lines of visible rows `..end`, last first.
    pub fn rev_rows(&self, end: usize) -> impl Iterator<Item = usize> + '_ {
        let end = end.min(self.len());
        // The runs that start before row `end`.
        let count = self.runs.partition_point(|run| run.rows_end < end) + usize::from(end > 0);
        (0..count).rev().flat_map(move |index| {
            let start = self.runs[index].start;
            let rows = self.runs[index].rows_end.min(end) - self.rows_start(index);
            (start..start + rows).rev()
        })
    }

    /// Keep the first `rows` lines and drop the rest.
    pub(crate) fn truncate(&mut self, rows: usize) {
        // The runs that end before `rows`, and the one it falls in.
        let keep = self.runs.partition_point(|run| run.rows_end < rows);
        if keep < self.runs.len() && self.rows_start(keep) < rows {
            self.runs[keep].rows_end = rows;
            self.runs.truncate(keep + 1);
        } else {
            self.runs.truncate(keep);
        }
    }

    /// The visible row run `index` starts at.
    fn rows_start(&self, index: usize) -> usize {
        index
            .checked_sub(1)
            .map_or(0, |before| self.runs[before].rows_end)
    }

    /// The source line after run `index`.
    fn end(&self, index: usize) -> usize {
        let run = self.runs[index];
        run.start + run.rows_end - self.rows_start(index)
    }
}

impl Extend<usize> for Visible {
    fn extend<I: IntoIterator<Item = usize>>(&mut self, sources: I) {
        for source in sources {
            self.push(source);
        }
    }
}

impl FromIterator<usize> for Visible {
    fn from_iter<I: IntoIterator<Item = usize>>(sources: I) -> Self {
        let mut visible = Self::default();
        visible.extend(sources);
        visible
    }
}

/// For tests that name the lines one by one.
impl PartialEq<[usize]> for Visible {
    fn eq(&self, sources: &[usize]) -> bool {
        self.iter().eq(sources.iter().copied())
    }
}

impl<const N: usize> PartialEq<[usize; N]> for Visible {
    fn eq(&self, sources: &[usize; N]) -> bool {
        *self == sources[..]
    }
}

/// The runs, as ranges of source lines: a set of a million lines prints
/// as one range.
impl std::fmt::Debug for Visible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries((0..self.runs.len()).map(|index| self.runs[index].start..self.end(index)))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(sources: &[usize]) -> Visible {
        sources.iter().copied().collect()
    }

    #[test]
    fn a_row_gives_its_source_line() {
        let visible = of(&[2, 3, 4, 9, 11, 12]);
        assert_eq!(visible.len(), 6);
        let rows: Vec<_> = (0..7).map(|row| visible.get(row)).collect();
        assert_eq!(
            rows,
            [Some(2), Some(3), Some(4), Some(9), Some(11), Some(12), None]
        );
    }

    #[test]
    fn a_source_line_gives_its_row_if_it_is_shown() {
        let visible = of(&[2, 3, 4, 9, 11, 12]);
        let rows: Vec<_> = (0..14).map(|source| visible.position(source)).collect();
        assert_eq!(
            rows,
            [
                None,
                None,
                Some(0),
                Some(1),
                Some(2),
                None,
                None,
                None,
                None,
                Some(3),
                None,
                Some(4),
                Some(5),
                None
            ]
        );
    }

    /// What `partition_point(|row| row < source)` gave on the old vector.
    #[test]
    fn rows_before_counts_the_shown_lines_before_a_source_line() {
        let visible = of(&[2, 3, 4, 9, 11, 12]);
        let counts: Vec<_> = (0..14).map(|source| visible.rows_before(source)).collect();
        assert_eq!(counts, [0, 0, 0, 1, 2, 3, 3, 3, 3, 3, 4, 4, 5, 6]);
        assert_eq!(Visible::default().rows_before(5), 0);
    }

    #[test]
    fn iter_from_walks_the_lines_from_a_row() {
        let visible = of(&[2, 3, 4, 9, 11, 12]);
        assert_eq!(visible.iter().collect::<Vec<_>>(), [2, 3, 4, 9, 11, 12]);
        assert_eq!(visible.iter_from(1).collect::<Vec<_>>(), [3, 4, 9, 11, 12]);
        assert_eq!(visible.iter_from(3).collect::<Vec<_>>(), [9, 11, 12]);
        assert_eq!(visible.iter_from(5).collect::<Vec<_>>(), [12]);
        assert_eq!(visible.iter_from(6).count(), 0);
    }

    /// Follow mode cuts the set back and adds to it. The result is the same
    /// as a set built at once, so `==` still says whether the lines moved.
    #[test]
    fn a_truncated_set_grows_like_a_new_one() {
        let mut visible = of(&[2, 3, 4, 9, 11, 12]);
        visible.truncate(4);
        assert_eq!(visible, of(&[2, 3, 4, 9]));
        visible.push(10);
        assert_eq!(visible, of(&[2, 3, 4, 9, 10]));

        visible.truncate(2);
        visible.push(4);
        assert_eq!(visible, of(&[2, 3, 4]));

        visible.truncate(9);
        assert_eq!(visible, of(&[2, 3, 4]));
        visible.truncate(0);
        assert_eq!(visible, Visible::default());
    }

    /// The window the view draws: clamped, as a stale bound from the last
    /// frame can be past the end.
    #[test]
    fn rows_gives_a_clamped_window() {
        let visible = of(&[2, 3, 4, 9, 11, 12]);
        assert_eq!(visible.rows(1, 4).collect::<Vec<_>>(), [3, 4, 9]);
        assert_eq!(visible.rows(4, 99).collect::<Vec<_>>(), [11, 12]);
        assert_eq!(visible.rows(9, 99).count(), 0);
    }

    #[test]
    fn rev_rows_walks_back_from_a_row() {
        let visible = of(&[2, 3, 4, 9, 11, 12]);
        assert_eq!(
            visible.rev_rows(6).collect::<Vec<_>>(),
            [12, 11, 9, 4, 3, 2]
        );
        assert_eq!(visible.rev_rows(4).collect::<Vec<_>>(), [9, 4, 3, 2]);
        assert_eq!(visible.rev_rows(2).collect::<Vec<_>>(), [3, 2]);
        assert_eq!(visible.rev_rows(99).count(), 6);
        assert_eq!(visible.rev_rows(0).count(), 0);
    }
}
