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
///
/// Outranks the kind styles below. A search hit is the transient, task-driven
/// signal, and an entry's kind is still carried by its trailing `/` and its
/// bold — so letting the match win costs nothing that was not said twice over.
const MATCH_STYLE: Style = Style::new().fg(Color::Yellow);

/// Directories: bright blue and bold, and `rebuild_list` adds a trailing `/`.
///
/// Three cues for one fact, deliberately. Navigating is a fast scanning task,
/// and the colour is caught first, the bold survives a theme with weak colour
/// contrast, and the slash survives no colour at all.
///
/// *Bright* blue rather than plain `Blue`, which is the classic
/// unreadable-on-a-dark-background case.
const DIR_STYLE: Style = Style::new()
    .fg(Color::LightBlue)
    .add_modifier(Modifier::BOLD);

/// Files with the executable bit set, following `yazi`'s palette.
///
/// This reports the file's *mode*, not whether it can be viewed — plenty of
/// executable scripts are perfectly readable text. Previewability is answered
/// by the view pane, which reads the actual bytes; a heuristic cue here could
/// only disagree with it.
const EXEC_STYLE: Style = Style::new().fg(Color::Green);

/// What an entry is, which is all the palette encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    Executable,
    Plain,
}

/// One row of the listing: its name, and what it is.
///
/// `name` stays exactly as it is on disk — the trailing `/` on a directory is
/// added when the row is drawn, never stored. Otherwise `selected_path` would
/// have to strip it back off, and a search for `dir$` would fail to match the
/// directory it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
}

impl Entry {
    fn style(&self) -> Style {
        match self.kind {
            Kind::Dir => DIR_STYLE,
            Kind::Executable => EXEC_STYLE,
            // The common case pays for no colour. With directories and
            // executables marked, a plain row is unambiguous by absence, and
            // the terminal's own theme governs the rows there are most of.
            Kind::Plain => Style::new(),
        }
    }

    /// The row as drawn: directories wear a trailing `/`, as `ls -F` does.
    fn display(&self) -> String {
        match self.kind {
            Kind::Dir => format!("{}/", self.name),
            _ => self.name.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct FileNav<'a> {
    pub dir: PathBuf,        // directory currently being listed
    pub entries: Vec<Entry>, // `PARENT` followed by the sorted contents of `dir`
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
                let style = match matcher {
                    Some(pattern) if pattern.is_match(&entry.name) => MATCH_STYLE,
                    _ => entry.style(),
                };
                ListItem::new(entry.display()).style(style)
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
            .find(|&index| matcher.is_match(&self.entries[index].name))?;

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
    /// Measures the row *as drawn*, so a directory's trailing `/` is counted;
    /// sizing from the bare name would clip the slash off the longest one.
    /// Only the two borders are added — there is no selection marker to
    /// reserve room for since #15 removed it (see #19 for its return).
    ///
    /// Measured in `char`s rather than display width, so wide glyphs (CJK,
    /// emoji) are under-measured and their names will clip.
    pub fn preferred_width(&self) -> u16 {
        const BORDERS: usize = 2;

        let longest = self
            .entries
            .iter()
            .map(|entry| entry.display().chars().count())
            .max()
            .unwrap_or(0);
        u16::try_from(longest + BORDERS).unwrap_or(u16::MAX)
    }

    /// The path the cursor is sitting on.
    fn selected_path(&self) -> Option<PathBuf> {
        let selected = self.state.selected()?;
        Some(self.dir.join(&self.entries.get(selected)?.name))
    }

    /// Ask for the highlighted entry to be previewed.
    ///
    /// Directories included. They used to be filtered out here, on the
    /// reasoning that they have nothing to show — but the consequence was
    /// that the view kept displaying the *previous* file, so moving onto a
    /// directory read as though the directory contained that text. The view
    /// renders `<directory>` for them instead, so the pane always describes
    /// what is actually selected.
    fn preview_selection(&self) -> Option<Action> {
        Some(Action::Preview(self.selected_path()?))
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
fn read_dir_entries(dir: &Path) -> Vec<Entry> {
    let mut entries = vec![Entry {
        name: PARENT.to_string(),
        // `..` is a directory and reads as one, rather than being a third
        // kind of thing with its own look.
        kind: Kind::Dir,
    }];

    let Ok(read_dir) = fs::read_dir(dir) else {
        return entries;
    };

    let mut listed: Vec<Entry> = read_dir
        .filter_map(|entry| {
            let entry = entry.ok()?;
            Some(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind: kind_of(&entry),
            })
        })
        .collect();
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    entries.append(&mut listed);
    entries
}

/// Classify one listed entry.
///
/// `file_type()` is answered from the `d_type` that `readdir` already
/// returned on macOS and Linux, so telling a directory from a file costs no
/// extra syscall. The executable bit does — it needs `metadata()`, one
/// `lstat` per entry — which is why this runs once per listing in `set_dir`
/// and is stored on the `Entry`, never recomputed per render.
fn kind_of(entry: &fs::DirEntry) -> Kind {
    match entry.file_type() {
        Ok(file_type) if file_type.is_dir() => Kind::Dir,
        _ if is_executable(entry) => Kind::Executable,
        _ => Kind::Plain,
    }
}

/// Whether the executable bit is set for anyone.
///
/// Unix-only; elsewhere there is no such bit and nothing is ever green.
///
/// Known false positive, which `yazi` shares: FAT and some network mounts
/// report every file as executable, which turns the whole pane green. That is
/// the filesystem talking, not a bug here.
#[cfg(unix)]
fn is_executable(entry: &fs::DirEntry) -> bool {
    use std::os::unix::fs::PermissionsExt;
    entry
        .metadata()
        .is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_entry: &fs::DirEntry) -> bool {
    false
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
            .highlight_style(highlight_style);
        StatefulWidget::render(&list, area, buf, &mut self.state);
    }
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
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} not among {:?}", nav.entries));
        nav.state.select(Some(index));
    }

