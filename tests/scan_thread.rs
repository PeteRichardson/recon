//! The one test that runs a real `Scanner` thread. Everything else about
//! scanning is tested over a `Cursor` in `src/scan.rs`.

use recon::filter::ActiveFilters;
use recon::scan::{Progress, Request, Scan, Scanned, Scanner};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn collect(rx: &mpsc::Receiver<Scanned>, want: usize) -> Vec<Scanned> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while got.len() < want {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(scanned) => got.push(scanned),
            Err(err) => panic!(
                "timed out with {} of {want} results ({err}): {got:?}",
                got.len()
            ),
        }
    }
    got
}

#[test]
fn a_scanner_thread_answers_every_file_it_is_given() {
    let dir = Path::new("target/test-scan-thread");
    fs::create_dir_all(dir).expect("fixture dir");
    let hit = dir.join("hit.log");
    let miss = dir.join("miss.log");
    let gone = dir.join("gone.log");
    fs::write(&hit, "one\nERROR deploy failed\nthree\n").expect("write");
    fs::write(&miss, "quiet\nquieter\n").expect("write");
    fs::remove_file(&gone).ok();

    let mut set = ActiveFilters::new();
    set.add("ERROR").expect("valid pattern");
    let (tx, rx) = mpsc::channel();
    let scanner = Scanner::new(tx);

    scanner.start(Request {
        cache_id: 7,
        matcher: set.matcher().expect("selects"),
        files: vec![
            (1, hit.clone(), Progress::default()),
            (2, miss.clone(), Progress::default()),
            (3, gone.clone(), Progress::default()),
        ],
    });

    let mut results = collect(&rx, 3);
    results.sort_by_key(|scanned| scanned.index);
    let matcher = set.matcher().expect("selects");

    assert!(results.iter().all(|scanned| scanned.cache_id == 7));
    let hit = &results[0];
    assert!(!hit.progress.eof, "kept reading past the match");
    assert!(hit.progress.seen.iter().any(|&bits| matcher.selects(bits)));
    assert!(hit.stamp.is_some());

    let miss = &results[1];
    assert!(miss.progress.eof);
    assert!(!miss.progress.seen.iter().any(|&bits| matcher.selects(bits)));

    let gone = &results[2];
    assert!(
        gone.progress.eof,
        "an unreadable file must answer, not hang"
    );
    assert!(gone.stamp.is_none());
}

#[test]
fn a_new_request_cancels_the_old_one() {
    let dir = Path::new("target/test-scan-thread-cancel");
    fs::create_dir_all(dir).expect("fixture dir");
    let big = dir.join("big.log");
    let mut text = String::new();
    for i in 0..200_000 {
        let _ = writeln!(text, "line {i}");
    }
    fs::write(&big, &text).expect("write");

    let mut set = ActiveFilters::new();
    set.add("never matches").expect("valid pattern");
    let (tx, rx) = mpsc::channel();
    let scanner = Scanner::new(tx);
    let request = |cache_id| Request {
        cache_id,
        matcher: set.matcher().expect("selects"),
        files: vec![(0, big.clone(), Progress::default())],
    };

    scanner.start(request(1));
    scanner.start(request(2));

    // Both workers report — the cancelled one with whatever it had.
    let results = collect(&rx, 2);
    assert!(
        results
            .iter()
            .any(|scanned| scanned.cache_id == 2 && scanned.progress.eof)
    );
    assert!(results.iter().any(|scanned| scanned.cache_id == 1));
}
