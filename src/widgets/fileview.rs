#[cfg(test)]
use crate::document::BINARY_SNIFF_BYTES;
use crate::document::{self, Sniff, read_lossy_line, read_utf16_lines, sniff};
use crate::syntax::{Highlighter, Span, Theme};
use crate::widgets::filenav::Entry;
/// `FileView` Widget
///
///
use color_eyre::Result;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tui_textarea::{CursorMove, Input, Key, Scrolling, TextArea};
use unicode_width::UnicodeWidthStr;

/// Lines read for a preview.
///
/// Not "enough to fill the pane", which is what this was: at 500 lines it
/// truncated essentially every real file recon is pointed at, and a truncated
/// document is one that filters and counts report wrong answers over — the
/// defect in #27. The cap exists to bound work on a genuinely enormous log,
/// not to avoid reading a log-sized log.
///
/// 50,000 is ~5x the largest file in the corpus this was measured against
/// (a few thousand real log files: p50 1,084 lines, max 10,000), and
/// `MAX_PREVIEW_BYTES` is the binding limit well before it at any realistic
/// line length.
///
/// `pub(crate)` for the tests in `lib.rs`: reaching the truncated branch
/// through `App` means a fixture past this cap, and one that hard-coded the
/// number would quietly stop testing truncation the next time it moves.
pub(crate) const PREVIEW_LINES: usize = 50_000;

/// Byte ceiling for a preview. A file with no newlines is a single enormous
/// line, which the line cap alone would happily read in full.
///
/// 10 MiB is ~10x the largest measured file and bounds the worst-case blocking
/// read at roughly 10 ms — read, `Document` clone and a filter pass together
/// run about 1 ms/MB, so this stays far inside the ~100 ms a delay would have
/// to reach before anyone noticed it.
const MAX_PREVIEW_BYTES: u64 = 10 << 20;

/// Pane height assumed before this pane has rendered even once.
///
/// Deliberately generous rather than small. An over-large window costs a few
/// hundred extra `String` clones for a single frame; an under-large one leaves
/// the bottom of the pane blank until the next rebuild, which is a visible
/// defect. Taller than any realistic terminal, so the first frame is always
/// fully populated and the real height takes over from the second.
pub(crate) const ASSUMED_PANE_HEIGHT: u16 = 200;

/// Rows of context kept between the cursor and the pane's top or bottom edge
/// while there is file beyond that edge to show — vim's `scrolloff`.
///
/// Without it a held `j` pins the cursor to the pane's last row and every
/// line below the selection is invisible until it is selected; the same for
/// `k` and the first row. Five is the value most vim and helix configurations
/// settle on: enough to read the line in its surroundings, small enough that
/// a short pane still has a majority of rows the cursor can move through
/// without scrolling. Shrunk on a pane too short to afford it — see
/// `scroll_margin`.
pub(crate) const SCROLL_MARGIN: usize = 5;

/// The margin a pane `height` rows tall can afford: `SCROLL_MARGIN`, or less
/// on a pane so short that five rows each side would leave the cursor no
/// row that is not in a margin. The cursor always keeps at least one row of
/// its own — on a 3-row pane the margin is 1, on a 1-row pane it is 0.
pub(crate) fn scroll_margin(height: usize) -> usize {
    SCROLL_MARGIN.min(height.saturating_sub(1) / 2)
}

/// How many screens of slack the window keeps beyond **each edge of the
/// viewport** — see `WINDOW_SCREENS` for why it is two rather than one.
const SLACK_SCREENS: usize = 2;

/// How many screens of the visible set `TextArea` is given: the viewport plus
/// `SLACK_SCREENS` above and below.
///
/// The slack is what keeps today's scroll behaviour intact. With a window of
/// exactly the viewport, every single `j` would run off the buffer's end and
/// force a `set_lines` rebuild — resetting the viewport and re-entering the
/// `pending_screen_row` machinery on the hottest path in the app. With slack
/// either side, ordinary movement (`j`, `k`, `Ctrl-E`, `Ctrl-Y`, and a page in
/// either direction) happens entirely inside the buffer and `TextArea` handles
/// it exactly as before.
///
/// **Two screens of slack, not one** (#108). The slack has two jobs, and they
/// add rather than overlap:
///
/// - one screen so that a single page in either direction *completes* instead
///   of being clamped by `Viewport::scroll`'s `saturating_sub`, and
/// - one screen of headroom so that ordinary scrolling can eat into the slack
///   for a while before a rebuild is owed.
///
/// With only one screen the two collide: the window would be built with exactly
/// a page of slack, `window_holds` would demand exactly a page, and the very
/// next `j` with the cursor on the pane's bottom row would owe a rebuild — a
/// `set_lines` per keystroke while holding `j`, which is the cost #7 exists to
/// avoid. Five screens is ~120 lines on a 24-row terminal. The memory that
/// issue reclaims is measured in gigabytes; this is not a tradeoff.
pub(crate) const WINDOW_SCREENS: usize = SLACK_SCREENS * 2 + 1;

/// The slice of the visible set to hand `TextArea`, as `(start, end)` indices
/// into it, given where the cursor sits in that set and which row of the pane
/// it is drawn on.
///
/// Returns the whole visible set when it is no longer than the window. That
/// degenerate case is not a special path — it is what makes a single code path
/// serve every file size without a threshold, and it is the case almost every
/// test in this repo exercises, since a fixture shorter than three screens is
/// windowed to itself and behaves exactly as it did before #7.
///
/// **Anchored on the viewport, not the cursor** (#108). The slack has to be
/// measured from the pane's edges because that is what a page scrolls: `[` and
/// `]` move the viewport, and `TextArea` clamps that move to the buffer it has.
/// The cursor sits somewhere *inside* the viewport — `[` leaves it on the
/// bottom row, `]` on the top — so slack measured from the cursor is short by
/// `screen_row` above and by `height - screen_row` below.
///
/// That shortfall was the whole of #108: on a 36-row terminal `[` parks the
/// cursor 32 rows below the viewport's top edge, leaving `35 - 32 = 3` rows of
/// buffer above it, and `Viewport::scroll`'s `saturating_sub` quietly clamped a
/// 33-row page to 3. Subtracting `screen_row` as well puts `SLACK_SCREENS`
/// above the viewport's top and the same below its bottom, whatever row the
/// cursor happens to be on.
pub(crate) fn window_for(
    visible_len: usize,
    height: u16,
    row: usize,
    screen_row: usize,
) -> (usize, usize) {
    let height = height.max(1) as usize;
    let span = height * WINDOW_SCREENS;
    if visible_len <= span {
        return (0, visible_len);
    }
    // Measured from the viewport's top edge, which is `screen_row` rows above
    // the cursor. Pulled back at the tail so the window stays a full span
    // rather than shrinking against the end of the document.
    let viewport_top = row.saturating_sub(screen_row);
    let start = viewport_top
        .saturating_sub(height * SLACK_SCREENS)
        .min(visible_len - span);
    (start, start + span)
}

/// Whether the window `start..end` still leaves a full screen beyond the
/// viewport in both directions, or a rebuild is owed.
///
/// **A page of buffer beyond each viewport edge.** Keeping that much intact
/// guarantees any single move of at most a page completes inside the buffer and
/// `TextArea` never clamps it short. `window_for` lays down `SLACK_SCREENS`, so
/// there is a screen of scrolling to spend before this comes due.
///
/// That guarantee is the point, not the tidiness. With a half-screen margin the
/// *second* consecutive `PageDown` runs into the buffer's end and is silently
/// truncated — the page moves less than a page, and the user sees a stutter
/// with no explanation.
///
/// Measured from the viewport's edges rather than the cursor's row, matching
/// `window_for`. The rule used to be "the cursor stays in the middle third",
/// which is the same thing only when the cursor is centred: `[` parks it on the
/// bottom row, where the middle-third test still passed with a full screen
/// nominally above the *cursor* but only three rows above the *viewport*. See
/// `window_for` for the arithmetic and #108 for what it cost.
///
/// A side that is already the end of the document never triggers a rebuild:
/// there is nothing further to window onto.
pub(crate) fn window_holds(
    visible_len: usize,
    start: usize,
    end: usize,
    row: usize,
    screen_row: usize,
) -> bool {
    if row < start || row >= end.max(start + 1) {
        return false;
    }
    // The height the window was built for; a full span is three screens of it.
    let height = (end - start) / WINDOW_SCREENS;
    if height == 0 {
        // Too short to have thirds; holding the row at all is enough.
        return true;
    }
    let viewport_top = row.saturating_sub(screen_row);
    let viewport_bottom = viewport_top + height - 1;
    let room_above = start == 0 || viewport_top >= start + height;
    let room_below = end >= visible_len || viewport_bottom + height < end;
    room_above && room_below
}

/// Shown in place of the contents when a file is not a document at all.
///
/// It used to say `not valid UTF-8`, which was both the check and a libel: a
/// single stray byte in a log condemned the whole file. The verdict now rests
/// on a NUL in the file's head, so the message names what was actually found
/// — undecodable bytes on their own no longer stop the file being read, and
/// a UTF-16 byte-order mark explains its NULs away (#165).
const BINARY_MESSAGE: &str = "<binary file: contains NUL bytes>";

/// Shown when the navigator's selection is a directory with nothing in it.
///
/// A directory now renders as its listing, so this survives only for the case
/// where there is no listing to show. Distinguished from `<directory>`, which
/// this used to be: "I gave you a directory and got nothing" is the one case
/// that could otherwise read as a bug rather than as an answer.
const EMPTY_DIRECTORY_MESSAGE: &str = "<empty directory>";

/// What `read_lines` or `read_preview` found: the lines it read, whether more
/// remain, and how many lines the whole file probably has.
struct Contents {
    lines: Vec<String>,
    truncated: bool,
    /// `None` when the file was read whole (the count is not a guess then, it
    /// is `lines.len()`) or when there was nothing to estimate from.
    estimated_lines: Option<usize>,
    /// Whether `lines` are the file's own text, as opposed to a directory
    /// listing or a message standing in for a file that could not be shown.
    /// Only text is syntax-coloured (#122): `<binary file>` beside a `.rs`
    /// path is not Rust, and a listing is not whatever its directory is
    /// named after.
    text: bool,
}

impl Contents {
    /// Contents that are really an error or a placeholder. Not truncated —
    /// there is nothing better to re-read later — and nothing to estimate.
    fn message(text: String) -> Self {
        Self {
            lines: vec![text],
            truncated: false,
            estimated_lines: None,
            text: false,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct FileView<'a> {
    /// The file being shown, as a path rather than a rendering of one.
    ///
    /// A `String` here meant storing `Path::display()`, which is explicitly
    /// lossy — so a name that is not valid UTF-8 came back as U+FFFD and every
    /// site that round-tripped it into a `Path` to re-read the file addressed
    /// something that does not exist (#79). Rendered at the one place that
    /// should render it: the pane title.
    filename: PathBuf,
    /// The lines as read — the whole file, or as much of it as `preview`
    /// took — shared with the `Document` that filters them (#122).
    ///
    /// `textarea` used to be the only copy, which was fine while it held the
    /// whole file and stopped being true when it became a window (#7):
    /// syntax colouring parses from the top of the file down, so it needs
    /// the lines the window was cut from, not the window. An `Arc` so that
    /// `App::sync_document` shares this rather than cloning it.
    source: Arc<Vec<String>>,
    /// Whether `source` is file text rather than a listing or a message —
    /// see `Contents::text`.
    text: bool,
    textarea: TextArea<'a>,
    /// The colours syntax colouring paints with. `Off` by default, so a
    /// `FileView` built without one renders exactly as it did before #122;
    /// `App::new` sets it from the config.
    theme: Theme,
    /// The colouring of `source`, filled in as rows are rendered. `None`
    /// when the theme is off, when no grammar claims the file, or when
    /// `source` is not text.
    highlighter: Option<Highlighter>,
    /// Showing a bounded preview rather than the whole file.
    truncated: bool,
    /// Roughly how many lines the whole file holds, while only a preview of it
    /// is loaded. `None` once the file is read in full, where the count is not
    /// a guess, and when there was nothing to estimate from.
    ///
    /// `read_preview` has always computed this to size the gutter; keeping it
    /// is what lets the status row report a truthful total, since `truncated`
    /// alone says the document's length is wrong without saying what is right.
    estimated_lines: Option<usize>,
    active: bool,
    /// Draw the title in the accent colour for this frame. Set by `App` for
    /// the one keypress after a cross-file step, so the eye catches that the
    /// file changed even when the notice is dismissed by the same key that
    /// raised it (#120).
    title_accent: bool,
    /// Whether the line-number gutter is drawn. Toggled with `#`.
    hide_line_numbers: bool,
    /// Set when the buffer currently holds the single blank placeholder line
    /// substituted by `show_lines_with_cursor` for an empty visible set (e.g.
    /// everything is hidden). An empty `line_numbers` override falls back to
    /// natural 1..N numbering, which would render "1" beside that blank row
    /// and read as "this file has one empty line" — so the gutter is
    /// suppressed outright instead.
    gutter_blank: bool,
    /// Set while the buffer holds a directory listing rather than a file.
    ///
    /// A third, independent reason to suppress the gutter — line numbers
    /// beside filenames number nothing. Deliberately *not* implemented by
    /// saving and restoring `hide_line_numbers`: a `#` pressed while a
    /// directory is on screen would then be clobbered on the way out, and the
    /// two values could disagree. A condition re-evaluated per render has
    /// nothing to keep in sync, which is the same shape `gutter_blank` uses.
    showing_directory: bool,
    /// A `scroll_cursor_to_row` request not yet applied. Applied against the
    /// real area the next time this pane renders — see the `Widget` impl —
    /// rather than acted on immediately, since the caller (a filter or
    /// hide-toggle rebuild) always runs before this frame has rendered even
    /// once, so there is no real area to scroll against yet.
    ///
    /// `get_or_insert`, not overwritten, if a second request arrives before
    /// the next render: the first row captured was measured against a
    /// viewport that was still valid, and a later one would be answering
    /// the same question against a viewport already disturbed by the first
    /// rebuild — see `scroll_cursor_to_row`.
    pending_screen_row: Option<u16>,
    /// Visible-set index of the buffer's **first row** (#7).
    ///
    /// `TextArea` holds a window of the visible set, not all of it, so
    /// `textarea.cursor().0` is an index into that window. The visible-set row
    /// is `window_start + textarea.cursor().0` — see `cursor_visible_row`,
    /// which is the only place that arithmetic is written down.
    ///
    /// Zero whenever the window is the whole visible set, which is every
    /// document shorter than three screens.
    window_start: usize,
    /// Height of the area this pane last rendered into, for sizing the window.
    ///
    /// `apply_view` runs outside `render` and has no `Rect` of its own, so the
    /// previous frame's height is the best available estimate. `area.height`
    /// rather than the inner text height: it over-estimates by the two border
    /// rows, and over-estimating a window is free while under-estimating it
    /// leaves the pane's bottom rows blank.
    ///
    /// `None` until the first render — see `ASSUMED_PANE_HEIGHT`.
    last_height: Option<u16>,
    /// Whether `textarea`'s viewport has been rendered since it was last
    /// replaced or reset. `TextArea::scroll` clamps the cursor into the
    /// viewport's *cached* dimensions, and a viewport `set_lines` has reset
    /// to zero rows collapses the cursor onto the scroll target — so
    /// anything that scrolls before a real render has primed those
    /// dimensions must prime them itself with a scratch render first. See
    /// `apply_pending_scroll`, which always does, and `keep_scroll_margin`,
    /// which only has to when this is false.
    viewport_primed: bool,
    /// Rows of text the pane last rendered — the area inside its borders.
    /// What the scroll keys measure the margin against (`scroll_view`);
    /// `render` has the real figure and this is the last one it saw. Zero
    /// before the first render, which makes the margin zero too.
    viewport_height: u16,
}

impl FileView<'_> {
    /// A view already showing `filename`. Test-only: `App` builds the pane
    /// with `default()` and then `load`s or `preview`s into it, and this is
    /// the one caller-free constructor the visibility sweep (#166) exposed
    /// to `dead_code` — the rest of #167 lives in `filter.rs` and
    /// `document.rs`.
    #[cfg(test)]
    pub(crate) fn new(filename: String) -> Self {
        let mut view = Self::default();
        view.load(Path::new(&filename));
        view
    }

    /// The file being shown.
    pub(crate) fn filename(&self) -> &Path {
        &self.filename
    }

    /// Whether the pane is showing a directory's listing rather than a file.
    pub(crate) fn showing_directory(&self) -> bool {
        self.showing_directory
    }

    /// Whether the pane is showing a file's own text — as opposed to a
    /// listing, or a message standing in for a file that could not be shown.
    pub(crate) fn is_text(&self) -> bool {
        self.text
    }

    /// The file's lines as read, for `App::sync_document`, which shares them
    /// with the `Document` rather than copying them.
    ///
    /// Not the textarea's lines: those are a *window* of the visible set once
    /// `show_window` has run. And an accessor rather than a `pub textarea`,
    /// because `window_start` must match what the textarea currently holds —
    /// `cursor_visible_row` is the only correct way to read a vertical
    /// position, and a caller mutating the textarea directly breaks it
    /// silently, off by `window_start`, which looks entirely plausible (#81).
    pub(crate) fn source(&self) -> &Arc<Vec<String>> {
        &self.source
    }

    /// Choose the colours syntax colouring paints with, or `Theme::Off`.
    ///
    /// Applies to the file already on screen as well as to later ones, so
    /// `App::new` can set it after the first `load`.
    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.rebuild_highlighter();
    }

