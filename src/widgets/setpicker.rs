//! The set picker (#284): every known set, to list or unlist.
//!
//! `L` opens it over the whole window, not over one pane. The filter pane
//! is what a change here alters, and a picker that left it visible and
//! unchanged while the user toggled would look like a bug.
//!
//! One row per known set except the scratch set, in alphabetical order —
//! the pane's `priority` order is a thing the user reads there, not here,
//! and a set the user seldom uses is found by name. The checkbox means
//! **listed**, never enabled: a listed set that is disabled is checked too.
//!
//! Changes are staged. Space flips a row's checkbox and nothing else;
//! `Enter` hands `App` the rows that differ from what they were, and `Esc`
//! hands it nothing. So the picker never touches `ActiveFilters`, and a
//! cancelled session leaves no trace.

use crate::filter::FilterSet;
use ratatui::prelude::{Buffer, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, Clear};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// What one key did to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SetPickerOutcome {
    /// Still open — the selection moved, a row flipped, or the key meant
    /// nothing.
    Open,
    /// `Esc`: closed, every change discarded.
    Cancelled,
    /// `Enter`: closed. Each `(set, listed)` is a set whose checkbox now
    /// differs from its listed state when the picker opened, by set index.
    Applied(Vec<(usize, bool)>),
}

#[derive(Debug)]
struct Entry {
    set: usize,
    name: String,
    description: String,
    /// Listed when the picker opened.
    was: bool,
    /// The checkbox as it stands now.
    listed: bool,
}

#[derive(Debug)]
pub(crate) struct SetPicker {
    entries: Vec<Entry>,
    selected: usize,
    /// The first entry drawn. `render` moves it so the selection stays in
    /// view, which is what makes a long list scroll.
    top: usize,
}

impl SetPicker {
    /// A picker over `sets`, in `ActiveFilters::sets` order. Set 0, the
    /// scratch set, is always listed and has no row. The list is never
    /// empty, because the built-in set is always known.
    pub(crate) fn new(sets: &[FilterSet]) -> Self {
        let mut entries: Vec<Entry> = sets
            .iter()
            .enumerate()
            .skip(1)
            .map(|(set, meta)| Entry {
                set,
                name: meta.name.clone(),
                description: meta.description.clone().unwrap_or_default(),
                was: meta.listed,
                listed: meta.listed,
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            entries,
            selected: 0,
            top: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    /// The names in row order, each with its checkbox, for tests.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<(String, bool)> {
        self.entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.listed))
            .collect()
    }

    /// Carry out a `Scope::Sets` action. `App`'s modal dispatch is the only
    /// caller, and swallows a key that resolves to nothing there, as it
    /// does for the profile picker.
    pub(crate) fn perform(&mut self, action: crate::keymap::ActionId) -> SetPickerOutcome {
        use crate::keymap::ActionId as A;
        match action {
            A::SetsDown => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            }
            A::SetsUp => self.selected = self.selected.saturating_sub(1),
            A::SetsToggle => {
                if let Some(entry) = self.entries.get_mut(self.selected) {
                    entry.listed = !entry.listed;
                }
            }
            A::SetsApply => {
                return SetPickerOutcome::Applied(
                    self.entries
                        .iter()
                        .filter(|entry| entry.listed != entry.was)
                        .map(|entry| (entry.set, entry.listed))
                        .collect(),
                );
            }
            A::SetsCancel => return SetPickerOutcome::Cancelled,
            // `resolve(Scope::Sets, ..)` only ever answers with one of the
            // arms above; stay inert rather than panic if that changes.
            _ => {}
        }
        SetPickerOutcome::Open
    }

    /// Draw the picker over all of `area`.
    ///
    /// `&mut` because it scrolls: the first row drawn follows the
    /// selection, and only the drawn height says how far.
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let block = Block::bordered()
            .title(" Filter sets ")
            .title_bottom(" Space list/unlist · Enter apply · Esc cancel ");
        let inner = block.inner(area);
        block.render(area, buf);

        let height = usize::from(inner.height);
        if height == 0 {
            return;
        }
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + height {
            self.top = self.selected + 1 - height;
        }

        let name_width = self
            .entries
            .iter()
            .map(|entry| UnicodeWidthStr::width(entry.name.as_str()))
            .max()
            .unwrap_or(0);
        // A column of padding each side, inside the border.
        let width = usize::from(inner.width.saturating_sub(2));
        for (y, (index, entry)) in
            (inner.y..inner.bottom()).zip(self.entries.iter().enumerate().skip(self.top))
        {
            let mark = if entry.listed { 'x' } else { ' ' };
            let pad = name_width - UnicodeWidthStr::width(entry.name.as_str());
            let line = format!(
                "[{mark}] {}{}  {}",
                entry.name,
                " ".repeat(pad),
                entry.description
            );
            let style = if index == self.selected {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            buf.set_stringn(
                inner.x + 1,
                y,
                truncated(line.trim_end(), width),
                width,
                style,
            );
        }
    }
}

