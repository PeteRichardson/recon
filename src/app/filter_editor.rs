//! The filter editor (#38, #312): a full-screen place to write a filter's
//! pattern against the open file, with every line it matches highlighted
//! while it is typed.
//!
//! `f I` opens it on a new, empty pattern, and `f C` on the selected filter's
//! pattern (#313). It covers the whole window, as the set picker does, and
//! takes every key while it is open. Enter adds the pattern to the scratch
//! set exactly as `f i` would, or replaces the selected filter's pattern
//! exactly as `f c` would; Esc changes nothing.

use super::App;
use super::prompt::SearchPrompt;
use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::prelude::Style;
use regex::Regex;
use std::sync::Arc;

/// Shown in the panel when Enter finds no pattern to add.
pub(super) const NO_PATTERN: &str = "type a pattern first";

/// What the filter editor holds while it is open.
#[derive(Debug)]
pub(super) struct FilterEditor {
    /// The pattern being typed. A `SearchPrompt` for its line editing only,
    /// so the editor's field and every other prompt move and delete the same
    /// way under the same keys; its `kind`, `origin` and `error` are unused.
    pub(super) field: SearchPrompt,
    /// The file's lines, shared with the file view that read them.
    pub(super) lines: Arc<Vec<String>>,
    /// The regex the highlight uses: the last pattern that compiled, so a
    /// half-typed `(` does not blank the screen. `None` while the pattern is
    /// empty — an empty regex matches every line, and a highlight on every
    /// line says nothing.
    pub(super) regex: Option<Regex>,
    /// Why the pattern as typed does not compile, or why Enter refused it.
    pub(super) error: Option<String>,
    /// How many of `lines` `regex` matches.
    pub(super) matches: usize,
    /// The first line drawn.
    pub(super) top: usize,
    /// How many lines the last render drew, for a page key.
    pub(super) page: usize,
    /// The colour the filter takes, so the highlight shows the line as the
    /// file view will show it after Enter: the next palette colour for a new
    /// filter, the filter's own for one being changed.
    pub(super) style: Style,
    /// The filter Enter changes, by index, or `None` for a new filter. The
    /// editor takes every key while it is open, so nothing can remove the
    /// filter under it; `replace_filter` checks the index anyway.
    pub(super) target: Option<usize>,
}

impl FilterEditor {
    /// An editor over `lines`, with an empty pattern.
    pub(super) fn new(lines: Arc<Vec<String>>, style: Style) -> Self {
        Self {
            field: SearchPrompt::default(),
            lines,
            regex: None,
            error: None,
            matches: 0,
            top: 0,
            page: 1,
            style,
            target: None,
        }
    }

    /// An editor over `lines` on the pattern of the filter at `index`, cursor
    /// at its end as `c` puts it, with the highlight and the count already
    /// showing.
    pub(super) fn editing(
        lines: Arc<Vec<String>>,
        style: Style,
        index: usize,
        pattern: String,
    ) -> Self {
        let mut editor = Self {
            field: SearchPrompt::editing(pattern, super::prompt::PromptKind::default()),
            target: Some(index),
            ..Self::new(lines, style)
        };
        editor.recompile();
        editor
    }

    /// Compile the pattern again after an edit, and count what it matches.
    ///
    /// A pattern that does not compile keeps the last highlight and count:
    /// the user is in the middle of typing, and the screen should not flash
    /// empty between `(` and `)`.
    fn recompile(&mut self) {
        let pattern = self.field.pattern.as_str();
        if pattern.is_empty() {
            self.regex = None;
            self.error = None;
            self.matches = 0;
            return;
        }
        match Regex::new(pattern) {
            Ok(regex) => {
                self.matches = self
                    .lines
                    .iter()
                    .filter(|line| regex.is_match(line))
                    .count();
                self.regex = Some(regex);
                self.error = None;
            }
            Err(error) => self.error = Some(error_line(&error)),
        }
    }

    /// Move the first line drawn by `delta` lines, kept inside the file.
    fn scroll(&mut self, delta: isize) {
        let last = self.lines.len().saturating_sub(1);
        self.top = self.top.saturating_add_signed(delta).min(last);
    }

    /// What the status row shows: how many lines the pattern matches.
    pub(super) fn status(&self) -> String {
        let total = grouped(self.lines.len());
        if self.regex.is_none() {
            return format!("{total} lines");
        }
        format!("{} of {total} lines match", grouped(self.matches))
    }
}

/// The one line of a `regex::Error` worth showing in a single row.
///
/// A syntax error's text is several lines: the pattern, a caret under the
/// fault, and `error: <reason>` last. The panel shows the pattern already,
/// so the reason is what is left to say.
fn error_line(error: &regex::Error) -> String {
    let text = error.to_string();
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("invalid pattern")
        .trim()
        .to_string()
}

/// `n` with a comma between each group of three digits: `50,000`.
pub(super) fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

