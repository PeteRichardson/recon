//! Pane geometry: how wide the navigator is, how tall the filter pane is, and
//! where the two dividers sit.
//!
//! Split out of `impl App` (#74), which had grown to 1,762 lines mixing this
//! with event routing, viewport translation, editor launching and status-line
//! formatting. The methods are unchanged — this is a move along a seam that
//! already existed, not a rewrite.
//!
//! What makes this a real seam rather than an arbitrary cut: everything here
//! answers "how big is each pane, given the terminal" from `App`'s own sizing
//! fields (`nav_width`, `filter_height`, `divider`, `filter_area`,
//! `dragging`). None of it touches the document, the filters, or the panes'
//! contents. The one exception is deliberate — `nav_width` asks the navigator
//! and the filter pane what width they would prefer, because an automatic
//! width that ignored its contents would clip them.
//!
//! `handle_divider` moved here alongside `divider_at` even though #74 named
//! only the latter. They are one mechanism read from two ends — `divider_at`
//! hit-tests a click against the last frame's boundaries, `handle_divider`
//! turns the resulting drag into a new size — and leaving half of it in
//! `lib.rs` would have split the pair that the geometry constants exist for.

use crate::App;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::Rect;
use std::time::{Duration, Instant};

/// Widest the nav pane will size itself to automatically.
pub(crate) const MAX_NAV_WIDTH: u16 = 40;

/// Narrowest either pane may be dragged, so a bordered block still renders.
///
/// This is nav's own floor — how little it may have — and is independent of
/// `MIN_FILE_VIEW_WIDTH` below, which bounds how much it may *take*. A drag
/// to the far edge, or a directory of short filenames, may still leave nav
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
pub(crate) const MIN_AUTO_NAV_WIDTH: u16 = 20;

/// Rows the navigator keeps even when the filter pane's stacked below it
/// wants more than the terminal can spare.
///
/// This used to be enforced by giving the navigator `Min(MIN_NAV_HEIGHT)`
/// and the filter pane `Length(filter_height)`, on the claim that `Min`
/// beats `Length` for priority. It doesn't: in ratatui-core 0.1.2 `Length`
/// adds its equality constraint an order of magnitude *stronger* than `Min`
/// adds its bound, so the filter pane's `Length` was actually the one
/// winning — a bare `Min(0)` navigator constraint could be squeezed to zero
/// rows by a tall filter pane while still being the *focused* pane,
/// stranding the user on a cursor they cannot see. That this is backwards
/// from what the constraint names suggest fooled an implementer and a
/// re-reviewer in turn.
///
/// The floor is now arithmetic instead of leaned on the solver: `App::render`
/// caps `filter_height` at `left.height.saturating_sub(MIN_NAV_HEIGHT)`
/// before handing it to `Length`, so the navigator's own constraint can be
/// `Min(0)` and still never drop below this floor whenever the terminal has
/// at least `MIN_NAV_HEIGHT` rows to give the left column in total. Below
/// that — a terminal shorter than the floor itself — the cap saturates to
/// zero, the filter pane gets nothing, and the navigator takes whatever the
/// terminal has, however little that is; there is no lower floor to fall
/// back to at that point. `MIN_PANE_WIDTH`'s reasoning applied to the other
/// axis: enough for a bordered block to render at all (top border, one
/// content row, bottom border), not enough to call comfortable.
pub(crate) const MIN_NAV_HEIGHT: u16 = 3;

/// Rows the filter pane sizes itself to *automatically*, at minimum.
///
/// The height analogue of `MIN_AUTO_NAV_WIDTH`, and it exists for the same
/// kind of reason: a pane sized purely by its contents is not necessarily a
/// pane sized usefully. An empty set asked for three rows — a title on the
/// top border, one row for the hint, a bottom border — which is the smallest
/// thing that can be drawn rather than a considered size, and it read as an
/// afterthought stuck under the navigator. #44 asks for the pane to look like
/// the headline feature it is before the first filter exists, on the grounds
/// that recon is heading further in a filter-forward direction, not less.
///
/// A floor, not a fixed height: a larger set still gets the rows it asks for,
/// up to the same two caps in `filter_pane_split_height` that have always
/// bounded it. Those caps also outrank this floor, so a short terminal is
/// unaffected — the navigator's floor and the half share are still what
/// govern there, and this only shows up once there is room to honour it.
///
/// Eight is a judgement call, as `MIN_AUTO_NAV_WIDTH`'s twenty is: six
/// content rows inside the borders, which holds a working filter set without
/// scrolling while still leaving the navigator the larger share of any
/// terminal tall enough for the floor to apply at all. Like that constant, a
/// good candidate for a config entry.
pub(crate) const MIN_AUTO_FILTER_HEIGHT: u16 = 8;

