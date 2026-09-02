use color_eyre::{Result, config::HookBuilder, eyre};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use recon::{App, Config};
use std::io::{self, Stdout};
use std::panic;

fn main() -> Result<()> {
    install_error_hooks()?;

    setup_logging();
    // Before `init_terminal`, and that ordering is load-bearing rather than
    // incidental. A config error printed after raw mode and the alternate
    // screen are in place is wiped off the screen a frame later, so the only
    // place a bad `config.toml` can be reported legibly is here — on stderr,
    // on the normal screen, while recon still has it. Hence a hard failure
    // rather than a warning: "warn and carry on" would be "carry on silently".
    //
    // This used to say "where the existing `[DEBUG] Config { .. }` line already
    // goes". That line was removed in `fadffdb` and the comment outlived it by
    // long enough to be cited in #83 as evidence the logging was abandoned. The
    // reasoning never depended on it — stderr before the alternate screen is
    // the only legible place regardless of what else is printed there.
    let config = Config::load()?;

    // Before the terminal, for the same reason the config error is: this prints
    // a snippet to be copied out of the scrollback, and the alternate screen
    // would take it away the moment it was drawn. It also *only* prints —
    // recon never writes `config.toml` (see `Cargo.toml`) and will not write a
    // shell rc either, so there is nothing to undo afterwards.
    if let Some(flavour) = &config.print_editor_config {
        print!(
            "{}",
            recon::editor::print_editor_config(
                flavour,
                std::env::var("TERM_PROGRAM").ok().as_deref(),
            )?
        );
        return Ok(());
    }

    let terminal = init_terminal()?;
    App::new(&config).run(terminal)?;
    restore_terminal()?;

    Ok(())
}

//===================================================================================

/// Install `color_eyre` panic and error hooks
///
/// The hooks restore the terminal to a usable state before printing the error message.
fn install_error_hooks() -> Result<()> {
    let (panic, error) = HookBuilder::default().into_hooks();
    let panic = panic.into_panic_hook();
    let error = error.into_eyre_hook();
    eyre::set_hook(Box::new(move |e| {
        let _ = restore_terminal();
        error(e)
    }))?;
    panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        panic(info);
    }));
    Ok(())
}

/// Names a file to write the log to. Unset means stderr.
///
/// The variable exists because of where recon spends its time: stderr goes to
/// the *normal* screen, and recon is holding the alternate one from
/// `init_terminal` until it exits. A line logged in between is drawn over the
/// TUI and stays there until the next full redraw, so on stderr only the
/// startup and shutdown call sites are safe to fire. Pointing this at a file
/// makes every call site usable, which is the whole reason the in-session ones
/// were worth adding (#83).
const LOG_FILE_VAR: &str = "RECON_LOG";

/// Bring up the logger, before anything has anything to say.
///
/// **`Info` as the floor, not `Debug` as a fixed level.** This is the actual
/// fix (#83): `filter_level(Debug)` put every `debug!` on stderr on every
/// ordinary run, which was harmless only while nothing logged.
/// `RUST_LOG=recon=debug` is the detail switch now.
///
/// `parse_default_env` is **redundant and kept on purpose.**
/// `env_logger::builder()` is `Builder::from_default_env()`, which has already
/// parsed `RUST_LOG` by the time this runs; `filter_level` only supplies the
/// default for when `RUST_LOG` says nothing, so it does not override it and the
/// two can be called in either order. Measured, not assumed — the pre-#83 code
/// honoured `RUST_LOG=recon=warn` correctly.
///
/// It stays because that is invisible at the call site. #83's own review note
/// asserted the opposite — that `RUST_LOG` was "silently ignored" — from
/// reading exactly this code, so an explicit call is the cheapest way to stop
/// the next reader reaching the same wrong conclusion. It costs one idempotent
/// call at startup.
fn setup_logging() {
    let mut builder = env_logger::builder();
    builder
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .format_target(false)
        .format_timestamp(None);

    if let Some(path) = std::env::var_os(LOG_FILE_VAR) {
        match std::fs::File::create(&path) {
            Ok(file) => {
                builder.target(env_logger::Target::Pipe(Box::new(file)));
            }
            // Warned about and carried on, which is the opposite of the call
            // `Config::load` makes two lines below — and deliberately. A
            // config file recon cannot read changes what the app *does*; a log
            // file it cannot open only changes what it records. Refusing to
            // start because a debugging aid is unavailable would be the wrong
            // trade. Printed rather than logged because the logger is, at this
            // exact moment, what has just failed to be set up.
            Err(err) => {
                eprintln!(
                    "recon: {LOG_FILE_VAR} names {}, which cannot be opened: {err}",
                    std::path::Path::new(&path).display(),
                );
                eprintln!("recon: continuing with logging on stderr");
            }
        }
    }

    builder.init();
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    // terminal.show_cursor()?;
    Ok(())
}
