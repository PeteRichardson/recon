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
use recon::App;
use recon::startup::{Start, Startup};
use std::fs::File;
use std::io::{self, IsTerminal, Write};
use std::panic;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

fn main() -> Result<ExitCode> {
    install_error_hooks()?;

    setup_logging();
    // Guarded here and not inside: the arguments cost syscalls to get.
    if log::log_enabled!(log::Level::Debug) {
        recon::logging::session_started(
            std::env::current_exe(),
            std::env::args_os().next(),
            std::process::id(),
        );
    }
    // Everything recon reads before it starts, in the order it must be
    // read, and every refusal while a message can still be seen (#409).
    let start = recon::startup::start();
    if !matches!(start, Ok(Start::Run(_))) {
        // A print command or a refusal: no TUI, and no config to read a log
        // file from, so `RECON_LOG` or stderr, as before #500.
        let config = recon::Config {
            log_file: std::env::var_os("RECON_LOG").map(Into::into),
            ..recon::Config::default()
        };
        choose_log_target(&config, false);
    }
    let startup = match start? {
        Start::Print { stdout, warnings } => {
            for warning in warnings {
                log::warn!("{warning}");
            }
            print!("{stdout}");
            return Ok(ExitCode::SUCCESS);
        }
        Start::Run(startup) => startup,
    };
    let config = &startup.config;
    recon::logging::startup_snapshot(&startup);

    // Batch mode (#143, #439): `--batch`, or `--emit` with no terminal on
    // stdin. The result is computed and printed, with no TUI.
    let batch = config.batch_reason(io::stdin().is_terminal());
    match batch {
        Some(reason) => log::debug!("mode: batch: {reason}"),
        None => log::debug!("mode: TUI"),
    }
    // Now, and not in `setup_logging`: the target depends on the mode and
    // on `config.toml` (#500). Before the terminal is taken, so a file that
    // cannot be opened is reported where it can be read.
    choose_log_target(config, batch.is_none());
    let exit = if batch.is_some() {
        // Batch mode has no panel, so the keymap warnings that would have filled
        // one go to stderr instead — the channel and the gate the reserved-key
        // notice in `keymap::config::build` already uses. Without this they were
        // collected, handed to `App::new`, and dropped on a path that never
        // builds an `App`: two classes of keymap warning under one switch,
        // one of which silently disappeared.
        //
        // Inside this branch and not above it, because in TUI mode they belong
        // to the panel. A copy on stderr would be drawn over by the alternate
        // screen moments later anyway.
        log_warnings(&startup);
        // Buffered, and written as the run goes (#219, #379): `io::stdout()`
        // alone is a `LineWriter`, one `write(2)` per line even into a pipe.
        // Dropped at the end of this block, so the lock is gone before
        // `deliver` takes stdout again.
        let mut out = io::BufWriter::new(io::stdout().lock());
        // `--follow` runs until one of these (#493). Registered only then:
        // any other run ends on its own, and a Ctrl-c there kills it as
        // it always did.
        let stop = Arc::new(AtomicBool::new(false));
        if config.follow {
            for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
                signal_hook::flag::register(signal, Arc::clone(&stop))?;
            }
        }
        recon::batch::run(&startup, &mut out, &stop)?
    } else {
        // Registered before the terminal is set up, so no SIGTERM can find
        // it raw with nothing to undo it (#382). The handler only sets the
        // flag; the loop sees it and leaves the way a quit key does, and the
        // restore below runs as for any other exit.
        let terminate = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminate))?;
        let terminal = init_terminal()?;
        if let Ok(size) = terminal.size() {
            log::debug!("terminal: {} columns, {} rows", size.width, size.height);
        }
        // One answer for the whole session, asked once, after the terminal is
        // up (raw mode must be on for a reply to arrive unsolicited) and
        // before the first frame. Batch mode never gets here, so `--emit`
        // and `--batch` are untouched by construction.
        let icons = recon::filetype::resolve_startup(config.icons_setting(), &probe_glyph_widths);
        let exit = App::new(&startup)
            .with_icons(icons)
            .with_image_picker(image_picker())
            .with_model(recon::generate::system())
            .with_terminate(terminate)
            .run(terminal)?;
        restore_terminal()?;
        exit
    };

    // Only now, with the alternate screen gone (or never entered), does
    // anything reach stdout: the result, if `--emit` asked for one, and the
    // summary that names its mode on stderr (#143).
    // Buffered for the reason the batch writer is (#379); `deliver`
    // flushes before it writes the summary.
    Ok(exit.deliver(
        config.emit,
        config.quiet,
        &mut io::BufWriter::new(io::stdout().lock()),
        &mut io::stderr(),
    ))
}

