//! The command palette (#520): what a line typed after `:` runs.
//!
//! A line is a command name and, after the first blank, its argument. The
//! argument is the rest of the line as typed, so a path with a space in it
//! needs no quotes.

use super::App;
use super::prompt::{EMPTY_PATTERN, INVALID_PATTERN, PromptKind, SearchPrompt};
use crate::commands::{Argument, Run};
use crate::filter::Sense;
use crate::keymap::{ActionId, Key, Scope};
use crate::widgets::Focus;
use crate::widgets::explorer::Kind;
use crate::widgets::filterlist::Row;
use std::path::{Path, PathBuf};

/// What Tab chooses among in the palette, from the first Tab until the
/// next edit: the rows of the list, for a command name, or the entries of a
/// folder, for a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Completion {
    /// The line before the part a choice replaces.
    base: String,
    /// What each choice writes after `base`.
    options: Vec<String>,
    /// The choice the line shows now.
    at: usize,
    /// The word the list matched, when the choice is among its rows: the
    /// list keeps showing those rows while Tab steps through them.
    pub(super) word: Option<String>,
}

impl Completion {
    /// The list row the line shows now, when the choice is among the rows.
    pub(super) fn chosen_row(&self) -> Option<usize> {
        self.word.is_some().then_some(self.at)
    }

    /// The line the choice makes.
    fn line(&self) -> String {
        format!("{}{}", self.base, self.options[self.at])
    }

    /// Step to the next choice, or the one before, round the end.
    fn step(&mut self, backwards: bool) {
        let count = self.options.len();
        self.at = if backwards {
            (self.at + count - 1) % count
        } else {
            (self.at + 1) % count
        };
    }
}

