//! `q` under `--emit`, when the output needs work the panes have not done
//! (#351, #352).
//!
//! `--emit lines` over a file the view holds only a preview of must read the
//! rest; `--emit files` while the scan has files to answer must wait for
//! them. Either way recon completes the work rather than emitting the part
//! it has, and the user can cancel it: the TUI stays up, the status row
//! shows progress, and `finishing.cancel` (Ctrl-c or Esc) ends the session
//! with nothing on stdout and exit 130.
//!
//! The file is read on a thread of its own, one line at a time, and the
//! lines it emits go to a spool file rather than to memory, so a 20 GB log
//! is slow and never dangerous. The files are read by the scan worker that
//! was already reading them; this only waits for its answers.

use super::viewport::is_interesting;
use super::{App, AppState};
use crate::document::{self, Mode};
use crate::emit;
use crate::filter::ActiveFilters;
use crate::keymap::ActionId;
use crate::syntax::KindSet;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

/// The work `q` started, while it runs.
#[derive(Debug)]
pub(super) enum Finish {
    /// The whole of a file the view holds a preview of.
    Lines(LinesJob),
    /// The files the scan has not answered yet. The scan worker reads them;
    /// nothing here does.
    Files,
}

/// The thread reading the rest of a file for `--emit lines`.
#[derive(Debug)]
pub(super) struct LinesJob {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    /// Bytes read so far, written by the thread.
    read: Arc<AtomicU64>,
    /// The file's length when the read started.
    total: u64,
    done: Receiver<io::Result<Option<Streamed>>>,
}

/// A file read to its end, with its visible lines in a spool.
#[derive(Debug)]
pub(super) struct Streamed {
    spool: std::fs::File,
    emitted: usize,
    interesting: usize,
}

impl App<'_> {
    /// `q`: quit and emit — after the work the output still needs, if it
    /// needs any.
    pub(super) fn quit_emitting(&mut self) {
        let finish = match self.emit {
            Some(emit::Emit::Lines) if self.lines_need_finishing() => {
                Some(Finish::Lines(LinesJob::start(
                    self.view.filename(),
                    self.filters.snapshot(),
                    self.document.mode(),
                    self.line_numbers,
                )))
            }
            Some(emit::Emit::Files) if self.files_need_finishing() => Some(Finish::Files),
            _ => None,
        };
        self.state = if finish.is_some() {
            AppState::Finishing
        } else {
            AppState::Quit { emit: true }
        };
        self.finishing = finish;
    }

    /// Whether `--emit lines` needs more of the file than the view holds:
    /// the view is showing a file's text, and only a preview of it (#351).
    fn lines_need_finishing(&self) -> bool {
        !self.view.showing_directory() && self.view.is_text() && self.view.is_truncated()
    }

    /// Whether `--emit files` would list a file the scan has not answered
    /// (#352). Dim mode lists it either way, but its answer is what the
    /// summary counts, and a completed run has no "unscanned".
    fn files_need_finishing(&self) -> bool {
        self.filters.is_scanning()
            && self
                .explorer
                .listed_files()
                .iter()
                .any(|file| file.matched.is_none())
    }

    /// See whether the work `q` started is done, and end the session when it
    /// is. Returns whether the screen needs drawing again.
    pub(super) fn poll_finish(&mut self) -> bool {
        let before = self.finishing_text();
        let done = match &self.finishing {
            None => return false,
            Some(Finish::Files) => !self.files_need_finishing(),
            Some(Finish::Lines(job)) => match job.done.try_recv() {
                Ok(result) => {
                    self.finished = Some(self.streamed_exit(&job.path, result));
                    true
                }
                Err(TryRecvError::Empty) => false,
                Err(TryRecvError::Disconnected) => {
                    self.finished = Some(self.streamed_exit(
                        &job.path,
                        Err(io::Error::other("the reading thread stopped")),
                    ));
                    true
                }
            },
        };
        if done {
            self.finishing = None;
            self.state = AppState::Quit { emit: true };
            return true;
        }
        self.finishing_text() != before
    }

    /// `finishing.cancel`: stop the work and emit nothing.
    pub(super) fn cancel_finish(&mut self) {
        if let Some(Finish::Lines(job)) = &self.finishing {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.finishing = None;
        self.state = AppState::Cancelled;
    }

    /// What the status row says while the work runs: how far it has got,
    /// and the key that cancels it, as the keymap in force binds it.
    pub(super) fn finishing_text(&self) -> Option<String> {
        let progress = match self.finishing.as_ref()? {
            Finish::Lines(job) => format!(
                "finishing {}: {} of {}",
                job.path.file_name().map_or_else(
                    || job.path.display().to_string(),
                    |name| { name.to_string_lossy().into_owned() }
                ),
                size(job.read.load(Ordering::Relaxed).min(job.total)),
                size(job.total),
            ),
            Finish::Files => {
                let (answered, total) = self.explorer.answered();
                format!("finishing: {answered} of {total} files scanned")
            }
        };
        Some(match self.keymap.label_for(ActionId::FinishingCancel) {
            Some(key) => format!("{progress} — {key} to cancel"),
            None => progress,
        })
    }

    /// The session's result from the reading thread's.
    fn streamed_exit(&self, path: &Path, result: io::Result<Option<Streamed>>) -> emit::Exit {
        match result {
            Ok(Some(streamed)) => emit::Exit::Spooled {
                summary: self.lines_summary(streamed.emitted, streamed.interesting),
                spool: streamed.spool,
            },
            // Only a cancel stops the thread early, and a cancel ends the
            // session before its answer is read.
            Ok(None) => emit::Exit::Cancelled,
            // Nothing on stdout: part of a file is not the file. Exit 2 is
            // headless mode's code for an input that could not be read.
            Err(err) => emit::Exit::Emit {
                lines: Vec::new(),
                summary: format!("recon: cannot read {}: {err}", path.display()),
                failed: 1,
            },
        }
    }
}

