//! Where a setting's value comes from, and what happens when the answer is
//! wrong.
//!
//! The full reasoning lives in
//! `docs/specs/2026-08-22-configuration-mechanism.md`. The short version:
//!
//! ```text
//! CLI flags  >  environment variables  >  config file  >  compiled-in defaults
//! ```
//!
//! The first two boundaries are `clap`'s job — its `env` feature makes
//! `#[arg(long, env = "RECON_FOO")]` resolve them with no code here. This
//! module owns the third: finding `config.toml`, parsing it, and folding it
//! *under* whatever the CLI and environment already decided.
//!
//! #18 delivered the mechanism with an empty schema; every actual setting lands
//! in its own issue against it. The first two are `editor.project` and
//! `editor.file` (#42, #41), which is why this module now has a nested section
//! to merge rather than nothing at all. `filters.palette` (#62) is the third,
//! and the first that is a list rather than a string — see [`FiltersConfig`]
//! for why it replaces the built-in palette wholesale, and
//! `non_empty_palette` for the one value it has to refuse.

use crate::editor;
use clap::Parser;
use ratatui::style::Color;
use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};

/// Directory under the config home that recon owns.
const CONFIG_DIR: &str = "recon";

/// The one file recon reads. Never written — see `Cargo.toml`.
const CONFIG_FILE: &str = "config.toml";

/// The fully resolved configuration the app runs on.
///
/// This is the *result* of the precedence chain, not one layer of it. `path`
/// is CLI-only by design: a positional argument naming what to open is a
/// per-invocation fact, and a default for it in a config file would make
/// `recon` with no arguments open something other than the current directory,
/// which is a surprise no setting is worth.
// `about = None` rather than the bare `about` clap defaults to: this struct
// now carries a doc comment, and a bare `about` would promote it to the
// description line of `--help`. The prose below is for whoever maintains the
// precedence chain, not for someone running `recon --help`.
#[derive(Parser, Debug)]
#[command(version, about = None, long_about = None)]
pub struct Config {
    /// File or directory to open. A directory is listed with its first entry
    /// selected; a file is opened with its own directory listed alongside.
    #[arg(default_value = ".")]
    pub path: String,

    /// Command template `o` runs, e.g. `zed {project} {file}:{line}`.
    ///
    /// Falls back to the `[editor]` stanza in `config.toml`, then `$VISUAL`,
    /// then `$EDITOR`, then a built-in default.
    //
    // Below the `///` line on purpose (#91): everything from here down is for
    // whoever maintains the precedence chain, and clap would print a `///` verbatim
    // into `--help`. Same split, and the same reason, as the `about = None` on the
    // struct above.
    //
    // `Option`, not a `default_value`: the compiled-in default lives at the bottom
    // of `editor::Templates::resolve`'s ladder, below `$VISUAL` and `$EDITOR`. A
    // clap default would fill this in before the file layer ever ran and win every
    // argument it was never meant to enter.
    #[arg(long, env = "RECON_EDITOR", value_name = "TEMPLATE")]
    pub editor: Option<String>,

    /// Command template `O` runs. Defaults to `--editor` with the `{project}`
    /// argument dropped, so one setting normally configures both keys.
    #[arg(
        long = "file-editor",
        env = "RECON_FILE_EDITOR",
        value_name = "TEMPLATE"
    )]
    pub file_editor: Option<String>,

    /// Print a ready-to-paste `[editor]` stanza and exit. Takes a flavour —
    /// `zed`, `vscode`, `wezterm-nvim`, … — or `auto` to guess from `$TERM_PROGRAM`.
    ///
    /// Prints to stdout and changes nothing on disk: copy the stanza into your
    /// `config.toml` yourself.
    //
    // The user-facing half of "recon never writes `config.toml`" (#91). The
    // decision itself is enforced in `Cargo.toml`, which drops toml's `display`
    // feature so the serializer does not exist in this build — a reader of
    // `--help` needs the promise, not the mechanism.
    #[arg(
        long,
        value_name = "FLAVOUR",
        num_args = 0..=1,
        default_missing_value = "auto",
    )]
    pub print_editor_config: Option<String>,

    /// The colours successive filters take, or `None` to use the compiled-in
    /// palette. See [`FiltersConfig::palette`].
    ///
    /// `#[arg(skip)]` because there is deliberately no flag for this: a palette
    /// is a settled preference, not a per-invocation decision, and a six-colour
    /// list is a miserable thing to type at a shell. It still lives on `Config`
    /// rather than being read straight from the file, so that every resolved
    /// setting is reachable from one place — and so that adding a flag later is
    /// a one-line change here rather than a new path through the app.
    #[arg(skip)]
    pub filter_palette: Option<Vec<Color>>,
}