/// The keymap warnings on stderr, for batch mode, which has no panel to
/// show them. The switch is the panel's, since these are the same warnings
/// by another route. `--print-keymap` gets its copy from `Start::Print`.
fn log_warnings(startup: &Startup) {
    if startup.config.warnings() {
        for warning in &startup.warnings {
            log::warn!("{warning}");
        }
    }
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
            // The only trace a worker's panic leaves. With a log file it
            // reaches the file; without one the TUI is up, so `Muted` drops
            // it too (#246). That cost is recorded in the 1.0 spec: the user sees
            // only that the marks on the files stop changing.
            log::error!("panic on thread {}: {info}", name.unwrap_or("<unnamed>"));
            return;
        }
        let _ = restore_terminal();
        panic(info);
    }));
    Ok(())
}

/// Bring up the logger, before anything has anything to say.
///
/// The logger holds each record until `choose_log_target` says where the
/// log goes (#500): that depends on `--log-file`, on `config.toml` and on
/// the mode, and none of them is known yet. See `Deferred`.
fn setup_logging() {
    let deferred = LOGGER.get_or_init(|| Deferred::new(log_builder().build()));
    log::set_max_level(deferred.filter.filter());
    // Only reachable if something else already installed a logger, which
    // nothing in this binary does. Reported and survived: a debugging aid
    // must not stop the program.
    if let Err(err) = log::set_logger(deferred) {
        eprintln!("recon: the logger could not be installed: {err}");
    }
}

/// The one logger, so that `choose_log_target` can reach it.
static LOGGER: OnceLock<Deferred<env_logger::Logger>> = OnceLock::new();

/// The logger without its target.
///
/// **The levels follow one rule**, written in `recon::logging`: `Info` for a
/// fact the user must see without a setting, `Debug` once per session or
/// per user action, `Trace` once per file, key or frame (#490).
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
fn log_builder() -> env_logger::Builder {
    let mut builder = env_logger::builder();
    builder
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .format_target(false)
        .format_timestamp(None);
    builder
}

/// Send the log to a file or to stderr, and pass on what waited (#500).
///
/// `config.log_file` is the file that `--log-file`, `RECON_LOG` or
/// `[log] file` names. Without one, the TUI writes to the default file when
/// `RUST_LOG` is set; see `recon::logfile::choose`.
///
/// **A file gets timestamps, the process ID and module names; stderr does
/// not** (#490, #500). A file is read later, often beside a bug report,
/// where the time between two records and the module that wrote each one
/// are the point. The file keeps many sessions, and two recons can append
/// to it at the same time, so each record says which process wrote it. On
/// stderr a record is read as it appears, and the short form stays.
fn choose_log_target(config: &recon::Config, tui: bool) {
    let Some(deferred) = LOGGER.get() else {
        return;
    };
    let env = |name| std::env::var(name).ok();
    let home = env("HOME");
    let path = recon::logfile::choose(
        config.log_file.as_deref(),
        home.as_deref(),
        tui,
        env("RUST_LOG").is_some_and(|value| !value.is_empty()),
        || recon::logfile::default_path(env("XDG_STATE_HOME").as_deref(), home.as_deref()),
    );

    let mut builder = log_builder();
    let file = path.and_then(|path| {
        // The default file's directory may not be there yet. A failure here
        // shows as the open failure below.
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match recon::logfile::Rotating::open(&path, config.log_max_bytes(), config.log_files()) {
            Ok(file) => Some(file),
            // Warned about and carried on, which is the opposite of the call
            // `Config::load` makes — and deliberately. A config file recon
            // cannot read changes what the app *does*; a log file it cannot
            // open only changes what it records. Printed rather than logged,
            // because the logger is what has just failed to be set up.
            Err(err) => {
                eprintln!("recon: cannot open the log file {}: {err}", path.display());
                eprintln!("recon: continuing with logging on stderr");
                None
            }
        }
    });
    let logger: Box<dyn log::Log> = if let Some(file) = file {
        let pid = std::process::id();
        builder
            .target(env_logger::Target::Pipe(Box::new(file)))
            .format(move |buf, record| {
                writeln!(
                    buf,
                    "[{} {pid} {:<5} {}] {}",
                    buf.timestamp_millis(),
                    record.level(),
                    record.target(),
                    record.args()
                )
            });
        Box::new(builder.build())
    } else {
        // Held only while the TUI draws on stderr. On `/dev/tty` (#382) a
        // record goes to wherever stderr was sent, and the screen never sees
        // it.
        Box::new(Muted {
            inner: builder.build(),
            hold: || TERMINAL_UP.load(Ordering::Relaxed) && TTY.get().is_none(),
        })
    };
    deferred.resolve(logger);
}

