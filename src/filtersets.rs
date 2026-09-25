//! `filters.toml`: the schema, its validation, and where it lives (#128).
//!
//! The full reasoning is in
//! `docs/specs/2026-09-03-saved-filter-sets-design.md`, *The file*. The
//! short version: one file beside `config.toml`, one `[sets.<name>]` table
//! per set, single-quoted regexes so nothing is escaped, and every way the
//! file can be wrong is refused **before the terminal is taken** — a warning
//! printed and then overwritten by the alternate screen is a warning nobody
//! reads, the same policy `config.toml` follows and for the same reason.
//!
//! This module produces [`LoadedSet`]s; what a set *is* — the scratch set,
//! effective-enabled, profiles as actions — belongs to the `filter` module.

use crate::config::parse_colour;
use crate::filter::{LoadedFilter, LoadedSet, Predicate, Sense};
use crate::syntax::Kind;
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The file, beside `config.toml`.
const FILE: &str = "filters.toml";

/// The end of a set file's name in a `RECON_FILTER_PATH` directory, beside
/// a bare [`FILE`] (#46).
const SUFFIX: &str = ".filters.toml";

/// A set's position in the pane when the file does not say. Lower is
/// nearer the top; the scratch set is always first regardless.
pub const DEFAULT_PRIORITY: i32 = 50;

/// The one `mode` the reserved key accepts. `mode` exists so that #40 (AND
/// within a set) has a key waiting; until then any other value is refused
/// rather than silently meaning OR.
const ONLY_MODE: &str = "or";

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
struct FileSchema {
    #[serde(default)]
    sets: BTreeMap<String, SetSchema>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
struct SetSchema {
    priority: Option<i32>,
    autoload: Option<bool>,
    /// Listed at startup (#282). `true` when absent; `false` wins over
    /// `autoload = true` rather than refusing the file (ADR 0002).
    listed: Option<bool>,
    /// One line of plain text for the set picker (#284). No default: a set
    /// without one shows a blank, except the built-in set, which recon
    /// describes itself.
    description: Option<String>,
    mode: Option<String>,
    #[serde(default)]
    profiles: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    filters: Vec<FilterSchema>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct FilterSchema {
    pattern: String,
    name: Option<String>,
    sense: Option<SenseSchema>,
    colour: Option<String>,
}

/// `sense` as the file spells it. A separate enum rather than deriving
/// `Deserialize` on `filter::Sense`, so the file format is decided here and
/// a rename in the model cannot silently change what a file means.
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum SenseSchema {
    Include,
    Context,
    Exclude,
}

impl From<SenseSchema> for Sense {
    fn from(sense: SenseSchema) -> Self {
        match sense {
            SenseSchema::Include => Self::Include,
            SenseSchema::Context => Self::Context,
            SenseSchema::Exclude => Self::Exclude,
        }
    }
}

/// Why `filters.toml` could not be loaded.
///
/// Every variant carries the path: with `$XDG_CONFIG_HOME` in play, *which*
/// file recon found is the first question when a set does not appear.
#[derive(Debug)]
pub enum Error {
    /// The file exists but could not be read. A missing file is not an error
    /// and never reaches here.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A `RECON_FILTER_PATH` directory exists but could not be listed. A
    /// missing one is not an error and never reaches here.
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Not valid TOML, or a key the schema does not define.
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    /// Parsed, but says something recon cannot use. `filter` names the
    /// offending filter — by its name, or its pattern — when there is one.
    Invalid {
        path: PathBuf,
        set: String,
        filter: Option<String>,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    f,
                    "could not read filter sets file {}: {source}",
                    path.display()
                )
            }
            Self::ReadDir { path, source } => {
                write!(
                    f,
                    "could not read filter sets directory {}: {source}",
                    path.display()
                )
            }
            // On its own line, as `ConfigError::Parse` does: toml's error is a
            // multi-line snippet with a caret, and reads badly after a colon.
            Self::Parse { path, source } => {
                write!(f, "invalid filter sets file {}\n{source}", path.display())
            }
            Self::Invalid {
                path,
                set,
                filter,
                message,
            } => {
                write!(
                    f,
                    "invalid filter sets file {}: [sets.{set}]",
                    path.display()
                )?;
                if let Some(filter) = filter {
                    write!(f, " filter '{filter}'")?;
                }
                write!(f, ": {message}")
            }
        }
    }
}

/// No `source()`, for the reason `ConfigError` gives: `Display` already
/// renders the underlying error, and `color_eyre` would print it twice.
impl std::error::Error for Error {}

/// Parse and validate one file's text. Pure: `path` is only for messages.
///
/// The built-in set is always in the result, whether or not the file names
/// it (#220); see [`LoadedSet::builtin_default`].
///
/// The result is sorted by `(priority, name)`, which is the pane's order.
/// Everything the spec lists as rejected is rejected here, with the set and
/// filter named, so that a user reading the message in a hurry can go
/// straight to the line.
pub fn parse(text: &str, path: &Path) -> Result<Vec<LoadedSet>, Error> {
    Ok(finish(parse_sets(text, path)?))
}

