use color_eyre::Result;
use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::prelude::{Backend, Buffer, Color, Constraint, Layout, Rect, Style, Terminal, Widget};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Shown in the prompt when a pattern will not compile, after vim's error.
const INVALID_PATTERN: &str = "E486: invalid pattern";

/// The badge saying hide-unmatched-lines mode is armed, and the style that
/// makes it loud enough to notice mid-skim.
///
/// Six columns rather than a spelled-out `[HIDE MODE ON]`, on a row that also
/// has to carry a filter count and the directory on a narrow terminal. A
/// filled colour block reads louder than a glyph at a quarter of the width —
/// which is the point, since issue #36 rejected a dim status-bar icon by name.
///
/// Both are config-schema candidates for #18, text and style alike.
///
/// **Filled block means state; brackets or a border mean action.** That is the
/// established TUI idiom — vim's statusline mode indicator, tmux status
/// segments and powerline segments are all filled blocks nobody clicks, while
/// buttons get `[ OK ]` or `< Cancel >`. Mouse control is planned (click a
/// file to view it, click a filter to toggle it), so the rule is recorded here
/// rather than re-derived: anything painted like this badge is not clickable.
const HIDE_BADGE_TEXT: &str = " HIDE ";
const HIDE_BADGE_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::LightYellow)
    .add_modifier(ratatui::style::Modifier::BOLD);

/// What an open prompt will do with the pattern being typed.
///
/// The `Edit` variants are what makes a filter's pattern changeable at all:
/// before them the only way to correct one was `d` and a full retype, which
/// pushed the replacement to the end of the set and so changed its colour and
/// its precedence in `verdict`. Committing one overwrites in place instead.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    #[default]
    Search,
    Filter,
    Exclude,
    /// Replace the pattern of the numbered filter at `index`.
    ///
    /// `sense` is carried purely so the prompt can draw the right sigil — the
    /// filter's own sense is untouched by the edit. Without it an excluding
    /// filter would edit under a `filter:` prompt, reading as though
    /// committing were about to turn it into an including one.
    ///
    /// The index is captured when the prompt opens and could in principle name
    /// a filter that is gone by the time `Enter` arrives. It cannot today: the
    /// prompt consumes every key while it is open, so nothing can delete a
    /// filter in between. `replace_filter` handles the case anyway rather than
    /// resting on that.
    Edit {
        index: usize,
        sense: filter::Sense,
    },
    /// Replace the live search's pattern.
    ///
    /// Carries no index because the search does not have one — it lives in its
    /// own slot on the `ActiveFilters`, which is the whole reason `/` does not
    /// renumber the filters the user built.
    EditSearch,
}

/// A search pattern being typed at the bottom of the screen.
#[derive(Debug, Default)]
struct SearchPrompt {
    pattern: String,
    error: Option<String>,
    kind: PromptKind,
}

impl SearchPrompt {
    /// The prefix the prompt draws, which names what committing will do.
    ///
    /// An edit shows the same sigil as the `i`, `x` or `/` that would have
    /// created the thing being edited, because it produces the same kind of
    /// thing. What differs is where it lands, and the pre-filled pattern
    /// already says that: a prompt that opens with text in it is editing
    /// something, and one that opens empty is making something new.
    fn sigil(&self) -> &'static str {
        match self.kind {
            PromptKind::Search | PromptKind::EditSearch => "/",
            PromptKind::Filter
            | PromptKind::Edit {
                sense: filter::Sense::Include,
                ..
            } => "filter: ",
            PromptKind::Exclude
            | PromptKind::Edit {
                sense: filter::Sense::Exclude,
                ..
            } => "exclude: ",
        }
    }

    /// What the bottom line shows: the error if the pattern was rejected,
    /// otherwise the pattern being typed behind its sigil.
    fn line(&self) -> String {
        match &self.error {
            Some(error) => error.clone(),
            None => format!("{}{}", self.sigil(), self.pattern),
        }
    }
}

pub mod config;
pub mod document;
pub mod editor;
pub mod filter;
pub mod help;
mod layout;
mod path;
mod viewport;
mod widgets;
pub use config::Config;
// Imported rather than left qualified: `App`'s own fields are typed with these
// three, so every mention of them in the struct and in `render` would otherwise
// need a `layout::` prefix. The pane-geometry constants are *not* re-exported
// here — nothing outside `layout` reads them any more except the tests, which
// import them directly (#74).
use document::{Document, Mode};
use editor::Launcher;
use filter::ActiveFilters;
use layout::{Divider, FilterHeight, NavWidth};
use widgets::filenav::FileNav;
use widgets::fileview::FileView;
use widgets::filterlist::FilterList;
use widgets::{Action, FilterCommand, Focus};

#[derive(Default)]
pub struct App<'a> {
    state: AppState,
    /// The three panes, named rather than collected (#73).
    ///
    /// They were a `Vec<AppWidget>` built once with exactly three entries,
    /// never pushed to or popped from, whose length `render` asserted on every
    /// frame. Because each position was untyped, `App` could not say "the file
    /// view" — it had to search for it, which twenty call sites did, each
    /// paying a linear scan and an `unwrap_or` fallback for a case that could
    /// not happen. Named fields make the invariant unrepresentable instead of
    /// merely checked, and the scans become field reads.
    nav: FileNav<'a>,
    view: FileView<'a>,
    filters_pane: FilterList,
    /// Which pane has focus, replacing an index into the old vec.
    focus: Focus,
    nav_width: NavWidth,
    /// Boundary column from the last render, for hit-testing mouse events that
    /// arrive before the next frame.
    divider: u16,
    filter_height: FilterHeight,
    /// The filter pane's rectangle from the last render.
    ///
    /// Its top edge is the horizontal divider — the counterpart of `divider`
    /// above, and hit-tested the same way, against the last frame. Its bottom
    /// edge is kept too, because that is what turns a dragged row into a
    /// height: the pane runs from the boundary the mouse is holding down to
    /// the bottom of the left column, and only a render knows where that is.
    filter_area: Rect,
    dragging: Option<Divider>,
    /// The last divider click, and which divider it was on.
    ///
    /// The axis is part of the record, not decoration: without it, a click on
    /// one divider followed quickly by a click on the other reads as a
    /// double-click and resets a pane the user never aimed at.
    last_divider_click: Option<(Divider, Instant)>,
    /// Open while a search pattern is being typed.
    search: Option<SearchPrompt>,
    filters: ActiveFilters,
    document: Document,
    /// Source line indices the file view's buffer was last rebuilt from, or
    /// `None` when what the buffer holds is unknown and a rebuild is owed
    /// unconditionally.
    ///
    /// `refresh_view` rebuilds only when the new visible set differs from
    /// this, so a filter change that leaves the same rows on screen does not
    /// reset the viewport's scroll position.
    ///
    /// `Option`, not a plain empty `Vec`: `sync_document` has to say "this
    /// buffer belongs to a document that no longer exists, rebuild whatever
    /// happens next", and an empty `Vec` cannot say that. An empty *visible
    /// set* is a real, reachable state — every line filtered away — and it
    /// compares equal to an empty `Vec`, so the guard read "nothing changed"
    /// and left the previous file's buffer on screen. `None` is unequal to
    /// every `Some`, so the rebuild always happens.
    last_visible: Option<Vec<usize>>,
    /// The window bounds `apply_view` last handed the file view, paired with
    /// `last_visible` as the rebuild-skip key (#7).
    ///
    /// The visible set alone is no longer enough to decide a rebuild can be
    /// skipped: scrolling into a new window leaves the visible set untouched
    /// and still needs the buffer replaced. Omitting this is the subtlest bug
    /// available here — the view would silently go on showing the old rows.
    last_window: Option<(usize, usize)>,
    /// The single pane filling the screen, or `None` for the normal split.
    ///
    /// Hiding the left column and maximising the file view are the same thing,
    /// so they share this one field. Two separate flags could disagree.
    zoom: Option<Focus>,
    /// Both editor command templates, resolved once at startup.
    ///
    /// Resolved at startup but *split* per keypress, so a typo in one template
    /// is reported by the key that uses it rather than refusing to start a log
    /// viewer over a setting most sessions never touch.
    editor: editor::Templates,
    /// How `o` actually starts an editor.
    ///
    /// Boxed behind the trait so tests can swap in a double that records the
    /// argv it would have run — there is no way to launch a real editor in CI,
    /// and the recorded command is the entire testable surface of "spawn".
    launcher: Box<dyn Launcher>,
    /// A transient message owning the status row until the next event.
    ///
    /// The row is otherwise derived purely from filter state and has nowhere to
    /// put "that editor is not installed". Transient rather than dismissible:
    /// it is a report, not a dialog, and anything the user does next clears it.
    status_message: Option<StatusMessage>,
    /// Where an editor that has *exited* non-zero reports itself.
    ///
    /// Out of band because the failure arrives long after the keypress —
    /// `spawn` only says the process started. Drained on the render loop, which
    /// already wakes 60 times a second.
    editor_outcomes: Option<std::sync::mpsc::Receiver<String>>,
    /// What `<space>` captured on its way into a peek, or `None` when not
    /// peeking (#48).
    ///
    /// Its presence *is* the peek flag — a separate `bool` could disagree with
    /// it, and the one thing this feature promises is that the second press
    /// puts back exactly what the first took away.
    peek: Option<PeekState>,
    /// Set when a prompt commits, so the `Enter` that committed it cannot also
    /// toggle the filter under the cursor (#48).
    ///
    /// Cleared by any key that is not `Enter`, and consumed by the first one
    /// that is. Deliberately event-counted rather than timed: a keypress count
    /// is exactly reproducible in a test, where "within 200ms" is not, and the
    /// two only disagree when a user deliberately presses `Enter` twice in a
    /// row — which costs them one extra press and is indistinguishable from
    /// the bounce anyway.
    swallow_next_enter: bool,
    /// Whether the keymap overlay is covering the panes (#25).
    ///
    /// A plain flag rather than a fourth pane: the three panes are persistent
    /// and reachable with `Tab`, and help is transient and dismissed by the
    /// next key. Joining that cycle would mean tabbing past help forever after
    /// using it once.
    help: bool,
}

/// What a peek has to put back when it ends (#48).
///
/// The mode *and* the filter flags, because `<space>` changes both: it is the
/// four-key "hide off, filters off, read, filters on, hide on" cycle from the
/// issue collapsed into one key and its undo.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PeekState {
    mode: Mode,
    flags: filter::EnabledFlags,
}

/// A one-off message shown on the status row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusMessage {
    text: String,
    /// Drawn red. Not derivable from the text — "opened src/lib.rs" and
    /// "zed: No such file or directory" are the same shape.
    error: bool,
}

/// Which of the two editor bindings is being carried out.
///
/// One enum rather than two methods: template resolution, argv splitting,
/// substitution, spawning and error reporting are identical for both keys, so
/// `O` costs one key and one match arm rather than a second copy of
/// `open_in_editor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorScope {
    /// `o` — walk up to the enclosing project.
    Project,
    /// `O` — the file alone, no walk-up.
    File,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum AppState {
    #[default]
    Running, // The app is running
    Quit, // The user has requested the app to quit
}