/// A logger that keeps each record until it knows where the log goes.
///
/// The target depends on what `config.toml` says, and recon logs while it
/// reads that file, so the logger starts before its target is known (#500).
/// The records that come first are kept in memory, then passed on in order
/// by `resolve`. Nothing is lost, and nothing reaches stderr early: with a
/// file, those records would have been drawn on the screen and not kept.
///
/// A record that waited gets its timestamp when it is passed on. They are
/// all from the first moments of the session, before the terminal is taken.
struct Deferred<F> {
    /// Decides what is kept while there is no target: `RUST_LOG`.
    filter: F,
    pending: Mutex<Vec<Pending>>,
    ready: OnceLock<Box<dyn log::Log>>,
}

/// A record kept by `Deferred`: what a `log::Record` borrows, owned.
struct Pending {
    level: log::Level,
    target: String,
    module_path: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    message: String,
}

impl<F: log::Log> Deferred<F> {
    fn new(filter: F) -> Self {
        Self {
            filter,
            pending: Mutex::new(Vec::new()),
            ready: OnceLock::new(),
        }
    }

    /// Send the log to `logger` from now on, starting with what waited.
    /// Only the first call has an effect.
    fn resolve(&self, logger: Box<dyn log::Log>) {
        // Under the lock, so that no record is kept after the drain.
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.ready.get().is_some() {
            return;
        }
        let ready = self.ready.get_or_init(|| logger);
        for kept in pending.drain(..) {
            ready.log(
                &log::Record::builder()
                    .args(format_args!("{}", kept.message))
                    .level(kept.level)
                    .target(&kept.target)
                    .module_path(kept.module_path.as_deref())
                    .file(kept.file.as_deref())
                    .line(kept.line)
                    .build(),
            );
        }
    }
}

impl<F: log::Log> log::Log for Deferred<F> {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        match self.ready.get() {
            Some(ready) => ready.enabled(metadata),
            None => self.filter.enabled(metadata),
        }
    }

    fn log(&self, record: &log::Record) {
        if let Some(ready) = self.ready.get() {
            ready.log(record);
            return;
        }
        if !self.filter.enabled(record.metadata()) {
            return;
        }
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // `resolve` can have run while this waited for the lock.
        if let Some(ready) = self.ready.get() {
            ready.log(record);
            return;
        }
        pending.push(Pending {
            level: record.level(),
            target: record.target().to_string(),
            module_path: record.module_path().map(str::to_string),
            file: record.file().map(str::to_string),
            line: record.line(),
            message: record.args().to_string(),
        });
    }

    fn flush(&self) {
        if let Some(ready) = self.ready.get() {
            ready.flush();
        }
    }
}

