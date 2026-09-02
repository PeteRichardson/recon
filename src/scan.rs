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

use crate::filter::Matcher;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

/// How far one file has been read, and what its lines matched.
///
/// `seen` holds every *distinct* per-line bitset met so far, deduplicated. It
/// stays tiny — a real log has single-digit distinct match combinations — so a
/// `Vec` with a linear `contains` beats a hash set. `scanned_to` is a byte
/// offset at a line boundary, which is what lets a later scan resume rather
/// than restart. `eof` says whether `seen` is complete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub seen: Vec<u64>,
    pub scanned_to: u64,
    pub eof: bool,
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
}
