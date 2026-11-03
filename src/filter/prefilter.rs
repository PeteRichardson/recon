//! Whether to ask a `RegexSet` `is_match` before `matches` (#592).
//!
//! `matches` must look for every pattern, so it is the slow call; `is_match`
//! stops at the first hit. On a log where few lines hit a filter, asking
//! `is_match` first skips `matches` on most lines: 57 ms became 33 ms on a
//! 2M-line log with four filters. A line that hits pays for both calls, so
//! where most lines hit — a broad include such as `INFO|DEBUG` — asking
//! first was 37–60% slower. The break-even was between 40% and 60% of lines.
//!
//! So a pass counts its hits, and each window decides the next one: at 30%
//! hits or more it calls `matches` directly. The answers are the same either
//! way; only the cost moves.

use regex::{RegexSet, SetMatches};

/// Lines in one window: enough that one burst of hits does not switch the
/// asking off, few enough that a change of section in a log is followed.
const WINDOW: u32 = 256;

/// How one pass over many lines asks the compiled set which patterns hit.
///
/// A [`VerdictPass`](super::VerdictPass) or a [`BitsPass`](super::BitsPass)
/// holds one, and asks it for each line in order. A fresh one is correct,
/// only slower, so a caller with one line makes one for it.
///
/// A hit is a hit of any pattern in the set, and the set holds disabled
/// filters too. So a broad filter that is disabled can switch the asking
/// off: the pass then costs what it did before #592, never more.
#[derive(Debug, Clone)]
pub(super) struct Prefilter {
    /// Whether `is_match` is asked before `matches`.
    asks_first: bool,
    /// Lines and hits in the window so far.
    lines: u32,
    hits: u32,
}

impl Default for Prefilter {
    /// Asking first: the usual log has few lines that a filter hits.
    fn default() -> Self {
        Self {
            asks_first: true,
            lines: 0,
            hits: 0,
        }
    }
}

impl Prefilter {
    /// Which patterns of `set` hit `line`, or `None` when none do.
    pub(super) fn matches(&mut self, set: &RegexSet, line: &str) -> Option<SetMatches> {
        let found = if self.asks_first && !set.is_match(line) {
            None
        } else {
            Some(set.matches(line)).filter(SetMatches::matched_any)
        };
        self.lines += 1;
        self.hits += u32::from(found.is_some());
        if self.lines == WINDOW {
            self.asks_first = self.hits * 10 < WINDOW * 3;
            self.lines = 0;
            self.hits = 0;
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> RegexSet {
        RegexSet::new(["alpha", r"beta\d"]).unwrap()
    }

    fn hits(found: Option<SetMatches>) -> Vec<usize> {
        found.map(|m| m.iter().collect()).unwrap_or_default()
    }

    #[test]
    fn answers_as_the_set_does_on_hits_and_misses() {
        let set = set();
        let mut prefilter = Prefilter::default();
        assert_eq!(hits(prefilter.matches(&set, "alpha beta1")), [0, 1]);
        assert_eq!(hits(prefilter.matches(&set, "beta2")), [1]);
        assert!(prefilter.matches(&set, "gamma").is_none());
        assert!(prefilter.matches(&set, "").is_none());
    }

    #[test]
    fn a_window_of_common_hits_stops_the_asking() {
        let set = set();
        let mut prefilter = Prefilter::default();
        for _ in 0..WINDOW {
            prefilter.matches(&set, "alpha");
        }
        assert!(!prefilter.asks_first);
        // Its answers are the same with the asking off.
        assert_eq!(hits(prefilter.matches(&set, "beta3 alpha")), [0, 1]);
        assert!(prefilter.matches(&set, "gamma").is_none());
    }

    #[test]
    fn a_window_of_rare_hits_starts_the_asking_again() {
        let set = set();
        let mut prefilter = Prefilter::default();
        for _ in 0..WINDOW {
            prefilter.matches(&set, "alpha");
        }
        for row in 0..WINDOW {
            prefilter.matches(&set, if row % 10 == 0 { "alpha" } else { "gamma" });
        }
        assert!(prefilter.asks_first);
    }

    #[test]
    fn hits_below_three_in_ten_keep_the_asking_on() {
        let set = set();
        let mut prefilter = Prefilter::default();
        for row in 0..WINDOW {
            prefilter.matches(&set, if row % 4 == 0 { "alpha" } else { "gamma" });
        }
        assert!(prefilter.asks_first);
    }
}