/// `/dev/tty`, opened once when stderr is not a terminal (#382). Unset means
/// the TUI draws on stderr. `init_terminal`, `restore_terminal` and so the
/// error and panic hooks all write through `screen`, which reads this, so
/// none of them can undo the setup on a different handle than the one that
/// did it.
static TTY: OnceLock<File> = OnceLock::new();

/// Where the TUI draws: stderr, or `/dev/tty` in its place.
enum Screen {
    Stderr(io::Stderr),
    Tty(&'static File),
}

impl Write for Screen {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Stderr(stderr) => stderr.write(buf),
            Self::Tty(tty) => tty.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Stderr(stderr) => stderr.flush(),
            Self::Tty(tty) => tty.flush(),
        }
    }
}

/// The screen the TUI draws on, as `open_screen` chose it.
fn screen() -> Screen {
    TTY.get()
        .map_or_else(|| Screen::Stderr(io::stderr()), Screen::Tty)
}

/// Choose the screen: stderr when it is a terminal, `/dev/tty` when it is
/// not (#382).
///
/// `dir=$(recon --emit cwd 2>/dev/null)` used to draw the whole TUI into
/// `/dev/null`: a blank terminal that still took keys, with no way to see
/// what they did. fzf answers the same case the same way. With stderr a
/// terminal nothing changes.
///
/// No `/dev/tty` — cron, `ssh` without `-t` — refuses with a message, before
/// raw mode, so there is nothing to undo.
fn open_screen() -> Result<()> {
    if io::stderr().is_terminal() {
        log::debug!("screen: stderr");
        return Ok(());
    }
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|err| {
            eyre::eyre!(
                "stderr is not a terminal and /dev/tty cannot be opened ({err}); \
                 the TUI needs a terminal to draw on. \
                 For a result without one, use --emit with stdin from a file or a pipe"
            )
        })?;
    log::debug!("screen: /dev/tty, as stderr is not a terminal");
    // Set once: this is the only caller, and it runs once.
    let _ = TTY.set(tty);
    Ok(())
}