/// One file's sets as the file wrote them: no built-in default added and
/// not yet sorted, so that [`load_all`] can tell a file that names the
/// built-in set from one that does not.
fn parse_sets(text: &str, path: &Path) -> Result<Vec<LoadedSet>, Error> {
    let file: FileSchema = toml::from_str(text).map_err(|source| Error::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let invalid = |set: &str, filter: Option<&str>, message: String| Error::Invalid {
        path: path.to_path_buf(),
        set: set.to_string(),
        filter: filter.map(str::to_string),
        message,
    };

    let mut sets = Vec::with_capacity(file.sets.len());
    for (name, schema) in file.sets {
        if name.is_empty() {
            return Err(invalid(&name, None, "a set's name cannot be empty".into()));
        }
        // One line (#284): the picker gives each set one row, and a line
        // break would draw over the next set's row.
        if schema
            .description
            .as_deref()
            .is_some_and(|text| text.contains(['\n', '\r']))
        {
            return Err(invalid(
                &name,
                None,
                "`description` must be one line of text".into(),
            ));
        }
        if let Some(mode) = &schema.mode
            && mode != ONLY_MODE
        {
            return Err(invalid(
                &name,
                None,
                format!("mode {mode:?} is not supported; only {ONLY_MODE:?} is"),
            ));
        }
        // A table naming a built-in set (#127) positions and switches it
        // and may give it profiles (#220); what it may not do is add
        // filters, which are recon's. A profile's members are the kinds'
        // plural names — `functions`, `types` — which the file never shows,
        // so a refusal lists them.
        if crate::filter::is_builtin_name(&name) {
            if !schema.filters.is_empty() {
                return Err(invalid(
                    &name,
                    None,
                    format!(
                        "{name:?} is a built-in set; its table may set `priority`, \
                         `autoload`, `listed`, `description` and `profiles` only"
                    ),
                ));
            }
            for (profile, members) in &schema.profiles {
                if let Some(missing) = members
                    .iter()
                    .find(|member| !Kind::ALL.iter().any(|kind| kind.plural() == *member))
                {
                    let kinds: Vec<&str> = Kind::ALL.iter().map(|kind| kind.plural()).collect();
                    return Err(invalid(
                        &name,
                        None,
                        format!(
                            "profile {profile:?} names {missing:?}, which is not a filter in \
                             this set; the built-in filters are {}",
                            kinds.join(", ")
                        ),
                    ));
                }
            }
            sets.push(LoadedSet {
                name,
                path: path.to_path_buf(),
                priority: schema.priority.unwrap_or(DEFAULT_PRIORITY),
                autoload: schema.autoload.unwrap_or(false),
                listed: schema.listed.unwrap_or(true),
                description: schema.description,
                profiles: schema.profiles,
                filters: Vec::new(),
                builtin: true,
            });
            continue;
        }
        if schema.filters.is_empty() {
            return Err(invalid(
                &name,
                None,
                "a set with no filters; add at least one [[sets.<name>.filters]]".into(),
            ));
        }

        let mut filters: Vec<LoadedFilter> = Vec::with_capacity(schema.filters.len());
        for entry in schema.filters {
            // What the pane will call it, and so what a message calls it
            // (#200): the `name` when there is one, the pattern otherwise.
            let display = entry.name.unwrap_or_else(|| entry.pattern.clone());
            let regex = Regex::new(&entry.pattern)
                .map_err(|err| invalid(&name, Some(&display), err.to_string()))?;
            let colour = entry
                .colour
                .as_deref()
                .map(parse_colour)
                .transpose()
                .map_err(|message| invalid(&name, Some(&display), message))?;
            if filters.iter().any(|filter| filter.name == display) {
                return Err(invalid(
                    &name,
                    Some(&display),
                    format!("two filters named {display:?}; give one a distinct `name`"),
                ));
            }
            filters.push(LoadedFilter {
                name: display,
                predicate: Predicate::Regex(regex),
                sense: entry.sense.map_or(Sense::Include, Into::into),
                colour,
            });
        }

        for (profile, members) in &schema.profiles {
            if let Some(missing) = members
                .iter()
                .find(|member| !filters.iter().any(|filter| &filter.name == *member))
            {
                return Err(invalid(
                    &name,
                    None,
                    format!(
                        "profile {profile:?} names {missing:?}, which is not a filter in this set"
                    ),
                ));
            }
        }

        sets.push(LoadedSet {
            name,
            path: path.to_path_buf(),
            priority: schema.priority.unwrap_or(DEFAULT_PRIORITY),
            autoload: schema.autoload.unwrap_or(false),
            listed: schema.listed.unwrap_or(true),
            description: schema.description,
            profiles: schema.profiles,
            filters,
            builtin: false,
        });
    }
    Ok(sets)
}

/// Make a list of loaded sets the pane's list: the built-in set present
/// and everything in `(priority, name)` order.
fn finish(mut sets: Vec<LoadedSet>) -> Vec<LoadedSet> {
    // The built-in set is always among the loaded sets (#220), so the
    // `--set` check in `main` and `with_sets` read one list. A file that
    // names it has already pushed it; one that does not gets the default.
    if !sets.iter().any(|set| set.builtin) {
        sets.push(LoadedSet::builtin_default());
    }
    sets.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.name.cmp(&b.name))
    });
    sets
}

