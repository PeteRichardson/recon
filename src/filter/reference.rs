//! Pattern references (#473, ADR 0008): `${set:name}`, or `${name}` for a
//! filter in the same set, stands for the current pattern of that filter.
//!
//! A pattern with a reference is a [`Predicate::Referring`]. It keeps the
//! pattern as typed — what the pane shows and what a set file saves — and
//! the expansion, which `recompile` makes again on every pattern change, so
//! a reference always means the target's pattern *now*. A broken reference
//! compiles to `NEVER`: its filter matches nothing, and no index in the
//! compiled set moves.

use super::{
    ActiveFilters, Engine, Filter, LineRegex, Lookaround, PatternError, Predicate, joined,
};
use regex::Regex;
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::LazyLock;

/// [`NEVER`](super::NEVER), compiled once: what a broken reference's line
/// regex is.
static NOTHING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(super::NEVER).expect("NEVER compiles"));

/// Why a target cannot stand in for a reference, given the reference as
/// written.
type Reason = fn(String) -> Broken;

/// One `${…}` in a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reference {
    /// The set before the `:`, or `None` for the referring filter's own set.
    set: Option<String>,
    name: String,
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.set {
            Some(set) => write!(f, "{set}:{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// Why a reference cannot be expanded. Its filter matches nothing, and the
/// pane gives this as the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Broken {
    /// No set of that name, or no filter of that `name` in it.
    NotFound(String),
    /// The target is a definition filter, which has no pattern.
    Definition(String),
    /// The target has references of its own. One level only, for now.
    HasReferences(String),
    /// The expansion would not compile.
    Invalid(String),
    /// Not expanded yet: what a pattern is between `Predicate::parse` and
    /// the next `recompile`.
    Unexpanded,
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(target) => write!(f, "{target} not found"),
            Self::Definition(target) => write!(f, "{target} is a definition filter"),
            Self::HasReferences(target) => {
                write!(f, "{target} points to a filter that has references")
            }
            Self::Invalid(error) => f.write_str(error),
            Self::Unexpanded => f.write_str("not expanded"),
        }
    }
}

/// Every reference in `pattern`, with its byte range. A `\` escapes the
/// character after it, so `\$\{` is a literal.
fn references(pattern: &str) -> Vec<(Range<usize>, Reference)> {
    let mut found = Vec::new();
    let mut at = 0;
    let bytes = pattern.as_bytes();
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            at += 2;
            continue;
        }
        // Compared as bytes: `$`, `{` and `\` are ASCII, so they never sit
        // inside a wider character, and `at` may.
        if bytes[at] == b'$'
            && bytes.get(at + 1) == Some(&b'{')
            && let Some(length) = pattern[at + 2..].find('}')
            && length > 0
        {
            let inner = &pattern[at + 2..at + 2 + length];
            let reference = match inner.split_once(':') {
                Some((set, name)) => Reference {
                    set: Some(set.to_string()),
                    name: name.to_string(),
                },
                None => Reference {
                    set: None,
                    name: inner.to_string(),
                },
            };
            let end = at + 2 + length + 1;
            found.push((at..end, reference));
            at = end;
            continue;
        }
        at += 1;
    }
    found
}

/// `pattern` with each reference as an empty group: what a syntax check
/// of the rest of the pattern compiles.
fn blanked(pattern: &str, found: &[(Range<usize>, Reference)]) -> String {
    substitute(pattern, found, |_| {
        Ok::<_, std::convert::Infallible>("(?:)".into())
    })
    .unwrap_or_else(|never| match never {})
}

/// `pattern` with each reference replaced by what `with` gives for it.
fn substitute<E>(
    pattern: &str,
    found: &[(Range<usize>, Reference)],
    mut with: impl FnMut(&Reference) -> Result<String, E>,
) -> Result<String, E> {
    let mut out = String::with_capacity(pattern.len());
    let mut from = 0;
    for (range, reference) in found {
        out.push_str(&pattern[from..range.start]);
        out.push_str(&with(reference)?);
        from = range.end;
    }
    out.push_str(&pattern[from..]);
    Ok(out)
}

