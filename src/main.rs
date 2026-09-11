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
use std::sync::atomic::{AtomicBool, Ordering};

fn main() -> Result<ExitCode> {
    install_error_hooks()?;

    setup_logging();
    let mut config = Config::load()?;

    // Before the filter sets are read, not after: this command prints an
    // `[editor]` stanza and exits, and it uses nothing from `filters.toml`.
    // A syntax error in that file used to stop a command that does not
    // consult it (#191).
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

    config.filter_sets = recon::filtersets::load_file()?;
    // Needs the loaded sets, which is why it is not inside `Config::load`
    // with `check_flags`. Still before any terminal setup: the message must
    // reach a screen that is not about to be replaced (#143).
    config.check_sets(&config.filter_sets)?;

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

/// What the panic hook must do, decided by the thread that panicked.
#[derive(Debug, PartialEq, Eq)]
enum OnPanic {
    /// Restore the terminal, then print the report.
    Report,
    /// End the thread and say nothing to the screen.
    Quiet,
}

/// Only the main thread may touch the terminal on the way out.
///
/// A worker that panicked used to run `restore_terminal` from the hook. That
/// tore down a TUI the user was still looking at, printed a report over it,
/// and — because the undo is one-shot (#222) — left the real exit with
/// nothing to do, so the alternate screen stayed up after recon ended (#245).
///
/// Rust names the main thread `main`. The scan worker is `recon-scan` and the
/// editor reaper is `recon-editor`. Anything else that is not `main` is
/// treated as a worker, which is the safe way round: a new thread added later
/// gets the quiet path by default and cannot damage the screen.
///
/// The scan worker's `catch_unwind` (`scan_caught`, `src/scan.rs`) only
/// guards the per-file scan; a panic anywhere else in that thread still
/// reaches this hook and ends the whole batch silently (`scan_caught`'s own
/// doc comment).
fn on_panic(thread: Option<&str>) -> OnPanic {
    if thread == Some("main") {
        OnPanic::Report
    } else {
        OnPanic::Quiet
    }
}

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
        let current = std::thread::current();
        let name = current.name();
        if on_panic(name) == OnPanic::Quiet {
            // The only trace a worker's panic leaves. With `RECON_LOG` it
            // reaches the file; without it the TUI is up, so `Muted` drops it
            // too (#246). That cost is recorded in the 1.0 spec: the user sees
            // only that the marks on the files stop changing.
            log::error!("panic on thread {}: {info}", name.unwrap_or("<unnamed>"));
            return;
        }
        let _ = restore_terminal();
        panic(info);
    }));
    Ok(())
}

