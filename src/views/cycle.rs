//! The view cycle: which views a file offers, in which order, and which one
//! `-` has stepped to (#455).
//!
//! One table of rows, the user's `[[filetype]]` rows first and the built-in
//! rows after them. The first row that matches a file gives its whole cycle;
//! rows never merge.

use super::View;
use super::command::{Command, Commands};
use regex::Regex;
use serde::Deserialize;
use std::cell::LazyCell;
use std::path::Path;
use std::sync::Arc;

/// One `[[filetype]]` row of the config. A row sets the cycle, the
/// hand-off, or both. Each one comes from the first row that matches the
/// file *and* sets it (#456), so a row with only a `handoff` leaves the
/// cycle to the rows after it.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(try_from = "RawFileType")]
pub struct FileType {
    /// The names of the files the row is for. With `mime` too, a file must
    /// match both.
    pub glob: Option<Glob>,
    /// The MIME type of the files the row is for, from the magic bytes in
    /// the file's head (#465). The config refuses a type recon cannot find.
    pub mime: Option<String>,
    /// The cycle, default first, by name: `raw`, `hex`, or a name from
    /// `[commands]` (#457). Never empty: the config refuses `[]`.
    pub views: Option<Vec<String>>,
    /// The argv `o` starts for the file, with `{file}` and `{line}`
    /// filled in. Never empty: the config refuses `[]`.
    pub handoff: Option<Vec<String>>,
}

#[cfg(test)]
impl FileType {
    /// A row that sets only the cycle, of built-in views.
    pub(crate) fn with_views(glob: &str, views: &[View]) -> Self {
        let table = Table::default();
        let names: Vec<&str> = views.iter().map(|view| table.view_name(*view)).collect();
        Self::with_view_names(glob, &names)
    }

    /// A row that sets only the cycle, by name.
    pub(crate) fn with_view_names(glob: &str, views: &[&str]) -> Self {
        Self {
            glob: Some(glob.parse().expect("a glob")),
            mime: None,
            views: Some(views.iter().map(ToString::to_string).collect()),
            handoff: None,
        }
    }

    /// This row, matching `mime` and, with no `glob`, any name.
    pub(crate) fn matching_mime(self, glob: Option<&str>, mime: &str) -> Self {
        Self {
            glob: glob.map(|glob| glob.parse().expect("a glob")),
            mime: Some(mime.to_string()),
            ..self
        }
    }

    /// A row that sets only the hand-off.
    pub(crate) fn with_hand_off(glob: &str, argv: &[&str]) -> Self {
        Self {
            glob: Some(glob.parse().expect("a glob")),
            mime: None,
            views: None,
            handoff: Some(argv.iter().map(ToString::to_string).collect()),
        }
    }
}

/// A row as the config writes it, before the checks that make a `FileType`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFileType {
    glob: Option<Glob>,
    mime: Option<String>,
    views: Option<Vec<String>>,
    handoff: Option<Vec<String>>,
}

impl TryFrom<RawFileType> for FileType {
    type Error = String;

    /// `views = []` names no view to show, and `handoff = []` no program to
    /// start, so each is a config error, not a row that matches and then
    /// does nothing. So is a row that sets neither, and a row that sets
    /// no `glob` and no `mime`, or a `mime` that recon cannot find.
    fn try_from(raw: RawFileType) -> Result<Self, Self::Error> {
        let row = Self {
            glob: raw.glob,
            mime: raw.mime,
            views: raw.views,
            handoff: raw.handoff,
        };
        if row.glob.is_none() && row.mime.is_none() {
            return Err("a [[filetype]] row sets no `glob` and no `mime`".to_string());
        }
        if let Some(mime) = &row.mime
            && !crate::mime::is_known(mime)
        {
            return Err(format!(
                "`{mime}` is not a MIME type that recon gives a file"
            ));
        }
        if row.views.as_ref().is_some_and(Vec::is_empty) {
            return Err("`views` names no view".to_string());
        }
        if row.handoff.as_ref().is_some_and(Vec::is_empty) {
            return Err("`handoff` names no program".to_string());
        }
        if row.views.is_none() && row.handoff.is_none() {
            return Err(format!(
                "the row for `{}` sets no `views` and no `handoff`",
                row.label()
            ));
        }
        Ok(row)
    }
}