/// `text` cut to `width` display columns, ending in `…` when it was cut, so
/// a long description says there is more of it.
fn truncated(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{ActiveFilters, test_support::loaded};
    use crate::keymap::ActionId as A;

    fn described(mut set: crate::filter::LoadedSet, text: &str) -> crate::filter::LoadedSet {
        set.description = Some(text.to_string());
        set
    }

    /// `zeta` sorts first in the pane by priority, last here by name.
    fn filters() -> ActiveFilters {
        ActiveFilters::with_sets(
            None,
            &[
                described(loaded("zeta", 10, true, &["z"]), "last by name"),
                loaded("alpha", 90, false, &["a"]),
            ],
        )
    }

    #[test]
    fn rows_are_every_named_set_in_alphabetical_order() {
        let p = SetPicker::new(filters().sets());
        let names: Vec<String> = p.rows().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["alpha", "definitions", "zeta"]);
    }

    /// The checkbox is the listed state: a listed set that is disabled is
    /// checked, and an unlisted one is not.
    #[test]
    fn the_checkbox_means_listed_not_enabled() {
        let mut filters = filters();
        let zeta = filters
            .sets()
            .iter()
            .position(|s| s.name == "zeta")
            .unwrap();
        filters.set_listed(zeta, false);
        let p = SetPicker::new(filters.sets());
        assert_eq!(
            p.rows(),
            [
                ("alpha".to_string(), true),
                ("definitions".to_string(), true),
                ("zeta".to_string(), false),
            ]
        );
    }

    #[test]
    fn space_toggles_and_enter_reports_only_what_changed() {
        let filters = filters();
        let mut p = SetPicker::new(filters.sets());
        assert_eq!(p.perform(A::SetsToggle), SetPickerOutcome::Open);
        p.perform(A::SetsDown);
        p.perform(A::SetsToggle);
        p.perform(A::SetsToggle);
        let alpha = filters
            .sets()
            .iter()
            .position(|s| s.name == "alpha")
            .unwrap();
        assert_eq!(
            p.perform(A::SetsApply),
            SetPickerOutcome::Applied(vec![(alpha, false)])
        );
    }

    #[test]
    fn esc_cancels() {
        let mut p = SetPicker::new(filters().sets());
        p.perform(A::SetsToggle);
        assert_eq!(p.perform(A::SetsCancel), SetPickerOutcome::Cancelled);
    }

    #[test]
    fn j_and_k_move_and_clamp() {
        let mut p = SetPicker::new(filters().sets());
        p.perform(A::SetsUp);
        assert_eq!(p.selected(), 0);
        for _ in 0..5 {
            p.perform(A::SetsDown);
        }
        assert_eq!(p.selected(), 2);
    }

    fn drawn(p: &mut SetPicker, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        p.render(area, &mut buf);
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn it_covers_the_area_with_a_checkbox_name_and_description() {
        let mut p = SetPicker::new(filters().sets());
        let rows = drawn(&mut p, 80, 8);
        assert!(rows[0].contains("Filter sets"), "{}", rows[0]);
        assert!(rows[0].starts_with('┌') && rows[7].starts_with('└'));
        assert!(rows[1].contains("[x] alpha"), "{}", rows[1]);
        assert!(
            rows[2].contains("[x] definitions  Functions"),
            "{}",
            rows[2]
        );
        assert!(
            rows[3].contains("[x] zeta         last by name"),
            "{}",
            rows[3]
        );
    }

    #[test]
    fn a_long_description_is_truncated() {
        let long = "word ".repeat(40);
        let sets =
            ActiveFilters::with_sets(None, &[described(loaded("a", 10, false, &["x"]), &long)]);
        let mut p = SetPicker::new(sets.sets());
        let rows = drawn(&mut p, 40, 6);
        assert!(rows[1].ends_with("… │"), "{}", rows[1]);
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selection_in_view() {
        let sets: Vec<_> = (0..10)
            .map(|n| loaded(&format!("set{n}"), 10, false, &["x"]))
            .collect();
        let filters = ActiveFilters::with_sets(None, &sets);
        let mut p = SetPicker::new(filters.sets());
        // Three rows inside the border.
        for _ in 0..6 {
            p.perform(A::SetsDown);
        }
        let rows = drawn(&mut p, 40, 5);
        assert!(rows[3].contains("set5"), "{rows:?}");
        let selected = &rows[3];
        assert!(
            !rows[1].contains("definitions"),
            "scrolled past the top: {rows:?}"
        );
        assert!(selected.contains("[x] set5"));
        for _ in 0..6 {
            p.perform(A::SetsUp);
        }
        let rows = drawn(&mut p, 40, 5);
        assert!(rows[1].contains("definitions"), "{rows:?}");
    }

    #[test]
    fn truncation_counts_display_columns() {
        assert_eq!(truncated("abcdef", 6), "abcdef");
        assert_eq!(truncated("abcdef", 4), "abc…");
        assert_eq!(truncated("日本語", 4), "日…");
        assert_eq!(truncated("abc", 0), "");
    }
}
