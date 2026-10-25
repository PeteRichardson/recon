//! The profile picker: a small overlay listing one set's profiles (#130),
//! then its named filters below a separator (#539).
//!
//! Profiles are deliberately not drawn in the filter pane. A comma-separated
//! list after a set's name is too wide for a column the explorer sizes, and
//! a row per profile would double the pane's height for a thing used once
//! per triage. So a set's header carries a bare `*` when it has profiles,
//! and `a` on that row opens this: a centred box over the panes, one profile
//! per line, drawn the way the `?` overlay is. It takes every key while open,
//! as the search prompt does, so `q` cannot quit from inside it.
//!
//! Choosing a profile or a filter is an *action*: `App` applies it — exactly
//! its filters on, the set's others off, as `--set NAME:ITEM` does — and the
//! picker closes.
//! Nothing remembers which profile was applied — see the spec's "a profile is
//! an action, not a live binding".

use super::listmotion::keep_in_view;
use ratatui::prelude::{Buffer, Line, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, Clear};
use unicode_width::UnicodeWidthStr;

/// What one key did to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PickerOutcome {
    /// Still open — the selection moved, or the key meant nothing.
    Open,
    /// `Esc`: closed without choosing.
    Closed,
    /// `Enter`: this profile or named filter was chosen, and the picker is
    /// closed. A loaded set never has a name that is both.
    Chosen(String),
}

#[derive(Debug)]
pub(crate) struct ProfilePicker {
    /// The set whose profiles these are, so `App` knows where to apply the
    /// choice.
    pub(crate) set: usize,
    /// The profiles, then the named filters.
    names: Vec<String>,
    /// How many of `names` are profiles: a separator row is drawn after
    /// them when filters follow.
    profiles: usize,
    selected: usize,
    /// The first row drawn, when the list is taller than the box. Kept
    /// between frames so that a move inside the visible rows does not
    /// scroll; `render` only moves it to keep the selection in view.
    top: usize,
}

impl ProfilePicker {
    /// A picker over `profiles`, then `filters`. Together they must not be
    /// empty — `App` reports a set with neither on the status row rather
    /// than opening an empty box.
    pub(crate) fn new(set: usize, profiles: Vec<String>, filters: Vec<String>) -> Self {
        let count = profiles.len();
        let mut names = profiles;
        names.extend(filters);
        debug_assert!(!names.is_empty(), "a picker needs something to pick");
        Self {
            set,
            names,
            profiles: count,
            selected: 0,
            top: 0,
        }
    }

    /// The row after the profiles, when there are profiles and filters.
    fn separator(&self) -> Option<usize> {
        (self.profiles > 0 && self.profiles < self.names.len()).then_some(self.profiles)
    }

    /// Every row: the names, and the separator between the profiles and
    /// the filters.
    fn rows(&self) -> usize {
        self.names.len() + usize::from(self.separator().is_some())
    }

    /// The row of `names[index]`: each filter sits one row lower, below the
    /// separator.
    fn row_of(&self, index: usize) -> usize {
        index + usize::from(self.separator().is_some_and(|sep| index >= sep))
    }

    #[cfg(test)]
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    /// Carry out a `Scope::Picker` action. `j`/`k` move; `Enter` chooses;
    /// `Esc` closes.
    ///
    /// `App`'s modal dispatch is the only caller (task 7, #199): it resolves
    /// the key against `Scope::Picker` first, and a key that names no action
    /// there is swallowed by `App` itself — the same "takes every key"
    /// behaviour the old `_ =>` arm gave directly, now given by `App` never
    /// calling this function at all. That also retires the modified-key
    /// guard this used to open with: `Ctrl-j` no longer resolves to
    /// `PickerDown` in the table (#120, #193), so it never reaches here
    /// either.
    pub(crate) fn perform(&mut self, action: crate::keymap::ActionId) -> PickerOutcome {
        use crate::keymap::ActionId as A;
        match action {
            A::PickerDown => {
                self.selected = (self.selected + 1).min(self.names.len().saturating_sub(1));
                PickerOutcome::Open
            }
            A::PickerUp => {
                self.selected = self.selected.saturating_sub(1);
                PickerOutcome::Open
            }
            A::PickerChoose => match self.names.get(self.selected) {
                Some(name) => PickerOutcome::Chosen(name.clone()),
                None => PickerOutcome::Closed,
            },
            A::PickerCancel => PickerOutcome::Closed,
            // See `Explorer::perform`'s trailing arm: `resolve(Scope::Picker,
            // ..)` only ever answers with one of the four arms above, so this
            // is unreached today, and stays inert rather than panicking if
            // that ever stops being true.
            _ => PickerOutcome::Open,
        }
    }

