//! Drawing the filter editor (#312): the file with every match highlighted,
//! and the panel that holds the pattern.

use super::super::filter_editor::FilterEditor;
use ratatui::prelude::{
    Buffer, Color, Constraint, Layout, Line, Modifier, Rect, Span, Style, Widget,
};
use ratatui::widgets::{Block, Clear};

/// How many columns a tab takes. A raw tab in a cell draws as nothing, and
/// log lines carry them.
const TAB: &str = "    ";

impl FilterEditor {
    /// Draw the editor over `area`, which it covers entirely. `dim` is how
    /// the file view draws a line no filter matches, so a line the pattern
    /// misses looks the same here as it will there.
    pub(in crate::app) fn render(&mut self, title: &str, dim: Style, area: Rect, buf: &mut Buffer) {
        use Constraint::{Length, Min};
        Clear.render(area, buf);
        let [file_area, panel_area] = Layout::vertical([Min(0), Length(4)]).areas(area);

        let block = Block::bordered().title(format!(" {title} "));
        let inner = block.inner(file_area);
        block.render(file_area, buf);
        self.page = usize::from(inner.height).max(1);
        for (y, line) in (inner.y..inner.bottom()).zip(self.lines.iter().skip(self.top)) {
            Line::from(self.spans(line, dim)).render(
                Rect {
                    y,
                    height: 1,
                    ..inner
                },
                buf,
            );
        }

        let block =
            Block::bordered()
                .title(" Filter editor ")
                .title_bottom(if self.target.is_some() {
                    " Enter change · Esc cancel · Up/Down/PgUp/PgDn scroll "
                } else {
                    " Enter add · Esc cancel · Up/Down/PgUp/PgDn scroll "
                });
        let inner = block.inner(panel_area);
        block.render(panel_area, buf);
        if inner.height == 0 {
            return;
        }
        let label = "pattern: ";
        let x = inner.x + 1;
        let width = inner.width.saturating_sub(2);
        buf.set_stringn(
            x,
            inner.y,
            format!("{label}{}", self.field.pattern),
            usize::from(width),
            Style::default(),
        );
        // The cursor as the prompt row draws it: the cell in reversed video.
        let column = label.chars().count() + self.field.cursor;
        if let Ok(column) = u16::try_from(column)
            && column < width
        {
            buf[(x + column, inner.y)].set_style(Style::default().add_modifier(Modifier::REVERSED));
        }
        if let Some(error) = &self.error
            && inner.height > 1
        {
            buf.set_stringn(
                x,
                inner.y + 1,
                error,
                usize::from(width),
                Style::default().fg(Color::Red),
            );
        }
    }

    /// One line of the file as spans: a matched line in the new filter's
    /// colour with each match reversed, a missed line dimmed.
    fn spans(&self, line: &str, dim: Style) -> Vec<Span<'static>> {
        let Some(regex) = self.regex.as_ref().filter(|regex| regex.is_match(line)) else {
            let style = if self.regex.is_some() {
                dim
            } else {
                Style::default()
            };
            return vec![Span::styled(line.replace('\t', TAB), style)];
        };
        let mut spans = Vec::new();
        let mut at = 0;
        for found in regex.find_iter(line) {
            if found.start() > at {
                spans.push(Span::styled(
                    line[at..found.start()].replace('\t', TAB),
                    self.style,
                ));
            }
            spans.push(Span::styled(
                found.as_str().replace('\t', TAB),
                self.style.add_modifier(Modifier::REVERSED),
            ));
            at = found.end();
        }
        if at < line.len() {
            spans.push(Span::styled(line[at..].replace('\t', TAB), self.style));
        }
        spans
    }
}
