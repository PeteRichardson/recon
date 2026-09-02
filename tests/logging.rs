//! The crate actually logs (#83).
//!
//! `env_logger` and `log` sat in `Cargo.toml` for months with `setup_logging`
//! wired into `main` and not one `warn!` or `debug!` anywhere in `src/` — the
//! `[DEBUG] Config { .. }` line they were added for had been deleted and
//! nothing noticed, because nothing could. These tests are what would have
//! noticed: they install a capturing logger and assert that ordinary failure
//! paths put something in it.
//!
//! An integration test rather than a unit test, and that is load-bearing rather
//! than stylistic. `log::set_logger` may be called at most once per process and
//! the unit-test binary runs hundreds of tests in one process, several of which
//! would race for it. Each integration test file gets a process to itself, so
//! the global slot here is uncontested.

use recon::{App, Config};
use std::sync::{Mutex, OnceLock};

/// Records the capturing logger has collected, newest last.
static CAPTURED: Mutex<Vec<(log::Level, String)>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        CAPTURED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((record.level(), record.args().to_string()));
    }

    fn flush(&self) {}
}

/// Install the capturing logger once, at `Trace` so nothing is filtered out
/// before it is counted.
fn install() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        log::set_logger(&Capture).expect("no other logger is installed in this process");
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// Records mentioning `needle`. Filtering by a fixture-specific string rather
/// than draining the buffer keeps the tests independent of each other, since
/// `cargo test` may run them in parallel against the one static.
fn records_mentioning(needle: &str) -> Vec<(log::Level, String)> {
    CAPTURED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter(|(_, message)| message.contains(needle))
        .cloned()
        .collect()
}

fn app_over(path: &str) -> App<'static> {
    App::new(&Config {
        path: path.to_string(),
        ..Config::default()
    })
}

/// A file recon cannot open is reported in the pane *and* in the log. The pane
/// tells the user it failed; the log is the only place the full path survives,
/// since the pane's title is elided when the pane is narrow.
#[test]
fn an_unreadable_file_is_logged() {
    install();
    let name = "no_such_file_for_the_logging_test.log";

    let _app = app_over(&format!("target/{name}"));

    let found = records_mentioning(name);
    assert!(
        !found.is_empty(),
        "opening a missing file logged nothing; captured so far: {:?}",
        CAPTURED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    assert!(
        found.iter().any(|(level, _)| *level == log::Level::Warn),
        "expected a warning, got {found:?}"
    );
}

/// The one that would have caught the original regression on its own: some
/// call site, somewhere, emits a record. `setup_logging` can be perfectly
/// configured and still be pointless, which is exactly the state #83 found.
#[test]
fn the_crate_logs_at_all() {
    install();

    let _app = app_over("target/another_missing_path_for_the_logging_test.log");

    assert!(
        !CAPTURED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        "no log record was emitted by any call site — `log` is a dead dependency again"
    );
}
