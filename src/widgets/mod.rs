pub mod filenav;
pub mod fileview;

use color_eyre::Result;
use crossterm::event::Event;
use ratatui::prelude::{Buffer, Rect, Widget};

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

    pub fn handle_events(&mut self, event: Event) -> Result<()> {
        match self {
            Self::FileNav(w) => w.handle_events(event),
            Self::FileView(w) => w.handle_events(event.into()),
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
