//! `App`: the terminal UI's state, its event loop, and how it draws.
//!
//! This file holds the struct, `new`, and the loop that `run` drives. Each
//! other file in `app/` adds one topic's `impl App<'_>` block. A child module
//! can read the private fields of `App`, so only a method or a type that a
//! different file uses needs `pub(super)`.

mod actions;
mod collect;
mod events;
mod filter_editor;
mod filters;
mod finish;
mod focus;
mod launch;
mod layout;
mod mouse;
mod navigation;
mod prompt;
mod render;
mod scanning;
mod search;
mod selection;
mod sync;
pub(crate) mod viewport;

use crate::startup::Startup;
use crate::{clipboard, document, editor, emit, filter, filtersets, panes, scan, widgets};
use color_eyre::Result;
use ratatui::prelude::{Backend, Rect, Terminal};
use std::time::Instant;
// Imported rather than left qualified: `App`'s own fields are typed with these
// three, so every mention of them in the struct and in `render` would otherwise
// need a `layout::` prefix. The pane-geometry constants are *not* re-exported
// here — nothing outside `layout` reads them any more except the tests, which
// import them directly (#74).
use clipboard::Clipboard;
use document::{Document, Mode};
use editor::Launcher;
use filter::ActiveFilters;
use layout::{ClickClock, Divider, PaneWidth};
use panes::Panes;
use widgets::Focus;
use widgets::explorer::Explorer;
use widgets::fileview::FileView;
use widgets::filterlist::FilterList;
// The types of `App`'s fields that live beside the code that uses them.
use filter_editor::FilterEditor;
use filters::PeekState;
use navigation::Crossing;
use prompt::{History, SearchPrompt};
use render::status::StatusMessage;
use scanning::{ScanCache, ScanState};
use search::Search;

