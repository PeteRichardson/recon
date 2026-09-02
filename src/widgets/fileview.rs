use crate::widgets::filenav::Entry;
/// `FileView` Widget
///
///
use color_eyre::Result;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read};
use std::path::Path;
use tui_textarea::{CursorMove, Input, Key, Scrolling, TextArea};

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
/// — undecodable bytes on their own no longer stop the file being read.
const BINARY_MESSAGE: &str = "<binary file: contains NUL bytes>";

/// How much of a file's head is sniffed for a NUL byte before deciding it is
/// binary rather than text.
///
/// Bounded because the decision has to be made before any of the file is
/// shown, and unbounded means reading a two-gigabyte log twice. A NUL past
/// this window is treated as ordinary data, on the same terms as any other
/// undecodable byte: it costs a replacement character where it sits and
/// nothing more.
const BINARY_SNIFF_BYTES: usize = 8 << 10;

/// Shown when the navigator's selection is a directory with nothing in it.
///
/// A directory now renders as its listing, so this survives only for the case
/// where there is no listing to show. Distinguished from `<directory>`, which
/// this used to be: "I gave you a directory and got nothing" is the one case
/// that could otherwise read as a bug rather than as an answer.
const EMPTY_DIRECTORY_MESSAGE: &str = "<empty directory>";

/// What `read_preview` found: the lines it read, whether more remain, and how
/// many lines the whole file probably has.
struct Preview {
    lines: Vec<String>,
    truncated: bool,
    /// `None` when the file was read whole (the count is not a guess then, it
    /// is `lines.len()`) or when there was nothing to estimate from.
    estimated_lines: Option<usize>,
}

