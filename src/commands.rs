//! The commands the palette lists (#520), and which of them a typed word
//! matches.
//!
//! A command that a key can run has one name, its keymap name; a short
//! name is only for a command that takes an argument, or for a well-known
//! vim alias, and it shares its command's row (ADR 0011). A user alias
//! from `[aliases]` is not a short name: it runs a chain of command lines,
//! and has a row of its own (#557).

use crate::filter::Sense;
use crate::keymap::{ActionId, Scope};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// One row of the palette's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Command {
    /// Every name that runs it, the main one first: `e` then `cd`.
    pub(crate) names: Vec<Cow<'static, str>>,
    /// What it takes after its name. For an alias, its last argument, for
    /// the label and Tab only: the commands of its chain parse it (#558).
    pub(crate) argument: Argument,
    /// Its line of help: the help overlay's text for an action, and the
    /// chain of an alias.
    pub(crate) help: Cow<'static, str>,
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
    /// Text that may be left out: a require row's extra text (#548).
    Text,
    /// A value an alias puts into its chain, of no kind Tab completes
    /// (#558).
    Value,
}

/// What running a command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Run {
    /// `e`: go to a folder or a file, and focus the explorer.
    Path,
    /// `explorer.set`: go to a folder or a file; the focus stays (#556).
    SetExplorer,
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
    /// A user alias: the command lines of its chain, which is its help, in
    /// order (#557).
    Alias,
    /// `w` and `w!`: write the visible lines to a file (#528). `true` for
    /// `w!`, which may replace a file.
    Write(bool),
}

impl Command {
    /// The keymap action it runs, or `None` for a command no key runs.
    pub(crate) fn action(&self) -> Option<ActionId> {
        match self.run {
            Run::Action(action) => Some(action),
            Run::Path
            | Run::SetExplorer
            | Run::Line
            | Run::EnableSet
            | Run::ListSet
            | Run::UnlistSet
            | Run::View
            | Run::AddFilter(_)
            | Run::Alias
            | Run::Write(_) => None,
        }
    }

    /// The one-word arguments of an alias before `argument` (#558): `$1`
    /// of `include $1 ; e $2`. 0 for every other row.
    fn lead(&self) -> usize {
        if self.run == Run::Alias {
            params(&self.help).lead()
        } else {
            0
        }
    }

    /// The part of `argument`, all of the line after the name, that Tab
    /// completes as `self.argument`: for an alias, the text after its
    /// one-word arguments (#558). `None` while they are not all typed.
    pub(crate) fn completed_part<'l>(&self, argument: &'l str) -> Option<&'l str> {
        let mut rest = argument;
        for _ in 0..self.lead() {
            rest = next_word(rest)?.1;
        }
        Some(rest)
    }

    /// The first column of its row: its names, then its argument.
    pub(crate) fn label(&self) -> String {
        let mut names = self.names.join(", ");
        names.extend((1..=self.lead()).map(|n| format!(" ${n}")));
        match self.argument {
            Argument::None => names,
            Argument::Path => format!("{names} <path>"),
            Argument::Number => format!("{names} <n>"),
            Argument::SetList => format!("{names} <set>[:<list>]"),
            Argument::Set => format!("{names} <set>"),
            Argument::View => format!("{names} <view>"),
            Argument::Pattern => format!("{names} <pattern>"),
            Argument::Text => format!("{names} [<text>]"),
            Argument::Value => format!("{names} <value>"),
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

/// The row of `commands` that `name` runs, by any of its names.
pub(crate) fn find<'a>(commands: &'a [Command], name: &str) -> Option<&'a Command> {
    commands
        .iter()
        .find(|command| command.names.iter().any(|n| n == name))
}

/// Whether `name` is `:<n>`, vim's form of `goto <n>`.
fn is_line_number(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

/// The row of `commands` that a command `line` runs, and its argument: the
/// name is the line up to the first blank, and the argument is the rest of
/// the line as typed, so a path with a space in it needs no quotes. A
/// number alone is `goto`, with the number as its argument.
pub(crate) fn resolve<'a, 'l>(
    commands: &'a [Command],
    line: &'l str,
) -> Result<(&'a Command, &'l str), String> {
    let line = line.trim();
    let (name, argument) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(name, rest)| (name, rest.trim()));
    if is_line_number(name) {
        let goto = find(commands, "goto").expect("goto is built in");
        return Ok((goto, name));
    }
    let command = find(commands, name).ok_or_else(|| format!("not a command: {name}"))?;
    Ok((command, argument))
}

/// Every command the palette runs: the built-in ones, then a row for each
/// of `aliases`, by name (#557). `load` has refused an alias the palette
/// cannot run.
pub(crate) fn with_aliases(aliases: &BTreeMap<String, String>) -> Vec<Command> {
    let mut commands = catalog().to_vec();
    commands.extend(aliases.iter().map(|(name, chain)| Command {
        names: vec![Cow::Owned(name.clone())],
        argument: last_argument(chain, params(chain)),
        help: Cow::Owned(chain.clone()),
        run: Run::Alias,
    }));
    commands
}