/// Fewest rows a *dragged* filter pane may be left with — the height
/// analogue of `MIN_PANE_WIDTH`, and its counterpart in the same way
/// `MIN_AUTO_FILTER_HEIGHT` is `MIN_AUTO_NAV_WIDTH`'s.
///
/// Distinct from `MIN_AUTO_FILTER_HEIGHT` above, which bounds a height nobody
/// asked for; this bounds a decision, and so is much smaller: top border, one
/// content row, bottom border. A user dragging the pane down to nothing
/// evidently wants it out of the way, and the app's answer to that is to keep
/// one usable row rather than to argue — the same trade `MIN_PANE_WIDTH`
/// makes, and the same reason a collapsed-but-focusable pane is the outcome
/// to avoid (see `MIN_NAV_HEIGHT`).
pub(crate) const MIN_FILTER_HEIGHT: u16 = 3;

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
/// Used as the ceiling on how much of the terminal the left column may
/// claim, in both `nav_width`'s auto-sizing and pinned (dragged) branches,
/// so a deliberate drag cannot starve the view any more than auto-sizing
/// can — the app already refuses to let a drag collapse a pane outright
/// (`MIN_PANE_WIDTH`); this is the same principle at a usable threshold.
pub(crate) const MIN_FILE_VIEW_WIDTH: u16 = 2 + (6 + 2) + 20;

/// Two clicks on the divider inside this window restore automatic sizing.
/// Crossterm does not report double-clicks, so they are timed here.
pub(crate) const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How the nav pane's width is decided.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavWidth {
    /// Snap to the longest entry, capped at `MAX_NAV_WIDTH`.
    #[default]
    Auto,
    /// Held at the width the user dragged to.
    Pinned(u16),
}

/// How the filter pane's height is decided — `NavWidth` on the other axis.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilterHeight {
    /// Size to the filter set, floored at `MIN_AUTO_FILTER_HEIGHT`.
    #[default]
    Auto,
    /// Held at the height the user dragged to.
    ///
    /// A height rather than the boundary row the drag actually reported: the
    /// pane is anchored to the bottom of the left column, so the row that
    /// means "six rows of filters" moves whenever the terminal is resized,
    /// and a stored row would silently mean something different afterwards.
    Pinned(u16),
}

/// Which pane boundary a drag in progress is moving.
///
/// The two dividers are hit-tested against different things — one a column,
/// one a row within the left column — but share a drag: a mouse that went
/// down on one must keep moving that one until it comes up, whatever it
/// passes over on the way. Naming the axis in the state is what guarantees
/// that; two independent `bool`s could both be set and would have to encode
/// the same invariant by convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Divider {
    /// Between the left column and the file view.
    Vertical,
    /// Between the navigator and the filter pane stacked below it.
    Horizontal,
}

