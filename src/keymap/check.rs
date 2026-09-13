//! Does the keymap the user asked for make sense?
//!
//! `config.toml` is a layer over the defaults: the built-in table is layer 0
//! and the file is layer 1, and a later layer wins. So a line the user wrote
//! beats a default the user did not touch. Two lines in one file are in the
//! same layer and nothing orders them, so they cannot resolve and recon
//! refuses the file.
//!
//! Two verdicts come out of that one rule. A key the user **wrote** that can
//! never be reached is an **error**: recon would do something other than what
//! the file says. A **default** the user did not write that goes dark is a
//! **warning**: recon does what the file says, and something else pays.
//!
//! This module is pure. It reads a built `Keymap` and the list of actions the
//! file named, and it returns findings. It opens no file, touches no terminal
//! and logs nothing, which is what makes every rule directly testable.

// Every item here is `pub(crate)` and nothing outside this module's own tests
// calls the checker until `Config::build_keymap` does. rustc's dead-code
// analysis has no root to reach them from, so the lib target reports all of
// them — and `Problem::Unreachable` is unconstructed even by this module's
// tests until the cross-scope rules arrive. Removed once the checker is wired
// in; leaving it would hide a checker that had stopped being called.
#![allow(dead_code)]

use std::fmt;

use super::{ActionId, Keymap, Scope};

/// Two different actions that claim one key.
///
/// An action that merely replaces its own keys is never a `Problem`: that is
/// the ordinary operation a `[keymap]` line performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Problem {
    /// Two written lines claim one key in one scope. An error: they are one
    /// layer, so nothing chooses between them.
    Ambiguous {
        scope: Scope,
        key: String,
        first: ActionId,
        second: ActionId,
    },
    /// A written pane line sits under a global binding. An error: the global
    /// scope is read first, so the line could never be reached.
    Unreachable {
        scope: Scope,
        key: String,
        written: ActionId,
        global: ActionId,
    },
    /// A written line took a key that a default held. A warning: `winner` is
    /// what the user asked for, and `loser` is what it cost. `remaining` is
    /// what `loser` still answers to, which is empty when it now has no key.
    Displaced {
        scope: Scope,
        key: String,
        winner: ActionId,
        loser: ActionId,
        remaining: Vec<String>,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ambiguous {
                scope,
                key,
                first,
                second,
            } => write!(
                f,
                "'{}' and '{}' both bind '{key}' in the {} scope. \
                 Nothing chooses between them: give one of them another key.",
                first.name(),
                second.name(),
                scope.name(),
            ),
            Self::Unreachable {
                scope,
                key,
                written,
                global,
            } => write!(
                f,
                "'{}' binds '{key}' in the {} scope, but '{}' binds '{key}' in the \
                 global scope, which recon reads first. '{}' would never get the key. \
                 Give '{}' another key, or take the key away with '{}' = [].",
                written.name(),
                scope.name(),
                global.name(),
                written.name(),
                global.name(),
                global.name(),
            ),
            Self::Displaced {
                scope,
                key,
                winner,
                loser,
                remaining,
            } => {
                write!(
                    f,
                    "'{}' takes '{key}' from '{}' in the {} scope. ",
                    winner.name(),
                    loser.name(),
                    scope.name(),
                )?;
                if remaining.is_empty() {
                    write!(
                        f,
                        "'{}' now has no key. To choose for yourself, give '{}' \
                         another key, or [] for none.",
                        loser.name(),
                        loser.name(),
                    )
                } else {
                    write!(
                        f,
                        "'{}' now has: {}. To choose for yourself, give '{}' \
                         another key, or [] for none.",
                        loser.name(),
                        remaining.join(", "),
                        loser.name(),
                    )
                }
            }
        }
    }
}

/// Everything `check` found, and what the build must do about it.
///
/// `evict` is the working half. A row named here loses its key, and removing
/// it is what makes a written line actually win — without it the built-in
/// table's row order decides, because `Keymap::resolve` takes the first
/// matching row and `Keymap::rebind` leaves every action at its original
/// position.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
    pub evict: Vec<(Scope, String, ActionId)>,
}