    /// The name of the grammar colouring the file, if one is.
    #[cfg(test)]
    pub(crate) fn syntax_name(&self) -> Option<&str> {
        self.highlighter.as_ref().map(Highlighter::syntax_name)
    }

    /// Install freshly read lines: as `source`, as the textarea's buffer,
    /// and as the input to a new highlighter.
    ///
    /// Shared by `load` and `preview_with_caps`, which differ only in what
    /// they say about the rest of the file. The buffer starts at the file's
    /// first line, so `window_start` goes back to zero: leaving a previous
    /// file's offset here would misreport the cursor's line until the next
    /// `apply_view` — see `cursor_visible_row`.
    ///
    /// **A window of the lines, not a copy of all of them** (#151). This
    /// used to clone every line into the textarea, and every production
    /// caller — `load` and `preview` both run through `sync_document` and
    /// `apply_view` — replaced that clone through `show_window` before the
    /// first draw, because `sync_document` clears the record `apply_view`
    /// keys its rebuild on. So a full load was resident three times at its
    /// peak, and every navigator arrow copied the preview twice, for a
    /// buffer nothing ever rendered. The textarea now gets exactly the window
    /// `apply_view` builds before the first render — `window_for` with the
    /// cursor on the first line and the pre-render pane height — which is the
    /// whole file when the file is shorter than that and a bounded slice when
    /// it is not. Bounded, rather than the single blank row it could be: the
    /// pane still shows a file on its own, which is what keeps this widget
    /// testable without an `App` around it.
    fn adopt(&mut self, lines: Vec<String>, text: bool) {
        self.source = Arc::new(lines);
        self.text = text;
        let (start, end) = window_for(self.source.len(), self.window_height(), 0, 0);
        self.textarea = TextArea::new(self.source[start..end].to_vec());
        // The buffer numbers its own rows when nothing supplies numbers, and a
        // window's rows run short of the file's. Reserve the width the whole
        // file needs so the gutter does not widen when `apply_view` numbers
        // it properly — the same reasoning `preview_with_caps` applies to a
        // truncated read. A file the buffer holds whole reserves nothing.
        if end < self.source.len() {
            self.textarea
                .set_min_line_number_width(digits(self.source.len()));
        }
        self.viewport_primed = false;
        self.window_start = 0;
        self.rebuild_highlighter();
    }

    /// Start colouring `source` afresh, or stop, according to the theme and
    /// the file. Costs a grammar lookup: nothing is parsed until a render
    /// asks for a row.
    fn rebuild_highlighter(&mut self) {
        self.highlighter = self
            .text
            .then(|| Highlighter::for_file(self.theme, &self.filename, &self.source))
            .flatten();
    }

    /// Give or take focus. The only writer of `active` (#81).
    pub(crate) fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Accent the title for one frame, or stop. The one writer of
    /// `title_accent` (#120).
    pub(crate) fn set_title_accent(&mut self, on: bool) {
        self.title_accent = on;
    }

    /// Whether the pane holds a bounded preview rather than the whole file.
    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Roughly how many lines the whole file holds, while only a preview is
    /// loaded. `None` once the file is read in full.
    pub(crate) fn estimated_lines(&self) -> Option<usize> {
        self.estimated_lines
    }

    /// Show `path` in the pane, replacing whatever was there.
    ///
    /// A file that cannot be read is reported in the pane itself rather than
    /// bringing the TUI down, since any entry in the nav pane can be selected.
    /// Rebuilding the `TextArea` also resets the cursor and scroll position.
    pub(crate) fn load(&mut self, path: &Path) {
        self.filename = path.to_path_buf();
        let contents = read_lines(path);
        self.adopt(contents.lines, contents.text);
        self.showing_directory = path.is_dir();
        self.truncated = false;
        // The whole file is here, so its length is a fact rather than a guess.
        self.estimated_lines = None;
        // A pending restore was measured against the buffer this just threw
        // away; carrying it into an unrelated file would apply it to the
        // wrong data entirely — see `sync_document`'s clearing of
        // `last_generation` in `lib.rs` for the same reasoning.
        self.pending_screen_row = None;
    }

    /// Show just enough of `path` to fill the pane.
    ///
    /// Used as the selection moves, where reading whole files on every cursor
    /// key would stutter on large logs. While the nav pane holds focus the
    /// view cannot be scrolled, so a screenful is all that can be seen; the
    /// rest is read by `handle_events` as soon as the view is actually used.
    pub(crate) fn preview(&mut self, path: &Path) {
        self.preview_with_caps(path, PREVIEW_LINES, MAX_PREVIEW_BYTES);
    }

    /// `preview` with both caps injected, so a test can reach the truncated
    /// branch without building a multi-megabyte fixture. See
    /// `read_preview_with_caps` for why the seam is here.
    fn preview_with_caps(&mut self, path: &Path, max_lines: usize, max_bytes: u64) {
        self.filename = path.to_path_buf();
        let preview = read_preview_with_caps(path, max_lines, max_bytes);
        self.showing_directory = path.is_dir();
        self.adopt(preview.lines, preview.text);
        self.truncated = preview.truncated;
        self.estimated_lines = preview.estimated_lines;
        // Size the gutter for the whole file, not for the slice of it on
        // screen. Without this the gutter fits the preview's own line count
        // and then widens the moment the rest of the file arrives, shifting
        // every line of text sideways on a pane the user is already reading.
        //
        // Only ever a *minimum*: if the estimate reads low, the real numbering
        // still wins once loaded, so a bad guess costs the same single redraw
        // that making no guess at all would have.
        if let Some(estimate) = preview.estimated_lines {
            self.textarea.set_min_line_number_width(digits(estimate));
        }
        // See `load`: a pending restore does not survive a switch to a
        // different file's buffer.
        self.pending_screen_row = None;
    }

    /// Style individual lines, indexed by line number.
    ///
    /// Filtering uses this to dim lines that match no filter and colour those
    /// that do. Rebuilding the textarea — which `load` and `preview` both do —
    /// clears these, so they must be re-applied after either.
    ///
    /// The line the cursor is on keeps its style too: `render` folds it into
    /// the cursor-line style, because the textarea replaces rather than merges.
    pub(crate) fn set_line_styles(&mut self, styles: Vec<Option<Style>>) {
        self.textarea.set_line_styles(styles);
    }

    /// Show these 0-based source line numbers in the gutter instead of
    /// numbering the buffer 1..N.
    ///
    /// Used when the buffer holds only the lines matching a filter, so the
    /// gutter still reads as positions in the original file. Cleared by
    /// `load` and `preview`, as above.
    pub(crate) fn set_line_numbers(&mut self, numbers: Vec<usize>) {
        self.textarea.set_line_numbers(numbers);
    }

    /// Mark the last line of each group of consecutive source lines, one flag
    /// per buffer row.
    ///
    /// Hiding unmatched lines leaves groups of matches butted up against each
    /// other with nothing to say how much was skipped between them (issue #2).
    /// A set flag underlines that row's gutter number — deliberately the
    /// number and not the text, which already carries the filter colours, and
    /// deliberately a modifier rather than a whole separator row, which would
    /// spend a line of the pane on every gap.
    ///
    /// Cleared by `load` and `preview`, as with `set_line_styles` and
    /// `set_line_numbers` above.
    pub(crate) fn set_group_ends(&mut self, ends: Vec<bool>) {
        self.textarea.set_line_number_styles(
            ends.into_iter()
                .map(|end| end.then(|| Style::default().add_modifier(Modifier::UNDERLINED)))
                .collect(),
        );
    }

    /// Suppress the gutter entirely, for the placeholder row shown when
    /// nothing is visible. See the `gutter_blank` field for why an empty
    /// `set_line_numbers` override is not enough on its own.
    pub(crate) fn set_gutter_blank(&mut self, blank: bool) {
        self.gutter_blank = blank;
    }

    /// Set or clear the pattern whose spans the pane highlights black-on-yellow.
    ///
    /// The search *filter* owns the pattern; this only controls whether it is
    /// painted. Passing `None` clears the highlight — `set_search_pattern`
    /// treats an empty query as "no pattern", so nothing is compiled on that
    /// path. `load`/`preview` replace the textarea outright, dropping this
    /// too, same as `set_line_styles`/`set_line_numbers` above, so
    /// `App::apply_view` re-applies it on every pass rather than only when
    /// the pattern changes.
    pub(crate) fn set_highlight(&mut self, pattern: Option<&str>) -> Result<(), regex::Error> {
        self.textarea.set_search_pattern(pattern.unwrap_or(""))
    }

    /// The pattern currently highlighted, if any.
    ///
    /// Test-only. `App::apply_view` writes the highlight and never reads it
    /// back; the one caller is `App::file_view_highlight`, itself a test
    /// helper (#76). Answers the issue's open question for this method: it is
    /// a leftover, not reserved API.
    #[cfg(test)]
    pub(crate) fn highlight(&self) -> Option<String> {
        self.textarea
            .search_pattern()
            .map(|pattern| pattern.as_str().to_string())
    }

    /// Replace the buffer's contents and put the cursor on `row`, without
    /// touching the file.
    ///
    /// Used when filtering hides lines: the view then holds a subset of the
    /// document. The cursor row is applied here rather than by a later jump,
    /// because `CursorMove::Jump` takes a `u16` and would silently truncate
    /// past 65,535 lines. `set_lines` clamps in `usize` instead, so the
    /// cursor's *data position* is applied directly rather than jumped to
    /// afterwards. The rendered *viewport* is still `u16` internally, though —
    /// that ceiling is unchanged, so the view still cannot scroll past
    /// 65,535 lines; only landing the cursor on the right line survives past
    /// it. The filename is left alone, since it still describes where these
    /// lines came from.
    /// Test-only. Production goes through `show_window`, which carries the
    /// window offset this cannot express; this is that call with an offset of
    /// zero, kept because several tests want the unwindowed form to compare a
    /// windowed result against (#76).
    #[cfg(test)]
    pub(crate) fn show_lines_with_cursor(&mut self, lines: Vec<String>, row: usize) {
        self.show_window(lines, 0, row);
    }

    /// Replace the buffer with a **window** of the visible set: `lines` are
    /// visible rows `window_start..window_start + lines.len()`, and `row` is
    /// the cursor's row *within that window*.
    ///
    /// `show_lines_with_cursor` is this with a window of zero offset, which is
    /// what an unwindowed document (anything shorter than three screens) always
    /// produces.
    pub(crate) fn show_window(&mut self, lines: Vec<String>, window_start: usize, row: usize) {
        // set_lines rejects an empty vector; an empty buffer is one blank line.
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        self.window_start = window_start;
        self.textarea.set_lines(lines, (row, 0));
        self.viewport_primed = false;
    }

    /// Visible-set index of the buffer's first row.
    pub(crate) fn window_start(&self) -> usize {
        self.window_start
    }

    /// Visible-set index of the buffer's last row, exclusive.
    pub(crate) fn window_end(&self) -> usize {
        self.window_start + self.textarea.lines().len()
    }

    /// Where the cursor sits in the **visible set**, translating out of the
    /// window.
    ///
    /// The one place `textarea.cursor().0` is read for a vertical position.
    /// Everything else goes through here, because getting the translation wrong
    /// fails silently — it yields a line number off by `window_start`, which
    /// looks entirely plausible.
    pub(crate) fn cursor_visible_row(&self) -> usize {
        self.window_start + self.textarea.cursor().0
    }

    /// The pane height to size a window against: the area last rendered into,
    /// or a generous assumption before the first render.
    pub(crate) fn window_height(&self) -> u16 {
        self.last_height.unwrap_or(ASSUMED_PANE_HEIGHT)
    }

    /// Which row of the pane the cursor is currently drawn on.
    ///
    /// Used to hold a line in place across a rebuild: `set_lines` resets the
    /// viewport, so without this the cursor re-anchors to the pane's last row
    /// and the view lurches whenever a filter changes.
    pub(crate) fn cursor_screen_row(&self) -> u16 {
        let (top, _) = self.textarea.scroll_top();
        // The subtraction is a screen offset, so it fits `u16` for any pane a
        // terminal can actually draw. `try_from` rather than `as` because
        // nothing in the *type* says so — the cursor is a buffer index, and a
        // cursor left below the viewport by a bug would wrap to a small row
        // under `as` and silently scroll the view somewhere plausible.
        // Saturating turns that into a visibly pinned cursor instead.
        u16::try_from(self.textarea.cursor().0.saturating_sub(usize::from(top))).unwrap_or(u16::MAX)
    }

    /// Which row of the pane a jump to visible-set row `row` should land
    /// on, or `None` when the pane is already showing that row and the jump
    /// should not scroll at all.
    ///
    /// `center`, and the target goes in the middle of the pane: the pane
    /// has to redraw anyway, and the middle gives context on both sides.
    /// Otherwise the target scrolls in by the minimum and lands on the edge
    /// of the scroll margin nearest where it came from — the same row a
    /// held `j` or `k` would have delivered it to.
    pub(crate) fn jump_landing_row(&self, row: usize, center: bool) -> Option<u16> {
        let height = self.viewport_height;
        let top = self.window_start + usize::from(self.textarea.scroll_top().0);
        if (top..top + usize::from(height)).contains(&row) {
            return None;
        }
        let margin = u16::try_from(scroll_margin(usize::from(height))).unwrap_or(0);
        Some(if center {
            height / 2
        } else if row < top {
            margin
        } else {
            height.saturating_sub(1 + margin)
        })
    }

    /// The cursor column as a character index (for `word_under_cursor`).
    pub(crate) fn cursor_col(&self) -> usize {
        self.textarea.cursor().1
    }

    /// Request that the cursor be scrolled onto `row` of the pane the next
    /// time it renders, as far as the buffer allows near its start or end.
    ///
    /// Called right after a rebuild, before this frame has rendered the pane
    /// even once — so there is no real area to scroll against yet, only a
    /// guess at one. This used to prime a throwaway render against the
    /// *previous* frame's area and scroll immediately, which was a
    /// reasonable guess only as long as the pane's area never changed
    /// between renders. It stopped being reasonable once the filter pane
    /// joined the layout: adding or removing a filter can now change the
    /// file view's own width mid-rebuild, which made the guess wrong and
    /// reintroduced the exact re-anchoring this mechanism exists to
    /// prevent. Recording the request and applying it later, against the
    /// real area `render` is about to use — see `apply_pending_scroll` —
    /// removes the need to guess at all.
    ///
    /// `get_or_insert`: see the field doc on `pending_screen_row` for why a
    /// second request before the next render must not overwrite the first.
    pub(crate) fn scroll_cursor_to_row(&mut self, row: u16) {
        self.pending_screen_row.get_or_insert(row);
    }

