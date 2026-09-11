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

/// A pattern set that will not compile turns the navigator's marking off.
/// That is a state the user can see, so it must reach the log (#187).
///
/// Driven through `ActiveFilters` and not through `App`: this is the unit
/// that owns `recompile`, `filter` is a public module, and `App`'s only
/// public entry points are `new`, `run` and `handle_event`.
#[test]
fn a_pattern_set_that_will_not_compile_is_logged() {
    install();
    let mut filters = recon::filter::ActiveFilters::new();

    // Each pattern is valid alone. Together they exceed the compiled size
    // limit, which is what makes RegexSet::new fail rather than Regex::new.
    for index in 1..40 {
        let pattern = format!("(?i)(aaaa{index}|bbbb{index}|cccc{index}){{200,400}}");
        let _ = filters.add(&pattern);
    }

    let records = records_mentioning("cannot compile the filter patterns");
    assert!(
        records.iter().any(|(level, _)| *level == log::Level::Warn),
        "no warning was logged: {records:?}"
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

/// A directory that cannot be listed reports `<err>` in the pane, where the
/// title is elided and the path is lost. The log is the only place the path
/// survives, exactly as for an unreadable file (#189).
///
/// `App::new`'s own `_ => view.load(argument)` arm cannot be the route here:
/// for *any* directory argument, `nav.selected_path()` is always `Some` —
/// `read_dir_entries` seeds `..` unconditionally even when the real listing
/// fails, so an unreadable directory's navigator still selects `..` rather
/// than nothing — and that keeps `App::new` on its `view.preview` arm, never
/// its `_` arm. So the fixture instead makes the *navigator's own directory*
/// readable, with one unreadable child inside it: the navigator lists the
/// parent fine and selects that child as its first (only) entry, previewing
/// it exactly as arrowing onto it would — which is `read_preview_with_caps`'s
/// own `is_dir` branch, i.e. `directory_listing` on the child directly.
///
/// Asserting on the fixture name alone is not enough here either: opening the
/// parent makes the navigator itself log nothing (it lists successfully), but
/// once the preview reaches the child, `read_preview_with_caps` calls
/// `directory_listing`, whose only failure this fixture can trigger is the
/// one this test exists to prove exists. Asserting on `directory_listing`'s
/// distinct phrasing, `"cannot show the listing for"`, rather than the
/// fixture name alone, makes plain which call site is under test (R8).
#[test]
fn a_directory_that_cannot_be_listed_is_logged() {
    use std::os::unix::fs::PermissionsExt;

    install();
    let child_name = "unlistable_child_for_the_logging_test";
    let parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("unlistable_child_for_the_logging_test_parent");
    let child = parent.join(child_name);
    std::fs::create_dir_all(&child).expect("create fixture dirs");
    std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o000))
        .expect("make the child directory unreadable");

    // The navigator lists `parent` (readable) and selects `child` — its only
    // entry — as the preview target, exactly as arrowing onto it would.
    let _app = app_over(&parent.display().to_string());

    // Restored before the assertion, so a failure does not leave a directory
    // behind that the next run cannot remove.
    std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o755)).ok();

    let records = records_mentioning("cannot show the listing for");
    assert!(
        records
            .iter()
            .any(|(level, message)| *level == log::Level::Warn && message.contains(child_name)),
        "no warning naming {child_name} was logged: {records:?}"
    );
}