/// Names a file to write the log to. Unset means stderr.
///
/// The variable exists because of where recon spends its time: stderr goes to
/// the *normal* screen, and recon is holding the alternate one from
/// `init_terminal` until it exits. A line logged in between would be drawn
/// over the TUI and stay there until the next full redraw, so on stderr recon
/// drops every record logged while the screen is held (#246). Only the
/// startup and shutdown call sites reach stderr.
///
/// Pointing this at a file makes every call site usable, which is the whole
/// reason the in-session ones were worth adding (#83), and it is the only way
/// to see them.
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
///
/// **`build` rather than `init`.** `init` installs the logger immediately,
/// and #246 needs to wrap it first. The two lines `init` would have run —
/// `set_boxed_logger` and `set_max_level` — are spelled out at the end of
/// the function instead. See `Muted`.
fn setup_logging() {
    let mut builder = env_logger::builder();
    builder
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .format_target(false)
        .format_timestamp(None);

    let mut to_file = false;
    if let Some(path) = std::env::var_os(LOG_FILE_VAR) {
        match std::fs::File::create(&path) {
            Ok(file) => {
                builder.target(env_logger::Target::Pipe(Box::new(file)));
                to_file = true;
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

    // `build` and not `init`, so that the logger can be wrapped before it is
    // installed (#246). `init` is `set_boxed_logger` plus `set_max_level`, so
    // both are done by hand here.
    let logger = builder.build();
    log::set_max_level(logger.filter());

    // The failed-to-open branch above falls back to stderr, so it takes the
    // muted path too — `to_file` tracks where the records actually go, not
    // whether the variable was set.
    let installed = if to_file {
        log::set_boxed_logger(Box::new(logger))
    } else {
        log::set_boxed_logger(Box::new(Muted {
            inner: logger,
            hold: || TERMINAL_UP.load(Ordering::Relaxed),
        }))
    };

    // Only reachable if something else already installed a logger, which
    // nothing in this binary does. Reported and survived for the same reason
    // as the file failure above: a debugging aid must not stop the program.
    if let Err(err) = installed {
        eprintln!("recon: the logger could not be installed: {err}");
    }
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
    // Set as soon as raw mode is on, not after the rest of setup below: the
    // hooks call `restore_terminal` the moment any `?` past this point turns
    // an error into a `Report`, and raw mode is what still needs undoing even
    // when the alternate screen and mouse capture were never reached.
    TERMINAL_UP.store(true, Ordering::Relaxed);
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

/// Whether `init_terminal` has run and its undo is still owed.
///
/// The error and panic hooks run for every failure, including one that
/// happens before any terminal setup — `--set` naming a set that
/// `filters.toml` does not define, an unreadable `filters.toml`, `-n`
/// without `--emit lines`. Without this flag they wrote `LeaveAlternateScreen`
/// and `DisableMouseCapture` to stderr anyway, which put about 30 bytes of
/// control characters in front of the message. Headless mode makes that
/// script-facing: a redirected stderr holds them verbatim (#222).
static TERMINAL_UP: AtomicBool = AtomicBool::new(false);

/// Undo `init_terminal`: leave the alternate screen and stop mouse reports.
///
/// The cursor needs nothing here. `init_terminal` never hides it — the
/// `execute!` above sends `EnterAlternateScreen`, `EnableMouseCapture` and
/// `Clear`, and no path in `src/` calls `hide_cursor`. A `show_cursor` call
/// sat commented out here for months with no reason recorded; it is deleted
/// rather than restored, because `LeaveAlternateScreen` already returns the
/// normal screen with the cursor it had.
fn restore_terminal() -> Result<()> {
    // `swap` and not `load`: the normal path calls this once and the hooks
    // can call it again on the way out, and the undo must happen one time.
    if !TERMINAL_UP.swap(false, Ordering::Relaxed) {
        return Ok(());
    }
    disable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

/// A logger that drops a record while the TUI owns the screen.
///
/// recon draws on stderr, so a record written between `init_terminal` and
/// `restore_terminal` lands on top of the frame and stays there until the
/// next full redraw. #83 added call sites that fire during a session, and
/// #189 added more, which turned a latent problem into one per navigator
/// keypress (#246).
///
/// Dropping is the least bad of the three answers. Buffering needs a bound
/// and a flush point, and nothing would ever read the buffer on the normal
/// exit path. Reporting on the status row needs a channel from every call
/// site, including ones inside worker threads. `RECON_LOG` already exists
/// and already puts every record somewhere the screen cannot see, so the
/// recovery costs the user one environment variable.
///
/// Installed only when no file target was set: with `RECON_LOG` the records
/// go to the file, so nothing is dropped.
struct Muted<L> {
    inner: L,
    /// True while a record must not reach stderr.
    ///
    /// A function pointer and not a direct read of `TERMINAL_UP`, so that a
    /// test can drive the gate without touching a process-wide static.
    hold: fn() -> bool,
}

impl<L: log::Log> log::Log for Muted<L> {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        !(self.hold)() && self.inner.enabled(metadata)
    }

    // This guard is the one that does the work: the `log!` macros call `log`
    // directly and never consult `enabled` first. `enabled` carries the same
    // guard so that `log_enabled!` gives the answer `log` will act on.
    fn log(&self, record: &log::Record) {
        if !(self.hold)() {
            self.inner.log(record);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log as _;
    use std::sync::Mutex;

    /// Keeps what reached it, so a test can see what `Muted` passed on.
    #[derive(Default)]
    struct Spy(Mutex<Vec<String>>);

    impl log::Log for Spy {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            self.0.lock().unwrap().push(record.args().to_string());
        }

        fn flush(&self) {}
    }

    /// Build and log one record in a single statement.
    ///
    /// `format_args!` borrows its arguments and cannot outlive the statement
    /// that made it, so the record cannot be returned from a helper — it has
    /// to be logged where it is built.
    fn record(logger: &dyn log::Log, message: &str) {
        logger.log(
            &log::Record::builder()
                .args(format_args!("{message}"))
                .level(log::Level::Warn)
                .build(),
        );
    }

    #[test]
    fn muted_drops_a_record_while_the_screen_is_held() {
        let muted = Muted {
            inner: Spy::default(),
            hold: || true,
        };

        record(&muted, "a scan warning, mid-session");

        assert!(
            muted.inner.0.lock().unwrap().is_empty(),
            "a record reached stderr while the TUI held the screen"
        );
    }

    #[test]
    fn muted_passes_a_record_when_the_screen_is_free() {
        let muted = Muted {
            inner: Spy::default(),
            hold: || false,
        };

        record(&muted, "a startup warning");

        let seen = muted.inner.0.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], "a startup warning");
    }

    #[test]
    fn muted_reports_not_enabled_while_the_screen_is_held() {
        let muted = Muted {
            inner: Spy::default(),
            hold: || true,
        };

        assert!(
            !muted.enabled(&log::Metadata::builder().level(log::Level::Warn).build()),
            "log_enabled! would disagree with what log() actually does"
        );
    }

    #[test]
    fn the_main_thread_reports_its_panic() {
        assert_eq!(on_panic(Some("main")), OnPanic::Report);
    }

    #[test]
    fn the_scan_worker_dies_quietly() {
        assert_eq!(on_panic(Some("recon-scan")), OnPanic::Quiet);
    }

    #[test]
    fn an_unnamed_thread_dies_quietly() {
        assert_eq!(on_panic(None), OnPanic::Quiet);
    }
}
