use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::{Backend, Buffer, Color, Constraint, Layout, Rect, Style, Terminal, Widget};
use std::time::{Duration, Instant};

/// Widest the nav pane will size itself to automatically.
const MAX_NAV_WIDTH: u16 = 40;

/// Narrowest either pane may be dragged, so a bordered block still renders.
const MIN_PANE_WIDTH: u16 = 3;

/// Two clicks on the divider inside this window restore automatic sizing.
/// Crossterm does not report double-clicks, so they are timed here.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Shown in the prompt when a pattern will not compile, after vim's error.
const INVALID_PATTERN: &str = "E486: invalid pattern";

/// What an open prompt will do with the pattern being typed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    #[default]
    Search,
    Filter,
}

/// A search pattern being typed at the bottom of the screen.
#[derive(Debug, Default)]
struct SearchPrompt {
    pattern: String,
    /// Started with `?` rather than `/`.
    reverse: bool,
    error: Option<String>,
    kind: PromptKind,
}

impl SearchPrompt {
    /// What the bottom line shows: the error if the pattern was rejected,
    /// otherwise the pattern being typed behind its `/` or `?` sigil.
    fn line(&self) -> String {
        match (&self.error, self.kind) {
            (Some(error), _) => error.clone(),
            (None, PromptKind::Filter) => format!("filter: {}", self.pattern),
            (None, PromptKind::Search) => format!(
                "{}{}",
                if self.reverse { '?' } else { '/' },
                self.pattern
            ),
        }
    }
}

/// How the nav pane's width is decided.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum NavWidth {
    /// Snap to the longest entry, capped at `MAX_NAV_WIDTH`.
    #[default]
    Auto,
    /// Held at the width the user dragged to.
    Pinned(u16),
}

pub mod document;
pub mod filter;
mod widgets;
use document::Document;
use filter::FilterSet;
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
    /// Open while a search pattern is being typed.
    search: Option<SearchPrompt>,
    filters: FilterSet,
    document: Document,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum AppState {
    #[default]
    Running, // The app is running
    Quit, // The user has requested the app to quit
}