impl FileType {
    /// The row as an error names it: its glob, else its MIME type.
    #[must_use]
    pub fn label(&self) -> &str {
        self.glob
            .as_ref()
            .map(Glob::text)
            .or(self.mime.as_deref())
            .unwrap_or_default()
    }
}

/// A shell-style glob: `*` and `?` inside one name, `**` across
/// directories, and `{a,b}` for either. Every other character is itself.
///
/// A glob with no `/` matches the file's name, wherever the file is. A
/// glob with a `/` matches the whole path.
#[derive(Debug, Clone)]
pub struct Glob {
    text: String,
    regex: Regex,
    whole_path: bool,
}

impl PartialEq for Glob {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Glob {}

impl std::str::FromStr for Glob {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut pattern = String::from("^");
        let mut braces = 0usize;
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '*' if chars.peek() == Some(&'*') => {
                    chars.next();
                    pattern.push_str(".*");
                }
                '*' => pattern.push_str("[^/]*"),
                '?' => pattern.push_str("[^/]"),
                '{' => {
                    braces += 1;
                    pattern.push_str("(?:");
                }
                '}' if braces > 0 => {
                    braces -= 1;
                    pattern.push(')');
                }
                ',' if braces > 0 => pattern.push('|'),
                _ => pattern.push_str(&regex::escape(ch.encode_utf8(&mut [0; 4]))),
            }
        }
        if braces > 0 {
            return Err(format!("glob `{text}` has a `{{` with no `}}`"));
        }
        pattern.push('$');
        let regex = Regex::new(&pattern).map_err(|err| format!("glob `{text}`: {err}"))?;
        Ok(Self {
            text: text.to_string(),
            regex,
            whole_path: text.contains('/'),
        })
    }
}

impl<'de> Deserialize<'de> for Glob {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

impl Glob {
    /// The glob as the config wrote it.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the glob matches `path`.
    #[must_use]
    pub fn matches(&self, path: &Path) -> bool {
        let subject = if self.whole_path {
            path.to_string_lossy()
        } else {
            path.file_name().unwrap_or_default().to_string_lossy()
        };
        self.regex.is_match(&subject)
    }
}

/// What a file's head says about it: the two things a row can ask that
/// the file's name does not tell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Head {
    /// A NUL in the head: the built-in binary row.
    pub(crate) binary: bool,
    /// The file's MIME type, as `mime::of` gives it (#465, #467). `None`
    /// only when the file could not be read.
    pub(crate) mime: Option<&'static str>,
}

/// What a row matches.
#[derive(Debug, Clone)]
enum Matcher {
    /// A user row: its glob, if it has one, and its MIME type, if it has
    /// one. A file must match each that the row sets.
    User {
        glob: Option<Glob>,
        mime: Option<String>,
    },
    /// A built-in row for the extensions the test accepts, in any case
    /// (#458): a picture, or a type Quick Look shows. Reads nothing: many
    /// a PDF has no NUL in its head, so the binary check would show it as
    /// raw.
    Extension(fn(&str) -> bool),
    /// The built-in row for a file with a NUL in its head.
    Binary,
    /// The built-in row for every other file.
    Any,
}

/// Where `o` sends a file (#456).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HandOff {
    /// A user row's `handoff`: an argv with `{file}` and `{line}` in it.
    Argv(Vec<String>),
    /// The built-in row for text: the editor's file template.
    Editor,
    /// The built-in row for a binary file: the operating system's default
    /// app.
    DefaultApp,
}

/// One row of the table. A user row sets `views`, `handoff` or both; a
/// built-in row sets both, so every file finds each one.
#[derive(Debug, Clone)]
struct Row {
    matcher: Matcher,
    views: Option<Vec<View>>,
    handoff: Option<HandOff>,
}

