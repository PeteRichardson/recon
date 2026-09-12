//! Visual mode and the yank (#67): selecting text in the file view and
//! putting it on the clipboard.
//!
//! **A selection is anchored to document lines, not screen rows.** The anchor
//! is a *source* line and character column; the cursor is the other end, as
//! it already is. Visibility is applied here, at yank time and at paint time,
//! never stored — so `u`, `!`, `space` and the digit keys keep working
//! mid-selection and none of them can invalidate it. They only change what
//! the yank will contain: **every line in `[anchor, cursor]` that is currently
//! visible**. Hide mode skips the hidden lines and `u` reveals them into the
//! selection; dimmed lines are visible and so are included. One rule, no
//! per-mode special case. See `docs/specs/2026-09-07-visual-mode-and-yank-design.md`.
//!
//! The two functions at the bottom are pure and take the document's pieces
//! rather than the document, so they can be tested on a handful of lines
//! without an `App` — and so `--emit` (#143) can reuse the yank if it ever
//! wants a selection.

use crate::App;
use crate::widgets::Focus;

/// The fixed end of a selection in progress. The moving end is the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Visual {
    /// Source line of the anchor.
    pub anchor: usize,
    /// Character column of the anchor. Ignored while `linewise`.
    pub col: usize,
    /// `V` rather than `v`: whole lines, whichever columns the ends are on.
    pub linewise: bool,
}

/// One end of a selection, as `(source line, character column)`. Ordered
/// lexicographically, which is what makes "the earlier end" one comparison.
type End = (usize, usize);

/// What a yank produced: the text, and how many lines it spans, for the
/// status row.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Yank {
    pub text: String,
    pub lines: usize,
}

/// The visible lines of `[anchor, cursor]`, ordered, with the columns each
/// end contributes: the first line starts at the earlier end's column and
/// the last stops after the later end's, unless the selection is line-wise
/// or that line is hidden — a hidden end contributes no column, so the
/// first visible line after it is taken whole.
///
/// Yields `(visible row, source line, start col, end col)` with the end
/// exclusive and both clamped to the line. The one walk both the yank and
/// the painting are built on, so the two cannot disagree about which lines
/// a selection covers.
fn visible_segments<'a>(
    lines: &'a [String],
    visible: &'a [usize],
    visual: &Visual,
    cursor: End,
) -> impl Iterator<Item = (usize, usize, usize, usize)> + 'a {
    let anchor: End = (visual.anchor, visual.col);
    let (lo, hi) = if anchor <= cursor {
        (anchor, cursor)
    } else {
        (cursor, anchor)
    };
    let linewise = visual.linewise;
    let first = visible.partition_point(|&source| source < lo.0);
    let last = visible.partition_point(|&source| source <= hi.0);
    (first..last).map(move |row| {
        let source = visible[row];
        let len = lines.get(source).map_or(0, |line| line.chars().count());
        let start = if !linewise && source == lo.0 {
            lo.1.min(len)
        } else {
            0
        };
        // Inclusive of the character under the cursor, as vim's `v` is.
        let end = if !linewise && source == hi.0 {
            (hi.1 + 1).min(len)
        } else {
            len
        };
        (row, source, start, end.max(start))
    })
}

/// The text a yank of this selection copies, or `None` when no line of the
/// range is visible.
///
/// Line-wise text ends in a newline, as vim's does, so pasting it into a
/// file lands whole lines; character-wise text has none, so a selected word
/// pastes into a prompt as a word.
pub(crate) fn yank_text(
    lines: &[String],
    visible: &[usize],
    visual: &Visual,
    cursor: End,
) -> Option<Yank> {
    let mut text = String::new();
    let mut count = 0;
    for (_, source, start, end) in visible_segments(lines, visible, visual, cursor) {
        if count > 0 {
            text.push('\n');
        }
        if let Some(line) = lines.get(source) {
            text.extend(line.chars().skip(start).take(end - start));
        }
        count += 1;
    }
    if count == 0 {
        return None;
    }
    if visual.linewise {
        text.push('\n');
    }
    Some(Yank { text, lines: count })
}

/// Where the selection is painted, in **visible-set** rows and character
/// columns: the first visible row and the column it starts at, and the last
/// visible row and the column (exclusive) it ends at. Rows between are whole.
/// `None` when no line of the range is visible.
pub(crate) fn visible_span(
    lines: &[String],
    visible: &[usize],
    visual: &Visual,
    cursor: End,
) -> Option<((usize, usize), (usize, usize))> {
    let mut segments = visible_segments(lines, visible, visual, cursor);
    let (first_row, _, start, first_end) = segments.next()?;
    let (last_row, last_end) = segments
        .last()
        .map_or((first_row, first_end), |(row, _, _, end)| (row, end));
    Some(((first_row, start), (last_row, last_end)))
}