impl App<'_> {
    /// Rows the filter pane wants when it is on screen.
    ///
    /// Not zero while no filter exists: `preferred_height` floors at one
    /// content row even for an empty set, so its hint has somewhere to draw
    /// (see `EMPTY_HINTS`).
    ///
    /// This used to carry an `unwrap_or(0)` documented as unreachable, because
    /// the pane had to be found by scanning a vec that might in principle not
    /// contain one. The pane is a field now, so there is no absent case left
    /// to describe (#73).
    pub(crate) fn filter_pane_height(&self) -> u16 {
        self.filters_pane
            .preferred_height(crate::widgets::filterlist::rows(&self.filters).len())
    }

    /// How much of `left_height` (the left column's total rows) the filter
    /// pane gets, out of the navigator's floor `MIN_NAV_HEIGHT`.
    ///
    /// The filter pane gets its preferred height, capped at whatever is left
    /// over once the navigator's floor is set aside — expressed as
    /// arithmetic rather than leaned on how ratatui's constraint solver
    /// weighs `Min` against `Length` (see `MIN_NAV_HEIGHT`'s doc comment for
    /// why that was the wrong thing to lean on). Kept as its own method,
    /// rather than inlined at its one call site in `render`, so the two
    /// floors this expresses are directly testable without going through a
    /// full render and inspecting cells for it.
    ///
    /// Also capped at half of `left_height`: `preferred_height` alone grows
    /// without bound as filters are added, so on an ordinary terminal a
    /// filter set that grows past a handful would otherwise pin the
    /// navigator at its bare floor *permanently* rather than only on a
    /// genuinely short terminal — the floor is meant as a last resort, not
    /// the navigator's everyday allotment. `List`/`ListState` already
    /// scrolls, so a capped pane loses nothing but simultaneous visibility:
    /// every filter stays reachable. The two caps compose via `min`: on a
    /// short terminal the floor-based one is tighter and wins, exactly as
    /// before this cap existed; on a tall one the half-based one is tighter
    /// and gives the navigator a proportional share instead of the bare
    /// floor.
    ///
    /// The preferred height is floored at `MIN_AUTO_FILTER_HEIGHT` before
    /// either cap applies, which is what makes the pane open at a usable size
    /// with nothing in it (#44). Deliberately *inside* the caps rather than
    /// applied to the result: a floor that outranked them would hand an empty
    /// pane eight of a twelve-row column, which is the navigator-starving
    /// behaviour the caps exist to prevent — and it would do it for a pane
    /// that has nothing to show.
    /// A dragged height skips both the starting floor and the half cap, and
    /// keeps only the navigator's floor. Both of the ones it skips exist to
    /// stop *automatic* sizing producing a silly split; a drag is a decision,
    /// and the same reasoning `MIN_AUTO_NAV_WIDTH` and `MIN_PANE_WIDTH` split
    /// on the other axis applies unchanged. What it keeps is the one bound
    /// that is not about taste: a navigator squeezed to nothing while still
    /// focusable strands the user on a cursor they cannot see.
    pub(crate) fn filter_pane_split_height(&self, left_height: u16) -> u16 {
        let wanted = match self.filter_height {
            FilterHeight::Auto => self
                .filter_pane_height()
                .max(MIN_AUTO_FILTER_HEIGHT)
                .min(left_height / 2),
            FilterHeight::Pinned(rows) => rows.max(MIN_FILTER_HEIGHT),
        };
        // Applied last, and to both branches, so that on a terminal too short
        // to honour any of this the navigator is what survives.
        wanted.min(left_height.saturating_sub(MIN_NAV_HEIGHT))
    }

    /// Resolve the nav pane's width within `area`.
    pub(crate) fn nav_width(&self, area: Rect) -> u16 {
        let width = match self.nav_width {
            // The column has to fit whichever pane currently wants more:
            // the navigator's longest entry, or the filter pane's longest
            // row. Either alone could otherwise get silently clipped by the
            // other's narrower automatic width.
            NavWidth::Auto => {
                let nav_width = self.nav.preferred_width();
                let filter_width = self.filters_pane.preferred_width(&self.filters);
                // Clamped, not just `.max`: the floor must not push a column
                // past the cap when both apply.
                nav_width
                    .max(filter_width)
                    .clamp(MIN_AUTO_NAV_WIDTH, MAX_NAV_WIDTH)
            }
            NavWidth::Pinned(width) => width,
        };

        // Whatever the source — auto-sizing or a drag — nav may not claim so
        // much that the file view drops below a genuinely usable width.
        // `MIN_PANE_WIDTH` is the fallback once the terminal is too narrow
        // even for that (its `.max` below): nav's own floor, unrelated to
        // this ceiling, is applied last.
        let widest = area
            .width
            .saturating_sub(MIN_FILE_VIEW_WIDTH)
            .max(MIN_PANE_WIDTH);
        width.clamp(MIN_PANE_WIDTH, widest)
    }

    /// Handle divider dragging, reporting whether the event was consumed.
    ///
    /// Anything not aimed at the divider falls through to the focused widget,
    /// so the file view keeps its scroll-wheel behaviour.
    pub(crate) fn handle_divider(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(divider) = self.divider_at(mouse.column, mouse.row) else {
                    return false;
                };
                let now = Instant::now();
                let double_click = self.last_divider_click.is_some_and(|(last, at)| {
                    last == divider && now.duration_since(at) <= DOUBLE_CLICK
                });

                if double_click {
                    match divider {
                        Divider::Vertical => self.nav_width = NavWidth::Auto,
                        Divider::Horizontal => self.filter_height = FilterHeight::Auto,
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
                    Some(Divider::Vertical) => self.nav_width = NavWidth::Pinned(mouse.column),
                    // The pane runs from wherever the mouse now is to the
                    // bottom of the left column, which is the one part of the
                    // last frame's geometry a row alone cannot supply.
                    Some(Divider::Horizontal) => {
                        self.filter_height = FilterHeight::Pinned(
                            self.filter_area.bottom().saturating_sub(mouse.row),
                        );
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
    /// A divider is the pair of adjacent borders between two panes, with a
    /// cell of slack either side so it is not fiddly to grab.
    ///
    /// The vertical one is tested first and so wins the single corner where
    /// the two meet. That is an arbitrary choice between two reasonable ones,
    /// but it has to be made somewhere and pinned — see
    /// `the_vertical_divider_wins_where_the_two_cross`.
    ///
    /// The horizontal one is additionally bounded to the left column, because
    /// that is the only place it exists: without that half of the test, a
    /// click anywhere across the file view at the same height would resize
    /// the filter pane.
    pub(crate) fn divider_at(&self, column: u16, row: u16) -> Option<Divider> {
        if column.abs_diff(self.divider) <= 1 {
            Some(Divider::Vertical)
        } else if column < self.divider && row.abs_diff(self.filter_area.y) <= 1 {
            Some(Divider::Horizontal)
        } else {
            None
        }
    }
}
