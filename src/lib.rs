use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, KeyCode};
use ratatui::prelude::{Backend, Buffer, Constraint, Layout, Rect, Terminal, Widget};
use std::time::Duration;

mod widgets;
use widgets::filenav::FileNav;
use widgets::fileview::FileView;
use widgets::{Action, AppWidget};

#[derive(Parser, Debug)]
#[command(version, about, long_about=None)]
pub struct Config {
    pub file: String,
}

#[derive(Default)]
pub struct App<'a> {
    state: AppState,
    widgets: Vec<AppWidget<'a>>,
    active_widget: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum AppState {
    #[default]
    Running, // The app is running
    Quit, // The user has requested the app to quit
}

impl App<'_> {
    pub fn new(config: &Config) -> Self {
        Self {
            state: AppState::Running,
            widgets: vec![
                AppWidget::FileNav(FileNav::new(config.file.clone())),
                AppWidget::FileView(FileView::new(config.file.clone())),
            ],
            active_widget: 0,
        }
    }

    /// This is the main event loop for the app.
    pub fn run<B>(mut self, mut terminal: Terminal<B>) -> Result<()>
    where
        B: Backend,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        while self.is_running() {
            terminal.draw(|frame| {
                let area = frame.area();
                frame.render_widget(&mut self, area);
            })?;
            self.handle_events()?;
        }
        Ok(())
    }

    const fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running)
    }

    /// Handle any events that have occurred since the last time the app was rendered.
    fn handle_events(&mut self) -> Result<()> {
        let timeout = Duration::from_secs_f32(1.0 / 60.0);
        if event::poll(timeout)? {
            let event = event::read()?;
            self.handle_event(event)?;
        }
        Ok(())
    }

    /// Dispatch a single event: app-wide keys first, then the focused widget.
    ///
    /// Split out from the polling loop so that it can be driven directly.
    pub fn handle_event(&mut self, event: event::Event) -> Result<()> {
        if let event::Event::Key(key) = event {
            match key.code {
                KeyCode::Char('q') => {
                    self.state = AppState::Quit;
                    return Ok(());
                }
                KeyCode::Tab => {
                    self.active_widget = (self.active_widget + 1) % self.widgets.len();
                    return Ok(());
                }
                _ => {}
            }
        }

        if let Some(action) = self.widgets[self.active_widget].handle_events(event)? {
            self.perform(action);
        }
        Ok(())
    }

    /// Carry out an action on behalf of the widget that raised it.
    fn perform(&mut self, action: Action) {
        for widget in &mut self.widgets {
            if let AppWidget::FileView(view) = widget {
                match &action {
                    Action::Load(path) => view.load(path),
                    Action::Preview(path) => view.preview(path),
                }
            }
        }
    }
}

impl Widget for &mut App<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use Constraint::Min;
        // calculate rects where widgets should be rendered
        assert!(self.widgets.len() == 2);
        let widget_areas: [Rect; 2] = Layout::horizontal([Min(20), Min(0)]).areas(area);
        for (i, w) in self.widgets.iter_mut().enumerate() {
            w.set_active(i == self.active_widget);
            w.render(widget_areas[i], buf);
        }
    }
}
