//! What recon writes to its log, and at which level (#490).
//!
//! `main` sets the logger up; this module holds the records that describe a
//! whole session — the startup snapshot and the shutdown summary — and the
//! counters the summary reads.
//!
//! **One rule for the levels**, the same one the book's "Logging" page gives:
//!
//! - `INFO`: a fact the user must see without a setting. `INFO` is the
//!   default level, and before the TUI starts a record goes to stderr, which
//!   a batch-mode script reads. So nothing new goes at `INFO`.
//! - `DEBUG`: once per session, or once per user action.
//! - `TRACE`: once per file, per key or per frame.
//!
//! `WARN` and `ERROR` stay for what went wrong, as before.
//!
//! **No cost when the level is off.** A record that costs much to build is
//! behind `log::log_enabled!`.
//!
//! **No file contents and no search text.** Paths and filter patterns are
//! acceptable: a bug report needs them, and they are the user's own setup.

use crate::startup::Startup;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// View cache lookups that found the view, and those that did not. Statics
/// and not fields, because the lookups happen on the view workers, which
/// have no `App`.
pub(crate) static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
pub(crate) static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

/// Add one to `counter`.
pub(crate) fn count(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// The first record of a session (#501): which binary, which build, and
/// which process, so that a reader can find each start and separate two
/// recons that write to one file. The pair of `session ended`.
///
/// `exe` is `std::env::current_exe()`. When that fails (Linux without
/// `/proc`), the name recon was started with, `argv0`, is the next best.
pub fn session_started(
    exe: std::io::Result<std::path::PathBuf>,
    argv0: Option<std::ffi::OsString>,
    pid: u32,
) {
    let path = exe.ok().map(std::path::PathBuf::into_os_string).or(argv0);
    let path = path.as_deref().map_or_else(
        || "(unknown)".into(),
        |path| std::path::Path::new(path).display().to_string(),
    );
    log::debug!(
        "session started: {path} {} pid {pid}",
        crate::config::VERSION
    );
}

/// The startup snapshot (#490): who this recon is and what it read.
///
/// At `DEBUG`, one record for each fact so that a reader can grep for one.
/// The shadowed sets are not here: the loader logs each one as it drops it.
pub fn startup_snapshot(startup: &Startup) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let config = &startup.config;
    // The build stamp `recon -V` shows, with the commit and `.dirty`
    // (#501): the `Cargo.toml` version alone is the same for two builds.
    log::debug!("version: {}", crate::config::VERSION);
    log::debug!("arguments: {:?}", std::env::args_os().collect::<Vec<_>>());
    match crate::config::config_path() {
        Some(path) => log::debug!("config file: {}", path.display()),
        None => log::debug!("config file: none (no config home)"),
    }
    for (name, value) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("RECON_") {
            log::debug!("environment: {}={}", name.display(), value.display());
        }
    }
    log::debug!(
        "filter path: {}",
        config.filter_path.as_deref().unwrap_or("(default)")
    );
    for set in &startup.filter_sets {
        log::debug!(
            "filter set {:?}: {}, {}, {} filters, from {}",
            set.name,
            if set.listed { "listed" } else { "unlisted" },
            if set.listed && set.autoload {
                "enabled"
            } else {
                "disabled"
            },
            set.filters.len(),
            if set.builtin {
                "recon".to_string()
            } else {
                set.path.display().to_string()
            },
        );
    }
    log::debug!("keymap: {} overrides", startup.bindings.overrides());
    // recon does not ask the terminal for its background (#231): the
    // record says whether a setting gave it, or it is the default.
    let source = |set: bool| if set { "set" } else { "default" };
    log::debug!(
        "theme: {} ({})",
        config.syntax_theme().name().unwrap_or("none"),
        source(config.theme.is_some()),
    );
    log::debug!(
        "background: {} ({})",
        match config.background() {
            crate::filter::Background::Dark => "dark",
            crate::filter::Background::Light => "light",
        },
        source(config.background.is_some()),
    );
}

/// How a TUI session ended, for the summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// `q`, which emits under `--emit`.
    Quit,
    /// `Q`, which never emits.
    QuitSilent,
    /// Ctrl-c, or a cancelled `q`.
    Cancelled,
    /// A SIGTERM.
    Terminated,
    /// The loop returned an error.
    Error,
}

