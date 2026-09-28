//! `App`: the terminal UI's state, its event loop, and how it draws.

use color_eyre::Result;
use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::prelude::{Backend, Buffer, Color, Constraint, Layout, Rect, Style, Terminal, Widget};
use std::borrow::Cow;
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

/// What the status row says when `/` or `n` passed the end of the file to
/// find its hit, and when `N` passed the start. One message for the search
/// and for `n` over interesting lines, since both step through
/// `step_visible`.
/// What `E`, `F` or `global.hide.view` says when the pane is the last one
/// shown (#300).
const LAST_PANE: &str = "at least one pane stays shown";
const WRAPPED_TO_TOP: &str = "wrapped to the top";
const WRAPPED_TO_BOTTOM: &str = "wrapped to the bottom";

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

/// How often `poll_stamps` checks the listing's stamps while the feature is on.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

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
    /// `S`: the scratch set's new name. Commits through `save_scratch_as`,
    /// and its error is that function's message rather than
    /// `INVALID_PATTERN` — a name is not a pattern.
    SaveSet,
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
}

/// The search: a regular expression the user typed after `/`, or took from
/// the word under the cursor with `*`.
///
/// A search is a motion, not a filter (ADR 0001). It lives here, on the
/// `App`, and not in the `ActiveFilters`: it never changes which lines are
/// visible, does not dim, marks no file in the explorer, does not answer to
/// `!` or `u`, and is not saved with a filter set. What it does is move the
/// cursor to its next hit among the visible lines and highlight the hits in
/// the window.
#[derive(Debug, Clone)]
struct Search {
    /// The pattern as typed, for the status row and for `p`.
    text: String,
    regex: regex::Regex,
}

impl Search {
    fn new(text: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            text: text.to_owned(),
            regex: regex::Regex::new(text)?,
        })
    }

    /// The character column of the first occurrence in `line`, if the line
    /// is a hit. The cursor lands there: on a long line the hit may be far
    /// off the left edge.
    fn first_column(&self, line: &str) -> Option<usize> {
        let found = self.regex.find(line)?;
        Some(line[..found.start()].chars().count())
    }
}

/// Where the user was when `/` opened (glossary: *origin*), in whichever
/// pane it opened over.
///
/// While the prompt is open every edit re-runs the search from here, never
/// from the position the last keystroke reached, so a pattern narrowed by
/// one more character cannot walk away from where the user started. Esc
/// puts all of it back, so a bad probe costs nothing; Enter drops it.
///
/// One variant per place a `/` can open, rather than one struct with
/// optional parts: each search restores different things, and a prompt
/// is only ever over one pane — the mouse is ignored while it is open, so
/// the focus cannot change under it.
#[derive(Debug, Clone)]
enum Origin {
    /// `/` over the file view or the filter pane, which forwards it.
    View(ViewOrigin),
    /// `/` over the explorer (#272).
    Explorer(ExplorerOrigin),
    /// `/` in the set picker (#285).
    Sets(SetsOrigin),
}

/// The file view's cursor and scroll, and the search that was set at the
/// time.
#[derive(Debug, Clone)]
struct ViewOrigin {
    /// The cursor's row in the visible set, and its column.
    row: usize,
    col: usize,
    /// The pane row the cursor was drawn on, so a restore puts the scroll
    /// back as well as the cursor.
    screen_row: u16,
    /// The search set before `/` opened, if any. Esc restores it with the
    /// cursor: the probe replaced it on screen, not in fact.
    search: Option<Search>,
}

/// The explorer's selected row, and the filename search that was set at
/// the time.
#[derive(Debug, Clone)]
struct ExplorerOrigin {
    /// The selected row as an `entries` index, which a scan answer that
    /// re-lists the rows in hide mode cannot move; the row it is on can.
    entry: usize,
    /// The filename search set before `/` opened, if any. Esc restores it
    /// with the row, so `n` afterwards repeats what it repeated before.
    search: Option<regex::Regex>,
}

/// The set picker's selected row, and the search that was set in it at
/// the time.
#[derive(Debug, Clone)]
struct SetsOrigin {
    row: usize,
    search: Option<regex::Regex>,
}

/// How many committed patterns a `History` keeps before the oldest goes.
const HISTORY_CAP: usize = 50;

/// The patterns Enter committed in one kind of `/` prompt, newest first
/// (#274), so a near-miss regex is recalled and corrected rather than
/// retyped.
///
/// One per prompt kind that has one: the file search, the explorer's
/// filename search and the set picker's search (#285) keep separate
/// histories, because a filename pattern offered in the file prompt is
/// noise. In memory only, so a session starts
/// empty. A pattern committed again moves to the front rather than appearing
/// twice, and the `HISTORY_CAP`th pattern pushes the oldest out. Only Enter
/// adds: a cancelled prompt leaves no trace, and `*` never goes through the
/// prompt at all.
#[derive(Debug, Default)]
struct History {
    /// Newest first: index 0 is what the first Up recalls.
    patterns: Vec<String>,
}

impl History {
    fn push(&mut self, pattern: &str) {
        self.patterns.retain(|kept| kept != pattern);
        self.patterns.insert(0, pattern.to_owned());
        self.patterns.truncate(HISTORY_CAP);
    }

    /// The pattern `steps` back from the newest, or `None` past the oldest.
    fn recall(&self, steps: usize) -> Option<&str> {
        self.patterns.get(steps).map(String::as_str)
    }
}

/// Which way Up and Down walk a `History`: Up towards the oldest pattern,
/// Down back towards the newest and then to an empty prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recall {
    Older,
    Newer,
}

/// A search pattern being typed at the bottom of the screen.
#[derive(Debug, Default)]
struct SearchPrompt {
    pattern: String,
    error: Option<String>,
    kind: PromptKind,
    cursor: usize,
    /// Set for every `/`, whichever pane it opened over: the search moves
    /// as it is typed, and this is where it moves from and where Esc goes
    /// back to. `None` for every filter prompt, which commits on Enter only.
    origin: Option<Origin>,
    /// Which history entry the pattern was recalled from, as steps back
    /// from the newest, or `None` while the prompt shows what was typed
    /// (#274). Up steps it further back, Down towards the newest and then
    /// off the end to an empty prompt. Typing after a recall leaves it
    /// where it is, so the next Up still goes to the entry before.
    recall: Option<usize>,
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
            PromptKind::Search => "/",
            PromptKind::Filter
            | PromptKind::Edit {
                sense: filter::Sense::Include | filter::Sense::Context,
                ..
            } => "filter: ",
            PromptKind::Exclude
            | PromptKind::Edit {
                sense: filter::Sense::Exclude,
                ..
            } => "exclude: ",
            PromptKind::SaveSet => "save as: ",
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

    /// An empty prompt of `kind`, cursor at its start.
    fn new(kind: PromptKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// A prompt pre-filled with `pattern`, cursor at its end — where `c`
    /// starts, so a `Backspace` or a typed character acts on the tail.
    fn editing(pattern: String, kind: PromptKind) -> Self {
        let cursor = pattern.chars().count();
        Self {
            pattern,
            kind,
            cursor,
            ..Self::default()
        }
    }

    /// Where the row draws the cursor: the column after the sigil and the
    /// characters before the cursor. `None` while an error is showing, which
    /// replaces the pattern on the row and has no cursor in it.
    fn cursor_column(&self) -> Option<usize> {
        self.error
            .is_none()
            .then(|| self.sigil().chars().count() + self.cursor)
    }

    /// Byte offset of character `index` — `cursor` is a character index,
    /// because `Left` and `Right` step by character and the row draws by
    /// column, and the pattern is `String`.
    fn byte_at(&self, index: usize) -> usize {
        self.pattern
            .char_indices()
            .nth(index)
            .map_or(self.pattern.len(), |(byte, _)| byte)
    }

    fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.pattern.insert(at, c);
        self.cursor += 1;
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.pattern.chars().count());
    }

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor = self.pattern.chars().count();
    }

    /// Delete the character before the cursor. `false` when there is none
    /// — at the start of the pattern, which is the only place the caller
    /// has to distinguish an empty pattern from a full one.
    fn delete_before(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let (start, end) = (self.byte_at(self.cursor - 1), self.byte_at(self.cursor));
        self.pattern.replace_range(start..end, "");
        self.cursor -= 1;
        true
    }

    /// Delete the character under the cursor; nothing at the end.
    fn delete_at(&mut self) {
        let (start, end) = (self.byte_at(self.cursor), self.byte_at(self.cursor + 1));
        if start < end {
            self.pattern.replace_range(start..end, "");
        }
    }

    /// vim's command-line `Ctrl-w`: blanks between the cursor and the word
    /// before it go, then the word — a run of identifier characters, the
    /// same definition `*` uses, or a run of anything else that is not a
    /// blank, so `foo::` loses `::` and then `foo` in two presses.
    fn delete_word_before(&mut self) {
        let chars: Vec<char> = self.pattern.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        if start > 0 {
            let in_word = is_word(chars[start - 1]);
            while start > 0
                && !chars[start - 1].is_whitespace()
                && is_word(chars[start - 1]) == in_word
            {
                start -= 1;
            }
        }
        self.delete_range(start, self.cursor);
    }

    /// vim's command-line `Ctrl-u`: everything before the cursor goes.
    fn delete_to_start(&mut self) {
        self.delete_range(0, self.cursor);
    }

    /// Delete characters `start..end` and leave the cursor at `start`.
    fn delete_range(&mut self, start: usize, end: usize) {
        let (from, to) = (self.byte_at(start), self.byte_at(end));
        self.pattern.replace_range(from..to, "");
        self.cursor = start;
    }
}

mod layout;
mod mouse;
mod selection;
pub(crate) mod viewport;

use crate::config::Config;
use crate::{
    clipboard, document, editor, emit, filter, filtersets, help, panes, path, scan, widgets,
};
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
use viewport::Step;
use widgets::explorer::Explorer;
use widgets::explorer::Match;
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

/// A cross-file step that just happened, for the notice over the file view
/// and the accent on its title. Lives exactly as long as a `StatusMessage`:
/// until the next keypress.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Crossing {
    backwards: bool,
    name: String,
}

impl Crossing {
    /// The direction word shared by the status report (`cross_file`) and the
    /// notice painted over the view (`render_crossing`), so the two only say
    /// "previous file" and "next file" in one place.
    fn label(&self) -> &'static str {
        if self.backwards {
            "previous file"
        } else {
            "next file"
        }
    }
}

/// A one-off message shown on the status row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusMessage {
    text: String,
    /// Drawn red. Not derivable from the text — "opened src/lib.rs" and
    /// "zed: No such file or directory" are the same shape.
    error: bool,
}

/// The scan cache: one [`scan::Record`] per file, valid for exactly one
/// pattern list in one directory.
///
/// `key` changing shifts bit positions, so every record means something
/// else; the whole cache is dropped and `id` bumped so in-flight results from
/// the old one are ignored on arrival. `dir` changing means different files.
/// A single file's record is dropped alone when its stamp moves.
#[derive(Debug, Default)]
struct ScanCache {
    id: u64,
    key: Vec<String>,
    dir: std::path::PathBuf,
    records: std::collections::HashMap<std::path::PathBuf, scan::Record>,
}

impl ScanCache {
    fn fresh(id: u64, key: Vec<String>, dir: std::path::PathBuf) -> Self {
        Self {
            id,
            key,
            dir,
            records: std::collections::HashMap::new(),
        }
    }
}

