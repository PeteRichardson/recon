use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::{Backend, Buffer, Constraint, Layout, Rect, Terminal, Widget};
use std::time::{Duration, Instant};

/// Widest the nav pane will size itself to automatically.
const MAX_NAV_WIDTH: u16 = 40;

/// Narrowest either pane may be dragged, so a bordered block still renders.
const MIN_PANE_WIDTH: u16 = 3;

/// Two clicks on the divider inside this window restore automatic sizing.
/// Crossterm does not report double-clicks, so they are timed here.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How the nav pane's width is decided.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum NavWidth {
    /// Snap to the longest entry, capped at `MAX_NAV_WIDTH`.
    #[default]
    Auto,
    /// Held at the width the user dragged to.
    Pinned(u16),
}

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
    nav_width: NavWidth,
    /// Boundary column from the last render, for hit-testing mouse events that
    /// arrive before the next frame.
    divider: u16,
    dragging: bool,
    last_divider_click: Option<Instant>,
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
            nav_width: NavWidth::Auto,
            divider: 0,
            dragging: false,
            last_divider_click: None,
        }
    }

    /// The nav pane, which owns the entry names the automatic width is based on.
    fn nav(&self) -> Option<&FileNav<'_>> {
        self.widgets.iter().find_map(|widget| match widget {
            AppWidget::FileNav(nav) => Some(nav),
            AppWidget::FileView(_) => None,
        })
    }

    /// Resolve the nav pane's width within `area`.
    fn nav_width(&self, area: Rect) -> u16 {
        let width = match self.nav_width {
            NavWidth::Auto => self
                .nav()
                .map_or(MAX_NAV_WIDTH, FileNav::preferred_width)
                .min(MAX_NAV_WIDTH),
            NavWidth::Pinned(width) => width,
        };

        // Whatever the source, neither pane may lose its borders.
        let widest = area
            .width
            .saturating_sub(MIN_PANE_WIDTH)
            .max(MIN_PANE_WIDTH);
        width.clamp(MIN_PANE_WIDTH, widest)
    }

    /// Handle divider dragging, reporting whether the event was consumed.
    ///
    /// Anything not aimed at the divider falls through to the focused widget,
    /// so the file view keeps its scroll-wheel behaviour.
    fn handle_divider(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if self.on_divider(mouse.column) => {
                let now = Instant::now();
                let double_click = self
                    .last_divider_click
                    .is_some_and(|last| now.duration_since(last) <= DOUBLE_CLICK);

                if double_click {
                    self.nav_width = NavWidth::Auto;
                    self.last_divider_click = None;
                } else {
                    self.dragging = true;
                    self.last_divider_click = Some(now);
                }
                true
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                self.nav_width = NavWidth::Pinned(mouse.column);
                // A real drag rules out the next click being a double-click,
                // which would otherwise discard the width just set.
                self.last_divider_click = None;
                true
            }
            MouseEventKind::Up(MouseButton::Left) if self.dragging => {
                self.dragging = false;
                true
            }
            _ => false,
        }
    }

    /// The divider is the pair of adjacent borders between the panes, with a
    /// column of slack either side so it is not fiddly to grab.
    fn on_divider(&self, column: u16) -> bool {
        column.abs_diff(self.divider) <= 1
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

        if let event::Event::Mouse(mouse) = event {
            if self.handle_divider(mouse) {
                return Ok(());
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
        use Constraint::{Length, Min};
        // calculate rects where widgets should be rendered
        assert!(self.widgets.len() == 2);
        let nav_width = self.nav_width(area);
        let widget_areas: [Rect; 2] = Layout::horizontal([Length(nav_width), Min(0)]).areas(area);

        // Remember the boundary so mouse events landing before the next frame
        // can be tested against it.
        self.divider = area.x + nav_width;

        for (i, w) in self.widgets.iter_mut().enumerate() {
            w.set_active(i == self.active_widget);
            w.render(widget_areas[i], buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseEvent};
    use ratatui::prelude::Buffer;
    use std::fs;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 10,
    };

    /// An app listing a directory with known entry names.
    fn app_over(name: &str, files: &[&str]) -> App<'static> {
        let dir = std::path::Path::new("target/test-appdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        for file in files {
            fs::write(dir.join(file), "x").expect("write fixture");
        }
        App::new(&Config {
            file: dir.join("placeholder").display().to_string(),
        })
    }

    fn draw(app: &mut App) {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
    }

    fn mouse(app: &mut App, kind: MouseEventKind, column: u16) {
        app.handle_event(event::Event::Mouse(MouseEvent {
            kind,
            column,
            row: 3,
            modifiers: KeyModifiers::empty(),
        }))
        .unwrap();
    }

    fn drag_to(app: &mut App, from: u16, to: u16) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), from);
        mouse(app, MouseEventKind::Drag(MouseButton::Left), to);
        mouse(app, MouseEventKind::Up(MouseButton::Left), to);
    }

    #[test]
    fn auto_width_snaps_to_the_longest_entry() {
        let mut app = app_over("snap", &["a.rs", "twelve_chars.rs"]);
        draw(&mut app);

        assert_eq!(app.nav_width(AREA), "twelve_chars.rs".len() as u16 + 4);
    }

    #[test]
    fn auto_width_is_capped_at_the_default() {
        let long = "a".repeat(200);
        let mut app = app_over("capped", &[long.as_str()]);
        draw(&mut app);

        assert_eq!(app.nav_width(AREA), MAX_NAV_WIDTH);
    }

    /// No floor: a directory of short names gets a narrow pane.
    #[test]
    fn auto_width_has_no_floor() {
        let mut app = app_over("tiny", &["a"]);
        draw(&mut app);

        assert!(
            app.nav_width(AREA) < 10,
            "expected a narrow pane, got {}",
            app.nav_width(AREA)
        );
    }

    #[test]
    fn dragging_the_divider_pins_the_width() {
        let mut app = app_over("drag", &["a.rs"]);
        draw(&mut app);
        let divider = app.divider;

        drag_to(&mut app, divider, 60);

        assert_eq!(app.nav_width, NavWidth::Pinned(60));
        assert_eq!(app.nav_width(AREA), 60);
    }

    #[test]
    fn a_pinned_width_survives_navigating_to_another_directory() {
        let mut app = app_over("pinned", &["a.rs"]);
        fs::create_dir_all("target/test-appdirs/pinned/subdir").expect("subdir");
        draw(&mut app);
        let divider = app.divider;
        drag_to(&mut app, divider, 55);

        // Walk onto the subdirectory and descend into it.
        for _ in 0..8 {
            app.handle_event(event::Event::Key(KeyCode::Down.into()))
                .unwrap();
        }
        app.handle_event(event::Event::Key(KeyCode::Enter.into()))
            .unwrap();
        draw(&mut app);

        assert_eq!(app.nav_width(AREA), 55, "navigation overrode the drag");
    }

    #[test]
    fn double_clicking_the_divider_restores_automatic_sizing() {
        let mut app = app_over("dblclick", &["a.rs"]);
        draw(&mut app);
        let divider = app.divider;
        drag_to(&mut app, divider, 70);
        assert_eq!(app.nav_width(AREA), 70);
        draw(&mut app);

        let divider = app.divider;
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);

        assert_eq!(app.nav_width, NavWidth::Auto);
        assert_eq!(app.nav_width(AREA), "a.rs".len() as u16 + 4);
    }

    #[test]
    fn dragging_cannot_collapse_either_pane() {
        let mut app = app_over("clamp", &["a.rs"]);
        draw(&mut app);

        let divider = app.divider;
        drag_to(&mut app, divider, 0);
        assert!(app.nav_width(AREA) >= MIN_PANE_WIDTH, "nav pane collapsed");

        draw(&mut app);
        let divider = app.divider;
        drag_to(&mut app, divider, AREA.width);
        assert!(
            AREA.width - app.nav_width(AREA) >= MIN_PANE_WIDTH,
            "file view collapsed"
        );
    }

    /// A click nowhere near the divider is still the focused widget's business.
    #[test]
    fn a_click_away_from_the_divider_is_not_a_drag() {
        let mut app = app_over("passthrough", &["a.rs"]);
        draw(&mut app);
        let before = app.nav_width(AREA);

        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 90);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 60);

        assert!(!app.dragging);
        assert_eq!(app.nav_width(AREA), before);
    }
}