impl Predicate {
    /// The predicate for a typed pattern: a `Regex`, or a `Referring` when
    /// it has a reference. The rest of a referring pattern is checked with
    /// each reference as an empty group, so a syntax error is refused here,
    /// as for any pattern; a reference that cannot be expanded is not.
    pub fn parse(pattern: &str) -> Result<Self, regex::Error> {
        let found = references(pattern);
        if found.is_empty() {
            return Regex::new(pattern).map(Self::Regex);
        }
        Regex::new(&blanked(pattern, &found))?;
        Ok(Self::Referring {
            typed: pattern.to_string(),
            expanded: Err(Broken::Unexpanded),
        })
    }

    /// The predicate for a typed pattern read by `engine`: `parse` for the
    /// standard engine, and an `Advanced` for the other (#474). As there,
    /// the rest of a pattern with references is checked with each one as
    /// an empty group, and the references wait for the next `recompile`.
    pub fn parse_as(pattern: &str, engine: Engine) -> Result<Self, PatternError> {
        if engine == Engine::Standard {
            return Ok(Self::parse(pattern)?);
        }
        Self::parse_also(pattern, None)
    }

    /// The advanced predicate for `pattern` and a require row's extra text
    /// `also` (#548), or `pattern` alone for `None`. Checked as `parse_as`
    /// checks an advanced pattern, both parts at once.
    pub fn parse_also(pattern: &str, also: Option<&str>) -> Result<Self, PatternError> {
        let source = joined(pattern, also);
        let found = references(&source);
        let compiled = if found.is_empty() {
            Ok(Lookaround::new(&source)?)
        } else {
            Lookaround::new(&blanked(&source, &found))?;
            Err(Broken::Unexpanded)
        };
        Ok(Self::Advanced {
            typed: pattern.to_string(),
            also: also.map(str::to_string),
            compiled,
        })
    }

    /// A require row's extra text (#548): `Some` only for an advanced
    /// predicate made with one, and then possibly empty.
    #[must_use]
    pub fn also(&self) -> Option<&str> {
        match self {
            Self::Advanced { also, .. } => also.as_deref(),
            _ => None,
        }
    }

    /// Whether the pattern as typed has a reference in it.
    #[must_use]
    pub fn has_references(&self) -> bool {
        self.typed()
            .is_some_and(|typed| !references(&joined(typed, self.also())).is_empty())
    }

    /// The pattern with its references expanded, or why one cannot be:
    /// `None` for a pattern with no reference. The filter editor shows it.
    #[must_use]
    pub fn expansion(&self) -> Option<Result<&str, &Broken>> {
        match self {
            Self::Referring { expanded, .. } => Some(expanded.as_ref().map(Regex::as_str)),
            Self::Advanced { compiled, .. } if self.has_references() => {
                Some(compiled.as_ref().map(Lookaround::as_str))
            }
            _ => None,
        }
    }

    /// Why this predicate matches nothing: a reference that cannot be
    /// expanded. `None` for every other predicate.
    #[must_use]
    pub fn broken(&self) -> Option<&Broken> {
        match self {
            Self::Referring {
                expanded: Err(broken),
                ..
            }
            | Self::Advanced {
                compiled: Err(broken),
                ..
            } => Some(broken),
            _ => None,
        }
    }

    /// The pattern as the user wrote it, references and all. `None` for a
    /// definition, which has no pattern.
    #[must_use]
    pub fn typed(&self) -> Option<&str> {
        match self {
            Self::Regex(regex) => Some(regex.as_str()),
            Self::Referring { typed, .. } | Self::Advanced { typed, .. } => Some(typed),
            Self::Definition(_) => None,
        }
    }

