//! Pane geometry: how wide each of the three columns is, and where the two
//! dividers sit.
//!
//! Split out of `impl App` (#74), which had grown to 1,762 lines mixing this
//! with event routing, viewport translation, editor launching and status-line
//! formatting.
//!
//! What makes this a real seam rather than an arbitrary cut: everything here
//! answers "how big is each pane, given the terminal" from `App`'s own sizing
//! fields (`explorer_width`, `filter_width`, `divider`, `filter_divider`,
//! `dragging`) and which panes are shown. None of it touches the document or
//! the panes' contents, with one deliberate exception — each side pane is
//! asked what width it would prefer, because an automatic width that ignored
//! its contents would clip them.
//!
//! The filter pane used to sit under the explorer, sharing its column and
//! sized by height. It is a column of its own on the right now (#300), so a
//! long filter set has the full height of the terminal, and both dividers
//! are vertical.
//!
//! `handle_divider` moved here alongside `divider_at` even though #74 named
//! only the latter. They are one mechanism read from two ends — `divider_at`
//! hit-tests a click against the last frame's boundaries, `handle_divider`
//! turns the resulting drag into a new size — and leaving half of it in
//! `lib.rs` would have split the pair that the geometry constants exist for.

use super::App;
use crate::panes::PaneSet;
use crate::widgets::Focus;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::Rect;
use std::time::{Duration, Instant};

/// Widest the explorer pane will size itself to automatically.
pub(crate) const MAX_EXPLORER_WIDTH: u16 = 40;

/// Narrowest either pane may be dragged, so a bordered block still renders.
///
/// This is explorer's own floor — how little it may have — and is independent of
/// `MIN_FILE_VIEW_WIDTH` below, which bounds how much it may *take*. A drag
/// to the far edge, or a directory of short filenames, may still leave explorer
/// narrower than `MIN_FILE_VIEW_WIDTH` would ask for; that is fine, since
/// nothing at that end is starving the file view.
pub(crate) const MIN_PANE_WIDTH: u16 = 3;

/// Narrowest the column sizes itself to *automatically*.
///
/// Distinct from `MIN_PANE_WIDTH`, which bounds a drag. A drag is a decision
/// and may still go narrower; this bounds a width nobody asked for.
///
/// Snapping to the longest entry with no floor meant entering a directory of
/// one short name took the column from 40 columns to 6 and moved every pane
/// on screen — see #33. That is the defect #26 fixed on the vertical axis,
/// where a layout shifting under the user was worth fixing for one row; this
/// was shifting by nearly forty columns.
///
/// Automatic sizing exists to stop the column being uselessly *wide*. Making
/// it as narrow as possible is a different goal, and a poor trade: the dozen
/// columns it wins for the file view cost a relayout mid-navigation. The ways
/// to actually maximise the file view are explicit and already there — `b`,
/// `z`, and a drag, which pins the width outright.
///
/// 20 fits an 18-character name inside the borders, which covers most of what
/// a source or log directory holds. It is a judgement call rather than a fact,
/// and a good candidate for a config entry once #18 lands.
pub(crate) const MIN_AUTO_EXPLORER_WIDTH: u16 = 20;

/// Widest the filter pane will size itself to automatically (#300).
///
/// The same cap as the explorer's, and for the same reason: automatic sizing
/// exists to fit the contents, not to hand one long pattern the terminal. A
/// row longer than this is cut at the pane's edge; a drag on the divider
/// makes the pane wider when the whole pattern matters.
pub(crate) const MAX_FILTER_WIDTH: u16 = 40;

/// Narrowest the filter pane sizes itself to automatically — the width
/// analogue of the old `MIN_AUTO_FILTER_HEIGHT` (#44), now that the pane is a
/// column. An empty pane still opens wide enough for its whole hint, "press
/// f i to add", inside the borders, so it reads as a place filters go rather
/// than a sliver.
pub(crate) const MIN_AUTO_FILTER_WIDTH: u16 = 20;

