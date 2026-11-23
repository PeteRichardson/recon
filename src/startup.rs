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
    /// The palette's aliases: `config.toml`'s, then each pack's (#627), with
    /// the `pack:name` of one that lost its bare name. `config.aliases` stays
    /// what `config.toml` said, as `config.commands` does.
    pub aliases: std::collections::BTreeMap<String, String>,
    /// What the `[keymap]` table cost, then each pack definition a
    /// collision renamed or dropped (#602), then each view a `[[filetype]]`
    /// row names that cannot run (#457): for the panel `App` draws on its first
    /// frame, or for stderr on a path with no panel.
    pub warnings: Vec<String>,
    /// The view cycle of each file type, with the commands that can run.
    /// Built once, here, so that each command is looked up on `PATH` once.
    pub(crate) table: std::sync::Arc<crate::views::cycle::Table>,
    /// Sets read from each `filters.toml`, in pane order (#128, #46).
    pub filter_sets: Vec<LoadedSet>,
    /// Whether listings draw filetype icons (#512). Off until `main`
    /// resolves the setting against the host terminal, which a test never
    /// does: its rows stay exactly as they were.
    pub icons: crate::filetype::Icons,
}

/// A `Config` with nothing read from disk: the default keymap, no warnings
/// and no sets. The table is built from the config's rows and commands. What a test builds when it needs an `App`.
impl From<Config> for Startup {
    fn from(config: Config) -> Self {
        let (table, _) = crate::views::cycle::Table::build(&config.filetypes, &config.commands);
        Self {
            aliases: config.aliases.clone(),
            config,
            bindings: Keymap::default(),
            warnings: Vec::new(),
            table: std::sync::Arc::new(table),
            filter_sets: Vec::new(),
            icons: crate::filetype::Icons::Off,
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
        /// recon exits 1 after it prints: `--print-startup` found load
        /// errors (#630).
        failed: bool,
    },
    /// Start a session, in the TUI or in batch mode. Boxed because it is the
    /// large one, and `Print` would otherwise be as big.
    Run(Box<Startup>),
}

/// Every error of one load (#630), shown together, in the order recon
/// found them.
#[derive(Debug)]
pub struct LoadErrors(pub Vec<String>);

impl std::fmt::Display for LoadErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let [only] = self.0.as_slice() {
            return f.write_str(only);
        }
        write!(f, "{} errors in the config files:", self.0.len())?;
        for error in &self.0 {
            write!(f, "\n\n{}", error.trim_end())?;
        }
        Ok(())
    }
}

impl std::error::Error for LoadErrors {}

/// Parse the command line, read what it needs, and refuse what cannot run.
///
/// Call this **before** the terminal is taken. Every error here must reach
/// a screen that a user can read (#143), and anything logged here is logged
/// before `Muted` starts dropping records (#246).
///
/// # Errors
///
/// A flag combination clap cannot express, an unreadable `config.toml` or
/// `filters.toml`, a `[keymap]` recon cannot obey, or a `--set`, `--list`
/// or `--unlist` naming a set no file defines.
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
            failed: false,
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
            failed: false,
        });
    }

    let read = std::time::Instant::now();
    let overlay = config.load()?;
    log::debug!(target: crate::config::LOAD_LOG, "config read in {:?}", read.elapsed());

    let read = std::time::Instant::now();
    // From here, an error that leaves the rest of the load intact goes into
    // `errors` and the load goes on, so that one run shows them all (#630).
    let loaded = crate::filtersets::load_file(
        config.filter_path.as_deref(),
        &config.filter_dirs,
        config.print_startup.is_some(),
    );
    let mut errors: Vec<String> = loaded.errors.iter().map(ToString::to_string).collect();
    let filter_sets = loaded.sets;
    log::debug!(
        target: crate::config::LOAD_LOG,
        "{} filter sets read in {:?}",
        filter_sets.len(),
        read.elapsed()
    );

    // The packs' commands, rows (#602) and aliases (#627) join
    // `config.toml`'s. A view no file defines, or an alias the palette
    // cannot run, refuses here, before the terminal is taken (#143). The
    // keymap below and the palette see the one merged list of aliases.
    let config_file = config
        .config_file_path()
        .unwrap_or_else(|| "config.toml".into());
    let mut merged = crate::views::pack::merge_all(
        &config_file,
        &config.filetypes,
        &config.commands,
        &config.aliases,
        &loaded.packs,
    );
    errors.extend(merged.errors.iter().map(ToString::to_string));
    let aliases = std::mem::take(&mut merged.aliases);

    // The `[keymap]` table, before `--print-keymap` below, which prints the
    // answer. An unknown action name, an unreadable key spelling or a keymap
    // recon cannot obey refuses to start while a message can still be read
    // (#61). Below the filter sets: a line can give a pack's alias a key
    // (#627), so the map in force depends on the set files, and a set file
    // recon cannot read leaves no map in force to print.
    let (bindings, keymap_warnings, keymap_errors) = crate::keymap::config::build_all(
        &overlay,
        &aliases,
        config.config_file_path(),
        config.warnings(),
    );
    errors.extend(keymap_errors.iter().map(ToString::to_string));
    // `--print-startup` shows what loaded with the errors, below.
    if !errors.is_empty() && config.print_startup.is_none() {
        return Err(LoadErrors(errors).into());
    }

    // The keymap warnings go to stderr first (#259), the same warnings a
    // normal start shows in its panel — which names this command as where
    // to read them all. Filtered here by the panel's own switch, since these
    // are the same warnings by another route. Above `check_sets`: a `--set`
    // flag does not change the keymap.
    if config.print_keymap.is_some() {
        return Ok(Start::Print {
            stdout: crate::keymap::print_keymap(&bindings, &Keymap::default()),
            warnings: if config.warnings() {
                keymap_warnings
            } else {
                Vec::new()
            },
            failed: false,
        });
    }

    // Looks each command up on `PATH` once, here, so that a missing one is
    // one warning at startup and not an error on each file.
    let (table, view_warnings) = crate::views::cycle::Table::build(&merged.rows, &merged.commands);
    let mut warnings = keymap_warnings;
    warnings.extend(std::mem::take(&mut merged.warnings));
    warnings.extend(view_warnings);
    warnings.extend(config.set_hints());

    // Below `--print-keymap`'s return, with the whole merge in hand: the
    // report shows every source and every warning, so it prints them to
    // stdout and leaves stderr empty (#628).
    if let Some(which) = config.print_startup {
        let home = std::env::var("HOME").ok();
        let report = crate::startup_report::Report {
            config_file: &config_file,
            filter_dirs: &loaded.dirs,
            env: crate::config::recon_env(),
            read: &loaded.files,
            skipped: &loaded.skipped,
            merged: &merged,
            bindings: &bindings,
            warnings: &warnings,
            errors: &errors,
            all: which == crate::config::PrintStartup::All,
            home: home.as_deref(),
        };
        return Ok(Start::Print {
            stdout: crate::startup_report::print(&report),
            warnings: Vec::new(),
            failed: !errors.is_empty(),
        });
    }

    // Needs the loaded sets, which is why it is not inside `Config::load`
    // with `check_flags`. Still before any terminal setup: the message must
    // reach a screen that is not about to be replaced (#143).
    config.check_sets(&filter_sets)?;

    Ok(Start::Run(Box::new(Startup {
        config,
        bindings,
        aliases,
        warnings,
        table: std::sync::Arc::new(table),
        filter_sets,
        icons: crate::filetype::Icons::Off,
    })))
}