    /// A fixture with one plain file, one executable file and one directory,
    /// which is the whole matrix the palette distinguishes.
    fn nav_with_kinds(name: &str) -> FileNav<'static> {
        let dir = Path::new("target/test-navdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("subdir")).expect("create subdir");
        fs::write(dir.join("plain.txt"), "x").expect("write plain");
        let script = dir.join("script.sh");
        fs::write(&script, "#!/bin/sh\n").expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        FileNav::new(dir.join("placeholder").display().to_string())
    }

    /// The rendered row for `name`, as cells, so styles can be read off it.
    fn row_cells(nav: &mut FileNav<'_>, name: &str) -> Vec<(String, Style)> {
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        nav.render(area, &mut buf);
        for y in 0..area.height {
            let row: Vec<(String, Style)> = (0..area.width)
                .map(|x| (buf[(x, y)].symbol().to_string(), buf[(x, y)].style()))
                .collect();
            let text: String = row.iter().map(|(sym, _)| sym.as_str()).collect();
            if text.contains(name) {
                return row;
            }
        }
        panic!("no row containing {name:?}");
    }

    fn row_text(nav: &mut FileNav<'_>, name: &str) -> String {
        row_cells(nav, name)
            .iter()
            .map(|(sym, _)| sym.as_str())
            .collect::<String>()
            .trim_matches(|c| c == '\u{2502}' || c == ' ')
            .to_string()
    }

    /// The style on the first cell of `name` itself, past the border and any
    /// indentation, so the assertion reads the name's own styling.
    fn name_style(nav: &mut FileNav<'_>, name: &str) -> Style {
        let cells = row_cells(nav, name);
        let first = name.chars().next().expect("empty name");
        cells
            .iter()
            .find(|(sym, _)| sym.starts_with(first))
            .map(|(_, style)| *style)
            .expect("name not drawn")
    }

    #[test]
    fn directories_get_a_trailing_slash() {
        let mut nav = nav_with_kinds("kinds_slash");

        assert!(
            row_text(&mut nav, "subdir").ends_with("subdir/"),
            "no trailing slash: {:?}",
            row_text(&mut nav, "subdir")
        );
        assert!(
            !row_text(&mut nav, "plain.txt").ends_with('/'),
            "a plain file was given a directory's slash"
        );
    }

