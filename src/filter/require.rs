//! Require rows (#548): a require filter made from another filter, whose
//! pattern is a reference to that filter and whose extra text is a field
//! of its own. The filter pane's Require section lists them.

use super::reference::spell;
use super::{ActiveFilters, Filter, PatternError, Predicate, Sense};
use std::fmt;

/// Why "require from this" made no row. The base filter is unchanged in
/// every case.
#[derive(Debug)]
pub enum RequireError {
    /// A reference needs a name, and the base must not change, so recon
    /// does not give it one.
    NoName,
    /// A name that a reference cannot spell: `}` ends one, and `:` would
    /// read as a set name.
    Unspellable(String),
    /// A built-in filter is a definition, with no pattern to refer to.
    NoPattern,
    /// The base has references of its own: one level only (ADR 0008).
    HasReferences(String),
    /// The extra text does not compile.
    Invalid(PatternError),
}

impl fmt::Display for RequireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoName => {
                f.write_str("name the filter first: a require row refers to it by name")
            }
            Self::Unspellable(name) => write!(f, "a reference cannot name {name:?}"),
            Self::NoPattern => f.write_str("this filter has no pattern to require"),
            Self::HasReferences(name) => {
                write!(
                    f,
                    "{name} has references; require from the filter it refers to"
                )
            }
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

impl From<PatternError> for RequireError {
    fn from(error: PatternError) -> Self {
        Self::Invalid(error)
    }
}

impl ActiveFilters {
    /// "Require from this" (#548): add a require row to the scratch set
    /// whose base is a reference to the filter at `from`, of any sense, and
    /// whose extra text is `also`. Returns the new row's index. The base is
    /// not changed: a reference takes only its pattern (ADR 0008).
    ///
    /// The row goes after the scratch set's other require filters, so the
    /// list is in the order the pane's Require section shows.
    pub fn add_require(&mut self, from: usize, also: &str) -> Result<usize, RequireError> {
        let base = self.filters.get(from).ok_or(RequireError::NoPattern)?;
        if base.predicate.is_definition() || !self.is_user_authored(from) {
            return Err(RequireError::NoPattern);
        }
        let name = base.name.clone().ok_or(RequireError::NoName)?;
        if base.predicate.has_references() {
            return Err(RequireError::HasReferences(name));
        }
        let reference = spell(&self.sets[0].name, &self.sets[base.set].name, &name)
            .ok_or_else(|| RequireError::Unspellable(name.clone()))?;
        let predicate = Predicate::parse_also(&reference, Some(also))?;
        let at = self
            .filters
            .iter()
            .position(|filter| filter.set != 0 || filter.sense != Sense::Require)
            .unwrap_or(self.filters.len());
        let style = self.next_style();
        self.insert_scratch_at(at, Filter::scratch(predicate, Sense::Require, style));
        Ok(at)
    }

    /// The scratch set's require filters, by index: the rows of the pane's
    /// Require section.
    pub fn require_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.filters_in(0)
            .filter(|(_, filter)| filter.sense == Sense::Require)
            .map(|(index, _)| index)
    }

    /// `Enter` on the Require head: switch every require row off when one
    /// is on, else every one on. Reports whether any changed.
    pub fn toggle_require_rows(&mut self) -> bool {
        let rows: Vec<usize> = self.require_rows().collect();
        let on = !rows.iter().any(|&index| self.filters[index].enabled);
        let mut changed = false;
        for index in rows {
            changed |= self.filters[index].enabled != on;
            self.filters[index].enabled = on;
        }
        changed
    }

    /// Give the require row at `index` the extra text `also`, keeping its
    /// base, and report whether it changed. A text that does not compile
    /// leaves the row as it was. A row answers to its base and its text, so
    /// the profiles follow the new name, as `set_pattern` says.
    pub fn set_also(&mut self, index: usize, also: &str) -> Result<bool, PatternError> {
        let Some(typed) = self.filters.get(index).and_then(|f| f.predicate.typed()) else {
            return Ok(false);
        };
        let predicate = Predicate::parse_also(typed, Some(also))?;
        let changed = self.change(index, |filter| filter.predicate = predicate);
        if changed {
            self.recompile();
        }
        Ok(changed)
    }
}

