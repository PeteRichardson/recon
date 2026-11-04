//! A filter pass on a worker thread for a very large file (#597).
//!
//! On a file of `PASS_LINES` lines or more, a filter change does not judge
//! the lines on the main thread. A worker judges them in blocks, and the
//! view shows each block's rows when it comes. Until the pass ends, only
//! rows that have a new verdict show, and the status row shows how far the
//! pass has got. Below the threshold, a filter change is as it always was.
//!
//! A new filter change drops the pass that runs and starts another. Each
//! pass has its own channel, so no block of an old pass can come after, and
//! a worker whose channel is gone stops at its next block. Under test the
//! pass runs in the test's own thread: each `poll_filtering` takes one
//! block.

use super::App;
use super::follow::Grew;
use super::viewport::is_interesting;
use crate::document::{Block, Pass};
use crate::filter::ActiveFilters;
use std::sync::mpsc::{Receiver, TryRecvError};

/// The fewest lines a file has for a filter change to judge them on a
/// worker. Today's pass over 2M lines takes 60-80 ms, so a pass over 4M is
/// about the first that a user sees as a stop.
pub(crate) const PASS_LINES: usize = 4_000_000;

/// The lines in each block a worker sends.
pub(crate) const BLOCK_LINES: usize = 64 * 1024;

/// Where a pass runs.
enum Runner {
    /// In the caller's thread, one block for each `poll_filtering`: for
    /// tests.
    #[cfg(test)]
    Here(std::iter::Peekable<Box<dyn Iterator<Item = Block>>>),
    /// On a worker. The channel closes when the worker has dropped its
    /// copy of the lines, which is what `append` waits for: until then,
    /// adding lines would copy the file.
    Worker(Receiver<Block>),
}

impl Runner {
    /// The blocks that came, and whether the pass is over. `wait`: all the
    /// blocks still to come, judged before this returns.
    fn take(&mut self, wait: bool) -> (Vec<Block>, bool) {
        match self {
            #[cfg(test)]
            Self::Here(blocks) => {
                if wait {
                    return (blocks.collect(), true);
                }
                let block = blocks.next().into_iter().collect();
                (block, blocks.peek().is_none())
            }
            Self::Worker(blocks) => {
                let mut taken = Vec::new();
                if wait {
                    taken.extend(blocks.iter());
                    return (taken, true);
                }
                loop {
                    match blocks.try_recv() {
                        Ok(block) => taken.push(block),
                        Err(TryRecvError::Empty) => return (taken, false),
                        Err(TryRecvError::Disconnected) => return (taken, true),
                    }
                }
            }
        }
    }
}

/// A pass that runs.
struct Running {
    runner: Runner,
    /// The source line the cursor was on when the pass started. The cursor
    /// goes to the last row shown until a block judges this line, then to
    /// it. `None` once the user moves the cursor.
    target: Option<usize>,
    /// The source line the pass put the cursor on, to see whether the user
    /// moved it since.
    placed: Option<usize>,
    /// An `n` (`Some(false)`) or `N` (`Some(true)`) that found no
    /// interesting line in the rows judged when it was pressed. An `n`
    /// lands on the first one a block judges after `target`; an `N`, and
    /// an `n` that finds none, steps again when the pass ends.
    step: Option<bool>,
    /// The row a waiting `n` looks from next, so no row is read twice.
    scanned: usize,
    /// What the followed file grew by while the pass ran, in order.
    held: Vec<Grew>,
}

/// Filter passes in blocks: when they run, and the one that runs.
pub(super) struct Filtering {
    /// A document with this many lines or more takes a pass in blocks.
    pub(super) threshold: usize,
    /// The lines in each block.
    pub(super) block: usize,
    /// Run the pass on a worker under test too.
    #[cfg(test)]
    pub(super) on_worker: bool,
    running: Option<Running>,
}

impl Default for Filtering {
    fn default() -> Self {
        Self {
            threshold: PASS_LINES,
            block: BLOCK_LINES,
            #[cfg(test)]
            on_worker: false,
            running: None,
        }
    }
}

impl Filtering {
    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Drop the pass that runs, if one does, and give back the followed
    /// file's lines it held.
    pub(super) fn stop(&mut self) -> Vec<Grew> {
        self.running
            .take()
            .map(|running| running.held)
            .unwrap_or_default()
    }

    /// Hold the new lines of a followed file until the pass ends. `Err`
    /// gives them back when no pass runs.
    pub(super) fn hold(&mut self, grew: Grew) -> Result<(), Grew> {
        match &mut self.running {
            Some(running) => {
                running.held.push(grew);
                Ok(())
            }
            None => Err(grew),
        }
    }