#[derive(Default)]
pub struct App<'a> {
    state: AppState,
    /// The work `q` under `--emit` started, while `state` is `Finishing`
    /// (#351, #352).
    finishing: Option<finish::Finish>,
    /// What that work produced, for `exit` to hand back.
    finished: Option<emit::Exit>,
    /// Set from outside the loop when a SIGTERM arrives (#382). `main`
    /// installs it; `None` in a test that does not ask for one.
    terminate: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// The three panes, named rather than collected (#73).
    ///
    /// They were a `Vec<AppWidget>` built once with exactly three entries,
    /// never pushed to or popped from, whose length `render` asserted on every
    /// frame. Because each position was untyped, `App` could not say "the file
    /// view" — it had to search for it, which twenty call sites did, each
    /// paying a linear scan and an `unwrap_or` fallback for a case that could
    /// not happen. Named fields make the invariant unrepresentable instead of
    /// merely checked, and the scans become field reads.
    explorer: Explorer<'a>,
    view: FileView<'a>,
    filters_pane: FilterList,
    /// Which pane has focus, replacing an index into the old vec.
    focus: Focus,
    explorer_width: PaneWidth,
    filter_width: PaneWidth,
    /// Which panes are on the screen, and what a zoom remembered (#300).
    panes: Panes,
    /// The explorer's divider from the last render: the first column to the
    /// explorer's right. For hit-testing mouse events that arrive before the
    /// next frame. `u16::MAX` while the explorer is hidden, or is the only
    /// pane, so no real column can hit it.
    divider: u16,
    /// The filter pane's divider from the last render — its first column —
    /// hit-tested the same way as `divider`. `u16::MAX` while there is no
    /// file view to its left.
    filter_divider: u16,
    /// The three panes' rectangles from the last render: a click is
    /// hit-tested against the frame the user was looking at when they
    /// clicked (#58). A hidden pane's is zero wide.
    filter_area: Rect,
    explorer_area: Rect,
    view_area: Rect,
    /// Everything above the status row. Its right edge is what turns a drag
    /// on the filter pane's divider into a width.
    panes_area: Rect,
    /// The status row itself; a click there opens the include prompt.
    status_area: Rect,
    dragging: Option<Divider>,
    /// The last divider click, and which divider it was on.
    ///
    /// The axis is part of the record, not decoration: without it, a click on
    /// one divider followed quickly by a click on the other reads as a
    /// double-click and resets a pane the user never aimed at.
    last_divider_click: Option<(Divider, Instant)>,
    /// Where the divider, explorer and view double-clicks read the time.
    click_clock: ClickClock,
    /// The last click on an explorer row, by visible-row index. Two on the
    /// same row inside `DOUBLE_CLICK` open a directory; the row is part of
    /// the record for the same reason the divider's axis is above.
    last_explorer_click: Option<(usize, Instant)>,
    /// Open while a search pattern, a filter pattern or a set name is being
    /// typed.
    prompt: Option<SearchPrompt>,
    /// The search, while one is set. `/` and `*` set it, `Esc` clears it,
    /// `p` turns it into a filter. It outlives a file load.
    search: Option<Search>,
    /// The patterns Enter committed in the file view's `/` prompt, for
    /// Up and Down in the next one (#274). The explorer's is separate.
    search_history: History,
    /// The patterns Enter committed in the explorer's `/` prompt.
    explorer_search_history: History,
    /// The patterns Enter committed in the set picker's `/` prompt (#285).
    sets_search_history: History,
    filters: ActiveFilters,
    document: Document,
    /// The `Document::generation` the file view's buffer was last rebuilt
    /// from, or `None` when what the buffer holds is unknown and a rebuild is
    /// owed unconditionally.
    ///
    /// The generation moves exactly when the document's visible set does, so
    /// `refresh_view` rebuilds only when the rows on screen differ, and a
    /// filter change that leaves the same rows does not reset the viewport's
    /// scroll position. It used to be the visible index vector itself,
    /// compared and copied whole on every `apply_view` (#159).
    ///
    /// `Option`, not a bare `u64`: `sync_document` has to say "this buffer
    /// belongs to a document that no longer exists, rebuild whatever happens
    /// next", and no number can say that — a fresh document starts at
    /// generation 0, as the previous one may well have. `None` is unequal to
    /// every `Some`, so the rebuild always happens.
    last_generation: Option<u64>,
    /// The window bounds `apply_view` last handed the file view, paired with
    /// `last_generation` as the rebuild-skip key (#7).
    ///
    /// The visible set alone is no longer enough to decide a rebuild can be
    /// skipped: scrolling into a new window leaves the visible set untouched
    /// and still needs the buffer replaced. Omitting this is the subtlest bug
    /// available here — the view would silently go on showing the old rows.
    last_window: Option<(usize, usize)>,
    /// Both editor command templates, resolved once at startup.
    ///
    /// Resolved at startup but *split* per keypress, so a typo in one template
    /// is reported by the key that uses it rather than refusing to start a log
    /// viewer over a setting most sessions never touch.
    editor: editor::Templates,
    /// Every key binding in force: the defaults, with `config.toml`'s
    /// `[keymap]` folded in (#61).
    ///
    /// State rather than the `const` table it replaced, because it now has two
    /// sources. Resolved once, here, so that every key lookup and every
    /// generated hint reads the same table — a hint that still consulted the
    /// defaults would name a key the user had moved.
    keymap: crate::keymap::Keymap,
    /// Whether a jump to a line off screen centres it — `[view]
    /// center_jumps`, resolved by `Config::center_jumps`.
    center_jumps: bool,
    /// What `--emit` asked for, or `None`. Read once, when the session ends.
    emit: Option<emit::Emit>,
    /// `-n`: prefix each emitted line with its source line number and a tab.
    line_numbers: bool,
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
    /// The cross-file step `n`, `N`, `.` or `,` just made, if any (#120).
    crossing: Option<Crossing>,
    /// The fixed end of a selection in progress, or `None` outside visual
    /// mode (#67). Anchored to a *source* line, so a filter or hide toggle
    /// mid-selection changes what a yank contains without invalidating it.
    /// Ends when focus leaves the view or the document is replaced — see
    /// `selection.rs`.
    visual: Option<selection::Visual>,
    /// Where `y` sends the selection. Boxed behind the trait for the reason
    /// `launcher` is: a test asserts on what would have been copied rather
    /// than touching the real pasteboard.
    clipboard: Box<dyn Clipboard>,
    /// Where the left button went down on the view's text, as a source line
    /// and column, while it is still held: the anchor a drag starts its
    /// selection from. `None` once released.
    press: Option<(usize, usize)>,
    /// The last click on the view's text, by source line, for the
    /// double-click that selects a word — timed here as `last_explorer_click` is.
    last_view_click: Option<(usize, Instant)>,
    /// Set when a prompt commits, so the `Enter` that committed it cannot also
    /// toggle the filter under the cursor (#48).
    ///
    /// Cleared by any key that is not `Enter`, and consumed by the first one
    /// that is. Deliberately event-counted rather than timed: a keypress count
    /// is exactly reproducible in a test, where "within 200ms" is not, and the
    /// two only disagree when a user deliberately presses `Enter` twice in a
    /// row — which costs them one extra press and is indistinguishable from
    /// the bounce anyway.
    ///
    /// A chain return spends this guard on its synthetic `n` and immediately
    /// re-arms it for the real `Enter` still pending — see
    /// `return_to_chain_origin`.
    swallow_next_enter: bool,
    /// Set when `q` found a file set with changes no file holds, and said so
    /// on the status row instead of quitting (#348). A `q` as the next key
    /// quits; any other key ends the warning, as it clears the row.
    quit_warned: bool,
    /// `quit_warned` as it was before the key being dispatched: whether
    /// this key is the second `q`. Set from it on each keypress.
    quit_confirmed: bool,
    /// The pane that had focus when `f` moved it to the filter pane, so a
    /// chain that commits a prompt — `f i … Enter`, `f x … Enter`,
    /// `f c … Enter` — can put focus back and step as `n` would (#120 §8,
    /// decision (a)). `None` once any other focus change, a prompt cancel,
    /// or a second `f` ends the chain; a toggle or delete inside the pane
    /// does not end it, because those are often one of several.
    chain_origin: Option<Focus>,
    /// Whether `f` had to show the filter pane to start the chain in
    /// `chain_origin`. The chain that returns hides it again, so a chain
    /// does not change the layout (#300). Read only by
    /// `return_to_chain_origin`, and set afresh by every `f`, so a chain
    /// that ended some other way leaves nothing stale behind.
    chain_shown_filters: bool,
    /// Whether the keymap overlay is covering the panes (#25).
    ///
    /// A plain flag rather than a fourth pane: the three panes are persistent
    /// and reachable with `Tab`, and help is transient and dismissed by the
    /// next key. Joining that cycle would mean tabbing past help forever after
    /// using it once.
    help: bool,
    /// Whether the startup keymap-warning panel is still up.
    ///
    /// Set in `App::new` when the config carried warnings and the switch
    /// leaves them on, and cleared by the first key. Like `help`, it is not a
    /// focus and joins no `Tab` cycle: it is a notice you read and put away.
    keymap_warnings_open: bool,
    /// The warning text, rendered once in `App::new` rather than per frame.
    keymap_warnings: Vec<String>,
    /// The profile picker, while one is open (#130). Takes every key, as a
    /// prompt does.
    picker: Option<widgets::picker::ProfilePicker>,
    /// The set picker, while it is open (#284). Takes every key, as the
    /// profile picker does.
    set_picker: Option<widgets::setpicker::SetPicker>,
    /// The filter editor, while it is open (#312). Takes every key, as the
    /// pickers do.
    filter_editor: Option<FilterEditor>,
    /// The model that writes a pattern from a request in the filter editor
    /// (#319), or `None` when this build has none. `main` sets it; a test
    /// installs a double.
    model: Option<std::sync::Arc<dyn crate::generate::Model>>,
    /// Where `S` writes (#131): `filters.toml` beside `config.toml`, or
    /// `None` when the environment names no home. A field rather than a
    /// call at save time so tests can point it at a fixture.
    save_path: Option<std::path::PathBuf>,
    /// Runs the explorer's file scans (#119). A `Box<dyn Scan>` for the same
    /// reason `launcher` is: tests swap in a recording double.
    scanner: Box<dyn scan::Scan>,
    /// Where the scanner's results arrive. Drained on the render tick by
    /// `drain_scan_results`, alongside `drain_editor_outcomes` and
    /// `poll_stamps`. Tests install their own channel here so `refresh_scan`'s
    /// scanner double can be exercised without a real one.
    scan_results: Option<std::sync::mpsc::Receiver<scan::Scanned>>,
    /// Every file's bitsets, keyed on the pattern list they were read for.
    scan_cache: ScanCache,
    /// What the last `refresh_scan` saw. Unchanged means nothing to do — the
    /// guard that keeps `j` in the file view from stat-ing the folder.
    last_scan: Option<ScanState>,
    /// When `poll_stamps` last started a stamp check.
    last_poll: Option<Instant>,
    /// Where the stamp check in flight will answer, if one is (#156).
    stamp_check: Option<std::sync::mpsc::Receiver<Vec<scan::Moved>>>,
    /// The active file's stamp moved since it was loaded. Shown as a badge,
    /// cleared by `r`.
    view_stale: bool,
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum AppState {
    #[default]
    Running,
    /// The user has asked to quit. `emit` is whether they pressed `q` (yes)
    /// or `Q` (no); what that means depends on whether `--emit` was given —
    /// see `App::exit`.
    Quit { emit: bool },
    /// `q` under `--emit`, and the output needs work first: the rest of a
    /// large file, or the files the scan has not answered (#351, #352).
    /// The session is still running; `finishing.cancel` ends it.
    Finishing,
    /// The user cancelled that work, or pressed Ctrl-c (#382). Nothing is
    /// emitted, and the exit code says so.
    Cancelled,
    /// A SIGTERM arrived (#382). Nothing is emitted, and the exit code says
    /// so.
    Terminated,
}