    /// The regex a line's text is tested with: for a broken reference, one
    /// that matches nothing, as its filter does. `None` for a definition.
    #[must_use]
    pub fn line_regex(&self) -> Option<LineRegex> {
        match self {
            Self::Definition(_) => None,
            Self::Advanced { .. } => Some(self.as_lookaround().map_or_else(
                || LineRegex::Standard(NOTHING.clone()),
                |lookaround| LineRegex::Advanced(lookaround.clone()),
            )),
            _ => Some(LineRegex::Standard(
                self.as_regex().cloned().unwrap_or_else(|| NOTHING.clone()),
            )),
        }
    }

    /// A referring predicate's expansion, made again from `expander` for a
    /// filter of set `own`. Reports whether it changed; compiles only then.
    fn expand(&mut self, expander: &Expander, own: &str) -> bool {
        match self {
            Self::Referring { typed, expanded } => reexpand(
                expanded,
                expander.expand(own, typed),
                Regex::as_str,
                |source| Ok(Regex::new(source)?),
            ),
            Self::Advanced {
                typed,
                also,
                compiled,
            } => {
                let source = joined(typed, also.as_deref());
                !references(&source).is_empty()
                    && reexpand(
                        compiled,
                        expander.expand(own, &source),
                        Lookaround::as_str,
                        |source| Ok(Lookaround::new(source)?),
                    )
            }
            Self::Regex(_) | Self::Definition(_) => false,
        }
    }
}

/// Put `now`, an expansion or why there is none, in `slot`, compiling it
/// with `compile` only when it differs from what `slot` holds. Reports
/// whether it changed. One for each engine's compiled type.
fn reexpand<T>(
    slot: &mut Result<T, Broken>,
    now: Result<String, Broken>,
    as_str: impl Fn(&T) -> &str,
    compile: impl FnOnce(&str) -> Result<T, PatternError>,
) -> bool {
    let same = match (&now, &*slot) {
        (Ok(source), Ok(compiled)) => source == as_str(compiled),
        (Err(now), Err(was)) => now == was,
        _ => false,
    };
    if !same {
        *slot = now.and_then(|source| {
            compile(&source).map_err(|error| Broken::Invalid(error.to_string()))
        });
    }
    !same
}

/// The reference a filter of set `own` writes for the filter `name` of
/// set `set` (#548): `${name}` in its own set, else `${set:name}`. `None`
/// when `references` could not read it back: `}` ends a reference, and a
/// `:` would read as a set name.
pub(super) fn spell(own: &str, set: &str, name: &str) -> Option<String> {
    if name.contains(['}', ':']) || set.contains(['}', ':']) {
        return None;
    }
    Some(if set == own {
        format!("${{{name}}}")
    } else {
        format!("${{{set}:{name}}}")
    })
}

impl Reference {
    /// The name of the set this reference is to, from a filter of set
    /// `own`. `None` for `${:name}`: the scratch set has no name a
    /// reference can give.
    fn set_from<'a>(&'a self, own: &'a str) -> Option<&'a str> {
        match self.set.as_deref() {
            Some("") => None,
            Some(set) => Some(set),
            None => Some(own),
        }
    }
}

impl ActiveFilters {
    /// Expand every referring filter in the list again, reporting whether
    /// any expansion changed. `recompile` calls it first, so the compiled
    /// set always has the targets' patterns as they are now.
    pub(super) fn expand_references(&mut self) -> bool {
        if !self
            .filters
            .iter()
            .any(|filter| filter.predicate.has_references())
        {
            return false;
        }
        let expander = self.expander();
        let mut changed = false;
        for filter in &mut self.filters {
            changed |= filter
                .predicate
                .expand(&expander, &self.sets[filter.set].name);
        }
        changed
    }

    /// Each filter that takes effect and has a broken reference, as
    /// `set:filter` and the reason. What stops a batch run (ADR 0008): a
    /// broken reference in a filter or set that is off decides nothing.
    #[must_use]
    pub fn broken_references(&self) -> Vec<(String, &Broken)> {
        self.filters
            .iter()
            .enumerate()
            .filter(|&(index, _)| self.effective(index))
            .filter_map(|(_, filter)| {
                let broken = filter.predicate.broken()?;
                let label = format!("{}:{}", self.sets[filter.set].name, filter.display_name());
                Some((label, broken))
            })
            .collect()
    }

