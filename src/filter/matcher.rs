//! The scan-side snapshot of the filter set: what a thread matching whole
//! files in the navigator needs from [`ActiveFilters`], and nothing more.

use super::{ActiveFilters, Combine, Sense};
use regex::RegexSet;

/// The bitset width. Up to 64 patterns in total; past this the navigator's
/// file matching switches off rather than shifting out of range.
const MAX_PATTERNS: usize = 64;

/// Which filter selected a file, for its colour in the navigator: the
/// filter's index. The lowest index wins — the view's "first matching filter
/// wins", applied per file.
///
/// Used to be an enum with a `Search` variant that outranked every numbered
/// filter. A search is a motion now (ADR 0001) and marks no file, so only
/// the index is left.
pub type Owner = usize;

/// A `Send` snapshot of the filter set, for a scan thread to match with (#119).
///
/// `ActiveFilters` is neither `Send` nor `Clone`. This is the three things a
/// scan needs from it: every pattern compiled into one `RegexSet` (cloning one
/// is an `Arc` bump), and which positions currently select or exclude. The set
/// covers every pattern whether or not it is enabled — a deliberate choice in
/// #86 — which is what makes a line's `bits` independent of the enabled mask,
/// and so reusable across toggles.
///
/// `Context` filters are in neither mask: they neither select nor exclude, so
/// a line only a context filter hit contributes nothing to a file's answer.
#[derive(Debug, Clone)]
pub struct Matcher {
    set: RegexSet,
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Include`.
    selects: u64,
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Exclude`.
    exclude: u64,
    /// How the include bits combine. In `And` mode a line selects only when
    /// every include bit in `selects` is set.
    combine: Combine,
}

impl Matcher {
    /// Which patterns hit `line`, enabled or not.
    #[must_use]
    pub fn bits(&self, line: &str) -> u64 {
        self.set
            .matches(line)
            .iter()
            .fold(0, |bits, index| bits | (1 << index))
    }

    /// Whether a line with these hits selects its file, and no enabled
    /// `Exclude` filter hits it.
    ///
    /// `Or`: an enabled `Include` filter hits it. `And`: every enabled
    /// `Include` filter hits it. With no include filter enabled, `And`
    /// selects nothing, the same as `Or`.
    #[must_use]
    pub fn selects(&self, bits: u64) -> bool {
        if bits & self.exclude != 0 {
            return false;
        }
        let includes = self.selects;
        match self.combine {
            Combine::Or => bits & includes != 0,
            Combine::And => includes != 0 && bits & includes == includes,
        }
    }

    /// Which filter selected a line with these hits, if any. In `And` mode
    /// every include filter hit, so this is the first enabled one — the same
    /// colour `verdict` gives the line.
    #[must_use]
    pub fn owner(&self, bits: u64) -> Option<Owner> {
        if !self.selects(bits) {
            return None;
        }
        Some((bits & self.selects).trailing_zeros() as usize)
    }

    /// `(selects, exclude, combine)`, for the caller that wants to know
    /// whether a toggle changed anything a scan cares about. The mode is part
    /// of it: the same bitset answers differently under each.
    #[must_use]
    pub fn masks(&self) -> (u64, u64, Combine) {
        (self.selects, self.exclude, self.combine)
    }
}

impl ActiveFilters {
    /// The snapshot a scan thread matches with, or `None` when there is no
    /// scan to run.
    ///
    /// `None` when nothing selects — no enabled `Include`, which is the same
    /// "nothing to match against" guard `Document` applies for #36 — and when
    /// the pattern count exceeds the bitset width.
    #[must_use]
    pub fn matcher(&self) -> Option<Matcher> {
        let (set, selects, exclude) = self.scan_masks()?;
        Some(Matcher {
            set: set.clone(),
            selects,
            exclude,
            combine: self.combine,
        })
    }

    /// Whether there is a scan to run: `matcher().is_some()` without
    /// cloning the set. For the render loop, which asks sixty times a
    /// second (#170).
    #[must_use]
    pub fn is_scanning(&self) -> bool {
        self.scan_masks().is_some()
    }

    /// The compiled set and the `(selects, exclude)` masks over it, or
    /// `None` when there is no scan to run. `matcher` and `is_scanning`
    /// share it so they cannot disagree.
    fn scan_masks(&self) -> Option<(&RegexSet, u64, u64)> {
        debug_assert!(
            self.compiled.as_ref().is_none_or(|set| self.in_step(set)),
            "the compiled set is out of step with the filters"
        );
        let set = self.compiled.as_ref().filter(|set| self.in_step(set))?;
        if set.len() > MAX_PATTERNS {
            return None;
        }
        let mut selects = 0u64;
        let mut exclude = 0u64;
        for (index, filter) in self.filters.iter().enumerate() {
            // A definition predicate is not a regex over the line's text,
            // and a scan reads text it never parses: its bit stays out of
            // every mask, so it neither selects nor excludes a file. The
            // view still applies it per line.
            if !self.effective(index) || filter.predicate.as_regex().is_none() {
                continue;
            }
            match filter.sense {
                Sense::Include => selects |= 1 << index,
                Sense::Exclude => exclude |= 1 << index,
                Sense::Context => {}
            }
        }
        if selects == 0 {
            return None;
        }
        Some((set, selects, exclude))
    }