    /// Where the box goes, centred in `area` and never larger than it.
    ///
    /// Split out of `render` so that the sizing can be tested without a
    /// buffer, and so that the two subtractions cannot underflow. They were
    /// safe only because both sides are clamped one line earlier, which is a
    /// property a later sizing edit can remove without any warning (#203).
    fn panel(&self, area: Rect) -> Rect {
        let widest = self
            .names
            .iter()
            .map(|name| UnicodeWidthStr::width(name.as_str()))
            .max()
            .unwrap_or(0);
        let rows = self.rows();
        let height = u16::try_from(rows + 2).unwrap_or(u16::MAX).min(area.height);
        // Two for the borders, two for a column of padding each side; and
        // room on the top border for the title, and the count when the list
        // scrolls (#539).
        let scrolls = rows + 2 > usize::from(area.height);
        let border = TITLE.width()
            + if scrolls {
                count(rows, '↑').width()
            } else {
                0
            }
            + 2;
        let width = u16::try_from((widest + 4).max(border))
            .unwrap_or(u16::MAX)
            .min(area.width);
        Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        }
    }

    /// Draw the picker centred in `area`, over whatever is already there.
    /// A list taller than the box scrolls to keep the selection in view,
    /// and the borders count the rows hidden above and below (#539).
    ///
    /// `&mut` because it scrolls, as the set picker does: only the drawn
    /// height says how far.
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let panel = self.panel(area);
        let rows = self.rows();
        let height = usize::from(panel.height.saturating_sub(2));
        self.top = keep_in_view(self.top, self.row_of(self.selected), height, rows);
        let (top, below) = (self.top, rows.saturating_sub(self.top + height));

        let mut block = Block::bordered().title(TITLE);
        if top > 0 {
            block = block.title_top(Line::from(count(top, '↑')).right_aligned());
        }
        if below > 0 {
            block = block.title_bottom(Line::from(count(below, '↓')).right_aligned());
        }
        // The panes are already drawn underneath; without this their borders
        // and text show through wherever the overlay writes nothing.
        Clear.render(panel, buf);
        let inner = block.inner(panel);
        block.render(panel, buf);

        // The screen line of a list row, if it is in view.
        let line = |row: usize| {
            (top..top + height)
                .contains(&row)
                .then(|| inner.y + u16::try_from(row - top).unwrap_or(u16::MAX))
        };
        if let Some(y) = self.separator().and_then(line) {
            let rule = "─".repeat(usize::from(inner.width));
            buf.set_stringn(
                inner.x,
                y,
                &rule,
                usize::from(inner.width),
                Style::default(),
            );
        }
        for (index, name) in self.names.iter().enumerate() {
            let Some(y) = line(self.row_of(index)) else {
                continue;
            };
            let style = if index == self.selected {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            buf.set_stringn(
                inner.x + 1,
                y,
                name,
                usize::from(inner.width.saturating_sub(2)),
                style,
            );
        }
    }
}

const TITLE: &str = " Profiles ";

