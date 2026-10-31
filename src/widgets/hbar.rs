//! The horizontal scroll bar under the file view's text (#578): where the
//! visible window sits along the widest line the user can see. The mirror of
//! `widgets::minimap` turned sideways — the map says where you are down the
//! file, this says where you are across it.
//!
//! This module is the widget and its geometry only. Reserving its row in the
//! pane and wiring clicks and drags to it are #580 and #581; the offset the
//! thumb tracks is the textarea's own scroll column, and the setter behind a
//! drag is the fork's `set_scroll_col` (#577).

use ratatui::prelude::{Buffer, Color, Rect};

/// Rows the bar takes from the text.
pub(crate) const HEIGHT: u16 = 1;

/// What the track is drawn as: a one-eighth-high line, from the same block
/// family as the minimap's filter bars.
const TRACK: &str = "▁";

/// What the thumb is drawn as. The minimap's rule reads "thick where the
/// thumb is" through weight and colour (`│` against `┃`); sideways it is the
/// same two channels — the thumb is the solid, lighter run of the row.
///
/// Deliberately whole cells rather than eighth blocks: Unicode has left-justified
/// partial blocks (`▏`..`▉`) but no right-justified counterparts beyond one
/// eighth, so a thumb ending mid-cell could draw its right cap but a thumb
/// *starting* mid-cell could not draw its left cap honestly. Whole cells avoid
/// the mismatch. Eighths are still the internal unit — the thumb's position and
/// width carry sub-cell precision, so a drag tracks the pointer closer than the
/// glyphs alone can show.
const THUMB: &str = "█";

/// The two colours, matching the minimap rule's pair.
const TRACK_COLOUR: Color = Color::DarkGray;
const THUMB_COLOUR: Color = Color::Gray;

/// A bar over content `content` columns wide with `view` of them on screen at
/// once. `content` is everything the view can scroll to show — the gutter and
/// the widest visible line together, the same figure the textarea reports as
/// its limit (`max_scroll_col` plus the width); `view` is the text area's
/// width in the same units, so the two agree on where column 0 is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hbar {
    content: usize,
    view: usize,
}

impl Hbar {
    pub(crate) const fn new(content: usize, view: usize) -> Self {
        Self { content, view }
    }

    /// Whether the bar has anything to say: something is cut off at the right.
    /// A zero-width view has nothing to say either — there is no window to
    /// place. The caller draws nothing when this is false.
    pub(crate) const fn shows(&self) -> bool {
        self.content > self.view && self.view > 0
    }

    /// The largest scroll offset: what overhangs the window.
    const fn max_scroll(&self) -> usize {
        self.content.saturating_sub(self.view)
    }

    /// Thumb width in cells: the view's share of the content, rounded up and
    /// never invisible — same `div_ceil` rule as the minimap's filter bar.
    fn thumb_cells(&self, cells: usize) -> usize {
        if self.content == 0 {
            return cells.max(1);
        }
        (self.view * cells)
            .div_ceil(self.content)
            .clamp(1, cells.max(1))
    }

    /// The thumb's first cell for scroll offset `col`: its share of the
    /// track left free by the thumb, rounded.
    fn thumb_start(&self, col: usize, cells: usize) -> usize {
        let free = cells.saturating_sub(self.thumb_cells(cells));
        let max = self.max_scroll();
        if free == 0 || max == 0 {
            return 0;
        }
        (2 * col.min(max) * free + max) / (2 * max)
    }

    /// Draw the bar for scroll offset `col` into `area`; only the area's
    /// first row is touched. Does nothing when there is nothing to show —
    /// including a zero-width area, which has no cells to place.
    pub(crate) fn render(&self, col: usize, area: Rect, buf: &mut Buffer) {
        if !self.shows() || area.width == 0 || area.height == 0 {
            return;
        }
        let start = self.thumb_start(col, usize::from(area.width));
        let thumb = self.thumb_cells(usize::from(area.width));
        for (i, x) in (area.x..area.right()).enumerate() {
            let cell = &mut buf[(x, area.y)];
            if i >= start && i < start + thumb {
                cell.set_symbol(THUMB).set_fg(THUMB_COLOUR);
            } else {
                cell.set_symbol(TRACK).set_fg(TRACK_COLOUR);
            }
        }
    }