/// The clap defaults, restated for the tests and callers that build a `Config`
/// directly rather than parsing one.
///
/// The two must agree, which `default_matches_the_parsed_defaults` pins. A
/// hand-written impl rather than `#[derive(Default)]` because `String::default`
/// is `""` and clap's default for `path` is `"."` — deriving would silently
/// introduce a second, different notion of "no arguments".
impl Default for Config {
    fn default() -> Self {
        Self {
            path: ".".to_string(),
            editor: None,
            file_editor: None,
            print_editor_config: None,
            filter_palette: None,
        }
    }
}

/// The config file's contents, as parsed.
///
/// `Deserialize` only. Nothing serializes this — the file is hand-edited and
/// recon never writes it.
#[derive(Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// `[editor]`. Absent when the file does not mention editors at all, which
    /// is different from present-but-empty only in that neither sets anything —
    /// both leave the ladder to the layers below.
    pub editor: Option<EditorConfig>,
    /// `[filters]`. Same absent-vs-empty reasoning as `editor`.
    pub filters: Option<FiltersConfig>,
}

/// The `[filters]` table.
#[derive(Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FiltersConfig {
    /// The colours successive filters take, replacing the built-in palette
    /// **wholesale** rather than slot by slot.
    ///
    /// Whole-list replacement because the list's *length* is a setting too: it
    /// decides when colours start repeating. A per-slot merge could not express
    /// "give me three colours and cycle them", and would leave a user who
    /// overrode two entries still guessing what the other four were.
    ///
    /// Each entry is written as a *string*, in one of three forms: a colour
    /// name (`red`, `bright-white`), a hex triple (`#00FF00`), or a 256-colour
    /// index (`'220'`). The index has to be quoted — bare `220` is a TOML
    /// integer, and this list is of strings.
    ///
    /// `Color` rather than `String` because a config file's job is to fail at
    /// *parse* time: storing the spellings would push "that isn't a colour"
    /// past startup and into the first filter the user adds. See
    /// `non_empty_palette`, which does the conversion and owns the message.
    #[serde(default, deserialize_with = "non_empty_palette")]
    pub palette: Option<Vec<Color>>,
}

/// The colour names `Color::from_str` accepts, for the error message. Kept
/// here rather than derived because ratatui offers no way to enumerate them,
/// and a message that lists nothing is the problem this exists to fix.
///
/// `bright-` is accepted as a synonym for `light-`, and spaces, dashes and
/// underscores are all ignored, so `light red`, `light-red` and `lightred` are
/// one name. Naming any of these is a *theme-dependent* choice — see
/// `filter::DEFAULT_PALETTE` for why recon's own defaults do not.
const COLOUR_NAMES: &str = "black, red, green, yellow, blue, magenta, cyan, \
     gray, dark-gray, light-red, light-green, light-yellow, light-blue, \
     light-magenta, light-cyan, white";