/// A border label for `rows` hidden rows on the side `arrow` points to.
fn count(rows: usize, arrow: char) -> String {
    format!(" {arrow} {rows} ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::ActionId as A;

    fn picker() -> ProfilePicker {
        ProfilePicker::new(1, vec!["default".into(), "loud".into()], vec![])
    }

    #[test]
    fn j_and_k_move_and_clamp() {
        let mut p = picker();
        assert_eq!(p.perform(A::PickerUp), PickerOutcome::Open);
        assert_eq!(p.selected(), 0);
        p.perform(A::PickerDown);
        p.perform(A::PickerDown);
        assert_eq!(p.selected(), 1);
        p.perform(A::PickerUp);
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn enter_chooses_and_esc_closes() {
        let mut p = picker();
        p.perform(A::PickerDown);
        assert_eq!(
            p.perform(A::PickerChoose),
            PickerOutcome::Chosen("loud".into())
        );
        assert_eq!(picker().perform(A::PickerCancel), PickerOutcome::Closed);
    }

    /// `resolve(Scope::Picker, ..)` never actually produces anything but the
    /// four arms above, but `perform` still answers safely if it ever did —
    /// see the comment on its trailing arm.
    #[test]
    fn an_unrelated_action_is_swallowed() {
        assert_eq!(picker().perform(A::GlobalQuit), PickerOutcome::Open);
    }

    #[test]
    fn it_draws_a_centred_titled_box_with_the_selection_reversed() {
        let mut p = picker();
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

    /// The set's named filters follow its profiles below a separator row
    /// (#539). `j` steps from the last profile to the first filter, never
    /// onto the separator, and `Enter` on a filter chooses its name.
    #[test]
    fn named_filters_follow_the_profiles_below_a_separator() {
        let mut p = ProfilePicker::new(
            1,
            vec!["default".into(), "loud".into()],
            vec!["INFO".into()],
        );
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        p.render(area, &mut buf);
        let row = |y: u16| -> String { (0..40).map(|x| buf[(x, y)].symbol()).collect() };
        // Three names and a separator: 6 rows tall, so it starts at row 2.
        assert!(row(3).contains("default"), "{}", row(3));
        assert!(row(4).contains("loud"), "{}", row(4));
        assert!(row(5).contains("───"), "{}", row(5));
        assert!(row(6).contains("INFO"), "{}", row(6));

        p.perform(A::PickerDown);
        p.perform(A::PickerDown);
        assert_eq!(
            p.perform(A::PickerChoose),
            PickerOutcome::Chosen("INFO".into())
        );
    }

    /// A list taller than the area scrolls to keep the selection visible,
    /// and the borders count the rows hidden above and below (#539). The
    /// shape is a real set's: 4 profiles and 30 named filters, 35 rows.
    #[test]
    fn a_long_list_scrolls_and_the_borders_count_the_hidden_rows() {
        let profiles = (0..4).map(|n| format!("p{n}")).collect();
        let filters = (0..30).map(|n| format!("f{n}")).collect();
        let mut p = ProfilePicker::new(1, profiles, filters);
        let area = Rect::new(0, 0, 40, 14);
        let draw = |p: &mut ProfilePicker| {
            let mut buf = Buffer::empty(area);
            p.render(area, &mut buf);
            (0..14)
                .map(|y| (0..40).map(|x| buf[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
        };

        // 12 rows inside the borders: 23 of the 35 are below.
        let rows = draw(&mut p);
        assert!(!rows[0].contains('↑'), "{}", rows[0]);
        assert!(rows[13].contains("↓ 23"), "{}", rows[13]);

        for _ in 0..33 {
            p.perform(A::PickerDown);
        }
        let rows = draw(&mut p);
        assert!(rows[0].contains("↑ 23"), "{}", rows[0]);
        assert!(!rows[13].contains('↓'), "{}", rows[13]);
        assert!(
            rows[12].contains("f29"),
            "the selection is drawn: {}",
            rows[12]
        );

        // Back up one row: the window stays where it is.
        p.perform(A::PickerUp);
        assert_eq!(draw(&mut p)[0], rows[0]);
    }

    /// Width is display columns, not characters: a CJK name occupies two
    /// columns per ideograph, and a box sized by `chars().count()` clips it
    /// (#97, #193).
    #[test]
    fn the_panel_is_sized_in_display_columns() {
        let p = ProfilePicker::new(1, vec!["日本語ログ".into()], vec![]);
        let area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 10,
        };

        // Five ideographs are ten columns, plus two borders and two of padding.
        assert_eq!(p.panel(area).width, 14);
    }

    /// The panel never leaves its area, at any size — including an area smaller
    /// than the box wants, which is what `saturating_sub` is there for (#203).
    #[test]
    fn the_panel_stays_inside_the_area_at_every_size() {
        let p = ProfilePicker::new(1, vec!["a-very-long-profile-name".into()], vec![]);

        for width in 0..40u16 {
            for height in 0..8u16 {
                let area = Rect {
                    x: 3,
                    y: 2,
                    width,
                    height,
                };
                let panel = p.panel(area);

                assert!(panel.width <= area.width, "wider than the area");
                assert!(panel.height <= area.height, "taller than the area");
                assert!(panel.x >= area.x, "left of the area");
                assert!(panel.y >= area.y, "above the area");
                assert!(
                    panel.x + panel.width <= area.x + area.width,
                    "right of the area at width {width}"
                );
                assert!(
                    panel.y + panel.height <= area.y + area.height,
                    "below the area at height {height}"
                );
            }
        }
    }
}
