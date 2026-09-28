//! The prompt on the bottom row: what it edits, its history, and where
//! an `Esc` puts things back.

use super::App;
use super::search::{Search, WRAPPED_TO_TOP};
use crate::filter;
use crate::widgets::Focus;
use crossterm::event::{self, KeyCode, KeyModifiers};

/// Shown in the prompt when a pattern will not compile, after vim's error.
pub(super) const INVALID_PATTERN: &str = "E486: invalid pattern";

/// What an open prompt will do with the pattern being typed.
///
/// The `Edit` variants are what makes a filter's pattern changeable at all:
/// before them the only way to correct one was `d` and a full retype, which
/// pushed the replacement to the end of the set and so changed its colour and
/// its precedence in `verdict`. Committing one overwrites in place instead.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptKind {
    #[default]
    Search,
    Filter,
    Exclude,
    /// `S`: the scratch set's new name. Commits through `save_scratch_as`,
    /// and its error is that function's message rather than
    /// `INVALID_PATTERN` — a name is not a pattern.
    SaveSet,
    /// Replace the pattern of the numbered filter at `index`.
    ///
    /// `sense` is carried purely so the prompt can draw the right sigil — the
    /// filter's own sense is untouched by the edit. Without it an excluding
    /// filter would edit under a `filter:` prompt, reading as though
    /// committing were about to turn it into an including one.
    ///
    /// The index is captured when the prompt opens and could in principle name
    /// a filter that is gone by the time `Enter` arrives. It cannot today: the
    /// prompt consumes every key while it is open, so nothing can delete a
    /// filter in between. `replace_filter` handles the case anyway rather than
    /// resting on that.
    Edit {
        index: usize,
        sense: filter::Sense,
    },
}

/// Where the user was when `/` opened (glossary: *origin*), in whichever
/// pane it opened over.
///
/// While the prompt is open every edit re-runs the search from here, never
/// from the position the last keystroke reached, so a pattern narrowed by
/// one more character cannot walk away from where the user started. Esc
/// puts all of it back, so a bad probe costs nothing; Enter drops it.
///
/// One variant per place a `/` can open, rather than one struct with
/// optional parts: each search restores different things, and a prompt
/// is only ever over one pane — the mouse is ignored while it is open, so
/// the focus cannot change under it.
#[derive(Debug, Clone)]
pub(super) enum Origin {
    /// `/` over the file view or the filter pane, which forwards it.
    View(ViewOrigin),
    /// `/` over the explorer (#272).
    Explorer(ExplorerOrigin),
    /// `/` in the set picker (#285).
    Sets(SetsOrigin),
}

/// The file view's cursor and scroll, and the search that was set at the
/// time.
#[derive(Debug, Clone)]
pub(super) struct ViewOrigin {
    /// The cursor's row in the visible set, and its column.
    row: usize,
    col: usize,
    /// The pane row the cursor was drawn on, so a restore puts the scroll
    /// back as well as the cursor.
    screen_row: u16,
    /// The search set before `/` opened, if any. Esc restores it with the
    /// cursor: the probe replaced it on screen, not in fact.
    search: Option<Search>,
}

/// The explorer's selected row, and the filename search that was set at
/// the time.
#[derive(Debug, Clone)]
pub(super) struct ExplorerOrigin {
    /// The selected row as an `entries` index, which a scan answer that
    /// re-lists the rows in hide mode cannot move; the row it is on can.
    pub(super) entry: usize,
    /// The filename search set before `/` opened, if any. Esc restores it
    /// with the row, so `n` afterwards repeats what it repeated before.
    search: Option<regex::Regex>,
}

/// The set picker's selected row, and the search that was set in it at
/// the time.
#[derive(Debug, Clone)]
pub(super) struct SetsOrigin {
    pub(super) row: usize,
    pub(super) search: Option<regex::Regex>,
}

/// How many committed patterns a `History` keeps before the oldest goes.
pub(super) const HISTORY_CAP: usize = 50;

