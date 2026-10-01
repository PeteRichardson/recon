//! The event loop's input side: read an event, and send it where it goes.

use super::App;
use super::prompt::{Origin, SearchPrompt, SetsOrigin};
use crate::widgets::{self, Focus};
use color_eyre::Result;
use crossterm::event::{self, KeyCode};
use std::time::Duration;

impl App<'_> {
    /// Handle any events that have occurred since the last time the app was
    /// rendered, and say whether anything changed.
    ///
    /// Still `Result`, unlike the rest of the chain: `event::poll` and
    /// `event::read` are genuine I/O and can genuinely fail. That is the whole
    /// distinction #80 draws — a `Result` here means something, precisely
    /// because the four functions below it no longer carry one they never use.
    ///
    /// The 1/60 s timeout stays. It is now a *wake* interval rather than a
    /// frame interval: the loop still comes back 60 times a second to check
    /// the editor channel, and draws only if it found something.
    pub(super) fn handle_events(&mut self) -> Result<bool> {
        // Here rather than in `handle_event`: an editor exits on its own
        // schedule, so nothing the user does is guaranteed to arrive after it.
        let drained = self.drain_editor_outcomes()
            | self.drain_scan_results()
            | self.poll_stamps()
            | self.drain_request()
            | self.poll_finish();
        let timeout = Duration::from_secs_f32(1.0 / 60.0);
        if event::poll(timeout)? {
            let event = event::read()?;
            self.handle_event(event);
            return Ok(true);
        }
        Ok(drained)
    }

    /// Dispatch a single event, then let the explorer's scan catch up.
    ///
    /// Split out from the polling loop so that it can be driven directly. The
    /// one thing added around `dispatch_event` is `refresh_scan`: the dispatch
    /// has two dozen early returns, and the scan guard has to run after every
    /// one of them.
    pub fn handle_event(&mut self, event: event::Event) {
        self.dispatch_event(event);
        // Here, beside `refresh_scan`, for the same reason: the dispatch has
        // two dozen early returns and every one of them may have moved
        // focus (#67).
        self.drop_visual_outside_the_view();
        self.refresh_scan(false);
    }

    /// Dispatch a single event: app-wide keys first, then the focused widget.
    ///
    /// Returns nothing. Dispatch cannot fail: every arm either mutates `App`
    /// or hands the event to a pane that also cannot fail. It used to return
    /// `Result<()>`, which cost a `?` at both call sites and an `.unwrap()` in
    /// roughly a hundred tests, and taught a reader to skim past `?` in a file
    /// where `Config::load`, `editor_command` and `set_highlight` genuinely can
    /// fail (#80).
    pub(super) fn dispatch_event(&mut self, event: event::Event) {
        // First of all the guards. The panel is up before any key is read, so
        // it can never meet an open prompt, and taking the key here is what
        // stops the dismissing key from also quitting, moving a cursor or
        // opening an editor.
        //
        // Only a *key* closes it, for the reason the help overlay gives: mouse
        // capture is on, and a mouse crossing the terminal would wipe a notice
        // the user is still reading.
        // While `q` finishes the work `--emit` needs (#351, #352), only the
        // key that cancels it does anything: every other key and every mouse
        // event is dropped, so nothing can change what is being emitted.
        if self.state == super::AppState::Finishing {
            if let event::Event::Key(key) = event
                && self.keymap.resolve(
                    crate::keymap::Scope::Finishing,
                    crate::keymap::normalise(key),
                ) == Some(crate::keymap::ActionId::FinishingCancel)
            {
                self.cancel_finish();
            }
            return;
        }

        // The terminal's interrupt (#382), before every modal: raw mode turns
        // Ctrl-c into a key, and a prompt, a picker or the filter editor would
        // otherwise swallow it. Below the finishing guard, whose own
        // `finishing.cancel` ends the session the same way. A key rebound to
        // `global.interrupt` gets the same precedence, so a printable one is
        // never typed into a prompt.
        if let event::Event::Key(key) = event
            && self
                .keymap
                .resolve(crate::keymap::Scope::Global, crate::keymap::normalise(key))
                == Some(crate::keymap::ActionId::GlobalInterrupt)
        {
            self.state = super::AppState::Cancelled;
            return;
        }

        if self.keymap_warnings_open {
            if matches!(event, event::Event::Key(_)) {
                self.keymap_warnings_open = false;
            }
            return;
        }

        // The status message lasts until the next *keypress*, and deliberately
        // not until the next event: mouse capture is on, so a mouse moving
        // across the terminal would wipe "zed: No such file or directory" off
        // the row before it could be read. `crossing` follows the same rule
        // for the same reason: its notice and the accent on the title are
        // read at a glance, and a mouse move must not wipe either before that
        // glance happens.
        if matches!(event, event::Event::Key(_)) {
            self.status_message = None;
            self.crossing = None;
            // The warning goes with the row it was on (#348).
            self.quit_confirmed = std::mem::take(&mut self.quit_warned);
        }

        // An open prompt takes precedence over every other binding.
        if self.prompt.is_some() {
            if let event::Event::Key(key) = event {
                self.handle_search_key(key);
            }
            return;
        }

        // The overlay is dismissed by the next key, and that key does nothing
        // else — it is a reference you glance at and put away, so anything that
        // required aiming at a particular key to close it would be one more
        // thing to have read the README to know (#25).
        //
        // After the prompt guard, so `?` inside a prompt is typed rather than
        // acted on, and before every other binding, so the dismissing key
        // cannot also quit, move a cursor, or open an editor.
        //
        // Only a *key* closes it. Mouse capture is on, so a mouse crossing the
        // terminal would otherwise wipe the overlay mid-read — the same
        // reasoning `status_message` gets above.
        if self.help {
            if matches!(event, event::Event::Key(_)) {
                self.help = false;
            }
            return;
        }

        // The picker takes every key while open, like the search prompt:
        // `q` inside it means nothing, and `Enter` applies rather than
        // toggling whatever the pane has selected underneath. A key that
        // resolves to nothing in `Scope::Picker` is swallowed right here —
        // it is not handed to `Global` or any other scope, and `perform` is
        // never called without an action to give it (task 7, #199).
        if let Some(picker) = self.picker.as_mut() {
            if let event::Event::Key(key) = event {
                let pressed = crate::keymap::normalise(key);
                if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Picker, pressed) {
                    match picker.perform(action) {
                        widgets::picker::PickerOutcome::Open => {}
                        widgets::picker::PickerOutcome::Closed => self.picker = None,
                        widgets::picker::PickerOutcome::Chosen(name) => {
                            let set = picker.set;
                            self.picker = None;
                            self.filters.apply_profile(set, &name);
                            self.refresh_view();
                        }
                    }
                }
            }
            return;
        }

        // The set picker is modal in the same way (#284), and swallows a
        // key that resolves to nothing in `Scope::Sets` for the same reason.
        if let Some(picker) = self.set_picker.as_mut() {
            if let event::Event::Key(key) = event {
                let pressed = crate::keymap::normalise(key);
                if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Sets, pressed) {
                    match picker.perform(action) {
                        widgets::setpicker::SetPickerOutcome::Open => {}
                        widgets::setpicker::SetPickerOutcome::Cancelled => self.set_picker = None,
                        widgets::setpicker::SetPickerOutcome::Applied(changes) => {
                            self.set_picker = None;
                            self.apply_listing(&changes);
                        }
                        widgets::setpicker::SetPickerOutcome::Search => {
                            let origin = Origin::Sets(SetsOrigin {
                                row: picker.selected(),
                                search: picker.search(),
                            });
                            self.prompt = Some(SearchPrompt {
                                origin: Some(origin),
                                ..SearchPrompt::default()
                            });
                        }
                        widgets::setpicker::SetPickerOutcome::NoHit => {
                            self.report("no more matches", false);
                        }
                    }
                }
            }
            return;
        }

        // The filter editor is modal too (#312). It swallows every event —
        // a mouse event as well, since nothing under it is on the screen.
        if self.filter_editor.is_some() {
            match event {
                event::Event::Key(key) => self.handle_filter_editor_key(key),
                // A drag along a line marks a phrase (#322).
                event::Event::Mouse(mouse) => self.handle_filter_editor_mouse(mouse),
                _ => {}
            }
            return;
        }

        // The bounce guard (#48). `Enter` both commits a prompt and toggles a
        // filter, and those are one keystroke apart, so the `Enter` that closed
        // a prompt must not fall through and switch a filter off.
        //
        // Placed after the prompt guard so it can only ever see the keypress
        // *following* the commit, and before every binding so no pane can act
        // on the swallowed key. Any other key means the user is still working
        // and the next `Enter` is meant.
        if let event::Event::Key(key) = event
            && std::mem::take(&mut self.swallow_next_enter)
            && key.code == KeyCode::Enter
        {
            return;
        }

        // The 400-line match this replaced (#199) is now a lookup plus a
        // match over actions, in `perform`: each arm there holds exactly the
        // body its key arm held here.
        if let event::Event::Key(key) = event {
            let pressed = crate::keymap::normalise(key);
            if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Global, pressed) {
                self.perform(action, pressed);
                return;
            }
            match key.code {
                // A filter-pane verb pressed anywhere else says so, for one
                // keypress, instead of doing nothing (#120 §9). Not a
                // redirect: making `i` global would collapse `f i` and `i`,
                // and `x`-not-`e` for exclude exists because `e` is a focus
                // key. The chain stays the answer; the hint teaches it.
                // These nine letters are unbound in the explorer and the
                // file view, so this arm shadows nothing.
                //
                // Not resolved through the table (#199): these are guidance,
                // not a binding, and deliberately have no `ActionId` — see
                // the doc comment on `keymap::ActionId`. The hint text is
                // generated from the table (task 8), so it names the key
                // that actually reaches the filter action, not the key this
                // arm happens to match.
                KeyCode::Char(c @ ('i' | 'x' | 'c' | 'd' | 'm' | 'a' | 's' | 'I' | 'C'))
                    // `pressed`, not `key.modifiers.is_empty()`: a terminal
                    // sets `SHIFT` on the `I`, as on every uppercase letter
                    // (#250), and `normalise` is what drops it.
                    if !pressed.ctrl && !pressed.alt && self.focus != Focus::Filters =>
                {
                    use crate::keymap::ActionId as A;
                    let (action, verb) = match c {
                        'i' => (A::FiltersInclude, "adds a filter"),
                        'x' => (A::FiltersExclude, "adds an excluding filter"),
                        'c' => (A::FiltersEdit, "changes the selected filter"),
                        'd' => (A::FiltersDelete, "deletes the selected filter"),
                        'm' => (A::FiltersContext, "toggles include and context"),
                        'a' => (A::FiltersProfile, "picks a profile for the set"),
                        'I' => (A::FiltersEditorNew, "opens the filter editor"),
                        'C' => (
                            A::FiltersEditorOpen,
                            "opens the selected filter in the filter editor",
                        ),
                        _ => (A::FiltersSolo, "solos the set"),
                    };
                    if let Some(hint) = self.keymap.hint_for(action, verb, A::GlobalFocusFilters) {
                        self.report(&hint, false);
                    }
                    return;
                }
                _ => {}
            }
        }

        if let event::Event::Mouse(mouse) = event
            && self.handle_divider(mouse)
        {
            return;
        }

        // A click that is not on a divider is aimed at a pane or the status
        // row (#58). Handled before the focused-pane dispatch below, because
        // the click decides which pane that is.
        if let event::Event::Mouse(mouse) = event
            && self.handle_click(mouse)
        {
            return;
        }

        // The wheel goes to the pane under the pointer, not the focused one
        // (#350) — so it is aimed like a click, and never reaches the
        // focused-pane dispatch below.
        if let event::Event::Mouse(mouse) = event
            && self.handle_wheel(mouse)
        {
            return;
        }

        // Filter pane keys are routed here rather than through the generic
        // `handle_events` dispatch below: applying them means mutating the
        // `ActiveFilters`, which only `App` owns, so `FilterList` cannot carry
        // them out itself — see `handle_filter_key`.
        if let event::Event::Key(key) = event
            && self.focus == Focus::Filters
        {
            self.handle_filter_key(key);
            return;
        }

        // Every `Scope::View` key resolves here rather than in the widget
        // itself: a few of these mean "the *document's* top" or
        // "the next paragraph anywhere", not "the top of the buffer that
        // happens to be loaded" (#7), and only `App` can see the document to
        // answer that. The rest could be left to the widget, but routing them
        // here too means the table — not a scattered arm — is the one place
        // plan 2b's rebinding has to reach.
        //
        // The old form of this intercept required `key.modifiers.is_empty()`,
        // and that guard *was* #250: a real terminal sets `SHIFT` on every
        // uppercase letter, so `G` carried it, failed the guard, and fell
        // through to the file view's own `G` arm, which only ever saw the
        // loaded buffer. The table normalises the key instead, so the scope
        // decides what a key means and the modifier cannot.
        //
        // An unresolved key is dropped here rather than handed on, and that
        // is what extends the guarantee above from the bound keys to every
        // key. It used to fall through to the `Focus::View` arm below
        // carrying its original, un-normalised event, and the widget's old
        // key dispatch matched on the character alone (`..` on the modifier
        // fields) — so an unbound *modified* key, `Alt-j` for instance,
        // reached the file view and moved the cursor as if the modifier were
        // never pressed, forcing a truncated preview to a full load on the
        // way in. The explorer and
        // the filter pane never had that gap: both resolve through their own
        // scope and drop an unresolved key rather than forwarding the raw
        // event (`Scope::for_focus` below; `handle_filter_key`). The view
        // matches them now.
        //
        // `[`/`]` are untouched by this, though the widget acts on them and
        // `Scope::View` has no row for either: they resolve in
        // `Scope::Global` above, which is checked first, and
        // `GlobalPageDown`/`GlobalPageUp` hand the widget its action
        // directly.
        if let event::Event::Key(key) = event
            && self.focus == Focus::View
        {
            let pressed = crate::keymap::normalise(key);
            if let Some(action) = self.keymap.resolve(crate::keymap::Scope::View, pressed) {
                self.perform(action, pressed);
            }
            return;
        }

        // The file view upgrades its own truncated preview to a full load on
        // first interaction, which rebuilds the textarea and clears its line
        // styles. That happens inside the widget, so it never reaches
        // `perform` — resync here instead, without re-reading the file.
        let was_truncated = self.file_view_truncated();
        let action = match self.focus {
            Focus::Explorer => {
                // The only call site `Scope::for_focus` has (#199): the
                // explorer's key resolves in its own scope here, then
                // `Explorer::perform` carries out whatever it named.
                let resolved = match event {
                    event::Event::Key(key) => {
                        let pressed = crate::keymap::normalise(key);
                        self.keymap
                            .resolve(crate::keymap::Scope::for_focus(self.focus), pressed)
                    }
                    _ => None,
                };
                // `n`/`N` land here whether they came from the user or from
                // `return_to_chain_origin`'s synthetic `n`; either way, a
                // `None` back means there was nothing to step to, and the
                // status row is the only way that reaches the user — the
                // key otherwise does nothing at all. `j`/`k` and every other
                // key that can also return `None` say nothing, so this is
                // gated on the action rather than on the result.
                let is_step_key = matches!(
                    resolved,
                    Some(
                        crate::keymap::ActionId::ExplorerHitNext
                            | crate::keymap::ActionId::ExplorerHitPrev
                    )
                );
                let action = resolved.and_then(|action| self.explorer.perform(action));
                if action.is_none() && is_step_key {
                    // "No matching file" is a claim about every file, and
                    // it is false while the worker is still out (#158):
                    // the chain's synthetic `n` arrives one line after
                    // `refresh_scan` started it, when every mark is still
                    // `Unknown`. Say what is true instead, and let the
                    // redraw `drain_scan_results` asks for land the answer.
                    // With no matcher there is no worker and the marks
                    // stay `Unknown` for good, so that case keeps the
                    // plain answer.
                    let text = if self.explorer.has_search() {
                        "no more matches"
                    } else if self.filters.is_scanning() && self.explorer.any_unscanned() {
                        "scanning…"
                    } else {
                        "no matching file"
                    };
                    self.report(text, false);
                }
                action
            }
            // Every key returned above, and the view has nothing to do with
            // a mouse event `handle_click` and `handle_wheel` did not take —
            // a right click, say — so this drops it rather than promoting a
            // truncated preview on a click that meant nothing.
            Focus::View => return,
            // Unreachable: filter-pane keys returned above, through
            // `handle_filter_key`. Applying them means mutating the
            // `ActiveFilters`, and the pane only ever borrows one, so it
            // cannot carry out its own commands.
            Focus::Filters => None,
        };
        if let Some(action) = action {
            self.perform_widget_action(action);
        } else if was_truncated && !self.file_view_truncated() {
            self.sync_document();
            self.refresh_view();
        }
        // Ordinary movement stays inside the window by design, but a page at
        // the edge of the middle third does not — see `window_holds`.
        self.ensure_window();
    }
}
