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

    /// What the status is now: the numbers the status bar gives, in the
    /// order a glance needs them.
    fn status(&self) -> Status {
        let name = self
            .view
            .filename()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.view.showing_directory() {
            return Status::idle(format!("{name}/"));
        }
        if self.filters.is_empty() {
            return Status::idle(format!("{name} · {} lines", self.total()));
        }
        let filters = count(self.filters.row_count(), "filter");
        // While a pass runs, the shown count goes up with each block: out
        // of the message, so that a pass sends two reports, not one per
        // block.
        if self.pass_running() {
            return Status::idle(format!("{name} · {filters}"));
        }
        let shown = short_count(self.document.visible().len());
        Status::idle(format!(
            "{name} · {filters} · {shown}/{} lines",
            self.total()
        ))
    }

    /// The total, short. A preview's total is marked as a guess with `~`,
    /// as the status bar marks it.
    fn total(&self) -> String {
        match self.total_lines() {
            Total::Exact(lines) => short_count(lines),
            Total::Preview(lines) | Total::Estimate(lines) => format!("~{}", short_count(lines)),
        }
    }
}