    /// A *jump's* request for the cursor's row, which replaces any restore
    /// a rebuild queued earlier in the same frame rather than yielding to
    /// it (#192).
    ///
    /// `scroll_cursor_to_row` keeps the first request because two rebuilds
    /// in one frame are the same question asked twice, and the first answer
    /// is the one measured against a valid viewport. A jump is a different
    /// question: it has decided where the target lands, and a restore queued
    /// by the file load that preceded it in the same keypress — `,` loads
    /// the previous file and then lands on its last hit — would put the
    /// cursor back on the row the *previous* file's cursor was drawn on.
    pub(crate) fn land_cursor_on_row(&mut self, row: u16) {
        self.pending_screen_row = Some(row);
    }

    /// Apply a pending `scroll_cursor_to_row` request, if any, against
    /// `area` — the pane's real area for the frame about to render.
    ///
    /// `set_lines` reset the viewport to zeroed dimensions, which are
    /// normally only repopulated by an actual render — and `scroll`'s own
    /// bookkeeping (`CursorMove::InViewport`) clamps the cursor to that
    /// cached size, so scrolling against a zeroed one collapses the cursor
    /// onto the scroll target instead of leaving it on its line. Priming
    /// with a throwaway render at the real area first gives `scroll` the
    /// pane's actual height to clamp against, so the cursor survives the
    /// nudge intact.
    fn apply_pending_scroll(&mut self, area: Rect) {
        let Some(row) = self.pending_screen_row.take() else {
            return;
        };
        let mut scratch = Buffer::empty(area);
        (&self.textarea).render(area, &mut scratch);

        let cursor = self.textarea.cursor().0;
        let (current_top, _) = self.textarea.scroll_top();
        // Never past the end of the buffer on this request's account: a
        // jump centred near the last line would otherwise pull blank rows
        // in below it. The same waiver `keep_scroll_margin` gives — a view
        // already past the end is left there.
        let height = usize::from(
            self.textarea
                .block()
                .map_or(area, |block| block.inner(area))
                .height,
        );
        let lines = self.textarea.lines().len();
        let max_top = lines.saturating_sub(height).max(usize::from(current_top));
        let desired_top = cursor.saturating_sub(usize::from(row)).min(max_top);
        // A line index only exceeds `i64::MAX` in a file no filesystem can
        // hold, but `as` would make that case scroll *backwards* rather than
        // to the end, so saturate instead.
        let delta = i64::try_from(desired_top).unwrap_or(i64::MAX) - i64::from(current_top);
        if delta != 0 {
            // Clamped into `i16` range first, so the conversion cannot fail.
            // `unwrap_or(0)` keeps that a skipped scroll rather than a panic
            // in a render path if the clamp above is ever changed.
            let step =
                i16::try_from(delta.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or(0);
            self.textarea.scroll((step, 0));
        }
    }

    /// Scroll the viewport by `by` — `Ctrl-E`/`Ctrl-Y`, a half page, a
    /// page — and then move the *cursor* into the margin the scroll pushed
    /// over it, as vim does.
    ///
    /// The other half of `keep_scroll_margin`. That rule moves the view to
    /// suit the cursor, which is right after a cursor motion and exactly
    /// wrong after a scroll: `Ctrl-E` with the cursor on the pane's first
    /// row would scroll the view down a line, and the render would scroll
    /// it straight back up to restore the margin — a key that does nothing.
    /// So a scroll settles the cursor itself, and by the time the render
    /// rule looks the margin already holds.
    ///
    /// The same waivers as the render rule: no top margin when the buffer's
    /// first row is on screen, no bottom margin when its last row is.
    fn scroll_view(&mut self, by: impl Into<Scrolling>) {
        self.textarea.scroll(by);
        let height = usize::from(self.viewport_height);
        if height == 0 {
            // Nothing has rendered yet, so there is no margin to keep.
            return;
        }
        let margin = scroll_margin(height);
        let top = usize::from(self.textarea.scroll_top().0);
        let lines = self.textarea.lines().len();
        let lowest = if top == 0 { 0 } else { top + margin };
        let highest = if top + height >= lines {
            lines.saturating_sub(1)
        } else {
            top + height - 1 - margin
        };
        let (row, col) = self.textarea.cursor();
        let clamped = row.clamp(lowest, highest.max(lowest));
        if clamped != row {
            self.textarea.set_cursor_position((clamped, col));
        }
    }

    /// Scroll the view, if the cursor has strayed into the top or bottom
    /// `scroll_margin`, so that it sits on the margin's edge instead — vim's
    /// `scrolloff`.
    ///
    /// Run from `render`, after `apply_pending_scroll`, because that is the
    /// one place with the pane's real height and the one moment every path
    /// that moves the cursor — a key in `handle_events`, an `n` landed by
    /// `App`, a rebuild's restore — has finished with it. The textarea's own
    /// follow rule runs inside its render and only ever scrolls by the
    /// minimum that keeps the cursor on screen; this runs first and keeps it
    /// `margin` rows further in, so by the time the textarea looks, there is
    /// nothing left for it to do.
    ///
    /// A margin is waived where there is nothing beyond it to show: the top
    /// one at the buffer's first row, the bottom one once the buffer's last
    /// row is on screen. That is what lets the cursor reach the last line of
    /// a file without the view scrolling past the end into blank rows. Only
    /// scrolls this rule itself asks for are clamped that way — a view the
    /// user has already pushed past the end with `Ctrl-E` is left where they
    /// put it.
    ///
    /// Buffer rows throughout, not visible-set rows: the window's slack
    /// (`window_holds`) is a full screen beyond each edge of the viewport,
    /// and a margin is at most half a screen, so this never runs into the
    /// end of the buffer before `ensure_window` has moved it.
    fn keep_scroll_margin(&mut self, area: Rect, height: usize) {
        let margin = scroll_margin(height);
        let top = usize::from(self.textarea.scroll_top().0);
        let cursor = self.textarea.cursor().0;
        let lines = self.textarea.lines().len();

        let desired = if cursor < top + margin {
            cursor.saturating_sub(margin)
        } else if cursor + margin >= top + height {
            (cursor + margin + 1).saturating_sub(height)
        } else {
            top
        };
        // Never past the end on this rule's account — but no further back
        // than where the view already is, either.
        let desired = desired.min(lines.saturating_sub(height).max(top));
        if desired == top {
            return;
        }
        self.scroll_top_to(desired, area);
    }

    /// Put row `desired` of the buffer on the pane's first row, priming the
    /// viewport first if nothing has rendered it since it was reset — see
    /// `viewport_primed`.
    fn scroll_top_to(&mut self, desired: usize, area: Rect) {
        if !self.viewport_primed {
            let mut scratch = Buffer::empty(area);
            (&self.textarea).render(area, &mut scratch);
            self.viewport_primed = true;
        }
        let (current_top, _) = self.textarea.scroll_top();
        let delta = i64::try_from(desired).unwrap_or(i64::MAX) - i64::from(current_top);
        if delta != 0 {
            let step =
                i16::try_from(delta.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or(0);
            self.textarea.scroll((step, 0));
        }
    }

    /// Put the cursor on `row` of the current buffer, clamped to it.
    ///
    /// Used by `n`/`N`, which decide *which* line to land on in `App` — the
    /// only place that can see both the verdicts and the cursor — and then
    /// ask the view to go there.
    pub(crate) fn set_cursor_row(&mut self, row: usize) {
        self.textarea.set_cursor_position((row, 0));
    }

    /// Not a `Result`: every arm is a cursor move or a local toggle, and none
    /// of them can fail (#80). The one genuinely fallible thing this pane does
    /// — `set_highlight` — is called from `App::apply_view`, not from here.
    pub(crate) fn handle_events(&mut self, input: Input) {
        // The user is interacting with the view, so a preview is no longer
        // enough: they can now scroll past the end of it.
        if self.truncated {
            let path = self.filename.clone();
            self.load(&path);
        }

        match input {
            Input {
                key: Key::Char('h'),
                ..
            }
            | Input { key: Key::Left, .. } => self.textarea.move_cursor(CursorMove::Back),
            Input {
                key: Key::Char('j'),
                ..
            }
            | Input { key: Key::Down, .. } => self.textarea.move_cursor(CursorMove::Down),
            Input {
                key: Key::Char('k'),
                ..
            }
            | Input { key: Key::Up, .. } => self.textarea.move_cursor(CursorMove::Up),
            Input {
                key: Key::Char('l'),
                ..
            }
            | Input {
                key: Key::Right, ..
            } => self.textarea.move_cursor(CursorMove::Forward),
            Input {
                key: Key::Char('w'),
                ..
            } => self.textarea.move_cursor(CursorMove::WordForward),
            Input {
                key: Key::Char('^'),
                ..
            }
            | Input {
                key: Key::Char('0'),
                ..
            } => self.textarea.move_cursor(CursorMove::Head),
            Input {
                key: Key::Char('#'),
                ..
            } => self.hide_line_numbers = !self.hide_line_numbers,
            Input {
                key: Key::Char('}'),
                ..
            } => self.textarea.move_cursor(CursorMove::ParagraphForward),
            Input {
                key: Key::Char('{'),
                ..
            } => self.textarea.move_cursor(CursorMove::ParagraphBack),
            Input {
                key: Key::Char('$'),
                ..
            } => self.textarea.move_cursor(CursorMove::End),
            Input {
                key: Key::Char('g'),
                ctrl: false,
                ..
            }
            | Input { key: Key::Home, .. } => self.textarea.move_cursor(CursorMove::Top),
            Input {
                key: Key::Char('G'),
                ctrl: false,
                ..
            }
            | Input { key: Key::End, .. } => self.textarea.move_cursor(CursorMove::Bottom),
            Input {
                key: Key::Char('e'),
                ctrl: true,
                ..
            } => self.scroll_view((1, 0)),
            Input {
                key: Key::Char('y'),
                ctrl: true,
                ..
            } => self.scroll_view((-1, 0)),
            Input {
                key: Key::Char('d'),
                ctrl: true,
                ..
            } => self.scroll_view(Scrolling::HalfPageDown),
            Input {
                key: Key::Char('u'),
                ctrl: true,
                ..
            } => self.scroll_view(Scrolling::HalfPageUp),
            Input {
                key: Key::Char('b'),
                ctrl: true,
                ..
            }
            | Input {
                key: Key::PageUp, ..
            } => self.scroll_view(Scrolling::PageUp),
            // Paired deliberately, and sitting next to the `{`/`}` paragraph
            // motions above: brackets move by page, braces by paragraph, both
            // left-is-back and right-is-forward. Neither needs Shift, which is
            // what `Ctrl-b` costs, and both exist on keyboards with no
            // `PageUp`/`PageDown`.
            //
            // These replaced `space` and `Enter` in #48. `space` became the
            // global peek and `Enter` the filter pane's toggle, so a file view
            // that still paged on either would give one key two meanings — the
            // thing #48 set out to remove.
            Input {
                key: Key::Char('['),
                ..
            } => self.scroll_view(Scrolling::PageUp),
            Input {
                key: Key::Char(']'),
                ..
            } => self.scroll_view(Scrolling::PageDown),
            Input {
                key: Key::Char('f'),
                ctrl: true,
                ..
            }
            | Input {
                key: Key::PageDown, ..
            } => self.scroll_view(Scrolling::PageDown),
            _ => (),
        }
    }
}

/// Read `path` whole, or a single-line message describing why it could not
/// be read. The reading itself is `document::read_lines` (#143); this wraps
/// its error the way the pane shows it.
///
/// Never `truncated`, and never estimating: the whole file is here.
fn read_lines(path: &Path) -> Contents {
    // See `read_preview`: a directory opens fine and then fails to read, so
    // it is recognised up front rather than surfacing an OS error string.
    if path.is_dir() {
        return directory_listing(path, usize::MAX);
    }
    match document::read_lines(path) {
        Ok(lines) => Contents {
            lines,
            truncated: false,
            estimated_lines: None,
            text: true,
        },
        Err(err) if document::is_binary(&err) => Contents::message(BINARY_MESSAGE.to_string()),
        // Logged as well as shown (#83). The pane gets `<{err}>` in place of
        // the file, which tells the user *that* it failed; the log is where
        // the full path lives, and the pane's title is elided when the pane
        // is narrow.
        Err(err) => {
            log::warn!("cannot read {}: {err}", path.display());
            Contents::message(format!("<{err}>"))
        }
    }
}

/// Read at most a screenful of `path`, reporting whether anything was left.
///
/// Bounded on both axes: `PREVIEW_LINES` lines and `MAX_PREVIEW_BYTES` bytes.
/// Because the reader stops as soon as either is reached, the cost does not
/// grow with the size of the file and there is no long-running work to cancel
/// when the selection moves on.
///
/// A file that cannot be read reports the reason and is *not* marked truncated
/// — there is nothing better to re-read later.
/// Both caps are injected rather than read from the constants.
///
/// `FileView::preview` is the only caller outside tests and always passes
/// `PREVIEW_LINES` and `MAX_PREVIEW_BYTES`. The parameters exist because those
/// constants are now 50,000 lines and 10 MiB: exercising either cap against
/// them means building a multi-megabyte fixture per test, and the byte cap
/// alone cost more than the entire rest of the suite. With the caps injectable
/// a handful of bytes is enough, and a test states the cap it is testing
/// instead of deriving it from a constant it does not control.
fn read_preview_with_caps(path: &Path, max_lines: usize, max_bytes: u64) -> Contents {
    // Checked before opening, not after failing to read. `File::open` on a
    // directory *succeeds* on macOS and the read then fails `EISDIR`, so
    // falling through to the error path below would display
    // `<Is a directory (os error 21)>` — platform-specific and meaningless
    // to a reader.
    if path.is_dir() {
        return directory_listing(path, max_lines);
    }
    // The same stat guards a FIFO (#221): its open would block until a
    // writer appeared, and this runs on every selection move.
    let file = match document::refuse_unreadable(path).and_then(|()| File::open(path)) {
        Ok(file) => file,
        Err(err) => return Contents::message(format!("<{err}>")),
    };
    // Read before the bytes are consumed; a file that cannot be stat'd simply
    // gets no estimate rather than failing the preview.
    let file_bytes = file.metadata().ok().map(|meta| meta.len());

    let mut reader = BufReader::new(file.take(max_bytes));
    let head = match sniff(&mut reader) {
        Ok((Sniff::Text, head)) => head,
        Ok((Sniff::Binary, _)) => return Contents::message(BINARY_MESSAGE.to_string()),
        // UTF-16 is decoded whole, so the byte cap is the only cap the read
        // itself knows; the line cap is applied to the result below.
        Ok((Sniff::Utf16(endian), head)) => {
            let mut lines = match read_utf16_lines(head, &mut reader, endian) {
                Ok(lines) => lines,
                Err(err) => return Contents::message(format!("<{err}>")),
            };
            let over_the_line_cap = lines.len() > max_lines;
            lines.truncate(max_lines);
            let remaining = reader.into_inner().limit();
            return capped(
                lines,
                over_the_line_cap || remaining == 0,
                file_bytes,
                max_bytes - remaining,
            );
        }
        Err(err) => return Contents::message(format!("<{err}>")),
    };

    // The sniffed bytes are content, so they go back in front of the rest.
    let mut reader = Cursor::new(head).chain(reader);
    let mut lines = Vec::new();
    let mut buf = Vec::new();

    while lines.len() < max_lines {
        match read_lossy_line(&mut reader, &mut buf) {
            Ok(Some(line)) => lines.push(line),
            Ok(None) => break,
            Err(err) => return Contents::message(format!("<{err}>")),
        }
    }

    // Either the line budget ran out, or the byte allowance did. A file that
    // ends exactly on a cap is reported as truncated, which only costs a
    // redundant re-read the first time the view is used.
    let remaining = reader.into_inner().1.into_inner().limit();
    let truncated = lines.len() == max_lines || remaining == 0;
    capped(lines, truncated, file_bytes, max_bytes - remaining)
}

/// A preview's `Contents`, with a line estimate only when it was cut short —
/// there is nothing to estimate about a file that was read whole.
fn capped(
    lines: Vec<String>,
    truncated: bool,
    file_bytes: Option<u64>,
    bytes_read: u64,
) -> Contents {
    let estimated_lines = if truncated {
        estimate_lines(file_bytes, bytes_read, lines.len())
    } else {
        None
    };
    Contents {
        lines,
        truncated,
        estimated_lines,
        text: true,
    }
}

/// A directory rendered as its contents, bounded by `max_lines`.
///
/// The view is the widest pane on screen and was spending all of it on the
/// word `<directory>`. Listing what is actually there turns a selected
/// directory into a look-ahead — and `l` on that selection makes the listing
/// the navigator's own, which is what stops it being a navigable-looking list
/// that cannot be navigated.
///
/// `..` is absent deliberately: it is the navigator's way back out, and there
/// is nothing here that could act on it.
/// Widest a name column grows before long names are allowed to push the
/// metadata out of line on their own row.
///
/// Same trade `MAX_NAV_WIDTH` makes: one pathological 200-character filename
/// would otherwise pad *every* row out to 200 columns and push the size and
/// time off screen for all of them. Capping means that one row misaligns
/// instead of all of them going blank.
const NAME_COLUMN_MAX: usize = 40;

/// Columns a size occupies, right-aligned. `1023B` and `999.9K` both fit.
const SIZE_COLUMN: usize = 6;

/// `bytes` as something readable at a glance, in the width `SIZE_COLUMN` gives.
///
/// Binary units, since this reports what the filesystem reports. One decimal
/// above 1 KiB: `18.4K` says as much as `18841` in fewer columns, and the
/// exact byte count is not what anyone scans a listing for.
fn format_size(bytes: u64) -> String {
    const UNITS: [(u64, char); 4] = [(1 << 30, 'G'), (1 << 20, 'M'), (1 << 10, 'K'), (1, 'B')];
    for (scale, suffix) in UNITS {
        if bytes >= scale {
            return if scale == 1 {
                format!("{bytes}B")
            } else {
                // Truncating rather than rounding, so a listing never claims a
                // file reached the next unit before it did.
                let whole = bytes / scale;
                let tenth = (bytes % scale) * 10 / scale;
                format!("{whole}.{tenth}{suffix}")
            };
        }
    }
    "0B".to_string()
}

/// `time` as a local calendar datetime, or `None` if it cannot be represented.
///
/// Local, not UTC: the logs recon reads carry local timestamps, and a listing
/// disagreeing with the lines inside the files would be its own small bug.
/// `jiff` resolves the zone from the system (`/etc/localtime` on Unix), which
/// is the part that is genuinely hard to get right by hand.
fn format_modified(time: std::time::SystemTime) -> Option<String> {
    let zoned = jiff::Zoned::try_from(time).ok()?;
    Some(zoned.strftime("%Y-%m-%d %H:%M").to_string())
}

/// One row: name, then size, then when it changed.
///
/// The metadata sits to the *right* of the name deliberately. The view pane
/// narrows when the navigator is wide, and a row clipped at the pane's edge
/// then loses the time first, the size next, and the name last — which is the
/// priority order this wants, achieved by layout rather than by logic that
/// would need a width the listing does not have when it is built.
fn listing_row(entry: &Entry, name_width: usize) -> String {
    let size = entry.size.map_or_else(|| "-".to_string(), format_size);
    let modified = entry.modified.and_then(format_modified).unwrap_or_default();
    // Padded by hand rather than with `{:<name_width$}`, which counts `char`s.
    // `name_width` is terminal columns, so a CJK name would otherwise be padded
    // as though its ideographs were one column each and push its own size
    // column one place right per glyph (#97).
    let name = entry.display();
    let pad = " ".repeat(name_width.saturating_sub(UnicodeWidthStr::width(name.as_str())));
    format!("{name}{pad}  {size:>SIZE_COLUMN$}  {modified}")
        .trim_end()
        .to_string()
}

fn directory_listing(path: &Path, max_lines: usize) -> Contents {
    let entries = match crate::widgets::filenav::sorted_entries(path) {
        Ok(entries) => entries,
        // Same shape as an unreadable file: say why, verbatim from the OS.
        Err(err) => return Contents::message(format!("<{err}>")),
    };
    if entries.is_empty() {
        return Contents::message(EMPTY_DIRECTORY_MESSAGE.to_string());
    }
    let total = entries.len();
    let shown = &entries[..entries.len().min(max_lines)];
    // Padded to the longest name actually on screen, so the columns line up
    // without every listing being as wide as the widest possible name.
    let name_width = shown
        .iter()
        .map(|entry| UnicodeWidthStr::width(entry.display().as_str()))
        .max()
        .unwrap_or(0)
        .min(NAME_COLUMN_MAX);
    let lines: Vec<String> = shown
        .iter()
        .map(|entry| listing_row(entry, name_width))
        .collect();
    let truncated = total > lines.len();
    Contents {
        lines,
        truncated,
        // Unlike a file, the real count is known exactly rather than scaled
        // from a sample — there is no guessing to do.
        estimated_lines: truncated.then_some(total),
        text: false,
    }
}

/// How many lines a file of `file_bytes` probably holds, given that its first
/// `bytes_read` bytes held `lines_read` lines.
///
/// Scaling the sample's bytes-per-line up to the whole file is a guess, but a
/// cheap one — the alternative is reading the file to count its newlines,
/// which is exactly the unbounded work `read_preview` exists to avoid. It only
/// has to be right to the *digit*, since all it feeds is a gutter width, so
/// being off by 10% costs nothing and being off by 10x costs one redraw: the
/// same redraw as having made no estimate at all.
///
/// `None` when there is nothing to scale from — no readable size, an empty
/// read, or a preview that consumed no bytes.
fn estimate_lines(file_bytes: Option<u64>, bytes_read: u64, lines_read: usize) -> Option<usize> {
    let file_bytes = file_bytes?;
    if bytes_read == 0 || lines_read == 0 {
        return None;
    }
    // u128: `file_bytes` is a real file size and `lines_read` is at most
    // PREVIEW_LINES, but their product still overflows u64 on a file above
    // ~37 PiB. Cheap to rule out rather than reason about.
    let estimate = (u128::from(file_bytes) * lines_read as u128).div_ceil(u128::from(bytes_read));
    Some(usize::try_from(estimate).unwrap_or(usize::MAX))
}

/// Decimal digits in `n`, for sizing the gutter. `0` and `1` both need one.
fn digits(n: usize) -> u8 {
    if n == 0 { 1 } else { n.ilog10() as u8 + 1 }
}

/// Widget impl for `FileView`
/// Test-only reach into the textarea, for assertions about styles, scroll
/// position and line numbers — none of which `App` has any business reading.
///
/// Its own `impl` block because `'a` is named nowhere else: on the main block
/// it would be an elidable lifetime in every non-test build.
#[cfg(test)]
impl<'a> FileView<'a> {
    pub(crate) fn textarea(&self) -> &TextArea<'a> {
        &self.textarea
    }

    pub(crate) fn textarea_mut(&mut self) -> &mut TextArea<'a> {
        &mut self.textarea
    }
}

