//! Advanced filters (#474, ADR 0008): a filter the user marked to use
//! Oniguruma, so lookaround can say "has `Summary` and `Updates`, in either
//! order", which the `regex` crate cannot.
//!
//! An advanced filter is a [`Predicate::Advanced`](super::Predicate). Its
//! slot in the compiled `RegexSet` is `NEVER`, as a definition's is, and
//! the verdict asks Oniguruma instead. Every other filter keeps the fast
//! path. Oniguruma backtracks, so each test has a step limit: a line that
//! passes it counts as no match, and the caller is told, so the filter row
//! can say how many lines did.

use regex::Regex;
use std::fmt;
use std::sync::Arc;

/// Oniguruma's own retry limit, and `[filters] advanced_step_limit`'s
/// default. A step count, not a clock, so a line gives the same answer on
/// every machine.
pub const DEFAULT_STEP_LIMIT: u32 = 10_000_000;

/// Which engine reads a filter's pattern. The user chooses it; recon never
/// moves a pattern to the advanced engine because the `regex` crate refused
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    /// The `regex` crate, in the compiled set: fast, and no lookaround.
    #[default]
    Standard,
    /// Oniguruma, one filter at a time, with a step limit.
    Advanced,
}

impl Engine {
    /// What a set file writes after `engine =`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Advanced => "advanced",
        }
    }

    /// The other engine: what the filter editor's key changes to.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Standard => Self::Advanced,
            Self::Advanced => Self::Standard,
        }
    }
}

/// An Oniguruma regex and the source it was compiled from. Cloning one is
/// an `Arc` bump, as cloning a `regex::Regex` is.
#[derive(Clone)]
pub struct Lookaround {
    regex: Arc<onig::Regex>,
    source: String,
}

impl fmt::Debug for Lookaround {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Lookaround").field(&self.source).finish()
    }
}