/// What the last argument of an alias's `chain` is (#558): the argument of
/// the command whose whole argument it is, so that Tab completes it as
/// that command does. `Value` when no command takes it whole, and `Text`
/// for a `$*` that no command takes whole, since it can be left out.
fn last_argument(chain: &str, params: Params) -> Argument {
    let Some(last) = params.last() else {
        return Argument::None;
    };
    let fallback = if params.star {
        Argument::Text
    } else {
        Argument::Value
    };
    segments(chain)
        .filter_map(|segment| resolve(catalog(), segment).ok())
        .find(|(_, argument)| pieces(argument).eq([last]))
        .map_or(fallback, |(command, _)| command.argument)
}

/// A built-in row.
fn row(names: &[&'static str], argument: Argument, help: &'static str, run: Run) -> Command {
    Command {
        names: names.iter().copied().map(Cow::Borrowed).collect(),
        argument,
        help: Cow::Borrowed(help),
        run,
    }
}

fn build() -> Vec<Command> {
    let mut commands = vec![
        row(
            &["e", "cd"],
            Argument::Path,
            "Go to a folder, or open a file in its folder",
            Run::Path,
        ),
        row(
            &["explorer.set"],
            Argument::Path,
            "Set the explorer's folder; the focus stays where it is",
            Run::SetExplorer,
        ),
        row(
            &["goto"],
            Argument::Number,
            "Go to line n of the file (also :<n>)",
            Run::Line,
        ),
        row(
            &["set"],
            Argument::SetList,
            "List and enable a filter set, as --set does",
            Run::EnableSet,
        ),
        row(
            &["list"],
            Argument::Set,
            "List a filter set and leave it off, as --list does",
            Run::ListSet,
        ),
        row(
            &["unlist"],
            Argument::Set,
            "Unlist a filter set, as --unlist does",
            Run::UnlistSet,
        ),
        row(
            &["view"],
            Argument::View,
            "Show the open file in a view, by name",
            Run::View,
        ),
        row(
            &["include"],
            Argument::Pattern,
            "Add an including filter to the scratch set",
            Run::AddFilter(Sense::Include),
        ),
        row(
            &["exclude"],
            Argument::Pattern,
            "Add an excluding filter to the scratch set",
            Run::AddFilter(Sense::Exclude),
        ),
        row(
            &["context"],
            Argument::Pattern,
            "Add a context filter to the scratch set",
            Run::AddFilter(Sense::Context),
        ),
        row(
            &["w"],
            Argument::Path,
            "Write the visible lines to a new file",
            Run::Write(false),
        ),
        row(
            &["w!"],
            Argument::Path,
            "Write the visible lines to a file, and replace it if it exists",
            Run::Write(true),
        ),
    ];
    for action in crate::keymap::every_action().filter(|a| is_command(*a)) {
        let mut names = vec![action.name()];
        // vim's alias, on the row of the action it names (ADR 0011).
        if action == ActionId::GlobalQuit {
            names.push("q");
        }
        // The short name of a command that takes an argument (#548).
        if action == ActionId::FiltersRequire {
            names.push("require");
        }
        commands.push(row(
            &names,
            // The digit its key gives it.
            match action {
                ActionId::GlobalFiltersToggle => Argument::Number,
                ActionId::FiltersRequire => Argument::Text,
                _ => Argument::None,
            },
            help_for(action),
            Run::Action(action),
        ));
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

/// The command lines of an alias's `chain`, in the order they run: the
/// parts between its `;`s, trimmed.
pub(crate) fn segments(chain: &str) -> impl Iterator<Item = &str> {
    chain.split(';').map(str::trim)
}

/// The command lines an alias's `chain` runs when `typed` follows its name
/// (#558): each segment, with `$<n>` replaced by the n-th argument. The
/// arguments are the words of `typed`, and the last one gets all of the
/// rest of the line, blanks and all, as a command's argument does: `$*`
/// when the chain has it, else the highest `$<n>`. `Err` names the first
/// `$<n>` that `typed` leaves out, or the text an alias with no argument
/// was given.
pub(crate) fn expand(chain: &str, typed: &str) -> Result<Vec<String>, String> {
    let params = params(chain);
    let mut rest = typed.trim();
    let mut words = Vec::new();
    for n in 1..=params.lead() {
        let (word, after) = next_word(rest).unwrap_or((rest, ""));
        if word.is_empty() {
            return Err(format!("missing argument ${n}"));
        }
        words.push(word);
        rest = after;
    }
    if !params.star {
        if params.count > 0 {
            if rest.is_empty() {
                return Err(format!("missing argument ${}", params.count));
            }
            words.push(rest);
        } else if !rest.is_empty() {
            return Err(format!("takes no argument, not `{rest}`"));
        }
    }
    Ok(segments(chain)
        .map(|segment| {
            pieces(segment)
                .map(|piece| match piece {
                    Piece::Text(text) => text,
                    Piece::Arg(n) => words[n - 1],
                    Piece::Star => rest,
                })
                .collect()
        })
        .collect())
}

/// The word at the start of `line`, and the rest of the line after its
/// blanks: how an alias's arguments are split (#558). `None` when `line`
/// has no blank, so its last word is not done.
fn next_word(line: &str) -> Option<(&str, &str)> {
    line.split_once(char::is_whitespace)
        .map(|(word, rest)| (word, rest.trim_start()))
}

/// The arguments an alias's chain takes (#558): the highest `$<n>` in it,
/// each `$<n>` in it as bit n of `used`, and whether it has `$*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Params {
    count: usize,
    used: u16,
    star: bool,
}

impl Params {
    /// The one-word arguments before the last one, which gets the rest of
    /// the line.
    fn lead(self) -> usize {
        if self.star {
            self.count
        } else {
            self.count.saturating_sub(1)
        }
    }

    /// The argument that gets the rest of the line: `$*`, else the highest
    /// `$<n>`. `None` for a chain with no argument.
    fn last(self) -> Option<Piece<'static>> {
        match (self.count, self.star) {
            (0, false) => None,
            (_, true) => Some(Piece::Star),
            (n, false) => Some(Piece::Arg(n)),
        }
    }

    /// The first `$<n>` below the highest one that the chain does not use.
    fn skipped(self) -> Option<usize> {
        (1..self.count).find(|n| self.used & (1 << n) == 0)
    }
}

fn params(chain: &str) -> Params {
    let mut params = Params {
        count: 0,
        used: 0,
        star: false,
    };
    for piece in segments(chain).flat_map(pieces) {
        match piece {
            Piece::Arg(n) => {
                params.count = params.count.max(n);
                params.used |= 1 << n;
            }
            Piece::Star => params.star = true,
            Piece::Text(_) => {}
        }
    }
    params
}

/// A part of a segment of an alias's chain (#558).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece<'a> {
    /// Text that runs as it is. `$$` is the text `$`, and so is a `$`
    /// before any other character.
    Text(&'a str),
    /// `$1` to `$9`: that argument.
    Arg(usize),
    /// `$*`: all of the line after the `$<n>` arguments.
    Star,
}

/// The parts of `segment`, in order.
fn pieces(segment: &str) -> impl Iterator<Item = Piece<'_>> {
    let mut rest = segment;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let text = rest.find('$').unwrap_or(rest.len());
        if text > 0 {
            let (text, after) = rest.split_at(text);
            rest = after;
            return Some(Piece::Text(text));
        }
        let (piece, len) = match rest.as_bytes().get(1) {
            Some(b'$') => (Piece::Text("$"), 2),
            Some(b'*') => (Piece::Star, 2),
            Some(&digit @ b'1'..=b'9') => (Piece::Arg(usize::from(digit - b'0')), 2),
            _ => (Piece::Text("$"), 1),
        };
        rest = &rest[len..];
        Some(piece)
    })
}

