//! What each keymap action does, and what a widget's action does.

use super::prompt::SearchPrompt;
use super::search::Search;
use super::{App, AppState, EditorScope};
use crate::widgets::{self, Action, Focus};
use crossterm::event::{self, KeyCode, KeyModifiers};

impl App<'_> {
    /// Run a named action.
    ///
    /// One arm per action, and the only place an action's behaviour lives.
    /// Before this, the same behaviour was reachable from up to four `match`
    /// arms in different files, which is the drift #199 describes.
    ///
    /// `Scope::Global` and `Scope::View` resolve into this. `Explorer`, `Filters`,
    /// `Prompt` and `Picker` never do — each resolves at its own call site
    /// and is carried out there or by its widget's own `perform`, never
    /// through here. Every variant that can never reach this function is
    /// still listed explicitly rather than caught by a wildcard — see the
    /// comment on those two blocks.
    ///
    /// Takes the resolved `Key` alongside the `ActionId`, even though most
    /// arms ignore it: `GlobalFiltersToggle` needs the digit that fired it,
    /// which the action alone cannot carry, and plan 2b's rebinding makes
    /// this a general problem rather than a one-off — a future action can
    /// just as legitimately want to know which of several keys reached it.
    /// The key is always in hand at the call site, since resolution starts
    /// from one, so the parameter costs nothing there.
    pub(super) fn perform(&mut self, action: crate::keymap::ActionId, pressed: crate::keymap::Key) {
        use crate::keymap::ActionId as A;
        match action {
            // `q`'s `DEFAULT` row carries no modifier (see `label_matches`),
            // so a modified key — e.g. Ctrl-f, which the file view uses for
            // page-down — never resolves to this action and reaches the
            // focused widget instead.
            A::GlobalQuit => self.state = AppState::Quit { emit: true },
            // `Q` quits without emitting (#143).
            A::GlobalQuitSilent => self.state = AppState::Quit { emit: false },
            A::GlobalFocusNext => self.focus_next(),
            A::GlobalFocusPrev => self.focus_prev(),
            A::GlobalFocusExplorer => self.reveal_and_focus(Focus::Explorer),
            A::GlobalFocusView => self.reveal_and_focus(Focus::View),
            // A second `f` while the pane already has focus is the sticky
            // gesture: the user is staying, so no chain to return to.
            A::GlobalFocusFilters => self.start_filter_chain(),
            A::GlobalHideExplorer => self.hide_pane(Focus::Explorer),
            A::GlobalHideView => self.hide_pane(Focus::View),
            A::GlobalHideFilters => self.hide_pane(Focus::Filters),
            A::GlobalHelp => self.help = true,
            A::GlobalSets => {
                self.set_picker = Some(widgets::setpicker::SetPicker::new(self.filters.sets()));
            }
            // Every `/` captures its origin: the search moves as it is
            // typed in every pane, and Esc goes back to where it opened.
            A::GlobalSearch => {
                self.prompt = Some(SearchPrompt {
                    origin: Some(self.capture_origin()),
                    ..SearchPrompt::default()
                });
            }
            // `p` is the bridge from search to filter (ADR 0001): the pattern
            // becomes a numbered include filter in the scratch set, and the
            // search is cleared — the filter's colour replaces the highlight.
            // Nothing happens with no search set; `refresh_view` is not free
            // (`evaluate` is O(lines × filters)), so it is only paid for
            // when the set actually changed.
            //
            // `p`'s `DEFAULT` row carries no modifier, so Ctrl-P and Alt-P
            // are left unclaimed here and fall through to the focused
            // widget, like every other plain-letter global binding.
            A::GlobalSearchPromote => {
                if let Some(search) = self.search.take() {
                    // The pattern compiled as a search, so it compiles as a
                    // filter: the same regex crate, the same syntax.
                    if let Err(err) = self.add_filter(&search.text) {
                        log::warn!("cannot promote the search {:?}: {err}", search.text);
                    }
                }
            }
            A::GlobalSearchWord => {
                if self.focus == Focus::Explorer {
                    if let Some(hint) = self.keymap.hint_for(
                        A::GlobalSearchWord,
                        "searches the word under the cursor",
                        A::GlobalFocusView,
                    ) {
                        self.report(&hint, false);
                    }
                    return;
                }
                match self.word_under_cursor() {
                    // `/` with the typing done, then `n`: the word is on the
                    // cursor's own line by definition, so the scan `/` runs
                    // from that line would find it there and go nowhere.
                    // An escaped literal always compiles; a failure here
                    // would be a regex-crate bug, not user input.
                    Some(word) => match Search::new(&regex::escape(&word)) {
                        Ok(search) => {
                            self.search = Some(search);
                            self.step_hit(false);
                        }
                        Err(_) => self.report("could not search for that word", true),
                    },
                    None => self.report("no word under the cursor", false),
                }
            }
            // Visual mode's `Esc` takes precedence over the search layers
            // below (#67): a stray press should end the selection without
            // also dropping the search the selection was made under.
            A::GlobalEscape => {
                if self.focus == Focus::View && self.visual.is_some() {
                    self.end_visual();
                    return;
                }
                // Layered (#120 §8): the focused pane's own search first,
                // then the file search. Clearing the search clears the
                // highlight with it, which `apply_view` paints from
                // `self.search`; nothing is re-evaluated, since a search
                // never changed a verdict.
                if self.focus == Focus::Explorer && self.explorer.clear_search() {
                    return;
                }
                if self.search.take().is_some() {
                    self.repaint_highlight();
                }
            }
            A::GlobalPeek => self.toggle_peek(),
            A::GlobalFiltersAnd => {
                self.filters.toggle_and();
                self.refresh_view();
            }
            // Three states, because "nothing is enabled" and "nothing was
            // captured" are different situations. Branching on the capture
            // alone makes `!` inert once every filter has been disabled by
            // hand: it would capture all-disabled and then faithfully
            // restore it, forever.
            A::GlobalFiltersDisable => {
                if self.filters.any_enabled() {
                    self.filters.disable_all_remembering();
                } else if self.filters.has_remembered() {
                    self.filters.restore_remembered();
                } else {
                    self.filters.set_all_enabled(true);
                }
                self.refresh_view();
            }
            // Toggle a numbered filter from anywhere (#120 §14). The number
            // is the one the pane draws in its gutter, and `numbered` is the
            // walk that draws it, so key and label cannot disagree. Set
            // headers and built-in filters have no number
            // and no key: `f Enter` covers them.
            //
            // The one arm that reads `pressed`: `resolve` collapses every
            // `'1'..='9'` to this one action (see `DEFAULT`'s "1-9" row), so
            // the digit itself has to come from the key that fired it.
            A::GlobalFiltersToggle => {
                // `to_digit` (rather than `c as u8 - b'0'`) rejects a
                // non-digit outright instead of underflowing: `resolve`
                // today only ever matches this to a `Char('1'..='9')`, but
                // plan 2b lets a `config.toml` bind this action to any key —
                // `!`, say — and `'!' as u8 - b'0'` wraps to a huge `usize`
                // rather than failing loudly.
                let KeyCode::Char(c) = pressed.code else {
                    return;
                };
                let Some(n) = c.to_digit(10).filter(|n| (1..=9).contains(n)) else {
                    return;
                };
                let n = n as usize;
                match widgets::filterlist::numbered(&self.filters).get(n - 1) {
                    Some(&index) => {
                        self.filters.toggle_enabled(index);
                        self.refresh_view();
                    }
                    None => self.report(&format!("no filter {n}"), false),
                }
            }
            // Global, unlike `n`: skipping a file is the outer loop of the
            // review workflow, and it should not matter which pane the inner
            // loop left focus in (#120 §2).
            A::GlobalFileNext => self.skip_file(false),
            A::GlobalFilePrev => self.skip_file(true),
            A::GlobalToggleHide => self.toggle_hiding(),
            A::GlobalZoomView => self.zoom_file_view(),
            A::GlobalZoomFocused => self.zoom_focused(),
            A::GlobalEditorProject => {
                let template = self.editor.project.clone();
                self.open_in_editor(&template, EditorScope::Project);
            }
            A::GlobalEditorFile => {
                let template = self.editor.file.clone();
                self.open_in_editor(&template, EditorScope::File);
            }
            A::GlobalReload => {
                self.explorer.reload();
                // `r` is the one place the listing is stat'd on this thread:
                // it is asked for, and `explorer.reload` has just read the
                // directory here anyway. `refresh_scan` trusts its records
                // (#156), so without this a change would wait for the poll.
                self.check_stamps();
                self.refresh_scan(true);
                self.reload_active_file();
            }
            // `v`/`V` start, switch or end a selection; from any pane but the
            // view they hint instead, as `*` does from the explorer (#67,
            // #120 §9) — there is no cursor column there to anchor to.
            A::GlobalVisualChar | A::GlobalVisualLine => {
                if self.focus == Focus::View {
                    self.promote_truncated_preview();
                    self.toggle_visual(matches!(action, A::GlobalVisualLine));
                } else {
                    // Always `GlobalVisualChar`, not `action`: this hint is
                    // shared by both `v` and `V`, and it deliberately always
                    // names lowercase `v` (task 8 fix round 1, #199) — using
                    // `action` here would have `V` describe itself.
                    if let Some(hint) = self.keymap.hint_for(
                        A::GlobalVisualChar,
                        "selects text in the file view",
                        A::GlobalFocusView,
                    ) {
                        self.report(&hint, false);
                    }
                }
            }
            // `y`'s `DEFAULT` row carries no modifier, so Ctrl-y is left
            // unclaimed here and still reaches the file view's own
            // scroll-up binding.
            A::GlobalYank => {
                if self.focus == Focus::View {
                    self.yank();
                } else {
                    // The trailing key is `v` (`GlobalVisualChar`), not `y`:
                    // copying needs a selection first, made with `v`, so the
                    // hint says "focus the view, then press v" rather than
                    // "then press y" (task 8 fix round 1, #199).
                    if let Some(hint) = self.keymap.hint_for_trailing(
                        A::GlobalYank,
                        "copies a selection in the file view",
                        A::GlobalFocusView,
                        A::GlobalVisualChar,
                    ) {
                        self.report(&hint, false);
                    }
                }
            }
            // `[`/`]` are bound only in `Scope::Global` — there is no
            // `Scope::View` row for them — and Global resolves before any
            // focus check, so this arm fires from every pane, the file view
            // included (#120 §3). `ActionId` carries no key of its own to
            // forward, so this rebuilds the canonical key and hands it
            // straight to `FileView::handle_events`, bypassing `Scope::View`
            // entirely, where the widget's own raw `[`/`]` arms do the
            // actual scrolling.
            A::GlobalPageDown => self.forward_to_view(event::Event::Key(KeyCode::Char(']').into())),
            A::GlobalPageUp => self.forward_to_view(event::Event::Key(KeyCode::Char('[').into())),
            // The four long-range motions: only `App` can see the whole
            // document, so `long_range_target` answers "which visible row"
            // and this places the cursor there, promoting a truncated
            // preview first exactly as the old intercept did (#7, #250).
            A::ViewGotoStart | A::ViewGotoEnd | A::ViewParagraphNext | A::ViewParagraphPrev => {
                if let Some(target) = self.long_range_target(action) {
                    self.promote_truncated_preview();
                    self.jump_to_visible_row(target);
                }
            }
            // Bound the same way in the view and the filter pane; only `App`
            // can see the document, so `n`/`N` land here rather than in
            // either widget.
            A::HitNext => self.step_interesting(false),
            A::HitPrev => self.step_interesting(true),
            // Everything else in the view scope has a live `FileView` arm
            // already, so this rebuilds the canonical key for that binding
            // and forwards it — same shape as `GlobalPageDown`/`GlobalPageUp`
            // above, and for the same reason: an `ActionId` carries no key of
            // its own. Forwarding the canonical key rather than `pressed`
            // matters once plan 2b lets a user rebind these — a rebound key
            // would otherwise reach `FileView` with no arm that matches it.
            A::ViewLeft => self.forward_to_view(event::Event::Key(KeyCode::Char('h').into())),
            A::ViewRight => self.forward_to_view(event::Event::Key(KeyCode::Char('l').into())),
            A::ViewUp => self.forward_to_view(event::Event::Key(KeyCode::Char('k').into())),
            A::ViewDown => self.forward_to_view(event::Event::Key(KeyCode::Char('j').into())),
            A::ViewWordForward => {
                self.forward_to_view(event::Event::Key(KeyCode::Char('w').into()));
            }
            A::ViewLineStart => self.forward_to_view(event::Event::Key(KeyCode::Char('0').into())),
            A::ViewLineEnd => self.forward_to_view(event::Event::Key(KeyCode::Char('$').into())),
            A::ViewToggleLineNumbers => {
                self.forward_to_view(event::Event::Key(KeyCode::Char('#').into()));
            }
            A::ViewScrollDown => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('e'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewScrollUp => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('y'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewHalfPageDown => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewHalfPageUp => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('u'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewPageDown => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('f'),
                KeyModifiers::CONTROL,
            ))),
            A::ViewPageUp => self.forward_to_view(event::Event::Key(event::KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL,
            ))),
            // Not yet wired, listed rather than caught by a wildcard (#199):
            // a wildcard here would strip the exhaustiveness check this
            // match exists to keep, and plan 2b's rebinding can reach a
            // stray variant from a user's `config.toml`, not just from a
            // future arm nobody wrote. Each group below is deleted by the
            // task that gives it a real arm, so a variant left behind after
            // that task lands is a build error rather than a silent no-op.
            //
            // Task 6 gave the explorer and filter-pane scopes their real
            // arms, but neither lives here: `Scope::Explorer` resolves at the
            // per-focus dispatch and is carried out by `Explorer::perform`,
            // and `Scope::Filters` resolves inside `handle_filter_key` and
            // is carried out there or by `FilterList::perform`. Neither call
            // site routes its result through this function, so no code path
            // today produces one of these variants here.
            //
            // Not `unreachable!()`, though: plan 2b lets a user's
            // `config.toml` bind a key in `Scope::Global` to one of these
            // actions, and `resolve(Scope::Global, key)` would then hand it
            // straight to this function — a typo in a config file, not a
            // programmer error, so it must not crash the TUI. `debug_assert!`
            // still catches a programmer error loudly in tests and debug
            // builds; a release binary just does nothing.
            A::ExplorerUp
            | A::ExplorerDown
            | A::ExplorerParent
            | A::ExplorerOpen
            | A::ExplorerGotoStart
            | A::ExplorerGotoEnd
            | A::ExplorerHalfPageDown
            | A::ExplorerHalfPageUp
            | A::ExplorerPageDown
            | A::ExplorerPageUp
            | A::ExplorerHitNext
            | A::ExplorerHitPrev
            | A::FiltersUp
            | A::FiltersDown
            | A::FiltersGotoStart
            | A::FiltersGotoEnd
            | A::FiltersHalfPageDown
            | A::FiltersHalfPageUp
            | A::FiltersPageDown
            | A::FiltersPageUp
            | A::FiltersToggle
            | A::FiltersInclude
            | A::FiltersExclude
            | A::FiltersEdit
            | A::FiltersDelete
            | A::FiltersContext
            | A::FiltersProfile
            | A::FiltersSolo
            | A::FiltersReset
            | A::FiltersSaveSet
            | A::FiltersEditorNew
            | A::FiltersEditorOpen => {
                debug_assert!(
                    false,
                    "{action:?} resolves against its own widget, never through `perform`"
                );
            }
            // The modal scopes (task 7, #199): `Scope::Prompt` resolves in
            // `handle_search_key`, `Scope::FilterEditor` in
            // `handle_filter_editor_key`, and `Scope::Picker` resolves in
            // `dispatch_event`'s picker guard, and each is carried out right
            // there — neither ever reaches this function, for the reason
            // Ruling 30 gives: a modal's `None` case must swallow the key
            // and return, not fall through to `Global` the way every pane
            // scope does, so there is no path from a modal scope into
            // `perform` for a fallthrough to take. Same `debug_assert!` as
            // the group above, and the same reason it exists: plan 2b's
            // `config.toml` can still bind a `Scope::Global` key to one of
            // these actions, and `resolve(Scope::Global, key)` would hand it
            // straight here. The help overlay dismisses on any key and has
            // no `ActionId` of its own, so it has no arm here at all.
            A::PromptCommit
            | A::PromptCancel
            | A::PromptLeft
            | A::PromptRight
            | A::PromptStart
            | A::PromptEnd
            | A::PromptDeleteBack
            | A::PromptDeleteForward
            | A::PromptDeleteWord
            | A::PromptDeleteStart
            | A::PromptHistoryPrev
            | A::PromptHistoryNext
            | A::PickerUp
            | A::PickerDown
            | A::PickerChoose
            | A::PickerCancel
            | A::SetsUp
            | A::SetsDown
            | A::SetsToggle
            | A::SetsApply
            | A::SetsCancel
            | A::SetsSearch
            | A::SetsHitNext
            | A::SetsHitPrev
            | A::FilterEditorCommit
            | A::FilterEditorCancel
            | A::FilterEditorScrollUp
            | A::FilterEditorScrollDown
            | A::FilterEditorPageUp
            | A::FilterEditorPageDown
            | A::FilterEditorFocus
            | A::FilterEditorMarkMatch
            | A::FilterEditorMarkNoMatch
            | A::FilterEditorMarkClear
            | A::FilterEditorVisualLine => {
                debug_assert!(
                    false,
                    "{action:?} resolves in its own modal dispatch, never through `perform`"
                );
            }
        }
    }

    /// Carry out an action on behalf of the widget that raised it.
    ///
    /// Named for the widget rather than plain `perform` (#199): that name now
    /// belongs to the keymap table's own dispatcher below, which runs a
    /// `keymap::ActionId` rather than one of these.
    pub(super) fn perform_widget_action(&mut self, action: Action) {
        match &action {
            Action::Load(path) => self.view.load(path),
            Action::LoadAndFocus(path) => {
                self.view.load(path);
                // `reveal_and_focus`, not `self.focus = ...`: `b` and `z` can
                // leave the file view off screen, and opening a file into a
                // pane the user cannot see would be worse than not moving at
                // all. This is the same reason the focus keys route here.
                self.reveal_and_focus(Focus::View);
            }
            Action::Preview(path) => self.view.preview(path),
        }
        self.sync_document();
        self.refresh_view();
    }
}
