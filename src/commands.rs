//! The commands the palette lists (#520), and which of them a typed word
//! matches.
//!
//! A command that a key can run has one name, its keymap name; a short
//! name is only for a command that takes an argument, or for a well-known
//! vim alias, and it shares its command's row (ADR 0011).

use crate::filter::Sense;
use crate::keymap::{ActionId, Scope};
use std::sync::LazyLock;

/// One row of the palette's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Command {
    /// Every name that runs it, the main one first: `e` then `cd`.
    pub(crate) names: Vec<&'static str>,
    /// What it takes after its name.
    pub(crate) argument: Argument,
    /// Its line of help: the help overlay's text for an action.
    pub(crate) help: &'static str,
    /// What running it does.
    pub(crate) run: Run,
}

/// What a command takes after its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Argument {
    None,
    /// A path, which Tab completes.
    Path,
    /// A number.
    Number,
    /// A set's name, then `:` and a list of its profiles and filters, which
    /// Tab completes.
    SetList,
    /// A set's name, which Tab completes.
    Set,
    /// A view's name, which Tab completes.
    View,
    /// A filter's pattern.
    Pattern,
}

/// What running a command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Run {
    /// `e`: go to a folder or a file.
    Path,
    /// `goto`: go to a line of the file.
    Line,
    /// `set`: list and enable a set.
    EnableSet,
    /// `list`: list a set, disabled.
    ListSet,
    /// `unlist`: unlist a set.
    UnlistSet,
    /// `view`: show the open file in a view.
    View,
    /// `include`, `exclude` and `context`: add a filter of that sense to
    /// the scratch set.
    AddFilter(Sense),
    /// A keymap action, whose keys the row shows.
    Action(ActionId),
}

impl Command {
    /// The keymap action it runs, or `None` for a command no key runs.
    pub(crate) fn action(&self) -> Option<ActionId> {
        match self.run {
            Run::Action(action) => Some(action),
            Run::Path
            | Run::Line
            | Run::EnableSet
            | Run::ListSet
            | Run::UnlistSet
            | Run::View
            | Run::AddFilter(_) => None,
        }
    }

    /// The first column of its row: its names, then its argument.
    pub(crate) fn label(&self) -> String {
        let names = self.names.join(", ");
        match self.argument {
            Argument::None => names,
            Argument::Path => format!("{names} <path>"),
            Argument::Number => format!("{names} <n>"),
            Argument::SetList => format!("{names} <set>[:<list>]"),
            Argument::Set => format!("{names} <set>"),
            Argument::View => format!("{names} <view>"),
            Argument::Pattern => format!("{names} <pattern>"),
        }
    }
}

/// Every command the palette runs, in the order an empty palette lists
/// them: the commands that take an argument, then the keymap actions in
/// the keymap's own order. Built once: every part of it is fixed when recon
/// is built.
pub(crate) fn catalog() -> &'static [Command] {
    static CATALOG: LazyLock<Vec<Command>> = LazyLock::new(build);
    &CATALOG
}

/// The command `name` runs, by any of its names.
pub(crate) fn named(name: &str) -> Option<&'static Command> {
    catalog()
        .iter()
        .find(|command| command.names.contains(&name))
}

fn build() -> Vec<Command> {
    let mut commands = vec![
        Command {
            names: vec!["e", "cd"],
            argument: Argument::Path,
            help: "Go to a folder, or open a file in its folder",
            run: Run::Path,
        },
        Command {
            names: vec!["goto"],
            argument: Argument::Number,
            help: "Go to line n of the file (also :<n>)",
            run: Run::Line,
        },
        Command {
            names: vec!["set"],
            argument: Argument::SetList,
            help: "List and enable a filter set, as --set does",
            run: Run::EnableSet,
        },
        Command {
            names: vec!["list"],
            argument: Argument::Set,
            help: "List a filter set and leave it off, as --list does",
            run: Run::ListSet,
        },
        Command {
            names: vec!["unlist"],
            argument: Argument::Set,
            help: "Unlist a filter set, as --unlist does",
            run: Run::UnlistSet,
        },
        Command {
            names: vec!["view"],
            argument: Argument::View,
            help: "Show the open file in a view, by name",
            run: Run::View,
        },
        Command {
            names: vec!["include"],
            argument: Argument::Pattern,
            help: "Add an including filter to the scratch set",
            run: Run::AddFilter(Sense::Include),
        },
        Command {
            names: vec!["exclude"],
            argument: Argument::Pattern,
            help: "Add an excluding filter to the scratch set",
            run: Run::AddFilter(Sense::Exclude),
        },
        Command {
            names: vec!["context"],
            argument: Argument::Pattern,
            help: "Add a context filter to the scratch set",
            run: Run::AddFilter(Sense::Context),
        },
    ];
    for action in crate::keymap::every_action().filter(|a| is_command(*a)) {
        let mut names = vec![action.name()];
        // vim's alias, on the row of the action it names (ADR 0011).
        if action == ActionId::GlobalQuit {
            names.push("q");
        }
        commands.push(Command {
            names,
            // The digit its key gives it.
            argument: if action == ActionId::GlobalFiltersToggle {
                Argument::Number
            } else {
                Argument::None
            },
            help: help_for(action),
            run: Run::Action(action),
        });
    }
    commands
}