/// The shutdown summary (#490), at `DEBUG`: one record for the session.
pub(crate) fn shutdown_summary(
    duration: Duration,
    ending: Ending,
    emit: Option<crate::emit::Emit>,
    opened: u64,
    scanned: u64,
) {
    let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
    log::debug!(
        "session ended: {ending:?} after {:.1}s, emit {}, {opened} files opened, \
         {scanned} scan answers, view cache {} hits {} misses",
        duration.as_secs_f64(),
        emit.map_or_else(|| "none".to_string(), |emit| format!("{emit:?}")),
        load(&CACHE_HITS),
        load(&CACHE_MISSES),
    );
}

/// One key and the action it dispatched (#492), at `TRACE`.
///
/// A plain character that a text field took and no binding did is search
/// text, or a pattern, name or prompt being typed, so its record does not
/// name it. `typing` says a text field was open when the key came.
pub(crate) fn key(key: crate::keymap::Key, action: Option<crate::keymap::ActionId>, typing: bool) {
    if !log::log_enabled!(log::Level::Trace) {
        return;
    }
    let plain_char =
        matches!(key.code, crossterm::event::KeyCode::Char(_)) && !key.ctrl && !key.alt;
    if typing && plain_char && action.is_none() {
        log::trace!("key (typed into a text field): no action");
        return;
    }
    log::trace!(
        "key {}: {}",
        crate::help::pressed_label(key),
        action.map_or("no action", crate::keymap::ActionId::name)
    );
}

/// A frame that takes longer than this to draw is a `WARN` (#492).
///
/// Three frames at 60 a second: a user sees a pause that long.
pub(crate) const SLOW_FRAME: Duration = Duration::from_millis(50);

/// The time to draw one frame (#492): `TRACE`, and also a `WARN` when it is
/// more than [`SLOW_FRAME`].
///
/// The `WARN` passes the default level, so `RECON_LOG` gets it without
/// `RUST_LOG`. Without `RECON_LOG` the log is stderr, and the logger in
/// `main` drops it while the screen is open.
pub(crate) fn frame_drawn(took: Duration) {
    log::trace!("frame drawn in {took:.1?}");
    if took > SLOW_FRAME {
        log::warn!("slow frame: drawn in {took:.1?}, more than {SLOW_FRAME:?}");
    }
}

/// A logger for the unit tests, which keeps every record.
///
/// `log::set_logger` can run once in a process, and the unit tests share
/// one, so it is installed once here and every test reads from it. A test
/// finds its own records by a text only its fixture logs, or by its thread:
/// tests run in parallel on threads of their own.
#[cfg(test)]
pub(crate) mod capture {
    use std::sync::{Mutex, OnceLock, PoisonError};
    use std::thread::ThreadId;

    /// One record: its thread, level and text.
    type Captured = (ThreadId, log::Level, String);

    static RECORDS: Mutex<Vec<Captured>> = Mutex::new(Vec::new());

