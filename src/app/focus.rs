//! Which pane has the focus, the zoom, and showing and hiding panes.

use super::App;
use crate::panes;
use crate::widgets::Focus;
use crossterm::event::{self, KeyCode};

/// What `E`, `F` or `global.hide.view` says when the pane is the last one
/// shown (#300).
pub(super) const LAST_PANE: &str = "at least one pane stays shown";

impl App<'_> {
    /// Move focus to the next shown pane, left to right, wrapping.
    ///
    /// A hidden pane is skipped (#300): focus is only ever on a pane that is
    /// on the screen, so the cursor is never somewhere the user cannot see.
    pub(super) fn focus_next(&mut self) {
        self.chain_origin = None;
        self.focus = self.panes.next(self.focus);
    }

    /// `Shift-Tab`: the other way round, with the same skip.
    pub(super) fn focus_prev(&mut self) {
        self.chain_origin = None;
        self.focus = self.panes.prev(self.focus);
    }

    /// `z`: hide every pane but the focused one, or — when it is already the
    /// only one shown — show the others again. See `Panes::zoom`.
    pub(super) fn zoom_focused(&mut self) {
        self.chain_origin = None;
        self.cancel_drag();
        self.panes.zoom(self.focus);
    }

    /// `b`: `t`, then `z` (#300). From a split, the file view is focused and
    /// takes the whole width; pressed again with only the view shown, the
    /// others come back and focus stays in the file view — you pressed `b` to
    /// read the file, so that is where you want to stay.
    pub(super) fn zoom_file_view(&mut self) {
        self.reveal_and_focus(Focus::View);
        self.zoom_focused();
    }

    /// Focus `pane`, showing it first if it is hidden, so the cursor never
    /// lands on a pane the user cannot see.
    ///
    /// Only `pane` is shown. The focus keys used to clear a zoom outright;
    /// now a zoom is only a hide, and `e` after `b` puts the explorer beside
    /// the view rather than bringing back the filter pane too.
    pub(super) fn reveal_and_focus(&mut self, pane: Focus) {
        self.chain_origin = None;
        self.panes.show(pane);
        self.focus = pane;
    }

    /// `E`, `F` and `global.hide.view`: take `pane` off the screen.
    ///
    /// The last shown pane is refused, with a message: a window with no
    /// pane in it has no key a user could find to undo it. Focus leaves a
    /// hidden pane by `Panes::hide`'s rule.
    pub(super) fn hide_pane(&mut self, pane: Focus) {
        match self.panes.hide(pane, self.focus) {
            Ok(focus) => {
                if focus != self.focus {
                    self.chain_origin = None;
                    self.focus = focus;
                }
                self.cancel_drag();
            }
            Err(panes::LastPane) => self.report(LAST_PANE, false),
        }
    }

    /// A drag in progress has no divider to keep tracking once a pane is
    /// hidden or shown — the `Drag` arm in `handle_divider` only checks
    /// `self.dragging`, not whether that divider is still on the screen — so
    /// it would otherwise go on silently re-pinning a width nothing explains.
    /// Every layout change cancels it outright.
    fn cancel_drag(&mut self) {
        self.dragging = None;
    }

    /// `f`: focus the filter pane, and remember where focus came from so a
    /// chain — `f i … Enter` — can go back there. A filter pane that was
    /// hidden is shown for the chain and hidden again when it returns.
    pub(super) fn start_filter_chain(&mut self) {
        let origin = (self.focus != Focus::Filters).then_some(self.focus);
        let shown = !self.panes.is_shown(Focus::Filters);
        self.reveal_and_focus(Focus::Filters);
        self.chain_origin = origin;
        self.chain_shown_filters = shown && origin.is_some();
    }

    /// End a chain that just committed: focus goes back to where `f` was
    /// pressed, and the app behaves as if `n` were pressed there — the
    /// first `fn` after `f i fn Enter` from the view, the next matching
    /// file from the explorer. Dispatching a real `n` rather than calling
    /// either step directly is what keeps "as if `n`" true per pane.
    ///
    /// Nothing to do when `f` was not what brought focus here.
    pub(super) fn return_to_chain_origin(&mut self) {
        let Some(origin) = self.chain_origin.take() else {
            return;
        };
        self.reveal_and_focus(origin);
        // `f` showed the filter pane to start this chain; the chain is over,
        // so it goes back to hidden. A chain does not change the layout.
        if std::mem::take(&mut self.chain_shown_filters) && origin != Focus::Filters {
            let _ = self.panes.hide(Focus::Filters, self.focus);
        }
        // The commit that got us here changed the filter set, but the
        // outer `handle_event` loop has not run `refresh_scan` yet — this
        // is still inside the same `dispatch_event` that is doing the
        // committing. Re-key the marks to `Unknown` now, before the
        // synthetic `n` below reads them, or it would step to (or fail to
        // find) a match against the filter set the user just replaced.
        self.refresh_scan(false);
        self.dispatch_event(event::Event::Key(event::KeyEvent::from(KeyCode::Char('n'))));
        // The synthetic `n` spent the bounce guard; re-arm it, since the
        // `Enter` that committed is still the last key the user pressed.
        self.swallow_next_enter = true;
    }

    /// Mark each pane as focused or not, before drawing.
    ///
    /// Three assignments rather than an enumerate-and-compare over a vec: the
    /// index that loop compared against no longer exists (#73).
    pub(super) fn set_active_pane(&mut self) {
        self.explorer.set_active(self.focus == Focus::Explorer);
        self.view.set_active(self.focus == Focus::View);
        self.filters_pane.set_active(self.focus == Focus::Filters);
    }
}
