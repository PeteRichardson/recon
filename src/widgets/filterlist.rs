//! The pane listing the filters that have been defined.
//!
//! It renders from a borrowed `FilterSet` rather than owning one: `App` owns
//! the set, and a copy here could go stale the moment a filter changed.

use crate::filter::{FilterSet, Sense, DIM_STYLE};
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, StatefulWidget};

/// Marks the row the cursor is on, matching the navigator pane.
const SELECTION: &str = ">>";

/// Rows of chrome the pane needs on top of one row per filter.
const BORDERS: u16 = 2;

#[derive(Debug, Default)]
pub struct FilterList {
    pub state: ListState,
    pub active: bool,
}

impl FilterList {
    pub fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub fn select_next(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let next = self.state.selected().map_or(0, |i| (i + 1).min(len - 1));
        self.state.select(Some(next));
    }

    pub fn select_previous(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let previous = self.state.selected().map_or(0, |i| i.saturating_sub(1));
        self.state.select(Some(previous));
    }

    /// Pull the selection back into range after the set has shrunk, and drop
    /// it entirely when nothing is left.
    pub fn clamp_selection(&mut self, len: usize) {
        match len {
            0 => self.state.select(None),
            _ => {
                let index = self.state.selected().unwrap_or(0).min(len - 1);
                self.state.select(Some(index));
            }
        }
    }

    /// Rows this pane wants: one per filter plus its borders, or none at all
    /// when there are no filters, so it costs nothing to a user who never
    /// defines one.
    pub fn preferred_height(&self, len: usize) -> u16 {
        match len {
            0 => 0,
            n => u16::try_from(n).unwrap_or(u16::MAX).saturating_add(BORDERS),
        }
    }

    /// Columns needed for the widest row.
    pub fn preferred_width(&self, filters: &FilterSet) -> u16 {
        let longest = (0..filters.len())
            .map(|index| Self::row_text(filters, index).chars().count())
            .max()
            .unwrap_or(0);
        u16::try_from(longest + SELECTION.len() + BORDERS as usize).unwrap_or(u16::MAX)
    }

    /// One filter's row: its number, whether it is on, which way it filters,
    /// and its pattern.
    ///
    /// The sense is spelled out because excluding filters carry no colour —
    /// nothing else on the row would distinguish them.
    fn row_text(filters: &FilterSet, index: usize) -> String {
        let Some(filter) = filters.filters().get(index) else {
            return String::new();
        };
        let mark = if filter.enabled { 'x' } else { ' ' };
        let sense = match filter.sense {
            Sense::Include => "inc",
            Sense::Exclude => "exc",
        };
        format!("{}[{}] {} {}", index + 1, mark, sense, filter.pattern.as_str())
    }

