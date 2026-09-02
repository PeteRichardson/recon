/// `FileNav`
///
use crate::filter::DIM_STYLE;
use crate::widgets::Action;
use color_eyre::Result;
use crossterm::event::{Event, KeyCode};
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{List, ListItem, ListState, StatefulWidget};
use regex::Regex;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;

/// The entry that climbs to the parent directory.
pub const PARENT: &str = "..";

/// Where the cursor lands after the listing is rebuilt.
///
/// `set_dir` used to hard-code index 0 — `..`, the way back *out* — which is
/// rarely what was wanted. Every caller now says what it is looking for.
enum Select {
    /// The first entry after `..`, falling back to `..` in a directory that
    /// has nothing else.
    ///
    /// Deliberately the first entry of *any* kind, not the first file.
    /// Skipping over directories to find a file would land the cursor
    /// somewhere different in every directory, depending on where its files
    /// happen to sort; a directory of twenty folders and one file would put
    /// it near the bottom. Starting at the top every time lets the
    /// alphabetical order do the work.
    First,
    /// A named entry, falling back to `First` when it is not there — the
    /// file was deleted, or the climb reached the filesystem root.
    ///
    /// The name as the OS gives it, for the same reason `Entry::name` is —
    /// climbing out of a directory whose name is not valid UTF-8 has to match
    /// that directory in the parent's listing, and a lossy name matches
    /// nothing.
    Named(OsString),
}

/// Applied to entries matching the current search pattern.
///
/// Outranks the kind styles below. A search hit is the transient, task-driven
/// signal, and an entry's kind is still carried by its trailing `/` and its
/// bold — so letting the match win costs nothing that was not said twice over.
/// Columns of chrome around the listing. There is no selection marker to
/// reserve room for since #15 removed it (see #19 for its return).
const BORDERS: usize = 2;

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

/// The `..` row: chrome, not content.
///
/// Reuses `filter.rs`'s `DIM_STYLE` — the same grey that already says "not what
/// you are looking for" on unmatched lines and on disabled filters — rather
/// than picking a second shade of grey that would drift from it.
///
/// `..` *is* a directory, and drawing it as one is defensible; but it is the
/// single row in every listing that is never the thing being looked for, and
/// bright blue and bold made it the loudest row on screen. Dimming leaves it
/// discoverable — it is still the only visible way up, and still the escape
/// hatch from a directory that would otherwise render as an empty box — while
/// letting the eye skip it.
const PARENT_STYLE: Style = DIM_STYLE;

/// What an entry is, which is all the palette encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    Executable,
    Plain,
    /// The `..` row. A directory, but never one you are looking *for* — see
    /// `PARENT_STYLE`.
    ///
    /// Its own variant rather than a `name == PARENT` test inside `style()`,
    /// because a real directory genuinely called `..` cannot exist, and every
    /// place that cares about the distinction (`style`, `display`,
    /// `activate_selection`) then asks the same question the same way.
    Parent,
}

/// One row of the listing: its name, and what it is.
///
/// `name` stays exactly as it is on disk — the trailing `/` on a directory is
/// added when the row is drawn, never stored. Otherwise `selected_path` would
/// have to strip it back off, and a search for `dir$` would fail to match the
/// directory it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The name as the OS gives it, not as Rust would like it to be.
    ///
    /// An `OsString` rather than a `String` because on Unix a filename is
    /// bytes, and not necessarily UTF-8. Storing the lossy conversion turned
    /// every invalid byte into U+FFFD, and `selected_path` then joined *that*
    /// back onto the directory — producing a path that does not exist, so
    /// previewing, loading and opening in an editor all silently addressed
    /// nothing while the file sat listed in the pane beside them.
    ///
    /// The lossy conversion happens once, in `display()`, where a replacement
    /// character is the correct and harmless answer.
    pub name: OsString,
    pub kind: Kind,
    /// Bytes, for a file. `None` for a directory — a `stat` there reports the
    /// size of the directory *file*, not of what is in it, so showing it
    /// beside real file sizes would be a wrong answer rather than a blank.
    /// Also `None` when the entry could not be stat'd at all.
    pub size: Option<u64>,
    /// Last modification time, or `None` when the entry could not be stat'd.
    ///
    /// Kept as a `SystemTime` rather than a formatted string: formatting is a
    /// rendering decision (and a width-dependent one), and doing it here would
    /// bake a format into the listing every consumer then has to live with.
    pub modified: Option<std::time::SystemTime>,
}

impl Entry {
    fn style(&self) -> Style {
        match self.kind {
            Kind::Dir => DIR_STYLE,
            Kind::Executable => EXEC_STYLE,
            Kind::Parent => PARENT_STYLE,
            // The common case pays for no colour. With directories and
            // executables marked, a plain row is unambiguous by absence, and
            // the terminal's own theme governs the rows there are most of.
            Kind::Plain => Style::new(),
        }
    }

    /// The row as drawn: directories wear a trailing `/`, as `ls -F` does.
    ///
    /// `..` included. The slash is a type marker rather than emphasis — `../`
    /// is what every other tool writes — and it is the one cue that survives a
    /// terminal with no colour at all, which is exactly the case where the
    /// dimming says nothing.
    ///
    /// The one place the name is converted lossily. A name that is not valid
    /// UTF-8 cannot be drawn as it is, and U+FFFD is what every other tool
    /// shows for it — but nothing that has to *find* the file again reads this.
    pub(crate) fn display(&self) -> String {
        let name = self.name.to_string_lossy();
        match self.kind {
            Kind::Dir | Kind::Parent => format!("{name}/"),
            _ => name.into_owned(),
        }
    }

    /// The name a search pattern is matched against.
    ///
    /// Lossy like `display()`, but without the trailing `/`: the slash is a
    /// drawing decision, and a search for `dir$` has to match the directory it
    /// names. A name that is not valid UTF-8 is matched on its U+FFFD form —
    /// that is what the pane shows, and so what a user can write a pattern
    /// against.
    fn matchable(&self) -> std::borrow::Cow<'_, str> {
        self.name.to_string_lossy()
    }
}