/// Priority of syntax spans among the textarea's custom highlights.
///
/// The fork's priority only orders highlights that start at the *same* byte,
/// so this is not what keeps search matches on top — `apply_syntax` cuts
/// the matches out of the spans for that. Lowest anyway, so that anything
/// else ever pushed at the same offset wins.
const SYNTAX_PRIORITY: u8 = 1;

/// Most source lines one frame may parse for colour.
///
/// About 50 ms in release. A window of filter hits thousands of lines apart
/// on a large file can otherwise ask for a resync per row — see
/// `Highlighter::ensure` — and this is what bounds that to one brief stall,
/// with the rows past the budget coloured on the next frame instead of
/// never rendered at all.
const SYNTAX_BUDGET: usize = 4096;

impl FileView<'_> {
    /// Paint this frame's syntax colours onto the buffer.
    ///
    /// Re-done every render rather than kept in sync: `set_lines` clears the
    /// textarea's custom highlights on every window rebuild, and which rows
    /// want colour depends on the cursor, the filters and the search, all of
    /// which change between frames. A few thousand pushes a frame, and
    /// nothing to keep in step — the same shape `showing_directory` uses.
    ///
    /// Three kinds of row are left alone, so what was on screen before #122
    /// is on screen unchanged:
    ///
    /// * **The cursor line while this pane is active.** It is drawn reversed,
    ///   and a custom highlight *replaces* the line's style within its range
    ///   rather than layering on it, so coloured words would punch holes in
    ///   the bar.
    /// * **A line a filter has styled** — coloured as a match or dimmed as a
    ///   miss. The filter colour is the information; syntax colour would
    ///   overwrite it, and un-dim a line that was dimmed on purpose.
    /// * **Search matches.** The fork's priority only orders highlights that
    ///   start at the same byte, so a syntax span *beginning inside* a match
    ///   would replace the black-on-yellow. The match ranges are cut out of
    ///   the spans before they are pushed, and the search style is painted
    ///   onto a plain background as before.
    fn apply_syntax(&mut self) {
        self.textarea.clear_custom_highlight();
        let Some(highlighter) = self.highlighter.as_mut() else {
            return;
        };
        let rows = self.textarea.lines().len();
        let cursor_row = self.active.then(|| self.textarea.cursor().0);
        // Buffer row → source line: the gutter override when there is one
        // (always, in production — see `App::apply_view`), otherwise the
        // window's offset, which is what a freshly loaded, unwindowed buffer
        // has.
        let numbers = self.textarea.line_numbers();
        let styles = self.textarea.line_styles();
        let pattern = self.textarea.search_pattern();
        let mut budget = SYNTAX_BUDGET;
        let mut pushes: Vec<(usize, Span)> = Vec::new();
        for (row, line) in self.textarea.lines().iter().enumerate() {
            if Some(row) == cursor_row || styles.get(row).copied().flatten().is_some() {
                continue;
            }
            let source = numbers.get(row).copied().unwrap_or(self.window_start + row);
            if !highlighter.ensure(&self.source, source, &mut budget) {
                continue;
            }
            let matches: Vec<(usize, usize)> = pattern
                .map(|pattern| {
                    pattern
                        .find_iter(line)
                        .map(|found| (found.start(), found.end()))
                        .filter(|(start, end)| start < end)
                        .collect()
                })
                .unwrap_or_default();
            for &span in highlighter.spans(source) {
                pushes.extend(around(span, &matches).map(|piece| (row, piece)));
            }
        }
        debug_assert!(pushes.iter().all(|(row, _)| *row < rows));
        for (row, span) in pushes {
            self.textarea.custom_highlight(
                ((row, span.start), (row, span.end)),
                span.style,
                SYNTAX_PRIORITY,
            );
        }
    }
}

/// `span` with `holes` cut out of it: the pieces that lie outside every hole.
///
/// `holes` are ascending and disjoint, which is what `Regex::find_iter`
/// yields. A span entirely inside a hole yields nothing.
fn around(span: Span, holes: &[(usize, usize)]) -> impl Iterator<Item = Span> + '_ {
    let mut start = span.start;
    let mut pieces = Vec::new();
    for &(hole_start, hole_end) in holes {
        if hole_end <= start {
            continue;
        }
        if hole_start >= span.end {
            break;
        }
        if hole_start > start {
            pieces.push(Span {
                start,
                end: hole_start,
                style: span.style,
            });
        }
        start = start.max(hole_end);
    }
    if start < span.end {
        pieces.push(Span {
            start,
            end: span.end,
            style: span.style,
        });
    }
    pieces.into_iter()
}

