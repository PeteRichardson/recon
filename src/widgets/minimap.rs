//! The minimap at the right of the file view's text (#523): where the
//! filter matches and the search hits are in the whole file, and where the
//! pane is. See `docs/specs/2026-10-24-minimap-design.md`.

use ratatui::prelude::{Buffer, Color, Rect};

/// Columns the map takes from the text: the rule, the search column and the
/// filter column.
pub(crate) const WIDTH: u16 = 3;

/// A left-eighths bar, by width in eighths: `▏` is one, `█` is eight.
const EIGHTHS: [&str; 9] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];

/// Dot rows in a braille cell: the search column's resolution, in slots a
/// map row.
const DOT_ROWS: usize = 4;

/// A braille cell's left and right dot on each dot row, top first, as bits
/// above U+2800.
const LEFT_DOTS: [u32; DOT_ROWS] = [0x01, 0x02, 0x04, 0x40];
const RIGHT_DOTS: [u32; DOT_ROWS] = [0x08, 0x10, 0x20, 0x80];

/// The search column's colour: the search highlight's.
const SEARCH_COLOUR: Color = Color::Yellow;

/// The background of the search and filter columns on the rows on screen.
const BAND: Color = Color::DarkGray;

/// The map, built once for a file, its verdicts and a height, and drawn on
/// every frame from what it keeps.
#[derive(Debug)]
pub(crate) struct Minimap {
    /// The file lines the map covers: the lines loaded now.
    lines: usize,
    /// Each row's filter bar: its width in eighths and its colour, or
    /// `None` for a group with no interesting line.
    filter: Vec<Option<(usize, Color)>>,
    /// Each row's search cell, as a braille character, or `None` for a
    /// group with no hit.
    search: Vec<Option<char>>,
}

/// The slot, of `slots`, that line `line` of `lines` falls in. A file
/// shorter than the map has one line a slot, from the top.
fn slot(line: usize, lines: usize, slots: usize) -> usize {
    line * slots / lines.max(slots).max(1)
}

/// The search column: a braille cell a row, with a dot on each dot row
/// that has a hit, and both dots where the hits are half or more of the
/// busiest dot row's.
fn search_cells(
    rows: usize,
    lines: usize,
    hits: impl IntoIterator<Item = usize>,
) -> Vec<Option<char>> {
    let mut counts = vec![0_usize; rows * DOT_ROWS];
    for line in hits {
        if let Some(count) = counts.get_mut(slot(line, lines, rows * DOT_ROWS)) {
            *count += 1;
        }
    }
    let peak = counts.iter().copied().max().unwrap_or(0);
    counts
        .chunks(DOT_ROWS)
        .map(|row| {
            let bits = row
                .iter()
                .enumerate()
                .filter(|&(_, &count)| count > 0)
                .fold(0, |bits, (dot_row, &count)| {
                    let many = count * 2 >= peak;
                    bits | LEFT_DOTS[dot_row] | if many { RIGHT_DOTS[dot_row] } else { 0 }
                });
            char::from_u32(0x2800 + bits).filter(|_| bits != 0)
        })
        .collect()
}

