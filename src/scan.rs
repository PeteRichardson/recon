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
//! `docs/specs/2026-09-02-explorer-filter-matches-design.md`.

use crate::document::{self, Endian, Sniff};
use crate::filter::{Matcher, Owner};
use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
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
    pub seen: Vec<crate::filter::Bits>,
    pub scanned_to: u64,
    pub eof: bool,
}

/// `(mtime, len)` of a file when it was scanned. A mismatch on re-stat means
/// the file changed; [`grew`] says whether the record can still be resumed.
pub type Stamp = (SystemTime, u64);

/// The most bytes of one line that are read and matched. The rest of the
/// line, up to its newline, is skipped and never held in memory, so a huge file
/// with no newline costs a pass over the disk, not its size in memory (#399).
pub const LINE_MAX_BYTES: u64 = 1 << 20;

/// Whether a file stamped `held` when `progress` was read, and `now` on disk,
/// only grew: its length went up, and the read stopped inside the old length.
///
/// That is the `tail -F` rule (#358): a log is appended to, so the bytes
/// already read are still there and the scan resumes at `scanned_to`. A
/// shorter file, or one of the same length with a new mtime, was rewritten
/// and is read from the top. A file rewritten to a greater length looks the
/// same as one that grew; `resume_at_line` catches the case where that
/// leaves `scanned_to` inside a line, and the rest is the price of the rule.
#[must_use]
pub fn grew(held: Option<Stamp>, now: Option<Stamp>, progress: &Progress) -> bool {
    matches!((held, now), (Some((_, old)), Some((_, new))) if new > old && progress.scanned_to <= old)
}

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
            .min()
    }
}

/// One file a [`Request`] asks for, named the way [`Scanned`] names its
/// answer (#171): `index` is the explorer row, `progress` is how far an
/// earlier scan got, so the worker resumes rather than restarts.
///
/// `stamp` is the stamp `progress` was read under. The worker stats the file
/// anyway, so it — not the UI thread — checks that the file is still the one
/// `progress` describes, and starts over when it is not (#156).
#[derive(Debug, Clone)]
pub struct FileToScan {
    pub index: usize,
    pub path: PathBuf,
    pub stamp: Option<Stamp>,
    pub progress: Progress,
}

/// One scan: which files, from where, matched with what.
///
/// `cache_id` is echoed on every [`Scanned`] so the receiver can drop results
/// from a cache that has since been replaced.
#[derive(Debug, Clone)]
pub struct Request {
    pub cache_id: u64,
    pub matcher: Matcher,
    pub files: Vec<FileToScan>,
}

/// One file's result. `index` is the explorer row the request named; the
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
        // `Builder` and not `thread::spawn`: spawn panics when the OS
        // refuses a thread, and takes the TUI down with it. A refusal here
        // costs the marking, which is worth a line in the log and nothing
        // more (#188).
        if let Err(err) = std::thread::Builder::new()
            .name("recon-scan".to_string())
            .spawn(move || worker(request, &tx, &flag))
        {
            log::warn!("cannot start the scan thread: {err}; files stay unmarked");
        }
    }

    fn cancel(&self) {
        self.cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store(true, Ordering::Relaxed);
    }
}

