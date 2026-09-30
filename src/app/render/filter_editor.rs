//! Drawing the filter editor (#312): the file with every match highlighted,
//! and the panel that holds the name, description, prompt, sense and
//! pattern, and with a model the request line and the model's explanation
//! (#319).

use super::super::filter_editor::{
    Check, EditorFocus, FilterEditor, GENERATED, Generated, Mark, PATTERN_CHANGED,
    PROMPT_CHANGED_NO_MODEL, TAB_WIDTH, prompt_changed,
};
use crate::filter::Sense;
use crate::keymap::{ActionId, Keymap};
use crate::widgets::pane_block;
use ratatui::prelude::{
    Buffer, Color, Constraint, Layout, Line, Modifier, Rect, Span, Style, Widget,
};
use ratatui::widgets::Clear;
use unicode_width::UnicodeWidthStr;

/// How many columns a tab takes. A raw tab in a cell draws as nothing, and
/// log lines carry them.
const TAB: &str = "    ";
const _: () = assert!(TAB.len() == TAB_WIDTH);

/// The columns before each line: the cursor, the mark, and a space (#314).
const GUTTER: u16 = 3;

/// A marked line the pattern gets wrong: the loudest style on the screen, so
/// a failed check is seen before anything else.
const FAIL: Style = Style::new()
    .fg(Color::White)
    .bg(Color::Red)
    .add_modifier(Modifier::BOLD);

/// The panel's rows: the five fields and the error line.
const PANEL_ROWS: u16 = 6;

/// The rows a model adds (#319): the request line and the explanation.
const MODEL_ROWS: u16 = 2;

/// The width of the longest field label, `description: `, so the fields
/// start in one column.
const LABEL_WIDTH: usize = 13;

/// Before an example that is not a line of the file (#318).
const NOT_IN_FILE: &str = "example, not in the file: ";

/// The mark of a line the pattern gets right.
const PASS: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);

/// A phrase mark (#322), added to the style of the text under it: a line
/// mark colours a whole line, a phrase mark underlines only its part.
const PHRASE: Modifier = Modifier::UNDERLINED.union(Modifier::BOLD);

/// The gutter of a line with a phrase mark and no line mark.
const PHRASE_GUTTER: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

