//! The view cycle: which views a file offers, in which order, and which one
//! `-` has stepped to (#455).
//!
//! One table of rows, the user's `[[filetype]]` rows first and the built-in
//! rows after them. The first row that matches a file gives its whole cycle;
//! rows never merge.

use super::View;
use regex::Regex;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

/// One `[[filetype]]` row of the config.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileType {
    /// The files the row is for.
    pub glob: Glob,
    /// The cycle, default first. Never empty: the config refuses `[]`.
    #[serde(deserialize_with = "some_views")]
    pub views: Vec<View>,
}

/// `views = []` names no view to show, so it is a config error, not a row
/// that matches and then shows nothing.
fn some_views<'de, D>(deserializer: D) -> Result<Vec<View>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let views = Vec::<View>::deserialize(deserializer)?;
    if views.is_empty() {
        return Err(D::Error::custom("`views` names no view"));
    }
    Ok(views)
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

/// What a row matches.
#[derive(Debug, Clone)]
enum Matcher {
    /// A user row.
    Glob(Glob),
    /// The built-in row for a file with a NUL in its head.
    Binary,
    /// The built-in row for every other file.
    Any,
}

#[derive(Debug, Clone)]
struct Row {
    matcher: Matcher,
    views: Vec<View>,
}

/// Every row, the user's first. Built once at startup and shared with the
/// workers, which resolve a file's row: only they may read the file's head.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    rows: Vec<Row>,
}

impl Default for Table {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl Table {
    /// The user's rows, then the built-in ones.
    pub(crate) fn new(user: &[FileType]) -> Self {
        let user = user.iter().map(|row| Row {
            matcher: Matcher::Glob(row.glob.clone()),
            views: row.views.clone(),
        });
        let built_in = [
            Row {
                matcher: Matcher::Binary,
                views: vec![View::Hex],
            },
            Row {
                matcher: Matcher::Any,
                views: vec![View::Raw, View::Hex],
            },
        ];
        Self {
            rows: user.chain(built_in).collect(),
        }
    }

    /// The first row that matches `path`. `binary` is asked only when a
    /// built-in row is reached, so a user row costs no read.
    pub(crate) fn row_for(&self, path: &Path, binary: impl FnOnce() -> bool) -> usize {
        // One built-in row asks, so `binary` runs at most once.
        let mut binary = Some(binary);
        self.rows
            .iter()
            .position(|row| match &row.matcher {
                Matcher::Glob(glob) => glob.matches(path),
                Matcher::Binary => binary.take().is_some_and(|binary| binary()),
                Matcher::Any => true,
            })
            .unwrap_or(self.rows.len() - 1)
    }

    /// The cycle of `row`.
    pub(crate) fn views(&self, row: usize) -> &[View] {
        &self.rows[row].views
    }

    fn len(&self) -> usize {
        self.rows.len()
    }
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
        FileType {
            glob: glob.parse().expect("a glob"),
            views: views.to_vec(),
        }
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
        let text = table.row_for(Path::new("a.txt"), || false);
        let binary = table.row_for(Path::new("a.bin"), || true);
        assert_eq!(table.views(text), [View::Raw, View::Hex]);
        assert_eq!(table.views(binary), [View::Hex]);
    }

    #[test]
    fn the_first_matching_row_wins_and_user_rows_go_first() {
        let table = Table::new(&[row("*.json", &[View::Hex]), row("*", &[View::Raw])]);
        assert_eq!(
            table.views(table.row_for(Path::new("a.json"), || false)),
            [View::Hex]
        );
        assert_eq!(
            table.views(table.row_for(Path::new("a.bin"), || true)),
            [View::Raw],
            "a user row goes before the built-in binary row"
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
        let log = table.row_for(Path::new("a.log"), || false);
        let txt = table.row_for(Path::new("a.txt"), || false);
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
        let row = table.row_for(Path::new("a.txt"), || false);
        let mut steps = Steps::new(&table);
        let sent = steps.clone();
        steps.next(&table, row);
        assert_eq!(sent.view(&table, row), View::Raw);
    }
}
