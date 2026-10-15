//! Showing a file: the request to the view pipeline, the short wait for
//! its answer, and what happens when the answer comes late (#455, ADR 0007).
//!
//! The main thread never reads a file. It sends a job and waits up to
//! `WAIT` for the answer, so that raw and hex show as they did when they
//! were read here. A later answer is adopted by the event loop, and an
//! answer for a file the user has already left is dropped.

use super::App;
use crate::views::cycle::{HandOff, Steps, Table};
use crate::views::pool::{Executor, Request};
use crate::views::read::{self, Contents};
use crate::views::{Done, Extent, FileType, Job, Shown, View};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the main thread waits for a view before it draws a
/// placeholder. Long enough for a raw or hex read, which then shows with no
/// "loading" frame; short enough that a slow view never stops a held key.
const WAIT: Duration = Duration::from_millis(30);

/// Files prefetched ahead of the selection in the direction of travel.
const PREFETCH_AHEAD: usize = 8;

/// Files prefetched behind the selection.
const PREFETCH_BEHIND: usize = 2;

/// What the pane shows while a view is late.
pub(super) const PLACEHOLDER: &str = "<loading…>";

/// The spinner's frames, one each `SPIN` while a view is late.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

const SPIN: Duration = Duration::from_millis(100);

/// Where the cursor goes once the view arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Landing {
    /// The top of the file: a new file, or a new view of one.
    Top,
    /// Back on this source line, or the nearest visible one: `r`, and a
    /// preview made whole.
    Keep(usize),
    /// The first interesting line from that end: a cross-file step.
    FirstInteresting { backwards: bool },
}

/// A request whose answer has not come.
#[derive(Debug, Clone)]
struct Pending {
    path: PathBuf,
    extent: Extent,
    landing: Landing,
    since: Instant,
    /// The spinner frame last drawn.
    frame: u128,
}

impl Pending {
    /// The spinner frame for now: one each `SPIN` since the request.
    fn frame_now(&self) -> u128 {
        self.since.elapsed().as_millis() / SPIN.as_millis()
    }
}

/// The view pipeline as `App` sees it.
pub(super) struct Loading {
    /// Where jobs run: a pool in the app, a test executor under test.
    pub(super) executor: Box<dyn Executor>,
    table: Arc<Table>,
    steps: Steps,
    /// The number of the newest request. An answer with another number is
    /// for a file the user has left.
    generation: u64,
    pending: Option<Pending>,
    /// The row of the file on screen and the view the pane shows it in:
    /// the one asked for, or hex when raw failed. `None` when the pane does
    /// not show a file's content: a directory, an error, a placeholder.
    shown: Option<Shown>,
    /// The explorer entry the last preview was for, to tell the direction
    /// of travel.
    last_entry: Option<usize>,
}

/// No rows of the user's, and an executor that runs nothing: what a
/// `App::default()` holds until `App::new` builds the real one.
impl Default for Loading {
    fn default() -> Self {
        Self::new(Box::default(), &[])
    }
}

impl Loading {
    pub(super) fn new(executor: Box<dyn Executor>, rows: &[FileType]) -> Self {
        let table = Table::new(rows);
        Self {
            executor,
            steps: Steps::new(&table),
            table: Arc::new(table),
            generation: 0,
            pending: None,
            shown: None,
            last_entry: None,
        }
    }

    fn job(&self, path: &Path, extent: Extent) -> Job {
        Job {
            path: path.to_path_buf(),
            extent,
            table: Arc::clone(&self.table),
            steps: self.steps.clone(),
        }
    }

    /// The view on screen, for the status line: `None` for raw, which is
    /// what a file is unless the status line says otherwise.
    pub(super) fn view_badge(&self) -> Option<&'static str> {
        self.shown
            .map(|shown| shown.view)
            .filter(|view| *view != View::Raw)
            .map(View::name)
    }

    /// Where `o` sends `path`, the file on screen (#456). The row the pane
    /// shows it in usually says whether it is binary; only a file shown by
    /// a user row, or not shown yet, is read here.
    pub(super) fn hand_off(&self, path: &Path) -> HandOff {
        let known = self.shown.and_then(|shown| self.table.binary(shown.row));
        self.table
            .hand_off(path, || known.unwrap_or_else(|| read::is_binary(path)))
    }

    /// The spinner's frame while a view is late.
    pub(super) fn spinner(&self) -> Option<char> {
        let pending = self.pending.as_ref()?;
        let frame = usize::try_from(pending.frame_now()).unwrap_or(0);
        Some(SPINNER[frame % SPINNER.len()])
    }

    /// Whether `path` is already on its way at `extent`.
    fn is_pending(&self, path: &Path, extent: Extent) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.path == path && pending.extent == extent)
    }
}