/// Everything `refresh_scan` depends on. Equal to last time ⇒ nothing to do.
///
/// The stamp stands in for the pattern list, the masks and the mode (OR or
/// AND, #39 — the same cached bitset answers differently under each), so the
/// comparison allocates nothing (#186). Only a change builds one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScanState {
    stamp: filter::ScanStamp,
    dir: std::path::PathBuf,
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

    /// Feed a key to the open search prompt.
    ///
    /// While it is open it consumes every key, so app-wide commands like `q`
    /// are typed into the pattern rather than acted on.
    fn handle_search_key(&mut self, key: event::KeyEvent) {
        let pressed = crate::keymap::normalise(key);
        if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Prompt, pressed) {
            use crate::keymap::ActionId as A;
            match action {
                A::PromptCancel => self.cancel_prompt(),
                A::PromptCommit => {
                    let Some(prompt) = self.prompt.as_ref() else {
                        return;
                    };
                    let (pattern, kind) = (prompt.pattern.clone(), prompt.kind);

                    // Enter on an empty search is a cancel, not a search
                    // for the empty pattern (which matches every line): the
                    // user is back at the origin, as if Esc.
                    if kind == PromptKind::Search && pattern.is_empty() && prompt.origin.is_some() {
                        self.cancel_prompt();
                        return;
                    }

                    // `SaveSet` is not a pattern: its failures are messages,
                    // shown in the prompt in place of `INVALID_PATTERN`.
                    if kind == PromptKind::SaveSet {
                        match self.save_scratch_as(&pattern) {
                            Ok(()) => {
                                self.prompt = None;
                                self.swallow_next_enter = true;
                            }
                            Err(message) => {
                                if let Some(prompt) = self.prompt.as_mut() {
                                    prompt.error = Some(message);
                                }
                            }
                        }
                        return;
                    }
                    let outcome = match kind {
                        PromptKind::Search => self.run_search(&pattern),
                        PromptKind::SaveSet => unreachable!("handled above"),
                        PromptKind::Filter => self.add_filter(&pattern),
                        PromptKind::Exclude => self.add_excluding_filter(&pattern),
                        PromptKind::Edit { index, .. } => self.replace_filter(index, &pattern),
                    };
                    if outcome.is_ok() {
                        let origin = self.prompt.take().and_then(|prompt| prompt.origin);
                        // Only Enter adds to a history (#274), and only a
                        // `/` prompt has one: the origin says which pane it
                        // opened over, and so which of the two it feeds.
                        if let Some(origin) = &origin {
                            self.history_for(origin).push(&pattern);
                        }
                        // The set picker takes the next key itself, before
                        // the bounce guard below could ever see it, so an
                        // armed guard would wait there and swallow an
                        // `Enter` long after the picker closed.
                        if matches!(origin, Some(Origin::Sets(_))) {
                            return;
                        }
                        // Enter keeps the position the search reached. The
                        // wrap it reported while typing went with the
                        // keystroke, so say it again if the hit is above
                        // where `/` opened: the jump upward was real. The
                        // explorer's search never reported a wrap and
                        // still does not: its `n` is silent about one too.
                        if let Some(Origin::View(origin)) = origin
                            && self.search.is_some()
                            && self.view.cursor_visible_row() < origin.row
                        {
                            self.report(WRAPPED_TO_TOP, false);
                        }
                        // Arm the bounce guard (#48). Only on the branch that
                        // actually closes the prompt: a rejected pattern leaves it
                        // open, so the next `Enter` is another commit attempt and
                        // never reaches the filter pane to be swallowed.
                        self.swallow_next_enter = true;
                        if matches!(
                            kind,
                            PromptKind::Filter | PromptKind::Exclude | PromptKind::Edit { .. }
                        ) {
                            self.return_to_chain_origin();
                        }
                    } else if let Some(prompt) = self.prompt.as_mut() {
                        prompt.error = Some(INVALID_PATTERN.to_string());
                    }
                }
                A::PromptDeleteBack => {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.error = None;
                        // Backspacing past the start of an *empty* prompt
                        // abandons it, as in vim. At the start of a pattern with
                        // text after the cursor there is nothing to delete and
                        // nothing to abandon: the text is what the user is
                        // keeping (#206).
                        if !prompt.delete_before() && prompt.pattern.is_empty() {
                            self.cancel_prompt();
                            return;
                        }
                    }
                    self.rescan_from_origin();
                }
                // The cursor keys (#206): vim's command-line set, plus the
                // readline pair for the ends, which the same hands type at a
                // shell. `Home`/`Ctrl-a` share `PromptStart` and `End`/`Ctrl-e`
                // share `PromptEnd` — two labels, one action, one arm each.
                A::PromptLeft => self.edit_prompt(SearchPrompt::move_left),
                A::PromptRight => self.edit_prompt(SearchPrompt::move_right),
                A::PromptStart => self.edit_prompt(SearchPrompt::move_to_start),
                A::PromptEnd => self.edit_prompt(SearchPrompt::move_to_end),
                A::PromptDeleteForward => self.edit_pattern(SearchPrompt::delete_at),
                A::PromptDeleteWord => self.edit_pattern(SearchPrompt::delete_word_before),
                A::PromptDeleteStart => self.edit_pattern(SearchPrompt::delete_to_start),
                A::PromptHistoryPrev => self.recall(Recall::Older),
                A::PromptHistoryNext => self.recall(Recall::Newer),
                // `resolve(Scope::Prompt, ..)` only ever answers with one of
                // the arms above — see `DEFAULT`'s `Scope::Prompt` rows — so
                // this is unreached today. Not a wildcard omitted by
                // accident: matched explicitly, like `Explorer::perform`'s
                // trailing arm, so a scope neither of us wired stays inert
                // instead of panicking a user's terminal.
                _ => {}
            }
            return;
        }
        match key.code {
            // No prompt binding uses a modified character, and in raw mode
            // a pasted line feed arrives as Ctrl-J (`Char('j')` with
            // CONTROL) rather than as a bare `\n` — dropping it here keeps
            // a pasted newline out of the single-line pattern (#120 §13).
            KeyCode::Char(_)
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            // A paste arrives as one `Char` per character. A newline in it
            // is dropped rather than typed: the pattern is single-line, and
            // a stray `\n` would silently make it match nothing (#120 §13).
            // Matched by guard rather than by literal so the bound-key
            // scanner in `help.rs`, which reads every `'x'` inside `Char(..)`,
            // does not take the escape's backslash for a key.
            KeyCode::Char(c) if c == '\n' || c == '\r' => {}
            // Every character that is not bound is not a binding, and cannot
            // be rebound: it types itself, which is what a prompt is for.
            KeyCode::Char(c) => self.edit_pattern(|prompt| prompt.insert(c)),
            _ => {}
        }
    }

    /// Apply one editing step to the open prompt. Any edit clears a pending
    /// error: the row goes back to showing the pattern, which is what the
    /// user is now correcting.
    fn edit_prompt(&mut self, edit: impl FnOnce(&mut SearchPrompt)) {
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.error = None;
            edit(prompt);
        }
    }

    /// `edit_prompt` for an edit that can change the pattern's text, which
    /// a search prompt answers by moving. The cursor keys go through
    /// `edit_prompt` directly: they change where the next character lands,
    /// not what the pattern says, and a scan per arrow key would be paid
    /// for nothing.
    fn edit_pattern(&mut self, edit: impl FnOnce(&mut SearchPrompt)) {
        self.edit_prompt(edit);
        self.rescan_from_origin();
    }

    /// Up or Down in a `/` prompt (#274): replace the pattern with the next
    /// older or newer committed one, cursor at its end, and move as typing
    /// it would — a recall is an edit, so it goes through `edit_pattern`
    /// and re-runs the scan from the origin. Up at the oldest stays there;
    /// Down past the newest empties the prompt, and Down on an empty prompt
    /// that recalled nothing does nothing. A filter prompt has no origin
    /// and so no history: both keys are inert in it, as they were unbound.
    fn recall(&mut self, direction: Recall) {
        let Some(prompt) = self.prompt.as_ref() else {
            return;
        };
        let Some(origin) = prompt.origin.as_ref() else {
            return;
        };
        let target = match direction {
            Recall::Older => Some(prompt.recall.map_or(0, |steps| steps + 1)),
            Recall::Newer => match prompt.recall {
                None => return,
                Some(steps) => steps.checked_sub(1),
            },
        };
        let history = self.history_for_ref(origin);
        let pattern = match target {
            Some(steps) => match history.recall(steps) {
                Some(pattern) => pattern.to_owned(),
                None => return,
            },
            None => String::new(),
        };
        self.edit_pattern(|prompt| {
            prompt.recall = target;
            prompt.cursor = pattern.chars().count();
            prompt.pattern = pattern;
        });
    }

    /// The history a `/` prompt with this origin reads and feeds: the file
    /// search's for the view (and the filter pane, which forwards `/` to
    /// it), the filename search's for the explorer.
    fn history_for(&mut self, origin: &Origin) -> &mut History {
        match origin {
            Origin::View(_) => &mut self.search_history,
            Origin::Explorer(_) => &mut self.explorer_search_history,
            Origin::Sets(_) => &mut self.sets_search_history,
        }
    }

    /// `history_for`, read-only.
    fn history_for_ref(&self, origin: &Origin) -> &History {
        match origin {
            Origin::View(_) => &self.search_history,
            Origin::Explorer(_) => &self.explorer_search_history,
            Origin::Sets(_) => &self.sets_search_history,
        }
    }

    /// Close the prompt without committing, and put back the origin if the
    /// prompt had one: the cursor, the scroll, and the search that was set
    /// before `/` opened — so the highlight the probe painted goes with it.
    /// In the explorer, the selected row, the preview it had, and the
    /// filename search that was set.
    fn cancel_prompt(&mut self) {
        let origin = self.prompt.take().and_then(|prompt| prompt.origin);
        self.chain_origin = None;
        match origin {
            Some(Origin::View(origin)) => {
                self.search.clone_from(&origin.search);
                self.restore_origin(&origin);
            }
            Some(Origin::Explorer(origin)) => {
                self.explorer.set_search(origin.search);
                self.move_explorer_to(origin.entry);
            }
            Some(Origin::Sets(origin)) => {
                if let Some(picker) = self.set_picker.as_mut() {
                    picker.set_search(origin.search);
                    picker.select(origin.row);
                }
            }
            None => {}
        }
    }

    /// Put the cursor and the scroll back where `/` found them.
    ///
    /// A selection comes back with them (#273): its anchor never moved, and
    /// its other end *is* the view's cursor, so restoring the cursor is
    /// restoring the selection. Nothing here touches `visual`.
    ///
    /// The landing row is requested first: `place_cursor_on_visible_row`
    /// keeps an earlier request when it rebuilds the buffer, and the render
    /// applies it whether or not a rebuild happened, so the cursor is drawn
    /// on the pane row it was on and the scroll follows.
    fn restore_origin(&mut self, origin: &ViewOrigin) {
        self.view.land_cursor_on_row(origin.screen_row);
        self.place_cursor_on_visible_row(origin.row);
        self.view.set_cursor_col(origin.col);
    }

    /// Move the explorer's selection to `entry` and preview it, as a `j`
    /// onto that row would — the view pane follows a filename search the
    /// same way it follows the cursor keys. Nothing when the selection is
    /// already there, so a probe that stays put does not reload the pane.
    fn move_explorer_to(&mut self, entry: usize) {
        if let Some(action) = self.explorer.go_to_entry(entry) {
            self.perform_widget_action(action);
        }
    }

    /// The origin for a `/` opened now, in the pane that has focus: the
    /// explorer's selected row and its filename search, or the file view's
    /// cursor and the search that is set. The filter pane forwards `/` to
    /// the view, so its origin is the view's.
    fn capture_origin(&self) -> Origin {
        match self.focus {
            Focus::Explorer => Origin::Explorer(ExplorerOrigin {
                entry: self.explorer.selected_entry().unwrap_or(0),
                search: self.explorer.search_matcher(),
            }),
            Focus::View | Focus::Filters => Origin::View(ViewOrigin {
                row: self.view.cursor_visible_row(),
                col: self.view.cursor_col(),
                screen_row: self.view.cursor_screen_row(),
                search: self.search.clone(),
            }),
        }
    }

    /// The search moves as it is typed: run it from the origin for the
    /// pattern the prompt holds now, and move to the first hit at or after
    /// the origin — the cursor to a line, with the window's hits
    /// highlighted, or the explorer's selection to an entry, with the
    /// matching names restyled and the view pane previewing it.
    ///
    /// Always from the origin, never from where the last keystroke landed:
    /// `foo` then `food` must not find the next `food` *after* the `foo` the
    /// shorter pattern reached. The scan stops at the first hit, and no
    /// count is kept, so a keystroke costs one walk that ends early.
    ///
    /// A pattern that does not compile yet (`foo(`) is silent: the highlight
    /// clears and the cursor sits at the origin. Only Enter reports it. The
    /// same for an empty pattern, and for one with no hit, which sits at
    /// the origin without a highlight to show.
    fn rescan_from_origin(&mut self) {
        let Some(prompt) = self.prompt.as_ref() else {
            return;
        };
        let (Some(origin), PromptKind::Search) = (prompt.origin.clone(), prompt.kind) else {
            return;
        };
        let pattern = prompt.pattern.clone();
        match origin {
            Origin::View(origin) => self.rescan_view_from(&origin, &pattern),
            Origin::Explorer(origin) => self.rescan_explorer_from(&origin, &pattern),
            Origin::Sets(origin) => self.rescan_sets_from(&origin, &pattern),
        }
    }

    /// `rescan_from_origin` for the file view.
    fn rescan_view_from(&mut self, origin: &ViewOrigin, pattern: &str) {
        let search = if pattern.is_empty() {
            None
        } else {
            Search::new(pattern).ok()
        };
        let Some(search) = search else {
            self.search = None;
            self.restore_origin(origin);
            return;
        };
        self.search = Some(search);
        self.promote_truncated_preview();
        let Some((row, column, wrapped)) = self.hit_from(origin.row) else {
            self.restore_origin(origin);
            return;
        };
        self.jump_to_visible_row(row);
        self.view.set_cursor_col(column);
        if wrapped {
            self.report(WRAPPED_TO_TOP, false);
        }
    }

    /// `rescan_from_origin` for the explorer (#272): the filename search
    /// is set so the matching names light up, and the selection goes to
    /// the first of them at or after the origin row, or back to the origin
    /// row when there is none — or nothing to look for yet.
    fn rescan_explorer_from(&mut self, origin: &ExplorerOrigin, pattern: &str) {
        let matcher = if pattern.is_empty() {
            None
        } else {
            regex::Regex::new(pattern).ok()
        };
        self.explorer.set_search(matcher);
        let entry = self.explorer.hit_from(origin.entry).unwrap_or(origin.entry);
        self.move_explorer_to(entry);
    }

    /// `rescan_from_origin` for the set picker (#285): the same as the
    /// explorer's, over the picker's rows.
    fn rescan_sets_from(&mut self, origin: &SetsOrigin, pattern: &str) {
        let search = if pattern.is_empty() {
            None
        } else {
            regex::Regex::new(pattern).ok()
        };
        if let Some(picker) = self.set_picker.as_mut() {
            picker.set_search(search);
            picker.select(picker.hit_from(origin.row).unwrap_or(origin.row));
        }
    }

    /// Run a committed `/` pattern against whichever pane has focus, or
    /// against the set picker when the prompt opened over it.
    ///
    /// In the explorer, Enter sets the filename search and keeps the row
    /// the typing reached, so `n`/`N` repeat it from there. In the file
    /// view and the filter pane, `/` sets the search and moves to its first
    /// hit.
    fn run_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        // The set picker: as the explorer, nothing moves on Enter, and a
        // pattern no row matches is reported.
        if let Some(Origin::Sets(origin)) = self
            .prompt
            .as_ref()
            .and_then(|prompt| prompt.origin.as_ref())
        {
            let row = origin.row;
            let matcher = regex::Regex::new(pattern)?;
            let mut no_hit = false;
            if let Some(picker) = self.set_picker.as_mut() {
                picker.set_search(Some(matcher));
                no_hit = picker.hit_from(row).is_none();
            }
            if no_hit {
                self.report(&format!("no sets match \"{pattern}\""), false);
            }
            return Ok(());
        }
        match self.focus {
            Focus::Explorer => {
                // Nothing moves: the typing already did. What Enter adds is
                // the report. A pattern no name matches used to close the
                // prompt with nothing moved and nothing said, which a user
                // cannot tell apart from `Esc` (#243); `n`/`N` already
                // report their dead end, so does this one. Checked from the
                // origin, which is where the typing looked from, or from the
                // selection when there was no prompt.
                let matcher = regex::Regex::new(pattern)?;
                let from = match self
                    .prompt
                    .as_ref()
                    .and_then(|prompt| prompt.origin.as_ref())
                {
                    Some(Origin::Explorer(origin)) => origin.entry,
                    _ => self.explorer.selected_entry().unwrap_or(0),
                };
                self.explorer.set_search(Some(matcher));
                if self.explorer.hit_from(from).is_none() {
                    self.report(&format!("no filenames match \"{pattern}\""), false);
                }
                Ok(())
            }
            // The filter pane forwards view-shaped keys to the view (#120):
            // a search started there is the same search.
            Focus::View | Focus::Filters => self.apply_search(pattern),
        }
    }

    /// Set the search and move to its first hit: the first visible line the
    /// pattern matches, from the cursor line — that line included — wrapping
    /// once to the top and saying so. The cursor lands on the column of the
    /// first occurrence. With no hit in the file, the status row says so and
    /// nothing moves.
    ///
    /// The search looks only at the visible lines, so in hide mode it finds
    /// nothing among the lines the filters removed, and it changes none of
    /// them: the visible set, the gutter numbers and the gaps are exactly
    /// what they were. Only the highlight is new, and `apply_view` paints
    /// that from `self.search` on every pass.
    ///
    /// The truncated-preview promotion comes first, as it does for `n` (see
    /// `promote_truncated_preview`). Without it a pattern that only occurs
    /// beyond a large preview's cap would be reported as having no hit. A
    /// peek is left alone: the user is searching the plain file they asked
    /// to see, and unlike a cross-file `n` the search never leaves it.
    ///
    /// A pattern that will not compile is reported and changes nothing, so
    /// the prompt can stay open over an intact previous search.
    fn apply_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let search = Search::new(pattern)?;
        self.search = Some(search);
        self.promote_truncated_preview();
        let from = self.view.cursor_visible_row();
        let Some((row, column, wrapped)) = self.hit_from(from) else {
            self.report_no_hit();
            self.repaint_highlight();
            return Ok(());
        };
        self.jump_to_visible_row(row);
        self.view.set_cursor_col(column);
        if wrapped {
            self.report(WRAPPED_TO_TOP, false);
        }
        Ok(())
    }

    /// Paint the search's highlight (or its absence) without moving anything.
    ///
    /// `apply_view` re-applies the highlight from `self.search` on every
    /// pass, and a jump runs through it, so this is only for the paths that
    /// set or clear the search and jump nowhere. Never after a jump: a jump
    /// queues a landing row, and a second `apply_view` on top of it moves
    /// the cursor. Cheap: nothing is re-evaluated, and the buffer is not
    /// rebuilt.
    fn repaint_highlight(&mut self) {
        self.apply_view(self.cursor_source());
    }

    /// `n`/`N` while a search is set: the next (previous) hit line among the
    /// visible lines, wrapping within this file and saying so. One stop per
    /// line, on the first occurrence's column. Never crosses files: a search
    /// is a motion within the file it was made in, and `n` with no search is
    /// the key that walks the filters' files. So, unlike that `n`, it has no
    /// reason to end a peek first.
    fn step_hit(&mut self, backwards: bool) {
        self.promote_truncated_preview();
        let Some(search) = self.search.clone() else {
            return;
        };
        let is_hit = |document: &Document, row: usize| {
            document
                .source_at(row)
                .and_then(|source| document.lines().get(source))
                .is_some_and(|line| search.regex.is_match(line))
        };
        match self.step_visible(backwards, is_hit) {
            Step::Nothing => {
                self.report_no_hit();
                self.repaint_highlight();
            }
            step => {
                self.land_on_first_occurrence(&search);
                if step == Step::Wrapped {
                    self.report(
                        if backwards {
                            WRAPPED_TO_BOTTOM
                        } else {
                            WRAPPED_TO_TOP
                        },
                        false,
                    );
                }
            }
        }
    }

    /// Put the cursor on the first occurrence of `search` in the line it is
    /// on. A no-op when the line is not a hit, which the stepping above rules
    /// out.
    fn land_on_first_occurrence(&mut self, search: &Search) {
        let row = self.view.cursor_visible_row();
        let column = self
            .document
            .source_at(row)
            .and_then(|source| self.document.lines().get(source))
            .and_then(|line| search.first_column(line));
        if let Some(column) = column {
            self.view.set_cursor_col(column);
        }
    }

    /// `no hit for /pattern`: what `/` and `n` say when the file has none.
    fn report_no_hit(&mut self) {
        let text = self
            .search
            .as_ref()
            .map_or_else(String::new, |search| format!("no hit for /{}", search.text));
        self.report(&text, false);
    }

    /// The first hit at or after visible row `from`, wrapping once: its
    /// visible row, the column of the first occurrence, and whether the
    /// walk passed the end of the file to reach it.
    ///
    /// Row `from` itself is considered first, so a hit on it is found
    /// without a wrap — the difference from `n`, which considers the
    /// cursor's row last.
    fn hit_from(&self, from: usize) -> Option<(usize, usize, bool)> {
        let search = self.search.as_ref()?;
        let visible = self.document.visible();
        let len = visible.len();
        if len == 0 {
            return None;
        }
        let from = from.min(len - 1);
        (0..len).find_map(|step| {
            let row = (from + step) % len;
            let line = self.document.lines().get(visible[row])?;
            let column = search.first_column(line)?;
            Some((row, column, from + step >= len))
        })
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

    /// The one place the mode is set. `Ctrl-H`/`H` is one key with one meaning
    /// in both panes: non-matching *lines* dim or hide in the view, and
    /// non-matching *files* dim or hide in the explorer (#119).
    fn set_mode(&mut self, mode: Mode) {
        self.document.set_mode(mode);
        self.explorer.set_mode(mode);
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
        self.set_mode(mode);
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
            self.set_mode(peek.mode);
        } else {
            self.peek = Some(PeekState {
                mode: self.document.mode(),
                flags: self.filters.enabled_flags(),
            });
            self.filters.set_all_enabled(false);
            // The same flip `toggle_hiding` does, deliberately: `<space>`
            // and `Ctrl-H` move the mode identically, and only the filter
            // switching below is the peek's own.
            self.set_mode(match self.document.mode() {
                Mode::Dimmed => Mode::FilteredOnly,
                Mode::FilteredOnly => Mode::Dimmed,
            });
        }
        // The full `evaluate`, not `recompute_visible` as `toggle_hiding` uses:
        // the enabled flags changed, so every line's verdict can differ. The
        // mode moved too, which `refresh_view` picks up on the same pass.
        self.refresh_view();
    }

    /// Put the filters back before a jump that leaves the peeked file.
    ///
    /// The peek disabled every filter, and the scan that answers "which
    /// files match" was told so: every explorer entry is `Match::Unknown`
    /// until `refresh_scan` runs again — which is normally after this
    /// keypress is dispatched, too late for a cross-file step made now. So
    /// the scan is refreshed here, and the answers come straight back from
    /// the scan cache for every file that has not changed on disk.
    ///
    /// This call only gets past `refresh_scan`'s "state unchanged" guard
    /// because of one invariant: `refresh_scan` records `None` as the last
    /// scan state whenever `self.filters.matcher()` is `None`, which is
    /// exactly what the peek forces by disabling every filter. Calling
    /// `toggle_peek` just above turns `matcher()` from `None` back into
    /// `Some(_)`, so the state computed here differs from the one recorded
    /// while peeked and the guard lets the scan through. Without that
    /// difference `refresh_scan(false)` would be a no-op and the explorer's
    /// answers would still read `Match::Unknown` for this step.
    fn restore_peek_before_moving(&mut self) {
        if self.peek.is_none() {
            return;
        }
        self.toggle_peek();
        self.refresh_scan(false);
    }

    /// Whether the file view is showing a bounded preview rather than the
    /// whole file.
    fn file_view_truncated(&self) -> bool {
        self.view.is_truncated()
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

    /// The output `--emit <kind>` asks for, from what the panes are showing
    /// (#143). Reads the visible sets; computes nothing new.
    fn collect(&self, kind: emit::Emit) -> emit::Exit {
        match kind {
            emit::Emit::Lines => self.collect_lines(),
            emit::Emit::Files => self.collect_files(),
            emit::Emit::Cwd => self.collect_cwd(),
        }
    }

    /// `--emit lines`: the file view's visible lines in the current mode,
    /// with `-n` prefixing each by its 1-based source line number and a tab.
    fn collect_lines(&self) -> emit::Exit {
        if self.view.showing_directory() {
            return emit::Exit::Emit {
                lines: Vec::new(),
                summary: "recon: emitted 0 lines — the view is showing a directory".to_string(),
                failed: 0,
            };
        }
        if !self.view.is_text() {
            return emit::Exit::Emit {
                lines: Vec::new(),
                summary: "recon: emitted 0 lines — the view is showing an error, not a file"
                    .to_string(),
                failed: 0,
            };
        }
        let text = self.document.lines();
        let visible = self.document.visible();
        let lines = visible
            .iter()
            .map(|&source| {
                let mut line = Vec::new();
                if self.line_numbers {
                    line.extend_from_slice(format!("{}\t", source + 1).as_bytes());
                }
                line.extend_from_slice(text[source].as_bytes());
                line
            })
            .collect();
        let name = self.view.filename().display();
        let summary = match self.document.mode() {
            Mode::Dimmed => format!(
                "recon: emitted {} lines of {name}, dim mode ({} match) — Ctrl-H to emit matches only",
                visible.len(),
                self.interesting_count(),
            ),
            Mode::FilteredOnly => {
                format!(
                    "recon: emitted {} lines of {name}, hide mode",
                    visible.len()
                )
            }
        };
        emit::Exit::Emit {
            lines,
            summary,
            failed: 0,
        }
    }

    /// `--emit files`: the explorer's listed files as absolute paths. Hide
    /// mode has already dropped the non-matching rows, so the list is the
    /// matches; dim mode lists every file and the summary says how many
    /// match, and how many the scan has not answered yet.
    ///
    /// The counting branch follows what the explorer can answer, not
    /// whether any filter is switched on: `matcher()` is `None` both with no
    /// including filter enabled (an exclude-only set, or none at all) and
    /// above `MAX_PATTERNS`, and in both states `refresh_scan` never runs, so
    /// every file sits at `Match::Unknown` and a "0 match, N unscanned" line
    /// would describe a scan that will never happen.
    fn collect_files(&self) -> emit::Exit {
        let listed = self.explorer.listed_files();
        let lines = listed
            .iter()
            .map(|file| emit::path_bytes(&file.path))
            .collect();
        let dir = self.explorer.dir().display();
        let count = listed.len();
        let summary = if self.filters.is_scanning() {
            match self.document.mode() {
                Mode::FilteredOnly => format!("recon: emitted {count} files from {dir}, hide mode"),
                Mode::Dimmed => {
                    let matched = listed.iter().filter(|f| f.matched == Some(true)).count();
                    let unscanned = listed.iter().filter(|f| f.matched.is_none()).count();
                    let counts = if unscanned == 0 {
                        format!("{matched} match")
                    } else {
                        format!("{matched} match, {unscanned} unscanned")
                    };
                    format!(
                        "recon: emitted {count} files from {dir}, dim mode ({counts}) — Ctrl-H to emit matches only"
                    )
                }
            }
        } else {
            let mode = match self.document.mode() {
                Mode::Dimmed => "dim mode",
                Mode::FilteredOnly => "hide mode",
            };
            format!("recon: emitted {count} files from {dir}, {mode}, no filter")
        };
        emit::Exit::Emit {
            lines,
            summary,
            failed: 0,
        }
    }

    /// `--emit cwd`: the directory the explorer is showing, one line.
    fn collect_cwd(&self) -> emit::Exit {
        let dir = self.explorer.dir();
        emit::Exit::Emit {
            lines: vec![emit::path_bytes(dir)],
            summary: format!("recon: emitted {}", dir.display()),
            failed: 0,
        }
    }

    const fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running)
    }

    /// Handle any events that have occurred since the last time the app was
    /// rendered, and say whether anything changed.
    ///
    /// Still `Result`, unlike the rest of the chain: `event::poll` and
    /// `event::read` are genuine I/O and can genuinely fail. That is the whole
    /// distinction #80 draws — a `Result` here means something, precisely
    /// because the four functions below it no longer carry one they never use.
    ///
    /// The 1/60 s timeout stays. It is now a *wake* interval rather than a
    /// frame interval: the loop still comes back 60 times a second to check
    /// the editor channel, and draws only if it found something.
    fn handle_events(&mut self) -> Result<bool> {
        // Here rather than in `handle_event`: an editor exits on its own
        // schedule, so nothing the user does is guaranteed to arrive after it.
        let drained = self.drain_editor_outcomes() | self.drain_scan_results() | self.poll_stamps();
        let timeout = Duration::from_secs_f32(1.0 / 60.0);
        if event::poll(timeout)? {
            let event = event::read()?;
            self.handle_event(event);
            return Ok(true);
        }
        Ok(drained)
    }

    /// Dispatch a single event, then let the explorer's scan catch up.
    ///
    /// Split out from the polling loop so that it can be driven directly. The
    /// one thing added around `dispatch_event` is `refresh_scan`: the dispatch
    /// has two dozen early returns, and the scan guard has to run after every
    /// one of them.
    pub fn handle_event(&mut self, event: event::Event) {
        self.dispatch_event(event);
        // Here, beside `refresh_scan`, for the same reason: the dispatch has
        // two dozen early returns and every one of them may have moved
        // focus (#67).
        self.drop_visual_outside_the_view();
        self.refresh_scan(false);
    }

    /// Dispatch a single event: app-wide keys first, then the focused widget.
    ///
    /// Returns nothing. Dispatch cannot fail: every arm either mutates `App`
    /// or hands the event to a pane that also cannot fail. It used to return
    /// `Result<()>`, which cost a `?` at both call sites and an `.unwrap()` in
    /// roughly a hundred tests, and taught a reader to skim past `?` in a file
    /// where `Config::load`, `editor_command` and `set_highlight` genuinely can
    /// fail (#80).
    fn dispatch_event(&mut self, event: event::Event) {
        // First of all the guards. The panel is up before any key is read, so
        // it can never meet an open prompt, and taking the key here is what
        // stops the dismissing key from also quitting, moving a cursor or
        // opening an editor.
        //
        // Only a *key* closes it, for the reason the help overlay gives: mouse
        // capture is on, and a mouse crossing the terminal would wipe a notice
        // the user is still reading.
        if self.keymap_warnings_open {
            if matches!(event, event::Event::Key(_)) {
                self.keymap_warnings_open = false;
            }
            return;
        }

        // The status message lasts until the next *keypress*, and deliberately
        // not until the next event: mouse capture is on, so a mouse moving
        // across the terminal would wipe "zed: No such file or directory" off
        // the row before it could be read. `crossing` follows the same rule
        // for the same reason: its notice and the accent on the title are
        // read at a glance, and a mouse move must not wipe either before that
        // glance happens.
        if matches!(event, event::Event::Key(_)) {
            self.status_message = None;
            self.crossing = None;
        }

        // An open prompt takes precedence over every other binding.
        if self.prompt.is_some() {
            if let event::Event::Key(key) = event {
                self.handle_search_key(key);
            }
            return;
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
            return;
        }

        // The picker takes every key while open, like the search prompt:
        // `q` inside it means nothing, and `Enter` applies rather than
        // toggling whatever the pane has selected underneath. A key that
        // resolves to nothing in `Scope::Picker` is swallowed right here —
        // it is not handed to `Global` or any other scope, and `perform` is
        // never called without an action to give it (task 7, #199).
        if let Some(picker) = self.picker.as_mut() {
            if let event::Event::Key(key) = event {
                let pressed = crate::keymap::normalise(key);
                if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Picker, pressed) {
                    match picker.perform(action) {
                        widgets::picker::PickerOutcome::Open => {}
                        widgets::picker::PickerOutcome::Closed => self.picker = None,
                        widgets::picker::PickerOutcome::Chosen(name) => {
                            let set = picker.set;
                            self.picker = None;
                            self.filters.apply_profile(set, &name);
                            self.refresh_view();
                        }
                    }
                }
            }
            return;
        }

        // The set picker is modal in the same way (#284), and swallows a
        // key that resolves to nothing in `Scope::Sets` for the same reason.
        if let Some(picker) = self.set_picker.as_mut() {
            if let event::Event::Key(key) = event {
                let pressed = crate::keymap::normalise(key);
                if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Sets, pressed) {
                    match picker.perform(action) {
                        widgets::setpicker::SetPickerOutcome::Open => {}
                        widgets::setpicker::SetPickerOutcome::Cancelled => self.set_picker = None,
                        widgets::setpicker::SetPickerOutcome::Applied(changes) => {
                            self.set_picker = None;
                            self.apply_listing(&changes);
                        }
                        widgets::setpicker::SetPickerOutcome::Search => {
                            let origin = Origin::Sets(SetsOrigin {
                                row: picker.selected(),
                                search: picker.search(),
                            });
                            self.prompt = Some(SearchPrompt {
                                origin: Some(origin),
                                ..SearchPrompt::default()
                            });
                        }
                        widgets::setpicker::SetPickerOutcome::NoHit => {
                            self.report("no more matches", false);
                        }
                    }
                }
            }
            return;
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
            return;
        }

        // The 400-line match this replaced (#199) is now a lookup plus a
        // match over actions, in `perform`: each arm there holds exactly the
        // body its key arm held here.
        if let event::Event::Key(key) = event {
            let pressed = crate::keymap::normalise(key);
            if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Global, pressed) {
                self.perform(action, pressed);
                return;
            }
            match key.code {
                // A filter-pane verb pressed anywhere else says so, for one
                // keypress, instead of doing nothing (#120 §9). Not a
                // redirect: making `i` global would collapse `f i` and `i`,
                // and `x`-not-`e` for exclude exists because `e` is a focus
                // key. The chain stays the answer; the hint teaches it.
                // These seven letters are unbound in the explorer and the
                // file view, so this arm shadows nothing.
                //
                // Not resolved through the table (#199): these are guidance,
                // not a binding, and deliberately have no `ActionId` — see
                // the doc comment on `keymap::ActionId`. The hint text is
                // generated from the table (task 8), so it names the key
                // that actually reaches the filter action, not the key this
                // arm happens to match.
                KeyCode::Char(c @ ('i' | 'x' | 'c' | 'd' | 'm' | 'a' | 's'))
                    if key.modifiers.is_empty() && self.focus != Focus::Filters =>
                {
                    use crate::keymap::ActionId as A;
                    let (action, verb) = match c {
                        'i' => (A::FiltersInclude, "adds a filter"),
                        'x' => (A::FiltersExclude, "adds an excluding filter"),
                        'c' => (A::FiltersEdit, "changes the selected filter"),
                        'd' => (A::FiltersDelete, "deletes the selected filter"),
                        'm' => (A::FiltersContext, "toggles include and context"),
                        'a' => (A::FiltersProfile, "picks a profile for the set"),
                        _ => (A::FiltersSolo, "solos the set"),
                    };
                    if let Some(hint) = self.keymap.hint_for(action, verb, A::GlobalFocusFilters) {
                        self.report(&hint, false);
                    }
                    return;
                }
                _ => {}
            }
        }

        if let event::Event::Mouse(mouse) = event
            && self.handle_divider(mouse)
        {
            return;
        }

        // A click that is not on a divider is aimed at a pane or the status
        // row (#58). Handled before the focused-pane dispatch below, because
        // the click decides which pane that is.
        if let event::Event::Mouse(mouse) = event
            && self.handle_click(mouse)
        {
            return;
        }

        // Filter pane keys are routed here rather than through the generic
        // `handle_events` dispatch below: applying them means mutating the
        // `ActiveFilters`, which only `App` owns, so `FilterList` cannot carry
        // them out itself — see `handle_filter_key`.
        if let event::Event::Key(key) = event
            && self.focus == Focus::Filters
        {
            self.handle_filter_key(key);
            return;
        }

        // Every `Scope::View` key resolves here rather than in the widget's
        // own `handle_events`: a few of these mean "the *document's* top" or
        // "the next paragraph anywhere", not "the top of the buffer that
        // happens to be loaded" (#7), and only `App` can see the document to
        // answer that. The rest could be left to the widget, but routing them
        // here too means the table — not a scattered arm — is the one place
        // plan 2b's rebinding has to reach.
        //
        // The old form of this intercept required `key.modifiers.is_empty()`,
        // and that guard *was* #250: a real terminal sets `SHIFT` on every
        // uppercase letter, so `G` carried it, failed the guard, and fell
        // through to the file view's own `G` arm, which only ever saw the
        // loaded buffer. The table normalises the key instead, so the scope
        // decides what a key means and the modifier cannot.
        //
        // An unresolved key is dropped here rather than handed on, and that
        // is what extends the guarantee above from the bound keys to every
        // key. It used to fall through to `Focus::View =>
        // self.forward_to_view(event)` below carrying its original,
        // un-normalised event, and `FileView::handle_events` matches on the
        // character alone (`..` on the modifier fields) — so an unbound
        // *modified* key, `Alt-j` for instance, reached the file view and
        // moved the cursor as if the modifier were never pressed, forcing a
        // truncated preview to a full load on the way in. The explorer and
        // the filter pane never had that gap: both resolve through their own
        // scope and drop an unresolved key rather than forwarding the raw
        // event (`Scope::for_focus` below; `handle_filter_key`). The view
        // matches them now.
        //
        // `[`/`]` are untouched by this, though the widget acts on them and
        // `Scope::View` has no row for either: they resolve in
        // `Scope::Global` above, which is checked first, and
        // `GlobalPageDown`/`GlobalPageUp` hand the widget a rebuilt key
        // directly. The `Focus::View` arm below is left for mouse events,
        // which resolve through no scope at all.
        if let event::Event::Key(key) = event
            && self.focus == Focus::View
        {
            let pressed = crate::keymap::normalise(key);
            if let Some(action) = self.keymap.resolve(crate::keymap::Scope::View, pressed) {
                self.perform(action, pressed);
            }
            return;
        }

        // The file view upgrades its own truncated preview to a full load on
        // first interaction, which rebuilds the textarea and clears its line
        // styles. That happens inside the widget, so it never reaches
        // `perform` — resync here instead, without re-reading the file.
        let was_truncated = self.file_view_truncated();
        let action = match self.focus {
            Focus::Explorer => {
                // The only call site `Scope::for_focus` has (#199): the
                // explorer's key resolves in its own scope here, then
                // `Explorer::perform` carries out whatever it named.
                let resolved = match event {
                    event::Event::Key(key) => {
                        let pressed = crate::keymap::normalise(key);
                        self.keymap
                            .resolve(crate::keymap::Scope::for_focus(self.focus), pressed)
                    }
                    _ => None,
                };
                // `n`/`N` land here whether they came from the user or from
                // `return_to_chain_origin`'s synthetic `n`; either way, a
                // `None` back means there was nothing to step to, and the
                // status row is the only way that reaches the user — the
                // key otherwise does nothing at all. `j`/`k` and every other
                // key that can also return `None` say nothing, so this is
                // gated on the action rather than on the result.
                let is_step_key = matches!(
                    resolved,
                    Some(
                        crate::keymap::ActionId::ExplorerHitNext
                            | crate::keymap::ActionId::ExplorerHitPrev
                    )
                );
                let action = resolved.and_then(|action| self.explorer.perform(action));
                if action.is_none() && is_step_key {
                    // "No matching file" is a claim about every file, and
                    // it is false while the worker is still out (#158):
                    // the chain's synthetic `n` arrives one line after
                    // `refresh_scan` started it, when every mark is still
                    // `Unknown`. Say what is true instead, and let the
                    // redraw `drain_scan_results` asks for land the answer.
                    // With no matcher there is no worker and the marks
                    // stay `Unknown` for good, so that case keeps the
                    // plain answer.
                    let text = if self.explorer.has_search() {
                        "no more matches"
                    } else if self.filters.is_scanning() && self.explorer.any_unscanned() {
                        "scanning…"
                    } else {
                        "no matching file"
                    };
                    self.report(text, false);
                }
                action
            }
            Focus::View => {
                self.forward_to_view(event);
                return;
            }
            // Unreachable: filter-pane keys returned above, through
            // `handle_filter_key`. Applying them means mutating the
            // `ActiveFilters`, and the pane only ever borrows one, so it
            // cannot carry out its own commands.
            Focus::Filters => None,
        };
        if let Some(action) = action {
            self.perform_widget_action(action);
        } else if was_truncated && !self.file_view_truncated() {
            self.sync_document();
            self.refresh_view();
        }
        // Ordinary movement stays inside the window by design, but a page at
        // the edge of the middle third does not — see `window_holds`.
        self.ensure_window();
    }

    /// Run a named action.
    ///
    /// One arm per action, and the only place an action's behaviour lives.
    /// Before this, the same behaviour was reachable from up to four `match`
    /// arms in different files, which is the drift #199 describes.
    ///
    /// `Scope::Global` and `Scope::View` resolve into this. `Explorer`, `Filters`,
    /// `Prompt` and `Picker` never do — each resolves at its own call site
    /// and is carried out there or by its widget's own `perform`, never
    /// through here. Every variant that can never reach this function is
    /// still listed explicitly rather than caught by a wildcard — see the
    /// comment on those two blocks.
    ///
    /// Takes the resolved `Key` alongside the `ActionId`, even though most
    /// arms ignore it: `GlobalFiltersToggle` needs the digit that fired it,
    /// which the action alone cannot carry, and plan 2b's rebinding makes
    /// this a general problem rather than a one-off — a future action can
    /// just as legitimately want to know which of several keys reached it.
    /// The key is always in hand at the call site, since resolution starts
    /// from one, so the parameter costs nothing there.
    fn perform(&mut self, action: crate::keymap::ActionId, pressed: crate::keymap::Key) {
        use crate::keymap::ActionId as A;
        match action {
            // `q`'s `DEFAULT` row carries no modifier (see `label_matches`),
            // so a modified key — e.g. Ctrl-f, which the file view uses for
            // page-down — never resolves to this action and reaches the
            // focused widget instead.
            A::GlobalQuit => self.state = AppState::Quit { emit: true },
            // `Q` quits without emitting (#143).
            A::GlobalQuitSilent => self.state = AppState::Quit { emit: false },
            A::GlobalFocusNext => self.focus_next(),
            A::GlobalFocusPrev => self.focus_prev(),
            A::GlobalFocusExplorer => self.reveal_and_focus(Focus::Explorer),
            A::GlobalFocusView => self.reveal_and_focus(Focus::View),
            // A second `f` while the pane already has focus is the sticky
            // gesture: the user is staying, so no chain to return to.
            A::GlobalFocusFilters => self.start_filter_chain(),
            A::GlobalHideExplorer => self.hide_pane(Focus::Explorer),
            A::GlobalHideView => self.hide_pane(Focus::View),
            A::GlobalHideFilters => self.hide_pane(Focus::Filters),
            A::GlobalHelp => self.help = true,
            A::GlobalSets => {
                self.set_picker = Some(widgets::setpicker::SetPicker::new(self.filters.sets()));
            }
            // Every `/` captures its origin: the search moves as it is
            // typed in every pane, and Esc goes back to where it opened.
            A::GlobalSearch => {
                self.prompt = Some(SearchPrompt {
                    origin: Some(self.capture_origin()),
                    ..SearchPrompt::default()
                });
            }
            // `p` is the bridge from search to filter (ADR 0001): the pattern
            // becomes a numbered include filter in the scratch set, and the
            // search is cleared — the filter's colour replaces the highlight.
            // Nothing happens with no search set; `refresh_view` is not free
            // (`evaluate` is O(lines × filters)), so it is only paid for
            // when the set actually changed.
            //
            // `p`'s `DEFAULT` row carries no modifier, so Ctrl-P and Alt-P
            // are left unclaimed here and fall through to the focused
            // widget, like every other plain-letter global binding.
            A::GlobalSearchPromote => {
                if let Some(search) = self.search.take() {
                    // The pattern compiled as a search, so it compiles as a
                    // filter: the same regex crate, the same syntax.
                    if let Err(err) = self.add_filter(&search.text) {
                        log::warn!("cannot promote the search {:?}: {err}", search.text);
                    }
                }
            }
            A::GlobalSearchWord => {
                if self.focus == Focus::Explorer {
                    if let Some(hint) = self.keymap.hint_for(
                        A::GlobalSearchWord,
                        "searches the word under the cursor",
                        A::GlobalFocusView,
                    ) {
                        self.report(&hint, false);
                    }
                    return;
                }
                match self.word_under_cursor() {
                    // `/` with the typing done, then `n`: the word is on the
                    // cursor's own line by definition, so the scan `/` runs
                    // from that line would find it there and go nowhere.
                    // An escaped literal always compiles; a failure here
                    // would be a regex-crate bug, not user input.
                    Some(word) => match Search::new(&regex::escape(&word)) {
                        Ok(search) => {
                            self.search = Some(search);
                            self.step_hit(false);
                        }
                        Err(_) => self.report("could not search for that word", true),
                    },
                    None => self.report("no word under the cursor", false),
                }
            }
            // Visual mode's `Esc` takes precedence over the search layers
            // below (#67): a stray press should end the selection without
            // also dropping the search the selection was made under.
            A::GlobalEscape => {
                if self.focus == Focus::View && self.visual.is_some() {
                    self.end_visual();
                    return;
                }
                // Layered (#120 §8): the focused pane's own search first,
                // then the file search. Clearing the search clears the
                // highlight with it, which `apply_view` paints from
                // `self.search`; nothing is re-evaluated, since a search
                // never changed a verdict.
                if self.focus == Focus::Explorer && self.explorer.clear_search() {
                    return;
                }
                if self.search.take().is_some() {
                    self.repaint_highlight();
                }
            }
            A::GlobalPeek => self.toggle_peek(),
            A::GlobalFiltersAnd => {
                self.filters.toggle_and();
                self.refresh_view();
            }
            // Three states, because "nothing is enabled" and "nothing was
            // captured" are different situations. Branching on the capture
            // alone makes `!` inert once every filter has been disabled by
            // hand: it would capture all-disabled and then faithfully
            // restore it, forever.
            A::GlobalFiltersDisable => {
                if self.filters.any_enabled() {
                    self.filters.disable_all_remembering();
                } else if self.filters.has_remembered() {
                    self.filters.restore_remembered();
                } else {
                    self.filters.set_all_enabled(true);
                }
                self.refresh_view();
            }
            // Toggle a numbered filter from anywhere (#120 §14). The number
            // is the one the pane draws in its gutter, and `numbered` is the
            // walk that draws it, so key and label cannot disagree. Set
            // headers and built-in filters have no number
            // and no key: `f Enter` covers them.
            //
            // The one arm that reads `pressed`: `resolve` collapses every
            // `'1'..='9'` to this one action (see `DEFAULT`'s "1-9" row), so
            // the digit itself has to come from the key that fired it.
            A::GlobalFiltersToggle => {
                // `to_digit` (rather than `c as u8 - b'0'`) rejects a
                // non-digit outright instead of underflowing: `resolve`
                // today only ever matches this to a `Char('1'..='9')`, but
                // plan 2b lets a `config.toml` bind this action to any key —
                // `!`, say — and `'!' as u8 - b'0'` wraps to a huge `usize`
                // rather than failing loudly.
                let KeyCode::Char(c) = pressed.code else {
                    return;
                };
                let Some(n) = c.to_digit(10).filter(|n| (1..=9).contains(n)) else {
                    return;
                };
                let n = n as usize;
                match widgets::filterlist::numbered(&self.filters).get(n - 1) {
                    Some(&index) => {
                        self.filters.toggle_enabled(index);
                        self.refresh_view();
                    }
                    None => self.report(&format!("no filter {n}"), false),
                }
            }
            // Global, unlike `n`: skipping a file is the outer loop of the
            // review workflow, and it should not matter which pane the inner
            // loop left focus in (#120 §2).
            A::GlobalFileNext => self.skip_file(false),
            A::GlobalFilePrev => self.skip_file(true),
            A::GlobalToggleHide => self.toggle_hiding(),
            A::GlobalZoomView => self.zoom_file_view(),
            A::GlobalZoomFocused => self.zoom_focused(),
            A::GlobalEditorProject => {
                let template = self.editor.project.clone();
                self.open_in_editor(&template, EditorScope::Project);
            }
            A::GlobalEditorFile => {
                let template = self.editor.file.clone();
                self.open_in_editor(&template, EditorScope::File);
            }
            A::GlobalReload => {
                self.explorer.reload();
                // `r` is the one place the listing is stat'd on this thread:
                // it is asked for, and `explorer.reload` has just read the
                // directory here anyway. `refresh_scan` trusts its records
                // (#156), so without this a change would wait for the poll.
                self.check_stamps();
                self.refresh_scan(true);
                self.reload_active_file();
            }
            // `v`/`V` start, switch or end a selection; from any pane but the
            // view they hint instead, as `*` does from the explorer (#67,
            // #120 §9) — there is no cursor column there to anchor to.
            A::GlobalVisualChar | A::GlobalVisualLine => {
                if self.focus == Focus::View {
                    self.promote_truncated_preview();
                    self.toggle_visual(matches!(action, A::GlobalVisualLine));
                } else {
                    // Always `GlobalVisualChar`, not `action`: this hint is
                    // shared by both `v` and `V`, and it deliberately always
                    // names lowercase `v` (task 8 fix round 1, #199) — using
                    // `action` here would have `V` describe itself.
                    if let Some(hint) = self.keymap.hint_for(
                        A::GlobalVisualChar,
                        "selects text in the file view",
                        A::GlobalFocusView,
                    ) {
                        self.report(&hint, false);
                    }
                }
            }
            // `y`'s `DEFAULT` row carries no modifier, so Ctrl-y is left
            // unclaimed here and still reaches the file view's own
            // scroll-up binding.
            A::GlobalYank => {
                if self.focus == Focus::View {
                    self.yank();
                } else {
                    // The trailing key is `v` (`GlobalVisualChar`), not `y`:
                    // copying needs a selection first, made with `v`, so the
                    // hint says "focus the view, then press v" rather than
                    // "then press y" (task 8 fix round 1, #199).
                    if let Some(hint) = self.keymap.hint_for_trailing(
                        A::GlobalYank,
                        "copies a selection in the file view",
                        A::GlobalFocusView,
                        A::GlobalVisualChar,
                    ) {
                        self.report(&hint, false);
                    }
                }
            }
            // `[`/`]` are bound only in `Scope::Global` — there is no
            // `Scope::View` row for them — and Global resolves before any
            // focus check, so this arm fires from every pane, the file view
            // included (#120 §3). `ActionId` carries no key of its own to
            // forward, so this rebuilds the canonical key and hands it
            // straight to `FileView::handle_events`, bypassing `Scope::View`
            // entirely, where the widget's own raw `[`/`]` arms do the
            // actual scrolling.
            A::GlobalPageDown => self.forward_to_view(event::Event::Key(KeyCode::Char(']').into())),
            A::GlobalPageUp => self.forward_to_view(event::Event::Key(KeyCode::Char('[').into())),
            // The four long-range motions: only `App` can see the whole
            // document, so `long_range_target` answers "which visible row"
            // and this places the cursor there, promoting a truncated
            // preview first exactly as the old intercept did (#7, #250).
            A::ViewGotoStart | A::ViewGotoEnd | A::ViewParagraphNext | A::ViewParagraphPrev => {
                if let Some(target) = self.long_range_target(action) {
                    self.promote_truncated_preview();
                    self.jump_to_visible_row(target);
                }
            }
            // Bound the same way in the view and the filter pane; only `App`
            // can see the document, so `n`/`N` land here rather than in
            // either widget.
            A::HitNext => self.step_interesting(false),
            A::HitPrev => self.step_interesting(true),
            // Everything else in the view scope has a live `FileView` arm
            // already, so this rebuilds the canonical key for that binding
            // and forwards it — same shape as `GlobalPageDown`/`GlobalPageUp`
            // above, and for the same reason: an `ActionId` carries no key of
            // its own. Forwarding the canonical key rather than `pressed`
            // matters once plan 2b lets a user rebind these — a rebound key
            // would otherwise reach `FileView` with no arm that matches it.
            A::ViewLeft => self.forward_to_view(event::Event::Key(KeyCode::Char('h').into())),
            A::ViewRight => self.forward_to_view(event::Event::Key(KeyCode::Char('l').into())),
            A::ViewUp => self.forward_to_view(event::Event::Key(KeyCode::Char('k').into())),
            A::ViewDown => self.forward_to_view(event::Event::Key(KeyCode::Char('j').into())),
            A::ViewWordForward => {
                self.forward_to_view(event::Event::Key(KeyCode::Char('w').into()));
            }
            A::ViewLineStart => self.forward_to_view(event::Event::Key(KeyCode::Char('0').into())),
            A::ViewLineEnd => self.forward_to_view(event::Event::Key(KeyCode::Char('$').into())),
            A::ViewToggleLineNumbers => {
                self.forward_to_view(event::Event::Key(KeyCode::Char('#').into()));
            }
            A::ViewScrollDown => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('e'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewScrollUp => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('y'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewHalfPageDown => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewHalfPageUp => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('u'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewPageDown => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('f'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewPageUp => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL,
            ))),
            // Not yet wired, listed rather than caught by a wildcard (#199):
            // a wildcard here would strip the exhaustiveness check this
            // match exists to keep, and plan 2b's rebinding can reach a
            // stray variant from a user's `config.toml`, not just from a
            // future arm nobody wrote. Each group below is deleted by the
            // task that gives it a real arm, so a variant left behind after
            // that task lands is a build error rather than a silent no-op.
            //
            // Task 6 gave the explorer and filter-pane scopes their real
            // arms, but neither lives here: `Scope::Explorer` resolves at the
            // per-focus dispatch and is carried out by `Explorer::perform`,
            // and `Scope::Filters` resolves inside `handle_filter_key` and
            // is carried out there or by `FilterList::perform`. Neither call
            // site routes its result through this function, so no code path
            // today produces one of these variants here.
            //
            // Not `unreachable!()`, though: plan 2b lets a user's
            // `config.toml` bind a key in `Scope::Global` to one of these
            // actions, and `resolve(Scope::Global, key)` would then hand it
            // straight to this function — a typo in a config file, not a
            // programmer error, so it must not crash the TUI. `debug_assert!`
            // still catches a programmer error loudly in tests and debug
            // builds; a release binary just does nothing.
            A::ExplorerUp
            | A::ExplorerDown
            | A::ExplorerParent
            | A::ExplorerOpen
            | A::ExplorerGotoStart
            | A::ExplorerGotoEnd
            | A::ExplorerHalfPageDown
            | A::ExplorerHalfPageUp
            | A::ExplorerPageDown
            | A::ExplorerPageUp
            | A::ExplorerHitNext
            | A::ExplorerHitPrev
            | A::FiltersUp
            | A::FiltersDown
            | A::FiltersGotoStart
            | A::FiltersGotoEnd
            | A::FiltersHalfPageDown
            | A::FiltersHalfPageUp
            | A::FiltersPageDown
            | A::FiltersPageUp
            | A::FiltersToggle
            | A::FiltersInclude
            | A::FiltersExclude
            | A::FiltersEdit
            | A::FiltersDelete
            | A::FiltersContext
            | A::FiltersProfile
            | A::FiltersSolo
            | A::FiltersReset
            | A::FiltersSaveSet => {
                debug_assert!(
                    false,
                    "{action:?} resolves against its own widget, never through `perform`"
                );
            }
            // The modal scopes (task 7, #199): `Scope::Prompt` resolves in
            // `handle_search_key` and `Scope::Picker` resolves in
            // `dispatch_event`'s picker guard, and each is carried out right
            // there — neither ever reaches this function, for the reason
            // Ruling 30 gives: a modal's `None` case must swallow the key
            // and return, not fall through to `Global` the way every pane
            // scope does, so there is no path from a modal scope into
            // `perform` for a fallthrough to take. Same `debug_assert!` as
            // the group above, and the same reason it exists: plan 2b's
            // `config.toml` can still bind a `Scope::Global` key to one of
            // these actions, and `resolve(Scope::Global, key)` would hand it
            // straight here. The help overlay dismisses on any key and has
            // no `ActionId` of its own, so it has no arm here at all.
            A::PromptCommit
            | A::PromptCancel
            | A::PromptLeft
            | A::PromptRight
            | A::PromptStart
            | A::PromptEnd
            | A::PromptDeleteBack
            | A::PromptDeleteForward
            | A::PromptDeleteWord
            | A::PromptDeleteStart
            | A::PromptHistoryPrev
            | A::PromptHistoryNext
            | A::PickerUp
            | A::PickerDown
            | A::PickerChoose
            | A::PickerCancel
            | A::SetsUp
            | A::SetsDown
            | A::SetsToggle
            | A::SetsApply
            | A::SetsCancel
            | A::SetsSearch
            | A::SetsHitNext
            | A::SetsHitPrev => {
                debug_assert!(
                    false,
                    "{action:?} resolves in its own modal dispatch, never through `perform`"
                );
            }
        }
    }

    /// Force the file view's truncated preview to a full load, the same
    /// thing its own `handle_events` does on first interaction. Needed by
    /// every path that moves the cursor or evaluates a pattern directly
    /// rather than going through that dispatch — see `promote_truncated_preview`,
    /// which wraps this for those callers.
    fn promote_file_view(&mut self) {
        let path = self.view.filename().to_path_buf();
        self.view.load(&path);
    }

    /// Re-read the active file, and put the cursor back on the line it was on.
    ///
    /// `load` rebuilds the buffer from the top, so this remembers the cursor's
    /// *source* line first and re-places it afterwards — the machinery a
    /// filter change already uses to rebuild without losing the reader's
    /// place. A file that shrank underneath the cursor (logrotate) simply
    /// clamps to what is left.
    fn reload_active_file(&mut self) {
        let path = self.view.filename().to_path_buf();
        if path.as_os_str().is_empty() {
            return;
        }
        let source = self.cursor_source();
        self.view.load(&path);
        self.sync_document();
        self.document.evaluate(&self.filters);
        let row = self
            .document
            .nearest_visible(source)
            .and_then(|nearest| self.document.visible_position(nearest))
            .unwrap_or(0);
        self.place_cursor_on_visible_row(row);
        self.view_stale = false;
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

    /// Hand one event to the file view, whichever pane has focus, with the
    /// same after-care the focused dispatch gives it: a truncated preview
    /// that promoted itself on this keypress is resynced without re-reading
    /// the file, and the window is checked after a page at its edge.
    fn forward_to_view(&mut self, event: event::Event) {
        let was_truncated = self.file_view_truncated();
        self.view.handle_events(event.into());
        if was_truncated && !self.file_view_truncated() {
            self.sync_document();
            self.refresh_view();
        }
        self.ensure_window();
    }

    /// `n`/`N` in the file view. With a search set, the next hit line in this
    /// file — see `step_hit`. Otherwise the next interesting line in this
    /// file, else the first interesting line of the next file the filters
    /// selected, else (when this is the only such file) wrap within it as
    /// `n` always has.
    ///
    /// The in-file step comes first so the loop the key drives — every hit in
    /// every file — never skips a hit. The cross-file step is what makes it a
    /// single loop rather than one per file (#120 §1).
    fn step_interesting(&mut self, backwards: bool) {
        if self.search.is_some() {
            self.step_hit(backwards);
            return;
        }
        // A jump that leaves the peeked context has nothing to come back to,
        // and with every filter disabled by the peek the step would find no
        // interesting line and cross files at once. Restore first (#120 §4).
        self.restore_peek_before_moving();
        // `n`/`N` bypass the widget's own `handle_events`, which is where a
        // truncated preview normally promotes itself on first interaction —
        // see `promote_truncated_preview`, which `apply_search` also calls
        // for the same reason.
        self.promote_truncated_preview();
        if let Some(target) = self.next_interesting_strict(backwards) {
            self.land_on(target);
            return;
        }
        if !self.cross_file(backwards) {
            // `step_to_interesting` is quiet about what it did, and a key
            // whose whole job is finding a hit should not be: say when the
            // walk passed the file's edge, and say when there was nothing.
            match self.step_to_interesting(backwards) {
                Step::Nothing => self.report("no interesting line", false),
                Step::Wrapped => self.report(
                    if backwards {
                        WRAPPED_TO_BOTTOM
                    } else {
                        WRAPPED_TO_TOP
                    },
                    false,
                ),
                Step::Landed => {}
            }
        }
    }

    /// Select, load and land in the next (previous) file the filters
    /// selected. `false` when there is no *other* such file — the explorer
    /// wraps, so "the only match is the one we are in" comes back as an
    /// unchanged selection rather than `None`.
    ///
    /// Reports the crossing three ways, all gone by the next keypress: the
    /// status row, the notice `render` paints over the file view, and the
    /// accent on the view's title. Log files look alike, and a step that
    /// silently changed which one is on screen would be worse than no step.
    fn cross_file(&mut self, backwards: bool) -> bool {
        let before = self.explorer.selected_entry();
        let Some(action) = self.explorer.step_to_match(backwards) else {
            return false;
        };
        if self.explorer.selected_entry() == before {
            return false;
        }
        self.perform_widget_action(action);
        self.promote_truncated_preview();
        if let Some(target) = self.first_interesting(backwards) {
            self.land_on(target);
        }
        let name = self.explorer.selected_name().unwrap_or_default();
        let crossing = Crossing { backwards, name };
        self.report(&format!("{} · {}", crossing.label(), crossing.name), false);
        self.crossing = Some(crossing);
        true
    }

    /// `.`/`,`: the cross-file half of `n`/`N`, without first exhausting the
    /// current file. Global, so the loop can skip a file from any pane.
    fn skip_file(&mut self, backwards: bool) {
        self.restore_peek_before_moving();
        if !self.cross_file(backwards) {
            self.report("no other file matches", false);
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
        let relative = self.view.filename().to_path_buf();
        if relative.as_os_str().is_empty() {
            self.report("nothing to open", true);
            return;
        }

        // `filename` is set even when the read failed — the pane shows the
        // error in place of the file's text — so a path that is not there is
        // the ordinary "the argument was a typo" case, not an impossible one.
        if !relative.exists() {
            self.report(
                &format!("cannot open {}: no such file", relative.display()),
                true,
            );
            return;
        }

        // Absolute, per the `{file}` contract. recon's working directory is not
        // the editor's — a GUI editor launched from a dock or a launcher agent
        // inherits neither — so a relative path is the one input guaranteed to
        // be interpreted differently at the far end.
        //
        // `lexical_absolute` rather than `canonicalize`: it does not touch the
        // filesystem and does not resolve symlinks, so the editor opens the
        // path the explorer is showing rather than wherever it happens to
        // point. For a file reached through a symlinked directory, that is the
        // one the user can find their way back to.
        //
        // That claim was false until #78. `Explorer::set_dir` canonicalized, so
        // the explorer had *already* resolved the link before this ran and
        // there was nothing left here to preserve. All three sites share this
        // one function now, which is what makes the sentence above true.
        let file = path::lexical_absolute(&relative);

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
        let program = argv
            .first()
            .map(|program| program.to_string_lossy().into_owned())
            .unwrap_or_default();
        match self.launcher.spawn(&argv) {
            // Reported rather than silent: a GUI editor can take seconds to
            // raise a window, and a key that appears to have done nothing is a
            // key that gets pressed again.
            Ok(()) => self.report(&format!("{program}: opening {}", file.display()), false),
            Err(err) => self.report(&format!("{program}: {err}"), true),
        }
    }

    /// `S`: write the scratch set to `filters.toml` as `name`, then adopt it
    /// in memory so the pane shows what a restart would show — without
    /// discarding any other set's current state (#131).
    ///
    /// The written text is re-parsed before anything is written or changed,
    /// so a file recon could not load back is never produced. Errors are
    /// messages for the prompt's error line; the scratch set is untouched
    /// on every one of them.
    fn save_scratch_as(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a set needs a name".into());
        }
        if self.filters.sets().iter().any(|set| set.name == name) {
            return Err(format!(
                "a set named {name:?} already exists; edit filters.toml to change it"
            ));
        }
        let Some(path) = self.save_path.clone() else {
            return Err("no config home ($XDG_CONFIG_HOME, $HOME unset); nowhere to save".into());
        };
        // Two scratch filters with one pattern would be two file filters
        // answering to the same name, which `parse` rejects with advice
        // about a `name` key the pane cannot set. Say it in the pane's
        // terms instead (#190).
        let mut patterns: Vec<String> = self
            .filters
            .filters_in(0)
            .map(|(_, filter)| filter.predicate.display())
            .collect();
        patterns.sort_unstable();
        if let Some([shared, _]) = patterns.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(format!(
                "two scratch filters share the pattern {shared:?}; delete one before saving"
            ));
        }
        let to_save = filtersets::SetToSave {
            name,
            filters: self
                .filters
                .filters_in(0)
                .map(|(_, filter)| (filter.predicate.display(), filter.sense))
                .collect(),
            default: self
                .filters
                .filters_in(0)
                .filter(|(_, filter)| filter.enabled)
                .map(|(_, filter)| filter.display_name())
                .collect(),
        };
        let before = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(format!("could not read {}: {err}", path.display())),
        };
        let after = filtersets::append_set(&before, &to_save)?;
        filtersets::parse(&after, &path).map_err(|err| err.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
        }
        // Write beside the file and rename over it (#153): `fs::write`
        // truncates first, so a crash, a `kill` or a full disk between the
        // truncate and the write would leave the user's hand-edited file
        // empty or partial, and the next start refuses to run on it. The
        // rename is atomic on every filesystem recon runs on, so the file is
        // always either the old text or the new.
        let file_name = path
            .file_name()
            .map_or_else(|| "filters.toml".into(), std::ffi::OsStr::to_os_string);
        let mut tmp_name = file_name;
        tmp_name.push(".tmp");
        let tmp = path.with_file_name(tmp_name);
        std::fs::write(&tmp, after)
            .map_err(|err| format!("could not write {}: {err}", tmp.display()))?;
        if let Err(err) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("could not replace {}: {err}", path.display()));
        }
        self.filters.adopt_scratch_as(name, path);
        self.refresh_view();
        self.report(&format!("saved set {name:?}"), false);
        Ok(())
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
    ///
    /// Returns whether it changed anything. An editor exits on its own
    /// schedule, so this is the one source of change with no keypress behind
    /// it — which makes it the reason `handle_events` cannot simply report
    /// "did an event arrive?" and be done.
    fn drain_editor_outcomes(&mut self) -> bool {
        let Some(outcomes) = self.editor_outcomes.as_ref() else {
            return false;
        };
        let mut latest = None;
        while let Ok(message) = outcomes.try_recv() {
            latest = Some(message);
        }
        let Some(text) = latest else {
            return false;
        };
        self.report(&text, true);
        true
    }

    /// Decide whether the explorer's answers need work, and start it (#119).
    ///
    /// Cheap-idempotent unless `force`: it compares the pattern generation, the
    /// masks and the directory to what it saw last time and returns at once
    /// if nothing moved. Runs after every event, so that guard is what keeps a
    /// keystroke in the file view from walking the listing at all.
    ///
    /// When it proceeds, every file the explorer lists is answered from the
    /// cache if it can be — `Record::answer` — and put on a request if it
    /// cannot. A toggle whose every answer is cached issues no request and
    /// touches no thread; that is the whole point of caching bitsets rather
    /// than answers.
    fn refresh_scan(&mut self, force: bool) {
        // Compared in place: this runs after every mouse move, and building
        // the pattern key, the directory and a clone of the set only to find
        // them unchanged was the cost of the common case (#186).
        let stamp = self.filters.scan_stamp();
        let unchanged = self
            .last_scan
            .as_ref()
            .map(|last| (last.stamp, last.dir.as_path()))
            == stamp.map(|stamp| (stamp, self.explorer.dir()));
        if !force && unchanged {
            return;
        }
        let dir = self.explorer.dir().to_path_buf();
        self.last_scan = stamp.map(|stamp| ScanState {
            stamp,
            dir: dir.clone(),
        });

        let Some(matcher) = self.filters.matcher() else {
            // Nothing selects: the feature is off, not "nothing matches".
            for (index, _) in self.explorer.files() {
                self.explorer.set_answer(index, Match::Unknown);
            }
            self.scanner.cancel();
            self.explorer.restyle();
            return;
        };

        let key = self.filters.pattern_key();
        if self.scan_cache.key != key || self.scan_cache.dir != dir {
            self.scan_cache = ScanCache::fresh(self.scan_cache.id + 1, key, dir);
        }

        // No `stat` here (#156): a held record is trusted as it stands.
        // `poll_stamps` finds a file that changed, off this thread, and a
        // resumed scan re-checks its own stamp in the worker.
        let mut pending = Vec::new();
        for (index, path) in self.explorer.files() {
            let answer = self
                .scan_cache
                .records
                .get(&path)
                .map(|record| self.answer_to_match(record, &matcher));
            let matched = if let Some(matched @ (Match::Yes(_) | Match::No)) = answer {
                matched
            } else {
                let (stamp, progress) = self
                    .scan_cache
                    .records
                    .get(&path)
                    .map(|record| (record.stamp, record.progress.clone()))
                    .unwrap_or_default();
                pending.push(scan::FileToScan {
                    index,
                    path,
                    stamp,
                    progress,
                });
                Match::Unknown
            };
            self.explorer.set_answer(index, matched);
        }

        if pending.is_empty() {
            self.scanner.cancel();
        } else {
            self.scanner.start(scan::Request {
                cache_id: self.scan_cache.id,
                matcher,
                files: pending,
            });
        }
        self.explorer.restyle();
    }

    /// Move scan results into the cache and the explorer, reporting whether
    /// anything on screen changed.
    ///
    /// A result is dropped if its cache id is stale — the pattern list changed
    /// while it was in flight, so its bitsets mean something else. Otherwise
    /// it replaces the held record only if it read further; a cancelled
    /// worker's partial can arrive after the fresh worker's complete. The row
    /// it names is checked against the path it is for before the explorer is
    /// told anything: the listing may have changed under it.
    fn drain_scan_results(&mut self) -> bool {
        let Some(results) = self.scan_results.as_ref() else {
            return false;
        };
        let matcher = self.filters.matcher();
        let mut changed = false;
        loop {
            let scanned = match results.try_recv() {
                Ok(scanned) => scanned,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::warn!(
                        "the scan worker is gone; answers stay unknown until the next change"
                    );
                    break;
                }
            };
            if scanned.cache_id != self.scan_cache.id {
                continue;
            }
            let further = self
                .scan_cache
                .records
                .get(&scanned.path)
                .is_none_or(|held| {
                    // A new stamp is a new file: the worker started it over,
                    // so it can have read less and still be the truth (#156).
                    scanned.stamp != held.stamp
                        || scanned.progress.scanned_to > held.progress.scanned_to
                        || (scanned.progress.eof && !held.progress.eof)
                });
            if !further {
                continue;
            }
            let record = scan::Record {
                stamp: scanned.stamp,
                progress: scanned.progress,
            };
            let matched = matcher
                .as_ref()
                .map_or(Match::Unknown, |m| self.answer_to_match(&record, m));
            self.scan_cache.records.insert(scanned.path.clone(), record);
            if self.explorer.path_at(scanned.index).as_ref() == Some(&scanned.path) {
                changed |= self.explorer.set_answer(scanned.index, matched);
            }
        }
        if changed {
            self.explorer.restyle();
        }
        changed
    }

    /// Re-stat the listing every `POLL_INTERVAL` while the feature is on.
    /// Returns whether anything changed.
    ///
    /// The `stat`s run on a thread of their own (#156): one per listed file
    /// is nothing for a small local folder and a visible stall for 20,000
    /// files or a network mount. This tick only starts a check and, on a
    /// later tick, applies its answer. One check is in flight at a time, so
    /// a slow mount cannot pile threads up.
    fn poll_stamps(&mut self) -> bool {
        let changed = self.drain_stamp_check();
        if !self.filters.is_scanning() || self.stamp_check.is_some() {
            return changed;
        }
        let now = Instant::now();
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < POLL_INTERVAL)
        {
            return changed;
        }
        self.last_poll = Some(now);
        self.stamp_check = scan::check_in_background(self.held_stamps());
        changed
    }

    /// Apply the in-flight stamp check's answer, if it has arrived.
    fn drain_stamp_check(&mut self) -> bool {
        let Some(check) = self.stamp_check.as_ref() else {
            return false;
        };
        match check.try_recv() {
            Ok(moved) => {
                self.stamp_check = None;
                self.apply_moved(moved)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                log::warn!("the stamp check ended without an answer");
                self.stamp_check = None;
                false
            }
        }
    }

    /// Every listed file that has a record, with the stamp the record holds —
    /// what a stamp check compares the disk against.
    fn held_stamps(&self) -> Vec<(std::path::PathBuf, Option<scan::Stamp>)> {
        self.explorer
            .files()
            .into_iter()
            .filter_map(|(_, path)| {
                let stamp = self.scan_cache.records.get(&path)?.stamp;
                Some((path, stamp))
            })
            .collect()
    }

    /// Drop and forget the records of files that moved on disk, then hand off
    /// to `refresh_scan(true)` to rescan them. The active file moving also
    /// raises the badge.
    ///
    /// The check ran on a snapshot, so each record is compared again before
    /// it is dropped: a scan result that arrived in the meantime may already
    /// carry the new stamp, and that record is kept.
    ///
    /// Deliberately does not issue its own request: `refresh_scan`'s `pending`
    /// is every file without a usable answer, which already covers the files
    /// this drops. Issuing a narrower request here would hand `Scanner::start`
    /// a file list that cancels an in-flight full scan without covering the
    /// files it had not reached yet, stranding them `Unknown` until `r`.
    fn apply_moved(&mut self, moved: Vec<scan::Moved>) -> bool {
        if !self.filters.is_scanning() {
            return false;
        }
        let moved: std::collections::HashMap<_, _> = moved
            .into_iter()
            .map(|scan::Moved { path, stamp }| (path, stamp))
            .collect();
        let active = self.view.filename().to_path_buf();
        let mut changed = false;
        for (index, path) in self.explorer.files() {
            let Some(stamp) = moved.get(&path) else {
                continue;
            };
            let Some(held) = self.scan_cache.records.get(&path) else {
                continue;
            };
            if held.stamp == *stamp {
                continue;
            }
            self.scan_cache.records.remove(&path);
            self.explorer.set_answer(index, Match::Unknown);
            if path == active {
                self.view_stale = true;
            }
            changed = true;
        }
        if changed {
            self.refresh_scan(true);
        }
        changed
    }

    /// A stamp check run to completion on this thread — what `poll_stamps`
    /// does over two ticks, without the thread or the wait. For `r`, which
    /// asks for it, and for tests.
    fn check_stamps(&mut self) -> bool {
        let moved = scan::moved(self.held_stamps());
        self.apply_moved(moved)
    }

    /// A record's answer as the explorer's `Match`, with the owning filter's
    /// colour on a yes.
    fn answer_to_match(&self, record: &scan::Record, matcher: &filter::Matcher) -> Match {
        match record.answer(matcher) {
            Some(true) => Match::Yes(self.match_style(record.owner(matcher))),
            Some(false) => Match::No,
            None => Match::Unknown,
        }
    }

    /// The style the view would draw a line selected by `owner` with. The
    /// explorer draws the file's name in it, so the two panes agree at a
    /// glance and the colour says *which* filter picked the file.
    fn match_style(&self, owner: Option<filter::Owner>) -> Style {
        owner
            .and_then(|index| self.filters.style_for(filter::Verdict::Included(index)))
            .unwrap_or_default()
    }

    /// Carry out an action on behalf of the widget that raised it.
    ///
    /// Named for the widget rather than plain `perform` (#199): that name now
    /// belongs to the keymap table's own dispatcher below, which runs a
    /// `keymap::ActionId` rather than one of these.
    fn perform_widget_action(&mut self, action: Action) {
        match &action {
            Action::Load(path) => self.view.load(path),
            Action::LoadAndFocus(path) => {
                self.view.load(path);
                // `reveal_and_focus`, not `self.focus = ...`: `b` and `z` can
                // leave the file view off screen, and opening a file into a
                // pane the user cannot see would be worse than not moving at
                // all. This is the same reason the focus keys route here.
                self.reveal_and_focus(Focus::View);
            }
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
        let lines = self.view.source().clone();
        // The hide toggle describes how the user is reading, not which file
        // they are reading, so it outlives the document exactly as the filter
        // set does — and for the same reason. The filters survived a load
        // only because `App` owns them separately from the `Document` this
        // line replaces; the mode lives *on* the document, so without
        // carrying it across, every load and every explorer preview silently
        // reset it to `Mode::default()`.
        //
        // That made the toggle almost unusable for its main purpose: skimming
        // a directory for the files a filter actually matches means moving
        // the explorer's selection, and every move fired a `Preview` through
        // here and undid the `Ctrl-H` that made the skim possible.
        let mode = self.document.mode();
        self.document = Document::for_file(self.view.filename(), lines);
        self.set_mode(mode);
        // The anchor was a line of the document this just replaced (#67).
        self.visual = None;
        // The buffer the view is showing belongs to the *previous* document,
        // so the record of what it was built from is meaningless now.
        // Clearing it forces the next `apply_view` to rebuild: two different
        // documents can easily carry an equal generation — every fresh one
        // starts at 0, and reloading the same file with a filter active
        // lands on the same count every time — which would otherwise leave
        // the just-loaded, unfiltered buffer in place under numbers and
        // styles sized for the filtered subset.
        self.last_generation = None;
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
        let rows = widgets::filterlist::rows(&self.filters).len();
        self.filters_pane.clamp_selection(rows);

        // The cursor is a source line index for the duration of the rebuild:
        // its row in the view is only meaningful against the old visible list.
        let cursor_source = self.cursor_source();
        self.document.evaluate(&self.filters);
        self.apply_view(cursor_source);
    }

    /// List or unlist each `(set, listed)` the set picker hands back (#284),
    /// then update the view once.
    ///
    /// The filter pane's cursor stays on the row it was on when that row is
    /// still there. Its index can move — a set above it may have gained or
    /// lost its rows — so it is followed by what it addresses, not by its
    /// position. A row that went with its set leaves the cursor where
    /// `refresh_view`'s clamp puts it.
    fn apply_listing(&mut self, changes: &[(usize, bool)]) {
        if changes.is_empty() {
            return;
        }
        // A peek holds every filter's flag by position, and a list or an
        // unlist moves the positions (#305). Put the flags back first, while
        // they still line up.
        if self.peek.is_some() {
            self.toggle_peek();
        }
        let before = widgets::filterlist::rows(&self.filters);
        let selected = self
            .filters_pane
            .selected()
            .and_then(|index| before.get(index).copied());
        for &(set, listed) in changes {
            self.filters.set_listed(set, listed);
        }
        if let Some(row) = selected {
            let after = widgets::filterlist::rows(&self.filters);
            self.filters_pane.follow(row, &after);
        }
        self.refresh_view();
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
    /// The `Edit` command is the exception and returns before that
    /// re-evaluate: it only opens a prompt, and nothing about the set changes
    /// until it commits — at which point `replace_filter` does the
    /// re-evaluating instead.
    ///
    /// `Scope::Filters` resolves here rather than through `perform`: `i`,
    /// `x` and `S` open a prompt, which only `App` owns, so they are carried
    /// out directly below; everything else is delegated to
    /// `FilterList::perform`, which only reports a command — only `App` can
    /// mutate the `ActiveFilters` the command names.
    fn handle_filter_key(&mut self, key: event::KeyEvent) {
        use crate::keymap::ActionId as A;

        // The explorer's `h`/`l` in this pane: a hint, not a redirect, for
        // the same reason as the filter verbs elsewhere (#120 §9). The keys
        // themselves have no `ActionId` in `Scope::Filters` — `FilterList`
        // cannot report, since the status row is `App`'s — so this stays a
        // pre-resolution special case, guarded on an empty modifier set as
        // before; but the text names the explorer actions they point at, so it
        // is generated from the table (task 8) rather than hand-written.
        if key.modifiers.is_empty() {
            match key.code {
                KeyCode::Char('h') => {
                    if let Some(hint) = self.keymap.hint_for(
                        A::ExplorerParent,
                        "goes up a directory",
                        A::GlobalFocusExplorer,
                    ) {
                        self.report(&hint, false);
                    }
                    return;
                }
                KeyCode::Char('l') => {
                    if let Some(hint) = self.keymap.hint_for(
                        A::ExplorerOpen,
                        "opens the entry",
                        A::GlobalFocusExplorer,
                    ) {
                        self.report(&hint, false);
                    }
                    return;
                }
                _ => {}
            }
        }

        let pressed = crate::keymap::normalise(key);
        let Some(action) = self.keymap.resolve(crate::keymap::Scope::Filters, pressed) else {
            return;
        };

        match action {
            // `i` and `x` open a prompt, and `self.prompt` is `App`'s —
            // `FilterList` cannot carry these out itself. Deliberately not
            // `FilterCommand` variants: that enum describes mutations of the
            // `ActiveFilters`, and opening a prompt is not one — see its doc
            // comment in `widgets/mod.rs`.
            A::FiltersInclude => self.prompt = Some(SearchPrompt::new(PromptKind::Filter)),
            A::FiltersExclude => self.prompt = Some(SearchPrompt::new(PromptKind::Exclude)),
            // `S` saves the scratch set (#131). Refused before the prompt
            // opens when there is nothing to save: a prompt for a name that
            // can go nowhere is worse than a message.
            A::FiltersSaveSet => {
                if self.filters.filters_in(0).next().is_none() {
                    self.report("nothing to save: the scratch set is empty", false);
                    return;
                }
                self.prompt = Some(SearchPrompt::new(PromptKind::SaveSet));
            }
            // Bound the same way in the file view (`Scope::View`); only
            // `App` can see the document, so this makes the same call the
            // view's arm in `perform` makes, rather than delegating to
            // `FilterList`, which has no "next" of its own.
            A::HitNext | A::HitPrev => self.perform(action, pressed),
            _ => {
                let rows = widgets::filterlist::rows(&self.filters);
                if let Some(command) = self.filters_pane.perform(action, &rows) {
                    self.apply_filter_command(command);
                }
            }
        }
    }

    /// Carry out a command the filter pane reported, from a key or a click
    /// (#58). The pane only borrows the `ActiveFilters` it draws, so this is
    /// where every mutation it asks for actually happens.
    fn apply_filter_command(&mut self, command: FilterCommand) {
        match command {
            FilterCommand::Toggle(index) => {
                self.filters.toggle_enabled(index);
            }
            FilterCommand::Delete(index) => {
                self.filters.remove(index);
            }
            FilterCommand::ToggleContext(index) => {
                self.filters.toggle_context(index);
            }
            FilterCommand::ToggleSet(set) => {
                self.filters.toggle_set(set);
            }
            FilterCommand::Solo(set) => {
                self.filters.solo(set);
            }
            FilterCommand::Reset => {
                self.filters.reset();
            }
            // Opens the picker, or says why not; the set is untouched until
            // a profile is chosen, so nothing to re-evaluate here.
            FilterCommand::PickProfile(set) => {
                let names: Vec<String> =
                    self.filters.sets()[set].profiles.keys().cloned().collect();
                if names.is_empty() {
                    self.report("no profiles in this set", false);
                } else {
                    self.picker = Some(widgets::picker::ProfilePicker::new(set, names));
                }
                return;
            }
            FilterCommand::BuiltInIsReadOnly => {
                self.report(
                    "built-in filters can be switched off or collapsed, not deleted or edited",
                    false,
                );
                return;
            }
            // Nothing to re-evaluate: the model did not change.
            FilterCommand::SetIsReadOnly => {
                self.report(
                    "sets are defined in filters.toml; edit the file to change one",
                    false,
                );
                return;
            }
            // The one command that changes nothing yet — it opens a prompt,
            // and the set is only touched if it commits. It returns early
            // rather than falling through to the `refresh_view` below: there
            // is nothing to re-evaluate, and `evaluate` is O(lines × filters).
            FilterCommand::Edit(index) => {
                // The row the pane reported is one it drew, so the filter is
                // there; falling out silently rather than indexing keeps that
                // a property of the pane's own bounds, not a promise this
                // function has to make.
                if let Some(filter) = self.filters.filters().get(index) {
                    self.prompt = Some(SearchPrompt::editing(
                        filter.predicate.display(),
                        PromptKind::Edit {
                            index,
                            sense: filter.sense,
                        },
                    ));
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

    /// Move focus to the next shown pane, left to right, wrapping.
    ///
    /// A hidden pane is skipped (#300): focus is only ever on a pane that is
    /// on the screen, so the cursor is never somewhere the user cannot see.
    fn focus_next(&mut self) {
        self.chain_origin = None;
        self.focus = self.panes.next(self.focus);
    }

    /// `Shift-Tab`: the other way round, with the same skip.
    fn focus_prev(&mut self) {
        self.chain_origin = None;
        self.focus = self.panes.prev(self.focus);
    }

    /// `z`: hide every pane but the focused one, or — when it is already the
    /// only one shown — show the others again. See `Panes::zoom`.
    fn zoom_focused(&mut self) {
        self.chain_origin = None;
        self.cancel_drag();
        self.panes.zoom(self.focus);
    }

    /// `b`: `t`, then `z` (#300). From a split, the file view is focused and
    /// takes the whole width; pressed again with only the view shown, the
    /// others come back and focus stays in the file view — you pressed `b` to
    /// read the file, so that is where you want to stay.
    fn zoom_file_view(&mut self) {
        self.reveal_and_focus(Focus::View);
        self.zoom_focused();
    }

    /// Focus `pane`, showing it first if it is hidden, so the cursor never
    /// lands on a pane the user cannot see.
    ///
    /// Only `pane` is shown. The focus keys used to clear a zoom outright;
    /// now a zoom is only a hide, and `e` after `b` puts the explorer beside
    /// the view rather than bringing back the filter pane too.
    fn reveal_and_focus(&mut self, pane: Focus) {
        self.chain_origin = None;
        self.panes.show(pane);
        self.focus = pane;
    }

    /// `E`, `F` and `global.hide.view`: take `pane` off the screen.
    ///
    /// The last shown pane is refused, with a message: a window with no
    /// pane in it has no key a user could find to undo it. Focus leaves a
    /// hidden pane by `Panes::hide`'s rule.
    fn hide_pane(&mut self, pane: Focus) {
        match self.panes.hide(pane, self.focus) {
            Ok(focus) => {
                if focus != self.focus {
                    self.chain_origin = None;
                    self.focus = focus;
                }
                self.cancel_drag();
            }
            Err(panes::LastPane) => self.report(LAST_PANE, false),
        }
    }

    /// A drag in progress has no divider to keep tracking once a pane is
    /// hidden or shown — the `Drag` arm in `handle_divider` only checks
    /// `self.dragging`, not whether that divider is still on the screen — so
    /// it would otherwise go on silently re-pinning a width nothing explains.
    /// Every layout change cancels it outright.
    fn cancel_drag(&mut self) {
        self.dragging = None;
    }

    /// `f`: focus the filter pane, and remember where focus came from so a
    /// chain — `f i … Enter` — can go back there. A filter pane that was
    /// hidden is shown for the chain and hidden again when it returns.
    fn start_filter_chain(&mut self) {
        let origin = (self.focus != Focus::Filters).then_some(self.focus);
        let shown = !self.panes.is_shown(Focus::Filters);
        self.reveal_and_focus(Focus::Filters);
        self.chain_origin = origin;
        self.chain_shown_filters = shown && origin.is_some();
    }

    /// End a chain that just committed: focus goes back to where `f` was
    /// pressed, and the app behaves as if `n` were pressed there — the
    /// first `fn` after `f i fn Enter` from the view, the next matching
    /// file from the explorer. Dispatching a real `n` rather than calling
    /// either step directly is what keeps "as if `n`" true per pane.
    ///
    /// Nothing to do when `f` was not what brought focus here.
    fn return_to_chain_origin(&mut self) {
        let Some(origin) = self.chain_origin.take() else {
            return;
        };
        self.reveal_and_focus(origin);
        // `f` showed the filter pane to start this chain; the chain is over,
        // so it goes back to hidden. A chain does not change the layout.
        if std::mem::take(&mut self.chain_shown_filters) && origin != Focus::Filters {
            let _ = self.panes.hide(Focus::Filters, self.focus);
        }
        // The commit that got us here changed the filter set, but the
        // outer `handle_event` loop has not run `refresh_scan` yet — this
        // is still inside the same `dispatch_event` that is doing the
        // committing. Re-key the marks to `Unknown` now, before the
        // synthetic `n` below reads them, or it would step to (or fail to
        // find) a match against the filter set the user just replaced.
        self.refresh_scan(false);
        self.dispatch_event(event::Event::Key(event::KeyEvent::from(KeyCode::Char('n'))));
        // The synthetic `n` spent the bounce guard; re-arm it, since the
        // `Enter` that committed is still the last key the user pressed.
        self.swallow_next_enter = true;
    }

    /// Mark each pane as focused or not, before drawing.
    ///
    /// Three assignments rather than an enumerate-and-compare over a vec: the
    /// index that loop compared against no longer exists (#73).
    fn set_active_pane(&mut self) {
        self.explorer.set_active(self.focus == Focus::Explorer);
        self.view.set_active(self.focus == Focus::View);
        self.filters_pane.set_active(self.focus == Focus::Filters);
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
