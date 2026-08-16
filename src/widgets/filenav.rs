/// FileNav
///
use crate::widgets::Action;
use color_eyre::Result;
use crossterm::event::{Event, KeyCode};
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, List, ListItem, ListState, StatefulWidget};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

/// The entry that climbs to the parent directory.
pub const PARENT: &str = "..";

/// Applied to entries matching the current search pattern.
const MATCH_STYLE: Style = Style::new().fg(Color::Yellow);

#[derive(Debug, Default)]
pub struct FileNav<'a> {
    pub dir: PathBuf,         // directory currently being listed
    pub entries: Vec<String>, // `PARENT` followed by the sorted contents of `dir`
    pub navlist: List<'a>,
    pub state: ListState,
    pub active: bool,
    /// Current search pattern, matched against entry names.
    matcher: Option<Regex>,
    /// Direction the search was started in, so `n` repeats and `N` reverses.
    search_reverse: bool,
}

impl FileNav<'_> {
    pub fn new(filename: String) -> Self {
        // A bare filename such as `Cargo.toml` has an empty parent, so fall
        // back to the current directory.
        let dir = match Path::new(&filename).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };

        let mut nav = Self::default();
        nav.set_dir(dir);
        nav
    }

    /// Re-list the pane at `dir`, selecting its first entry.
    ///
    /// The path is canonicalized so that walking into a directory and back out
    /// again lands on the original path rather than accumulating `src/..`
    /// segments. Canonicalization fails on a path that does not exist, in
    /// which case the path is kept as-is and the listing comes back empty.
    fn set_dir(&mut self, dir: PathBuf) {
        self.dir = fs::canonicalize(&dir).unwrap_or(dir);
        self.entries = read_dir_entries(&self.dir);
        self.rebuild_list();
        self.state = ListState::default().with_selected(Some(0));
    }

    /// Rebuild the rendered list, styling entries that match the search.
    ///
    /// The selection highlight is applied over the top of this at render time,
    /// so the current match still reads as selected.
    fn rebuild_list(&mut self) {
        let matcher = self.matcher.as_ref();
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| {
                let item = ListItem::new(entry.clone());
                match matcher {
                    Some(pattern) if pattern.is_match(entry) => item.style(MATCH_STYLE),
                    _ => item,
                }
            })
            .collect();
        self.navlist = List::new(items);
    }

    /// Start a search over the entry names, moving to the first match.
    ///
    /// The pattern is a regular expression, matching how the file view
    /// searches, so `^foo` anchors to the start of a name.
    pub fn search(&mut self, pattern: &str, reverse: bool) -> Result<Option<Action>, regex::Error> {
        self.matcher = Some(Regex::new(pattern)?);
        self.search_reverse = reverse;
        self.rebuild_list();
        Ok(self.step_search(reverse))
    }

    /// Repeat the current search: `n` keeps its direction, `N` flips it.
    fn repeat_search(&mut self, opposite: bool) -> Option<Action> {
        self.step_search(self.search_reverse != opposite)
    }

    /// Move the selection to the next matching entry, wrapping around.
    ///
    /// Starts one entry away from the cursor so a repeat always moves, and
    /// checks the current entry last so a lone match holds its place.
    fn step_search(&mut self, reverse: bool) -> Option<Action> {
        let matcher = self.matcher.as_ref()?;
        let count = self.entries.len();
        if count == 0 {
            return None;
        }
        let start = self.state.selected().unwrap_or(0);

        let found = (1..=count)
            .map(|offset| {
                if reverse {
                    (start + count - offset) % count
                } else {
                    (start + offset) % count
                }
            })
            .find(|&index| matcher.is_match(&self.entries[index]))?;

        self.state.select(Some(found));
        self.preview_selection()
    }

    pub fn handle_events(&mut self, event: Event) -> Result<Option<Action>> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.select_previous();
                    return Ok(self.preview_selection());
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.select_next();
                    return Ok(self.preview_selection());
                }
                KeyCode::Enter => return Ok(self.activate_selection()),
                KeyCode::Char('n') => return Ok(self.repeat_search(false)),
                KeyCode::Char('N') => return Ok(self.repeat_search(true)),
                _ => {}
            }
        }

        Ok(None)
    }

    /// Columns needed to show the longest entry in full.
    ///
    /// Counts the two borders and the two-column `>>` marker on top of the
    /// name. Measured in `char`s rather than display width, so wide glyphs
    /// (CJK, emoji) are under-measured and their names will clip.
    pub fn preferred_width(&self) -> u16 {
        const BORDERS_AND_MARKER: usize = 4;

        let longest = self
            .entries
            .iter()
            .map(|entry| entry.chars().count())
            .max()
            .unwrap_or(0);
        u16::try_from(longest + BORDERS_AND_MARKER).unwrap_or(u16::MAX)
    }

    /// The path the cursor is sitting on.
    fn selected_path(&self) -> Option<PathBuf> {
        let selected = self.state.selected()?;
        Some(self.dir.join(self.entries.get(selected)?))
    }

    /// Ask for the highlighted entry to be previewed, if it is a file.
    ///
    /// Directories and `PARENT` have nothing to show, so they yield no action
    /// and the view keeps whatever it was already displaying.
    fn preview_selection(&self) -> Option<Action> {
        let path = self.selected_path()?;
        path.is_file().then_some(Action::Preview(path))
    }

    /// Open the highlighted entry: descend into a directory in place, or ask
    /// for a file to be loaded into the file view in full.
    fn activate_selection(&mut self) -> Option<Action> {
        let path = self.selected_path()?;

        if path.is_dir() {
            self.set_dir(path);
            None
        } else {
            Some(Action::Load(path))
        }
    }

    fn select_previous(&mut self) {
        self.state.select_previous();
    }

    fn select_next(&mut self) {
        self.state.select_next();
    }
}