impl Preview {
    /// A preview that is really an error or a placeholder. Not truncated —
    /// there is nothing better to re-read later — and nothing to estimate.
    fn message(text: String) -> Self {
        Self {
            lines: vec![text],
            truncated: false,
            estimated_lines: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct FileView<'a> {
    pub filename: String, // name of the log file to view
    pub textarea: TextArea<'a>,
    pub truncated: bool, // showing a bounded preview rather than the whole file
    /// Roughly how many lines the whole file holds, while only a preview of it
    /// is loaded. `None` once the file is read in full, where the count is not
    /// a guess, and when there was nothing to estimate from.
    ///
    /// `read_preview` has always computed this to size the gutter; keeping it
    /// is what lets the status row report a truthful total, since `truncated`
    /// alone says the document's length is wrong without saying what is right.
    pub estimated_lines: Option<usize>,
    pub active: bool,
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
}

impl FileView<'_> {
    pub fn new(filename: String) -> Self {
        let mut view = Self::default();
        view.load(Path::new(&filename));
        view
    }

    /// Show `path` in the pane, replacing whatever was there.
    ///
    /// A file that cannot be read is reported in the pane itself rather than
    /// bringing the TUI down, since any entry in the nav pane can be selected.
    /// Rebuilding the `TextArea` also resets the cursor and scroll position.
    pub fn load(&mut self, path: &Path) {
        self.filename = path.display().to_string();
        self.textarea = TextArea::new(read_lines(path));
        // This buffer is the file's own lines, not a window onto a visible set,
        // so row 0 of it is row 0 of the document. Leaving a previous file's
        // offset here would misreport the cursor's line until the next
        // `apply_view` — see `cursor_visible_row`.
        self.window_start = 0;
        self.showing_directory = path.is_dir();
        self.truncated = false;
        // The whole file is here, so its length is a fact rather than a guess.
        self.estimated_lines = None;
        // A pending restore was measured against the buffer this just threw
        // away; carrying it into an unrelated file would apply it to the
        // wrong data entirely — see `sync_document`'s clearing of
        // `last_visible` in `lib.rs` for the same reasoning.
        self.pending_screen_row = None;
    }

    /// Show just enough of `path` to fill the pane.
    ///
    /// Used as the selection moves, where reading whole files on every cursor
    /// key would stutter on large logs. While the nav pane holds focus the
    /// view cannot be scrolled, so a screenful is all that can be seen; the
    /// rest is read by `handle_events` as soon as the view is actually used.
    pub fn preview(&mut self, path: &Path) {
        self.preview_with_caps(path, PREVIEW_LINES, MAX_PREVIEW_BYTES);
    }

    /// `preview` with both caps injected, so a test can reach the truncated
    /// branch without building a multi-megabyte fixture. See
    /// `read_preview_with_caps` for why the seam is here.
    fn preview_with_caps(&mut self, path: &Path, max_lines: usize, max_bytes: u64) {
        self.filename = path.display().to_string();
        let preview = read_preview_with_caps(path, max_lines, max_bytes);
        self.showing_directory = path.is_dir();
        self.textarea = TextArea::new(preview.lines);
        // See `load`: a fresh buffer is never a window onto anything.
        self.window_start = 0;
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
    pub fn set_line_styles(&mut self, styles: Vec<Option<Style>>) {
        self.textarea.set_line_styles(styles);
    }

    /// Show these 0-based source line numbers in the gutter instead of
    /// numbering the buffer 1..N.
    ///
    /// Used when the buffer holds only the lines matching a filter, so the
    /// gutter still reads as positions in the original file. Cleared by
    /// `load` and `preview`, as above.
    pub fn set_line_numbers(&mut self, numbers: Vec<usize>) {
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
    pub fn set_group_ends(&mut self, ends: Vec<bool>) {
        self.textarea.set_line_number_styles(
            ends.into_iter()
                .map(|end| end.then(|| Style::default().add_modifier(Modifier::UNDERLINED)))
                .collect(),
        );
    }

    /// Suppress the gutter entirely, for the placeholder row shown when
    /// nothing is visible. See the `gutter_blank` field for why an empty
    /// `set_line_numbers` override is not enough on its own.
    pub fn set_gutter_blank(&mut self, blank: bool) {
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
    pub fn set_highlight(&mut self, pattern: Option<&str>) -> Result<(), regex::Error> {
        self.textarea.set_search_pattern(pattern.unwrap_or(""))
    }

    /// The pattern currently highlighted, if any.
    ///
    /// Test-only. `App::apply_view` writes the highlight and never reads it
    /// back; the one caller is `App::file_view_highlight`, itself a test
    /// helper (#76). Answers the issue's open question for this method: it is
    /// a leftover, not reserved API.
    #[cfg(test)]
    pub fn highlight(&self) -> Option<String> {
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
    pub fn show_lines_with_cursor(&mut self, lines: Vec<String>, row: usize) {
        self.show_window(lines, 0, row);
    }

    /// Replace the buffer with a **window** of the visible set: `lines` are
    /// visible rows `window_start..window_start + lines.len()`, and `row` is
    /// the cursor's row *within that window*.
    ///
    /// `show_lines_with_cursor` is this with a window of zero offset, which is
    /// what an unwindowed document (anything shorter than three screens) always
    /// produces.
    pub fn show_window(&mut self, lines: Vec<String>, window_start: usize, row: usize) {
        // set_lines rejects an empty vector; an empty buffer is one blank line.
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        self.window_start = window_start;
        self.textarea.set_lines(lines, (row, 0));
    }

    /// Visible-set index of the buffer's first row.
    pub fn window_start(&self) -> usize {
        self.window_start
    }

    /// Visible-set index of the buffer's last row, exclusive.
    pub fn window_end(&self) -> usize {
        self.window_start + self.textarea.lines().len()
    }

    /// Where the cursor sits in the **visible set**, translating out of the
    /// window.
    ///
    /// The one place `textarea.cursor().0` is read for a vertical position.
    /// Everything else goes through here, because getting the translation wrong
    /// fails silently — it yields a line number off by `window_start`, which
    /// looks entirely plausible.
    pub fn cursor_visible_row(&self) -> usize {
        self.window_start + self.textarea.cursor().0
    }

    /// The pane height to size a window against: the area last rendered into,
    /// or a generous assumption before the first render.
    pub fn window_height(&self) -> u16 {
        self.last_height.unwrap_or(ASSUMED_PANE_HEIGHT)
    }

    /// Which row of the pane the cursor is currently drawn on.
    ///
    /// Used to hold a line in place across a rebuild: `set_lines` resets the
    /// viewport, so without this the cursor re-anchors to the pane's last row
    /// and the view lurches whenever a filter changes.
    pub fn cursor_screen_row(&self) -> u16 {
        let (top, _) = self.textarea.scroll_top();
        // The subtraction is a screen offset, so it fits `u16` for any pane a
        // terminal can actually draw. `try_from` rather than `as` because
        // nothing in the *type* says so — the cursor is a buffer index, and a
        // cursor left below the viewport by a bug would wrap to a small row
        // under `as` and silently scroll the view somewhere plausible.
        // Saturating turns that into a visibly pinned cursor instead.
        u16::try_from(self.textarea.cursor().0.saturating_sub(usize::from(top))).unwrap_or(u16::MAX)
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
    pub fn scroll_cursor_to_row(&mut self, row: u16) {
        self.pending_screen_row.get_or_insert(row);
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
        let desired_top = cursor.saturating_sub(usize::from(row));
        let (current_top, _) = self.textarea.scroll_top();
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

    /// Put the cursor on `row` of the current buffer, clamped to it.
    ///
    /// Used by `n`/`N`, which decide *which* line to land on in `App` — the
    /// only place that can see both the verdicts and the cursor — and then
    /// ask the view to go there.
    pub fn set_cursor_row(&mut self, row: usize) {
        self.textarea.set_cursor_position((row, 0));
    }

    /// Not a `Result`: every arm is a cursor move or a local toggle, and none
    /// of them can fail (#80). The one genuinely fallible thing this pane does
    /// — `set_highlight` — is called from `App::apply_view`, not from here.
    pub fn handle_events(&mut self, input: Input) {
        // The user is interacting with the view, so a preview is no longer
        // enough: they can now scroll past the end of it.
        if self.truncated {
            let path = Path::new(&self.filename).to_path_buf();
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
            } => self.textarea.scroll((1, 0)),
            Input {
                key: Key::Char('y'),
                ctrl: true,
                ..
            } => self.textarea.scroll((-1, 0)),
            Input {
                key: Key::Char('d'),
                ctrl: true,
                ..
            } => self.textarea.scroll(Scrolling::HalfPageDown),
            Input {
                key: Key::Char('u'),
                ctrl: true,
                ..
            } => self.textarea.scroll(Scrolling::HalfPageUp),
            Input {
                key: Key::Char('b'),
                ctrl: true,
                ..
            }
            | Input {
                key: Key::PageUp, ..
            } => self.textarea.scroll(Scrolling::PageUp),
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
            } => self.textarea.scroll(Scrolling::PageUp),
            Input {
                key: Key::Char(']'),
                ..
            } => self.textarea.scroll(Scrolling::PageDown),
            Input {
                key: Key::Char('f'),
                ctrl: true,
                ..
            }
            | Input {
                key: Key::PageDown, ..
            } => self.textarea.scroll(Scrolling::PageDown),
            _ => (),
        }
    }
}

/// Whether the head of `reader` looks like binary rather than text, along with
/// the bytes that had to be read to decide — they are the file's first bytes
/// and belong back in front of the stream.
///
/// A NUL byte is the signal, not a decode error. A decode error says one byte
/// in the file is not UTF-8, which is routine in a log; a NUL in the first few
/// KiB says the file is not a document at all.
fn sniff_binary<R: Read>(reader: &mut R) -> std::io::Result<(bool, Vec<u8>)> {
    let mut head = Vec::new();
    (&mut *reader)
        .take(BINARY_SNIFF_BYTES as u64)
        .read_to_end(&mut head)?;
    Ok((head.contains(&0), head))
}

/// Read one newline-terminated line, decoded lossily. `None` at end of file.
///
/// Lossy, not fatal: one bad byte in a two-gigabyte log must not cost the
/// other two gigabytes. U+FFFD marks the spot in place and the read carries
/// on, which is the whole difference from `lines()` — that short-circuits the
/// entire file on its first undecodable byte.
fn read_lossy_line<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<Option<String>> {
    buf.clear();
    if reader.read_until(b'\n', buf)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(buf)
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    ))
}

/// Read `path` into lines, or a single-line message describing why it could not
/// be read.
///
/// `File::open` succeeds on a directory on Unix and only fails when read, so
/// that case is recognised up front; anything else the OS refuses is reported
/// verbatim. A file whose head holds a NUL is reported as binary; one that
/// merely holds undecodable bytes is read anyway, a U+FFFD per bad sequence.
fn read_lines(path: &Path) -> Vec<String> {
    // See `read_preview`: a directory opens fine and then fails to read, so
    // it is recognised up front rather than surfacing an OS error string.
    if path.is_dir() {
        return directory_listing(path, usize::MAX).lines;
    }
    // Logged as well as shown (#83). The pane gets `<{err}>` in place of the
    // file, which tells the user *that* it failed; the log is where the
    // full path lives, and the pane's title is elided when the pane is narrow.
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            log::warn!("cannot open {}: {err}", path.display());
            return vec![format!("<{err}>")];
        }
    };

    let mut reader = BufReader::new(file);
    let (binary, head) = match sniff_binary(&mut reader) {
        Ok(sniffed) => sniffed,
        Err(err) => {
            log::warn!("cannot read the start of {}: {err}", path.display());
            return vec![format!("<{err}>")];
        }
    };
    if binary {
        return vec![BINARY_MESSAGE.to_string()];
    }

    // The sniffed bytes are content, so they go back in front of the rest.
    let mut reader = Cursor::new(head).chain(reader);
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    loop {
        match read_lossy_line(&mut reader, &mut buf) {
            Ok(Some(line)) => lines.push(line),
            Ok(None) => break,
            Err(err) => {
                // The line number is worth having: this one fails partway
                // through a file that opened cleanly, so "which line" is the
                // only thing that distinguishes it from the two above.
                log::warn!(
                    "cannot read {} at line {}: {err}",
                    path.display(),
                    lines.len() + 1,
                );
                return vec![format!("<{err}>")];
            }
        }
    }
    lines
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
fn read_preview_with_caps(path: &Path, max_lines: usize, max_bytes: u64) -> Preview {
    // Checked before opening, not after failing to read. `File::open` on a
    // directory *succeeds* on macOS and the read then fails `EISDIR`, so
    // falling through to the error path below would display
    // `<Is a directory (os error 21)>` — platform-specific and meaningless
    // to a reader.
    if path.is_dir() {
        return directory_listing(path, max_lines);
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return Preview::message(format!("<{err}>")),
    };
    // Read before the bytes are consumed; a file that cannot be stat'd simply
    // gets no estimate rather than failing the preview.
    let file_bytes = file.metadata().ok().map(|meta| meta.len());

    let mut reader = BufReader::new(file.take(max_bytes));
    let (binary, head) = match sniff_binary(&mut reader) {
        Ok(sniffed) => sniffed,
        Err(err) => return Preview::message(format!("<{err}>")),
    };
    if binary {
        return Preview::message(BINARY_MESSAGE.to_string());
    }

    // The sniffed bytes are content, so they go back in front of the rest.
    let mut reader = Cursor::new(head).chain(reader);
    let mut lines = Vec::new();
    let mut buf = Vec::new();

    while lines.len() < max_lines {
        match read_lossy_line(&mut reader, &mut buf) {
            Ok(Some(line)) => lines.push(line),
            Ok(None) => break,
            Err(err) => return Preview::message(format!("<{err}>")),
        }
    }

    // Either the line budget ran out, or the byte allowance did. A file that
    // ends exactly on a cap is reported as truncated, which only costs a
    // redundant re-read the first time the view is used.
    let remaining = reader.into_inner().1.into_inner().limit();
    let truncated = lines.len() == max_lines || remaining == 0;
    let estimated_lines = if truncated {
        estimate_lines(file_bytes, max_bytes - remaining, lines.len())
    } else {
        None
    };
    Preview {
        lines,
        truncated,
        estimated_lines,
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
    format!(
        "{:<name_width$}  {:>SIZE_COLUMN$}  {}",
        entry.display(),
        size,
        modified
    )
    .trim_end()
    .to_string()
}

fn directory_listing(path: &Path, max_lines: usize) -> Preview {
    let entries = match crate::widgets::filenav::sorted_entries(path) {
        Ok(entries) => entries,
        // Same shape as an unreadable file: say why, verbatim from the OS.
        Err(err) => return Preview::message(format!("<{err}>")),
    };
    if entries.is_empty() {
        return Preview::message(EMPTY_DIRECTORY_MESSAGE.to_string());
    }
    let total = entries.len();
    let shown = &entries[..entries.len().min(max_lines)];
    // Padded to the longest name actually on screen, so the columns line up
    // without every listing being as wide as the widest possible name.
    let name_width = shown
        .iter()
        .map(|entry| entry.display().chars().count())
        .max()
        .unwrap_or(0)
        .min(NAME_COLUMN_MAX);
    let lines: Vec<String> = shown
        .iter()
        .map(|entry| listing_row(entry, name_width))
        .collect();
    let truncated = total > lines.len();
    Preview {
        lines,
        truncated,
        // Unlike a file, the real count is known exactly rather than scaled
        // from a sample — there is no guessing to do.
        estimated_lines: truncated.then_some(total),
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
        self.textarea.set_block(crate::widgets::pane_block(
            self.filename.clone(),
            self.active,
        ));
        // Apply any scroll requested since the last render — see
        // `scroll_cursor_to_row` and `apply_pending_scroll` — only now, once
        // the block above is set: `apply_pending_scroll`'s scratch render
        // needs to see the same borders the real render below is about to
        // draw, or it computes an inner height that is two rows too tall on
        // the first frame after `load`/`preview` replace the textarea (which
        // drops its block along with everything else).
        self.apply_pending_scroll(area);
        (&self.textarea).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::fs;
    use std::sync::Mutex;

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

    /// Every fixture file name claimed so far in this process. `fixture` and
    /// `byte_fixture` both write directly into `target/test-fixtures/<name>`,
    /// so they share one namespace.
    static FIXTURE_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Panic loudly if `name` has already been used for a fixture file in
    /// this process, instead of letting two tests race to write the same
    /// path — the same class of bug that caused a release-only flake in the
    /// `target/test-appdirs` fixtures (see `lib.rs`'s `claim_fixture_dir`).
    fn claim_fixture_name(name: &str) {
        let mut names = FIXTURE_NAMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !names.iter().any(|used| used == name),
            "fixture file name {name:?} is already in use by another test — pick a unique name"
        );
        names.push(name.to_string());
    }

    /// Write a fixture under `target/` so the tests do not depend on whatever
    /// happens to be in the working tree.
    fn fixture(name: &str, contents: &str) -> std::path::PathBuf {
        claim_fixture_name(name);
        let dir = Path::new("target/test-fixtures");
        fs::create_dir_all(dir).expect("create fixture dir");
        let path = dir.join(name);
        fs::write(&path, contents).expect("write fixture");
        path
    }

    /// Write a fixture of raw bytes, for content that is not valid UTF-8.
    fn byte_fixture(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        claim_fixture_name(name);
        let dir = Path::new("target/test-fixtures");
        fs::create_dir_all(dir).expect("create fixture dir");
        let path = dir.join(name);
        fs::write(&path, bytes).expect("write fixture");
        path
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
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview_with_caps(&path, 10, MAX_PREVIEW_BYTES);

        assert_eq!(view.textarea.lines().len(), 10);
        assert!(
            view.truncated,
            "a capped preview should be marked truncated"
        );
    }

    /// The value the shipped constant actually takes, kept separate from the
    /// mechanism above: a file past `PREVIEW_LINES` still truncates.
    #[test]
    fn the_real_line_cap_still_truncates_a_file_past_it() {
        let path = long_file("past_cap.txt", PREVIEW_LINES + 10);
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(&path);

        assert_eq!(view.textarea.lines().len(), PREVIEW_LINES);
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
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(&path);

        assert_eq!(view.textarea.lines().len(), 10_000);
        assert!(
            !view.truncated,
            "a log-sized file was previewed rather than read whole"
        );
    }

    #[test]
    fn preview_of_a_short_file_is_complete() {
        let path = long_file("short.txt", 3);
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());
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

    #[test]
    fn interacting_with_a_complete_file_changes_nothing() {
        let path = long_file("complete.txt", 3);
        let mut view = FileView::new("Cargo.toml".to_string());
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
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(&path);

        let lines = view.textarea.lines();
        assert_ne!(
            lines[0], BINARY_MESSAGE,
            "a NUL past the sniff window condemned the whole file"
        );
        assert_eq!(lines.len(), 2, "lines lost to a late NUL: {lines:?}");
    }

    #[test]
    fn preview_reports_a_binary_file() {
        let path = byte_fixture("preview_binary.bin", &[0xff, 0xfe, 0x00, 0x80]);
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(&path);

        assert_eq!(contents(&view), "<binary file: contains NUL bytes>");
        assert!(
            !view.truncated,
            "an error message is not a truncated preview"
        );
    }

    #[test]
    fn preview_of_a_missing_file_shows_a_message() {
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let view = FileView::new("Cargo.toml".to_string());
        assert!(contents(&view).contains("tui-textarea-2"));
    }

    #[test]
    fn load_replaces_contents_and_title() {
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(Path::new("src/lib.rs"));

        let text = contents(&view);
        assert!(
            text.contains("pub struct App"),
            "did not load lib.rs:\n{text}"
        );
        assert!(!text.contains("[dependencies]"), "old contents lingered");
        assert!(view.filename.contains("lib.rs"), "title not updated");
    }

    /// A missing file must render a message, not panic the whole TUI.
    #[test]
    fn missing_file_shows_a_message() {
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(Path::new("no/such/file.txt"));

        let text = contents(&view);
        assert!(
            text.starts_with('<') && text.ends_with('>'),
            "not a message: {text}"
        );
        assert!(!text.contains("[dependencies]"), "old contents lingered");
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
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(Path::new("src"));

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
        let path = byte_fixture("load_binary.bin", &[0xff, 0xfe, 0x00, 0x80]);
        let mut view = FileView::new("Cargo.toml".to_string());

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
        claim_fixture_name(name);
        let dir = Path::new("target/test-fixtures").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
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
        let mut view = FileView::new("Cargo.toml".to_string());

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
    #[test]
    fn a_directory_previews_as_its_listing() {
        let dir = dir_fixture("dir_listing", &["alpha.txt", "beta.txt"], &["subdir"]);
        let mut view = FileView::new("Cargo.toml".to_string());

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

    /// A directory with nothing in it is the one case that could read as a
    /// bug rather than an answer, so it says so rather than rendering blank.
    ///
    /// Replaces `a_directory_previews_as_a_directory`, which asserted the
    /// `<directory>` placeholder for *every* directory. That placeholder now
    /// survives only here — a directory with contents renders them.
    #[test]
    fn an_empty_directory_says_so() {
        let dir = dir_fixture("dir_empty", &[], &[]);
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

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
        let mut view = FileView::new("Cargo.toml".to_string());

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
}