    /// How many references point into each set, aligned with `sets`, from
    /// every known filter, listed or not. What the set picker shows, so
    /// the user sees what unlisting or renaming a set would touch.
    #[must_use]
    pub fn references_into(&self) -> Vec<usize> {
        let mut counts = vec![0; self.sets.len()];
        for filter in self.known_filters() {
            let Some(typed) = filter.predicate.typed() else {
                continue;
            };
            let own = &self.sets[filter.set].name;
            for (_, reference) in references(typed) {
                if let Some(set) = reference
                    .set_from(own)
                    .and_then(|name| self.sets.iter().position(|meta| meta.name == name))
                {
                    counts[set] += 1;
                }
            }
        }
        counts
    }

    /// Every filter of every set, listed or not: an unlisted set's filters
    /// are not in the list, so they are read as loaded.
    fn known_filters(&self) -> impl Iterator<Item = &Filter> {
        let unlisted = self
            .sets
            .iter()
            .zip(&self.as_loaded)
            .filter(|(meta, _)| !meta.listed)
            .flat_map(|(_, filters)| filters);
        self.filters.iter().chain(unlisted)
    }

    /// What a reference can stand for, read now. The filter editor keeps
    /// one (#473), so its live matches use the expansion. Only a filter
    /// with a `name` of its own is a target.
    #[must_use]
    pub fn expander(&self) -> Expander {
        let mut targets = HashMap::new();
        for filter in self.known_filters() {
            let meta = &self.sets[filter.set];
            let Some(name) = &filter.name else {
                continue;
            };
            if filter.set != 0 && !meta.filter_names.contains(name) {
                continue;
            }
            let pattern = match &filter.predicate {
                Predicate::Regex(regex) => Ok(regex.as_str().to_string()),
                Predicate::Definition(_) => Err(Broken::Definition as Reason),
                Predicate::Referring { .. } => Err(Broken::HasReferences as Reason),
                // An advanced pattern is Oniguruma's: a standard filter
                // that names it gets it as typed, and may not compile.
                Predicate::Advanced { .. } if filter.predicate.has_references() => {
                    Err(Broken::HasReferences as Reason)
                }
                Predicate::Advanced { typed, also, .. } => {
                    Ok(joined(typed, also.as_deref()).into_owned())
                }
            };
            targets.insert((meta.name.clone(), name.clone()), pattern);
        }
        Expander { targets }
    }

    /// The name of the set of the filter at `index`, or of the scratch set
    /// for `None`, a new filter: what a reference with no set is to.
    #[must_use]
    pub fn set_name_of(&self, index: Option<usize>) -> &str {
        let set = index
            .and_then(|index| self.filters.get(index))
            .map_or(0, |filter| filter.set);
        &self.sets[set].name
    }
}

/// Every filter a reference can name, as `ActiveFilters::expander` read
/// them, to expand a pattern without the filters at hand.
#[derive(Debug, Clone, Default)]
pub struct Expander {
    /// By set and name: the target's pattern, or the reason it cannot be
    /// one, given the reference as written.
    targets: HashMap<(String, String), Result<String, Reason>>,
}

impl Expander {
    /// `pattern`, from a filter of set `own`, with each reference replaced
    /// by its target's pattern in `(?:…)`.
    fn expand(&self, own: &str, pattern: &str) -> Result<String, Broken> {
        substitute(pattern, &references(pattern), |reference| {
            let written = reference.to_string();
            let target = reference
                .set_from(own)
                .and_then(|set| self.targets.get(&(set.to_string(), reference.name.clone())));
            match target {
                None => Err(Broken::NotFound(written)),
                Some(Ok(pattern)) => Ok(format!("(?:{pattern})")),
                Some(Err(reason)) => Err(reason(written)),
            }
        })
    }

