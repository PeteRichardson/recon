//! `App`: the terminal UI's state, its event loop, and how it draws.
//!
//! This file holds the struct, `new`, the loop that `run` drives, and the
//! drawing code. Each other file in `app/` adds one topic's `impl App<'_>`
//! block. A child module can read the private fields of `App`, so only a
//! method or a type that a different file uses needs `pub(super)`.

mod actions;
mod collect;
mod events;
mod filters;
mod focus;
mod launch;
mod layout;
mod mouse;
mod navigation;
mod prompt;
mod scanning;
mod search;
mod selection;
mod sync;
pub(crate) mod viewport;

use crate::config::Config;
use crate::{clipboard, document, editor, emit, filter, filtersets, help, panes, scan, widgets};
use color_eyre::Result;
use ratatui::prelude::{Backend, Buffer, Color, Constraint, Layout, Rect, Style, Terminal, Widget};
use std::borrow::Cow;
use std::time::Instant;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Imported rather than left qualified: `App`'s own fields are typed with these
// three, so every mention of them in the struct and in `render` would otherwise
// need a `layout::` prefix. The pane-geometry constants are *not* re-exported
// here — nothing outside `layout` reads them any more except the tests, which
// import them directly (#74).
use clipboard::Clipboard;
use document::{Document, Mode};
use editor::Launcher;
use filter::ActiveFilters;
use layout::{Divider, PaneWidth};
use panes::Panes;
use widgets::Focus;
use widgets::explorer::Explorer;
use widgets::fileview::FileView;
use widgets::filterlist::FilterList;
// The types of `App`'s fields that live beside the code that uses them.
use filters::PeekState;
use navigation::Crossing;
use prompt::{History, SearchPrompt};
use scanning::{ScanCache, ScanState};
use search::Search;

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

/// The badge saying the file on screen is not the file on disk (#119).
///
/// Raised by `poll_stamps` when the *active* file's stamp moves, cleared by
/// reloading. The explorer's answer for that file updates on its own; the
/// view does not reload on its own. That is a real inconsistency between the
/// panes, and the badge exists so it is never a silent one: one key resolves
/// it.
///
/// A function rather than a `const` (task 8 fix round 2, #199): the badge
/// names the key that reloads, so — like the on-screen hints — it is
/// generated from `DEFAULT` rather than hard-coded, and stays correct after
/// a rebind. The default keymap renders this byte-identically to the old
/// constant, `" changed on disk · r "`, leading and trailing spaces
/// included — see `badges` in `Widget::render`, which pads each badge with
/// exactly the same column of separation regardless of where its text came
/// from.
///
/// `None` when a `[keymap]` line has left `global.reload` with no key at all
/// (#61): the badge exists to name the key that resolves the inconsistency,
/// and " changed on disk · " with nothing after it names none — a badge the
/// user cannot act on is worse than no badge.
fn stale_badge_text(keymap: &crate::keymap::Keymap) -> Option<String> {
    let key = keymap.label_for(crate::keymap::ActionId::GlobalReload)?;
    Some(format!(" changed on disk · {key} "))
}

/// The badge saying the include filters are combined with AND (#39). Same style
/// as `HIDE`, and for the same reason: the mode changes what the pane shows
/// and is easy to forget while moving fast.
const AND_BADGE_TEXT: &str = " AND ";

/// The longest pattern the search badge shows before eliding the tail. The
/// badge shares the status row with the filter summary and the path; a
/// regex this long is still recognisable from its head.
const SEARCH_BADGE_MAX: usize = 32;

/// The badges saying a selection is in progress (#67), character-wise and
/// line-wise. Same style as `HIDE`, and for the same reason: the mode
/// changes what the next keys do — `y` copies, `Esc` ends it — and is easy
/// to forget mid-scroll. Vim's own words for the two, shortened to fit.
const VISUAL_BADGE_TEXT: &str = " VISUAL ";
const VLINE_BADGE_TEXT: &str = " V-LINE ";

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum AppState {
    #[default]
    Running,
    /// The user has asked to quit. `emit` is whether they pressed `q` (yes)
    /// or `Q` (no); what that means depends on whether `--emit` was given —
    /// see `App::exit`.
    Quit { emit: bool },
}