impl Minimap {
    /// A map `rows` high over a file of `lines` lines. `interesting` is each
    /// interesting line with the filter that made it so; `hits` is each
    /// visible line the search matches; `colour` is a filter's colour.
    pub(crate) fn build(
        rows: usize,
        lines: usize,
        interesting: impl IntoIterator<Item = (usize, usize)>,
        hits: impl IntoIterator<Item = usize>,
        colour: impl Fn(usize) -> Color,
    ) -> Self {
        // Matches by row and by filter. A handful of filters on a pane's
        // height of rows, so a small table rather than a map per row.
        let mut counts: Vec<Vec<usize>> = vec![Vec::new(); rows];
        for (line, filter) in interesting {
            let Some(row) = counts.get_mut(slot(line, lines, rows)) else {
                continue;
            };
            if row.len() <= filter {
                row.resize(filter + 1, 0);
            }
            row[filter] += 1;
        }
        let totals: Vec<usize> = counts.iter().map(|row| row.iter().sum()).collect();
        let peak = totals.iter().copied().max().unwrap_or(0);
        let filter = counts
            .iter()
            .zip(&totals)
            .map(|(row, &total)| {
                if total == 0 {
                    return None;
                }
                let width = (total * 8).div_ceil(peak).clamp(1, 8);
                // The most matches; on a tie, the first filter.
                let (busiest, _) = row
                    .iter()
                    .enumerate()
                    .rev()
                    .max_by_key(|&(_, count)| *count)?;
                Some((width, colour(busiest)))
            })
            .collect();
        Self {
            lines,
            filter,
            search: search_cells(rows, lines, hits),
        }
    }

    /// How many rows the map was built for.
    pub(crate) fn rows(&self) -> usize {
        self.filter.len()
    }

    /// The file line a click on map row `row` goes to: the first line of
    /// that row's group, or the file's last line below the end of it.
    pub(crate) fn line_at(&self, row: usize) -> usize {
        let rows = self.rows().max(1);
        let span = self.lines.max(rows);
        // The smallest line `slot` puts on `row` or below it.
        (row * span)
            .div_ceil(rows)
            .min(self.lines.saturating_sub(1))
    }