/// The TUI draws on **stderr** (#143), so stdout carries nothing but what
/// `--emit` asks for and can be piped or captured while the TUI is up — the
/// same arrangement fzf uses. Unconditional rather than switched on whether
/// stdout is a terminal: one code path, and a difference nobody could see.
/// When stderr is not a terminal, `/dev/tty` takes its place (#382) — see
/// `open_screen`.
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
fn init_terminal() -> Result<Terminal<CrosstermBackend<io::BufWriter<Screen>>>> {
    open_screen()?;
    enable_raw_mode()?;
    // Set as soon as raw mode is on, not after the rest of setup below: the
    // hooks call `restore_terminal` the moment any `?` past this point turns
    // an error into a `Report`, and raw mode is what still needs undoing even
    // when the alternate screen and mouse capture were never reached.
    TERMINAL_UP.store(true, Ordering::Relaxed);
    execute!(
        screen(),
        EnterAlternateScreen,
        EnableMouseCapture,
        Clear(ClearType::All)
    )?;
    let backend = CrosstermBackend::new(io::BufWriter::new(screen()));
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Tier 1 of the filetype-icon resolution (#512): ask the terminal how wide
/// each candidate glyph actually drew.
///
/// For each glyph, write it plus a tracking character at the top-left of the
/// (fresh, empty) alternate screen, then ask where the cursor went —
/// `crossterm::cursor::position`, the one query-and-parse helper crossterm
/// has, which sends `ESC [ 6 n` and reads the `ESC <row> ; <col> R` reply
/// through the same event machinery the main loop will use, polling with a
/// timeout. The column after the pair names the advance the glyph got: two
/// cells (glyph + tracker, one each) is a real glyph, one overlaps the next
/// row's text, three shifts every column after it (#97). Both wrong answers
/// are corrupting, and both are why the glyph list is walked here and not
/// trusted from the font's metadata — no terminal tells a child process
/// what font it draws with.
///
/// Known limit, by design: a missing glyph drawn as a tofu box advances
/// exactly one cell and is indistinguishable from a real one here. That is
/// cosmetic, and it is what Tier 2 (the `auto` detection) is for: tofu only
/// reaches the screen when the user forced `icons = "on"`.
///
/// Runs once, after `init_terminal` has raw mode on and before the event
/// loop reads a key. The query is written to *stdout*, as crossterm
/// dictates, so it is asked only when stdout is a terminal too — with
/// `--emit` into a pipe, the question would go into the pipe and no answer
/// would ever come; the image picker gates on the same pair for the same
/// reason. A terminal that never answers costs one timeout and nothing
/// more: the first failed query gives up, no glyph is dropped, and `auto`
/// then keeps icons off on its own.
fn probe_glyph_widths() -> Vec<char> {
    use std::io::Write as _;

    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        log::debug!("icon width probe: stdin or stdout is not a terminal, no one to answer");
        return Vec::new();
    }
    let mut writer = screen();

    let mut dropped = Vec::new();
    for glyph in recon::filetype::all_glyphs() {
        // Draw from a known corner every time, wiping what the last pair
        // left: the alternate screen is still blank, and the first paint
        // covers whatever survives.
        let mut draw = || -> std::io::Result<()> {
            crossterm::execute!(
                writer,
                crossterm::cursor::MoveTo(0, 0),
                crossterm::terminal::Clear(ClearType::UntilNewLine)
            )?;
            writer.write_all(glyph.encode_utf8(&mut [0u8; 4]).as_bytes())?;
            writer.write_all(b"X")?;
            writer.flush()
        };
        if draw().is_err() {
            log::debug!("icon width probe: cannot write to the screen; no glyph is dropped");
            return Vec::new();
        }
        match crossterm::cursor::position() {
            // Cursor sits on column 2: the glyph advanced one cell.
            Ok((_, 2)) => {}
            Ok((_, cols)) => {
                log::debug!(
                    "icon U+{:04X} advanced {} columns, not 2 for glyph+tracker",
                    glyph as u32,
                    cols
                );
                dropped.push(glyph);
            }
            Err(_) => {
                log::debug!(
                    "icon width probe: the terminal stopped answering; no glyph is dropped"
                );
                return Vec::new();
            }
        }
    }
    // Nothing the probe drew may survive to the first frame.
    let _ = crossterm::execute!(
        writer,
        crossterm::cursor::MoveTo(0, 0),
        crossterm::terminal::Clear(ClearType::All)
    );
    dropped
}

/// How the image view draws a picture (#458): kitty, iTerm2 or sixel when
/// the terminal answers that it can, else half-blocks.
///
/// Asked once, after `init_terminal` and before the event loop reads a
/// key, as `ratatui-image` requires. The query writes to stdout and reads
/// the answer from stdin, so it is asked only when both are the terminal:
/// with `--emit` into a pipe, the question would go into the pipe and no
/// answer would come. Inside tmux the picture is half-blocks, which need no
/// passthrough.
fn image_picker() -> ratatui_image::picker::Picker {
    use ratatui_image::picker::Picker;
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        log::debug!("image protocol: half-blocks, as stdin or stdout is not a terminal");
        return Picker::halfblocks();
    }
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|err| {
        log::warn!("the terminal did not say how it draws pictures: {err}");
        Picker::halfblocks()
    });
    let env = |name| std::env::var(name).ok();
    let (detected, tmux) = (picker.protocol_type(), picker.tmux_detected());
    let protocol = protocol_for(
        detected,
        tmux,
        env("TERM_PROGRAM").as_deref(),
        env("LC_TERMINAL").as_deref(),
    );
    log::debug!(
        "image protocol: {protocol:?}, {}",
        if tmux {
            "inside tmux".to_string()
        } else {
            format!("the terminal answered {detected:?}")
        }
    );
    picker.set_protocol_type(protocol);
    picker
}

