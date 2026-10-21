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

/// The startup snapshot (#490): who this recon is and what it read.
///
/// At `DEBUG`, one record for each fact so that a reader can grep for one.
/// The shadowed sets are not here: the loader logs each one as it drops it.
pub fn startup_snapshot(startup: &Startup) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let config = &startup.config;
    log::debug!("recon {}", env!("CARGO_PKG_VERSION"));
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