impl App<'_> {
    pub fn new(config: &Config) -> Self {
        let mut app = Self {
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
            search: None,
            filters: FilterSet::new(),
            document: Document::default(),
        };
        app.sync_document();
        app.restyle();
        app
    }

    /// Feed a key to the open search prompt.
    ///
    /// While it is open it consumes every key, so app-wide commands like `q`
    /// are typed into the pattern rather than acted on.
    fn handle_search_key(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Esc => self.search = None,
            KeyCode::Enter => {
                let Some(prompt) = self.search.as_ref() else {
                    return;
                };
                let (pattern, reverse, kind) =
                    (prompt.pattern.clone(), prompt.reverse, prompt.kind);

                let outcome = match kind {
                    PromptKind::Search => self.run_search(&pattern, reverse),
                    PromptKind::Filter => self.add_filter(&pattern),
                };
                if outcome.is_ok() {
                    self.search = None;
                } else if let Some(prompt) = self.search.as_mut() {
                    prompt.error = Some(INVALID_PATTERN.to_string());
                }
            }
            KeyCode::Backspace => {
                if let Some(prompt) = self.search.as_mut() {
                    prompt.error = None;
                    // Backspacing past the start abandons the search, as in vim.
                    if prompt.pattern.pop().is_none() {
                        self.search = None;
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(prompt) = self.search.as_mut() {
                    prompt.error = None;
                    prompt.pattern.push(c);
                }
            }
            _ => {}
        }
    }

    /// Run a committed pattern against whichever pane has focus.
    fn run_search(&mut self, pattern: &str, reverse: bool) -> Result<(), regex::Error> {
        let action = match &mut self.widgets[self.active_widget] {
            AppWidget::FileNav(nav) => nav.search(pattern, reverse)?,
            AppWidget::FileView(view) => {
                view.search(pattern, reverse)?;
                None
            }
        };

        if let Some(action) = action {
            self.perform(action);
        }
        Ok(())
    }

    /// Add an including filter, colouring it distinctly from its predecessors.
    fn add_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.add(pattern)?;
        self.restyle();
        Ok(())
    }

    /// The nav pane, which owns the entry names the automatic width is based on.
    fn nav(&self) -> Option<&FileNav<'_>> {
        self.widgets.iter().find_map(|widget| match widget {
            AppWidget::FileNav(nav) => Some(nav),
            AppWidget::FileView(_) => None,
        })
    }

    /// Whether the file view is showing a bounded preview rather than the
    /// whole file.
    fn file_view_truncated(&self) -> bool {
        self.widgets.iter().any(|widget| match widget {
            AppWidget::FileView(view) => view.truncated,
            AppWidget::FileNav(_) => false,
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
        // An open prompt takes precedence over every other binding.
        if self.search.is_some() {
            if let event::Event::Key(key) = event {
                self.handle_search_key(key);
            }
            return Ok(());
        }

        if let event::Event::Key(key) = event {
            match key.code {
                // Guarded to an empty modifier set so a modified key — e.g.
                // Ctrl-f, which the file view uses for page-down — falls
                // through to the focused widget instead of being swallowed
                // here.
                KeyCode::Char('q') if key.modifiers.is_empty() => {
                    self.state = AppState::Quit;
                    return Ok(());
                }
                KeyCode::Tab => {
                    self.active_widget = (self.active_widget + 1) % self.widgets.len();
                    return Ok(());
                }
                KeyCode::Char(sigil @ ('/' | '?')) if key.modifiers.is_empty() => {
                    self.search = Some(SearchPrompt {
                        reverse: sigil == '?',
                        ..SearchPrompt::default()
                    });
                    return Ok(());
                }
                KeyCode::Char('f') if key.modifiers.is_empty() => {
                    self.search = Some(SearchPrompt {
                        kind: PromptKind::Filter,
                        ..SearchPrompt::default()
                    });
                    return Ok(());
                }
                KeyCode::Char('!') if key.modifiers.is_empty() => {
                    // Toggle the whole set, so an unfiltered view is one
                    // keystroke away without losing the filters themselves.
                    let enable = !self.filters.any_enabled();
                    self.filters.set_all_enabled(enable);
                    self.restyle();
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

        // The file view upgrades its own truncated preview to a full load on
        // first interaction, which rebuilds the textarea and clears its line
        // styles. That happens inside the widget, so it never reaches
        // `perform` — resync here instead, without re-reading the file.
        let was_truncated = self.file_view_truncated();
        if let Some(action) = self.widgets[self.active_widget].handle_events(event)? {
            self.perform(action);
        } else if was_truncated && !self.file_view_truncated() {
            self.sync_document();
            self.restyle();
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
        self.sync_document();
        self.restyle();
    }

    /// Take the view's current contents as the document to filter.
    ///
    /// The view owns the reading — including its preview truncation and its
    /// error messages — so the document follows it rather than re-reading.
    /// Note the consequence: while a file is only previewed (the view
    /// truncates large files), the document holds just that preview, so
    /// filters see only the truncated slice until the view is focused and
    /// loads the file in full.
    fn sync_document(&mut self) {
        let Some(lines) = self.widgets.iter().find_map(|w| match w {
            AppWidget::FileView(view) => Some(view.textarea.lines().to_vec()),
            AppWidget::FileNav(_) => None,
        }) else {
            return;
        };
        self.document = Document::new(lines);
    }

    /// Re-evaluate the filters and push the resulting styles into the view.
    ///
    /// Loading or previewing rebuilds the textarea, which clears its line
    /// styles, so this must run after any change to the contents as well as
    /// after any change to the filters.
    fn restyle(&mut self) {
        self.document.evaluate(&self.filters);
        let styles = self.document.line_styles(&self.filters);
        for widget in &mut self.widgets {
            if let AppWidget::FileView(view) = widget {
                view.set_line_styles(styles.clone());
            }
        }
    }

    /// A one-line summary of the filter state, empty when no filters exist.
    ///
    /// Dimming alone does not say *why* lines are dim, or that a filter is
    /// defined but currently disabled — the pane would just look ordinary.
    fn status_text(&self) -> String {
        if self.filters.is_empty() {
            return String::new();
        }
        let count = self.filters.len();
        let noun = if count == 1 { "filter" } else { "filters" };
        if !self.filters.any_enabled() {
            return format!("{count} {noun} (disabled)");
        }
        format!(
            "{count} {noun}   {}/{} lines match",
            self.document.match_count(),
            self.document.lines().len()
        )
    }
}

impl Widget for &mut App<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use Constraint::{Length, Min};
        // calculate rects where widgets should be rendered
        assert!(self.widgets.len() == 2);

        // The search prompt borrows the bottom row while it is open; failing
        // that, the status line borrows it whenever there is something to show.
        let status = self.status_text();
        let (area, prompt_area) = if self.search.is_some() || !status.is_empty() {
            let [panes, prompt] = Layout::vertical([Min(0), Length(1)]).areas(area);
            (panes, Some(prompt))
        } else {
            (area, None)
        };

        let nav_width = self.nav_width(area);
        let widget_areas: [Rect; 2] = Layout::horizontal([Length(nav_width), Min(0)]).areas(area);

        // Remember the boundary so mouse events landing before the next frame
        // can be tested against it.
        self.divider = area.x + nav_width;

        for (i, w) in self.widgets.iter_mut().enumerate() {
            w.set_active(i == self.active_widget);
            w.render(widget_areas[i], buf);
        }

        if let Some(prompt_area) = prompt_area {
            let (text, style) = match self.search.as_ref() {
                Some(prompt) if prompt.error.is_some() => {
                    (prompt.line(), Style::default().fg(Color::Red))
                }
                Some(prompt) => (prompt.line(), Style::default()),
                None => (status, Style::default().fg(Color::DarkGray)),
            };
            buf.set_stringn(
                prompt_area.x,
                prompt_area.y,
                text,
                prompt_area.width as usize,
                style,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers, MouseEvent};
    use ratatui::prelude::Buffer;
    use ratatui::style::Modifier; // the tests assert on Modifier::DIM
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

    fn key(app: &mut App, code: KeyCode) {
        app.handle_event(event::Event::Key(code.into())).unwrap();
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            key(app, KeyCode::Char(c));
        }
    }

    fn prompt_line(app: &mut App) -> String {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        let y = AREA.height - 1;
        (0..AREA.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn slash_opens_a_search_prompt() {
        let mut app = app_over("prompt", &["a.rs"]);

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "foo");

        assert_eq!(prompt_line(&mut app), "/foo");
    }

    #[test]
    fn question_mark_opens_a_backward_search() {
        let mut app = app_over("prompt_back", &["a.rs"]);

        key(&mut app, KeyCode::Char('?'));
        typed(&mut app, "foo");

        assert_eq!(prompt_line(&mut app), "?foo");
    }

    /// The prompt swallows keys that are otherwise app-wide commands.
    #[test]
    fn q_while_searching_is_typed_not_quit() {
        let mut app = app_over("prompt_q", &["a.rs"]);

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "q");

        assert!(app.is_running(), "q closed the app while typing a pattern");
        assert_eq!(prompt_line(&mut app), "/q");
    }

    #[test]
    fn backspace_deletes_then_cancels() {
        let mut app = app_over("prompt_bs", &["a.rs"]);
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "ab");

        key(&mut app, KeyCode::Backspace);
        assert_eq!(prompt_line(&mut app), "/a");

        key(&mut app, KeyCode::Backspace);
        key(&mut app, KeyCode::Backspace);

        assert!(app.search.is_none(), "backspace on empty did not cancel");
    }

    #[test]
    fn esc_cancels_the_prompt() {
        let mut app = app_over("prompt_esc", &["a.rs"]);
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "foo");

        key(&mut app, KeyCode::Esc);

        assert!(app.search.is_none());
    }

    #[test]
    fn committing_a_search_closes_the_prompt_and_moves_the_selection() {
        let mut app = app_over("prompt_commit", &["alpha.rs", "gamma.rs"]);
        draw(&mut app);

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "gamma");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_none(), "prompt stayed open");
        let nav = app.nav().expect("nav pane");
        assert_eq!(nav.entries[nav.state.selected().unwrap()], "gamma.rs");
    }

    #[test]
    fn an_invalid_pattern_keeps_the_prompt_open_with_an_error() {
        let mut app = app_over("prompt_bad", &["a.rs"]);

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "[");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_some(), "prompt closed on an invalid pattern");
        assert!(
            prompt_line(&mut app).contains("E486"),
            "no error shown: {}",
            prompt_line(&mut app)
        );
    }

    /// Searching in the nav pane jumps to the file *and* previews it.
    #[test]
    fn a_nav_search_previews_the_matched_file() {
        let mut app = app_over("prompt_preview", &["alpha.rs", "gamma.rs"]);
        fs::write("target/test-appdirs/prompt_preview/gamma.rs", "GAMMA MARKER\n").unwrap();
        draw(&mut app);

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "gamma");
        key(&mut app, KeyCode::Enter);

        let mut buf = Buffer::empty(AREA);
        (&mut app).render(AREA, &mut buf);
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("GAMMA MARKER"), "matched file was not previewed");
    }

    /// The prompt only takes a row while it is open.
    #[test]
    fn the_prompt_row_appears_only_while_searching() {
        let mut app = app_over("prompt_layout", &["a.rs"]);
        let idle = prompt_line(&mut app);

        key(&mut app, KeyCode::Char('/'));

        assert_ne!(idle, prompt_line(&mut app));
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

    #[test]
    fn f_opens_a_filter_prompt() {
        let mut app = app_over("filter_prompt", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");

        assert_eq!(prompt_line(&mut app), "filter: foo");
    }

    #[test]
    fn committing_a_filter_adds_it() {
        let mut app = app_over("filter_add", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_none(), "prompt stayed open");
        assert_eq!(app.filters.len(), 1);
    }

    #[test]
    fn an_invalid_filter_pattern_keeps_the_prompt_open() {
        let mut app = app_over("filter_bad", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "[");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_some(), "prompt closed on an invalid pattern");
        assert!(prompt_line(&mut app).contains("E486"));
        assert_eq!(app.filters.len(), 0, "a rejected pattern must not be added");
    }

    #[test]
    fn esc_cancels_a_filter_prompt_without_adding() {
        let mut app = app_over("filter_esc", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");
        key(&mut app, KeyCode::Esc);

        assert!(app.search.is_none());
        assert_eq!(app.filters.len(), 0);
    }

    /// The prompt swallows keys, so `q` types rather than quits — as for search.
    #[test]
    fn q_while_filtering_is_typed_not_quit() {
        let mut app = app_over("filter_q", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "q");

        assert!(app.is_running());
        assert_eq!(prompt_line(&mut app), "filter: q");
    }

    #[test]
    fn successive_filters_take_different_colours() {
        let mut app = app_over("filter_colours", &["a.rs"]);

        for pattern in ["foo", "bar"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }

        let styles: Vec<_> = app.filters.filters().iter().map(|f| f.style.fg).collect();
        assert_ne!(styles[0], styles[1]);
    }

    /// Returns the styles the file view is currently rendering with.
    fn view_line_styles(app: &App) -> Vec<Option<Style>> {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.textarea.line_styles().to_vec()),
                AppWidget::FileNav(_) => None,
            })
            .expect("no file view")
    }

    fn app_over_file(name: &str, body: &str) -> App<'static> {
        let dir = std::path::Path::new("target/test-appdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        let file = dir.join("log.txt");
        fs::write(&file, body).expect("write fixture");
        App::new(&Config {
            file: file.display().to_string(),
        })
    }

    #[test]
    fn committing_a_filter_styles_the_view() {
        let mut app = app_over_file("restyle", "alpha\nbeta\ngamma\n");

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let styles = view_line_styles(&app);
        assert_eq!(styles.len(), 3, "a style slot per line");
        assert!(styles[1].is_some(), "matching line unstyled");
        assert!(
            styles[0]
                .expect("unmatched line unstyled")
                .add_modifier
                .contains(Modifier::DIM),
            "unmatched line not dimmed"
        );
    }

    #[test]
    fn an_unfiltered_view_has_no_styles() {
        let app = app_over_file("restyle_none", "alpha\nbeta\n");

        assert!(view_line_styles(&app).iter().all(Option::is_none));
    }

    /// Filters describe a log format, so they outlive the file they were
    /// defined against — and must be re-applied after a load clears them.
    #[test]
    fn filters_survive_loading_another_file() {
        let mut app = app_over_file("restyle_reload", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let dir = std::path::Path::new("target/test-appdirs/restyle_reload");
        fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
        app.perform(Action::Load(dir.join("other.txt")));

        let styles = view_line_styles(&app);
        assert_eq!(styles.len(), 2, "styles not re-applied to the new file");
        assert!(styles[0].is_some(), "match in the new file unstyled");
    }

    #[test]
    fn bang_disables_every_filter_and_restores_them() {
        let mut app = app_over_file("restyle_bang", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));
        assert!(
            view_line_styles(&app).iter().all(Option::is_none),
            "! did not clear the styling"
        );

        key(&mut app, KeyCode::Char('!'));
        assert!(
            view_line_styles(&app)[1].is_some(),
            "! did not restore the filters"
        );
    }

    /// The bottom row when no prompt is open.
    fn status_line(app: &mut App) -> String {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        let y = AREA.height - 1;
        (0..AREA.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// With no filters there is nothing to report, so the bottom row is not
    /// surrendered at all: the panes still reach it and draw their own border
    /// there. Costing a row to say nothing is what this checks against.
    #[test]
    fn no_row_is_surrendered_without_filters() {
        let mut app = app_over_file("status_none", "alpha\n");

        let bottom = status_line(&mut app);

        assert!(
            !bottom.contains("filters"),
            "reported filter state when no filters exist: {bottom}"
        );
        assert!(
            bottom.contains('└'),
            "the panes did not reach the bottom row, so a row was spent on nothing: {bottom}"
        );
    }

    #[test]
    fn the_status_line_reports_filters_and_matches() {
        let mut app = app_over_file("status_some", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let status = status_line(&mut app);

        assert!(
            status.contains("1 filter") && !status.contains("1 filters"),
            "count is not singular-aware: {status}"
        );
        assert!(status.contains("1/3"), "match count missing: {status}");
    }

    #[test]
    fn the_status_line_says_when_filters_are_disabled() {
        let mut app = app_over_file("status_off", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));

        assert!(
            status_line(&mut app).contains("disabled"),
            "no indication the filters are off: {}",
            status_line(&mut app)
        );
    }

    /// An open prompt takes the row, as it already does.
    #[test]
    fn a_prompt_still_takes_the_bottom_row() {
        let mut app = app_over_file("status_prompt", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "foo");

        assert_eq!(status_line(&mut app), "filter: foo");
    }

    /// Arrowing onto a large log only previews it (bounded by
    /// `PREVIEW_LINES` = 500 in `fileview.rs`), so a filter added at that
    /// point is evaluated against the truncated slice. `FileView` upgrades
    /// itself to a full load the moment it is actually used — inside its own
    /// `handle_events`, invisible to `perform` — which rebuilds the textarea
    /// and clears its line styles. Without a resync, the view is left
    /// unfiltered and the style vector stuck at the stale preview length.
    #[test]
    fn upgrading_a_truncated_preview_resyncs_styles_without_reloading() {
        let dir = std::path::Path::new("target/test-appdirs").join("preview_upgrade_resync");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        // Comfortably more than PREVIEW_LINES (500), so the first preview is
        // truncated. The match sits inside the first 500 lines too, so it is
        // visible both before and after the upgrade to a full load.
        let body: String = (0..600)
            .map(|i| {
                if i == 10 {
                    "MATCH line\n".to_string()
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        fs::write(dir.join("big.log"), &body).expect("write fixture");

        let mut app = App::new(&Config {
            file: dir.join("placeholder").display().to_string(),
        });

        // Arrow onto the log from the nav pane: this previews it rather than
        // reading the whole 600-line file.
        key(&mut app, KeyCode::Down);

        // Add a filter while the view still only holds the preview.
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "MATCH");
        key(&mut app, KeyCode::Enter);

        let preview_styles = view_line_styles(&app);
        assert_eq!(preview_styles.len(), 500, "sanity: preview is capped");
        assert!(preview_styles[10].is_some(), "match line unstyled in the preview");

        // Tab into the file view and press a key: this is exactly what
        // upgrades the truncated preview to a full load inside
        // `FileView::handle_events`.
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Char('j'));

        let styles = view_line_styles(&app);
        assert_eq!(
            styles.len(),
            600,
            "style vector was not resynced to the fully loaded buffer"
        );
        assert!(
            styles[10].is_some(),
            "matching line lost its style after the preview upgraded to a full load"
        );
    }

    /// `Ctrl-f` must reach the file view's own page-down binding, not the
    /// global `f` handler that opens a filter prompt.
    #[test]
    fn ctrl_f_scrolls_the_file_view_instead_of_opening_a_filter_prompt() {
        let body: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("ctrl_f_scroll", &body);
        draw(&mut app); // establish the file view's rendered size
        key(&mut app, KeyCode::Tab); // focus the file view

        let before = view_cursor_row(&app);
        app.handle_event(event::Event::Key(KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();

        assert!(app.search.is_none(), "Ctrl-f opened a filter prompt");
        assert!(
            view_cursor_row(&app) > before,
            "Ctrl-f did not scroll the file view"
        );
    }

    fn view_cursor_row(app: &App) -> usize {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.textarea.cursor().0),
                AppWidget::FileNav(_) => None,
            })
            .expect("no file view")
    }
}
