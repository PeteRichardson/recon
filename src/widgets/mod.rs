pub mod filenav;
pub mod fileview;
pub mod filterlist;

use color_eyre::Result;
use crossterm::event::Event;
use ratatui::prelude::{Buffer, Rect, Widget};
use std::path::PathBuf;

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

pub enum AppWidget<'a> {
    FileNav(filenav::FileNav<'a>),
    FileView(fileview::FileView<'a>),
    FilterList(filterlist::FilterList),
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
            // Toggling and deleting filters from this pane is Task 6's job;
            // for now it simply does not react to input.
            Self::FilterList(_) => Ok(None),
        }
    }
}

impl Widget for &mut AppWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self {
            AppWidget::FileNav(w) => w.render(area, buf),
            AppWidget::FileView(w) => w.render(area, buf),
            // `FilterList::render` needs a borrowed `FilterSet`, which this
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
