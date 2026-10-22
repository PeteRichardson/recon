//! Follow mode (#493): `Ctrl-t` keeps the file in the view current as it
//! grows, as `tail -F` does.
//!
//! Follow mode is a property of a file, for the session. Only the file on
//! screen has a tail: the others are only marked in the explorer, and a
//! followed file is read whole again when the view comes back to it, so its
//! tail starts from what is on screen. The tail reads on a thread of its
//! own (ADR 0007); under test it reads when the test calls `poll_follow`.

use super::App;
use super::loading::Landing;
use crate::follow::{Background, Change, Command, Report, Watcher};
use crate::path::file_label;
use crate::scan;
use crate::views::Extent;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Shown when `Ctrl-t` finds the file in a view that has no lines of its own.
pub(super) const TEXT_VIEW_ONLY: &str = "follow mode works in the text view";

/// The badge saying the file on screen is followed, without its padding:
/// with autoscroll paused it counts the new lines too.
pub(super) const FOLLOW_BADGE: &str = "FOLLOW";

/// Where the tail runs.
enum Runner {
    /// In the caller's thread, when it calls `poll_follow`: for tests.
    #[cfg_attr(not(test), allow(dead_code))]
    Here(Watcher),
    /// On the follow thread, which the first tail starts.
    NotStarted,
    Running(Background),
    /// The OS refused the thread; it is not asked again.
    Refused,
}

impl Default for Runner {
    fn default() -> Self {
        if cfg!(test) {
            Self::Here(Watcher::default())
        } else {
            Self::NotStarted
        }
    }
}

/// Follow mode's state.
#[derive(Default)]
pub(super) struct Following {
    /// Every followed file, by absolute path.
    files: HashSet<PathBuf>,
    runner: Runner,
    /// The watch in force: its number, and the file it is for. A report
    /// for an older number is for a file the view has left.
    watch: Option<(u64, PathBuf)>,
    /// The number the next watch takes.
    next_id: u64,
    /// Lines that came while the cursor was not on the last line, which
    /// paused autoscroll. The status row counts them until it resumes.
    unseen: usize,
}

impl Following {
    pub(super) fn is_followed(&self, path: &Path) -> bool {
        self.files.contains(path)
    }

    /// The followed files, for the explorer's marks.
    pub(super) fn files(&self) -> &HashSet<PathBuf> {
        &self.files
    }

    /// Follow `path` from the start of the session: `--follow FILE`.
    pub(super) fn start_with(&mut self, path: PathBuf) {
        self.files.insert(path);
    }

    fn send(&mut self, command: Command) {
        if matches!(self.runner, Runner::NotStarted) {
            if command == Command::Stop {
                return;
            }
            self.runner = Background::spawn().map_or(Runner::Refused, Runner::Running);
        }
        match &mut self.runner {
            Runner::Here(watcher) => watcher.obey(command),
            Runner::Running(background) => background.send(command),
            Runner::NotStarted | Runner::Refused => {}
        }
    }

    fn reports(&mut self) -> Vec<Report> {
        match &mut self.runner {
            Runner::Here(watcher) => watcher.poll().into_iter().collect(),
            Runner::Running(background) => background.reports(),
            Runner::NotStarted | Runner::Refused => Vec::new(),
        }
    }
}

