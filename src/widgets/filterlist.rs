//! The pane listing the filters that have been defined.
//!
//! It renders from a borrowed `ActiveFilters` rather than owning one: `App` owns
//! the set, and a copy here could go stale the moment a filter changed.

use super::FilterCommand;
use crate::filter::{ActiveFilters, DIM_STYLE, Filter, Sense};
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

/// Map a pane row to the numbered filter it addresses, or `None` when the
/// row is the live search's own row (row 0, and only when `has_search`).
///
/// A free function taking `has_search` rather than a method on `ActiveFilters`,
/// because `FilterList::handle_key` has no `ActiveFilters` in scope — only the
/// bool it was told — while `resolve_row` does have one and calls this too.
/// Before this was pulled out, `handle_key` and `resolve_row` each carried
/// their own copy of this `(has_search, row)` match, which is exactly how
/// they could disagree about which filter a row addresses.
fn filter_index_for_row(has_search: bool, row: usize) -> Option<usize> {
    match (has_search, row) {
        (true, 0) => None,
        (true, row) => Some(row - 1),
        (false, row) => Some(row),
    }
}

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
    /// `ActiveFilters` they act on belongs to `App` — this pane only borrows one
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
        // addresses a filter one lower. `target` is a thin wrapper over
        // `filter_index_for_row`, the same translation `resolve_row` uses,
        // so `space` and `d` below share one mapping with each other and
        // with the pane's own labels rather than each keeping a copy.
        let target = |row: usize| filter_index_for_row(has_search, row);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next(rows);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous(rows);
                None
            }
            // `Enter`, not `space`: #48 made `space` the global peek, because
            // a key that toggles a filter in this pane and flips hide mode
            // everywhere else is the pane-dependent meaning that change exists
            // to remove.
            //
            // `Enter` is also the key that *commits* the prompt `c`, `i` and
            // `x` open, which is why it was left unbound here until now — a
            // doubled press would commit a pattern and then silently switch a
            // filter off. `App` swallows exactly one `Enter` immediately after
            // a commit; see `swallow_next_enter` in `lib.rs`. The guard lives
            // there rather than here because only `App` knows a prompt closed.
            KeyCode::Enter => Some(match target(self.selected()?) {
                Some(index) => FilterCommand::Toggle(index),
                None => FilterCommand::ToggleSearch,
            }),
            KeyCode::Char('d') => Some(match target(self.selected()?) {
                Some(index) => FilterCommand::Delete(index),
                None => FilterCommand::DeleteSearch,
            }),
            // `c` for change, as in vim.
            KeyCode::Char('c') => Some(match target(self.selected()?) {
                Some(index) => FilterCommand::Edit(index),
                None => FilterCommand::EditSearch,
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
    pub fn preferred_height(&self, rows: usize) -> u16 {
        u16::try_from(rows.max(1))
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
    pub fn preferred_width(&self, filters: &ActiveFilters) -> u16 {
        let longest = (0..filters.row_count())
            .map(|row| Self::row_text(filters, row).chars().count())
            .max()
            .unwrap_or(0);
        u16::try_from(longest + BORDERS as usize).unwrap_or(u16::MAX)
    }

    /// A pane row's label and the filter behind it, search row included.
    ///
    /// Row 0 is the live search when one exists, marked `/` rather than a
    /// number — it has no number, because it does not occupy a position in
    /// `filters`. Its precedence here matches its precedence in `verdict`.
    ///
    /// `row_text` and `render`'s per-row styling both call this rather than
    /// keeping their own copy of the label/filter lookup. It is built on
    /// `filter_index_for_row`, the single source of truth for the
    /// `(has_search, row) -> filter` mapping itself — `handle_key` needs
    /// that same mapping without a `ActiveFilters` in scope, so it calls that
    /// free function directly rather than this one.
    fn resolve_row(filters: &ActiveFilters, row: usize) -> Option<(String, &Filter)> {
        let has_search = filters.search().is_some();
        match filter_index_for_row(has_search, row) {
            None => filters.search().map(|search| ("/".to_string(), search)),
            Some(index) => filters
                .filters()
                .get(index)
                .map(|filter| ((index + 1).to_string(), filter)),
        }
    }

    /// One row of the pane: its number, whether it is on, which way it
    /// filters, and its pattern.
    ///
    /// The sense is spelled out because excluding filters carry no colour —
    /// nothing else on the row would distinguish them.
    fn row_text(filters: &ActiveFilters, row: usize) -> String {
        let Some((label, filter)) = Self::resolve_row(filters, row) else {
            return String::new();
        };
        let mark = if filter.enabled { 'x' } else { ' ' };
        let sense = match filter.sense {
            Sense::Include => "inc",
            Sense::Exclude => "exc",
        };
        format!("{}[{}] {} {}", label, mark, sense, filter.pattern.as_str())
    }

    pub fn render(&mut self, filters: &ActiveFilters, area: Rect, buf: &mut Buffer) {
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
                // `row_count` bounds this loop, so `resolve_row` always
                // resolves here in practice; falling back to the default
                // style on `None` rather than panicking keeps that a
                // property of the loop bound, not a promise `resolve_row`
                // also has to keep.
                let style = Self::resolve_row(filters, row)
                    .map(|(_, filter)| {
                        if !filter.enabled {
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
                        }
                    })
                    .unwrap_or_default();
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

    fn set_of(includes: &[&str], excludes: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in includes {
            set.add(pattern).expect("valid pattern");
        }
        for pattern in excludes {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    fn rendered(list: &mut FilterList, filters: &ActiveFilters, width: u16) -> Vec<String> {
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

    /// No other test in this file draws the pane with a search present:
    /// `rendered`'s callers never set one, and the `lib.rs` tests that set a
    /// search never draw. `resolve_row` is the single source of truth that
    /// `row_text` and `render`'s styling both read the row-to-filter mapping
    /// from (see its doc comment), which is what keeps them from drifting
    /// apart the way two separate copies of the same mapping could. Pinning
    /// the search row and a numbered filter row to their own distinct
    /// styles in the same render is the regression test for that: a future
    /// change that broke the mapping would show up here as a colour
    /// mismatch.
    #[test]
    fn the_search_row_and_a_filter_row_are_each_styled_correctly() {
        let mut filters = set_of(&["ERROR"], &[]);
        filters.set_search("timeout").expect("valid pattern");
        filters.search_set_enabled(false);
        let mut list = FilterList::default();
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);

        list.render(&filters, area, &mut buf);

        assert!(
            (0..area.width).any(|x| buf[(x, 1)].style().fg == DIM_STYLE.fg),
            "the disabled search row (row 0) should use the dim style"
        );
        let expected = filters.filters()[0].style.fg;
        assert!(
            (0..area.width).any(|x| buf[(x, 2)].style().fg == expected),
            "the numbered filter's own colour is not shown on its row"
        );
    }

    /// `preferred_width` must size the column from every row, search
    /// included. Reverting its loop bound from `row_count()` to `len()`
    /// silently drops the last row from consideration; putting the longer
    /// pattern on the *last* filter, rather than the first or the search,
    /// means that drop is not masked by some other row happening to already
    /// be the longest.
    #[test]
    fn preferred_width_accounts_for_every_row_including_the_last_when_a_search_exists() {
        let mut filters = set_of(&["a", "a pattern much longer than the rest"], &[]);
        filters.set_search("x").expect("valid pattern");
        let list = FilterList::default();

        let width = list.preferred_width(&filters) as usize;
        let last_row_text = FilterList::row_text(&filters, filters.row_count() - 1);

        assert!(
            width >= last_row_text.chars().count() + BORDERS as usize,
            "preferred_width ({width}) is too narrow for the last row: {last_row_text:?}"
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

        let width = list.preferred_width(&ActiveFilters::new()) as usize;

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
        let filters = ActiveFilters::new();
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
        let filters = ActiveFilters::new();
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
        let filters = ActiveFilters::new();
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
        let filters = ActiveFilters::new();
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
        let mut set = ActiveFilters::new();
        set.add("ERROR").expect("valid pattern");
        set.set_search("timeout").expect("valid pattern");

        assert_eq!(FilterList::row_text(&set, 0), "/[x] inc timeout");
        assert_eq!(FilterList::row_text(&set, 1), "1[x] inc ERROR");
    }

    #[test]
    fn without_a_search_the_numbered_filters_start_at_row_zero() {
        let mut set = ActiveFilters::new();
        set.add("ERROR").expect("valid pattern");

        assert_eq!(FilterList::row_text(&set, 0), "1[x] inc ERROR");
    }

    /// The offset is the whole risk in this task: `Enter` on row 1 must toggle
    /// filter 0, not filter 1.
    #[test]
    fn enter_below_the_search_row_toggles_the_right_filter() {
        let mut set = ActiveFilters::new();
        set.add("ERROR").expect("valid pattern");
        set.set_search("timeout").expect("valid pattern");
        let mut list = FilterList::default();
        list.state.select(Some(1));

        let command = list.handle_key(KeyEvent::from(KeyCode::Enter), set.row_count(), true);

        assert_eq!(command, Some(FilterCommand::Toggle(0)));
    }

    #[test]
    fn enter_on_the_search_row_toggles_the_search() {
        let mut set = ActiveFilters::new();
        set.set_search("timeout").expect("valid pattern");
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Enter), set.row_count(), true);

        assert_eq!(command, Some(FilterCommand::ToggleSearch));
    }

    /// `space` gave this pane up in #48: it became the global peek, and a
    /// pane that still claimed it would swallow the peek whenever the filter
    /// pane happened to be focused — the exact confusion the change exists to
    /// remove.
    #[test]
    fn space_no_longer_toggles_a_filter() {
        let mut set = ActiveFilters::new();
        set.add("ERROR").expect("valid pattern");
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char(' ')), set.row_count(), false);

        assert_eq!(command, None, "`space` is still claimed by the filter pane");
    }

    /// Cross-checks `handle_key`'s row-to-filter translation against
    /// `resolve_row`'s, the two callers `filter_index_for_row` unifies. Before
    /// that extraction each kept its own copy of the `(has_search, row)`
    /// match, so a future edit to one without the other — the issue #8
    /// scenario in the review that prompted this test — would toggle or
    /// delete a different filter than the row on screen names. Checks every
    /// row over a set with both numbered filters and a search, so both the
    /// search row and the off-by-one shift below it are covered.
    #[test]
    fn handle_key_and_resolve_row_agree_on_which_filter_a_row_addresses() {
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add("beta").expect("valid pattern");
        set.set_search("gamma").expect("valid pattern");
        let mut list = FilterList::default();

        for row in 0..set.row_count() {
            list.state.select(Some(row));
            let command = list
                .handle_key(KeyEvent::from(KeyCode::Enter), set.row_count(), true)
                .unwrap_or_else(|| panic!("row {row}: no command"));
            let (label, _) =
                FilterList::resolve_row(&set, row).unwrap_or_else(|| panic!("row {row}: no row"));

            match command {
                FilterCommand::ToggleSearch => assert_eq!(
                    label, "/",
                    "row {row}: handle_key says the search, resolve_row says {label}"
                ),
                FilterCommand::Toggle(index) => assert_eq!(
                    label,
                    (index + 1).to_string(),
                    "row {row}: handle_key resolved filter {index}, resolve_row's label \
                     for this row is {label}"
                ),
                other => panic!("row {row}: unexpected command {other:?}"),
            }
        }
    }

    #[test]
    fn d_on_the_search_row_deletes_the_search() {
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char('d')), 1, true);

        assert_eq!(command, Some(FilterCommand::DeleteSearch));
    }

    /// `c` uses the same row-to-filter translation `space` and `d` do, so it
    /// inherits the off-by-one below the search row — and must be pinned
    /// against it the same way.
    #[test]
    fn c_below_the_search_row_edits_the_right_filter() {
        let mut list = FilterList::default();
        list.state.select(Some(1));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char('c')), 2, true);

        assert_eq!(command, Some(FilterCommand::Edit(0)));
    }

    #[test]
    fn c_on_the_search_row_edits_the_search() {
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char('c')), 1, true);

        assert_eq!(command, Some(FilterCommand::EditSearch));
    }
}
