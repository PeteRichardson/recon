//! The profile picker: a small overlay listing one set's profiles (#130).
//!
//! Profiles are deliberately not drawn in the filter pane. A comma-separated
//! list after a set's name is too wide for a column the navigator sizes, and
//! a row per profile would double the pane's height for a thing used once
//! per triage. So a set's header carries a bare `*` when it has profiles,
//! and `a` on that row opens this: a centred box over the panes, one profile
//! per line, drawn the way the `?` overlay is. It takes every key while open,
//! as the search prompt does, so `q` cannot quit from inside it.
//!
//! Choosing a profile is an *action*: `App` applies it and the picker closes.
//! Nothing remembers which profile was applied — see the spec's "a profile is
//! an action, not a live binding".

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::{Buffer, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, Clear};

/// What one key did to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PickerOutcome {
    /// Still open — the selection moved, or the key meant nothing.
    Open,
    /// `Esc`: closed without choosing.
    Closed,
    /// `Enter`: this profile was chosen, and the picker is closed.
    Chosen(String),
}

#[derive(Debug)]
pub(crate) struct ProfilePicker {
    /// The set whose profiles these are, so `App` knows where to apply the
    /// choice.
    pub(crate) set: usize,
    names: Vec<String>,
    selected: usize,
}

impl ProfilePicker {
    /// A picker over `names`, which must not be empty — `App` reports a set
    /// with no profiles on the status row rather than opening an empty box.
    pub(crate) fn new(set: usize, names: Vec<String>) -> Self {
        debug_assert!(!names.is_empty(), "a picker needs something to pick");
        Self {
            set,
            names,
            selected: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    /// Feed a key. `j`/`k` and the arrows move; `Enter` chooses; `Esc`
    /// closes; everything else is swallowed, as a prompt swallows it.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> PickerOutcome {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.names.len().saturating_sub(1));
                PickerOutcome::Open
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                PickerOutcome::Open
            }
            KeyCode::Enter => match self.names.get(self.selected) {
                Some(name) => PickerOutcome::Chosen(name.clone()),
                None => PickerOutcome::Closed,
            },
            KeyCode::Esc => PickerOutcome::Closed,
            _ => PickerOutcome::Open,
        }
    }

    /// Draw the picker centred in `area`, over whatever is already there.
    pub(crate) fn render(&self, area: Rect, buf: &mut Buffer) {
        let widest = self
            .names
            .iter()
            .map(|name| name.chars().count())
            .max()
            .unwrap_or(0);
        // Two for the borders, two for a column of padding each side.
        let width = u16::try_from(widest + 4)
            .unwrap_or(u16::MAX)
            .min(area.width);
        let height = u16::try_from(self.names.len() + 2)
            .unwrap_or(u16::MAX)
            .min(area.height);
        let panel = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        // The panes are already drawn underneath; without this their borders
        // and text show through wherever the overlay writes nothing.
        Clear.render(panel, buf);
        let block = Block::bordered().title(" Profiles ");
        let inner = block.inner(panel);
        block.render(panel, buf);
        for (offset, name) in self.names.iter().enumerate() {
            let Ok(y) = u16::try_from(offset) else { break };
            if y >= inner.height {
                break;
            }
            let style = if offset == self.selected {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            buf.set_stringn(
                inner.x + 1,
                inner.y + y,
                name,
                usize::from(inner.width.saturating_sub(2)),
                style,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn picker() -> ProfilePicker {
        ProfilePicker::new(1, vec!["default".into(), "loud".into()])
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn j_and_k_move_and_clamp() {
        let mut p = picker();
        assert_eq!(p.handle_key(key(KeyCode::Char('k'))), PickerOutcome::Open);
        assert_eq!(p.selected(), 0);
        p.handle_key(key(KeyCode::Char('j')));
        p.handle_key(key(KeyCode::Char('j')));
        assert_eq!(p.selected(), 1);
        p.handle_key(key(KeyCode::Up));
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn enter_chooses_and_esc_closes() {
        let mut p = picker();
        p.handle_key(key(KeyCode::Char('j')));
        assert_eq!(
            p.handle_key(key(KeyCode::Enter)),
            PickerOutcome::Chosen("loud".into())
        );
        assert_eq!(
            picker().handle_key(key(KeyCode::Esc)),
            PickerOutcome::Closed
        );
    }

    #[test]
    fn other_keys_are_swallowed() {
        assert_eq!(
            picker().handle_key(key(KeyCode::Char('q'))),
            PickerOutcome::Open
        );
    }

    #[test]
    fn it_draws_a_centred_titled_box_with_the_selection_reversed() {
        let p = picker();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        p.render(area, &mut buf);
        let row = |y: u16| -> String { (0..40).map(|x| buf[(x, y)].symbol()).collect() };
        // Widest name is 7, so the panel is 11 wide and 4 tall, centred.
        assert!(row(3).contains("Profiles"), "{}", row(3));
        assert!(row(4).contains("default"), "{}", row(4));
        assert!(row(5).contains("loud"), "{}", row(5));
        let x = row(4).find("default").expect("drawn");
        let x = u16::try_from(x).unwrap();
        assert!(
            buf[(x, 4)]
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buf[(x, 5)]
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }
}