#[derive(Debug, Default)]
pub struct FileNav<'a> {
    pub dir: PathBuf,        // directory currently being listed
    pub entries: Vec<Entry>, // `PARENT` followed by the sorted contents of `dir`
    pub navlist: List<'a>,
    pub state: ListState,
    pub active: bool,
    /// Terminal columns the widest row needs, measured when the listing is
    /// built rather than on every frame.
    ///
    /// `preferred_width` used to call `Entry::display()` for every entry to
    /// find the longest — a heap allocation per entry, at 60 Hz, purely to
    /// re-measure something that can only change when `entries` does (#84).
    /// Kept in step by `rebuild_list`, which is the one place the drawn rows
    /// are built, so this cannot go stale even by a path `set_dir` never takes.
    widest: usize,
    /// Current search pattern, matched against entry names.
    matcher: Option<Regex>,
    /// Direction the search was started in, so `n` repeats and `N` reverses.
    search_reverse: bool,
}

impl FileNav<'_> {
    /// Open the pane on `path`, which may name either a file or a directory.
    ///
    /// A directory is listed itself, with the cursor on its first entry. A
    /// file has its parent listed, with the cursor on the file — which is
    /// already open in the view, so starting on `..` was strictly less
    /// useful.
    pub fn new(path: String) -> Self {
        let path = Path::new(&path);
        let mut nav = Self::default();

        if path.is_dir() {
            nav.set_dir(path.to_path_buf(), Select::First);
            return nav;
        }

        // A bare filename such as `Cargo.toml` has an empty parent, so fall
        // back to the current directory.
        let dir = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let select = path
            .file_name()
            .map_or(Select::First, |name| Select::Named(name.to_owned()));
        nav.set_dir(dir, select);
        nav
    }

    /// Re-list the pane at `dir`, selecting its first entry.
    ///
    /// The path is absolutised and its `.` / `..` segments collapsed, so that
    /// walking into a directory and back out again lands on the original path
    /// rather than accumulating `src/..` segments.
    ///
    /// This used to be `fs::canonicalize`, which did the same two jobs and a
    /// third nobody asked for: it resolved symlinks (#78). `editor::project_root`
    /// and `App::open_in_editor` both deliberately do not, and both say so at
    /// length — so descending into a symlinked directory left the navigator
    /// showing one path and `o` opening another. `lexical_absolute` is now the
    /// single rule all three share; see its doc comment for why the collapse is
    /// lexical rather than filesystem-truthful.
    ///
    /// Unlike canonicalization this cannot fail on a path that does not exist,
    /// so a bad path now lists empty at the path the user actually named,
    /// rather than at whatever the unresolved fallback happened to be.
    fn set_dir(&mut self, dir: PathBuf, select: Select) {
        self.dir = crate::path::lexical_absolute(&dir);
        self.entries = read_dir_entries(&self.dir);
        self.rebuild_list();
        self.state = ListState::default().with_selected(Some(self.index_of(&select)));
    }

    /// Resolve a `Select` against the listing just built.
    ///
    /// `..` is always entry 0, so "the first entry" is index 1 — and index 0
    /// only when there is nothing else, which is the empty-directory case.
    fn index_of(&self, select: &Select) -> usize {
        let first = usize::from(self.entries.len() > 1);
        match select {
            Select::Named(name) => self
                .entries
                .iter()
                .position(|entry| &entry.name == name)
                .unwrap_or(first),
            Select::First => first,
        }
    }

    /// Climb to the parent directory, landing on the directory just left.
    ///
    /// Reads the name to select from `self.dir` *before* moving, which is why
    /// this needs no bookkeeping: the directory being left is the one whose
    /// listing is on screen. (Remembering the cursor in every directory ever
    /// visited, as ranger does, would need a map; this case is free.)
    ///
    /// `None` at the filesystem root, where there is no parent to climb to
    /// and the pane stays exactly as it was.
    fn go_to_parent(&mut self) -> Option<Action> {
        let leaving = self.dir.file_name()?.to_owned();
        let parent = self.dir.parent()?.to_path_buf();
        self.set_dir(parent, Select::Named(leaving));
        self.preview_selection()
    }

    /// Rebuild the rendered list, styling entries that match the search.
    ///
    /// The selection highlight is applied over the top of this at render time,
    /// so the current match still reads as selected.
    fn rebuild_list(&mut self) {
        let matcher = self.matcher.as_ref();
        // One pass builds the rows and measures them. `display()` allocates,
        // and this is the one place it has to — doing it again per frame in
        // `preferred_width` was #84.
        let mut widest = 0;
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| {
                let style = match matcher {
                    Some(pattern) if pattern.is_match(&entry.matchable()) => MATCH_STYLE,
                    _ => entry.style(),
                };
                let text = entry.display();
                widest = widest.max(UnicodeWidthStr::width(text.as_str()));
                ListItem::new(text).style(style)
            })
            .collect();
        self.widest = widest;
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
            .find(|&index| matcher.is_match(&self.entries[index].matchable()))?;

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
                // `h`/`l` act on the pane, mirroring the movement they mean
                // in a file manager. `l` is `Enter` in every case, including
                // on a file: a key that works on some rows and silently does
                // nothing on others is worse than one that always does the
                // obvious thing.
                KeyCode::Left | KeyCode::Char('h') => return Ok(self.go_to_parent()),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    return Ok(self.activate_selection());
                }
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
    /// Measured in terminal columns, not `char`s. A CJK ideograph or an emoji
    /// occupies two columns, so counting chars sized the pane to about half the
    /// width it needed and clipped every row (#97).
    ///
    /// A field read. `App::nav_width` calls this from inside `App::render`, so
    /// the old version allocated a `String` per directory entry on every frame
    /// to re-measure a listing that had not changed (#84).
    pub fn preferred_width(&self) -> u16 {
        u16::try_from(self.widest + BORDERS).unwrap_or(u16::MAX)
    }

    /// The path the cursor is sitting on.
    ///
    /// `pub(crate)` for `App::new`, which builds the file view from whatever
    /// the navigator selected rather than from the command-line argument.
    pub(crate) fn selected_path(&self) -> Option<PathBuf> {
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
        // `..` opens like a directory, but descending into it means climbing
        // *out* — and the cursor should land on the directory being left, not
        // on that directory's own first entry.
        //
        // Asks the kind rather than comparing the name: a real entry called
        // `..` cannot exist, so the two agree, but the kind is the field that
        // actually carries the distinction.
        if self.entries.get(self.state.selected()?)?.kind == Kind::Parent {
            return self.go_to_parent();
        }

        let path = self.selected_path()?;

        if path.is_dir() {
            self.set_dir(path, Select::First);
            // Preview what was just selected. Descending used to raise no
            // action at all, so the view kept showing the file from the
            // directory you had just left.
            self.preview_selection()
        } else {
            Some(Action::Load(path))
        }
    }

    fn select_previous(&mut self) {
        self.state.select_previous();
    }

    /// Move down one, stopping at the last entry.
    ///
    /// Clamps explicitly rather than using `ListState::select_next`, which
    /// increments without knowing the list length: at the bottom it moved the
    /// selection *past* the last entry, where `selected_path` returns `None`
    /// and previewing silently stopped until you pressed `k`. Rendering hid
    /// it, because `List` clamps the highlight for drawing. `FilterList`
    /// already clamps the same way.
    fn select_next(&mut self) {
        let last = self.entries.len().saturating_sub(1);
        let next = self
            .state
            .selected()
            .map_or(0, |index| (index + 1).min(last));
        self.state.select(Some(next));
    }
}