impl LinesJob {
    fn start(path: &Path, filters: ActiveFilters, mode: Mode, line_numbers: bool) -> Self {
        let path = path.to_path_buf();
        let cancel = Arc::new(AtomicBool::new(false));
        let read = Arc::new(AtomicU64::new(0));
        let total = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        let (send, done) = std::sync::mpsc::channel();
        {
            let (path, cancel, read) = (path.clone(), Arc::clone(&cancel), Arc::clone(&read));
            std::thread::spawn(move || {
                let result = stream(&path, &filters, mode, line_numbers, &cancel, &read);
                // The receiver is gone only when the session ended first.
                let _ = send.send(result);
            });
        }
        Self {
            path,
            cancel,
            read,
            total,
            done,
        }
    }
}

/// Read `path` to its end, and write the lines visible in `mode` to a spool,
/// each prefixed by its line number and a tab under `-n` — what
/// `collect_lines` emits from a document held whole, from a file that is
/// not. `None` when `cancel` stopped it.
///
/// A definition filter needs the grammar of the whole file, which one line
/// at a time cannot give, so with one enabled the file is read whole, as
/// the view reads it when it is promoted.
pub(super) fn stream(
    path: &Path,
    filters: &ActiveFilters,
    mode: Mode,
    line_numbers: bool,
    cancel: &AtomicBool,
    read: &AtomicU64,
) -> io::Result<Option<Streamed>> {
    let mut out = BufWriter::new(emit::spool()?);
    let anything_including = filters.any_including();
    let (mut emitted, mut interesting) = (0, 0);
    let mut write = |number: usize, text: &str, verdict| -> io::Result<()> {
        if is_interesting(&verdict) {
            interesting += 1;
        }
        if document::shows(mode, verdict, anything_including) {
            if line_numbers {
                write!(out, "{number}\t")?;
            }
            out.write_all(text.as_bytes())?;
            out.write_all(b"\n")?;
            emitted += 1;
        }
        Ok(())
    };
    let complete = if filters.needs_kinds() {
        let mut whole = document::Document::read(path)?;
        read.store(
            std::fs::metadata(path).map_or(0, |meta| meta.len()),
            Ordering::Relaxed,
        );
        whole.evaluate(filters);
        let mut complete = true;
        for (row, (text, verdict)) in whole.lines().iter().zip(whole.verdicts()).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                complete = false;
                break;
            }
            write(row + 1, text, *verdict)?;
        }
        complete
    } else {
        let mut number = 0;
        document::each_line(path, read, |text| {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            number += 1;
            write(number, &text, filters.verdict(&text, KindSet::EMPTY))?;
            Ok(true)
        })?
    };
    if !complete {
        return Ok(None);
    }
    let spool = out.into_inner().map_err(io::IntoInnerError::into_error)?;
    Ok(Some(Streamed {
        spool,
        emitted,
        interesting,
    }))
}

/// A byte count for the progress line: `812 B`, `4.1 KB`, `20.0 GB`.
fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::size;

    #[test]
    fn sizes_read_as_a_person_would_say_them() {
        assert_eq!(size(812), "812 B");
        assert_eq!(size(4_100), "4.1 KB");
        assert_eq!(size(20_000_000_000), "20.0 GB");
    }
}
