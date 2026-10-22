//! Follow mode (#493): watch one file grow, as `tail -F` does.
//!
//! A [`Tail`] starts at the end of the lines already on screen and, on each
//! [`Tail::poll`], reads only the bytes added since, as the scan does for a
//! file that grew (#358). It follows the path, not the open file: a file
//! that became shorter, or another file put at its path (logrotate), is
//! [`Change::Reset`], and the caller reads it again from the start.
//!
//! The TUI polls on a thread of its own (ADR 0007: the main thread never
//! reads a file); batch mode polls in its own loop.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// How often a followed file is checked. Only the followed file: the other
/// listed files keep the 2 s stamp check.
pub(crate) const INTERVAL: Duration = Duration::from_millis(250);

/// A file followed by path, from the end of the lines already shown.
#[derive(Debug)]
pub(crate) struct Tail {
    path: PathBuf,
    /// Where the next read starts: after the last whole line, or at the
    /// start of the last line when it has no line end yet.
    offset: u64,
    /// How many bytes of the file are read: the lines on screen, then the
    /// size at the last read.
    end: u64,
    /// Whether the last line read has no line end yet. The next read starts
    /// with it again, and its new text replaces it.
    partial: bool,
    /// Which file the path named at the start, to see it replaced.
    identity: Identity,
    /// Why every change has to read the file whole: it is not UTF-8 text,
    /// or it held fewer lines than are shown.
    whole: bool,
    /// Whether the last poll found no file at the path.
    gone: bool,
    /// The file's stamp at the last poll, for the explorer's scan.
    stamp: Option<crate::scan::Stamp>,
}

/// The device and inode a path named. Nothing where the platform has no
/// inodes: a replaced file is then seen only when it is shorter.
#[cfg(unix)]
type Identity = (u64, u64);
#[cfg(not(unix))]
type Identity = ();

/// What a poll found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    /// New lines at the end. `replace_last`: the first of them is the last
    /// line shown, which had no line end and has grown.
    Grew {
        replace_last: bool,
        lines: Vec<String>,
    },
    /// The file became shorter, was replaced, or came back after it was
    /// deleted: read it again from the start. The tail is spent.
    Reset,
    /// There is no file at the path. The tail waits for it to come back.
    Gone,
}

impl Tail {
    /// A tail of `path` after its first `held` lines: the lines on screen,
    /// read as `document::read_lines` reads them.
    pub(crate) fn after_lines(path: &Path, held: usize) -> Self {
        let mut tail = Self {
            path: path.to_path_buf(),
            offset: 0,
            end: 0,
            partial: false,
            identity: Identity::default(),
            whole: false,
            gone: false,
            stamp: None,
        };
        match tail.skip(held) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => tail.gone = true,
            Err(err) => {
                log::debug!("follow: {}: {err}", path.display());
                tail.whole = true;
            }
        }
        tail
    }

    /// Read past the first `held` lines, to find where the new bytes start.
    fn skip(&mut self, held: usize) -> io::Result<()> {
        let file = File::open(&self.path)?;
        let meta = file.metadata()?;
        self.identity = identity(&meta);
        let mut reader = BufReader::new(file);
        let (sniff, head) = crate::document::sniff(&mut reader)?;
        if sniff != crate::document::Sniff::Text {
            self.whole = true;
            self.end = meta.len();
            return Ok(());
        }
        let mut reader = Cursor::new(head).chain(reader);
        let mut line = Vec::new();
        for _ in 0..held {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)? as u64;
            if read == 0 {
                // Shorter than the screen: it changed since it was read.
                self.whole = true;
                return Ok(());
            }
            self.end += read;
            if line.ends_with(b"\n") {
                self.offset += read;
            } else {
                self.partial = true;
            }
        }
        Ok(())
    }

    /// Look at the file again, and say what changed since the last look.
    pub(crate) fn poll(&mut self) -> Option<Change> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return (!std::mem::replace(&mut self.gone, true)).then_some(Change::Gone);
            }
            Err(err) => {
                log::debug!("follow: {}: {err}", self.path.display());
                return None;
            }
        };
        self.stamp = crate::scan::stamp_of(&meta).ok();
        if self.gone || identity(&meta) != self.identity || meta.len() < self.end {
            return Some(Change::Reset);
        }
        if meta.len() == self.end {
            return None;
        }
        if self.whole {
            return Some(Change::Reset);
        }
        match self.read_new() {
            Ok(change) => change,
            Err(err) => {
                log::debug!("follow: {}: {err}", self.path.display());
                None
            }
        }
    }

    /// Read from `offset` to the end, and split what came into lines, as
    /// `document::read_lines` splits them.
    fn read_new(&mut self) -> io::Result<Option<Change>> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(file);
        let replace_last = self.partial;
        let mut lines = Vec::new();
        let mut buf = Vec::new();
        let mut read = 0;
        self.partial = false;
        while let Some(line) = crate::document::read_lossy_line(&mut reader, &mut buf)? {
            read += buf.len() as u64;
            if buf.ends_with(b"\n") {
                self.offset += buf.len() as u64;
            } else {
                self.partial = true;
            }
            lines.push(line);
        }
        self.end = self.offset + if self.partial { buf.len() as u64 } else { 0 };
        log::debug!(
            "follow: {} grew by {read} bytes, {} lines",
            self.path.display(),
            lines.len()
        );
        Ok((!lines.is_empty()).then_some(Change::Grew {
            replace_last,
            lines,
        }))
    }

    /// Whether the last line read has no line end yet.
    pub(crate) fn partial(&self) -> bool {
        self.partial
    }
}

