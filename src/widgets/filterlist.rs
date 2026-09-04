//! The pane listing the filters that have been defined.
//!
//! It renders from a borrowed `ActiveFilters` rather than owning one: `App` owns
//! the set, and a copy here could go stale the moment a filter changed.
//!
//! Since #129 the pane has two levels. [`rows`] is the one description of
//! what is on screen — the live search's row, the scratch set's filters with
//! no header of their own, then each named set as a header row with its
//! filters beneath it while it is enabled. Labels, styles, keys and height all
//! derive from that list, so they cannot disagree about what a row is.

use super::FilterCommand;
use crate::filter::{ActiveFilters, DIM_STYLE, SEARCH_STYLE, Sense};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{List, ListItem, ListState, StatefulWidget};
use unicode_width::UnicodeWidthStr;

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

/// Columns a named set's filters are indented under their header.
const INDENT: &str = "  ";

/// One row of the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    /// The live search, marked `/` rather than numbered: it occupies no
    /// position in the known list, so `/` and `Esc` never renumber filters.
    Search,
    /// A named set's header, by set index. Never 0: the scratch set draws no
    /// header, which is what keeps a user with no `filters.toml` looking at
    /// exactly the pane they had before sets existed.
    Header(usize),
    /// A filter, by known-list index.
    Filter(usize),
}

/// The pane, top to bottom: the search row if there is one; the scratch
/// set's filters; then, for each named set in pane order, a header and —
/// only while the set is enabled — its filters.
///
/// A disabled set is one `[ ]` row and an enabled set whose filters are all
/// off is a `[x]` row over a column of `[ ]` rows. They mean different
/// things and the pane says so.
pub(crate) fn rows(filters: &ActiveFilters) -> Vec<Row> {
    let mut out = Vec::new();
    if filters.search().is_some() {
        out.push(Row::Search);
    }
    out.extend(filters.filters_in(0).map(|(index, _)| Row::Filter(index)));
    for (set, meta) in filters.sets().iter().enumerate().skip(1) {
        out.push(Row::Header(set));
        if meta.enabled {
            out.extend(filters.filters_in(set).map(|(index, _)| Row::Filter(index)));
        }
    }
    out
}

#[derive(Debug, Default)]
pub(crate) struct FilterList {
    pub state: ListState,
    pub active: bool,
}

impl FilterList {
    pub(crate) fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    pub(crate) fn select_next(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let next = self.state.selected().map_or(0, |i| (i + 1).min(len - 1));
        self.state.select(Some(next));
    }

    pub(crate) fn select_previous(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let previous = self.state.selected().map_or(0, |i| i.saturating_sub(1));
        self.state.select(Some(previous));
    }

    /// Pull the selection back into range after the list has shrunk, and drop
    /// it entirely when nothing is left.
    pub(crate) fn clamp_selection(&mut self, len: usize) {
        if len == 0 {
            self.state.select(None);
        } else {
            let index = self.state.selected().unwrap_or(0).min(len - 1);
            self.state.select(Some(index));
        }
    }