impl App<'_> {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        let argument = std::path::Path::new(&config.path);
        let nav = FileNav::new(config.path.clone());
        let mut view = FileView::default();

        match nav.selected_path() {
            // A directory argument *selects* an entry rather than being
            // handed one, so it is previewed — bounded by `PREVIEW_LINES` —
            // exactly as arrowing onto it would be. Loading it in full would
            // read a whole log at startup merely because it sorts first,
            // which is the cost the preview mechanism exists to avoid.
            Some(selected) if argument.is_dir() => view.preview(&selected),
            // A file argument loads the argument itself, not the navigator's
            // selection. They are the same path when the file exists; when it
            // does not, the navigator falls back to the first entry, and
            // loading *that* would silently open some other file in response
            // to a typo. Reporting the argument is what recon already does.
            _ => view.load(argument),
        }

        // Created here rather than lazily on the first `o`: the sender has to
        // outlive every launcher clone, and a channel built on demand would
        // need a second field to remember whether it already existed.
        let (outcomes_tx, outcomes_rx) = std::sync::mpsc::channel();

        let mut app = Self {
            state: AppState::Running,
            nav,
            view,
            filters_pane: FilterList::default(),
            focus: Focus::Nav,
            nav_width: NavWidth::Auto,
            divider: 0,
            filter_height: FilterHeight::Auto,
            filter_area: Rect::ZERO,
            dragging: None,
            last_divider_click: None,
            search: None,
            filters: match &config.filter_palette {
                Some(palette) => ActiveFilters::with_palette(palette.clone()),
                None => ActiveFilters::new(),
            },
            document: Document::default(),
            last_visible: None,
            last_window: None,
            zoom: None,
            editor: config.editor_templates(),
            launcher: Box::new(editor::ProcessLauncher::new(outcomes_tx)),
            status_message: None,
            editor_outcomes: Some(outcomes_rx),
            peek: None,
            swallow_next_enter: false,
            help: false,
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
                let (pattern, kind) = (prompt.pattern.clone(), prompt.kind);

                let outcome = match kind {
                    PromptKind::Search => self.run_search(&pattern),
                    PromptKind::Filter => self.add_filter(&pattern),
                    PromptKind::Exclude => self.add_excluding_filter(&pattern),
                    PromptKind::Edit { index, .. } => self.replace_filter(index, &pattern),
                    // Straight to `apply_search`, deliberately not through
                    // `run_search`: that dispatches on the *focused* pane, and
                    // the filter pane — the only pane this prompt can be
                    // opened from — has an arm there that does nothing at all.
                    // Routing through it would discard the pattern silently.
                    PromptKind::EditSearch => self.apply_search(&pattern),
                };
                if outcome.is_ok() {
                    self.search = None;
                    // Arm the bounce guard (#48). Only on the branch that
                    // actually closes the prompt: a rejected pattern leaves it
                    // open, so the next `Enter` is another commit attempt and
                    // never reaches the filter pane to be swallowed.
                    self.swallow_next_enter = true;
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

    /// Run a committed `/` pattern against whichever pane has focus.
    ///
    /// The navigator has its own search over filenames and keeps it. In the
    /// file view, `/` now sets the live search *filter* — the pane has no
    /// search of its own any more.
    fn run_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let mut view_search = false;
        let action = match self.focus {
            Focus::Nav => self.nav.search(pattern, false)?,
            Focus::View => {
                // Deferred rather than done here: setting the filter needs
                // `&mut self` for `refresh_view`, and the borrow taken to
                // reach the pane is still live.
                view_search = true;
                None
            }
            // Unreachable in practice: `/` is not opened at all while the
            // filter pane has focus (see its guard in `handle_event`), and the
            // prompt swallows every key including `Tab` while it is open.
            //
            // The pane *can* open a search prompt — `c` on the search row —
            // but that commits through `apply_search` rather than here,
            // precisely because this arm does nothing. Routing it through this
            // function would swallow the edited pattern in silence.
            Focus::Filters => None,
        };

        if let Some(action) = action {
            self.perform(action);
        }
        if view_search {
            self.apply_search(pattern)?;
        }
        Ok(())
    }

    /// Set the live search filter and move to its first hit.
    ///
    /// Defined as "set it, then do exactly what `n` does" — which includes
    /// the truncated-preview promotion `n` performs before stepping (see
    /// `promote_truncated_preview`). Without that, a pattern that only
    /// matches beyond a large preview's cap would evaluate against the
    /// preview alone: the cursor would not move, and the status line would
    /// read as "no matches" over a file that in fact has one.
    ///
    /// One movement path rather than two, and the buffer rebuild that adding
    /// a filter triggers in `Mode::FilteredOnly` is completed by
    /// `refresh_view` before anything moves a cursor through it.
    ///
    /// A pattern that will not compile is reported and changes nothing, so the
    /// prompt can stay open over an intact previous search.
    fn apply_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.set_search(pattern)?;
        self.promote_truncated_preview();
        self.refresh_view();
        self.step_to_interesting(false);
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

    /// Overwrite one filter's pattern, keeping its position — and with it the
    /// colour and the precedence that position decides.
    ///
    /// `refresh_view`'s full `Document::evaluate`, not `recompute_visible`:
    /// the pattern is what decides which lines match, so the cached verdicts
    /// are stale in a way only a re-evaluate can fix. Narrower than a delete —
    /// the numbering is untouched, so only *this* filter's verdicts can have
    /// changed — but `evaluate` is the only thing that recomputes any of them.
    ///
    /// A filter that has vanished is reported as `Ok` rather than an error:
    /// the pattern the user typed is fine, there is simply nothing left to put
    /// it on, and leaving the prompt open under `E486: invalid pattern` would
    /// blame the pattern for it. Unreachable today — see `PromptKind::Edit`.
    fn replace_filter(&mut self, index: usize, pattern: &str) -> Result<(), regex::Error> {
        if self.filters.set_pattern(index, pattern)? {
            self.refresh_view();
        }
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

    /// `<space>`: show the plain file, or put the filtered view back (#48).
    ///
    /// The issue's complaint is a four-key cycle — leave hide mode, clear the
    /// filters, read the code, then undo both — repeated at every match. This
    /// is that cycle as one key and its own undo.
    ///
    /// **A flip, not a destination** (#65). The mode toggles, which is what #48
    /// asked for in as many words; ending the peek restores what was captured.
    ///
    /// This was originally written the other way — forcing `Mode::Dimmed` —
    /// on the premise that flipping *into* `FilteredOnly` with every filter
    /// just disabled would show only `Included` lines and blank the pane. That
    /// premise was already false when it was written: `recompute_visible`'s #36
    /// guard makes `FilteredOnly` show the whole file when nothing is
    /// including. Recorded because the mistake is easy to make twice, and the
    /// arm below looks wrong until you know about the guard.
    ///
    /// It rests on what hide mode *means*, which is not "hide every unmatched
    /// line" but:
    ///
    /// > if something is including, hide unmatched lines; if nothing is, show
    /// > everything.
    ///
    /// So hiding is a standing preference — armed or not — rather than a
    /// description of what is currently on screen. That is why the ` HIDE `
    /// badge appearing over a plain, unfiltered file is honest rather than a
    /// lie: see `HIDE_BADGE_TEXT`, whose doc already says *armed*.
    ///
    /// The rendered lines are identical either way, which is precisely why the
    /// original deviation from #48 went unnoticed for so long. The badge is the
    /// only visible difference.
    ///
    /// The capture is held here rather than in `ActiveFilters::remembered`,
    /// which `!` owns — see `enabled_flags` for why sharing one slot loses the
    /// other feature's undo.
    fn toggle_peek(&mut self) {
        if let Some(peek) = self.peek.take() {
            self.filters.apply_enabled_flags(&peek.flags);
            self.document.set_mode(peek.mode);
        } else {
            self.peek = Some(PeekState {
                mode: self.document.mode(),
                flags: self.filters.enabled_flags(),
            });
            self.filters.set_all_enabled(false);
            // The same flip `toggle_hiding` does, deliberately: `<space>`
            // and `Ctrl-H` move the mode identically, and only the filter
            // switching below is the peek's own.
            self.document.set_mode(match self.document.mode() {
                Mode::Dimmed => Mode::FilteredOnly,
                Mode::FilteredOnly => Mode::Dimmed,
            });
        }
        // The full `evaluate`, not `recompute_visible` as `toggle_hiding` uses:
        // the enabled flags changed, so every line's verdict can differ. The
        // mode moved too, which `refresh_view` picks up on the same pass.
        self.refresh_view();
    }

    /// Whether the file view is showing a bounded preview rather than the
    /// whole file.
    fn file_view_truncated(&self) -> bool {
        self.view.truncated
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
        // Here rather than in `handle_event`: an editor exits on its own
        // schedule, so nothing the user does is guaranteed to arrive after it.
        // This loop already wakes 60 times a second whether or not anything
        // happened, which is exactly the property the report needs.
        self.drain_editor_outcomes();
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
        // The status message lasts until the next *keypress*, and deliberately
        // not until the next event: mouse capture is on, so a mouse moving
        // across the terminal would wipe "zed: No such file or directory" off
        // the row before it could be read.
        if matches!(event, event::Event::Key(_)) {
            self.status_message = None;
        }

        // An open prompt takes precedence over every other binding.
        if self.search.is_some() {
            if let event::Event::Key(key) = event {
                self.handle_search_key(key);
            }
            return Ok(());
        }

        // The overlay is dismissed by the next key, and that key does nothing
        // else — it is a reference you glance at and put away, so anything that
        // required aiming at a particular key to close it would be one more
        // thing to have read the README to know (#25).
        //
        // After the prompt guard, so `?` inside a prompt is typed rather than
        // acted on, and before every other binding, so the dismissing key
        // cannot also quit, move a cursor, or open an editor.
        //
        // Only a *key* closes it. Mouse capture is on, so a mouse crossing the
        // terminal would otherwise wipe the overlay mid-read — the same
        // reasoning `status_message` gets above.
        if self.help {
            if matches!(event, event::Event::Key(_)) {
                self.help = false;
            }
            return Ok(());
        }

        // The bounce guard (#48). `Enter` both commits a prompt and toggles a
        // filter, and those are one keystroke apart, so the `Enter` that closed
        // a prompt must not fall through and switch a filter off.
        //
        // Placed after the prompt guard so it can only ever see the keypress
        // *following* the commit, and before every binding so no pane can act
        // on the swallowed key. Any other key means the user is still working
        // and the next `Enter` is meant.
        if let event::Event::Key(key) = event
            && std::mem::take(&mut self.swallow_next_enter)
            && key.code == KeyCode::Enter
        {
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
                // `?` and not `F1`: it is the conventional help key in every
                // pager and file manager this app borrows from, and it is free
                // — it used to run a backward search, which `n`/`N` cover from
                // both directions now.
                //
                // Not guarded on `.is_empty()`: `?` is Shift-`/` on most
                // layouts, and crossterm reports the modifier, so that guard
                // would make the key unreachable outside a test harness. This
                // is the same trap `O` and `n`/`N` document.
                KeyCode::Char('?')
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.help = true;
                    return Ok(());
                }
                // The filter pane has nothing to search over, so the prompt
                // is not opened at all while it has focus — opening it and
                // then having `Enter` silently do nothing looked like the
                // keystroke was simply swallowed.
                //
                // `?` used to open a backward search here. `n`/`N` cover both
                // directions now, so it is unbound and reserved for the help
                // view (#25).
                KeyCode::Char('/') if key.modifiers.is_empty() && self.focus != Focus::Filters => {
                    self.search = Some(SearchPrompt::default());
                    return Ok(());
                }
                // Global, like `!`: the search is app state, not the file
                // view's, and having to focus a particular pane to switch it
                // off would make it easy to leave one running by accident.
                //
                // This departs from vim, where Esc leaves the pattern alone.
                // Here the search is a filter, and a filter that cannot be
                // turned off is a leak — one typed ten minutes ago would keep
                // changing what is on screen with nothing to stop it.
                //
                // An open prompt already took this key: `handle_event` returns
                // early while `self.search` is `Some`, so Esc there cancels the
                // prompt rather than reaching past it.
                //
                // Guarded on `.is_empty()`, unlike `n`/`N` below: that guard was
                // wrong for them because crossterm attaches SHIFT to every
                // uppercase letter a real terminal sends. Esc isn't a
                // printable character, so there is no case to attach SHIFT to;
                // in the legacy key-reporting mode this app runs in (no
                // keyboard-enhancement flags — see `main.rs`), a bare Esc byte
                // carries no modifiers at all, so `is_empty()` is simply
                // correct here rather than a trap.
                KeyCode::Esc if key.modifiers.is_empty() => {
                    // `clear_search` reports whether there was one to drop, the
                    // same shape `p`'s `promote_search` guard uses just below:
                    // `refresh_view` is not free — `evaluate` is
                    // O(lines × filters) — and Esc is a key people tap out of
                    // habit, so it should not pay for a re-evaluate when there
                    // was nothing to clear.
                    if self.filters.clear_search() {
                        self.refresh_view();
                    }
                    return Ok(());
                }
                // Global rather than pane-scoped: the user has just searched
                // and should not have to go and find the filter pane to keep
                // the result.
                //
                // Guarded on `.is_empty()`, same reasoning as `q`/`f`/`!`
                // above rather than `n`/`N`/`H`: `p` is lowercase, and
                // crossterm only attaches SHIFT to uppercase characters, so
                // there is no real-terminal case this guard would make
                // unreachable. Leaving Ctrl-P and Alt-P unclaimed here lets
                // them fall through to the focused widget, matching every
                // other plain-letter global binding on this branch.
                KeyCode::Char('p') if key.modifiers.is_empty() => {
                    // `promote_search` pays nothing when the slot is empty;
                    // `refresh_view` is not free — `evaluate` is
                    // O(lines × filters) — so it is only paid for when the
                    // set actually changed. `p` will be pressed speculatively.
                    if self.filters.promote_search() {
                        self.refresh_view();
                    }
                    return Ok(());
                }
                // `f` moves focus; creating a filter is `i` / `x` once the
                // pane has it. That costs a keystroke from outside the pane
                // and none from inside, in exchange for one focus key per
                // pane — see `handle_filter_key`, which owns `i` and `x`
                // because opening a prompt is `App`'s to do.
                KeyCode::Char('f') if key.modifiers.is_empty() => {
                    self.reveal_and_focus(Focus::Filters);
                    return Ok(());
                }
                // Global, and claimed above every pane rather than in any of
                // them (#48). The whole point of the key is that it means one
                // thing everywhere: a `<space>` that toggled hide mode in two
                // panes and a filter in the third is the pane-dependent meaning
                // this replaced, and recovering from the wrong one cost seconds
                // every time.
                //
                // That is also why the filter pane gave `space` up for `Enter`
                // rather than keeping both — see `FilterList::handle_key`.
                KeyCode::Char(' ') if key.modifiers.is_empty() => {
                    self.toggle_peek();
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
                // Scoped to the file view rather than global: `n` in the
                // navigator is the navigator's key, and hoisting the binding
                // up here to reach the verdicts must not change that.
                //
                // Not guarded with `.is_empty()`, unlike `/` above: crossterm
                // attaches SHIFT to every uppercase character a real terminal
                // sends, so an `is_empty()` guard would make `N` unreachable
                // outside a test harness that never sets it. CONTROL/ALT is
                // the same tolerance `H` uses just below, for the same reason.
                KeyCode::Char(c @ ('n' | 'N'))
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && self.focus == Focus::View =>
                {
                    // `n`/`N` bypass the widget's own `handle_events`, which is
                    // where a truncated preview normally promotes itself on
                    // first interaction — see `promote_truncated_preview`,
                    // which `apply_search` also calls for the same reason.
                    self.promote_truncated_preview();
                    self.step_to_interesting(c == 'N');
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
                    self.reveal_and_focus(Focus::Nav);
                    return Ok(());
                }
                KeyCode::Char('t') if key.modifiers.is_empty() => {
                    self.reveal_and_focus(Focus::View);
                    return Ok(());
                }
                KeyCode::Char('z') if key.modifiers.is_empty() => {
                    self.zoom_focused();
                    return Ok(());
                }
                // Global rather than pane-scoped, like `!` and `p`: the file
                // the user means is whatever the file view is showing, and
                // that follows the navigator's selection already. Having to
                // focus a particular pane first would make `o` fail in the one
                // place it is most natural to press — while browsing the
                // navigator.
                //
                // Guarded on `.is_empty()`, same reasoning as `q`/`f`/`p`: `o`
                // is lowercase, so crossterm never attaches SHIFT to it, and
                // leaving Ctrl-O and Alt-O unclaimed lets them fall through to
                // the focused widget.
                KeyCode::Char('o') if key.modifiers.is_empty() => {
                    let template = self.editor.project.clone();
                    self.open_in_editor(&template, EditorScope::Project);
                    return Ok(());
                }
                // `O` (#41): the same method, the other template, no walk-up.
                // Everything that makes the key work — resolution, splitting,
                // substitution, spawning, error reporting — is already shared,
                // so the sibling really does cost one arm.
                //
                // Not guarded on `.is_empty()`, unlike `o` directly above:
                // crossterm attaches SHIFT to every uppercase character a real
                // terminal sends, so that guard would make `O` unreachable
                // outside a test harness. Excluding CONTROL/ALT instead is the
                // convention `H` and `n`/`N` already follow, and it keeps
                // Ctrl-O and Alt-O falling through exactly as before.
                KeyCode::Char('O')
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    let template = self.editor.file.clone();
                    self.open_in_editor(&template, EditorScope::File);
                    return Ok(());
                }
                _ => {}
            }
        }

        if let event::Event::Mouse(mouse) = event
            && self.handle_divider(mouse)
        {
            return Ok(());
        }

        // Filter pane keys are routed here rather than through the generic
        // `handle_events` dispatch below: applying them means mutating the
        // `ActiveFilters`, which only `App` owns, so `FilterList` cannot carry
        // them out itself — see `handle_filter_key`.
        if let event::Event::Key(key) = event
            && self.focus == Focus::Filters
        {
            self.handle_filter_key(key);
            return Ok(());
        }

        // Four file-view keys move the cursor further than a page and so
        // cannot be left to the widget: `TextArea` holds a window of the
        // visible set, and each of these means "the *document's* top" or "the
        // next paragraph anywhere", not "the top of the buffer that happens to
        // be loaded" (#7). Intercepted here, resolved against the whole visible
        // set, and applied through `place_cursor_on_visible_row`, which brings
        // the window with it.
        //
        // Same shape as `n`/`N` above, and for the same reason: only `App` can
        // see the document. The cost is that the file view's bindings now live
        // in two places, which is the drift #25 is about — hence one table,
        // here, rather than four scattered arms.
        if let event::Event::Key(key) = event
            && key.modifiers.is_empty()
            && self.focus == Focus::View
            && let Some(target) = self.long_range_target(key.code)
        {
            self.promote_truncated_preview();
            self.place_cursor_on_visible_row(target);
            return Ok(());
        }

        // The file view upgrades its own truncated preview to a full load on
        // first interaction, which rebuilds the textarea and clears its line
        // styles. That happens inside the widget, so it never reaches
        // `perform` — resync here instead, without re-reading the file.
        let was_truncated = self.file_view_truncated();
        let action = match self.focus {
            Focus::Nav => self.nav.handle_events(event)?,
            Focus::View => {
                self.view.handle_events(event.into())?;
                None
            }
            // Unreachable: filter-pane keys returned above, through
            // `handle_filter_key`. Applying them means mutating the
            // `ActiveFilters`, and the pane only ever borrows one, so it
            // cannot carry out its own commands.
            Focus::Filters => None,
        };
        if let Some(action) = action {
            self.perform(action);
        } else if was_truncated && !self.file_view_truncated() {
            self.sync_document();
            self.refresh_view();
        }
        // Ordinary movement stays inside the window by design, but a page at
        // the edge of the middle third does not — see `window_holds`.
        self.ensure_window();
        Ok(())
    }

    /// Force the file view's truncated preview to a full load, the same
    /// thing its own `handle_events` does on first interaction. Needed by
    /// every path that moves the cursor or evaluates a pattern directly
    /// rather than going through that dispatch — see `promote_truncated_preview`,
    /// which wraps this for those callers.
    fn promote_file_view(&mut self) {
        let path = std::path::Path::new(&self.view.filename).to_path_buf();
        self.view.load(&path);
    }

    /// Promote a truncated preview to a full load and bring the document up
    /// to date with it. No-op when the preview is not truncated.
    ///
    /// `n`/`N` and a committed `/` both bypass `FileView::handle_events`,
    /// which is where a truncated preview normally promotes itself on first
    /// interaction — one moves the cursor directly, the other evaluates a
    /// pattern via `apply_search`, and neither goes through that dispatch.
    /// Without this, either would silently act on the bounded preview alone:
    /// `n` would wrap inside it forever, and `/` would report "no matches"
    /// for a pattern that only occurs past the preview's cap.
    ///
    /// `refresh_view` is folded in here, guarded the same way, since a
    /// promotion is pointless without the document catching up to the newly
    /// loaded lines before anything steps a cursor through them.
    fn promote_truncated_preview(&mut self) {
        if self.file_view_truncated() {
            self.promote_file_view();
            self.sync_document();
            self.refresh_view();
        }
    }

    /// Hand the selected file to an editor.
    ///
    /// Every step after the walk-up is shared by both bindings, which is what
    /// made `O` one key and one `match` arm rather than a second copy of this:
    /// `scope` decides whether to climb, and the template decides what the
    /// command looks like. Nothing else differs.
    ///
    /// Failures are reported on the status row and swallowed. recon is a
    /// viewer; a missing editor is not a reason to bring the TUI down over a
    /// key the user may have pressed by accident.
    fn open_in_editor(&mut self, template: &str, scope: EditorScope) {
        let name = self.view.filename.clone();
        if name.is_empty() {
            self.report("nothing to open", true);
            return;
        }

        // `filename` is set even when the read failed — the pane shows the
        // error in place of the file's text — so a path that is not there is
        // the ordinary "the argument was a typo" case, not an impossible one.
        let relative = std::path::Path::new(&name);
        if !relative.exists() {
            self.report(&format!("cannot open {name}: no such file"), true);
            return;
        }

        // Absolute, per the `{file}` contract. recon's working directory is not
        // the editor's — a GUI editor launched from a dock or a launcher agent
        // inherits neither — so a relative path is the one input guaranteed to
        // be interpreted differently at the far end.
        //
        // `lexical_absolute` rather than `canonicalize`: it does not touch the
        // filesystem and does not resolve symlinks, so the editor opens the
        // path the navigator is showing rather than wherever it happens to
        // point. For a file reached through a symlinked directory, that is the
        // one the user can find their way back to.
        //
        // That claim was false until #78. `FileNav::set_dir` canonicalized, so
        // the navigator had *already* resolved the link before this ran and
        // there was nothing left here to preserve. All three sites share this
        // one function now, which is what makes the sentence above true.
        let file = path::lexical_absolute(relative);

        let project = match scope {
            EditorScope::Project => editor::project_root(&file),
            // No walk-up. `O` exists precisely for `~/.zshrc` kept inside a
            // dotfiles repo, where climbing would fling open the whole repo —
            // and the file template has no `{project}` in it to receive this
            // anyway, so it is only ever a fallback for a hand-written one.
            EditorScope::File => file.parent().unwrap_or(&file).to_path_buf(),
        };

        // 1-based: `cursor_source` indexes the document's lines, and every
        // editor's `:line` argument counts from one.
        let line = self.cursor_source() + 1;
        let argv = match editor::editor_command(template, &project, &file, line) {
            Ok(argv) => argv,
            // A template error is the user's typo in a setting, and it can only
            // be reported here: it is not caught at startup, precisely so a bad
            // template does not stop recon opening a log.
            Err(err) => {
                self.report(&err.to_string(), true);
                return;
            }
        };

        // `editor_command` rejects an empty template, so there is always a
        // program — read defensively anyway rather than indexing, since this
        // runs inside a TUI where a panic takes the terminal with it.
        let program = argv.first().cloned().unwrap_or_default();
        match self.launcher.spawn(&argv) {
            // Reported rather than silent: a GUI editor can take seconds to
            // raise a window, and a key that appears to have done nothing is a
            // key that gets pressed again.
            Ok(()) => self.report(&format!("{program}: opening {}", file.display()), false),
            Err(err) => self.report(&format!("{program}: {err}"), true),
        }
    }

    /// Put a one-off message on the status row, replacing any previous one.
    fn report(&mut self, text: &str, error: bool) {
        self.status_message = Some(StatusMessage {
            text: text.to_string(),
            error,
        });
    }

    /// Move any editor exit reports onto the status row.
    ///
    /// `try_recv` in a loop, never `recv`: this runs on the render loop and
    /// must not block. Only the last message survives — the row holds one line,
    /// and the most recent failure is the one the user is still wondering
    /// about.
    fn drain_editor_outcomes(&mut self) {
        let Some(outcomes) = self.editor_outcomes.as_ref() else {
            return;
        };
        let mut latest = None;
        while let Ok(message) = outcomes.try_recv() {
            latest = Some(message);
        }
        if let Some(text) = latest {
            self.report(&text, true);
        }
    }

    /// Carry out an action on behalf of the widget that raised it.
    fn perform(&mut self, action: Action) {
        match &action {
            Action::Load(path) => self.view.load(path),
            Action::Preview(path) => self.view.preview(path),
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
        let lines = self.view.textarea.lines().to_vec();
        // The hide toggle describes how the user is reading, not which file
        // they are reading, so it outlives the document exactly as the filter
        // set does — and for the same reason. The filters survived a load
        // only because `App` owns them separately from the `Document` this
        // line replaces; the mode lives *on* the document, so without
        // carrying it across, every load and every navigator preview silently
        // reset it to `Mode::default()`.
        //
        // That made the toggle almost unusable for its main purpose: skimming
        // a directory for the files a filter actually matches means moving
        // the navigator's selection, and every move fired a `Preview` through
        // here and undid the `Ctrl-H` that made the skim possible.
        let mode = self.document.mode();
        self.document = Document::new(lines);
        self.document.set_mode(mode);
        // The buffer the view is showing belongs to the *previous* document,
        // so the record of what it was built from is meaningless now.
        // Clearing it forces the next `apply_view` to rebuild: two different
        // documents can easily produce an equal visible list — reloading the
        // same file with a filter active produces an identical one every
        // time, which would otherwise leave the just-loaded, unfiltered
        // buffer in place under numbers and styles sized for the filtered
        // subset.
        self.last_visible = None;
        // Both halves of the rebuild-skip key, or the surviving half could
        // still match and skip a rebuild this just decided is owed.
        self.last_window = None;
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
        let rows = self.filters.row_count();
        self.filters_pane.clamp_selection(rows);

        // The cursor is a source line index for the duration of the rebuild:
        // its row in the view is only meaningful against the old visible list.
        let cursor_source = self.cursor_source();
        self.document.evaluate(&self.filters);
        self.apply_view(cursor_source);
    }

    /// Handle a key aimed at the filter pane.
    ///
    /// This borrows the pane and the `ActiveFilters` together — something
    /// neither `FilterList` nor `Action` can do on their own, since the pane
    /// only ever borrows the set to render it — applies whatever command the
    /// pane reports, and re-evaluates. A delete renumbers the remaining
    /// filters, so `refresh_view`'s full `Document::evaluate` is required
    /// here: every cached `Verdict::Included` is a positional index that a
    /// patch would leave stale.
    ///
    /// The two `Edit` commands are the exception and return before that
    /// re-evaluate: they only open a prompt, and nothing about the set changes
    /// until it commits — at which point `replace_filter` or `apply_search`
    /// does the re-evaluating instead.
    ///
    /// Takes the whole `KeyEvent`, not just its `KeyCode`: `FilterList::handle_key`
    /// needs the modifiers to guard `space`/`d`/`j`/`k` against CONTROL and
    /// ALT, the same way every other global binding is guarded — see its
    /// doc comment.
    fn handle_filter_key(&mut self, key: event::KeyEvent) {
        // `i` and `x` are handled here rather than in `FilterList::handle_key`
        // because they open a prompt, and `self.search` is `App`'s. They are
        // deliberately not `FilterCommand` variants: that enum describes
        // mutations of the `ActiveFilters`, and opening a prompt is not one — see
        // its doc comment in `widgets/mod.rs`.
        //
        // `e` would read better than `x` for "exclude" and cannot be used: the
        // global match above runs first and returns, so a bare `e` never
        // reaches this function. Guarding the global arm the way `/` is
        // guarded would cost `e`-to-explorer from this pane, which is the
        // one thing these focus keys exist to provide.
        if key.modifiers.is_empty() {
            let kind = match key.code {
                KeyCode::Char('i') => Some(PromptKind::Filter),
                KeyCode::Char('x') => Some(PromptKind::Exclude),
                _ => None,
            };
            if let Some(kind) = kind {
                self.search = Some(SearchPrompt {
                    kind,
                    ..SearchPrompt::default()
                });
                return;
            }
        }

        let rows = self.filters.row_count();
        let has_search = self.filters.search().is_some();
        let Some(command) = self.filters_pane.handle_key(key, rows, has_search) else {
            return;
        };
        match command {
            FilterCommand::Toggle(index) => {
                self.filters.toggle_enabled(index);
            }
            FilterCommand::Delete(index) => {
                self.filters.remove(index);
            }
            FilterCommand::ToggleSearch => {
                let enabled = self.filters.search().is_some_and(|search| search.enabled);
                self.filters.search_set_enabled(!enabled);
            }
            FilterCommand::DeleteSearch => {
                self.filters.clear_search();
            }
            // The two commands that change nothing yet — they open a prompt,
            // and the set is only touched if it commits. Both return early
            // rather than falling through to the `refresh_view` below: there
            // is nothing to re-evaluate, and `evaluate` is O(lines × filters).
            FilterCommand::Edit(index) => {
                // The row the pane reported is one it drew, so the filter is
                // there; falling out silently rather than indexing keeps that
                // a property of the pane's own bounds, not a promise this
                // function has to make.
                if let Some(filter) = self.filters.filters().get(index) {
                    self.search = Some(SearchPrompt {
                        pattern: filter.pattern.as_str().to_string(),
                        kind: PromptKind::Edit {
                            index,
                            sense: filter.sense,
                        },
                        ..SearchPrompt::default()
                    });
                }
                return;
            }
            FilterCommand::EditSearch => {
                if let Some(search) = self.filters.search() {
                    self.search = Some(SearchPrompt {
                        pattern: search.pattern.as_str().to_string(),
                        kind: PromptKind::EditSearch,
                        ..SearchPrompt::default()
                    });
                }
                return;
            }
        }
        // Deleting the last filter used to collapse the pane, so focus had to
        // be pushed off it. The pane stays now, so focus stays too — moving it
        // would be a jump the user did not ask for, off a pane still on screen.
        self.refresh_view();
    }

    /// A one-line summary of the filter state, empty when no filters exist.
    ///
    /// Dimming alone does not say *why* lines are dim, or that a filter is
    /// defined but currently disabled — the pane would just look ordinary.
    fn status_text(&self) -> String {
        // `FilteredOnly` only hides anything when something enabled is
        // including: issue #36's guard in `Document::recompute_visible`
        // shows the whole file instead once nothing is, so `any_including`
        // has to gate the funnel here too — otherwise a filter that exists
        // but is disabled (or a disabled search) would claim lines are
        // hidden while the guard is already showing everything.
        //
        // An excluding filter (`x`) is counted on its own, regardless of
        // mode: it removes its matches in `Dimmed` mode too, which is the
        // entire point of it, so gating the funnel on `FilteredOnly` alone
        // let `x` empty the pane with nothing on the status line saying so.
        let hiding = (self.document.mode() == Mode::FilteredOnly && self.filters.any_including())
            || self.filters.any_excluding();
        let funnel = if hiding { "▼ " } else { "" };
        if self.filters.is_empty() && self.filters.search().is_none() {
            // With no filters and no search at all, `any_including` and
            // `any_excluding` are both trivially false, so `hiding` above is
            // always false too — there is nothing this early return could be
            // discarding. Filters that exist but are all disabled are a
            // different state, one the same `hiding` expression already
            // keeps honest below (see the `!any_enabled` branch): nothing
            // enabled can be including or excluding either, so the funnel
            // stays off there as well.
            return String::new();
        }
        // `row_count`, not `len`: a live search with no numbered filters is
        // still one filter as far as this row is concerned — `len` alone
        // would report "0 filters" while a search was visibly active. See
        // `ActiveFilters::row_count`, which the filter pane counts rows by for
        // exactly this reason.
        let count = self.filters.row_count();
        let noun = if count == 1 { "filter" } else { "filters" };
        if !self.filters.any_enabled() {
            return format!("{funnel}{count} {noun} (disabled)");
        }
        // Report what is actually on screen (lines *shown*) rather than how
        // many matched an including filter: an excluding filter alone can
        // remove lines while matching nothing, which used to read as "0
        // matched" over a pane that had in fact lost lines.
        let (total, previewing) = self.total_lines_text();
        let note = if previewing { " (preview)" } else { "" };
        format!(
            "{funnel}{count} {noun}   {}/{total} lines shown{note}",
            self.document.visible().len(),
        )
    }

    /// The file's line count as the row should report it, and whether that is
    /// a preview's estimate rather than a fact.
    ///
    /// The document holds whatever the view holds, so while the view is
    /// showing a bounded preview `document.lines().len()` is the *preview's*
    /// length — the cap, not the file's count — and reporting it unqualified is
    /// not a rounding error but a wrong answer. The estimate is a guess, but
    /// it is a guess marked as one.
    fn total_lines_text(&self) -> (String, bool) {
        let loaded = self.document.lines().len();
        match (self.view.truncated, self.view.estimated_lines) {
            // Truncated with nothing to scale from: the count is still the
            // preview's, so it stays flagged even without a better number.
            (true, None) => (loaded.to_string(), true),
            (true, Some(estimate)) => (format!("~{estimate}"), true),
            (false, _) => (loaded.to_string(), false),
        }
    }

    /// The directory the navigator is listing.
    fn nav_dir(&self) -> &std::path::Path {
        self.nav.dir.as_path()
    }

    /// The whole bottom row: filter state first, then the directory in
    /// whatever width is left.
    ///
    /// Filter state comes first because it cannot degrade — a count with its
    /// digits cut off is wrong rather than short — whereas the path elides
    /// from the left and stays readable. That is the priority order the row
    /// needs on a narrow terminal, where all of this cannot fit at once.
    fn status_bar_text(&self, width: usize) -> String {
        let status = self.status_text();
        let dir = self.nav_dir().display().to_string();
        if status.is_empty() {
            return elide_left(&dir, width);
        }
        // Two spaces of separation, and the path only gets what survives the
        // status text. Too narrow for any of it and the path is dropped
        // entirely rather than rendered as a lone ellipsis.
        let spent = status.chars().count() + 2;
        match width.checked_sub(spent) {
            Some(room) if room > 1 => format!("{status}  {}", elide_left(&dir, room)),
            _ => status,
        }
    }

    /// Move focus to the next pane.
    ///
    /// Every pane is always on screen, so this is a plain rotation. It used to
    /// skip the filter pane while an empty set collapsed it out of the layout,
    /// since focusing a pane that is not drawn would strand the user with no
    /// visible cursor; the pane no longer collapses, so the special case is
    /// gone with it and the cycle is three deep at all times.
    ///
    /// The `nav_index`/`file_view_index`/`filter_list_index` helpers this
    /// used to rotate between are gone with the vec they searched: a `Focus`
    /// names its pane directly, so there is no lookup left to get wrong
    /// (#73).
    fn focus_next(&mut self) {
        self.focus = self.focus.next();
        // The zoomed pane is always the focused pane, so the cursor is never
        // on a pane that is not on screen. This lives inside `focus_next`
        // itself, rather than beside its call site, so a future caller of
        // `focus_next` cannot forget it.
        if self.zoom.is_some() {
            self.zoom = Some(self.focus);
        }
    }

    /// Zoom `target`, or restore the split if it is already zoomed. Reports
    /// whether the pane ended up zoomed.
    fn toggle_zoom(&mut self, target: Focus) -> bool {
        // A drag in progress has no divider to keep tracking once zoomed —
        // the `Drag` arm in `handle_divider` only checks `self.dragging`, not
        // whether a divider is actually on screen — so it would otherwise
        // keep silently re-pinning `nav_width` or `filter_height` while
        // nothing is drawn to explain why, with the new size only appearing
        // on un-zoom. Zooming (in either direction) cancels it outright.
        // Unzooming is a no-op here in practice, since a drag can only start
        // via a click that `divider_at` accepted, and that can't happen while
        // already zoomed.
        self.dragging = None;
        self.zoom = match self.zoom {
            Some(pane) if pane == target => None,
            _ => Some(target),
        };
        self.zoom == Some(target)
    }

    /// Maximise the focused pane, or restore the split if it already is.
    fn zoom_focused(&mut self) {
        self.toggle_zoom(self.focus);
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
        if self.toggle_zoom(Focus::View) {
            self.focus = Focus::View;
        }
    }

    /// Bring the left column back and put the cursor in it.
    /// Focus `pane`, un-zooming first so it is actually visible.
    ///
    /// Clearing the zoom is the whole reason the focus keys are not just
    /// `focus = ...`: `b` and `z` can leave a pane hidden, and a focus key
    /// that moved the cursor onto a pane the user cannot see would be worse
    /// than no key at all.
    fn reveal_and_focus(&mut self, pane: Focus) {
        self.zoom = None;
        self.focus = pane;
    }

    /// Mark each pane as focused or not, before drawing.
    ///
    /// Three assignments rather than an enumerate-and-compare over a vec: the
    /// index that loop compared against no longer exists (#73).
    fn set_active_pane(&mut self) {
        self.nav.active = self.focus == Focus::Nav;
        self.view.active = self.focus == Focus::View;
        self.filters_pane.active = self.focus == Focus::Filters;
    }

    /// Draw one pane into `area`.
    ///
    /// The filter pane is handed the `ActiveFilters` it needs; the other two
    /// need nothing beyond themselves. This replaced a free `render_widget`
    /// function that existed only to route around a `Widget` impl one variant
    /// could never satisfy (#75) — each pane is now called directly with the
    /// arguments it actually takes.
    fn render_pane(&mut self, pane: Focus, area: Rect, buf: &mut Buffer) {
        match pane {
            Focus::Nav => self.nav.render(area, buf),
            Focus::View => self.view.render(area, buf),
            Focus::Filters => self.filters_pane.render(&self.filters, area, buf),
        }
    }
}