/// Every row, the user's first. Built once at startup and shared with the
/// workers, which resolve a file's row: only they may read the file's head.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    rows: Vec<Row>,
    /// `[commands]` in name order: `View::Command(i)` is the `i`th.
    commands: Vec<(String, Command)>,
}

impl Default for Table {
    fn default() -> Self {
        Self::build(&[], &Commands::default()).0
    }
}

impl Table {
    /// The user's rows, then the built-in ones, with no commands.
    #[cfg(test)]
    pub(crate) fn new(user: &[FileType]) -> Self {
        Self::build(user, &Commands::default()).0
    }

    /// The user's rows, then the built-in ones, and a warning for each view
    /// a row names that cannot run (#457): a command that is not on `PATH`,
    /// or `quicklook` where there is no Quick Look (#458). Each such view
    /// is left out of its cycle, and a row left with no view gives its
    /// cycle to the rows after it.
    pub(crate) fn build(user: &[FileType], commands: &Commands) -> (Self, Vec<String>) {
        let mut commands: Vec<(String, Command)> = commands
            .0
            .iter()
            .map(|(name, command)| (name.clone(), command.clone()))
            .collect();
        let mut warnings = Vec::new();
        let mut warned = std::collections::HashSet::new();
        let mut warn = |name: &str, warning: String| {
            if warned.insert(name.to_string()) {
                warnings.push(warning);
            }
        };
        let quick_look = Command::quick_look();
        if let Some(command) = &quick_look {
            commands.push((QUICK_LOOK.to_string(), command.clone()));
        }
        let quick_look = quick_look.map(|_| View::Command(commands.len() - 1));
        let mut resolve = |name: &str| -> Option<View> {
            if let Some(view) = View::built_in(name) {
                return Some(view);
            }
            if name == QUICK_LOOK {
                if quick_look.is_none() {
                    warn(
                        name,
                        format!(
                            "view `{name}` needs Quick Look (`qlmanage`), which this system does not have; it is skipped"
                        ),
                    );
                }
                return quick_look;
            }
            // The config refuses a name it does not define, so this is
            // only `None` for a table built by hand.
            let index = commands.iter().position(|(known, _)| known == name)?;
            let command = &commands[index].1;
            let program = &command.argv[0];
            if super::command::on_path(program) {
                return Some(View::Command(index));
            }
            warn(
                name,
                format!("command `{name}`: `{program}` is not on PATH; the view is skipped"),
            );
            None
        };
        let user: Vec<Row> = user
            .iter()
            .map(|row| Row {
                matcher: Matcher::User {
                    glob: row.glob.clone(),
                    mime: row.mime.clone(),
                },
                views: row
                    .views
                    .as_ref()
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(|name| resolve(name))
                            .collect::<Vec<_>>()
                    })
                    .filter(|views| !views.is_empty()),
                handoff: row.handoff.clone().map(HandOff::Argv),
            })
            .collect();
        let built_in = [
            Row {
                matcher: Matcher::Extension(is_picture),
                views: Some(vec![View::Image, View::Hex]),
                handoff: Some(HandOff::DefaultApp),
            },
            Row {
                matcher: Matcher::Extension(quick_look_shows),
                views: quick_look.map(|view| vec![view, View::Hex]),
                handoff: Some(HandOff::DefaultApp),
            },
            Row {
                matcher: Matcher::Binary,
                views: Some(vec![View::Hex]),
                handoff: Some(HandOff::DefaultApp),
            },
            Row {
                matcher: Matcher::Any,
                views: Some(vec![View::Raw, View::Hex]),
                handoff: Some(HandOff::Editor),
            },
        ];
        let table = Self {
            rows: user.into_iter().chain(built_in).collect(),
            commands,
        };
        (table, warnings)
    }

    /// The command behind `view`, for a command view.
    pub(crate) fn command(&self, view: View) -> Option<&Command> {
        match view {
            View::Command(index) => Some(&self.commands[index].1),
            View::Raw | View::Hex | View::Image => None,
        }
    }

    /// The name of `view`, as the config and the status line say it.
    pub(crate) fn view_name(&self, view: View) -> &str {
        match view {
            View::Raw => "raw",
            View::Hex => "hex",
            View::Image => "image",
            View::Command(index) => &self.commands[index].0,
        }
    }

    /// The row whose cycle `path` shows: the first that matches it and
    /// has `views`. `head` is asked only when a mime row or a built-in row
    /// is reached, so a glob row costs no read.
    pub(crate) fn row_for(&self, path: &Path, head: impl FnOnce() -> Head) -> usize {
        self.first(path, head, |row| row.views.is_some())
    }

    /// Where `o` sends `path`: the hand-off of the first row that matches
    /// it and has one (#456). `head` is asked as `row_for` asks it.
    pub(crate) fn hand_off(&self, path: &Path, head: impl FnOnce() -> Head) -> HandOff {
        let row = self.first(path, head, |row| row.handoff.is_some());
        self.rows[row].handoff.clone().unwrap_or(HandOff::Editor)
    }

    /// The first row that matches `path` and that `sets` accepts. Every
    /// built-in row sets every key, so the last row is the answer when no
    /// other is.
    fn first(
        &self,
        path: &Path,
        head: impl FnOnce() -> Head,
        sets: impl Fn(&Row) -> bool,
    ) -> usize {
        // Many rows can ask, but `head` runs at most once.
        let head = LazyCell::new(head);
        self.rows
            .iter()
            .position(|row| {
                sets(row)
                    && match &row.matcher {
                        Matcher::User { glob, mime } => {
                            glob.as_ref().is_none_or(|glob| glob.matches(path))
                                && mime.as_ref().is_none_or(|mime| head.mime == Some(mime))
                        }
                        Matcher::Extension(accepts) => path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(accepts),
                        Matcher::Binary => head.binary,
                        Matcher::Any => true,
                    }
            })
            .unwrap_or(self.rows.len() - 1)
    }

    /// The cycle of `row`: a row from `row_for`, which always has one.
    pub(crate) fn views(&self, row: usize) -> &[View] {
        self.rows[row].views.as_deref().unwrap_or_default()
    }

    fn len(&self) -> usize {
        self.rows.len()
    }
}

