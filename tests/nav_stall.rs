//! Moving the navigator's selection onto a large file must not stall the
//! UI thread (#265).
//!
//! Every selection move previews the file and evaluates every line against
//! the active filters. With no filter effective and no search enabled, that
//! evaluation owes no regex work at all — but the built-in definitions set
//! is always present, and its eleven never-match placeholders in the
//! compiled `RegexSet` cost real time per line. On a 30,000-line file the
//! one keypress took ~10 s in a debug build, so the budget here cannot pass
//! by luck.

use crossterm::event::{Event, KeyCode, KeyEvent};
use recon::{App, Config};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// Same order of magnitude as the log that was measured, and well inside
/// the preview caps, so the whole file is evaluated.
const LINES: usize = 30_000;

/// What makes the placeholder expensive. The regex crate's lazy DFA cannot
/// answer a Unicode `\b` on non-ASCII text and hands each such line to a
/// far slower engine, so a log whose every line carries an arrow — the one
/// measured in #265 — pays the full price. A pure-ASCII fixture passes even
/// on the broken code.
const NON_ASCII: &str = "\u{2192}";

/// The worst case this test tolerates for one `j`. The fixed code takes a
/// few hundred milliseconds in debug; the broken code, about ten seconds.
const BUDGET: Duration = Duration::from_secs(1);

/// A directory holding a three-line file that sorts first and a large log
/// after it, so a single `j` from the first entry lands on the log.
fn fixture() -> std::path::PathBuf {
    let dir = std::path::Path::new("target/test-navdirs/nav_stall");
    std::fs::remove_dir_all(dir).ok();
    std::fs::create_dir_all(dir).expect("create fixture dir");
    std::fs::write(dir.join("aaa.txt"), "one\ntwo\nthree\n").expect("write fixture");
    let mut log = String::with_capacity(LINES * 96);
    for i in 0..LINES {
        let _ = writeln!(
            log,
            "2026-09-23T10:{:02}:{:02}.{:03}Z INFO  server[4242] request {i} GET /api/items/{} {NON_ASCII} 200 {}ms",
            (i / 60) % 60,
            i % 60,
            i % 1000,
            i % 977,
            i % 37
        );
    }
    std::fs::write(dir.join("big.log"), log).expect("write fixture");
    dir.to_path_buf()
}

#[test]
fn selecting_a_large_file_with_no_filters_active_is_quick() {
    let config = Config {
        path: fixture().join("aaa.txt").display().to_string(),
        ..Config::default()
    };
    let mut app = App::new(&config);

    let started = Instant::now();
    app.handle_event(Event::Key(KeyEvent::from(KeyCode::Char('j'))));
    let elapsed = started.elapsed();

    assert!(
        elapsed < BUDGET,
        "one `j` onto a {LINES}-line file took {elapsed:.2?}; the budget is {BUDGET:?}"
    );
}
