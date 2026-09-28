//! The background scan that matches files against the filters, and
//! the checks that see a file change on disk.

use super::App;
use crate::widgets::explorer::Match;
use crate::{filter, scan};
use ratatui::prelude::Style;
use std::time::{Duration, Instant};

/// How often `poll_stamps` checks the listing's stamps while the feature is on.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The scan cache: one [`scan::Record`] per file, valid for exactly one
/// pattern list in one directory.
///
/// `key` changing shifts bit positions, so every record means something
/// else; the whole cache is dropped and `id` bumped so in-flight results from
/// the old one are ignored on arrival. `dir` changing means different files.
/// A single file's record is dropped alone when its stamp moves.
#[derive(Debug, Default)]
pub(super) struct ScanCache {
    pub(super) id: u64,
    key: Vec<String>,
    dir: std::path::PathBuf,
    pub(super) records: std::collections::HashMap<std::path::PathBuf, scan::Record>,
}

impl ScanCache {
    fn fresh(id: u64, key: Vec<String>, dir: std::path::PathBuf) -> Self {
        Self {
            id,
            key,
            dir,
            records: std::collections::HashMap::new(),
        }
    }
}

/// Everything `refresh_scan` depends on. Equal to last time ⇒ nothing to do.
///
/// The stamp stands in for the pattern list, the masks and the mode (OR or
/// AND, #39 — the same cached bitset answers differently under each), so the
/// comparison allocates nothing (#186). Only a change builds one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScanState {
    stamp: filter::ScanStamp,
    dir: std::path::PathBuf,
}