    /// Handle a key, reporting any change `App` must make to the filter set.
    ///
    /// Selection movement is handled here because it is the pane's own
    /// state; mutations are only reported, never applied, because the
    /// `ActiveFilters` they act on belongs to `App` — this pane only borrows one
    /// to render it. `rows` is what the pane is showing, from [`rows`], so
    /// the key and the label agree on which filter or set a row addresses.
    ///
    /// Guarded against CONTROL and ALT the same way every global binding in
    /// `App::handle_event` is: without this, `Ctrl-D` — half-page-down in the
    /// file view, and exactly the muscle memory a vim user arrives with —
    /// silently deleted the selected filter instead, since the routing that
    /// reaches this pane discarded modifiers entirely.
    pub(crate) fn handle_key(&mut self, key: KeyEvent, rows: &[Row]) -> Option<FilterCommand> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next(rows.len());
                return None;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous(rows.len());
                return None;
            }
            _ => {}
        }
        let row = rows.get(self.selected()?).copied()?;
        match (key.code, row) {
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
            (KeyCode::Enter, Row::Filter(index)) => Some(FilterCommand::Toggle(index)),
            (KeyCode::Enter, Row::Search) => Some(FilterCommand::ToggleSearch),
            (KeyCode::Enter, Row::Header(set)) => Some(FilterCommand::ToggleSet(set)),
            (KeyCode::Char('d'), Row::Filter(index)) => Some(FilterCommand::Delete(index)),
            (KeyCode::Char('d'), Row::Search) => Some(FilterCommand::DeleteSearch),
            // `c` for change, as in vim.
            (KeyCode::Char('c'), Row::Filter(index)) => Some(FilterCommand::Edit(index)),
            (KeyCode::Char('c'), Row::Search) => Some(FilterCommand::EditSearch),
            // `m` as in *metadata*: the filter keeps showing its lines but
            // stops choosing files in the navigator (#119). The search has no
            // context form.
            (KeyCode::Char('m'), Row::Filter(index)) => Some(FilterCommand::ToggleContext(index)),
            // A set is defined by the file, and the pane says so rather than
            // doing nothing (#120's "no silent keys").
            (KeyCode::Char('d' | 'c' | 'm'), Row::Header(_)) => Some(FilterCommand::SetIsReadOnly),
            _ => None,
        }
    }

    /// Rows this pane wants: one per row plus its borders, and never fewer
    /// than the single row a hint needs.
    ///
    /// An empty set used to ask for nothing, which collapsed the pane out of
    /// the layout entirely. It is now on screen whenever the navigator is, so
    /// the floor is one content row rather than zero — see `EMPTY_HINTS`.
    pub(crate) fn preferred_height(&self, rows: usize) -> u16 {
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
    pub(crate) fn preferred_width(&self, filters: &ActiveFilters) -> u16 {
        let longest = Self::texts(filters)
            .iter()
            .map(|(_, text)| UnicodeWidthStr::width(text.as_str()))
            .max()
            .unwrap_or(0);
        u16::try_from(longest + BORDERS as usize).unwrap_or(u16::MAX)
    }

    /// Every row with its text, in pane order.
    ///
    /// Filter rows are numbered by a running count over the rows shown, top
    /// to bottom and continuously across sets. Numbers are labels, not
    /// addresses — nothing binds a digit to a filter — so enabling a set
    /// renumbers the rows below it the same way deleting a filter does.
    fn texts(filters: &ActiveFilters) -> Vec<(Row, String)> {
        let mut number = 0;
        rows(filters)
            .into_iter()
            .map(|row| {
                if matches!(row, Row::Filter(_)) {
                    number += 1;
                }
                (row, Self::row_text(filters, row, number))
            })
            .collect()
    }

    /// One row's text: its number or `/`, whether it is on, which way it
    /// filters, and its name — or, for a header, the set's flag and name.
    ///
    /// The sense is spelled out because excluding filters carry no colour —
    /// nothing else on the row would distinguish them. A header carries `*`
    /// when the set has profiles, so the picker key (#130) is discoverable.
    fn row_text(filters: &ActiveFilters, row: Row, number: usize) -> String {
        match row {
            Row::Search => {
                let search = filters
                    .search()
                    .expect("Row::Search only when a search exists");
                format!(
                    "/[{}] inc {}",
                    mark(search.enabled),
                    search.predicate.display()
                )
            }
            Row::Header(set) => {
                let meta = &filters.sets()[set];
                let star = if meta.profiles.is_empty() { "" } else { " *" };
                format!("[{}] {}{star}", mark(meta.enabled), meta.name)
            }
            Row::Filter(index) => {
                let filter = &filters.filters()[index];
                let indent = if filter.set == 0 { "" } else { INDENT };
                format!(
                    "{indent}{number}[{}] {} {}",
                    mark(filter.enabled),
                    sense_word(filter.sense),
                    filter.display_name()
                )
            }
        }
    }

    /// How one row is painted.
    ///
    /// An including filter wears its own colour, so the pane and the file
    /// view agree at a glance; the search is always `Sense::Include` and
    /// takes `SEARCH_STYLE` the same way. Disabled rows and disabled
    /// headers take `DIM_STYLE` — the file view's precedent for dimming:
    /// `Modifier::DIM` alone is silently ignored by many terminals, so an
    /// explicit grey foreground is what actually shows the difference, and
    /// the `[ ]` marker still carries the signal if colour fails.
    fn row_style(filters: &ActiveFilters, row: Row) -> Style {
        match row {
            Row::Search => match filters.search() {
                Some(search) if search.enabled => SEARCH_STYLE,
                _ => DIM_STYLE,
            },
            Row::Header(set) if filters.sets()[set].enabled => Style::default(),
            Row::Header(_) => DIM_STYLE,
            Row::Filter(index) => {
                let filter = &filters.filters()[index];
                if !filter.enabled {
                    return DIM_STYLE;
                }
                match filter.sense {
                    Sense::Include | Sense::Context => filter.style,
                    Sense::Exclude => Style::default().fg(Color::DarkGray),
                }
            }
        }
    }

    pub(crate) fn render(&mut self, filters: &ActiveFilters, area: Rect, buf: &mut Buffer) {
        let texts = Self::texts(filters);
        // An empty set draws the hint instead of no rows at all. `DIM_STYLE`
        // is the same grey the file view and the disabled-filter rows use, so
        // the hint reads as chrome rather than as a filter someone defined.
        //
        // Omitted rather than clipped when the column is too narrow to hold
        // it: `preferred_width` deliberately lets the navigator win the width
        // (see its doc comment), so a narrow column is an expected state, not
        // a broken one, and half a sentence of advice is worse than none.
        if texts.is_empty() {
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

        let items: Vec<ListItem> = texts
            .into_iter()
            .map(|(row, text)| ListItem::new(text).style(Self::row_style(filters, row)))
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

fn mark(enabled: bool) -> char {
    if enabled { 'x' } else { ' ' }
}

fn sense_word(sense: Sense) -> &'static str {
    match sense {
        Sense::Include => "inc",
        Sense::Context => "ctx",
        Sense::Exclude => "exc",
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

    /// The text of pane row `row`, as `render` would draw it.
    fn text_at(filters: &ActiveFilters, row: usize) -> String {
        FilterList::texts(filters)
            .into_iter()
            .nth(row)
            .map_or_else(|| panic!("no row {row}"), |(_, text)| text)
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

    /// A file filter's row sits under its set's header and shows the
    /// filter's own name; a scratch filter's row is unchanged from before
    /// sets existed.
    #[test]
    fn file_filters_sit_under_their_set_by_name() {
        let mut loaded = crate::filter::test_support::loaded("w", 50, true, &["associated"]);
        loaded.filters[0].name = "assoc".into();
        let mut filters = ActiveFilters::with_sets(None, &[loaded]);
        filters.add("typed").expect("valid");
        let mut list = FilterList::default();
        let rows = rendered(&mut list, &filters, 30);
        assert!(rows[1].contains("1[x] inc typed"), "{rows:?}");
        assert!(rows[2].contains("[x] w"), "{rows:?}");
        assert!(rows[3].contains("  2[ ] inc assoc"), "{rows:?}");
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

    /// A pattern can hold wide glyphs as readily as a filename can, and this
    /// pane sizes itself the same way the navigator does — so it needs the
    /// same measurement (#97).
    #[test]
    fn preferred_width_counts_display_columns_not_chars() {
        let filters = set_of(&["日本語"], &[]);
        let list = FilterList::default();

        let row = text_at(&filters, 0);
        assert!(
            usize::from(list.preferred_width(&filters))
                >= UnicodeWidthStr::width(row.as_str()) + BORDERS as usize,
            "too narrow for {row:?} at {} columns",
            UnicodeWidthStr::width(row.as_str())
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
        let (_, last_row_text) = FilterList::texts(&filters).pop().expect("rows");

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

        assert_eq!(text_at(&set, 0), "/[x] inc timeout");
        assert_eq!(text_at(&set, 1), "1[x] inc ERROR");
    }

    #[test]
    fn without_a_search_the_numbered_filters_start_at_row_zero() {
        let mut set = ActiveFilters::new();
        set.add("ERROR").expect("valid pattern");

        assert_eq!(text_at(&set, 0), "1[x] inc ERROR");
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

        let command = list.handle_key(KeyEvent::from(KeyCode::Enter), &rows(&set));

        assert_eq!(command, Some(FilterCommand::Toggle(0)));
    }

    #[test]
    fn enter_on_the_search_row_toggles_the_search() {
        let mut set = ActiveFilters::new();
        set.set_search("timeout").expect("valid pattern");
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Enter), &rows(&set));

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

        let command = list.handle_key(KeyEvent::from(KeyCode::Char(' ')), &rows(&set));

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
    fn handle_key_and_the_labels_agree_on_which_filter_a_row_addresses() {
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add("beta").expect("valid pattern");
        set.set_search("gamma").expect("valid pattern");
        let mut list = FilterList::default();
        let rows = rows(&set);

        for (row, (_, text)) in FilterList::texts(&set).into_iter().enumerate() {
            list.state.select(Some(row));
            let command = list
                .handle_key(KeyEvent::from(KeyCode::Enter), &rows)
                .unwrap_or_else(|| panic!("row {row}: no command"));
            match command {
                FilterCommand::ToggleSearch => {
                    assert!(text.starts_with('/'), "row {row}: {text:?}");
                }
                FilterCommand::Toggle(index) => assert!(
                    text.starts_with(&(index + 1).to_string()),
                    "row {row}: handle_key resolved filter {index}, the label is {text:?}"
                ),
                other => panic!("row {row}: unexpected command {other:?}"),
            }
        }
    }

    #[test]
    fn d_on_the_search_row_deletes_the_search() {
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char('d')), &[Row::Search]);

        assert_eq!(command, Some(FilterCommand::DeleteSearch));
    }

    /// `c` uses the same row-to-filter translation `space` and `d` do, so it
    /// inherits the off-by-one below the search row — and must be pinned
    /// against it the same way.
    #[test]
    fn c_below_the_search_row_edits_the_right_filter() {
        let mut list = FilterList::default();
        list.state.select(Some(1));

        let command = list.handle_key(
            KeyEvent::from(KeyCode::Char('c')),
            &[Row::Search, Row::Filter(0)],
        );

        assert_eq!(command, Some(FilterCommand::Edit(0)));
    }

    #[test]
    fn c_on_the_search_row_edits_the_search() {
        let mut list = FilterList::default();
        list.state.select(Some(0));

        let command = list.handle_key(KeyEvent::from(KeyCode::Char('c')), &[Row::Search]);

        assert_eq!(command, Some(FilterCommand::EditSearch));
    }

    #[test]
    fn m_reports_a_context_toggle_for_the_selected_filter() {
        let mut list = FilterList::default();
        list.select_next(2);
        list.select_next(2);

        let command = list.handle_key(
            KeyEvent::from(KeyCode::Char('m')),
            &[Row::Filter(0), Row::Filter(1)],
        );

        assert_eq!(command, Some(FilterCommand::ToggleContext(1)));
    }

    /// The search cannot be context: it always selects. `m` on its row is nothing.
    #[test]
    fn m_on_the_search_row_does_nothing() {
        let mut list = FilterList::default();
        list.select_next(1);

        assert_eq!(
            list.handle_key(KeyEvent::from(KeyCode::Char('m')), &[Row::Search]),
            None
        );
    }

    #[test]
    fn a_context_filter_reads_ctx_in_its_row() {
        let mut filters = set_of(&["foo"], &[]);
        filters.toggle_context(0);

        assert_eq!(text_at(&filters, 0), "1[x] ctx foo");
    }

    // ---- two levels (#129) --------------------------------------------------

    fn two_sets(a_enabled: bool, b_enabled: bool) -> ActiveFilters {
        let a = crate::filter::test_support::loaded("a", 10, a_enabled, &["x", "y"]);
        let b = crate::filter::test_support::loaded("b", 20, b_enabled, &["z"]);
        let mut filters = ActiveFilters::with_sets(None, &[a, b]);
        filters.add("scratch").expect("valid");
        filters
    }

    /// Search, scratch (no header), then each set: header, and rows only
    /// while enabled.
    #[test]
    fn rows_follow_the_spec_order() {
        let mut filters = two_sets(true, false);
        filters.set_search("s").expect("valid");
        assert_eq!(
            rows(&filters),
            vec![
                Row::Search,
                Row::Filter(0),
                Row::Header(1),
                Row::Filter(1),
                Row::Filter(2),
                Row::Header(2),
            ]
        );
    }

    #[test]
    fn with_no_sets_rows_are_exactly_todays() {
        let filters = set_of(&["a", "b"], &[]);
        assert_eq!(rows(&filters), vec![Row::Filter(0), Row::Filter(1)]);
    }

    #[test]
    fn an_enabled_set_shows_a_header_and_indented_rows() {
        let filters = two_sets(true, false);
        let mut list = FilterList::default();
        let rows = rendered(&mut list, &filters, 30);
        assert!(rows[1].contains("1[x] inc scratch"), "{rows:?}");
        assert!(rows[2].contains("[x] a"), "{rows:?}");
        assert!(rows[3].contains("  2[ ] inc x"), "{rows:?}");
        assert!(rows[4].contains("  3[ ] inc y"), "{rows:?}");
        assert!(rows[5].contains("[ ] b"), "{rows:?}");
        assert!(
            !rows.iter().any(|row| row.contains("inc z")),
            "a disabled set shows no filters: {rows:?}"
        );
    }

    #[test]
    fn numbers_run_over_what_is_shown() {
        let mut filters = two_sets(false, true);
        let mut list = FilterList::default();
        let rows_before = rendered(&mut list, &filters, 30);
        assert!(rows_before[4].contains("2[ ] inc z"), "{rows_before:?}");
        filters.set_enabled_set(1, true);
        let rows_after = rendered(&mut list, &filters, 30);
        assert!(
            rows_after[6].contains("4[ ] inc z"),
            "enabling a set above renumbers below: {rows_after:?}"
        );
    }

    #[test]
    fn a_header_carries_a_star_when_the_set_has_profiles() {
        let mut a = crate::filter::test_support::loaded("a", 10, true, &["x"]);
        a.profiles.insert("default".into(), vec!["x".into()]);
        let filters = ActiveFilters::with_sets(None, &[a]);
        let mut list = FilterList::default();
        let rows = rendered(&mut list, &filters, 30);
        assert!(rows[1].contains("[x] a *"), "{rows:?}");
    }

    #[test]
    fn a_disabled_header_is_dimmed_and_an_enabled_one_is_not() {
        let filters = two_sets(true, false);
        let mut list = FilterList::default();
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);
        list.render(&filters, area, &mut buf);
        assert_eq!(buf[(1, 5)].style().fg, DIM_STYLE.fg, "[ ] b is dimmed");
        assert_ne!(buf[(1, 2)].style().fg, DIM_STYLE.fg, "[x] a is not");
    }

    #[test]
    fn preferred_width_counts_header_rows() {
        let a = crate::filter::test_support::loaded("a-very-long-set-name", 10, false, &["x"]);
        let filters = ActiveFilters::with_sets(None, &[a]);
        let list = FilterList::default();
        assert!(list.preferred_width(&filters) as usize >= "[ ] a-very-long-set-name".len() + 2);
    }

    #[test]
    fn enter_on_a_header_toggles_the_set() {
        let filters = two_sets(true, false);
        let rows = rows(&filters);
        let mut list = FilterList::default();
        list.state.select(Some(1)); // Header(1)
        assert_eq!(
            list.handle_key(KeyEvent::from(KeyCode::Enter), &rows),
            Some(FilterCommand::ToggleSet(1))
        );
    }

    #[test]
    fn d_c_m_on_a_header_report_read_only() {
        let filters = two_sets(true, false);
        let rows = rows(&filters);
        let mut list = FilterList::default();
        list.state.select(Some(1));
        for c in ['d', 'c', 'm'] {
            assert_eq!(
                list.handle_key(KeyEvent::from(KeyCode::Char(c)), &rows),
                Some(FilterCommand::SetIsReadOnly),
                "{c}"
            );
        }
    }

    /// Collapsing a set shortens the list; the selection follows.
    #[test]
    fn selection_clamps_to_the_rows_shown() {
        let mut filters = two_sets(true, true);
        let mut list = FilterList::default();
        list.state.select(Some(rows(&filters).len() - 1));
        filters.set_enabled_set(2, false);
        list.clamp_selection(rows(&filters).len());
        assert_eq!(list.selected(), Some(rows(&filters).len() - 1));
    }
}