/// Whether the palette runs `action`: a global action or one of a pane's.
/// The actions of a prompt and of a modal screen are not commands, since
/// the palette never opens over them.
fn is_command(action: ActionId) -> bool {
    crate::keymap::scopes_of(action).iter().all(|scope| {
        matches!(
            scope,
            Scope::Global | Scope::Explorer | Scope::View | Scope::Filters
        )
    })
}

/// The help overlay's text for `action`: the first row that names it.
fn help_for(action: ActionId) -> &'static str {
    crate::help::KEYMAP
        .iter()
        .flat_map(|section| section.bindings)
        .find(|binding| binding.names.contains(&action.name()))
        .map_or("", |binding| binding.action)
}

/// The rows of `commands` that `word` matches, best first, as indexes.
///
/// An empty word matches every row, in order. Otherwise a row matches when
/// one of its names starts with the word, then when the word's letters are
/// in one of its names in order (`gfol` in `global.follow`), and last when
/// its help text holds the word (`folder` finds `e, cd`). Inside the first
/// two, the shorter span of the name the word covers comes first; a tie
/// keeps the list's order. Case is ignored.
pub(crate) fn matches(commands: &[Command], word: &str) -> Vec<usize> {
    let word = word.to_lowercase();
    let mut ranked: Vec<((u8, usize), usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| rank(command, &word).map(|rank| (rank, index)))
        .collect();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, index)| index).collect()
}

/// How well `word` matches `command`, lower is better, or `None`.
fn rank(command: &Command, word: &str) -> Option<(u8, usize)> {
    if word.is_empty() {
        return Some((0, 0));
    }
    let names = command.names.iter().map(|name| name.to_lowercase());
    let by_name = names
        .filter_map(|name| {
            if name.starts_with(word) {
                Some((0, name.len()))
            } else {
                span(&name, word).map(|span| (1, span))
            }
        })
        .min();
    by_name.or_else(|| command.help.to_lowercase().contains(word).then_some((2, 0)))
}

/// The length of the shortest part of `name` that holds the letters of
/// `word` in order, or `None` when it does not hold them.
fn span(name: &str, word: &str) -> Option<usize> {
    let name: Vec<char> = name.chars().collect();
    let word: Vec<char> = word.chars().collect();
    (0..name.len())
        .filter(|&start| name[start] == word[0])
        .filter_map(|start| {
            let mut at = start;
            for &c in &word {
                at += name[at..].iter().position(|&n| n == c)? + 1;
            }
            Some(at - start)
        })
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The main names of the rows `word` matches, best first.
    fn found(word: &str) -> Vec<&'static str> {
        let commands = catalog();
        matches(commands, word)
            .into_iter()
            .map(|index| commands[index].names[0])
            .collect()
    }

    #[test]
    fn an_empty_word_lists_every_command_in_order() {
        let commands = catalog();
        assert_eq!(
            matches(commands, ""),
            (0..commands.len()).collect::<Vec<_>>()
        );
        assert_eq!(commands[0].names, ["e", "cd"]);
        assert_eq!(commands[1].names, ["goto"]);
    }

    #[test]
    fn every_command_names_a_keymap_action_by_its_keymap_name() {
        let commands = catalog();
        for action in crate::keymap::every_action().filter(|a| is_command(*a)) {
            assert!(
                commands
                    .iter()
                    .any(|c| c.run == Run::Action(action) && c.names[0] == action.name()),
                "{} is not listed under its keymap name",
                action.name()
            );
        }
        assert!(
            !commands.iter().any(|c| c.names[0] == "prompt.cancel"),
            "an action of the prompt is not a command"
        );
    }

    #[test]
    fn an_alias_shares_its_commands_row() {
        let commands = catalog();
        let quit = commands
            .iter()
            .find(|c| c.run == Run::Action(ActionId::GlobalQuit))
            .expect("global.quit is listed");
        assert_eq!(quit.names, ["global.quit", "q"]);
        assert_eq!(found("cd")[0], "e");
        assert_eq!(found("q")[0], "global.quit");
    }

    #[test]
    fn a_prefix_comes_before_letters_in_order() {
        // `goto` starts with `go`; `global.toggle.hide` only holds the
        // letters.
        let found = found("go");
        assert_eq!(found[0], "goto");
        assert!(found.contains(&"global.toggle.hide"), "{found:?}");
    }

    #[test]
    fn letters_in_order_find_a_long_name() {
        assert_eq!(found("gfol")[0], "global.follow");
        assert_eq!(found("expdown")[0], "explorer.down");
    }

    #[test]
    fn the_shorter_span_comes_first() {
        // Both hold `vgs`; `view.goto.start` holds it in a shorter span than
        // any name where the letters are far apart.
        assert_eq!(found("vgs")[0], "view.goto.start");
    }

    #[test]
    fn the_help_text_is_searched_too() {
        assert_eq!(found("folder")[0], "e");
        assert_eq!(found("GROWS"), ["global.follow"]);
    }

    #[test]
    fn a_word_nothing_holds_matches_nothing() {
        assert_eq!(found("zzzz"), Vec::<&str>::new());
    }

    #[test]
    fn every_command_has_a_line_of_help() {
        for command in catalog() {
            assert!(!command.help.is_empty(), "{} has no help", command.names[0]);
        }
    }
}
