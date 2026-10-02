//! What recon reads before it starts, and in which order (#409).
//!
//! `Config` is the command line with the environment and `config.toml`
//! folded in. The keymap in force, its warnings and the filter sets are
//! answers computed *from* it, in an order that matters: each print command
//! must be answered before the first file it does not need, and every
//! refusal must happen before the terminal is taken. The order lives here,
//! not in `main`, and [`start`] hands back everything at once.

use crate::config::{Config, PrintKeymap};
use crate::filter::LoadedSet;
use crate::keymap::Keymap;
use color_eyre::Result;

/// Everything a session needs, once start-up has run.
pub struct Startup {
    /// The resolved settings.
    pub config: Config,
    /// Every binding in force: the defaults with `[keymap]` folded in (#61).
    pub bindings: Keymap,
    /// What the `[keymap]` table cost, for the panel `App` draws on its
    /// first frame, or for stderr on a path with no panel.
    pub keymap_warnings: Vec<String>,
    /// Sets read from each `filters.toml`, in pane order (#128, #46).
    pub filter_sets: Vec<LoadedSet>,
}

/// A `Config` with nothing read from disk: the default keymap, no warnings
/// and no sets. What a test builds when it needs an `App`.
impl From<Config> for Startup {
    fn from(config: Config) -> Self {
        Self {
            config,
            bindings: Keymap::default(),
            keymap_warnings: Vec::new(),
            filter_sets: Vec::new(),
        }
    }
}

/// What [`start`] decided.
pub enum Start {
    /// A print command's answer. recon writes `warnings` to stderr, then
    /// `stdout` to stdout, and exits: stdout keeps the stanza alone, so it
    /// still pastes (#259).
    Print {
        stdout: String,
        warnings: Vec<String>,
    },
    /// Start a session, in the TUI or in batch mode. Boxed because it is the
    /// large one, and `Print` would otherwise be as big.
    Run(Box<Startup>),
}

/// Parse the command line, read what it needs, and refuse what cannot run.
///
/// Call this **before** the terminal is taken. Every error here must reach
/// a screen that a user can read (#143), and anything logged here is logged
/// before `Muted` starts dropping records (#246).
///
/// # Errors
///
/// A flag combination clap cannot express, an unreadable `config.toml` or
/// `filters.toml`, a `[keymap]` recon cannot obey, or a `--set` or
/// `--unlist` naming a set no file defines.
pub fn start() -> Result<Start> {
    let mut config = Config::from_args()?;

    // Before `config.toml` and the filter sets are read, not after: this
    // command prints an `[editor]` stanza and exits, and it uses nothing from
    // either file, so a syntax error in either must not stop it (#191,
    // #366).
    if let Some(flavour) = &config.print_editor_config {
        return Ok(Start::Print {
            stdout: crate::editor::print_editor_config(
                flavour,
                std::env::var("TERM_PROGRAM").ok().as_deref(),
            )?,
            warnings: Vec::new(),
        });
    }

    // `--print-keymap defaults` prints the built-in table, which no
    // `config.toml` can change — so no `config.toml` may stop it. Above
    // `config.load` for that reason: this is the command a user reaches for
    // when recon refuses their keymap, and it was being refused by the very
    // file it exists to diagnose. That is #191's shape, moved off
    // `filters.toml` and onto `config.toml`. It sat above the keymap build
    // alone at first, which still let a TOML syntax error stop it (#366).
    //
    // Plain `--print-keymap` deliberately stays below: it prints the map **in
    // force**, and a file recon cannot obey leaves no map in force to print.
    if config.print_keymap == Some(PrintKeymap::Defaults) {
        let defaults = Keymap::default();
        return Ok(Start::Print {
            stdout: crate::keymap::print_keymap(&defaults, &defaults),
            warnings: Vec::new(),
        });
    }

    let overlay = config.load()?;

    // The `[keymap]` table, before `--print-keymap` below, which prints the
    // answer. An unknown action name, an unreadable key spelling or a keymap
    // recon cannot obey refuses to start while a message can still be read
    // (#61). Above the filter sets for the same reason the two print
    // commands are: this reads nothing from `filters.toml`.
    let (bindings, keymap_warnings) = crate::keymap::config::build(&overlay, config.warnings())?;

    // Same reasoning as `--print-editor-config` above: this command reads
    // nothing from `filters.toml`, so it must not have to survive one to run
    // (#191). It must stay above `load_file` for that reason, which is why
    // the keymap build moved up rather than this moving down.
    //
    // The keymap warnings go to stderr first (#259), the same warnings a
    // normal start shows in its panel — which names this command as where
    // to read them all. Filtered here by the panel's own switch, since these
    // are the same warnings by another route.
    if config.print_keymap.is_some() {
        return Ok(Start::Print {
            stdout: crate::keymap::print_keymap(&bindings, &Keymap::default()),
            warnings: if config.warnings() {
                keymap_warnings
            } else {
                Vec::new()
            },
        });
    }

    let filter_sets = crate::filtersets::load_file(config.filter_path.as_deref())?;
    // Needs the loaded sets, which is why it is not inside `Config::load`
    // with `check_flags`. Still before any terminal setup: the message must
    // reach a screen that is not about to be replaced (#143).
    config.check_sets(&filter_sets)?;

    Ok(Start::Run(Box::new(Startup {
        config,
        bindings,
        keymap_warnings,
        filter_sets,
    })))
}
