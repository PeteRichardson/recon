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
//! effective-enabled, profiles as actions — belongs to `filter.rs`.

use crate::config::parse_colour;
use crate::filter::{LoadedFilter, LoadedSet, Predicate, Sense};
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The file, beside `config.toml`.
const FILE: &str = "filters.toml";

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
/// The result is sorted by `(priority, name)`, which is the pane's order.
/// Everything the spec lists as rejected is rejected here, with the set and
/// filter named, so that a user reading the message in a hurry can go
/// straight to the line.
pub fn parse(text: &str, path: &Path) -> Result<Vec<LoadedSet>, Error> {
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
        if let Some(mode) = &schema.mode
            && mode != ONLY_MODE
        {
            return Err(invalid(
                &name,
                None,
                format!("mode {mode:?} is not supported; only {ONLY_MODE:?} is"),
            ));
        }
        // A table naming a built-in set (#127) positions and switches it,
        // and may carry nothing else: its filters are recon's.
        if crate::filter::is_builtin_name(&name) {
            if !schema.filters.is_empty() || !schema.profiles.is_empty() {
                return Err(invalid(
                    &name,
                    None,
                    format!(
                        "{name:?} is a built-in set; its table may set `priority` and \
                         `autoload` only"
                    ),
                ));
            }
            sets.push(LoadedSet {
                name,
                path: path.to_path_buf(),
                priority: schema.priority.unwrap_or(DEFAULT_PRIORITY),
                autoload: schema.autoload.unwrap_or(false),
                profiles: BTreeMap::new(),
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
            let regex = Regex::new(&entry.pattern)
                .map_err(|err| invalid(&name, Some(&entry.pattern), err.to_string()))?;
            let colour = entry
                .colour
                .as_deref()
                .map(parse_colour)
                .transpose()
                .map_err(|message| invalid(&name, Some(&entry.pattern), message))?;
            let display = entry.name.unwrap_or_else(|| entry.pattern.clone());
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
            profiles: schema.profiles,
            filters,
            builtin: false,
        });
    }
    sets.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(sets)
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

/// Read and parse one file. A file that is not there is no sets.
fn load_from(path: &Path) -> Result<Vec<LoadedSet>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // The overwhelmingly common case, and not a failure: recon runs with
        // the scratch set alone. Only this one kind is forgiven — a
        // permission error or a directory in the file's place is real.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(source) => {
            return Err(Error::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    parse(&text, path)
}

/// The file layer. Call before the terminal is initialised: an error here
/// refuses to start, for the reason `Config::load` gives.
pub fn load_file() -> Result<Vec<LoadedSet>, Error> {
    let Some(path) = path() else {
        log::debug!("no config home ($XDG_CONFIG_HOME, $HOME unset); no filters.toml read");
        return Ok(Vec::new());
    };
    // Which file was read is the first thing anyone asks when a set does
    // not appear; absence is logged too, since "no file" and "the wrong
    // file" look identical from the pane (#83).
    if path.exists() {
        log::debug!("reading filter sets from {}", path.display());
    } else {
        log::debug!("no filter sets file at {}", path.display());
    }
    load_from(&path)
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

    #[test]
    fn an_empty_file_has_no_sets() {
        assert!(parsed("").is_empty());
    }

    #[test]
    fn a_minimal_set_takes_every_default() {
        let sets = parsed(MINIMAL);
        assert_eq!(sets.len(), 1);
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
        assert_eq!(names, ["zebra", "alpha", "beta"]);
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

    /// `[sets.definitions]` positions and switches the built-in set (#127).
    #[test]
    fn a_builtin_set_table_carries_priority_and_autoload_only() {
        let sets = parsed("[sets.definitions]\npriority = 80\nautoload = true\n");
        assert_eq!(sets.len(), 1);
        assert!(sets[0].builtin);
        assert_eq!((sets[0].priority, sets[0].autoload), (80, true));
        assert!(sets[0].filters.is_empty());
        assert!(
            rejected("[sets.definitions]\n[[sets.definitions.filters]]\npattern = 'x'\n")
                .contains("built-in")
        );
        assert!(
            rejected("[sets.definitions]\n[sets.definitions.profiles]\ndefault = []\n")
                .contains("built-in")
        );
        // An empty table is fine: it names the set and changes nothing.
        assert!(parsed("[sets.definitions]\n")[0].builtin);
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
        assert_eq!(sets.len(), 2);
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
        assert_eq!(sets[0].filters[0].predicate.display(), "it's");
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
    fn a_missing_file_is_no_sets() {
        let sets =
            load_from(Path::new("target/test-config/no-such-filters.toml")).expect("not an error");
        assert!(sets.is_empty());
    }

    #[test]
    fn a_directory_in_the_files_place_is_an_error() {
        let dir = Path::new("target/test-config/filters-as-a-dir.toml");
        std::fs::create_dir_all(dir).expect("mkdir");
        assert!(matches!(load_from(dir), Err(Error::Read { .. })));
    }
}