/// The patterns Enter committed in one kind of `/` prompt, newest first
/// (#274), so a near-miss regex is recalled and corrected rather than
/// retyped.
///
/// One per prompt kind that has one: the file search, the explorer's
/// filename search and the set picker's search (#285) keep separate
/// histories, because a filename pattern offered in the file prompt is
/// noise. In memory only, so a session starts
/// empty. A pattern committed again moves to the front rather than appearing
/// twice, and the `HISTORY_CAP`th pattern pushes the oldest out. Only Enter
/// adds: a cancelled prompt leaves no trace, and `*` never goes through the
/// prompt at all.
#[derive(Debug, Default)]
pub(super) struct History {
    /// Newest first: index 0 is what the first Up recalls.
    patterns: Vec<String>,
}

impl History {
    fn push(&mut self, pattern: &str) {
        self.patterns.retain(|kept| kept != pattern);
        self.patterns.insert(0, pattern.to_owned());
        self.patterns.truncate(HISTORY_CAP);
    }

    /// The pattern `steps` back from the newest, or `None` past the oldest.
    fn recall(&self, steps: usize) -> Option<&str> {
        self.patterns.get(steps).map(String::as_str)
    }
}

/// Which way Up and Down walk a `History`: Up towards the oldest pattern,
/// Down back towards the newest and then to an empty prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recall {
    Older,
    Newer,
}

/// A search pattern being typed at the bottom of the screen.
#[derive(Debug, Default)]
pub(super) struct SearchPrompt {
    pub(super) pattern: String,
    pub(super) error: Option<String>,
    pub(super) kind: PromptKind,
    pub(super) cursor: usize,
    /// Set for every `/`, whichever pane it opened over: the search moves
    /// as it is typed, and this is where it moves from and where Esc goes
    /// back to. `None` for every filter prompt, which commits on Enter only.
    pub(super) origin: Option<Origin>,
    /// Which history entry the pattern was recalled from, as steps back
    /// from the newest, or `None` while the prompt shows what was typed
    /// (#274). Up steps it further back, Down towards the newest and then
    /// off the end to an empty prompt. Typing after a recall leaves it
    /// where it is, so the next Up still goes to the entry before.
    pub(super) recall: Option<usize>,
}

impl SearchPrompt {
    /// The prefix the prompt draws, which names what committing will do.
    ///
    /// An edit shows the same sigil as the `i`, `x` or `/` that would have
    /// created the thing being edited, because it produces the same kind of
    /// thing. What differs is where it lands, and the pre-filled pattern
    /// already says that: a prompt that opens with text in it is editing
    /// something, and one that opens empty is making something new.
    pub(super) fn sigil(&self) -> &'static str {
        match self.kind {
            PromptKind::Search => "/",
            PromptKind::Filter
            | PromptKind::Edit {
                sense: filter::Sense::Include | filter::Sense::Context,
                ..
            } => "filter: ",
            PromptKind::Exclude
            | PromptKind::Edit {
                sense: filter::Sense::Exclude,
                ..
            } => "exclude: ",
            PromptKind::SaveSet => "save as: ",
        }
    }

    /// What the bottom line shows: the error if the pattern was rejected,
    /// otherwise the pattern being typed behind its sigil.
    pub(super) fn line(&self) -> String {
        match &self.error {
            Some(error) => error.clone(),
            None => format!("{}{}", self.sigil(), self.pattern),
        }
    }

    /// An empty prompt of `kind`, cursor at its start.
    pub(super) fn new(kind: PromptKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// A prompt pre-filled with `pattern`, cursor at its end — where `c`
    /// starts, so a `Backspace` or a typed character acts on the tail.
    pub(super) fn editing(pattern: String, kind: PromptKind) -> Self {
        let cursor = pattern.chars().count();
        Self {
            pattern,
            kind,
            cursor,
            ..Self::default()
        }
    }

    /// Where the row draws the cursor: the column after the sigil and the
    /// characters before the cursor. `None` while an error is showing, which
    /// replaces the pattern on the row and has no cursor in it.
    pub(super) fn cursor_column(&self) -> Option<usize> {
        self.error
            .is_none()
            .then(|| self.sigil().chars().count() + self.cursor)
    }

    /// Byte offset of character `index` — `cursor` is a character index,
    /// because `Left` and `Right` step by character and the row draws by
    /// column, and the pattern is `String`.
    fn byte_at(&self, index: usize) -> usize {
        self.pattern
            .char_indices()
            .nth(index)
            .map_or(self.pattern.len(), |(byte, _)| byte)
    }

    fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.pattern.insert(at, c);
        self.cursor += 1;
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.pattern.chars().count());
    }

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor = self.pattern.chars().count();
    }

    /// Delete the character before the cursor. `false` when there is none
    /// — at the start of the pattern, which is the only place the caller
    /// has to distinguish an empty pattern from a full one.
    fn delete_before(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let (start, end) = (self.byte_at(self.cursor - 1), self.byte_at(self.cursor));
        self.pattern.replace_range(start..end, "");
        self.cursor -= 1;
        true
    }

    /// Delete the character under the cursor; nothing at the end.
    fn delete_at(&mut self) {
        let (start, end) = (self.byte_at(self.cursor), self.byte_at(self.cursor + 1));
        if start < end {
            self.pattern.replace_range(start..end, "");
        }
    }

    /// vim's command-line `Ctrl-w`: blanks between the cursor and the word
    /// before it go, then the word — a run of identifier characters, the
    /// same definition `*` uses, or a run of anything else that is not a
    /// blank, so `foo::` loses `::` and then `foo` in two presses.
    fn delete_word_before(&mut self) {
        let chars: Vec<char> = self.pattern.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        if start > 0 {
            let in_word = is_word(chars[start - 1]);
            while start > 0
                && !chars[start - 1].is_whitespace()
                && is_word(chars[start - 1]) == in_word
            {
                start -= 1;
            }
        }
        self.delete_range(start, self.cursor);
    }

    /// vim's command-line `Ctrl-u`: everything before the cursor goes.
    fn delete_to_start(&mut self) {
        self.delete_range(0, self.cursor);
    }

    /// Delete characters `start..end` and leave the cursor at `start`.
    fn delete_range(&mut self, start: usize, end: usize) {
        let (from, to) = (self.byte_at(start), self.byte_at(end));
        self.pattern.replace_range(from..to, "");
        self.cursor = start;
    }
}

