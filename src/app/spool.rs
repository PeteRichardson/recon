//! A command view's spool on screen (#573).
//!
//! The view shows a spool as follow mode shows a growing file: the tail
//! reads the lines the command adds. This adds what only a spool has: a
//! badge that says the command runs and counts the lines so far, `Ctrl-c`
//! to stop it, and, when it is over, the file read again whole, as an index
//! when it is large (#51). The app keeps one spool, for the file on screen:
//! a new view drops it, which kills the command and deletes the file.

use super::loading::Landing;
use super::{App, AppState};
use crate::views::spool::{Spool, State};
use std::path::Path;
use std::sync::Arc;

/// The spool of the view on screen.
pub(super) struct Spooled {
    spool: Spool,
    /// Whether the command is over and its lines are on screen.
    over: bool,
}

impl App<'_> {
    /// Keep the spool of the view just adopted, or none: the one before is
    /// dropped, which kills its command and deletes its file.
    pub(super) fn keep_spool(&mut self, spool: Option<Spool>) {
        self.spool = spool.map(|spool| Spooled { spool, over: false });
    }

    /// The spool, while its command runs.
    fn running(&mut self) -> Option<&mut Spooled> {
        self.spool.as_mut().filter(|spooled| !spooled.over)
    }

    /// The file the running command writes into, for the tail.
    pub(super) fn running_spool(&self) -> Option<&Path> {
        self.spool
            .as_ref()
            .filter(|spooled| !spooled.over)
            .map(|spooled| spooled.spool.path())
    }

    /// The badge that says the command runs, with the lines so far.
    pub(super) fn spool_badge(&self) -> Option<String> {
        self.running_spool()?;
        let lines = crate::batch::count(self.document.lines().len(), "line");
        Some(format!(" RUNNING · {lines} "))
    }

    /// `Ctrl-c` (#382): stop a command view's running command, and keep its
    /// lines, as in a shell (#573). With none, quit; the next one quits.
    pub(super) fn interrupt(&mut self) {
        match self.running() {
            Some(spooled) => spooled.spool.stop(),
            None => self.state = AppState::Cancelled,
        }
    }

    /// Take the end of the command once it is over, and say whether the
    /// screen changed: its lines read again whole, and the reason when it
    /// stopped.
    pub(super) fn poll_spool(&mut self) -> bool {
        let Some(spooled) = self.running() else {
            return false;
        };
        let state = spooled.spool.state();
        if state == State::Running {
            return false;
        }
        let lines = spooled.spool.take_lines();
        spooled.over = true;
        // The tail stops: the lines below are all of them.
        self.rewatch();
        if let Some(lines) = lines {
            self.show_spooled_lines(lines);
        }
        if let State::Stopped(reason) = state {
            let name = self.loading.view_badge().unwrap_or("command");
            let text = format!("{name} stopped: {reason}");
            log::warn!("spool: {text}");
            self.report(&text, true);
        }
        true
    }

    /// Show `lines`, all the command wrote, in place of the lines the tail
    /// read. The cursor keeps its line, or the end when it was there.
    fn show_spooled_lines(&mut self, lines: crate::lines::Lines) {
        let landing = if self.cursor_at_end() {
            Landing::End
        } else {
            Landing::Keep(self.cursor_source())
        };
        self.view.extend_source(Arc::new(lines), 0);
        self.sync_document(None);
        self.land(landing);
    }
}