/// What `S` writes: the scratch set, under a name (#131).
///
/// Patterns and senses only. No `name` key — the pattern is the name, which
/// is what the `default` profile refers to — and no `priority`, `autoload`
/// or `colour`: each is a one-line hand edit to a file `S` has just shown
/// the shape of, and a default the user did not ask for is a thing to
/// delete later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetToSave<'a> {
    pub name: &'a str,
    /// Each filter's pattern and sense, in pane order.
    pub filters: Vec<(String, Sense)>,
    /// The patterns of the filters enabled right now, which become the
    /// set's `default` profile so it opens the way it was saved.
    pub default: Vec<String>,
}

/// Append `set` to the file's `text`, touching nothing else.
///
/// A `[sets.<name>]` already in `text` is refused, not replaced (#154):
/// `Table::insert` would overwrite it, comment and all, and the caller's
/// name check runs over the sets it *loaded*, which is not the file as it
/// is now — a table added by hand since startup is exactly what it cannot
/// see.
///
/// `toml_edit` rather than `toml`'s serializer, which stays off in
/// `Cargo.toml`: a hand-edited file's comments, key order and whitespace all
/// survive, and the new tables go at the end. A pattern goes in as a
/// single-quoted literal string wherever TOML allows one — no `\\` tax on
/// the way out, matching the way in — and as a basic string only when it
/// holds a `'` or a newline, which a literal cannot.
pub fn append_set(text: &str, set: &SetToSave<'_>) -> Result<String, String> {
    use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

    let mut doc: DocumentMut = text
        .parse()
        .map_err(|err: toml_edit::TomlError| err.to_string())?;
    let sets = doc
        .as_table_mut()
        .entry("sets")
        .or_insert_with(|| {
            let mut table = Table::new();
            // `[sets]` on its own says nothing; only `[sets.<name>]` should
            // appear, which is what implicit means.
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| "`sets` is not a table".to_string())?;
    sets.set_implicit(true);
    if sets.contains_key(set.name) {
        return Err(format!(
            "a set named {:?} is already in filters.toml; it was added since recon started",
            set.name
        ));
    }

    let mut table = Table::new();
    if !set.default.is_empty() {
        let mut profiles = Table::new();
        let mut members = Array::new();
        for member in &set.default {
            members.push(member.as_str());
        }
        profiles.insert("default", value(members));
        table.insert("profiles", Item::Table(profiles));
    }
    let mut filters = ArrayOfTables::new();
    for (pattern, sense) in &set.filters {
        let mut filter = Table::new();
        filter.insert("pattern", literal_string(pattern)?);
        let sense = match sense {
            Sense::Include => None,
            Sense::Context => Some("context"),
            Sense::Exclude => Some("exclude"),
        };
        if let Some(sense) = sense {
            filter.insert("sense", value(sense));
        }
        filters.push(filter);
    }
    table.insert("filters", Item::ArrayOfTables(filters));
    sets.insert(set.name, Item::Table(table));
    Ok(doc.to_string())
}

/// `pattern` as a TOML string value, single-quoted when it can be.
///
/// `toml_edit` exposes no way to choose a value's quoting, so the literal
/// form is made by parsing one and moving the value across; its
/// representation travels with it.
fn literal_string(pattern: &str) -> Result<toml_edit::Item, String> {
    use toml_edit::{DocumentMut, value};

    if pattern.contains('\'') || pattern.contains('\n') || pattern.contains('\r') {
        return Ok(value(pattern));
    }
    let mut one: DocumentMut = format!("pattern = '{pattern}'\n")
        .parse()
        .map_err(|err: toml_edit::TomlError| err.to_string())?;
    Ok(one
        .as_table_mut()
        .remove("pattern")
        .expect("the snippet defines `pattern`"))
}

/// Where `filters.toml` lives: beside `config.toml`, by the same rules.
/// Takes the environment as arguments for the reason `config_path_from`
/// gives — tests must not set real variables.
#[must_use]
pub fn path_from(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    Some(crate::config::config_home_from(xdg_config_home, home)?.join(FILE))
}

/// Where recon looks for `filters.toml`, or `None` when the environment
/// names no home to look in.
#[must_use]
pub fn path() -> Option<PathBuf> {
    path_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// The directories `RECON_FILTER_PATH` names, in order (#46).
///
/// Takes the environment as arguments, as [`path_from`] does. An empty
/// entry is skipped rather than read as the current directory, as `PATH`
/// would: a repository's sets are read only when the user names its
/// directory. A leading `~` is expanded against `home`, since a shell does
/// not reliably expand one after a colon. A directory named twice is read
/// once, at its first position.
#[must_use]
pub fn search_path_from(filter_path: Option<&str>, home: Option<&str>) -> Vec<PathBuf> {
    let home = home.filter(|home| !home.is_empty());
    let entries = filter_path
        .map(|list| std::env::split_paths(list).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in entries {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let dir = match (home, dir.strip_prefix("~")) {
            (Some(home), Ok(rest)) => Path::new(home).join(rest),
            _ => dir,
        };
        let key = crate::path::lexical_absolute(&dir);
        if dirs
            .iter()
            .any(|seen| crate::path::lexical_absolute(seen) == key)
        {
            continue;
        }
        dirs.push(dir);
    }
    dirs
}

/// Every file recon reads sets from, highest precedence first: `own`, the
/// user's `filters.toml`, then each of `dirs`' set files (#46).
///
/// `own` is always first, so a personal set shadows a team set of the same
/// name wherever the path puts the team's directory — and it is also the
/// one file `S` writes, so a saved set is found again by the next start.
///
/// A set file is `filters.toml` or `<name>.filters.toml`, so a directory
/// can hold one file, as the user's config directory does, or one per group
/// of sets — `deploy.filters.toml`, `triage.filters.toml`. They are read in
/// file-name order, so within a directory the name decides which set
/// shadows which, as in a `conf.d` directory. Not `*filters.toml`, which
/// would take `oldfilters.toml` too; and not `.filters.toml`, which has no
/// name and is a hidden file besides. A directory
/// that does not exist is skipped, as a missing `PATH` entry is no error to
/// a shell; one that cannot be read is an error.
pub fn set_files(own: Option<PathBuf>, dirs: &[PathBuf]) -> Result<Vec<PathBuf>, Error> {
    let mut files: Vec<PathBuf> = own.into_iter().collect();
    for dir in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                log::debug!("no filter sets directory at {}", dir.display());
                continue;
            }
            Err(source) => {
                return Err(Error::ReadDir {
                    path: dir.clone(),
                    source,
                });
            }
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| Error::ReadDir {
                path: dir.clone(),
                source,
            })?;
            let name = entry.file_name();
            let is_set_file = name.to_str().is_some_and(|name| {
                name == FILE || (name.len() > SUFFIX.len() && name.ends_with(SUFFIX))
            });
            // `Path::is_file` follows a symlink, so a linked file counts;
            // a directory that happens to carry the suffix does not.
            if is_set_file && entry.path().is_file() {
                found.push(entry.path());
            }
        }
        found.sort();
        if found.is_empty() {
            log::debug!("no *{SUFFIX} files in {}", dir.display());
        }
        files.extend(found);
    }
    Ok(files)
}

