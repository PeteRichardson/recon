//! The command palette's list (#520): the commands that match what the
//! user typed, drawn above the bottom row, each with its key and its help.

use super::App;
use crate::commands::{self, Command};
use ratatui::prelude::{Buffer, Color, Line, Rect, Span, Style, Widget};
use ratatui::style::Modifier;
use ratatui::widgets::Clear;
use unicode_width::UnicodeWidthStr;

/// The most rows the list takes, so the panes stay in sight above it.
const MAX_ROWS: usize = 8;

impl App<'_> {
    /// The commands the palette's line matches, best first: by the word
    /// typed so far, or — once a blank follows the word — the command that
    /// word names. `None` when no palette is open.
    pub(in crate::app) fn palette_matches(&self) -> Option<Vec<&Command>> {
        let prompt = self.palette()?;
        // While Tab chooses a name, the list stays the one its word matched.
        let line = prompt
            .completion
            .as_ref()
            .and_then(|completion| completion.word.as_deref())
            .unwrap_or_else(|| prompt.pattern.trim_start());
        let rows = if let Some((word, _)) = line.split_once(char::is_whitespace) {
            commands::find(&self.commands, word).into_iter().collect()
        } else {
            commands::matches(&self.commands, line)
                .into_iter()
                .map(|index| &self.commands[index])
                .collect()
        };
        Some(rows)
    }

    /// How many rows the list takes above the bottom row in an area
    /// `height` rows tall, and none without a palette. For the tests, which
    /// read the rows the list takes.
    #[cfg(test)]
    pub(in crate::app) fn palette_list_height(&self, height: u16) -> usize {
        self.palette_matches()
            .map_or(0, |rows| list_height(rows.len(), height))
    }

    /// Draw the list at the bottom of `area`, the area above the bottom
    /// row: three columns, the names with the argument, the keys, and the
    /// help. The row Tab chose is in reverse video.
    pub(super) fn render_palette(&self, area: Rect, buf: &mut Buffer) {
        let Some(rows) = self.palette_matches() else {
            return;
        };
        let height = list_height(rows.len(), area.height);
        if height == 0 {
            return;
        }
        let list = Rect {
            y: area.bottom() - height as u16,
            height: height as u16,
            ..area
        };
        Clear.render(list, buf);
        let dim = Style::default().fg(Color::DarkGray);
        if rows.is_empty() {
            buf.set_stringn(
                list.x + 1,
                list.y,
                "no command matches",
                usize::from(list.width),
                dim,
            );
            return;
        }
        let cells: Vec<(String, String)> = rows[..height]
            .iter()
            .map(|command| (command.label(), self.keys_cell(command)))
            .collect();
        let names_width = cells.iter().map(|(n, _)| n.width()).max().unwrap_or(0);
        let keys_width = cells.iter().map(|(_, k)| k.width()).max().unwrap_or(0);
        let chosen = self
            .palette()
            .and_then(|prompt| prompt.completion.as_ref())
            .and_then(super::super::palette::Completion::chosen_row);
        for (i, (command, (names, keys))) in rows.iter().zip(&cells).enumerate() {
            let row = Rect {
                y: list.y + i as u16,
                height: 1,
                ..list
            };
            let pad = |text: &str, width: usize| " ".repeat(width - text.width());
            let line = Line::from(vec![
                Span::raw(format!(" {names}{}  ", pad(names, names_width))),
                Span::styled(format!("{keys}{}  ", pad(keys, keys_width)), dim),
                Span::raw(command.help.as_ref()),
            ]);
            buf.set_line(row.x, row.y, &line, row.width);
            if chosen == Some(i) {
                buf.set_style(row, Style::default().add_modifier(Modifier::REVERSED));
            }
        }
    }

    /// The second column of a row: the keys that run its action now, or
    /// nothing for a command no key runs.
    fn keys_cell(&self, command: &Command) -> String {
        command
            .action()
            .map(|action| self.keymap.labels_for(action).join(" / "))
            .unwrap_or_default()
    }
}

/// One row per match up to `MAX_ROWS` and half of `height`, and one to say
/// nothing matches.
fn list_height(matches: usize, height: u16) -> usize {
    matches.clamp(1, MAX_ROWS).min(usize::from(height / 2))
}