/// List `dir`, sorted, with `PARENT` first.
///
/// An unreadable directory still yields `PARENT`, so the user can always climb
/// back out rather than being stranded in an empty pane. Nothing here panics.
fn read_dir_entries(dir: &Path) -> Vec<Entry> {
    let mut entries = vec![Entry {
        name: PARENT.into(),
        // Its own kind, not `Kind::Dir`. It behaves like a directory in every
        // way that matters to navigation, but it is drawn as chrome — see
        // `PARENT_STYLE`.
        kind: Kind::Parent,
        // Not stat'd: `..` is a way out of this listing rather than an entry
        // in it, and the navigator shows neither field anyway.
        size: None,
        modified: None,
    }];

    // A directory that cannot be read still offers the way out: `..` alone,
    // rather than an empty box that reads as "recon is broken". The error is
    // still swallowed on purpose — but it is logged on the way past (#83),
    // because on screen an unreadable directory and a genuinely empty one are
    // the same single `..` row.
    match sorted_entries(dir) {
        Ok(listed) => entries.extend(listed),
        Err(err) => log::warn!("cannot list {}: {err}", dir.display()),
    }
    entries
}

/// The directory's own contents, sorted, without the `..` that
/// `read_dir_entries` prepends.
///
/// Split out for the file view, which lists a directory as a look-ahead when
/// one is selected. `..` is deliberately absent there: it is the navigator's
/// way back out, and nothing in a look-ahead can act on it.
///
/// Returns the error rather than swallowing it, so the view can say *why* a
/// directory is unreadable. The navigator discards it — see above.
pub(crate) fn sorted_entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut listed: Vec<Entry> = fs::read_dir(dir)?
        .filter_map(|entry| Some(describe(&entry.ok()?)))
        .collect();
    // `sort_by_cached_key` rather than `sort_by`: the key allocates, and this
    // computes it once per entry instead of twice per comparison.
    listed.sort_by_cached_key(sort_key);
    Ok(listed)
}

/// Directories first, then case-insensitively by name.
///
/// The order `ls`, Finder and `yazi` all use — the same `yazi` whose palette
/// `EXEC_STYLE` follows. `String`'s `Ord` is bytewise, which put every
/// capitalised name in a block above every lowercase one (`Cargo.toml`,
/// `README.md`, `app.log`, `src`) and interleaved directories among files.
///
/// The raw name comes last so that two names differing only in case have one
/// fixed order rather than inheriting whatever `read_dir` happened to yield.
/// It also keeps names that are not valid UTF-8 ordered by their actual bytes:
/// the lossy fold collapses every invalid byte to the same U+FFFD, so without
/// it a directory of them would have no stable order at all.
///
/// `Kind::Parent` never reaches here — `read_dir_entries` prepends `..` after
/// this has run, which is what keeps it pinned to the top rather than sorted
/// among the directories.
fn sort_key(entry: &Entry) -> (bool, String, OsString) {
    (
        entry.kind != Kind::Dir,
        entry.name.to_string_lossy().to_lowercase(),
        entry.name.clone(),
    )
}