impl App<'_> {
    #[must_use]
    pub fn new(config: &Config) -> Self {
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
            ActiveFilters::with_sets(Some(config.filter_palette()), &config.filter_sets);
        filters.set_background(config.background());
        for (set, profile) in config.sets_to_enable() {
            // `Config::check_sets` refused an unknown name in `main` before
            // the terminal came up; a failure here is a hand-built `Config`
            // in a test, and the set is left off rather than the app brought
            // down over it.
            if let Err(err) = filters.enable_named(&set, profile.as_deref()) {
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
            // Resolved by `main` before the terminal came up
            // (`Config::build_keymap`), so nothing fallible happens here —
            // this function returns `Self` and has nowhere to put an error.
            // A `Config` built by hand in a test carries the defaults.
            keymap: config.bindings.clone(),
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
            chain_origin: None,
            chain_shown_filters: false,
            help: false,
            keymap_warnings: config.keymap_warnings.clone(),
            keymap_warnings_open: config.warnings() && !config.keymap_warnings.is_empty(),
            picker: None,
            set_picker: None,
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
        while self.is_running() {
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
    pub(crate) fn exit(&self) -> emit::Exit {
        match (self.state, self.emit) {
            (AppState::Quit { emit: true }, Some(kind)) => self.collect(kind),
            _ => emit::Exit::Silent,
        }
    }

    const fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running)
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
        // but is disabled would claim lines are hidden while the guard is
        // already showing everything.
        //
        // An excluding filter (`x`) is counted on its own, regardless of
        // mode: it removes its matches in `Dimmed` mode too, which is the
        // entire point of it, so gating the funnel on `FilteredOnly` alone
        // let `x` empty the pane with nothing on the status line saying so.
        let hiding = (self.document.mode() == Mode::FilteredOnly && self.filters.any_including())
            || self.filters.any_excluding();
        let funnel = if hiding { "▼ " } else { "" };
        if self.filters.is_empty() {
            // With no filters at all, `any_including` and
            // `any_excluding` are both trivially false, so `hiding` above is
            // always false too — there is nothing this early return could be
            // discarding. Filters that exist but are all disabled are a
            // different state, one the same `hiding` expression already
            // keeps honest below (see the `!any_enabled` branch): nothing
            // enabled can be including or excluding either, so the funnel
            // stays off there as well.
            return String::new();
        }
        // `row_count`, not `len`: the user-authored filters, which is what
        // the pane numbers. See `ActiveFilters::row_count`.
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
        // With the filter pane hidden, nothing on the screen says which
        // filters colour or remove lines, so the count is of those in effect
        // rather than of rows a pane is not showing (#300).
        let on = self.filters.effective_count();
        let filters = if !self.panes.is_shown(Focus::Filters) && on > 0 {
            format!("filters: {on} on")
        } else {
            format!("{count} {noun}")
        };
        let off = self
            .filters
            .scan_off()
            // Permanent rather than a message, because the cause stays until
            // the user removes it: the explorer marks nothing and its `n`
            // finds nothing, and without this nothing on the screen says why.
            .map(|off| format!("   {off}"))
            .unwrap_or_default();
        format!(
            "{funnel}{filters}   {}/{total} lines shown{note}{off}",
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
        match (self.view.is_truncated(), self.view.estimated_lines()) {
            // Truncated with nothing to scale from: the count is still the
            // preview's, so it stays flagged even without a better number.
            (true, None) => (loaded.to_string(), true),
            (true, Some(estimate)) => (format!("~{estimate}"), true),
            (false, _) => (loaded.to_string(), false),
        }
    }

    /// The directory the explorer is listing.
    fn explorer_dir(&self) -> &std::path::Path {
        self.explorer.dir()
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
        let dir = self.explorer_dir().display().to_string();
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

    /// Draw one pane into `area`.
    ///
    /// The filter pane is handed the `ActiveFilters` it needs; the other two
    /// need nothing beyond themselves. This replaced a free `render_widget`
    /// function that existed only to route around a `Widget` impl one variant
    /// could never satisfy (#75) — each pane is now called directly with the
    /// arguments it actually takes.
    fn render_pane(&mut self, pane: Focus, area: Rect, buf: &mut Buffer) {
        match pane {
            Focus::Explorer => self.explorer.render(area, buf),
            Focus::View => self.view.render(area, buf),
            Focus::Filters => self.filters_pane.render(&self.filters, area, buf),
        }
    }

    /// The one-line notice a cross-file step leaves over the file view until
    /// the next keypress. Centred, bordered, and cleared underneath so it
    /// reads over any text. Nothing is drawn when there was no crossing.
    fn render_crossing(&self, view_area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Block, Clear, Paragraph};
        let Some(crossing) = &self.crossing else {
            return;
        };
        let text = format!(
            "{} {} · {}",
            if crossing.backwards { "▲" } else { "▼" },
            crossing.label(),
            crossing.name
        );
        let width = u16::try_from(UnicodeWidthStr::width(text.as_str()) + 4)
            .unwrap_or(u16::MAX)
            .min(view_area.width);
        if width < 5 || view_area.height < 3 {
            return;
        }
        let x = view_area.x + (view_area.width - width) / 2;
        let y = view_area.y + (view_area.height - 3) / 2;
        let area = Rect {
            x,
            y,
            width,
            height: 3,
        };
        Clear.render(area, buf);
        Paragraph::new(text)
            .centered()
            .block(Block::bordered().border_style(Style::default().fg(Color::Yellow)))
            .render(area, buf);
    }

    /// The keymap warnings, over everything, until the first key.
    ///
    /// Modelled on `render_crossing`: cleared underneath, bordered, and
    /// silent when the area cannot hold it. Not on the help overlay's column
    /// flow — that exists to fit ninety short key rows into columns, and
    /// these are a few long sentences.
    ///
    /// Nothing scrolls. "Any key closes it" and a scroll key cannot both be
    /// true, which is the rule the help overlay already keeps. When the
    /// warnings outrun the space, the border says how many were cut and names
    /// `--print-keymap`, which prints every one of them in full.
    fn render_keymap_warnings(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
        if !self.keymap_warnings_open || self.keymap_warnings.is_empty() {
            return;
        }
        let width = area.width.min(76);
        if width < 20 || area.height < 5 {
            return;
        }
        let inner_width = usize::from(width - 4);

        // How many whole warnings fit, each measured at the rendered rows it
        // will wrap to — not at the count of warnings admitted, which is a
        // different number from the rows they cost once `Wrap` runs.
        let budget = usize::from(area.height.min(20)) - 4;
        let mut lines: Vec<String> = Vec::new();
        // Rendered rows consumed, not warnings admitted. The two are not the
        // same number, and conflating them is what let content overflow a box
        // sized for it.
        let mut rows = 0usize;
        let mut shown = 0;
        for warning in &self.keymap_warnings {
            let text = format!("• {warning}");
            let wrapped = text.len().div_ceil(inner_width.max(1));
            // Every entry after the first also costs the blank row between it
            // and the one before.
            let cost = if lines.is_empty() {
                wrapped
            } else {
                wrapped + 1
            };
            if !lines.is_empty() && rows + cost > budget {
                break;
            }
            rows += cost;
            lines.push(text);
            shown += 1;
        }
        let cut = self.keymap_warnings.len() - shown;

        // Two rows for the closing hint and the blank row above it, two for
        // the borders.
        let height = u16::try_from(rows + 4).unwrap_or(u16::MAX).min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };

        let title = if cut == 0 {
            " Keymap warnings ".to_string()
        } else {
            format!(" Keymap warnings ({cut} more — run recon --print-keymap) ")
        };

        Clear.render(rect, buf);
        Paragraph::new(format!("{}\n\nAny key closes this.", lines.join("\n\n")))
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(title)
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .render(rect, buf);
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

/// The search badge: `/pattern`, padded like the other badges, with the
/// tail elided past `SEARCH_BADGE_MAX` characters.
fn search_badge_text(pattern: &str) -> String {
    let shown: String = pattern.chars().take(SEARCH_BADGE_MAX).collect();
    let ellipsis = if pattern.chars().count() > SEARCH_BADGE_MAX {
        "…"
    } else {
        ""
    };
    format!(" /{shown}{ellipsis} ")
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
        self.panes_area = area;
        self.status_area = prompt_area;

        // The badge mirrors the mode and nothing else. `▼` answers a
        // different question — "are lines missing from the pane right now?" —
        // and is deliberately false when hide mode is armed with nothing
        // including, because the #36 guard in `Document::recompute_visible` is
        // showing the whole file. Both facts are worth reporting, so they get
        // an indicator each; conflating them means one of them lies.
        //
        // Painting the badge here rather than inside `status_text` is what
        // makes that unconditional structurally: `status_text` returns early
        // with an empty string when there are no filters, which is exactly
        // the state the issue was reported from. A badge threaded
        // through that function would need a second conditional to dodge the
        // early return, and a conditional can go stale.
        // `Cow` rather than `&str` (task 8 fix round 2, #199): every other
        // badge is a fixed string, but the stale badge names a key generated
        // from the table, and the search badge carries the pattern, so those
        // own a freshly-built `String` when present.
        //
        // The search badge is what says a search is set and what `n` will
        // step by, now that the filter pane has no row for it (ADR 0001).
        let badges: Vec<Cow<'static, str>> = [
            (self.document.mode() == Mode::FilteredOnly).then_some(Cow::Borrowed(HIDE_BADGE_TEXT)),
            self.filters
                .is_and()
                .then_some(Cow::Borrowed(AND_BADGE_TEXT)),
            self.search
                .as_ref()
                .map(|search| Cow::Owned(search_badge_text(&search.text))),
            self.visual_badge().map(Cow::Borrowed),
            self.view_stale
                .then(|| stale_badge_text(&self.keymap).map(Cow::Owned))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .collect();
        // One column of separation, so the colour block never abuts the text
        // beside it. Taken out of the status text's budget rather than added
        // to the row, so `status_bar_text`'s existing priority order — filter
        // state first, then whatever the path can elide itself into — still
        // has an accurate width to work with on a narrow terminal.
        let badge_width: usize = badges.iter().map(|text| text.chars().count() + 1).sum();
        let room = (prompt_area.width as usize).saturating_sub(badge_width);
        let status = self.status_bar_text(room);

        // Three columns, `[explorer | file view | filter pane]` (#300). A
        // hidden pane gets a zero-wide rectangle and is not drawn; the rest
        // share the width by `layout::columns`.
        let [explorer_area, view_area, filter_area] = self.pane_rects(area);

        // Remember the boundaries so mouse events landing before the next
        // frame can be tested against them. A divider exists only between
        // two shown panes: the explorer's where anything is to its right,
        // the filter pane's only where the file view is to its left — with
        // the view hidden, the filter pane fills what the explorer leaves
        // and the explorer's divider is the one that moves. A missing one is
        // parked at `u16::MAX`, past any real terminal width.
        self.divider = if explorer_area.width > 0 && explorer_area.right() < area.right() {
            explorer_area.right()
        } else {
            u16::MAX
        };
        self.filter_divider = if filter_area.width > 0 && view_area.width > 0 {
            filter_area.x
        } else {
            u16::MAX
        };
        self.explorer_area = explorer_area;
        self.view_area = view_area;
        self.filter_area = filter_area;

        self.set_active_pane();
        self.view.set_title_accent(self.crossing.is_some());
        self.view.set_selection(self.painted_selection());
        for (pane, pane_area) in [
            (Focus::Explorer, explorer_area),
            (Focus::View, view_area),
            (Focus::Filters, filter_area),
        ] {
            if self.panes.is_shown(pane) {
                self.render_pane(pane, pane_area, buf);
            }
        }
        if self.panes.is_shown(Focus::View) {
            self.render_crossing(view_area, buf);
        }

        // Last, so it covers whatever the panes just drew — and over `area`,
        // which is everything above the status row rather than the whole
        // frame. The row below carries the HIDE badge and the current
        // directory, and both are still true while the keymap is up; hiding
        // them would mean the one screen that explains `Ctrl-H` is also the one
        // screen that stops showing whether it is on.
        if let Some(picker) = &self.picker {
            picker.render(area, buf);
        }
        if let Some(picker) = self.set_picker.as_mut() {
            picker.render(area, buf);
        }
        if self.help {
            help::render(area, buf, &self.keymap);
        }
        self.render_keymap_warnings(area, buf);

        // An open prompt takes the rest of the row; nothing but the badge
        // competes with it. The badge stays because the mode it reports is
        // still armed while a filter is being typed — which is precisely when
        // the pane is about to change underfoot.
        //
        // A transient message sits between the prompt and the derived status
        // text: it is more urgent than a filter count (it reports something
        // that just happened, and only lives until the next keypress) and less
        // urgent than a prompt (which the user is actively typing into).
        //
        // A message that arrives while a prompt is open — the wrap a search
        // reports as it is typed — is drawn after the prompt's text rather
        // than dropped: the prompt row is the only row there is, and the
        // message is about what the last keystroke just did.
        let (text, style) = match (self.prompt.as_ref(), self.status_message.as_ref()) {
            (Some(prompt), _) if prompt.error.is_some() => {
                (prompt.line(), Style::default().fg(Color::Red))
            }
            (Some(prompt), Some(message)) => (
                format!("{}   {}", prompt.line(), message.text),
                Style::default(),
            ),
            (Some(prompt), None) => (prompt.line(), Style::default()),
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
        let mut x = prompt_area.x;
        for badge in &badges {
            buf.set_stringn(
                x,
                prompt_area.y,
                badge.as_ref(),
                prompt_area.width as usize,
                HIDE_BADGE_STYLE,
            );
            x += u16::try_from(badge.chars().count() + 1).unwrap_or(u16::MAX);
        }
        buf.set_stringn(
            prompt_area.x + badge_width as u16,
            prompt_area.y,
            text,
            room,
            style,
        );
        // The prompt's cursor, drawn as the file view draws its own: the
        // cell in reversed video. The terminal cursor is hidden for the
        // whole session, so without this an edit in the middle of a pattern
        // (#206) would have nothing on screen to say where the middle is.
        // One blank past the text when the cursor is at the end.
        if let Some(column) = self.prompt.as_ref().and_then(SearchPrompt::cursor_column)
            && column < room
        {
            let x = prompt_area.x + badge_width as u16 + column as u16;
            buf[(x, prompt_area.y)]
                .set_style(style.add_modifier(ratatui::style::Modifier::REVERSED));
        }
    }
}

#[cfg(test)]
mod tests;