impl App<'_> {
    /// The open prompt, when it is the command palette.
    pub(super) fn palette(&self) -> Option<&SearchPrompt> {
        self.prompt
            .as_ref()
            .filter(|prompt| prompt.kind == PromptKind::Command)
    }

    /// Tab (or Shift-Tab, `backwards`) in the palette: step to the next (or
    /// previous) choice when Tab already chose one; otherwise start. For the
    /// command name, the choices are the rows of the list. For a path, the
    /// entries of its folder that start with what is typed: one fits and it
    /// is written whole, a directory with its `/`; more fit and the part
    /// they share is written, and only a Tab that cannot write more steps
    /// through them. Anything else, and every other prompt, is left as it
    /// is.
    pub(super) fn complete(&mut self, backwards: bool) {
        let Some(prompt) = self.palette() else {
            return;
        };
        let line = prompt.pattern.clone();
        let started = prompt.completion.is_some();
        let completion = if started {
            None
        } else {
            match self.completion_for(&line) {
                Ok(completion) => Some(completion),
                // One choice, or more that share more than was typed: the
                // line is written, and nothing is left to step through.
                Err(Some(written)) => {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.error = None;
                        prompt.set_pattern(written);
                    }
                    return;
                }
                Err(None) => return,
            }
        };
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        if let Some(mut completion) = completion {
            if backwards {
                completion.at = completion.options.len() - 1;
            }
            prompt.completion = Some(completion);
        } else if let Some(completion) = prompt.completion.as_mut() {
            completion.step(backwards);
        }
        if let Some(line) = prompt.completion.as_ref().map(Completion::line) {
            prompt.error = None;
            prompt.set_pattern(line);
        }
    }

    /// What a first Tab on `line` chooses among: `Ok` with two or more
    /// choices to step through, or `Err` with the line to write at once —
    /// `None` when there is nothing to complete.
    fn completion_for(&self, line: &str) -> Result<Completion, Option<String>> {
        let Some((name, _)) = line.split_once(char::is_whitespace) else {
            let options: Vec<String> = crate::commands::matches(&self.commands, line)
                .into_iter()
                .map(|index| {
                    let command = &self.commands[index];
                    let space = if command.argument == Argument::None {
                        ""
                    } else {
                        " "
                    };
                    format!("{}{space}", command.names[0])
                })
                .collect();
            if options.is_empty() {
                return Err(None);
            }
            return Ok(Completion {
                base: String::new(),
                options,
                at: 0,
                word: Some(line.to_string()),
            });
        };
        let command = crate::commands::find(&self.commands, name);
        let mut argument = line[name.len()..].trim_start();
        if let Some(command) = command {
            argument = command.completed_part(argument).ok_or(None)?;
        }
        let base = &line[..line.len() - argument.len()];
        let takes = command.map(|command| command.argument);
        let (base, partial, options) = match (takes, argument.split_once(':')) {
            (Some(Argument::Path), _) => self.path_choices(base, argument)?,
            (Some(Argument::View), _) => (
                base.to_string(),
                argument,
                self.loading.view_names(argument),
            ),
            (Some(Argument::Set | Argument::SetList), None) => {
                (base.to_string(), argument, self.set_names(argument))
            }
            (Some(Argument::SetList), Some((set, list))) => {
                let done = list.rfind(',').map_or(0, |comma| comma + 1);
                let partial = &list[done..];
                (
                    format!("{base}{set}:{}", &list[..done]),
                    partial,
                    self.set_items(set, partial),
                )
            }
            _ => return Err(None),
        };
        let shared = shared_prefix(&options);
        if options.is_empty() {
            return Err(None);
        }
        if options.len() == 1 || shared.len() > partial.len() {
            return Err(Some(format!("{base}{shared}")));
        }
        Ok(Completion {
            base,
            options,
            at: 0,
            word: None,
        })
    }

    /// What Tab chooses among for the path `argument`, after `base`: the
    /// line before the part a choice replaces, that part, and the entries of
    /// its folder.
    fn path_choices<'a>(
        &self,
        base: &str,
        argument: &'a str,
    ) -> Result<(String, &'a str, Vec<String>), Option<String>> {
        let (folder, partial) = argument
            .rfind('/')
            .map_or(("", argument), |slash| argument.split_at(slash + 1));
        // No folder typed is the explorer's, not the home folder that `e`
        // alone goes to.
        let dir = if folder.is_empty() {
            self.explorer.dir().to_path_buf()
        } else {
            self.resolve_path(folder).map_err(|_| None)?
        };
        Ok((
            format!("{base}{folder}"),
            partial,
            entries_starting_with(&dir, partial),
        ))
    }

    /// The names of the known sets that start with `partial`, as the set
    /// picker sorts them.
    fn set_names(&self, partial: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .filters
            .named_sets()
            .iter()
            .filter(|meta| meta.name.starts_with(partial))
            .map(|meta| meta.name.clone())
            .collect();
        names.sort_unstable();
        names
    }

    /// The items of the set called `set` that start with `partial`: what a
    /// `set` list can name.
    fn set_items(&self, set: &str, partial: &str) -> Vec<String> {
        let Ok(index) = self.filters.named(set) else {
            return Vec::new();
        };
        self.filters.sets()[index]
            .list_items()
            .filter(|item| item.starts_with(partial))
            .cloned()
            .collect()
    }

    /// Run one command line from the palette. `Err` is the message the
    /// palette shows in place of the line; the palette stays open with it.
    pub(super) fn run_command(&mut self, line: &str) -> Result<(), String> {
        if line.trim().is_empty() {
            // Enter on an empty palette closes it, as Esc does.
            return Ok(());
        }
        let (command, argument) = crate::commands::resolve(&self.commands, line)?;
        match command.run {
            Run::Path => self.go_to_path(argument),
            Run::SetExplorer => self.set_explorer(argument),
            Run::Line => self.go_to_line(argument),
            Run::EnableSet => self.enable_set(argument),
            Run::ListSet => self.set_listing("list", argument, true),
            Run::UnlistSet => self.set_listing("unlist", argument, false),
            Run::View => self.choose_view(argument),
            Run::AddFilter(sense) => self.add_filter_from_palette(sense, argument),
            Run::Action(action) => self.run_action(action, command.argument, argument),
            Run::Alias => {
                let (alias, chain) = (command.names[0].to_string(), command.help.to_string());
                self.run_alias(&alias, &chain, argument)
            }
        }
    }

    /// The key of a user alias (#559): run it, as its name in the palette
    /// does. A key cannot give arguments, so an alias that takes them opens
    /// the palette with its name typed instead, and a chain that fails
    /// opens it with the error.
    pub(super) fn run_alias_key(&mut self, alias: &str) {
        let takes_arguments = crate::commands::find(&self.commands, alias)
            .is_some_and(|command| command.argument != Argument::None);
        if takes_arguments {
            self.prompt = Some(SearchPrompt::editing(
                format!("{alias} "),
                PromptKind::Command,
            ));
        } else if let Err(message) = self.run_command(alias) {
            let mut palette = SearchPrompt::editing(alias.to_string(), PromptKind::Command);
            palette.error = Some(message);
            self.prompt = Some(palette);
        }
    }

    /// A user alias (#557): run each command line of its chain in order, as
    /// if it was typed in the palette, with the `typed` arguments put in
    /// (#558). The first that fails stops the chain, and its error names
    /// the alias and the line; the lines before it stay done.
    fn run_alias(&mut self, alias: &str, chain: &str, typed: &str) -> Result<(), String> {
        let lines =
            crate::commands::expand(chain, typed).map_err(|err| format!("{alias}: {err}"))?;
        for segment in lines {
            self.run_command(&segment)
                .map_err(|err| format!("{alias}: `{segment}`: {err}"))?;
        }
        Ok(())
    }

    /// Run a keymap action by its name, on the pane its key works in: that
    /// pane shows and takes the focus first, so the action is seen. A global
    /// action works where the focus is, as its key does.
    fn run_action(
        &mut self,
        action: ActionId,
        takes: Argument,
        argument: &str,
    ) -> Result<(), String> {
        // `global.filters.toggle`, the one action that takes a number: its
        // key's digit, given as the argument instead.
        if takes == Argument::Number {
            let number = argument
                .parse()
                .map_err(|_| format!("{}: give a filter number", action.name()))?;
            self.toggle_numbered(number);
            return Ok(());
        }
        // `require <text>` (#548): from the filter the pane has selected,
        // with the text given. With no text it is `&`, below.
        if action == ActionId::FiltersRequire && !argument.is_empty() {
            let rows = crate::widgets::filterlist::rows(&self.filters);
            let selected = self.filters_pane.selected().and_then(|row| rows.get(row));
            let Some(Row::Filter(index) | Row::BuiltIn(index)) = selected.copied() else {
                return Err(format!("{}: select a filter first", action.name()));
            };
            self.reveal_and_focus(Focus::Filters);
            return self.require_from(index, Some(argument));
        }
        // No key fired it, and no other action reads the key.
        let pressed = Key {
            code: crossterm::event::KeyCode::Null,
            ctrl: false,
            alt: false,
        };
        let scopes = crate::keymap::scopes_of(action);
        let pane = if scopes.contains(&Scope::Global) {
            None
        } else if scopes.contains(&Scope::Explorer) {
            Some(Focus::Explorer)
        } else if scopes.contains(&Scope::Filters)
            && (self.focus == Focus::Filters || !scopes.contains(&Scope::View))
        {
            // `hit.next` has a row in the filter pane and in the view: it
            // stays in the filter pane when it is run there, as its key does.
            Some(Focus::Filters)
        } else {
            Some(Focus::View)
        };
        if let Some(pane) = pane {
            self.reveal_and_focus(pane);
        }
        match pane {
            None | Some(Focus::View) => self.perform(action, pressed),
            Some(Focus::Explorer) => self.perform_on_explorer(action),
            Some(Focus::Filters) => self.perform_on_filters(action, pressed),
        }
        Ok(())
    }

    /// `e` and its alias `cd`: `explorer.set`, then `explorer.focus`, so
    /// the change is seen.
    fn go_to_path(&mut self, argument: &str) -> Result<(), String> {
        self.set_explorer(argument)?;
        self.run_action(ActionId::ExplorerFocus, Argument::None, "")
    }

    /// `explorer.set` (#556): set the explorer's folder to `argument`, or
    /// to the folder of the file it names, which then opens — the same as
    /// the path on the shell command line. The focus stays where it is, and
    /// a hidden explorer stays hidden. The working directory of recon does
    /// not change.
    fn set_explorer(&mut self, argument: &str) -> Result<(), String> {
        let path = self.resolve_path(argument)?;
        if !path.exists() {
            return Err(format!("{argument}: no such file or directory"));
        }
        if let Err(err) = readable(&path) {
            return Err(format!("cannot read {argument}: {err}"));
        }
        let before = self.explorer.dir().to_path_buf();
        if let Some(action) = self.explorer.go_to(&path) {
            self.perform_widget_action(action);
        }
        if self.explorer.dir() != before {
            self.last_folder = Some(before);
        }
        Ok(())
    }

    /// `set [+]NAME[:ITEMS]`: list the set if it is unlisted and enable it,
    /// with its `default` profile or the items of the list — the same as
    /// `--set` (#441, #538). `+NAME` starts from the flags the pane shows
    /// now, and a list that changes nothing gets the same hint as at startup.
    /// The filter pane does not take the focus: the
    /// view shows the change. A name recon does not know changes nothing,
    /// not even a peek.
    fn enable_set(&mut self, argument: &str) -> Result<(), String> {
        let spec = crate::filter::SetSpec::parse(argument);
        nonempty("set", &spec.name)?;
        let (index, members) = self
            .filters
            .resolve_named(&spec)
            .map_err(|err| err.to_string())?;
        self.relist(|app| {
            app.filters.enable_resolved(index, members.as_deref());
            app.log_filters(format_args!("enable set {:?}", spec.name));
        });
        if let Some(hint) = spec.hint() {
            self.report(&hint, false);
        }
        Ok(())
    }

    /// `list NAME` and `unlist NAME`: list or unlist the set, as the set
    /// picker does. `unlist` is `--unlist`. `list` gives an unlisted set
    /// back disabled, as `--list` does (#545); a listed one does not
    /// change, so a set that is on stays on.
    fn set_listing(&mut self, command: &str, set: &str, listed: bool) -> Result<(), String> {
        let index = self
            .filters
            .named(nonempty(command, set)?)
            .map_err(|err| err.to_string())?;
        self.apply_listing(&[(index, listed)]);
        Ok(())
    }

    /// `include RE`, `exclude RE` and `context RE`: add a filter of `sense`
    /// to the scratch set, as `i` and Enter do in the filter pane (#527).
    /// An empty pattern is refused as the prompt refuses it (#347). The
    /// focus stays where it is: the view shows the change.
    fn add_filter_from_palette(&mut self, sense: Sense, pattern: &str) -> Result<(), String> {
        if pattern.is_empty() {
            return Err(EMPTY_PATTERN.to_string());
        }
        self.add_filter_as(pattern, sense)
            .map_err(|_| INVALID_PATTERN.to_string())
    }

    /// `goto <n>` and `:<n>`: put the cursor on line `n` of the file — the
    /// number the line numbers show, not a count of visible lines. A line a
    /// filter hides lands on the next visible line, and the status row says
    /// so; a number past the end lands on the last line, as in vim. The file
    /// view shows and takes the focus.
    fn go_to_line(&mut self, argument: &str) -> Result<(), String> {
        let number: usize = argument
            .parse()
            .map_err(|_| format!("not a line number: {argument}"))?;
        if self.view.has_image() || self.document.lines().is_empty() {
            return Err("this view has no lines".to_string());
        }
        self.promote_truncated_preview();
        let last = self.document.lines().len() - 1;
        let source = number.saturating_sub(1).min(last);
        let landed = self
            .document
            .nearest_visible(source)
            .ok_or_else(|| "no line is visible".to_string())?;
        let row = self
            .document
            .visible_position(landed)
            .ok_or_else(|| "no line is visible".to_string())?;
        self.reveal_and_focus(Focus::View);
        self.jump_to_visible_row(row);
        if landed != source {
            self.report(
                &format!("line {} is hidden; at line {}", source + 1, landed + 1),
                false,
            );
        }
        Ok(())
    }

    /// The path `argument` names: the home folder when it is empty or `~`,
    /// the last folder when it is `-`, and otherwise the path, a `~/` at its
    /// start read as the home folder and a relative one resolved against the
    /// explorer's folder.
    fn resolve_path(&self, argument: &str) -> Result<PathBuf, String> {
        let no_home = || "no home folder: HOME is not set".to_string();
        let path = match argument {
            "" => PathBuf::from(self.home.as_deref().ok_or_else(no_home)?),
            "-" => self
                .last_folder
                .clone()
                .ok_or_else(|| "no last folder yet".to_string())?,
            _ => {
                let path = crate::config::expand_tilde(Path::new(argument), self.home.as_deref());
                if path.starts_with("~") {
                    return Err(no_home());
                }
                self.explorer.dir().join(path)
            }
        };
        Ok(crate::path::lexical_absolute(&path))
    }
}