/// Read and parse one file, as [`parse_sets`] leaves it. A file that is not
/// there is no sets.
fn read_sets(path: &Path) -> Result<Vec<LoadedSet>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // The overwhelmingly common case, and not a failure: recon runs with
        // the scratch set and the built-in one. Only this one kind is
        // forgiven — a permission error or a directory in the file's place
        // is real.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            log::debug!("no filter sets file at {}", path.display());
            return Ok(Vec::new());
        }
        Err(source) => {
            return Err(Error::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    // Which file was read is the first thing anyone asks when a set does
    // not appear; absence is logged too, since "no file" and "the wrong
    // file" look identical from the pane (#83).
    log::debug!("reading filter sets from {}", path.display());
    parse_sets(&text, path)
}

/// Read and parse one file. A file that is not there is no sets.
#[cfg(test)]
fn load_from(path: &Path) -> Result<Vec<LoadedSet>, Error> {
    load_all(&[path.to_path_buf()])
}

/// Read every file in `files`, highest precedence first, into one list.
///
/// A set whose name an earlier file already gave is shadowed, the way a
/// later `PATH` entry is: it is dropped whole, never merged, so what a set
/// holds can always be read from one file (#46). The built-in set's table
/// shadows the same way. Every file is still read and validated, shadowed
/// sets included — an error in any of them refuses to start, for the
/// reason `Config::load` gives, and a broken team file should not wait to
/// be found until the personal set shadowing it is deleted.
fn load_all(files: &[PathBuf]) -> Result<Vec<LoadedSet>, Error> {
    let mut sets: Vec<LoadedSet> = Vec::new();
    for file in files {
        for set in read_sets(file)? {
            if let Some(winner) = sets.iter().find(|seen| seen.name == set.name) {
                log::debug!(
                    "set {:?} in {} is shadowed by the one in {}",
                    set.name,
                    set.path.display(),
                    winner.path.display()
                );
                continue;
            }
            sets.push(set);
        }
    }
    Ok(finish(sets))
}

/// The file layer. Call before the terminal is initialised: an error here
/// refuses to start, for the reason `Config::load` gives. `filter_path` is
/// `--filter-path` or `RECON_FILTER_PATH`, as clap resolved it.
pub fn load_file(filter_path: Option<&str>) -> Result<Vec<LoadedSet>, Error> {
    let own = path();
    if own.is_none() {
        log::debug!("no config home ($XDG_CONFIG_HOME, $HOME unset); no filters.toml read");
    }
    let dirs = search_path_from(filter_path, std::env::var("HOME").ok().as_deref());
    load_all(&set_files(own, &dirs)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Vec<LoadedSet> {
        parse(text, Path::new("t/filters.toml")).expect("valid file")
    }

    fn rejected(text: &str) -> String {
        parse(text, Path::new("t/filters.toml"))
            .expect_err("invalid file")
            .to_string()
    }

    const MINIMAL: &str = "[sets.a]\n[[sets.a.filters]]\npattern = 'foo'\n";

    /// The built-in set is the loader's to supply (#220): a file that never
    /// names it still yields it, at its defaults, so `--set definitions` is
    /// validated against the same list `with_sets` builds from.
    #[test]
    fn an_empty_file_has_only_the_builtin_set() {
        let sets = parsed("");
        assert_eq!(sets.len(), 1);
        let builtin = &sets[0];
        assert!(builtin.builtin);
        assert_eq!(builtin.name, crate::filter::DEFINITIONS_SET);
        assert_eq!(builtin.priority, DEFAULT_PRIORITY);
        assert!(!builtin.autoload);
        assert!(builtin.profiles.is_empty());
        assert!(builtin.filters.is_empty());
    }

    #[test]
    fn a_minimal_set_takes_every_default() {
        let sets = parsed(MINIMAL);
        assert_eq!(sets.len(), 2, "the set and the built-in one");
        let a = &sets[0];
        assert_eq!(a.name, "a");
        assert_eq!(a.path, Path::new("t/filters.toml"));
        assert_eq!(a.priority, DEFAULT_PRIORITY);
        assert!(!a.autoload);
        assert!(a.profiles.is_empty());
        assert_eq!(a.filters[0].name, "foo", "name falls back to the pattern");
        assert_eq!(a.filters[0].sense, Sense::Include);
        assert_eq!(a.filters[0].colour, None);
    }

    #[test]
    fn every_key_is_read() {
        let sets = parsed(
            r#"
[sets.w]
priority = 10
autoload = true
mode = "or"
[sets.w.profiles]
default = ["assoc"]
[[sets.w.filters]]
name = "assoc"
pattern = 'wlan\d+: associated'
colour = "red"
[[sets.w.filters]]
pattern = 'retry'
sense = "exclude"
[[sets.w.filters]]
pattern = 'beacon'
sense = "context"
"#,
        );
        let w = &sets[0];
        assert_eq!((w.priority, w.autoload), (10, true));
        assert_eq!(w.profiles["default"], vec!["assoc".to_string()]);
        assert_eq!(w.filters[0].colour, Some(ratatui::style::Color::Red));
        assert_eq!(
            w.filters[0].predicate.display(),
            r"wlan\d+: associated",
            "a literal string keeps its backslashes"
        );
        assert_eq!(w.filters[1].sense, Sense::Exclude);
        assert_eq!(w.filters[1].name, "retry");
        assert_eq!(w.filters[2].sense, Sense::Context);
    }

    #[test]
    fn sets_sort_by_priority_then_name() {
        let sets = parsed(
            "[sets.zebra]\npriority = 10\n[[sets.zebra.filters]]\npattern = 'z'\n\
             [sets.beta]\n[[sets.beta.filters]]\npattern = 'b'\n\
             [sets.alpha]\n[[sets.alpha.filters]]\npattern = 'a'\n",
        );
        let names: Vec<&str> = sets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["zebra", "alpha", "beta", "definitions"],
            "the built-in set sorts among the file's by the same rule"
        );
    }

    #[test]
    fn a_bad_pattern_names_the_file_set_and_filter() {
        let message = rejected("[sets.a]\n[[sets.a.filters]]\npattern = '('\n");
        assert!(message.contains("t/filters.toml"), "{message}");
        assert!(message.contains("[sets.a]"), "{message}");
        assert!(message.contains("filter '('"), "{message}");
    }

    #[test]
    fn a_bad_colour_explains_the_forms() {
        let message = rejected("[sets.a]\n[[sets.a.filters]]\npattern = 'x'\ncolour = 'reddish'\n");
        assert!(message.contains("hex triple"), "{message}");
        assert!(message.contains("filter 'x'"), "{message}");
    }

    #[test]
    fn duplicate_names_are_rejected_after_the_fallback() {
        let message = rejected(
            "[sets.a]\n[[sets.a.filters]]\npattern = 'x'\n[[sets.a.filters]]\nname = 'x'\npattern = 'y'\n",
        );
        assert!(message.contains("two filters named \"x\""), "{message}");
    }

    #[test]
    fn a_profile_must_name_real_filters() {
        let message = rejected(
            "[sets.a]\n[sets.a.profiles]\ndefault = ['nope']\n[[sets.a.filters]]\npattern = 'x'\n",
        );
        assert!(message.contains("profile \"default\""), "{message}");
        assert!(message.contains("\"nope\""), "{message}");
    }

    /// `[sets.definitions]` positions and switches the built-in set (#127)
    /// and may carry profiles over its filters' display names (#220); what
    /// it may not do is add filters, which are recon's.
    #[test]
    fn a_builtin_set_table_carries_priority_autoload_and_profiles() {
        let sets = parsed("[sets.definitions]\npriority = 80\nautoload = true\n");
        assert_eq!(sets.len(), 1, "the table stands in for the default");
        assert!(sets[0].builtin);
        assert_eq!((sets[0].priority, sets[0].autoload), (80, true));
        assert!(sets[0].filters.is_empty());
        assert!(
            rejected("[sets.definitions]\n[[sets.definitions.filters]]\npattern = 'x'\n")
                .contains("built-in")
        );
        let sets = parsed(
            "[sets.definitions]\n[sets.definitions.profiles]\ndefault = ['functions']\n\
             types = ['types', 'structs', 'enums']\n",
        );
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].profiles["default"], vec!["functions".to_string()]);
        assert_eq!(sets[0].profiles["types"].len(), 3);
        // An empty table is fine: it names the set and changes nothing.
        assert!(parsed("[sets.definitions]\n")[0].builtin);
    }

    /// A built-in profile's members are checked against the kinds, the same
    /// way a file set's are checked against its filters, and the message
    /// lists what the names are — nothing in the file shows them.
    #[test]
    fn a_builtin_profile_must_name_definition_kinds() {
        let message = rejected(
            "[sets.definitions]\n[sets.definitions.profiles]\ntypes = ['types', 'nope']\n",
        );
        assert!(message.contains("[sets.definitions]"), "{message}");
        assert!(message.contains("profile \"types\""), "{message}");
        assert!(message.contains("\"nope\""), "{message}");
        assert!(message.contains("functions"), "lists the kinds: {message}");
        assert!(message.contains("sections"), "lists the kinds: {message}");
    }

    /// `Error::Invalid` names the filter by its `name` when it has one (#200);
    /// the pattern is the fallback, not the rule.
    #[test]
    fn a_bad_filter_is_named_by_its_name_when_it_has_one() {
        let message = rejected("[sets.a]\n[[sets.a.filters]]\nname = 'opener'\npattern = '('\n");
        assert!(message.contains("filter 'opener'"), "{message}");
        assert!(!message.contains("filter '('"), "{message}");
        let message = rejected(
            "[sets.a]\n[[sets.a.filters]]\nname = 'warm'\npattern = 'x'\ncolour = 'reddish'\n",
        );
        assert!(message.contains("filter 'warm'"), "{message}");
    }

    #[test]
    fn a_set_needs_a_filter() {
        assert!(rejected("[sets.a]\n").contains("no filters"));
    }

    #[test]
    fn mode_accepts_only_or() {
        assert!(
            parse(
                "[sets.a]\nmode = 'or'\n[[sets.a.filters]]\npattern = 'x'\n",
                Path::new("t")
            )
            .is_ok()
        );
        assert!(
            rejected("[sets.a]\nmode = 'and'\n[[sets.a.filters]]\npattern = 'x'\n")
                .contains("mode \"and\"")
        );
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        assert!(
            rejected("[sets.a]\ncolor = 'red'\n[[sets.a.filters]]\npattern = 'x'\n")
                .contains("color")
        );
        assert!(
            rejected("[sets.a]\n[[sets.a.filters]]\npattern = 'x'\ncolor = 'red'\n")
                .contains("color")
        );
    }

    #[test]
    fn an_empty_set_name_is_rejected() {
        assert!(rejected("[sets.\"\"]\n[[sets.\"\".filters]]\npattern = 'x'\n").contains("empty"));
    }

    // ---- saving (#131) -----------------------------------------------------

    #[test]
    fn append_set_preserves_comments_and_other_sets() {
        let before = "# my sets\n[sets.a]\n# keep me\n[[sets.a.filters]]\npattern = 'x'\n";
        let after = append_set(
            before,
            &SetToSave {
                name: "bug 57",
                filters: vec![
                    (r"\bERROR\b".into(), Sense::Include),
                    ("DEBUG".into(), Sense::Exclude),
                    ("ctx".into(), Sense::Context),
                ],
                default: vec![r"\bERROR\b".into()],
            },
        )
        .expect("edits");
        assert!(
            after.starts_with(before),
            "existing text is untouched:\n{after}"
        );
        assert!(after.contains("[sets.\"bug 57\"]"), "{after}");
        assert!(
            after.contains(r"pattern = '\bERROR\b'"),
            "single-quoted literal:\n{after}"
        );
        assert!(after.contains("sense = \"exclude\""), "{after}");
        assert!(after.contains("sense = \"context\""), "{after}");
        assert!(
            after.contains(r"default = ['\bERROR\b']"),
            "the profile member is a literal string too:\n{after}"
        );
        assert!(!after.contains("autoload"), "{after}");
        assert!(
            !after.contains("\n[sets]\n"),
            "no bare [sets] header:\n{after}"
        );
        let sets = parse(&after, Path::new("t")).expect("round-trips");
        assert_eq!(sets.len(), 3, "a, bug 57 and the built-in set");
        assert_eq!(sets[1].name, "bug 57");
        assert_eq!(sets[1].filters[0].predicate.display(), r"\bERROR\b");
        assert_eq!(sets[1].profiles["default"], vec![r"\bERROR\b".to_string()]);
    }

    #[test]
    fn append_set_starts_an_empty_file() {
        let after = append_set(
            "",
            &SetToSave {
                name: "n",
                filters: vec![("x".into(), Sense::Include)],
                default: vec![],
            },
        )
        .expect("edits");
        assert!(after.contains("[sets.n]"), "{after}");
        assert!(!after.contains("profiles"), "no empty default: {after}");
        assert!(parse(&after, Path::new("t")).is_ok());
    }

    /// `Table::insert` would replace an existing `[sets.<name>]`, comment
    /// and all. The caller checks the name against what it loaded, which is
    /// not the file as it is now (#154).
    #[test]
    fn append_set_refuses_a_name_the_file_already_holds() {
        let before = "# my file\n[sets.bug]\n[[sets.bug.filters]]\npattern = 'ORIGINAL'\n";
        let err = append_set(
            before,
            &SetToSave {
                name: "bug",
                filters: vec![("NEW".into(), Sense::Include)],
                default: vec![],
            },
        )
        .expect_err("refused");
        assert!(
            err.contains("a set named \"bug\" is already in filters.toml"),
            "{err}"
        );
        assert!(err.contains("since recon started"), "{err}");
    }

    /// A pattern a literal string cannot hold falls back to a basic string,
    /// escaped, and still round-trips.
    #[test]
    fn a_pattern_with_a_quote_falls_back_to_a_basic_string() {
        let after = append_set(
            "",
            &SetToSave {
                name: "q",
                filters: vec![("it's".into(), Sense::Include)],
                default: vec![],
            },
        )
        .expect("edits");
        assert!(after.contains("pattern = \"it's\""), "{after}");
        let sets = parse(&after, Path::new("t")).expect("round-trips");
        let q = sets
            .iter()
            .find(|set| set.name == "q")
            .expect("q was appended");
        assert_eq!(q.filters[0].predicate.display(), "it's");
    }

    #[test]
    fn the_path_sits_beside_config_toml() {
        assert_eq!(
            path_from(Some("/x"), Some("/h")),
            Some(PathBuf::from("/x/recon/filters.toml"))
        );
        assert_eq!(
            path_from(None, Some("/h")),
            Some(PathBuf::from("/h/.config/recon/filters.toml"))
        );
        assert_eq!(path_from(Some("relative"), None), None);
    }

    #[test]
    fn a_missing_file_is_the_builtin_set_alone() {
        let sets =
            load_from(Path::new("target/test-config/no-such-filters.toml")).expect("not an error");
        assert_eq!(sets.len(), 1);
        assert!(sets[0].builtin);
    }

    #[test]
    fn a_directory_in_the_files_place_is_an_error() {
        let dir = Path::new("target/test-config/filters-as-a-dir.toml");
        std::fs::create_dir_all(dir).expect("mkdir");
        assert!(matches!(load_from(dir), Err(Error::Read { .. })));
    }

    // ---- listed (#282) -----------------------------------------------------

    #[test]
    fn listed_defaults_to_true_and_can_be_false() {
        let file = "[sets.a]\n[[sets.a.filters]]\npattern = 'x'\n";
        let sets = parsed(file);
        assert!(
            sets.iter().all(|set| set.listed),
            "a, and the default built-in"
        );
        let sets = parsed(&format!(
            "{file}[sets.b]\nlisted = false\n[[sets.b.filters]]\npattern = 'y'\n"
        ));
        let b = sets.iter().find(|set| set.name == "b").expect("b");
        assert!(!b.listed);
    }

    /// The one contradiction the file is not refused for (ADR 0002).
    #[test]
    fn listed_false_with_autoload_true_is_accepted() {
        let sets = parsed(
            "[sets.a]\nlisted = false\nautoload = true\n[[sets.a.filters]]\npattern = 'x'\n",
        );
        assert_eq!((sets[0].listed, sets[0].autoload), (false, true));
    }

    #[test]
    fn a_builtin_set_table_accepts_listed() {
        let sets = parsed("[sets.definitions]\nlisted = false\n");
        assert!(sets[0].builtin);
        assert!(!sets[0].listed);
    }

    // ---- description (#284) ------------------------------------------------

    #[test]
    fn description_is_optional() {
        let sets = parsed(&format!(
            "{MINIMAL}[sets.b]\ndescription = 'Rust source'\n[[sets.b.filters]]\npattern = 'y'\n"
        ));
        let a = sets.iter().find(|set| set.name == "a").expect("a");
        let b = sets.iter().find(|set| set.name == "b").expect("b");
        assert_eq!(a.description, None);
        assert_eq!(b.description.as_deref(), Some("Rust source"));
    }

    #[test]
    fn a_builtin_set_table_accepts_description() {
        let sets = parsed("[sets.definitions]\ndescription = 'mine'\n");
        assert!(sets[0].builtin);
        assert_eq!(sets[0].description.as_deref(), Some("mine"));
    }

    #[test]
    fn a_description_of_more_than_one_line_is_refused() {
        let message = rejected(
            "[sets.a]\ndescription = \"\"\"one\ntwo\"\"\"\n[[sets.a.filters]]\npattern = 'x'\n",
        );
        assert!(message.contains("one line"), "{message}");
    }

    // ---- RECON_FILTER_PATH (#46) -------------------------------------------

    /// A fresh directory under `target/`, emptied first so a rerun starts
    /// from what the test writes and nothing else.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = Path::new("target/test-config/filter-path").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    fn set_file(name: &str, pattern: &str) -> String {
        format!("[sets.{name}]\n[[sets.{name}.filters]]\npattern = '{pattern}'\n")
    }

    #[test]
    fn the_path_is_each_entry_in_order() {
        assert_eq!(
            search_path_from(Some("/team:/proj/.recon"), Some("/h")),
            [PathBuf::from("/team"), PathBuf::from("/proj/.recon")]
        );
        assert!(search_path_from(None, Some("/h")).is_empty());
        assert!(search_path_from(Some(""), Some("/h")).is_empty());
    }

    /// An empty entry means the current directory to `PATH`. Here it means
    /// nothing: a directory is read only when it is named.
    #[test]
    fn an_empty_entry_is_not_the_current_directory() {
        assert_eq!(
            search_path_from(Some(":/team::"), None),
            [PathBuf::from("/team")]
        );
    }

    /// Only a shell's first `~` in an assignment is expanded, and zsh
    /// expands none after a colon; recon expands each entry's itself.
    #[test]
    fn a_leading_tilde_is_home() {
        assert_eq!(
            search_path_from(Some("~/team:~"), Some("/h")),
            [PathBuf::from("/h/team"), PathBuf::from("/h")]
        );
        assert_eq!(
            search_path_from(Some("~/team"), None),
            [PathBuf::from("~/team")],
            "with no home, the entry is left as written"
        );
    }

    #[test]
    fn a_directory_named_twice_is_read_once() {
        assert_eq!(
            search_path_from(Some("/team:/other:/team/:/team/x/.."), None),
            [PathBuf::from("/team"), PathBuf::from("/other")]
        );
    }

    #[test]
    fn a_directory_gives_its_set_files_in_name_order() {
        let root = scratch_dir("listing");
        let team = root.join("team");
        for name in [
            "triage.filters.toml",
            "deploy.filters.toml",
            "filters.toml",
            ".filters.toml",
            "notes.toml",
            "oldfilters.toml",
            "errors.filters.toml.bak",
        ] {
            write(&team.join(name), "");
        }
        std::fs::create_dir_all(team.join("dir.filters.toml")).expect("mkdir");
        let own = root.join("own").join(FILE);
        let files = set_files(Some(own.clone()), std::slice::from_ref(&team)).expect("lists");
        assert_eq!(
            files,
            [
                own,
                team.join("deploy.filters.toml"),
                team.join("filters.toml"),
                team.join("triage.filters.toml"),
            ]
        );
    }

    #[test]
    fn a_missing_directory_on_the_path_is_no_files() {
        let root = scratch_dir("missing");
        let files = set_files(None, &[root.join("nowhere")]).expect("not an error");
        assert!(files.is_empty());
    }

    #[test]
    fn a_file_in_a_directorys_place_is_an_error() {
        let root = scratch_dir("not-a-dir");
        let file = root.join("team");
        write(&file, "");
        let err = set_files(None, std::slice::from_ref(&file)).expect_err("refused");
        assert!(matches!(err, Error::ReadDir { .. }));
        assert!(err.to_string().contains("directory"), "{err}");
    }

    #[test]
    fn an_earlier_file_shadows_a_later_one() {
        let root = scratch_dir("shadow");
        let own = root.join("own").join(FILE);
        let team = root.join("team");
        write(&own, &set_file("bug", "mine"));
        write(
            &team.join("a.filters.toml"),
            &format!("{}{}", set_file("bug", "theirs"), set_file("wifi", "wlan")),
        );
        write(&team.join("b.filters.toml"), &set_file("wifi", "later"));
        let files = set_files(Some(own.clone()), std::slice::from_ref(&team)).expect("lists");
        let sets = load_all(&files).expect("loads");
        let names: Vec<&str> = sets.iter().map(|set| set.name.as_str()).collect();
        assert_eq!(names, ["bug", "definitions", "wifi"]);
        assert_eq!(sets[0].filters[0].predicate.display(), "mine");
        assert_eq!(sets[0].path, own);
        assert_eq!(sets[2].filters[0].predicate.display(), "wlan");
        assert_eq!(sets[2].path, team.join("a.filters.toml"));
    }

    /// The built-in set's table shadows like any other, so the first file
    /// that names it decides it, and a file that does not name it does not
    /// hide a later one that does.
    #[test]
    fn the_builtin_table_comes_from_the_first_file_that_names_it() {
        let root = scratch_dir("builtin");
        let (own, team) = (root.join("own.toml"), root.join("team.toml"));
        write(&own, &set_file("a", "x"));
        write(&team, "[sets.definitions]\npriority = 7\n");
        let sets = load_all(&[own, team]).expect("loads");
        let builtins: Vec<&LoadedSet> = sets.iter().filter(|set| set.builtin).collect();
        assert_eq!(builtins.len(), 1);
        assert_eq!(builtins[0].priority, 7);
    }

    #[test]
    fn a_bad_file_on_the_path_refuses_with_its_own_path() {
        let root = scratch_dir("bad");
        let (own, team) = (root.join("own.toml"), root.join("team.toml"));
        write(&own, &set_file("a", "x"));
        write(&team, "[sets.b]\n");
        let err = load_all(&[own, team.clone()]).expect_err("refused");
        assert!(
            err.to_string().contains(&team.display().to_string()),
            "{err}"
        );
    }
}