/// The name of the built-in Quick Look command, which a row can name.
const QUICK_LOOK: &str = "quicklook";

/// The extensions of the binary types Quick Look shows and the image view
/// does not (#458). Not every binary file: Quick Look does not stop for a
/// file it has no thumbnail for, so each one would wait for the timeout.
const QUICK_LOOK_EXTENSIONS: &[&str] = &[
    "pdf", "ai", "psd", "heic", "heif", "avif", "jxl", "key", "pages", "numbers", "dng", "cr2",
    "cr3", "nef", "arw", "orf", "raf", "rw2", "mov", "mp4", "m4v", "usdz",
];

/// Whether `ext` names a picture the image view decodes, in any case.
fn is_picture(ext: &str) -> bool {
    image::ImageFormat::from_extension(ext).is_some_and(|format| format.reading_enabled())
}

/// Whether `ext` names a type Quick Look shows, in any case.
fn quick_look_shows(ext: &str) -> bool {
    QUICK_LOOK_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(ext))
}

/// How far `-` has stepped in each row's cycle, for this session. Shared
/// with each job as a snapshot: a step taken later does not change a job
/// already sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Steps(Arc<Vec<usize>>);

impl Steps {
    /// Every row at its first view.
    pub(crate) fn new(table: &Table) -> Self {
        Self(Arc::new(vec![0; table.len()]))
    }

    /// The view `row` shows now.
    pub(crate) fn view(&self, table: &Table, row: usize) -> View {
        table.views(row)[self.0[row]]
    }