/// List `dir`, sorted, with `PARENT` first.
///
/// An unreadable directory still yields `PARENT`, so the user can always climb
/// back out rather than being stranded in an empty pane. Nothing here panics.
fn read_dir_entries(dir: &Path) -> Vec<String> {
    let mut entries = vec![PARENT.to_string()];

    let Ok(read_dir) = fs::read_dir(dir) else {
        return entries;
    };

    let mut names: Vec<String> = read_dir
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    names.sort();
    entries.append(&mut names);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn press(nav: &mut FileNav<'_>, code: KeyCode) -> Option<Action> {
        nav.handle_events(Event::Key(KeyEvent::from(code))).unwrap()
    }

    fn enter(nav: &mut FileNav<'_>) -> Option<Action> {
        press(nav, KeyCode::Enter)
    }

    /// Move the selection onto a named entry, so tests don't hard-code indices
    /// that shift as the working tree changes.
    fn select(nav: &mut FileNav<'_>, name: &str) {
        let index = nav
            .entries
            .iter()
            .position(|e| e == name)
            .unwrap_or_else(|| panic!("{name} not among {:?}", nav.entries));
        nav.state.select(Some(index));
    }

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
    fn parent_entry_comes_first() {
        let nav = FileNav::new("Cargo.toml".to_string());
        assert_eq!(nav.entries.first().map(String::as_str), Some(PARENT));
    }

    #[test]
    fn entries_after_parent_are_sorted() {
        let nav = FileNav::new("Cargo.toml".to_string());
        let rest = &nav.entries[1..];
        let mut sorted = rest.to_vec();
        sorted.sort();
        assert_eq!(rest, sorted.as_slice());
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

    /// An unreadable directory still offers `..` so the user can escape.
    #[test]
    fn missing_directory_still_offers_parent() {
        let nav = FileNav::new("no/such/dir/file.txt".to_string());
        assert_eq!(nav.entries, vec![PARENT.to_string()]);
    }

    /// Build a directory with known contents, so width assertions do not
    /// depend on whatever happens to be in the working tree.
    fn nav_over(name: &str, files: &[&str]) -> FileNav<'static> {
        let dir = Path::new("target/test-navdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        for file in files {
            fs::write(dir.join(file), "x").expect("write fixture");
        }
        FileNav::new(dir.join("placeholder").display().to_string())
    }

    /// A nav pane over a known set of names, for search assertions. Each test
    /// gets its own directory, since tests run in parallel.
    fn searchable(name: &str) -> FileNav<'static> {
        nav_over(name, &["alpha.rs", "beta.rs", "beta2.rs", "gamma.rs"])
    }

    fn selected_name<'n>(nav: &'n FileNav<'_>) -> &'n str {
        &nav.entries[nav.state.selected().expect("nothing selected")]
    }

    #[test]
    fn search_selects_a_matching_entry() {
        let mut nav = searchable("search_sel");

        nav.search("beta", false).expect("valid pattern");

        assert_eq!(selected_name(&nav), "beta.rs");
    }

    #[test]
    fn search_is_a_regex() {
        let mut nav = searchable("search_regex");

        nav.search(r"^gam+a\.rs$", false).expect("valid pattern");

        assert_eq!(selected_name(&nav), "gamma.rs");
    }

    /// Jumping to a file previews it, exactly as moving the cursor does.
    #[test]
    fn search_asks_for_the_matched_file_to_be_previewed() {
        let mut nav = searchable("search_preview");

        let action = nav.search("gamma", false).expect("valid pattern");

        match action {
            Some(Action::Preview(path)) => {
                assert_eq!(path.file_name().unwrap(), "gamma.rs");
            }
            other => panic!("expected a Preview action, got {other:?}"),
        }
    }

    #[test]
    fn n_cycles_to_the_next_match() {
        let mut nav = searchable("search_cycle");
        nav.search("beta", false).expect("valid pattern");

        press(&mut nav, KeyCode::Char('n'));
        assert_eq!(selected_name(&nav), "beta2.rs");

        press(&mut nav, KeyCode::Char('N'));
        assert_eq!(selected_name(&nav), "beta.rs");
    }

    #[test]
    fn search_wraps_around_the_listing() {
        let mut nav = searchable("search_wrap");
        nav.search("beta", false).expect("valid pattern");
        press(&mut nav, KeyCode::Char('n')); // beta2.rs, the last match

        press(&mut nav, KeyCode::Char('n'));

        assert_eq!(selected_name(&nav), "beta.rs", "search did not wrap");
    }

    #[test]
    fn a_backward_search_walks_upwards() {
        let mut nav = searchable("search_back");
        select(&mut nav, "gamma.rs");

        nav.search("beta", true).expect("valid pattern");

        assert_eq!(selected_name(&nav), "beta2.rs");
    }

    #[test]
    fn an_invalid_pattern_is_reported() {
        let mut nav = searchable("search_bad");

        assert!(nav.search("[", false).is_err());
    }

    #[test]
    fn a_pattern_matching_nothing_leaves_the_selection_alone() {
        let mut nav = searchable("search_nomatch");
        select(&mut nav, "alpha.rs");

        let action = nav.search("zzz", false).expect("valid pattern");

        assert!(action.is_none());
        assert_eq!(selected_name(&nav), "alpha.rs");
    }

    #[test]
    fn matching_entries_are_highlighted() {
        let mut nav = searchable("search_highlight");
        nav.search("beta", false).expect("valid pattern");
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        nav.render(area, &mut buf);

        let row_style = |name: &str| {
            let y = nav.entries.iter().position(|e| e == name).unwrap() as u16 + 1;
            buf[(4, y)].style()
        };
        assert_ne!(
            row_style("beta2.rs"),
            row_style("alpha.rs"),
            "matching entries are not styled differently"
        );
    }

    #[test]
    fn preferred_width_fits_the_longest_entry() {
        let nav = nav_over("widths", &["a.rs", "much_longer_name.rs"]);

        // Two borders plus the two-column `>>` marker, plus the name itself.
        assert_eq!(nav.preferred_width(), "much_longer_name.rs".len() as u16 + 4);
    }

    /// `..` is always present, so even an empty directory has a width.
    #[test]
    fn preferred_width_of_an_empty_directory_covers_the_parent_entry() {
        let nav = nav_over("empty", &[]);

        assert_eq!(nav.entries, vec![PARENT.to_string()]);
        assert_eq!(nav.preferred_width(), PARENT.len() as u16 + 4);
    }

    #[test]
    fn preferred_width_tracks_the_directory_being_listed() {
        let mut nav = nav_over("outer", &["short.rs"]);
        let narrow = nav.preferred_width();
        fs::create_dir_all("target/test-navdirs/outer/a_much_longer_subdir")
            .expect("create subdir");
        nav.set_dir(Path::new("target/test-navdirs/outer").to_path_buf());

        assert!(
            nav.preferred_width() > narrow,
            "width did not grow for a longer entry"
        );
    }

    /// Moving onto a file previews it, so no keypress beyond the cursor move
    /// is needed to see its contents.
    #[test]
    fn moving_onto_a_file_requests_a_preview() {
        let mut nav = nav_over("move_down", &["alpha.rs", "beta.rs"]);
        select(&mut nav, "alpha.rs");

        match press(&mut nav, KeyCode::Down) {
            Some(Action::Preview(path)) => {
                assert_eq!(path.file_name().unwrap(), "beta.rs");
            }
            other => panic!("expected a Preview action, got {other:?}"),
        }
    }

    #[test]
    fn moving_up_onto_a_file_requests_a_preview() {
        // Own fixture directory: the repo's own listing changes as files are
        // added, which would silently move which entries are adjacent.
        let mut nav = nav_over("move_up", &["alpha.rs", "beta.rs"]);
        select(&mut nav, "beta.rs");

        match press(&mut nav, KeyCode::Up) {
            Some(Action::Preview(path)) => {
                assert_eq!(path.file_name().unwrap(), "alpha.rs");
            }
            other => panic!("expected a Preview action, got {other:?}"),
        }
    }

    /// Directories have nothing to show, so the view keeps the last file.
    #[test]
    fn moving_onto_a_directory_requests_nothing() {
        // Own fixture directory: the repo's own listing shifts as files are
        // added, which silently changes which entry follows which.
        let mut nav = nav_over("move_onto_dir", &["alpha.rs"]);
        let dir = Path::new("target/test-navdirs/move_onto_dir");
        fs::create_dir_all(dir.join("beta_dir")).expect("create subdir");
        nav.set_dir(dir.to_path_buf());
        select(&mut nav, "alpha.rs"); // `beta_dir` sorts next

        assert!(press(&mut nav, KeyCode::Down).is_none());
    }

    #[test]
    fn moving_onto_the_parent_entry_requests_nothing() {
        // Own fixture directory: the repo's own listing shifts as files are
        // added, which silently changes which entry follows which.
        let mut nav = nav_over("parent_entry", &["alpha.rs"]);
        select(&mut nav, "alpha.rs"); // `..` is above it

        assert!(press(&mut nav, KeyCode::Up).is_none());
    }

    #[test]
    fn enter_on_a_file_requests_a_load() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        select(&mut nav, "Cargo.toml");

        let action = enter(&mut nav);

        match action {
            Some(Action::Load(path)) => {
                assert!(path.is_file(), "{path:?} is not a file");
                assert_eq!(path.file_name().unwrap(), "Cargo.toml");
            }
            other => panic!("expected a Load action, got {other:?}"),
        }
    }

    #[test]
    fn enter_on_a_directory_navigates_into_it() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        select(&mut nav, "src");

        let action = enter(&mut nav);

        assert!(action.is_none(), "descending should not request a load");
        assert!(
            nav.entries.iter().any(|e| e == "lib.rs"),
            "did not descend into src, entries: {:?}",
            nav.entries
        );
        assert_eq!(nav.dir.file_name().unwrap(), "src");
    }

    #[test]
    fn descending_resets_the_selection() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        select(&mut nav, "src");
        enter(&mut nav);
        assert_eq!(nav.state.selected(), Some(0));
    }

    #[test]
    fn parent_entry_climbs_back_up() {
        let mut nav = FileNav::new("src/lib.rs".to_string());
        let start = nav.dir.clone();
        select(&mut nav, PARENT);

        let action = enter(&mut nav);

        assert!(action.is_none());
        assert_eq!(nav.dir, start.parent().unwrap());
        assert!(nav.entries.iter().any(|e| e == "Cargo.toml"));
    }

    /// Navigating in and back out should land on the original directory, not
    /// accumulate `./src/..` path segments.
    #[test]
    fn round_trip_returns_to_the_same_directory() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        let start = nav.dir.clone();

        select(&mut nav, "src");
        enter(&mut nav);
        select(&mut nav, PARENT);
        enter(&mut nav);

        assert_eq!(nav.dir, start);
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
        assert!(text.contains(PARENT), "parent entry not drawn:\n{text}");
    }

    /// The block title tracks the directory being listed, so the user can tell
    /// where they have navigated to.
    #[test]
    fn title_shows_the_current_directory() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        // Wide enough that the absolute path is not truncated away.
        let area = Rect::new(0, 0, 120, 10);
        let mut buf = Buffer::empty(area);

        nav.render(area, &mut buf);

        let title: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(
            title.contains(&nav.dir.display().to_string()),
            "title does not show the current directory:\n{title}"
        );
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
            .block(Block::bordered().title(self.dir.display().to_string()))
            .highlight_style(highlight_style)
            .highlight_symbol(">>")
            .repeat_highlight_symbol(true);
        StatefulWidget::render(&list, area, buf, &mut self.state);
    }
}