impl Lookaround {
    /// Compile `source` with Perl syntax and named groups (`perl_ng`): of
    /// Oniguruma's syntaxes, the nearest to the `regex` crate's, so a
    /// referenced pattern means the same in most cases.
    pub fn new(source: &str) -> Result<Self, onig::Error> {
        let regex = onig::Regex::with_options(
            source,
            onig::RegexOptions::REGEX_OPTION_NONE,
            onig::Syntax::perl_ng(),
        )?;
        Ok(Self {
            regex: Arc::new(regex),
            source: source.to_string(),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Where the first match in `line` at or after byte `from` is, or
    /// `Err(OverLimit)` when the search took more than `limit` steps.
    pub fn find_at(
        &self,
        line: &str,
        from: usize,
        limit: u32,
    ) -> Result<Option<(usize, usize)>, OverLimit> {
        let mut region = onig::Region::new();
        Ok(self
            .search(line, from, limit, Some(&mut region))?
            .and_then(|_| region.pos(0)))
    }

    /// Whether the pattern matches `line`, or `Err(OverLimit)`. Asks for
    /// no match positions, so Oniguruma fills no region: this runs on
    /// every line.
    pub fn test(&self, line: &str, limit: u32) -> Result<bool, OverLimit> {
        Ok(self.search(line, 0, limit, None)?.is_some())
    }

    /// One search from byte `from`, with `limit` steps for the whole
    /// search. Oniguruma's match limit is for one start position, and a
    /// search tries each one, so it alone does not limit a line (#551).
    fn search(
        &self,
        line: &str,
        from: usize,
        limit: u32,
        region: Option<&mut onig::Region>,
    ) -> Result<Option<usize>, OverLimit> {
        let mut param = onig::MatchParam::default();
        // Still set: Oniguruma's own match limit applies otherwise, and
        // would cap a `limit` above it.
        param.set_retry_limit_in_match(limit);
        // SAFETY: `as_raw` is the live parameter that `param` owns and
        // frees on drop, and this setter only writes a field in it. `onig`
        // has no safe method for it.
        unsafe {
            onig_sys::onig_set_retry_limit_in_search_of_match_param(param.as_raw(), limit.into());
        }
        self.regex
            .search_with_param(
                line,
                from,
                line.len(),
                onig::SearchOptions::SEARCH_OPTION_NONE,
                region,
                param,
            )
            // The retry limit, or the stack limit behind it: either way
            // the pattern did too much work on this line.
            .map_err(|_| OverLimit)
    }
}

/// A line took more steps than the limit allows. It counts as no match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverLimit;

/// Why a typed pattern does not compile, in the engine that read it.
#[derive(Debug)]
pub enum PatternError {
    Standard(regex::Error),
    Advanced(onig::Error),
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Standard(error) => error.fmt(f),
            Self::Advanced(error) => f.write_str(error.description()),
        }
    }
}

impl std::error::Error for PatternError {}

impl PatternError {
    /// The one line of the error worth showing in a single row.
    ///
    /// A `regex` syntax error's text is several lines: the pattern, a caret
    /// under the fault, and `error: <reason>` last. The pattern is shown
    /// already, so the reason is what is left to say. Oniguruma's is one
    /// line.
    #[must_use]
    pub fn reason(&self) -> String {
        let text = self.to_string();
        let line = text
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("invalid pattern")
            .trim();
        line.to_string()
    }
}

impl From<regex::Error> for PatternError {
    fn from(error: regex::Error) -> Self {
        Self::Standard(error)
    }
}

impl From<onig::Error> for PatternError {
    fn from(error: onig::Error) -> Self {
        Self::Advanced(error)
    }
}

/// The regex a line's text is tested with, in whichever engine the filter
/// uses: what the filter editor highlights with and checks marks against.
#[derive(Debug, Clone)]
pub enum LineRegex {
    Standard(Regex),
    Advanced(Lookaround),
}

impl LineRegex {
    /// Compile `pattern` in `engine`: a pattern with no references.
    pub fn new(pattern: &str, engine: Engine) -> Result<Self, PatternError> {
        Ok(match engine {
            Engine::Standard => Self::Standard(Regex::new(pattern)?),
            Engine::Advanced => Self::Advanced(Lookaround::new(pattern)?),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Standard(regex) => regex.as_str(),
            Self::Advanced(lookaround) => lookaround.as_str(),
        }
    }

    /// Whether the regex matches `line`. A line over the step limit does
    /// not, as in the verdict.
    #[must_use]
    pub fn is_match(&self, line: &str, limit: u32) -> bool {
        match self {
            Self::Standard(regex) => regex.is_match(line),
            Self::Advanced(lookaround) => lookaround.test(line, limit).unwrap_or(false),
        }
    }

    /// Every match in `line`, as byte ranges, left to right. An empty match
    /// moves the search on one character, as `regex::Regex::find_iter`
    /// does. A search over the step limit ends the list.
    #[must_use]
    pub fn find_iter(&self, line: &str, limit: u32) -> Vec<(usize, usize)> {
        match self {
            Self::Standard(regex) => regex
                .find_iter(line)
                .map(|m| (m.start(), m.end()))
                .collect(),
            Self::Advanced(lookaround) => {
                let mut found = Vec::new();
                let mut from = 0;
                while from <= line.len() {
                    let Ok(Some((start, end))) = lookaround.find_at(line, from, limit) else {
                        break;
                    };
                    found.push((start, end));
                    from = if end > start {
                        end
                    } else {
                        // One character on, on a character boundary.
                        line[end..]
                            .chars()
                            .next()
                            .map_or(line.len() + 1, |c| end + c.len_utf8())
                    };
                }
                found
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::set_with;
    use super::super::*;

    fn advanced(pattern: &str) -> ActiveFilters {
        let mut set = set_with(&["placeholder"]);
        set.edit(0, pattern, Engine::Advanced, Details::default())
            .expect("valid pattern");
        set
    }

    #[test]
    fn an_advanced_filter_matches_two_words_in_either_order() {
        let set = advanced("^(?=.*Summary)(?=.*Updates)");
        assert_eq!(
            set.verdict("Updates in the Summary", KindSet::EMPTY),
            Verdict::Included(0)
        );
        assert_eq!(
            set.verdict("Summary of Updates", KindSet::EMPTY),
            Verdict::Included(0)
        );
        assert_eq!(
            set.verdict("Summary only", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    /// The issue's example: the org's pattern and the team's word, in
    /// either order, with the org's filter off.
    #[test]
    fn an_advanced_filter_uses_a_reference_in_a_lookahead() {
        let org = super::super::test_support::loaded("org", 50, false, &["SUMMARY"]);
        let mut set = ActiveFilters::with_sets(None, &[org]);
        set.add("placeholder").expect("valid");
        let team = set
            .filters()
            .iter()
            .position(|filter| filter.set == 0)
            .expect("the scratch filter");
        set.edit(
            team,
            "^(?=.*${org:SUMMARY})(?=.*Updates)",
            Engine::Advanced,
            Details::default(),
        )
        .expect("valid pattern");

        assert_eq!(set.filters()[team].predicate.broken(), None);
        for line in ["Updates in SUMMARY", "SUMMARY of Updates"] {
            assert_eq!(
                set.verdict(line, KindSet::EMPTY),
                Verdict::Included(team),
                "{line:?}"
            );
        }
        assert_eq!(
            set.verdict("SUMMARY only", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    /// A syntax error is refused in the engine that reads the pattern:
    /// lookaround is fine in an advanced filter and an error in a standard
    /// one.
    #[test]
    fn each_engine_refuses_its_own_syntax_errors() {
        let mut set = set_with(&["placeholder"]);
        assert!(
            set.edit(0, "(?=x)", Engine::Standard, Details::default())
                .is_err()
        );
        assert!(
            set.edit(0, "(?=x)", Engine::Advanced, Details::default())
                .is_ok()
        );
        assert!(
            set.edit(0, "(?=x", Engine::Advanced, Details::default())
                .is_err()
        );
        assert_eq!(set.filters()[0].predicate.engine(), Engine::Advanced);
        set.set_pattern(0, "(?<=a)b").expect("still advanced");
        assert_eq!(set.verdict("ab", KindSet::EMPTY), Verdict::Included(0));
    }

    /// The explorer's scan tests an advanced filter too, so it selects
    /// and owns a file as any include filter does.
    #[test]
    fn the_explorer_selects_a_file_through_an_advanced_filter() {
        let set = advanced("^(?=.*Summary)(?=.*Updates)");
        let matcher = set.matcher().expect("an advanced include filter selects");
        let bits = matcher.bits("Updates for the Summary");
        assert!(matcher.selects(bits));
        assert_eq!(matcher.owner(bits), Some(0));
        assert!(!matcher.selects(matcher.bits("Summary only")));
        assert_eq!(set.scan_off(), None);
    }

    /// In the scan as in the view, a line over the limit is no match.
    #[test]
    fn the_scan_counts_a_line_over_the_limit_as_no_match() {
        let mut set = advanced(SLOW);
        set.set_step_limit(1_000);
        let matcher = set.matcher().expect("selects");
        assert!(!matcher.selects(matcher.bits(&format!("{}!", "a".repeat(40)))));
        assert!(matcher.selects(matcher.bits("abc")));
    }

    /// A pattern that backtracks without end on a line it cannot match.
    const SLOW: &str = r"^(\w|\w)*$";

    #[test]
    fn a_line_over_the_step_limit_is_no_match_and_is_reported() {
        let mut set = advanced(SLOW);
        set.set_step_limit(1_000);
        let slow = format!("{}!", "a".repeat(40));
        let mut over = Vec::new();
        assert_eq!(
            set.verdict_noting(&slow, KindSet::EMPTY, |index| over.push(index)),
            Verdict::Unmatched
        );
        assert_eq!(over, vec![0], "the filter's index, once");

        over.clear();
        assert_eq!(
            set.verdict_noting("abc", KindSet::EMPTY, |index| over.push(index)),
            Verdict::Included(0),
            "a line inside the limit still matches"
        );
        assert_eq!(over, Vec::<usize>::new());
    }

    /// A pattern with no `^` that starts with `.*` (ADR 0008's shape) reads
    /// to the end of the line from each start position, so a long line
    /// that does not match costs the square of its length (#551).
    const UNANCHORED: &str = "(?=.*a)(?=.*bc)";

    /// PR #550's line was 129,648 characters long. It has no `bc`, so it
    /// does not match, and a `b` at every other character, so each start
    /// position backtracks at each `b` after it: steps the limit counts.
    fn long_line() -> String {
        "ab".repeat(65_000)
    }

    /// `f`'s answer, after a check that it took less than a second.
    fn within_a_second<T>(f: impl FnOnce() -> T) -> T {
        let started = std::time::Instant::now();
        let answer = f();
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
        answer
    }

    /// The limit is for the whole line, not for each start position: the
    /// search stops, and the line is no match and is reported.
    #[test]
    fn a_long_line_is_over_the_limit_for_the_whole_search() {
        let set = advanced(UNANCHORED);
        let line = long_line();
        let mut over = Vec::new();
        assert_eq!(
            within_a_second(|| {
                set.verdict_noting(&line, KindSet::EMPTY, |index| over.push(index))
            }),
            Verdict::Unmatched
        );
        assert_eq!(over, vec![0]);
    }

    /// The scan reads the same long line as fast, as no match.
    #[test]
    fn the_scan_stops_on_a_long_line_over_the_limit() {
        let set = advanced(UNANCHORED);
        let matcher = set.matcher().expect("selects");
        let line = long_line();
        assert!(!within_a_second(|| matcher.selects(matcher.bits(&line))));
    }

    /// With `^`, there is one start position, so the same long line is
    /// inside the limit and matches as before. The pattern asks for `b`,
    /// not `bc`, so that the line matches.
    #[test]
    fn an_anchored_pattern_on_a_long_line_is_inside_the_limit() {
        let set = advanced("^(?=.*a)(?=.*b)");
        let mut over = Vec::new();
        assert_eq!(
            set.verdict_noting(&long_line(), KindSet::EMPTY, |index| over.push(index)),
            Verdict::Included(0)
        );
        assert_eq!(over, Vec::<usize>::new());
    }

    /// The limit is a step count, so a higher one lets the same line
    /// finish: here, as an honest no match, with nothing reported.
    #[test]
    fn a_higher_step_limit_lets_the_line_finish() {
        let mut set = advanced(SLOW);
        let line = format!("{}!", "a".repeat(12));
        let over_at = |set: &ActiveFilters| {
            let mut over = 0;
            set.verdict_noting(&line, KindSet::EMPTY, |_| over += 1);
            over
        };
        set.set_step_limit(10);
        assert_eq!(over_at(&set), 1);
        set.set_step_limit(DEFAULT_STEP_LIMIT);
        assert_eq!(over_at(&set), 0);
    }
}
