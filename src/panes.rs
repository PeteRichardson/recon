//! Which panes are shown, and the rules that change it (#300).
//!
//! The three panes sit in one row, `[explorer | file view | filter pane]`,
//! and any of them can be hidden. The rules are few, but each one reads the
//! others: a hide moves focus, `Tab` skips what a hide took away, and a zoom
//! is only a hide of every pane but one. They live here, apart from `App`,
//! so the whole set can be tested without a terminal, a document or a scan.
//!
//! Zoom is not a state of its own. It used to be `zoom: Option<Focus>`,
//! which could name one pane or none and so could not say "the explorer is
//! hidden but the other two are not". Now a zoom is a hide, and what `z`
//! does comes from what is on the screen when it is pressed: more than one
//! pane shown, and it hides all but the focused one; only one, and it shows
//! the others again.

use crate::widgets::Focus;
use serde::Deserialize;

/// A pane by the name `config.toml` and `--hide-pane` use.
///
/// Public, unlike `Focus`, only because `Config` carries it across the crate
/// boundary. The names have no short forms: `view`, not `v` or `file`, so a
/// config file reads the same as the glossary in `CONTEXT.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Pane {
    Explorer,
    View,
    Filters,
}

impl From<Pane> for Focus {
    fn from(pane: Pane) -> Self {
        match pane {
            Pane::Explorer => Self::Explorer,
            Pane::View => Self::View,
            Pane::Filters => Self::Filters,
        }
    }
}

/// Every pane, in `Tab` order: left to right.
const ORDER: [Focus; 3] = [Focus::Explorer, Focus::View, Focus::Filters];

/// A set of panes. Three bits, one per pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaneSet(u8);

impl PaneSet {
    pub(crate) const ALL: Self = Self(0b111);

    fn bit(pane: Focus) -> u8 {
        match pane {
            Focus::Explorer => 0b001,
            Focus::View => 0b010,
            Focus::Filters => 0b100,
        }
    }

    pub(crate) fn only(pane: Focus) -> Self {
        Self(Self::bit(pane))
    }

    pub(crate) fn contains(self, pane: Focus) -> bool {
        self.0 & Self::bit(pane) != 0
    }

    fn with(self, pane: Focus) -> Self {
        Self(self.0 | Self::bit(pane))
    }

    fn without(self, pane: Focus) -> Self {
        Self(self.0 & !Self::bit(pane))
    }

    pub(crate) fn len(self) -> u32 {
        self.0.count_ones()
    }
}

/// A hide that would leave no pane on the screen. Refused, and the caller
/// says so on the status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LastPane;

/// The panes on the screen, and what a zoom remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Panes {
    shown: PaneSet,
    /// What the last zoom hid the others from, so the next one can put them
    /// back. `None` when nothing is remembered: a zoom from one pane then
    /// shows all three.
    remembered: Option<PaneSet>,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            shown: PaneSet::ALL,
            remembered: None,
        }
    }
}

impl Panes {
    /// Every pane shown except `hidden`.
    ///
    /// Hiding all three is refused before this is reached — `Config::load`
    /// stops with an error — so a list that names all three can only come
    /// from a `Config` built by hand. The file view is kept then, rather than
    /// a window with nothing in it.
    pub(crate) fn hiding(hidden: impl IntoIterator<Item = Focus>) -> Self {
        let mut shown = hidden.into_iter().fold(PaneSet::ALL, PaneSet::without);
        if shown.len() == 0 {
            shown = PaneSet::only(Focus::View);
        }
        Self {
            shown,
            remembered: None,
        }
    }

    pub(crate) fn is_shown(self, pane: Focus) -> bool {
        self.shown.contains(pane)
    }

    pub(crate) fn shown(self) -> PaneSet {
        self.shown
    }

    /// Put `pane` on the screen. A no-op when it already is.
    pub(crate) fn show(&mut self, pane: Focus) {
        self.shown = self.shown.with(pane);
    }

    /// Take `pane` off the screen, and say where focus goes.
    ///
    /// Focus moves only when it was on `pane`: to the file view if the file
    /// view is still shown — it is what the other two panes serve — and
    /// otherwise to the next shown pane in `Tab` order. A pane already hidden
    /// stays hidden and leaves focus where it is.
    pub(crate) fn hide(&mut self, pane: Focus, focus: Focus) -> Result<Focus, LastPane> {
        if !self.is_shown(pane) {
            return Ok(focus);
        }
        let shown = self.shown.without(pane);
        if shown.len() == 0 {
            return Err(LastPane);
        }
        self.shown = shown;
        if focus != pane {
            return Ok(focus);
        }
        if self.is_shown(Focus::View) {
            return Ok(Focus::View);
        }
        Ok(self.next(focus))
    }

    /// `z`. More than one pane shown: hide all but `focus`, and remember what
    /// was shown. Only one: show what was remembered, or all three.
    ///
    /// The focused pane is always in the result. A remembered set need not
    /// hold it — `z` in the view, `f`, then hide the view — and a restore
    /// that took focus off the screen would strand the cursor.
    pub(crate) fn zoom(&mut self, focus: Focus) {
        if self.shown.len() > 1 {
            self.remembered = Some(self.shown);
            self.shown = PaneSet::only(focus);
        } else {
            self.shown = self.remembered.take().unwrap_or(PaneSet::ALL).with(focus);
        }
    }