/// The protocol to draw pictures with, given the one the terminal's answer
/// chose, whether recon runs inside tmux, and the variables that name the
/// terminal (#458).
///
/// Inside tmux it is half-blocks, which need no passthrough. iTerm2 answers that it has kitty graphics, but it does not have the
/// Unicode placeholders that `ratatui-image` draws a kitty picture with, so
/// the picture never shows. iTerm2's own protocol does. Half-blocks stay:
/// they are what a terminal with no graphics, or tmux, gets.
fn protocol_for(
    detected: ratatui_image::picker::ProtocolType,
    tmux: bool,
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
) -> ratatui_image::picker::ProtocolType {
    use ratatui_image::picker::ProtocolType;
    let iterm2 = term_program.is_some_and(|name| name.contains("iTerm"))
        || lc_terminal.is_some_and(|name| name.contains("iTerm"));
    if tmux {
        ProtocolType::Halfblocks
    } else if iterm2 && detected != ProtocolType::Halfblocks {
        ProtocolType::Iterm2
    } else {
        detected
    }
}

/// Whether `init_terminal` has run and its undo is still owed.
///
/// The error and panic hooks run for every failure, including one that
/// happens before any terminal setup — `--set` naming a set that
/// `filters.toml` does not define, an unreadable `filters.toml`, `-n`
/// without `--emit lines`. Without this flag they wrote `LeaveAlternateScreen`
/// and `DisableMouseCapture` to stderr anyway, which put about 30 bytes of
/// control characters in front of the message. Batch mode makes that
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
    execute!(screen(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

/// A logger that drops a record while the TUI owns the screen.
///
/// recon draws on stderr, so a record written between `init_terminal` and
/// `restore_terminal` lands on top of the frame and stays there until the
/// next full redraw. #83 added call sites that fire during a session, and
/// #189 added more, which turned a latent problem into one per explorer
/// keypress (#246).
///
/// Dropping is the least bad of the three answers. Buffering needs a bound
/// and a flush point, and nothing would ever read the buffer on the normal
/// exit path. Reporting on the status row needs a channel from every call
/// site, including ones inside worker threads. A log file already puts
/// every record somewhere the screen cannot see, and with `RUST_LOG` set
/// the TUI writes to one by default (#500).
///
/// Installed only when the log has no file: with one the records go to the
/// file, so nothing is dropped.
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

    /// iTerm2 says it has kitty graphics, but not the Unicode placeholders
    /// `ratatui-image` draws kitty pictures with, so a kitty picture never
    /// shows there (#458). Its own protocol does.
    #[test]
    fn iterm2_draws_with_its_own_protocol() {
        use ratatui_image::picker::ProtocolType::{Halfblocks, Iterm2, Kitty, Sixel};
        assert_eq!(protocol_for(Kitty, false, Some("iTerm.app"), None), Iterm2);
        assert_eq!(protocol_for(Sixel, false, None, Some("iTerm2")), Iterm2);
        assert_eq!(protocol_for(Kitty, false, Some("ghostty"), None), Kitty);
        assert_eq!(protocol_for(Sixel, false, None, None), Sixel);
        assert_eq!(
            protocol_for(Halfblocks, false, Some("iTerm.app"), None),
            Halfblocks,
            "with no answer, half-blocks stay"
        );
        assert_eq!(
            protocol_for(Kitty, true, Some("iTerm.app"), None),
            Halfblocks,
            "inside tmux"
        );
    }

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

    /// A record logged before recon knows where the log goes is kept, and
    /// reaches the target once it is chosen, before the later records.
    #[test]
    fn deferred_passes_the_early_records_on_when_the_target_is_chosen() {
        let deferred = Deferred::new(Spy::default());
        let target: &'static Spy = Box::leak(Box::default());

        record(&deferred, "before the config is read");
        assert!(target.0.lock().unwrap().is_empty());
        deferred.resolve(Box::new(target));
        record(&deferred, "after");

        assert_eq!(
            *target.0.lock().unwrap(),
            ["before the config is read", "after"]
        );
    }

    /// The filter decides what is kept: `RUST_LOG` applies to the records
    /// that wait, too.
    #[test]
    fn deferred_keeps_only_what_the_filter_lets_through() {
        let deferred = Deferred::new(Muted {
            inner: Spy::default(),
            hold: || true,
        });
        let target: &'static Spy = Box::leak(Box::default());

        record(&deferred, "filtered out");
        deferred.resolve(Box::new(target));

        assert!(target.0.lock().unwrap().is_empty());
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