/// What a filter with no `name` answers to (#548): its pattern, and a
/// require row's extra text after ` & `, so two rows on one base differ.
/// `filtersets::parse` names a file's filter the same way.
#[must_use]
pub fn label(pattern: &str, also: Option<&str>) -> String {
    match also {
        Some(also) if !also.is_empty() => format!("{pattern} & {also}"),
        _ => pattern.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::loaded;
    use super::super::*;

    /// The `DeployLogs` shape: a named context filter for the host line,
    /// listed and enabled.
    fn deploy_logs() -> ActiveFilters {
        let mut set = loaded("DeployLogs", 50, false, &["^Robot name: "]);
        set.filters[0].name = "robot_name".into();
        set.filters[0].sense = Sense::Context;
        let mut filters = ActiveFilters::with_sets(None, &[set]);
        filters.enable_named("DeployLogs", None).expect("known");
        filters
    }

    fn base(filters: &ActiveFilters) -> usize {
        filters
            .filters()
            .iter()
            .position(|filter| filter.name.as_deref() == Some("robot_name"))
            .expect("present")
    }

    #[test]
    fn a_require_row_needs_its_base_and_its_text_on_one_line() {
        let mut filters = deploy_logs();
        let from = base(&filters);
        let row = filters.add_require(from, "Zeus").expect("named");
        let filter = &filters.filters()[row];
        assert_eq!(filter.sense, Sense::Require);
        assert_eq!(filter.set, 0, "in the scratch set");
        assert_eq!(filter.predicate.typed(), Some("${DeployLogs:robot_name}"));
        assert_eq!(filter.predicate.also(), Some("Zeus"));
        filters.set_enabled(base(&filters), false);
        assert_eq!(
            filters.verdict("Robot name: Zeus", KindSet::EMPTY),
            Verdict::Context(row)
        );
        assert_eq!(
            filters.verdict("Robot name: Hera", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert_eq!(
            filters.verdict("Zeus Robot name: ", KindSet::EMPTY),
            Verdict::Unmatched,
            "the base's ^ still anchors"
        );
    }

    #[test]
    fn a_filter_with_no_name_gives_an_error_and_changes_nothing() {
        let mut filters = ActiveFilters::new();
        filters.add("^Host: ").expect("valid");
        let error = filters.add_require(0, "Zeus").expect_err("no name");
        assert!(matches!(error, RequireError::NoName));
        assert_eq!(filters.len(), 1);
        assert_eq!(filters.filters()[0].name, None, "not named for the user");
        assert_eq!(filters.filters()[0].sense, Sense::Include);
    }

    #[test]
    fn an_empty_text_requires_the_base_alone() {
        let mut filters = deploy_logs();
        let row = filters.add_require(base(&filters), "").expect("named");
        filters.set_enabled(base(&filters), false);
        assert_eq!(
            filters.verdict("Robot name: Hera", KindSet::EMPTY),
            Verdict::Context(row)
        );
    }

    #[test]
    fn a_new_text_changes_what_the_row_requires_and_not_its_base() {
        let mut filters = deploy_logs();
        let row = filters.add_require(base(&filters), "Zeus").expect("named");
        assert!(filters.set_also(row, "Hera").expect("valid"));
        filters.set_enabled(base(&filters), false);
        assert_eq!(
            filters.filters()[row].predicate.typed(),
            Some("${DeployLogs:robot_name}")
        );
        assert_eq!(
            filters.verdict("Robot name: Hera", KindSet::EMPTY),
            Verdict::Context(row)
        );
        assert_eq!(
            filters.verdict("Robot name: Zeus", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert!(
            filters.set_also(row, "(").is_err(),
            "an invalid text is refused"
        );
        assert_eq!(filters.filters()[row].predicate.also(), Some("Hera"));
    }

    #[test]
    fn the_text_can_be_one_the_base_also_matches() {
        let mut set = loaded("Git", 50, false, &["[0-9a-f]{7}"]);
        set.filters[0].name = "sha".into();
        let mut filters = ActiveFilters::with_sets(None, &[set]);
        filters.enable_named("Git", None).expect("known");
        let sha = filters
            .filters()
            .iter()
            .position(|f| f.set != 0)
            .expect("loaded");
        let row = filters.add_require(sha, "deadbee").expect("named");
        filters.set_enabled(sha + 1, false);
        assert_eq!(
            filters.verdict("commit deadbee", KindSet::EMPTY),
            Verdict::Context(row)
        );
    }

    #[test]
    fn two_rows_on_one_base_answer_to_different_names() {
        let mut filters = deploy_logs();
        let zeus = filters.add_require(base(&filters), "Zeus").expect("named");
        let hera = filters.add_require(base(&filters), "Hera").expect("named");
        assert_eq!(
            filters.filters()[zeus].display_name(),
            "${DeployLogs:robot_name} & Zeus"
        );
        assert_ne!(
            filters.filters()[zeus].display_name(),
            filters.filters()[hera].display_name()
        );
    }
}