impl Report {
    /// Nothing to say: the keymap is consistent.
    pub(crate) fn is_silent(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty()
    }
}

/// One key in one scope, and every action that claims it, in table order.
struct Claim<'a> {
    scope: Scope,
    key: &'a str,
    actions: Vec<ActionId>,
}

/// Group the built table's rows by scope and key, keeping table order.
///
/// A group of one is the ordinary case and the overwhelming majority. A group
/// of two or more is a contest, and every rule in this module is about one.
fn claims(built: &Keymap) -> Vec<Claim<'_>> {
    let mut groups: Vec<Claim<'_>> = Vec::new();
    for (scope, label, action) in &built.entries {
        match groups
            .iter_mut()
            .find(|claim| claim.scope == *scope && claim.key == label.as_str())
        {
            Some(claim) => {
                // An action bound to one key two times in one scope cannot
                // happen through `rebind`, but de-duplicating costs one line
                // and keeps a group's length meaning "how many actions".
                if !claim.actions.contains(action) {
                    claim.actions.push(*action);
                }
            }
            None => groups.push(Claim {
                scope: *scope,
                key: label.as_str(),
                actions: vec![*action],
            }),
        }
    }
    groups
}

/// Check the keymap the user asked for.
///
/// `written` is every action the `[keymap]` table named. It is the only thing
/// that tells a written claim from a default one, which is why the defaults
/// are not an argument: an action is in this list or it is not.
pub(crate) fn check(built: &Keymap, written: &[ActionId]) -> Report {
    let mut report = Report::default();
    let claims = claims(built);

    // Pass one: two actions claiming one key inside a single scope.
    for claim in &claims {
        if claim.actions.len() < 2 {
            continue;
        }
        let mine: Vec<ActionId> = claim
            .actions
            .iter()
            .copied()
            .filter(|action| written.contains(action))
            .collect();

        if let [first, second, ..] = mine.as_slice() {
            report.errors.push(Problem::Ambiguous {
                scope: claim.scope,
                key: claim.key.to_string(),
                first: *first,
                second: *second,
            });
            continue;
        }

        // Exactly one written claim: it wins, and every default claim loses.
        // No written claim at all cannot occur — no scope in `DEFAULT` holds
        // one key two times, and `the_defaults_hold_no_duplicate_key` proves
        // it — so the loop body simply does not run.
        if let [winner] = mine.as_slice() {
            for loser in claim.actions.iter().copied().filter(|a| a != winner) {
                report
                    .evict
                    .push((claim.scope, claim.key.to_string(), loser));
                report.warnings.push(Problem::Displaced {
                    scope: claim.scope,
                    key: claim.key.to_string(),
                    winner: *winner,
                    loser,
                    remaining: Vec::new(), // filled below, once every eviction is known
                });
            }
        }
    }

    // Pass two: the global scope against each pane.
    //
    // Only these four scopes can cross. `Prompt` and `Picker` take every key
    // while they are open, so a key bound in one of them and also globally is
    // the design and not a contest. `Help` binds nothing at all.
    let winner_in = |scope: Scope, key: &str| -> Option<ActionId> {
        let claim = claims
            .iter()
            .find(|claim| claim.scope == scope && claim.key == key)?;
        // The same rule pass one applied: a written claim wins, and otherwise
        // the single default claim does.
        claim
            .actions
            .iter()
            .copied()
            .find(|action| written.contains(action))
            .or_else(|| claim.actions.first().copied())
    };

    for claim in &claims {
        if !matches!(claim.scope, Scope::Nav | Scope::View | Scope::Filters) {
            continue;
        }
        let Some(global) = winner_in(Scope::Global, claim.key) else {
            continue;
        };
        let Some(pane) = winner_in(claim.scope, claim.key) else {
            continue;
        };

        if written.contains(&pane) {
            // The user wrote a pane line that the global scope will always
            // answer first. recon would ignore what the file says.
            report.errors.push(Problem::Unreachable {
                scope: claim.scope,
                key: claim.key.to_string(),
                written: pane,
                global,
            });
        } else if written.contains(&global) {
            // The user wrote the global line and got what they asked for.
            // The pane default is what it cost.
            report
                .evict
                .push((claim.scope, claim.key.to_string(), pane));
            report.warnings.push(Problem::Displaced {
                scope: claim.scope,
                key: claim.key.to_string(),
                winner: global,
                loser: pane,
                remaining: Vec::new(),
            });
        }
        // Neither written: two defaults crossing, which `DEFAULT` does not do
        // and `no_default_key_is_in_both_global_and_a_pane` pins.
    }

    fill_remaining(built, &mut report);
    report
}