/// Describe one listed entry: what it is, how big, and when it changed.
///
/// `file_type()` is answered from the `d_type` that `readdir` already returned
/// on macOS and Linux, so telling a directory from a file costs no extra
/// syscall. Everything else needs `metadata()` — one `lstat` per entry — which
/// is why this runs once per listing in `set_dir` and `sorted_entries`, and is
/// stored on the `Entry`, never recomputed per render.
///
/// That `lstat` happens exactly once and feeds all three fields. It used to be
/// made for the executable bit alone and the `Metadata` thrown away, so size
/// and mtime cost nothing new here — only directories are stat'd where they
/// were not before, which is what gives them an mtime to show.
fn describe(entry: &fs::DirEntry) -> Entry {
    let meta = entry.metadata().ok();
    let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
    let kind = if is_dir {
        Kind::Dir
    } else if meta.as_ref().is_some_and(is_executable) {
        Kind::Executable
    } else {
        Kind::Plain
    };
    Entry {
        // Exactly the bytes `readdir` returned. Converting here is what made a
        // non-UTF-8 name unopenable — see `Entry::name`.
        name: entry.file_name(),
        kind,
        size: if is_dir {
            None
        } else {
            meta.as_ref().map(fs::Metadata::len)
        },
        modified: meta.as_ref().and_then(|meta| meta.modified().ok()),
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
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

impl Widget for &mut FileNav<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut highlight_style = Style::new().add_modifier(Modifier::REVERSED);
        if self.active {
            highlight_style = highlight_style.fg(Color::Green);
        }
        // The block is drawn separately rather than attached to the list, and
        // the list is moved rather than cloned. Both of `Block`- and
        // `highlight_style`-attaching take `self` by value, and the obvious
        // way to satisfy that — `self.navlist.clone()` — deep-copies every
        // `ListItem`, and with it every `Line` and `Span`, once per frame.
        // `App::run` redraws unconditionally at 60 Hz, so a five-thousand
        // entry directory paid for five thousand copies a second while the
        // user sat still (#72). A `Block` is a handful of scalars, so
        // rebuilding it per frame costs nothing; `mem::take` is a pointer
        // swap whatever the entry count.
        let block = crate::widgets::pane_block(self.dir.display().to_string(), self.active);
        let inner = block.inner(area);
        block.render(area, buf);

        let list = std::mem::take(&mut self.navlist).highlight_style(highlight_style);
        StatefulWidget::render(&list, inner, buf, &mut self.state);
        self.navlist = list;
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

    /// `..` reads as chrome, not as content: it is the one row that is never
    /// what you are looking for, so it wears the same grey that already means
    /// "not the thing you want" on unmatched lines and disabled filters.
    ///
    /// It used to be drawn as a directory — bright blue and bold — which put it
    /// in open competition with the actual listing.
    #[test]
    fn the_parent_entry_is_dimmed_as_chrome() {
        let mut nav = nav_with_kinds("kinds_parent");

        let style = name_style(&mut nav, PARENT);

        assert_eq!(style.fg, DIM_STYLE.fg, "`..` is not drawn in the dim grey");
        assert!(
            style.add_modifier.contains(Modifier::DIM),
            "`..` lost the DIM modifier that terminals honouring it use"
        );
        assert!(
            !style.add_modifier.contains(Modifier::BOLD),
            "`..` is still bold, which is the emphasis this removes"
        );
    }

    /// The trailing `/` stays. It is a type marker rather than emphasis —
    /// `../` is what every other tool writes — and it is the cue that survives
    /// a terminal with no colour at all.
    #[test]
    fn the_parent_entry_keeps_its_trailing_slash() {
        let mut nav = nav_with_kinds("kinds_parent_slash");

        assert!(row_text(&mut nav, PARENT).ends_with("../"));
    }

    /// The case this change could quietly break: reverse video is the only
    /// selection cue since #15, and it now has to land on a *dimmed* row.
    ///
    /// Reversing a grey foreground gives a grey background, which still reads
    /// as selected — but a `..` that looked unselected while the cursor was on
    /// it would be a straight regression, and nothing else on screen would say
    /// where the cursor is.
    #[test]
    fn a_selected_parent_entry_still_reads_as_selected() {
        let mut nav = nav_with_kinds("kinds_parent_selected");
        select(&mut nav, PARENT);

        let selected = name_style(&mut nav, PARENT);
        let mut unselected_nav = nav_with_kinds("kinds_parent_unselected");
        select(&mut unselected_nav, "plain.txt");
        let unselected = name_style(&mut unselected_nav, PARENT);

        assert!(
            selected.add_modifier.contains(Modifier::REVERSED),
            "the selected `..` is not drawn as selected"
        );
        assert_ne!(
            selected, unselected,
            "a selected `..` is indistinguishable from an unselected one"
        );
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

    /// A fixture directory with a subdirectory that has its own contents, for
    /// exercising movement in and back out.
    fn nested_fixture(name: &str) -> std::path::PathBuf {
        let dir = Path::new("target/test-navdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("beta_dir")).expect("create subdir");
        fs::write(dir.join("beta_dir/inner_a.txt"), "x").expect("write");
        fs::write(dir.join("beta_dir/inner_b.txt"), "x").expect("write");
        fs::write(dir.join("alpha.txt"), "x").expect("write");
        fs::write(dir.join("gamma.txt"), "x").expect("write");
        dir
    }

    // ---- #21: h / l movement -------------------------------------------

    #[test]
    fn h_and_left_go_to_the_parent_directory() {
        for code in [KeyCode::Char('h'), KeyCode::Left] {
            let dir = nested_fixture("keys_up");
            let mut nav = FileNav::new(dir.join("alpha.txt").display().to_string());
            let start = nav.dir.clone();

            press(&mut nav, code);

            assert_eq!(
                nav.dir,
                start.parent().expect("fixture has a parent"),
                "{code:?} did not climb out"
            );
        }
    }

    /// `h` acts on the pane, not on the selected entry — it climbs out
    /// whether a file, a directory or `..` is under the cursor.
    #[test]
    fn h_goes_up_regardless_of_what_is_selected() {
        let dir = nested_fixture("keys_up_any");
        let mut nav = FileNav::new(dir.join("alpha.txt").display().to_string());
        select(&mut nav, "beta_dir");
        let start = nav.dir.clone();

        press(&mut nav, KeyCode::Char('h'));

        assert_eq!(nav.dir, start.parent().expect("has parent"));
    }

    #[test]
    fn l_and_right_descend_into_the_selected_directory() {
        for code in [KeyCode::Char('l'), KeyCode::Right] {
            let dir = nested_fixture("keys_down");
            let mut nav = FileNav::new(dir.join("alpha.txt").display().to_string());
            select(&mut nav, "beta_dir");

            press(&mut nav, code);

            assert_eq!(nav.dir.file_name().expect("named"), "beta_dir", "{code:?}");
        }
    }

    /// `l` is `Enter`, in every case — including on a file, where both load
    /// it. A key that works on some rows and silently does nothing on others
    /// is worse than one that always does the obvious thing.
    #[test]
    fn l_on_a_file_loads_it_like_enter() {
        let dir = nested_fixture("keys_l_file");
        let mut nav = FileNav::new(dir.join("alpha.txt").display().to_string());
        select(&mut nav, "gamma.txt");

        let action = press(&mut nav, KeyCode::Char('l'));

        assert!(
            matches!(&action, Some(Action::Load(path)) if path.ends_with("gamma.txt")),
            "expected a load, got {action:?}"
        );
    }

    // ---- #21: where the cursor lands ------------------------------------

    /// Entering a directory lands on its first entry, not on `..`. You went
    /// in to get at something inside; `..` is the way back out.
    #[test]
    fn descending_selects_the_first_entry_and_previews_it() {
        let dir = nested_fixture("keys_first_entry");
        let mut nav = FileNav::new(dir.join("alpha.txt").display().to_string());
        select(&mut nav, "beta_dir");

        let action = press(&mut nav, KeyCode::Char('l'));

        assert_eq!(
            selected_name(&nav),
            "inner_a.txt",
            "did not land on the first entry"
        );
        assert!(
            matches!(&action, Some(Action::Preview(path)) if path.ends_with("inner_a.txt")),
            "descending did not preview what it selected, got {action:?}"
        );
    }

    /// "First" means the first entry, file or directory — not the first
    /// *file*. Skipping over directories would put the cursor somewhere
    /// different in every directory depending on where its files sort.
    #[test]
    fn first_entry_means_first_entry_even_when_it_is_a_directory() {
        let dir = Path::new("target/test-navdirs/keys_first_is_dir");
        fs::remove_dir_all(dir).ok();
        fs::create_dir_all(dir.join("outer/aaa_dir")).expect("create");
        fs::write(dir.join("outer/zzz.txt"), "x").expect("write");
        let mut nav = FileNav::new(dir.join("placeholder").display().to_string());
        select(&mut nav, "outer");

        press(&mut nav, KeyCode::Char('l'));

        assert_eq!(
            selected_name(&nav),
            "aaa_dir",
            "skipped the directory to find a file"
        );
    }

    /// An empty directory has nothing but `..`, so that is where the cursor
    /// goes — the fallback, not the default.
    #[test]
    fn descending_into_an_empty_directory_selects_the_parent_entry() {
        let dir = Path::new("target/test-navdirs/keys_empty");
        fs::remove_dir_all(dir).ok();
        fs::create_dir_all(dir.join("hollow")).expect("create");
        let mut nav = FileNav::new(dir.join("placeholder").display().to_string());
        select(&mut nav, "hollow");

        press(&mut nav, KeyCode::Char('l'));

        assert_eq!(selected_name(&nav), PARENT);
    }

    /// Climbing out lands on the directory just left, so going up and into a
    /// sibling does not mean scrolling the whole parent listing.
    #[test]
    fn going_up_selects_the_directory_just_left() {
        let dir = nested_fixture("keys_back_out");
        let mut nav = FileNav::new(dir.join("alpha.txt").display().to_string());
        select(&mut nav, "beta_dir");
        press(&mut nav, KeyCode::Char('l'));
        assert_eq!(
            nav.dir.file_name().expect("named"),
            "beta_dir",
            "precondition"
        );

        let action = press(&mut nav, KeyCode::Char('h'));

        assert_eq!(
            selected_name(&nav),
            "beta_dir",
            "did not land on the directory just left"
        );
        assert!(
            matches!(&action, Some(Action::Preview(path)) if path.ends_with("beta_dir")),
            "going up did not preview what it selected, got {action:?}"
        );
    }

    /// `Enter` on `..` is the same movement as `h`, and lands the same way.
    #[test]
    fn enter_on_the_parent_entry_also_selects_the_directory_just_left() {
        let dir = nested_fixture("keys_enter_parent");
        let mut nav = FileNav::new(dir.join("beta_dir/inner_a.txt").display().to_string());
        select(&mut nav, PARENT);

        enter(&mut nav);

        assert_eq!(selected_name(&nav), "beta_dir");
    }

    // ---- #21 item 5 / #22: what the startup argument selects ------------

    /// Launched with a file, the cursor starts on that file — it is already
    /// open in the view, so starting on `..` was strictly less useful.
    #[test]
    fn a_file_argument_selects_that_file() {
        let dir = nested_fixture("arg_file");
        let nav = FileNav::new(dir.join("gamma.txt").display().to_string());

        assert_eq!(selected_name(&nav), "gamma.txt");
    }

    #[test]
    fn a_directory_argument_lists_that_directory_and_selects_its_first_entry() {
        let dir = nested_fixture("arg_dir");
        let nav = FileNav::new(dir.display().to_string());

        assert_eq!(
            nav.dir.file_name().expect("named"),
            "arg_dir",
            "listed the parent"
        );
        // `beta_dir` rather than `alpha.txt`: directories sort first (#96).
        assert_eq!(selected_name(&nav), "beta_dir");
    }

    /// `j` at the bottom of the listing must stay on the last entry. It used
    /// to run off the end, leaving `selected_path` returning `None` so the
    /// preview quietly stopped updating — invisible, because the rendered
    /// highlight is clamped separately.
    #[test]
    fn select_next_stops_at_the_last_entry() {
        let mut nav = nav_over("clamp_bottom", &["alpha.txt"]);
        let last = nav.entries.len() - 1;
        nav.state.select(Some(last));

        let action = press(&mut nav, KeyCode::Char('j'));

        assert_eq!(nav.state.selected(), Some(last), "ran off the end");
        assert!(
            action.is_some(),
            "preview stopped at the bottom of the list"
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
        assert_eq!(names(&nav).first().map(String::as_str), Some(PARENT));
    }

    /// The view lists a directory with size and modification time, and both
    /// come off the `metadata()` call `kind_of` already made for the
    /// executable bit and then discarded. Fetching them again would be a
    /// second `lstat` per entry for data already in hand.
    ///
    /// Size is `None` for a directory: the number a `stat` reports there is
    /// the size of the directory *file*, not of its contents, and showing it
    /// beside real file sizes would be a wrong answer rather than a missing
    /// one.
    #[test]
    fn entries_carry_size_and_modification_time() {
        let dir = Path::new("target/test-navdirs").join("entry_metadata");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("subdir")).expect("create fixture");
        fs::write(dir.join("alpha.txt"), "12345").expect("write fixture");

        let entries = sorted_entries(&dir).expect("fixture is readable");
        let file = entries
            .iter()
            .find(|e| e.name == "alpha.txt")
            .expect("file listed");
        let subdir = entries
            .iter()
            .find(|e| e.name == "subdir")
            .expect("directory listed");

        assert_eq!(file.size, Some(5), "size not carried on the entry");
        assert!(file.modified.is_some(), "mtime not carried on the entry");
        assert_eq!(subdir.size, None, "a directory reported a content size");
        assert!(
            subdir.modified.is_some(),
            "a directory carries an mtime like anything else"
        );
    }

    /// The real repo root, in the pane's own order — a guard that the listing
    /// is sorted at all, over a directory nobody curated for the test.
    #[test]
    fn entries_after_parent_are_sorted() {
        let nav = FileNav::new("Cargo.toml".to_string());
        let rest = &nav.entries[1..];
        let mut sorted = rest.to_vec();
        sorted.sort_by_cached_key(sort_key);

        assert_eq!(drawn(rest), drawn(&sorted));
    }

    // ---- #96: how the listing is ordered --------------------------------

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.into(),
            kind,
            size: None,
            modified: None,
        }
    }

    /// Entries as drawn, so the assertions below read in the order the user
    /// actually sees and a directory is visibly one.
    fn drawn(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(Entry::display).collect()
    }

    fn listing(nav: &FileNav<'_>) -> Vec<String> {
        drawn(&nav.entries)
    }

    fn sort_fixture(name: &str, dirs: &[&str], files: &[&str]) -> FileNav<'static> {
        let dir = Path::new("target/test-navdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        for sub in dirs {
            fs::create_dir_all(dir.join(sub)).expect("create fixture subdir");
        }
        for file in files {
            fs::write(dir.join(file), "x").expect("write fixture");
        }
        FileNav::new(dir.join("placeholder").display().to_string())
    }

    /// Bytewise order put every capitalised name in a block above every
    /// lowercase one, so `Cargo.toml` and `README.md` sorted above `app.log`
    /// regardless of letter. `ls`, Finder and `yazi` all fold case instead.
    #[test]
    fn entries_sort_case_insensitively() {
        let nav = sort_fixture(
            "sort_case",
            &[],
            &["Cargo.toml", "app.log", "README.md", "beta.rs"],
        );

        assert_eq!(
            listing(&nav),
            ["../", "app.log", "beta.rs", "Cargo.toml", "README.md"]
        );
    }

    /// Directories group at the top rather than interleaving with files, which
    /// is what every other file lister this pane's palette follows does.
    #[test]
    fn directories_sort_before_files() {
        let nav = sort_fixture(
            "sort_dirs_first",
            &["src", "zeta_dir"],
            &["app.log", "beta.rs"],
        );

        assert_eq!(
            listing(&nav),
            ["../", "src/", "zeta_dir/", "app.log", "beta.rs"]
        );
    }

    /// Case folding must not make two names compare equal and let their order
    /// wobble between listings. `sort_by` is stable, but its input — the order
    /// `read_dir` happens to yield — is not.
    ///
    /// Built in memory rather than on disk: the two names this needs are the
    /// same file on a case-insensitive volume, which is the default on macOS.
    #[test]
    fn names_differing_only_in_case_have_a_fixed_order() {
        let mut entries = vec![
            entry("Readme.md", Kind::Plain),
            entry("README.md", Kind::Plain),
        ];

        entries.sort_by_cached_key(sort_key);

        assert_eq!(drawn(&entries), ["README.md", "Readme.md"]);
    }

    /// A tie broken on the raw name, so it is broken the same way every time
    /// regardless of the order `read_dir` yielded.
    #[test]
    fn the_case_tiebreak_does_not_depend_on_the_starting_order() {
        let mut reversed = vec![
            entry("README.md", Kind::Plain),
            entry("Readme.md", Kind::Plain),
        ];

        reversed.sort_by_cached_key(sort_key);

        assert_eq!(drawn(&reversed), ["README.md", "Readme.md"]);
    }

    // ---- #71: names that are not valid UTF-8 -----------------------------

    /// A name that is not valid UTF-8.
    ///
    /// A lone `0xff` can never appear in UTF-8, so `to_string_lossy` replaces
    /// it with U+FFFD — and a path rebuilt from *that* names a file which does
    /// not exist.
    #[cfg(unix)]
    fn non_utf8_name() -> std::ffi::OsString {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(b"broken\xffname.txt".to_vec())
    }

    /// Create the fixture, or `None` on a filesystem that refuses the name.
    ///
    /// APFS and HFS+ enforce valid UTF-8 in filenames and reject this one with
    /// `EILSEQ`, so on macOS there is no such file to list and nothing to
    /// assert. ext4, XFS, tmpfs and every other Unix filesystem take arbitrary
    /// bytes, which is where these two tests actually run.
    #[cfg(unix)]
    fn non_utf8_fixture(name: &str, make: fn(&Path) -> std::io::Result<()>) -> Option<PathBuf> {
        let dir = Path::new("target/test-navdirs").join(name);
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("create fixture dir");
        match make(&dir) {
            Ok(()) => Some(dir),
            Err(err) => {
                eprintln!("skipping {name}: this filesystem rejects non-UTF-8 names ({err})");
                None
            }
        }
    }

    /// The pane listed the file, and then could not open the file it listed:
    /// `selected_path` joined the lossy name back onto the directory, giving a
    /// path with a U+FFFD in it that no `open` will ever find.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_filename_resolves_to_the_file_on_disk() {
        let raw = non_utf8_name();
        let Some(dir) = non_utf8_fixture("non_utf8_name", |dir| {
            fs::write(dir.join(non_utf8_name()), "x")
        }) else {
            return;
        };

        let mut nav = FileNav::new(dir.join("placeholder").display().to_string());
        assert_eq!(
            nav.entries.len(),
            2,
            "expected `..` and the one fixture file"
        );
        nav.state.select(Some(1));

        let path = nav.selected_path().expect("an entry is selected");

        assert_eq!(
            path.file_name(),
            Some(raw.as_os_str()),
            "the name was mangled on the way back to a path"
        );
        assert!(path.is_file(), "{path:?} does not exist on disk");
    }

    /// The same defect in the other direction: climbing out of a directory
    /// selects it by name in the parent's listing, and a lossy name matches
    /// nothing there — so the cursor fell back to the first entry instead.
    #[cfg(unix)]
    #[test]
    fn climbing_out_of_a_non_utf8_directory_lands_back_on_it() {
        let raw = non_utf8_name();
        let Some(dir) = non_utf8_fixture("non_utf8_dir", |dir| {
            fs::create_dir_all(dir.join(non_utf8_name()))?;
            fs::write(dir.join("aaa_first.txt"), "x")
        }) else {
            return;
        };

        let mut nav = FileNav::new(dir.join(&raw).join("inner").display().to_string());
        assert_eq!(
            nav.dir.file_name(),
            Some(raw.as_os_str()),
            "precondition: the pane is inside the odd directory"
        );

        press(&mut nav, KeyCode::Char('h'));

        let landed = nav.selected_path().expect("nothing selected");
        assert_eq!(
            landed.file_name(),
            Some(raw.as_os_str()),
            "did not land on the directory just left"
        );
    }

    /// The join that #71 is about, without a filesystem that has to accept the
    /// name — the defect was in rebuilding the path, not in reading the name.
    #[cfg(unix)]
    #[test]
    fn selected_path_joins_the_name_as_the_os_gave_it() {
        let raw = non_utf8_name();
        let mut nav = FileNav {
            dir: PathBuf::from("/tmp/somewhere"),
            entries: vec![Entry {
                name: raw.clone(),
                kind: Kind::Plain,
                size: None,
                modified: None,
            }],
            ..Default::default()
        };
        nav.state.select(Some(0));

        assert_eq!(
            nav.selected_path(),
            Some(PathBuf::from("/tmp/somewhere").join(&raw))
        );
    }

    /// A name that cannot be rendered still has to be *drawn* — lossily, which
    /// is the one place a replacement character is the right answer.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_name_is_drawn_lossily() {
        let odd = Entry {
            name: non_utf8_name(),
            kind: Kind::Plain,
            size: None,
            modified: None,
        };

        assert_eq!(odd.display(), "broken\u{fffd}name.txt");
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

    fn selected_name(nav: &FileNav<'_>) -> String {
        nav.entries[nav.state.selected().expect("nothing selected")]
            .name
            .to_string_lossy()
            .into_owned()
    }

    /// Entry names in listing order, for assertions that are about the
    /// listing rather than about how a row is drawn.
    fn names(nav: &FileNav<'_>) -> Vec<String> {
        nav.entries
            .iter()
            .map(|e| e.name.to_string_lossy().into_owned())
            .collect()
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

    /// A CJK name occupies two terminal columns per ideograph, so a pane sized
    /// by `char` count comes out about half the width it needs and clips every
    /// row (#97).
    #[test]
    fn preferred_width_counts_display_columns_not_chars() {
        let nav = nav_over("widths_cjk", &["日本語のファイル.rs"]);

        // 8 ideographs at 2 columns each, `の` included, plus `.rs` at 1 each,
        // plus two borders. Counting chars would give 11 + 2.
        assert_eq!(nav.preferred_width(), 16 + 3 + 2);
    }

    /// The cache #84 introduces must not outlive the listing it was measured
    /// from. Recomputing here from `entries` catches staleness on *any*
    /// mutation path, not just the two `set_dir` reaches.
    #[test]
    fn the_cached_width_matches_a_fresh_computation() {
        let mut nav = nav_over("widths_cache", &["a.rs", "much_longer_name.rs"]);

        let fresh = |nav: &FileNav<'_>| {
            nav.entries
                .iter()
                .map(|entry| UnicodeWidthStr::width(entry.display().as_str()))
                .max()
                .unwrap_or(0) as u16
                + 2
        };
        assert_eq!(nav.preferred_width(), fresh(&nav));

        nav.go_to_parent();
        assert_eq!(
            nav.preferred_width(),
            fresh(&nav),
            "climbing to the parent left the width measuring the old listing"
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
        nav.set_dir(
            Path::new("target/test-navdirs/outer").to_path_buf(),
            Select::First,
        );

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
        nav.set_dir(dir.to_path_buf(), Select::First);
        // `beta_dir` sorts directly under `..`, ahead of `alpha.rs`, since
        // directories come first (#96).
        select(&mut nav, PARENT);

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

        assert!(
            matches!(action, Some(Action::Preview(_))),
            "descending should preview what it selected, got {action:?}"
        );
        assert!(
            nav.entries.iter().any(|e| e.name == "lib.rs"),
            "did not descend into src, entries: {:?}",
            nav.entries
        );
        assert_eq!(nav.dir.file_name().unwrap(), "src");
    }

    #[test]
    /// Descending lands on the first entry — index 1, since `..` is 0. It
    /// used to land on `..` itself, which is the way back out.
    fn descending_selects_the_first_entry_not_the_parent() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        select(&mut nav, "src");
        enter(&mut nav);
        assert_eq!(nav.state.selected(), Some(1));
    }

    #[test]
    fn parent_entry_climbs_back_up() {
        let mut nav = FileNav::new("src/lib.rs".to_string());
        let start = nav.dir.clone();
        select(&mut nav, PARENT);

        let action = enter(&mut nav);

        assert!(
            matches!(action, Some(Action::Preview(_))),
            "climbing out should preview what it selected, got {action:?}"
        );
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

    /// Startup lands on the file recon was launched with, not on `..`.
    #[test]
    fn starts_on_the_launched_file() {
        let nav = FileNav::new("Cargo.toml".to_string());
        assert_eq!(selected_name(&nav), "Cargo.toml");
    }

    /// The movement primitives are about moving, so these pin the starting
    /// index rather than inheriting whatever the startup argument selected.
    #[test]
    fn select_next_advances_selection() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        nav.state.select(Some(0));
        nav.select_next();
        assert_eq!(nav.state.selected(), Some(1));
    }

    #[test]
    fn select_previous_clamps_at_first_entry() {
        let mut nav = FileNav::new("Cargo.toml".to_string());
        nav.state.select(Some(0));
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

    /// Over a fixture rather than the working directory: a 20x10 area leaves
    /// eight rows for the listing, so in a directory with more entries than
    /// that the selected file only stays on screen by scrolling `..` off the
    /// top — and this test would fail on the listing's length rather than on
    /// anything it means to assert.
    #[test]
    fn renders_entries_into_the_buffer() {
        let mut nav = nav_over("render_entries", &["alpha.rs", "beta.rs"]);
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        nav.render(area, &mut buf);

        let text: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(text.contains("alpha.rs"), "entries not drawn:\n{text}");
        assert!(text.contains(PARENT), "parent entry not drawn:\n{text}");
    }

    /// `render` moves `navlist` out of the pane to style it and moves it back,
    /// rather than cloning the whole item vector once a frame. That leaves the
    /// pane momentarily holding an empty list, so the second frame is the one
    /// that catches a missing hand-back — the first would draw fine either way.
    #[test]
    fn a_second_render_draws_the_same_entries() {
        let mut nav = nav_over("render_twice", &["alpha.rs", "beta.rs"]);
        let area = Rect::new(0, 0, 20, 10);

        let mut first = Buffer::empty(area);
        nav.render(area, &mut first);
        let mut second = Buffer::empty(area);
        nav.render(area, &mut second);

        let text: String = second
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            text.contains("alpha.rs"),
            "entries lost by frame two:\n{text}"
        );
        assert_eq!(first, second, "consecutive frames differ");
    }

    /// The block title tracks the directory being listed, so the user can tell
    /// where they have navigated to.
    #[test]
    fn title_shows_the_current_directory() {
        let mut nav = nav_over("title_dir", &["alpha.rs"]);
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
        let mut nav = nav_over("highlight_follows", &["alpha.rs", "beta.rs"]);

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
        nav.state.select(Some(0));
        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('j'))))
            .unwrap();
        assert_eq!(nav.state.selected(), Some(1), "j should move down");

        nav.handle_events(Event::Key(KeyEvent::from(KeyCode::Char('k'))))
            .unwrap();
        assert_eq!(nav.state.selected(), Some(0), "k should move back up");
    }

    /// #78: the navigator must show the path the user walked, not wherever a
    /// symlink points. `set_dir` used to call `fs::canonicalize`, which
    /// resolved the link — so `o` (which absolutises without resolving) opened
    /// a path the navigator had never displayed. The two rules disagreed on
    /// the one case both were written to settle.
    #[cfg(unix)]
    #[test]
    fn descending_into_a_symlinked_directory_keeps_the_link_path() {
        let root = Path::new("target/test-navdirs/symlink_descend");
        fs::remove_dir_all(root).ok();
        fs::create_dir_all(root.join("real")).expect("create real");
        fs::write(root.join("real/inside.txt"), "x").expect("write");
        std::os::unix::fs::symlink("real", root.join("link")).expect("symlink");

        let mut nav = FileNav::new(root.join("placeholder").display().to_string());
        select(&mut nav, "link");
        nav.activate_selection();

        assert!(
            nav.dir.ends_with("symlink_descend/link"),
            "the navigator resolved the symlink away: {:?}",
            nav.dir
        );
        assert!(
            names(&nav).iter().any(|n| n == "inside.txt"),
            "sanity: the linked directory's contents should still list"
        );
    }

    /// The other half of #78's fix: dropping `canonicalize` must not cost the
    /// absolutising it was really providing. `Path::parent` on a bare `.` is
    /// `None`, so without it `go_to_parent` cannot climb out of the directory
    /// recon was launched in.
    #[test]
    fn a_relative_start_directory_can_still_be_climbed_out_of() {
        let mut nav = FileNav::new(".".to_string());

        assert!(
            nav.dir.is_absolute(),
            "a relative argument must be absolutised: {:?}",
            nav.dir
        );
        assert!(
            nav.go_to_parent().is_some(),
            "could not climb out of a relative start directory"
        );
    }

    /// `recon ..` should title itself with the parent directory, not with
    /// `<cwd>/..` — and climbing out of it must not land back in the cwd.
    #[test]
    fn a_parent_argument_is_collapsed_rather_than_kept_literally() {
        let nav = FileNav::new("..".to_string());
        let cwd = std::env::current_dir().expect("cwd");

        assert_eq!(nav.dir, cwd.parent().expect("cwd has a parent"));
    }
}