    /// Every pattern's source, in compiled order. What a scan
    /// cache is keyed on: a change here shifts bit positions, so cached
    /// bitsets mean something else. Sense and enabled are deliberately not
    /// part of it — they are masks over the same bits.
    #[must_use]
    pub fn pattern_key(&self) -> Vec<String> {
        self.filters
            .iter()
            .map(|filter| filter.predicate.key())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::set_with;
    use super::super::*;
    use super::MAX_PATTERNS;

    // ---- the matcher snapshot --------------------------------------------

    /// The invariant the navigator rests on, stated the way the spec states
    /// it: a line selects its file when an enabled `Include` filter hits it
    /// and no enabled `Exclude` does. Deliberately *not*
    /// derived from `verdict`'s index — that is a colouring rule (first match
    /// wins), and a context filter can win the colour of a line an include
    /// filter also hit. Selecting and colouring are different questions.
    #[test]
    fn the_matcher_agrees_with_the_spec_on_what_selects_a_file() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        set.add_excluding("noise").expect("valid pattern");
        let matcher = set.matcher().expect("something selects");

        for line in [
            "alpha",
            "beta",
            "delta",
            "alpha noise",
            "beta delta",
            "gamma",
            "gamma noise",
            "beta gamma",
            "nothing here",
            "alpha beta",
            "beta noise",
            "delta noise",
        ] {
            let hit = |sense: Sense| {
                set.filters().iter().any(|f| {
                    f.enabled && f.sense == sense && f.predicate.holds(line, KindSet::EMPTY)
                })
            };
            let expected = hit(Sense::Include) && !hit(Sense::Exclude);

            assert_eq!(
                matcher.selects(matcher.bits(line)),
                expected,
                "matcher and the spec disagree on {line:?}"
            );
            // A selected line is always one the view shows.
            if expected {
                assert!(
                    matches!(set.verdict(line, KindSet::EMPTY), Verdict::Included(_)),
                    "{line:?} selects its file but the view would not show it"
                );
            }
        }
    }

    /// The owner is the lowest *selecting* filter — which is not always the
    /// filter `verdict` colours the line with. A line hit by context filter 1
    /// and include filter 2 is drawn in filter 1's colour (first wins) but the
    /// *file* is owned by filter 2: it is the one that selected it.
    #[test]
    fn the_owner_is_the_lowest_selecting_filter() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        let matcher = set.matcher().expect("something selects");

        assert_eq!(matcher.owner(matcher.bits("beta")), None);
        assert_eq!(matcher.owner(matcher.bits("beta delta")), Some(2));
        assert_eq!(
            set.verdict("beta delta", KindSet::EMPTY),
            Verdict::Included(2),
            "the include filter outranks the context one, so the view's colour \
             is the navigator's"
        );
        assert_eq!(matcher.owner(matcher.bits("alpha delta")), Some(0));
    }

    #[test]
    fn a_disabled_filter_neither_selects_nor_excludes() {
        let mut set = set_with(&["alpha"]);
        set.add_excluding("noise").expect("valid pattern");
        set.set_enabled(1, false);
        let matcher = set.matcher().expect("alpha selects");

        assert!(matcher.selects(matcher.bits("alpha noise")));
    }

    /// Nothing selecting means nothing to match against — the #36 guard.
    #[test]
    fn no_matcher_without_a_selecting_filter() {
        assert!(ActiveFilters::new().matcher().is_none(), "empty set");

        let mut context_only = set_with(&["alpha"]);
        context_only.toggle_context(0);
        assert!(context_only.matcher().is_none(), "context only");

        let mut exclude_only = ActiveFilters::new();
        exclude_only.add_excluding("noise").expect("valid pattern");
        assert!(exclude_only.matcher().is_none(), "exclude only");

        let mut disabled = set_with(&["alpha"]);
        disabled.set_enabled(0, false);
        assert!(disabled.matcher().is_none(), "disabled");
    }

    /// 64 is the width of the bitset; the 65th pattern switches the feature off
    /// rather than wrapping a shift.
    #[test]
    fn no_matcher_past_sixty_four_patterns() {
        let mut set = ActiveFilters::new();
        // The built-in definitions set holds one of the 64 slots per kind.
        let room = 64 - Kind::ALL.len();
        for i in 0..room {
            set.add(&format!("p{i}")).expect("valid pattern");
        }
        assert!(set.matcher().is_some());

        set.add(&format!("p{room}")).expect("valid pattern");
        assert!(set.matcher().is_none());
    }

    #[test]
    fn the_pattern_key_lists_every_pattern_in_compiled_order() {
        let mut set = set_with(&["alpha", "beta"]);
        let mut expected = vec!["alpha".to_string(), "beta".to_string()];
        expected.extend(Kind::ALL.iter().map(|kind| format!("\u{1}{kind}")));

        assert_eq!(set.pattern_key(), expected, "typed, then the built-in set");

        set.toggle_context(0);
        assert_eq!(set.pattern_key(), expected, "sense is not part of the key");
    }

    /// `is_scanning` answers `matcher().is_some()` without building one
    /// (#170), in every state that makes `matcher` return `None`.
    #[test]
    fn is_scanning_agrees_with_the_matcher() {
        let agree = |set: &ActiveFilters| {
            assert_eq!(set.is_scanning(), set.matcher().is_some());
            set.is_scanning()
        };
        let mut set = ActiveFilters::new();
        assert!(!agree(&set), "no filters");
        set.add_definition(Kind::Function);
        assert!(!agree(&set), "a definition alone");
        set.add("foo").expect("valid");
        assert!(agree(&set), "an include");
        set.filters[1].sense = Sense::Exclude;
        assert!(!agree(&set), "exclude-only");
        set.filters[1].sense = Sense::Include;
        set.set_enabled(1, false);
        assert!(!agree(&set), "the include disabled");
        set.set_enabled(1, true);
        for n in 0..MAX_PATTERNS {
            set.add(&format!("p{n}")).expect("valid");
        }
        assert!(!agree(&set), "over the bitset width");
    }
}
