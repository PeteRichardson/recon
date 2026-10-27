//! `:w` in the command palette (#528): the lines `--emit lines` would print
//! on quit, written to a file while the session goes on.
//!
//! Over a file the view holds only a preview of, the rest is read on a
//! thread into a spool, as `q` reads it (#351), and the same thread writes
//! the file only when the read is done: a cancel or a read error leaves no
//! part of a file.

use super::App;
use super::finish::{LinesJob, Streamed};
use crate::emit;
use crate::keymap::ActionId;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// The read `:w` started over a preview, while it runs.
#[derive(Debug)]
pub(super) struct Writing {
    /// The file the thread writes when the read is done.
    path: PathBuf,
    job: LinesJob,
    /// The progress the status row shows, to draw again only when it moves.
    shown: String,
}

impl App<'_> {
    /// `w PATH` and `w! PATH`: write the file view's visible lines to
    /// `PATH`, a relative one in the explorer's folder. `w` refuses a file
    /// that exists, and `w!` replaces it.
    pub(super) fn write_lines(&mut self, argument: &str, replace: bool) -> Result<(), String> {
        let name = if replace { "w!" } else { "w" };
        if argument.is_empty() {
            return Err(format!("{name}: give a path"));
        }
        if self.writing.is_some() {
            return Err(match self.keymap.label_for(ActionId::GlobalEscape) {
                Some(key) => format!("{name}: a write is running; {key} stops it"),
                None => format!("{name}: a write is running"),
            });
        }
        if self.view.showing_directory() {
            return Err(format!("{name}: the view is showing a folder, not a file"));
        }
        if !self.view.is_text() {
            return Err(format!("{name}: the view is showing an error, not a file"));
        }
        let path = self.resolve_path(argument)?;
        if path.is_dir() {
            return Err(format!("{argument}: is a folder"));
        }
        // Before the read, which can be long; `create` checks again.
        let exists = || format!("{argument}: the file exists; w! replaces it");
        if !replace && path.exists() {
            return Err(exists());
        }
        if self.lines_need_finishing() {
            let target = path.clone();
            let job = self.start_lines_job(move |mut streamed: Streamed| {
                let mut out = BufWriter::new(create(&target, replace)?);
                streamed.copy_to(&mut out)?;
                out.flush()?;
                Ok(streamed)
            });
            self.writing = Some(Writing {
                shown: job.progress(),
                path,
                job,
            });
            return Ok(());
        }
        let count = self.document.visible().len();
        let file = create(&path, replace).map_err(|err| match err.kind() {
            io::ErrorKind::AlreadyExists => exists(),
            _ => format!("{argument}: {err}"),
        })?;
        let mut out = BufWriter::new(file);
        let written = match self.collect(emit::Emit::Lines) {
            emit::Exit::Emit { lines, .. } => emit::write_lines(&lines, &mut out),
            emit::Exit::Spooled { mut spool, .. } => emit::write_spool(&mut spool, &mut out),
            _ => Ok(()),
        };
        written
            .and_then(|()| out.flush())
            .map_err(|err| format!("{argument}: {err}"))?;
        self.report_written(count, &path);
        Ok(())
    }

    /// See whether the write `:w` started is done. Returns whether the
    /// screen needs drawing again.
    pub(super) fn poll_write(&mut self) -> bool {
        let Some(writing) = self.writing.as_mut() else {
            return false;
        };
        let Some(result) = writing.job.poll() else {
            let progress = writing.job.progress();
            let moved = progress != writing.shown;
            writing.shown = progress;
            return moved;
        };
        let path = self.writing.take().expect("checked above").path;
        match result {
            Ok(Some(streamed)) => self.report_written(streamed.emitted(), &path),
            // A cancel takes `writing` before the thread answers.
            Ok(None) => {}
            Err(err) => self.report(&format!("w: cannot write {}: {err}", path.display()), true),
        }
        true
    }

    /// Esc, when nothing else took it: stop the read `:w` started, and write
    /// nothing.
    pub(super) fn cancel_write(&mut self) {
        if let Some(writing) = self.writing.take() {
            writing.job.stop();
            self.report("w: stopped; nothing written", false);
        }
    }

    /// What the status row says while the read runs: how far it has got,
    /// and the key that stops it.
    pub(super) fn writing_text(&self) -> Option<String> {
        let writing = self.writing.as_ref()?;
        let progress = format!(
            "writing {}: {}",
            crate::path::file_label(&writing.path),
            writing.shown,
        );
        Some(self.with_cancel_key(progress, ActionId::GlobalEscape))
    }

    fn report_written(&mut self, count: usize, path: &Path) {
        self.report(&format!("wrote {count} lines to {}", path.display()), false);
    }
}

/// Open `path` to write: a new file only, unless `replace`. One call, so no
/// file can appear between a check and the open.
fn create(path: &Path, replace: bool) -> io::Result<File> {
    let mut options = File::options();
    options.write(true);
    if replace {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(path)
}