/// Refuse the alias `name = chain` when the palette cannot run it (#557).
/// The name: one that is already a command's, which it would hide, a
/// number, which is `goto`, and one with a blank or a `;` in it, where the
/// palette ends a name or a chain. The chain: an empty command line, or one
/// that names no built-in command. An alias of `aliases` is not a built-in:
/// an alias cannot run another one yet. An argument in the place of a
/// command name is no built-in either, and a chain that skips a `$<n>`
/// below its highest one would lose that word (#558).
pub(crate) fn check_alias(
    name: &str,
    chain: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<(), String> {
    if name.is_empty() {
        return Err("an alias needs a name".to_string());
    }
    if is_line_number(name) {
        return Err("a number is not a name: `:<n>` goes to that line".to_string());
    }
    if name.contains(|c: char| c.is_whitespace() || c == ';') {
        return Err("a name cannot have a blank or a `;` in it".to_string());
    }
    if find(catalog(), name).is_some() {
        return Err("the name is already a command".to_string());
    }
    for segment in segments(chain) {
        if segment.is_empty() {
            return Err("an empty command between `;`s".to_string());
        }
        let first = segment.split_whitespace().next().unwrap_or_default();
        if aliases.contains_key(first) {
            return Err(format!(
                "names alias `{first}`; an alias cannot run another alias"
            ));
        }
        resolve(catalog(), segment).map_err(|_| format!("`{first}` is not a command"))?;
    }
    let params = params(chain);
    if let Some(n) = params.skipped() {
        return Err(format!("uses ${} but not ${n}", params.count));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The main names of the rows `word` matches, best first.
    fn found(word: &str) -> Vec<&'static str> {
        let commands = catalog();
        matches(commands, word)
            .into_iter()
            .map(|index| commands[index].names[0].as_ref())
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
        assert_eq!(commands[1].names, ["explorer.set"]);
        assert_eq!(commands[2].names, ["goto"]);
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

    /// `w!` is a row of its own, since it runs another command than `w`
    /// (#528); `write` finds both by their help.
    #[test]
    fn w_and_w_bang_are_rows_of_their_own() {
        assert_eq!(found("w")[..2], ["w", "w!"]);
        assert_eq!(found("w!"), ["w!"]);
        assert_eq!(found("write")[..2], ["w", "w!"]);
        let label = |name| find(catalog(), name).expect("listed").label();
        assert_eq!(label("w"), "w <path>");
        assert_eq!(label("w!"), "w! <path>");
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