impl App<'_> {
    #[must_use]
    pub fn new(startup: &Startup) -> Self {
        let config = &startup.config;
        // Absolute from here on, which is the rule the explorer, the scan
        // cache and `check_stamps` already share. Held as typed, the
        // argument was a second spelling of one path: `check_stamps`
        // compared `path == active` against `dir.join(name)` and never
        // matched, so the changed-on-disk badge never fired for a file
        // opened from the command line, and the title changed from
        // `app.log` to the full path after the first navigation (#157).
        let argument = crate::path::lexical_absolute(std::path::Path::new(&config.path));
        let argument = argument.as_path();
        let mut explorer = Explorer::new(config.path.clone());
        explorer.set_background(config.background());
        let mut view = FileView::default();
        // Before the load, not after: `load` looks up the file's grammar, and
        // `set_theme` after it would look it up a second time (#186). Here
        // there is nothing shown yet, so this costs nothing.
        view.set_theme(config.syntax_theme());

        match explorer.selected_path() {
            // A directory argument *selects* an entry rather than being
            // handed one, so it is previewed — bounded by `PREVIEW_LINES` —
            // exactly as arrowing onto it would be. Loading it in full would
            // read a whole log at startup merely because it sorts first,
            // which is the cost the preview mechanism exists to avoid.
            Some(selected) if argument.is_dir() => view.preview(&selected),
            // A file argument loads the argument itself, not the explorer's
            // selection. They are the same path when the file exists; when it
            // does not, the explorer falls back to the first entry, and
            // loading *that* would silently open some other file in response
            // to a typo. Reporting the argument is what recon already does.
            _ => view.load(argument),
        }

        // Created here rather than lazily on the first `o`: the sender has to
        // outlive every launcher clone, and a channel built on demand would
        // need a second field to remember whether it already existed.
        let (outcomes_tx, outcomes_rx) = std::sync::mpsc::channel();
        let (scan_tx, scan_rx) = std::sync::mpsc::channel();

        // The background's palette, unless the file set one (#231); the
        // background itself follows for the dim grey.
        let mut filters =
            ActiveFilters::with_sets(Some(config.filter_palette()), &startup.filter_sets);
        filters.set_background(config.background());
        for (set, items) in config.sets_to_enable() {
            // `Config::check_sets` refused an unknown name in
            // `startup::start` before the terminal came up; a failure here
            // is a hand-built `Startup`
            // in a test, and the set is left off rather than the app brought
            // down over it.
            if let Err(err) = filters.enable_named(&set, items.as_deref()) {
                log::warn!("--set {set}: {err}");
            }
        }
        // `check_sets` refused `--set X --unlist X`, so the order of the two
        // loops decides nothing.
        for set in &config.unlist {
            if let Err(err) = filters.unlist_named(set) {
                log::warn!("--unlist {set}: {err}");
            }
        }

        // `Config::load` refused a list naming all three; `hiding` keeps the
        // file view for a `Config` built by hand that does.
        let panes = Panes::hiding(config.hide_panes().iter().map(|&pane| Focus::from(pane)));

        let mut app = Self {
            state: AppState::Running,
            finishing: None,
            finished: None,
            terminate: None,
            explorer,
            view,
            filters_pane: FilterList::default(),
            focus: panes.settle(Focus::Explorer),
            explorer_width: PaneWidth::Auto,
            filter_width: PaneWidth::Auto,
            panes,
            divider: 0,
            filter_divider: u16::MAX,
            filter_area: Rect::ZERO,
            explorer_area: Rect::ZERO,
            view_area: Rect::ZERO,
            panes_area: Rect::ZERO,
            status_area: Rect::ZERO,
            dragging: None,
            last_divider_click: None,
            click_clock: ClickClock::default(),
            last_explorer_click: None,
            prompt: None,
            search: None,
            search_history: History::default(),
            explorer_search_history: History::default(),
            sets_search_history: History::default(),
            filters,
            document: Document::default(),
            last_generation: None,
            last_window: None,
            editor: config.editor_templates(),
            // Resolved by `startup::start` before the terminal came up
            // (`keymap::config::build`), so nothing fallible happens here —
            // this function returns `Self` and has nowhere to put an error.
            // A `Startup` built from a bare `Config` carries the defaults.
            keymap: startup.bindings.clone(),
            center_jumps: config.center_jumps(),
            emit: config.emit,
            line_numbers: config.line_numbers,
            launcher: Box::new(editor::ProcessLauncher::new(outcomes_tx)),
            status_message: None,
            editor_outcomes: Some(outcomes_rx),
            peek: None,
            crossing: None,
            visual: None,
            clipboard: Box::new(clipboard::ProcessClipboard::new(
                &config.clipboard_template(),
            )),
            press: None,
            last_view_click: None,
            swallow_next_enter: false,
            quit_warned: false,
            quit_confirmed: false,
            chain_origin: None,
            chain_shown_filters: false,
            help: false,
            keymap_warnings: startup.keymap_warnings.clone(),
            keymap_warnings_open: config.warnings() && !startup.keymap_warnings.is_empty(),
            picker: None,
            set_picker: None,
            filter_editor: None,
            model: None,
            save_path: filtersets::path(),
            scanner: Box::new(scan::Scanner::new(scan_tx)),
            scan_results: Some(scan_rx),
            scan_cache: ScanCache::default(),
            last_scan: None,
            last_poll: None,
            stamp_check: None,
            view_stale: false,
        };
        // `--hide`: on the document before `sync_document`, which carries
        // the mode across into the document it builds for the loaded file.
        if config.hide {
            app.set_mode(Mode::FilteredOnly);
        }
        app.sync_document();
        app.refresh_view();
        app
    }

    /// Give the filter editor a model to write patterns with (#319).
    #[must_use]
    pub fn with_model(mut self, model: Option<std::sync::Arc<dyn crate::generate::Model>>) -> Self {
        self.model = model;
        self
    }

    /// Watch `flag`, and end the session once it is set (#382). `main` sets
    /// it from a SIGTERM handler, which can do nothing safer than store a
    /// flag. The loop looks at it on every wake, at most 1/60 s apart, and
    /// leaves the way a quit key does, so the terminal is restored on the
    /// normal path rather than from inside the handler.
    #[must_use]
    pub fn with_terminate(mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.terminate = Some(flag);
        self
    }

    /// This is the main event loop for the app.
    ///
    /// Draw once, then only when something happened. It used to redraw
    /// unconditionally, which meant 60 full render passes a second on a
    /// terminal nobody was touching (#85). ratatui diffs the cell buffer, so
    /// the *writes* stayed small and the cost was invisible — but the render
    /// tree is not diffed away, and every one of those passes still rebuilt
    /// the explorer's `List`, every `Entry::display()` string, and every
    /// filter row. For a log viewer that sits open on a desk all day that is
    /// the difference between idling at 0% and idling at a few percent.
    pub fn run<B>(mut self, mut terminal: Terminal<B>) -> Result<emit::Exit>
    where
        B: Backend,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut dirty = true;
        loop {
            if self
                .terminate
                .as_ref()
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            {
                self.stop_finishing();
                self.state = AppState::Terminated;
            }
            if !self.is_running() {
                break;
            }
            if dirty {
                terminal.draw(|frame| {
                    let area = frame.area();
                    frame.render_widget(&mut self, area);
                })?;
            }
            dirty = self.handle_events()?;
        }
        Ok(self.exit())
    }

    /// What this session hands back, given how it ended (#143). `Emit`
    /// only when `q` ended it *and* `--emit` was given; a `Q`, a missing
    /// `--emit`, or a session still running is `Silent`.
    ///
    /// Work `q` finished hands back what it produced; a cancelled one hands
    /// back `Cancelled` (#351, #352).
    pub(crate) fn exit(&mut self) -> emit::Exit {
        match (self.state, self.emit) {
            (AppState::Cancelled, _) => emit::Exit::Cancelled,
            (AppState::Terminated, _) => emit::Exit::Terminated,
            (AppState::Quit { emit: true }, Some(kind)) => {
                self.finished.take().unwrap_or_else(|| self.collect(kind))
            }
            _ => emit::Exit::Silent,
        }
    }

    const fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running | AppState::Finishing)
    }
}

#[cfg(test)]
mod tests;