    /// The predicate for `pattern` typed in a filter of set `own`, with its
    /// references expanded: what the filter would be. The filter editor and
    /// `c` check a pattern with this, in the engine the filter uses.
    pub fn predicate(
        &self,
        own: &str,
        pattern: &str,
        engine: Engine,
    ) -> Result<Predicate, PatternError> {
        let mut predicate = Predicate::parse_as(pattern, engine)?;
        predicate.expand(self, own);
        Ok(predicate)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::loaded;
    use super::super::*;

    /// A file filter the file gave `name`, with `pattern`.
    fn named(name: &str, pattern: &str) -> LoadedFilter {
        LoadedFilter {
            name: name.to_string(),
            ..loaded("", 0, false, &[pattern]).filters.remove(0)
        }
    }

    fn set(name: &str, filters: Vec<LoadedFilter>) -> LoadedSet {
        LoadedSet {
            filters,
            ..loaded(name, 50, false, &[])
        }
    }

    /// The issue's example: `org:Summary` is off, and the team filter
    /// still uses its pattern.
    fn org_and_team(team: &str) -> ActiveFilters {
        let org = set("org", vec![named("Summary", "^SUMMARY")]);
        let team = set("team", vec![named("Update summaries", team)]);
        let mut filters = ActiveFilters::with_sets(None, &[org, team]);
        filters
            .enable_named("team", Some(&["Update summaries".to_string()]))
            .expect("known");
        filters
    }

    fn index_of(filters: &ActiveFilters, name: &str) -> usize {
        filters
            .filters()
            .iter()
            .position(|filter| filter.name.as_deref() == Some(name))
            .expect("present")
    }

    #[test]
    fn a_reference_matches_with_the_target_s_pattern_while_the_target_is_off() {
        let filters = org_and_team("${org:Summary}.*Updates");
        let team = index_of(&filters, "Update summaries");
        assert_eq!(
            filters.verdict("SUMMARY: Updates", KindSet::EMPTY),
            Verdict::Included(team)
        );
        assert_eq!(
            filters.verdict("SUMMARY only", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert_eq!(
            filters.verdict("x SUMMARY: Updates", KindSet::EMPTY),
            Verdict::Unmatched,
            "the target's ^ still anchors"
        );
    }

    #[test]
    fn a_reference_points_into_an_unlisted_set() {
        let mut org = set("org", vec![named("Summary", "^SUMMARY")]);
        org.listed = false;
        let team = set("team", vec![named("u", "${org:Summary}.*Updates")]);
        let mut filters = ActiveFilters::with_sets(None, &[org, team]);
        filters.enable_named("team", None).expect("known");
        filters.set_all_enabled(true);
        assert_eq!(
            filters.verdict("SUMMARY: Updates", KindSet::EMPTY),
            Verdict::Included(index_of(&filters, "u"))
        );
    }

    fn reason(filters: &ActiveFilters, name: &str) -> Option<String> {
        filters.filters()[index_of(filters, name)]
            .predicate
            .broken()
            .map(ToString::to_string)
    }

    #[test]
    fn a_definition_filter_is_not_a_target() {
        let filters = org_and_team("${definitions:functions}");
        assert_eq!(
            reason(&filters, "Update summaries").as_deref(),
            Some("definitions:functions is a definition filter")
        );
    }

    #[test]
    fn a_target_with_references_of_its_own_is_broken_at_one_level() {
        let org = set(
            "org",
            vec![
                named("Summary", "^SUMMARY"),
                named("Long", "${Summary}.{80}"),
            ],
        );
        let team = set(
            "team",
            vec![named("t", "${org:Long}"), named("me", "${me}")],
        );
        let filters = ActiveFilters::with_sets(None, &[org, team]);
        assert_eq!(reason(&filters, "Long"), None, "one level is fine");
        assert_eq!(
            reason(&filters, "t").as_deref(),
            Some("org:Long points to a filter that has references")
        );
        assert_eq!(
            reason(&filters, "me").as_deref(),
            Some("me points to a filter that has references"),
            "a filter that names itself"
        );
    }

    #[test]
    fn a_reference_with_no_set_is_to_the_same_set() {
        let org = set(
            "org",
            vec![
                named("Summary", "^SUMMARY"),
                named("Updates", "${Summary}.*Updates"),
            ],
        );
        let team = set("team", vec![named("Summary", "^TEAM")]);
        let mut filters = ActiveFilters::with_sets(None, &[org, team]);
        filters
            .enable_named("org", Some(&["Updates".to_string()]))
            .expect("known");
        assert_eq!(
            filters.verdict("SUMMARY: Updates", KindSet::EMPTY),
            Verdict::Included(index_of(&filters, "Updates"))
        );
    }

    #[test]
    fn a_filter_with_no_name_is_not_a_target() {
        let mut unnamed = named("^SUMMARY", "^SUMMARY");
        unnamed.named = false;
        let org = set("org", vec![unnamed]);
        let team = set("team", vec![named("t", "${org:^SUMMARY}")]);
        let filters = ActiveFilters::with_sets(None, &[org, team]);
        assert_eq!(
            reason(&filters, "t").as_deref(),
            Some("org:^SUMMARY not found")
        );
    }

    #[test]
    fn an_escaped_dollar_and_brace_is_a_literal() {
        let predicate = Predicate::parse(r"\$\{org:Summary}").expect("valid");
        assert!(matches!(predicate, Predicate::Regex(_)));
        assert!(predicate.holds("${org:Summary}", KindSet::EMPTY));
    }

    #[test]
    fn a_pattern_with_wide_characters_is_read_by_character() {
        assert!(matches!(
            Predicate::parse("日本語"),
            Ok(Predicate::Regex(_))
        ));
        assert!(matches!(
            Predicate::parse("é${語}"),
            Ok(Predicate::Referring { .. })
        ));
        assert!(Predicate::parse(r"\é${語}").is_err(), "not a regex escape");
    }

    #[test]
    fn a_syntax_error_outside_the_reference_is_refused() {
        assert!(Predicate::parse("${org:Summary}(").is_err());
    }

    #[test]
    fn a_reference_follows_a_new_pattern_on_its_target() {
        let mut filters = org_and_team("${org:Summary}.*Updates");
        let summary = index_of(&filters, "Summary");
        filters.set_pattern(summary, "^RECAP").expect("valid");
        let team = index_of(&filters, "Update summaries");
        assert_eq!(
            filters.verdict("RECAP: Updates", KindSet::EMPTY),
            Verdict::Included(team)
        );
        assert_eq!(
            filters.verdict("SUMMARY: Updates", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    #[test]
    fn a_renamed_target_breaks_the_old_name_and_answers_to_the_new_one() {
        let mut filters = org_and_team("${org:Summary}");
        let summary = index_of(&filters, "Summary");
        let recap = Details {
            name: Some("Recap".to_string()),
            ..Details::default()
        };
        filters.set_details(summary, recap);
        assert_eq!(
            reason(&filters, "Update summaries").as_deref(),
            Some("org:Summary not found")
        );
        let team = index_of(&filters, "Update summaries");
        filters.set_pattern(team, "${org:Recap}").expect("valid");
        assert_eq!(reason(&filters, "Update summaries"), None);
    }

    #[test]
    fn a_missing_target_matches_nothing_and_says_why() {
        let filters = org_and_team("${org:Nope}|Updates");
        let team = index_of(&filters, "Update summaries");
        assert_eq!(
            filters.verdict("Updates", KindSet::EMPTY),
            Verdict::Unmatched,
            "only the broken filter matches nothing, the whole of it"
        );
        assert_eq!(
            filters.filters()[team]
                .predicate
                .broken()
                .map(ToString::to_string),
            Some("org:Nope not found".to_string())
        );
        let missing_set = org_and_team("${nowhere:Summary}");
        let team = index_of(&missing_set, "Update summaries");
        assert_eq!(
            missing_set.filters()[team]
                .predicate
                .broken()
                .map(ToString::to_string),
            Some("nowhere:Summary not found".to_string())
        );
    }
}