impl App<'_> {
    /// Feed a key to the open search prompt.
    ///
    /// While it is open it consumes every key, so app-wide commands like `q`
    /// are typed into the pattern rather than acted on.
    pub(super) fn handle_search_key(&mut self, key: event::KeyEvent) {
        let pressed = crate::keymap::normalise(key);
        if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Prompt, pressed) {
            use crate::keymap::ActionId as A;
            match action {
                A::PromptCancel => self.cancel_prompt(),
                A::PromptCommit => {
                    let Some(prompt) = self.prompt.as_ref() else {
                        return;
                    };
                    let (pattern, kind) = (prompt.pattern.clone(), prompt.kind);

                    // Enter on an empty search is a cancel, not a search
                    // for the empty pattern (which matches every line): the
                    // user is back at the origin, as if Esc.
                    if kind == PromptKind::Search && pattern.is_empty() && prompt.origin.is_some() {
                        self.cancel_prompt();
                        return;
                    }

                    // `SaveSet` is not a pattern: its failures are messages,
                    // shown in the prompt in place of `INVALID_PATTERN`.
                    if kind == PromptKind::SaveSet {
                        match self.save_scratch_as(&pattern) {
                            Ok(()) => {
                                self.prompt = None;
                                self.swallow_next_enter = true;
                            }
                            Err(message) => {
                                if let Some(prompt) = self.prompt.as_mut() {
                                    prompt.error = Some(message);
                                }
                            }
                        }
                        return;
                    }
                    let outcome = match kind {
                        PromptKind::Search => self.run_search(&pattern),
                        PromptKind::SaveSet => unreachable!("handled above"),
                        PromptKind::Filter => self.add_filter(&pattern),
                        PromptKind::Exclude => self.add_excluding_filter(&pattern),
                        PromptKind::Edit { index, .. } => self.replace_filter(index, &pattern),
                    };
                    if outcome.is_ok() {
                        let origin = self.prompt.take().and_then(|prompt| prompt.origin);
                        // Only Enter adds to a history (#274), and only a
                        // `/` prompt has one: the origin says which pane it
                        // opened over, and so which of the two it feeds.
                        if let Some(origin) = &origin {
                            self.history_for(origin).push(&pattern);
                        }
                        // The set picker takes the next key itself, before
                        // the bounce guard below could ever see it, so an
                        // armed guard would wait there and swallow an
                        // `Enter` long after the picker closed.
                        if matches!(origin, Some(Origin::Sets(_))) {
                            return;
                        }
                        // Enter keeps the position the search reached. The
                        // wrap it reported while typing went with the
                        // keystroke, so say it again if the hit is above
                        // where `/` opened: the jump upward was real. The
                        // explorer's search never reported a wrap and
                        // still does not: its `n` is silent about one too.
                        if let Some(Origin::View(origin)) = origin
                            && self.search.is_some()
                            && self.view.cursor_visible_row() < origin.row
                        {
                            self.report(WRAPPED_TO_TOP, false);
                        }
                        // Arm the bounce guard (#48). Only on the branch that
                        // actually closes the prompt: a rejected pattern leaves it
                        // open, so the next `Enter` is another commit attempt and
                        // never reaches the filter pane to be swallowed.
                        self.swallow_next_enter = true;
                        if matches!(
                            kind,
                            PromptKind::Filter | PromptKind::Exclude | PromptKind::Edit { .. }
                        ) {
                            self.return_to_chain_origin();
                        }
                    } else if let Some(prompt) = self.prompt.as_mut() {
                        prompt.error = Some(INVALID_PATTERN.to_string());
                    }
                }
                A::PromptDeleteBack => {
                    if let Some(prompt) = self.prompt.as_mut() {
                        prompt.error = None;
                        // Backspacing past the start of an *empty* prompt
                        // abandons it, as in vim. At the start of a pattern with
                        // text after the cursor there is nothing to delete and
                        // nothing to abandon: the text is what the user is
                        // keeping (#206).
                        if !prompt.delete_before() && prompt.pattern.is_empty() {
                            self.cancel_prompt();
                            return;
                        }
                    }
                    self.rescan_from_origin();
                }
                // The cursor keys (#206): vim's command-line set, plus the
                // readline pair for the ends, which the same hands type at a
                // shell. `Home`/`Ctrl-a` share `PromptStart` and `End`/`Ctrl-e`
                // share `PromptEnd` — two labels, one action, one arm each.
                A::PromptLeft => self.edit_prompt(SearchPrompt::move_left),
                A::PromptRight => self.edit_prompt(SearchPrompt::move_right),
                A::PromptStart => self.edit_prompt(SearchPrompt::move_to_start),
                A::PromptEnd => self.edit_prompt(SearchPrompt::move_to_end),
                A::PromptDeleteForward => self.edit_pattern(SearchPrompt::delete_at),
                A::PromptDeleteWord => self.edit_pattern(SearchPrompt::delete_word_before),
                A::PromptDeleteStart => self.edit_pattern(SearchPrompt::delete_to_start),
                A::PromptHistoryPrev => self.recall(Recall::Older),
                A::PromptHistoryNext => self.recall(Recall::Newer),
                // `resolve(Scope::Prompt, ..)` only ever answers with one of
                // the arms above — see `DEFAULT`'s `Scope::Prompt` rows — so
                // this is unreached today. Not a wildcard omitted by
                // accident: matched explicitly, like `Explorer::perform`'s
                // trailing arm, so a scope neither of us wired stays inert
                // instead of panicking a user's terminal.
                _ => {}
            }
            return;
        }
        match key.code {
            // No prompt binding uses a modified character, and in raw mode
            // a pasted line feed arrives as Ctrl-J (`Char('j')` with
            // CONTROL) rather than as a bare `\n` — dropping it here keeps
            // a pasted newline out of the single-line pattern (#120 §13).
            KeyCode::Char(_)
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            // A paste arrives as one `Char` per character. A newline in it
            // is dropped rather than typed: the pattern is single-line, and
            // a stray `\n` would silently make it match nothing (#120 §13).
            // Matched by guard rather than by literal so the bound-key
            // scanner in `help.rs`, which reads every `'x'` inside `Char(..)`,
            // does not take the escape's backslash for a key.
            KeyCode::Char(c) if c == '\n' || c == '\r' => {}
            // Every character that is not bound is not a binding, and cannot
            // be rebound: it types itself, which is what a prompt is for.
            KeyCode::Char(c) => self.edit_pattern(|prompt| prompt.insert(c)),
            _ => {}
        }
    }

    /// Apply one editing step to the open prompt. Any edit clears a pending
    /// error: the row goes back to showing the pattern, which is what the
    /// user is now correcting.
    fn edit_prompt(&mut self, edit: impl FnOnce(&mut SearchPrompt)) {
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.error = None;
            edit(prompt);
        }
    }

    /// `edit_prompt` for an edit that can change the pattern's text, which
    /// a search prompt answers by moving. The cursor keys go through
    /// `edit_prompt` directly: they change where the next character lands,
    /// not what the pattern says, and a scan per arrow key would be paid
    /// for nothing.
    fn edit_pattern(&mut self, edit: impl FnOnce(&mut SearchPrompt)) {
        self.edit_prompt(edit);
        self.rescan_from_origin();
    }

    /// Up or Down in a `/` prompt (#274): replace the pattern with the next
    /// older or newer committed one, cursor at its end, and move as typing
    /// it would — a recall is an edit, so it goes through `edit_pattern`
    /// and re-runs the scan from the origin. Up at the oldest stays there;
    /// Down past the newest empties the prompt, and Down on an empty prompt
    /// that recalled nothing does nothing. A filter prompt has no origin
    /// and so no history: both keys are inert in it, as they were unbound.
    fn recall(&mut self, direction: Recall) {
        let Some(prompt) = self.prompt.as_ref() else {
            return;
        };
        let Some(origin) = prompt.origin.as_ref() else {
            return;
        };
        let target = match direction {
            Recall::Older => Some(prompt.recall.map_or(0, |steps| steps + 1)),
            Recall::Newer => match prompt.recall {
                None => return,
                Some(steps) => steps.checked_sub(1),
            },
        };
        let history = self.history_for_ref(origin);
        let pattern = match target {
            Some(steps) => match history.recall(steps) {
                Some(pattern) => pattern.to_owned(),
                None => return,
            },
            None => String::new(),
        };
        self.edit_pattern(|prompt| {
            prompt.recall = target;
            prompt.cursor = pattern.chars().count();
            prompt.pattern = pattern;
        });
    }

    /// The history a `/` prompt with this origin reads and feeds: the file
    /// search's for the view (and the filter pane, which forwards `/` to
    /// it), the filename search's for the explorer.
    fn history_for(&mut self, origin: &Origin) -> &mut History {
        match origin {
            Origin::View(_) => &mut self.search_history,
            Origin::Explorer(_) => &mut self.explorer_search_history,
            Origin::Sets(_) => &mut self.sets_search_history,
        }
    }

    /// `history_for`, read-only.
    fn history_for_ref(&self, origin: &Origin) -> &History {
        match origin {
            Origin::View(_) => &self.search_history,
            Origin::Explorer(_) => &self.explorer_search_history,
            Origin::Sets(_) => &self.sets_search_history,
        }
    }

    /// Close the prompt without committing, and put back the origin if the
    /// prompt had one: the cursor, the scroll, and the search that was set
    /// before `/` opened — so the highlight the probe painted goes with it.
    /// In the explorer, the selected row, the preview it had, and the
    /// filename search that was set.
    fn cancel_prompt(&mut self) {
        let origin = self.prompt.take().and_then(|prompt| prompt.origin);
        self.chain_origin = None;
        match origin {
            Some(Origin::View(origin)) => {
                self.search.clone_from(&origin.search);
                self.restore_origin(&origin);
            }
            Some(Origin::Explorer(origin)) => {
                self.explorer.set_search(origin.search);
                self.move_explorer_to(origin.entry);
            }
            Some(Origin::Sets(origin)) => {
                if let Some(picker) = self.set_picker.as_mut() {
                    picker.set_search(origin.search);
                    picker.select(origin.row);
                }
            }
            None => {}
        }
    }

    /// Put the cursor and the scroll back where `/` found them.
    ///
    /// A selection comes back with them (#273): its anchor never moved, and
    /// its other end *is* the view's cursor, so restoring the cursor is
    /// restoring the selection. Nothing here touches `visual`.
    ///
    /// The landing row is requested first: `place_cursor_on_visible_row`
    /// keeps an earlier request when it rebuilds the buffer, and the render
    /// applies it whether or not a rebuild happened, so the cursor is drawn
    /// on the pane row it was on and the scroll follows.
    fn restore_origin(&mut self, origin: &ViewOrigin) {
        self.view.land_cursor_on_row(origin.screen_row);
        self.place_cursor_on_visible_row(origin.row);
        self.view.set_cursor_col(origin.col);
    }

    /// Move the explorer's selection to `entry` and preview it, as a `j`
    /// onto that row would — the view pane follows a filename search the
    /// same way it follows the cursor keys. Nothing when the selection is
    /// already there, so a probe that stays put does not reload the pane.
    fn move_explorer_to(&mut self, entry: usize) {
        if let Some(action) = self.explorer.go_to_entry(entry) {
            self.perform_widget_action(action);
        }
    }

    /// The origin for a `/` opened now, in the pane that has focus: the
    /// explorer's selected row and its filename search, or the file view's
    /// cursor and the search that is set. The filter pane forwards `/` to
    /// the view, so its origin is the view's.
    pub(super) fn capture_origin(&self) -> Origin {
        match self.focus {
            Focus::Explorer => Origin::Explorer(ExplorerOrigin {
                entry: self.explorer.selected_entry().unwrap_or(0),
                search: self.explorer.search_matcher(),
            }),
            Focus::View | Focus::Filters => Origin::View(ViewOrigin {
                row: self.view.cursor_visible_row(),
                col: self.view.cursor_col(),
                screen_row: self.view.cursor_screen_row(),
                search: self.search.clone(),
            }),
        }
    }

    /// The search moves as it is typed: run it from the origin for the
    /// pattern the prompt holds now, and move to the first hit at or after
    /// the origin — the cursor to a line, with the window's hits
    /// highlighted, or the explorer's selection to an entry, with the
    /// matching names restyled and the view pane previewing it.
    ///
    /// Always from the origin, never from where the last keystroke landed:
    /// `foo` then `food` must not find the next `food` *after* the `foo` the
    /// shorter pattern reached. The scan stops at the first hit, and no
    /// count is kept, so a keystroke costs one walk that ends early.
    ///
    /// A pattern that does not compile yet (`foo(`) is silent: the highlight
    /// clears and the cursor sits at the origin. Only Enter reports it. The
    /// same for an empty pattern, and for one with no hit, which sits at
    /// the origin without a highlight to show.
    fn rescan_from_origin(&mut self) {
        let Some(prompt) = self.prompt.as_ref() else {
            return;
        };
        let (Some(origin), PromptKind::Search) = (prompt.origin.clone(), prompt.kind) else {
            return;
        };
        let pattern = prompt.pattern.clone();
        match origin {
            Origin::View(origin) => self.rescan_view_from(&origin, &pattern),
            Origin::Explorer(origin) => self.rescan_explorer_from(&origin, &pattern),
            Origin::Sets(origin) => self.rescan_sets_from(&origin, &pattern),
        }
    }

    /// `rescan_from_origin` for the file view.
    fn rescan_view_from(&mut self, origin: &ViewOrigin, pattern: &str) {
        let search = if pattern.is_empty() {
            None
        } else {
            Search::new(pattern).ok()
        };
        let Some(search) = search else {
            self.search = None;
            self.restore_origin(origin);
            return;
        };
        self.search = Some(search);
        self.promote_truncated_preview();
        let Some((row, column, wrapped)) = self.hit_from(origin.row) else {
            self.restore_origin(origin);
            return;
        };
        self.jump_to_visible_row(row);
        self.view.set_cursor_col(column);
        if wrapped {
            self.report(WRAPPED_TO_TOP, false);
        }
    }

    /// `rescan_from_origin` for the explorer (#272): the filename search
    /// is set so the matching names light up, and the selection goes to
    /// the first of them at or after the origin row, or back to the origin
    /// row when there is none — or nothing to look for yet.
    fn rescan_explorer_from(&mut self, origin: &ExplorerOrigin, pattern: &str) {
        let matcher = if pattern.is_empty() {
            None
        } else {
            regex::Regex::new(pattern).ok()
        };
        self.explorer.set_search(matcher);
        let entry = self.explorer.hit_from(origin.entry).unwrap_or(origin.entry);
        self.move_explorer_to(entry);
    }

    /// `rescan_from_origin` for the set picker (#285): the same as the
    /// explorer's, over the picker's rows.
    fn rescan_sets_from(&mut self, origin: &SetsOrigin, pattern: &str) {
        let search = if pattern.is_empty() {
            None
        } else {
            regex::Regex::new(pattern).ok()
        };
        if let Some(picker) = self.set_picker.as_mut() {
            picker.set_search(search);
            picker.select(picker.hit_from(origin.row).unwrap_or(origin.row));
        }
    }
}
