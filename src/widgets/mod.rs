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
}

impl AppWidget<'_> {
    /// Mark this widget as the one currently receiving input.
    pub fn set_active(&mut self, active: bool) {
        match self {
            Self::FileNav(w) => w.active = active,
            Self::FileView(w) => w.active = active,
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
        }
    }
}

impl Widget for &mut AppWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self {
            AppWidget::FileNav(w) => w.render(area, buf),
            AppWidget::FileView(w) => w.render(area, buf),
        }
    }
}