    /// Step `row` to its next view, back to the first after the last.
    pub(crate) fn next(&mut self, table: &Table, row: usize) {
        let step = &mut Arc::make_mut(&mut self.0)[row];
        *step = (*step + 1) % table.views(row).len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(glob: &str, views: &[View]) -> FileType {
        FileType::with_views(glob, views)
    }

    /// The cycle of the row `path` shows, by name.
    fn view_names<'a>(table: &'a Table, path: &str, binary: bool) -> Vec<&'a str> {
        let row = table.row_for(Path::new(path), || head(binary));
        table
            .views(row)
            .iter()
            .map(|view| table.view_name(*view))
            .collect()
    }

    /// A head with no MIME type, binary or not.
    fn head(binary: bool) -> Head {
        Head { binary, mime: None }
    }

    const MACH_O: &str = "application/x-mach-binary";

    /// The head of a Mach-O file.
    fn mach_o() -> Head {
        Head {
            binary: true,
            mime: Some(MACH_O),
        }
    }

    fn hand_off_row(glob: &str, program: &str) -> FileType {
        FileType::with_hand_off(glob, &[program, "{file}"])
    }

    #[test]
    fn a_glob_without_a_slash_matches_the_name_anywhere() {
        let glob: Glob = "*.json".parse().unwrap();
        assert!(glob.matches(Path::new("/a/b/c.json")));
        assert!(!glob.matches(Path::new("/a/b/c.jsonl")));
        assert!(!glob.matches(Path::new("/a.json/c.txt")));
    }

    #[test]
    fn braces_match_either_name() {
        let glob: Glob = "*.{jpg,png}".parse().unwrap();
        assert!(glob.matches(Path::new("x.jpg")));
        assert!(glob.matches(Path::new("x.png")));
        assert!(!glob.matches(Path::new("x.gif")));
    }

    #[test]
    fn a_glob_with_a_slash_matches_the_whole_path() {
        let glob: Glob = "**/logs/*.txt".parse().unwrap();
        assert!(glob.matches(Path::new("/var/logs/a.txt")));
        assert!(!glob.matches(Path::new("/var/logs/sub/a.txt")));
        assert!(!glob.matches(Path::new("/var/a.txt")));
    }

    #[test]
    fn a_dot_is_a_dot_not_any_character() {
        let glob: Glob = "a.txt".parse().unwrap();
        assert!(!glob.matches(Path::new("abtxt")));
    }

    #[test]
    fn an_open_brace_is_an_error() {
        assert!("*.{a,b".parse::<Glob>().is_err());
    }

    #[test]
    fn with_no_rows_a_text_file_cycles_raw_then_hex_and_a_binary_one_hex() {
        let table = Table::default();
        let text = table.row_for(Path::new("a.txt"), || head(false));
        let binary = table.row_for(Path::new("a.bin"), || head(true));
        assert_eq!(table.views(text), [View::Raw, View::Hex]);
        assert_eq!(table.views(binary), [View::Hex]);
    }

    #[test]
    fn the_first_matching_row_wins_and_user_rows_go_first() {
        let table = Table::new(&[row("*.json", &[View::Hex]), row("*", &[View::Raw])]);
        assert_eq!(
            table.views(table.row_for(Path::new("a.json"), || head(false))),
            [View::Hex]
        );
        assert_eq!(
            table.views(table.row_for(Path::new("a.bin"), || head(true))),
            [View::Raw],
            "a user row goes before the built-in binary row"
        );
    }

    #[test]
    fn a_row_with_no_views_leaves_the_cycle_to_the_rows_after_it() {
        let table = Table::new(&[hand_off_row("*.md", "marked"), row("*.md", &[View::Hex])]);
        assert_eq!(
            table.views(table.row_for(Path::new("a.md"), || head(false))),
            [View::Hex]
        );
    }

    #[test]
    fn the_hand_off_comes_from_the_first_matching_row_that_has_one() {
        let table = Table::new(&[
            row("*.md", &[View::Hex]),
            hand_off_row("*.md", "marked"),
            hand_off_row("*.txt", "other"),
        ]);
        let argv = |program: &str| HandOff::Argv(vec![program.to_string(), "{file}".to_string()]);
        assert_eq!(
            table.hand_off(Path::new("a.md"), || head(false)),
            argv("marked")
        );
        assert_eq!(
            table.hand_off(Path::new("a.txt"), || head(false)),
            argv("other")
        );
        assert_eq!(
            table.hand_off(Path::new("a.log"), || head(false)),
            HandOff::Editor
        );
        assert_eq!(
            table.hand_off(Path::new("a.png"), || head(true)),
            HandOff::DefaultApp
        );
    }

    #[test]
    fn a_mime_row_matches_a_file_whose_head_has_that_type() {
        let table = Table::new(&[row("*", &[View::Raw]).matching_mime(None, MACH_O)]);
        let views = |head: Head| table.views(table.row_for(Path::new("tool"), || head));
        assert_eq!(views(mach_o()), [View::Raw]);
        assert_eq!(views(head(true)), [View::Hex], "another binary file");
        assert_eq!(views(head(false)), [View::Raw, View::Hex], "a text file");
    }

    #[test]
    fn a_row_with_a_glob_and_a_mime_type_needs_both() {
        let table = Table::new(&[row("*", &[View::Raw]).matching_mime(Some("*.dylib"), MACH_O)]);
        let views = |path: &str| table.views(table.row_for(Path::new(path), mach_o));
        assert_eq!(views("libz.dylib"), [View::Raw]);
        assert_eq!(views("tool"), [View::Hex]);
    }

    #[test]
    fn a_mime_row_that_does_not_match_leaves_the_head_to_the_binary_row() {
        let table = Table::new(&[
            row("*", &[View::Raw]).matching_mime(None, "application/x-executable"),
            row("*.json", &[View::Raw]),
        ]);
        let row = table.row_for(Path::new("tool"), mach_o);
        assert_eq!(table.views(row), [View::Hex]);
    }

    #[test]
    fn a_mime_row_can_set_the_hand_off() {
        let table = Table::new(&[hand_off_row("*", "hopper").matching_mime(None, MACH_O)]);
        assert_eq!(
            table.hand_off(Path::new("tool"), mach_o),
            HandOff::Argv(vec!["hopper".to_string(), "{file}".to_string()])
        );
        assert_eq!(
            table.hand_off(Path::new("a.bin"), || head(true)),
            HandOff::DefaultApp
        );
    }

    #[test]
    fn a_user_row_does_not_read_the_file() {
        let table = Table::new(&[row("*.json", &[View::Hex])]);
        table.row_for(Path::new("a.json"), || panic!("read the head"));
    }

    #[test]
    fn a_step_wraps_and_holds_for_its_row_only() {
        let table = Table::new(&[row("*.log", &[View::Raw, View::Hex])]);
        let log = table.row_for(Path::new("a.log"), || head(false));
        let txt = table.row_for(Path::new("a.txt"), || head(false));
        let mut steps = Steps::new(&table);

        steps.next(&table, log);
        assert_eq!(steps.view(&table, log), View::Hex);
        assert_eq!(
            steps.view(&table, txt),
            View::Raw,
            "another row is unchanged"
        );

        steps.next(&table, log);
        assert_eq!(steps.view(&table, log), View::Raw, "the step wraps");
    }

    #[test]
    fn a_snapshot_does_not_see_a_later_step() {
        let table = Table::default();
        let row = table.row_for(Path::new("a.txt"), || head(false));
        let mut steps = Steps::new(&table);
        let sent = steps.clone();
        steps.next(&table, row);
        assert_eq!(sent.view(&table, row), View::Raw);
    }

    fn commands(entries: &[(&'static str, &str, Option<&str>)]) -> Commands {
        Commands::of(entries.iter().map(|(name, program, output)| {
            let command = Command::of(&[program]);
            (
                *name,
                output.map_or(command.clone(), |output| command.with_output(output)),
            )
        }))
    }

    #[test]
    fn a_command_view_is_in_the_cycle_by_its_name() {
        let (table, warnings) = Table::build(
            &[FileType::with_view_names("*.json", &["jq", "raw"])],
            &commands(&[("jq", "cat", None)]),
        );
        let row = table.row_for(Path::new("a.json"), || head(false));
        let views = table.views(row);
        assert_eq!(views.len(), 2);
        assert_eq!(table.view_name(views[0]), "jq");
        assert_eq!(views[1], View::Raw);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_view_that_cannot_run_is_skipped_with_one_warning() {
        let (table, warnings) = Table::build(
            &[
                FileType::with_view_names("*.json", &["gone", "hex"]),
                FileType::with_view_names("*.js", &["gone", "raw"]),
            ],
            &commands(&[("gone", "recon-no-such-program", None)]),
        );
        let views = |path: &str| {
            table
                .views(table.row_for(Path::new(path), || head(false)))
                .to_vec()
        };
        assert_eq!(views("a.json"), [View::Hex]);
        assert_eq!(views("a.js"), [View::Raw]);
        assert_eq!(warnings.len(), 1, "one for gone: {warnings:?}");
        assert!(warnings[0].contains("`recon-no-such-program` is not on PATH"));
    }

    /// `image`, and a command that makes a picture, are views like any
    /// other (#458).
    #[test]
    fn image_views_are_in_the_cycle() {
        let (table, warnings) = Table::build(
            &[FileType::with_view_names(
                "*.mmd",
                &["image", "picture", "raw"],
            )],
            &commands(&[("picture", "cat", Some("image/png"))]),
        );
        assert_eq!(
            view_names(&table, "a.mmd", false),
            ["image", "picture", "raw"]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// The built-in rows, after the user's: a picture by its extension, in
    /// any case, then a binary file, then the rest. The image row reads
    /// nothing.
    #[test]
    fn a_picture_shows_as_an_image_by_its_extension() {
        let table = Table::default();
        for name in ["a.png", "a.JPG", "a.jpeg", "a.gif", "a.webp", "a.tiff"] {
            let row = table.row_for(Path::new(name), || panic!("read the head"));
            assert_eq!(table.views(row), [View::Image, View::Hex], "{name}");
        }
        assert_eq!(
            table.hand_off(Path::new("a.png"), || panic!("read the head")),
            HandOff::DefaultApp
        );
        let row = table.row_for(Path::new("a.txt"), || head(false));
        assert_eq!(table.views(row), [View::Raw, View::Hex]);
    }

    /// On macOS a file of a type Quick Look shows has its thumbnail first,
    /// by its extension alone: many a PDF has no NUL in its head. Any other
    /// binary file shows its dump at once: Quick Look does not stop for a
    /// file it has no thumbnail for.
    #[test]
    fn a_file_of_a_known_type_shows_quick_look_on_macos() {
        let table = Table::default();
        let names = |path: &str, binary: bool| view_names(&table, path, binary);
        if cfg!(target_os = "macos") {
            assert_eq!(names("a.pdf", false), ["quicklook", "hex"]);
            assert_eq!(names("a.HEIC", true), ["quicklook", "hex"]);
            let row = table.row_for(Path::new("a.psd"), || panic!("read the head"));
            assert!(table.command(table.views(row)[0]).unwrap().is_image());
        } else {
            assert_eq!(names("a.pdf", true), ["hex"]);
        }
        assert_eq!(
            names("a.bin", true),
            ["hex"],
            "a type Quick Look does not show"
        );
    }

    /// `quicklook` is a view a row can name; where there is no Quick Look,
    /// the row skips it with a warning.
    #[test]
    fn a_row_can_name_quicklook() {
        let (table, warnings) = Table::build(
            &[FileType::with_view_names("*.key", &["quicklook", "hex"])],
            &Commands::default(),
        );
        let names = view_names(&table, "a.key", false);
        if cfg!(target_os = "macos") {
            assert_eq!(names, ["quicklook", "hex"]);
            assert!(warnings.is_empty(), "{warnings:?}");
        } else {
            assert_eq!(names, ["hex"]);
            assert!(warnings[0].contains("view `quicklook`"), "{warnings:?}");
        }
    }

    #[test]
    fn a_row_left_with_no_view_gives_its_cycle_to_the_rows_after_it() {
        let (table, _) = Table::build(
            &[FileType::with_view_names("*.json", &["gone"])],
            &commands(&[("gone", "recon-no-such-program", None)]),
        );
        let row = table.row_for(Path::new("a.json"), || head(false));
        assert_eq!(table.views(row), [View::Raw, View::Hex]);
    }
}
