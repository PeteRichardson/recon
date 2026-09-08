//! Selection and paging over a [`ListState`], shared by the two panes that
//! are a list — the navigator and the filter pane (#194).
//!
//! Each pane used to carry its own `ListState`, its own `last_height`, its
//! own `ASSUMED_PAGE` and a near-identical set of motions over them, with
//! the second copy pointing at the first in a comment. The motions know
//! nothing about what the rows *are*; every one takes the row count from
//! the pane, which is the one thing the panes disagree on.

use ratatui::widgets::ListState;

/// A page, in rows, before the pane has been drawn once. `App::run` renders
/// before it reads a key, so this only ever matters to a test — but a zero
/// page would make `PageDown` a silent no-op, and #120 forbids silent keys.
const ASSUMED_PAGE: usize = 20;

/// A cursor over `len` rows, where `len` is whatever the pane says it is on
/// each call, and the page size the pane last rendered at.
#[derive(Debug, Default)]
pub(crate) struct ListMotion {
    state: ListState,
    /// Inner height at the last render, so page motions know their page.
    /// `None` until then; see `ASSUMED_PAGE`.
    last_height: Option<u16>,
}

impl ListMotion {
    pub(crate) fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    /// Put the cursor on `row`, or nowhere. No clamping: the pane knows its
    /// rows and says where the cursor goes after a rebuild.
    pub(crate) fn select(&mut self, row: Option<usize>) {
        self.state.select(row);
    }

    /// Forget the cursor and the scroll offset, for a listing built afresh.
    pub(crate) fn reset(&mut self) {
        self.state = ListState::default();
    }

    /// Move down one, stopping at the last row.
    ///
    /// Clamps explicitly rather than using `ListState::select_next`, which
    /// increments without knowing the list length: at the bottom it moved the
    /// navigator's selection *past* the last entry, where `selected_path`
    /// returned `None` and previewing silently stopped until `k` was pressed.
    /// Rendering hid it, because `List` clamps the highlight for drawing.
    pub(crate) fn select_next(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let next = self.state.selected().map_or(0, |i| (i + 1).min(len - 1));
        self.state.select(Some(next));
    }

    /// Move up one, stopping at the first row. Nothing selected counts as
    /// the top, not the bottom — `ListState::select_previous` would wrap to
    /// the end.
    pub(crate) fn select_previous(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let previous = self.state.selected().map_or(0, |i| i.saturating_sub(1));
        self.state.select(Some(previous));
    }

    pub(crate) fn select_first(&mut self, len: usize) {
        if len > 0 {
            self.state.select(Some(0));
        }
    }

    pub(crate) fn select_last(&mut self, len: usize) {
        if let Some(last) = len.checked_sub(1) {
            self.state.select(Some(last));
        }
    }

    /// Move by `delta` rows, clamping at both ends. Positive is down. Nothing
    /// selected counts as row 0, so a page down from nowhere lands a page in.
    pub(crate) fn move_by(&mut self, delta: isize, len: usize) {
        let Some(last) = len.checked_sub(1) else {
            return;
        };
        let from = self.state.selected().unwrap_or(0);
        self.state
            .select(Some(from.saturating_add_signed(delta).min(last)));
    }

    /// Pull the selection back into range after the list has shrunk, and drop
    /// it entirely when nothing is left. Never `None` while there is a row to
    /// be on.
    pub(crate) fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.state.select(None);
        } else {
            let index = self.state.selected().unwrap_or(0).min(len - 1);
            self.state.select(Some(index));
        }
    }

    /// Rows in a page: the pane's inner height at the last render, never
    /// zero.
    pub(crate) fn page_rows(&self) -> usize {
        self.last_height.map_or(ASSUMED_PAGE, usize::from).max(1)
    }

    /// Note the inner height the pane just rendered at, so the next page
    /// motion is a real page.
    pub(crate) fn rendered(&mut self, inner_height: u16) {
        self.last_height = Some(inner_height);
    }

    /// The state itself, for `StatefulWidget::render`, which scrolls it.
    pub(crate) fn state_mut(&mut self) -> &mut ListState {
        &mut self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_and_previous_clamp_at_both_ends_and_start_from_the_top() {
        let mut list = ListMotion::default();
        list.select_previous(3);
        assert_eq!(
            list.selected(),
            Some(0),
            "nothing selected counts as the top"
        );
        list.select_next(3);
        list.select_next(3);
        list.select_next(3);
        assert_eq!(list.selected(), Some(2), "stops at the last row");
        list.select_previous(3);
        assert_eq!(list.selected(), Some(1));
    }

    #[test]
    fn an_empty_list_takes_no_motion() {
        let mut list = ListMotion::default();
        list.select_next(0);
        list.select_previous(0);
        list.select_first(0);
        list.select_last(0);
        list.move_by(5, 0);
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn first_and_last_go_to_the_ends() {
        let mut list = ListMotion::default();
        list.select_last(4);
        assert_eq!(list.selected(), Some(3));
        list.select_first(4);
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn move_by_clamps_and_treats_nothing_as_row_zero() {
        let mut list = ListMotion::default();
        list.move_by(2, 10);
        assert_eq!(list.selected(), Some(2), "from nowhere is from row 0");
        list.move_by(100, 10);
        assert_eq!(list.selected(), Some(9));
        list.move_by(-100, 10);
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn clamp_pulls_the_selection_back_and_drops_it_for_an_empty_list() {
        let mut list = ListMotion::default();
        list.select(Some(7));
        list.clamp(3);
        assert_eq!(list.selected(), Some(2));
        list.clamp(0);
        assert_eq!(list.selected(), None);
        list.clamp(2);
        assert_eq!(list.selected(), Some(0), "never None while there is a row");
    }

    #[test]
    fn a_page_is_the_rendered_height_and_never_zero() {
        let mut list = ListMotion::default();
        assert_eq!(list.page_rows(), ASSUMED_PAGE, "before the first render");
        list.rendered(12);
        assert_eq!(list.page_rows(), 12);
        list.rendered(0);
        assert_eq!(list.page_rows(), 1, "a zero page would be a silent key");
    }

    #[test]
    fn reset_forgets_the_cursor() {
        let mut list = ListMotion::default();
        list.select(Some(3));
        list.reset();
        assert_eq!(list.selected(), None);
    }
}