/// Columns the file view needs to stay genuinely readable, not merely
/// present — derived, not tuned:
/// - 2 for its own left and right border columns.
/// - The gutter's overhead, `digits + 2` (one padding column plus the
///   trailing space after the number — see `LineHighlighter::line_number`
///   in `vendor/tui-textarea-2/src/highlight.rs`), budgeted at 6 digits: a
///   log under a million lines comfortably fits, and this project has
///   already exercised a 70,000-line file (`the_round_trip_survives_more_than_65535_lines`).
/// - 20 for a recognisable fragment of a line: the length of an ISO 8601
///   timestamp (`2024-01-01T12:00:00`, 19 characters) plus a trailing space
///   — a reasonable proxy for "the start of a real log line", not an
///   arbitrary round number.
///
/// Used as the ceiling on how much of the terminal the explorer and the
/// filter pane may claim together, in both their auto-sizing and pinned
/// (dragged) branches, so a deliberate drag cannot starve the view any more
/// than auto-sizing can — the app already refuses to let a drag collapse a
/// pane outright (`MIN_PANE_WIDTH`); this is the same principle at a usable
/// threshold. Only while the file view is shown: a hidden view has no floor.
pub(crate) const MIN_FILE_VIEW_WIDTH: u16 = 2 + (6 + 2) + 20;

/// Two clicks on the divider inside this window restore automatic sizing.
/// Crossterm does not report double-clicks, so they are timed here.
pub(crate) const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The time a click is read at, for telling a double-click from two clicks.
///
/// `Instant::now` unless a test has set it. Two real clicks in a test are
/// two `Instant::now` calls with a render or a directory listing between
/// them, so a busy machine could make them more than `DOUBLE_CLICK` apart
/// and turn a double-click into two single ones (#391).
#[derive(Debug, Default)]
pub(crate) struct ClickClock(Option<Instant>);

impl ClickClock {
    pub(crate) fn now(&self) -> Instant {
        self.0.unwrap_or_else(Instant::now)
    }

    /// Read every later click at `at`, until set again.
    #[cfg(test)]
    pub(crate) fn set(&mut self, at: Instant) {
        self.0 = Some(at);
    }

    /// Whether a test has set the time.
    #[cfg(test)]
    pub(crate) fn is_set(&self) -> bool {
        self.0.is_some()
    }
}

/// How a side pane's width is decided — the explorer's and the filter
/// pane's alike.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneWidth {
    /// Snap to the longest row, between the pane's floor and its cap.
    #[default]
    Auto,
    /// Held at the width the user dragged to.
    Pinned(u16),
}

/// Which pane boundary a drag in progress is moving.
///
/// Both dividers are columns now (#300), but they still move different
/// things — one the explorer's width, one the filter pane's — and a mouse
/// that went down on one must keep moving that one until it comes up,
/// whatever it passes over on the way. Naming the divider in the state is
/// what guarantees that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Divider {
    /// The explorer's right edge: between the explorer and whatever is
    /// shown to its right.
    Explorer,
    /// The filter pane's left edge, between the file view and the filter
    /// pane. There is none while the file view is hidden: the filter pane
    /// then fills what the explorer leaves, so the explorer's divider is the
    /// only one.
    Filters,
}

/// The widths of the three panes, `[explorer, view, filters]`, across
/// `width` columns. A hidden pane gets zero.
///
/// `explorer` and `filters` are what each side pane would like, auto or
/// pinned. The file view takes the rest and keeps `MIN_FILE_VIEW_WIDTH`
/// while the terminal allows it: the filter pane gives way first, down to
/// `MIN_PANE_WIDTH`, and then the explorer. No pane is ever hidden to make
/// room — on a terminal too narrow for every floor the view is what goes
/// short. With the view hidden, the filter pane fills what the explorer
/// leaves; a lone pane fills everything.
///
/// A free function over plain numbers so the priority order is testable
/// without an `App`.
pub(crate) fn columns(width: u16, shown: PaneSet, explorer: u16, filters: u16) -> [u16; 3] {
    let has = |pane| shown.contains(pane);
    if !has(Focus::View) {
        return match (has(Focus::Explorer), has(Focus::Filters)) {
            (true, true) => {
                let explorer = explorer
                    .min(width.saturating_sub(MIN_PANE_WIDTH))
                    .max(MIN_PANE_WIDTH)
                    .min(width);
                [explorer, 0, width - explorer]
            }
            (true, false) => [width, 0, 0],
            _ => [0, 0, width],
        };
    }
    let room = width.saturating_sub(MIN_FILE_VIEW_WIDTH);
    let mut explorer = if has(Focus::Explorer) {
        explorer.max(MIN_PANE_WIDTH)
    } else {
        0
    };
    let mut filters = if has(Focus::Filters) {
        filters.max(MIN_PANE_WIDTH)
    } else {
        0
    };
    if explorer + filters > room && filters > 0 {
        filters = room.saturating_sub(explorer).max(MIN_PANE_WIDTH);
    }
    if explorer + filters > room && explorer > 0 {
        explorer = room.saturating_sub(filters).max(MIN_PANE_WIDTH);
    }
    // Below every floor: the side panes keep what they have while it fits,
    // and the view takes whatever is left, which may be nothing.
    let explorer = explorer.min(width);
    let filters = filters.min(width - explorer);
    [explorer, width - explorer - filters, filters]
}