    /// Draw the map into `area`, `WIDTH` columns wide. `band` is the first
    /// and the last file line on screen.
    pub(crate) fn render(&self, band: Option<(usize, usize)>, area: Rect, buf: &mut Buffer) {
        let rows = self.rows();
        let band =
            band.map(|(first, last)| slot(first, self.lines, rows)..=slot(last, self.lines, rows));
        for (row, y) in (area.y..area.bottom()).take(rows).enumerate() {
            let on_screen = band.as_ref().is_some_and(|band| band.contains(&row));
            // The rule reads as a scroll bar: thick where the thumb is.
            let (rule, rule_colour) = if on_screen {
                ("┃", Color::Gray)
            } else {
                ("│", Color::DarkGray)
            };
            buf[(area.x, y)].set_symbol(rule).set_fg(rule_colour);
            for x in [area.x + 1, area.x + 2] {
                buf[(x, y)].reset();
            }
            if let Some(dots) = self.search[row] {
                buf[(area.x + 1, y)].set_char(dots).set_fg(SEARCH_COLOUR);
            }
            if let Some((width, colour)) = self.filter[row] {
                buf[(area.x + 2, y)]
                    .set_symbol(EIGHTHS[width])
                    .set_fg(colour);
            }
            if on_screen {
                buf[(area.x + 1, y)].set_bg(BAND);
                buf[(area.x + 2, y)].set_bg(BAND);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The symbol and foreground of column `x` on map row `y`.
    fn cell(buf: &Buffer, x: u16, y: u16) -> (String, Color) {
        let cell = &buf[(x, y)];
        (cell.symbol().to_string(), cell.fg)
    }

    fn drawn(map: &Minimap, band: Option<(usize, usize)>) -> Buffer {
        let area = Rect::new(0, 0, WIDTH, u16::try_from(map.rows()).unwrap());
        let mut buf = Buffer::empty(area);
        map.render(band, area, &mut buf);
        buf
    }

    #[test]
    fn the_filter_bar_is_as_wide_as_the_group_is_busy() {
        // 100 lines on 10 rows: 10 lines a row. Row 0 has 8 matches, the
        // most; row 5 has 4, half as many; row 9 has 1.
        let interesting = (0..8).chain(50..54).chain([95]).map(|line| (line, 0));
        let map = Minimap::build(10, 100, interesting, std::iter::empty(), |_| Color::Red);
        let buf = drawn(&map, None);
        assert_eq!(cell(&buf, 2, 0), ("█".to_string(), Color::Red));
        assert_eq!(cell(&buf, 2, 5), ("▌".to_string(), Color::Red));
        assert_eq!(cell(&buf, 2, 9).0, "▏", "one match still shows");
        assert_eq!(cell(&buf, 2, 3).0, " ", "no match, no bar");
    }

    fn colour_of(filter: usize) -> Color {
        [Color::Red, Color::Green, Color::Blue][filter]
    }

    #[test]
    fn the_bar_takes_the_colour_of_the_busiest_filter_and_the_first_on_a_tie() {
        // Row 0: filter 2 twice, filter 1 once. Row 1: filters 1 and 2 once each.
        let interesting = [(0, 2), (1, 1), (2, 2), (10, 2), (11, 1)];
        let map = Minimap::build(10, 100, interesting, std::iter::empty(), colour_of);
        let buf = drawn(&map, None);
        assert_eq!(cell(&buf, 2, 0).1, Color::Blue);
        assert_eq!(cell(&buf, 2, 1).1, Color::Green);
    }

    #[test]
    fn a_search_hit_is_a_dot_on_its_quarter_of_the_row() {
        // 40 lines on 10 rows: 4 lines a row, 1 line a dot row. Line 0 is
        // the top dot row of row 0, line 7 the bottom dot row of row 1.
        // Line 7 has the most hits, so it is "many": both dots. Lines 20
        // and 21 have a quarter as many: one dot each.
        let hits = [0, 7, 7, 7, 7, 20, 21];
        let map = Minimap::build(10, 40, std::iter::empty(), hits, colour_of);
        let buf = drawn(&map, None);
        assert_eq!(cell(&buf, 1, 0), ("⠁".to_string(), Color::Yellow));
        assert_eq!(cell(&buf, 1, 1).0, "⣀", "line 7: both bottom dots");
        assert_eq!(cell(&buf, 1, 5).0, "⠃", "lines 20 and 21: one dot each");
        assert_eq!(cell(&buf, 1, 2).0, " ", "no hit, no dot");
    }

    #[test]
    fn the_rows_on_screen_thicken_the_rule_and_shade_the_columns() {
        // 100 lines on 10 rows; lines 30 to 49 on screen are rows 3 and 4.
        let map = Minimap::build(10, 100, [(31, 0)], [42], colour_of);
        let buf = drawn(&map, Some((30, 49)));
        for y in [3, 4] {
            assert_eq!(buf[(0, y)].symbol(), "┃", "row {y}");
            assert_eq!(buf[(1, y)].bg, BAND, "row {y}");
            assert_eq!(buf[(2, y)].bg, BAND, "row {y}");
        }
        for y in [2, 5] {
            assert_eq!(buf[(0, y)].symbol(), "│", "row {y}");
            assert_ne!(buf[(1, y)].bg, BAND, "row {y}");
        }
    }

    fn empty_map(rows: usize, lines: usize) -> Minimap {
        Minimap::build(
            rows,
            lines,
            std::iter::empty(),
            std::iter::empty(),
            colour_of,
        )
    }

    #[test]
    fn a_click_on_a_row_is_the_first_line_of_its_group() {
        let map = empty_map(10, 100);
        assert_eq!(map.line_at(0), 0);
        assert_eq!(map.line_at(5), 50);
        assert_eq!(map.line_at(9), 90);
        assert_eq!(map.line_at(12), 99, "past the last row is the last line");
    }

    #[test]
    fn a_file_shorter_than_the_map_has_a_line_a_row_from_the_top() {
        let map = empty_map(10, 3);
        assert_eq!(map.line_at(2), 2);
        assert_eq!(map.line_at(7), 2, "below the file is its last line");
        let buf = drawn(&Minimap::build(10, 3, [(2, 0)], [], colour_of), None);
        assert_eq!(buf[(2, 2)].symbol(), "█");
    }
}
