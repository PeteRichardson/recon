//! recon's status, reported to the terminal after each frame (#638).
//!
//! A report goes out only when the status differs from the last one sent,
//! so scrolling, which changes nothing that the status says, sends nothing.
//! Long work ends with `done` until the next key (#640). See ADR 0022.

use std::io::Write;
use std::time::{Duration, Instant};

use super::App;
use super::render::status::Total;
use crate::batch::count;
use crate::program_status::{self, Status, short_count};

/// How long busy work must take for its end to be `done`, not `idle`.
/// Shorter work is over before the user can look away, and a `done`
/// marker for each move in the explorer would mean nothing (ADR 0022).
pub(super) const LONG_WORK: Duration = Duration::from_secs(2);

/// The writer the reports go to, and the last status sent on it.
pub(super) struct StatusReports {
    sink: Box<dyn Write>,
    last: Option<Status>,
    /// When the busy period began, and the last work in it.
    busy: Option<(Instant, Work)>,
    /// The result of long work, sent until the next key.
    done: Option<Status>,
}

/// The kind of busy work, for the result its end reports.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Work {
    Finishing,
    Loading,
    Filtering,
    Spool(String),
    Stdin,
    Scan,
}

impl StatusReports {
    /// Note `work` as busy at `now`. The period goes on from its start.
    fn working(&mut self, work: Work, now: Instant) {
        let since = self.busy.take().map_or(now, |(since, _)| since);
        self.busy = Some((since, work));
        self.done = None;
    }

    /// End the busy period at `now`, and give its last work if the period
    /// was long.
    fn ended(&mut self, now: Instant) -> Option<Work> {
        let (since, work) = self.busy.take()?;
        (now.duration_since(since) >= LONG_WORK).then_some(work)
    }

    /// The user pressed a key, so the result has been seen.
    pub(super) fn seen(&mut self) {
        self.done = None;
    }

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
        self.status_reports = Some(StatusReports {
            sink,
            last: None,
            busy: None,
            done: None,
        });
        self
    }

    /// Send the status, if it changed since the last report.
    pub(super) fn report_status(&mut self) {
        // Out of `self` for the call, as `status` reads the rest of it.
        let Some(mut reports) = self.status_reports.take() else {
            return;
        };
        // The click clock, so that a test sets one time for both.
        let now = self.click_clock.now();
        let name = self.view_name();
        let status = if let Some((work, status)) = self.busy(&name) {
            reports.working(work, now);
            status
        } else {
            if let Some(result) = reports
                .ended(now)
                .and_then(|work| self.result(&work, &name))
            {
                reports.done = Some(Status::done(result));
            }
            reports.done.clone().unwrap_or_else(|| self.idle(&name))
        };
        reports.send(status);
        self.status_reports = Some(reports);
    }

    /// The numbers the status bar gives, in the order a glance needs them.
    fn idle(&self, name: &str) -> Status {
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
    fn busy(&self, name: &str) -> Option<(Work, Status)> {
        let work = |kind, msg, progress| Some((kind, Status::working(msg, progress)));
        if self.finishing.is_some() {
            return work(Work::Finishing, "finishing".to_string(), None);
        }
        if let Some(path) = self.loading.pending_path() {
            let msg = format!("loading {}", crate::path::file_label(path));
            return work(Work::Loading, msg, None);
        }
        if let Some(percent) = self.pass_percent() {
            return work(Work::Filtering, format!("filtering {name}"), Some(percent));
        }
        let lines = || short_lines(self.document.lines().len());
        if self.running_spool().is_some() {
            let command = self.loading.view_badge().unwrap_or("command").to_string();
            let msg = format!("running {command} · {}", lines());
            return work(Work::Spool(command), msg, None);
        }
        if self.reading_stdin() {
            return work(Work::Stdin, format!("reading stdin · {}", lines()), None);
        }
        // A file with no answer is work only while a scan is out for it:
        // with no filter that selects, or before the first scan starts,
        // each file has no answer and nothing reads it.
        if self.last_scan.is_some() {
            let (answered, files) = self.explorer.answered();
            if answered < files {
                return work(
                    Work::Scan,
                    format!("scanning {}", count(files, "file")),
                    None,
                );
            }
        }
        None
    }

    /// What `work` that ended gave, for its `done` report. `q` that
    /// finished ends recon, so it has none.
    fn result(&self, work: &Work, name: &str) -> Option<String> {
        let lines = || short_lines(self.document.lines().len());
        Some(match work {
            Work::Finishing => return None,
            Work::Loading => format!("loaded {name}: {}", self.total()),
            Work::Filtering => {
                let shown = short_count(self.document.visible().len());
                format!("filtered {name}: {shown} of {}", self.total())
            }
            Work::Spool(command) => format!("ran {command}: {}", lines()),
            Work::Stdin => format!("read stdin: {}", lines()),
            Work::Scan => format!("scanned {}", count(self.explorer.answered().1, "file")),
        })
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
