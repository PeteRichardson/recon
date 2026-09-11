use color_eyre::{Result, config::HookBuilder, eyre};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use recon::{App, Config};
use std::io::{self, IsTerminal, Stderr};
use std::panic;
use std::process::ExitCode;

fn main() -> Result<ExitCode> {
    install_error_hooks()?;

    setup_logging();
    let mut config = Config::load()?;
    config.filter_sets = recon::filtersets::load_file()?;
    // Needs the loaded sets, which is why it is not inside `Config::load`
    // with `check_flags`. Still before any terminal setup: the message must
    // reach a screen that is not about to be replaced (#143).
    config.check_sets(&config.filter_sets)?;

    if let Some(flavour) = &config.print_editor_config {
        print!(
            "{}",
            recon::editor::print_editor_config(
                flavour,
                std::env::var("TERM_PROGRAM").ok().as_deref(),
            )?
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Headless (#143): `--emit` with no terminal on stdin. A TUI needs stdin
    // for its keys, so a pipe or `/dev/null` there is not a session that
    // could have been driven anyway; the result is computed and printed
    // instead. `recon --emit lines app.log` from a terminal still gets the
    // TUI, and `< /dev/null` forces headless from one.
    let headless = config.emit.is_some() && !io::stdin().is_terminal();
    let exit = if headless {
        recon::headless::run(&config)?
    } else {
        let terminal = init_terminal()?;
        let exit = App::new(&config).run(terminal)?;
        restore_terminal()?;
        exit
    };

    // Only now, with the alternate screen gone (or never entered), does
    // anything reach stdout: the result, if `--emit` asked for one, and the
    // summary that names its mode on stderr (#143).
    Ok(exit.deliver(
        config.emit,
        config.quiet,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
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

/// The TUI draws on **stderr** (#143), so stdout carries nothing but what
/// `--emit` asks for and can be piped or captured while the TUI is up — the
/// same arrangement fzf uses. Unconditional rather than switched on whether
/// stdout is a terminal: one code path, and a difference nobody could see.
///
/// The writer is a `BufWriter`: unlike `Stdout`, `Stderr` carries no
/// buffering of its own, so every `queue!`'d cell write during a redraw would
/// otherwise be its own `write(2)`. `Terminal::draw` flushes the backend at
/// the end of every frame, so nothing is left sitting in the buffer between
/// draws.
///
/// The screen is cleared with a `Clear(All)` on the same `execute!` as the
/// alternate screen, **not** with `Terminal::clear`. That method snapshots
/// the cursor first, and crossterm answers a cursor-position query by
/// writing `ESC [ 6 n` to *stdout* regardless of where the backend draws —
/// into the pipe, when there is one, where no terminal will ever answer it.
/// `recon --emit cwd | xargs …` then sat on a black screen for two seconds
/// and died with "the cursor position could not be read". Nothing else in
/// the draw path asks where the cursor is.
fn init_terminal() -> Result<Terminal<CrosstermBackend<io::BufWriter<Stderr>>>> {
    enable_raw_mode()?;
    execute!(
        io::stderr(),
        EnterAlternateScreen,
        EnableMouseCapture,
        Clear(ClearType::All)
    )?;
    let backend = CrosstermBackend::new(io::BufWriter::new(io::stderr()));
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Undo `init_terminal`: leave the alternate screen and stop mouse reports.
///
/// The cursor needs nothing here. `init_terminal` never hides it — the
/// `execute!` above sends `EnterAlternateScreen`, `EnableMouseCapture` and
/// `Clear`, and no path in `src/` calls `hide_cursor`. A `show_cursor` call
/// sat commented out here for months with no reason recorded; it is deleted
/// rather than restored, because `LeaveAlternateScreen` already returns the
/// normal screen with the cursor it had.
fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}
