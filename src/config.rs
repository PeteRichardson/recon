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
//! The schema is deliberately empty. #18 delivers the mechanism; every actual
//! setting lands in its own issue against it.

use clap::Parser;
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
}

/// The config file's contents, as parsed.
///
/// Empty until the first setting lands, which means **every** key is currently
/// an unknown key. That is correct rather than a gap: there is nothing valid
/// to write in the file yet, so anything in it is a mistake worth reporting.
///
/// `Deserialize` only. Nothing serializes this — the file is hand-edited and
/// recon never writes it.
#[derive(Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {}

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
    match config_path() {
        Some(path) => load_from(&path),
        // Nowhere to look is the same outcome as nothing to find.
        None => Ok(FileConfig::default()),
    }
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
    /// Empty while the schema is. The exhaustive destructure is the point: add
    /// a field to [`FileConfig`] and this stops compiling until the new setting
    /// is actually merged here, so a setting cannot be added to the file format
    /// and silently never applied.
    fn apply(&mut self, file: &FileConfig) {
        let FileConfig {} = file;
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
            .unwrap_or_else(|poison| poison.into_inner());
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
            FileConfig {}
        );
    }

    #[test]
    fn empty_file_parses_to_defaults() {
        let path = fixture("empty.toml", "");
        assert_eq!(
            load_from(&path).expect("empty file is valid"),
            FileConfig {}
        );
    }

    /// Comments are the entire reason TOML was chosen over JSON, so a file that
    /// is nothing but comments has to be valid.
    #[test]
    fn comments_only_file_parses_to_defaults() {
        let path = fixture("comments.toml", "# recon config\n# nothing set yet\n");
        assert_eq!(load_from(&path).expect("comments are valid"), FileConfig {});
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

    /// With an empty schema there is nothing for the file to override, so the
    /// CLI layer must survive `apply` untouched. This test is what will catch
    /// the first setting being merged the wrong way round.
    #[test]
    fn apply_leaves_the_cli_layer_alone() {
        let mut config = Config {
            path: "src/lib.rs".to_string(),
        };
        config.apply(&FileConfig {});
        assert_eq!(config.path, "src/lib.rs");
    }
}