impl App<'_> {
    /// `f I`: open the filter editor on the open file, with an empty
    /// pattern.
    ///
    /// A truncated preview is loaded in full first, as `n` and `/` do: the
    /// count is a claim about the whole file.
    pub(super) fn open_filter_editor(&mut self) {
        self.promote_truncated_preview();
        let lines = self.view.source().clone();
        self.filter_editor = Some(FilterEditor::new(lines, self.filters.next_style()));
    }

    /// `f C`: open the filter editor on the filter at `index` (#313). A
    /// definition filter has no pattern to show, and says so instead.
    pub(super) fn open_filter_editor_on(&mut self, index: usize) {
        let Some(filter) = self.filters.filters().get(index) else {
            return;
        };
        let Some(regex) = filter.predicate.as_regex() else {
            self.report(
                "a definition filter has no pattern to edit; c turns it into one",
                false,
            );
            return;
        };
        let (pattern, style) = (regex.as_str().to_string(), filter.style);
        self.promote_truncated_preview();
        let lines = self.view.source().clone();
        self.filter_editor = Some(FilterEditor::editing(lines, style, index, pattern));
    }

    /// Feed a key to the open filter editor. It takes every key: a key
    /// `Scope::FilterEditor` does not bind is tried as a prompt editing key, and a
    /// character that is neither is typed into the pattern.
    pub(super) fn handle_filter_editor_key(&mut self, key: event::KeyEvent) {
        use crate::keymap::ActionId as A;
        let pressed = crate::keymap::normalise(key);
        if let Some(action) = self
            .keymap
            .resolve(crate::keymap::Scope::FilterEditor, pressed)
        {
            match action {
                A::FilterEditorCommit => self.commit_filter_editor(),
                A::FilterEditorCancel => {
                    self.filter_editor = None;
                    // As a cancelled `f i` prompt: the chain ends where it is.
                    self.chain_origin = None;
                }
                A::FilterEditorScrollUp => self.edit_filter_editor(|editor| editor.scroll(-1)),
                A::FilterEditorScrollDown => self.edit_filter_editor(|editor| editor.scroll(1)),
                A::FilterEditorPageUp => self.edit_filter_editor(|editor| {
                    editor.scroll(-isize::try_from(editor.page).unwrap_or(isize::MAX));
                }),
                A::FilterEditorPageDown => self.edit_filter_editor(|editor| {
                    editor.scroll(isize::try_from(editor.page).unwrap_or(isize::MAX));
                }),
                // `resolve(Scope::FilterEditor, ..)` answers only with the arms
                // above; matched rather than left to a panic, as in
                // `handle_search_key`.
                _ => {}
            }
            return;
        }
        if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Prompt, pressed) {
            let edit: fn(&mut SearchPrompt) = match action {
                A::PromptLeft => SearchPrompt::move_left,
                A::PromptRight => SearchPrompt::move_right,
                A::PromptStart => SearchPrompt::move_to_start,
                A::PromptEnd => SearchPrompt::move_to_end,
                // Unlike a prompt, Backspace at the start never closes the
                // editor: the file view it covers is still worth reading.
                A::PromptDeleteBack => |field| {
                    field.delete_before();
                },
                A::PromptDeleteForward => SearchPrompt::delete_at,
                A::PromptDeleteWord => SearchPrompt::delete_word_before,
                A::PromptDeleteStart => SearchPrompt::delete_to_start,
                // Commit and cancel are the editor's own keys, above, and
                // the editor keeps no history.
                _ => return,
            };
            self.edit_filter_editor(|editor| {
                let before = editor.field.pattern.clone();
                edit(&mut editor.field);
                if editor.field.pattern != before {
                    editor.recompile();
                }
            });
            return;
        }
        match key.code {
            // The same three refusals as `handle_search_key`: no modified
            // character is typed, and a pasted newline is dropped.
            KeyCode::Char(_)
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            KeyCode::Char(c) if c == '\n' || c == '\r' => {}
            KeyCode::Char(c) => self.edit_filter_editor(|editor| {
                editor.field.insert(c);
                editor.recompile();
            }),
            _ => {}
        }
    }

    fn edit_filter_editor(&mut self, edit: impl FnOnce(&mut FilterEditor)) {
        if let Some(editor) = self.filter_editor.as_mut() {
            edit(editor);
        }
    }

    /// Enter: add the pattern as `f i … Enter` would, or change the target
    /// filter's as `f c … Enter` would, and close. A pattern that is empty
    /// or does not compile keeps the editor open, with the reason in the
    /// panel.
    fn commit_filter_editor(&mut self) {
        let Some(editor) = self.filter_editor.as_mut() else {
            return;
        };
        if editor.field.pattern.is_empty() {
            editor.error = Some(NO_PATTERN.to_string());
            return;
        }
        if editor.error.is_some() {
            return;
        }
        let (pattern, target) = (editor.field.pattern.clone(), editor.target);
        let outcome = match target {
            None => self.add_filter(&pattern),
            Some(index) => self.replace_filter(index, &pattern),
        };
        if let Err(error) = outcome {
            if let Some(editor) = self.filter_editor.as_mut() {
                editor.error = Some(error_line(&error));
            }
            return;
        }
        self.filter_editor = None;
        // The same close as a filter prompt's commit: arm the bounce guard
        // (#48), and end the chain where it started.
        self.swallow_next_enter = true;
        self.return_to_chain_origin();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_puts_a_comma_between_each_three_digits() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(50_000), "50,000");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn an_error_shows_its_reason_line() {
        // Through a `String`, so clippy does not refuse the bad literal.
        let pattern = String::from("foo(");
        let error = Regex::new(&pattern).expect_err("unclosed group");
        assert_eq!(error_line(&error), "error: unclosed group");
    }
}