/// Parse the palette, refusing an empty list and explaining a bad colour.
///
/// **Empty.** `palette = []` is the one value the renderer cannot use: filter
/// colours are picked by `index % palette.len()`, so an empty palette divides
/// by zero on the *first* filter the user adds — well after startup, with a
/// file already open and nothing on screen to connect the crash to a config
/// file. Failing here means the message names the file and recon never starts.
///
/// **Unparseable.** Entries arrive as `String` and go through
/// `Color::from_str` here rather than via ratatui's own `Deserialize`,
/// purely for the error message: ratatui's is "Failed to parse Colors", which
/// never says what a *good* value looks like. In a file the user hand-edits,
/// that is the whole difference between fixing the typo and going to read
/// ratatui's source. The accepted forms are identical either way.
///
/// `default` alongside `deserialize_with` because the latter would otherwise
/// make the key mandatory, which is the opposite of what an `Option` says.
fn non_empty_palette<'de, D>(deserializer: D) -> Result<Option<Vec<Color>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use std::str::FromStr;

    let Some(spellings) = Option::<Vec<String>>::deserialize(deserializer)? else {
        return Ok(None);
    };

    if spellings.is_empty() {
        return Err(D::Error::custom(
            "[filters] palette needs at least one colour; omit the key entirely \
             to use recon's built-in palette",
        ));
    }

    spellings
        .iter()
        .map(|spelling| {
            Color::from_str(spelling).map_err(|_| {
                D::Error::custom(format!(
                    "{spelling:?} is not a colour. Use a name ({COLOUR_NAMES}), \
                     a hex triple (#RRGGBB), or a 256-colour index as a string \
                     (\"0-255\", e.g. \"220\")"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The `[editor]` table.
///
/// One key per binding, because `o` and `O` answer genuinely different
/// questions and a user may well want `-n` on one of them. `file` is optional
/// even so: leaving it out derives it from `project`, which is the case one
/// line of config is meant to cover.
#[derive(Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditorConfig {
    /// The template `o` runs.
    pub project: Option<String>,
    /// The template `O` runs.
    pub file: Option<String>,
}

/// Why a config file could not be turned into a [`FileConfig`].
///
/// Both variants carry the path. An error that says "invalid config" without
/// saying *which file* is nearly useless when `$XDG_CONFIG_HOME` is in play
/// and the user is not sure which of two files recon actually found.
#[derive(Debug)]
pub enum ConfigError {
    /// The file exists but could not be read — permissions, or a directory
    /// where a file was expected. A missing file is not an error and never
    /// reaches here.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file was read but is not valid TOML, or holds a key the schema
    /// does not define.
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "could not read config file {}: {source}", path.display())
            }
            // On its own line: `toml`'s error renders as a multi-line snippet
            // with a caret under the offending span, which is the most useful
            // part of the message and reads badly jammed after a colon.
            Self::Parse { path, source } => {
                write!(f, "invalid config file {}\n{source}", path.display())
            }
        }
    }
}

/// Deliberately no `source()`. [`fmt::Display`] already renders the underlying
/// error, and `color_eyre` prints the chain as well as the message — reporting
/// it in both places would show the same TOML snippet twice on a screen the
/// user is reading in a hurry.
impl std::error::Error for ConfigError {}

/// Resolve the config file's path from the two environment variables that
/// decide it.
///
/// Taking the environment as arguments rather than reading it is what keeps
/// this testable: `std::env::set_var` is process-global and `unsafe` in edition
/// 2024, and this repo's tests run in parallel. See the spec's "Testing
/// precedence" for the rule and for what to do when a test genuinely must set a
/// real variable.
fn config_path_from(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let config_home = xdg_config_home
        .filter(|dir| !dir.is_empty())
        .map(Path::new)
        // The XDG base directory specification requires an absolute path and
        // says a relative one is invalid and must be ignored. Resolving it
        // instead would make the config recon loads depend on the directory
        // the shell happened to launch it from.
        .filter(|dir| dir.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| {
            home.filter(|dir| !dir.is_empty())
                .map(|dir| Path::new(dir).join(".config"))
        })?;

    Some(config_home.join(CONFIG_DIR).join(CONFIG_FILE))
}

/// Where recon looks for `config.toml`, or `None` when the environment names
/// no home to look in.
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    config_path_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Read and parse one config file. A file that is not there yields defaults.
fn load_from(path: &Path) -> Result<FileConfig, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // Having written no config file is the overwhelmingly common case and
        // is not a failure — recon runs on compiled-in defaults. Only this one
        // `io::ErrorKind` is forgiven; a permission error or a directory in
        // the file's place is a real problem and is reported as one.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileConfig::default());
        }
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// The file layer of the precedence chain.
pub fn load_file() -> Result<FileConfig, ConfigError> {
    // Nowhere to look is the same outcome as nothing to find.
    let Some(path) = config_path() else {
        log::debug!("no config home ($XDG_CONFIG_HOME, $HOME unset); no config file read");
        return Ok(FileConfig::default());
    };
    // Which file was actually read is the first thing anyone asks when a
    // setting does not take effect, and the answer depends on
    // `$XDG_CONFIG_HOME` and `$HOME` — neither of which is visible from the
    // symptom. Absence is logged too: "no config file" and "the wrong config
    // file" look identical from the outside (#83).
    if path.exists() {
        log::debug!("reading config from {}", path.display());
    } else {
        log::debug!("no config file at {}", path.display());
    }
    load_from(&path)
}

impl Config {
    /// Run the whole precedence chain: parse the CLI (which `clap` has already
    /// resolved against the environment), read the config file, and fold the
    /// file in underneath.
    ///
    /// Fails rather than warns. recon enters raw mode and the alternate screen
    /// moments after this returns, so a warning printed and then continued is
    /// wiped off the screen before it can be read — "warn and carry on" is
    /// "carry on silently" in practice. Call this **before** the terminal is
    /// initialised.
    pub fn load() -> Result<Self, ConfigError> {
        let mut config = Self::parse();
        config.apply(&load_file()?);
        Ok(config)
    }

