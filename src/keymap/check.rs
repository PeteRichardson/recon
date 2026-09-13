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

use std::fmt;

use super::{ActionId, Keymap, Scope};
use crate::help::Chord;

/// Two different actions that claim one key.
///
/// An action that merely replaces its own keys is never a `Problem`: that is
/// the ordinary operation a `[keymap]` line performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Problem {
    /// Two or more written lines claim one key in one scope. An error: they
    /// are one layer, so nothing chooses between them.
    ///
    /// Every claimant, not the first two. "Every fault is reported together,
    /// so one run tells you everything to correct" is what the README
    /// promises, and reporting every error at once rather than one at a time
    /// is the answer the project owner gave when asked directly. An error
    /// naming two of three claimants would send a user back to the file a
    /// second time for a fault recon had already seen.
    Ambiguous {
        scope: Scope,
        key: String,
        actions: Vec<ActionId>,
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

/// `'a'`, `'b'` and `'c'` — a list a sentence can hold.
fn join_and(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ambiguous {
                scope,
                key,
                actions,
            } => {
                let names: Vec<String> = actions
                    .iter()
                    .map(|action| format!("'{}'", action.name()))
                    .collect();
                // Two read as a pair. Three or more need different advice:
                // "give one of them another key" would leave the file still
                // ambiguous between the two that remain.
                if let [first, second] = names.as_slice() {
                    write!(
                        f,
                        "{first} and {second} both bind '{key}' in the {} scope. \
                         Nothing chooses between them: give one of them another key.",
                        scope.name(),
                    )
                } else {
                    write!(
                        f,
                        "{} all bind '{key}' in the {} scope. Nothing chooses between \
                         them: leave the key to one of them and give the rest another key.",
                        join_and(&names),
                        scope.name(),
                    )
                }
            }
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
///
/// Its `String` is one concrete key, rendered by `Chord::label`, which
/// `Keymap::evict` reads back through `help::chords_for_label` — so a row
/// holding a range loses only the key it lost, not the whole range. The one
/// chord `Chord::label` cannot render back is a function key, and no eviction
/// can carry one: the loser of an eviction is always an action the file did
/// not write, so its rows are `DEFAULT`'s own, and
/// `every_default_key_renders_back_to_a_label` pins that every one of those
/// can be read back.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
    pub evict: Vec<(Scope, String, ActionId)>,
}

/// One key in one scope, and every action that claims it, in table order.
struct Claim {
    scope: Scope,
    key: Chord,
    actions: Vec<ActionId>,
}

/// Group the built table's rows by scope and concrete key, keeping table
/// order.
///
/// A group of one is the ordinary case and the overwhelming majority. A group
/// of two or more is a contest, and every rule in this module is about one.
///
/// **By key, not by label text.** `resolve` answers a keypress through
/// `help::label_matches`, which expands a label before comparing, so `5` and
/// `1-9` are two spellings that the same keypress reaches. Grouping by the
/// label string could not see that, and the contest it missed is exactly the
/// one this module exists to report: a written `'nav.up' = '5'` that the
/// global `1-9` answers first was accepted in silence.
///
/// Each entry is therefore expanded to the keys it actually names and indexed
/// by each of them. An equivalence class over labels would not do: `1-3` and
/// `3-5` contest while `1-3` and `5-7` do not, so contesting is not
/// transitive and cannot define a grouping.
fn claims(built: &Keymap) -> Vec<Claim> {
    let mut groups: Vec<Claim> = Vec::new();
    for (scope, label, action) in &built.entries {
        for key in crate::help::chords_for_label(label) {
            match groups
                .iter_mut()
                .find(|claim| claim.scope == *scope && claim.key == key)
            {
                Some(claim) => {
                    // An action bound to one key two times in one scope cannot
                    // happen through `rebind`, but de-duplicating costs one
                    // line and keeps a group's length meaning "how many
                    // actions". Two labels of one action reaching one key —
                    // `'global.quit' = ['1-9', '5']` — is the shape that now
                    // arrives here, and it is not a contest with itself.
                    if !claim.actions.contains(action) {
                        claim.actions.push(*action);
                    }
                }
                None => groups.push(Claim {
                    scope: *scope,
                    key,
                    actions: vec![*action],
                }),
            }
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

        if mine.len() >= 2 {
            report.errors.push(Problem::Ambiguous {
                scope: claim.scope,
                key: claim.key.label(),
                actions: mine.clone(),
            });
            continue;
        }

        // Exactly one written claim: it wins, and every default claim loses.
        // No written claim at all cannot occur — no scope in `DEFAULT` holds
        // one key two times, and `the_defaults_hold_no_duplicate_key` proves
        // it — so the loop body simply does not run.
        if let [winner] = mine.as_slice() {
            for loser in claim.actions.iter().copied().filter(|a| a != winner) {
                report.evict.push((claim.scope, claim.key.label(), loser));
                report.warnings.push(Problem::Displaced {
                    scope: claim.scope,
                    key: claim.key.label(),
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
    let winner_in = |scope: Scope, key: Chord| -> Option<ActionId> {
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
                key: claim.key.label(),
                written: pane,
                global,
            });
        } else if written.contains(&global) {
            // The user wrote the global line and got what they asked for.
            // The pane default is what it cost.
            report.evict.push((claim.scope, claim.key.label(), pane));
            report.warnings.push(Problem::Displaced {
                scope: claim.scope,
                key: claim.key.label(),
                winner: global,
                loser: pane,
                remaining: Vec::new(),
            });
        }
        // Neither written: two defaults crossing, which `DEFAULT` does not do
        // and `no_default_key_is_in_both_global_and_a_pane` pins.
    }

    widen_evictions(built, &mut report);
    fill_remaining(built, &mut report);
    report
}

/// Take a key from every scope an action holds it in, once it is taken from
/// one of them.
///
/// `hit.next` and `hit.prev` are the only actions with a row in two scopes,
/// and a `[keymap]` line names an action with no scope in it. So "n reaches
/// `hit.next` in the filter pane but not in the file view" is a state the
/// config file has no way to write down — and evicting one scope at a time
/// built exactly that state. `'view.line.end' = ['n']` left `hit.next`
/// holding the filter pane's `n`; `--print-keymap` then printed
/// `'hit.next' = 'n'` with no comment, because `labels_for` deduplicates
/// across scopes, and pasting that line back bound `n` in both scopes again
/// and was refused as ambiguous. The README calls that output ready to copy
/// from, so the map it prints has to be a map a user can write down.
///
/// The eviction is therefore all or nothing. It costs the action a key in a
/// pane that contested nothing, which is a real cost and is not hidden: the
/// warning names what the action still answers to, and `fill_remaining` runs
/// after this, so it counts both scopes.
fn widen_evictions(built: &Keymap, report: &mut Report) {
    // Over a snapshot: a row added here is the widened one, in another scope
    // of the same action for the same key, so one pass reaches every scope.
    for (scope, key, action) in report.evict.clone() {
        let lost = crate::help::chords_for_label(&key);
        for (entry_scope, label, entry_action) in &built.entries {
            if *entry_action != action || *entry_scope == scope {
                continue;
            }
            if crate::help::chords_for_label(label)
                .iter()
                .any(|chord| lost.contains(chord))
            {
                let row = (*entry_scope, key.clone(), action);
                if !report.evict.contains(&row) {
                    report.evict.push(row);
                }
            }
        }
    }
}

/// Say what each displaced action still answers to, once every eviction is
/// known.
///
/// Done in a second pass rather than inline: an action can lose two keys in
/// one file, and a message naming what is left must count both losses.
///
/// Asked of a map with the evictions already applied rather than by filtering
/// the labels of the map without them. The two agreed while a key was a label,
/// and stopped agreeing when it became a concrete key: an action losing one
/// key of a range keeps the rest, which a filter comparing a lost `5` against
/// a held `1-9` would report as having lost nothing at all.
fn fill_remaining(built: &Keymap, report: &mut Report) {
    let mut after = built.clone();
    after.evict(&report.evict);
    for problem in &mut report.warnings {
        let Problem::Displaced {
            loser, remaining, ..
        } = problem
        else {
            continue;
        };
        *remaining = after
            .labels_for(*loser)
            .into_iter()
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
        let (built, _) = Keymap::new(&overlay(pairs)).expect("valid");
        check(&built, &written(pairs))
    }

    /// A user who has written nothing must be told nothing.
    ///
    /// Load-bearing rather than trivial since `claims` began expanding labels
    /// (the fix wave's item 1): `1-9` is a `DEFAULT` label, and an expansion
    /// that manufactured a collision between it and any other default would
    /// surface here as a report on a config file that does not exist.
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
                actions: vec![ActionId::GlobalQuit, ActionId::GlobalReload],
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
        let (built, _) = Keymap::new(&overlay).expect("valid");
        let written: Vec<ActionId> = crate::keymap::every_action().collect();

        let report = check(&built, &written);
        assert_eq!(report, Report::default(), "{report:?}");
    }

    // ---- a key is a key, however it is spelled (fix wave, item 1) --------

    /// The defect the concrete-key grouping exists to remove. `resolve`
    /// expands a label before matching, so the global `1-9` answers `5` and
    /// answers it first; a written `nav.up = '5'` can never fire. Comparing
    /// label strings saw `'5'` and `'1-9'` as unrelated and let the file
    /// through in silence.
    #[test]
    fn a_pane_line_under_a_default_range_is_an_error() {
        let report = report(&[("nav.up", &["5"])]);

        assert_eq!(report.warnings, vec![]);
        assert_eq!(report.evict, vec![]);
        assert_eq!(
            report.errors,
            vec![Problem::Unreachable {
                scope: Scope::Nav,
                key: "5".to_string(),
                written: ActionId::NavUp,
                global: ActionId::GlobalFiltersToggle,
            }],
            "'5' and '1-9' are one key however differently they are spelled"
        );
    }

    /// Two written ranges that overlap on one key. The message must name the
    /// key they collide on and not the range either was written as: `1-3`
    /// against `3-5` is a fault about `3`, and telling the user about `1-3`
    /// would name two keys that are not in contest.
    #[test]
    fn two_written_ranges_are_ambiguous_on_the_key_they_share() {
        let report = report(&[("global.quit", &["1-3"]), ("global.reload", &["3-5"])]);

        assert_eq!(
            report.errors,
            vec![Problem::Ambiguous {
                scope: Scope::Global,
                key: "3".to_string(),
                actions: vec![ActionId::GlobalQuit, ActionId::GlobalReload],
            }],
            "1 and 2 are quit's alone, 4 and 5 are reload's: only 3 is contested"
        );
    }

    /// `help::label_matches` tests the two modifiers separately from the
    /// expansion, so `Ctrl-q` and `q` are two keys that share a character and
    /// contest nothing. An expansion-based identity would lose that, because
    /// `keys_for_label` strips the prefix before expanding — which is why
    /// `Chord` carries the flags alongside the key.
    #[test]
    fn a_modified_key_does_not_contest_with_the_plain_one() {
        let report = report(&[("global.quit", &["Ctrl-q"]), ("global.reload", &["q"])]);

        assert_eq!(
            report,
            Report::default(),
            "Ctrl-q and q must not be read as one key: {report:?}"
        );
    }

    /// A written line takes one key of a default range, and the range keeps
    /// the rest. What `global.filters.toggle` still answers to has to be the
    /// eight digits it kept — a message naming `1-9` would tell the user that
    /// `5` still toggles a filter, which is the thing that just stopped being
    /// true.
    #[test]
    fn taking_one_key_of_a_range_leaves_the_rest_of_it() {
        let report = report(&[("global.editor.project", &["5"])]);

        assert_eq!(report.errors, vec![]);
        assert_eq!(
            report.warnings,
            vec![Problem::Displaced {
                scope: Scope::Global,
                key: "5".to_string(),
                winner: ActionId::GlobalEditorProject,
                loser: ActionId::GlobalFiltersToggle,
                remaining: ["1", "2", "3", "4", "6", "7", "8", "9"]
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            }]
        );
        assert_eq!(
            report.evict,
            vec![(
                Scope::Global,
                "5".to_string(),
                ActionId::GlobalFiltersToggle
            )],
            "one key of the row goes, not the row"
        );
    }

    /// Three written lines on one key name all three.
    ///
    /// The README promises that one run tells you everything to correct, and
    /// an error naming two of three claimants breaks that promise: the user
    /// corrects the file, runs recon again and is told about the third. It
    /// matters more after the grouping became concrete, since one range can
    /// now put several written lines in one group.
    #[test]
    fn three_written_lines_on_one_key_name_every_one_of_them() {
        let report = report(&[
            ("global.quit", &["="]),
            ("global.reload", &["="]),
            ("global.peek", &["="]),
        ]);

        assert_eq!(report.errors.len(), 1, "{report:?}");
        let Problem::Ambiguous {
            scope,
            key,
            actions,
        } = &report.errors[0]
        else {
            panic!("a contested key is Ambiguous: {report:?}");
        };
        assert_eq!(*scope, Scope::Global);
        assert_eq!(key, "=");
        assert_eq!(actions.len(), 3, "{actions:?}");
        for claimant in [
            ActionId::GlobalQuit,
            ActionId::GlobalReload,
            ActionId::GlobalPeek,
        ] {
            assert!(actions.contains(&claimant), "{actions:?}");
        }

        // The message is what a user actually sees, so it has to name them
        // all as well.
        let said = report.errors[0].to_string();
        for name in ["global.quit", "global.reload", "global.peek"] {
            assert!(said.contains(name), "{name} is missing from: {said}");
        }
    }
}
