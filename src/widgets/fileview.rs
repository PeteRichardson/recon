/// FileView Widget
///
///
use color_eyre::Result;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, Borders};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::Path;
use tui_textarea::{CursorMove, Input, Key, Scrolling, TextArea};

#[derive(Debug, Default)]
pub struct FileView<'a> {
    pub filename: String, // name of the log file to view
    pub textarea: TextArea<'a>,
    pub active: bool,
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
    }

    pub fn handle_events(&mut self, input: Input) -> Result<()> {
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
            } => self.textarea.move_cursor(CursorMove::Head),
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
        Err(err) if err.kind() == ErrorKind::InvalidData => {
            vec!["<binary file: not valid UTF-8>".to_string()]
        }
        Err(err) => vec![format!("<{err}>")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents(view: &FileView<'_>) -> String {
        view.textarea.lines().join("\n")
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
        self.textarea
            .set_line_number_style(Style::default().fg(Color::DarkGray));
        let mut style = Style::default();
        if self.active {
            style = style.fg(Color::Green).add_modifier(Modifier::REVERSED);
        }
        self.textarea.set_cursor_line_style(style);
        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(self.filename.clone()),
        );
        (&self.textarea).render(area, buf);
    }
}