/// `scan`, with a panic turned into a finished file.
///
/// `Scanner` keeps its own `Sender`, so a worker that panics never
/// disconnects the channel: `drain_scan_results`'s `Disconnected` warning
/// cannot fire, and the file stays `Unknown` for ever with nothing said. A
/// caught panic is reported as `eof: true` instead — the file is answered,
/// wrongly but finitely, and the explorer stops waiting (#188).
fn scan_caught(
    lines: impl Lines,
    matcher: &Matcher,
    progress: Progress,
    cancel: &AtomicBool,
) -> Progress {
    // `scan_lines` takes `progress` by value, so the closure moves it —
    // leaving nothing behind to fall back to once it has panicked. Clone
    // first so the caller's progress survives the unwind.
    let fallback = progress.clone();
    let guarded = std::panic::AssertUnwindSafe(|| scan_lines(lines, matcher, progress, cancel));
    std::panic::catch_unwind(guarded).unwrap_or_else(|_| {
        log::warn!("a file's scan panicked; it is reported as read to the end");
        Progress {
            eof: true,
            ..fallback
        }
    })
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
    for FileToScan {
        index,
        path,
        stamp: held,
        progress,
    } in files
    {
        let stamp = stamp(&path).ok();
        // Resuming a file that changed since `progress` was read would add
        // new bytes to old bitsets at an offset that may no longer be a line
        // boundary. `refresh_scan` no longer stats to catch this (#156), so
        // it is caught here, where the stat is already paid for. A file that
        // only grew keeps what was read, and its new end is read (#358).
        let progress = if stamp == held {
            progress
        } else if grew(held, stamp, &progress) {
            Progress {
                eof: false,
                ..progress
            }
        } else {
            Progress::default()
        };
        // `refuse_unreadable` first: a file replaced by a FIFO since the
        // listing would block `File::open` for ever and hold this thread
        // (#399).
        let progress = match document::refuse_unreadable(&path).and_then(|()| File::open(&path)) {
            Ok(file) => scan_file(file, &matcher, progress, cancel),
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

/// Scan an open file from where `progress` stopped, and return how far it
/// got. The explorer's worker and `--emit files` both read a file this way.
///
/// The head is sniffed first, as the view sniffs it. A UTF-16 file with a
/// byte-order mark is decoded before it is matched: matched as raw bytes,
/// `E\0R\0R\0O\0R\0` never hits `ERROR`, and the explorer would say no
/// to a file the view colours (#357). Everything else is read as bytes, a
/// binary file included.
///
/// A read error ends the file: `eof`, with what was read so far.
pub fn scan_file<F: Read + Seek>(
    mut file: F,
    matcher: &Matcher,
    progress: Progress,
    cancel: &AtomicBool,
) -> Progress {
    let sniffed = file
        .seek(SeekFrom::Start(0))
        .and_then(|_| document::sniff(&mut file));
    let endian = match sniffed {
        Ok((Sniff::Utf16(endian), _)) => Some(endian),
        Ok((Sniff::Text | Sniff::Binary, _)) => None,
        Err(err) => {
            log::warn!("scan stopped early: {err}");
            return Progress {
                eof: true,
                ..progress
            };
        }
    };
    let newline: &[u8] = match endian {
        Some(Endian::Little) => &[b'\n', 0],
        Some(Endian::Big) => &[0, b'\n'],
        None => b"\n",
    };
    let progress = match resume_at_line(&mut file, progress, newline) {
        Ok(progress) => progress,
        Err(err) => {
            log::warn!("scan stopped early: {err}");
            return Progress {
                eof: true,
                ..Progress::default()
            };
        }
    };
    let reader = BufReader::new(file);
    match endian {
        Some(endian) => {
            let from_top = progress.scanned_to == 0;
            scan_caught(
                Utf16Lines::new(reader, endian, from_top),
                matcher,
                progress,
                cancel,
            )
        }
        None => scan_caught(ByteLines::new(reader), matcher, progress, cancel),
    }
}

/// Position `file` where the scan goes on: at `progress.scanned_to` when the
/// bytes just before it are a line end, else at the top with nothing kept.
///
/// `scanned_to` is a line boundary when it was read, but a file that grew
/// (#358) may have grown a line that was cut short: a last line written
/// without its newline yet. Resuming there would match its second half as a
/// line of its own, so the file is read again from the top. A seek or read
/// that fails also starts over: that costs a re-read but stays correct,
/// where a scan from an unknown position would give a wrong `scanned_to`.
fn resume_at_line<F: Read + Seek>(
    file: &mut F,
    progress: Progress,
    newline: &[u8],
) -> io::Result<Progress> {
    let at = progress.scanned_to;
    let ends_a_line = |file: &mut F| -> io::Result<bool> {
        let width = newline.len() as u64;
        if at < width {
            return Ok(false);
        }
        file.seek(SeekFrom::Start(at - width))?;
        let mut end = [0; 2];
        let end = &mut end[..newline.len()];
        Ok(read_up_to(file, end)? == end.len() && end == newline)
    };
    if at > 0 {
        match ends_a_line(file) {
            Ok(true) => return Ok(progress),
            Ok(false) => {}
            Err(err) => log::warn!("cannot resume at {at}: {err}"),
        }
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(Progress::default())
}

/// Read into `buf` until it is full or the reader ends; how many bytes were
/// read.
fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}

/// A listed file whose stamp on disk is not the one its record holds, with
/// the stamp it has now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub path: PathBuf,
    pub stamp: Option<Stamp>,
}

/// Stat every `(path, held stamp)` pair and return the files that moved.
///
/// One `stat` per file — on a 20,000-file folder or a network mount, far too
/// slow for the UI thread (#156). [`check_in_background`] runs it on a thread
/// of its own; tests call it directly.
#[must_use]
pub fn moved(held: Vec<(PathBuf, Option<Stamp>)>) -> Vec<Moved> {
    held.into_iter()
        .filter_map(|(path, held)| {
            let stamp = stamp(&path).ok();
            (stamp != held).then_some(Moved { path, stamp })
        })
        .collect()
}

/// Run [`moved`] on a thread and return where its one answer will arrive.
///
/// `None` when the OS refuses the thread: the poll is skipped, logged, and
/// tried again on the next interval — the same tolerance as `Scanner` (#188).
#[must_use]
pub fn check_in_background(held: Vec<(PathBuf, Option<Stamp>)>) -> Option<Receiver<Vec<Moved>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    match std::thread::Builder::new()
        .name("recon-stamps".to_string())
        .spawn(move || {
            // The receiver may be gone by the time this is done — the app
            // quit. Nothing to report then.
            let _ = tx.send(moved(held));
        }) {
        Ok(_) => Some(rx),
        Err(err) => {
            log::warn!("cannot start the stamp thread: {err}; changes on disk go unseen");
            None
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

/// Test doubles. `pub(crate)` so `app`'s tests can install one.
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
/// file, at EOF, or when `cancel` is set. [`scan_file`] is the same over a
/// file, with the sniff and the resume check.
#[must_use]
pub fn scan<R: BufRead>(
    reader: R,
    matcher: &Matcher,
    progress: Progress,
    cancel: &AtomicBool,
) -> Progress {
    scan_lines(ByteLines::new(reader), matcher, progress, cancel)
}

/// One line at a time from a file, in the encoding it was sniffed as.
trait Lines {
    /// The next line, without its line end, and how many bytes of the file
    /// it took. `None` at the end of the file.
    fn next_line(&mut self) -> io::Result<Option<(u64, Cow<'_, str>)>>;
}

/// Lines as bytes, decoded lossily from UTF-8.
///
/// Bytes, not `str`: a log with one bad byte on line 40,000 must still get an
/// answer. `from_utf8_lossy` is a `Cow` that allocates only on an invalid line,
/// the same tolerance `read_lines` got in 7d6e587.
///
/// A line longer than [`LINE_MAX_BYTES`] is matched on its first
/// `LINE_MAX_BYTES` and the rest skipped to its newline (#399).
struct ByteLines<R> {
    reader: R,
    buf: Vec<u8>,
}

impl<R> ByteLines<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buf: Vec::new(),
        }
    }
}

impl<R: BufRead> Lines for ByteLines<R> {
    fn next_line(&mut self) -> io::Result<Option<(u64, Cow<'_, str>)>> {
        self.buf.clear();
        let mut read = (&mut self.reader)
            .take(LINE_MAX_BYTES)
            .read_until(b'\n', &mut self.buf)? as u64;
        if read == 0 {
            return Ok(None);
        }
        if read == LINE_MAX_BYTES && self.buf.last() != Some(&b'\n') {
            read += self.reader.skip_until(b'\n')? as u64;
        }
        let line = match String::from_utf8_lossy(&self.buf) {
            Cow::Borrowed(line) => Cow::Borrowed(line.trim_end_matches(['\n', '\r'])),
            Cow::Owned(line) => Cow::Owned(line.trim_end_matches(['\n', '\r']).to_string()),
        };
        Ok(Some((read, line)))
    }
}

/// Lines of a UTF-16 file, decoded one line at a time in `endian`'s order.
///
/// Streamed, unlike `document::read_utf16_lines`: a scan stops at the first
/// line that selects, and resumes at a line end, which here is the two-byte
/// unit `0A 00` (or `00 0A`). The byte-order mark is dropped from the first
/// line, an unpaired surrogate becomes U+FFFD, and so does an odd byte at
/// the end. A line is cut at [`LINE_MAX_BYTES`] of the file, as `ByteLines`
/// cuts it.
struct Utf16Lines<R> {
    reader: R,
    endian: Endian,
    units: Vec<u16>,
    line: String,
    from_top: bool,
}

impl<R> Utf16Lines<R> {
    fn new(reader: R, endian: Endian, from_top: bool) -> Self {
        Self {
            reader,
            endian,
            units: Vec::new(),
            line: String::new(),
            from_top,
        }
    }
}

impl<R: BufRead> Lines for Utf16Lines<R> {
    fn next_line(&mut self) -> io::Result<Option<(u64, Cow<'_, str>)>> {
        self.units.clear();
        let mut read = 0;
        loop {
            let mut pair = [0; 2];
            let got = read_up_to(&mut self.reader, &mut pair)?;
            read += got as u64;
            if got < pair.len() {
                if got == 1 {
                    self.units.push(0xfffd);
                }
                break;
            }
            let unit = match self.endian {
                Endian::Little => u16::from_le_bytes(pair),
                Endian::Big => u16::from_be_bytes(pair),
            };
            if read <= LINE_MAX_BYTES {
                self.units.push(unit);
            }
            if unit == u16::from(b'\n') {
                break;
            }
        }
        if read == 0 {
            return Ok(None);
        }
        self.line = String::from_utf16_lossy(&self.units);
        let mut line = self.line.trim_end_matches(['\n', '\r']);
        if std::mem::take(&mut self.from_top) {
            line = line.strip_prefix('\u{feff}').unwrap_or(line);
        }
        Ok(Some((read, Cow::Borrowed(line))))
    }
}

/// The scan itself, over lines in any encoding.
///
/// Early exit is why a matching file is free: a 2 GB log that matches on line
/// three costs three lines. The price is that `seen` is only complete at
/// `eof`, which `Record::answer` accounts for. The line end is stripped so
/// `foo$` matches the way it does against a `Document` line.
///
/// `cancel` is checked per line — an atomic load, not a syscall — and a
/// cancelled scan returns what it has. Nothing read is ever thrown away.
fn scan_lines(
    mut lines: impl Lines,
    matcher: &Matcher,
    mut progress: Progress,
    cancel: &AtomicBool,
) -> Progress {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return progress;
        }
        let (read, line) = match lines.next_line() {
            Ok(Some(next)) => next,
            Ok(None) => {
                progress.eof = true;
                return progress;
            }
            Err(err) => {
                log::warn!("scan stopped early: {err}");
                progress.eof = true;
                return progress;
            }
        };
        progress.scanned_to += read;
        let bits = matcher.bits(&line);
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

    fn record(seen: &[crate::filter::Bits], eof: bool) -> Record {
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
        let m = set.matcher().expect("selects");

        assert_eq!(
            record(&[m.bits("beta"), m.bits("alpha")], false).owner(&m),
            Some(0)
        );
        assert_eq!(record(&[m.bits("beta")], false).owner(&m), Some(1));
        assert_eq!(record(&[0], true).owner(&m), None);
    }

    /// A panic inside one file's scan must not lose the file. It is reported
    /// complete, so the explorer stops waiting on it, and the run continues
    /// with the next file (#188).
    #[test]
    fn a_panicking_read_is_reported_complete_rather_than_lost() {
        struct Panicking;

        impl std::io::Read for Panicking {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("the reader panicked");
            }
        }

        impl std::io::BufRead for Panicking {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                panic!("the reader panicked");
            }

            fn consume(&mut self, _: usize) {}
        }

        // `matcher` and `never` are this module's own test helpers.
        let matcher = matcher(&["hit"], &[]);
        let cancel = never();

        let progress = scan_caught(
            ByteLines::new(Panicking),
            &matcher,
            Progress::default(),
            &cancel,
        );

        assert!(progress.eof, "a panicked file must not stay unanswered");
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

    /// Run the worker over one file and return what it sent.
    fn work(path: &Path, stamp: Option<Stamp>, progress: Progress) -> Scanned {
        let (tx, rx) = std::sync::mpsc::channel();
        worker(
            Request {
                cache_id: 1,
                matcher: matcher(&["alpha"], &[]),
                files: vec![FileToScan {
                    index: 0,
                    path: path.to_path_buf(),
                    stamp,
                    progress,
                }],
            },
            &tx,
            &never(),
        );
        rx.recv().expect("one result")
    }

    /// `refresh_scan` no longer stats before a resume (#156), so the worker
    /// must notice the file changed and read it from the top: resuming at
    /// the old offset would skip the new first line.
    #[test]
    fn a_file_whose_stamp_moved_is_read_from_the_top() {
        let dir = std::path::Path::new("target/test-scan");
        std::fs::create_dir_all(dir).expect("fixture dir");
        let path = dir.join("restart.txt");
        std::fs::write(&path, "alpha\nnoise noise\n").expect("write");
        let old = Some((SystemTime::UNIX_EPOCH, 3));
        let partial = Progress {
            seen: vec![0],
            scanned_to: 6,
            eof: false,
        };

        let result = work(&path, old, partial);

        assert!(result.progress.seen.contains(&0b1), "{:?}", result.progress);
        assert_eq!(result.progress.scanned_to, 6, "stopped at the first line");
    }

    #[test]
    fn a_file_whose_stamp_held_is_resumed() {
        let dir = std::path::Path::new("target/test-scan");
        std::fs::create_dir_all(dir).expect("fixture dir");
        let path = dir.join("resume.txt");
        std::fs::write(&path, "alpha\nnoise\n").expect("write");
        let partial = Progress {
            seen: vec![0],
            scanned_to: 6,
            eof: false,
        };

        let result = work(&path, stamp(&path).ok(), partial);

        assert_eq!(result.progress.seen, vec![0], "re-read the first line");
        assert!(result.progress.eof);
    }

    #[test]
    fn moved_names_only_the_files_whose_stamp_changed() {
        let dir = std::path::Path::new("target/test-scan");
        std::fs::create_dir_all(dir).expect("fixture dir");
        let same = dir.join("moved-same.txt");
        let changed = dir.join("moved-changed.txt");
        std::fs::write(&same, "x").expect("write");
        std::fs::write(&changed, "x").expect("write");

        let result = moved(vec![
            (same.clone(), stamp(&same).ok()),
            (changed.clone(), Some((SystemTime::UNIX_EPOCH, 1))),
        ]);

        assert_eq!(
            result,
            vec![Moved {
                stamp: stamp(&changed).ok(),
                path: changed,
            }]
        );
    }

    #[test]
    fn a_background_check_answers_once() {
        let dir = std::path::Path::new("target/test-scan");
        std::fs::create_dir_all(dir).expect("fixture dir");
        let path = dir.join("background.txt");
        std::fs::write(&path, "x").expect("write");

        let rx = check_in_background(vec![(path.clone(), None)]).expect("a thread");

        let answer = rx.recv().expect("an answer");
        assert_eq!(answer.len(), 1);
        assert_eq!(answer[0].path, path);
    }

    // ---- growth, encodings and hostile files (#358, #357, #399) -----------

    /// `matcher(&["alpha"])` sets bit 0 only, so a `seen` that still holds
    /// this was kept from the progress the worker was handed, not re-read.
    const KEPT: crate::filter::Bits = 0b100;

    fn fixture(name: &str, content: &[u8]) -> PathBuf {
        let dir = std::path::Path::new("target/test-scan");
        std::fs::create_dir_all(dir).expect("fixture dir");
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write");
        path
    }

    fn append(path: &Path, content: &[u8]) {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open to append")
            .write_all(content)
            .expect("append");
    }

    fn read_to(scanned_to: u64) -> Progress {
        Progress {
            seen: vec![KEPT],
            scanned_to,
            eof: true,
        }
    }

    #[test]
    fn only_a_longer_file_read_inside_its_old_length_grew() {
        let at = |len| Some((SystemTime::UNIX_EPOCH, len));
        let to = |scanned_to| Progress {
            scanned_to,
            ..Progress::default()
        };

        assert!(grew(at(10), at(20), &to(10)));
        assert!(!grew(at(10), at(5), &to(5)), "a shorter file was rewritten");
        assert!(!grew(at(10), at(10), &to(10)), "same length, new mtime");
        assert!(!grew(at(10), at(20), &to(15)), "read past the old length");
        assert!(!grew(None, at(20), &to(0)), "no stamp to compare");
        assert!(!grew(at(10), None, &to(0)), "gone from disk");
    }

    /// A log that grew is read from where the last scan stopped, not from
    /// byte 0 (#358).
    #[test]
    fn a_file_that_grew_is_resumed_not_restarted() {
        let path = fixture("grew.txt", b"noise\n");
        let held = stamp(&path).ok();
        append(&path, b"alpha\n");

        let result = work(&path, held, read_to(6));

        assert!(
            result.progress.seen.contains(&KEPT),
            "{:?}",
            result.progress
        );
        assert!(result.progress.seen.contains(&0b1), "{:?}", result.progress);
        assert_eq!(result.progress.scanned_to, 12);
        assert_eq!(result.stamp, stamp(&path).ok());
    }

    /// The last scan ended inside a line written without its newline yet.
    /// Its second half is not a line of its own, so the file is read again.
    #[test]
    fn a_file_that_grew_a_cut_line_is_read_from_the_top() {
        let path = fixture("grew-cut.txt", b"noise alp");
        let held = stamp(&path).ok();
        append(&path, b"ha\n");

        let result = work(&path, held, read_to(9));

        assert!(
            !result.progress.seen.contains(&KEPT),
            "{:?}",
            result.progress
        );
        assert!(result.progress.seen.contains(&0b1), "missed `noise alpha`");
    }

    #[test]
    fn a_shorter_file_is_read_from_the_top() {
        let path = fixture("shrank.txt", b"alpha\nnoise\n");

        let result = work(&path, Some((SystemTime::UNIX_EPOCH, 100)), read_to(40));

        assert!(
            !result.progress.seen.contains(&KEPT),
            "{:?}",
            result.progress
        );
        assert!(result.progress.seen.contains(&0b1));
    }

    fn utf16(text: &str, endian: Endian) -> Vec<u8> {
        std::iter::once(0xfeff)
            .chain(text.encode_utf16())
            .flat_map(|unit| match endian {
                Endian::Little => unit.to_le_bytes(),
                Endian::Big => unit.to_be_bytes(),
            })
            .collect()
    }

    /// Matched as bytes, `E\0R\0R\0O\0R\0` never hits `ERROR` (#357).
    #[test]
    fn a_utf16_file_is_decoded_before_it_is_matched() {
        let m = matcher(&["^ERROR$"], &[]);
        for endian in [Endian::Little, Endian::Big] {
            let bytes = utf16("ok\r\nERROR\r\n", endian);

            let progress = scan_file(Cursor::new(&bytes), &m, Progress::default(), &never());

            assert_eq!(progress.seen, vec![0, 0b1], "{endian:?}");
            assert_eq!(progress.scanned_to, bytes.len() as u64, "{endian:?}");
        }
    }

    /// The byte-order mark is not part of the first line.
    #[test]
    fn a_utf16_first_line_loses_its_byte_order_mark() {
        let bytes = utf16("alpha\n", Endian::Little);

        let progress = scan_file(
            Cursor::new(&bytes),
            &matcher(&["^alpha"], &[]),
            Progress::default(),
            &never(),
        );

        assert_eq!(progress.seen, vec![0b1]);
    }

    #[test]
    fn a_utf16_file_resumes_at_its_line_end() {
        let first = utf16("noise\n", Endian::Little);
        let mut bytes = first.clone();
        bytes.extend(utf16("alpha\n", Endian::Little).into_iter().skip(2));

        let progress = scan_file(
            Cursor::new(&bytes),
            &matcher(&["alpha"], &[]),
            read_to(first.len() as u64),
            &never(),
        );

        assert!(progress.seen.contains(&KEPT), "{progress:?}");
        assert!(progress.seen.contains(&0b1), "{progress:?}");
        assert_eq!(progress.scanned_to, bytes.len() as u64);
    }

    /// A line past the cap is matched on its head, and the scan goes on at
    /// the next line rather than reading the rest into memory (#399).
    #[test]
    fn a_line_longer_than_the_cap_is_cut_and_the_next_line_is_read() {
        let cap = usize::try_from(LINE_MAX_BYTES).expect("fits");
        let mut text = "x".repeat(cap + 10);
        text.push_str(" alpha\nalpha\n");

        let progress = scan(
            Cursor::new(&text),
            &matcher(&["alpha"], &[]),
            Progress::default(),
            &never(),
        );

        assert_eq!(progress.seen, vec![0, 0b1], "alpha past the cut was seen");
        assert_eq!(progress.scanned_to, text.len() as u64);
    }

    #[test]
    fn a_utf16_line_longer_than_the_cap_is_cut_and_the_next_line_is_read() {
        let cap = usize::try_from(LINE_MAX_BYTES).expect("fits");
        let mut text = "x".repeat(cap);
        text.push_str(" alpha\nalpha\n");
        let bytes = utf16(&text, Endian::Little);

        let progress = scan_file(
            Cursor::new(&bytes),
            &matcher(&["alpha"], &[]),
            Progress::default(),
            &never(),
        );

        assert_eq!(progress.seen, vec![0, 0b1], "alpha past the cut was seen");
        assert_eq!(progress.scanned_to, bytes.len() as u64);
    }

    /// A file replaced by a FIFO after the listing is refused, not opened:
    /// the open would block for ever with no writer (#399).
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let path = std::path::Path::new("target/test-scan/fifo");
        let _ = std::fs::remove_file(path);
        std::fs::create_dir_all("target/test-scan").expect("fixture dir");
        let made = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo");
        assert!(made.success());

        let (tx, rx) = std::sync::mpsc::channel();
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send(work(&path, None, Progress::default()));
        });

        let result = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the worker blocked on the FIFO");
        assert!(result.progress.eof);
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
