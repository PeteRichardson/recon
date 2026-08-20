/// FileView Widget
///
///
use color_eyre::Result;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read};
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

/// Shown in place of the contents when a file is not UTF-8.
const BINARY_MESSAGE: &str = "<binary file: not valid UTF-8>";

/// Shown when the navigator's selection is a directory.
///
/// Directories used to raise no action at all, so the pane kept displaying
/// whatever file was there before — which reads as though the directory
/// contains that text. Saying so explicitly is the point: the pane always
/// describes what is selected.
const DIRECTORY_MESSAGE: &str = "<directory>";

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
    /// Direction the current search was started in, so `n` repeats it and `N`
    /// reverses it.
    search_reverse: bool,
    /// Whether the line-number gutter is drawn. Toggled with `#`.
    hide_line_numbers: bool,
    /// Set when the buffer currently holds the single blank placeholder line
    /// substituted by `show_lines_with_cursor` for an empty visible set (e.g.
    /// everything is hidden). An empty `line_numbers` override falls back to
    /// natural 1..N numbering, which would render "1" beside that blank row
    /// and read as "this file has one empty line" — so the gutter is
    /// suppressed outright instead.
    gutter_blank: bool,
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
        self.textarea = TextArea::new(preview.lines);
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

    /// Suppress the gutter entirely, for the placeholder row shown when
    /// nothing is visible. See the `gutter_blank` field for why an empty
    /// `set_line_numbers` override is not enough on its own.
    pub fn set_gutter_blank(&mut self, blank: bool) {
        self.gutter_blank = blank;
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
    pub fn show_lines_with_cursor(&mut self, lines: Vec<String>, row: usize) {
        // set_lines rejects an empty vector; an empty buffer is one blank line.
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        self.textarea.set_lines(lines, (row, 0));
    }

    /// Which row of the pane the cursor is currently drawn on.
    ///
    /// Used to hold a line in place across a rebuild: `set_lines` resets the
    /// viewport, so without this the cursor re-anchors to the pane's last row
    /// and the view lurches whenever a filter changes.
    pub fn cursor_screen_row(&self) -> u16 {
        let (top, _) = self.textarea.scroll_top();
        self.textarea.cursor().0.saturating_sub(top as usize) as u16
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
        let desired_top = cursor.saturating_sub(row as usize);
        let (current_top, _) = self.textarea.scroll_top();
        let delta = desired_top as i64 - current_top as i64;
        if delta != 0 {
            self.textarea
                .scroll((delta.clamp(i16::MIN as i64, i16::MAX as i64) as i16, 0));
        }
    }

    /// Start a search, moving to the first match from the cursor.
    ///
    /// The pattern is a regular expression, so an invalid one is reported
    /// rather than silently matching nothing. The search wraps around the
    /// buffer and every match is highlighted.
    pub fn search(&mut self, pattern: &str, reverse: bool) -> Result<bool, regex::Error> {
        self.textarea.set_search_pattern(pattern)?;
        self.search_reverse = reverse;
        Ok(self.step_search(reverse))
    }

    /// Repeat the current search: `n` keeps its direction, `N` flips it.
    pub fn repeat_search(&mut self, opposite: bool) -> bool {
        self.step_search(self.search_reverse != opposite)
    }

    fn step_search(&mut self, reverse: bool) -> bool {
        if reverse {
            self.textarea.search_back(false)
        } else {
            self.textarea.search_forward(false)
        }
    }

    pub fn handle_events(&mut self, input: Input) -> Result<()> {
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
                key: Key::Char('n'),
                ctrl: false,
                ..
            } => {
                self.repeat_search(false);
            }
            Input {
                key: Key::Char('N'),
                ctrl: false,
                ..
            } => {
                self.repeat_search(true);
            }
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
            // Paired deliberately: Enter under the pinky pages down, space
            // under the thumb pages back up.
            Input {
                key: Key::Enter, ..
            } => self.textarea.scroll(Scrolling::PageDown),
            Input {
                key: Key::Char(' '),
                ..
            } => self.textarea.scroll(Scrolling::PageUp),
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
        Ok(())
    }
}

