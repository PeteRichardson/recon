//! The `/` search: running a pattern, and stepping from hit to hit.

use super::App;
use super::prompt::Origin;
use super::viewport::Step;
use crate::document::Document;
use crate::widgets::Focus;
use color_eyre::Result;

/// What the status row says when `/` or `n` passed the end of the file to
/// find its hit, and when `N` passed the start. One message for the search
/// and for `n` over interesting lines, since both step through
/// `step_visible`.
pub(super) const WRAPPED_TO_TOP: &str = "wrapped to the top";

pub(super) const WRAPPED_TO_BOTTOM: &str = "wrapped to the bottom";

/// The search: a regular expression the user typed after `/`, or took from
/// the word under the cursor with `*`.
///
/// A search is a motion, not a filter (ADR 0001). It lives here, on the
/// `App`, and not in the `ActiveFilters`: it never changes which lines are
/// visible, does not dim, marks no file in the explorer, does not answer to
/// `!` or `u`, and is not saved with a filter set. What it does is move the
/// cursor to its next hit among the visible lines and highlight the hits in
/// the window.
#[derive(Debug, Clone)]
pub(super) struct Search {
    /// The pattern as typed, for the status row and for `p`.
    pub(super) text: String,
    regex: regex::Regex,
}

impl Search {
    pub(super) fn new(text: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            text: text.to_owned(),
            regex: regex::Regex::new(text)?,
        })
    }

    /// The character column of the first occurrence in `line`, if the line
    /// is a hit. The cursor lands there: on a long line the hit may be far
    /// off the left edge.
    fn first_column(&self, line: &str) -> Option<usize> {
        let found = self.regex.find(line)?;
        Some(line[..found.start()].chars().count())
    }
}