/// A change to the file a [`Watcher`] watches, for watch `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) id: u64,
    pub(crate) change: Change,
    /// The file's stamp when the change was seen.
    pub(crate) stamp: Option<crate::scan::Stamp>,
}

/// What a [`Watcher`] is told to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// Watch `path` after its first `held` lines, as watch `id`.
    Watch {
        id: u64,
        path: PathBuf,
        held: usize,
    },
    Stop,
}

/// At most one tail, and the commands that change it. The follow thread
/// runs one; a test runs one in its own thread, so a poll is a step it
/// controls.
#[derive(Debug, Default)]
pub(crate) struct Watcher {
    tail: Option<(u64, Tail)>,
}

impl Watcher {
    /// Do what `command` says. Starting a tail reads the file's head, so
    /// this runs where reading is allowed.
    pub(crate) fn obey(&mut self, command: Command) {
        self.tail = match command {
            Command::Watch { id, path, held } => Some((id, Tail::after_lines(&path, held))),
            Command::Stop => None,
        };
    }

    /// Poll the tail, if there is one. A reset ends it: the caller reads
    /// the file again and starts a new one.
    pub(crate) fn poll(&mut self) -> Option<Report> {
        let (id, tail) = self.tail.as_mut()?;
        let change = tail.poll()?;
        let report = Report {
            id: *id,
            stamp: tail.stamp,
            change,
        };
        if report.change == Change::Reset {
            self.tail = None;
        }
        Some(report)
    }
}

/// A [`Watcher`] on a thread of its own, polled every [`INTERVAL`].
#[derive(Debug)]
pub(crate) struct Background {
    commands: Sender<Command>,
    reports: Receiver<Report>,
}

impl Background {
    /// Start the thread. `None` when the OS refuses it: follow mode then
    /// sees no changes, and says so in the log.
    pub(crate) fn spawn() -> Option<Self> {
        let (commands, inbox) = std::sync::mpsc::channel();
        let (outbox, reports) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("recon-follow".to_string())
            .spawn(move || run(&inbox, &outbox));
        match spawned {
            Ok(_) => Some(Self { commands, reports }),
            Err(err) => {
                log::warn!("cannot start the follow thread: {err}; follow mode sees no changes");
                None
            }
        }
    }

    pub(crate) fn send(&self, command: Command) {
        // The thread ends only when this end is dropped.
        let _ = self.commands.send(command);
    }

    /// The reports that came since the last call.
    pub(crate) fn reports(&self) -> Vec<Report> {
        self.reports.try_iter().collect()
    }
}