impl FilterEditor {
    /// Draw the editor over `area`, which it covers entirely. `dim` is how
    /// the file view draws a line no filter matches, so a line the pattern
    /// misses looks the same here as it will there.
    pub(in crate::app) fn render(
        &mut self,
        title: &str,
        dim: Style,
        keymap: &Keymap,
        area: Rect,
        buf: &mut Buffer,
    ) {
        use Constraint::{Length, Min};
        Clear.render(area, buf);
        let rows = if self.model {
            PANEL_ROWS + MODEL_ROWS
        } else {
            PANEL_ROWS
        };
        let [file_area, panel_area] = Layout::vertical([Min(0), Length(rows + 2)]).areas(area);

        // The focused part's frame is thick and green, as a focused pane's
        // is in the main window.
        let block = pane_block(format!(" {title} "), self.focus == EditorFocus::Lines);
        let inner = block.inner(file_area);
        block.render(file_area, buf);
        self.lines_area = inner;
        self.page = usize::from(inner.height).max(1);
        self.reveal_cursor();
        // A row, not a line: with matches only (#315), row `top` is not line
        // `top`.
        for (y, index) in
            (inner.y..inner.bottom()).zip((self.top..).map_while(|row| self.line_at(row)))
        {
            let line = self.text(index).unwrap_or_default();
            let check = self.check(index);
            let mut spans = self.gutter(index, check);
            // An example the file does not have (#318) says so before its
            // text, so it is not read as a line of the file.
            if index >= self.lines.len() {
                spans.push(Span::styled(
                    NOT_IN_FILE,
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.extend(self.spans(index, line, dim, check));
            // A failed check fills its whole row, not only its text.
            let row = if check.is_some_and(|check| !check.passes) {
                FAIL
            } else {
                Style::default()
            };
            Line::from(spans).style(row).render(
                Rect {
                    y,
                    height: 1,
                    ..inner
                },
                buf,
            );
        }

        let keys = self.key_hints(keymap);
        let block =
            pane_block(" Filter editor ", self.focus != EditorFocus::Lines).title_bottom(keys);
        let inner = block.inner(panel_area);
        block.render(panel_area, buf);
        let x = inner.x + 1;
        let width = inner.width.saturating_sub(2);
        // Row 3 is the sense, which is a choice, not text. The request line
        // is row 5, with a model only.
        let mut fields = vec![
            ("name:", &self.name, EditorFocus::Name, 0),
            (
                "description:",
                &self.description,
                EditorFocus::Description,
                1,
            ),
            ("prompt:", &self.prompt, EditorFocus::Prompt, 2),
            ("pattern:", &self.field, EditorFocus::Pattern, 4),
        ];
        if self.model {
            fields.push(("request:", &self.request, EditorFocus::Request, 5));
        }
        for (label, field, focus, row) in fields {
            let y = inner.y + row;
            if y >= inner.bottom() {
                continue;
            }
            buf.set_stringn(
                x,
                y,
                format!("{label:LABEL_WIDTH$}{}", field.pattern),
                usize::from(width),
                Style::default(),
            );
            // The cursor as the prompt row draws it: the cell in reversed
            // video. Only in the field the keys go to: on the lines, the
            // cursor line is the one to watch.
            let column = LABEL_WIDTH + field.cursor;
            if self.focus == focus
                && let Ok(column) = u16::try_from(column)
                && column < width
            {
                buf[(x + column, y)].set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }
        // After the prompt, whether the model wrote the pattern from it.
        // Not in the consolidation step (#322): the prompt there is the
        // model's proposal, and the status row says what it is.
        if let Some((text, style)) = self.generated_note(keymap)
            && self.consolidation.is_none()
            && inner.height > 2
        {
            // Terminal columns, not chars: a wide character takes two.
            let column = LABEL_WIDTH + UnicodeWidthStr::width(self.prompt.pattern.as_str()) + 2;
            if let Ok(column) = u16::try_from(column)
                && column < width
            {
                buf.set_stringn(
                    x + column,
                    inner.y + 2,
                    text,
                    usize::from(width - column),
                    style,
                );
            }
        }
        if inner.height > 3 {
            Line::from(self.sense_spans()).render(
                Rect {
                    x,
                    y: inner.y + 3,
                    width,
                    height: 1,
                },
                buf,
            );
        }
        // What the model said of the pattern it gave, under the request.
        if let Some(explanation) = &self.explanation
            && self.model
            && inner.height > 6
        {
            buf.set_stringn(
                x,
                inner.y + 6,
                format!("{:LABEL_WIDTH$}{explanation}", "model:"),
                usize::from(width),
                Style::default().fg(Color::DarkGray),
            );
        }
        if let Some(error) = &self.error
            && inner.height >= rows
        {
            buf.set_stringn(
                x,
                inner.y + rows - 1,
                error,
                usize::from(width),
                Style::default().fg(Color::Red),
            );
        }
    }

    /// The keys the panel's bottom border names for the focused field.
    ///
    /// Every key the keymap can move is looked up (#386): the border spelled
    /// them, so a rebind left it naming keys that no longer did what it
    /// said. A key a rebind left unbound drops out with its verb. The sense
    /// row's keys are the field's own, like the characters a text field
    /// takes, and no `[keymap]` line reaches them.
    fn key_hints(&self, keymap: &Keymap) -> String {
        use ActionId::{
            FilterEditorCancel, FilterEditorCommit, FilterEditorFailureNext,
            FilterEditorFailurePrev, FilterEditorFocus, FilterEditorFocusPrev,
            FilterEditorMarkClear, FilterEditorMarkMatch, FilterEditorMarkNoMatch,
            FilterEditorPageDown, FilterEditorPageUp, FilterEditorRedo, FilterEditorRegenerate,
            FilterEditorScrollDown, FilterEditorScrollUp, FilterEditorToggleMatchesOnly,
            FilterEditorUndo, FilterEditorUnmarkedNext, FilterEditorUnmarkedPrev,
            FilterEditorVisualLine,
        };
        // Keys joined by `/`, then the verb: `Ctrl-z/Ctrl-y undo/redo`. All
        // or nothing, as a pair with one key missing reads as the wrong verb.
        let hint = |actions: &[ActionId], verb: &str| -> Option<String> {
            let keys: Option<Vec<&str>> = actions.iter().map(|&a| keymap.label_for(a)).collect();
            Some(format!("{} {verb}", keys?.join("/")))
        };
        // The scroll keys each do their own part, so the ones still bound
        // are named even when a rebind took another.
        let scroll = || -> Option<String> {
            let keys: Vec<&str> = [
                FilterEditorScrollUp,
                FilterEditorScrollDown,
                FilterEditorPageUp,
                FilterEditorPageDown,
            ]
            .iter()
            .filter_map(|&a| keymap.label_for(a))
            .collect();
            (!keys.is_empty()).then(|| format!("{} scroll", keys.join("/")))
        };
        let enter = hint(
            &[FilterEditorCommit],
            if self.target.is_some() {
                "change"
            } else {
                "add"
            },
        );
        let cancel = hint(&[FilterEditorCancel], "cancel");
        let undo = hint(&[FilterEditorUndo, FilterEditorRedo], "undo/redo");
        let fields = hint(
            &[FilterEditorFocus, FilterEditorFocusPrev],
            "next/previous field",
        );
        // With a model, the request line is between the pattern and the
        // lines in the ring.
        let (after_pattern, before_lines) = if self.model {
            ("request", "request")
        } else {
            ("lines", "pattern")
        };
        let parts: Vec<Option<String>> = match self.focus {
            _ if self.consolidation.is_some() => vec![
                hint(&[FilterEditorCommit], "save with this prompt"),
                hint(&[FilterEditorCancel], "save with the prompt as it was"),
            ],
            EditorFocus::Pattern => vec![
                enter,
                cancel,
                scroll(),
                undo,
                hint(&[FilterEditorFocus], after_pattern),
                hint(&[FilterEditorFocusPrev], "prompt"),
            ],
            EditorFocus::Request => vec![
                hint(&[FilterEditorCommit], "send"),
                cancel,
                undo,
                hint(&[FilterEditorFocus], "lines"),
                hint(&[FilterEditorFocusPrev], "pattern"),
            ],
            EditorFocus::Prompt if self.model => vec![
                enter,
                cancel,
                hint(&[FilterEditorRegenerate], "regenerate"),
                scroll(),
                fields,
            ],
            EditorFocus::Name | EditorFocus::Description | EditorFocus::Prompt => {
                vec![enter, cancel, scroll(), fields]
            }
            EditorFocus::Sense => vec![
                Some("Space/Left/Right change · i include · c context · x exclude".to_string()),
                enter,
                cancel,
                fields,
            ],
            EditorFocus::Lines => vec![
                hint(&[FilterEditorMarkMatch], "must match"),
                hint(&[FilterEditorMarkNoMatch], "must not"),
                hint(&[FilterEditorMarkClear], "clear"),
                hint(&[FilterEditorVisualLine], "range"),
                hint(
                    &[FilterEditorFailureNext, FilterEditorFailurePrev],
                    "failed",
                ),
                hint(
                    &[FilterEditorUnmarkedNext, FilterEditorUnmarkedPrev],
                    "unmarked",
                ),
                hint(&[FilterEditorToggleMatchesOnly], "matches only"),
                enter,
                cancel,
                hint(&[FilterEditorFocus], "name"),
                hint(&[FilterEditorFocusPrev], before_lines),
            ],
        };
        let parts: Vec<String> = parts.into_iter().flatten().collect();
        format!(" {} ", parts.join(" · "))
    }

    /// What the prompt row says of the pattern's origin (#321), and how:
    /// nothing for an ordinary filter.
    fn generated_note(&self, keymap: &Keymap) -> Option<(String, Style)> {
        let warn = Style::default().fg(Color::Yellow);
        match self.generated() {
            Generated::Yes => Some((GENERATED.to_string(), Style::default().fg(Color::Cyan))),
            Generated::PatternChanged => Some((PATTERN_CHANGED.to_string(), warn)),
            Generated::PromptChanged if self.model => Some((
                prompt_changed(keymap.label_for(ActionId::FilterEditorRegenerate)),
                warn,
            )),
            Generated::PromptChanged => Some((PROMPT_CHANGED_NO_MODEL.to_string(), warn)),
            Generated::No => None,
        }
    }

    /// The sense row: the three senses, the filter's in bold, and in
    /// reversed video while the keys go to it; the other two dimmed.
    fn sense_spans(&self) -> Vec<Span<'static>> {
        let mut spans = vec![Span::raw(format!("{:LABEL_WIDTH$}", "sense:"))];
        for (sense, word) in [
            (Sense::Include, "include"),
            (Sense::Context, "context"),
            (Sense::Exclude, "exclude"),
        ] {
            let style = if sense != self.sense {
                Style::default().fg(Color::DarkGray)
            } else if self.focus == EditorFocus::Sense {
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(word, style));
            spans.push(Span::raw("  "));
        }
        spans.pop();
        spans
    }

    /// The columns before line `index`: `>` on the cursor line and `|` on
    /// the rest of a visual range while the keys go to the lines, then the
    /// line's mark, `+` or `-`, in the style of its check.
    fn gutter(&self, index: usize, check: Option<Check>) -> Vec<Span<'static>> {
        let (first, last) = self.range();
        let cursor = match self.focus {
            EditorFocus::Lines if index == self.cursor => Span::styled(
                ">",
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ),
            EditorFocus::Lines if self.anchor.is_some() && (first..=last).contains(&index) => {
                Span::styled("|", Style::default().add_modifier(Modifier::REVERSED))
            }
            _ => Span::raw(" "),
        };
        let mark = match check {
            None if self.phrases.iter().any(|phrase| phrase.line == index) => {
                Span::styled("~ ", PHRASE_GUTTER)
            }
            None => Span::raw("  "),
            Some(Check { mark, passes }) => {
                let symbol = match mark {
                    Mark::MustMatch => "+ ",
                    Mark::MustNotMatch => "- ",
                };
                Span::styled(symbol, if passes { PASS } else { FAIL })
            }
        };
        debug_assert_eq!(
            usize::from(GUTTER),
            cursor.width() + mark.width(),
            "the gutter is not {GUTTER} columns"
        );
        vec![cursor, mark]
    }

    /// Line `index`, `line`, as spans: a matched line in the new filter's
    /// colour with each match reversed, a missed line dimmed. A line whose
    /// check fails is in `FAIL` instead, its matches still reversed, so a
    /// must-not-match line shows what the pattern wrongly matched. Each
    /// phrase mark, and the selection a drag is making, is underlined in
    /// bold over that (#322).
    fn spans(
        &self,
        index: usize,
        line: &str,
        dim: Style,
        check: Option<Check>,
    ) -> Vec<Span<'static>> {
        let failed = check.is_some_and(|check| !check.passes);
        let found: Vec<(usize, usize)> = self
            .regex
            .as_ref()
            .map(|regex| {
                regex
                    .find_iter(line)
                    .map(|m| (m.start(), m.end()))
                    .collect()
            })
            .unwrap_or_default();
        let phrases: Vec<(usize, usize)> = self
            .phrases
            .iter()
            .chain(&self.selection)
            .filter(|phrase| phrase.line == index)
            .map(|phrase| (phrase.start, phrase.end))
            .collect();
        let style = if failed {
            FAIL
        } else if !found.is_empty() {
            self.style
        } else if self.regex.is_some() {
            dim
        } else {
            Style::default()
        };
        let mut cuts: Vec<usize> = found
            .iter()
            .chain(&phrases)
            .flat_map(|&(start, end)| [start, end])
            .chain([0, line.len()])
            .filter(|&at| at <= line.len() && line.is_char_boundary(at))
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let inside = |ranges: &[(usize, usize)], at: usize| {
            ranges.iter().any(|&(start, end)| start <= at && at < end)
        };
        let mut spans: Vec<Span<'static>> = cuts
            .windows(2)
            .map(|pair| {
                let (start, end) = (pair[0], pair[1]);
                let mut part = style;
                if inside(&found, start) {
                    part = part.add_modifier(Modifier::REVERSED);
                }
                if inside(&phrases, start) {
                    part = part.add_modifier(PHRASE);
                }
                Span::styled(line[start..end].replace('\t', TAB), part)
            })
            .collect();
        if spans.is_empty() {
            spans.push(Span::styled(String::new(), style));
        }
        spans
    }

    /// The display column of line `index`'s text under screen column
    /// `column` (#322), past the gutter and the note before an example the
    /// file does not have: `None` left of the text.
    pub(in crate::app) fn text_column(&self, index: usize, column: u16) -> Option<usize> {
        let mut before = usize::from(self.lines_area.x) + usize::from(GUTTER);
        if index >= self.lines.len() {
            before += UnicodeWidthStr::width(NOT_IN_FILE);
        }
        usize::from(column).checked_sub(before)
    }
}