impl App<'_> {
    /// Run a committed `/` pattern against whichever pane has focus, or
    /// against the set picker when the prompt opened over it.
    ///
    /// In the explorer, Enter sets the filename search and keeps the row
    /// the typing reached, so `n`/`N` repeat it from there. In the file
    /// view and the filter pane, `/` sets the search and moves to its first
    /// hit.
    pub(super) fn run_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        // The set picker: as the explorer, nothing moves on Enter, and a
        // pattern no row matches is reported.
        if let Some(Origin::Sets(origin)) = self
            .prompt
            .as_ref()
            .and_then(|prompt| prompt.origin.as_ref())
        {
            let row = origin.row;
            let matcher = regex::Regex::new(pattern)?;
            let mut no_hit = false;
            if let Some(picker) = self.set_picker.as_mut() {
                picker.set_search(Some(matcher));
                no_hit = picker.hit_from(row).is_none();
            }
            if no_hit {
                self.report(&format!("no sets match \"{pattern}\""), false);
            }
            return Ok(());
        }
        match self.focus {
            Focus::Explorer => {
                // Nothing moves: the typing already did. What Enter adds is
                // the report. A pattern no name matches used to close the
                // prompt with nothing moved and nothing said, which a user
                // cannot tell apart from `Esc` (#243); `n`/`N` already
                // report their dead end, so does this one. Checked from the
                // origin, which is where the typing looked from, or from the
                // selection when there was no prompt.
                let matcher = regex::Regex::new(pattern)?;
                let from = match self
                    .prompt
                    .as_ref()
                    .and_then(|prompt| prompt.origin.as_ref())
                {
                    Some(Origin::Explorer(origin)) => origin.entry,
                    _ => self.explorer.selected_entry().unwrap_or(0),
                };
                self.explorer.set_search(Some(matcher));
                if self.explorer.hit_from(from).is_none() {
                    self.report(&format!("no filenames match \"{pattern}\""), false);
                }
                Ok(())
            }
            // The filter pane forwards view-shaped keys to the view (#120):
            // a search started there is the same search.
            Focus::View | Focus::Filters => self.apply_search(pattern),
        }
    }

    /// Set the search and move to its first hit: the first visible line the
    /// pattern matches, from the cursor line — that line included — wrapping
    /// once to the top and saying so. The cursor lands on the column of the
    /// first occurrence. With no hit in the file, the status row says so and
    /// nothing moves.
    ///
    /// The search looks only at the visible lines, so in hide mode it finds
    /// nothing among the lines the filters removed, and it changes none of
    /// them: the visible set, the gutter numbers and the gaps are exactly
    /// what they were. Only the highlight is new, and `apply_view` paints
    /// that from `self.search` on every pass.
    ///
    /// The truncated-preview promotion comes first, as it does for `n` (see
    /// `promote_truncated_preview`). Without it a pattern that only occurs
    /// beyond a large preview's cap would be reported as having no hit. A
    /// peek is left alone: the user is searching the plain file they asked
    /// to see, and unlike a cross-file `n` the search never leaves it.
    ///
    /// A pattern that will not compile is reported and changes nothing, so
    /// the prompt can stay open over an intact previous search.
    fn apply_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let search = Search::new(pattern)?;
        self.search = Some(search);
        self.promote_truncated_preview();
        let from = self.view.cursor_visible_row();
        let Some((row, column, wrapped)) = self.hit_from(from) else {
            self.report_no_hit();
            self.repaint_highlight();
            return Ok(());
        };
        self.jump_to_visible_row(row);
        self.view.set_cursor_col(column);
        if wrapped {
            self.report(WRAPPED_TO_TOP, false);
        }
        Ok(())
    }

    /// Paint the search's highlight (or its absence) without moving anything.
    ///
    /// `apply_view` re-applies the highlight from `self.search` on every
    /// pass, and a jump runs through it, so this is only for the paths that
    /// set or clear the search and jump nowhere. Never after a jump: a jump
    /// queues a landing row, and a second `apply_view` on top of it moves
    /// the cursor. Cheap: nothing is re-evaluated, and the buffer is not
    /// rebuilt.
    pub(super) fn repaint_highlight(&mut self) {
        self.apply_view(self.cursor_source());
    }

    /// `n`/`N` while a search is set: the next (previous) hit line among the
    /// visible lines, wrapping within this file and saying so. One stop per
    /// line, on the first occurrence's column. Never crosses files: a search
    /// is a motion within the file it was made in, and `n` with no search is
    /// the key that walks the filters' files. So, unlike that `n`, it has no
    /// reason to end a peek first.
    pub(super) fn step_hit(&mut self, backwards: bool) {
        self.promote_truncated_preview();
        let Some(search) = self.search.clone() else {
            return;
        };
        let is_hit = |document: &Document, row: usize| {
            document
                .source_at(row)
                .and_then(|source| document.lines().get(source))
                .is_some_and(|line| search.regex.is_match(line))
        };
        match self.step_visible(backwards, is_hit) {
            Step::Nothing => {
                self.report_no_hit();
                self.repaint_highlight();
            }
            step => {
                self.land_on_first_occurrence(&search);
                if step == Step::Wrapped {
                    self.report(
                        if backwards {
                            WRAPPED_TO_BOTTOM
                        } else {
                            WRAPPED_TO_TOP
                        },
                        false,
                    );
                }
            }
        }
    }

    /// Put the cursor on the first occurrence of `search` in the line it is
    /// on. A no-op when the line is not a hit, which the stepping above rules
    /// out.
    fn land_on_first_occurrence(&mut self, search: &Search) {
        let row = self.view.cursor_visible_row();
        let column = self
            .document
            .source_at(row)
            .and_then(|source| self.document.lines().get(source))
            .and_then(|line| search.first_column(line));
        if let Some(column) = column {
            self.view.set_cursor_col(column);
        }
    }

    /// `no hit for /pattern`: what `/` and `n` say when the file has none.
    fn report_no_hit(&mut self) {
        let text = self
            .search
            .as_ref()
            .map_or_else(String::new, |search| format!("no hit for /{}", search.text));
        self.report(&text, false);
    }

    /// The first hit at or after visible row `from`, wrapping once: its
    /// visible row, the column of the first occurrence, and whether the
    /// walk passed the end of the file to reach it.
    ///
    /// Row `from` itself is considered first, so a hit on it is found
    /// without a wrap — the difference from `n`, which considers the
    /// cursor's row last.
    pub(super) fn hit_from(&self, from: usize) -> Option<(usize, usize, bool)> {
        let search = self.search.as_ref()?;
        let visible = self.document.visible();
        let len = visible.len();
        if len == 0 {
            return None;
        }
        let from = from.min(len - 1);
        (0..len).find_map(|step| {
            let row = (from + step) % len;
            let line = self.document.lines().get(visible[row])?;
            let column = search.first_column(line)?;
            Some((row, column, from + step >= len))
        })
    }
}