/// The follow thread: obey each command, and poll between them. Ends when
/// the app drops its end of either channel.
fn run(inbox: &Receiver<Command>, outbox: &Sender<Report>) {
    let mut watcher = Watcher::default();
    loop {
        match inbox.recv_timeout(INTERVAL) {
            Ok(command) => watcher.obey(command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if let Some(report) = watcher.poll()
            && outbox.send(report).is_err()
        {
            return;
        }
    }
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn identity(_meta: &std::fs::Metadata) -> Identity {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{append, fixture_file};

    fn grew(lines: &[&str]) -> Change {
        Change::Grew {
            replace_last: false,
            lines: lines.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn a_file_that_grew_gives_only_its_new_lines() {
        let path = fixture_file("follow_grew.log", b"one\ntwo\n");
        let mut tail = Tail::after_lines(&path, 2);
        assert_eq!(tail.poll(), None);
        append(&path, b"three\nfour\n");
        assert_eq!(tail.poll(), Some(grew(&["three", "four"])));
        assert_eq!(tail.poll(), None);
    }

    /// Lines written after the screen was read, before the tail started,
    /// are not lost: the tail starts after the lines shown, not at the end.
    #[test]
    fn lines_written_before_the_tail_started_still_come() {
        let path = fixture_file("follow_late_start.log", b"one\n");
        append(&path, b"two\n");
        let mut tail = Tail::after_lines(&path, 1);
        assert_eq!(tail.poll(), Some(grew(&["two"])));
    }

    /// A last line with no line end is shown, and its new text replaces it.
    #[test]
    fn a_line_without_its_end_is_replaced_as_it_grows() {
        let path = fixture_file("follow_partial.log", b"one\ntw");
        let mut tail = Tail::after_lines(&path, 2);
        assert!(tail.partial());
        append(&path, b"o\nthr");
        assert_eq!(
            tail.poll(),
            Some(Change::Grew {
                replace_last: true,
                lines: vec!["two".to_string(), "thr".to_string()],
            })
        );
        assert!(tail.partial());
        append(&path, b"ee\r\n");
        assert_eq!(
            tail.poll(),
            Some(Change::Grew {
                replace_last: true,
                lines: vec!["three".to_string()],
            })
        );
        assert!(!tail.partial());
    }

    #[test]
    fn a_shorter_file_is_read_again_from_the_start() {
        let path = fixture_file("follow_truncated.log", b"one\ntwo\n");
        let mut tail = Tail::after_lines(&path, 2);
        std::fs::write(&path, b"x\n").expect("truncate");
        assert_eq!(tail.poll(), Some(Change::Reset));
    }

    /// logrotate: a new file at the path, even one as long as the old.
    #[cfg(unix)]
    #[test]
    fn a_replaced_file_is_read_again_from_the_start() {
        let path = fixture_file("follow_replaced.log", b"one\n");
        let mut tail = Tail::after_lines(&path, 1);
        let next = path.with_extension("next");
        std::fs::write(&next, b"one\ntwo\nthree\n").expect("write the new file");
        std::fs::rename(&next, &path).expect("rotate");
        assert_eq!(tail.poll(), Some(Change::Reset));
    }

    #[test]
    fn a_deleted_file_is_gone_once_then_read_again_when_it_comes_back() {
        let path = fixture_file("follow_deleted.log", b"one\n");
        let mut tail = Tail::after_lines(&path, 1);
        std::fs::remove_file(&path).expect("delete");
        assert_eq!(tail.poll(), Some(Change::Gone));
        assert_eq!(tail.poll(), None);
        std::fs::write(&path, b"new\n").expect("write it again");
        assert_eq!(tail.poll(), Some(Change::Reset));
    }

    /// A file shorter than the screen when the tail starts changed in
    /// between: it is read whole at the next change.
    #[test]
    fn a_file_shorter_than_the_screen_is_read_again_when_it_changes() {
        let path = fixture_file("follow_short.log", b"one\n");
        let mut tail = Tail::after_lines(&path, 3);
        assert_eq!(tail.poll(), None);
        append(&path, b"two\n");
        assert_eq!(tail.poll(), Some(Change::Reset));
    }
}
