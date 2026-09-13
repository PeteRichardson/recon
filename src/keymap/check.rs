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
    ///
    /// Two different facts, kept apart rather than apportioned, and each field
    /// named for the one it holds. `taken_in` is where **this winner** took
    /// the key. `lost_in` is every scope the loser lost it in, whoever took it
    /// there.
    ///
    /// They differ whenever eviction is widened: `hit.next` losing `n` to a
    /// file-view line loses it in the filter pane too, which nothing
    /// contested. And they differ again when two lines take one key from one
    /// two-scope action — `'view.line.end' = 'n'` beside `'filters.solo' =
    /// 'n'` — where crediting either winner with both scopes says it took a
    /// key the *other* line took. A user who then deleted one line would
    /// expect `n` back in a pane that never gets it.
    ///
    /// One list serving both is what produced that, and a field called
    /// `scopes` holding the loser's losses is what made the wrong sentence
    /// feel right. Two fields, two names, two facts.
    Displaced {
        taken_in: Vec<Scope>,
        key: String,
        winner: ActionId,
        loser: ActionId,
        remaining: Vec<String>,
        lost_in: Vec<Scope>,
    },
}

/// `scope` or `scopes`, to agree with how many are named.
fn scope_noun(count: usize) -> &'static str {
    if count == 1 { "scope" } else { "scopes" }
}

/// The scopes in `Scope`'s own order, each once.
///
/// A list of scopes is a **set**. Which order the rows happened to be evicted
/// in says nothing about the keymap, so it must not reach the sentence:
/// `'filters.solo' = 'n'` evicted the filter pane first and the file view
/// second and said "the filters and view scopes", while `'view.line.end' =
/// 'n'` reported the identical loss the other way round.
///
/// Sorting into the discriminant order is what makes it canonical, and that
/// order is already the precedence this whole module reasons in. Comparing two
/// sorted lists is then the set comparison the second clause actually wants —
/// which also makes the sentence safe against anything that ever sorts
/// `evict`, where a `Vec` comparison would start printing a clause that merely
/// restates the first.
fn in_order(scopes: &[Scope]) -> Vec<Scope> {
    let mut ordered = scopes.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    ordered
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
                taken_in,
                key,
                winner,
                loser,
                remaining,
                lost_in,
            } => {
                // Canonical before a word is said about either, so the
                // sentence cannot depend on which line provoked the loss.
                let taken = in_order(taken_in);
                let lost = in_order(lost_in);
                let names: Vec<String> =
                    taken.iter().map(|scope| scope.name().to_string()).collect();
                write!(
                    f,
                    "'{}' takes '{key}' from '{}'",
                    winner.name(),
                    loser.name(),
                )?;
                // The same guard the second clause carries, for the same
                // reason: `taken_in` is unreachable-empty by construction, and
                // `Display` is a release build's only protection if that ever
                // stops being true. Without it an empty list renders "in the
                // scopes" with a doubled space, exactly as `lost_in` did.
                if !names.is_empty() {
                    write!(
                        f,
                        " in the {} {}",
                        join_and(&names),
                        scope_noun(names.len()),
                    )?;
                }
                // Only when the loser lost the key somewhere this winner did
                // not take it. A set comparison, not a `Vec` one: the same
                // scopes in another order are not a second fact, and saying
                // them again would only restate the clause above. Equal sets
                // are the overwhelmingly common case — one winner, one scope
                // — and read as they always did, with nothing appended.
                //
                // The emptiness test belongs here, where the text is produced,
                // and not beside the assignment that fills `lost_in`. An empty
                // list there is indistinguishable from the empty list the
                // warning was built with, so a guard at that end cannot tell
                // the two apart; here it is the difference between saying
                // nothing and printing "in the  scopes" with a doubled space,
                // an empty `join_and` and a pluralised `scope_noun`.
                if !lost.is_empty() && lost != taken {
                    let all: Vec<String> =
                        lost.iter().map(|scope| scope.name().to_string()).collect();
                    write!(
                        f,
                        "; '{}' loses '{key}' in the {} {}",
                        loser.name(),
                        join_and(&all),
                        scope_noun(all.len()),
                    )?;
                }
                write!(f, ". ")?;
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
///
/// The three fields are **private**, which constrains the rest of the crate:
/// three `pub` fields become read-only accessors, and a `Report` can no longer
/// be built by struct literal outside this module.
///
/// That is defence in depth, and not what makes the pairing true — an earlier
/// version of this comment claimed it was. Privacy is only module-deep: the
/// `tests` child module below still reaches these fields directly, and the
/// writer most likely to break the pairing is a new checking pass added beside
/// `check`, `widen_evictions` and `fill_scopes`, which is precisely where
/// passes live and precisely where privacy does nothing.
///
/// What carries the invariant is `displace`, the only thing that creates a
/// `Displaced` warning, together with the `debug_assert!` in `fill_scopes`
/// that catches a pass raising one without its eviction. That assert is
/// debug-only, which is why the emptiness test in `Display` matters: it is a
/// release build's whole protection against the malformed sentence.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    errors: Vec<Problem>,
    warnings: Vec<Problem>,
    evict: Vec<(Scope, String, ActionId)>,
}

