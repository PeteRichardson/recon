/// FileView Widget
///
///
use color_eyre::Result;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, Borders};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::Path;
use tui_textarea::{CursorMove, Input, Key, Scrolling, TextArea};

/// Lines read for a preview: comfortably more than any real terminal height,
/// so the pane is always filled without reading a whole log file.
const PREVIEW_LINES: usize = 500;

/// Byte ceiling for a preview. A file with no newlines is a single enormous
/// line, which the line cap alone would happily read in full.
const MAX_PREVIEW_BYTES: u64 = 1 << 20;

/// Shown in place of the contents when a file is not UTF-8.
const BINARY_MESSAGE: &str = "<binary file: not valid UTF-8>";

#[derive(Debug, Default)]
pub struct FileView<'a> {
    pub filename: String, // name of the log file to view
    pub textarea: TextArea<'a>,
    pub truncated: bool, // showing a bounded preview rather than the whole file
    pub active: bool,
    /// Direction the current search was started in, so `n` repeats it and `N`
    /// reverses it.
    search_reverse: bool,
    /// Whether the line-number gutter is drawn. Toggled with `#`.
    hide_line_numbers: bool,
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
    }

    /// Show just enough of `path` to fill the pane.
    ///
    /// Used as the selection moves, where reading whole files on every cursor
    /// key would stutter on large logs. While the nav pane holds focus the
    /// view cannot be scrolled, so a screenful is all that can be seen; the
    /// rest is read by `handle_events` as soon as the view is actually used.
    pub fn preview(&mut self, path: &Path) {
        self.filename = path.display().to_string();
        let (lines, truncated) = read_preview(path);
        self.textarea = TextArea::new(lines);
        self.truncated = truncated;
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
                key: Key::Char('b'),
                ctrl: false,
                ..
            } => self.textarea.move_cursor(CursorMove::WordBack),
            Input {
                key: Key::Char('^'),
                ..
            }
            | Input {
                key: Key::Char('0'),
                ..
            } => self.textarea.move_cursor(CursorMove::Head),
            Input {
                key: Key::Char('e'),
                ctrl: false,
                ..
            } => self.textarea.move_cursor(CursorMove::WordEnd),
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
            Input {
                key: Key::Char(' '),
                ..
            }
            | Input {
                key: Key::Enter, ..
            } => self.textarea.scroll(Scrolling::PageDown),
            // Input {
            //     key: Key::Char(' '),
            //     shift: true,
            //     ..
            // }
            // | Input {
            //     key: Key::Enter,
            //     shift: true,
            //     ..
            // } => textarea.scroll(Scrolling::PageUp),
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
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return vec![format!("<{err}>")],
    };

    match BufReader::new(file).lines().collect::<std::io::Result<Vec<_>>>() {
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
fn read_preview(path: &Path) -> (Vec<String>, bool) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return (vec![format!("<{err}>")], false),
    };

    let mut reader = BufReader::new(file.take(MAX_PREVIEW_BYTES));
    let mut lines = Vec::new();
    let mut line = String::new();

    while lines.len() < PREVIEW_LINES {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => lines.push(line.trim_end_matches(['\n', '\r']).to_string()),
            Err(err) if err.kind() == ErrorKind::InvalidData => {
                return (vec![BINARY_MESSAGE.to_string()], false)
            }
            Err(err) => return (vec![format!("<{err}>")], false),
        }
    }

    // Either the line budget ran out, or the byte allowance did. A file that
    // ends exactly on a cap is reported as truncated, which only costs a
    // redundant re-read the first time the view is used.
    let truncated = lines.len() == PREVIEW_LINES || reader.into_inner().limit() == 0;
    (lines, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn contents(view: &FileView<'_>) -> String {
        view.textarea.lines().join("\n")
    }

    /// Write a fixture under `target/` so the tests do not depend on whatever
    /// happens to be in the working tree.
    fn fixture(name: &str, contents: &str) -> std::path::PathBuf {
        let dir = Path::new("target/test-fixtures");
        fs::create_dir_all(dir).expect("create fixture dir");
        let path = dir.join(name);
        fs::write(&path, contents).expect("write fixture");
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

        assert!(rendered(&mut view).contains("1 alpha"), "gutter did not return");
    }

    #[test]
    fn zero_moves_to_the_start_of_the_line() {
        let mut view = view_of("motions_zero.txt", "hello world\n");
        send(&mut view, Key::Char('$'));
        assert_ne!(view.textarea.cursor().1, 0);

        send(&mut view, Key::Char('0'));

        assert_eq!(view.textarea.cursor(), (0, 0));
    }

    #[test]
    fn e_moves_to_the_end_of_the_word() {
        let mut view = view_of("motions_e.txt", "hello world\n");

        send(&mut view, Key::Char('e'));

        // On the last character of `hello`, not the start of `world`.
        assert_eq!(view.textarea.cursor(), (0, 4));
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
        assert_eq!(view.textarea.cursor().0, 2, "search starts after the cursor");

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

    #[test]
    fn preview_stops_at_the_line_cap() {
        let path = long_file("long.txt", PREVIEW_LINES * 3);
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(&path);

        assert_eq!(view.textarea.lines().len(), PREVIEW_LINES);
        assert!(view.truncated, "a capped preview should be marked truncated");
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
    #[test]
    fn preview_byte_caps_a_file_without_newlines() {
        let blob = "x".repeat((MAX_PREVIEW_BYTES as usize) * 3);
        let path = fixture("blob.txt", &blob);
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(&path);

        let read: usize = view.textarea.lines().iter().map(String::len).sum();
        assert!(
            read as u64 <= MAX_PREVIEW_BYTES,
            "read {read} bytes, over the {MAX_PREVIEW_BYTES} cap"
        );
        assert!(view.truncated);
    }

    /// While the nav pane has focus the preview is all that is on screen, but
    /// the moment the view is used it must hold the whole file.
    #[test]
    fn interacting_upgrades_a_truncated_preview() {
        let path = long_file("upgrade.txt", PREVIEW_LINES * 2);
        let mut view = FileView::new("Cargo.toml".to_string());
        view.preview(&path);
        assert!(view.truncated);

        view.handle_events(Input {
            key: Key::Down,
            ..Default::default()
        })
        .unwrap();

        assert_eq!(view.textarea.lines().len(), PREVIEW_LINES * 2);
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
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(Path::new("target/debug/recon"));

        assert_eq!(contents(&view), "<binary file: not valid UTF-8>");
        assert!(!view.truncated, "an error message is not a truncated preview");
    }

    #[test]
    fn preview_of_a_missing_file_shows_a_message() {
        let mut view = FileView::new("Cargo.toml".to_string());

        view.preview(Path::new("no/such/file.txt"));

        let text = contents(&view);
        assert!(text.starts_with('<') && text.ends_with('>'), "not a message: {text}");
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
        assert!(text.contains("pub struct App"), "did not load lib.rs:\n{text}");
        assert!(!text.contains("[dependencies]"), "old contents lingered");
        assert!(view.filename.contains("lib.rs"), "title not updated");
    }

    /// A missing file must render a message, not panic the whole TUI.
    #[test]
    fn missing_file_shows_a_message() {
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(Path::new("no/such/file.txt"));

        let text = contents(&view);
        assert!(text.starts_with('<') && text.ends_with('>'), "not a message: {text}");
        assert!(!text.contains("[dependencies]"), "old contents lingered");
    }

    /// `File::open` succeeds on a directory on Unix; the failure only surfaces
    /// when reading, and must not be mistaken for a UTF-8 problem.
    #[test]
    fn directory_shows_a_message() {
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(Path::new("src"));

        let text = contents(&view);
        assert!(text.starts_with('<') && text.ends_with('>'), "not a message: {text}");
        assert!(!text.contains("not valid UTF-8"), "directory misreported as binary");
    }

    #[test]
    fn binary_file_is_reported_as_binary() {
        let mut view = FileView::new("Cargo.toml".to_string());

        view.load(Path::new("target/debug/recon"));

        assert_eq!(contents(&view), "<binary file: not valid UTF-8>");
    }
}

/// Widget impl for `FileView`
impl Widget for &mut FileView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.hide_line_numbers {
            self.textarea.remove_line_number();
        } else {
            self.textarea
                .set_line_number_style(Style::default().fg(Color::DarkGray));
        }
        let mut style = Style::default();
        if self.active {
            style = style.fg(Color::Green).add_modifier(Modifier::REVERSED);
        }
        self.textarea.set_cursor_line_style(style);
        self.textarea
            .set_search_style(Style::default().fg(Color::Black).bg(Color::Yellow));
        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(self.filename.clone()),
        );
        (&self.textarea).render(area, buf);
    }
}