/// The names in `dir` that start with `partial`, in the explorer's order,
/// a directory's with a `/` after it. A name that starts with `.` only when
/// `partial` does, as a shell completes.
fn entries_starting_with(dir: &Path, partial: &str) -> Vec<String> {
    let Ok(entries) = crate::widgets::explorer::sorted_entries(dir) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry.name.to_str()?;
            if !name.starts_with(partial) || (name.starts_with('.') && !partial.starts_with('.')) {
                return None;
            }
            Some(if entry.kind == Kind::Dir {
                format!("{name}/")
            } else {
                name.to_string()
            })
        })
        .collect()
}

/// `set`, or the error of `command` given no set name.
fn nonempty<'a>(command: &str, set: &'a str) -> Result<&'a str, String> {
    if set.is_empty() {
        Err(format!("{command}: give a set name"))
    } else {
        Ok(set)
    }
}

/// The longest start every one of `options` has.
fn shared_prefix(options: &[String]) -> String {
    let Some(first) = options.first() else {
        return String::new();
    };
    let mut shared = first.as_str();
    for option in &options[1..] {
        let end = shared
            .char_indices()
            .zip(option.chars())
            .find(|((_, a), b)| a != b)
            .map_or(shared.len().min(option.len()), |((at, _), _)| at);
        shared = &shared[..end];
    }
    shared.to_string()
}

/// Whether recon can read `path`: list it, for a directory, or open it, for
/// a file.
fn readable(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::read_dir(path).map(drop)
    } else {
        // A FIFO's open waits for a writer: refused first, as a load does.
        crate::document::refuse_unreadable(path)?;
        std::fs::File::open(path).map(drop)
    }
}
