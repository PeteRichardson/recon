//! recon's status, reported to the terminal after each frame (#638).
//!
//! A report goes out only when the status differs from the last one sent,
//! so scrolling, which changes nothing that the status says, sends nothing.
//! See ADR 0022.

use std::io::Write;

use super::App;
use super::render::status::Total;
use crate::batch::count;
use crate::program_status::{self, Status, short_count};

/// The writer the reports go to, and the last status sent on it.
pub(super) struct StatusReports {
    sink: Box<dyn Write>,
    last: Option<Status>,
}

impl StatusReports {
    /// Write `status`, unless it is the last one written.
    fn send(&mut self, status: Status) {
        if self.last.as_ref() == Some(&status) {
            return;
        }
        let bytes = program_status::encode(&status);
        // A report that cannot be written is lost, not an error: the
        // session goes on, and the next change tries again.
        if let Err(err) = self.sink.write_all(&bytes).and_then(|()| self.sink.flush()) {
            log::debug!("program status: {err}");
        }
        self.last = Some(status);
    }
}

impl App<'_> {
    /// Report the status to `sink` after each frame. `main` gives the
    /// screen the frames go to, never stdout: `--emit` writes there.
    #[must_use]
    pub fn with_status_reports(mut self, sink: Box<dyn Write>) -> Self {
        self.status_reports = Some(StatusReports { sink, last: None });
        self
    }

    /// Send the status, if it changed since the last report.
    pub(super) fn report_status(&mut self) {
        // Out of `self` for the call, as `status` reads the rest of it.
        let Some(mut reports) = self.status_reports.take() else {
            return;
        };
        reports.send(self.status());
        self.status_reports = Some(reports);
    }

    /// What the status is now: the busy work, if there is any, or the
    /// numbers the status bar gives, in the order a glance needs them.
    fn status(&self) -> Status {
        let name = self.view_name();
        if let Some(busy) = self.busy(&name) {
            return busy;
        }
        if self.view.showing_directory() {
            return Status::idle(format!("{name}/"));
        }
        if self.filters.is_empty() {
            return Status::idle(format!("{name} · {}", self.total()));
        }
        let filters = count(self.filters.row_count(), "filter");
        let shown = short_count(self.document.visible().len());
        Status::idle(format!("{name} · {filters} · {shown}/{}", self.total()))
    }

    /// The base name of what the view shows, or the name its title gives
    /// stdin's file.
    fn view_name(&self) -> String {
        if let Some(label) = self.stdin_label() {
            return label.to_string();
        }
        crate::path::file_label(self.view.filename())
    }

    /// The busy work, when there is some. One report holds one piece of
    /// work, so with more than one, the order below chooses: the work
    /// that blocks the user first (ADR 0022).
    fn busy(&self, name: &str) -> Option<Status> {
        let work = |msg: String| Some(Status::working(msg, None));
        if self.finishing.is_some() {
            return work("finishing".to_string());
        }
        if let Some(path) = self.loading.pending_path() {
            return work(format!("loading {}", crate::path::file_label(path)));
        }
        if let Some(percent) = self.pass_percent() {
            return Some(Status::working(format!("filtering {name}"), Some(percent)));
        }
        let lines = || short_lines(self.document.lines().len());
        if self.running_spool().is_some() {
            let command = self.loading.view_badge().unwrap_or("command");
            return work(format!("running {command} · {}", lines()));
        }
        if self.reading_stdin() {
            return work(format!("reading stdin · {}", lines()));
        }
        // A file with no answer is work only while a scan is out for it:
        // with no filter that selects, or before the first scan starts,
        // each file has no answer and nothing reads it.
        if self.last_scan.is_some() {
            let (answered, files) = self.explorer.answered();
            if answered < files {
                return work(format!("scanning {}", count(files, "file")));
            }
        }
        None
    }

    /// The total, short, with its noun: `1 line`, `88k lines`. A
    /// preview's total is marked as a guess with `~`, as the status bar
    /// marks it.
    fn total(&self) -> String {
        let (lines, mark) = match self.total_lines() {
            Total::Exact(lines) => (lines, ""),
            Total::Preview(lines) | Total::Estimate(lines) => (lines, "~"),
        };
        format!("{mark}{}", short_lines(lines))
    }
}

/// `lines` short, with its noun: `1 line`, `88k lines`.
fn short_lines(lines: usize) -> String {
    let noun = if lines == 1 { "line" } else { "lines" };
    format!("{} {noun}", short_count(lines))
}