/// Shorten `text` to `width` columns by dropping characters from the *left*,
/// marking the cut with a leading `…`.
///
/// The tail is what identifies a path: `…/projects/recon/src` still says where
/// you are, where the same cut taken from the right would not.
/// Measured in terminal columns, not `char`s. A CJK ideograph or an emoji
/// occupies two columns, so counting chars over-filled the budget by one column
/// per wide glyph and the row then overran the terminal it was sized for (#97).
///
/// The tail is accumulated from the right — the one place where columns and
/// chars genuinely differ in *how* the cut is taken, not merely in what it
/// measures, since a wide glyph can no longer be assumed to cost one.
fn elide_left(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    // One column goes to the ellipsis itself.
    let budget = width - 1;
    let mut taken = 0;
    let mut start = text.len();
    for (index, ch) in text.char_indices().rev() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if taken + w > budget {
            break;
        }
        taken += w;
        start = index;
    }
    format!("…{}", &text[start..])
}

impl Widget for &mut App<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use Constraint::{Length, Min};
        // The `assert!(self.widgets.len() == 3)` that used to open this method
        // is gone (#87). It ran 60 times a second, in release, to re-check an
        // invariant established once in `App::new`; three named fields make
        // "there are exactly three panes" unrepresentable rather than merely
        // checked, so there is nothing left to assert.

        // The bottom row is reserved unconditionally. It used to be taken only
        // when there was something to put there, which meant the panes resized
        // under the user the moment the first filter appeared and resized back
        // when the last one went. The row now always has the current directory
        // to show, so the objection that answered — spending a row to say
        // nothing — no longer applies.
        let [area, prompt_area] = Layout::vertical([Min(0), Length(1)]).areas(area);

        // The badge mirrors the mode and nothing else. `▼` answers a
        // different question — "are lines missing from the pane right now?" —
        // and is deliberately false when hide mode is armed with nothing
        // including, because the #36 guard in `Document::recompute_visible` is
        // showing the whole file. Both facts are worth reporting, so they get
        // an indicator each; conflating them means one of them lies.
        //
        // Painting the badge here rather than inside `status_text` is what
        // makes that unconditional structurally: `status_text` returns early
        // with an empty string when there are no filters and no search, which
        // is exactly the state the issue was reported from. A badge threaded
        // through that function would need a second conditional to dodge the
        // early return, and a conditional can go stale.
        let badge = (self.document.mode() == Mode::FilteredOnly).then_some(HIDE_BADGE_TEXT);
        // One column of separation, so the colour block never abuts the text
        // beside it. Taken out of the status text's budget rather than added
        // to the row, so `status_bar_text`'s existing priority order — filter
        // state first, then whatever the path can elide itself into — still
        // has an accurate width to work with on a narrow terminal.
        let badge_width = badge.map_or(0, |text| text.chars().count() + 1);
        let room = (prompt_area.width as usize).saturating_sub(badge_width);
        let status = self.status_bar_text(room);

        // A zoomed pane takes the whole pane area; the others are not drawn.
        // This deliberately falls through to the status/prompt drawing below
        // rather than returning, so the status line survives a zoom.
        if let Some(zoomed) = self.zoom {
            debug_assert_eq!(
                zoomed, self.focus,
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
            // The horizontal divider is parked for the same reason and by the
            // same argument — but it has to be parked at `u16::MAX` rather
            // than zeroed. `divider_at` reaches its row test only for a column
            // strictly left of `self.divider`, and *every* column is strictly
            // left of `u16::MAX`, so the parked vertical divider is no guard
            // at all here: a zeroed rect would put the horizontal divider on
            // row 0 and let a click on the top row of a zoomed pane resize
            // something invisible.
            self.filter_area = Rect {
                x: 0,
                y: u16::MAX,
                width: 0,
                height: 0,
            };
            self.set_active_pane();
            self.render_pane(zoomed, area, buf);
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

            // Remember the boundaries so mouse events landing before the next
            // frame can be tested against them. The filter pane's whole rect
            // is kept, not just its top edge: a drag needs the bottom of the
            // left column to turn the row it is holding into a height.
            self.divider = area.x + nav_width;
            self.filter_area = filter_area;

            // Each pane gets the area that matches what it is. That used to
            // need saying — the areas did not share the vec's order, and
            // indexing one by the other's position panicked the moment a
            // third widget existed — but pairing a named pane with its named
            // area leaves nothing to mismatch (#73).
            self.set_active_pane();
            self.render_pane(Focus::Nav, nav_area, buf);
            self.render_pane(Focus::View, right, buf);
            self.render_pane(Focus::Filters, filter_area, buf);
        }

        // Last, so it covers whatever the panes just drew — and over `area`,
        // which is everything above the status row rather than the whole
        // frame. The row below carries the HIDE badge and the current
        // directory, and both are still true while the keymap is up; hiding
        // them would mean the one screen that explains `Ctrl-H` is also the one
        // screen that stops showing whether it is on.
        if self.help {
            help::render(area, buf);
        }

        // An open prompt takes the rest of the row; nothing but the badge
        // competes with it. The badge stays because the mode it reports is
        // still armed while a filter is being typed — which is precisely when
        // the pane is about to change underfoot.
        //
        // A transient message sits between the prompt and the derived status
        // text: it is more urgent than a filter count (it reports something
        // that just happened, and only lives until the next keypress) and less
        // urgent than a prompt (which the user is actively typing into).
        let (text, style) = match (self.search.as_ref(), self.status_message.as_ref()) {
            (Some(prompt), _) if prompt.error.is_some() => {
                (prompt.line(), Style::default().fg(Color::Red))
            }
            (Some(prompt), _) => (prompt.line(), Style::default()),
            (None, Some(message)) if message.error => {
                (message.text.clone(), Style::default().fg(Color::Red))
            }
            // Not dimmed like the derived row below: this one is a report the
            // user is meant to notice, and it is gone by the next keystroke.
            (None, Some(message)) => (message.text.clone(), Style::default()),
            (None, None) => (status, Style::default().fg(Color::DarkGray)),
        };
        // Two writes at two styles, rather than converting the row to
        // `Line`/`Span`s: a bigger diff through the prompt path that shares
        // this row, for no gain until something wants a third style here.
        if let Some(badge) = badge {
            buf.set_stringn(
                prompt_area.x,
                prompt_area.y,
                badge,
                prompt_area.width as usize,
                HIDE_BADGE_STYLE,
            );
        }
        buf.set_stringn(
            prompt_area.x + badge_width as u16,
            prompt_area.y,
            text,
            room,
            style,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The pane-geometry constants moved to `layout` with the methods that use
    // them (#74), and are deliberately not re-exported from the crate root —
    // production code outside that module has no business reading them. The
    // layout tests still assert against them by name, so they are imported
    // here rather than through `use super::*`.
    use crate::filter::Verdict;
    use crate::layout::{
        MAX_NAV_WIDTH, MIN_AUTO_FILTER_HEIGHT, MIN_AUTO_NAV_WIDTH, MIN_FILE_VIEW_WIDTH,
        MIN_FILTER_HEIGHT, MIN_NAV_HEIGHT, MIN_PANE_WIDTH,
    };
    use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::prelude::Buffer;
    use ratatui::style::Modifier; // the tests assert on Modifier::DIM
    use std::fmt::Write as _;
    use std::fs;
    use std::sync::Mutex;

    /// `n` newline-terminated lines, `line 0` through `line n-1`.
    ///
    /// One buffer appended to, not `(0..n).map(|i| format!(...)).collect()`:
    /// the latter allocates and drops a `String` per line, which is quadratic
    /// and showed up across ~15 fixtures in this file and `widgets/fileview.rs`
    /// (#90). `write!` into a `String` cannot fail, hence the discarded result.
    fn numbered_lines(n: usize) -> String {
        (0..n).fold(String::new(), |mut body, i| {
            let _ = writeln!(body, "line {i}");
            body
        })
    }

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
    ///
    /// **Compared case-insensitively, because the filesystem is** (#69). This
    /// guard was `used == name` and so had a hole exactly the shape of the bug
    /// it exists to prevent: macOS ships case-insensitive APFS, so `o_ctrl` and
    /// `O_ctrl` name one directory, and five `o_*`/`O_*` fixture pairs sat on
    /// top of each other undetected. The failure was a `NotFound` on
    /// `fs::write` roughly one run in five — one test's `remove_dir_all`
    /// landing between the other's `create_dir_all` and its `fs::write`.
    ///
    /// Deliberately not conditioned on the host filesystem. A guard that only
    /// fired on macOS would let a colliding pair be added on Linux and
    /// rediscovered by whoever next ran the suite on a Mac; refusing the pair
    /// everywhere costs nothing but a fixture rename.
    ///
    /// `eq_ignore_ascii_case` rather than a full Unicode case fold: fixture
    /// names here are hand-written ASCII identifiers, and the ASCII form needs
    /// no allocation. A non-ASCII fixture name would slip through, which is a
    /// smaller hole than the one this closes and not one this suite can reach.
    fn claim_fixture_dir(name: &str) {
        let mut names = FIXTURE_DIR_NAMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !names.iter().any(|used| used.eq_ignore_ascii_case(name)),
            "fixture directory name {name:?} is already in use by another test \
             (compared case-insensitively — macOS treats {name:?} and its \
             other-case spellings as one directory) — pick a unique name"
        );
        names.push(name.to_string());
    }

    /// Two fixture names differing only in case are a collision, not two names.
    ///
    /// macOS ships a case-insensitive filesystem by default, so
    /// `target/test-appdirs/o_ctrl` and `target/test-appdirs/O_ctrl` are one
    /// directory. A case-sensitive guard sees two distinct strings, says
    /// nothing, and lets the two tests race to `remove_dir_all` and
    /// `create_dir_all` the same path — one of them deleting the other's
    /// `logs/` between its `create_dir_all` and its `fs::write` (#69).
    ///
    /// The probe names are deliberately not any real fixture's: claiming a name
    /// here consumes it for the rest of the process.
    #[test]
    #[should_panic(expected = "already in use")]
    fn a_fixture_name_differing_only_in_case_is_a_collision() {
        claim_fixture_dir("zz_case_probe");
        claim_fixture_dir("ZZ_CASE_PROBE");
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
            path: dir.join("placeholder").display().to_string(),
            ..Config::default()
        })
    }

    fn draw(app: &mut App) {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
    }

    fn mouse_at(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
        app.handle_event(event::Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }))
        .unwrap();
    }

    /// Row 3 is inside the navigator on every fixture area used here, and so
    /// is clear of the horizontal divider's own hit test — these helpers are
    /// about the vertical divider, and a row that could land on both would
    /// make which one they exercise a matter of hit-test ordering.
    fn mouse(app: &mut App, kind: MouseEventKind, column: u16) {
        mouse_at(app, kind, column, 3);
    }

    fn drag_to(app: &mut App, from: u16, to: u16) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), from);
        mouse(app, MouseEventKind::Drag(MouseButton::Left), to);
        mouse(app, MouseEventKind::Up(MouseButton::Left), to);
    }

    /// Column 1 is well inside the left column on every fixture area used
    /// here, so the vertical divider's hit test cannot claim these events —
    /// the mirror of the note on `mouse` above.
    const INSIDE_LEFT_COLUMN: u16 = 1;

    fn drag_rows_to(app: &mut App, from: u16, to: u16) {
        mouse_at(
            app,
            MouseEventKind::Down(MouseButton::Left),
            INSIDE_LEFT_COLUMN,
            from,
        );
        mouse_at(
            app,
            MouseEventKind::Drag(MouseButton::Left),
            INSIDE_LEFT_COLUMN,
            to,
        );
        mouse_at(
            app,
            MouseEventKind::Up(MouseButton::Left),
            INSIDE_LEFT_COLUMN,
            to,
        );
    }

    /// The name is comfortably longer than `MIN_AUTO_NAV_WIDTH` on purpose:
    /// below the floor the column reports the floor, so a short name would
    /// make this pass without the snapping it is named for ever happening.
    #[test]
    fn auto_width_snaps_to_the_longest_entry() {
        const LONGEST: &str = "a_twenty_five_char_name.rs";
        let mut app = app_over("snap", &["a.rs", LONGEST]);
        draw(&mut app);

        // Name plus two borders; the `>>` marker used to add two more.
        assert!(
            LONGEST.len() as u16 + 2 > MIN_AUTO_NAV_WIDTH,
            "fixture no longer exercises snapping"
        );
        assert_eq!(app.nav_width(AREA), LONGEST.len() as u16 + 2);
    }

    #[test]
    fn auto_width_is_capped_at_the_default() {
        let long = "a".repeat(200);
        let mut app = app_over("capped", &[long.as_str()]);
        draw(&mut app);

        assert_eq!(app.nav_width(AREA), MAX_NAV_WIDTH);
    }

    /// A directory of short names gets the floor, not the width of its
    /// longest entry.
    ///
    /// Replaces `auto_width_has_no_floor`, which asserted the opposite —
    /// "a directory of short names gets a narrow pane". Snapping that tight
    /// is what made entering a one-entry directory move every pane on
    /// screen; see #33. Automatic sizing exists to stop the column being
    /// uselessly wide, not to win back every column it can.
    #[test]
    fn auto_width_does_not_shrink_below_the_floor() {
        let mut app = app_over("tiny", &["a"]);
        draw(&mut app);

        assert_eq!(app.nav_width(AREA), MIN_AUTO_NAV_WIDTH);
    }

    /// The floor is not a fixed width: a directory of longer names still
    /// widens past it, up to the cap.
    #[test]
    fn auto_width_still_grows_past_the_floor() {
        let name = "a".repeat(MIN_AUTO_NAV_WIDTH as usize + 5);
        let mut app = app_over("above_floor", &[name.as_str()]);
        draw(&mut app);

        assert!(
            app.nav_width(AREA) > MIN_AUTO_NAV_WIDTH,
            "the floor became a fixed width, got {}",
            app.nav_width(AREA)
        );
    }

    /// The floor governs automatic sizing only. A drag is a decision, and it
    /// may still take the column down to `MIN_PANE_WIDTH`.
    #[test]
    fn the_floor_does_not_apply_to_a_dragged_width() {
        let mut app = app_over("drag_below_floor", &["a.rs"]);
        draw(&mut app);
        let divider = app.divider;

        drag_to(&mut app, divider, MIN_PANE_WIDTH);

        assert_eq!(app.nav_width(AREA), MIN_PANE_WIDTH);
    }

    /// On a terminal too narrow to honour both, the file view's floor wins:
    /// the navigator giving up columns it would like is better than the pane
    /// the app exists for becoming unusable.
    #[test]
    fn the_file_views_floor_outranks_the_navigators() {
        let mut app = app_over("narrow_term", &["a"]);
        let narrow = Rect {
            x: 0,
            y: 0,
            width: MIN_FILE_VIEW_WIDTH + MIN_AUTO_NAV_WIDTH - 5,
            height: 10,
        };
        draw(&mut app);

        assert!(
            app.nav_width(narrow) < MIN_AUTO_NAV_WIDTH,
            "the floor starved the file view, got {}",
            app.nav_width(narrow)
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
        // Back to automatic sizing, which for a name this short is the floor
        // rather than the name's own width — see `MIN_AUTO_NAV_WIDTH`.
        assert_eq!(app.nav_width(AREA), MIN_AUTO_NAV_WIDTH);
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

    /// The left column's rows on `AREA`, which every horizontal-divider test
    /// below reasons about: the status row is taken off the top-level split
    /// before the columns are laid out.
    const LEFT_HEIGHT: u16 = AREA.height - 1;

    /// #44's second half. The pane's height was content-driven and nothing
    /// else, which the original design called "no comparable target to drag"
    /// — true while it collapsed to nothing when empty, and no longer true
    /// now it opens at `MIN_AUTO_FILTER_HEIGHT` whatever it holds.
    ///
    /// Dragging *up* makes the pane taller, because the divider is its top
    /// border: the pane is anchored to the bottom of the column, so what the
    /// drag moves is where it starts, not where it ends.
    #[test]
    fn dragging_the_horizontal_divider_pins_the_filter_panes_height() {
        let mut app = app_over("hdrag", &["a.rs"]);
        draw(&mut app);
        let divider = app.filter_area.y;

        drag_rows_to(&mut app, divider, divider - 2);

        // Two rows up from the boundary, on a column whose bottom is
        // `LEFT_HEIGHT`, is two rows more pane than it had.
        assert_eq!(app.filter_height, FilterHeight::Pinned(LEFT_HEIGHT - 3));
        assert_eq!(app.filter_pane_split_height(LEFT_HEIGHT), LEFT_HEIGHT - 3);
    }

    /// The counterpart of `the_floor_does_not_apply_to_a_dragged_width`, and
    /// the reason the two floors are separate constants: a drag is a
    /// decision, so it may leave the pane well under the height it opens at.
    ///
    /// Measured against a 40-row column, where automatic sizing would give
    /// `MIN_AUTO_FILTER_HEIGHT` — on `AREA` the caps would produce a small
    /// number anyway and the test would pass without the drag being honoured.
    #[test]
    fn the_starting_height_does_not_apply_to_a_dragged_height() {
        let mut app = app_over("hdrag_small", &["a.rs"]);
        draw(&mut app);
        let divider = app.filter_area.y;

        // Down to exactly the drag floor: `LEFT_HEIGHT - MIN_FILTER_HEIGHT`
        // is the boundary row that leaves three rows below it.
        drag_rows_to(&mut app, divider, LEFT_HEIGHT - MIN_FILTER_HEIGHT);

        assert_eq!(
            app.filter_pane_split_height(40),
            MIN_FILTER_HEIGHT,
            "the starting height overrode a deliberate drag"
        );
    }

    /// A drag past the bottom of the column asks for a pane of no rows at
    /// all. It gets `MIN_FILTER_HEIGHT` — enough to still be visible and so
    /// still be recognisably the thing that was just dragged, rather than a
    /// pane that vanishes while keeping focus (see `MIN_NAV_HEIGHT`).
    #[test]
    fn dragging_cannot_collapse_the_filter_pane() {
        let mut app = app_over("hdrag_collapse", &["a.rs"]);
        draw(&mut app);
        let divider = app.filter_area.y;

        drag_rows_to(&mut app, divider, AREA.height * 2);

        assert_eq!(
            app.filter_pane_split_height(LEFT_HEIGHT),
            MIN_FILTER_HEIGHT,
            "the filter pane collapsed"
        );
    }

    /// The other end: a drag to the top of the column asks for everything.
    /// The navigator's floor is what stops it, and it stops it at exactly the
    /// floor — the half cap governs automatic sizing only, so a drag can
    /// legitimately take more than half.
    #[test]
    fn dragging_cannot_collapse_the_navigator() {
        let mut app = app_over("hdrag_nav_floor", &["a.rs"]);
        draw(&mut app);
        let divider = app.filter_area.y;

        drag_rows_to(&mut app, divider, 0);

        let filter_height = app.filter_pane_split_height(LEFT_HEIGHT);
        assert_eq!(
            LEFT_HEIGHT - filter_height,
            MIN_NAV_HEIGHT,
            "the navigator did not keep exactly its floor"
        );
        assert!(
            filter_height > LEFT_HEIGHT / 2,
            "the half cap bound a drag it has no business binding: \
             {filter_height}"
        );
    }

    #[test]
    fn double_clicking_the_horizontal_divider_restores_automatic_sizing() {
        let mut app = app_over("hdbl", &["a.rs"]);
        draw(&mut app);
        let divider = app.filter_area.y;
        drag_rows_to(&mut app, divider, divider - 2);
        // Without this, the test passes just as well when the drag never
        // happened — `Auto` is the state it starts in.
        assert_ne!(
            app.filter_height,
            FilterHeight::Auto,
            "sanity: the drag did not pin a height to restore from"
        );
        draw(&mut app);

        let divider = app.filter_area.y;
        mouse_at(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            INSIDE_LEFT_COLUMN,
            divider,
        );
        mouse_at(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            INSIDE_LEFT_COLUMN,
            divider,
        );

        assert_eq!(app.filter_height, FilterHeight::Auto);
    }

    /// The horizontal divider only exists inside the left column. Without the
    /// column half of the hit test, every row of the file view at the same
    /// height would resize the filter pane — including a click on the very
    /// line the user is reading.
    #[test]
    fn the_horizontal_divider_is_not_hit_from_the_file_view() {
        let mut app = app_over("hdrag_right", &["a.rs"]);
        draw(&mut app);
        let row = app.filter_area.y;
        let in_file_view = AREA.width - 2;

        mouse_at(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            in_file_view,
            row,
        );
        mouse_at(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            in_file_view,
            0,
        );

        assert_eq!(app.filter_height, FilterHeight::Auto);
    }

    /// The two dividers cross at one corner. The vertical one spans the whole
    /// height and is the older, more-reached-for target, so it wins there —
    /// an arbitrary choice, but one that has to be made and pinned, since a
    /// silent flip would move the wrong pane under a user aiming for a corner.
    #[test]
    fn the_vertical_divider_wins_where_the_two_cross() {
        let mut app = app_over("cross", &["a.rs"]);
        draw(&mut app);
        let column = app.divider;
        let row = app.filter_area.y;

        mouse_at(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            column,
            row,
        );
        mouse_at(&mut app, MouseEventKind::Drag(MouseButton::Left), 60, row);

        assert_eq!(app.nav_width, NavWidth::Pinned(60));
        assert_eq!(app.filter_height, FilterHeight::Auto);
    }

    fn key(app: &mut App, code: KeyCode) {
        app.handle_event(event::Event::Key(code.into())).unwrap();
    }

    /// `key`, with Control held.
    fn ctrl(app: &mut App, code: KeyCode) {
        app.handle_event(event::Event::Key(event::KeyEvent::new(
            code,
            KeyModifiers::CONTROL,
        )))
        .unwrap();
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            key(app, KeyCode::Char(c));
        }
    }

    /// How many `Tab` presses can be needed to reach any pane from any other.
    ///
    /// A bound for the tab-until-focused helpers below, not an invariant the
    /// production code checks — `Focus` has three variants, so a full cycle is
    /// three presses. It replaces the `app.widgets.len()` those loops used to
    /// read, which is one of the things the vec was doing that a `Focus` does
    /// not need to (#73).
    const PANE_COUNT: usize = 3;

    /// Put focus on the file view, however many `Tab` presses that takes.
    ///
    /// A fixed `key(Tab); ...; key(Tab);` pair only reaches the file view
    /// because `Tab` used to be a two-state toggle; the filter pane joining
    /// the cycle once a filter exists already broke that assumption for
    /// three tests. Tabbing until the target is reached, rather than a fixed
    /// number of times, is robust to wherever focus started.
    fn focus_file_view(app: &mut App) {
        for _ in 0..PANE_COUNT {
            if app.focus == Focus::View {
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
        for _ in 0..PANE_COUNT {
            if app.focus == Focus::Filters {
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
        let nav = &app.nav;
        assert_eq!(nav.entries[nav.state.selected().unwrap()].name, "gamma.rs");
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
        let text: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
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

        assert_eq!(app.dragging, None);
        assert_eq!(app.nav_width(AREA), before);
    }

    #[test]
    fn i_opens_a_filter_prompt_in_the_filter_pane() {
        let mut app = app_over("filter_prompt", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "foo");

        assert_eq!(prompt_line(&mut app), "filter: foo");
    }

    #[test]
    fn committing_a_filter_adds_it() {
        let mut app = app_over("filter_add", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "foo");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_none(), "prompt stayed open");
        assert_eq!(app.filters.len(), 1);
    }

    #[test]
    fn an_invalid_filter_pattern_keeps_the_prompt_open() {
        let mut app = app_over("filter_bad", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "q");

        assert!(app.is_running());
        assert_eq!(prompt_line(&mut app), "filter: q");
    }

    #[test]
    fn successive_filters_take_different_colours() {
        let mut app = app_over("filter_colours", &["a.rs"]);

        for pattern in ["foo", "bar"] {
            key(&mut app, KeyCode::Char('f'));
            key(&mut app, KeyCode::Char('i'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }

        let styles: Vec<_> = app.filters.filters().iter().map(|f| f.style.fg).collect();
        assert_ne!(styles[0], styles[1]);
    }

    /// #62: `[filters] palette` is parsed and merged in `config`, but the value
    /// is only worth anything if `App::new` actually hands it to the filter set.
    /// This is the seam where a correctly-parsed setting would otherwise be
    /// silently dropped.
    #[test]
    fn a_configured_palette_colours_the_filters() {
        let dir = std::path::Path::new("target/test-appdirs").join("configured_palette");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        fs::write(dir.join("a.rs"), "x").expect("write fixture");

        let mut app = App::new(&Config {
            path: dir.join("placeholder").display().to_string(),
            filter_palette: Some(vec![Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6)]),
            ..Config::default()
        });

        for pattern in ["foo", "bar"] {
            key(&mut app, KeyCode::Char('f'));
            key(&mut app, KeyCode::Char('i'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }

        let styles: Vec<_> = app.filters.filters().iter().map(|f| f.style.fg).collect();
        assert_eq!(
            styles,
            vec![Some(Color::Rgb(1, 2, 3)), Some(Color::Rgb(4, 5, 6))],
            "the configured palette never reached the filter set"
        );
    }

    /// Returns the styles the file view is currently rendering with.
    fn view_line_styles(app: &App) -> Vec<Option<Style>> {
        app.view.textarea.line_styles().to_vec()
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

    /// `[` keeps paging a full screen once the window has been rebuilt at the
    /// pane's real height (#108).
    ///
    /// The window's slack was measured from the *cursor*, but a page scrolls
    /// the *viewport*, whose top edge sits a full pane above the cursor when
    /// `[` has parked it on the bottom row — which is exactly where `[` leaves
    /// it. That left 3 buffer rows above the viewport on a 36-row terminal, so
    /// `Viewport::scroll`'s `saturating_sub` clamped a 33-row page to 3, and
    /// the re-anchor afterwards reproduced the same state indefinitely.
    ///
    /// The reproduction needs a real-height window, which is why it pages down
    /// so far first: the startup window is built before any render at
    /// `ASSUMED_PANE_HEIGHT`, is 600 rows wide, and hides the bug for the first
    /// twelve pages. `]` is unaffected throughout — it parks the cursor on the
    /// pane's *top* row, where the viewport's edge and the cursor coincide.
    #[test]
    fn page_up_keeps_paging_a_full_screen_after_the_window_is_rebuilt() {
        let body = numbered_lines(7_000);
        // 36 rows, as reported: a 35-row file-view pane, 33 rows inside its
        // border. A page is one row short of the inner height.
        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 36,
        };
        let mut app = app_over_file("page_up_full_screen.txt", &body);
        let mut buf = Buffer::empty(area);
        (&mut app).render(area, &mut buf);
        key(&mut app, KeyCode::Char('t'));
        (&mut app).render(area, &mut buf);

        let top = |app: &App| -> usize {
            let (scroll, _) = app.view.textarea.scroll_top();
            app.view.window_start() + scroll as usize
        };

        // Far enough down that the startup window has been replaced by one
        // sized to the real pane. Thirteen is the first page that does it.
        for _ in 0..13 {
            key(&mut app, KeyCode::Char(']'));
            (&mut app).render(area, &mut buf);
        }

        // Every page up moves a full page, not just the first two.
        for press in 1..=6 {
            let before = top(&app);
            key(&mut app, KeyCode::Char('['));
            (&mut app).render(area, &mut buf);
            let moved = before - top(&app);
            assert_eq!(
                moved,
                33,
                "`[` press {press} moved {moved} lines, not a full page \
                 (view top {before} -> {})",
                top(&app),
            );
        }
    }

    /// Scrolling line by line must not rebuild the buffer on every keystroke.
    ///
    /// This is the other half of #108 and the reason the window carries two
    /// screens of slack rather than one. `window_holds` asks for a page of
    /// buffer beyond each viewport edge so that a page never clamps; if
    /// `window_for` laid down exactly that much, the requirement would be met
    /// with zero margin and the first `j` that scrolled the viewport would owe
    /// a rebuild — a `set_lines` per keystroke while holding `j`, which is the
    /// cost #7 exists to avoid. The second screen is the margin.
    #[test]
    fn scrolling_line_by_line_does_not_rebuild_the_window_every_keystroke() {
        let body = numbered_lines(7_000);
        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 36,
        };
        let mut app = app_over_file("no_rebuild_thrash.txt", &body);
        let mut buf = Buffer::empty(area);
        (&mut app).render(area, &mut buf);
        key(&mut app, KeyCode::Char('t'));
        (&mut app).render(area, &mut buf);

        // Page down far enough to replace the startup window — built before
        // any render at `ASSUMED_PANE_HEIGHT`, it is 200 screens wide and would
        // absorb every scroll below without ever rebuilding, which is a test
        // that passes for the wrong reason.
        for _ in 0..13 {
            key(&mut app, KeyCode::Char(']'));
            (&mut app).render(area, &mut buf);
        }
        // Get the cursor onto the pane's bottom row, where further `j` scrolls
        // the viewport rather than just moving the cursor down it.
        for _ in 0..40 {
            key(&mut app, KeyCode::Down);
            (&mut app).render(area, &mut buf);
        }

        let mut window = (app.view.window_start(), app.view.window_end());
        let mut rebuilds = 0;
        let presses = 60;
        for _ in 0..presses {
            key(&mut app, KeyCode::Down);
            (&mut app).render(area, &mut buf);
            let now = (app.view.window_start(), app.view.window_end());
            if now != window {
                rebuilds += 1;
                window = now;
            }
        }

        // Measured: with one screen of slack this is 30 rebuilds over 60
        // presses — every other keystroke. With two it is 0, and a third screen
        // buys nothing further, which is what fixes `SLACK_SCREENS` at 2.
        assert!(
            rebuilds <= presses / 10,
            "{rebuilds} window rebuilds over {presses} single-line scrolls — \
             the slack is not absorbing ordinary movement"
        );
    }

    fn app_over_file(name: &str, body: &str) -> App<'static> {
        let file = fixture_path(name, body);
        App::new(&Config {
            path: file.display().to_string(),
            ..Config::default()
        })
    }

    /// Launched on a directory, the view shows the first entry's contents —
    /// not `<directory>`, which is what pointing the view at the argument
    /// itself would have produced.
    #[test]
    fn a_directory_argument_previews_the_first_entry() {
        let dir = std::path::Path::new("target/test-appdirs/arg_is_a_dir");
        fs::remove_dir_all(dir).ok();
        fs::create_dir_all(dir).expect("create fixture dir");
        fs::write(dir.join("aaa.txt"), "first file contents\n").expect("write");
        fs::write(dir.join("zzz.txt"), "last file contents\n").expect("write");

        let mut app = App::new(&Config {
            path: dir.display().to_string(),
            ..Config::default()
        });

        let shown = rendered(&mut app);
        assert!(
            shown.contains("first file contents"),
            "the first entry was not previewed:\n{shown}"
        );
        assert!(
            !shown.contains("<directory>"),
            "the view was pointed at the directory itself:\n{shown}"
        );
    }

    /// A directory argument *selects* its first entry rather than being
    /// handed it, so it is previewed — bounded — not read in full. Otherwise
    /// starting recon in a directory of large logs reads one of them whole.
    #[test]
    fn a_directory_argument_previews_rather_than_loads() {
        let dir = std::path::Path::new("target/test-appdirs/arg_dir_bounded");
        fs::remove_dir_all(dir).ok();
        fs::create_dir_all(dir).expect("create fixture dir");
        // Past the line cap, which is what "previewed rather than loaded" now
        // means: below the cap the two are the same thing, deliberately, since
        // reading a log-sized file whole costs well under a millisecond.
        let lines = crate::widgets::fileview::PREVIEW_LINES + 100;
        let body = numbered_lines(lines);
        fs::write(dir.join("big.log"), &body).expect("write");

        let app = App::new(&Config {
            path: dir.display().to_string(),
            ..Config::default()
        });

        // Asked of the *document*, not the view. Since #7 the textarea holds
        // only a window of what is visible, so its length says how tall the
        // pane is, not how much of the file was read.
        assert_eq!(
            app.document.lines().len(),
            crate::widgets::fileview::PREVIEW_LINES,
            "the first entry was read in full instead of previewed"
        );
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let dir = std::path::Path::new("target/test-appdirs/restyle_reload");
        fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
        app.perform(Action::Load(dir.join("other.txt")));

        let styles = view_line_styles(&app);
        assert_eq!(styles.len(), 2, "styles not re-applied to the new file");
        assert!(styles[0].is_some(), "match in the new file unstyled");
    }

    /// The hide toggle describes how you are reading, not which file you are
    /// reading — so it outlives a load exactly as the filter set does.
    ///
    /// `sync_document` replaces `self.document` wholesale, and `Document::new`
    /// starts at `Mode::default()`; the filters survived only because `App`
    /// owns them separately. The mode had no such owner.
    #[test]
    fn the_hide_mode_survives_loading_another_file() {
        let mut app = app_over_file("hide_mode_load", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('H'));
        assert_eq!(app.document.mode(), Mode::FilteredOnly, "sanity: hiding");
        assert_eq!(view_lines(&app), vec!["beta".to_string()]);

        let dir = std::path::Path::new("target/test-appdirs/hide_mode_load");
        fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
        app.perform(Action::Load(dir.join("other.txt")));

        assert_eq!(
            app.document.mode(),
            Mode::FilteredOnly,
            "the load reset the hide toggle"
        );
        assert_eq!(
            view_lines(&app),
            vec!["beta again".to_string()],
            "the new file came back unhidden"
        );
    }

    /// The workflow in the issue is cursor movement in the navigator, which
    /// fires `Preview`, not `Load` — so previews must hold the mode too, or
    /// the whole point (skimming a directory for files that are not blank) is
    /// lost on every keystroke.
    #[test]
    fn the_hide_mode_survives_a_navigator_preview() {
        let mut app = app_over_file("hide_mode_preview", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('H'));

        let dir = std::path::Path::new("target/test-appdirs/hide_mode_preview");
        fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
        app.perform(Action::Preview(dir.join("other.txt")));

        assert_eq!(
            app.document.mode(),
            Mode::FilteredOnly,
            "the preview reset the hide toggle"
        );
        assert_eq!(view_lines(&app), vec!["beta again".to_string()]);
    }

    /// The payoff the issue asks for: skimming a directory while hiding, a
    /// file with no matches shows an empty pane, which is the signal to move
    /// on. Without the mode surviving, every such file came back full of
    /// unmatched text and there was nothing to skim.
    #[test]
    fn a_file_with_no_matches_shows_an_empty_view_while_hiding() {
        let mut app = app_over_file("hide_mode_no_match", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('H'));

        let dir = std::path::Path::new("target/test-appdirs/hide_mode_no_match");
        fs::write(dir.join("quiet.txt"), "nothing\nhere\n").expect("write");
        app.perform(Action::Preview(dir.join("quiet.txt")));

        assert_eq!(app.document.mode(), Mode::FilteredOnly);
        assert!(
            app.document.visible().is_empty(),
            "a file with no matches still had visible lines while hiding"
        );
        assert!(
            view_lines(&app).iter().all(String::is_empty),
            "expected a blank pane, got {:?}",
            view_lines(&app)
        );
    }

    /// The rebuild guard's empty-set collision, reachable with no hide toggle
    /// involved at all: an excluding filter that removes every line leaves
    /// `visible()` legitimately empty, and that used to compare equal to the
    /// `last_visible` that `sync_document` had just cleared. The guard read
    /// "nothing changed" and left the freshly loaded file on screen in full —
    /// showing exactly the lines the filter existed to remove.
    ///
    /// This is why `last_visible` is an `Option`: "the buffer holds no rows"
    /// and "what the buffer holds is unknown" are different claims, and only
    /// the second one may force a rebuild.
    #[test]
    fn loading_a_file_that_every_filter_excludes_leaves_a_blank_view() {
        let mut app = app_over_file("exclude_all_load", "noise one\nnoise two\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);
        assert!(
            app.document.visible().is_empty(),
            "sanity: the filter excluded every line"
        );

        let dir = std::path::Path::new("target/test-appdirs/exclude_all_load");
        fs::write(dir.join("other.txt"), "noise three\nnoise four\n").expect("write");
        app.perform(Action::Load(dir.join("other.txt")));

        assert_eq!(
            app.document.mode(),
            Mode::Dimmed,
            "sanity: the hide toggle plays no part in this one"
        );
        assert!(app.document.visible().is_empty());
        assert!(
            view_lines(&app).iter().all(String::is_empty),
            "the excluded lines came back on screen: {:?}",
            view_lines(&app)
        );
    }

    /// Toggling back after a load must still restore the whole file — the
    /// mode surviving must not leave the document unable to leave it.
    #[test]
    fn the_hide_mode_can_still_be_toggled_off_after_a_load() {
        let mut app = app_over_file("hide_mode_untoggle", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('H'));

        let dir = std::path::Path::new("target/test-appdirs/hide_mode_untoggle");
        fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
        app.perform(Action::Load(dir.join("other.txt")));
        key(&mut app, KeyCode::Char('H'));

        assert_eq!(app.document.mode(), Mode::Dimmed);
        assert_eq!(
            view_lines(&app),
            vec!["beta again".to_string(), "nothing".to_string()],
            "toggling off did not restore the whole file"
        );
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

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
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
        key(&mut app, KeyCode::Char('i'));
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
            key(&mut app, KeyCode::Char('i'));
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
            key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('!'));
        assert!(!app.filters.any_enabled());

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
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

    /// The bottom row at an arbitrary width, for the narrow-terminal cases
    /// `AREA`'s 120 columns are far too generous to reach.
    fn status_line_at(app: &mut App, width: u16) -> String {
        let area = Rect {
            x: 0,
            y: 0,
            width,
            height: 10,
        };
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        let y = area.height - 1;
        (0..area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Row index of the lowest pane border, which is where the pane area ends.
    ///
    /// A pane's own `└` is the honest probe for "the panes reach this row" —
    /// it is drawn by the border, not by anything the status line writes.
    fn pane_bottom_row(app: &mut App) -> u16 {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        (0..AREA.height)
            .rev()
            // Plain or thick: a focused pane draws a heavy border, so its
            // own corner is `┗` rather than `└`.
            .find(|&y| (0..AREA.width).any(|x| matches!(buf[(x, y)].symbol(), "└" | "┗")))
            .expect("no bordered pane was drawn")
    }

    /// The bottom row is permanent, so the panes keep the same rows whether or
    /// not a filter exists. The layout shifting under the user on the way to
    /// the first filter is the defect this pins.
    #[test]
    fn the_panes_do_not_move_when_the_first_filter_appears() {
        let mut app = app_over_file("status_stable", "alpha\nbeta\n");
        let before = pane_bottom_row(&mut app);

        app.add_filter("alpha").expect("valid pattern");

        assert_eq!(
            before,
            pane_bottom_row(&mut app),
            "the panes resized when the first filter appeared"
        );
    }

    /// What earns the row its permanence: the directory is there to show
    /// whether or not a filter is, so the row is never blank.
    ///
    /// Asserts on the tail because the path elides from the left — the tail is
    /// what identifies where you are.
    #[test]
    fn the_status_line_names_the_current_directory() {
        let mut app = app_over_file("status_dir", "alpha\n");

        let bottom = status_line(&mut app);

        assert!(
            bottom.contains("status_dir"),
            "the directory is not named: {bottom}"
        );
    }

    /// While a preview is on screen the document holds only the preview's
    /// lines, so reporting that count as the file's total is confidently
    /// wrong: the file reads as exactly `PREVIEW_LINES` long, whatever its
    /// real length. Report the estimate instead, and say it is one.
    #[test]
    fn the_status_line_marks_a_previewed_total_as_an_estimate() {
        let mut app = app_over_file("status_preview", "alpha\n");
        app.add_filter("line").expect("valid pattern");

        // Past the preview's line cap, so the view truncates and there is an
        // estimate to report.
        let body = numbered_lines(crate::widgets::fileview::PREVIEW_LINES + 100);
        let dir = std::path::Path::new("target/test-appdirs/status_preview");
        fs::write(dir.join("big.txt"), &body).expect("write");
        app.perform(Action::Preview(dir.join("big.txt")));

        let bottom = status_line(&mut app);

        assert!(
            bottom.contains("(preview)"),
            "the total is not flagged as a preview: {bottom}"
        );
        assert!(
            bottom.contains('~'),
            "the total is not marked as an estimate: {bottom}"
        );
        assert!(
            !bottom.contains(&format!("/{} ", crate::widgets::fileview::PREVIEW_LINES)),
            "reported the preview's own line count as the file's total: {bottom}"
        );
    }

    /// `elide_left` cuts to a column budget, not a `char` budget. Counting
    /// chars over-fills by one column per wide glyph, and the status row then
    /// overruns the terminal it was supposed to fit inside (#97).
    #[test]
    fn eliding_a_path_of_wide_glyphs_fits_the_column_budget() {
        // 6 ideographs = 12 columns, plus `/x` = 14. Asked for 10.
        let path = "日本語ロググ/x";
        assert_eq!(UnicodeWidthStr::width(path), 14);

        let elided = elide_left(path, 10);

        assert!(
            UnicodeWidthStr::width(elided.as_str()) <= 10,
            "elided to {} columns, budget was 10: {elided:?}",
            UnicodeWidthStr::width(elided.as_str())
        );
        assert!(elided.starts_with('…'), "the cut is unmarked: {elided:?}");
        assert!(
            elided.ends_with("/x"),
            "cut from the wrong end — the tail is what identifies a path: {elided:?}"
        );
    }

    /// A path that already fits is returned whole, wide glyphs included.
    #[test]
    fn a_path_of_wide_glyphs_that_fits_is_not_elided() {
        let path = "日本語/x";
        assert_eq!(UnicodeWidthStr::width(path), 8);
        assert_eq!(elide_left(path, 8), path);
    }

    /// The row cannot hold everything on a narrow terminal, so it has a
    /// priority: the counts survive whole and the path gives way.
    ///
    /// That order is not arbitrary. A path cut down to `…/status_narrow` still
    /// says where you are, where `3/3 lines sh` is not a shorter truth but a
    /// broken one — so the part that cannot degrade goes first.
    #[test]
    fn a_narrow_status_line_keeps_the_counts_and_elides_the_path() {
        let mut app = app_over_file("status_narrow", "alpha\nbeta\ngamma\n");
        app.add_filter("beta").expect("valid pattern");

        let bottom = status_line_at(&mut app, 44);

        assert!(
            bottom.contains("1 filter") && bottom.contains("3/3 lines shown"),
            "the counts were cut to make room for the path: {bottom}"
        );
        // The tail is the whole point: clipping the row at its width would
        // leave the *head* (`/Users/pete/pro…`), which says nothing about
        // where you are. Eliding from the left keeps the part that does.
        assert!(
            bottom.contains("status_narrow"),
            "the path was cut from the right, so its identifying tail is gone: {bottom}"
        );
    }

    /// With no filters the row reports no filter state — but it is still
    /// surrendered, because it is permanent.
    ///
    /// Replaces `no_row_is_surrendered_without_filters`, which asserted that
    /// the panes reached the bottom row when there was nothing to report. That
    /// is the conditional layout whose shifting-under-the-user is the defect
    /// here; its objection was that a permanent row costs a row to say
    /// nothing, and the row now always names the directory instead.
    #[test]
    fn no_filter_state_is_reported_without_filters() {
        let mut app = app_over_file("status_none", "alpha\n");

        let bottom = status_line(&mut app);

        assert!(
            !bottom.contains("filters"),
            "reported filter state when no filters exist: {bottom}"
        );
        assert!(
            !bottom.contains('└') && !bottom.contains('┗'),
            "the panes still reach the bottom row, so the layout still shifts: {bottom}"
        );
    }

    #[test]
    fn the_status_line_reports_filters_and_lines_shown() {
        let mut app = app_over_file("status_some", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));

        assert!(
            status_line(&mut app).contains("disabled"),
            "no indication the filters are off: {}",
            status_line(&mut app)
        );
    }

    /// With every filter disabled, `any_including` is false, so issue #36's
    /// guard in `Document::recompute_visible` shows the whole file even in
    /// `FilteredOnly` mode — nothing is actually hidden. Gating `hiding` on
    /// `mode == FilteredOnly` alone (rather than requiring `any_including`
    /// too) would still show the funnel here, claiming lines were hidden
    /// over a pane that is in fact showing everything.
    #[test]
    fn the_status_line_does_not_show_a_funnel_when_disabled_filters_cannot_hide_anything() {
        let mut app = app_over_file("status_off_no_funnel", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));
        key(&mut app, KeyCode::Char('H'));

        assert!(
            status_line(&mut app).contains("disabled"),
            "sanity: still reports disabled: {}",
            status_line(&mut app)
        );
        assert!(
            !status_line(&mut app).contains('▼'),
            "funnel claimed lines were hidden while the #36 guard is \
             showing everything: {}",
            status_line(&mut app)
        );
    }

    /// An open prompt takes the row, as it already does.
    #[test]
    fn a_prompt_still_takes_the_bottom_row() {
        let mut app = app_over_file("status_prompt", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "foo");

        assert_eq!(status_line(&mut app), "filter: foo");
    }

    /// Arrowing onto a large log only previews it (bounded by
    /// `PREVIEW_LINES` in `fileview.rs`), so a filter added at that
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
        // Past PREVIEW_LINES, so the first preview is truncated. The match
        // sits inside the preview too, so it is visible both before and after
        // the upgrade to a full load.
        let body: String = (0..crate::widgets::fileview::PREVIEW_LINES + 100)
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
            path: dir.join("placeholder").display().to_string(),
            ..Config::default()
        });

        // The nav pane previews the log rather than reading the whole
        // 600-line file. The startup argument names a file that does not
        // exist, so the navigator falls back to the first real entry — which
        // is the log — and `Down` holds it there.
        key(&mut app, KeyCode::Down);

        // Add a filter while the view still only holds the preview.
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "MATCH");
        key(&mut app, KeyCode::Enter);

        let preview_styles = view_line_styles(&app);
        // Length asked of the document; alignment asked of the view. Since #7
        // the style vector covers the *window*, so it is the document that says
        // whether the preview was capped, and the buffer that says whether the
        // styles line up with what is drawn.
        assert_eq!(
            app.document.lines().len(),
            crate::widgets::fileview::PREVIEW_LINES,
            "sanity: preview is capped"
        );
        assert_eq!(
            preview_styles.len(),
            view_lines(&app).len(),
            "styles do not line up with the buffer"
        );
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
            app.document.lines().len(),
            crate::widgets::fileview::PREVIEW_LINES + 100,
            "the preview did not upgrade to a full load"
        );
        assert_eq!(
            styles.len(),
            view_lines(&app).len(),
            "style vector was not resynced to the fully loaded buffer"
        );
        assert!(
            styles[10].is_some(),
            "matching line lost its style after the preview upgraded to a full load"
        );
    }

    /// `Ctrl-f` must reach the file view's own page-down binding, not the
    /// global `f` handler that moves focus to the filter pane.
    #[test]
    fn ctrl_f_scrolls_the_file_view_instead_of_focusing_the_filter_pane() {
        let body = numbered_lines(100);
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
        app.view.textarea.cursor().0
    }

    /// The text the file view is currently showing, one entry per row.
    fn view_lines(app: &App) -> Vec<String> {
        app.view.textarea.lines().to_vec()
    }

    fn view_line_numbers(app: &App) -> Vec<usize> {
        app.view.textarea.line_numbers().to_vec()
    }

    // ---- windowed viewport (#7) -----------------------------------------

    /// Long enough to be windowed at any plausible pane height, short enough
    /// to stay under `PREVIEW_LINES` so nothing here is also testing
    /// truncation. Every tenth line is blank, giving `{` and `}` something to
    /// find.
    fn app_over_long_file(name: &str) -> App<'static> {
        let body: String = (0..LONG_FILE_LINES)
            .map(|i| {
                if i % 10 == 0 {
                    "\n".to_string()
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        let mut app = app_over_file(name, &body);
        // Gives the view a real pane height to size its window against;
        // before the first render it assumes `ASSUMED_PANE_HEIGHT`.
        draw(&mut app);
        focus_file_view(&mut app);
        app
    }

    const LONG_FILE_LINES: usize = 5_000;

    /// **The structural form of #7's memory acceptance criterion.**
    ///
    /// The win is not measured in bytes — that is allocator- and
    /// platform-dependent, and the classic flaky test. What causes the win is
    /// directly assertable instead: the buffer holds a window, however long the
    /// document is. Before this change the two numbers below were equal, and
    /// the file was resident twice.
    #[test]
    fn the_view_holds_a_window_not_the_whole_document() {
        let mut app = app_over_long_file("window_bounded");

        // `G` forces a re-window against the real pane height, so this is not
        // just measuring the pre-render assumption.
        key(&mut app, KeyCode::Char('G'));

        assert_eq!(
            app.document.visible().len(),
            LONG_FILE_LINES,
            "sanity: the document still holds every line"
        );
        let held = view_lines(&app).len();
        // Against the constant, not a literal: the window grew from three
        // screens to five in #108, and a hard-coded 3 here made that read as a
        // regression in the memory criterion rather than the deliberate
        // widening of the slack that it was.
        let span = crate::widgets::fileview::WINDOW_SCREENS * AREA.height as usize;
        assert!(
            held <= span,
            "the buffer holds {held} lines for a {}-row pane — not a window",
            AREA.height
        );
    }

    /// `g` means the document's first line, not the first line of whichever
    /// window happens to be loaded. Without interception this lands on
    /// `window_start`, which looks entirely plausible and is wrong.
    #[test]
    fn g_jumps_to_the_documents_first_line() {
        let mut app = app_over_long_file("window_g_top");
        key(&mut app, KeyCode::Char('G'));
        assert_ne!(cursor_source(&app), 0, "sanity: moved away from the top");

        key(&mut app, KeyCode::Char('g'));

        assert_eq!(cursor_source(&app), 0);
    }

    #[test]
    fn capital_g_jumps_to_the_documents_last_line() {
        let mut app = app_over_long_file("window_g_bottom");

        key(&mut app, KeyCode::Char('G'));

        assert_eq!(cursor_source(&app), LONG_FILE_LINES - 1);
    }

    /// A paragraph move can travel further than the window. Resolved against
    /// the document, `}` finds the next blank line; left to the widget it would
    /// stop at the buffer's edge.
    #[test]
    fn brace_moves_by_paragraph_across_the_whole_document() {
        let mut app = app_over_long_file("window_paragraph");

        key(&mut app, KeyCode::Char('}'));
        // Line 0 is blank and the cursor starts there, so the next blank below
        // is line 10.
        assert_eq!(cursor_source(&app), 10);

        key(&mut app, KeyCode::Char('}'));
        assert_eq!(cursor_source(&app), 20);

        key(&mut app, KeyCode::Char('{'));
        assert_eq!(cursor_source(&app), 10);
    }

    /// The off-by-`window_start` failure this change is most exposed to.
    /// `textarea.cursor().0` indexes the buffer; the source line is
    /// `window_start` further down. Reading it untranslated yields a line
    /// number that is wrong but entirely believable.
    #[test]
    fn the_cursor_reports_its_source_line_from_inside_a_window() {
        let mut app = app_over_long_file("window_cursor_source");

        key(&mut app, KeyCode::Char('G'));

        let view = &app.view;
        assert!(
            view.window_start() > 0,
            "sanity: the end of the file must be a moved window"
        );
        assert_eq!(cursor_source(&app), LONG_FILE_LINES - 1);
    }

    /// A window starting at visible row N must number its gutter from N, not
    /// from 1. This is why the numbers override became unconditional.
    #[test]
    fn the_gutter_numbers_a_window_by_its_source_lines() {
        let mut app = app_over_long_file("window_gutter");

        key(&mut app, KeyCode::Char('G'));

        let numbers = view_line_numbers(&app);
        assert_eq!(
            numbers.last().copied(),
            Some(LONG_FILE_LINES - 1),
            "the last row of the last window must be the file's last line"
        );
        assert!(
            numbers[0] > 0,
            "a window at the end of the file must not renumber from the top"
        );
    }

    /// Paging repeatedly must walk the document, not stall at a window edge.
    /// This is the case the middle-third rule exists for: with a smaller margin
    /// the second page runs into the buffer's end and is silently truncated.
    #[test]
    fn paging_down_repeatedly_walks_past_window_boundaries() {
        let mut app = app_over_long_file("window_paging");
        let mut last = cursor_source(&app);

        for page in 0..40 {
            key(&mut app, KeyCode::PageDown);
            let now = cursor_source(&app);
            assert!(now >= last, "page {page} moved backwards: {last} -> {now}");
            last = now;
        }

        assert!(
            last > 3 * AREA.height as usize,
            "paging never left the first window (reached line {last})"
        );
    }

    /// End to end through the real render path, against a window that has
    /// *moved*. The scroll machinery is the risky part of this change — the
    /// pending-scroll priming render, the viewport reset by `set_lines` — and
    /// the other tests here read the buffer rather than the screen. This one
    /// checks that what is actually painted is the end of the file.
    #[test]
    fn a_moved_window_renders_the_lines_it_holds() {
        let mut app = app_over_long_file("window_render");

        key(&mut app, KeyCode::Char('G'));
        let screen = rendered(&mut app);

        assert!(
            screen.contains(&format!("line {}", LONG_FILE_LINES - 1)),
            "the last line of the file was not painted:\n{screen}"
        );
        assert!(
            !screen.contains("line 1 "),
            "the top of the file is still on screen after jumping to the end:\n{screen}"
        );
    }

    /// `n` targets a row in the visible set, which a windowed buffer does not
    /// contain. Handing that row straight to the textarea clamps it to the
    /// buffer's last line — silently, and nowhere near the match.
    #[test]
    fn n_reaches_a_match_far_outside_the_window() {
        let mut app = app_over_long_file("window_n_far");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "line 4321$");
        key(&mut app, KeyCode::Enter);
        focus_file_view(&mut app);

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(cursor_source(&app), 4321);
    }

    #[test]
    fn an_excluding_filter_removes_its_lines_from_the_view() {
        let mut app = app_over_file("exclude_view", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        assert_eq!(
            view_lines(&app),
            vec!["alpha".to_string(), "gamma".to_string()]
        );
    }

    /// Which rows the gutter is currently marking as ending a group.
    fn view_group_ends(app: &App) -> Vec<bool> {
        app.view
            .textarea
            .line_number_styles()
            .iter()
            .map(Option::is_some)
            .collect()
    }

    /// Issue #2. Hiding butts groups of matches against each other; the mark
    /// on the last row of a group is what says the file did not run
    /// continuously from one to the next.
    #[test]
    fn hiding_marks_the_last_row_of_each_group() {
        let mut app = app_over_file(
            "gap_marks",
            "beta 1\nbeta 2\nalpha\nbeta 3\nbeta 4\ntrailing\n",
        );
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        ctrl(&mut app, KeyCode::Char('h'));

        assert_eq!(view_lines(&app).len(), 4, "the gap was not hidden");
        assert_eq!(view_group_ends(&app), vec![false, true, false, true]);
    }

    /// With nothing hidden there are no gaps, so nothing may be marked — the
    /// mark has to mean something, and a file shown whole has no boundaries
    /// to draw.
    #[test]
    fn nothing_is_marked_while_the_whole_file_is_shown() {
        let mut app = app_over_file("gap_marks_off", "beta\nalpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_lines(&app).len(), 3, "sanity: nothing is hidden");
        assert!(
            view_group_ends(&app).iter().all(|end| !end),
            "a gap was marked with no lines hidden"
        );
    }

    /// The marks are indexed by buffer row, so they must be re-derived
    /// whenever the visible set changes — a stale set would underline rows
    /// that are no longer where a group ends.
    #[test]
    fn lifting_the_hiding_clears_the_marks() {
        let mut app = app_over_file("gap_marks_restore", "beta\nalpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        ctrl(&mut app, KeyCode::Char('h'));
        assert!(
            view_group_ends(&app).iter().any(|&end| end),
            "sanity: hiding marked a gap"
        );

        ctrl(&mut app, KeyCode::Char('h'));

        assert!(
            view_group_ends(&app).iter().all(|end| !end),
            "the marks survived the file coming back whole"
        );
    }

    /// The gutter keeps the original numbering, so a hidden line leaves a gap.
    #[test]
    fn the_gutter_shows_source_line_numbers_when_lines_are_hidden() {
        let mut app = app_over_file("exclude_gutter", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        // 0-based source indices: rows 0 and 2 render as 1 and 3.
        assert_eq!(view_line_numbers(&app), vec![0, 2]);
    }

    #[test]
    fn styles_still_line_up_with_the_rebuilt_buffer() {
        let mut app = app_over_file("exclude_styles", "alpha\nnoise\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
        typed(&mut app, "noise");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_line_styles(&app).len(), view_lines(&app).len());
    }

    /// With nothing excluded the gutter numbers the file straight through.
    ///
    /// This used to assert the override was *absent*, letting the fork number
    /// the buffer 1..N itself. #7 made the override unconditional, because a
    /// windowed buffer starting at visible row 1,000 would otherwise be
    /// numbered 1, 2, 3. The invariant worth protecting was never "no
    /// override" — it was "the numbers are the file's own", which is what this
    /// now checks directly.
    #[test]
    fn without_hiding_the_gutter_numbers_the_file_straight_through() {
        let mut app = app_over_file("no_hiding", "alpha\nbeta\n");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(view_lines(&app).len(), 2);
        assert_eq!(
            view_line_numbers(&app),
            vec![0, 1],
            "the gutter must number an unhidden file straight through"
        );
    }

    /// Lifting the hiding must restore the whole buffer. Leaving a stale subset
    /// behind is worse than never hiding: the remaining rows would renumber
    /// from 1 and claim to be the whole file.
    #[test]
    fn disabling_an_excluding_filter_restores_the_hidden_lines() {
        let mut app = app_over_file("exclude_restore", "alpha\nnoise\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
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
        assert_eq!(
            view_line_numbers(&app),
            vec![0, 1, 2],
            "the gutter must renumber back to the file's own lines"
        );
    }

    /// The cursor's source line, derived from where it sits in the view.
    fn cursor_source(app: &App) -> usize {
        let row = app.view.cursor_visible_row();
        app.document.source_at(row).unwrap_or(row)
    }

    fn cursor_screen_row(app: &App) -> u16 {
        app.view.cursor_screen_row()
    }

    /// Move the cursor to `row` without going through `CursorMove::Jump`,
    /// whose `u16` argument would silently truncate on the large-file test
    /// below.
    fn move_cursor_to_visible_row(app: &mut App, row: usize) {
        let lines = app.view.textarea.lines().to_vec();
        app.view.textarea.set_lines(lines, (row, 0));
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
        // 13 rows, not 12: one goes to the permanent status row, leaving the
        // 12-row bordered pane this test's page arithmetic below depends on.
        let area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 13,
        };
        let body = numbered_lines(200);
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
        key(&mut app, KeyCode::Char('i'));
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
        let body = numbered_lines(200);
        let mut app = app_over_file("scroll_hold", &body);
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
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
        // swapping `x` for `i`) stopped the buffer from actually changing
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
        let body = numbered_lines(200);
        let mut app = app_over_file("scroll_hold_above", &body);
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
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
        let body = numbered_lines(200);
        let mut app = app_over_file("pane_scroll_hold", &body);
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
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
        key(&mut app, KeyCode::Enter);
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
        let body = numbered_lines(20);
        let mut app = app_over_file("round_trip", &body);
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
        // The *document* is whole again. The view holds a window of it since
        // #7, so its length measures the pane rather than the file.
        assert_eq!(
            app.document.visible().len(),
            TOTAL,
            "context did not come back"
        );
    }

    /// Toggling into hidden mode from a line that is not a match snaps forward
    /// to the next one, and toggling back lands on that.
    #[test]
    fn hiding_from_an_unmatched_line_snaps_to_the_next_match() {
        let mut app = app_over_file("snap_to_match", "alpha\nbeta\ngamma\nbeta two\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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

    /// An excluding filter (`x`) removes lines in `Dimmed` mode too — that is
    /// the entire point of it — so the funnel must not be gated on
    /// `FilteredOnly` alone. Before this, `x` matching every line rendered a
    /// blank pane with no indication anything was going on.
    #[test]
    fn the_status_line_shows_a_funnel_for_an_excluding_filter_while_dimmed() {
        let mut app = app_over_file("funnel_dimmed_exclude", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
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

    /// Issue #36's remaining half. `▼` answers "are lines missing right now?",
    /// which is deliberately false here — with nothing including, the #36 guard
    /// in `Document::recompute_visible` shows the whole file. But hide mode
    /// *is* armed: define a filter and the pane visibly starts hiding, with no
    /// second keypress. This is the state the issue was reported against, and
    /// the one the funnel can never cover without lying.
    #[test]
    fn the_badge_shows_hide_mode_with_no_filters_at_all() {
        let mut app = app_over_file("badge_bare", "alpha\nbeta\n");
        assert!(
            !status_line(&mut app).contains(HIDE_BADGE_TEXT.trim()),
            "sanity: no badge before Ctrl-H"
        );

        key(&mut app, KeyCode::Char('H'));

        assert_eq!(
            app.document.mode(),
            Mode::FilteredOnly,
            "sanity: hide mode is armed"
        );
        assert!(
            status_line(&mut app).contains(HIDE_BADGE_TEXT.trim()),
            "hide mode armed with nothing on the row saying so: {}",
            status_line(&mut app)
        );
    }

    #[test]
    fn the_badge_shows_hide_mode_alongside_enabled_filters() {
        let mut app = app_over_file("badge_enabled", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));

        let row = status_line(&mut app);
        assert!(row.contains(HIDE_BADGE_TEXT.trim()), "no badge: {row}");
        assert!(
            row.contains('▼'),
            "sanity: the funnel still fires too: {row}"
        );
    }

    /// The badge tracks the mode, not the filter set, so `!` must not take it
    /// away — the mode is still armed and re-enabling the filters resumes
    /// hiding immediately. This is also where `▼` and the badge visibly
    /// disagree, which is the whole reason they are two indicators.
    #[test]
    fn the_badge_survives_disabling_every_filter() {
        let mut app = app_over_file("badge_disabled", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));
        key(&mut app, KeyCode::Char('H'));

        let row = status_line(&mut app);
        assert!(
            row.contains(HIDE_BADGE_TEXT.trim()),
            "badge went away with the filters, but the mode did not: {row}"
        );
        assert!(
            !row.contains('▼'),
            "sanity: the funnel stays honest and off here: {row}"
        );
    }

    #[test]
    fn no_badge_while_dimming() {
        let mut app = app_over_file("badge_dimmed", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: still dimming");
        let row = status_line(&mut app);
        assert!(
            !row.contains(HIDE_BADGE_TEXT.trim()),
            "badge claimed hide mode while dimming: {row}"
        );
        assert!(row.contains('▼'), "sanity: the funnel is on: {row}");
    }

    /// The issue rejected "a dim tiny icon" by name. Reverse video is what
    /// makes this louder than the `DarkGray` row it sits on, so the styling is
    /// the feature, not decoration — assert on it rather than on the text.
    #[test]
    fn the_badge_is_painted_prominently_not_dimmed() {
        let mut app = app_over_file("badge_style", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('H'));

        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        let y = AREA.height - 1;

        // Every column of the badge, not just one: a colour block with a gap
        // in it is not the thing the issue asked for. Compared field by field
        // rather than as a whole `Style` because a painted cell also carries a
        // default `underline_color` that the constant never mentions.
        for x in 0..HIDE_BADGE_TEXT.chars().count() as u16 {
            let style = buf[(x, y)].style();
            assert_eq!(style.fg, HIDE_BADGE_STYLE.fg, "column {x} foreground");
            assert_eq!(style.bg, HIDE_BADGE_STYLE.bg, "column {x} background");
            assert_eq!(
                style.add_modifier, HIDE_BADGE_STYLE.add_modifier,
                "column {x} modifiers"
            );
            assert!(
                !style.add_modifier.contains(Modifier::DIM),
                "the badge is the dim glyph the issue rejected"
            );
            assert!(
                style.bg.is_some() && style.fg.is_some(),
                "reverse video needs both a foreground and a background"
            );
        }
    }

    /// The badge is drawn ahead of the status text and takes columns from its
    /// budget, so a narrow row has to keep eliding the path from the left
    /// rather than letting anything run past the edge.
    #[test]
    fn a_narrow_row_keeps_the_badge_and_still_elides_the_directory() {
        let mut app = app_over_file("badge_narrow", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('H'));

        let width = 40;
        let row = status_line_at(&mut app, width);

        assert!(row.contains(HIDE_BADGE_TEXT.trim()), "badge dropped: {row}");
        assert!(
            row.chars().count() <= width as usize,
            "the row overflowed {width} columns: {row}"
        );
        assert!(
            row.contains('…'),
            "the directory stopped eliding from the left: {row}"
        );
    }

    /// An open prompt shares the row rather than displacing the badge. Hide
    /// mode is armed while a filter is being typed, and typing one is exactly
    /// when the pane is about to change under you.
    #[test]
    fn an_open_prompt_still_shows_the_badge() {
        let mut app = app_over_file("badge_prompt", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('H'));

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "foo");

        let row = status_line(&mut app);
        assert!(row.contains(HIDE_BADGE_TEXT.trim()), "badge dropped: {row}");
        assert!(row.contains("filter: foo"), "prompt lost: {row}");
    }

    /// An excluding-only filter set never produces an `Included` verdict, so
    /// the old `match_count`-based report always read "0 matched" even while
    /// visibly removing lines — indistinguishable from the filter matching
    /// nothing at all. The status line must describe what is on screen.
    #[test]
    fn the_status_line_reports_lines_shown_not_matched() {
        let mut app = app_over_file("status_shown_not_matched", "alpha\nnoise\ngamma\n");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
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

    /// Issue #36's guard made this state impossible: with nothing including
    /// (no numbered filters and no search), hiding shows the whole file
    /// rather than blanking it, so the status line must not claim the two
    /// are in tension.
    #[test]
    fn hiding_with_no_filters_does_not_claim_the_file_is_empty() {
        let mut app = app_over_file("status_no_filters", "alpha\nbeta\n");

        key(&mut app, KeyCode::Char('H'));

        let status = status_line(&mut app);
        assert!(
            !status.to_lowercase().contains("nothing to show"),
            "the status line still reports a blank pane that no longer happens: {status}"
        );
    }

    /// A live search with no numbered filters is still one filter as far as
    /// this row is concerned. `self.filters.len()` alone counts only the
    /// numbered set and would report "0 filters" while a search was visibly
    /// active and changing what's on screen — the regression this pins.
    #[test]
    fn a_search_alone_counts_as_one_filter_on_the_status_line() {
        let mut app = app_over_file("status_search_only", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let status = status_line(&mut app);

        assert!(
            status.contains("1 filter") && !status.contains("1 filters"),
            "a lone search is not counted, or not counted singularly: {status}"
        );
        assert!(
            !status.contains("0 filters"),
            "the search-only status line still claims there are no filters: {status}"
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
        // `../` is the probe for "the navigator is drawn": every listing has
        // a parent entry, and the block title cannot contain `..` because
        // `set_dir` collapses it (#78). This used to look for the `>>`
        // selection marker, which no longer exists — leaving the assertion
        // vacuously true.
        assert!(!after.contains("../"), "the navigator is still on screen");
    }

    /// `b` restores the split, but deliberately leaves the cursor where it
    /// moved it: you pressed `b` to read the file, so being dropped back into
    /// the navigator on the way out would be the surprise. `e` is the way back.
    #[test]
    fn b_toggles_back_but_leaves_focus_in_the_file_view() {
        let mut app = app_over_file("zoom_b_back", "alpha\n");
        assert_eq!(app.focus, Focus::Nav);

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
            app.focus,
            Focus::View,
            "focus was dragged back to the navigator"
        );
        assert_eq!(app.zoom, None);
    }

    /// Hiding the column the cursor is in must move focus somewhere visible,
    /// or the user is left typing into a pane that is not on screen.
    #[test]
    fn b_moves_focus_out_of_the_hidden_column() {
        let mut app = app_over_file("zoom_b_focus", "alpha\n");
        assert_eq!(app.focus, Focus::Nav, "starts in the navigator");

        key(&mut app, KeyCode::Char('b'));

        assert_eq!(app.focus, Focus::View);
    }

    /// `e` is how you get back, so it must work from a hidden state.
    #[test]
    fn e_reveals_the_left_column_and_focuses_it() {
        let mut app = app_over_file("zoom_e", "alpha\n");
        key(&mut app, KeyCode::Char('b'));

        key(&mut app, KeyCode::Char('e'));

        assert_eq!(app.zoom, None, "the left column is still hidden");
        assert_eq!(app.focus, Focus::Nav);
    }

    #[test]
    fn e_focuses_the_navigator_even_when_nothing_is_hidden() {
        let mut app = app_over_file("zoom_e_visible", "alpha\n");
        focus_file_view(&mut app);
        assert_ne!(app.focus, Focus::Nav);

        key(&mut app, KeyCode::Char('e'));

        assert_eq!(app.focus, Focus::Nav);
    }

    /// `t` reaches the text pane directly, the way `e` reaches the explorer.
    #[test]
    fn t_focuses_the_file_view() {
        let mut app = app_over_file("focus_t", "alpha\n");
        assert_ne!(app.focus, Focus::View);

        key(&mut app, KeyCode::Char('t'));

        assert_eq!(app.focus, Focus::View);
    }

    /// `f` reaches the filter pane rather than opening a filter prompt.
    ///
    /// Creating a filter moves inside the pane (`i` / `x`), which costs a
    /// keystroke from elsewhere and buys one focus key per pane.
    #[test]
    fn f_focuses_the_filter_pane() {
        let mut app = app_over_file("focus_f", "alpha\n");
        assert_ne!(app.focus, Focus::Filters);

        key(&mut app, KeyCode::Char('f'));

        assert_eq!(app.focus, Focus::Filters);
        assert!(
            app.search.is_none(),
            "f opened a prompt instead of moving focus"
        );
    }

    /// `x` is the exclude half of the pair. `e` would read better and cannot
    /// be used — the global match runs first, so a bare `e` never reaches the
    /// filter pane at all.
    #[test]
    fn x_opens_an_excluding_filter_prompt_in_the_filter_pane() {
        let mut app = app_over("exclude_prompt", &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
        typed(&mut app, "noise");

        assert_eq!(prompt_line(&mut app), "exclude: noise");
    }

    /// `i` and `x` belong to the filter pane, not to the app. Bound globally
    /// they would swallow a keystroke from every other pane — which is exactly
    /// what `f` and `F` used to do, and the reason they moved.
    #[test]
    fn i_and_x_do_nothing_outside_the_filter_pane() {
        for (code, name) in [
            (KeyCode::Char('i'), "filter_keys_scoped_i"),
            (KeyCode::Char('x'), "filter_keys_scoped_x"),
        ] {
            // A fixture directory per iteration: `claim_fixture_dir` panics on
            // a reused name, deliberately, so tests cannot race over one.
            let mut app = app_over_file(name, "alpha\n");

            // The navigator has focus at startup.
            key(&mut app, code);
            assert!(
                app.search.is_none(),
                "{code:?} opened a prompt from the navigator"
            );

            key(&mut app, KeyCode::Char('t'));
            key(&mut app, code);
            assert!(
                app.search.is_none(),
                "{code:?} opened a prompt from the file view"
            );
        }
    }

    /// `F` created an excluding filter and is retired; `x` in the pane does it
    /// now. Pinned so the old binding cannot quietly come back alongside the
    /// new one and leave two ways to do the same thing.
    #[test]
    fn capital_f_no_longer_opens_a_prompt() {
        let mut app = app_over_file("capital_f_retired", "alpha\n");

        key(&mut app, KeyCode::Char('F'));

        assert!(app.search.is_none(), "F still opens a prompt");
    }

    /// The navigator's own top-left corner, as symbol and style.
    ///
    /// The corner is the probe because it is drawn by the border and by
    /// nothing else — a row's highlight, a title, and the pane's contents all
    /// stay out of it.
    fn nav_corner(app: &mut App) -> (String, Style) {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        (buf[(0, 0)].symbol().to_string(), buf[(0, 0)].style())
    }

    /// Focus has to be visible on the pane, not just on the row inside it.
    ///
    /// Colour *and* weight, deliberately: this is the argument #19 makes about
    /// the selection marker. A single channel fails on a theme with weak
    /// contrast and for a colour-blind reader, and the cue this replaces —
    /// a green foreground on one already-reversed row — was exactly that.
    #[test]
    fn the_focused_pane_border_differs_in_colour_and_weight() {
        let mut app = app_over_file("focus_border", "alpha\n");
        // The navigator holds focus at startup.
        let (focused_symbol, focused_style) = nav_corner(&mut app);

        key(&mut app, KeyCode::Char('t'));
        let (unfocused_symbol, unfocused_style) = nav_corner(&mut app);

        assert_ne!(
            focused_style.fg, unfocused_style.fg,
            "the border colour is the same focused and unfocused"
        );
        assert_ne!(
            focused_symbol, unfocused_symbol,
            "the border weight is the same focused and unfocused, so the cue \
             is colour alone"
        );
    }

    /// `z` maximises whatever has focus — including the navigator, for long
    /// filenames.
    #[test]
    fn z_zooms_the_navigator_when_it_has_focus() {
        // A distinctive marker, not "alpha": the navigator titles its block
        // with the absolutised checkout path, which could itself contain
        // "alpha" on some checkout — the negative assertion below would then
        // pass or fail depending on where the repo happens to be checked
        // out, rather than on what the test claims to check.
        let mut app = app_over_file("zoom_z_nav", "ZOOMMARKER\n");

        key(&mut app, KeyCode::Char('z'));

        let after = rendered(&mut app);
        // See `b_hides_the_left_column` for why `../` is the probe.
        assert!(after.contains("../"), "the navigator is not on screen");
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
            path: file.display().to_string(),
            ..Config::default()
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
        assert_eq!(with_z.focus, with_b.focus);
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
        assert_eq!(
            app.dragging,
            Some(Divider::Vertical),
            "sanity: the divider click started a drag"
        );

        key(&mut app, KeyCode::Char('b'));
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 60);

        assert_eq!(app.dragging, None, "the drag survived into the zoom");
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

        assert_eq!(app.zoom, Some(app.focus), "focus moved off the zoomed pane");
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
        key(&mut app, KeyCode::Char('i'));
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
    /// The pane is on screen whenever the navigator is, filters or not — so a
    /// user who has never pressed `f i` still sees where filters will appear,
    /// and the layout does not shift under them the first time they add one.
    fn the_filter_pane_is_present_before_any_filter_is_defined() {
        let mut app = app_over_file("pane_absent", "alpha\n");

        let empty = rendered(&mut app);
        assert!(empty.contains("Filters"), "no filter pane before a filter");
        assert!(empty.contains("f i"), "empty pane drew no hint: {empty}");

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        let populated = rendered(&mut app);
        assert!(populated.contains("Filters"));
        assert!(
            !populated.contains("f i"),
            "hint outlived the empty pane: {populated}"
        );
    }

    #[test]
    fn the_filter_pane_lists_the_patterns() {
        let mut app = app_over_file("pane_lists", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
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
    /// The pane is always on screen now, so `Tab` always stops on it. The
    /// cycle is three panes deep whether or not a filter exists — the rule is
    /// "visible means focusable", with no special case for an empty set.
    fn tab_reaches_the_filter_pane_while_it_is_empty() {
        let mut app = app_over_file("pane_focus", "alpha\n");
        draw(&mut app);
        assert!(app.filters.is_empty(), "precondition: no filters");

        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);

        assert_eq!(
            app.focus,
            Focus::Filters,
            "Tab skipped the empty filter pane"
        );

        key(&mut app, KeyCode::Tab);

        assert_eq!(
            app.focus,
            Focus::Nav,
            "focus did not return to the navigator"
        );
    }

    #[test]
    fn tab_reaches_the_filter_pane_once_a_filter_exists() {
        let mut app = app_over_file("pane_focus_on", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        draw(&mut app);

        let mut seen = vec![app.focus];
        for _ in 0..3 {
            key(&mut app, KeyCode::Tab);
            seen.push(app.focus);
        }

        assert!(
            seen.contains(&Focus::Filters),
            "the filter pane never took focus: {seen:?}"
        );
    }

    /// The README documents the exact order `Tab` cycles the panes in
    /// (navigator, file view, filter pane), which follows `Focus::next`
    /// rather than anything visual — the filter pane sits *above* the file
    /// view on screen but *after* it in the cycle. Nothing else pins that
    /// order, so a reshuffle of `Focus::next` would otherwise leave the
    /// README quietly wrong with an otherwise green suite.
    ///
    /// Asserting against named `Focus` variants is what makes a reordering
    /// break this test loudly. The bare `0`/`1`/`2` indices this used to
    /// compare would have gone on passing while meaning something new (#73).
    #[test]
    fn tab_cycles_navigator_then_file_view_then_filter_pane() {
        let mut app = app_over_file("tab_cycle_order", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        // Creating a filter now *leaves* focus on the filter pane — `f` moved
        // it there. This test is about the cycle, not about where creating a
        // filter lands, so come back to the navigator deliberately rather than
        // assuming the setup left focus untouched.
        key(&mut app, KeyCode::Char('e'));
        draw(&mut app);

        assert_eq!(
            app.focus,
            Focus::Nav,
            "should start focused on the navigator"
        );

        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.focus,
            Focus::View,
            "one Tab from the navigator should reach the file view"
        );

        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.focus,
            Focus::Filters,
            "two Tabs from the navigator should reach the filter pane"
        );

        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.focus,
            Focus::Nav,
            "three Tabs should cycle back to the navigator"
        );
    }

    /// `App::render` has two branches that draw a pane — the ordinary split
    /// and the zoom special case — and only the split branch is exercised by
    /// the tests above. Both go through `render_pane`, which is what hands
    /// the filter pane the `ActiveFilters` it cannot hold itself; a zoom
    /// branch that bypassed it would focus an invisible pane showing a blank
    /// screen.
    #[test]
    fn zooming_the_filter_pane_shows_its_contents() {
        let mut app = app_over_file("zoom_filter_pane", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        // Bounded, like `focus_file_view`, rather than an unbounded `while`:
        // if the filter pane ever stopped being reachable, a `while` here
        // would hang the test instead of failing it.
        for _ in 0..PANE_COUNT {
            if app.focus == Focus::Filters {
                break;
            }
            key(&mut app, KeyCode::Tab);
        }
        assert_eq!(
            app.focus,
            Focus::Filters,
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
        key(&mut app, KeyCode::Char('i'));
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
            key(&mut app, KeyCode::Char('i'));
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
            key(&mut app, KeyCode::Char('i'));
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
            key(&mut app, KeyCode::Char('i'));
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

    /// #44: the pane is a headline feature and read as an afterthought at the
    /// three rows an empty set asked for — a title, one line of hint, a
    /// border. It now opens at `MIN_AUTO_FILTER_HEIGHT` whatever it holds, so
    /// the room to define filters in is visible before the first one exists.
    ///
    /// Driven through `filter_pane_split_height` rather than a render, for
    /// the same reason the two tests above are: the arithmetic is the claim,
    /// and inspecting cells to recover a height only obscures which floor
    /// produced it.
    #[test]
    fn an_empty_filter_pane_still_opens_at_its_starting_height() {
        let app = app_over_file("empty_filter_height", "alpha\n");

        assert_eq!(app.filters.row_count(), 0, "the fixture defined a filter");
        // 40 rows is comfortably clear of both caps (half is 20, the
        // navigator's floor leaves 37), so the floor is unambiguously what
        // this measures.
        assert_eq!(
            app.filter_pane_split_height(40),
            MIN_AUTO_FILTER_HEIGHT,
            "an empty pane did not claim its starting height"
        );
    }

    /// #44's claim is visual, and every other test of it drives
    /// `filter_pane_split_height` directly — a method the renderer merely
    /// calls, and could stop calling. This one goes through `render` on an
    /// ordinary terminal and reads back the rect the pane was actually
    /// handed.
    #[test]
    fn an_empty_filter_pane_renders_at_its_starting_height() {
        let mut app = app_over("empty_filter_render", &["a.rs"]);
        let tall = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        };
        let mut buf = Buffer::empty(tall);

        (&mut app).render(tall, &mut buf);

        assert_eq!(app.filter_area.height, MIN_AUTO_FILTER_HEIGHT);
        assert_eq!(
            app.filter_area.bottom(),
            tall.height - 1,
            "the pane is not sitting on the bottom of the left column \
             (the status row is the one below it)"
        );
    }

    /// The starting height is a floor, not a fixed size: a set larger than it
    /// still gets the rows it asks for. Guards the difference between
    /// `.max(MIN_AUTO_FILTER_HEIGHT)` and assigning it, which the test above
    /// alone cannot tell apart.
    #[test]
    fn the_starting_height_does_not_cap_a_larger_filter_set() {
        let mut app = app_over_file("tall_filter_set", "alpha\n");
        for i in 0..12 {
            key(&mut app, KeyCode::Char('f'));
            key(&mut app, KeyCode::Char('i'));
            typed(&mut app, &format!("f{i}"));
            key(&mut app, KeyCode::Enter);
        }

        // 12 filters want 14 rows, and a 40-row column can spare them.
        assert_eq!(
            app.filter_pane_split_height(40),
            12 + 2,
            "the starting height capped a set that asked for more"
        );
    }

    /// The starting height is the pane's *preference*, so it is subject to
    /// the same two caps `preferred_height` always was — it does not get to
    /// claim eight rows out of a twelve-row column just because it is a
    /// floor. Here the half cap is the tighter of the two and governs.
    #[test]
    fn the_starting_height_still_yields_to_the_half_cap() {
        let app = app_over_file("empty_filter_short", "alpha\n");

        assert_eq!(
            app.filter_pane_split_height(12),
            6,
            "the starting height overrode the half cap"
        );
    }

    fn app_with_two_filters(name: &str) -> App<'static> {
        let mut app = app_over_file(name, "alpha\nbeta\ngamma\n");
        for pattern in ["alpha", "beta"] {
            key(&mut app, KeyCode::Char('f'));
            key(&mut app, KeyCode::Char('i'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
        }
        settle(&mut app);
        app
    }

    /// Disarm the bounce guard the setup above leaves behind (#48).
    ///
    /// The helper's last keystroke is the `Enter` that commits a pattern, which
    /// arms the guard — so without this the *first* `Enter` a test presses is
    /// swallowed as a bounce, and every `Enter`-driven test would be asserting
    /// against the guard rather than the binding.
    ///
    /// `Esc` because it moves no selection and, with no live search set, is a
    /// genuine no-op: its arm is guarded on `clear_search()` reporting that
    /// there was something to clear.
    fn settle(app: &mut App) {
        key(app, KeyCode::Esc);
    }

    #[test]
    fn enter_toggles_the_selected_filter_from_the_pane() {
        let mut app = app_with_two_filters("pane_toggle");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Enter);

        assert!(!app.filters.filters()[0].enabled);
    }

    /// Toggling must re-evaluate: the view is what the pane is controlling.
    #[test]
    fn toggling_a_filter_restyles_the_view() {
        let mut app = app_with_two_filters("pane_toggle_view");
        focus_filter_pane(&mut app);
        let before = view_line_styles(&app);

        key(&mut app, KeyCode::Enter);

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

    /// `c` reopens the prompt over the selected filter's own pattern. Starting
    /// it empty would be no better than `d` then `f i`, which is the retyping
    /// this binding exists to remove.
    #[test]
    fn c_opens_the_prompt_prefilled_with_the_selected_pattern() {
        let mut app = app_with_two_filters("pane_edit_prefill");
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('j'));

        key(&mut app, KeyCode::Char('c'));

        let prompt = app.search.as_ref().expect("the prompt should be open");
        assert_eq!(prompt.pattern, "beta");
        assert_eq!(
            prompt.line(),
            "filter: beta",
            "an edit should read like the `i` that would have created it"
        );
    }

    /// The point of the whole issue: the edited filter keeps its slot, so it
    /// keeps its colour and its precedence. Deleting and retyping put the
    /// replacement at the end and silently reordered the set.
    #[test]
    fn committing_an_edit_replaces_the_pattern_in_place() {
        let mut app = app_with_two_filters("pane_edit_commit");
        focus_filter_pane(&mut app);
        let colour = app.filters.filters()[0].style;

        key(&mut app, KeyCode::Char('c'));
        typed(&mut app, "X");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.filters.len(), 2, "the edit added a filter");
        assert_eq!(app.filters.filters()[0].pattern.as_str(), "alphaX");
        assert_eq!(
            app.filters.filters()[0].style,
            colour,
            "the filter lost its colour, so it moved"
        );
        assert_eq!(app.filters.filters()[1].pattern.as_str(), "beta");
        assert!(app.search.is_none(), "the prompt should have closed");
    }

    /// The pattern decides which lines match, so an edit owes the document a
    /// full `evaluate` — not the `recompute_visible` a mode flip gets away
    /// with. Without it the pane shows the new pattern over the old pattern's
    /// colouring.
    #[test]
    fn committing_an_edit_restyles_the_view() {
        let mut app = app_over_file("pane_edit_view", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        let before = view_line_styles(&app);

        key(&mut app, KeyCode::Char('c'));
        for _ in 0.."alpha".len() {
            key(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "gamma");
        key(&mut app, KeyCode::Enter);

        assert_ne!(
            view_line_styles(&app),
            before,
            "the view still reflects the old pattern"
        );
    }

    /// Same contract `f i` has: the prompt stays open over an intact filter, so
    /// the typo can be corrected rather than retyped from nothing.
    #[test]
    fn an_invalid_edit_reports_and_leaves_the_filter_alone() {
        let mut app = app_with_two_filters("pane_edit_invalid");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('c'));
        typed(&mut app, "[");
        key(&mut app, KeyCode::Enter);

        let prompt = app.search.as_ref().expect("the prompt should stay open");
        assert_eq!(prompt.line(), INVALID_PATTERN);
        assert_eq!(
            app.filters.filters()[0].pattern.as_str(),
            "alpha",
            "a rejected pattern overwrote the filter"
        );
    }

    #[test]
    fn escape_abandons_an_edit_and_leaves_the_filter_untouched() {
        let mut app = app_with_two_filters("pane_edit_escape");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('c'));
        typed(&mut app, "X");
        key(&mut app, KeyCode::Esc);

        assert!(app.search.is_none());
        assert_eq!(app.filters.filters()[0].pattern.as_str(), "alpha");
    }

    /// Backspacing past the start cancels the prompt, as in vim — and a
    /// pre-filled prompt is the first place that rule is reachable by
    /// *deleting what was already there*. It must abandon the edit, exactly as
    /// `Esc` does, rather than commit an empty pattern: an empty regex matches
    /// every line, so the filter would silently start colouring the whole file.
    #[test]
    fn backspacing_an_edit_away_cancels_it_rather_than_emptying_the_filter() {
        let mut app = app_with_two_filters("pane_edit_backspace");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('c'));
        for _ in 0..="alpha".len() {
            key(&mut app, KeyCode::Backspace);
        }

        assert!(app.search.is_none(), "the prompt should have cancelled");
        assert_eq!(
            app.filters.filters()[0].pattern.as_str(),
            "alpha",
            "backspacing out of the prompt emptied the filter"
        );
    }

    /// An edit changes the pattern and nothing else. The sense in particular
    /// has to survive, and it is what the prompt's sigil must report — an
    /// excluding filter editing under a `filter:` prompt would read as though
    /// committing were about to turn it into an including one.
    #[test]
    fn editing_an_excluding_filter_keeps_its_sense() {
        let mut app = app_over_file("pane_edit_exclude", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('x'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('c'));
        assert_eq!(
            app.search.as_ref().expect("prompt open").line(),
            "exclude: alpha"
        );
        typed(&mut app, "X");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.filters.filters()[0].sense, filter::Sense::Exclude);
        assert_eq!(app.filters.filters()[0].pattern.as_str(), "alphaX");
    }

    /// A filter the user had switched off must not come back on just because
    /// its pattern was corrected.
    #[test]
    fn editing_preserves_the_filters_enabled_state() {
        let mut app = app_with_two_filters("pane_edit_enabled");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('c'));
        typed(&mut app, "X");
        key(&mut app, KeyCode::Enter);

        assert!(
            !app.filters.filters()[0].enabled,
            "the edit switched a disabled filter back on"
        );
    }

    /// The same modifier guard `Ctrl-D` and `Ctrl-Space` get. `Ctrl-C` is the
    /// interrupt every terminal user has in their fingers, and it must not
    /// open a prompt that then swallows every following key.
    #[test]
    fn ctrl_c_does_not_open_an_edit_prompt() {
        let mut app = app_with_two_filters("pane_ctrl_c");
        focus_filter_pane(&mut app);

        app.handle_event(event::Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();

        assert!(app.search.is_none(), "Ctrl-C opened the edit prompt");
    }

    /// With no filters the pane draws its hint and has no selection, so `c` has
    /// nothing to address. It must be inert rather than opening a prompt whose
    /// `Enter` would silently do nothing.
    #[test]
    fn c_on_an_empty_set_opens_nothing() {
        let mut app = app_over_file("pane_edit_empty", "alpha\n");
        key(&mut app, KeyCode::Char('f'));

        key(&mut app, KeyCode::Char('c'));

        assert!(app.search.is_none());
    }

    /// Same defect, for the toggle binding.
    #[test]
    fn ctrl_enter_does_not_toggle_the_selected_filter() {
        let mut app = app_with_two_filters("pane_ctrl_space");
        focus_filter_pane(&mut app);

        app.handle_event(event::Event::Key(KeyEvent::new(
            KeyCode::Enter,
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
    /// naive patch (splice the `ActiveFilters` but leave cached verdicts alone),
    /// the stale `Verdict::Included(1)` left over from the deleted filter's
    /// own line would index *past* the now-length-1 `ActiveFilters` — `None`,
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
            key(&mut app, KeyCode::Char('i'));
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
    /// Deleting the last filter used to collapse the pane, which forced focus
    /// off it. The pane now stays, so focus stays too — moving it would be a
    /// jump the user did not ask for, and the pane they are looking at is
    /// still on screen and still the one they were working in.
    fn deleting_the_last_filter_keeps_the_pane_and_the_focus() {
        let mut app = app_over_file("pane_delete_last", "alpha\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('d'));
        draw(&mut app);

        assert!(app.filters.is_empty());
        let text = rendered(&mut app);
        assert!(
            text.contains("Filters"),
            "pane vanished with its last filter"
        );
        assert!(text.contains("f i"), "pane lost its empty hint");
        assert!(
            app.focus == Focus::Filters,
            "focus was moved off a pane that is still on screen"
        );
    }

    /// Deleting the last filter while the pane is zoomed must not leave
    /// `App::zoom` naming a pane focus has moved off. `App::render` also
    /// carries `debug_assert_eq!(zoomed, self.focus)` for exactly
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
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "alpha");
        key(&mut app, KeyCode::Enter);
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('z'));
        assert_eq!(app.zoom, Some(app.focus), "z did not zoom the filter pane");

        key(&mut app, KeyCode::Char('d'));

        assert!(
            app.zoom.is_none() || app.zoom == Some(app.focus),
            "zoom ({:?}) outlived the pane it named (focus = {:?})",
            app.zoom,
            app.focus
        );
        assert!(
            app.focus == Focus::Filters,
            "focus left the filter pane, which deleting its last filter no longer collapses"
        );

        // The disjunction above is satisfiable by a blank frame that merely
        // avoids naming the wrong pane; this pins the stronger claim that
        // whatever the zoom now points at is genuinely drawn, not empty.
        // Focus stays on the filter pane now that its last filter no longer
        // collapses it, so the zoomed pane is the filter pane — it used to be
        // the navigator, which is why this looked for a filename before.
        let text = rendered(&mut app);
        assert!(
            text.contains("Filters") && text.contains("press f"),
            "the pane the zoom now names is not actually on screen: {text}"
        );
    }

    #[test]
    fn j_and_k_move_the_filter_selection() {
        let mut app = app_with_two_filters("pane_select");
        focus_filter_pane(&mut app);

        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Enter);

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
        key(&mut app, KeyCode::Enter);

        assert!(
            !app.filters.filters()[0].enabled,
            "k did not move the selection back"
        );
        assert!(
            !app.filters.filters()[1].enabled,
            "the wrong filter was toggled after k"
        );
    }

    /// `n` walks the union of filter hits and search hits, in source order. This
    /// is the whole point of the design: one notion of an interesting line.
    #[test]
    fn n_steps_between_filter_and_search_matches_alike() {
        let mut app = app_over_file("n_union", "alpha\nERROR one\nbeta\ntimeout two\ngamma\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("ERROR").expect("valid pattern");
        app.filters.set_search("timeout").expect("valid pattern");
        app.refresh_view();

        key(&mut app, KeyCode::Char('n'));
        assert_eq!(cursor_source(&app), 1, "did not reach the filter match");

        key(&mut app, KeyCode::Char('n'));
        assert_eq!(cursor_source(&app), 3, "did not reach the search match");
    }

    /// Line-oriented, not span-oriented: three hits on one line is one stop.
    /// `recon` is a line-focused tool, and the alternative cannot be explained
    /// without explaining the implementation.
    #[test]
    fn n_stops_once_on_a_line_with_several_matches() {
        let mut app = app_over_file("n_once", "foo foo foo\nbar\nfoo\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.set_search("foo").expect("valid pattern");
        app.refresh_view();

        key(&mut app, KeyCode::Char('n'));
        assert_eq!(cursor_source(&app), 2, "stopped more than once on line 0");
    }

    /// The cursor starts *past* the only hit, so landing back on it requires
    /// an actual wrap through index 0 — not merely "the cursor never moved",
    /// which is what the previous version of this test asserted (cursor
    /// already sat on the sole hit, so it passed even with `n` unbound).
    #[test]
    fn n_wraps_at_the_end_of_the_file() {
        let mut app = app_over_file("n_wrap", "hit\nplain\nplain\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        app.filters.set_search("hit").expect("valid pattern");
        app.refresh_view();
        assert_eq!(cursor_source(&app), 2, "sanity: cursor starts past the hit");

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(cursor_source(&app), 0, "did not wrap around to the hit");
    }

    /// Three hits, cursor on the middle one: forward and backward from there
    /// land on different lines (4 and 0 respectively), so this actually
    /// exercises direction. The previous version started at row 0 with only
    /// two hits, where forward and backward both wrap to the same place —
    /// it would have passed even if `N` were wired to walk forward.
    #[test]
    fn capital_n_walks_backwards() {
        let mut app = app_over_file("n_back", "hit a\nplain\nhit b\nplain\nhit c\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        app.filters.set_search("hit").expect("valid pattern");
        app.refresh_view();
        assert_eq!(
            cursor_source(&app),
            2,
            "sanity: cursor starts on the middle hit"
        );

        key(&mut app, KeyCode::Char('N'));

        assert_eq!(
            cursor_source(&app),
            0,
            "N did not walk backwards to the earlier hit (forward would reach 4)"
        );
    }

    /// Quiet, not a panic and not a jump to line 0.
    #[test]
    fn n_with_nothing_interesting_does_nothing() {
        let mut app = app_over_file("n_empty", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('j'));
        let before = cursor_source(&app);

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(cursor_source(&app), before);
    }

    /// `n` belongs to the file view. Hoisting it into `App` must not make it
    /// global — in the navigator it is still the navigator's key.
    ///
    /// `app_over` writes a single-line "x" into every fixture file, which
    /// made the previous version of this test vacuous: with one line, the
    /// only hit is already on row 0, so a leaked, fully global `n` binding
    /// would still be a no-op and the test would pass regardless. This fixture
    /// puts the hit on row 1, so a leak is observable as the cursor moving.
    #[test]
    fn n_in_the_navigator_does_not_move_the_file_view_cursor() {
        claim_fixture_dir("n_nav");
        let dir = std::path::Path::new("target/test-appdirs").join("n_nav");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        fs::write(dir.join("alpha.log"), "alpha\nx\n").expect("write fixture");

        let mut app = App::new(&Config {
            path: dir.join("placeholder").display().to_string(),
            ..Config::default()
        });
        // The startup argument names a file that does not exist, so `App::new`
        // loads that (an error message, not real content) and the navigator
        // falls back to selecting the one real entry. `Down` is what actually
        // previews it into the file view — the same two-step construction
        // `n_promotes_a_truncated_preview_before_stepping` uses, for the same
        // reason.
        key(&mut app, KeyCode::Down);
        app.filters.set_search("x").expect("valid pattern");
        app.refresh_view();
        key(&mut app, KeyCode::Char('e'));
        let before = cursor_source(&app);
        assert_eq!(before, 0, "sanity: cursor starts above the hit on row 1");

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(cursor_source(&app), before, "n leaked out of the file view");
    }

    #[test]
    fn slash_sets_the_search_filter_and_moves_to_its_first_hit() {
        let mut app = app_over_file("slash_filter", "alpha\nbeta\ngamma\nbeta again\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert!(
            app.filters.search().is_some(),
            "the search did not become a filter"
        );
        assert_eq!(cursor_source(&app), 1);
    }

    /// A search survives loading another file, exactly as the numbered filters
    /// do — it is one of them now.
    #[test]
    fn the_search_filter_survives_a_file_load() {
        let mut app = app_over("slash_survives", &["alpha.log", "beta.log"]);
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "x");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('e'));
        key(&mut app, KeyCode::Char('j'));

        assert!(
            app.filters.search().is_some(),
            "the search did not outlive the load"
        );
    }

    /// With hiding on, a bare search is an instant grep — the capability the
    /// merge unlocks.
    #[test]
    fn a_search_with_hiding_on_collapses_the_file_to_its_matches() {
        let mut app = app_over_file("slash_grep", "alpha\nbeta\ngamma\nbeta again\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('H'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.document.visible(), &[1, 3]);
    }

    #[test]
    fn an_invalid_search_pattern_leaves_the_prompt_open() {
        let mut app = app_over_file("slash_bad", "alpha\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "[");
        key(&mut app, KeyCode::Enter);

        assert!(app.search.is_some(), "prompt closed on an invalid pattern");
        assert!(
            app.filters.search().is_none(),
            "a rejected pattern became a filter"
        );
    }

    #[test]
    fn escape_clears_the_search_filter() {
        let mut app = app_over_file("esc_clears", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert!(app.filters.search().is_some(), "sanity: search set");

        key(&mut app, KeyCode::Esc);

        assert!(app.filters.search().is_none());
    }

    /// Issue #36's guard is what makes this safe: clearing the last thing that
    /// was including must not leave a blank pane behind.
    #[test]
    fn escape_while_hiding_restores_the_file_rather_than_blanking_it() {
        let mut app = app_over_file("esc_hiding", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('H'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.document.visible(), &[1], "sanity: grepped down");

        key(&mut app, KeyCode::Esc);

        assert_eq!(app.document.visible(), &[0, 1, 2], "the pane went blank");
    }

    /// An open prompt still wins: Esc there cancels the prompt, as it always has,
    /// rather than reaching past it to delete an established search.
    #[test]
    fn escape_in_an_open_prompt_still_cancels_the_prompt() {
        let mut app = app_over_file("esc_prompt", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "gamma");

        key(&mut app, KeyCode::Esc);

        assert!(app.search.is_none(), "the prompt did not close");
        assert!(
            app.filters.search().is_some(),
            "Esc reached past the prompt"
        );
    }

    #[test]
    fn escape_with_no_search_does_nothing() {
        let mut app = app_over_file("esc_noop", "alpha\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        app.refresh_view();

        key(&mut app, KeyCode::Esc);

        assert_eq!(app.filters.len(), 1, "Esc touched the numbered filters");
    }

    /// `refresh_view` runs `Document::evaluate`, which is O(lines × filters)
    /// — not free — and Esc is a key people tap out of habit, so clearing an
    /// empty search must not pay for it. Nothing in the filter set itself
    /// tells "refreshed and found nothing new" apart from "never refreshed",
    /// so this reaches for `apply_view`'s own tell instead: it only
    /// overwrites `last_visible` when the visible set it just computed
    /// differs from what is already there. Seeding a value that can never
    /// match the real one means a leftover mismatch after Esc is direct
    /// evidence that `refresh_view` never ran.
    #[test]
    fn escape_with_no_search_does_not_refresh() {
        let mut app = app_over_file("esc_no_refresh", "alpha\n");
        key(&mut app, KeyCode::Char('t'));
        app.last_visible = Some(vec![usize::MAX]);

        key(&mut app, KeyCode::Esc);

        assert_eq!(
            app.last_visible,
            Some(vec![usize::MAX]),
            "Esc refreshed the view with nothing to clear"
        );
    }

    /// Probe, keep, probe again — a filter set assembled without retyping a
    /// regex that was hard to get right. Feeds #8.
    #[test]
    fn p_promotes_the_search_and_frees_the_slot() {
        let mut app = app_over_file("p_promote", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('p'));

        assert_eq!(app.filters.len(), 1, "the search did not become a filter");
        assert!(
            app.filters.search().is_none(),
            "the slot is not free for the next probe"
        );
    }

    #[test]
    fn two_probes_promote_into_two_filters() {
        let mut app = app_over_file("p_twice", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('t'));
        for pattern in ["beta", "gamma"] {
            key(&mut app, KeyCode::Char('/'));
            typed(&mut app, pattern);
            key(&mut app, KeyCode::Enter);
            key(&mut app, KeyCode::Char('p'));
        }

        assert_eq!(app.filters.len(), 2);
        // Not `match_count`: it counts `Included` and `Searched` verdicts
        // identically, and `apply_search`'s own refresh already set it to 2
        // the moment the live search matched — before either promotion ran.
        // Checking the verdicts themselves is what actually depends on `p`'s
        // `refresh_view`: promoting moves a line from the search's slot to a
        // numbered one, and only a re-evaluate updates its `Verdict` to
        // reflect that move.
        assert_eq!(
            app.document.verdicts()[1],
            Verdict::Included(0),
            "beta's verdict was not updated after being promoted"
        );
        assert_eq!(
            app.document.verdicts()[2],
            Verdict::Included(1),
            "gamma's verdict was not updated after being promoted"
        );
    }

    #[test]
    fn p_with_no_search_does_nothing() {
        let mut app = app_over_file("p_noop", "alpha\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('p'));

        assert!(app.filters.is_empty());
    }

    /// Same reasoning as `ctrl_modified_letters_still_reach_the_file_view`:
    /// a modified `p` must fall through rather than be swallowed by the
    /// promote binding, which only claims the bare, unmodified key.
    #[test]
    fn ctrl_p_does_not_promote_the_search() {
        let mut app = app_over_file("p_ctrl", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        app.handle_event(event::Event::Key(event::KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();

        assert!(
            app.filters.is_empty(),
            "Ctrl-P was taken as a promote command"
        );
        assert!(
            app.filters.search().is_some(),
            "Ctrl-P consumed the search slot"
        );
    }

    /// `?` is reserved for the help view (#25). With n/N covering both
    /// directions there is nothing left for it to do.
    #[test]
    fn question_mark_no_longer_opens_a_prompt() {
        let mut app = app_over_file("question_inert", "alpha\n");
        key(&mut app, KeyCode::Char('t'));

        key(&mut app, KeyCode::Char('?'));

        assert!(app.search.is_none(), "? still opens a prompt");
    }

    /// `/` in the navigator still searches filenames — that pane has its own
    /// search and is untouched by this work. Asserts the selection actually
    /// moved, not just that no filter was created: a `/` that did nothing at
    /// all would also pass the filter-only assertion.
    #[test]
    fn slash_in_the_navigator_still_searches_filenames() {
        let mut app = app_over("slash_nav", &["alpha.log", "zebra.log"]);
        key(&mut app, KeyCode::Char('e'));

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "zebra");
        key(&mut app, KeyCode::Enter);

        assert!(
            app.filters.search().is_none(),
            "a nav search became a filter"
        );
        let nav = &app.nav;
        assert_eq!(
            nav.entries[nav.state.selected().unwrap()].name,
            "zebra.log",
            "the nav search did not move the selection"
        );
    }

    /// `apply_search`'s ordering — `refresh_view` before `step_to_interesting`
    /// — doesn't show up in `a_search_with_hiding_on_collapses_the_file_to_its_matches`
    /// because the cursor starts on row 0 there: forward-from-0 and "the
    /// first hit" agree by coincidence regardless of order. Here the cursor
    /// starts past both hits. A swapped order runs `step_to_interesting`
    /// against verdicts that don't know about the new search yet, so it finds
    /// nothing and no-ops; `refresh_view` then merely re-anchors the
    /// unmoved cursor to its nearest surviving neighbour (`beta2`, source 3)
    /// once it finally runs — landing one hit short of forward-from-cursor's
    /// real answer (`beta1`, source 1).
    #[test]
    fn slash_moves_forward_from_the_cursor_only_after_the_rebuild_completes() {
        let mut app = app_over_file("slash_order", "alpha\nbeta1\ngamma\nbeta2\ndelta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('H'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        assert_eq!(
            cursor_source(&app),
            4,
            "sanity: cursor starts past both hits"
        );

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(
            cursor_source(&app),
            1,
            "landed on the second hit (3), not the first hit reachable forward \
             from the cursor (1) — refresh_view did not complete before \
             step_to_interesting ran"
        );
    }

    /// `key()` builds every event with `KeyModifiers::NONE`, which cannot
    /// express a real keypress: crossterm attaches SHIFT to every uppercase
    /// character a terminal actually sends. A guard written as
    /// `key.modifiers.is_empty()` looks correct under `key()` and is
    /// unreachable in production — this fires the event by hand, the way
    /// crossterm really would, to catch exactly that class of bug.
    #[test]
    fn capital_n_tolerates_the_shift_modifier_a_real_terminal_sends() {
        let mut app = app_over_file("n_shift", "hit a\nplain\nhit b\nplain\nhit c\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        app.filters.set_search("hit").expect("valid pattern");
        app.refresh_view();
        assert_eq!(
            cursor_source(&app),
            2,
            "sanity: cursor starts on the middle hit"
        );

        app.handle_event(event::Event::Key(KeyEvent::new(
            KeyCode::Char('N'),
            KeyModifiers::SHIFT,
        )))
        .unwrap();

        assert_eq!(
            cursor_source(&app),
            0,
            "N with the SHIFT modifier a real terminal sends did not walk backwards"
        );
    }

    /// `n`/`N` bypass `FileView::handle_events`, which is where a truncated
    /// preview normally promotes itself to a full load on first interaction.
    /// Without repeating that promotion, `n` on a large log would silently
    /// wrap inside the preview and never reach a hit past it.
    #[test]
    fn n_promotes_a_truncated_preview_before_stepping() {
        claim_fixture_dir("n_truncated_promote");
        let dir = std::path::Path::new("target/test-appdirs").join("n_truncated_promote");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        // Past PREVIEW_LINES, so the first preview is truncated; the hit sits
        // beyond the preview boundary, reachable only once promoted.
        let hit_at = crate::widgets::fileview::PREVIEW_LINES + 50;
        let body: String = (0..crate::widgets::fileview::PREVIEW_LINES + 100)
            .map(|i| {
                if i == hit_at {
                    "HIT line\n".to_string()
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        fs::write(dir.join("big.log"), &body).expect("write fixture");

        let mut app = App::new(&Config {
            path: dir.join("placeholder").display().to_string(),
            ..Config::default()
        });
        // The navigator falls back to the first real entry (the log) and
        // previews it, exactly as `upgrading_a_truncated_preview_resyncs_styles_without_reloading`
        // does — the startup argument names a file that does not exist.
        key(&mut app, KeyCode::Down);
        focus_file_view(&mut app);
        app.filters.set_search("HIT").expect("valid pattern");
        app.refresh_view();

        key(&mut app, KeyCode::Char('n'));

        assert_eq!(
            cursor_source(&app),
            hit_at,
            "n did not reach a hit beyond the truncated preview"
        );
    }

    /// `apply_search` is documented as "set it, then do exactly what `n`
    /// does" — including the truncated-preview promotion above. This drives
    /// `/` through the real key path (unlike `n_promotes_a_truncated_preview_before_stepping`,
    /// which sets the search directly via `filters.set_search` and so never
    /// exercises `apply_search` at all) against a preview truncated the same
    /// way, with the only hit past the boundary. If `apply_search` skips the
    /// promotion, the pattern is evaluated against the preview alone: there
    /// is no hit in range, `step_to_interesting` is a no-op, and the cursor
    /// stays wherever it started.
    #[test]
    fn slash_promotes_a_truncated_preview_before_landing_on_a_hit() {
        claim_fixture_dir("slash_truncated_promote");
        let dir = std::path::Path::new("target/test-appdirs").join("slash_truncated_promote");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        // Same shape as the `n` fixture above: past PREVIEW_LINES, with the
        // only hit beyond the preview boundary.
        let hit_at = crate::widgets::fileview::PREVIEW_LINES + 50;
        let body: String = (0..crate::widgets::fileview::PREVIEW_LINES + 100)
            .map(|i| {
                if i == hit_at {
                    "HIT line\n".to_string()
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        fs::write(dir.join("big.log"), &body).expect("write fixture");

        let mut app = App::new(&Config {
            path: dir.join("placeholder").display().to_string(),
            ..Config::default()
        });
        key(&mut app, KeyCode::Down);
        focus_file_view(&mut app);
        assert_eq!(cursor_source(&app), 0, "sanity: cursor starts at the top");

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "HIT");
        key(&mut app, KeyCode::Enter);

        assert_eq!(
            cursor_source(&app),
            hit_at,
            "/ did not reach a hit beyond the truncated preview"
        );
    }

    /// End to end through the real key path: the pane's `d` on the search row
    /// must remove the search and nothing else.
    #[test]
    fn deleting_the_search_row_leaves_the_numbered_filters_alone() {
        let mut app = app_over_file("pane_del_search", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('d'));

        assert!(app.filters.search().is_none());
        assert_eq!(
            app.filters.len(),
            1,
            "a numbered filter was deleted instead"
        );
    }

    /// `handle_filter_key` passes `row_count`, not `len`, as the bound `j`/`k`
    /// clamp movement to. Passing `len` instead is a distinct bug from the
    /// row-to-filter translation covered above — it under-counts how far the
    /// selection is allowed to travel rather than mistranslating where it
    /// lands — and needs two numbered filters plus a search to show up: with
    /// only one numbered filter, `len` and `row_count - 1` land on the same
    /// row by coincidence.
    #[test]
    fn j_below_the_search_row_can_still_reach_the_last_filter() {
        let mut app = app_over_file("pane_move_search", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        app.filters.add("gamma").expect("valid pattern");
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Enter);

        assert!(
            !app.filters.filters()[1].enabled,
            "two downs from the search row should have reached the second filter"
        );
        assert!(
            app.filters.filters()[0].enabled,
            "the first filter was toggled instead of the second"
        );
    }

    /// `FilterCommand::ToggleSearch`'s dispatch is untested elsewhere: unlike
    /// `Toggle(index)`, which routes through the already-covered
    /// `toggle_enabled`, nothing presses `space` on the search row through
    /// the real key path. Both directions are asserted because a mutation
    /// that always disables the search (rather than flipping it) passes the
    /// first press — it starts enabled, and disabling is the right move —
    /// and only fails on the second, when the correct behaviour is to
    /// re-enable it.
    #[test]
    fn enter_on_the_search_row_toggles_search_enabled_both_ways() {
        let mut app = app_over_file("pane_toggle_search", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Enter);

        assert!(
            !app.filters.search().expect("search still set").enabled,
            "space on the search row should have disabled it"
        );

        key(&mut app, KeyCode::Enter);

        assert!(
            app.filters.search().expect("search still set").enabled,
            "a second space should have re-enabled the search"
        );
    }

    /// `c` reaches the search row exactly as `space` and `d` do. Leaving it
    /// inert there would make the key look broken on one row of a pane where
    /// every other binding works on all of them.
    ///
    /// It commits through `apply_search` rather than `run_search`: the latter
    /// dispatches on the *focused* pane, and the filter pane's arm there does
    /// nothing at all — so routing this through it would open a prompt whose
    /// `Enter` silently discarded the pattern.
    #[test]
    fn c_on_the_search_row_edits_the_search() {
        let mut app = app_over_file("pane_edit_search", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('c'));

        let prompt = app.search.as_ref().expect("the prompt should be open");
        assert_eq!(prompt.pattern, "beta");
        assert_eq!(prompt.line(), "/beta", "the search row edits under `/`");

        // Backspace first, so this covers editing the pre-filled text rather
        // than only appending to it.
        key(&mut app, KeyCode::Backspace);
        typed(&mut app, "a2");
        key(&mut app, KeyCode::Enter);

        assert_eq!(
            app.filters
                .search()
                .expect("the search should still be set")
                .pattern
                .as_str(),
            "beta2"
        );
        assert_eq!(
            app.filters.len(),
            1,
            "editing the search touched the numbered set"
        );
    }

    /// An edit of the search row must not renumber anything: the search lives
    /// in its own slot precisely so that setting it cannot shift the filters
    /// a `Verdict::Included` indexes into.
    #[test]
    fn editing_the_search_does_not_promote_it_into_the_numbered_set() {
        let mut app = app_over_file("pane_edit_search_slot", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('c'));
        typed(&mut app, "2");
        key(&mut app, KeyCode::Enter);

        assert_eq!(app.filters.len(), 1);
        assert_eq!(app.filters.verdict("alpha line"), Verdict::Included(0));
    }

    /// `filter_pane_height` must count the search row too. Reverting it to
    /// `self.filters.len()` gives a pane one row short whenever a search
    /// exists alongside at least one filter, clipping the last row — a
    /// numbered filter is required alongside the search because with zero
    /// filters `preferred_height`'s `.max(1)` floor produces the same answer
    /// either way, masking the bug.
    #[test]
    fn filter_pane_height_counts_the_search_row_too() {
        let mut app = app_over_file("pane_height_search", "alpha\nbeta\n");
        app.filters.add("alpha").expect("valid pattern");
        app.filters.set_search("beta").expect("valid pattern");

        let height = app.filter_pane_height();
        let expected = app.filters_pane.preferred_height(app.filters.row_count());

        assert_eq!(
            height, expected,
            "filter_pane_height did not count the search row"
        );
    }

    /// Both paths that clear the search — `Esc` and the pane's `d` on the
    /// search row — funnel through `refresh_view`, which reclamps the
    /// pane's selection to the new `row_count`. Nothing pinned that down: a
    /// future path that cleared the search without going through
    /// `refresh_view` would leave the selection pointing past the end of a
    /// now-shorter list. Selecting row 1 — the numbered filter, the last row
    /// while the search still occupies row 0 — before clearing makes that
    /// observable: it is out of range the moment the search row disappears.
    #[test]
    fn clearing_the_search_leaves_the_selection_in_range() {
        let mut app = app_over_file("pane_clear_search_selection", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        app.filters.add("alpha").expect("valid pattern");
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.filters_pane.selected(),
            Some(1),
            "setup: selection should be on the numbered filter's row"
        );

        key(&mut app, KeyCode::Esc);

        assert!(app.filters.search().is_none(), "setup: search not cleared");
        assert_eq!(
            app.filters_pane.selected(),
            Some(0),
            "selection was left pointing past the end after the search row disappeared"
        );
    }

    impl App<'_> {
        /// The pattern the file view is currently highlighting, for tests.
        fn file_view_highlight(&self) -> Option<String> {
            self.view.highlight()
        }
    }

    /// Toggling hide mode shrinks the visible line set, which rebuilds the
    /// pane's buffer through `FileView::show_lines_with_cursor`. That call
    /// does not itself clear the textarea's search pattern — `set_lines`
    /// resets history, selection, custom highlights, atomic ranges and the
    /// viewport, but never touches the search pattern — and `apply_view`
    /// recomputes and re-applies the highlight unconditionally on every
    /// pass regardless, the same as `styles`/`numbers` above. This confirms
    /// a filter-driven rebuild does not disturb it; the swap that actually
    /// clears the pattern is covered separately, below.
    #[test]
    fn the_span_highlight_survives_a_rebuild() {
        let mut app = app_over_file("hl_rebuild", "alpha\nbeta\ngamma\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('H'));

        assert!(
            app.file_view_highlight().is_some(),
            "the highlight was lost"
        );
    }

    /// Unlike a filter-driven rebuild, `load`/`preview` replace the textarea
    /// outright (see `FileView::load`), which drops any pattern the old one
    /// held. Filters and the hide mode already had to survive that same
    /// swap — `filters_survive_loading_another_file`,
    /// `the_hide_mode_survives_loading_another_file` — the highlight is no
    /// different.
    #[test]
    fn the_span_highlight_survives_loading_another_file() {
        let mut app = app_over_file("hl_reload", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        let dir = std::path::Path::new("target/test-appdirs/hl_reload");
        fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
        app.perform(Action::Load(dir.join("other.txt")));

        assert!(
            app.file_view_highlight().is_some(),
            "the highlight did not survive loading another file"
        );
    }

    /// `!` promises one keystroke back to an unfiltered view. Yellow left glowing
    /// on an inert view breaks that promise.
    #[test]
    fn disabling_everything_clears_the_span_highlight() {
        let mut app = app_over_file("hl_bang", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Char('!'));
        assert!(
            app.file_view_highlight().is_none(),
            "highlights outlived '!'"
        );

        key(&mut app, KeyCode::Char('!'));
        assert!(
            app.file_view_highlight().is_some(),
            "the highlight did not come back"
        );
    }

    #[test]
    fn clearing_the_search_clears_the_span_highlight() {
        let mut app = app_over_file("hl_esc", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        key(&mut app, KeyCode::Esc);

        assert!(app.file_view_highlight().is_none());
    }

    // ---- `o`: open the enclosing project (#42) ---------------------------

    use editor::double::RecordingLauncher;
    use std::rc::Rc;

    /// A fixture project: `target/test-appdirs/<name>/` with a `go.mod` marker
    /// in it and `log.txt` inside a `logs/` subdirectory, so the walk-up has an
    /// actual level to climb.
    ///
    /// `go.mod` rather than `Cargo.toml` purely so nothing under `target/`
    /// looks like a crate to any tool that goes wandering; every marker in the
    /// table is proved equivalent by `every_marker_in_the_table_is_recognised`
    /// over in `editor.rs`.
    /// The template is pinned to the compiled-in default rather than left to
    /// resolve, and that is not belt-and-braces: `Config::editor_templates`
    /// reads the real `$VISUAL`/`$EDITOR`, so on any machine whose developer
    /// has one set (`hx`, here) these tests would assert against *their*
    /// editor. Pinning the top rung takes the environment out of it. The ladder
    /// below is unit-tested with the environment injected, in `editor.rs`.
    fn app_over_project(name: &str, body: &str) -> (App<'static>, std::path::PathBuf) {
        claim_fixture_dir(name);
        let root = std::path::Path::new("target/test-appdirs").join(name);
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(root.join("logs")).expect("create fixture project");
        fs::write(root.join("go.mod"), "module fixture\n").expect("write marker");
        let file = root.join("logs/log.txt");
        fs::write(&file, body).expect("write fixture");
        let app = App::new(&Config {
            path: file.display().to_string(),
            editor: Some(editor::DEFAULT_PROJECT_TEMPLATE.to_string()),
            ..Config::default()
        });
        (
            app,
            std::path::absolute(&root).expect("absolute fixture root"),
        )
    }

    /// Swap in the recording double and hand back a handle the test can read
    /// afterwards. `Rc`, not a clone: the app and the test must see the same
    /// recording, and `Launcher` takes `&self` so no mutability is shared.
    fn record_launches(app: &mut App, launcher: RecordingLauncher) -> Rc<RecordingLauncher> {
        let launcher = Rc::new(launcher);
        app.launcher = Box::new(Rc::clone(&launcher));
        launcher
    }

    fn absolute(path: &std::path::Path) -> String {
        std::path::absolute(path)
            .expect("absolute path")
            .display()
            .to_string()
    }

    /// The headline acceptance criterion: `o` opens the *enclosing project* of
    /// the selected file, at the line the cursor is on.
    #[test]
    fn o_opens_the_enclosing_project_at_the_cursors_line() {
        let (mut app, root) = app_over_project("o_project", "alpha\nbeta\ngamma\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        key(&mut app, KeyCode::Char('o'));

        assert_eq!(
            launcher.only_command(),
            [
                "zed".to_string(),
                absolute(&root),
                format!("{}:1", absolute(&root.join("logs/log.txt"))),
            ]
        );
    }

    /// The line is the cursor's, not always 1 — and it is 1-based, because
    /// every editor's `:line` argument is.
    #[test]
    fn the_editor_lands_on_the_line_the_cursor_is_on() {
        let (mut app, root) = app_over_project("o_line", "alpha\nbeta\ngamma\ndelta\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        focus_file_view(&mut app);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Char('o'));

        let argv = launcher.only_command();
        assert_eq!(
            argv.last().expect("a file argument"),
            &format!("{}:3", absolute(&root.join("logs/log.txt")))
        );
    }

    /// Global, not pane-scoped. The navigator is where `o` is most natural to
    /// press, so requiring the file view to be focused first would break it in
    /// the one place it matters most.
    #[test]
    fn o_works_from_the_navigator_pane() {
        let (mut app, _root) = app_over_project("o_from_nav", "alpha\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        assert!(
            app.focus == Focus::Nav,
            "the navigator should have focus at startup"
        );
        key(&mut app, KeyCode::Char('o'));

        assert!(!launcher.is_empty(), "`o` did nothing from the navigator");
    }

    /// The template is a setting, so a configured one has to actually reach the
    /// command — this is the whole ladder in `config.rs` proved end to end.
    #[test]
    fn a_configured_template_is_what_runs() {
        claim_fixture_dir("o_template");
        let root = std::path::Path::new("target/test-appdirs/o_template");
        fs::remove_dir_all(root).ok();
        fs::create_dir_all(root).expect("create fixture dir");
        fs::write(root.join("go.mod"), "module fixture\n").expect("write marker");
        let file = root.join("log.txt");
        fs::write(&file, "alpha\n").expect("write fixture");

        let mut app = App::new(&Config {
            path: file.display().to_string(),
            editor: Some("code {project} -g {file}:{line}".to_string()),
            ..Config::default()
        });
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        key(&mut app, KeyCode::Char('o'));

        assert_eq!(
            launcher.only_command(),
            [
                "code".to_string(),
                absolute(root),
                "-g".to_string(),
                format!("{}:1", absolute(&file)),
            ]
        );
    }

    /// A missing or failing command must say so, and recon must keep running.
    #[test]
    fn a_failing_command_is_reported_on_the_status_row() {
        let (mut app, _root) = app_over_project("o_fail", "alpha\n");
        record_launches(&mut app, RecordingLauncher::failing("no such file"));

        key(&mut app, KeyCode::Char('o'));

        let row = status_line(&mut app);
        assert!(
            row.contains("zed"),
            "the row does not name the command: {row}"
        );
        assert!(
            row.contains("no such file"),
            "the row does not say why: {row}"
        );
        assert!(app.is_running(), "a failed launch brought the app down");
    }

    /// A typo in the template is the user's, and it can only be caught here:
    /// templates are deliberately not validated at startup, so that a bad one
    /// never stops recon opening a log.
    #[test]
    fn a_broken_template_is_reported_rather_than_run() {
        claim_fixture_dir("o_broken_template");
        let root = std::path::Path::new("target/test-appdirs/o_broken_template");
        fs::remove_dir_all(root).ok();
        fs::create_dir_all(root).expect("create fixture dir");
        let file = root.join("log.txt");
        fs::write(&file, "alpha\n").expect("write fixture");

        let mut app = App::new(&Config {
            path: file.display().to_string(),
            editor: Some("zed 'unclosed".to_string()),
            ..Config::default()
        });
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        key(&mut app, KeyCode::Char('o'));

        assert!(launcher.is_empty(), "a broken template still ran something");
        let row = status_line(&mut app);
        assert!(row.contains("unclosed"), "the row does not explain: {row}");
    }

    /// The startup argument can name a file that is not there — the pane shows
    /// the error in place of its text — and `o` must not hand that path to an
    /// editor as though it existed.
    #[test]
    fn o_refuses_a_file_that_is_not_there() {
        claim_fixture_dir("o_missing");
        let dir = std::path::Path::new("target/test-appdirs/o_missing");
        fs::remove_dir_all(dir).ok();
        fs::create_dir_all(dir).expect("create fixture dir");

        let mut app = App::new(&Config {
            path: dir.join("nope.log").display().to_string(),
            ..Config::default()
        });
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        key(&mut app, KeyCode::Char('o'));

        assert!(
            launcher.is_empty(),
            "a missing file was handed to an editor"
        );
        assert!(status_line(&mut app).contains("no such file"));
    }

    /// Transient means transient: the next keypress takes the row back.
    #[test]
    fn the_status_message_lasts_until_the_next_keypress() {
        let (mut app, _root) = app_over_project("o_transient", "alpha\n");
        record_launches(&mut app, RecordingLauncher::failing("boom"));

        key(&mut app, KeyCode::Char('o'));
        assert!(status_line(&mut app).contains("boom"));

        key(&mut app, KeyCode::Tab);
        assert!(
            !status_line(&mut app).contains("boom"),
            "the message outlived the keypress after it"
        );
    }

    /// A mouse move must *not* clear it. Mouse capture is on, so anything less
    /// deliberate than a keypress would wipe the message before it was read.
    #[test]
    fn a_mouse_event_does_not_clear_the_status_message() {
        let (mut app, _root) = app_over_project("o_mouse", "alpha\n");
        record_launches(&mut app, RecordingLauncher::failing("boom"));

        key(&mut app, KeyCode::Char('o'));
        mouse(&mut app, MouseEventKind::Moved, 60);

        assert!(status_line(&mut app).contains("boom"));
    }

    /// An open prompt outranks the message: the user is typing into that row.
    #[test]
    fn a_prompt_outranks_the_status_message() {
        let (mut app, _root) = app_over_project("o_prompt", "alpha\n");
        record_launches(&mut app, RecordingLauncher::failing("boom"));

        key(&mut app, KeyCode::Char('o'));
        key(&mut app, KeyCode::Char('/'));

        assert_eq!(prompt_line(&mut app), "/");
    }

    /// `o` is only claimed unmodified, so Ctrl-O still reaches the focused
    /// widget — the same guard `q`, `f` and `p` use.
    #[test]
    fn ctrl_o_is_not_the_open_key() {
        let (mut app, _root) = app_over_project("o_ctrl", "alpha\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        ctrl(&mut app, KeyCode::Char('o'));

        assert!(launcher.is_empty(), "Ctrl-O was swallowed as the open key");
    }

    // ---- `O`: open the file alone (#41) ----------------------------------

    /// `key`, with Shift held — what a real terminal sends for an uppercase
    /// character. `key(Char('O'))` alone is the harness-only case; both have to
    /// work, which is why `O` is guarded on CONTROL/ALT rather than on
    /// `.is_empty()`.
    fn shift(app: &mut App, code: KeyCode) {
        app.handle_event(event::Event::Key(event::KeyEvent::new(
            code,
            KeyModifiers::SHIFT,
        )))
        .unwrap();
    }

    /// The headline acceptance criterion: `O` opens the file *alone*, at the
    /// cursor's line, with no project argument — and the fixture is inside a
    /// project, so this also proves no walk-up happened.
    #[test]
    fn shift_o_opens_the_file_alone_at_the_cursors_line() {
        let (mut app, root) = app_over_project("shift_o_file", "alpha\nbeta\ngamma\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        shift(&mut app, KeyCode::Char('O'));

        assert_eq!(
            launcher.only_command(),
            [
                "zed".to_string(),
                format!("{}:1", absolute(&root.join("logs/log.txt"))),
            ]
        );
    }

    /// Stated separately from the argv assertion above because it is the point
    /// of the key: `~/.zshrc` inside a dotfiles repo has a marker above it, and
    /// `O` exists so that marker is never consulted.
    #[test]
    fn shift_o_performs_no_walk_up_even_inside_a_project() {
        let (mut app, root) = app_over_project("shift_o_no_walk_up", "alpha\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        shift(&mut app, KeyCode::Char('O'));

        let argv = launcher.only_command();
        assert!(
            !argv.contains(&absolute(&root)),
            "`O` climbed to the project root anyway: {argv:?}"
        );
        assert_eq!(argv.len(), 2, "`O` passed more than a program and a file");
    }

    /// The cursor's line, 1-based, on this path too — the shared half of
    /// `open_in_editor` proved through the other key.
    #[test]
    fn shift_o_lands_on_the_line_the_cursor_is_on() {
        let (mut app, root) = app_over_project("shift_o_line", "alpha\nbeta\ngamma\ndelta\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        focus_file_view(&mut app);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        shift(&mut app, KeyCode::Char('O'));

        assert_eq!(
            launcher.only_command().last().expect("a file argument"),
            &format!("{}:3", absolute(&root.join("logs/log.txt")))
        );
    }

    /// Global, not pane-scoped, exactly like `o`.
    #[test]
    fn shift_o_works_from_the_navigator_pane() {
        let (mut app, _root) = app_over_project("shift_o_from_nav", "alpha\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        assert!(
            app.focus == Focus::Nav,
            "the navigator should have focus at startup"
        );
        shift(&mut app, KeyCode::Char('O'));

        assert!(!launcher.is_empty(), "`O` did nothing from the navigator");
    }

    /// A harness that sends no modifier at all must still reach the key. The
    /// tests above pin the real terminal's SHIFT; this pins the other case, so
    /// neither guard can be tightened into breaking the other.
    #[test]
    fn shift_o_is_reached_with_no_modifier_attached() {
        let (mut app, _root) = app_over_project("shift_o_bare", "alpha\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        key(&mut app, KeyCode::Char('O'));

        assert!(!launcher.is_empty(), "a bare `O` did not reach the key");
    }

    /// The derive rung, end to end: one line of config for `o` makes `O` work,
    /// by dropping the `{project}` entry rather than by string surgery.
    #[test]
    fn the_file_template_is_derived_from_the_project_template() {
        claim_fixture_dir("shift_o_derived");
        let root = std::path::Path::new("target/test-appdirs/shift_o_derived");
        fs::remove_dir_all(root).ok();
        fs::create_dir_all(root).expect("create fixture dir");
        fs::write(root.join("go.mod"), "module fixture\n").expect("write marker");
        let file = root.join("log.txt");
        fs::write(&file, "alpha\n").expect("write fixture");

        let mut app = App::new(&Config {
            path: file.display().to_string(),
            editor: Some("code {project} -g {file}:{line}".to_string()),
            ..Config::default()
        });
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        shift(&mut app, KeyCode::Char('O'));

        assert_eq!(
            launcher.only_command(),
            [
                "code".to_string(),
                "-g".to_string(),
                format!("{}:1", absolute(&file)),
            ]
        );
    }

    /// An explicit `editor.file` outranks the derived one — the rung above it
    /// on the ladder, proved through the key rather than in isolation.
    #[test]
    fn an_explicit_file_template_beats_the_derived_one() {
        claim_fixture_dir("shift_o_explicit");
        let root = std::path::Path::new("target/test-appdirs/shift_o_explicit");
        fs::remove_dir_all(root).ok();
        fs::create_dir_all(root).expect("create fixture dir");
        let file = root.join("log.txt");
        fs::write(&file, "alpha\n").expect("write fixture");

        let mut app = App::new(&Config {
            path: file.display().to_string(),
            editor: Some("code {project} -g {file}:{line}".to_string()),
            file_editor: Some("subl -n {file}:{line}".to_string()),
            ..Config::default()
        });
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        shift(&mut app, KeyCode::Char('O'));

        assert_eq!(
            launcher.only_command(),
            [
                "subl".to_string(),
                "-n".to_string(),
                format!("{}:1", absolute(&file)),
            ]
        );
    }

    /// `o` must keep its own template when the two differ — the one assertion
    /// that catches the arms being wired to the same field.
    #[test]
    fn the_two_keys_do_not_share_a_template() {
        claim_fixture_dir("shift_o_distinct");
        let root = std::path::Path::new("target/test-appdirs/shift_o_distinct");
        fs::remove_dir_all(root).ok();
        fs::create_dir_all(root).expect("create fixture dir");
        let file = root.join("log.txt");
        fs::write(&file, "alpha\n").expect("write fixture");

        let mut app = App::new(&Config {
            path: file.display().to_string(),
            editor: Some("zed {project} {file}:{line}".to_string()),
            file_editor: Some("subl {file}:{line}".to_string()),
            ..Config::default()
        });
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        key(&mut app, KeyCode::Char('o'));

        assert_eq!(
            launcher.only_command().first().expect("a program"),
            "zed",
            "`o` ran the file template"
        );
    }

    /// A failing launch reports and recon keeps running on this path too.
    #[test]
    fn a_failing_shift_o_is_reported_on_the_status_row() {
        let (mut app, _root) = app_over_project("shift_o_fail", "alpha\n");
        record_launches(&mut app, RecordingLauncher::failing("no such file"));

        shift(&mut app, KeyCode::Char('O'));

        let row = status_line(&mut app);
        assert!(
            row.contains("no such file"),
            "the row does not say why: {row}"
        );
        assert!(app.is_running(), "a failed launch brought the app down");
    }

    /// Ctrl-O and Alt-O stay unclaimed, so they fall through to the focused
    /// widget — the same tolerance `H` and `n`/`N` use.
    #[test]
    fn ctrl_shift_o_is_not_the_open_key() {
        let (mut app, _root) = app_over_project("shift_o_ctrl", "alpha\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        ctrl(&mut app, KeyCode::Char('O'));

        assert!(
            launcher.is_empty(),
            "Ctrl-Shift-O was swallowed as the open key"
        );
    }

    /// An open prompt outranks every binding, and an uppercase global is the
    /// one most likely to break that: `O` is an ordinary character to type into
    /// a search. The guard is the early return `handle_event` already makes for
    /// `self.search`, so this pins the behaviour rather than adding to it.
    #[test]
    fn shift_o_typed_into_a_prompt_is_text_not_the_open_key() {
        let (mut app, _root) = app_over_project("shift_o_prompt", "alpha\n");
        let launcher = record_launches(&mut app, RecordingLauncher::default());

        key(&mut app, KeyCode::Char('/'));
        shift(&mut app, KeyCode::Char('O'));

        assert!(launcher.is_empty(), "`O` fired from inside a prompt");
        assert_eq!(prompt_line(&mut app), "/O");
    }

    // ---- `<space>`: peek at the plain file (#48) -------------------------

    /// Every enabled flag, filters then search — the state a peek has to put
    /// back untouched.
    fn enabled_flags(app: &App) -> (Vec<bool>, Option<bool>) {
        (
            app.filters.filters().iter().map(|f| f.enabled).collect(),
            app.filters.search().map(|search| search.enabled),
        )
    }

    /// A two-line file with one enabled filter on `beta`, hiding unmatched
    /// lines — the state the issue describes pressing `<space>` from.
    fn app_hiding(name: &str) -> App<'static> {
        let mut app = app_over_file(name, "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('H'));
        app
    }

    /// The headline acceptance criterion: one keypress replaces the four-key
    /// cycle in the issue — mode flips and every filter comes off, leaving the
    /// plain file.
    #[test]
    fn space_shows_the_plain_unfiltered_file() {
        let mut app = app_hiding("space_plain");
        assert_eq!(
            view_lines(&app),
            vec!["beta".to_string()],
            "sanity: not hiding to begin with"
        );

        key(&mut app, KeyCode::Char(' '));

        assert_eq!(app.document.mode(), Mode::Dimmed, "still hiding");
        assert_eq!(
            view_lines(&app),
            vec!["alpha".to_string(), "beta".to_string()],
            "the whole file is not on screen"
        );
        assert!(
            view_line_styles(&app).iter().all(Option::is_none),
            "the peek left filter colouring behind"
        );
    }

    /// The property the issue states outright: *"Hitting `<space>` twice in a
    /// row gets you back to exactly where you were."*
    #[test]
    fn space_twice_restores_everything_exactly() {
        let mut app = app_hiding("space_round_trip");
        let mode = app.document.mode();
        let lines = view_lines(&app);
        let styles = view_line_styles(&app);
        let flags = enabled_flags(&app);
        let cursor = app.cursor_source();

        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Char(' '));

        assert_eq!(app.document.mode(), mode, "the mode did not come back");
        assert_eq!(view_lines(&app), lines, "the visible set did not come back");
        assert_eq!(view_line_styles(&app), styles, "the colouring did not");
        assert_eq!(enabled_flags(&app), flags, "the filter flags did not");
        assert_eq!(app.cursor_source(), cursor, "the cursor moved");
    }

    /// A peek from `Mode::Dimmed` must not empty the pane. It doesn't, and the
    /// reason is `Document::recompute_visible`'s #36 guard rather than anything
    /// the peek does: with nothing including, `FilteredOnly` shows the whole
    /// file. This test predates #65 and passed under the old forced-`Dimmed`
    /// peek too — it is kept because the *guard* is what it is really pinning,
    /// and that guard is now load-bearing for `<space>`.
    #[test]
    fn space_from_dimmed_mode_does_not_empty_the_pane() {
        let mut app = app_over_file("space_from_dimmed", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: dimming");

        key(&mut app, KeyCode::Char(' '));

        assert_eq!(
            view_lines(&app),
            vec!["alpha".to_string(), "beta".to_string()],
            "the peek emptied the pane"
        );
    }

    /// #65, the headline: #48 asked for `<space>` to **toggle** dimmed/hide, and
    /// the peek forced `Mode::Dimmed` instead. From the dimmed view that made
    /// `<space>` a pure filter switch — indistinguishable from `!`.
    ///
    /// It is safe to flip because hide mode does not mean "hide every unmatched
    /// line"; it means "*if* something is including, hide unmatched lines".
    /// With the filters off there is nothing to hide against, so
    /// `recompute_visible`'s #36 guard shows the whole file either way.
    #[test]
    fn space_from_dimmed_mode_arms_hiding() {
        let mut app = app_over_file("space_arms_hiding", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: dimming");

        key(&mut app, KeyCode::Char(' '));

        assert_eq!(
            app.document.mode(),
            Mode::FilteredOnly,
            "`<space>` did not toggle the mode"
        );
    }

    /// The other half of the toggle, and the half that already worked. Pinned
    /// alongside its opposite so a future change cannot fix one direction by
    /// breaking the other.
    #[test]
    fn space_from_hide_mode_disarms_hiding() {
        let mut app = app_hiding("space_disarms_hiding");
        assert_eq!(
            app.document.mode(),
            Mode::FilteredOnly,
            "sanity: hiding to begin with"
        );

        key(&mut app, KeyCode::Char(' '));

        assert_eq!(app.document.mode(), Mode::Dimmed, "hiding did not come off");
    }

    /// The mode a peek leaves armed is real, so the badge that reports it must
    /// appear — even though the plain file is on screen and nothing is being
    /// hidden. The badge tracks what is *armed*, which is what makes the flip
    /// honest rather than a lie on the status row (see `HIDE_BADGE_TEXT`).
    #[test]
    fn the_hide_badge_reports_a_peek_that_armed_hiding() {
        let mut app = app_over_file("space_badge", "alpha\nbeta\n");
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);
        assert!(
            !status_line(&mut app).contains(HIDE_BADGE_TEXT.trim()),
            "sanity: no badge while merely dimming"
        );

        key(&mut app, KeyCode::Char(' '));

        let row = status_line(&mut app);
        assert!(
            row.contains(HIDE_BADGE_TEXT.trim()),
            "the peek armed hiding but the badge does not say so: {row}"
        );
    }

    /// Global, like `!` and `o`: the file the user means is whatever the view
    /// is showing, so the peek must not require focusing a particular pane.
    #[test]
    fn space_peeks_from_the_navigator_pane() {
        let mut app = app_hiding("space_from_nav");
        key(&mut app, KeyCode::Char('e'));
        assert!(
            app.focus == Focus::Nav,
            "sanity: the navigator should have focus"
        );

        key(&mut app, KeyCode::Char(' '));

        assert_eq!(app.document.mode(), Mode::Dimmed, "`space` did nothing");
    }

    /// The peek and `!` both turn every filter off, and they must not share one
    /// slot: a peek taken while `!` is holding a capture would overwrite it,
    /// and `!` would then restore all-disabled forever.
    #[test]
    fn the_peek_leaves_the_bang_capture_alone() {
        let mut app = app_hiding("space_vs_bang");
        let flags = enabled_flags(&app);

        key(&mut app, KeyCode::Char('!'));
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Char('!'));

        assert_eq!(
            enabled_flags(&app),
            flags,
            "`!` could not restore what it captured after a peek"
        );
    }

    /// An open prompt outranks every binding — `space` is an ordinary
    /// character to type into a pattern.
    #[test]
    fn space_typed_into_a_prompt_is_text_not_a_peek() {
        let mut app = app_hiding("space_prompt");
        // `/` is deliberately inert while the filter pane has focus — that pane
        // has nothing to search over — and `app_hiding` leaves it focused, so
        // the prompt has to be opened from the file view.
        key(&mut app, KeyCode::Char('t'));
        let mode = app.document.mode();

        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char(' '));

        assert_eq!(app.document.mode(), mode, "the peek fired from a prompt");
        // The pattern itself, not the rendered row: the row carries the HIDE
        // badge here, and a trailing space does not survive rendering.
        assert_eq!(
            app.search.as_ref().map(|prompt| prompt.pattern.as_str()),
            Some(" "),
            "the space did not reach the pattern"
        );
    }

    // ---- `Enter`: the filter pane's toggle, and its bounce guard (#48) ----

    /// The reason `Enter` was left unbound here until now: it is also the key
    /// that *commits* the prompt `i`, `x` and `c` open. A doubled press would
    /// otherwise commit the pattern and then silently switch a filter off.
    #[test]
    fn the_enter_that_commits_a_prompt_does_not_also_toggle() {
        let mut app = app_hiding("enter_bounce");
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('c'));
        key(&mut app, KeyCode::Enter);
        let flags = enabled_flags(&app);

        key(&mut app, KeyCode::Enter);

        assert_eq!(enabled_flags(&app), flags, "the bounce toggled a filter");
    }

    /// The guard swallows exactly one `Enter`, and only the one immediately
    /// after the commit — any other key in between means the user is still
    /// working the pane and meant it.
    #[test]
    fn an_enter_after_an_intervening_key_still_toggles() {
        let mut app = app_hiding("enter_after_key");
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('c'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('k'));
        let flags = enabled_flags(&app);

        key(&mut app, KeyCode::Enter);

        assert_ne!(
            enabled_flags(&app),
            flags,
            "the guard outlived its keypress"
        );
    }

    /// Only one. A second doubled press is a deliberate toggle, not a bounce.
    #[test]
    fn the_guard_swallows_only_a_single_enter() {
        let mut app = app_hiding("enter_one_guard");
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('c'));
        key(&mut app, KeyCode::Enter);
        let flags = enabled_flags(&app);

        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);

        assert_ne!(
            enabled_flags(&app),
            flags,
            "the second Enter was swallowed too"
        );
    }

    /// Tall and wide on purpose: `AREA` is ten rows, and the overlay flows its
    /// sections into however many columns the area allows, so a ten-row buffer
    /// would clip away everything these tests assert on.
    const HELP_AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 160,
        height: 44,
    };

    /// The whole rendered frame as text, at `HELP_AREA`.
    fn screen(app: &mut App) -> String {
        let mut buf = Buffer::empty(HELP_AREA);
        app.render(HELP_AREA, &mut buf);
        (0..HELP_AREA.height)
            .map(|y| {
                (0..HELP_AREA.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn question_mark_opens_the_help_overlay() {
        let mut app = app_over("help_open", &["a.rs"]);

        key(&mut app, KeyCode::Char('?'));

        let screen = screen(&mut app);
        assert!(
            screen.contains("Quit"),
            "the help overlay did not draw:\n{screen}"
        );
    }

    /// A real terminal sends `?` as Shift-`/`, so an `is_empty()` guard would
    /// make the binding unreachable outside a test harness — the trap `O` and
    /// `n`/`N` already document.
    #[test]
    fn shift_does_not_stop_the_help_overlay_opening() {
        let mut app = app_over("help_shift", &["a.rs"]);

        app.handle_event(event::Event::Key(event::KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::SHIFT,
        )))
        .unwrap();

        assert!(app.help, "Shift-? did not reach the help binding");
    }

    #[test]
    fn any_key_closes_the_help_overlay() {
        let mut app = app_over("help_close", &["a.rs"]);
        key(&mut app, KeyCode::Char('?'));

        key(&mut app, KeyCode::Char('j'));

        let screen = screen(&mut app);
        assert!(
            !screen.contains("Quit"),
            "the help overlay outlived its dismissing key:\n{screen}"
        );
    }

    /// The dismissing key is consumed. Otherwise the key that closes help also
    /// acts, and the most likely one to be pressed is `q`.
    #[test]
    fn the_key_that_closes_help_does_nothing_else() {
        let mut app = app_over("help_swallow", &["a.rs"]);
        key(&mut app, KeyCode::Char('?'));

        key(&mut app, KeyCode::Char('q'));

        assert!(app.is_running(), "the key that closed help also quit");
    }

    /// A mouse moving across the terminal must not wipe the overlay away — the
    /// same reasoning `handle_event` already applies to the status message.
    #[test]
    fn a_mouse_event_does_not_close_the_help_overlay() {
        let mut app = app_over("help_mouse", &["a.rs"]);
        key(&mut app, KeyCode::Char('?'));

        mouse(&mut app, MouseEventKind::Moved, 10);

        assert!(app.help, "a mouse event closed the help overlay");
    }

    /// An open prompt consumes every key, `?` included — it is a perfectly
    /// ordinary character in a regular expression.
    #[test]
    fn question_mark_is_typed_into_an_open_prompt() {
        let mut app = app_over("help_prompt", &["a.rs"]);
        key(&mut app, KeyCode::Char('/'));

        typed(&mut app, "ab?");

        assert!(!app.help, "`?` opened help from inside a prompt");
        assert_eq!(prompt_line(&mut app), "/ab?");
    }

    /// The overlay covers the panes, not the status row: the row carries the
    /// HIDE badge and the current directory, and both stay true while help is
    /// up.
    #[test]
    fn the_help_overlay_leaves_the_status_row_alone() {
        let mut app = app_over("help_status", &["a.rs"]);
        let before = prompt_line(&mut app);

        key(&mut app, KeyCode::Char('?'));

        assert_eq!(prompt_line(&mut app), before);
    }

    /// `?` spends the post-commit `Enter` guard (#48) exactly as any other key
    /// does — the guard's whole rule is "cleared by any key that is not
    /// `Enter`". Otherwise reading the keymap between a commit and a toggle
    /// leaves a guard armed with nothing left to guard against, and the next
    /// deliberate `Enter` silently does nothing.
    #[test]
    fn reading_the_keymap_spends_the_post_commit_enter_guard() {
        let mut app = app_hiding("help_enter_guard");
        focus_filter_pane(&mut app);
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('c'));
        key(&mut app, KeyCode::Enter);
        let flags = enabled_flags(&app);

        key(&mut app, KeyCode::Char('?'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);

        assert_ne!(
            enabled_flags(&app),
            flags,
            "the guard outlived the key that should have spent it"
        );
    }
}