    struct Capture;

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            RECORDS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((
                    std::thread::current().id(),
                    record.level(),
                    record.args().to_string(),
                ));
        }

        fn flush(&self) {}
    }

    /// Install the logger, at `Trace`, if no test has yet.
    pub(crate) fn install() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            log::set_logger(&Capture).expect("no other logger in the test process");
            log::set_max_level(log::LevelFilter::Trace);
        });
    }

    /// The records `keep` accepts, by thread and text, oldest first.
    fn records(keep: impl Fn(ThreadId, &str) -> bool) -> Vec<(log::Level, String)> {
        RECORDS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(thread, _, text)| keep(*thread, text))
            .map(|(_, level, text)| (*level, text.clone()))
            .collect()
    }

    /// The records this thread logged.
    pub(crate) fn here() -> Vec<(log::Level, String)> {
        let me = std::thread::current().id();
        records(|thread, _| thread == me)
    }

    /// The records this thread logged at `level` that contain `needle`.
    pub(crate) fn here_at(level: log::Level, needle: &str) -> Vec<String> {
        here()
            .into_iter()
            .filter(|(at, text)| *at == level && text.contains(needle))
            .map(|(_, text)| text)
            .collect()
    }

    /// The records from any thread that contain `needle`.
    pub(crate) fn mentioning(needle: &str) -> Vec<(log::Level, String)> {
        records(|_, text| text.contains(needle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_a_trace_record_with_its_time() {
        capture::install();
        frame_drawn(Duration::from_millis(10));
        assert_eq!(
            capture::here(),
            [(log::Level::Trace, "frame drawn in 10.0ms".to_string())]
        );
    }

    #[test]
    fn a_slow_frame_is_also_a_warning() {
        capture::install();
        frame_drawn(Duration::from_millis(60));
        assert_eq!(
            capture::here(),
            [
                (log::Level::Trace, "frame drawn in 60.0ms".to_string()),
                (
                    log::Level::Warn,
                    "slow frame: drawn in 60.0ms, more than 50ms".to_string()
                ),
            ]
        );
    }

    /// The theme and background records of one snapshot.
    fn theme_and_background(config: crate::config::Config) -> Vec<String> {
        capture::install();
        let before = capture::here().len();
        startup_snapshot(&Startup::from(config));
        capture::here()[before..]
            .iter()
            .filter(|(level, text)| {
                *level == log::Level::Debug
                    && (text.starts_with("theme: ") || text.starts_with("background: "))
            })
            .map(|(_, text)| text.clone())
            .collect()
    }

    #[test]
    fn the_snapshot_gives_the_default_theme_and_background() {
        assert_eq!(
            theme_and_background(crate::config::Config::default()),
            ["theme: ANSI (default)", "background: dark (default)"]
        );
    }

    #[test]
    fn the_snapshot_gives_a_theme_and_background_that_were_set() {
        let config = crate::config::Config {
            theme: Some(crate::syntax::Theme::Off),
            background: Some(crate::filter::Background::Light),
            ..crate::config::Config::default()
        };
        assert_eq!(
            theme_and_background(config),
            ["theme: none (set)", "background: light (set)"]
        );
    }

    /// The full build stamp, as `recon -V` gives it, and not only the
    /// version in `Cargo.toml`: two builds of one release are different.
    #[test]
    fn the_snapshot_gives_the_version_that_recon_v_shows() {
        capture::install();
        startup_snapshot(&Startup::from(crate::config::Config::default()));
        // `config`'s tests pin `VERSION` to what `recon -V` prints.
        assert_eq!(
            capture::here_at(log::Level::Debug, "version: "),
            [format!("version: {}", crate::config::VERSION)]
        );
    }

    /// The records from `session_started`.
    fn started(
        exe: std::io::Result<std::path::PathBuf>,
        argv0: Option<&str>,
    ) -> Vec<(log::Level, String)> {
        capture::install();
        let before = capture::here().len();
        session_started(exe, argv0.map(Into::into), 4242);
        capture::here()[before..].to_vec()
    }

    /// The one record `session_started` should give for `path`.
    fn started_with(path: &str) -> [(log::Level, String); 1] {
        let version = crate::config::VERSION;
        [(
            log::Level::Debug,
            format!("session started: {path} {version} pid 4242"),
        )]
    }

    #[test]
    fn a_session_starts_with_the_binary_the_version_and_the_process() {
        assert_eq!(
            started(Ok("/opt/bin/recon".into()), Some("recon")),
            started_with("/opt/bin/recon")
        );
    }

    #[test]
    fn without_the_binary_path_the_name_it_was_started_with_is_given() {
        let gone = || Err(std::io::Error::other("no /proc"));
        assert_eq!(
            started(gone(), Some("./target/debug/recon")),
            started_with("./target/debug/recon")
        );
        assert_eq!(started(gone(), None), started_with("(unknown)"));
    }

    #[test]
    fn the_summary_is_one_debug_record() {
        capture::install();
        shutdown_summary(
            Duration::from_millis(1500),
            Ending::Terminated,
            Some(crate::emit::Emit::Cwd),
            3,
            2,
        );
        let records = capture::here();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].0, log::Level::Debug);
        assert!(
            records[0].1.starts_with(
                "session ended: Terminated after 1.5s, emit Cwd, 3 files opened, 2 scan answers,"
            ),
            "{records:?}"
        );
    }
}
