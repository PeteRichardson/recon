//! Mouse clicks on the panes and the status row (#58).
//!
//! Divider drags live in `layout.rs`, because they are about geometry. This
//! is about *aim*: a click lands on the frame the user was looking at, so
//! every hit test here is against the rectangles `App::render` remembered,
//! and what happens next is whatever the keyboard would have done to the
//! thing under the pointer — a click is the pointing-at version of "move
//! the cursor here and press `Enter`", never a new verb.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Margin, Position, Rect};
use std::time::Instant;

use crate::layout::DOUBLE_CLICK;
use crate::widgets::{Focus, filterlist};
use crate::{App, PromptKind, SearchPrompt};

impl App<'_> {
    /// Handle a click, reporting whether it was consumed.
    ///
    /// Only the left button's press: a release and a drag are the divider's
    /// (see `handle_divider`, which runs first), the wheel is the focused
    /// pane's, and there is nothing here for the other buttons to mean.
    pub(crate) fn handle_click(&mut self, mouse: MouseEvent) -> bool {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return false;
        }
        let at = Position::new(mouse.column, mouse.row);
        if self.status_area.contains(at) {
            self.click_status();
            return true;
        }
        let Some(pane) = self.pane_at(at) else {
            return false;
        };
        // Focus follows the click before anything else, so a click on a
        // border — which names a pane without naming a row — still does the
        // one thing it can mean. `focus_next` clears the chain origin on
        // every move for the same reason this does: the `f` that started it
        // was left behind by a hand that went to the mouse.
        if self.focus != pane {
            self.chain_origin = None;
            self.focus = pane;
        }
        // Rows start inside the border; a click on the border itself has
        // done its work already.
        let inner = self.pane_area(pane).inner(Margin::new(1, 1));
        if !inner.contains(at) {
            return true;
        }
        let line = mouse.row - inner.y;
        match pane {
            Focus::Nav => self.click_nav(line),
            Focus::Filters => self.click_filter(line),
            Focus::View => self.click_view(line),
        }
        true
    }

    /// The pane under `at`, if any.
    ///
    /// While zoomed there is exactly one pane and it fills everything above
    /// the status row; the three per-pane rectangles are from whichever split
    /// frame was last drawn and must not be consulted.
    fn pane_at(&self, at: Position) -> Option<Focus> {
        if let Some(zoomed) = self.zoom {
            return self.panes_area.contains(at).then_some(zoomed);
        }
        [Focus::Nav, Focus::View, Focus::Filters]
            .into_iter()
            .find(|&pane| self.pane_area(pane).contains(at))
    }

    /// Where `pane` was drawn in the last frame — the whole frame above the
    /// status row when it is the zoomed pane.
    pub(crate) fn pane_area(&self, pane: Focus) -> Rect {
        if self.zoom.is_some() {
            return self.panes_area;
        }
        match pane {
            Focus::Nav => self.nav_area,
            Focus::View => self.view_area,
            Focus::Filters => self.filter_area,
        }
    }

    /// A navigator row: select it, and open it as `FileNav::click` decides.
    /// Two clicks on the same row inside `DOUBLE_CLICK` are a double-click,
    /// timed here because crossterm does not report them — the same clock the
    /// divider uses.
    fn click_nav(&mut self, line: u16) {
        let Some(row) = self.nav.row_at(line) else {
            return;
        };
        let now = Instant::now();
        let double = self
            .last_nav_click
            .is_some_and(|(last, at)| last == row && now.duration_since(at) <= DOUBLE_CLICK);
        // A double-click is spent: a third click starts a new pair rather than
        // continuing this one, or holding the button down would descend a
        // level every 400ms.
        self.last_nav_click = (!double).then_some((row, now));
        if let Some(action) = self.nav.click(line, double) {
            self.perform(action);
        }
        self.ensure_window();
    }

    /// A filter row: select it and do what `Enter` does there.
    fn click_filter(&mut self, line: u16) {
        let rows = filterlist::rows(&self.filters);
        if let Some(command) = self.filters_pane.click(line, &rows) {
            self.apply_filter_command(command);
        }
    }

    /// A row of the look-ahead listing: the navigator enters the directory
    /// the view is showing and opens the entry clicked, which is the `l`,
    /// cursor motion and `Enter` that would otherwise get there. Anything
    /// else in the view — a file's text, a message — has no row to act on,
    /// and the click has already moved focus, which is all it can mean.
    fn click_view(&mut self, line: u16) {
        if !self.view.showing_directory() {
            return;
        }
        let Some(index) = self.view.line_at(line) else {
            return;
        };
        let dir = self.view.filename().to_path_buf();
        if let Some(action) = self.nav.open_listed(&dir, index) {
            self.perform(action);
        }
        self.ensure_window();
    }

    /// The status row: `f i`, as one click. Focus goes to the filter pane
    /// with the same chain origin `f` records, so committing the pattern
    /// returns focus to where the click came from and steps to the first
    /// match there, exactly as the keys would.
    fn click_status(&mut self) {
        let origin = (self.focus != Focus::Filters).then_some(self.focus);
        self.reveal_and_focus(Focus::Filters);
        self.chain_origin = origin;
        self.search = Some(SearchPrompt::new(PromptKind::Filter));
    }
}
