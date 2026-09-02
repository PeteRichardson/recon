//! Which files would show a line under the active filters — answered in the
//! background, one file at a time, without loading any of them (#119).
//!
//! Two layers, kept apart so the interesting behaviour is testable without a
//! thread. [`scan`] is a pure core over any `BufRead`: it records which
//! patterns hit each line as a bitset and stops at the first line that selects
//! the file. [`Scanner`] is the thread that drives it per file and streams
//! [`Scanned`] results over a channel.
//!
//! The bitsets are the point. A file matches under a mask iff some line's
//! bitset has a selecting bit and no excluding bit — a few `u64` ops — so a
//! filter toggle re-answers a whole folder with no I/O. See the design at
//! `docs/specs/2026-09-02-navigator-filter-matches-design.md`.

use crate::filter::{Matcher, Owner};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

/// How far one file has been read, and what its lines matched.
///
/// `seen` holds every *distinct* per-line bitset met so far, deduplicated. It
/// stays tiny — a real log has single-digit distinct match combinations — so a
/// `Vec` with a linear `contains` beats a hash set. `scanned_to` is a byte
/// offset at a line boundary, which is what lets a later scan resume rather
/// than restart. `eof` says whether `seen` is complete — or an error ended
/// the read; either way nothing more can be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub seen: Vec<u64>,
    pub scanned_to: u64,
    pub eof: bool,
}

/// `(mtime, len)` of a file when it was scanned. A mismatch on re-stat means
/// the record is for a file that no longer exists in that form.
pub type Stamp = (SystemTime, u64);

/// Read a file's [`Stamp`].
///
/// # Errors
/// Whatever `fs::metadata` reports — a missing file, no permission.
pub fn stamp(path: &Path) -> std::io::Result<Stamp> {
    let meta = std::fs::metadata(path)?;
    Ok((meta.modified()?, meta.len()))
}

/// One file's scan state, held in `App`'s cache.
///
/// `stamp` is `None` when the file could not be stat'd; two `None`s compare
/// equal, so an unreadable file is not re-tried on every poll.
#[derive(Debug, Clone)]
pub struct Record {
    pub stamp: Option<Stamp>,
    pub progress: Progress,
}

impl Record {
    /// Whether the file matches under `m`, if that can be known from what has
    /// been read. `None` means resume the scan from `progress.scanned_to`.
    ///
    /// Three outcomes, and the middle one is what makes early exit and the
    /// cache coexist: a selecting bitset answers yes at once; `eof` with none
    /// answers no; a partial read with none is the only case that costs I/O,
    /// and only for the unread remainder.
    #[must_use]
    pub fn answer(&self, m: &Matcher) -> Option<bool> {
        if self.progress.seen.iter().any(|&bits| m.selects(bits)) {
            return Some(true);
        }
        if self.progress.eof {
            return Some(false);
        }
        None
    }

    /// Which filter selected the file, for its colour: the highest-ranked
    /// owner across every seen bitset.
    #[must_use]
    pub fn owner(&self, m: &Matcher) -> Option<Owner> {
        self.progress
            .seen
            .iter()
            .filter_map(|&bits| m.owner(bits))
            .min_by_key(|owner| owner.rank())
    }
}

/// One scan: which files, from where, matched with what.
///
/// `cache_id` is echoed on every [`Scanned`] so the receiver can drop results
/// from a cache that has since been replaced. `files` carries each file's
/// existing [`Progress`] so the worker resumes rather than restarts.
#[derive(Debug, Clone)]
pub struct Request {
    pub cache_id: u64,
    pub matcher: Matcher,
    pub files: Vec<(usize, PathBuf, Progress)>,
}

/// One file's result. `index` is the navigator row the request named; the
/// receiver checks it still names `path` before using it.
#[derive(Debug, Clone)]
pub struct Scanned {
    pub cache_id: u64,
    pub index: usize,
    pub path: PathBuf,
    pub stamp: Option<Stamp>,
    pub progress: Progress,
}

/// Something that runs scan requests. `&self`, like `editor::Launcher`, so a
/// test can hold an `Rc` to a recording double while `App` owns the box.
pub trait Scan {
    /// Start scanning. An in-flight scan is cancelled first; its partial
    /// results still arrive.
    fn start(&self, request: Request);
    /// Stop the in-flight scan, if any.
    fn cancel(&self);
}

/// The real thing: at most one worker thread, results over an `mpsc` channel.
///
/// Holds no cache and no state between requests — that is `App`'s. Its one
/// piece of state is the cancel flag of the current worker.
pub struct Scanner {
    tx: Sender<Scanned>,
    cancel: Mutex<Arc<AtomicBool>>,
}

