//! Drawing the filter editor (#312): the file with every match highlighted,
//! and the panel that holds the name, description, prompt, sense and
//! pattern.

use super::super::filter_editor::{Check, EditorFocus, FilterEditor, Mark};
use crate::filter::Sense;
use crate::widgets::pane_block;
use ratatui::prelude::{
    Buffer, Color, Constraint, Layout, Line, Modifier, Rect, Span, Style, Widget,
};
use ratatui::widgets::Clear;

/// How many columns a tab takes. A raw tab in a cell draws as nothing, and
/// log lines carry them.
const TAB: &str = "    ";

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

/// The width of the longest field label, `description: `, so the fields
/// start in one column.
const LABEL_WIDTH: usize = 13;

/// The mark of a line the pattern gets right.
const PASS: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);

impl FilterEditor {
    /// Draw the editor over `area`, which it covers entirely. `dim` is how
    /// the file view draws a line no filter matches, so a line the pattern
    /// misses looks the same here as it will there.
    pub(in crate::app) fn render(&mut self, title: &str, dim: Style, area: Rect, buf: &mut Buffer) {
        use Constraint::{Length, Min};
        Clear.render(area, buf);
        let [file_area, panel_area] =
            Layout::vertical([Min(0), Length(PANEL_ROWS + 2)]).areas(area);

        // The focused part's frame is thick and green, as a focused pane's
        // is in the main window.
        let block = pane_block(format!(" {title} "), self.focus == EditorFocus::Lines);
        let inner = block.inner(file_area);
        block.render(file_area, buf);
        self.page = usize::from(inner.height).max(1);
        self.reveal_cursor();
        let lines = self.lines.clone();
        // A row, not a line: with matches only (#315), row `top` is not line
        // `top`.
        for (y, index) in
            (inner.y..inner.bottom()).zip((self.top..).map_while(|row| self.line_at(row)))
        {
            let line = &lines[index];
            let check = self.check(index);
            let mut spans = self.gutter(index, check);
            spans.extend(self.spans(line, dim, check));
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

        let enter = if self.target.is_some() {
            "Enter change"
        } else {
            "Enter add"
        };
        let keys = match self.focus {
            EditorFocus::Pattern => format!(
                " {enter} · Esc cancel · Up/Down/PgUp/PgDn scroll · Ctrl-z/Ctrl-y undo/redo · Tab lines · Shift-Tab prompt "
            ),
            EditorFocus::Name | EditorFocus::Description | EditorFocus::Prompt => format!(
                " {enter} · Esc cancel · Up/Down/PgUp/PgDn scroll · Tab/Shift-Tab next/previous field "
            ),
            EditorFocus::Sense => format!(
                " Space/Left/Right change · i include · c context · x exclude · {enter} · Esc cancel · Tab/Shift-Tab next/previous field "
            ),
            EditorFocus::Lines => format!(
                " + must match · - must not · = clear · V range · f/F failed · n/N unmarked · u matches only · {enter} · Esc cancel · Tab name · Shift-Tab pattern "
            ),
        };
        let block =
            pane_block(" Filter editor ", self.focus != EditorFocus::Lines).title_bottom(keys);
        let inner = block.inner(panel_area);
        block.render(panel_area, buf);
        let x = inner.x + 1;
        let width = inner.width.saturating_sub(2);
        // Row 3 is the sense, which is a choice, not text.
        let fields = [
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
        if let Some(error) = &self.error
            && inner.height >= PANEL_ROWS
        {
            buf.set_stringn(
                x,
                inner.y + PANEL_ROWS - 1,
                error,
                usize::from(width),
                Style::default().fg(Color::Red),
            );
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

    /// One line of the file as spans: a matched line in the new filter's
    /// colour with each match reversed, a missed line dimmed. A line whose
    /// check fails is in `FAIL` instead, its matches still reversed, so a
    /// must-not-match line shows what the pattern wrongly matched.
    fn spans(&self, line: &str, dim: Style, check: Option<Check>) -> Vec<Span<'static>> {
        let failed = check.is_some_and(|check| !check.passes);
        let Some(regex) = self.regex.as_ref().filter(|regex| regex.is_match(line)) else {
            let style = if failed {
                FAIL
            } else if self.regex.is_some() {
                dim
            } else {
                Style::default()
            };
            return vec![Span::styled(line.replace('\t', TAB), style)];
        };
        let style = if failed { FAIL } else { self.style };
        let mut spans = Vec::new();
        let mut at = 0;
        for found in regex.find_iter(line) {
            if found.start() > at {
                spans.push(Span::styled(
                    line[at..found.start()].replace('\t', TAB),
                    style,
                ));
            }
            spans.push(Span::styled(
                found.as_str().replace('\t', TAB),
                style.add_modifier(Modifier::REVERSED),
            ));
            at = found.end();
        }
        if at < line.len() {
            spans.push(Span::styled(line[at..].replace('\t', TAB), style));
        }
        spans
    }
}
