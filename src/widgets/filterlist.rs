//! The pane listing the filters that have been defined.
//!
//! It renders from a borrowed `FilterSet` rather than owning one: `App` owns
//! the set, and a copy here could go stale the moment a filter changed.

use super::FilterCommand;
use crate::filter::{DIM_STYLE, Filter, FilterSet, Sense};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{List, ListItem, ListState, StatefulWidget};

/// Rows of chrome the pane needs on top of one row per filter.
const BORDERS: u16 = 2;

/// Candidate texts for the pane's single row when no filter is defined,
/// longest first. `render` draws the first one that fits the column.
///
/// The pane is on screen whenever the navigator is, so with an empty set it
/// would otherwise be a titled box with nothing in it — space taken and
/// nothing said. Naming the binding that fills it turns that row into the one
/// place `f i` is discoverable without reading the README.
///
/// `f i`, not `i`, because the hint has to be right wherever focus happens to
/// be — and it is: `f` focuses this pane, and pressing it while the pane
/// already has focus is a no-op, so the pair works from anywhere.
///
/// Three forms rather than one because the column is sized by the *navigator*
/// (see `preferred_width`), and a directory of short names leaves well under
/// 16 columns inside the borders — the full sentence would then never be
/// drawn at all.
///
/// The bare `f i` exists because the binding grew a second key: a directory of
/// names as short as `log.txt` gives the pane 7 content columns, which fitted
/// the old `press f` exactly and does not fit `press f i`. Dropping to the
/// keys alone keeps the binding visible there rather than trading it away for
/// the politer wording. Below 3 columns `render` draws none of them: half a
/// sentence of advice is worse than none.
const EMPTY_HINTS: [&str; 3] = ["press f i to add", "press f i", "f i"];

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

    /// Handle a key, reporting any change `App` must make to the filter set.
    ///
    /// Selection movement is handled here because it is the pane's own
    /// state; mutations are only reported, never applied, because the
    /// `FilterSet` they act on belongs to `App` — this pane only borrows one
    /// to render it.
    ///
    /// Guarded against CONTROL and ALT the same way every global binding in
    /// `App::handle_event` is: without this, `Ctrl-D` — half-page-down in the
    /// file view, and exactly the muscle memory a vim user arrives with —
    /// silently deleted the selected filter instead, since the routing that
    /// reaches this pane discarded modifiers entirely. Takes the whole
    /// `KeyEvent`, not just its `KeyCode`, so the guard is possible at all;
    /// 2c-ii's `x` and digit bindings are also modifier-sensitive, so this
    /// signature is owed either way.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        rows: usize,
        has_search: bool,
    ) -> Option<FilterCommand> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        // Row 0 is the live search when one exists, so every row below it
        // addresses a filter one lower. Doing this translation here, once,
        // keeps the offset out of `App` entirely.
        let target = |row: usize| -> Option<usize> {
            match (has_search, row) {
                (true, 0) => None,
                (true, row) => Some(row - 1),
                (false, row) => Some(row),
            }
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next(rows);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous(rows);
                None
            }
            KeyCode::Char(' ') => Some(match target(self.selected()?) {
                Some(index) => FilterCommand::Toggle(index),
                None => FilterCommand::ToggleSearch,
            }),
            KeyCode::Char('d') => Some(match target(self.selected()?) {
                Some(index) => FilterCommand::Delete(index),
                None => FilterCommand::DeleteSearch,
            }),
            _ => None,
        }
    }

    /// Rows this pane wants: one per filter plus its borders, and never fewer
    /// than the single row a hint needs.
    ///
    /// An empty set used to ask for nothing, which collapsed the pane out of
    /// the layout entirely. It is now on screen whenever the navigator is, so
    /// the floor is one content row rather than zero — see `EMPTY_HINTS`.
    pub fn preferred_height(&self, len: usize) -> u16 {
        u16::try_from(len.max(1))
            .unwrap_or(u16::MAX)
            .saturating_add(BORDERS)
    }

    /// Columns needed for the widest row.
    ///
    /// Two borders and the widest row. There is no selection marker to
    /// reserve room for — #15 removed it from both panes; #19 brings it back
    /// as a setting.
    ///
    /// Deliberately does **not** account for `EMPTY_HINTS`. `App::nav_width`
    /// sizes the left column to whichever pane wants more, so counting the
    /// hint here would put a ~18-column floor under the column for every user
    /// who has not defined a filter — including the directory of short names
    /// that `auto_width_has_no_floor` exists to keep narrow. The hint is
    /// guidance, not content: it yields to the navigator rather than widening
    /// the column, and `render` simply omits it when the column is too narrow
    /// to hold it.
    pub fn preferred_width(&self, filters: &FilterSet) -> u16 {
        let longest = (0..filters.row_count())
            .map(|row| Self::row_text(filters, row).chars().count())
            .max()
            .unwrap_or(0);
        u16::try_from(longest + BORDERS as usize).unwrap_or(u16::MAX)
    }

    /// One row of the pane: its number, whether it is on, which way it
    /// filters, and its pattern.
    ///
    /// Row 0 is the live search when one exists, marked `/` rather than a
    /// number — it has no number, because it does not occupy a position in
    /// `filters`. Its precedence here matches its precedence in `verdict`.
    ///
    /// The sense is spelled out because excluding filters carry no colour —
    /// nothing else on the row would distinguish them.
    fn row_text(filters: &FilterSet, row: usize) -> String {
        let (label, filter) = match (filters.search(), row) {
            (Some(search), 0) => ("/".to_string(), Some(search)),
            (search, row) => {
                let index = row - usize::from(search.is_some());
                ((index + 1).to_string(), filters.filters().get(index))
            }
        };
        let Some(filter) = filter else {
            return String::new();
        };
        let mark = if filter.enabled { 'x' } else { ' ' };
        let sense = match filter.sense {
            Sense::Include => "inc",
            Sense::Exclude => "exc",
        };
        format!("{}[{}] {} {}", label, mark, sense, filter.pattern.as_str())
    }

    /// The filter a pane row refers to, search row included.
    fn row_filter(filters: &FilterSet, row: usize) -> Option<&Filter> {
        match (filters.search(), row) {
            (Some(search), 0) => Some(search),
            (search, row) => filters.filters().get(row - usize::from(search.is_some())),
        }
    }

    pub fn render(&mut self, filters: &FilterSet, area: Rect, buf: &mut Buffer) {
        // An empty set draws the hint instead of no rows at all. `DIM_STYLE`
        // is the same grey the file view and the disabled-filter rows use, so
        // the hint reads as chrome rather than as a filter someone defined.
        //
        // Omitted rather than clipped when the column is too narrow to hold
        // it: `preferred_width` deliberately lets the navigator win the width
        // (see its doc comment), so a narrow column is an expected state, not
        // a broken one, and half a sentence of advice is worse than none.
        if filters.row_count() == 0 {
            let interior = area.width.saturating_sub(BORDERS) as usize;
            let rows: Vec<ListItem> = EMPTY_HINTS
                .iter()
                .find(|hint| hint.chars().count() <= interior)
                .map(|hint| vec![ListItem::new(*hint).style(DIM_STYLE)])
                .unwrap_or_default();
            let hint = List::new(rows).block(crate::widgets::pane_block("Filters", self.active));
            Widget::render(&hint, area, buf);
            return;
        }

        let items: Vec<ListItem> = (0..filters.row_count())
            .map(|row| {
                // `row_count` bounds this loop, so every row in range has a
                // filter behind it — the search at row 0, a numbered filter
                // at every row after.
                let filter = Self::row_filter(filters, row).expect("row within row_count");
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
                        // and the file view agree at a glance. The search is
                        // always `Sense::Include`, so it takes this branch too,
                        // showing `SEARCH_STYLE` the same way.
                        Sense::Include => filter.style,
                        Sense::Exclude => Style::default().fg(Color::DarkGray),
                    }
                };
                ListItem::new(Self::row_text(filters, row)).style(style)
            })
            .collect();

        let mut highlight = Style::new().add_modifier(Modifier::REVERSED);
        if self.active {
            highlight = highlight.fg(Color::Green);
        }

        let list = List::new(items)
            .block(crate::widgets::pane_block("Filters", self.active))
            .highlight_style(highlight);
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

        assert!(
            foo.contains("[x]"),
            "enabled filter should show [x]:\n{foo}"
        );
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
    fn an_empty_set_has_no_selection() {
        let mut list = FilterList::default();

        list.clamp_selection(0);

        assert_eq!(list.selected(), None);
    }

    /// The pane is on screen whenever the navigator is, so an empty set still
    /// reserves its borders plus the one row a hint is drawn on.
    #[test]
    fn an_empty_pane_reserves_a_row_for_the_hint() {
        let list = FilterList::default();

        assert_eq!(list.preferred_height(0), BORDERS + 1);
    }

    /// One filter must not make the pane shorter than no filters did.
    #[test]
    fn one_filter_is_no_shorter_than_an_empty_pane() {
        let list = FilterList::default();

        assert_eq!(list.preferred_height(1), list.preferred_height(0));
    }

    #[test]
    fn the_pane_grows_with_the_number_of_filters() {
        let list = FilterList::default();

        assert!(list.preferred_height(3) > list.preferred_height(1));
    }

    /// The hint must not widen the left column. `App::nav_width` takes the
    /// larger of the two panes' preferred widths, so counting the hint here
    /// would put a floor under the column for everyone who has not defined a
    /// filter — see `auto_width_has_no_floor` in `lib.rs`.
    #[test]
    fn the_hint_does_not_widen_the_column() {
        let list = FilterList::default();

        let width = list.preferred_width(&FilterSet::new()) as usize;

        assert!(
            width < EMPTY_HINTS[EMPTY_HINTS.len() - 1].chars().count(),
            "an empty pane is asking for {width} columns to fit its hint"
        );
    }

    /// The short form is what keeps the binding visible at the widths the
    /// navigator actually produces — a directory of short names leaves about
    /// nine columns inside the borders, well under the full sentence.
    #[test]
    fn a_column_too_narrow_for_the_sentence_still_shows_the_binding() {
        let mut list = FilterList::default();
        let filters = FilterSet::new();
        let area = Rect::new(0, 0, 11, 3);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        let row: String = (0..area.width).map(|col| buf[(col, 1)].symbol()).collect();
        assert!(
            row.contains("press f"),
            "no binding shown at width 11: {row:?}"
        );
        assert!(
            !row.contains("to add"),
            "the full sentence was clipped in rather than falling back: {row:?}"
        );
    }

    /// Half a sentence of advice is worse than none, so a column too narrow
    /// for the whole hint gets a plain bordered box instead of a clipped one.
    #[test]
    fn a_narrow_pane_omits_the_hint_rather_than_clipping_it() {
        let mut list = FilterList::default();
        let filters = FilterSet::new();
        // One column short of the *shortest* hint, derived rather than
        // hard-coded: adding a shorter fallback moves this threshold, and a
        // literal width would quietly start testing a pane that does fit one.
        let shortest = EMPTY_HINTS[EMPTY_HINTS.len() - 1].chars().count() as u16;
        let area = Rect::new(0, 0, shortest + BORDERS - 1, 3);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        let text: String = (0..area.width)
            .map(|col| buf[(col, 1)].symbol())
            .collect::<String>();
        assert!(
            text.trim_matches(['│', ' ']).is_empty(),
            "clipped hint drawn in a narrow pane: {text:?}"
        );
    }

    #[test]
    fn an_empty_pane_draws_the_hint() {
        let mut list = FilterList::default();
        let filters = FilterSet::new();
        let area = Rect::new(0, 0, 30, 3);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        let text: String = (0..area.height)
            .map(|row| {
                (0..area.width)
                    .map(|col| buf[(col, row)].symbol())
                    .collect::<String>()
            })
            .collect();
        assert!(
            text.contains("Filters"),
            "no title on an empty pane: {text}"
        );
        assert!(
            text.contains(EMPTY_HINTS[0]),
            "no hint on an empty pane: {text}"
        );
    }

    /// The hint is guidance, not content — it must not read as a filter.
    #[test]
    fn the_hint_is_dimmed() {
        let mut list = FilterList::default();
        let filters = FilterSet::new();
        let area = Rect::new(0, 0, 30, 3);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        let hint_cell = (0..area.width)
            .map(|col| &buf[(col, 1)])
            .find(|cell| cell.symbol().trim() == "p")
            .expect("hint row not drawn");
        assert_eq!(hint_cell.style().fg, DIM_STYLE.fg);
    }

    #[test]
    fn the_search_row_is_drawn_first_and_carries_a_slash() {
        let mut set = FilterSet::new();
        set.add("ERROR").expect("valid pattern");
        set.set_search("timeout").expect("valid pattern");

        assert_eq!(FilterList::row_text(&set, 0), "/[x] inc timeout");
        assert_eq!(FilterList::row_text(&set, 1), "1[x] inc ERROR");
    }

    #[test]
    fn without_a_search_the_numbered_filters_start_at_row_zero() {
        let mut set = FilterSet::new();
        set.add("ERROR").expect("valid pattern");

        assert_eq!(FilterList::row_text(&set, 0), "1[x] inc ERROR");
    }

    /// The offset is the whole risk in this task: `space` on row 1 must toggle
    /// filter 0, not filter 1.
    #[test]
    fn space_below_the_search_row_toggles_the_right_filter() {
        let mut set = FilterSet::new();
        set.add("ERROR").expect("valid pattern");
        set.set_search("timeout").expect("valid pattern");
        let mut list = FilterList::default();
        list.state.select(Some(1));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char(' ')), set.row_count(), true);

        assert_eq!(command, Some(FilterCommand::Toggle(0)));
    }

    #[test]
    fn space_on_the_search_row_toggles_the_search() {
        let mut set = FilterSet::new();
        set.set_search("timeout").expect("valid pattern");
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char(' ')), set.row_count(), true);

        assert_eq!(command, Some(FilterCommand::ToggleSearch));
    }

    #[test]
    fn d_on_the_search_row_deletes_the_search() {
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char('d')), 1, true);

        assert_eq!(command, Some(FilterCommand::DeleteSearch));
    }
}