impl App<'_> {
    /// `Ctrl-t`: follow the file in the view, or stop following it.
    pub(super) fn toggle_follow(&mut self) {
        let path = self.view.filename().to_path_buf();
        if !self.can_follow() {
            self.report(TEXT_VIEW_ONLY, false);
            return;
        }
        let name = file_label(&path);
        if self.following.files.remove(&path) {
            log::debug!("follow: off for {}", path.display());
            self.report(&format!("stopped following {name}"), false);
        } else {
            log::debug!("follow: on for {}", path.display());
            self.following.files.insert(path);
            self.report(&format!("following {name}"), false);
            // A preview is read whole first; its load starts the tail.
            self.promote_truncated_preview();
        }
        self.explorer.set_followed(self.following.files());
        self.view_stale = false;
        self.rewatch();
    }

    /// Whether the view shows a file's own text, in the raw view.
    fn can_follow(&self) -> bool {
        self.loading.shows_raw() && self.view.is_text() && !self.view.showing_directory()
    }

    /// Start the tail of the file on screen when it is followed and read
    /// whole, and stop any other. Called after every load.
    pub(super) fn rewatch(&mut self) {
        let path = self.view.filename().to_path_buf();
        let wanted =
            self.following.is_followed(&path) && self.can_follow() && !self.view.is_truncated();
        if !wanted {
            if self.following.watch.take().is_some() {
                self.following.send(Command::Stop);
            }
            return;
        }
        let id = self.following.next_id;
        self.following.next_id += 1;
        self.following.watch = Some((id, path.clone()));
        self.following.unseen = 0;
        let held = self.document.lines().len();
        self.following.send(Command::Watch { id, path, held });
    }

    /// Whether the file on screen has a tail: what the badge says.
    pub(super) fn following_shown(&self) -> bool {
        self.following
            .watch
            .as_ref()
            .is_some_and(|(_, path)| path == self.view.filename())
    }

    /// The follow badge, with the count of lines autoscroll has not shown.
    pub(super) fn follow_badge(&self) -> Option<String> {
        if !self.following_shown() {
            return None;
        }
        Some(match self.following.unseen {
            0 => format!(" {FOLLOW_BADGE} "),
            1 => format!(" {FOLLOW_BADGE} · 1 new line "),
            unseen => format!(" {FOLLOW_BADGE} · {unseen} new lines "),
        })
    }

    /// Take what the tail found since the last call, and say whether the
    /// screen changed.
    pub(super) fn poll_follow(&mut self) -> bool {
        let mut changed = false;
        for report in self.following.reports() {
            let Some((id, path)) = self.following.watch.clone() else {
                continue;
            };
            if report.id != id {
                continue;
            }
            changed = true;
            match report.change {
                Change::Grew {
                    replace_last,
                    lines,
                } => self.take_new_lines(lines, replace_last, &path, report.stamp),
                Change::Reset => {
                    log::debug!("follow: {} read again from the start", path.display());
                    let landing = if self.cursor_at_end() {
                        Landing::End
                    } else {
                        Landing::Keep(self.cursor_source())
                    };
                    self.following.watch = None;
                    self.open_view(&path, Extent::Full, landing);
                    let name = file_label(&path);
                    self.report(
                        &format!(
                            "{name} became shorter or was replaced: read again from the start"
                        ),
                        false,
                    );
                }
                Change::Gone => {
                    let name = file_label(&path);
                    self.report(&format!("{name} is gone: follow mode waits for it"), false);
                }
            }
        }
        changed
    }

    /// Add the lines the followed file grew by, filtered as the others
    /// are. With the cursor on the last line the view scrolls to them;
    /// otherwise the cursor stays, and the status row counts them.
    fn take_new_lines(
        &mut self,
        lines: Vec<String>,
        replace_last: bool,
        path: &Path,
        stamp: Option<scan::Stamp>,
    ) {
        let at_end = self.cursor_at_end();
        let cursor = self.cursor_source();
        let visible = self.document.visible().len();
        let from = self.document.lines().len() - usize::from(replace_last);
        self.view.release_source();
        self.document.append(lines, replace_last, &self.filters);
        self.filters.note_over_limit(self.document.over_limit());
        self.view
            .extend_source(Arc::clone(self.document.shared_lines()), from);
        // A replaced line can leave the visible set as it was.
        self.last_generation = None;
        if at_end {
            self.place_cursor_at_end();
        } else {
            self.apply_view(cursor);
            self.following.unseen += self.document.visible().len().saturating_sub(visible);
        }
        // The explorer's mark for the file: its scan reads only the new part.
        if self.filters.is_scanning() {
            self.apply_moved(vec![scan::Moved {
                path: path.to_path_buf(),
                stamp,
            }]);
        }
    }

    /// Whether the cursor is on the last visible line: autoscroll's rule.
    pub(super) fn cursor_at_end(&self) -> bool {
        let visible = self.document.visible().len();
        self.view.cursor_visible_row() + 1 >= visible
    }

    /// Put the cursor on the last visible line, at the bottom of the pane.
    pub(super) fn place_cursor_at_end(&mut self) {
        let Some(last) = self.document.visible().len().checked_sub(1) else {
            return;
        };
        self.view.land_cursor_on_last_row();
        self.place_cursor_on_visible_row(last);
    }

    /// Autoscroll resumes when the cursor is back on the last line: the
    /// count of lines it did not show starts again.
    pub(super) fn note_follow_cursor(&mut self) {
        if self.following.unseen > 0 && self.cursor_at_end() {
            self.following.unseen = 0;
        }
    }
}