impl Report {
    /// The faults that stop recon starting.
    pub(crate) fn errors(&self) -> &[Problem] {
        &self.errors
    }

    /// What the file cost, which recon reports and then carries on.
    pub(crate) fn warnings(&self) -> &[Problem] {
        &self.warnings
    }

    /// The rows `Keymap::evict` has to remove for a written line to win.
    pub(crate) fn evict(&self) -> &[(Scope, String, ActionId)] {
        &self.evict
    }

    /// Record a default losing a key: the row that must go, and the warning
    /// that says so.
    ///
    /// One method because the two must always happen together, and saying so
    /// by construction is better than checking for it afterwards. An eviction
    /// with no warning takes a key in silence; a warning with no eviction
    /// reports a loss that never happened, and `fill_scopes` would then find
    /// no scope to name it in.
    fn displace(&mut self, scope: Scope, key: String, winner: ActionId, loser: ActionId) {
        self.evict.push((scope, key.clone(), loser));
        self.warnings.push(Problem::Displaced {
            taken_in: vec![scope],
            key,
            winner,
            loser,
            // Both filled once every eviction is known.
            remaining: Vec::new(),
            lost_in: Vec::new(),
        });
    }
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
                report.displace(claim.scope, claim.key.label(), *winner, loser);
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
            report.displace(claim.scope, claim.key.label(), global, pane);
        }
        // Neither written: two defaults crossing, which `DEFAULT` does not do
        // and `no_default_key_is_in_both_global_and_a_pane` pins.
    }

    widen_evictions(built, &mut report);
    fill_scopes(&mut report);
    fill_remaining(built, &mut report);
    report
}

