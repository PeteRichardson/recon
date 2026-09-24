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
//!
//! `/` searches the rows (#285): a regex over the name and the description,
//! the same as every other `/` in recon. It is a motion (ADR 0001): it moves
//! the selection and highlights the rows it matches, and never hides one.
//! `App` owns the prompt, its origin and its history; the picker owns the
//! search that is set, and `n`/`N`.

use crate::filter::FilterSet;
use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style, Widget};
use ratatui::widgets::{Block, Clear};
use regex::Regex;
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
    /// `/`: `App` opens a search prompt over the picker, which stays open.
    Search,
    /// `n` or `N` with a search set that no row matches.
    NoHit,
}

/// The rows the search matches: the navigator's `MATCH_STYLE`, so a hit
/// looks the same in both lists.
const MATCH_STYLE: Style = Style::new().fg(Color::Yellow);

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
    /// The search `/` set, if any.
    search: Option<Regex>,
}

impl Entry {
    /// Whether `search` matches the name or the description.
    fn is_hit(&self, search: &Regex) -> bool {
        search.is_match(&self.name) || search.is_match(&self.description)
    }
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
            search: None,
        }
    }

    /// The selected row.
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    /// Select `row`, clamped to the last row.
    pub(crate) fn select(&mut self, row: usize) {
        self.selected = row.min(self.entries.len().saturating_sub(1));
    }

    /// Set the search, or clear it with `None`, without moving.
    pub(crate) fn set_search(&mut self, search: Option<Regex>) {
        self.search = search;
    }

    /// The search that is set, for a prompt to keep and put back on Esc.
    pub(crate) fn search(&self) -> Option<Regex> {
        self.search.clone()
    }

    /// The first row at or after `from` that the search matches, wrapping
    /// once. `None` with no search set or no hit. Row `from` is considered
    /// first, so the typing does not move off a row that still matches.
    pub(crate) fn hit_from(&self, from: usize) -> Option<usize> {
        let search = self.search.as_ref()?;
        let count = self.entries.len();
        (0..count)
            .map(|step| (from + step) % count)
            .find(|&row| self.entries[row].is_hit(search))
    }

    /// `n`/`N`: select the next (previous) row the search matches, wrapping,
    /// with the selected row considered last. `NoHit` when the search
    /// matches no row; nothing moves then. Nothing at all with no search.
    fn step_hit(&mut self, backwards: bool) -> SetPickerOutcome {
        let Some(search) = self.search.as_ref() else {
            return SetPickerOutcome::Open;
        };
        let count = self.entries.len();
        let found = (1..=count)
            .map(|offset| {
                if backwards {
                    (self.selected + count - offset) % count
                } else {
                    (self.selected + offset) % count
                }
            })
            .find(|&row| self.entries[row].is_hit(search));
        match found {
            Some(row) => {
                self.selected = row;
                SetPickerOutcome::Open
            }
            None => SetPickerOutcome::NoHit,
        }
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
            A::SetsSearch => return SetPickerOutcome::Search,
            A::SetsHitNext => return self.step_hit(false),
            A::SetsHitPrev => return self.step_hit(true),
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
            .title_bottom(" Space list/unlist · / search · Enter apply · Esc cancel ");
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
            let mut style = if self
                .search
                .as_ref()
                .is_some_and(|search| entry.is_hit(search))
            {
                MATCH_STYLE
            } else {
                Style::default()
            };
            if index == self.selected {
                style = style.add_modifier(Modifier::REVERSED);
            }
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

    /// Rows `alpha`, `definitions`, `zeta`; only `zeta` has a description
    /// of its own.
    fn searched(p: &mut SetPicker, pattern: &str) {
        p.set_search(Some(Regex::new(pattern).unwrap()));
    }

    #[test]
    fn the_search_matches_the_name_or_the_description() {
        let mut p = SetPicker::new(filters().sets());
        searched(&mut p, "^alp");
        assert_eq!(p.hit_from(0), Some(0));
        searched(&mut p, "last by");
        assert_eq!(p.hit_from(0), Some(2), "the description was not searched");
        searched(&mut p, "nothing like it");
        assert_eq!(p.hit_from(0), None);
    }

    #[test]
    fn hit_from_starts_at_the_row_and_wraps() {
        let mut p = SetPicker::new(filters().sets());
        searched(&mut p, "a");
        assert_eq!(p.hit_from(2), Some(2), "the origin row was not first");
        searched(&mut p, "alpha");
        assert_eq!(p.hit_from(1), Some(0), "the search did not wrap");
    }

    #[test]
    fn n_and_big_n_step_between_hits_and_wrap() {
        let mut p = SetPicker::new(filters().sets());
        searched(&mut p, "^(alpha|zeta)$");
        assert_eq!(p.perform(A::SetsHitNext), SetPickerOutcome::Open);
        assert_eq!(p.selected(), 2);
        p.perform(A::SetsHitNext);
        assert_eq!(p.selected(), 0, "n did not wrap");
        p.perform(A::SetsHitPrev);
        assert_eq!(p.selected(), 2, "N did not wrap");
        p.perform(A::SetsHitPrev);
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn n_with_no_hit_says_so_and_stays() {
        let mut p = SetPicker::new(filters().sets());
        p.select(1);
        assert_eq!(p.perform(A::SetsHitNext), SetPickerOutcome::Open);
        assert_eq!(p.selected(), 1, "n moved with no search set");
        searched(&mut p, "nothing like it");
        assert_eq!(p.perform(A::SetsHitNext), SetPickerOutcome::NoHit);
        assert_eq!(p.selected(), 1);
    }

    #[test]
    fn slash_asks_for_a_prompt() {
        let mut p = SetPicker::new(filters().sets());
        assert_eq!(p.perform(A::SetsSearch), SetPickerOutcome::Search);
    }

    /// A hit is highlighted and no row is hidden.
    #[test]
    fn hits_are_highlighted_and_every_row_stays() {
        let mut p = SetPicker::new(filters().sets());
        searched(&mut p, "zeta");
        let area = Rect::new(0, 0, 60, 6);
        let mut buf = Buffer::empty(area);
        p.render(area, &mut buf);
        let fg = |y: u16| buf[(6, y)].fg;
        assert_eq!(fg(3), Color::Yellow, "the hit was not highlighted");
        assert_ne!(fg(2), Color::Yellow, "a row that is no hit was highlighted");
        let rows = drawn(&mut p, 60, 6);
        assert!(rows[1].contains("alpha") && rows[2].contains("definitions"));
    }

    #[test]
    fn truncation_counts_display_columns() {
        assert_eq!(truncated("abcdef", 6), "abcdef");
        assert_eq!(truncated("abcdef", 4), "abc…");
        assert_eq!(truncated("日本語", 4), "日…");
        assert_eq!(truncated("abc", 0), "");
    }
}