    pub fn render(&mut self, filters: &FilterSet, area: Rect, buf: &mut Buffer) {
        let items: Vec<ListItem> = (0..filters.len())
            .map(|index| {
                let filter = &filters.filters()[index];
                let style = if !filter.enabled {
                    // Matches the file view's precedent for dimming
                    // (`src/filter.rs`'s `DIM_STYLE`): `Modifier::DIM` alone is
                    // silently ignored by many terminals, so an explicit grey
                    // foreground is what actually shows the difference. The
                    // `[ ]` marker still carries the signal if colour fails.
                    DIM_STYLE
                } else {
                    match filter.sense {
                        // An including filter wears its own colour, so the pane
                        // and the file view agree at a glance.
                        Sense::Include => filter.style,
                        Sense::Exclude => Style::default().fg(Color::DarkGray),
                    }
                };
                ListItem::new(Self::row_text(filters, index)).style(style)
            })
            .collect();

        let mut highlight = Style::new().add_modifier(Modifier::REVERSED);
        if self.active {
            highlight = highlight.fg(Color::Green);
        }

        let list = List::new(items)
            .block(Block::bordered().title("Filters"))
            .highlight_style(highlight)
            .highlight_symbol(SELECTION);
        StatefulWidget::render(&list, area, buf, &mut self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_of(includes: &[&str], excludes: &[&str]) -> FilterSet {
        let mut set = FilterSet::new();
        for pattern in includes {
            set.add(pattern).expect("valid pattern");
        }
        for pattern in excludes {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    fn rendered(list: &mut FilterList, filters: &FilterSet, width: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, 8);
        let mut buf = Buffer::empty(area);
        list.render(filters, area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn each_filter_gets_a_row_showing_its_pattern() {
        let filters = set_of(&["foo", "bar"], &[]);
        let mut list = FilterList::default();

        let rows = rendered(&mut list, &filters, 30).join("\n");

        assert!(rows.contains("foo"), "pattern missing:\n{rows}");
        assert!(rows.contains("bar"), "pattern missing:\n{rows}");
    }

    /// A disabled filter must be distinguishable at a glance from an enabled
    /// one, since that is the pane's main job. Comparing whole rows with only
    /// the pattern stripped out would pass even if the marker were hardcoded,
    /// since each row also carries its own 1-based index — so this asserts
    /// the specific marker that carries the meaning.
    #[test]
    fn enabled_and_disabled_filters_are_marked_differently() {
        let mut filters = set_of(&["foo", "bar"], &[]);
        filters.set_enabled(1, false);
        let mut list = FilterList::default();

        let rows = rendered(&mut list, &filters, 30);
        let foo = rows.iter().find(|r| r.contains("foo")).expect("foo row");
        let bar = rows.iter().find(|r| r.contains("bar")).expect("bar row");

        assert!(foo.contains("[x]"), "enabled filter should show [x]:\n{foo}");
        assert!(
            bar.contains("[ ]"),
            "disabled filter should show [ ], not [x]:\n{bar}"
        );
    }

    /// Excluding filters carry no colour, so the pane must say what they are
    /// some other way. As above, this asserts the specific marker rather than
    /// comparing whole rows, since the leading index would make any such
    /// comparison pass regardless of the sense marker.
    #[test]
    fn excluding_filters_are_marked_as_excluding() {
        let filters = set_of(&["foo"], &["noise"]);
        let mut list = FilterList::default();

        let rows = rendered(&mut list, &filters, 30);
        let noise = rows
            .iter()
            .find(|r| r.contains("noise"))
            .expect("noise row");
        let foo = rows.iter().find(|r| r.contains("foo")).expect("foo row");

        assert!(
            noise.contains("exc"),
            "excluding filter should be marked exc:\n{noise}"
        );
        assert!(
            foo.contains("inc"),
            "including filter should be marked inc:\n{foo}"
        );
    }

    #[test]
    fn preferred_width_grows_with_the_longest_pattern() {
        let list = FilterList::default();
        let short = set_of(&["foo"], &[]);
        let long = set_of(&["a pattern much longer than foo"], &[]);

        assert!(
            list.preferred_width(&long) > list.preferred_width(&short),
            "width should grow with the longest pattern"
        );
    }

    /// The `u16::try_from(..).unwrap_or(u16::MAX)` fallback must saturate on
    /// overflow rather than wrap, the way an `as u16` truncation would — this
    /// project has already shipped a real defect from that kind of silent
    /// wraparound elsewhere.
    #[test]
    fn preferred_width_saturates_rather_than_wrapping_on_overflow() {
        let huge_pattern = "a".repeat(usize::from(u16::MAX) + 100);
        let filters = set_of(&[huge_pattern.as_str()], &[]);
        let list = FilterList::default();

        assert_eq!(
            list.preferred_width(&filters),
            u16::MAX,
            "an overflowing width should saturate at u16::MAX, not wrap"
        );
    }

    /// A disabled excluding filter must read as dimmed, not as the ordinary
    /// excluding grey — disabled wins visually. Nothing else pinned this
    /// ordering, so a future change to how the style branches combine could
    /// silently flip it.
    #[test]
    fn a_disabled_excluding_filter_is_dimmed_not_grey() {
        let mut filters = set_of(&[], &["noise"]);
        filters.set_enabled(0, false);
        let mut list = FilterList::default();
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        assert!(
            (0..area.width).any(|x| buf[(x, 1)].style().fg == DIM_STYLE.fg),
            "a disabled excluding filter should use the dim style"
        );
        assert!(
            !(0..area.width).any(|x| buf[(x, 1)].style().fg == Some(Color::DarkGray)),
            "a disabled excluding filter should not render in the ordinary excluding grey"
        );
    }

    #[test]
    fn an_including_filter_shows_its_colour() {
        let filters = set_of(&["foo"], &[]);
        let mut list = FilterList::default();
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        let expected = filters.filters()[0].style.fg;
        assert!(
            (0..area.width).any(|x| buf[(x, 1)].style().fg == expected),
            "the filter's colour is not shown anywhere on its row"
        );
    }

    /// Pressing `j` on a freshly focused pane must land on the first filter,
    /// not skip past it — a first keypress that lands on row two would be a
    /// bug.
    #[test]
    fn the_first_move_lands_on_the_first_row() {
        let mut list = FilterList::default();

        list.select_next(2);

        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut list = FilterList::default();

        list.select_next(2);
        assert_eq!(list.selected(), Some(0));
        list.select_next(2);
        assert_eq!(list.selected(), Some(1));
        list.select_next(2);
        assert_eq!(list.selected(), Some(1), "selection ran past the end");
        list.select_previous(2);
        assert_eq!(list.selected(), Some(0));
    }

    /// Deleting the last filter must not leave the selection pointing past the
    /// end of the list.
    #[test]
    fn clamping_pulls_the_selection_back_into_range() {
        let mut list = FilterList::default();
        list.select_next(3);
        list.select_next(3);
        list.select_next(3);
        assert_eq!(list.selected(), Some(2));

        list.clamp_selection(1);

        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn an_empty_set_has_no_selection_and_no_height() {
        let mut list = FilterList::default();

        list.clamp_selection(0);

        assert_eq!(list.selected(), None);
        assert_eq!(list.preferred_height(0), 0, "an empty pane must take no rows");
    }

    #[test]
    fn the_pane_grows_with_the_number_of_filters() {
        let list = FilterList::default();

        assert!(list.preferred_height(3) > list.preferred_height(1));
    }
}