impl App<'_> {
    /// Decide whether the explorer's answers need work, and start it (#119).
    ///
    /// Cheap-idempotent unless `force`: it compares the pattern generation, the
    /// masks and the directory to what it saw last time and returns at once
    /// if nothing moved. Runs after every event, so that guard is what keeps a
    /// keystroke in the file view from walking the listing at all.
    ///
    /// When it proceeds, every file the explorer lists is answered from the
    /// cache if it can be — `Record::answer` — and put on a request if it
    /// cannot. A toggle whose every answer is cached issues no request and
    /// touches no thread; that is the whole point of caching bitsets rather
    /// than answers.
    pub(super) fn refresh_scan(&mut self, force: bool) {
        // Compared in place: this runs after every mouse move, and building
        // the pattern key, the directory and a clone of the set only to find
        // them unchanged was the cost of the common case (#186).
        let stamp = self.filters.scan_stamp();
        let unchanged = self
            .last_scan
            .as_ref()
            .map(|last| (last.stamp, last.dir.as_path()))
            == stamp.map(|stamp| (stamp, self.explorer.dir()));
        if !force && unchanged {
            return;
        }
        let dir = self.explorer.dir().to_path_buf();
        self.last_scan = stamp.map(|stamp| ScanState {
            stamp,
            dir: dir.clone(),
        });

        let Some(matcher) = self.filters.matcher() else {
            // Nothing selects: the feature is off, not "nothing matches".
            for (index, _) in self.explorer.files() {
                self.explorer.set_answer(index, Match::Unknown);
            }
            self.scanner.cancel();
            self.explorer.restyle();
            return;
        };

        let key = self.filters.pattern_key();
        if self.scan_cache.key != key || self.scan_cache.dir != dir {
            self.scan_cache = ScanCache::fresh(self.scan_cache.id + 1, key, dir);
        }

        // No `stat` here (#156): a held record is trusted as it stands.
        // `poll_stamps` finds a file that changed, off this thread, and a
        // resumed scan re-checks its own stamp in the worker.
        let mut pending = Vec::new();
        for (index, path) in self.explorer.files() {
            let answer = self
                .scan_cache
                .records
                .get(&path)
                .map(|record| self.answer_to_match(record, &matcher));
            let matched = if let Some(matched @ (Match::Yes(_) | Match::No)) = answer {
                matched
            } else {
                let (stamp, progress) = self
                    .scan_cache
                    .records
                    .get(&path)
                    .map(|record| (record.stamp, record.progress.clone()))
                    .unwrap_or_default();
                pending.push(scan::FileToScan {
                    index,
                    path,
                    stamp,
                    progress,
                });
                Match::Unknown
            };
            self.explorer.set_answer(index, matched);
        }

        if pending.is_empty() {
            self.scanner.cancel();
        } else {
            self.scanner.start(scan::Request {
                cache_id: self.scan_cache.id,
                matcher,
                files: pending,
            });
        }
        self.explorer.restyle();
    }

    /// Move scan results into the cache and the explorer, reporting whether
    /// anything on screen changed.
    ///
    /// A result is dropped if its cache id is stale — the pattern list changed
    /// while it was in flight, so its bitsets mean something else. Otherwise
    /// it replaces the held record only if it read further; a cancelled
    /// worker's partial can arrive after the fresh worker's complete. The row
    /// it names is checked against the path it is for before the explorer is
    /// told anything: the listing may have changed under it.
    pub(super) fn drain_scan_results(&mut self) -> bool {
        let Some(results) = self.scan_results.as_ref() else {
            return false;
        };
        let matcher = self.filters.matcher();
        let mut changed = false;
        loop {
            let scanned = match results.try_recv() {
                Ok(scanned) => scanned,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::warn!(
                        "the scan worker is gone; answers stay unknown until the next change"
                    );
                    break;
                }
            };
            if scanned.cache_id != self.scan_cache.id {
                continue;
            }
            let further = self
                .scan_cache
                .records
                .get(&scanned.path)
                .is_none_or(|held| {
                    // A new stamp is a new file: the worker started it over,
                    // so it can have read less and still be the truth (#156).
                    scanned.stamp != held.stamp
                        || scanned.progress.scanned_to > held.progress.scanned_to
                        || (scanned.progress.eof && !held.progress.eof)
                });
            if !further {
                continue;
            }
            let record = scan::Record {
                stamp: scanned.stamp,
                progress: scanned.progress,
            };
            let matched = matcher
                .as_ref()
                .map_or(Match::Unknown, |m| self.answer_to_match(&record, m));
            self.scan_cache.records.insert(scanned.path.clone(), record);
            if self.explorer.path_at(scanned.index).as_ref() == Some(&scanned.path) {
                changed |= self.explorer.set_answer(scanned.index, matched);
            }
        }
        if changed {
            self.explorer.restyle();
        }
        changed
    }

    /// Re-stat the listing every `POLL_INTERVAL` while the feature is on.
    /// Returns whether anything changed.
    ///
    /// The `stat`s run on a thread of their own (#156): one per listed file
    /// is nothing for a small local folder and a visible stall for 20,000
    /// files or a network mount. This tick only starts a check and, on a
    /// later tick, applies its answer. One check is in flight at a time, so
    /// a slow mount cannot pile threads up.
    pub(super) fn poll_stamps(&mut self) -> bool {
        let changed = self.drain_stamp_check();
        if !self.filters.is_scanning() || self.stamp_check.is_some() {
            return changed;
        }
        let now = Instant::now();
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < POLL_INTERVAL)
        {
            return changed;
        }
        self.last_poll = Some(now);
        self.stamp_check = scan::check_in_background(self.held_stamps());
        changed
    }

    /// Apply the in-flight stamp check's answer, if it has arrived.
    fn drain_stamp_check(&mut self) -> bool {
        let Some(check) = self.stamp_check.as_ref() else {
            return false;
        };
        match check.try_recv() {
            Ok(moved) => {
                self.stamp_check = None;
                self.apply_moved(moved)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                log::warn!("the stamp check ended without an answer");
                self.stamp_check = None;
                false
            }
        }
    }

    /// Every listed file that has a record, with the stamp the record holds —
    /// what a stamp check compares the disk against.
    fn held_stamps(&self) -> Vec<(std::path::PathBuf, Option<scan::Stamp>)> {
        self.explorer
            .files()
            .into_iter()
            .filter_map(|(_, path)| {
                let stamp = self.scan_cache.records.get(&path)?.stamp;
                Some((path, stamp))
            })
            .collect()
    }

    /// Drop and forget the records of files that moved on disk, then hand off
    /// to `refresh_scan(true)` to rescan them. The active file moving also
    /// raises the badge.
    ///
    /// The check ran on a snapshot, so each record is compared again before
    /// it is dropped: a scan result that arrived in the meantime may already
    /// carry the new stamp, and that record is kept.
    ///
    /// Deliberately does not issue its own request: `refresh_scan`'s `pending`
    /// is every file without a usable answer, which already covers the files
    /// this drops. Issuing a narrower request here would hand `Scanner::start`
    /// a file list that cancels an in-flight full scan without covering the
    /// files it had not reached yet, stranding them `Unknown` until `r`.
    pub(super) fn apply_moved(&mut self, moved: Vec<scan::Moved>) -> bool {
        if !self.filters.is_scanning() {
            return false;
        }
        let moved: std::collections::HashMap<_, _> = moved
            .into_iter()
            .map(|scan::Moved { path, stamp }| (path, stamp))
            .collect();
        let active = self.view.filename().to_path_buf();
        let mut changed = false;
        for (index, path) in self.explorer.files() {
            let Some(stamp) = moved.get(&path) else {
                continue;
            };
            let Some(held) = self.scan_cache.records.get(&path) else {
                continue;
            };
            if held.stamp == *stamp {
                continue;
            }
            self.scan_cache.records.remove(&path);
            self.explorer.set_answer(index, Match::Unknown);
            if path == active {
                self.view_stale = true;
            }
            changed = true;
        }
        if changed {
            self.refresh_scan(true);
        }
        changed
    }

    /// A stamp check run to completion on this thread — what `poll_stamps`
    /// does over two ticks, without the thread or the wait. For `r`, which
    /// asks for it, and for tests.
    pub(super) fn check_stamps(&mut self) -> bool {
        let moved = scan::moved(self.held_stamps());
        self.apply_moved(moved)
    }

    /// A record's answer as the explorer's `Match`, with the owning filter's
    /// colour on a yes.
    fn answer_to_match(&self, record: &scan::Record, matcher: &filter::Matcher) -> Match {
        match record.answer(matcher) {
            Some(true) => Match::Yes(self.match_style(record.owner(matcher))),
            Some(false) => Match::No,
            None => Match::Unknown,
        }
    }

    /// The style the view would draw a line selected by `owner` with. The
    /// explorer draws the file's name in it, so the two panes agree at a
    /// glance and the colour says *which* filter picked the file.
    fn match_style(&self, owner: Option<filter::Owner>) -> Style {
        owner
            .and_then(|index| self.filters.style_for(filter::Verdict::Included(index)))
            .unwrap_or_default()
    }
}