impl Widget for &mut FileView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Recorded for the *next* `apply_view`, which runs outside render and
        // has no area of its own to size a window against. See `last_height`.
        self.last_height = Some(area.height);
        if self.hide_line_numbers || self.gutter_blank || self.showing_directory {
            self.textarea.remove_line_number();
        } else {
            self.textarea
                .set_line_number_style(Style::default().fg(Color::DarkGray));
        }
        // The textarea replaces rather than merges a line's style, so the
        // cursor line would otherwise discard whatever the filters gave it and
        // read as unfiltered. Start from that line's own style and add the
        // focus decoration on top: REVERSED always, but the green foreground
        // only when the line has no colour of its own — otherwise a matched
        // line under the cursor would be indistinguishable from a dimmed one.
        let cursor_row = self.textarea.cursor().0;
        let own_style = self
            .textarea
            .line_styles()
            .get(cursor_row)
            .copied()
            .flatten();
        let mut style = own_style.unwrap_or_default();
        if self.active {
            if own_style.is_none() {
                style = style.fg(Color::Green);
            }
            style = style.add_modifier(Modifier::REVERSED);
        }
        self.textarea.set_cursor_line_style(style);
        self.textarea
            .set_search_style(Style::default().fg(Color::Black).bg(Color::Yellow));
        self.apply_syntax();
        // The one place the path is rendered, and the one place a lossy
        // conversion is both correct and harmless — see the `filename` field.
        let title = self.filename.display().to_string();
        let title = if self.title_accent {
            ratatui::text::Line::from(ratatui::text::Span::styled(
                title,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            ratatui::text::Line::from(title)
        };
        self.textarea
            .set_block(crate::widgets::pane_block(title, self.active));
        // Apply any scroll requested since the last render — see
        // `scroll_cursor_to_row` and `apply_pending_scroll` — only now, once
        // the block above is set: `apply_pending_scroll`'s scratch render
        // needs to see the same borders the real render below is about to
        // draw, or it computes an inner height that is two rows too tall on
        // the first frame after `load`/`preview` replace the textarea (which
        // drops its block along with everything else).
        self.apply_pending_scroll(area);
        // The block is set, so its inner area is the rows the text gets.
        let inner_height = usize::from(
            self.textarea
                .block()
                .map_or(area, |block| block.inner(area))
                .height,
        );
        self.keep_scroll_margin(area, inner_height);
        (&self.textarea).render(area, buf);
        self.viewport_primed = true;
        self.viewport_height = u16::try_from(inner_height).unwrap_or(u16::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{fixture_dir, fixture_file};
    use std::fmt::Write as _;
    use std::fs;

    fn contents(view: &FileView<'_>) -> String {
        view.textarea.lines().join("\n")
    }

    /// `n` newline-terminated lines, `line 0` through `line n-1`.
    ///
    /// One buffer appended to, not `(0..n).map(|i| format!(...)).collect()`:
    /// the latter allocates and drops a `String` per line, which is quadratic
    /// and showed up across ~15 fixtures in this file and `lib.rs` (#90).
    /// `write!` into a `String` cannot fail, hence the discarded result.
    fn numbered_lines(n: usize) -> String {
        (0..n).fold(String::new(), |mut body, i| {
            let _ = writeln!(body, "line {i}");
            body
        })
    }

    // ---- window arithmetic (#7) ----------------------------------------

    /// The degenerate case, and the one almost every other test in this repo
    /// runs in: a document shorter than the window is windowed to itself, so
    /// nothing about the buffer or the cursor changes.
    #[test]
    fn a_short_document_is_its_own_window() {
        assert_eq!(window_for(50, 24, 0, 0), (0, 50));
        assert_eq!(window_for(50, 24, 49, 23), (0, 50));
        // Exactly a full span still fits whole.
        assert_eq!(window_for(120, 24, 40, 0), (0, 120));
    }

    #[test]
    fn a_long_document_gets_five_screens_around_the_viewport() {
        // Cursor on the pane's top row, so the viewport's top edge is the
        // cursor's own row and the slack below it is the whole difference.
        let (start, end) = window_for(10_000, 24, 5_000, 0);

        assert_eq!(end - start, 120, "five screens of 24");
        assert_eq!(
            start,
            5_000 - 48,
            "two screens of slack above the viewport's top edge"
        );
    }

    /// The #108 case. The cursor is on the pane's *bottom* row, where `[`
    /// leaves it, so the viewport's top edge is a full pane above the cursor
    /// and the window has to start from there — not from the cursor, which
    /// would leave only `height - screen_row` rows above the viewport and clamp
    /// the next page up to that.
    #[test]
    fn the_window_is_measured_from_the_viewport_not_the_cursor() {
        let height = 24usize;
        let row = 5_000usize;
        let screen_row = height - 1;
        let (start, end) = window_for(10_000, height as u16, row, screen_row);

        let viewport_top = row - screen_row;
        assert_eq!(
            viewport_top - start,
            height * SLACK_SCREENS,
            "slack above the viewport's top edge"
        );
        let viewport_bottom = viewport_top + height - 1;
        assert_eq!(
            end - 1 - viewport_bottom,
            height * SLACK_SCREENS,
            "slack below the viewport's bottom edge"
        );
    }

    #[test]
    fn the_window_does_not_run_off_the_start() {
        assert_eq!(window_for(10_000, 24, 3, 0), (0, 120));
    }

    /// Pulled back rather than truncated, so the buffer stays a full span and
    /// the last screen of the file is not rendered against a stub.
    #[test]
    fn the_window_is_pulled_back_at_the_end() {
        let (start, end) = window_for(10_000, 24, 9_999, 23);

        assert_eq!(end, 10_000);
        assert_eq!(end - start, 120);
    }

    /// A zero-height pane is possible mid-resize; `window_for` must not divide
    /// or multiply its way into a panic or an empty buffer.
    #[test]
    fn a_zero_height_pane_still_yields_a_window() {
        let (start, end) = window_for(10_000, 0, 500, 0);

        assert!(end > start, "a zero-height pane must still hold something");
    }

    // ---- the slack rule --------------------------------------------------

    /// Freshly built, a window holds: `window_for` lays down two screens of
    /// slack and `window_holds` asks for one.
    #[test]
    fn a_freshly_built_window_holds() {
        for screen_row in [0usize, 12, 23] {
            let (start, end) = window_for(10_000, 24, 5_000, screen_row);
            assert!(
                window_holds(10_000, start, end, 5_000, screen_row),
                "screen_row {screen_row} owed a rebuild immediately"
            );
        }
    }

    /// A window is owed a rebuild once the viewport has eaten into the slack
    /// far enough that the *next* page would not fit. Window 0..120 was built
    /// for height 24, so the viewport's bottom edge may reach row 95.
    #[test]
    fn running_the_slack_down_below_a_page_needs_a_rebuild() {
        // Cursor on the pane's top row: viewport is [row, row + 23].
        assert!(window_holds(10_000, 0, 120, 71, 0), "71 + 23 + 24 < 120");
        assert!(!window_holds(10_000, 0, 120, 73, 0), "73 + 23 + 24 >= 120");
    }

    #[test]
    fn running_the_slack_down_upwards_needs_a_rebuild() {
        // Window 100..220, height 24. The viewport's top edge must stay at or
        // below 124 to keep a page above it.
        assert!(window_holds(10_000, 100, 220, 124, 0));
        assert!(!window_holds(10_000, 100, 220, 123, 0));
    }

    /// The #108 regression at the `window_holds` end: the same cursor row holds
    /// or does not depending on which row of the pane it is drawn on, because
    /// that is what decides where the viewport's edges are. Under the old
    /// cursor-only rule both of these answered the same way, and the `[` case
    /// answered "holds" while only three rows sat above the viewport.
    #[test]
    fn the_screen_row_decides_where_the_viewport_edges_are() {
        // Cursor at visible row 146, window 100..220, height 24.
        // On the pane's top row the viewport is [146, 169], leaving 46 rows
        // above it — comfortably more than the page it must be able to move.
        assert!(window_holds(10_000, 100, 220, 146, 0));
        // On the pane's bottom row the same cursor puts the viewport at
        // [123, 146], leaving only 23 rows above it: a page up would clamp.
        assert!(!window_holds(10_000, 100, 220, 146, 23));
    }

    /// There is nothing above row 0 to window onto, so the top of the first
    /// window is legitimately reachable without a rebuild. Without this,
    /// sitting on line 1 would rebuild on every keystroke forever.
    #[test]
    fn the_document_start_is_not_a_margin() {
        assert!(window_holds(10_000, 0, 120, 0, 0));
    }

    #[test]
    fn the_document_end_is_not_a_margin() {
        assert!(window_holds(10_000, 9_880, 10_000, 9_999, 23));
    }

    /// An unwindowed document is both ends at once, so it can never ask for a
    /// rebuild — which is what keeps small files on exactly today's code path.
    #[test]
    fn an_unwindowed_document_never_needs_a_rebuild() {
        for row in 0..50 {
            assert!(
                window_holds(50, 0, 50, row, row.min(23)),
                "row {row} forced a rebuild"
            );
        }
    }

    /// The guarantee the rule exists for, driven the way the app drives it: a
    /// page moves the *viewport*, and every page must complete inside the
    /// buffer rather than being clamped at its edge.
    ///
    /// Both directions, and with the cursor on the pane row each key actually
    /// leaves it on — `]` on the top row, `[` on the bottom. Pinning
    /// `screen_row` to 0 would pass against the pre-#108 code.
    #[test]
    fn a_page_in_either_direction_always_fits_in_the_buffer() {
        let height = 24usize;
        let visible = 10_000usize;

        for (name, down, screen_row) in [("]", true, 0usize), ("[", false, height - 1)] {
            let mut viewport_top = 5_000usize;
            let (mut start, mut end) = window_for(visible, height as u16, viewport_top, 0);

            for page in 0..20 {
                if down {
                    assert!(
                        viewport_top + height - 1 + height < end,
                        "`{name}` page {page}: only {} rows below the viewport, needs {height}",
                        end - (viewport_top + height),
                    );
                    viewport_top += height;
                } else {
                    assert!(
                        viewport_top >= start + height,
                        "`{name}` page {page}: only {} rows above the viewport, needs {height}",
                        viewport_top - start,
                    );
                    viewport_top -= height;
                }
                let row = viewport_top + screen_row;
                if !window_holds(visible, start, end, row, screen_row) {
                    (start, end) = window_for(visible, height as u16, row, screen_row);
                }
            }
        }
    }

    /// Write a fixture under `target/` so the tests do not depend on whatever
    /// happens to be in the working tree. Claimed through the shared
    /// registry (#164), so a name another module's directory fixture uses is
    /// refused here too.
    fn fixture(name: &str, contents: &str) -> std::path::PathBuf {
        fixture_file(name, contents.as_bytes())
    }

    /// Write a fixture of raw bytes, for content that is not valid UTF-8.
    fn byte_fixture(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        fixture_file(name, bytes)
    }

    /// A view over nothing, for the tests whose first act is to `load` or
    /// `preview` something else. These used to open the real `Cargo.toml`
    /// for a buffer they replaced on the next line, which read the working
    /// tree in twenty-odd tests that had nothing to say about it (#163). A
    /// path that does not exist reads nothing and needs no fixture name.
    fn placeholder_view() -> FileView<'static> {
        FileView::new("target/test-fixtures/placeholder-never-written".to_string())
    }

    fn long_file(name: &str, lines: usize) -> std::path::PathBuf {
        let body = numbered_lines(lines);
        fixture(name, &body)
    }

    /// A view over known text, so cursor assertions are exact.
    fn view_of(name: &str, body: &str) -> FileView<'static> {
        let path = fixture(name, body);
        FileView::new(path.display().to_string())
    }

    fn send(view: &mut FileView<'_>, key: Key) {
        view.handle_events(Input {
            key,
            ..Default::default()
        });
    }

    fn rendered(view: &mut FileView<'_>) -> String {
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn line_numbers_show_by_default() {
        let mut view = view_of("numbers_on.txt", "alpha\nbeta\n");

        assert!(
            rendered(&mut view).contains("1 alpha"),
            "no line number gutter:\n{}",
            rendered(&mut view)
        );
    }

    #[test]
    fn hash_toggles_line_numbers_off_and_back_on() {
        let mut view = view_of("numbers_toggle.txt", "alpha\nbeta\n");

        send(&mut view, Key::Char('#'));
        let without = rendered(&mut view);
        assert!(
            without.contains("alpha") && !without.contains("1 alpha"),
            "gutter still present:\n{without}"
        );

        send(&mut view, Key::Char('#'));

        assert!(
            rendered(&mut view).contains("1 alpha"),
            "gutter did not return"
        );
    }

    #[test]
    fn zero_moves_to_the_start_of_the_line() {
        let mut view = view_of("motions_zero.txt", "hello world\n");
        send(&mut view, Key::Char('$'));
        assert_ne!(view.textarea.cursor().1, 0);

        send(&mut view, Key::Char('0'));

        assert_eq!(view.textarea.cursor(), (0, 0));
    }

    /// `w` is the only word motion left in the view — `b` and `e` were
    /// reassigned to global window commands — so it is the one thing the
    /// README and the zoom/paging plan point to as compensation. Mirrors the
    /// shape of the deleted `e_moves_to_the_end_of_the_word` test.
    #[test]
    fn w_moves_to_the_start_of_the_next_word() {
        let mut view = view_of("motions_w.txt", "hello world\n");

        send(&mut view, Key::Char('w'));

        // On the first character of `world`, not still inside `hello`.
        assert_eq!(view.textarea.cursor(), (0, 6));
    }

    #[test]
    fn braces_move_by_paragraph() {
        let mut view = view_of("motions_para.txt", "one\ntwo\n\nthree\nfour\n\nfive\n");

        send(&mut view, Key::Char('}'));
        let after_forward = view.textarea.cursor().0;
        assert!(after_forward > 0, "}} did not move forward");

        send(&mut view, Key::Char('{'));

        assert!(
            view.textarea.cursor().0 < after_forward,
            "{{ did not move back"
        );
    }

    /// The cap is injected rather than taken from `PREVIEW_LINES`, so the
    /// fixture is 30 lines instead of 150,000. The behaviour under test is the
    /// cap, not its value.
    #[test]
    fn preview_stops_at_the_line_cap() {
        let path = long_file("long.txt", 30);
        let mut view = placeholder_view();

        view.preview_with_caps(&path, 10, MAX_PREVIEW_BYTES);

        assert_eq!(view.textarea.lines().len(), 10);
        assert!(
            view.truncated,
            "a capped preview should be marked truncated"
        );
    }

    // ---- adopt hands the textarea a window (#151) -----------------------

    /// `adopt` used to give the textarea a copy of every line read, and every
    /// production caller replaced that copy through `show_window` before the
    /// first draw — so a full load was resident three times at its peak and
    /// every navigator arrow copied the preview twice. The textarea now gets
    /// the window `apply_view` would build before the first render, and no
    /// more.
    #[test]
    fn a_fresh_load_hands_the_textarea_a_window_not_the_whole_file() {
        let path = long_file("adopt_window.txt", 5000);
        let view = FileView::new(path.display().to_string());

        assert_eq!(view.source().len(), 5000, "the file itself is read whole");
        assert_eq!(
            view.textarea.lines().len(),
            WINDOW_SCREENS * usize::from(ASSUMED_PANE_HEIGHT),
            "the buffer got a copy of every line"
        );
        assert_eq!(view.window_start(), 0);
    }

    /// The gutter still fits the whole file's numbers when the buffer is
    /// only a window of it: a 5000-line file needs four digits even though
    /// the window's own rows number to three.
    #[test]
    fn a_windowed_load_reserves_the_gutter_the_whole_file_needs() {
        let path = long_file("adopt_window_gutter.txt", 5000);
        let mut view = FileView::new(path.display().to_string());

        assert_eq!(gutter_digits(&mut view), 4);
    }

    #[test]
    fn a_file_shorter_than_the_window_is_its_own_window() {
        let path = long_file("adopt_short.txt", 30);
        let view = FileView::new(path.display().to_string());

        assert_eq!(view.textarea.lines().len(), 30);
        assert_eq!(view.window_start(), 0);
    }

    /// The value the shipped constant actually takes, kept separate from the
    /// mechanism above: a file past `PREVIEW_LINES` still truncates.
    #[test]
    fn the_real_line_cap_still_truncates_a_file_past_it() {
        let path = long_file("past_cap.txt", PREVIEW_LINES + 10);
        let mut view = placeholder_view();

        view.preview(&path);

        assert_eq!(view.source().len(), PREVIEW_LINES);
        assert!(view.truncated);
    }

    /// The kind of log recon is actually used on runs to about 1 MB and
    /// 10,000 lines at the very top end, and reading one whole costs well
    /// under a millisecond. Previewing a file that size bought nothing and
    /// cost the misleading truncated state reported in #27, so the caps sit
    /// above it and it is simply read.
    #[test]
    fn a_log_sized_file_is_read_whole_rather_than_previewed() {
        let path = long_file("log_sized.txt", 10_000);
        let mut view = placeholder_view();

        view.preview(&path);

        assert_eq!(view.source().len(), 10_000);
        assert!(
            !view.truncated,
            "a log-sized file was previewed rather than read whole"
        );
    }

    #[test]
    fn preview_of_a_short_file_is_complete() {
        let path = long_file("short.txt", 3);
        let mut view = placeholder_view();

        view.preview(&path);

        assert_eq!(view.textarea.lines().len(), 3);
        assert!(!view.truncated, "a fully read file is not truncated");
    }

    /// A file with no newlines is a single enormous line, so the line cap alone
    /// would read the whole thing. The byte cap has to stop it.
    ///
    /// The cap is injected: against the real 10 MiB constant this test built a
    /// 30 MiB fixture and cost more than the entire rest of the suite, which
    /// is the whole reason `read_preview_with_caps` takes the caps.
    #[test]
    fn preview_byte_caps_a_file_without_newlines() {
        let cap: u64 = 64;
        let blob = "x".repeat((cap as usize) * 3);
        let path = fixture("blob.txt", &blob);
        let mut view = placeholder_view();

        view.preview_with_caps(&path, PREVIEW_LINES, cap);

        let read: usize = view.textarea.lines().iter().map(String::len).sum();
        assert!(read as u64 <= cap, "read {read} bytes, over the {cap} cap");
        assert!(view.truncated);
    }

    /// While the nav pane has focus the preview is all that is on screen, but
    /// the moment the view is used it must hold the whole file.
    #[test]
    fn interacting_upgrades_a_truncated_preview() {
        let path = long_file("upgrade.txt", 30);
        let mut view = placeholder_view();
        view.preview_with_caps(&path, 10, MAX_PREVIEW_BYTES);
        assert!(view.truncated);

        view.handle_events(Input {
            key: Key::Down,
            ..Default::default()
        });

        // The upgrade goes through `load`, which is uncapped — so the whole
        // file arrives regardless of the cap the preview was taken with.
        assert_eq!(view.textarea.lines().len(), 30);
        assert!(!view.truncated);
    }

    /// The pane must keep the path it was given, not a rendering of it.
    ///
    /// On Unix a filename is bytes. Storing `Path::display()` — explicitly
    /// lossy — turned an invalid byte into U+FFFD, and every path that
    /// round-tripped the field back into a `Path` to re-read the file then
    /// addressed something that does not exist: `handle_events` promoting a
    /// truncated preview, `App::promote_file_view`, and `App::open_in_editor`
    /// (#79).
    ///
    /// Asserted on the stored value rather than by reading such a file back.
    /// APFS rejects a filename that is not valid UTF-8 outright, and CI runs on
    /// macOS, so a filesystem fixture could not reach this on any machine this
    /// project builds on — while the lossy round trip is the defect itself and
    /// needs no file to exist.
    #[test]
    #[cfg(unix)]
    fn a_filename_that_is_not_utf8_survives_being_stored() {
        use std::os::unix::ffi::OsStrExt;

        // `\xff` is not a valid UTF-8 byte in any position.
        let name = std::ffi::OsStr::from_bytes(b"log_\xffname.txt");
        let path = Path::new("target/test-fixtures").join(name);

        let mut view = FileView::default();
        view.load(&path);

        assert_eq!(
            view.filename(),
            path,
            "the pane is holding a lossy rendering, so re-reading it addresses nothing"
        );
    }

    #[test]
    fn interacting_with_a_complete_file_changes_nothing() {
        let path = long_file("complete.txt", 3);
        let mut view = placeholder_view();
        view.preview(&path);

        view.handle_events(Input {
            key: Key::Down,
            ..Default::default()
        });

        assert_eq!(view.textarea.lines().len(), 3);
    }

    /// A log with one corrupt byte in it is still a log. The bad byte costs
    /// itself — one U+FFFD — and nothing else: every other line is still
    /// there, in place, on its own row.
    #[test]
    fn a_stray_invalid_byte_does_not_discard_the_preview() {
        let path = byte_fixture(
            "preview_stray_byte.log",
            b"alpha\nbra\xffvo\ncharlie\n".as_slice(),
        );
        let mut view = placeholder_view();

        view.preview(&path);

        let lines = view.textarea.lines();
        assert_eq!(lines.len(), 3, "lines lost to one bad byte: {lines:?}");
        assert_eq!(lines[0], "alpha");
        assert_eq!(lines[1], "bra\u{fffd}vo", "bad byte not replaced in place");
        assert_eq!(lines[2], "charlie");
    }

    /// The uncapped read agrees with the preview: one bad byte is one bad
    /// byte, not a two-gigabyte file rendered as a single message.
    #[test]
    fn a_stray_invalid_byte_does_not_discard_the_loaded_file() {
        let path = byte_fixture(
            "load_stray_byte.log",
            b"alpha\nbra\xffvo\ncharlie\n".as_slice(),
        );
        let mut view = placeholder_view();

        view.load(&path);

        let lines = view.textarea.lines();
        assert_eq!(lines.len(), 3, "lines lost to one bad byte: {lines:?}");
        assert_eq!(lines[0], "alpha");
        assert_eq!(lines[1], "bra\u{fffd}vo", "bad byte not replaced in place");
        assert_eq!(lines[2], "charlie");
    }

    /// What makes a file binary is a NUL, not a decode error — the sniff is
    /// bounded to the head of the file, so a NUL further in is data, not a
    /// verdict on the whole file.
    #[test]
    fn a_nul_past_the_sniff_window_is_not_a_binary_verdict() {
        let mut bytes = vec![b'x'; BINARY_SNIFF_BYTES];
        bytes.extend_from_slice(b"\ntail\0end\n");
        let path = byte_fixture("late_nul.log", &bytes);
        let mut view = placeholder_view();

        view.load(&path);

        let lines = view.textarea.lines();
        assert_ne!(
            lines[0], BINARY_MESSAGE,
            "a NUL past the sniff window condemned the whole file"
        );
        assert_eq!(lines.len(), 2, "lines lost to a late NUL: {lines:?}");
    }

    /// UTF-16 with a byte-order mark is text, whatever its NUL count (#165):
    /// both the preview and the full load decode it, and the byte cap still
    /// holds — a preview cut mid-file is truncated, not an error.
    #[test]
    fn preview_and_load_decode_a_utf16_file() {
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(
            "alpha\nbravo\ncharlie\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        let path = byte_fixture("utf16.txt", &bytes);
        let mut view = placeholder_view();

        view.preview(&path);
        assert_eq!(view.textarea.lines(), ["alpha", "bravo", "charlie"]);
        assert!(!view.truncated);

        view.preview_with_caps(&path, 2, MAX_PREVIEW_BYTES);
        assert_eq!(view.textarea.lines(), ["alpha", "bravo"]);
        assert!(view.truncated, "the line cap applies to UTF-16 too");

        // 2 (mark) + 12 (alpha\n) + 4 (br) = 18 bytes: the cap lands mid "bravo".
        view.preview_with_caps(&path, PREVIEW_LINES, 18);
        assert_eq!(view.textarea.lines(), ["alpha", "br"]);
        assert!(view.truncated, "the byte cap applies to UTF-16 too");

        view.load(&path);
        assert_eq!(view.textarea.lines(), ["alpha", "bravo", "charlie"]);
        assert!(!view.truncated);
    }

    #[test]
    fn preview_reports_a_binary_file() {
        let path = byte_fixture("preview_binary.bin", b"\x7fELF\x02\x01\x01\x00");
        let mut view = placeholder_view();

        view.preview(&path);

        assert_eq!(contents(&view), "<binary file: contains NUL bytes>");
        assert!(
            !view.truncated,
            "an error message is not a truncated preview"
        );
    }

    #[test]
    fn preview_of_a_missing_file_shows_a_message() {
        let mut view = placeholder_view();

        view.preview(Path::new("no/such/file.txt"));

        let text = contents(&view);
        assert!(
            text.starts_with('<') && text.ends_with('>'),
            "not a message: {text}"
        );
        assert!(!view.truncated);
    }

    #[test]
    fn loads_file_contents() {
        let path = fixture("new_loads.txt", "the launched file's text\n");
        let view = FileView::new(path.display().to_string());
        assert!(contents(&view).contains("the launched file's text"));
    }

    #[test]
    fn load_replaces_contents_and_title() {
        let first = fixture("load_replaces_first.txt", "first file\n");
        let second = fixture("load_replaces_second.rs", "pub struct Second;\n");
        let mut view = FileView::new(first.display().to_string());

        view.load(&second);

        let text = contents(&view);
        assert!(
            text.contains("pub struct Second"),
            "did not load the second file:\n{text}"
        );
        assert!(!text.contains("first file"), "old contents lingered");
        assert!(
            view.filename()
                .to_string_lossy()
                .contains("load_replaces_second.rs"),
            "title not updated"
        );
    }

    /// A missing file must render a message, not panic the whole TUI.
    #[test]
    fn missing_file_shows_a_message() {
        let first = fixture("missing_file_first.txt", "first file\n");
        let mut view = FileView::new(first.display().to_string());

        view.load(Path::new("no/such/file.txt"));

        let text = contents(&view);
        assert!(
            text.starts_with('<') && text.ends_with('>'),
            "not a message: {text}"
        );
        assert!(!text.contains("first file"), "old contents lingered");
    }

    /// `File::open` succeeds on a directory on Unix; the failure only surfaces
    /// when reading, and must not be mistaken for a UTF-8 problem or leak the
    /// raw `EISDIR` through.
    ///
    /// The directory is recognised *before* opening, which is what keeps both
    /// of those out — the assertion moved from "some `<message>`" to "the
    /// listing" when directories started rendering their contents, but the
    /// failure it guards against is unchanged.
    #[test]
    fn a_directory_is_not_misreported_as_binary_or_an_os_error() {
        let dir = dir_fixture("load_a_directory", &["lib.rs"], &[]);
        let mut view = placeholder_view();

        view.load(&dir);

        let text = contents(&view);
        assert!(text.contains("lib.rs"), "not the listing: {text}");
        assert!(
            !text.contains("binary file"),
            "directory misreported as binary: {text}"
        );
        assert!(
            !text.contains("os error"),
            "raw OS error leaked through: {text}"
        );
    }

    #[test]
    fn binary_file_is_reported_as_binary() {
        let path = byte_fixture("load_binary.bin", b"\x7fELF\x02\x01\x01\x00");
        let mut view = placeholder_view();

        view.load(&path);

        assert_eq!(contents(&view), "<binary file: contains NUL bytes>");
    }

    /// Whether any cell in row `y` carries `colour` as its foreground.
    fn row_has_fg(buf: &Buffer, y: u16, colour: Color) -> bool {
        (0..buf.area.width).any(|x| buf[(x, y)].style().fg == Some(colour))
    }

    /// The row containing `needle`. The view draws a bordered block, so text
    /// does not begin at row 0 and row indices cannot be assumed.
    fn row_of(buf: &Buffer, needle: &str) -> u16 {
        (0..buf.area.height)
            .find(|&y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .contains(needle)
            })
            .unwrap_or_else(|| panic!("no row containing {needle:?}"))
    }

    /// Whether the row's gutter number is underlined. Anchored on the digit
    /// itself rather than a fixed column: the pane's border and the gutter's
    /// right-alignment padding both move it.
    fn gutter_is_underlined(buf: &Buffer, y: u16) -> bool {
        let digit = (0..buf.area.width)
            .find(|&x| {
                buf[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
            })
            .unwrap_or_else(|| panic!("no gutter digit on row {y}"));
        buf[(digit, y)]
            .style()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    }

    #[test]
    fn line_styles_reach_the_rendered_view() {
        let mut view = view_of("line_styles.txt", "alpha\nbeta\n");
        view.set_line_styles(vec![None, Some(Style::default().fg(Color::Yellow))]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        let beta = row_of(&buf, "beta");
        assert!(row_has_fg(&buf, beta, Color::Yellow), "beta not styled");
        assert!(
            !row_has_fg(&buf, alpha, Color::Yellow),
            "alpha wrongly styled"
        );
    }

    #[test]
    fn overridden_line_numbers_reach_the_gutter() {
        let mut view = view_of("line_numbers.txt", "beta\ndelta\n");
        view.set_line_numbers(vec![1, 3]);

        let text = rendered(&mut view);

        assert!(text.contains("2 beta"), "gutter not overridden:\n{text}");
        assert!(text.contains("4 delta"), "gutter not overridden:\n{text}");
    }

    /// Issue #2. The gutter number is underlined on the last line of a group,
    /// which is the only thing on screen saying a run of matches stopped
    /// there rather than continuing into the line below.
    #[test]
    fn a_group_end_underlines_its_gutter_number() {
        let mut view = view_of("group_end.txt", "beta\ndelta\n");
        view.set_group_ends(vec![true, false]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let beta = row_of(&buf, "beta");
        let delta = row_of(&buf, "delta");
        assert!(
            gutter_is_underlined(&buf, beta),
            "the group's last number is not underlined"
        );
        assert!(
            !gutter_is_underlined(&buf, delta),
            "a number mid-group is underlined"
        );
    }

    /// The mark belongs to the gutter alone. Underlining the text would
    /// collide with the filter colours already living there, which is the
    /// whole reason the mark went into the gutter in the first place.
    #[test]
    fn a_group_end_leaves_the_line_text_unmarked() {
        let mut view = view_of("group_end_text.txt", "beta\ndelta\n");
        view.set_group_ends(vec![true, false]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let beta = row_of(&buf, "beta");
        let text_col = (0..area.width)
            .find(|&x| buf[(x, beta)].symbol() == "b")
            .expect("no line text on the row");
        assert!(
            !buf[(text_col, beta)]
                .style()
                .add_modifier
                .contains(Modifier::UNDERLINED),
            "the mark bled into the line text"
        );
    }

    /// An empty `line_numbers` override falls back to natural 1..N
    /// numbering, which is correct when nothing is overridden at all — but
    /// wrong for the single blank placeholder row shown when hiding leaves
    /// nothing visible: that row would render "1" and read as "this file has
    /// one empty line". `set_gutter_blank` suppresses it instead.
    #[test]
    fn gutter_blank_suppresses_the_gutter_even_without_an_override() {
        let mut view = view_of("gutter_blank.txt", "alpha\nbeta\n");
        assert!(
            rendered(&mut view).contains("1 alpha"),
            "sanity: the gutter shows by default"
        );

        view.set_gutter_blank(true);

        let text = rendered(&mut view);
        assert!(
            !text.contains("1 alpha"),
            "gutter number still shown:\n{text}"
        );
    }

    /// Loading a file rebuilds the `TextArea`, which drops both. Phase 2 must
    /// re-apply them after every load; this pins the behaviour so that is not
    /// discovered by surprise.
    #[test]
    fn loading_a_file_clears_line_styles_and_numbers() {
        let path = fixture("reload.txt", "alpha\nbeta\n");
        let mut view = view_of("reload_start.txt", "x\n");
        view.set_line_styles(vec![Some(Style::default().fg(Color::Yellow))]);
        view.set_line_numbers(vec![41]);
        view.set_group_ends(vec![true]);

        view.load(&path);

        assert!(view.textarea.line_styles().is_empty());
        assert!(view.textarea.line_numbers().is_empty());
        assert!(view.textarea.line_number_styles().is_empty());
    }

    /// The cursor line must not escape dimming: the textarea replaces rather
    /// than merges line styles, so `render` has to fold the line's own style
    /// into the cursor-line style.
    #[test]
    fn the_cursor_line_keeps_its_own_line_style() {
        let mut view = view_of("cursor_dim.txt", "alpha\nbeta\n");
        // The cursor starts on row 0.
        view.set_line_styles(vec![
            Some(Style::default().fg(Color::Yellow)),
            Some(Style::default().fg(Color::Yellow)),
        ]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        assert!(
            row_has_fg(&buf, alpha, Color::Yellow),
            "the cursor's line lost its style"
        );
    }

    /// With focus, the cursor line keeps its own style *and* gains the focus
    /// decoration — the decoration is layered on, not substituted.
    ///
    /// The seeded style uses a background colour deliberately: the active
    /// branch sets a foreground, so seeding one would be overwritten and the
    /// test would pass even if the fold were skipped entirely.
    #[test]
    fn an_active_view_still_marks_the_cursor_line() {
        let mut view = view_of("cursor_active.txt", "alpha\nbeta\n");
        view.active = true;
        view.set_line_styles(vec![Some(Style::default().bg(Color::Blue)); 2]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        let beta = row_of(&buf, "beta");

        // The cursor line kept the style it was given...
        assert!(
            (0..area.width).any(|x| buf[(x, alpha)].style().bg == Some(Color::Blue)),
            "the cursor line lost its own style under focus"
        );
        // ...and gained the focus decoration on top of it.
        assert!(
            (0..area.width).any(|x| buf[(x, alpha)]
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)),
            "cursor line not marked when active"
        );
        // A non-cursor line is unaffected either way.
        assert!(
            (0..area.width).any(|x| buf[(x, beta)].style().bg == Some(Color::Blue)),
            "a non-cursor line lost its style"
        );
    }

    /// A cursor line that already carries a filter's own foreground colour
    /// must keep it under focus: overwriting it with the green decoration
    /// would make a matched line and a dimmed line look identical under the
    /// one line you're actually looking at.
    #[test]
    fn a_filter_coloured_cursor_line_keeps_its_colour_when_active() {
        let mut view = view_of("cursor_filtered.txt", "alpha\nbeta\n");
        view.active = true;
        // The cursor starts on row 0, which is given its own foreground here.
        view.set_line_styles(vec![Some(Style::default().fg(Color::Magenta)), None]);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        assert!(
            row_has_fg(&buf, alpha, Color::Magenta),
            "the cursor line lost its filter colour under focus"
        );
        assert!(
            (0..area.width).any(|x| buf[(x, alpha)]
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)),
            "cursor line not marked when active"
        );
    }

    /// `[` and `]` page in opposite directions, sitting next to the `{`/`}`
    /// paragraph motions: brackets move by page, braces by paragraph, and both
    /// keep left-is-back, right-is-forward.
    ///
    /// They replaced `space`/`Enter` in #48. `space` became the global peek and
    /// `Enter` the filter pane's toggle, and neither can also page here.
    ///
    /// Asserts the **distance**, not just the direction. This test used to
    /// check only that `]` moved the cursor off row 0 and that `[` moved it
    /// back somewhere above that, which #108 slipped straight through: `[` was
    /// paging three lines instead of thirty-three and every assertion here
    /// still held. A page is the pane's inner height — the area less its two
    /// border rows, with no overlap row kept.
    #[test]
    fn brackets_page_up_and_down() {
        let body = numbered_lines(200);
        let mut view = view_of("bracket_pages.txt", &body);
        let area = Rect::new(0, 0, 40, 10);
        let page = area.height as usize - 2;
        let mut buf = Buffer::empty(area);
        (&mut view).render(area, &mut buf);

        let top = |view: &FileView<'_>| view.textarea.scroll_top().0 as usize;
        assert_eq!(top(&view), 0, "sanity: starts at the top of the file");

        send(&mut view, Key::Char(']'));
        (&mut view).render(area, &mut buf);
        assert_eq!(top(&view), page, "`]` did not page down a full screen");

        send(&mut view, Key::Char('['));
        (&mut view).render(area, &mut buf);
        assert_eq!(top(&view), 0, "`[` did not page back up a full screen");
    }

    /// The keys `[` and `]` took over must not still page, or the file view
    /// would quietly keep a second copy of a binding that now belongs to
    /// another pane.
    #[test]
    fn space_and_enter_no_longer_page() {
        let body = numbered_lines(200);
        let mut view = view_of("no_page_keys.txt", &body);
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        (&mut view).render(area, &mut buf);
        let start = view.textarea.cursor().0;

        send(&mut view, Key::Enter);
        send(&mut view, Key::Char(' '));
        (&mut view).render(area, &mut buf);

        assert_eq!(
            view.textarea.cursor().0,
            start,
            "space or Enter still moved the file view"
        );
    }

    /// Without line styles, the old behaviour is unchanged.
    #[test]
    fn without_line_styles_the_cursor_line_is_unchanged() {
        let mut view = view_of("cursor_plain.txt", "alpha\nbeta\n");
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        (&mut view).render(area, &mut buf);

        let alpha = row_of(&buf, "alpha");
        assert!(!row_has_fg(&buf, alpha, Color::Yellow));
    }

    /// Pins the fix directly at the `FileView` level, without going through
    /// `App`: `show_lines_with_cursor` resets the viewport to zeroed
    /// dimensions, and `apply_pending_scroll` (run from `render`, on the
    /// next frame) has to prime a throwaway render against the real area
    /// before scrolling — without that, `TextArea::scroll`'s own
    /// `CursorMove::InViewport` bookkeeping clamps *the cursor itself* (not
    /// just the view) onto the scroll target, since the zeroed height
    /// collapses its valid range to a single row. This is the piece most
    /// likely to rot silently — the `App`-level tests only exercise it
    /// indirectly, through a whole filter toggle.
    #[test]
    fn scroll_cursor_to_row_primes_the_viewport_before_scrolling() {
        let body = numbered_lines(200);
        let mut view = view_of("scroll_cursor_prime.txt", &body);
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        // Establishes the pane's real viewport dimensions, the way the
        // pane's first real render of the session would.
        (&mut view).render(area, &mut buf);

        // Simulate a rebuild: `show_lines_with_cursor` calls `set_lines`,
        // which resets the viewport to zeroed dimensions, exactly as a real
        // filter toggle's rebuild would.
        let lines = view.textarea.lines().to_vec();
        view.show_lines_with_cursor(lines, 150);
        view.scroll_cursor_to_row(3);

        // The scroll is only *requested* until the next render applies it —
        // see `apply_pending_scroll` — against that render's real area.
        (&mut view).render(area, &mut buf);

        assert_eq!(
            view.textarea.cursor().0,
            150,
            "the cursor's line moved — the zeroed-viewport clamp corrupted \
             the cursor's data position, not just the view"
        );
        assert_eq!(
            view.cursor_screen_row(),
            3,
            "the cursor did not land on the requested screen row"
        );
    }

    /// Under the old immediate-priming design, a restore requested before
    /// any render had ever happened had no real area to prime against and
    /// had to be a no-op. The deferred design has no such gap: the request
    /// is just recorded, and the pane's very first render — there does not
    /// need to be an earlier one — supplies a real area to apply it
    /// against. This pins that positive claim directly, rather than a
    /// "no-op" claim the new design no longer makes true.
    #[test]
    fn a_scroll_requested_before_any_render_is_applied_on_the_first_render() {
        let body = numbered_lines(200);
        let mut view = view_of("scroll_cursor_no_area.txt", &body);

        let lines = view.textarea.lines().to_vec();
        view.show_lines_with_cursor(lines, 150);
        view.scroll_cursor_to_row(3);

        // The pane's first render ever — nothing has primed anything before
        // this.
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        (&mut view).render(area, &mut buf);

        assert_eq!(
            view.textarea.cursor().0,
            150,
            "the cursor's line moved — the zeroed-viewport clamp corrupted \
             the cursor's data position, not just the view"
        );
        assert_eq!(
            view.cursor_screen_row(),
            3,
            "the requested scroll was dropped instead of being applied on \
             the first render"
        );
    }

    /// `get_or_insert`, not overwrite: if a second rebuild (and a second
    /// `scroll_cursor_to_row` call) happens before the next render, the
    /// *first* row is the one that was measured against a viewport still
    /// valid at the time, and must be what is applied — a second request
    /// arriving before the deferred restore has ever run must not silently
    /// replace it.
    #[test]
    fn a_second_pending_scroll_before_the_next_render_does_not_overwrite_the_first() {
        let body = numbered_lines(200);
        let mut view = view_of("scroll_get_or_insert.txt", &body);
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        (&mut view).render(area, &mut buf);

        let lines = view.textarea.lines().to_vec();
        view.show_lines_with_cursor(lines.clone(), 150);
        view.scroll_cursor_to_row(3);
        view.show_lines_with_cursor(lines, 160);
        view.scroll_cursor_to_row(6);

        (&mut view).render(area, &mut buf);

        assert_eq!(
            view.cursor_screen_row(),
            3,
            "the second scroll request overwrote the first instead of being ignored"
        );
    }

    // ---- scroll margin ---------------------------------------------------

    /// Render `view` into a pane `height` rows tall, borders included, so
    /// the viewport learns its real dimensions the way a frame would give it.
    fn render_at(view: &mut FileView<'_>, height: u16) {
        let area = Rect::new(0, 0, 40, height);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
    }

    /// A pane 14 rows tall outside the borders is 12 inside, so the margin
    /// is `SCROLL_MARGIN` rows on each edge and the cursor can occupy rows
    /// 5..=6 of the pane while there is file above and below it.
    const MARGIN_PANE: u16 = 14;
    const MARGIN_INNER: usize = 12;

    /// One keypress, then the frame that follows it, so the viewport has
    /// settled before the next key the way it would between real events.
    fn press(view: &mut FileView<'_>, key: Key) {
        send(view, key);
        render_at(view, MARGIN_PANE);
    }

    #[test]
    fn j_into_the_bottom_margin_scrolls_the_view_rather_than_the_cursor_down() {
        let mut view = view_of("margin_j.txt", &numbered_lines(200));
        render_at(&mut view, MARGIN_PANE);
        let lowest = MARGIN_INNER - 1 - SCROLL_MARGIN;

        for _ in 0..lowest {
            press(&mut view, Key::Char('j'));
        }
        assert_eq!(
            view.textarea.scroll_top().0,
            0,
            "scrolled before the margin was reached"
        );
        assert_eq!(view.cursor_screen_row(), lowest as u16);

        press(&mut view, Key::Char('j'));
        assert_eq!(
            view.textarea.cursor().0,
            lowest + 1,
            "the cursor did not move down a line"
        );
        assert_eq!(
            view.cursor_screen_row(),
            lowest as u16,
            "the cursor entered the bottom margin instead of the view scrolling"
        );
        assert_eq!(view.textarea.scroll_top().0, 1);
    }

    /// `Ctrl-E` moves the view, not the cursor, so the cursor is the thing
    /// that has to give way when the scroll pushes the margin over it.
    /// Left to the render-time rule alone, the view would be scrolled
    /// straight back and `Ctrl-E` would do nothing at all.
    #[test]
    fn ctrl_e_moves_the_cursor_down_into_the_margin_rather_than_undoing_the_scroll() {
        let mut view = view_of("margin_ctrl_e.txt", &numbered_lines(200));
        render_at(&mut view, MARGIN_PANE);

        view.handle_events(Input {
            key: Key::Char('e'),
            ctrl: true,
            ..Default::default()
        });
        render_at(&mut view, MARGIN_PANE);

        assert_eq!(view.textarea.scroll_top().0, 1, "the scroll was undone");
        assert_eq!(view.textarea.cursor().0, 1 + SCROLL_MARGIN);
    }

    #[test]
    fn page_down_leaves_the_cursor_a_margin_below_the_new_top() {
        let mut view = view_of("margin_page_down.txt", &numbered_lines(200));
        render_at(&mut view, MARGIN_PANE);

        press(&mut view, Key::Char(']'));

        assert_eq!(
            view.textarea.scroll_top().0 as usize,
            MARGIN_INNER,
            "not a full page"
        );
        assert_eq!(view.textarea.cursor().0, MARGIN_INNER + SCROLL_MARGIN);
    }

    #[test]
    fn ctrl_y_moves_the_cursor_up_out_of_the_bottom_margin() {
        let mut view = view_of("margin_ctrl_y.txt", &numbered_lines(200));
        render_at(&mut view, MARGIN_PANE);
        press(&mut view, Key::Char(']'));
        // Down to the last row the cursor may occupy without scrolling.
        press(&mut view, Key::Char('j'));
        let lowest = MARGIN_INNER - 1 - SCROLL_MARGIN;
        assert_eq!(view.cursor_screen_row() as usize, lowest, "sanity");
        let before = view.textarea.cursor().0;

        view.handle_events(Input {
            key: Key::Char('y'),
            ctrl: true,
            ..Default::default()
        });
        render_at(&mut view, MARGIN_PANE);

        assert_eq!(
            view.textarea.scroll_top().0 as usize,
            MARGIN_INNER - 1,
            "the scroll was undone"
        );
        assert_eq!(
            view.textarea.cursor().0,
            before - 1,
            "the cursor stayed in the margin"
        );
    }

    #[test]
    fn k_into_the_top_margin_scrolls_the_view_rather_than_the_cursor_up() {
        let mut view = view_of("margin_k.txt", &numbered_lines(200));
        render_at(&mut view, MARGIN_PANE);
        // Two pages down: the top margin is real here, not waived by row 0.
        press(&mut view, Key::Char(']'));
        press(&mut view, Key::Char(']'));
        let top = view.textarea.scroll_top().0 as usize;
        assert_eq!(view.textarea.cursor().0, top + SCROLL_MARGIN, "sanity");

        press(&mut view, Key::Char('k'));

        assert_eq!(view.textarea.cursor().0, top + SCROLL_MARGIN - 1);
        assert_eq!(
            view.cursor_screen_row() as usize,
            SCROLL_MARGIN,
            "the cursor entered the top margin instead of the view scrolling"
        );
        assert_eq!(view.textarea.scroll_top().0 as usize, top - 1);
    }

    /// The bottom margin is waived at the end of the file: the last line is
    /// selectable and sits on the pane's last row, with no blank rows
    /// scrolled in below it to make room for a margin that has nothing in it.
    #[test]
    fn the_last_line_reaches_the_bottom_row_without_scrolling_past_the_end() {
        let mut view = view_of("margin_end.txt", &numbered_lines(200));
        render_at(&mut view, MARGIN_PANE);

        press(&mut view, Key::End);

        assert_eq!(view.textarea.cursor().0, 199);
        assert_eq!(view.cursor_screen_row() as usize, MARGIN_INNER - 1);
        assert_eq!(view.textarea.scroll_top().0 as usize, 200 - MARGIN_INNER);
    }

    /// And the top margin at the start of it.
    #[test]
    fn the_first_line_reaches_the_top_row() {
        let mut view = view_of("margin_start.txt", &numbered_lines(200));
        render_at(&mut view, MARGIN_PANE);
        press(&mut view, Key::Char(']'));

        press(&mut view, Key::Home);

        assert_eq!(view.textarea.cursor().0, 0);
        assert_eq!(view.cursor_screen_row(), 0);
        assert_eq!(view.textarea.scroll_top().0, 0);
    }

    #[test]
    fn a_short_pane_shrinks_the_margin_to_leave_the_cursor_a_row() {
        assert_eq!(scroll_margin(12), SCROLL_MARGIN);
        assert_eq!(
            scroll_margin(11),
            SCROLL_MARGIN,
            "one row left for the cursor"
        );
        assert_eq!(scroll_margin(7), 3);
        assert_eq!(scroll_margin(3), 1);
        assert_eq!(scroll_margin(2), 0);
        assert_eq!(scroll_margin(1), 0);
        assert_eq!(scroll_margin(0), 0);
    }

    /// Columns the line-number gutter occupies, read back off a real render.
    ///
    /// A row is `border + margin + right-aligned number + margin + text`, so
    /// on the buffer's first row — whose number is always `1` — the sole
    /// digit sits at column `1 + 1 + lnum_len - 1`, and the width falls out
    /// as `column - 1`.
    ///
    /// Indexes buffer cells rather than searching a joined `String`: the
    /// border is `\u{2502}`, three bytes in UTF-8, so `str::find` returns a
    /// byte offset two greater than the column and every measurement taken
    /// that way is quietly wrong.
    fn gutter_digits(view: &mut FileView<'_>) -> usize {
        let area = Rect::new(0, 0, 60, 4);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let digit_column = (0..area.width)
            .find(|&x| {
                buf[(x, 1)]
                    .symbol()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
            })
            .expect("no line number rendered") as usize;
        digit_column - 1
    }

    /// A directory of known contents, for the listing tests.
    fn dir_fixture(name: &str, files: &[&str], subdirs: &[&str]) -> std::path::PathBuf {
        let dir = fixture_dir(name);
        for file in files {
            fs::write(dir.join(file), "x").expect("write fixture file");
        }
        for sub in subdirs {
            fs::create_dir_all(dir.join(sub)).expect("create fixture subdir");
        }
        dir
    }

    /// Line numbers beside directory entries number nothing — a listing is not
    /// a document, and "3" against the third filename is noise.
    ///
    /// Derived from what is on screen rather than saved and restored. Stashing
    /// the user's `#` preference and putting it back would clobber a `#`
    /// pressed *while* the directory was up; a condition re-evaluated per
    /// render cannot desync.
    #[test]
    fn the_gutter_is_suppressed_for_a_directory_and_returns_for_a_file() {
        let file = fixture("gutter_dir_file.txt", "alpha\nbeta\n");
        let dir = dir_fixture("gutter_dir", &["one.txt", "two.txt"], &[]);
        let mut view = FileView::new(file.display().to_string());
        assert!(
            rendered(&mut view).contains("1 alpha"),
            "sanity: the gutter shows for a file"
        );

        view.preview(&dir);
        let listing = rendered(&mut view);
        assert!(
            !listing.contains("1 one.txt"),
            "line numbers drawn beside directory entries:\n{listing}"
        );

        view.preview(&file);
        assert!(
            rendered(&mut view).contains("1 alpha"),
            "the gutter did not come back for a file"
        );
    }

    /// A `#` pressed while a directory is on screen still sets the user's
    /// preference, and it survives the return to a file.
    ///
    /// This is the case that separates a derived condition from saving and
    /// restoring `hide_line_numbers`: a save/restore would put the
    /// pre-directory value back and silently discard the keystroke. Pressing
    /// `#` once, not twice — two presses cancel out and would pass against
    /// either implementation.
    #[test]
    fn a_hide_toggle_pressed_over_a_directory_still_takes_effect() {
        let file = fixture("gutter_dir_toggle_file.txt", "alpha\nbeta\n");
        let dir = dir_fixture("gutter_dir_toggle", &["one.txt"], &[]);
        let mut view = FileView::new(file.display().to_string());
        assert!(
            rendered(&mut view).contains("1 alpha"),
            "sanity: the gutter starts visible"
        );

        view.preview(&dir);
        send(&mut view, Key::Char('#'));
        view.preview(&file);

        let text = rendered(&mut view);
        assert!(
            !text.contains("1 alpha"),
            "the `#` pressed over the directory was discarded on the way out:\n{text}"
        );
    }

    /// The view is the pane with width to spare — the navigator is capped at
    /// `MAX_NAV_WIDTH` and could never carry these — so the listing shows what
    /// `ls -l` would: how big, and when it last changed.
    ///
    /// A directory gets `-` for size rather than the number `stat` reports,
    /// which is the size of the directory file and not of its contents.
    #[test]
    fn the_listing_shows_size_and_modification_time() {
        let dir = dir_fixture("dir_columns", &["alpha.txt"], &["subdir"]);
        let mut view = placeholder_view();

        view.preview(&dir);

        let text = contents(&view);
        let file_row = text
            .lines()
            .find(|line| line.contains("alpha.txt"))
            .expect("file listed")
            .to_string();
        let dir_row = text
            .lines()
            .find(|line| line.contains("subdir/"))
            .expect("directory listed")
            .to_string();

        // `dir_fixture` writes one byte.
        assert!(file_row.contains("1B"), "size missing: {file_row:?}");
        // A local calendar date, not a raw SystemTime or a UTC instant.
        let date = regex::Regex::new(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}").expect("valid");
        assert!(date.is_match(&file_row), "mtime missing: {file_row:?}");
        assert!(
            date.is_match(&dir_row),
            "directory has no mtime: {dir_row:?}"
        );
        assert!(
            dir_row.contains(" - "),
            "a directory should report no content size: {dir_row:?}"
        );
    }

    /// Selecting a directory shows what is *in* it. The view is the widest
    /// pane on screen and was spending all of it on the word `<directory>`.
    ///
    /// It is a look-ahead rather than a pane you act in, which is what keeps
    /// it from being the "navigable-looking list you cannot navigate" that
    /// #15 rejected: `l` on the selected directory makes it the navigator's
    /// listing, so there is a one-key path from looking to being there.
    /// Moving the selection onto a FIFO fires a preview, and `File::open` on
    /// a FIFO blocks until a writer appears — the TUI would hang on an arrow
    /// key. The stat guard turns it into a message (#221).
    #[cfg(unix)]
    #[test]
    fn a_non_regular_file_previews_as_a_message_without_opening_it() {
        let dir = fixture_dir("preview_socket");
        let sock = dir.join("sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind");
        let mut view = placeholder_view();

        view.preview(&sock);

        assert_eq!(contents(&view), "<not a regular file>");
    }

    #[test]
    fn a_directory_previews_as_its_listing() {
        let dir = dir_fixture("dir_listing", &["alpha.txt", "beta.txt"], &["subdir"]);
        let mut view = placeholder_view();

        view.preview(&dir);

        let text = contents(&view);
        assert!(text.contains("alpha.txt"), "entry missing: {text}");
        assert!(text.contains("beta.txt"), "entry missing: {text}");
        assert!(
            text.contains("subdir/"),
            "directory not marked with `/`: {text}"
        );
        assert!(
            !text.contains(".."),
            "a look-ahead should not offer `..`, which is not actionable here: {text}"
        );
    }

    /// The listing pads the name column so the size column lines up. Pad by
    /// `char` count and a CJK name pushes its own size column one place right
    /// per ideograph, so the columns stop being columns (#97).
    ///
    /// Both halves of the fix are needed and this fails if either is missing:
    /// `name_width` must be measured in columns, *and* the padding must be
    /// applied in columns — `{:<width$}` counts chars.
    #[test]
    fn the_listing_name_column_aligns_across_wide_glyphs() {
        use crate::widgets::filenav::Match;
        let named = |name: &str| Entry {
            name: name.into(),
            kind: crate::widgets::filenav::Kind::Plain,
            size: Some(1),
            modified: None,
            matched: Match::Unknown,
        };
        // Both names are 10 terminal columns wide: 3 ideographs (6) + `.txt`,
        // against 10 ASCII characters. Padded to 12, both rows must come out
        // the same width — with `modified` empty, the row is exactly the padded
        // name plus a fixed size column, so total width *is* the alignment.
        let wide = named("日本語.txt");
        let ascii = named("ascii.txt0");
        assert_eq!(UnicodeWidthStr::width("日本語.txt"), 10);

        let wide_row = listing_row(&wide, 12);
        let ascii_row = listing_row(&ascii, 12);

        assert_eq!(
            UnicodeWidthStr::width(wide_row.as_str()),
            UnicodeWidthStr::width(ascii_row.as_str()),
            "the size columns do not start at the same place\n  wide: {wide_row:?}\n ascii: {ascii_row:?}"
        );
    }

    /// A directory with nothing in it is the one case that could read as a
    /// bug rather than an answer, so it says so rather than rendering blank.
    ///
    /// Replaces `a_directory_previews_as_a_directory`, which asserted the
    /// `<directory>` placeholder for *every* directory. That placeholder now
    /// survives only here — a directory with contents renders them.
    #[test]
    fn an_empty_directory_says_so() {
        let dir = dir_fixture("dir_empty", &[], &[]);
        let mut view = placeholder_view();

        view.preview(&dir);

        assert_eq!(contents(&view), EMPTY_DIRECTORY_MESSAGE);
        assert!(!view.truncated, "a directory is not a truncated preview");
    }

    /// `load` is only reached for files today — the navigator descends into a
    /// directory rather than loading it — but it must not be the one place
    /// that leaks a raw OS error if that ever changes. It lists the directory
    /// exactly as `preview` does, unbounded, since `load` is the uncapped path.
    #[test]
    fn loading_a_directory_lists_it_the_same_way() {
        let dir = dir_fixture("dir_load", &["gamma.txt"], &[]);
        let mut view = placeholder_view();

        view.load(&dir);

        let text = contents(&view);
        assert_eq!(text.lines().count(), 1, "one row per entry: {text:?}");
        assert!(text.contains("gamma.txt"), "entry missing: {text:?}");
    }

    /// Issue #1. A preview holds `PREVIEW_LINES` rows, so the gutter is sized
    /// for 500 while the file has thousands; focusing the view loads the rest
    /// and the gutter widens, shifting every line of text sideways on a pane
    /// the user is already reading. The two renders must agree.
    #[test]
    fn a_preview_reserves_the_gutter_the_full_file_will_need() {
        let path = long_file("gutter_preview.txt", 5000);
        let mut view = placeholder_view();

        view.preview(&path);
        let previewed = gutter_digits(&mut view);
        view.load(&path);
        let loaded = gutter_digits(&mut view);

        assert_eq!(
            previewed, loaded,
            "gutter jumped from {previewed} to {loaded} when the file finished loading"
        );
        assert_eq!(loaded, 4, "a 5000-line file needs four digits");
    }

    /// The helper has to be able to see the defect it is asserting the
    /// absence of, or the test above passes for the wrong reason.
    #[test]
    fn gutter_digits_tracks_the_line_count() {
        let short = long_file("gutter_sensitivity.txt", 5);
        let mut view = FileView::new(short.display().to_string());

        assert_eq!(gutter_digits(&mut view), 1);
    }

    /// A file read whole is not a preview, so there is nothing to estimate
    /// and nothing to reserve — it must not pay for a column it never uses.
    #[test]
    fn a_short_file_reserves_nothing() {
        let path = long_file("gutter_short.txt", 12);
        let mut view = placeholder_view();

        view.preview(&path);

        assert!(!view.truncated, "precondition: the file was read whole");
        assert_eq!(view.textarea.min_line_number_width(), 0);
        assert_eq!(gutter_digits(&mut view), 2, "12 lines need two digits");
    }

    /// The estimate scales the preview's own bytes-per-line up to the file's
    /// size, so lines far longer than average must not under-reserve. This
    /// file has the same 5000 lines as the one above but each is ~200 bytes
    /// longer, which a byte-blind estimate would read as a much longer file.
    #[test]
    fn the_estimate_scales_with_line_length() {
        let padding = "x".repeat(200);
        let body = (0..5000).fold(String::new(), |mut body, i| {
            let _ = writeln!(body, "line {i} {padding}");
            body
        });
        let path = fixture("gutter_wide.txt", &body);
        let mut view = placeholder_view();

        view.preview(&path);
        let previewed = gutter_digits(&mut view);
        view.load(&path);

        assert_eq!(
            previewed,
            gutter_digits(&mut view),
            "long lines threw the estimate off"
        );
    }

    /// A full load knows the real count, so it must drop the reservation
    /// rather than leave the previous file's propped up under a short one.
    #[test]
    fn loading_clears_a_reservation_the_preview_made() {
        let big = long_file("gutter_clear_big.txt", 5000);
        let small = long_file("gutter_clear_small.txt", 5);
        let mut view = placeholder_view();

        // Capped low so the preview truncates and so reserves gutter room;
        // 5000 lines is well inside the shipped cap and would be read whole.
        view.preview_with_caps(&big, 100, MAX_PREVIEW_BYTES);
        assert!(
            view.textarea.min_line_number_width() > 0,
            "precondition: the preview reserved room"
        );
        view.load(&small);

        assert_eq!(
            view.textarea.min_line_number_width(),
            0,
            "the previous file's reservation outlived it"
        );
        assert_eq!(gutter_digits(&mut view), 1);
    }

    // ---- syntax colouring (#122) --------------------------------------

    fn coloured_view(name: &str, body: &str) -> FileView<'static> {
        let mut view = view_of(name, body);
        view.set_theme(Theme::builtin());
        view
    }

    fn buffer(view: &mut FileView<'_>) -> Buffer {
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        buf
    }

    /// The style of the first cell of `needle` on row `y`.
    ///
    /// Found cell by cell rather than with `str::find` on the joined row: the
    /// border is a multi-byte glyph, so a byte index into the row is not a
    /// column.
    fn style_at(buf: &Buffer, y: u16, needle: &str) -> Style {
        let symbols: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        let x = (0..symbols.len())
            .find(|&x| symbols[x..].concat().starts_with(needle))
            .unwrap_or_else(|| panic!("no {needle:?} on row {y}: {:?}", symbols.concat()));
        buf[(x as u16, y)].style()
    }

    fn fg_at(buf: &Buffer, y: u16, needle: &str) -> Color {
        style_at(buf, y, needle)
            .fg
            .expect("a cell always has a foreground")
    }

    // Bodies below put the line under test *second*: the textarea draws its
    // cursor cell at column 0 of the cursor row whether or not the pane is
    // active, and that cell's style is the cursor's, not the text's.

    #[test]
    fn syntax_colours_reach_the_rendered_view() {
        let mut view = coloured_view("syntax_rust.rs", "let x = 1;\nfn main() {} // hi");
        assert_eq!(view.syntax_name(), Some("Rust"));
        let buf = buffer(&mut view);
        let y = row_of(&buf, "fn main");
        assert_eq!(
            fg_at(&buf, y, "fn"),
            Color::Magenta,
            "ansi: keywords are slot 5"
        );
        assert_eq!(
            fg_at(&buf, y, "// hi"),
            Color::Green,
            "ansi: comments are slot 2"
        );
        assert_eq!(
            fg_at(&buf, y, "()"),
            Color::Reset,
            "punctuation takes the terminal's default"
        );
    }

    #[test]
    fn without_a_theme_nothing_is_coloured() {
        let mut view = view_of("syntax_off.rs", "let x = 1;\nfn main() {} // hi");
        assert_eq!(view.syntax_name(), None);
        let buf = buffer(&mut view);
        let y = row_of(&buf, "fn main");
        assert_eq!(fg_at(&buf, y, "fn"), Color::Reset);
        assert_eq!(fg_at(&buf, y, "// hi"), Color::Reset);
    }

    #[test]
    fn a_theme_set_after_loading_recolours_the_file_already_shown() {
        let mut view = view_of("syntax_late.rs", "let x = 1;\nfn main() {}");
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn"), "fn"), Color::Reset);

        view.set_theme(Theme::builtin());
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn"), "fn"), Color::Magenta);

        view.set_theme(Theme::Off);
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn"), "fn"), Color::Reset);
    }

    #[test]
    fn a_file_no_grammar_claims_is_not_coloured() {
        let mut view = coloured_view("syntax_notes.txt", "let x = 1;\nfn main() {}");
        assert_eq!(view.syntax_name(), None);
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn"), "fn"), Color::Reset);
    }

    #[test]
    fn a_line_a_filter_styled_keeps_the_filter_s_colour() {
        let mut view = coloured_view("syntax_filtered.rs", "let x = 1;\nfn a() {}\nfn b() {}");
        view.set_line_styles(vec![None, Some(Style::default().fg(Color::Yellow)), None]);
        let buf = buffer(&mut view);
        let a = row_of(&buf, "fn a");
        let b = row_of(&buf, "fn b");
        assert_eq!(
            fg_at(&buf, a, "fn"),
            Color::Yellow,
            "the filter's colour, whole line"
        );
        assert_eq!(fg_at(&buf, a, "()"), Color::Yellow);
        assert_eq!(
            fg_at(&buf, b, "fn"),
            Color::Magenta,
            "an unfiltered line is coloured"
        );
    }

    #[test]
    fn the_active_cursor_line_is_the_focus_bar_not_coloured_words() {
        let mut view = coloured_view("syntax_cursor.rs", "fn a() {}\nfn b() {}");
        view.set_active(true);
        let buf = buffer(&mut view);
        let a = row_of(&buf, "fn a");
        let b = row_of(&buf, "fn b");
        let bar = style_at(&buf, a, "n a");
        assert_eq!(bar.fg, Some(Color::Green));
        assert!(bar.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(style_at(&buf, a, "()"), bar, "one unbroken bar");
        assert_eq!(fg_at(&buf, b, "fn"), Color::Magenta);

        // Inactive, the cursor line is an ordinary line again.
        view.set_active(false);
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, a, "n a"), Color::Magenta);
    }

    #[test]
    fn a_search_match_stays_black_on_yellow_over_syntax_colour() {
        let mut view = coloured_view("syntax_search.rs", "let x = 1;\nfn main() {} // hi");
        // `n m` straddles the end of the keyword and the start of the name, so
        // a syntax span begins inside the match — the case the fork's
        // priority does not cover.
        view.set_highlight(Some("n m")).unwrap();
        let buf = buffer(&mut view);
        let y = row_of(&buf, "fn main");
        assert_eq!(
            fg_at(&buf, y, "f"),
            Color::Magenta,
            "the keyword up to the match"
        );
        for needle in ["n m", " m", "m"] {
            let hit = style_at(&buf, y, needle);
            assert_eq!(hit.bg, Some(Color::Yellow), "{needle:?} is a search hit");
            assert_eq!(hit.fg, Some(Color::Black), "{needle:?} is a search hit");
        }
        assert_ne!(
            style_at(&buf, y, "ain").bg,
            Some(Color::Yellow),
            "the match ends"
        );
        assert_eq!(
            fg_at(&buf, y, "// hi"),
            Color::Green,
            "colour resumes past it"
        );
    }

    #[test]
    fn a_windowed_buffer_is_coloured_by_source_line_not_buffer_row() {
        let body = "plain\nplain\nplain\nfn a() {}";
        let mut view = coloured_view("syntax_window.rs", body);

        // The gutter override says which source line each row is.
        view.show_window(vec!["fn a() {}".to_string()], 3, 0);
        view.set_line_numbers(vec![3]);
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn a"), "n a"), Color::Magenta);

        // Without one, the window's offset does.
        view.show_window(vec!["fn a() {}".to_string()], 3, 0);
        view.set_line_numbers(Vec::new());
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn a"), "n a"), Color::Magenta);

        // A row claiming to be source line 0 gets line 0's (absent) colour,
        // whatever text it holds — proof the lookup is by source line.
        view.show_window(vec!["fn a() {}".to_string()], 0, 0);
        view.set_line_numbers(vec![0]);
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn a"), "n a"), Color::Reset);
    }

    #[test]
    fn a_binary_file_with_a_source_extension_is_not_coloured() {
        let path = byte_fixture("syntax_binary.rs", b"\0\0\0");
        let mut view = FileView::new(path.display().to_string());
        view.set_theme(Theme::builtin());
        assert_eq!(view.syntax_name(), None);
        let buf = buffer(&mut view);
        let y = row_of(&buf, "binary");
        assert_eq!(fg_at(&buf, y, "binary"), Color::Reset);
    }

    #[test]
    fn a_directory_named_like_a_source_file_is_not_coloured() {
        let dir = dir_fixture("syntax_listing.rs", &["main.rs"], &[]);
        let mut view = FileView::new(dir.display().to_string());
        view.set_theme(Theme::builtin());
        assert_eq!(view.syntax_name(), None);
    }

    #[test]
    fn a_preview_is_coloured_and_so_is_the_load_that_replaces_it() {
        let path = fixture(
            "syntax_preview.rs",
            "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}",
        );
        let mut view = FileView::default();
        view.set_theme(Theme::builtin());
        view.preview_with_caps(&path, 2, MAX_PREVIEW_BYTES);
        assert!(view.is_truncated());
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn b"), "fn"), Color::Magenta);

        view.load(&path);
        assert!(!view.is_truncated());
        let buf = buffer(&mut view);
        assert_eq!(fg_at(&buf, row_of(&buf, "fn d"), "fn"), Color::Magenta);
    }

    #[test]
    fn around_cuts_the_holes_out_of_a_span() {
        let span = |start, end| Span {
            start,
            end,
            style: Style::default().fg(Color::Red),
        };
        let pieces = |holes: &[(usize, usize)]| around(span(2, 10), holes).collect::<Vec<_>>();

        assert_eq!(pieces(&[]), [span(2, 10)]);
        assert_eq!(
            pieces(&[(4, 6)]),
            [span(2, 4), span(6, 10)],
            "a hole in the middle"
        );
        assert_eq!(pieces(&[(0, 4)]), [span(4, 10)], "a hole over the start");
        assert_eq!(pieces(&[(8, 12)]), [span(2, 8)], "a hole over the end");
        assert_eq!(
            pieces(&[(0, 2), (10, 12)]),
            [span(2, 10)],
            "holes that only touch"
        );
        assert!(pieces(&[(2, 10)]).is_empty(), "a hole that is the span");
        assert!(
            pieces(&[(0, 12)]).is_empty(),
            "a hole that swallows the span"
        );
        assert_eq!(
            pieces(&[(3, 4), (5, 6), (7, 8)]),
            [span(2, 3), span(4, 5), span(6, 7), span(8, 10)]
        );
    }
}