impl Scanner {
    #[must_use]
    pub fn new(tx: Sender<Scanned>) -> Self {
        Self {
            tx,
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
        }
    }
}

impl Scan for Scanner {
    fn start(&self, request: Request) {
        self.cancel();
        let flag = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&flag);
        let tx = self.tx.clone();
        std::thread::spawn(move || worker(request, &tx, &flag));
    }

    fn cancel(&self) {
        self.cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store(true, Ordering::Relaxed);
    }
}

/// The thread body. One file at a time; a cancel between files stops the
/// walk, a cancel inside one returns that file's partial progress — and it is
/// still sent, so nothing read is thrown away.
///
/// The cancel check is *after* a file is processed, not before: a cancel can
/// land the instant a new worker is spawned, before it has run a single
/// instruction (thread creation is not instantaneous), and a check up front
/// would then drop the first file's result entirely. `scan` already handles
/// an already-cancelled flag — it returns `progress` unchanged on its first
/// check — so the file's (possibly untouched) result still reaches `tx`.
fn worker(request: Request, tx: &Sender<Scanned>, cancel: &AtomicBool) {
    let Request {
        cache_id,
        matcher,
        files,
    } = request;
    for (index, path, progress) in files {
        let stamp = stamp(&path).ok();
        let progress = match File::open(&path) {
            Ok(mut file) => {
                // A seek failure leaves the file positioned who-knows-where,
                // so scanning from `progress.scanned_to` as if the seek had
                // worked would let `scan` add to an offset that no longer
                // matches where the read actually started — overshooting the
                // true `scanned_to` and reporting `eof: true` too early, a
                // confident wrong answer. Starting over with
                // `Progress::default()` costs a re-read but stays correct.
                let progress = if let Err(err) = file.seek(SeekFrom::Start(progress.scanned_to)) {
                    log::warn!(
                        "{}: cannot resume at {}: {err}",
                        path.display(),
                        progress.scanned_to
                    );
                    Progress::default()
                } else {
                    progress
                };
                scan(BufReader::new(file), &matcher, progress, cancel)
            }
            // Unreadable answers "no", complete: it will show nothing. Not
            // retried until its stamp changes.
            Err(err) => {
                log::warn!("{}: {err}", path.display());
                Progress {
                    eof: true,
                    ..progress
                }
            }
        };
        let sent = tx.send(Scanned {
            cache_id,
            index,
            path,
            stamp,
            progress,
        });
        if sent.is_err() || cancel.load(Ordering::Relaxed) {
            return;
        }
    }
}

/// Runs nothing. So `App` can hold a `Box<dyn Scan>` in a `#[derive(Default)]`
/// struct without the field becoming an `Option` — the same reason
/// `editor::Launcher` has one. `App::new` replaces it with a real `Scanner`.
struct NoScanner;

impl Scan for NoScanner {
    fn start(&self, _: Request) {}
    fn cancel(&self) {}
}

impl Default for Box<dyn Scan> {
    fn default() -> Self {
        Box::new(NoScanner)
    }
}

/// Test doubles. `pub(crate)` so `lib.rs`'s tests can install one.
#[cfg(test)]
pub(crate) mod double {
    use super::{Request, Scan};
    use std::sync::{Mutex, PoisonError};

    /// Records every request and cancel, runs nothing.
    #[derive(Default)]
    pub(crate) struct RecordingScanner {
        pub requests: Mutex<Vec<Request>>,
        pub cancels: Mutex<usize>,
    }

