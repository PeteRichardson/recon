/// FileView Widget
///
///
use color_eyre::Result;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, Borders};
use std::fs::File;
use std::io::{BufRead, BufReader};
use tui_textarea::{CursorMove, Input, Key, Scrolling, TextArea};

#[derive(Debug, Default)]
pub struct FileView<'a> {
    pub filename: String, // name of the log file to view
    pub textarea: TextArea<'a>,
    pub active: bool,
}

impl FileView<'_> {
    pub fn new(filename: String) -> Self {
        let file = File::open(filename.clone()).expect("no such file");
        let buf = BufReader::new(file);
        let lines: Vec<String> = buf
            .lines()
            .map(|l| l.expect("Could not parse line"))
            .collect();

        Self {
            filename,
            textarea: TextArea::new(lines.clone()),
            active: false,
        }
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