impl App<'_> {
    /// Show `path` read to `extent`, and put the cursor where `landing`
    /// says. Waits up to `WAIT` for the answer; past that the pane shows a
    /// placeholder, and `drain_views` shows the file when it comes.
    pub(super) fn open_view(&mut self, path: &Path, extent: Extent, landing: Landing) {
        let loading = &mut self.loading;
        loading.generation += 1;
        let generation = loading.generation;
        let job = loading.job(path, extent);
        loading.executor.submit(Request { generation, job });
        loading.pending = Some(Pending {
            path: path.to_path_buf(),
            extent,
            landing,
            since: Instant::now(),
            frame: 0,
        });
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.loading.executor.recv_timeout(left) {
                Some(answer) if answer.generation == generation => {
                    self.adopt_view(answer.done, landing);
                    return;
                }
                // An answer for a file the user has left.
                Some(_) => {}
                None => break,
            }
        }
        // `r` and a preview made whole keep the file on screen meanwhile.
        if !matches!(landing, Landing::Keep(_)) {
            self.loading.shown = None;
            self.view
                .show(path, &Contents::message(PLACEHOLDER.to_string()));
            self.sync_document();
            self.refresh_view();
        }
    }

    /// Show any answer that has come since the last wake, and say whether
    /// the screen needs drawing: for an answer, or a new spinner frame.
    pub(super) fn drain_views(&mut self) -> bool {
        let mut changed = false;
        while let Some(answer) = self.loading.executor.recv_timeout(Duration::ZERO) {
            let Some(pending) = self.loading.pending.as_ref() else {
                continue;
            };
            // Only the newest request is pending; any other answer is for a
            // file the user has left.
            if answer.generation == self.loading.generation {
                let landing = pending.landing;
                self.adopt_view(answer.done, landing);
                changed = true;
            }
        }
        // Drawn again once per spinner frame, not on every wake.
        changed
            || self.loading.pending.as_mut().is_some_and(|pending| {
                let frame = pending.frame_now();
                std::mem::replace(&mut pending.frame, frame) != frame
            })
    }

    /// Put an answer on screen.
    fn adopt_view(&mut self, done: Done, landing: Landing) {
        self.loading.pending = None;
        let contents = &done.contents;
        self.view.show(&done.path, contents);
        self.loading.shown = done
            .shown
            .filter(|_| contents.text || contents.hex)
            .map(|shown| Shown {
                view: if contents.fallback {
                    View::Hex
                } else {
                    shown.view
                },
                ..shown
            });
        self.sync_document();
        match landing {
            Landing::Top => self.refresh_view(),
            Landing::Keep(source) => {
                self.document.evaluate(&self.filters);
                let row = self
                    .document
                    .nearest_visible(source)
                    .and_then(|nearest| self.document.visible_position(nearest))
                    .unwrap_or(0);
                self.place_cursor_on_visible_row(row);
            }
            Landing::FirstInteresting { backwards } => {
                self.refresh_view();
                if let Some(target) = self.first_interesting(backwards) {
                    self.land_on(target);
                }
            }
        }
        if let Some(shown) = done.shown
            && contents.fallback
        {
            self.report(
                &format!("{} failed → hex: binary file", shown.view.name()),
                true,
            );
        }
    }

    /// Queue the views of the files near the explorer's selection, so that
    /// they are ready when the selection reaches them.
    pub(super) fn prefetch_near_selection(&mut self) {
        let entry = self.explorer.selected_entry();
        let backwards = matches!(
            (self.loading.last_entry, entry),
            (Some(last), Some(now)) if now < last
        );
        self.loading.last_entry = entry;
        let jobs = self
            .explorer
            .files_near(PREFETCH_AHEAD, PREFETCH_BEHIND, backwards)
            .iter()
            .map(|path| self.loading.job(path, Extent::Preview))
            .collect();
        self.loading.executor.prefetch(jobs);
    }

    /// Make a preview whole, keeping the cursor on its line. No-op when the
    /// view is not a preview, or the whole file is already on its way.
    ///
    /// Every path that uses the view calls this first: a key in the view,
    /// `n`/`N`, and a committed `/`. Without it, `n` would wrap inside the
    /// preview for ever, and `/` would report "no matches" for a pattern
    /// that only occurs past the preview's cap. The read is a job (#383):
    /// when it is late, the key acts on the preview, and the whole file
    /// replaces it with the cursor kept on its line.
    pub(super) fn promote_truncated_preview(&mut self) {
        let path = self.view.filename().to_path_buf();
        if !self.file_view_truncated() || self.loading.is_pending(&path, Extent::Full) {
            return;
        }
        let source = self.cursor_source();
        self.open_view(&path, Extent::Full, Landing::Keep(source));
    }

    /// How much of the file on screen is read: a new read of it reads as
    /// much again, so `r` on a preview of a 2 GB log stays a preview (#383).
    pub(super) fn current_extent(&self) -> Extent {
        if self.file_view_truncated() {
            Extent::Preview
        } else {
            Extent::Full
        }
    }

    /// `-`: the next view in the file's cycle, back to the first after the
    /// last. The step holds for every file of the same row for the session.
    pub(super) fn next_view(&mut self) {
        let Some(shown) = self.loading.shown else {
            self.report("no file to show in another view", false);
            return;
        };
        if self.loading.table.views(shown.row).len() < 2 {
            self.report("this file has one view", false);
            return;
        }
        self.loading.steps.next(&self.loading.table, shown.row);
        let path = self.view.filename().to_path_buf();
        self.open_view(&path, self.current_extent(), Landing::Top);
    }
}