/// Gather what each winner took onto one warning, then say what the loser lost
/// altogether.
///
/// Two steps, because the two facts come from different places.
///
/// **What a winner took is gathered from the warnings themselves.** Each
/// raising recorded the one scope where it saw that winner beat that loser, so
/// merging the raisings that share a key, a **winner** and a loser rebuilds
/// exactly the set of scopes that winner took the key in.
///
/// Merging on the winner as well as the key and the loser is what keeps both
/// properties at once. Two raisings of one loss still collapse, by either
/// route — pass two meeting a global line against each pane, or pass one
/// meeting a two-scope action's own groups — so a panel that truncates never
/// spends two rows on one loss. Two *different* lines taking one key from one
/// two-scope action do not collapse, and neither is credited with the scope
/// the other took. Merging on the key and loser alone gave the first property
/// by giving both warnings the same union, and paid for it with the second.
///
/// **The loser's total loss comes from the evictions**, which is the one place
/// that knows every row that went: `widen_evictions` removes rows that no pass
/// reported, so no warning can know the total on its own.
fn fill_scopes(report: &mut Report) {
    let mut merged: Vec<Problem> = Vec::new();
    for problem in report.warnings.drain(..) {
        let Problem::Displaced {
            taken_in,
            key,
            winner,
            loser,
            ..
        } = &problem
        else {
            merged.push(problem);
            continue;
        };
        let (key, winner, loser) = (key.clone(), *winner, *loser);
        let raised_in = taken_in.clone();

        let same = merged.iter_mut().find(|seen| {
            matches!(
                seen,
                Problem::Displaced { key: k, winner: w, loser: l, .. }
                    if *k == key && *w == winner && *l == loser
            )
        });
        if let Some(Problem::Displaced { taken_in, .. }) = same {
            for scope in raised_in {
                if !taken_in.contains(&scope) {
                    taken_in.push(scope);
                }
            }
        } else {
            merged.push(problem);
        }
    }
    report.warnings = merged;

    let evicted = report.evict.clone();
    for problem in &mut report.warnings {
        let Problem::Displaced {
            taken_in,
            key,
            loser,
            lost_in,
            ..
        } = problem
        else {
            continue;
        };
        let (lost_key, lost_by) = (key.clone(), *loser);
        let lost: Vec<Scope> = evicted
            .iter()
            .filter(|(_, evicted_key, action)| *evicted_key == lost_key && *action == lost_by)
            .map(|(scope, _, _)| *scope)
            .collect();
        // `Report::displace` pushes the eviction and the warning together, so
        // a warning always has at least the row it was raised for. Asserted
        // rather than guarded: a future path that raised one without the other
        // should fail here in a debug build. What protects a release build is
        // the emptiness test in `Display`, which is where the malformed
        // sentence would otherwise be written — a guard at this end could not
        // do it, because skipping the assignment leaves `lost_in` at the very
        // same empty value `Report::displace` built it with.
        debug_assert!(
            !lost.is_empty(),
            "a displaced warning with no eviction behind it: {lost_by:?} lost {lost_key:?}"
        );
        // Stored in `Scope` order, not only rendered in it. `Problem` derives
        // `PartialEq`, so two reports describing one keymap have to compare
        // equal; leaving the lists in the order the rows happened to go would
        // revive the order bug inside any later dedup, snapshot or cache keyed
        // on that equality.
        //
        // `Display` keeps its own `in_order` calls regardless, and they are
        // not redundant: they make the comparison set-based for **any**
        // `Problem`, including one built by hand that never passes through
        // here — which is exactly what
        // `equal_scope_sets_in_another_order_are_not_said_twice` constructs.
        let ordered = in_order(taken_in);
        *taken_in = ordered;
        *lost_in = in_order(&lost);
    }
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
                taken_in: vec![Scope::Global],
                key: "o".to_string(),
                winner: ActionId::GlobalQuit,
                loser: ActionId::GlobalEditorProject,
                remaining: vec![],
                lost_in: vec![Scope::Global],
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
                    taken_in: vec![Scope::Nav],
                    key: "j".to_string(),
                    winner: ActionId::GlobalQuit,
                    loser: ActionId::NavDown,
                    remaining: vec!["Down".to_string()],
                    lost_in: vec![Scope::Nav],
                },
                Problem::Displaced {
                    taken_in: vec![Scope::View],
                    key: "j".to_string(),
                    winner: ActionId::GlobalQuit,
                    loser: ActionId::ViewDown,
                    remaining: vec!["Down".to_string()],
                    lost_in: vec![Scope::View],
                },
                Problem::Displaced {
                    taken_in: vec![Scope::Filters],
                    key: "j".to_string(),
                    winner: ActionId::GlobalQuit,
                    loser: ActionId::FiltersDown,
                    remaining: vec!["Down".to_string()],
                    lost_in: vec![Scope::Filters],
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
                taken_in: vec![Scope::Global],
                key: "5".to_string(),
                winner: ActionId::GlobalEditorProject,
                loser: ActionId::GlobalFiltersToggle,
                remaining: ["1", "2", "3", "4", "6", "7", "8", "9"]
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                lost_in: vec![Scope::Global],
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

    /// A loss that spans two scopes reports both facts: where the winner took
    /// the key, and everywhere the loser lost it.
    ///
    /// `hit.next` holds a row in the file view and another in the filter
    /// pane. The file view is where the contest is; the filter pane loses `n`
    /// without ever contesting it, because eviction is all or nothing across
    /// an action's scopes. Naming only the file view left a user unable to
    /// tell why an uncontested filter-pane binding had gone — but crediting
    /// `view.line.end` with both scopes says it took a key in a pane it never
    /// touched, which is the fault the next test pins.
    #[test]
    fn a_two_scope_loss_names_every_scope_it_happened_in() {
        let report = report(&[("view.line.end", &["n"])]);

        assert_eq!(report.errors, vec![]);
        assert_eq!(
            report.warnings,
            vec![Problem::Displaced {
                taken_in: vec![Scope::View],
                key: "n".to_string(),
                winner: ActionId::ViewLineEnd,
                loser: ActionId::HitNext,
                remaining: vec![],
                lost_in: vec![Scope::View, Scope::Filters],
            }],
            "the winner took it in the view; the loser lost it in both"
        );

        let said = report.warnings[0].to_string();
        assert!(
            said.contains("takes 'n' from 'hit.next' in the view scope"),
            "the winner's own scope, alone: {said}"
        );
        assert!(
            said.contains("'hit.next' loses 'n' in the view and filters scopes"),
            "and everywhere the loser lost it: {said}"
        );
    }

    /// Two lines taking one key from one two-scope action: each names only
    /// the scope it took the key in.
    ///
    /// `fill_scopes` gathered scopes by key and loser alone, so both warnings
    /// were handed the union and each sentence credited its winner with the
    /// scope the *other* line took. The eviction data was right — `hit.next`
    /// really does lose `n` in both — so recon behaved correctly and only the
    /// sentence lied. A user who deleted the `view.line.end` line would expect
    /// `n` back in the filter pane, and it would not come back.
    #[test]
    fn two_winners_on_one_key_each_name_only_their_own_scope() {
        let report = report(&[("view.line.end", &["n"]), ("filters.solo", &["n"])]);

        assert_eq!(report.errors, vec![]);
        assert_eq!(
            report.warnings,
            vec![
                Problem::Displaced {
                    taken_in: vec![Scope::View],
                    key: "n".to_string(),
                    winner: ActionId::ViewLineEnd,
                    loser: ActionId::HitNext,
                    remaining: vec![],
                    lost_in: vec![Scope::View, Scope::Filters],
                },
                Problem::Displaced {
                    taken_in: vec![Scope::Filters],
                    key: "n".to_string(),
                    winner: ActionId::FiltersSolo,
                    loser: ActionId::HitNext,
                    remaining: vec![],
                    lost_in: vec![Scope::View, Scope::Filters],
                },
            ],
            "two winners, two warnings, neither crediting the other's scope"
        );

        let view = report.warnings[0].to_string();
        assert!(
            view.contains("'view.line.end' takes 'n' from 'hit.next' in the view scope;"),
            "{view}"
        );
        assert!(
            !view.contains("takes 'n' from 'hit.next' in the view and filters"),
            "view.line.end never took 'n' in the filter pane: {view}"
        );

        let filters = report.warnings[1].to_string();
        assert!(
            filters.contains("'filters.solo' takes 'n' from 'hit.next' in the filters scope;"),
            "{filters}"
        );
        // The total loss is still reported, by both, because both caused part
        // of it and either line is where a reader will be looking.
        for said in [&view, &filters] {
            assert!(
                said.contains("'hit.next' loses 'n' in the view and filters scopes"),
                "{said}"
            );
        }
    }

    /// One loss met in two scopes is one warning, not two — by both of the
    /// routes that reach the collapse.
    ///
    /// A panel that truncates must not spend two rows on one loss, so the
    /// merge matters; and it is reached by two different pieces of code, only
    /// one of which used to be guarded.
    ///
    /// **Pass two** meets a global line against each pane separately, so
    /// `'global.quit' = 'n'` raises `hit.next` once in the file view and again
    /// in the filter pane. **Pass one** gets there another way: `'hit.next' =
    /// 'N'` puts one winner and one loser in the View group and again in the
    /// Filters group. Both describe a single loss — same key, same winner,
    /// same loser — and merge into one warning carrying both scopes.
    ///
    /// The two-scope test above reaches neither: it raises only one warning to
    /// begin with.
    #[test]
    fn one_loss_met_in_two_scopes_is_one_warning() {
        // Named rather than shadowing the `report` helper, since this test
        // builds two reports.
        let displaced = |report: &Report, against: ActionId| -> Vec<Problem> {
            report
                .warnings
                .iter()
                .filter(|problem| {
                    matches!(problem, Problem::Displaced { loser, .. } if *loser == against)
                })
                .cloned()
                .collect()
        };

        // Pass two's route.
        let global = report(&[("global.quit", &["n"])]);
        assert_eq!(
            displaced(&global, ActionId::HitNext),
            vec![Problem::Displaced {
                taken_in: vec![Scope::View, Scope::Filters],
                key: "n".to_string(),
                winner: ActionId::GlobalQuit,
                loser: ActionId::HitNext,
                remaining: vec![],
                lost_in: vec![Scope::View, Scope::Filters],
            }],
            "one loss, one warning, naming both scopes: {:?}",
            global.warnings
        );

        // The navigator's `n` is a different action losing its own key, so it
        // stays a warning in its own right — collapsing must not swallow it.
        assert!(
            !displaced(&global, ActionId::NavHitNext).is_empty(),
            "nav.hit.next lost 'n' too and must still be reported: {:?}",
            global.warnings
        );

        // Pass one's route, which is different code reaching the same merge.
        let pane = report(&[("hit.next", &["N"])]);
        assert_eq!(
            displaced(&pane, ActionId::HitPrev),
            vec![Problem::Displaced {
                taken_in: vec![Scope::View, Scope::Filters],
                key: "N".to_string(),
                winner: ActionId::HitNext,
                loser: ActionId::HitPrev,
                remaining: vec![],
                lost_in: vec![Scope::View, Scope::Filters],
            }],
            "pass one reaches the collapse too: {:?}",
            pane.warnings
        );
        assert_eq!(
            pane.evict.len(),
            2,
            "the row goes from both scopes, though one warning says so: {:?}",
            pane.evict
        );
    }

    /// One loss, provoked three ways, said the same way each time.
    ///
    /// `hit.next` loses `n` in both scopes under every one of these, but the
    /// order the rows were evicted in differed: `'filters.solo' = 'n'` took
    /// the filter pane first and the file view second, and the sentence said
    /// "the filters and view scopes" while the other two said "the view and
    /// filters scopes". Same loss, same set, different words — the order rows
    /// happened to go in is not information about the keymap.
    #[test]
    fn the_sentence_does_not_depend_on_the_order_rows_were_evicted_in() {
        let sentences = |report: &Report| -> Vec<String> {
            report
                .warnings
                .iter()
                .filter(|problem| {
                    matches!(problem, Problem::Displaced { loser, .. } if *loser == ActionId::HitNext)
                })
                .map(ToString::to_string)
                .collect()
        };

        let filters = report(&[("filters.solo", &["n"])]);
        let view = report(&[("view.line.end", &["n"])]);
        let both = report(&[("view.line.end", &["n"]), ("filters.solo", &["n"])]);

        for report in [&filters, &view, &both] {
            let said = sentences(report);
            assert!(!said.is_empty(), "hit.next must be reported: {report:?}");
            for sentence in said {
                assert!(
                    sentence.contains("loses 'n' in the view and filters scopes"),
                    "one canonical order, whichever line provoked it: {sentence}"
                );
                assert!(
                    !sentence.contains("filters and view"),
                    "the eviction order leaked into the sentence: {sentence}"
                );
            }
        }
    }

    /// The same scopes in another order are one fact, not two.
    ///
    /// The second clause exists to add what the first did not say. Comparing
    /// the two lists as `Vec`s rather than as sets made that depend on their
    /// order, which is safe only while `evict` is built in one particular way:
    /// anything that later sorted it — a canonical scope order for
    /// `--print-keymap`, say — would produce a set-equal, order-different pair
    /// and the clause would print as a restatement of the one before it.
    #[test]
    fn equal_scope_sets_in_another_order_are_not_said_twice() {
        let problem = Problem::Displaced {
            taken_in: vec![Scope::View, Scope::Filters],
            key: "n".to_string(),
            winner: ActionId::ViewLineEnd,
            loser: ActionId::HitNext,
            remaining: vec![],
            lost_in: vec![Scope::Filters, Scope::View],
        };

        let said = problem.to_string();
        assert!(
            !said.contains("loses 'n'"),
            "the same set reordered is not a second fact: {said}"
        );
        assert!(
            said.contains("takes 'n' from 'hit.next' in the view and filters scopes"),
            "and the one clause it does print is canonical: {said}"
        );
    }

    /// A warning with no scopes on its loser says nothing about them.
    ///
    /// The state is unreachable — `Report::displace` gives every warning its
    /// eviction — and the `debug_assert!` in `fill_scopes` records that. This
    /// pins what a **release** build does if the fact ever stops being true,
    /// which is the half that was wrong: the guard originally sat beside the
    /// assignment, where skipping it left `lost_in` at the very same empty
    /// value `displace` had built it with, so both branches ended identical
    /// and the malformed sentence printed anyway.
    #[test]
    fn an_empty_lost_in_prints_no_second_clause() {
        let problem = Problem::Displaced {
            taken_in: vec![Scope::View],
            key: "n".to_string(),
            winner: ActionId::ViewLineEnd,
            loser: ActionId::HitNext,
            remaining: vec![],
            lost_in: vec![],
        };

        let said = problem.to_string();
        assert!(
            !said.contains("loses 'n'"),
            "there is nothing to say about scopes that are not there: {said}"
        );
        assert!(
            !said.contains("in the  scopes"),
            "the doubled space is the malformed render this guards: {said}"
        );
        assert!(
            said.contains("'view.line.end' takes 'n' from 'hit.next' in the view scope."),
            "and the clause it can say is unharmed: {said}"
        );
    }

    /// The scope lists are stored in `Scope` order, not the order the rows
    /// happened to be evicted in.
    ///
    /// `in_order` runs inside `fmt`, so printing has been canonical since the
    /// order first leaked into a sentence — but the stored lists were left as
    /// they were built, and `Problem` derives `PartialEq`. Two reports
    /// describing one keymap would compare unequal, which is invisible today
    /// and would revive the order bug inside any later dedup, snapshot or
    /// cache keyed on that equality.
    ///
    /// `'filters.solo' = 'n'` is the case that shows it: pass one evicts the
    /// filter pane, and `widen_evictions` adds the file view afterwards, so
    /// the list is built in the opposite order to the one `Scope` defines.
    #[test]
    fn the_stored_scope_lists_are_canonical() {
        let report = report(&[("filters.solo", &["n"])]);

        let Some(Problem::Displaced {
            taken_in, lost_in, ..
        }) = report.warnings().iter().find(|problem| {
            matches!(problem, Problem::Displaced { loser, .. } if *loser == ActionId::HitNext)
        }) else {
            panic!("hit.next must be reported: {report:?}");
        };

        assert_eq!(
            *taken_in,
            vec![Scope::Filters],
            "filters.solo took the key in the filter pane alone"
        );
        assert_eq!(
            *lost_in,
            vec![Scope::View, Scope::Filters],
            "stored in Scope order, not the order the rows were evicted in"
        );
    }

    /// The first clause guards its scope list exactly as the second does.
    ///
    /// `taken_in` is unreachable-empty by construction, the same as `lost_in`,
    /// and is protected in a release build only here. Without the guard it
    /// renders `takes 'n' from 'hit.next' in the  scopes` — the identical
    /// doubled-space render that was removed from the second clause, left in
    /// place on the first only because nobody looked at the adjacent line.
    #[test]
    fn an_empty_taken_in_prints_no_scope_phrase() {
        let problem = Problem::Displaced {
            taken_in: vec![],
            key: "n".to_string(),
            winner: ActionId::ViewLineEnd,
            loser: ActionId::HitNext,
            remaining: vec![],
            lost_in: vec![Scope::View, Scope::Filters],
        };

        let said = problem.to_string();
        assert!(
            !said.contains("in the  "),
            "the doubled space is the malformed render this guards: {said}"
        );
        assert!(
            said.starts_with("'view.line.end' takes 'n' from 'hit.next';"),
            "the clause drops its scope phrase and stays a sentence: {said}"
        );
        assert!(
            said.contains("'hit.next' loses 'n' in the view and filters scopes"),
            "and what is known is still said: {said}"
        );
    }

    /// No two distinct chords render the same label.
    ///
    /// This is what makes `fill_scopes` safe to merge warnings on the label
    /// string rather than on the chord itself: two chords sharing a label would
    /// fuse two warnings about genuinely different keys.
    ///
    /// The property holds by an argument that lives in `help.rs` and is easy to
    /// lose, which is why it is pinned here, beside the code that relies on it.
    ///
    /// An earlier version of this comment said two collisions existed inside
    /// `Chord::label` and both were unconstructible. **There was a third and it
    /// was constructible**, which is why the corpus below now carries the
    /// labels that reach it rather than only the shapes the argument had
    /// already considered. `Char(' ')` renders `"space"` whatever its prefix,
    /// and a prefixed label could reach that arm two ways: `Ctrl- -!` through
    /// the range arm, and `Ctrl- ` through the single character arm.
    /// `keys_for_label` now refuses `Char(' ')` to any prefixed label, so the
    /// collision is gone — by a carve-out, not by luck.
    ///
    /// The two that really were unconstructible: `Ctrl-space` is unreadable,
    /// because `"space"` is matched as a whole word before any prefix is
    /// stripped; and a chord with both modifiers cannot be built, because
    /// `chords_for_label` reads both flags with `starts_with` and no string
    /// starts with `Ctrl-` and `Alt-` at once.
    ///
    /// **Injectivity, not a round trip.** A round trip is too strong and would
    /// fail: `'F5'` is a readable label whose chord renders `"any function
    /// key"`, which reads back as nothing.
    /// `every_default_key_renders_back_to_a_label` passes only because no
    /// `DEFAULT` label is a function key, so it does not generalise to the
    /// labels a user writes. Every function key collapsing to one chord is
    /// `named_matches`'s doing and predates this branch; it costs no
    /// injectivity, because the three F-chords still render apart.
    #[test]
    fn a_chord_renders_to_a_label_no_other_chord_shares() {
        // The whole built-in table, plus every other shape the grammar reads.
        let mut labels: Vec<String> = crate::keymap::DEFAULT
            .iter()
            .map(|(_, label, _)| (*label).to_string())
            .collect();
        for extra in [
            "q",
            "Q",
            "Ctrl-q",
            "Alt-q",
            "space",
            "Shift-Tab",
            "BackTab",
            "Ctrl-BackTab",
            "Alt-BackTab",
            "Enter",
            "Ctrl-Enter",
            "Alt-Enter",
            "F1",
            "F12",
            "Ctrl-F1",
            "Alt-F5",
            "1-9",
            "a-f",
            "*-/",
            "5-<",
            // The three that broke injectivity, or would have. The first two
            // reach `Char(' ')` through a prefix — the range arm and the
            // single character arm — and both rendered as the plain space
            // chord until `keys_for_label` carved it out. The third is the
            // bare range that must keep its space.
            "Ctrl- -!",
            "Ctrl- ",
            " -!",
        ] {
            labels.push(extra.to_string());
        }

        let mut chords: Vec<Chord> = labels
            .iter()
            .flat_map(|label| crate::help::chords_for_label(label))
            .collect();
        chords.sort_unstable();
        chords.dedup();

        // Named rather than counted: a count tells you injectivity broke, and
        // the two chords tell you what to do about it — which is the whole
        // value of the test on the day it fails.
        let mut seen: Vec<(String, Chord)> = Vec::new();
        for chord in &chords {
            let label = chord.label();
            assert!(
                !seen.iter().any(|(taken, _)| *taken == label),
                "two distinct chords render {label:?}: {:?} and {chord:?} — \
                 `fill_scopes` merges on the label, so it would fuse warnings \
                 about different keys",
                seen.iter()
                    .find(|(taken, _)| *taken == label)
                    .map(|(_, other)| *other),
            );
            seen.push((label, *chord));
        }

        // The spellings the argument above turns on being unreadable. If any
        // of these ever parses, `Char(' ')` or `Named(\"BackTab\")` gains a
        // prefix it cannot render, and the collision becomes reachable.
        for unreadable in ["Ctrl-space", "Alt-space", "Ctrl-Shift-Tab"] {
            assert!(
                crate::help::chords_for_label(unreadable).is_empty(),
                "{unreadable} must stay unreadable"
            );
        }
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