/// Read `path` into lines, or a single-line message describing why it could not
/// be read.
///
/// `File::open` succeeds on a directory on Unix and only fails when read, so
/// the two error paths are kept distinct: `InvalidData` means the bytes are not
/// UTF-8, anything else is reported verbatim from the OS.
fn read_lines(path: &Path) -> Vec<String> {
    // See `read_preview`: a directory opens fine and then fails to read, so
    // it is recognised up front rather than surfacing an OS error string.
    if path.is_dir() {
        return vec![DIRECTORY_MESSAGE.to_string()];
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return vec![format!("<{err}>")],
    };

    match BufReader::new(file)
        .lines()
        .collect::<std::io::Result<Vec<_>>>()
    {
        Ok(lines) => lines,
        Err(err) if err.kind() == ErrorKind::InvalidData => vec![BINARY_MESSAGE.to_string()],
        Err(err) => vec![format!("<{err}>")],
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
fn read_preview_with_caps(path: &Path, max_lines: usize, max_bytes: u64) -> Preview {
    // Checked before opening, not after failing to read. `File::open` on a
    // directory *succeeds* on macOS and the read then fails `EISDIR`, so
    // falling through to the error path below would display
    // `<Is a directory (os error 21)>` — platform-specific and meaningless
    // to a reader.
    if path.is_dir() {
        return Preview::message(DIRECTORY_MESSAGE.to_string());
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return Preview::message(format!("<{err}>")),
    };
    // Read before the bytes are consumed; a file that cannot be stat'd simply
    // gets no estimate rather than failing the preview.
    let file_bytes = file.metadata().ok().map(|meta| meta.len());

    let mut reader = BufReader::new(file.take(max_bytes));
    let mut lines = Vec::new();
    let mut line = String::new();

    while lines.len() < max_lines {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => lines.push(line.trim_end_matches(['\n', '\r']).to_string()),
            Err(err) if err.kind() == ErrorKind::InvalidData => {
                return Preview::message(BINARY_MESSAGE.to_string());
            }
            Err(err) => return Preview::message(format!("<{err}>")),
        }
    }

    // Either the line budget ran out, or the byte allowance did. A file that
    // ends exactly on a cap is reported as truncated, which only costs a
    // redundant re-read the first time the view is used.
    let remaining = reader.into_inner().limit();
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
        if self.hide_line_numbers || self.gutter_blank {
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
    use std::fs;
    use std::sync::Mutex;

    fn contents(view: &FileView<'_>) -> String {
        view.textarea.lines().join("\n")
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
            .unwrap_or_else(|poison| poison.into_inner());
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
        let body: String = (0..lines).map(|i| format!("line {i}\n")).collect();
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
        })
        .unwrap();
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

    #[test]
    fn search_jumps_to_the_first_match() {
        let mut view = view_of("search.txt", "alpha\nbeta\ngamma\nbeta\n");

        let found = view.search("beta", false).expect("valid pattern");

        assert!(found);
        assert_eq!(view.textarea.cursor().0, 1);
    }

    #[test]
    fn search_supports_regex() {
        let mut view = view_of("search_re.txt", "alpha\nbeta\ngamma\n");

        assert!(view.search("^gam+a$", false).expect("valid pattern"));

        assert_eq!(view.textarea.cursor().0, 2);
    }

    /// `n` repeats in the direction the search started; `N` reverses it.
    #[test]
    fn n_and_shift_n_cycle_matches() {
        let mut view = view_of("search_cycle.txt", "beta\nx\nbeta\ny\nbeta\n");
        view.search("beta", false).expect("valid pattern");
        assert_eq!(
            view.textarea.cursor().0,
            2,
            "search starts after the cursor"
        );

        send(&mut view, Key::Char('n'));
        assert_eq!(view.textarea.cursor().0, 4);

        send(&mut view, Key::Char('N'));
        assert_eq!(view.textarea.cursor().0, 2);
    }

    #[test]
    fn search_wraps_around_the_buffer() {
        let mut view = view_of("search_wrap.txt", "beta\nx\ny\n");
        view.search("beta", false).expect("valid pattern");

        send(&mut view, Key::Char('n'));

        assert_eq!(view.textarea.cursor().0, 0, "search did not wrap");
    }

    #[test]
    fn a_backward_search_walks_upwards() {
        let mut view = view_of("search_back.txt", "beta\nx\nbeta\ny\n");
        view.textarea.move_cursor(CursorMove::Bottom);

        assert!(view.search("beta", true).expect("valid pattern"));

        assert_eq!(view.textarea.cursor().0, 2);
    }

    #[test]
    fn an_invalid_pattern_is_reported() {
        let mut view = view_of("search_bad.txt", "alpha\n");

        assert!(view.search("[", false).is_err());
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
        })
        .unwrap();

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
        })
        .unwrap();

        assert_eq!(view.textarea.lines().len(), 3);
    }

    #[test]
    fn preview_reports_a_binary_file() {
        let path = byte_fixture("preview_binary.bin", &[0xff, 0xfe, 0x00, 0x80]);
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(&path);

        assert_eq!(contents(&view), "<binary file: not valid UTF-8>");
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
    /// when reading, and must not be mistaken for a UTF-8 problem.
    #[test]
    fn directory_shows_a_message() {
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(Path::new("src"));

        let text = contents(&view);
        assert!(
            text.starts_with('<') && text.ends_with('>'),
            "not a message: {text}"
        );
        assert!(
            !text.contains("not valid UTF-8"),
            "directory misreported as binary"
        );
    }

    #[test]
    fn binary_file_is_reported_as_binary() {
        let path = byte_fixture("load_binary.bin", &[0xff, 0xfe, 0x00, 0x80]);
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(&path);

        assert_eq!(contents(&view), "<binary file: not valid UTF-8>");
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

    /// Loading a file rebuilds the TextArea, which drops both. Phase 2 must
    /// re-apply them after every load; this pins the behaviour so that is not
    /// discovered by surprise.
    #[test]
    fn loading_a_file_clears_line_styles_and_numbers() {
        let path = fixture("reload.txt", "alpha\nbeta\n");
        let mut view = view_of("reload_start.txt", "x\n");
        view.set_line_styles(vec![Some(Style::default().fg(Color::Yellow))]);
        view.set_line_numbers(vec![41]);

        view.load(&path);

        assert!(view.textarea.line_styles().is_empty());
        assert!(view.textarea.line_numbers().is_empty());
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

    /// `space` and `Enter` page in opposite directions, so a thumb on the
    /// space bar and a pinky on Enter can scan a file in both directions.
    #[test]
    fn space_pages_up_and_enter_pages_down() {
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut view = view_of("space_pages.txt", &body);
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        (&mut view).render(area, &mut buf);

        send(&mut view, Key::Enter);
        (&mut view).render(area, &mut buf);
        let after_enter = view.textarea.cursor().0;
        assert!(after_enter > 0, "Enter did not page down");

        send(&mut view, Key::Char(' '));
        (&mut view).render(area, &mut buf);

        assert!(
            view.textarea.cursor().0 < after_enter,
            "space did not page back up"
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
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
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
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
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
        let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
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

    /// Selecting a directory in the navigator used to leave the previous
    /// file's text on screen, which reads as though the directory contains it.
    #[test]
    fn a_directory_previews_as_a_directory() {
        let path = fixture("dir_preview_file.txt", "alpha\nbeta\n");
        let mut view = FileView::new(path.display().to_string());
        assert!(contents(&view).contains("alpha"), "precondition");

        view.preview(path.parent().expect("fixture has a parent"));

        assert_eq!(contents(&view), DIRECTORY_MESSAGE);
        assert!(!view.truncated, "a directory is not a truncated preview");
    }

    /// `load` is only reached for files today — the navigator descends into a
    /// directory rather than loading it — but it must not be the one place
    /// that leaks a raw OS error if that ever changes.
    #[test]
    fn loading_a_directory_reports_it_the_same_way() {
        let path = fixture("dir_load_file.txt", "alpha\n");
        let mut view = FileView::new(path.display().to_string());

        view.load(path.parent().expect("fixture has a parent"));

        assert_eq!(contents(&view), DIRECTORY_MESSAGE);
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
        let body: String = (0..5000)
            .map(|i| format!("line {i} {}\n", "x".repeat(200)))
            .collect();
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