    /// The column a click on bar cell `cell` of a `cells`-wide bar goes to:
    /// the offset at which the thumb's centre sits under that cell, so the
    /// text moves with the pointer during a drag instead of jumping by a
    /// thumb's width. Clamped to the scrollable range like the setter it
    /// feeds.
    pub(crate) fn col_at(&self, cell: usize, cells: u16) -> usize {
        let cells = usize::from(cells).max(1);
        let max = self.max_scroll();
        if !self.shows() {
            return 0;
        }
        let thumb = self.thumb_cells(cells);
        let free = cells - thumb;
        if free == 0 {
            // A thumb spanning the whole track cannot slide, but the track
            // still maps: clicks spread across the range rather than sticking.
            return (2 * cell + 1) * max / (2 * cells);
        }
        // Invert `thumb_start` for the cell the thumb's centre occupies:
        // `cell` is where the pointer is, the centre is half a thumb back.
        let centre = i64::try_from(cell)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(thumb - 1).unwrap_or(i64::MAX) / 2);
        let col = if centre <= 0 {
            0
        } else {
            (2 * centre * i64::try_from(max).unwrap_or(i64::MAX)
                + i64::try_from(free).unwrap_or(i64::MAX))
                / (2 * i64::try_from(free).unwrap_or(1))
        };
        usize::try_from(col.clamp(0, i64::try_from(max).unwrap_or(i64::MAX))).unwrap_or(max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The symbols of the bar row, one string per cell.
    fn row(buf: &Buffer, width: u16) -> Vec<String> {
        (0..width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect()
    }

    fn drawn(bar: &Hbar, col: usize, width: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, HEIGHT);
        let mut buf = Buffer::empty(area);
        bar.render(col, area, &mut buf);
        row(&buf, width)
    }

    /// 100 columns of content, 20 on screen, a 40-cell bar: the thumb is 8
    /// cells, the free track 32 cells, the range 80 columns — 2.5 columns a
    /// cell, so 0, 40 and 80 land the thumb's first cell exactly.
    #[test]
    fn the_thumb_sits_at_the_start_middle_and_end() {
        let bar = Hbar::new(100, 20);
        assert!(bar.shows());
        let cells = |col: usize| {
            let cells = drawn(&bar, col, 40);
            cells.iter().position(|s| s == THUMB).unwrap()
        };
        assert_eq!(cells(0), 0);
        assert_eq!(cells(40), 16);
        assert_eq!(cells(80), 32);
    }

    #[test]
    fn the_thumb_is_the_view_share_of_the_track_and_the_rest_is_track() {
        let cells = drawn(&Hbar::new(100, 20), 0, 40);
        let filled = cells.iter().filter(|s| *s == THUMB).count();
        assert_eq!(
            filled, 8,
            "a fifth of the content on screen is a fifth of the bar"
        );
        assert_eq!(cells.iter().filter(|s| *s == TRACK).count(), 32);
        assert!(
            cells[8..].iter().all(|s| s == TRACK),
            "the run is contiguous from the start"
        );
    }

    #[test]
    fn content_equal_to_the_view_shows_nothing() {
        let bar = Hbar::new(20, 20);
        assert!(!bar.shows());
        let area = Rect::new(0, 0, 40, HEIGHT);
        let mut buf = Buffer::empty(area);
        bar.render(0, area, &mut buf);
        assert!(
            row(&buf, 40).iter().all(|s| s == " "),
            "no bar where nothing is cut off"
        );
        assert_eq!(bar.col_at(10, 40), 0, "and a click there goes nowhere");
    }

    #[test]
    fn twice_the_view_width_halves_the_track() {
        let bar = Hbar::new(40, 20);
        let cells = drawn(&bar, 10, 40);
        let start = cells.iter().position(|s| s == THUMB).unwrap();
        let filled = cells.iter().filter(|s| *s == THUMB).count();
        assert_eq!(filled, 20);
        assert_eq!(start, 10, "mid-range on a halfway-full bar");
        // The thumb spans cells 10..=29, so its centre is shared between 19
        // and 20, and each resolves within one column of mid-range.
        assert_eq!(bar.col_at(19, 40), 10);
        assert_eq!(bar.col_at(20, 40), 11);
    }

    #[test]
    fn a_one_cell_bar_is_all_thumb_and_clicks_still_spread() {
        let bar = Hbar::new(100, 20);
        let cells = drawn(&bar, 40, 1);
        assert_eq!(cells, vec![THUMB.to_string()]);
        assert_eq!(
            bar.col_at(0, 1),
            40,
            "the single cell is the middle of the range"
        );
        // A wider bar is not stuck: with one thumb cell and one free cell
        // the only placements are the two ends, and the clicks reach them.
        assert_eq!(bar.col_at(0, 2), 0);
        assert_eq!(bar.col_at(1, 2), 80);
    }

    #[test]
    fn a_click_on_the_thumb_centre_round_trips_within_one_column() {
        // 100 content, 20 view, 40 cells: thumb 8, free 32, range 80 —
        // exact multiples, so the round trip is exact, plus an awkward size
        // where it can only be within one.
        for (content, view, cells) in [(100_usize, 20_usize, 40_u16), (200, 37, 45)] {
            let bar = Hbar::new(content, view);
            let max = bar.max_scroll();
            for col in [0, max / 4, max / 2, max * 3 / 4, max] {
                let thumb = bar.thumb_cells(usize::from(cells));
                let centre = bar.thumb_start(col, usize::from(cells)) + (thumb - 1) / 2;
                let back = bar.col_at(centre, cells);
                assert!(
                    back.abs_diff(col) <= 1,
                    "col {col} -> cell {centre} -> col {back} ({content}/{view}/{cells})"
                );
            }
        }
    }

    #[test]
    fn clicks_clamp_inside_the_range() {
        let bar = Hbar::new(100, 20);
        assert_eq!(bar.col_at(0, 40), 0);
        assert_eq!(
            bar.col_at(99, 40),
            80,
            "past the last cell is the last column"
        );
    }

    #[test]
    fn a_zero_width_area_draws_nothing_and_a_zero_view_shows_nothing() {
        let bar = Hbar::new(100, 20);
        let area = Rect::new(0, 0, 0, HEIGHT);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        bar.render(40, area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert!(!Hbar::new(100, 0).shows());
    }

    #[test]
    fn the_thumb_is_lighter_than_the_track() {
        let bar = Hbar::new(100, 20);
        let area = Rect::new(0, 0, 40, HEIGHT);
        let mut buf = Buffer::empty(area);
        bar.render(0, area, &mut buf);
        assert_eq!(buf[(0, 0)].fg, THUMB_COLOUR);
        assert_eq!(buf[(39, 0)].fg, TRACK_COLOUR);
    }
}
