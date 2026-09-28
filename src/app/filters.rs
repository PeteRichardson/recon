//! Adding and editing filters, the hide mode, the peek, and the keys
//! the filter pane takes.

use super::App;
use super::prompt::{PromptKind, SearchPrompt};
use crate::document::Mode;
use crate::widgets::{self, FilterCommand};
use crate::{filter, filtersets};
use color_eyre::Result;
use crossterm::event::{self, KeyCode};

/// What a peek has to put back when it ends (#48).
///
/// The mode *and* the filter flags, because `<space>` changes both: it is the
/// four-key "hide off, filters off, read, filters on, hide on" cycle from the
/// issue collapsed into one key and its undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PeekState {
    mode: Mode,
    flags: filter::EnabledFlags,
}

impl App<'_> {
    /// Add an including filter, colouring it distinctly from its predecessors.
    pub(super) fn add_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.add(pattern)?;
        self.refresh_view();
        Ok(())
    }

    /// Add an excluding filter: its matches leave the view entirely.
    pub(super) fn add_excluding_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.filters.add_excluding(pattern)?;
        self.refresh_view();
        Ok(())
    }

    /// Overwrite one filter's pattern, keeping its position — and with it the
    /// colour and the precedence that position decides.
    ///
    /// `refresh_view`'s full `Document::evaluate`, not `recompute_visible`:
    /// the pattern is what decides which lines match, so the cached verdicts
    /// are stale in a way only a re-evaluate can fix. Narrower than a delete —
    /// the numbering is untouched, so only *this* filter's verdicts can have
    /// changed — but `evaluate` is the only thing that recomputes any of them.
    ///
    /// A filter that has vanished is reported as `Ok` rather than an error:
    /// the pattern the user typed is fine, there is simply nothing left to put
    /// it on, and leaving the prompt open under `E486: invalid pattern` would
    /// blame the pattern for it. Unreachable today — see `PromptKind::Edit`.
    pub(super) fn replace_filter(
        &mut self,
        index: usize,
        pattern: &str,
    ) -> Result<(), regex::Error> {
        if self.filters.set_pattern(index, pattern)? {
            self.refresh_view();
        }
        Ok(())
    }

    /// The one place the mode is set. `Ctrl-H`/`H` is one key with one meaning
    /// in both panes: non-matching *lines* dim or hide in the view, and
    /// non-matching *files* dim or hide in the explorer (#119).
    pub(super) fn set_mode(&mut self, mode: Mode) {
        self.document.set_mode(mode);
        self.explorer.set_mode(mode);
    }

    /// Flip between dimming unmatched lines and hiding them.
    ///
    /// Unlike a filter change, this always rebuilds the buffer: which rows
    /// are visible necessarily changes (that is the point of the toggle), so
    /// there is no "nothing changed" case to guard `refresh_view` against
    /// here the way `add_filter` needs. `apply_view` still holds the
    /// cursor's screen row across that rebuild, the same as any other
    /// caller, so the toggle does not re-anchor the view even though it
    /// always rebuilds. It calls `recompute_visible` rather than
    /// `refresh_view`'s full `evaluate`, though: the mode is the only thing
    /// that changed, and no verdict can be different, so redoing the whole
    /// filter pass would be pure waste on a large document.
    pub(super) fn toggle_hiding(&mut self) {
        let mode = match self.document.mode() {
            Mode::Dimmed => Mode::FilteredOnly,
            Mode::FilteredOnly => Mode::Dimmed,
        };
        let cursor_source = self.cursor_source();
        self.set_mode(mode);
        self.document.recompute_visible();
        self.apply_view(cursor_source);
    }

    /// `<space>`: show the plain file, or put the filtered view back (#48).
    ///
    /// The issue's complaint is a four-key cycle — leave hide mode, clear the
    /// filters, read the code, then undo both — repeated at every match. This
    /// is that cycle as one key and its own undo.
    ///
    /// **A flip, not a destination** (#65). The mode toggles, which is what #48
    /// asked for in as many words; ending the peek restores what was captured.
    ///
    /// This was originally written the other way — forcing `Mode::Dimmed` —
    /// on the premise that flipping *into* `FilteredOnly` with every filter
    /// just disabled would show only `Included` lines and blank the pane. That
    /// premise was already false when it was written: `recompute_visible`'s #36
    /// guard makes `FilteredOnly` show the whole file when nothing is
    /// including. Recorded because the mistake is easy to make twice, and the
    /// arm below looks wrong until you know about the guard.
    ///
    /// It rests on what hide mode *means*, which is not "hide every unmatched
    /// line" but:
    ///
    /// > if something is including, hide unmatched lines; if nothing is, show
    /// > everything.
    ///
    /// So hiding is a standing preference — armed or not — rather than a
    /// description of what is currently on screen. That is why the ` HIDE `
    /// badge appearing over a plain, unfiltered file is honest rather than a
    /// lie: see `HIDE_BADGE_TEXT`, whose doc already says *armed*.
    ///
    /// The rendered lines are identical either way, which is precisely why the
    /// original deviation from #48 went unnoticed for so long. The badge is the
    /// only visible difference.
    ///
    /// The capture is held here rather than in `ActiveFilters::remembered`,
    /// which `!` owns — see `enabled_flags` for why sharing one slot loses the
    /// other feature's undo.
    pub(super) fn toggle_peek(&mut self) {
        if let Some(peek) = self.peek.take() {
            self.filters.apply_enabled_flags(&peek.flags);
            self.set_mode(peek.mode);
        } else {
            self.peek = Some(PeekState {
                mode: self.document.mode(),
                flags: self.filters.enabled_flags(),
            });
            self.filters.set_all_enabled(false);
            // The same flip `toggle_hiding` does, deliberately: `<space>`
            // and `Ctrl-H` move the mode identically, and only the filter
            // switching below is the peek's own.
            self.set_mode(match self.document.mode() {
                Mode::Dimmed => Mode::FilteredOnly,
                Mode::FilteredOnly => Mode::Dimmed,
            });
        }
        // The full `evaluate`, not `recompute_visible` as `toggle_hiding` uses:
        // the enabled flags changed, so every line's verdict can differ. The
        // mode moved too, which `refresh_view` picks up on the same pass.
        self.refresh_view();
    }

    /// Put the filters back before a jump that leaves the peeked file.
    ///
    /// The peek disabled every filter, and the scan that answers "which
    /// files match" was told so: every explorer entry is `Match::Unknown`
    /// until `refresh_scan` runs again — which is normally after this
    /// keypress is dispatched, too late for a cross-file step made now. So
    /// the scan is refreshed here, and the answers come straight back from
    /// the scan cache for every file that has not changed on disk.
    ///
    /// This call only gets past `refresh_scan`'s "state unchanged" guard
    /// because of one invariant: `refresh_scan` records `None` as the last
    /// scan state whenever `self.filters.matcher()` is `None`, which is
    /// exactly what the peek forces by disabling every filter. Calling
    /// `toggle_peek` just above turns `matcher()` from `None` back into
    /// `Some(_)`, so the state computed here differs from the one recorded
    /// while peeked and the guard lets the scan through. Without that
    /// difference `refresh_scan(false)` would be a no-op and the explorer's
    /// answers would still read `Match::Unknown` for this step.
    pub(super) fn restore_peek_before_moving(&mut self) {
        if self.peek.is_none() {
            return;
        }
        self.toggle_peek();
        self.refresh_scan(false);
    }

    /// Whether the file view is showing a bounded preview rather than the
    /// whole file.
    pub(super) fn file_view_truncated(&self) -> bool {
        self.view.is_truncated()
    }

    /// `S`: write the scratch set to `filters.toml` as `name`, then adopt it
    /// in memory so the pane shows what a restart would show — without
    /// discarding any other set's current state (#131).
    ///
    /// The written text is re-parsed before anything is written or changed,
    /// so a file recon could not load back is never produced. Errors are
    /// messages for the prompt's error line; the scratch set is untouched
    /// on every one of them.
    pub(super) fn save_scratch_as(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a set needs a name".into());
        }
        if self.filters.sets().iter().any(|set| set.name == name) {
            return Err(format!(
                "a set named {name:?} already exists; edit filters.toml to change it"
            ));
        }
        let Some(path) = self.save_path.clone() else {
            return Err("no config home ($XDG_CONFIG_HOME, $HOME unset); nowhere to save".into());
        };
        // Two scratch filters that answer to one name would be two file
        // filters answering to the same name, which `parse` rejects with
        // advice about a `name` key. Say it in the pane's terms instead
        // (#190): a filter with no name answers to its pattern.
        let mut names: Vec<(String, bool)> = self
            .filters
            .filters_in(0)
            .map(|(_, filter)| (filter.display_name(), filter.name.is_some()))
            .collect();
        names.sort_unstable();
        if let Some(pair) = names.windows(2).find(|pair| pair[0].0 == pair[1].0) {
            let shared = &pair[0].0;
            return Err(if pair[0].1 || pair[1].1 {
                format!("two scratch filters share the name {shared:?}; change one before saving")
            } else {
                format!(
                    "two scratch filters share the pattern {shared:?}; delete one before saving"
                )
            });
        }
        let to_save = filtersets::SetToSave {
            name,
            filters: self
                .filters
                .filters_in(0)
                .map(|(_, filter)| filtersets::FilterToSave {
                    name: filter.name.clone(),
                    description: filter.description.clone(),
                    prompt: filter.prompt.clone(),
                    examples: filter.examples.clone(),
                    ..filtersets::FilterToSave::new(filter.predicate.display(), filter.sense)
                })
                .collect(),
            default: self
                .filters
                .filters_in(0)
                .filter(|(_, filter)| filter.enabled)
                .map(|(_, filter)| filter.display_name())
                .collect(),
        };
        let before = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(format!("could not read {}: {err}", path.display())),
        };
        let after = filtersets::append_set(&before, &to_save)?;
        filtersets::parse(&after, &path).map_err(|err| err.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
        }
        // Write beside the file and rename over it (#153): `fs::write`
        // truncates first, so a crash, a `kill` or a full disk between the
        // truncate and the write would leave the user's hand-edited file
        // empty or partial, and the next start refuses to run on it. The
        // rename is atomic on every filesystem recon runs on, so the file is
        // always either the old text or the new.
        let file_name = path
            .file_name()
            .map_or_else(|| "filters.toml".into(), std::ffi::OsStr::to_os_string);
        let mut tmp_name = file_name;
        tmp_name.push(".tmp");
        let tmp = path.with_file_name(tmp_name);
        std::fs::write(&tmp, after)
            .map_err(|err| format!("could not write {}: {err}", tmp.display()))?;
        if let Err(err) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("could not replace {}: {err}", path.display()));
        }
        self.filters.adopt_scratch_as(name, path);
        self.refresh_view();
        self.report(&format!("saved set {name:?}"), false);
        Ok(())
    }

    /// Handle a key aimed at the filter pane.
    ///
    /// This borrows the pane and the `ActiveFilters` together — something
    /// neither `FilterList` nor `Action` can do on their own, since the pane
    /// only ever borrows the set to render it — applies whatever command the
    /// pane reports, and re-evaluates. A delete renumbers the remaining
    /// filters, so `refresh_view`'s full `Document::evaluate` is required
    /// here: every cached `Verdict::Included` is a positional index that a
    /// patch would leave stale.
    ///
    /// The `Edit` command is the exception and returns before that
    /// re-evaluate: it only opens a prompt, and nothing about the set changes
    /// until it commits — at which point `replace_filter` does the
    /// re-evaluating instead.
    ///
    /// `Scope::Filters` resolves here rather than through `perform`: `i`,
    /// `x` and `S` open a prompt, which only `App` owns, so they are carried
    /// out directly below; everything else is delegated to
    /// `FilterList::perform`, which only reports a command — only `App` can
    /// mutate the `ActiveFilters` the command names.
    pub(super) fn handle_filter_key(&mut self, key: event::KeyEvent) {
        use crate::keymap::ActionId as A;

        // The explorer's `h`/`l` in this pane: a hint, not a redirect, for
        // the same reason as the filter verbs elsewhere (#120 §9). The keys
        // themselves have no `ActionId` in `Scope::Filters` — `FilterList`
        // cannot report, since the status row is `App`'s — so this stays a
        // pre-resolution special case, guarded on an empty modifier set as
        // before; but the text names the explorer actions they point at, so it
        // is generated from the table (task 8) rather than hand-written.
        if key.modifiers.is_empty() {
            match key.code {
                KeyCode::Char('h') => {
                    if let Some(hint) = self.keymap.hint_for(
                        A::ExplorerParent,
                        "goes up a directory",
                        A::GlobalFocusExplorer,
                    ) {
                        self.report(&hint, false);
                    }
                    return;
                }
                KeyCode::Char('l') => {
                    if let Some(hint) = self.keymap.hint_for(
                        A::ExplorerOpen,
                        "opens the entry",
                        A::GlobalFocusExplorer,
                    ) {
                        self.report(&hint, false);
                    }
                    return;
                }
                _ => {}
            }
        }

        let pressed = crate::keymap::normalise(key);
        let Some(action) = self.keymap.resolve(crate::keymap::Scope::Filters, pressed) else {
            return;
        };

        match action {
            // `i` and `x` open a prompt, and `self.prompt` is `App`'s —
            // `FilterList` cannot carry these out itself. Deliberately not
            // `FilterCommand` variants: that enum describes mutations of the
            // `ActiveFilters`, and opening a prompt is not one — see its doc
            // comment in `widgets/mod.rs`.
            A::FiltersInclude => self.prompt = Some(SearchPrompt::new(PromptKind::Filter)),
            A::FiltersExclude => self.prompt = Some(SearchPrompt::new(PromptKind::Exclude)),
            // `I` opens the filter editor (#312), which is `App`'s for the
            // same reason a prompt is.
            A::FiltersEditorNew => self.open_filter_editor(),
            // `S` saves the scratch set (#131). Refused before the prompt
            // opens when there is nothing to save: a prompt for a name that
            // can go nowhere is worse than a message.
            A::FiltersSaveSet => {
                if self.filters.filters_in(0).next().is_none() {
                    self.report("nothing to save: the scratch set is empty", false);
                    return;
                }
                self.prompt = Some(SearchPrompt::new(PromptKind::SaveSet));
            }
            // Bound the same way in the file view (`Scope::View`); only
            // `App` can see the document, so this makes the same call the
            // view's arm in `perform` makes, rather than delegating to
            // `FilterList`, which has no "next" of its own.
            A::HitNext | A::HitPrev => self.perform(action, pressed),
            _ => {
                let rows = widgets::filterlist::rows(&self.filters);
                if let Some(command) = self.filters_pane.perform(action, &rows) {
                    self.apply_filter_command(command);
                }
            }
        }
    }

    /// Carry out a command the filter pane reported, from a key or a click
    /// (#58). The pane only borrows the `ActiveFilters` it draws, so this is
    /// where every mutation it asks for actually happens.
    pub(super) fn apply_filter_command(&mut self, command: FilterCommand) {
        match command {
            FilterCommand::Toggle(index) => {
                self.filters.toggle_enabled(index);
            }
            FilterCommand::Delete(index) => {
                self.filters.remove(index);
            }
            FilterCommand::ToggleContext(index) => {
                self.filters.toggle_context(index);
            }
            FilterCommand::ToggleSet(set) => {
                self.filters.toggle_set(set);
            }
            FilterCommand::Solo(set) => {
                self.filters.solo(set);
            }
            FilterCommand::Reset => {
                self.filters.reset();
            }
            // Opens the picker, or says why not; the set is untouched until
            // a profile is chosen, so nothing to re-evaluate here.
            FilterCommand::PickProfile(set) => {
                let names: Vec<String> =
                    self.filters.sets()[set].profiles.keys().cloned().collect();
                if names.is_empty() {
                    self.report("no profiles in this set", false);
                } else {
                    self.picker = Some(widgets::picker::ProfilePicker::new(set, names));
                }
                return;
            }
            FilterCommand::EditInEditor(index) => {
                self.open_filter_editor_on(index);
                return;
            }
            FilterCommand::BuiltInIsReadOnly => {
                self.report(
                    "built-in filters can be switched off or collapsed, not deleted or edited",
                    false,
                );
                return;
            }
            // Nothing to re-evaluate: the model did not change.
            FilterCommand::SetIsReadOnly => {
                self.report(
                    "sets are defined in filters.toml; edit the file to change one",
                    false,
                );
                return;
            }
            // The one command that changes nothing yet — it opens a prompt,
            // and the set is only touched if it commits. It returns early
            // rather than falling through to the `refresh_view` below: there
            // is nothing to re-evaluate, and `evaluate` is O(lines × filters).
            FilterCommand::Edit(index) => {
                // The row the pane reported is one it drew, so the filter is
                // there; falling out silently rather than indexing keeps that
                // a property of the pane's own bounds, not a promise this
                // function has to make.
                if let Some(filter) = self.filters.filters().get(index) {
                    self.prompt = Some(SearchPrompt::editing(
                        filter.predicate.display(),
                        PromptKind::Edit {
                            index,
                            sense: filter.sense,
                        },
                    ));
                }
                return;
            }
        }
        // Deleting the last filter used to collapse the pane, so focus had to
        // be pushed off it. The pane stays now, so focus stays too — moving it
        // would be a jump the user did not ask for, off a pane still on screen.
        self.refresh_view();
    }
}