    fn runner(&self, pass: Pass, filters: ActiveFilters) -> Option<Runner> {
        #[cfg(test)]
        if !self.on_worker {
            let blocks: Box<dyn Iterator<Item = Block>> =
                Box::new(pass.blocks(filters, self.block));
            return Some(Runner::Here(blocks.peekable()));
        }
        let (send, blocks) = std::sync::mpsc::channel();
        let size = self.block;
        let spawned = std::thread::Builder::new()
            .name("recon-filter".into())
            .spawn(move || {
                // A panic here stops this pass only (#245): its channel
                // closes before the pass ends, and the view says so.
                let judged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    for block in pass.blocks(filters, size) {
                        if send.send(block).is_err() {
                            return;
                        }
                    }
                }));
                if judged.is_err() {
                    log::error!("filter pass: the worker panicked");
                }
            });
        match spawned {
            Ok(_) => Some(Runner::Worker(blocks)),
            // #387: no thread is not a reason to stop. The caller judges
            // the lines here, as it does below the threshold.
            Err(err) => {
                log::warn!("filter pass: no worker thread ({err}); judging here");
                None
            }
        }
    }
}

impl App<'_> {
    /// Start a pass in blocks for a filter change, when the document is
    /// large enough. `false` when it is not: then the caller judges the
    /// lines itself.
    pub(super) fn start_pass(&mut self) -> bool {
        let lines = self.document.lines().len();
        if lines == 0 || lines < self.filtering.threshold {
            return false;
        }
        // The line the cursor is on, before the visible set goes.
        let target = self.cursor_source();
        let held = self.filtering.stop();
        let pass = self.document.begin_pass(&self.filters);
        let Some(runner) = self.filtering.runner(pass, self.filters.snapshot()) else {
            self.judge_here();
            self.take_held(held);
            return true;
        };
        self.filtering.running = Some(Running {
            runner,
            target: Some(target),
            placed: None,
            step: None,
            scanned: 0,
            held,
        });
        self.filters.note_over_limit(self.document.over_limit());
        self.apply_view(target);
        true
    }

    /// Take what the pass judged since the last call, and say whether the
    /// screen changed.
    pub(super) fn poll_filtering(&mut self) -> bool {
        self.take_blocks(false)
    }

    /// Wait for the pass to end, when one runs: for `q` and `:w`, which
    /// read every visible line.
    pub(super) fn finish_pass_here(&mut self) {
        self.take_blocks(true);
    }

    fn take_blocks(&mut self, wait: bool) -> bool {
        let Some(running) = &mut self.filtering.running else {
            return false;
        };
        let (blocks, over) = running.runner.take(wait);
        let changed = !blocks.is_empty();
        for block in blocks {
            self.document.take_block(&self.filters, block);
        }
        if changed {
            self.filters.note_over_limit(self.document.over_limit());
            self.place_cursor_in_pass();
        }
        if !over {
            return changed;
        }
        let Some(running) = self.filtering.running.take() else {
            return changed;
        };
        if !self.document.judged() {
            self.report(
                "the filter pass stopped: the view shows only part of the file",
                true,
            );
        } else if let Some(backwards) = running.step {
            // No block had the line: `n` and `N` wrap or go to the next
            // file, as they do with no pass.
            self.step_interesting(backwards);
        }
        self.take_held(running.held);
        true
    }

    /// `n` or `N` while a pass runs, after the rows judged so far had no
    /// interesting line for it: it waits for the blocks. `false`, and
    /// nothing done, when no pass runs.
    ///
    /// The step is from the line the cursor goes to, while it still goes
    /// to one: the line it was on before the pass.
    pub(super) fn step_when_judged(&mut self, backwards: bool) -> bool {
        let cursor = self.cursor_source();
        let Some(running) = &mut self.filtering.running else {
            return false;
        };
        let from = *running.target.get_or_insert(cursor);
        running.step = Some(backwards);
        running.scanned = from + 1;
        true
    }

    fn take_held(&mut self, held: Vec<Grew>) {
        for grew in held {
            self.take_new_lines(grew);
        }
    }

    /// Put the cursor where the pass rule says, after a block: on the last
    /// row shown until a block judges its line, then on it. A waiting `n`
    /// lands on the first interesting line a block judges. A cursor the
    /// user moved stays where the user put it.
    fn place_cursor_in_pass(&mut self) {
        let here = self.document.source_at(self.view.cursor_visible_row());
        let verdicts = self.document.verdicts();
        let judged = verdicts.len();
        let Some(running) = &mut self.filtering.running else {
            return;
        };
        if running.placed.is_some() && here != running.placed {
            running.target = None;
            running.step = None;
        }
        let mut hit = None;
        if running.step == Some(false) {
            hit = (running.scanned..judged).find(|&row| is_interesting(verdicts[row]));
            running.scanned = running.scanned.max(judged);
        }
        if hit.is_some() {
            running.step = None;
            running.target = None;
        }
        match (hit, running.target) {
            (Some(hit), _) => self.land_on(hit),
            (None, Some(target)) if target < judged => self.apply_view(target),
            (None, Some(_)) => self.place_cursor_at_end(),
            (None, None) => self.apply_view(here.unwrap_or(0)),
        }
        let placed = self.document.source_at(self.view.cursor_visible_row());
        if let Some(running) = &mut self.filtering.running {
            running.placed = placed;
        }
    }

    /// The pass's progress for the status row: `filtering 37%`.
    pub(super) fn filtering_text(&self) -> Option<String> {
        if !self.filtering.is_running() || self.document.judged() {
            return None;
        }
        let total = self.document.lines().len().max(1);
        let judged = self.document.verdicts().len();
        Some(format!("filtering {}%", judged * 100 / total))
    }
}
