/// FileNav
///
use color_eyre::Result;
use crossterm::event::{Event, KeyCode};
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, List, ListState, StatefulWidget};
use std::fs;
use std::path::Path;

#[derive(Debug, Default)]
pub struct FileNav<'a> {
    pub filename: String, // name of the log file to view
    pub entries: Vec<String>, // names of the files alongside `filename`
    pub navlist: List<'a>,
    pub state: ListState,
    pub active: bool,
}

impl FileNav<'_> {
    pub fn new(filename: String) -> Self {
        let entries = read_dir_entries(&filename);

        Self {
            filename,
            navlist: List::new(entries.clone()),
            entries,
            state: ListState::default().with_selected(Some(0)),
            active: false,
        }
    }

    pub fn handle_events(&mut self, event: Event) -> Result<()> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => self.select_next(),
                _ => {}
            }
        }

        Ok(())
    }

    fn select_previous(&mut self) {
        self.state.select_previous();
    }

    fn select_next(&mut self) {
        self.state.select_next();
    }
}

/// List the names of the files sitting alongside `filename`, sorted.
///
/// A bare filename such as `Cargo.toml` has an empty parent, so fall back to
/// the current directory. An unreadable directory yields no entries rather
/// than panicking, so the nav pane degrades to an empty box.
fn read_dir_entries(filename: &str) -> Vec<String> {
    let dir = match Path::new(filename).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };

    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut entries: Vec<String> = read_dir
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    entries.sort();
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare filename has no directory component, so the nav pane should fall
    /// back to the current directory rather than listing nothing.
    #[test]
    fn bare_filename_lists_current_directory() {
        let nav = FileNav::new("Cargo.toml".to_string());
        assert!(
            nav.entries.iter().any(|e| e == "Cargo.toml"),
            "expected Cargo.toml among entries, got {:?}",
            nav.entries
        );
        assert!(nav.entries.iter().any(|e| e == "src"));
    }

    #[test]
    fn entries_are_sorted() {
        let nav = FileNav::new("Cargo.toml".to_string());
        let mut sorted = nav.entries.clone();
        sorted.sort();
        assert_eq!(nav.entries, sorted);
    }

    #[test]
    fn path_with_directory_lists_that_directory() {
        let nav = FileNav::new("src/lib.rs".to_string());
        assert!(
            nav.entries.iter().any(|e| e == "lib.rs"),
            "expected lib.rs among entries, got {:?}",
            nav.entries
        );
        assert!(!nav.entries.iter().any(|e| e == "Cargo.toml"));
    }

    /// An unreadable directory should render an empty pane, not panic.
    #[test]
    fn missing_directory_yields_no_entries() {
        let nav = FileNav::new("no/such/dir/file.txt".to_string());
        assert!(nav.entries.is_empty());
    }

    #[test]
    fn starts_with_first_entry_selected() {
        let nav = FileNav::new("Cargo.toml".to_string());
        assert_eq!(nav.state.selected(), Some(0));
    }

    #[test]
    fn select_next_advances_selection() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        nav.select_next();
        assert_eq!(nav.state.selected(), Some(1));
    }

    #[test]
    fn select_previous_clamps_at_first_entry() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        nav.select_previous();
        assert_eq!(nav.state.selected(), Some(0));
    }

    /// Render the pane and return the row carrying the `>>` selection marker.
    fn highlighted_row(nav: &mut FileNav<'_>) -> String {
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        nav.render(area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .find(|row| row.contains(">>"))
            .expect("nothing rendered with a selection marker")
    }

    #[test]
    fn renders_entries_into_the_buffer() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        nav.render(area, &mut buf);

        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Cargo.toml"), "entries not drawn:\n{text}");
        assert!(text.contains("List"), "block title not drawn");
    }

    #[test]
    fn rendered_highlight_follows_selection() {
        let mut nav = FileNav::new("Cargo.toml".to_string());

        let first = highlighted_row(&mut nav);
        nav.select_next();
        let second = highlighted_row(&mut nav);

        assert_ne!(
            first, second,
            "selection marker did not move (still on {first})"
        );
    }

    #[test]
    fn j_and_k_move_in_vim_directions() {
        use crossterm::event::{KeyCode, KeyEvent};

        let mut nav = FileNav::new("Cargo.toml".to_string());
        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('j'))))
            .unwrap();
        assert_eq!(nav.state.selected(), Some(1), "j should move down");

        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('k'))))
            .unwrap();
        assert_eq!(nav.state.selected(), Some(0), "k should move back up");
    }
}

impl Widget for &mut FileNav<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut highlight_style = Style::new().add_modifier(Modifier::REVERSED);
        if self.active {
            highlight_style = highlight_style.fg(Color::Green);
        }
        let list = self
            .navlist
            .clone()
            .block(Block::bordered().title("List"))
            .highlight_style(highlight_style)
            .highlight_symbol(">>")
            .repeat_highlight_symbol(true);
        StatefulWidget::render(&list, area, buf, &mut self.state);
    }
}