    /// Fold the file layer under the layers already resolved.
    ///
    /// The exhaustive destructure is the point: add a field to [`FileConfig`]
    /// and this stops compiling until the new setting is actually merged here,
    /// so a setting cannot be added to the file format and silently never
    /// applied.
    ///
    /// `get_or_insert_with` is the direction of the whole chain in one call —
    /// the file only fills a hole the CLI and environment left. Assigning would
    /// invert it and make `config.toml` beat `--editor`.
    ///
    /// Each section merges inside its own `if let` rather than after an early
    /// `return`. One `let ... else { return }` per section would make a file
    /// that sets `[filters]` but no `[editor]` skip the filter merge entirely —
    /// a bug whose symptom is "my setting parses fine and does nothing".
    fn apply(&mut self, file: &FileConfig) {
        let FileConfig { editor, filters } = file;

        if let Some(EditorConfig { project, file }) = editor {
            if let Some(project) = project {
                self.editor.get_or_insert_with(|| project.clone());
            }
            if let Some(file) = file {
                self.file_editor.get_or_insert_with(|| file.clone());
            }
        }

        if let Some(FiltersConfig { palette }) = filters
            && let Some(palette) = palette
        {
            self.filter_palette.get_or_insert_with(|| palette.clone());
        }
    }

    /// Resolve both editor templates, running the rungs below the config file.
    ///
    /// Reading `$VISUAL`/`$EDITOR` here rather than in `Config::load` keeps
    /// [`editor::Templates::resolve`] pure and testable — the same rule
    /// `config_path` follows for `$XDG_CONFIG_HOME`.
    #[must_use]
    pub fn editor_templates(&self) -> editor::Templates {
        editor::Templates::resolve(
            self.editor.as_deref(),
            self.file_editor.as_deref(),
            std::env::var("VISUAL").ok().as_deref(),
            std::env::var("EDITOR").ok().as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Every fixture name claimed so far in this process, so two tests cannot
    /// race to write one path. Same guard, and same reasoning, as
    /// `fileview.rs`'s `FIXTURE_NAMES`.
    static CONFIG_FIXTURE_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn claim_fixture_name(name: &str) {
        let mut names = CONFIG_FIXTURE_NAMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !names.iter().any(|used| used == name),
            "config fixture name {name:?} is already in use by another test — pick a unique name"
        );
        names.push(name.to_string());
    }

    /// Write a config fixture under `target/` so tests never depend on the
    /// developer's real `~/.config/recon/config.toml`.
    fn fixture(name: &str, contents: &str) -> PathBuf {
        claim_fixture_name(name);
        let dir = Path::new("target/test-config");
        fs::create_dir_all(dir).expect("create config fixture dir");
        let path = dir.join(name);
        fs::write(&path, contents).expect("write config fixture");
        path
    }

    // ---- what `--help` shows a user ------------------------------------

    /// The width the README's `## Usage` block is rendered at.
    ///
    /// clap wraps to the terminal when it has one and not at all when it does
    /// not, so a raw `recon -h` redirected to a file produces 169-column lines.
    /// Pinning a width here is what makes the block both readable on GitHub and
    /// byte-comparable in `readme_usage_block_matches_the_real_help` below. 80
    /// matches the widest fenced block already in the README.
    ///
    /// Applied only when rendering for the README, never to the command the app
    /// actually runs — a real `recon -h` should still wrap to the user's own
    /// terminal.
    const README_HELP_WIDTH: usize = 80;

    /// The README's `## Usage` block, as embedded at compile time.
    ///
    /// `include_str!` rather than reading the file at run time, matching
    /// `help.rs`'s `SOURCES`: no working-directory assumption, and the test
    /// cannot pass by silently failing to find the file.
    const README: &str = include_str!("../README.md");

    /// Pull the first fenced block after the `## Usage` heading out of `README`.
    fn readme_usage_block() -> String {
        let after_heading = README
            .split_once("\n## Usage\n")
            .expect("README has no `## Usage` heading")
            .1;
        let fenced = after_heading
            .split_once("```\n")
            .expect("no fenced block after `## Usage`")
            .1;
        fenced
            .split_once("```")
            .expect("unterminated fenced block after `## Usage`")
            .0
            .trim_end()
            .to_string()
    }

    /// The README's `## Usage` block is the CLI's contract, and it drifted three
    /// flags behind the binary (#92) — `--editor`, `--file-editor` and
    /// `--print-editor-config` were all missing, and so was `[OPTIONS]` in the
    /// usage line itself. Every one of them was documented at length further
    /// down the README, so this was drift in the one block a reader treats as
    /// authoritative rather than a gap in coverage.
    ///
    /// This is `help.rs`'s `every_bound_key_is_documented` applied to the CLI
    /// surface — that module's own doc names generating README sections as "the
    /// obvious next step", and this is that step for the flags. Comparing
    /// rendered output rather than checking for flag names means a *changed*
    /// description fails too, not only an added flag.
    #[test]
    fn readme_usage_block_matches_the_real_help() {
        use clap::CommandFactory;
        let rendered = Config::command()
            .term_width(README_HELP_WIDTH)
            .render_help()
            .to_string();
        let rendered = rendered.trim_end();

        assert_eq!(
            readme_usage_block(),
            rendered,
            "\nREADME's `## Usage` block is out of date. Replace it with:\n\
             \n```\n{rendered}\n```\n"
        );
    }

    /// `--help` is end-user documentation, and rationale aimed at whoever
    /// maintains the precedence chain must not leak into it (#91).
    ///
    /// The struct itself already had this right — `about = None` is there so
    /// its doc comment stays out of `--help` — but the same split was never
    /// carried down to the fields, so `--editor` printed a paragraph about why
    /// it is an `Option` rather than a `default_value`, complete with a rustdoc
    /// intra-doc link rendered raw at a private module path.
    ///
    /// Checks the rendered text rather than the source: a `//` comment that
    /// drifts back to `///` is exactly the regression, and only clap's own
    /// output can see it. `-h` was never affected — clap takes only the first
    /// paragraph — so this asserts against the long help specifically.
    #[test]
    fn long_help_carries_no_maintainer_rationale() {
        use clap::CommandFactory;
        let help = Config::command().render_long_help().to_string();

        // A raw intra-doc link. Renders as a hyperlink in rustdoc and as
        // literal brackets-and-backticks in a terminal, always at a path the
        // reader cannot reach.
        assert!(
            !help.contains("[`"),
            "`--help` contains a rustdoc intra-doc link:\n{help}"
        );
        // Phrases that only mean anything to someone reading this file.
        for jargon in [
            "default_value",
            "clap default",
            "the file layer",
            "`Option`, not a",
        ] {
            assert!(
                !help.contains(jargon),
                "`--help` explains {jargon:?} to end users:\n{help}"
            );
        }
    }

    /// The flags still document themselves — the fix is to move the rationale
    /// out, not to strip the help text down to nothing.
    #[test]
    fn long_help_still_describes_every_flag() {
        use clap::CommandFactory;
        let help = Config::command().render_long_help().to_string();

        for expected in [
            "Command template `o` runs",
            "Command template `O` runs",
            "$VISUAL",
            "ready-to-paste",
        ] {
            assert!(
                help.contains(expected),
                "`--help` no longer mentions {expected:?}:\n{help}"
            );
        }
    }

    // ---- the CLI layer -------------------------------------------------

    /// The argument is optional and defaults to the current directory, so
    /// bare `recon` is `recon .`.
    #[test]
    fn no_argument_defaults_to_the_current_directory() {
        let config = Config::try_parse_from(["recon"]).expect("parses with no argument");

        assert_eq!(config.path, ".");
    }

    #[test]
    fn an_explicit_argument_still_wins() {
        let config = Config::try_parse_from(["recon", "some/path.log"]).expect("parses");

        assert_eq!(config.path, "some/path.log");
    }

    // ---- path resolution ----------------------------------------------

    #[test]
    fn xdg_config_home_wins_over_home() {
        let path = config_path_from(Some("/xdg"), Some("/home/pete"));
        assert_eq!(path, Some(PathBuf::from("/xdg/recon/config.toml")));
    }

    #[test]
    fn falls_back_to_home_dot_config() {
        let path = config_path_from(None, Some("/home/pete"));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/pete/.config/recon/config.toml"))
        );
    }