impl App<'_> {
    /// The width the explorer asks for, before `columns` fits it in.
    pub(crate) fn explorer_wanted(&self) -> u16 {
        match self.explorer_width {
            PaneWidth::Auto => self
                .explorer
                .preferred_width()
                .clamp(MIN_AUTO_EXPLORER_WIDTH, MAX_EXPLORER_WIDTH),
            PaneWidth::Pinned(width) => width,
        }
    }

    /// The width the filter pane asks for, before `columns` fits it in.
    ///
    /// Floored at `MIN_AUTO_FILTER_WIDTH` so an empty pane still has room for
    /// its hint; `preferred_width` itself leaves the hint out.
    pub(crate) fn filter_wanted(&self) -> u16 {
        match self.filter_width {
            PaneWidth::Auto => self
                .filters_pane
                .preferred_width(&self.filters)
                .clamp(MIN_AUTO_FILTER_WIDTH, MAX_FILTER_WIDTH),
            PaneWidth::Pinned(width) => width,
        }
    }

    /// The three panes' rectangles within `area`, `[explorer, view,
    /// filters]`, left to right. A hidden pane's is zero wide, so nothing
    /// hit-tests against it.
    pub(crate) fn pane_rects(&self, area: Rect) -> [Rect; 3] {
        let [explorer, view, filters] = columns(
            area.width,
            self.panes.shown(),
            self.explorer_wanted(),
            self.filter_wanted(),
        );
        let at = |x: u16, width: u16| Rect { x, width, ..area };
        [
            at(area.x, explorer),
            at(area.x + explorer, view),
            at(area.x + explorer + view, filters),
        ]
    }

    /// The explorer's width within `area`, as the next render would draw it.
    #[cfg(test)]
    pub(crate) fn explorer_width(&self, area: Rect) -> u16 {
        self.pane_rects(area)[0].width
    }

    /// The filter pane's width within `area`, as the next render would draw
    /// it.
    #[cfg(test)]
    pub(crate) fn filter_pane_width(&self, area: Rect) -> u16 {
        self.pane_rects(area)[2].width
    }

    /// Handle divider dragging, reporting whether the event was consumed.
    ///
    /// Anything not aimed at a divider falls through to `handle_click` and
    /// `handle_wheel`, so a click and the wheel still reach the panes.
    pub(crate) fn handle_divider(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(divider) = self.divider_at(mouse.column, mouse.row) else {
                    return false;
                };
                let now = self.click_clock.now();
                let double_click = self.last_divider_click.is_some_and(|(last, at)| {
                    last == divider && now.duration_since(at) <= DOUBLE_CLICK
                });

                if double_click {
                    match divider {
                        Divider::Explorer => self.explorer_width = PaneWidth::Auto,
                        Divider::Filters => self.filter_width = PaneWidth::Auto,
                    }
                    self.last_divider_click = None;
                } else {
                    self.dragging = Some(divider);
                    self.last_divider_click = Some((divider, now));
                }
                true
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                match self.dragging {
                    Some(Divider::Explorer) => {
                        self.explorer_width =
                            PaneWidth::Pinned(mouse.column.saturating_sub(self.panes_area.x));
                    }
                    // The pane runs from wherever the mouse now is to the
                    // right edge of the window, which is the one part of the
                    // last frame's geometry a column alone cannot supply.
                    // A width rather than the column: the pane is anchored to
                    // the right, so a stored column would mean a different
                    // width after the terminal is resized.
                    Some(Divider::Filters) => {
                        self.filter_width =
                            PaneWidth::Pinned(self.panes_area.right().saturating_sub(mouse.column));
                    }
                    None => return false,
                }
                // A real drag rules out the next click being a double-click,
                // which would otherwise discard the size just set.
                self.last_divider_click = None;
                true
            }
            MouseEventKind::Up(MouseButton::Left) if self.dragging.is_some() => {
                self.dragging = None;
                true
            }
            _ => false,
        }
    }

    /// The divider under `(column, row)`, if either is.
    ///
    /// A divider is the pair of adjacent borders between two panes — one
    /// pane's right border and the next one's left — and exactly that pair,
    /// with no slack: a click on a row does something (#58), and slack would
    /// swallow a pane's first or last column.
    ///
    /// Both are tested against the column only. Each runs the full height of
    /// the panes, so a row cannot miss one; the status row below them is not
    /// a pane, and a click there never reaches this. The explorer's is tested
    /// first and so wins where a very narrow view puts the two side by side.
    pub(crate) fn divider_at(&self, column: u16, row: u16) -> Option<Divider> {
        if row >= self.panes_area.bottom() {
            return None;
        }
        // `column <= divider` first, so the subtraction cannot underflow: the
        // pair is the divider column and the one to its left, never the right.
        let on = |divider: u16| column <= divider && divider - column <= 1;
        if on(self.divider) {
            Some(Divider::Explorer)
        } else if on(self.filter_divider) {
            Some(Divider::Filters)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::Panes;
    use Focus::{Explorer, Filters, View};

    fn shown(hidden: &[Focus]) -> PaneSet {
        Panes::hiding(hidden.iter().copied()).shown()
    }

    #[test]
    fn a_wide_terminal_gives_each_side_pane_what_it_asks() {
        assert_eq!(columns(120, shown(&[]), 25, 30), [25, 65, 30]);
    }

    /// The filter pane gives way first, down to `MIN_PANE_WIDTH`, and then
    /// the explorer; the file view keeps `MIN_FILE_VIEW_WIDTH`.
    #[test]
    fn a_narrow_terminal_shrinks_the_filter_pane_first() {
        assert_eq!(columns(70, shown(&[]), 25, 30), [25, 30, 15]);
        assert_eq!(columns(50, shown(&[]), 25, 30), [17, 30, MIN_PANE_WIDTH]);
    }

    /// Below every floor the side panes keep their floors and the view goes
    /// short. No pane is hidden to make room.
    #[test]
    fn a_tiny_terminal_hides_nothing() {
        let [explorer, view, filters] = columns(10, shown(&[]), 25, 30);
        assert_eq!((explorer, filters), (MIN_PANE_WIDTH, MIN_PANE_WIDTH));
        assert_eq!(view, 4);
    }

    #[test]
    fn a_hidden_pane_gets_no_columns_and_the_view_takes_them() {
        assert_eq!(columns(120, shown(&[Explorer]), 25, 30), [0, 90, 30]);
        assert_eq!(columns(120, shown(&[Filters]), 25, 30), [25, 95, 0]);
        assert_eq!(
            columns(120, shown(&[Explorer, Filters]), 25, 30),
            [0, 120, 0]
        );
    }

    /// With the view hidden the filter pane fills what the explorer leaves,
    /// and a lone pane fills everything.
    #[test]
    fn with_the_view_hidden_the_filter_pane_fills_the_rest() {
        assert_eq!(columns(120, shown(&[View]), 25, 30), [25, 0, 95]);
        assert_eq!(columns(120, shown(&[View, Filters]), 25, 30), [120, 0, 0]);
        assert_eq!(columns(120, shown(&[View, Explorer]), 25, 30), [0, 0, 120]);
    }
}