    impl RecordingScanner {
        pub(crate) fn requests(&self) -> Vec<Request> {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl Scan for RecordingScanner {
        fn start(&self, request: Request) {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
        }

        fn cancel(&self) {
            *self.cancels.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        }
    }

    impl Scan for std::rc::Rc<RecordingScanner> {
        fn start(&self, request: Request) {
            (**self).start(request);
        }

        fn cancel(&self) {
            (**self).cancel();
        }
    }
}

/// Read lines from `reader` — already positioned at `from.scanned_to` — and
/// record what each one matched, stopping at the first line that selects the
/// file, at EOF, or when `cancel` is set.
///
/// Early exit is why a matching file is free: a 2 GB log that matches on line
/// three costs three lines. The price is that `seen` is only complete at
/// `eof`, which `Record::answer` accounts for.
///
/// Bytes, not `str`: a log with one bad byte on line 40,000 must still get an
/// answer. `from_utf8_lossy` is a `Cow` that allocates only on an invalid line,
/// the same tolerance `read_lines` got in 7d6e587. The newline is stripped so
/// `foo$` matches the way it does against a `Document` line.
///
/// `cancel` is checked per line — an atomic load, not a syscall — and a
/// cancelled scan returns what it has. Nothing read is ever thrown away.
pub fn scan<R: BufRead>(
    mut reader: R,
    matcher: &Matcher,
    mut progress: Progress,
    cancel: &AtomicBool,
) -> Progress {
    let mut buf = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return progress;
        }
        buf.clear();
        let read = match reader.read_until(b'\n', &mut buf) {
            Ok(read) => read,
            Err(err) => {
                log::warn!("scan stopped early: {err}");
                progress.eof = true;
                return progress;
            }
        };
        if read == 0 {
            progress.eof = true;
            return progress;
        }
        progress.scanned_to += read as u64;
        let line = String::from_utf8_lossy(&buf);
        let bits = matcher.bits(line.trim_end_matches(['\n', '\r']));
        if !progress.seen.contains(&bits) {
            progress.seen.push(bits);
        }
        if matcher.selects(bits) {
            return progress;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::ActiveFilters;
    use std::io::Cursor;

    fn matcher(includes: &[&str], excludes: &[&str]) -> Matcher {
        let mut set = ActiveFilters::new();
        for pattern in includes {
            set.add(pattern).expect("valid pattern");
        }
        for pattern in excludes {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set.matcher().expect("something selects")
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn stops_at_the_first_selecting_line() {
        let text = "one\ntwo\nalpha here\nthree\n";
        let progress = scan(
            Cursor::new(text),
            &matcher(&["alpha"], &[]),
            Progress::default(),
            &never(),
        );

        assert!(!progress.eof, "kept reading past the first match");
        assert_eq!(progress.scanned_to, "one\ntwo\nalpha here\n".len() as u64);
        assert!(
            progress.seen.contains(&0b1),
            "the matching line's bitset was not recorded"
        );
    }

    #[test]
    fn reads_to_eof_when_nothing_selects() {
        let text = "one\ntwo\nthree";
        let progress = scan(
            Cursor::new(text),
            &matcher(&["alpha"], &[]),
            Progress::default(),
            &never(),
        );

        assert!(progress.eof);
        assert_eq!(
            progress.scanned_to,
            text.len() as u64,
            "a last line without a newline still counts"
        );
        assert_eq!(progress.seen, vec![0]);
    }

    #[test]
    fn resumes_from_where_it_stopped_and_reads_nothing_twice() {
        let text = "alpha\nbeta\n";
        let m = matcher(&["alpha", "beta"], &[]);
        let first = scan(Cursor::new(text), &m, Progress::default(), &never());
        assert_eq!(first.scanned_to, 6);

        // The driver seeks; the core is handed a reader already positioned.
        let mut rest = Cursor::new(text);
        rest.set_position(first.scanned_to);
        let second = scan(rest, &m, first.clone(), &never());

        assert_eq!(second.scanned_to, text.len() as u64);
        assert!(second.seen.contains(&0b10));
    }

    #[test]
    fn distinct_bitsets_are_recorded_once_each() {
        let text = "x\nx\nx\nbeta\nbeta\n";
        // `beta` is a context-only hit: it must be recorded but must not stop the scan.
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add("beta").expect("valid pattern");
        set.toggle_context(1);
        let progress = scan(
            Cursor::new(text),
            &set.matcher().expect("alpha selects"),
            Progress::default(),
            &never(),
        );

        assert!(progress.eof);
        assert_eq!(progress.seen, vec![0, 0b10]);
    }

    #[test]
    fn an_excluded_line_does_not_select_but_is_still_recorded() {
        let text = "alpha noise\nalpha\n";
        let progress = scan(
            Cursor::new(text),
            &matcher(&["alpha"], &["noise"]),
            Progress::default(),
            &never(),
        );

        assert_eq!(
            progress.scanned_to,
            text.len() as u64,
            "stopped on the excluded line"
        );
        assert_eq!(progress.seen, vec![0b11, 0b01]);
    }

    #[test]
    fn cancel_returns_what_it_had_so_far() {
        let text = "one\ntwo\n";
        let cancel = AtomicBool::new(true);
        let progress = scan(
            Cursor::new(text),
            &matcher(&["alpha"], &[]),
            Progress::default(),
            &cancel,
        );

        assert_eq!(
            progress,
            Progress::default(),
            "read a line after being told to stop"
        );
    }

    #[test]
    fn a_line_that_is_not_utf8_does_not_abort_the_file() {
        let bytes = b"one\n\xff\xfe bad\nalpha\n";
        let progress = scan(
            Cursor::new(&bytes[..]),
            &matcher(&["alpha"], &[]),
            Progress::default(),
            &never(),
        );

        assert_eq!(progress.scanned_to, bytes.len() as u64);
        assert!(progress.seen.contains(&0b1));
    }

    /// Patterns anchored at the end must see the line without its newline,
    /// the way `Document` lines have none.
    #[test]
    fn the_newline_is_not_part_of_the_line() {
        let progress = scan(
            Cursor::new("alpha\r\n"),
            &matcher(&["alpha$"], &[]),
            Progress::default(),
            &never(),
        );

        assert!(progress.seen.contains(&0b1));
    }

    // ---- records ---------------------------------------------------------

    fn record(seen: &[u64], eof: bool) -> Record {
        Record {
            stamp: None,
            progress: Progress {
                seen: seen.to_vec(),
                scanned_to: 0,
                eof,
            },
        }
    }

    #[test]
    fn a_seen_selecting_bitset_answers_yes_without_reading() {
        let m = matcher(&["alpha"], &[]);
        assert_eq!(record(&[0, 0b1], false).answer(&m), Some(true));
    }

    #[test]
    fn eof_with_no_selecting_bitset_answers_no() {
        let m = matcher(&["alpha"], &[]);
        assert_eq!(record(&[0], true).answer(&m), Some(false));
    }

    #[test]
    fn partial_with_no_selecting_bitset_needs_a_resume() {
        let m = matcher(&["alpha"], &[]);
        assert_eq!(record(&[0], false).answer(&m), None);
    }

    /// The same bitsets, a different mask: this is the toggle that costs no I/O.
    #[test]
    fn the_answer_follows_the_mask_not_the_scan() {
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add_excluding("noise").expect("valid pattern");
        let rec = record(&[0b11], true); // every alpha line also had noise

        assert_eq!(rec.answer(&set.matcher().expect("selects")), Some(false));
        set.set_enabled(1, false);
        assert_eq!(rec.answer(&set.matcher().expect("selects")), Some(true));
    }

    #[test]
    fn the_owner_is_the_highest_ranked_across_every_seen_bitset() {
        let mut set = ActiveFilters::new();
        set.add("alpha").expect("valid pattern");
        set.add("beta").expect("valid pattern");
        set.set_search("gamma").expect("valid pattern");
        let m = set.matcher().expect("selects");

        assert_eq!(
            record(&[0b010, 0b001], false).owner(&m),
            Some(Owner::Filter(0))
        );
        assert_eq!(
            record(&[0b010, 0b100], false).owner(&m),
            Some(Owner::Search)
        );
        assert_eq!(record(&[0], true).owner(&m), None);
    }

    #[test]
    fn stamp_reads_mtime_and_length() {
        let dir = std::path::Path::new("target/test-scan");
        std::fs::create_dir_all(dir).expect("fixture dir");
        let path = dir.join("stamp.txt");
        std::fs::write(&path, "hello").expect("write");

        let (_, len) = stamp(&path).expect("stat");
        assert_eq!(len, 5);
        assert!(stamp(&dir.join("missing.txt")).is_err());
    }

    // ---- the recording double --------------------------------------------

    /// The double itself, not just its trait impl: a later task's `App`
    /// tests lean on `requests()`/`cancels` reflecting reality, so that
    /// contract is worth its own cheap check here rather than only being
    /// exercised indirectly once `App` wires it in.
    #[test]
    fn the_recording_scanner_records_starts_and_counts_cancels() {
        let recording = double::RecordingScanner::default();
        assert!(recording.requests().is_empty());

        recording.start(Request {
            cache_id: 7,
            matcher: matcher(&["alpha"], &[]),
            files: vec![],
        });
        recording.cancel();

        let requests = recording.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].cache_id, 7);
        assert_eq!(
            *recording
                .cancels
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            1
        );
    }

    /// `Rc<RecordingScanner>` delegates rather than recording separately —
    /// the point of the `Rc` impl, which lets a test keep a handle while
    /// `App` owns the `Box<dyn Scan>`.
    #[test]
    fn an_rc_recording_scanner_shares_its_recording() {
        let recording = std::rc::Rc::new(double::RecordingScanner::default());
        let handle = std::rc::Rc::clone(&recording);

        Scan::start(
            &handle,
            Request {
                cache_id: 3,
                matcher: matcher(&["alpha"], &[]),
                files: vec![],
            },
        );
        Scan::cancel(&handle);

        assert_eq!(recording.requests().len(), 1);
        assert_eq!(
            *recording
                .cancels
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            1
        );
    }
}