    /// The next shown pane after `from` in `Tab` order, wrapping. `from`
    /// itself when it is the only one.
    pub(crate) fn next(self, from: Focus) -> Focus {
        self.step(from, 1)
    }

    /// `Shift-Tab`: the other way round.
    pub(crate) fn prev(self, from: Focus) -> Focus {
        self.step(from, ORDER.len() - 1)
    }

    fn step(self, from: Focus, by: usize) -> Focus {
        let start = ORDER.iter().position(|&pane| pane == from).unwrap_or(0);
        (1..=ORDER.len())
            .map(|offset| ORDER[(start + offset * by) % ORDER.len()])
            .find(|&pane| self.is_shown(pane))
            .unwrap_or(from)
    }

    /// A shown pane for focus to rest on, starting from `focus`: `focus`
    /// itself when shown, else the file view, else the next in `Tab` order.
    /// The same rule a hide follows, for a startup that hid the default pane.
    pub(crate) fn settle(self, focus: Focus) -> Focus {
        if self.is_shown(focus) {
            focus
        } else if self.is_shown(Focus::View) {
            Focus::View
        } else {
            self.next(focus)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Focus::{Explorer, Filters, View};

    fn shown(panes: Panes) -> Vec<Focus> {
        ORDER.into_iter().filter(|&p| panes.is_shown(p)).collect()
    }

    #[test]
    fn every_pane_starts_shown() {
        assert_eq!(shown(Panes::default()), [Explorer, View, Filters]);
    }

    #[test]
    fn each_pane_can_be_hidden_and_shown_again() {
        for pane in ORDER {
            let mut panes = Panes::default();
            panes.hide(pane, View).expect("two others remain");
            assert!(!panes.is_shown(pane), "{pane:?} still shown");
            panes.show(pane);
            assert!(panes.is_shown(pane), "{pane:?} not shown again");
        }
    }

    #[test]
    fn the_last_shown_pane_cannot_be_hidden() {
        let mut panes = Panes::hiding([Explorer, Filters]);
        assert_eq!(panes.hide(View, View), Err(LastPane));
        assert!(panes.is_shown(View));
    }

    #[test]
    fn hiding_the_focused_pane_moves_focus_to_the_view() {
        let mut panes = Panes::default();
        assert_eq!(panes.hide(Filters, Filters), Ok(View));
        let mut panes = Panes::default();
        assert_eq!(panes.hide(Explorer, Explorer), Ok(View));
    }

    #[test]
    fn hiding_the_focused_view_moves_focus_to_the_next_pane() {
        let mut panes = Panes::default();
        assert_eq!(panes.hide(View, View), Ok(Filters));
        let mut panes = Panes::hiding([Filters]);
        assert_eq!(panes.hide(View, View), Ok(Explorer), "Tab order wraps");
    }

    #[test]
    fn hiding_another_pane_leaves_focus_alone() {
        let mut panes = Panes::default();
        assert_eq!(panes.hide(Filters, Explorer), Ok(Explorer));
    }

    #[test]
    fn tab_skips_hidden_panes() {
        let panes = Panes::hiding([View]);
        assert_eq!(panes.next(Explorer), Filters);
        assert_eq!(panes.next(Filters), Explorer);
        assert_eq!(panes.prev(Explorer), Filters);
        assert_eq!(panes.prev(Filters), Explorer);
        let one = Panes::hiding([Explorer, Filters]);
        assert_eq!(one.next(View), View);
        assert_eq!(one.prev(View), View);
    }

    #[test]
    fn zoom_hides_all_but_the_focused_pane_and_restores_them() {
        let mut panes = Panes::hiding([Explorer]);
        panes.zoom(Filters);
        assert_eq!(shown(panes), [Filters]);
        panes.zoom(Filters);
        assert_eq!(shown(panes), [View, Filters], "the explorer stays hidden");
    }

    #[test]
    fn zoom_from_one_pane_with_nothing_remembered_shows_all() {
        let mut panes = Panes::hiding([Explorer, Filters]);
        panes.zoom(View);
        assert_eq!(shown(panes), [Explorer, View, Filters]);
    }

    /// The example in #300: `z` in the view, `f`, `z`. Two panes are shown
    /// at the second `z`, so it zooms the filter pane. It does not restore.
    #[test]
    fn zoom_reads_the_screen_not_a_mode() {
        let mut panes = Panes::default();
        panes.zoom(View);
        panes.show(Filters);
        panes.zoom(Filters);
        assert_eq!(shown(panes), [Filters]);
        panes.zoom(Filters);
        assert_eq!(shown(panes), [View, Filters], "what the second z hid");
    }

    #[test]
    fn a_restore_always_includes_the_focused_pane() {
        let mut panes = Panes::hiding([Filters]);
        panes.zoom(View);
        panes.show(Filters);
        panes.hide(View, Filters).expect("the filter pane remains");
        panes.zoom(Filters);
        assert_eq!(shown(panes), [Explorer, View, Filters]);
    }

    #[test]
    fn hiding_all_three_by_hand_keeps_the_view() {
        assert_eq!(shown(Panes::hiding(ORDER)), [View]);
    }

    #[test]
    fn settle_prefers_the_view_then_tab_order() {
        assert_eq!(Panes::hiding([Explorer]).settle(Explorer), View);
        assert_eq!(Panes::hiding([Explorer, View]).settle(Explorer), Filters);
        assert_eq!(Panes::default().settle(Explorer), Explorer);
    }
}