    /// Three cues on directories, not one: colour for the eye, bold for a
    /// theme with weak colour, and the slash for no colour at all.
    #[test]
    fn directories_are_bright_blue_and_bold() {
        let mut nav = nav_with_kinds("kinds_dir_style");

        let style = name_style(&mut nav, "subdir");

        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "directories are not bold"
        );
    }

    /// Follows yazi: colour reports the file's *mode*, so green means the
    /// executable bit is set and says nothing about whether it can be viewed.
    #[cfg(unix)]
    #[test]
    fn executable_files_are_green() {
        let mut nav = nav_with_kinds("kinds_exec");

        assert_eq!(name_style(&mut nav, "script.sh").fg, Some(Color::Green));
    }

    /// The common case pays for no colour: with directories and executables
    /// marked, a plain row is unambiguous by absence, and the terminal's own
    /// theme governs the rows there are most of.
    #[test]
    fn ordinary_files_are_left_unstyled() {
        let mut nav = nav_with_kinds("kinds_plain");

        let style = name_style(&mut nav, "plain.txt");

        // A rendered cell carries `Reset` rather than `None`; either way it
        // must not be wearing one of the kind colours.
        assert!(
            matches!(style.fg, None | Some(Color::Reset)),
            "an ordinary file was coloured: {:?}",
            style.fg
        );
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }

    /// `..` is a directory and reads as one, rather than being a third thing.
    #[test]
    fn the_parent_entry_is_styled_as_a_directory() {
        let mut nav = nav_with_kinds("kinds_parent");

        assert_eq!(name_style(&mut nav, PARENT).fg, Some(Color::LightBlue));
        assert!(row_text(&mut nav, PARENT).ends_with("../"));
    }

    /// A search hit outranks the kind colour: it is the transient, task-driven
    /// signal, and the kind is still carried by the slash and the bold.
    #[test]
    fn a_search_match_outranks_the_kind_colour() {
        let mut nav = nav_with_kinds("kinds_search");
        nav.search("subdir", false).expect("valid pattern");

        assert_eq!(name_style(&mut nav, "subdir").fg, Some(Color::Yellow));
    }

    /// The width is measured from the row *as drawn*, so the longest entry
    /// being a directory means its slash has to be counted too — sizing from
    /// the bare name would clip the slash off the one entry that has one.
    #[test]
    fn the_width_counts_a_directory_s_trailing_slash() {
        let dir = Path::new("target/test-navdirs/kinds_width");
        fs::remove_dir_all(dir).ok();
        fs::create_dir_all(dir.join("the_longest_entry_here")).expect("create subdir");
        fs::write(dir.join("short.txt"), "x").expect("write");
        let nav = FileNav::new(dir.join("placeholder").display().to_string());

        assert_eq!(
            nav.preferred_width(),
            "the_longest_entry_here".len() as u16 + 1 + 2,
            "expected the name, its slash, and two borders"
        );
    }

    /// The selection marker is gone; reverse video is what says "selected".
    #[test]
    fn no_selection_marker_is_drawn() {
        let mut nav = nav_with_kinds("kinds_no_marker");
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        nav.render(area, &mut buf);

        let text: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol())
            .collect();

        assert!(!text.contains(">>"), "a selection marker is still drawn");
    }

    /// With the `>>` marker gone, reverse video is the only thing that says
    /// "selected" — and it now has to say it on top of a coloured, bold row.
    ///
    /// Checked because it is the way this change could quietly be worse than
    /// what it replaced. On a selected directory the kind colour becomes the
    /// *background* rather than being lost, and the trailing `/` and the bold
    /// survive either way, which is the whole reason for carrying three cues.
    #[test]
    fn a_selected_directory_is_still_marked_as_selected_and_as_a_directory() {
        let mut nav = nav_with_kinds("kinds_selected_dir");
        select(&mut nav, "subdir");

        let cells = row_cells(&mut nav, "subdir");
        let style = cells
            .iter()
            .find(|(sym, _)| sym == "s")
            .map(|(_, style)| *style)
            .expect("subdir not drawn");

        assert!(
            style.add_modifier.contains(Modifier::REVERSED),
            "the selected row is not drawn as selected"
        );
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "the directory lost its bold when selected"
        );
        assert!(
            row_text(&mut nav, "subdir").ends_with('/'),
            "the directory lost its slash when selected"
        );
    }

    /// A bare filename has no directory component, so the nav pane should fall
    /// back to the current directory rather than listing nothing.
    #[test]
    fn bare_filename_lists_current_directory() {
        let nav = FileNav::new("Cargo.toml".to_string());
        assert!(
            nav.entries.iter().any(|e| e.name == "Cargo.toml"),
            "expected Cargo.toml among entries, got {:?}",
            nav.entries
        );
        assert!(nav.entries.iter().any(|e| e.name == "src"));
    }

    #[test]
    fn parent_entry_comes_first() {
        let nav = FileNav::new("Cargo.toml".to_string());
        assert_eq!(nav.entries.first().map(|e| e.name.as_str()), Some(PARENT));
    }

    #[test]
    fn entries_after_parent_are_sorted() {
        let nav = FileNav::new("Cargo.toml".to_string());
        let rest: Vec<&str> = names(&nav)[1..].to_vec();
        let mut sorted = rest.clone();
        sorted.sort_unstable();
        assert_eq!(rest, sorted);
    }

    #[test]
    fn path_with_directory_lists_that_directory() {
        let nav = FileNav::new("src/lib.rs".to_string());
        assert!(
            nav.entries.iter().any(|e| e.name == "lib.rs"),
            "expected lib.rs among entries, got {:?}",
            nav.entries
        );
        assert!(!nav.entries.iter().any(|e| e.name == "Cargo.toml"));
    }

    /// An unreadable directory still offers `..` so the user can escape.
    #[test]
    fn missing_directory_still_offers_parent() {
        let nav = FileNav::new("no/such/dir/file.txt".to_string());
        assert_eq!(names(&nav), vec![PARENT]);
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
        &nav.entries[nav.state.selected().expect("nothing selected")].name
    }

    /// Entry names in listing order, for assertions that are about the
    /// listing rather than about how a row is drawn.
    fn names<'n>(nav: &'n FileNav<'_>) -> Vec<&'n str> {
        nav.entries.iter().map(|e| e.name.as_str()).collect()
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
            let y = nav.entries.iter().position(|e| e.name == name).unwrap() as u16 + 1;
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

        // Two borders plus the name. The `>>` marker used to add two more.
        assert_eq!(
            nav.preferred_width(),
            "much_longer_name.rs".len() as u16 + 2
        );
    }

    /// `..` is always present, so even an empty directory has a width.
    ///
    /// Two changes land on this number at once: the `>>` marker no longer
    /// reserves two columns, and `..` is a directory so it is drawn `../`,
    /// which claims one back. Net one column narrower than before.
    #[test]
    fn preferred_width_of_an_empty_directory_covers_the_parent_entry() {
        let nav = nav_over("empty", &[]);

        assert_eq!(names(&nav), vec![PARENT]);
        // `../` plus two borders.
        assert_eq!(nav.preferred_width(), PARENT.len() as u16 + 1 + 2);
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
    /// Moving onto a directory asks for it to be previewed, so the view can
    /// say `<directory>`. It used to request nothing, which left the previous
    /// file's text on screen under a directory's name.
    fn moving_onto_a_directory_requests_a_preview() {
        // Own fixture directory: the repo's own listing shifts as files are
        // added, which silently changes which entry follows which.
        let mut nav = nav_over("move_onto_dir", &["alpha.rs"]);
        let dir = Path::new("target/test-navdirs/move_onto_dir");
        fs::create_dir_all(dir.join("beta_dir")).expect("create subdir");
        nav.set_dir(dir.to_path_buf());
        select(&mut nav, "alpha.rs"); // `beta_dir` sorts next

        let action = press(&mut nav, KeyCode::Down);

        assert_eq!(
            selected_name(&nav),
            "beta_dir",
            "moved onto the wrong entry"
        );
        assert!(
            matches!(&action, Some(Action::Preview(path)) if path.ends_with("beta_dir")),
            "expected a preview of the directory, got {action:?}"
        );
    }

    #[test]
    /// `..` is a directory like any other, and previews as one.
    fn moving_onto_the_parent_entry_requests_a_preview() {
        // Own fixture directory: the repo's own listing shifts as files are
        // added, which silently changes which entry follows which.
        let mut nav = nav_over("parent_entry", &["alpha.rs"]);
        select(&mut nav, "alpha.rs"); // `..` is above it

        let action = press(&mut nav, KeyCode::Up);

        assert_eq!(selected_name(&nav), PARENT);
        assert!(
            matches!(action, Some(Action::Preview(_))),
            "expected a preview of the parent directory, got {action:?}"
        );
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
            nav.entries.iter().any(|e| e.name == "lib.rs"),
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
        assert!(nav.entries.iter().any(|e| e.name == "Cargo.toml"));
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

    /// Render the pane and return the selected row.
    ///
    /// Finds it by the reverse-video attribute rather than by a `>>` marker:
    /// the marker is gone, and reverse video is now the only thing that says
    /// "selected", so this probes what actually carries the meaning.
    fn highlighted_row(nav: &mut FileNav<'_>) -> String {
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        nav.render(area, &mut buf);
        (0..area.height)
            .find(|&y| {
                (0..area.width).any(|x| {
                    buf[(x, y)]
                        .style()
                        .add_modifier
                        .contains(Modifier::REVERSED)
                })
            })
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .expect("no row is drawn as selected")
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
