use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::{Backend, Buffer, Color, Constraint, Layout, Rect, Style, Terminal, Widget};
use std::time::{Duration, Instant};

/// Widest the nav pane will size itself to automatically.
const MAX_NAV_WIDTH: u16 = 40;

/// Narrowest either pane may be dragged, so a bordered block still renders.
///
/// This is nav's own floor — how little it may have — and is independent of
/// `MIN_FILE_VIEW_WIDTH` below, which bounds how much it may *take*. A drag
/// to the far edge, or a directory of short filenames, may still leave nav
/// narrower than `MIN_FILE_VIEW_WIDTH` would ask for; that is fine, since
/// nothing at that end is starving the file view.
const MIN_PANE_WIDTH: u16 = 3;

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
const MIN_NAV_HEIGHT: u16 = 3;

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
const MIN_FILE_VIEW_WIDTH: u16 = 2 + (6 + 2) + 20;

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
    Exclude,
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
            (None, PromptKind::Exclude) => format!("exclude: {}", self.pattern),
            (None, PromptKind::Search) => {
                format!("{}{}", if self.reverse { '?' } else { '/' }, self.pattern)
            }
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
use document::{Document, Mode};
use filter::FilterSet;
use widgets::filenav::FileNav;
use widgets::fileview::FileView;
use widgets::filterlist::FilterList;
use widgets::{Action, AppWidget, FilterCommand};

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
    /// Source line indices the file view's buffer was last rebuilt from.
    /// `refresh_view` rebuilds only when the new visible set differs from
    /// this, so a filter change that leaves the same rows on screen does not
    /// reset the viewport's scroll position.
    last_visible: Vec<usize>,
    /// The single widget filling the screen, or `None` for the normal split.
    ///
    /// Hiding the left column and maximising the file view are the same thing,
    /// so they share this one field. Two separate flags could disagree.
    zoom: Option<usize>,
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
                AppWidget::FilterList(FilterList::default()),
            ],
            active_widget: 0,
            nav_width: NavWidth::Auto,
            divider: 0,
            dragging: false,
            last_divider_click: None,
            search: None,
            filters: FilterSet::new(),
            document: Document::default(),
            last_visible: Vec::new(),
            zoom: None,
        };
        app.sync_document();
        app.refresh_view();
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
                    PromptKind::Exclude => self.add_excluding_filter(&pattern),
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
            // Unreachable in practice: `/` and `?` are no longer opened at
            // all while the filter pane has focus (see their guard in
            // `handle_event`), and the prompt swallows every key including
            // `Tab` while it is open, so focus cannot move to this pane
            // before `Enter` gets here. Kept so this match stays exhaustive
            // against a fourth `AppWidget` variant, and as a harmless
            // fallback if that guard is ever loosened.
            AppWidget::FilterList(_) => None,
        };

        if let Some(action) = action {
            self.perform(action);
        }
        Ok(())
    }

    /// Add an including filter, colouring it distinctly from its predecessors.
    fn add_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.add(pattern)?;
        self.refresh_view();
        Ok(())
    }

    /// Add an excluding filter: its matches leave the view entirely.
    fn add_excluding_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.add_excluding(pattern)?;
        self.refresh_view();
        Ok(())
    }

    /// Flip between dimming unmatched lines and hiding them.
    ///
    /// Unlike a filter change, this always rebuilds the buffer: which rows
    /// are visible necessarily changes (that is the point of the toggle), so
    /// there is no "nothing changed" case to guard `refresh_view` against
    /// here the way `add_filter` needs. `apply_view` still holds the
    /// cursor's screen row across that rebuild, the same as any other
    /// caller, so the toggle does not re-anchor the view even though it
    /// always rebuilds. It calls `recompute_visible` rather than
    /// `refresh_view`'s full `evaluate`, though: the mode is the only thing
    /// that changed, and no verdict can be different, so redoing the whole
    /// filter pass would be pure waste on a large document.
    fn toggle_hiding(&mut self) {
        let mode = match self.document.mode() {
            Mode::Dimmed => Mode::FilteredOnly,
            Mode::FilteredOnly => Mode::Dimmed,
        };
        let cursor_source = self.cursor_source();
        self.document.set_mode(mode);
        self.document.recompute_visible();
        self.apply_view(cursor_source);
    }

    /// The nav pane, which owns the entry names the automatic width is based on.
    fn nav(&self) -> Option<&FileNav<'_>> {
        self.widgets.iter().find_map(|widget| match widget {
            AppWidget::FileNav(nav) => Some(nav),
            AppWidget::FileView(_) | AppWidget::FilterList(_) => None,
        })
    }

    /// The filter pane, which owns its own selection and rendering state
    /// independent of the `FilterSet` model `App` holds.
    fn filter_list(&self) -> Option<&FilterList> {
        self.widgets.iter().find_map(|widget| match widget {
            AppWidget::FilterList(list) => Some(list),
            AppWidget::FileNav(_) | AppWidget::FileView(_) => None,
        })
    }

    /// Rows the filter pane wants, which is none while no filter exists — so
    /// it costs nothing to a user who never defines one.
    fn filter_pane_height(&self) -> u16 {
        self.filter_list()
            .map(|list| list.preferred_height(self.filters.len()))
            .unwrap_or(0)
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
    fn filter_pane_split_height(&self, left_height: u16) -> u16 {
        self.filter_pane_height()
            .min(left_height / 2)
            .min(left_height.saturating_sub(MIN_NAV_HEIGHT))
    }

    /// Whether the file view is showing a bounded preview rather than the
    /// whole file.
    fn file_view_truncated(&self) -> bool {
        self.widgets.iter().any(|widget| match widget {
            AppWidget::FileView(view) => view.truncated,
            AppWidget::FileNav(_) | AppWidget::FilterList(_) => false,
        })
    }

    /// Resolve the nav pane's width within `area`.
    fn nav_width(&self, area: Rect) -> u16 {
        let width = match self.nav_width {
            // The column has to fit whichever pane currently wants more:
            // the navigator's longest entry, or the filter pane's longest
            // row. Either alone could otherwise get silently clipped by the
            // other's narrower automatic width.
            NavWidth::Auto => {
                let nav_width = self.nav().map_or(MAX_NAV_WIDTH, FileNav::preferred_width);
                let filter_width = self
                    .filter_list()
                    .map_or(0, |list| list.preferred_width(&self.filters));
                nav_width.max(filter_width).min(MAX_NAV_WIDTH)
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
                    self.focus_next();
                    return Ok(());
                }
                // The filter pane has nothing to search over, so the prompt
                // is not opened at all while it has focus — opening it and
                // then having `Enter` silently do nothing (`run_search`'s
                // `FilterList` arm) looked like the keystroke was simply
                // swallowed, with no feedback that anything was wrong.
                KeyCode::Char(sigil @ ('/' | '?'))
                    if key.modifiers.is_empty()
                        && !matches!(
                            self.widgets[self.active_widget],
                            AppWidget::FilterList(_)
                        ) =>
                {
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
                KeyCode::Char('F')
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.search = Some(SearchPrompt {
                        kind: PromptKind::Exclude,
                        ..SearchPrompt::default()
                    });
                    return Ok(());
                }
                KeyCode::Char('!') if key.modifiers.is_empty() => {
                    // Three states, because "nothing is enabled" and "nothing
                    // was captured" are different situations. Branching on the
                    // capture alone makes ! inert once every filter has been
                    // disabled by hand: it would capture all-disabled and then
                    // faithfully restore it, forever.
                    if self.filters.any_enabled() {
                        self.filters.disable_all_remembering();
                    } else if self.filters.has_remembered() {
                        self.filters.restore_remembered();
                    } else {
                        self.filters.set_all_enabled(true);
                    }
                    self.refresh_view();
                    return Ok(());
                }
                KeyCode::Char('H')
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.toggle_hiding();
                    return Ok(());
                }
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.toggle_hiding();
                    return Ok(());
                }
                KeyCode::Char('b') if key.modifiers.is_empty() => {
                    self.zoom_file_view();
                    return Ok(());
                }
                KeyCode::Char('e') if key.modifiers.is_empty() => {
                    self.reveal_and_focus_nav();
                    return Ok(());
                }
                KeyCode::Char('z') if key.modifiers.is_empty() => {
                    self.zoom_focused();
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

        // Filter pane keys are routed here rather than through the generic
        // `handle_events` dispatch below: applying them means mutating the
        // `FilterSet`, which only `App` owns, so `FilterList` cannot carry
        // them out itself — see `handle_filter_key`.
        if let event::Event::Key(key) = event {
            if matches!(self.widgets[self.active_widget], AppWidget::FilterList(_)) {
                self.handle_filter_key(key);
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
            self.refresh_view();
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
        self.refresh_view();
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
            AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
        }) else {
            return;
        };
        self.document = Document::new(lines);
        // The buffer the view is showing belongs to the *previous* document,
        // so the record of what it was built from is meaningless now.
        // Clearing it forces the next `apply_view` to rebuild: two different
        // documents can easily produce an equal visible list — reloading the
        // same file with a filter active produces an identical one every
        // time, which would otherwise leave the just-loaded, unfiltered
        // buffer in place under numbers and styles sized for the filtered
        // subset.
        self.last_visible.clear();
    }

    /// Re-evaluate the filters and rebuild what the view shows.
    fn refresh_view(&mut self) {
        // Whatever changed the set may have changed how many filters there
        // are, so the pane's selection has to be pulled back into range (or
        // established at 0 on a set that just became non-empty) before
        // anything else runs. Every mutation path — `add_filter`,
        // `add_excluding_filter`, and the pane's own toggle/delete — funnels
        // through this method, so putting the call here rather than at each
        // call site means a future fourth path cannot forget it.
        let len = self.filters.len();
        for widget in &mut self.widgets {
            if let AppWidget::FilterList(list) = widget {
                list.clamp_selection(len);
            }
        }

        // The cursor is a source line index for the duration of the rebuild:
        // its row in the view is only meaningful against the old visible list.
        let cursor_source = self.cursor_source();
        self.document.evaluate(&self.filters);
        self.apply_view(cursor_source);
    }

    /// Handle a key aimed at the filter pane.
    ///
    /// This borrows the pane and the `FilterSet` together — something
    /// neither `FilterList` nor `Action` can do on their own, since the pane
    /// only ever borrows the set to render it — applies whatever command the
    /// pane reports, moves focus off the pane if the set is now empty (which
    /// also carries the zoom along, since `focus_next` already keeps that
    /// invariant), and re-evaluates. A delete renumbers the remaining
    /// filters, so `refresh_view`'s full `Document::evaluate` is required
    /// here: every cached `Verdict::Included` is a positional index that a
    /// patch would leave stale.
    ///
    /// Takes the whole `KeyEvent`, not just its `KeyCode`: `FilterList::handle_key`
    /// needs the modifiers to guard `space`/`d`/`j`/`k` against CONTROL and
    /// ALT, the same way every other global binding is guarded — see its
    /// doc comment.
    fn handle_filter_key(&mut self, key: event::KeyEvent) {
        let len = self.filters.len();
        let Some(AppWidget::FilterList(list)) = self.widgets.get_mut(self.active_widget) else {
            return;
        };
        let Some(command) = list.handle_key(key, len) else {
            return;
        };
        match command {
            FilterCommand::Toggle(index) => {
                self.filters.toggle_enabled(index);
            }
            FilterCommand::Delete(index) => {
                self.filters.remove(index);
            }
        }
        if self.filters.is_empty() {
            self.focus_next();
        }
        self.refresh_view();
    }

    /// Which row of the file view pane the cursor is currently drawn on.
    fn file_view_screen_row(&self) -> u16 {
        self.widgets
            .iter()
            .find_map(|widget| match widget {
                AppWidget::FileView(view) => Some(view.cursor_screen_row()),
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .unwrap_or(0)
    }

    /// Push the document's current verdicts onto the file view.
    ///
    /// `cursor_source` is where the cursor was *before* whatever changed the
    /// verdicts or the mode — it is remapped here onto the now-current
    /// visible set, since a filter or the hide toggle may have removed the
    /// exact line it was on.
    ///
    /// When nothing is hidden the buffer is the whole document and the gutter
    /// numbers itself. As soon as a line is hidden the buffer holds only the
    /// visible lines, so the gutter must be told each row's *source* number or
    /// it would renumber 1..N and the line numbers would be lies. And when
    /// hiding leaves nothing visible at all, the buffer falls back to a single
    /// blank placeholder row — which must not get a gutter number of its own,
    /// or it reads as "this file has one empty line".
    ///
    /// Rebuilding the buffer (`TextArea::set_lines`) resets the viewport's
    /// scroll position, so it only happens when the visible *set* of rows has
    /// actually changed from what the buffer was last built with. Otherwise a
    /// filter change that matches nothing — or any other no-op change to the
    /// verdicts — would still re-anchor the view to wherever the cursor's row
    /// happens to land in a freshly reset viewport, which reads as the view
    /// jumping a full page for no reason. The gutter numbers and line styles
    /// are cheap and must stay in sync regardless, so those are always
    /// reapplied.
    ///
    /// A rebuild that *does* happen still resets the viewport, so the screen
    /// row the cursor was drawn on is captured before touching anything and
    /// restored once the buffer is back in place — every caller rebuilds
    /// through here, so capturing it inside this method rather than asking
    /// each caller to pass it in (the way `cursor_source` is) means no future
    /// caller can forget and leave the view re-anchoring under it. Nothing
    /// above this point touches the textarea, so capturing before any of it
    /// runs is safe — unlike `cursor_source`, which has to be captured by the
    /// caller before `recompute_visible` destroys the *old* visible list it
    /// is measured against.
    fn apply_view(&mut self, cursor_source: usize) {
        let screen_row = self.file_view_screen_row();
        let cursor_source = self
            .document
            .nearest_visible(cursor_source)
            .unwrap_or(cursor_source);

        let hiding = self.document.visible().len() < self.document.lines().len();
        let nothing_visible = hiding && self.document.visible().is_empty();
        let styles = self.document.visible_styles(&self.filters);
        let numbers: Vec<usize> = if hiding {
            self.document.visible().to_vec()
        } else {
            Vec::new()
        };

        // `CursorMove::Jump` takes a `u16`, which silently truncates past
        // 65,535 lines and lands the cursor 65,536 lines from its target on a
        // large log. `set_lines` clamps in `usize` and replaces the buffer in
        // the same call, so the row is applied directly rather than jumped to
        // afterwards. (The rendered viewport is still `u16`-limited, though —
        // see `FileView::show_lines_with_cursor`.)
        let row = self.document.visible_position(cursor_source).unwrap_or(0);
        let rebuild = self.document.visible() != self.last_visible.as_slice();
        let lines = if rebuild {
            self.last_visible = self.document.visible().to_vec();
            Some(self.document.visible_lines())
        } else {
            None
        };

        let Some(view) = self.widgets.iter_mut().find_map(|w| match w {
            AppWidget::FileView(view) => Some(view),
            AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
        }) else {
            return;
        };
        if let Some(lines) = lines {
            view.show_lines_with_cursor(lines, row);
        }
        view.set_line_numbers(numbers);
        view.set_line_styles(styles);
        view.set_gutter_blank(nothing_visible);
        // Only a rebuild resets the viewport, so only a rebuild needs the
        // cursor nudged back onto `screen_row` — the whole point of
        // `scroll_cursor_to_row` is undoing that reset. Requesting it
        // unconditionally used to queue a no-op nudge on every call that hid
        // nothing new (every navigator arrow key goes through `Preview` →
        // `refresh_view` → here, whether or not a filter is even defined),
        // and `apply_pending_scroll` pays for that with a full scratch
        // render of the file view on the very next frame regardless of
        // whether there was anything to correct.
        if rebuild {
            view.scroll_cursor_to_row(screen_row);
        }
    }

    /// The source line the cursor is on, mapped through the *current* visible
    /// list before it is rebuilt.
    fn cursor_source(&self) -> usize {
        self.widgets
            .iter()
            .find_map(|widget| match widget {
                AppWidget::FileView(view) => {
                    let row = view.textarea.cursor().0;
                    Some(self.document.source_at(row).unwrap_or(row))
                }
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .unwrap_or(0)
    }

    /// A one-line summary of the filter state, empty when no filters exist.
    ///
    /// Dimming alone does not say *why* lines are dim, or that a filter is
    /// defined but currently disabled — the pane would just look ordinary.
    fn status_text(&self) -> String {
        // `FilteredOnly` is not the only way lines leave the screen: an
        // excluding filter (`F`) removes its matches in `Dimmed` mode too,
        // which is the entire point of it. Showing the funnel only for
        // `FilteredOnly` let `F` empty the pane with nothing on the status
        // line saying so.
        let hiding = self.document.mode() == Mode::FilteredOnly || self.filters.any_excluding();
        let funnel = if hiding { "▼ " } else { "" };
        if self.filters.is_empty() {
            // With no filters every line is unmatched, so `FilteredOnly`
            // shows nothing at all. Reporting it here is the only way the
            // user can tell the file is intact rather than gone.
            return if hiding {
                format!("{funnel}nothing to show — no filters")
            } else {
                String::new()
            };
        }
        let count = self.filters.len();
        let noun = if count == 1 { "filter" } else { "filters" };
        if !self.filters.any_enabled() {
            return format!("{funnel}{count} {noun} (disabled)");
        }
        // Report what is actually on screen (lines *shown*) rather than how
        // many matched an including filter: an excluding filter alone can
        // remove lines while matching nothing, which used to read as "0
        // matched" over a pane that had in fact lost lines.
        format!(
            "{funnel}{count} {noun}   {}/{} lines shown",
            self.document.visible().len(),
            self.document.lines().len()
        )
    }

    fn nav_index(&self) -> usize {
        self.index_of(|widget| matches!(widget, AppWidget::FileNav(_)))
    }

    fn file_view_index(&self) -> usize {
        self.index_of(|widget| matches!(widget, AppWidget::FileView(_)))
    }

    /// Mirrors `nav_index`/`file_view_index`. Nothing in production code
    /// needs the filter pane's position yet — Task 5 has no "jump to the
    /// filter pane" command of its own — so this is test-only for now,
    /// unlike its two siblings, which `zoom_file_view` and
    /// `reveal_and_focus_nav` both call for real.
    #[cfg(test)]
    fn filter_list_index(&self) -> usize {
        self.index_of(|widget| matches!(widget, AppWidget::FilterList(_)))
    }

    /// `unwrap_or(0)` used to paper over "the pane is not registered" as
    /// index 0 — the navigator — turning that bug into a silent wrong-pane
    /// zoom instead of a panic: with three panes now in `self.widgets`, `b`
    /// (`zoom_file_view` -> `file_view_index`) would zoom the navigator
    /// instead of the file view, with nothing to say why. A caller asking
    /// for a widget kind that genuinely is not present is a programming
    /// error, not a state this method should quietly paper over.
    fn index_of(&self, predicate: impl Fn(&AppWidget<'_>) -> bool) -> usize {
        self.widgets.iter().position(predicate).expect(
            "every AppWidget variant index_of is asked for must be registered in self.widgets",
        )
    }

    /// Move focus to the next pane, skipping the filter pane while it is
    /// collapsed — focusing a pane that is not on screen would strand the
    /// user with no visible cursor.
    fn focus_next(&mut self) {
        let count = self.widgets.len();
        for step in 1..=count {
            let candidate = (self.active_widget + step) % count;
            let collapsed = matches!(self.widgets[candidate], AppWidget::FilterList(_))
                && self.filters.is_empty();
            if !collapsed {
                self.active_widget = candidate;
                break;
            }
        }
        // The zoomed pane is always the focused pane, so the cursor is never
        // on a pane that is not on screen. This lives inside `focus_next`
        // itself, rather than beside its call site, so a future caller of
        // `focus_next` cannot forget it — the early `break` above makes that
        // easy to drop if it were bolted on afterwards instead.
        if self.zoom.is_some() {
            self.zoom = Some(self.active_widget);
        }
    }

    /// Zoom `target`, or restore the split if it is already zoomed. Reports
    /// whether the pane ended up zoomed.
    fn toggle_zoom(&mut self, target: usize) -> bool {
        // A drag in progress has no divider to keep tracking once zoomed —
        // the `Drag` arm in `handle_divider` only checks `self.dragging`, not
        // whether a divider is actually on screen — so it would otherwise
        // keep silently re-pinning `nav_width` while nothing is drawn to
        // explain why, with the new width only appearing on un-zoom. Zooming
        // (in either direction) cancels it outright. Unzooming is a no-op
        // here in practice, since a drag can only start via a click that
        // `on_divider` accepted, and that can't happen while already zoomed.
        self.dragging = false;
        self.zoom = match self.zoom {
            Some(index) if index == target => None,
            _ => Some(target),
        };
        self.zoom == Some(target)
    }

    /// Maximise the focused pane, or restore the split if it already is.
    fn zoom_focused(&mut self) {
        self.toggle_zoom(self.active_widget);
    }

    /// Give the file its full width. Focus follows, because the pane the
    /// cursor was in may no longer be on screen.
    ///
    /// Restoring the split on the second press deliberately leaves focus in
    /// the file view rather than dragging it back to the navigator: you
    /// pressed `b` to read the file, so that is where you want to stay. `e`
    /// is the documented way back, precisely so `b` does not have to carry
    /// that job too.
    fn zoom_file_view(&mut self) {
        let view = self.file_view_index();
        if self.toggle_zoom(view) {
            self.active_widget = view;
        }
    }

    /// Bring the left column back and put the cursor in it.
    fn reveal_and_focus_nav(&mut self) {
        self.zoom = None;
        self.active_widget = self.nav_index();
    }
}

/// Render one widget into `area`, reaching past `AppWidget`'s own `Widget`
/// impl for the filter pane so it actually draws something.
///
/// `AppWidget::FilterList`'s arm of that impl deliberately renders nothing —
/// it has no `FilterSet` to render against, since `App` owns the one true
/// set and a copy on the widget could go stale the moment a filter changed.
/// Both branches of `App::render` (the ordinary split and the zoom special
/// case) go through this helper, so the filter pane cannot go blank in one
/// of them while working in the other.
fn render_widget(widget: &mut AppWidget<'_>, filters: &FilterSet, area: Rect, buf: &mut Buffer) {
    match widget {
        AppWidget::FilterList(list) => list.render(filters, area, buf),
        AppWidget::FileNav(_) | AppWidget::FileView(_) => widget.render(area, buf),
    }
}

impl Widget for &mut App<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use Constraint::{Length, Min};
        // calculate rects where widgets should be rendered
        assert!(self.widgets.len() == 3);

        // The search prompt borrows the bottom row while it is open; failing
        // that, the status line borrows it whenever there is something to show.
        let status = self.status_text();
        let (area, prompt_area) = if self.search.is_some() || !status.is_empty() {
            let [panes, prompt] = Layout::vertical([Min(0), Length(1)]).areas(area);
            (panes, Some(prompt))
        } else {
            (area, None)
        };

        // A zoomed pane takes the whole pane area; the others are not drawn.
        // This deliberately falls through to the status/prompt drawing below
        // rather than returning, so the status line survives a zoom.
        if let Some(index) = self.zoom {
            debug_assert_eq!(
                index, self.active_widget,
                "the zoomed pane must be the focused pane"
            );
            // There is no divider to drag while zoomed. `run` draws exactly
            // one frame per event read, so `divider` is always recomputed by
            // `render` before the next mouse event can be hit-tested — there
            // is no frame after un-zooming that could carry a stale
            // `u16::MAX` forward. Parking it here at `u16::MAX` — well past
            // any real terminal width — is a second, independent reason a
            // stray hit-test could not land on it even without that guarantee.
            self.divider = u16::MAX;
            for (i, widget) in self.widgets.iter_mut().enumerate() {
                widget.set_active(i == self.active_widget);
            }
            if let Some(widget) = self.widgets.get_mut(index) {
                render_widget(widget, &self.filters, area, buf);
            }
        } else {
            let nav_width = self.nav_width(area);
            let [left, right] = Layout::horizontal([Length(nav_width), Min(0)]).areas(area);

            // The filter pane sits under the navigator inside the left
            // column; it claims its preferred height first, leaving the
            // navigator whatever remains, down to `MIN_NAV_HEIGHT` on a very
            // short terminal — see
            // `a_short_terminal_shows_a_real_filter_row_not_just_the_title`.
            // `filter_pane_split_height` does the capping arithmetically, so
            // the navigator's own constraint here can be a bare `Min(0)` and
            // still never drop below its floor while a terminal has enough
            // rows to give the left column at all.
            let filter_height = self.filter_pane_split_height(left.height);
            let [nav_area, filter_area] =
                Layout::vertical([Min(0), Length(filter_height)]).areas(left);

            // Remember the boundary so mouse events landing before the next
            // frame can be tested against it.
            self.divider = area.x + nav_width;

            for (i, w) in self.widgets.iter_mut().enumerate() {
                w.set_active(i == self.active_widget);
                // Each widget gets the area that matches what it is, not
                // its position in `self.widgets` — the three areas above
                // don't share that vec's order, and indexing into a
                // two-element array by a three-widget position (as this
                // used to) panics the moment a third widget exists.
                let widget_area = match w {
                    AppWidget::FileNav(_) => nav_area,
                    AppWidget::FileView(_) => right,
                    AppWidget::FilterList(_) => filter_area,
                };
                render_widget(w, &self.filters, widget_area, buf);
            }
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
    use crossterm::event::{KeyEvent, MouseEvent};
    use ratatui::prelude::Buffer;
    use ratatui::style::Modifier; // the tests assert on Modifier::DIM
    use std::fs;
    use std::sync::Mutex;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 10,
    };

    /// Every fixture directory name claimed so far in this process.
    /// `app_over` and `app_over_file` both derive `target/test-appdirs/<name>`
    /// from `name`, so they share one namespace.
    static FIXTURE_DIR_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Panic loudly if `name` has already been used for a fixture directory
    /// in this process, instead of letting two tests race to
    /// `remove_dir_all`/`create_dir_all` the same path. That race is exactly
    /// what caused a real, release-only flake: both tests "succeeded" and
    /// just clobbered each other's files depending on interleaving.
    fn claim_fixture_dir(name: &str) {
        let mut names = FIXTURE_DIR_NAMES
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(
            !names.iter().any(|used| used == name),
            "fixture directory name {name:?} is already in use by another test — pick a unique name"
        );
        names.push(name.to_string());
    }

    /// An app listing a directory with known entry names.
    fn app_over(name: &str, files: &[&str]) -> App<'static> {
        claim_fixture_dir(name);
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
        // The bound above is `MIN_PANE_WIDTH` (3), which the file view's own
        // floor, `MIN_FILE_VIEW_WIDTH` (30), also satisfies — so it alone
        // cannot tell the two floors apart. A drag all the way to the far
        // edge is the pinned-width equivalent of
        // `a_long_filter_pattern_on_a_narrow_terminal_leaves_the_file_view_its_floor`,
        // so it gets the same exact-equality assertion: the doc comment on
        // `MIN_FILE_VIEW_WIDTH` claims the ceiling applies to a drag just as
        // much as to auto-sizing, and nothing was pinning that claim.
        assert_eq!(
            AREA.width - app.nav_width(AREA),
            MIN_FILE_VIEW_WIDTH,
            "a hard drag to the far edge did not stop at the file view's floor"
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

    /// Put focus on the file view, however many `Tab` presses that takes.
    ///
    /// A fixed `key(Tab); ...; key(Tab);` pair only reaches the file view
    /// because `Tab` used to be a two-state toggle; the filter pane joining
    /// the cycle once a filter exists already broke that assumption for
    /// three tests, and a later phase adding a fourth pane would break it
    /// again. Tabbing until the target is reached, rather than a fixed
    /// number of times, is robust to however many panes exist.
    fn focus_file_view(app: &mut App) {
        let target = app.file_view_index();
        for _ in 0..app.widgets.len() {
            if app.active_widget == target {
                return;
            }
            key(app, KeyCode::Tab);
        }
        panic!("could not reach the file view by tabbing");
    }

    /// Put focus on the filter pane, however many `Tab` presses that takes.
    /// Bounded the same way as `focus_file_view`, for the same reason: a
    /// fixed count would break the moment a fourth pane joined the cycle.
    fn focus_filter_pane(app: &mut App) {
        let target = app.filter_list_index();
        for _ in 0..app.widgets.len() {
            if app.active_widget == target {
                return;
            }
            key(app, KeyCode::Tab);
        }
        panic!("could not reach the filter pane by tabbing");
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
        fs::write(
            "target/test-appdirs/prompt_preview/gamma.rs",
            "GAMMA MARKER\n",
        )
        .unwrap();
        draw(&mut app);

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "gamma");
        key(&mut app, KeyCode::Enter);

        let mut buf = Buffer::empty(AREA);
        (&mut app).render(AREA, &mut buf);
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.contains("GAMMA MARKER"),
            "matched file was not previewed"
        );
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
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .expect("no file view")
    }

    /// Create (or recreate) `target/test-appdirs/<name>/log.txt` with `body`,
    /// claiming the fixture directory name first so a duplicate is rejected
    /// loudly rather than racing another test for the same path.
    fn fixture_path(name: &str, body: &str) -> std::path::PathBuf {
        claim_fixture_dir(name);
        let dir = std::path::Path::new("target/test-appdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        let file = dir.join("log.txt");
        fs::write(&file, body).expect("write fixture");
        file
    }

    fn app_over_file(name: &str, body: &str) -> App<'static> {
        let file = fixture_path(name, body);
        App::new(&Config {
            file: file.display().to_string(),
        })
    }

    /// A duplicate fixture directory name must fail loudly and immediately,
    /// not race with whatever other test already claimed it. The panic
    /// happens in `claim_fixture_dir` before any filesystem work, and
    /// `claim_fixture_dir` recovers the lock via `into_inner` on poison, so
    /// this does not wedge the guard for every test that runs after it.
    #[test]
    fn a_duplicate_fixture_directory_name_is_rejected() {
        let _first = app_over("dup_fixture_name", &["a.rs"]);

        let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app_over("dup_fixture_name", &["a.rs"]);
        }));

        assert!(
            second.is_err(),
            "a duplicate fixture directory name was not rejected"
        );
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

    /// `sync_document` replaces `self.document` wholesale, so whatever
    /// `last_visible` (finding 1's rebuild-skip guard) held is meaningless
    /// afterwards — it describes a buffer built from the *previous*
    /// document. Reloading the *same* file while an excluding filter is
    /// active reproduces an identical `visible()` list every time (the
    /// document is genuinely equal), which the guard alone cannot tell apart
    /// from "nothing changed". `FileNav` fires `Action::Load` unconditionally
    /// on `Enter`, even over the entry that is already open, so this is not a
    /// contrived path.
    #[test]
    fn reloading_the_same_file_reapplies_an_active_excluding_filter() {
        let mut app = app_over_file("reload_same_file", "alpha\nnoise\ngamma\n");
        let path =
            std::path::Path::new("target/test-appdirs/reload_same_file/log.txt").to_path_buf();

        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            view_lines(&app),
            vec!["alpha".to_string(), "gamma".to_string()],
            "sanity: the excluding filter hid a line"
        );

        app.perform(Action::Load(path));

        assert_eq!(
            view_lines(&app),
            vec!["alpha".to_string(), "gamma".to_string()],
            "the reload brought back the hidden line: the buffer kept the \
             freshly loaded, unfiltered content instead of being rebuilt"
        );
        assert_eq!(
            view_line_styles(&app).len(),
            view_lines(&app).len(),
            "styles/numbers were sized for the filtered subset but the buffer came back full-length"
        );
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

    /// `!` must put back what the user had, not switch everything on.
    #[test]
    fn bang_restores_the_per_filter_state_it_captured() {
        let mut app = app_over_file("bang_restore", "alpha\nbeta\n");
        for pattern in ["alpha", "beta"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        app.filters.set_enabled(1, false);

        key(&mut app, KeyCode::Char('!'));
        assert!(!app.filters.any_enabled());

        key(&mut app, KeyCode::Char('!'));

        assert!(app.filters.filters()[0].enabled);
        assert!(
            !app.filters.filters()[1].enabled,
            "a filter the user had disabled was switched back on"
        );
    }

    /// With every filter disabled by hand and nothing captured, `!` has no
    /// prior state to restore — so it enables everything, rather than
    /// capturing all-disabled and becoming inert.
    #[test]
    fn bang_re_enables_when_everything_was_disabled_by_hand() {
        let mut app = app_over_file("bang_escape", "alpha\nbeta\n");
        for pattern in ["alpha", "beta"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        app.filters.set_enabled(0, false);
        app.filters.set_enabled(1, false);

        key(&mut app, KeyCode::Char('!'));

        assert!(
            app.filters.any_enabled(),
            "! left the user with no way back"
        );
    }

    /// Adding a filter while `!` has a capture pending must not strand it.
    /// The capture describes a set that no longer exists, so it is dropped and
    /// the next `!` captures afresh — otherwise `!` sees an enabled filter,
    /// finds a capture already pending, and silently does nothing forever.
    #[test]
    fn adding_a_filter_after_bang_does_not_leave_bang_inert() {
        let mut app = app_over_file("bang_after_add", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('!'));
        assert!(!app.filters.any_enabled());

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));
        assert!(
            !app.filters.any_enabled(),
            "! did not disable the new filter"
        );

        key(&mut app, KeyCode::Char('!'));
        assert!(
            app.filters.any_enabled(),
            "! went inert - nothing came back"
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
    fn the_status_line_reports_filters_and_lines_shown() {
        let mut app = app_over_file("status_some", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let status = status_line(&mut app);

        assert!(
            status.contains("1 filter") && !status.contains("1 filters"),
            "count is not singular-aware: {status}"
        );
        // An including filter alone dims rather than removes, so every line
        // is still shown even though only one of them matched.
        assert!(
            status.contains("3/3"),
            "lines-shown count missing: {status}"
        );
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
        assert!(
            preview_styles[10].is_some(),
            "match line unstyled in the preview"
        );

        // Tab into the file view and press a key: this is exactly what
        // upgrades the truncated preview to a full load inside
        // `FileView::handle_events`. A filter is already defined here, so a
        // bare `Tab` would no longer land on the file view once the filter
        // pane joins the cycle — `focus_file_view` tabs however many times
        // that takes.
        focus_file_view(&mut app);
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
        focus_file_view(&mut app);

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
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .expect("no file view")
    }

    /// The text the file view is currently showing, one entry per row.
    fn view_lines(app: &App) -> Vec<String> {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.textarea.lines().to_vec()),
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .expect("no file view")
    }

    fn view_line_numbers(app: &App) -> Vec<usize> {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.textarea.line_numbers().to_vec()),
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .expect("no file view")
    }

    #[test]
    fn an_excluding_filter_removes_its_lines_from_the_view() {
        let mut app = app_over_file("exclude_view", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        assert_eq!(
            view_lines(&app),
            vec!["alpha".to_string(), "gamma".to_string()]
        );
    }

    /// The gutter keeps the original numbering, so a hidden line leaves a gap.
    #[test]
    fn the_gutter_shows_source_line_numbers_when_lines_are_hidden() {
        let mut app = app_over_file("exclude_gutter", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        // 0-based source indices: rows 0 and 2 render as 1 and 3.
        assert_eq!(view_line_numbers(&app), vec![0, 2]);
    }

    #[test]
    fn styles_still_line_up_with_the_rebuilt_buffer() {
        let mut app = app_over_file("exclude_styles", "alpha\nnoise\nbeta\n");
        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_line_styles(&app).len(), view_lines(&app).len());
    }

    /// With nothing excluded the buffer is the whole file and the gutter is
    /// left to number itself.
    #[test]
    fn without_hiding_the_gutter_is_not_overridden() {
        let mut app = app_over_file("no_hiding", "alpha\nbeta\n");

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_lines(&app).len(), 2);
        assert!(
            view_line_numbers(&app).is_empty(),
            "the gutter was overridden when nothing is hidden"
        );
    }

    /// Lifting the hiding must restore the whole buffer. Leaving a stale subset
    /// behind is worse than never hiding: the gutter override is cleared at the
    /// same moment, so the remaining rows would renumber from 1 and claim to be
    /// the whole file.
    #[test]
    fn disabling_an_excluding_filter_restores_the_hidden_lines() {
        let mut app = app_over_file("exclude_restore", "alpha\nnoise\ngamma\n");
        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);
        assert_eq!(view_lines(&app).len(), 2, "the line was not hidden");

        key(&mut app, KeyCode::Char('!'));

        assert_eq!(
            view_lines(&app),
            vec![
                "alpha".to_string(),
                "noise".to_string(),
                "gamma".to_string()
            ],
            "the hidden line did not come back"
        );
        assert_eq!(
            view_line_styles(&app).len(),
            view_lines(&app).len(),
            "styles no longer line up with the buffer"
        );
        assert!(
            view_line_numbers(&app).is_empty(),
            "the gutter is still overridden with nothing hidden"
        );
    }

    /// The cursor's source line, derived from where it sits in the view.
    fn cursor_source(app: &App) -> usize {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => {
                    let row = view.textarea.cursor().0;
                    Some(app.document.source_at(row).unwrap_or(row))
                }
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .expect("no file view")
    }

    fn cursor_screen_row(app: &App) -> u16 {
        app.widgets
            .iter()
            .find_map(|w| match w {
                AppWidget::FileView(view) => Some(view.cursor_screen_row()),
                AppWidget::FileNav(_) | AppWidget::FilterList(_) => None,
            })
            .expect("no file view")
    }

    /// Move the cursor to `row` without going through `CursorMove::Jump`,
    /// whose `u16` argument would silently truncate on the large-file test
    /// below.
    fn move_cursor_to_visible_row(app: &mut App, row: usize) {
        for widget in &mut app.widgets {
            if let AppWidget::FileView(view) = widget {
                let lines = view.textarea.lines().to_vec();
                view.textarea.set_lines(lines, (row, 0));
            }
        }
    }

    /// The row (if any) whose rendered text contains `needle`.
    fn row_containing(buf: &Buffer, needle: &str) -> Option<u16> {
        (0..buf.area.height).find(|&y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
    }

    /// A regression against the previous phase: `restyle` (Phase 2a) only
    /// set styles and never touched the buffer, so a filter change left the
    /// view exactly where it was. `refresh_view` rebuilding unconditionally
    /// broke that, because rebuilding resets the textarea's viewport —
    /// so any filter change re-anchored the scroll on the next render, even
    /// one that changed nothing about what is on screen.
    ///
    /// Demonstrated exactly as found: the cursor is scrolled so its line
    /// sits at the top of the pane, then a filter that matches nothing is
    /// added. Nothing about the visible rows changed, so the view must not
    /// move.
    #[test]
    fn a_filter_matching_nothing_does_not_move_the_viewport() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 12,
        };
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("no_op_filter_viewport", &body);
        focus_file_view(&mut app);

        // Render once so the textarea knows its viewport size (10 rows of
        // content inside the 12-row, bordered pane), then page down nine
        // screens so the cursor's line lands exactly at the top of the pane.
        let mut buf = Buffer::empty(area);
        (&mut app).render(area, &mut buf);
        for _ in 0..9 {
            app.handle_event(event::Event::Key(KeyEvent::new(
                KeyCode::Char('f'),
                KeyModifiers::CONTROL,
            )))
            .unwrap();
        }

        let mut buf = Buffer::empty(area);
        (&mut app).render(area, &mut buf);
        let before = row_containing(&buf, "line 90").expect("line 90 should be on screen");

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "zzz_never_matches");
        key(&mut app, KeyCode::Enter);

        let mut buf = Buffer::empty(area);
        (&mut app).render(area, &mut buf);
        let after = row_containing(&buf, "line 90").expect("line 90 should still be on screen");

        assert_eq!(
            before, after,
            "adding a filter that matched nothing moved the viewport"
        );
    }

    /// Toggling a filter changes the visible set, so the buffer is rebuilt —
    /// but the line under the cursor must stay on the same screen row rather
    /// than the view re-anchoring beneath it.
    ///
    /// An *excluding* filter is used deliberately: an including filter in
    /// the default (`Dimmed`) mode never changes `visible` at all — it only
    /// changes styling — so `!` would trigger no rebuild and the test would
    /// pass trivially, before any fix exists. Excluded lines are dropped
    /// from `visible` in every mode, so toggling one genuinely forces the
    /// rebuild this test is about.
    #[test]
    fn toggling_a_filter_leaves_the_cursor_on_the_same_screen_row() {
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("scroll_hold", &body);
        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "line 1[5-9][0-9]"); // excludes 150..=199, well below the cursor
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        // Put the cursor well down the file. Moving down one line at a time
        // like this pins it to the pane's *last* screen row — the viewport
        // scrolls minimally to keep it in view, landing it on the bottom
        // edge every time.
        for _ in 0..120 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('j'));
        }
        draw(&mut app);
        let pinned_row = cursor_screen_row(&app);

        // Pull it back off that row: the pane's last row is exactly where a
        // reset viewport would re-anchor the cursor after a rebuild, so
        // parking there would make the bug and the fix indistinguishable.
        for _ in 0..3 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('k'));
        }
        draw(&mut app);
        let before_row = cursor_screen_row(&app);
        let before_source = cursor_source(&app);
        let before_len = view_lines(&app).len();
        assert!(
            before_row < pinned_row,
            "test setup did not move the cursor off the pane's last row \
             (pinned_row = {pinned_row}, before_row = {before_row}) — \
             this test would pass whether or not the fix exists"
        );

        key(&mut app, KeyCode::Char('!'));
        draw(&mut app);

        // A structural guard on the rebuild itself: if a future change (e.g.
        // swapping `F` for `f`) stopped the buffer from actually changing
        // size, the assertions below would pass trivially whether or not the
        // fix exists — the same failure mode correction (a) already covers,
        // pinned here so it can't quietly regress.
        assert_ne!(
            view_lines(&app).len(),
            before_len,
            "the buffer did not change size, so `!` did not force a rebuild \
             here — this test would pass whether or not the fix exists"
        );
        assert_eq!(
            cursor_screen_row(&app),
            before_row,
            "the view re-anchored instead of holding the line in place"
        );
        assert_eq!(
            cursor_source(&app),
            before_source,
            "the cursor changed line"
        );
    }

    /// Companion to the test above: there the excluded block sits *below*
    /// the cursor, so the cursor's own absolute buffer row never moves — only
    /// the viewport reset is exercised. Here the excluded block sits
    /// *above* it, so restoring the excluded lines shifts the cursor's
    /// buffer row itself (lines appear above it), which is the case
    /// `scroll_cursor_to_row`'s `desired_top` / `saturating_sub` clamping
    /// exists for. The screen row must still hold.
    #[test]
    fn toggling_a_filter_above_the_cursor_also_leaves_the_cursor_on_the_same_screen_row() {
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("scroll_hold_above", &body);
        key(&mut app, KeyCode::Char('F'));
        // Excludes source lines 0..=49, anchored so e.g. "line 100" is not
        // also matched as a substring of "line 10".
        typed(&mut app, "^line ([0-9]|[1-4][0-9])$");
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        // The excluded block snaps the initial cursor forward to line 50
        // (the nearest remaining visible line), then this drives it further
        // down — well clear of the excluded block either way.
        for _ in 0..120 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('j'));
        }
        draw(&mut app);
        let pinned_row = cursor_screen_row(&app);

        for _ in 0..3 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('k'));
        }
        draw(&mut app);
        let before_row = cursor_screen_row(&app);
        let before_source = cursor_source(&app);
        let before_len = view_lines(&app).len();
        assert!(
            before_row < pinned_row,
            "test setup did not move the cursor off the pane's last row \
             (pinned_row = {pinned_row}, before_row = {before_row}) — \
             this test would pass whether or not the fix exists"
        );

        key(&mut app, KeyCode::Char('!'));
        draw(&mut app);

        assert_ne!(
            view_lines(&app).len(),
            before_len,
            "the buffer did not change size, so `!` did not force a rebuild \
             here — this test would pass whether or not the fix exists"
        );
        assert_eq!(
            cursor_screen_row(&app),
            before_row,
            "the view re-anchored instead of holding the line in place"
        );
        assert_eq!(
            cursor_source(&app),
            before_source,
            "the cursor changed line"
        );
    }

    /// The two tests above drive this same screen-row criterion through `!`
    /// and (below) `H` — both global bindings — but never through the
    /// filter pane's own `space` key, even though the spec calls pane
    /// toggling "the dominant interaction" once filters exist. Without this,
    /// a future `handle_filter_key` change that bypassed `refresh_view` —
    /// patching the cached verdicts in place instead of re-evaluating, say
    /// — would pass every screen-row test in this file while breaking the
    /// one interaction the pane exists for.
    #[test]
    fn toggling_a_filter_from_the_pane_leaves_the_cursor_on_the_same_screen_row() {
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("pane_scroll_hold", &body);
        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "line 1[5-9][0-9]"); // excludes 150..=199, well below the cursor
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        for _ in 0..120 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('j'));
        }
        draw(&mut app);
        let pinned_row = cursor_screen_row(&app);

        for _ in 0..3 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('k'));
        }
        draw(&mut app);
        let before_row = cursor_screen_row(&app);
        let before_source = cursor_source(&app);
        let before_len = view_lines(&app).len();
        assert!(
            before_row < pinned_row,
            "test setup did not move the cursor off the pane's last row \
             (pinned_row = {pinned_row}, before_row = {before_row}) — \
             this test would pass whether or not the fix exists"
        );

        // The pane's own key, not `!` — this is the criterion this test adds.
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char(' '));
        draw(&mut app);

        assert_ne!(
            view_lines(&app).len(),
            before_len,
            "the buffer did not change size, so the pane's toggle did not \
             force a rebuild here — this test would pass whether or not the \
             fix exists"
        );
        assert_eq!(
            cursor_screen_row(&app),
            before_row,
            "the view re-anchored instead of holding the line in place"
        );
        assert_eq!(
            cursor_source(&app),
            before_source,
            "the cursor changed line"
        );
    }

    /// `H` always rebuilds the buffer — unlike a filter change, there is no
    /// "visible set happens to be unchanged" case to skip it — so it is the
    /// most literal instance of the rebuild this whole task is about. It
    /// must hold the cursor's screen row exactly as `!` does, via the same
    /// `apply_view` path.
    #[test]
    fn h_holds_the_cursor_on_the_same_screen_row() {
        // Lines below 100 are never matched, so `FilteredOnly` drops them
        // and keeps only 100..199 — a large, predictable rebuild — while the
        // cursor, parked inside the matched range, stays visible throughout.
        let body: String = (0..200)
            .map(|i| {
                if i < 100 {
                    format!("plain {i}\n")
                } else {
                    format!("match {i}\n")
                }
            })
            .collect();
        let mut app = app_over_file("h_scroll_hold", &body);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "^match ");
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        for _ in 0..120 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('j'));
        }
        draw(&mut app);
        let pinned_row = cursor_screen_row(&app);

        for _ in 0..3 {
            focus_file_view(&mut app);
            key(&mut app, KeyCode::Char('k'));
        }
        draw(&mut app);
        let before_row = cursor_screen_row(&app);
        let before_source = cursor_source(&app);
        let before_len = view_lines(&app).len();
        assert!(
            before_row < pinned_row,
            "test setup did not move the cursor off the pane's last row \
             (pinned_row = {pinned_row}, before_row = {before_row}) — \
             this test would pass whether or not the fix exists"
        );

        key(&mut app, KeyCode::Char('H'));
        draw(&mut app);

        assert_ne!(
            view_lines(&app).len(),
            before_len,
            "the buffer did not change size, so `H` did not force a rebuild \
             here — this test would pass whether or not the fix exists"
        );
        assert_eq!(
            cursor_screen_row(&app),
            before_row,
            "the view re-anchored instead of holding the line in place"
        );
        assert_eq!(
            cursor_source(&app),
            before_source,
            "the cursor changed line"
        );
    }

    #[test]
    fn h_hides_lines_that_match_no_filter() {
        let mut app = app_over_file("toggle_hide", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert_eq!(view_lines(&app).len(), 3, "nothing hidden yet");

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(view_lines(&app), vec!["beta".to_string()]);
    }

    #[test]
    fn ctrl_h_toggles_the_same_way() {
        let mut app = app_over_file("toggle_ctrl_h", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        app.handle_event(event::Event::Key(event::KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();

        assert_eq!(view_lines(&app), vec!["beta".to_string()]);
    }

    /// The workflow: filter, hide, scroll to a match, show everything again,
    /// and land on that exact line with its context around it.
    #[test]
    fn the_round_trip_returns_to_the_chosen_line() {
        let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let mut app = app_over_file("round_trip", &body);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "line 1[0-9]");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));
        // Visible rows are now source lines 10..=19; pick the third of them.
        move_cursor_to_visible_row(&mut app, 2);
        assert_eq!(cursor_source(&app), 12);

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(cursor_source(&app), 12, "did not return to the same line");
        assert_eq!(view_lines(&app).len(), 20, "context did not come back");
    }

    /// `CursorMove::Jump` takes a `u16`, so a restore that went through it
    /// would silently truncate a row above 65,535 and land 65,536 lines from
    /// the chosen one instead of on it.
    #[test]
    fn the_round_trip_survives_more_than_65535_lines() {
        const TOTAL: usize = 70_000;
        const TARGET: usize = 66_000;
        let body: String = (0..TOTAL)
            .map(|i| {
                if i == TARGET {
                    "MATCH\n".to_string()
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        let mut app = app_over_file("large_round_trip", &body);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "MATCH");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));
        assert_eq!(
            cursor_source(&app),
            TARGET,
            "did not land on the sole match"
        );

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(
            cursor_source(&app),
            TARGET,
            "did not return to the same line past the u16 boundary"
        );
        assert_eq!(view_lines(&app).len(), TOTAL, "context did not come back");
    }

    /// Toggling into hidden mode from a line that is not a match snaps forward
    /// to the next one, and toggling back lands on that.
    #[test]
    fn hiding_from_an_unmatched_line_snaps_to_the_next_match() {
        let mut app = app_over_file("snap_to_match", "alpha\nbeta\ngamma\nbeta two\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        move_cursor_to_visible_row(&mut app, 2); // gamma, unmatched
        assert_eq!(cursor_source(&app), 2);

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(cursor_source(&app), 3, "did not snap to the next match");
    }

    /// Hiding with nothing to show must not panic or lose the cursor.
    #[test]
    fn hiding_with_no_matches_is_survivable() {
        let mut app = app_over_file("no_matches", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "zzz");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));
        assert!(view_lines(&app).iter().all(String::is_empty) || view_lines(&app).is_empty());

        key(&mut app, KeyCode::Char('H'));
        assert_eq!(view_lines(&app).len(), 2, "did not come back");
    }

    /// When hiding leaves nothing visible, the buffer falls back to a single
    /// blank placeholder row. Before this, an empty `line_numbers` override
    /// fell back to natural 1..N numbering, so the gutter rendered "1" next
    /// to that blank row — reading as "this file has one empty line" when
    /// really both of its lines are just hidden.
    #[test]
    fn hiding_everything_does_not_show_a_phantom_line_number() {
        let mut app = app_over_file("no_matches_gutter", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "zzz");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));

        let mut buf = Buffer::empty(AREA);
        (&mut app).render(AREA, &mut buf);

        // Row 0 is the file view's top border, so row 1 is its first content
        // row — where the blank placeholder for "nothing visible" is drawn.
        // (The row still ends in the pane's own right-hand border, hence
        // checking for digits rather than requiring the whole row blank.)
        let divider = app.divider;
        let content_row: String = ((divider + 1)..AREA.width)
            .map(|x| buf[(x, 1)].symbol())
            .collect();
        assert!(
            !content_row.chars().any(|c| c.is_ascii_digit()),
            "expected no gutter number on the placeholder row, got: {content_row:?}"
        );
    }

    #[test]
    fn the_status_line_shows_a_funnel_while_hiding() {
        let mut app = app_over_file("funnel", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert!(!status_line(&mut app).contains('▼'));

        key(&mut app, KeyCode::Char('H'));

        assert!(
            status_line(&mut app).contains('▼'),
            "no indication that lines are hidden: {}",
            status_line(&mut app)
        );
    }

    /// An excluding filter (`F`) removes lines in `Dimmed` mode too — that is
    /// the entire point of it — so the funnel must not be gated on
    /// `FilteredOnly` alone. Before this, `F` matching every line rendered a
    /// blank pane with no indication anything was going on.
    #[test]
    fn the_status_line_shows_a_funnel_for_an_excluding_filter_while_dimmed() {
        let mut app = app_over_file("funnel_dimmed_exclude", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        assert_eq!(
            app.document.mode(),
            Mode::Dimmed,
            "sanity: the mode never changed, only the filter set"
        );
        assert!(
            status_line(&mut app).contains('▼'),
            "no funnel shown for an excluding filter while dimmed: {}",
            status_line(&mut app)
        );
    }

    /// An excluding-only filter set never produces an `Included` verdict, so
    /// the old `match_count`-based report always read "0 matched" even while
    /// visibly removing lines — indistinguishable from the filter matching
    /// nothing at all. The status line must describe what is on screen.
    #[test]
    fn the_status_line_reports_lines_shown_not_matched() {
        let mut app = app_over_file("status_shown_not_matched", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('F'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        let status = status_line(&mut app);

        assert!(
            status.contains("2/3"),
            "expected the two lines actually shown, got: {status}"
        );
        assert!(
            !status.contains("0/3"),
            "reported the (always-zero) match count instead of lines shown: {status}"
        );
    }

    /// With no filters, `H` empties the view entirely (every line is
    /// unmatched). Without a status row saying so, the pane just looks like a
    /// bare border and the user cannot tell the file is intact.
    #[test]
    fn hiding_with_no_filters_still_reports_status() {
        let mut app = app_over_file("hide_no_filters", "alpha\nbeta\n");

        key(&mut app, KeyCode::Char('H'));

        let status = status_line(&mut app);
        assert!(status.contains('▼'), "no funnel shown: {status}");
        assert!(
            status.to_lowercase().contains("no filters"),
            "does not explain why nothing is shown: {status}"
        );
    }

    fn rendered(app: &mut App) -> String {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        (0..AREA.height)
            .map(|y| {
                (0..AREA.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `b` gives the file its full width by hiding the left column.
    #[test]
    fn b_hides_the_left_column() {
        let mut app = app_over_file("zoom_b", "alpha\n");
        assert!(rendered(&mut app).contains("alpha"));
        let before = rendered(&mut app);

        key(&mut app, KeyCode::Char('b'));

        let after = rendered(&mut app);
        assert_ne!(before, after, "the layout did not change");
        assert!(after.contains("alpha"), "the file view went missing");
        assert!(
            !after.contains(">>"),
            "the navigator's selection marker is still on screen"
        );
    }

    /// `b` restores the split, but deliberately leaves the cursor where it
    /// moved it: you pressed `b` to read the file, so being dropped back into
    /// the navigator on the way out would be the surprise. `e` is the way back.
    #[test]
    fn b_toggles_back_but_leaves_focus_in_the_file_view() {
        let mut app = app_over_file("zoom_b_back", "alpha\n");
        assert_eq!(app.active_widget, app.nav_index());

        // Capture the baseline with focus already where `b b` will leave it,
        // so the comparison below isolates the layout claim rather than also
        // depending on the active/inactive distinction being style-only
        // (which `rendered`, collecting `symbol()` alone, cannot see).
        focus_file_view(&mut app);
        let before = rendered(&mut app);
        key(&mut app, KeyCode::Tab); // back to the real starting point

        key(&mut app, KeyCode::Char('b'));
        key(&mut app, KeyCode::Char('b'));

        assert_eq!(rendered(&mut app), before, "b did not restore the split");
        assert_eq!(
            app.active_widget,
            app.file_view_index(),
            "focus was dragged back to the navigator"
        );
        assert_eq!(app.zoom, None);
    }

    /// Hiding the column the cursor is in must move focus somewhere visible,
    /// or the user is left typing into a pane that is not on screen.
    #[test]
    fn b_moves_focus_out_of_the_hidden_column() {
        let mut app = app_over_file("zoom_b_focus", "alpha\n");
        assert_eq!(
            app.active_widget,
            app.nav_index(),
            "starts in the navigator"
        );

        key(&mut app, KeyCode::Char('b'));

        assert_eq!(app.active_widget, app.file_view_index());
    }

    /// `e` is how you get back, so it must work from a hidden state.
    #[test]
    fn e_reveals_the_left_column_and_focuses_it() {
        let mut app = app_over_file("zoom_e", "alpha\n");
        key(&mut app, KeyCode::Char('b'));

        key(&mut app, KeyCode::Char('e'));

        assert_eq!(app.zoom, None, "the left column is still hidden");
        assert_eq!(app.active_widget, app.nav_index());
    }

    #[test]
    fn e_focuses_the_navigator_even_when_nothing_is_hidden() {
        let mut app = app_over_file("zoom_e_visible", "alpha\n");
        focus_file_view(&mut app);
        assert_ne!(app.active_widget, app.nav_index());

        key(&mut app, KeyCode::Char('e'));

        assert_eq!(app.active_widget, app.nav_index());
    }

    /// `z` maximises whatever has focus — including the navigator, for long
    /// filenames.
    #[test]
    fn z_zooms_the_navigator_when_it_has_focus() {
        // A distinctive marker, not "alpha": the navigator titles its block
        // with the canonicalized checkout path, which could itself contain
        // "alpha" on some checkout — the negative assertion below would then
        // pass or fail depending on where the repo happens to be checked
        // out, rather than on what the test claims to check.
        let mut app = app_over_file("zoom_z_nav", "ZOOMMARKER\n");

        key(&mut app, KeyCode::Char('z'));

        let after = rendered(&mut app);
        assert!(after.contains(">>"), "the navigator is not on screen");
        assert!(
            !after.contains("ZOOMMARKER"),
            "the file view is still showing"
        );
    }

    /// With focus in the file view, `z` and `b` do the same thing.
    #[test]
    fn z_in_the_file_view_matches_b() {
        // Both apps must point at the exact same file: the file view's
        // border includes its full path as a title, so two different
        // fixture directories would make `rendered` disagree on the title
        // text alone, regardless of whether the zoom layouts truly match.
        let file = fixture_path("zoom_view_parity", "alpha\n");
        let config = Config {
            file: file.display().to_string(),
        };

        let mut with_z = App::new(&config);
        focus_file_view(&mut with_z);
        key(&mut with_z, KeyCode::Char('z'));

        let mut with_b = App::new(&config);
        key(&mut with_b, KeyCode::Char('b'));

        assert_eq!(rendered(&mut with_z), rendered(&mut with_b));
        // `rendered` only sees symbols, so two layouts that differ solely in
        // which pane is focused — a thicker border, a title glyph — could
        // still render identically today. These pin the claim the symbols
        // alone cannot: `z` and `b` leave the app in the exact same state,
        // not just looking the same.
        assert_eq!(with_z.active_widget, with_b.active_widget);
        assert_eq!(with_z.zoom, with_b.zoom);
    }

    #[test]
    fn z_toggles_back() {
        let mut app = app_over_file("zoom_z_back", "alpha\n");
        let before = rendered(&mut app);

        key(&mut app, KeyCode::Char('z'));
        key(&mut app, KeyCode::Char('z'));

        assert_eq!(rendered(&mut app), before);
    }

    /// A drag started on the divider must not survive into a zoom: there is
    /// no divider to drag while zoomed, but the `Drag` arm in
    /// `handle_divider` only checks `self.dragging`, so without
    /// `toggle_zoom` cancelling it, moving the mouse mid-drag would silently
    /// re-pin `nav_width` while nothing is drawn to explain why.
    #[test]
    fn zooming_mid_drag_cancels_the_drag() {
        let mut app = app_over_file("zoom_drag_cancel", "alpha\n");
        draw(&mut app);
        let divider = app.divider;
        let before = app.nav_width(AREA);

        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);
        assert!(app.dragging, "sanity: the divider click started a drag");

        key(&mut app, KeyCode::Char('b'));
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 60);

        assert!(!app.dragging, "the drag survived into the zoom");
        assert_eq!(
            app.nav_width(AREA),
            before,
            "nav_width changed from a drag that continued while zoomed"
        );
    }

    /// Tab while zoomed must not leave the cursor on an invisible pane: the
    /// zoom follows the focus.
    #[test]
    fn tab_while_zoomed_moves_the_zoom_with_the_focus() {
        let mut app = app_over_file("zoom_tab", "alpha\n");
        key(&mut app, KeyCode::Char('z'));

        focus_file_view(&mut app);

        assert_eq!(
            app.zoom,
            Some(app.active_widget),
            "focus moved off the zoomed pane"
        );
        assert!(
            rendered(&mut app).contains("alpha"),
            "the focused pane is not visible"
        );
    }

    /// The modifier guard: an earlier phase shipped a global key that swallowed
    /// a Ctrl- binding the file view needed.
    #[test]
    fn ctrl_modified_letters_still_reach_the_file_view() {
        let mut app = app_over_file("zoom_ctrl", "alpha\nbeta\n");
        focus_file_view(&mut app);

        for code in [KeyCode::Char('b'), KeyCode::Char('e'), KeyCode::Char('z')] {
            app.handle_event(event::Event::Key(event::KeyEvent::new(
                code,
                KeyModifiers::CONTROL,
            )))
            .unwrap();
            assert_eq!(app.zoom, None, "a Ctrl- key was taken as a zoom command");
        }
    }

    /// The status/prompt row is split off above the pane split and drawn
    /// below it, so a zoomed pane must not skip past that drawing.
    #[test]
    fn the_status_line_still_renders_while_zoomed() {
        let mut app = app_over_file("zoom_status", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('z'));

        let status = status_line(&mut app);
        assert!(
            status.contains("lines shown"),
            "expected filter status text while zoomed, got: {status}"
        );
    }

    /// The pane costs nothing until a filter exists.
    #[test]
    fn the_filter_pane_is_absent_until_a_filter_is_defined() {
        let mut app = app_over_file("pane_absent", "alpha\n");

        assert!(!rendered(&mut app).contains("Filters"));

        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        assert!(rendered(&mut app).contains("Filters"));
    }

    #[test]
    fn the_filter_pane_lists_the_patterns() {
        let mut app = app_over_file("pane_lists", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        // `.contains("alpha")` alone would also be satisfied by the file
        // view drawing the log's own one-line body (see
        // `b_hides_the_left_column`, which asserts exactly that over the
        // same body with no filter pane in play) — so this asserts a
        // substring only `FilterList::row_text` produces: the row's index,
        // enabled marker, sense and pattern together.
        assert!(rendered(&mut app).contains("1[x] inc alpha"));
    }

    /// Tab reaches the filter pane once it exists, and skips it before then.
    #[test]
    fn tab_skips_the_filter_pane_while_it_is_collapsed() {
        let mut app = app_over_file("pane_focus", "alpha\n");
        draw(&mut app);

        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);

        assert_eq!(
            app.active_widget,
            app.nav_index(),
            "focus did not return to the navigator"
        );
    }

    #[test]
    fn tab_reaches_the_filter_pane_once_a_filter_exists() {
        let mut app = app_over_file("pane_focus_on", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        let filter_index = app.filter_list_index();
        let mut seen = vec![app.active_widget];
        for _ in 0..3 {
            key(&mut app, KeyCode::Tab);
            seen.push(app.active_widget);
        }

        assert!(
            seen.contains(&filter_index),
            "the filter pane never took focus: {seen:?}"
        );
    }

    /// The README documents the exact order `Tab` cycles the panes in
    /// (navigator, file view, filter pane), which follows `self.widgets`'
    /// construction order in `App::new` rather than anything visual — the
    /// filter pane sits *above* the file view on screen but *after* it in
    /// the cycle. Nothing else pinned that order, so a reshuffle of
    /// `self.widgets` (plausible in a later phase that adds a fourth pane)
    /// would leave the README quietly wrong with an otherwise green suite.
    /// Asserting against the index accessors, rather than bare `0`/`1`/`2`
    /// literals, means a reordering breaks this test loudly instead of the
    /// assertions silently tracking whatever the new order happens to be.
    #[test]
    fn tab_cycles_navigator_then_file_view_then_filter_pane() {
        let mut app = app_over_file("tab_cycle_order", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        assert_eq!(
            app.active_widget,
            app.nav_index(),
            "should start focused on the navigator"
        );

        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.active_widget,
            app.file_view_index(),
            "one Tab from the navigator should reach the file view"
        );

        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.active_widget,
            app.filter_list_index(),
            "two Tabs from the navigator should reach the filter pane"
        );

        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.active_widget,
            app.nav_index(),
            "three Tabs should cycle back to the navigator"
        );
    }

    /// `App::render` has two branches that reach a widget's own `render`
    /// method — the ordinary split and the zoom special case — and only the
    /// split branch is exercised by the tests above. `AppWidget`'s own
    /// `Widget` impl deliberately renders nothing for the filter pane (see
    /// its comment): the real rendering happens in a helper that both
    /// branches of `App::render` must go through, or zooming the filter pane
    /// would focus an invisible pane showing a blank screen.
    #[test]
    fn zooming_the_filter_pane_shows_its_contents() {
        let mut app = app_over_file("zoom_filter_pane", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        // Bounded, like `focus_file_view`, rather than an unbounded `while`:
        // if the filter pane ever stopped being reachable, a `while` here
        // would hang the test instead of failing it.
        let filter_index = app.filter_list_index();
        for _ in 0..app.widgets.len() {
            if app.active_widget == filter_index {
                break;
            }
            key(&mut app, KeyCode::Tab);
        }
        assert_eq!(
            app.active_widget, filter_index,
            "could not reach the filter pane by tabbing"
        );
        key(&mut app, KeyCode::Char('z'));

        let after = rendered(&mut app);
        assert!(
            after.contains("Filters"),
            "the filter pane's border is not on screen: {after}"
        );
        // The full row, not a bare `contains("alpha")`: the file view (whose
        // gutter would also print a bare `alpha`-free line number) is not
        // drawn while zoomed, which is what let `contains("alpha")` alone
        // pass here — incidentally, not because it actually pinned the
        // filter pane's own content. `the_filter_pane_lists_the_patterns`
        // already caught and fixed this exact trap once.
        assert!(
            after.contains("1[x] inc alpha"),
            "the filter pattern's row is not on screen: {after}"
        );
    }

    /// The left column takes the wider of the navigator's and the filter
    /// pane's preferred widths — but a long filter pattern must not push it
    /// past `MAX_NAV_WIDTH` any more than a long file name already does.
    #[test]
    fn a_long_filter_pattern_does_not_push_the_column_past_the_cap() {
        let mut app = app_over_file("wide_filter", "alpha\n");
        let long_pattern = "a".repeat(200);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, &long_pattern);
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        assert_eq!(app.nav_width(AREA), MAX_NAV_WIDTH);
    }

    /// On a wide terminal `MAX_NAV_WIDTH` alone governs, exactly as before
    /// the filter pane existed — `MIN_FILE_VIEW_WIDTH` never binds here.
    /// Companion to `a_long_filter_pattern_does_not_push_the_column_past_the_cap`,
    /// but asserting the *other* bound is the one doing nothing, not just
    /// that the column stopped somewhere reasonable.
    #[test]
    fn on_a_wide_terminal_the_max_width_cap_governs_not_the_new_floor() {
        let mut app = app_over_file("wide_filter_floor_slack", "alpha\n");
        let long_pattern = "a".repeat(50);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, &long_pattern);
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        assert_eq!(
            app.nav_width(AREA),
            MAX_NAV_WIDTH,
            "MAX_NAV_WIDTH is not governing"
        );
        assert!(
            AREA.width - app.nav_width(AREA) > MIN_FILE_VIEW_WIDTH,
            "the file view is sitting at its floor on a wide terminal — the \
             floor, not MAX_NAV_WIDTH, is what's actually governing here"
        );
    }

    /// On a narrow terminal, a filter pattern that would otherwise want more
    /// than the terminal can spare must not starve the file view below its
    /// floor — asserted as the exact width, not merely "greater than zero".
    #[test]
    fn a_long_filter_pattern_on_a_narrow_terminal_leaves_the_file_view_its_floor() {
        let narrow = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 12,
        };
        let mut app = app_over_file("narrow_filter_floor", "alpha\n");
        let long_pattern = "a".repeat(50);
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, &long_pattern);
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        assert_eq!(
            narrow.width - app.nav_width(narrow),
            MIN_FILE_VIEW_WIDTH,
            "the file view lost its floor"
        );
    }

    /// If the terminal is too short for the filter pane's requested height,
    /// the navigator is squeezed to its floor (`MIN_NAV_HEIGHT`) so the
    /// filter pane — the pane the user is actively working with — gets a
    /// genuine content row rather than merely surviving as a title with
    /// nothing under it.
    ///
    /// The area here is picked so the filter pane gets exactly 3 rows (top
    /// border, one content row, bottom border): asserting on the actual row
    /// text `1[x] inc one`, not just `"Filters"` on the border, is the
    /// point — a one-row (title-only) or zero-row pane both contain
    /// `"Filters"` too (the title is drawn on the top border, which survives
    /// down to a single row), so that alone cannot tell a real, usable pane
    /// apart from a vanished one that still happens to have focus. That gap
    /// is exactly how a prior version of this test passed at 40×5 while a
    /// 40×4 terminal made the pane vanish entirely while still focused,
    /// silently routing keys (e.g. `d`) to content the user could not see.
    #[test]
    fn a_short_terminal_shows_a_real_filter_row_not_just_the_title() {
        let mut app = app_over_file("short_terminal", "alpha\n");
        for pattern in ["one", "two", "three", "four", "five", "six"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }

        // Status row: 1. Left column: 6 rows, split 3 (nav floor) / 3
        // (filter: still short of the 8 all six filters would need, so the
        // floors are still genuinely competing). Wide enough that the
        // filter pane's auto width comfortably fits a full row's text
        // rather than truncating it — this test is about the *height*
        // floor, so the width must not be the thing hiding the row.
        let short = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 7,
        };
        let mut buf = Buffer::empty(short);
        // Must not panic even though the filter pane alone wants more rows
        // (6 filters + 2 borders = 8) than the whole terminal has.
        (&mut app).render(short, &mut buf);

        let text: String = (0..short.height)
            .flat_map(|y| (0..short.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol())
            .collect();
        assert!(
            text.contains("1[x] inc one"),
            "the filter pane shrank to its title with no content row visible \
             while still focusable: {text}"
        );
    }

    /// `filter_pane_split_height` is the arithmetic Important 2 replaced a
    /// reliance on constraint-solver internals with. This drives it directly
    /// at a height where the two floors genuinely compete — the filter pane
    /// wants more than is available, and the navigator's floor is what
    /// limits how much of it can win — and asserts the exact split, matching
    /// the measured (not documented) behaviour of the constraint-solver
    /// version this replaced: `Min(3) + Length(8)` over 4 rows produced nav
    /// 3, filter 1.
    #[test]
    fn the_two_height_floors_compete_and_split_arithmetically() {
        let mut app = app_over_file("short_terminal_split", "alpha\n");
        for pattern in ["one", "two", "three", "four", "five", "six"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        // The filter pane wants 6 + 2 = 8 rows; only 4 are available for the
        // whole left column, so both floors are in play at once.
        let filter_height = app.filter_pane_split_height(4);

        assert_eq!(filter_height, 1, "the filter pane did not get its share");
        assert_eq!(
            4 - filter_height,
            MIN_NAV_HEIGHT,
            "the navigator did not keep exactly its floor"
        );
    }

    /// Before the cap in `filter_pane_split_height`, `preferred_height` grew
    /// without bound as filters were added — so a filter set that grows past
    /// a handful would pin the navigator at its bare floor *permanently* on
    /// any terminal, not only a genuinely short one. `List`/`ListState`
    /// already scrolls the pane, so nothing is lost by capping it.
    #[test]
    fn the_filter_pane_cannot_pin_the_navigator_at_its_bare_floor() {
        let mut app = app_over_file("many_filters_cap", "alpha\n");
        for i in 0..20 {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, &format!("f{i}"));
            key(&mut app, KeyCode::Enter);
        }

        // 20 filters want 22 rows. On a column with 20 rows to give, the
        // floor-only cap (`left_height - MIN_NAV_HEIGHT` = 17) would still
        // let the filter pane take all but the navigator's bare floor.
        let filter_height = app.filter_pane_split_height(20);

        assert!(
            filter_height <= 10,
            "the filter pane claimed more than half the column: {filter_height}"
        );
        assert!(
            20 - filter_height > MIN_NAV_HEIGHT,
            "the navigator was pinned at its bare floor despite ample room \
             (filter_height = {filter_height})"
        );
    }

    fn app_with_two_filters(name: &str) -> App<'static> {
        let mut app = app_over_file(name, "alpha\nbeta\ngamma\n");
        for pattern in ["alpha", "beta"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        app
    }

    #[test]
    fn space_toggles_the_selected_filter() {
        let mut app = app_with_two_filters("pane_toggle");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char(' '));

        assert!(!app.filters.filters()[0].enabled);
    }

    /// Toggling must re-evaluate: the view is what the pane is controlling.
    #[test]
    fn toggling_a_filter_restyles_the_view() {
        let mut app = app_with_two_filters("pane_toggle_view");
        focus_filter_pane(&mut app);
        let before = view_line_styles(&app);

        key(&mut app, KeyCode::Char(' '));

        assert_ne!(before, view_line_styles(&app), "the view did not follow");
    }

    #[test]
    fn d_deletes_the_selected_filter() {
        let mut app = app_with_two_filters("pane_delete");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('d'));

        assert_eq!(app.filters.len(), 1);
    }

    /// Important 1: the routing into the filter pane used to pass only
    /// `key.code`, discarding the modifiers — so every global binding's
    /// "no CONTROL/ALT" guard was silently bypassed once a key reached this
    /// pane. `Ctrl-D` is half-page-down in the file view, documented in the
    /// README, and exactly the muscle memory a vim user arrives with; here
    /// it used to delete the selected filter outright, with no confirmation
    /// and no undo.
    #[test]
    fn ctrl_d_does_not_delete_the_selected_filter() {
        let mut app = app_with_two_filters("pane_ctrl_d");
        focus_filter_pane(&mut app);

        app.handle_event(event::Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();

        assert_eq!(app.filters.len(), 2, "Ctrl-D deleted a filter");
    }

    /// Same defect, for the toggle binding.
    #[test]
    fn ctrl_space_does_not_toggle_the_selected_filter() {
        let mut app = app_with_two_filters("pane_ctrl_space");
        focus_filter_pane(&mut app);

        app.handle_event(event::Event::Key(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::CONTROL,
        )))
        .unwrap();

        assert!(
            app.filters.filters()[0].enabled,
            "Ctrl-Space toggled a filter"
        );
    }

    /// Finding 11: the filter pane has nothing to search over, so `/` (and
    /// `?`) must not even open the prompt while it has focus — opening it
    /// and then having `Enter` silently do nothing looked like the
    /// keystroke was simply dropped, with no feedback that anything was
    /// wrong.
    #[test]
    fn slash_does_nothing_while_the_filter_pane_is_focused() {
        let mut app = app_with_two_filters("filter_pane_slash");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('/'));

        assert!(app.search.is_none(), "a prompt opened over the filter pane");
    }

    /// Deleting renumbers the filters, so every cached verdict is stale.
    ///
    /// Two filters, deleting the first — this test's original form — cannot
    /// actually distinguish a full re-evaluate from a naive patch: on a
    /// naive patch (splice the `FilterSet` but leave cached verdicts alone),
    /// the stale `Verdict::Included(1)` left over from the deleted filter's
    /// own line would index *past* the now-length-1 `FilterSet` — `None`,
    /// not a collision — and the test would fail via `.expect()` panicking,
    /// before the colour assertion it advertises ever ran.
    ///
    /// Three filters, deleting the *middle* one ("beta"), produces a genuine
    /// in-range collision instead: beta's own filter is gone, so beta must
    /// read as unmatched (dim) once re-evaluated — but a naive patch leaves
    /// beta's stale `Verdict::Included(1)` in place, and after the splice,
    /// array position 1 is no longer beta's old filter — it is gamma's,
    /// shifted down from position 2. `style_for` finds a filter there
    /// (`.get(1)` succeeds), so the naive patch renders beta in *gamma's*
    /// colour: a real, in-range wrong-colour failure, not a panic.
    #[test]
    fn deleting_a_filter_re_evaluates_rather_than_patching() {
        let mut app = app_over_file("pane_delete_verdicts_mid", "alpha\nbeta\ngamma\n");
        for pattern in ["alpha", "beta", "gamma"] {
            key(&mut app, KeyCode::Char('f'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('j')); // move off "alpha" onto "beta", the middle filter

        key(&mut app, KeyCode::Char('d')); // removes "beta"; "gamma" becomes index 1

        let styles = view_line_styles(&app);

        // beta's own filter was just deleted, so beta must be unmatched —
        // plain dim, never coloured as if it still matched something. A
        // stale, un-re-evaluated verdict left over from beta's own
        // (deleted) filter would instead land on gamma's — see the doc
        // comment above — which is the collision this test exists to
        // catch. Checked first, and as a colour comparison rather than an
        // `.expect()`, so that exact failure surfaces directly instead of
        // being masked by a panic from the sanity check below.
        let beta = styles[1].map(|s| s.fg);
        assert_ne!(
            beta,
            Some(app.filters.filters()[1].style.fg),
            "beta is coloured with the wrong (gamma's) filter's style"
        );

        // gamma is unaffected in content and still matches its own filter,
        // now shifted down to index 1.
        let gamma = styles[2].expect("gamma still matches a filter");
        assert_eq!(
            gamma.fg,
            app.filters.filters()[1].style.fg,
            "gamma is not coloured with its own (shifted) filter's style"
        );
    }

    #[test]
    fn deleting_the_last_filter_collapses_the_pane_and_moves_focus() {
        let mut app = app_over_file("pane_delete_last", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('d'));
        draw(&mut app);

        assert!(app.filters.is_empty());
        assert!(!rendered(&mut app).contains("Filters"));
        assert!(
            !matches!(app.widgets[app.active_widget], AppWidget::FilterList(_)),
            "focus was left on a pane that is no longer on screen"
        );
    }

    /// Deleting the last filter while the pane is zoomed must not leave
    /// `App::zoom` naming a pane focus has moved off. `App::render` also
    /// carries `debug_assert_eq!(index, self.active_widget)` for exactly
    /// this invariant, but that macro compiles out entirely in `--release`
    /// — this test's assertion must catch the same defect on its own, not
    /// merely lean on the debug build to do it. The prior form of this test
    /// wrapped its assertion in `if let Some(index) = app.zoom`, which is
    /// vacuously true whenever the zoom is `None` — asserting the
    /// disjunction directly below closes that gap.
    #[test]
    fn deleting_the_last_filter_while_zoomed_keeps_the_zoom_invariant() {
        let mut app = app_over_file("pane_delete_last_zoomed", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('z'));
        assert_eq!(
            app.zoom,
            Some(app.active_widget),
            "z did not zoom the filter pane"
        );

        key(&mut app, KeyCode::Char('d'));

        assert!(
            app.zoom.is_none() || app.zoom == Some(app.active_widget),
            "zoom ({:?}) outlived the pane it named (active_widget = {})",
            app.zoom,
            app.active_widget
        );
        assert!(
            !matches!(app.widgets[app.active_widget], AppWidget::FilterList(_)),
            "focus (and so the zoom, if any) was left on the now-collapsed filter pane"
        );

        // The disjunction above is satisfiable by a blank frame that merely
        // avoids naming the wrong pane; this pins the stronger claim that
        // whatever the zoom now points at is genuinely drawn, not empty.
        let text = rendered(&mut app);
        assert!(
            text.contains("log.txt"),
            "the pane the zoom now names is not actually on screen: {text}"
        );
    }

    #[test]
    fn j_and_k_move_the_filter_selection() {
        let mut app = app_with_two_filters("pane_select");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char(' '));

        assert!(app.filters.filters()[0].enabled, "toggled the wrong filter");
        assert!(
            !app.filters.filters()[1].enabled,
            "j did not move the selection"
        );

        // `k` back up, then toggle again: if `k` did not move the selection
        // back to filter 0, this toggle would re-hit filter 1 instead — and
        // since filter 1 is already disabled, that would re-enable it rather
        // than disabling filter 0, so the assertions below would fail.
        key(&mut app, KeyCode::Char('k'));
        key(&mut app, KeyCode::Char(' '));

        assert!(
            !app.filters.filters()[0].enabled,
            "k did not move the selection back"
        );
        assert!(
            !app.filters.filters()[1].enabled,
            "the wrong filter was toggled after k"
        );
    }
}