    /// The macOS decision, pinned as a test rather than left to a crate.
    /// `directories` would return `~/Library/Application Support/recon` here;
    /// recon deliberately uses `~/.config` on every platform, because that is
    /// where users of a terminal tool look for a file they hand-edit.
    #[test]
    fn macos_uses_dot_config_not_application_support() {
        let path = config_path_from(None, Some("/Users/pete"))
            .expect("a home directory yields a config path");
        assert_eq!(path, PathBuf::from("/Users/pete/.config/recon/config.toml"));
        assert!(
            !path.to_string_lossy().contains("Application Support"),
            "recon must not use the macOS app-support directory: {path:?}"
        );
    }

    /// The XDG spec says a relative path in one of its variables is invalid and
    /// must be ignored. Honouring that matters because the fallback is a real,
    /// working location — silently resolving `recon/config.toml` against
    /// whatever directory recon was launched from would make the config load
    /// depend on the shell's cwd.
    #[test]
    fn relative_xdg_config_home_is_ignored() {
        let path = config_path_from(Some("relative/dir"), Some("/home/pete"));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/pete/.config/recon/config.toml"))
        );
    }

    /// An exported-but-empty variable is the common shell accident
    /// (`export XDG_CONFIG_HOME=$SOMETHING_UNSET`) and reads as unset.
    #[test]
    fn empty_xdg_config_home_is_ignored() {
        let path = config_path_from(Some(""), Some("/home/pete"));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/pete/.config/recon/config.toml"))
        );
    }

    #[test]
    fn empty_home_is_ignored() {
        assert_eq!(config_path_from(None, Some("")), None);
    }

    /// No home, no config file — and no error either. recon still runs on
    /// compiled-in defaults.
    #[test]
    fn no_home_yields_no_path() {
        assert_eq!(config_path_from(None, None), None);
    }

    // ---- loading -------------------------------------------------------

    /// The overwhelmingly common case: nobody has written a config file. It
    /// must not be an error, or recon would refuse to start out of the box.
    #[test]
    fn missing_file_is_not_an_error() {
        let path = Path::new("target/test-config/definitely-not-here.toml");
        assert_eq!(
            load_from(path).expect("missing file is fine"),
            FileConfig::default()
        );
    }

    #[test]
    fn empty_file_parses_to_defaults() {
        let path = fixture("empty.toml", "");
        assert_eq!(
            load_from(&path).expect("empty file is valid"),
            FileConfig::default()
        );
    }

    /// Comments are the entire reason TOML was chosen over JSON, so a file that
    /// is nothing but comments has to be valid.
    #[test]
    fn comments_only_file_parses_to_defaults() {
        let path = fixture("comments.toml", "# recon config\n# nothing set yet\n");
        assert_eq!(
            load_from(&path).expect("comments are valid"),
            FileConfig::default()
        );
    }

    #[test]
    fn malformed_toml_is_a_parse_error_naming_the_file() {
        let path = fixture("malformed.toml", "this is not = = toml\n");
        let err = load_from(&path).expect_err("malformed TOML must fail");
        assert!(
            matches!(err, ConfigError::Parse { .. }),
            "expected a parse error, got {err:?}"
        );
        let rendered = err.to_string();
        assert!(
            rendered.contains("malformed.toml"),
            "the error must name the file it came from: {rendered}"
        );
    }

    /// A typo'd key in otherwise valid TOML. Left to serde's default this
    /// would parse cleanly and the setting would simply never apply — the
    /// most confusing config failure there is.
    #[test]
    fn unknown_key_is_rejected_and_named() {
        let path = fixture("unknown-key.toml", "nav_wdith = 20\n");
        let err = load_from(&path).expect_err("an unknown key must fail");
        assert!(
            matches!(err, ConfigError::Parse { .. }),
            "expected a parse error, got {err:?}"
        );
        let rendered = err.to_string();
        assert!(
            rendered.contains("nav_wdith"),
            "the error must name the offending key: {rendered}"
        );
        assert!(
            rendered.contains("unknown-key.toml"),
            "the error must name the file: {rendered}"
        );
    }

    /// A directory where a file is expected is readable-but-not-a-file, which
    /// is the `Read` variant rather than `Parse`. Distinguishing them matters:
    /// one means "fix your TOML", the other means "fix your filesystem".
    #[test]
    fn unreadable_file_is_a_read_error_naming_the_file() {
        let dir = Path::new("target/test-config/a-directory.toml");
        fs::create_dir_all(dir).expect("create directory fixture");
        let err = load_from(dir).expect_err("a directory is not readable as a file");
        assert!(
            matches!(err, ConfigError::Read { .. }),
            "expected a read error, got {err:?}"
        );
        assert!(
            err.to_string().contains("a-directory.toml"),
            "the error must name the file: {err}"
        );
    }

    // ---- merge ---------------------------------------------------------

    /// The positional argument is CLI-only, so it must survive `apply`
    /// untouched however much the file holds.
    #[test]
    fn apply_leaves_the_cli_layer_alone() {
        let mut config = Config {
            path: "src/lib.rs".to_string(),
            ..Config::default()
        };
        config.apply(&FileConfig::default());
        assert_eq!(config.path, "src/lib.rs");
    }

    /// The clap defaults and the hand-written `Default` are two statements of
    /// the same thing, and this is what stops them drifting.
    #[test]
    fn default_matches_the_parsed_defaults() {
        let parsed = Config::try_parse_from(["recon"]).expect("parses with no argument");
        let default = Config::default();
        assert_eq!(parsed.path, default.path);
        assert_eq!(parsed.editor, default.editor);
        assert_eq!(parsed.file_editor, default.file_editor);
        assert_eq!(parsed.print_editor_config, default.print_editor_config);
    }

    // ---- the editor settings --------------------------------------------

    #[test]
    fn the_editor_flags_parse() {
        let config = Config::try_parse_from([
            "recon",
            "--editor",
            "code {project} -g {file}:{line}",
            "--file-editor",
            "code -g {file}:{line}",
        ])
        .expect("parses");
        assert_eq!(
            config.editor.as_deref(),
            Some("code {project} -g {file}:{line}")
        );
        assert_eq!(config.file_editor.as_deref(), Some("code -g {file}:{line}"));
    }

    #[test]
    fn an_editor_section_parses() {
        let path = fixture(
            "editor.toml",
            "[editor]\nproject = 'zed {project} {file}:{line}'\nfile = 'zed -n {file}:{line}'\n",
        );
        let parsed = load_from(&path).expect("valid");
        let editor = parsed.editor.expect("the section is present");
        assert_eq!(
            editor.project.as_deref(),
            Some("zed {project} {file}:{line}")
        );
        assert_eq!(editor.file.as_deref(), Some("zed -n {file}:{line}"));
    }

    /// One line of config is meant to configure both keys, so `file` has to be
    /// optional inside a section that sets `project`.
    #[test]
    fn an_editor_section_may_set_project_alone() {
        let path = fixture(
            "editor-project-only.toml",
            "[editor]\nproject = 'subl {file}'\n",
        );
        let editor = load_from(&path).expect("valid").editor.expect("present");
        assert_eq!(editor.project.as_deref(), Some("subl {file}"));
        assert_eq!(editor.file, None);
    }

    /// `deny_unknown_fields` applies to the nested table too — a typo inside
    /// `[editor]` is exactly as invisible as one at the top level.
    #[test]
    fn an_unknown_key_inside_editor_is_rejected_and_named() {
        let path = fixture("editor-typo.toml", "[editor]\nporject = 'zed {file}'\n");
        let err = load_from(&path).expect_err("a typo'd key must fail");
        assert!(err.to_string().contains("porject"), "{err}");
    }

    /// The direction of the whole chain, on the settings that now have all four
    /// layers. Getting this backwards would make `config.toml` beat `--editor`,
    /// which is the bug `get_or_insert_with` exists to prevent.
    #[test]
    fn the_file_only_fills_holes_the_cli_left() {
        let file = FileConfig {
            editor: Some(EditorConfig {
                project: Some("from-file {file}".to_string()),
                file: Some("from-file-solo {file}".to_string()),
            }),
            ..FileConfig::default()
        };

        let mut set_on_the_cli = Config {
            editor: Some("from-cli {file}".to_string()),
            ..Config::default()
        };
        set_on_the_cli.apply(&file);
        assert_eq!(set_on_the_cli.editor.as_deref(), Some("from-cli {file}"));
        // The hole the CLI left is still filled.
        assert_eq!(
            set_on_the_cli.file_editor.as_deref(),
            Some("from-file-solo {file}")
        );

        let mut unset = Config::default();
        unset.apply(&file);
        assert_eq!(unset.editor.as_deref(), Some("from-file {file}"));
    }

    /// A file with no `[editor]` at all must leave both settings alone rather
    /// than clearing them.
    #[test]
    fn a_file_without_an_editor_section_changes_nothing() {
        let mut config = Config {
            editor: Some("from-cli {file}".to_string()),
            ..Config::default()
        };
        config.apply(&FileConfig::default());
        assert_eq!(config.editor.as_deref(), Some("from-cli {file}"));
    }

    // ---- --print-editor-config -------------------------------------------

    /// Bare, it means "guess"; with a value, it means that flavour. The bare
    /// form is the one people will actually type.
    #[test]
    fn print_editor_config_takes_an_optional_flavour() {
        let bare = Config::try_parse_from(["recon", "--print-editor-config"]).expect("parses");
        assert_eq!(bare.print_editor_config.as_deref(), Some("auto"));

        let named =
            Config::try_parse_from(["recon", "--print-editor-config", "vscode"]).expect("parses");
        assert_eq!(named.print_editor_config.as_deref(), Some("vscode"));

        let absent = Config::try_parse_from(["recon"]).expect("parses");
        assert_eq!(absent.print_editor_config, None);
    }

    // ---- the filter palette ----------------------------------------------

    /// #62's second half. The list replaces the built-in palette wholesale —
    /// see [`FiltersConfig`] for why it is not a per-slot merge.
    #[test]
    fn a_filters_palette_parses() {
        let path = fixture(
            "filters-palette.toml",
            "[filters]\npalette = ['red', 'blue', 'green']\n",
        );
        let filters = load_from(&path).expect("valid").filters.expect("present");
        assert_eq!(
            filters.palette,
            Some(vec![Color::Red, Color::Blue, Color::Green])
        );
    }

    /// All three spellings ratatui's own parser accepts, because a user who
    /// wants a colour outside the sixteen ANSI names needs one of the other
    /// two and should not have to discover which by trial and error.
    #[test]
    fn a_palette_entry_may_be_a_name_a_hex_or_an_index() {
        let path = fixture(
            "filters-palette-forms.toml",
            "[filters]\npalette = ['magenta', '#00FF00', '220']\n",
        );
        let filters = load_from(&path).expect("valid").filters.expect("present");
        assert_eq!(
            filters.palette,
            Some(vec![
                Color::Magenta,
                Color::Rgb(0, 255, 0),
                Color::Indexed(220),
            ])
        );
    }

    #[test]
    fn an_unparseable_colour_is_a_parse_error_naming_the_file() {
        let path = fixture(
            "filters-palette-bad.toml",
            "[filters]\npalette = ['red', 'octarine']\n",
        );
        let err = load_from(&path).expect_err("an unknown colour must fail");
        assert!(
            matches!(err, ConfigError::Parse { .. }),
            "expected a parse error, got {err:?}"
        );
        assert!(
            err.to_string().contains("filters-palette-bad.toml"),
            "the error must name the file: {err}"
        );
    }

    /// ratatui's own message for a bad colour is "Failed to parse Colors",
    /// which does not say what a good one looks like. In a hand-edited config
    /// file that is the difference between a one-second fix and a trip to the
    /// source, so recon replaces it.
    #[test]
    fn an_unparseable_colour_says_what_is_accepted() {
        let path = fixture(
            "filters-palette-guidance.toml",
            "[filters]\npalette = ['octarine']\n",
        );
        let rendered = load_from(&path)
            .expect_err("an unknown colour must fail")
            .to_string();

        assert!(
            rendered.contains("octarine"),
            "the error must quote the offending value: {rendered}"
        );
        for expected in ["#RRGGBB", "0-255", "magenta"] {
            assert!(
                rendered.contains(expected),
                "the error must show the {expected:?} form: {rendered}"
            );
        }
    }

    /// `palette = []` reads as "no colours at all", which is not a thing a
    /// filter list can be rendered with — `ActiveFilters` would divide by zero
    /// on the first filter added, i.e. long after startup, with the file
    /// already open. Rejecting it here trades a panic for a message that names
    /// the file.
    #[test]
    fn an_empty_palette_is_rejected() {
        let path = fixture("filters-palette-empty.toml", "[filters]\npalette = []\n");
        let err = load_from(&path).expect_err("an empty palette must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("at least one colour"),
            "the error must say what is wrong: {rendered}"
        );
    }

    /// `deny_unknown_fields` reaches into `[filters]` too, same as `[editor]`.
    #[test]
    fn an_unknown_key_inside_filters_is_rejected_and_named() {
        let path = fixture("filters-typo.toml", "[filters]\npallete = ['red']\n");
        let err = load_from(&path).expect_err("a typo'd key must fail");
        assert!(err.to_string().contains("pallete"), "{err}");
    }

    /// There is no CLI flag for the palette, so the file layer is the only one
    /// that can set it — but `apply` must still fold it in, or the setting
    /// parses and then silently never applies.
    #[test]
    fn the_file_palette_reaches_the_resolved_config() {
        let file = FileConfig {
            filters: Some(FiltersConfig {
                palette: Some(vec![Color::Red, Color::Blue]),
            }),
            ..FileConfig::default()
        };

        let mut config = Config::default();
        config.apply(&file);
        assert_eq!(config.filter_palette, Some(vec![Color::Red, Color::Blue]));
    }

    /// A file with no `[filters]` leaves the palette unset, which is what makes
    /// the compiled-in default the default.
    #[test]
    fn a_file_without_a_filters_section_leaves_the_palette_unset() {
        let mut config = Config::default();
        config.apply(&FileConfig::default());
        assert_eq!(config.filter_palette, None);
    }
}