impl App<'_> {
    /// The cursor as a selection end: its source line and character column.
    fn cursor_end(&self) -> End {
        (self.cursor_source(), self.view.cursor_col())
    }

    /// `v` or `V`: start a selection at the cursor, switch an existing one
    /// between character-wise and line-wise, or — pressed again in the mode
    /// it is already in — end it, as vim does.
    pub(crate) fn toggle_visual(&mut self, linewise: bool) {
        self.visual = match self.visual {
            Some(visual) if visual.linewise == linewise => None,
            Some(visual) => Some(Visual { linewise, ..visual }),
            None => {
                let (anchor, col) = self.cursor_end();
                Some(Visual {
                    anchor,
                    col,
                    linewise,
                })
            }
        };
    }

    /// Start a character-wise selection anchored at `(source, col)` rather
    /// than at the cursor: what a mouse drag and a double-click need, where
    /// the anchor is the press point and the cursor has already moved on.
    pub(crate) fn start_visual_at(&mut self, anchor: usize, col: usize) {
        self.visual = Some(Visual {
            anchor,
            col,
            linewise: false,
        });
    }

    /// `Esc` in visual mode, and every focus change: the selection is
    /// dropped, not parked.
    pub(crate) fn end_visual(&mut self) {
        self.visual = None;
    }

    /// The badge for the status row while a selection is in progress.
    pub(crate) fn visual_badge(&self) -> Option<&'static str> {
        self.visual.map(|visual| {
            if visual.linewise {
                crate::VLINE_BADGE_TEXT
            } else {
                crate::VISUAL_BADGE_TEXT
            }
        })
    }

    /// `y`: copy the visible part of the selection to the clipboard and end
    /// visual mode. With nothing selected, say how to select something —
    /// vim's `y` waits for a motion here, and a key that silently does
    /// nothing is the class of thing #120 §9 removed.
    pub(crate) fn yank(&mut self) {
        let Some(visual) = self.visual.take() else {
            // Only the key is generated, not a full `hint_for` — this line
            // has no verb of its own before "starts a selection", so it
            // calls `label_for` directly (task 8 fix round 1, #199).
            //
            // Owned before `report` takes `self` mutably, and the advice is
            // left off rather than half-said when a `[keymap]` line has left
            // the action with no key to name (#61).
            let key = self
                .keymap
                .label_for(crate::keymap::ActionId::GlobalVisualChar)
                .map(str::to_string);
            let text = match key {
                Some(key) => format!("nothing selected · {key} starts a selection"),
                None => "nothing selected".to_string(),
            };
            self.report(&text, false);
            return;
        };
        let cursor = self.cursor_end();
        let Some(yank) = yank_text(
            self.document.lines(),
            self.document.visible(),
            &visual,
            cursor,
        ) else {
            self.report("nothing visible to yank", false);
            return;
        };
        match self.clipboard.copy(&yank.text) {
            Ok(()) => {
                let what = if yank.lines == 1 && !visual.linewise {
                    let chars = yank.text.chars().count();
                    format!("{chars} character{}", if chars == 1 { "" } else { "s" })
                } else {
                    format!(
                        "{} line{}",
                        yank.lines,
                        if yank.lines == 1 { "" } else { "s" }
                    )
                };
                self.report(&format!("yanked {what}"), false);
            }
            Err(err) => self.report(&format!("clipboard: {err}"), true),
        }
    }

    /// The selection as the file view paints it: buffer rows and character
    /// columns, clipped to the window the buffer holds. `None` outside
    /// visual mode or when nothing in the range is visible.
    ///
    /// An end that falls outside the window is clipped to the window's edge
    /// row, taken whole: the rows between the ends are whole rows, and a
    /// window edge is always between the ends when this clips at all.
    pub(crate) fn painted_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let visual = self.visual?;
        let (start, end) = visible_span(
            self.document.lines(),
            self.document.visible(),
            &visual,
            self.cursor_end(),
        )?;
        let window_start = self.view.window_start();
        let window_end = self.view.window_end();
        if end.0 < window_start || start.0 >= window_end || window_end == 0 {
            return None;
        }
        let start = if start.0 < window_start {
            (0, 0)
        } else {
            (start.0 - window_start, start.1)
        };
        let end = if end.0 >= window_end {
            let row = window_end - 1 - window_start;
            (row, self.view.line_len(row))
        } else {
            (end.0 - window_start, end.1)
        };
        Some((start, end))
    }

    /// The one rule for where a selection can live: in the view, over the
    /// document it was started on. Run after every event, so leaving the
    /// view by any route — `e`, `f`, `Tab`, a click — ends it without each
    /// route having to remember to.
    pub(crate) fn drop_visual_outside_the_view(&mut self) {
        if self.focus != Focus::View {
            self.visual = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| (*line).to_string()).collect()
    }

    fn all(n: usize) -> Vec<usize> {
        (0..n).collect()
    }

    fn charwise(anchor: usize, col: usize) -> Visual {
        Visual {
            anchor,
            col,
            linewise: false,
        }
    }

    fn linewise(anchor: usize) -> Visual {
        Visual {
            anchor,
            col: 0,
            linewise: true,
        }
    }

    #[test]
    fn a_charwise_yank_is_inclusive_of_the_cursor_and_has_no_trailing_newline() {
        let text = lines(&["alpha beta", "gamma delta", "epsilon"]);
        let yank = yank_text(&text, &all(3), &charwise(0, 6), (2, 2)).unwrap();
        assert_eq!(yank.text, "beta\ngamma delta\neps");
        assert_eq!(yank.lines, 3);
    }

    #[test]
    fn a_selection_made_backwards_yanks_the_same_text() {
        let text = lines(&["alpha beta", "gamma delta", "epsilon"]);
        let forward = yank_text(&text, &all(3), &charwise(0, 6), (2, 2)).unwrap();
        let backward = yank_text(&text, &all(3), &charwise(2, 2), (0, 6)).unwrap();
        assert_eq!(forward, backward);
    }

    #[test]
    fn a_linewise_yank_takes_whole_lines_and_ends_in_a_newline() {
        let text = lines(&["alpha beta", "gamma delta", "epsilon"]);
        let yank = yank_text(&text, &all(3), &linewise(1), (2, 3)).unwrap();
        assert_eq!(yank.text, "gamma delta\nepsilon\n");
        assert_eq!(yank.lines, 2);
        let one = yank_text(&text, &all(3), &linewise(1), (1, 3)).unwrap();
        assert_eq!(one.text, "gamma delta\n");
        assert_eq!(one.lines, 1);
    }

    #[test]
    fn hidden_lines_between_the_ends_are_skipped() {
        let text = lines(&["a0", "b1", "a2", "b3", "a4"]);
        // Hide mode showing only the `a` lines.
        let visible = vec![0, 2, 4];
        let yank = yank_text(&text, &visible, &linewise(0), (4, 0)).unwrap();
        assert_eq!(yank.text, "a0\na2\na4\n");
        assert_eq!(yank.lines, 3);
        // Revealing them grows the selection to include them — the anchor is
        // a document line, so the same selection over the full visible set
        // simply yanks more.
        let yank = yank_text(&text, &all(5), &linewise(0), (4, 0)).unwrap();
        assert_eq!(yank.lines, 5);
    }

    #[test]
    fn a_hidden_anchor_contributes_no_column() {
        let text = lines(&["skip me", "keep this", "and this"]);
        let visible = vec![1, 2];
        // The anchor is on the hidden line 0 at column 5; the first visible
        // line is taken from its start rather than from column 5.
        let yank = yank_text(&text, &visible, &charwise(0, 5), (2, 2)).unwrap();
        assert_eq!(yank.text, "keep this\nand");
    }

    #[test]
    fn a_range_with_nothing_visible_yanks_nothing() {
        let text = lines(&["a", "b", "c"]);
        assert_eq!(yank_text(&text, &[2], &linewise(0), (1, 0)), None);
        assert_eq!(yank_text(&text, &[], &linewise(0), (1, 0)), None);
    }

    #[test]
    fn a_cursor_past_the_end_of_the_line_is_clamped() {
        // `$` leaves the cursor one past the last character.
        let text = lines(&["abc"]);
        let yank = yank_text(&text, &all(1), &charwise(0, 1), (0, 3)).unwrap();
        assert_eq!(yank.text, "bc");
        let yank = yank_text(&text, &all(1), &charwise(0, 9), (0, 9)).unwrap();
        assert_eq!(yank.text, "");
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let text = lines(&["éé foo"]);
        let yank = yank_text(&text, &all(1), &charwise(0, 1), (0, 3)).unwrap();
        assert_eq!(yank.text, "é f");
    }

    #[test]
    fn the_painted_span_is_in_visible_rows_with_the_ends_columns() {
        let text = lines(&["a0", "b1", "a2", "b3", "a4"]);
        let visible = vec![0, 2, 4];
        assert_eq!(
            visible_span(&text, &visible, &charwise(0, 1), (4, 0)),
            Some(((0, 1), (2, 1)))
        );
        // A hidden anchor: the first visible row after it, from column 0.
        assert_eq!(
            visible_span(&text, &visible, &charwise(1, 1), (4, 0)),
            Some(((1, 0), (2, 1)))
        );
        // Line-wise: whole rows, whatever the columns.
        assert_eq!(
            visible_span(&text, &visible, &linewise(2), (4, 0)),
            Some(((1, 0), (2, 2)))
        );
        // One line, character-wise.
        assert_eq!(
            visible_span(&text, &all(5), &charwise(1, 0), (1, 1)),
            Some(((1, 0), (1, 2)))
        );
        assert_eq!(visible_span(&text, &[4], &linewise(0), (2, 0)), None);
    }
}