/// Say what each displaced action still answers to, once every eviction is
/// known.
///
/// Done in a second pass rather than inline: an action can lose two keys in
/// one file, and a message naming what is left must count both losses.
fn fill_remaining(built: &Keymap, report: &mut Report) {
    let evicted = report.evict.clone();
    for problem in &mut report.warnings {
        let Problem::Displaced {
            loser, remaining, ..
        } = problem
        else {
            continue;
        };
        *remaining = built
            .labels_for(*loser)
            .into_iter()
            .filter(|label| {
                !evicted
                    .iter()
                    .any(|(_, key, action)| action == loser && key == label)
            })
            .map(str::to_string)
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{ActionId, Keymap, Scope};

    /// One `[keymap]` line, as `Keymap::new` takes it.
    fn overlay(pairs: &[(&str, &[&str])]) -> crate::config::KeymapConfig {
        let mut bindings = std::collections::BTreeMap::new();
        for (action, keys) in pairs {
            bindings.insert(
                (*action).to_string(),
                keys.iter().map(|key| (*key).to_string()).collect(),
            );
        }
        crate::config::KeymapConfig { bindings }
    }

    /// The actions a `[keymap]` table names, resolved to ids — what `check`
    /// takes as `written`.
    fn written(pairs: &[(&str, &[&str])]) -> Vec<ActionId> {
        pairs
            .iter()
            .map(|(action, _)| crate::keymap::action_named(action).expect("a real action"))
            .collect()
    }

    fn report(pairs: &[(&str, &[&str])]) -> Report {
        let built = Keymap::new(&overlay(pairs)).expect("valid");
        check(&built, &written(pairs))
    }

    #[test]
    fn the_bare_defaults_say_nothing() {
        let report = check(&Keymap::default(), &[]);
        assert_eq!(report, Report::default(), "a clean config must stay silent");
    }

    #[test]
    fn replacing_an_actions_own_keys_is_not_a_collision() {
        // Pete's case: '=' is free and 'q' is global.quit's own default, so
        // adding '=' beside 'q' takes nothing from anybody.
        let report = report(&[("global.quit", &["q", "="])]);
        assert_eq!(report, Report::default());
    }

    #[test]
    fn unbinding_an_action_is_not_a_warning() {
        let report = report(&[("global.quit", &[])]);
        assert_eq!(report, Report::default(), "an empty list is deliberate");
    }

    #[test]
    fn two_written_lines_on_one_key_in_one_scope_are_an_error() {
        let report = report(&[("global.quit", &["="]), ("global.reload", &["="])]);

        assert_eq!(report.warnings, vec![]);
        assert_eq!(report.evict, vec![]);
        assert_eq!(
            report.errors,
            vec![Problem::Ambiguous {
                scope: Scope::Global,
                key: "=".to_string(),
                first: ActionId::GlobalQuit,
                second: ActionId::GlobalReload,
            }],
            "one layer gives no order, so recon must refuse"
        );
    }

    #[test]
    fn a_written_line_takes_a_key_from_another_actions_default() {
        // 'o' is global.editor.project's only default key. global.quit asks
        // for it, so the editor binding is left with nothing at all.
        let report = report(&[("global.quit", &["o"])]);

        assert_eq!(report.errors, vec![]);
        assert_eq!(
            report.warnings,
            vec![Problem::Displaced {
                scope: Scope::Global,
                key: "o".to_string(),
                winner: ActionId::GlobalQuit,
                loser: ActionId::GlobalEditorProject,
                remaining: vec![],
            }],
            "global.editor.project has only 'o', so it is left with no key at all"
        );
        assert_eq!(
            report.evict,
            vec![(
                Scope::Global,
                "o".to_string(),
                ActionId::GlobalEditorProject
            )],
            "the default's row must go, or the written line does not win"
        );
    }

    #[test]
    fn a_global_line_over_pane_defaults_warns_once_for_each_pane() {
        // 'j' is nav.down, view.down and filters.down by default.
        let report = report(&[("global.quit", &["j"])]);

        assert_eq!(report.errors, vec![]);
        assert_eq!(
            report.warnings,
            vec![
                Problem::Displaced {
                    scope: Scope::Nav,
                    key: "j".to_string(),
                    winner: ActionId::GlobalQuit,
                    loser: ActionId::NavDown,
                    remaining: vec!["Down".to_string()],
                },
                Problem::Displaced {
                    scope: Scope::View,
                    key: "j".to_string(),
                    winner: ActionId::GlobalQuit,
                    loser: ActionId::ViewDown,
                    remaining: vec!["Down".to_string()],
                },
                Problem::Displaced {
                    scope: Scope::Filters,
                    key: "j".to_string(),
                    winner: ActionId::GlobalQuit,
                    loser: ActionId::FiltersDown,
                    remaining: vec!["Down".to_string()],
                },
            ],
            "each pane loses 'j' and keeps 'Down'"
        );
        assert_eq!(report.evict.len(), 3);
    }

    #[test]
    fn a_pane_line_under_a_default_global_binding_is_an_error() {
        // 'q' is global.quit's default. nav.up can never get it.
        let report = report(&[("nav.up", &["q"])]);

        assert_eq!(report.warnings, vec![]);
        assert_eq!(report.evict, vec![]);
        assert_eq!(
            report.errors,
            vec![Problem::Unreachable {
                scope: Scope::Nav,
                key: "q".to_string(),
                written: ActionId::NavUp,
                global: ActionId::GlobalQuit,
            }]
        );
    }

    #[test]
    fn a_pane_line_under_a_written_global_binding_is_also_an_error() {
        // '=' is bound nowhere by default, so the only two claims on it are
        // the two this config writes.
        let report = report(&[("global.reload", &["="]), ("nav.up", &["="])]);

        assert_eq!(
            report.errors,
            vec![Problem::Unreachable {
                scope: Scope::Nav,
                key: "=".to_string(),
                written: ActionId::NavUp,
                global: ActionId::GlobalReload,
            }],
            "who holds the global key does not change the verdict"
        );
    }

    #[test]
    fn a_modal_scope_never_crosses_with_global() {
        // 'Ctrl-a' is prompt.start, and the prompt scope is the only scope
        // that binds it. A global line claiming it is not a collision: while
        // a prompt is open it owns the whole keyboard, so the two never meet.
        // (`Enter` would be the wrong key to test with — it is bound in Nav
        // and Filters as well, so it genuinely does cross.)
        let report = report(&[("global.reload", &["Ctrl-a"])]);
        assert_eq!(report.errors, vec![], "{report:?}");
        assert_eq!(report.warnings, vec![], "{report:?}");
    }

    #[test]
    fn a_pasted_effective_dump_says_nothing() {
        // Every action written explicitly, each with the keys it already has.
        // This is what `--print-keymap` emits, so pasting it back must be a
        // silent no-op or the flag's whole purpose fails.
        let defaults = Keymap::default();
        let pairs: Vec<(String, Vec<String>)> = crate::keymap::every_action()
            .map(|action| {
                (
                    action.name().to_string(),
                    defaults
                        .labels_for(action)
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                )
            })
            .collect();
        let mut bindings = std::collections::BTreeMap::new();
        for (name, keys) in &pairs {
            bindings.insert(name.clone(), keys.clone());
        }
        let overlay = crate::config::KeymapConfig { bindings };
        let built = Keymap::new(&overlay).expect("valid");
        let written: Vec<ActionId> = crate::keymap::every_action().collect();

        let report = check(&built, &written);
        assert!(report.is_silent(), "{report:?}");
        assert_eq!(report.evict, vec![]);
    }
}
