pub mod filenav;
pub mod fileview;
pub mod filterlist;

use color_eyre::Result;
use crossterm::event::Event;
use ratatui::prelude::{Buffer, Color, Rect, Style, Widget};
use ratatui::widgets::{Block, BorderType};
use std::path::PathBuf;

/// The bordered block every pane draws, carrying whether it has focus.
///
/// Focus used to be signalled by one thing: the selected row's foreground
/// turning green inside an already reverse-video highlight. That is a
/// low-contrast shift on a single row, while the border — the largest element
/// the pane owns — said nothing at all.
///
/// Colour *and* weight, not colour alone. This is the argument #19 makes about
/// the selection marker: a single visual channel fails on a theme with weak
/// contrast and for a colour-blind reader, and border weight survives a
/// terminal with no colour whatsoever. Green because it is already this app's
/// focus colour, so nothing new is introduced.
///
/// One helper rather than four call sites styling their own blocks — the
/// filter pane alone builds two, and they are exactly where a copy would drift.
pub fn pane_block<'a>(title: impl Into<ratatui::text::Line<'a>>, active: bool) -> Block<'a> {
    let block = Block::bordered().title(title);
    if active {
        block
            .border_type(BorderType::Thick)
            .border_style(Style::new().fg(Color::Green))
    } else {
        block
    }
}

/// A request raised by a widget that only `App` can carry out, because it
/// needs to reach a sibling widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Show this file in the file view, reading all of it.
    Load(PathBuf),
    /// Show enough of this file to fill the pane, as the selection passes over
    /// it. Bounded so that holding a cursor key stays responsive.
    Preview(PathBuf),
}

/// The three panes, as one type so `App` can hold them in a single list and
/// dispatch focus by index.
///
/// `clippy::large_enum_variant` fires here because the variants differ sharply
/// in size — `FileView` is 824 bytes (it owns a `TextArea`), `FileNav` 496 and
/// `FilterList` 32 — and boxing the largest is the usual answer. It is the
/// wrong answer here. Exactly three of these ever exist, in `App::widgets`, so
/// the total over-allocation is about 1.1 KB for the life of the process,
/// while the box would add a heap indirection to `render` and `handle_events`
/// on the file view — the pane that redraws every frame and takes nearly every
/// keystroke. Paying a per-frame cost to save a kilobyte once is a bad trade.
///
/// Revisit if `AppWidget` ever lands in a collection that grows with the file
/// or the directory, where the waste would scale with it.
#[allow(clippy::large_enum_variant)]
pub enum AppWidget<'a> {
    FileNav(filenav::FileNav<'a>),
    FileView(fileview::FileView<'a>),
    FilterList(filterlist::FilterList),
}

/// What a keypress in the filter pane asks `App` to do.
///
/// `FilterList` cannot mutate the `ActiveFilters` it only borrows for rendering,
/// so it reports what the user asked for and lets `App` — the set's owner —
/// carry it out. This is not carried on `Action`: that enum is about a
/// widget asking `App` to show a *file* (`Load`/`Preview`), and a filter
/// request is a different kind of thing that would only muddy it.
///
/// The line this enum draws is **"needs the pane's selection"**, not "mutates
/// the set". `i` and `x` are not variants here even though they do end in a
/// mutation, because they address no row — they are `App`'s own keys that
/// happen to be typed while this pane has focus, and `App` handles them
/// directly. `Edit` is a variant despite only opening a prompt, because it
/// asks about *the selected row*, and the row-to-filter mapping lives in this
/// module (see `filter_index_for_row`, which is deliberately the single place
/// that translation happens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCommand {
    Toggle(usize),
    Delete(usize),
    /// Reopen the prompt over this filter's existing pattern, to overwrite it
    /// in place. The one variant `App` answers by opening a prompt rather than
    /// by changing the set — nothing is mutated until that prompt commits.
    Edit(usize),
    /// The search row, which carries no index: the live search lives in its
    /// own slot on the `ActiveFilters`, not at a position in `filters`.
    ToggleSearch,
    DeleteSearch,
    EditSearch,
}

impl AppWidget<'_> {
    /// Mark this widget as the one currently receiving input.
    pub fn set_active(&mut self, active: bool) {
        match self {
            Self::FileNav(w) => w.active = active,
            Self::FileView(w) => w.active = active,
            Self::FilterList(w) => w.active = active,
        }
    }

    /// Feed an event to this widget, returning any action it wants `App` to
    /// perform on its behalf.
    pub fn handle_events(&mut self, event: Event) -> Result<Option<Action>> {
        match self {
            Self::FileNav(w) => w.handle_events(event),
            Self::FileView(w) => {
                w.handle_events(event.into())?;
                Ok(None)
            }
            // Keys aimed at this pane are intercepted earlier, by
            // `App::handle_event`, and routed through `App::handle_filter_key`
            // instead of reaching here: applying them means mutating the
            // `ActiveFilters`, and this widget only ever borrows one, so it
            // cannot carry out its own commands. This arm exists only so the
            // match stays exhaustive; it never actually runs for a key.
            Self::FilterList(_) => Ok(None),
        }
    }
}

impl Widget for &mut AppWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self {
            AppWidget::FileNav(w) => w.render(area, buf),
            AppWidget::FileView(w) => w.render(area, buf),
            // `FilterList::render` needs a borrowed `ActiveFilters`, which this
            // type has no way to hold: `App` owns the one true set, and a
            // copy here could go stale the moment a filter changed. Rather
            // than give `AppWidget` one, `App::render` special-cases this
            // variant and calls `FilterList::render` directly with the set
            // it owns — see `render_widget` in `lib.rs`. This arm exists
            // only so `AppWidget` remains a normal `Widget`; it deliberately
            // draws nothing and is not expected to ever actually run.
            //
            // `render_widget` is the only thing keeping that promise: a
            // future caller reaching for `widget.render(...)` directly would
            // otherwise get a silent blank pane. Loud in every debug test
            // run, without changing release behaviour.
            AppWidget::FilterList(_) => {
                debug_assert!(false, "render the filter pane through render_widget");
            }
        }
    }
}
