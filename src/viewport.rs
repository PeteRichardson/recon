//! Translation between the three coordinate spaces the file view lives in.
//!
//! Split out of `impl App` (#74). The methods are unchanged — this is a move
//! along a seam that already existed, not a rewrite.
//!
//! recon holds the same file in three different numbering schemes at once, and
//! almost every bug in this area has been a confusion between two of them:
//!
//! | Space | Index into | Named by |
//! |---|---|---|
//! | **source line** | every line of the file as read | `Document::lines` |
//! | **visible row** | the lines surviving the filters | `Document::visible` |
//! | **buffer row** | the window `TextArea` actually holds | `FileView::show_window` |
//!
//! A filter changes the second without touching the first; scrolling changes
//! the third without touching either. `apply_view` is where all three are
//! reconciled, which is why it is the longest function here and carries the
//! most commentary.
//!
//! #74 called this "the hardest part of `App` to hold in your head, and
//! already documented as a unit" — the unit is now a module, so the table
//! above has somewhere to live that isn't a comment buried mid-file.

use crate::App;
use crate::filter::Verdict;
use crate::widgets;
use crossterm::event::KeyCode;

/// The one definition of an interesting line, shared by every query that
/// steps or lands on one.
pub(crate) fn is_interesting(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Included(_) | Verdict::Searched)
}

impl App<'_> {
    /// The visible-set row a long-range file-view key asks for, or `None` if
    /// this key is not one of them.
    ///
    /// | Key | Means |
    /// |---|---|
    /// | `g` / `Home` | the first visible row |
    /// | `G` / `End` | the last visible row |
    /// | `}` | the next blank line below, or the last row |
    /// | `{` | the previous blank line above, or the first row |
    ///
    /// Paragraph moves read `document.lines()`, which costs nothing extra:
    /// this issue removes `TextArea`'s duplicate of the text, not `Document`'s.
    /// Half B (#51) is what makes the text unavailable, and it will have to
    /// answer this differently.
    pub(crate) fn long_range_target(&self, code: KeyCode) -> Option<usize> {
        let visible = self.document.visible();
        if visible.is_empty() {
            return None;
        }
        let last = visible.len() - 1;
        let from = self
            .document
            .visible_position(self.cursor_source())
            .unwrap_or(0);
        let blank = |row: usize| {
            self.document
                .lines()
                .get(visible[row])
                .is_some_and(|line| line.trim().is_empty())
        };
        match code {
            KeyCode::Char('g') | KeyCode::Home => Some(0),
            KeyCode::Char('G') | KeyCode::End => Some(last),
            KeyCode::Char('}') => Some(((from + 1)..=last).find(|&row| blank(row)).unwrap_or(last)),
            KeyCode::Char('{') => Some((0..from).rev().find(|&row| blank(row)).unwrap_or(0)),
            _ => None,
        }
    }

    /// The pane height to size the file view's window against — the area it
    /// last rendered into, or a generous assumption before the first render.
    pub(crate) fn file_view_window_height(&self) -> u16 {
        self.view.window_height()
    }

    /// Rebuild the file view's window if the viewport has eaten into the screen
    /// of slack the window keeps on either side of it (#7, #108).
    ///
    /// Called after every event that can move the cursor. Cheap when nothing is
    /// owed: two comparisons against bounds the view already knows. The rebuild
    /// itself goes through `apply_view`, which recomputes the window from the
    /// cursor's new position, so this decides only *whether*, never *where*.
    ///
    /// The cursor's screen row is part of the question, not just its row in the
    /// visible set: the slack that matters is the buffer beyond the *viewport*,
    /// and the viewport's top edge is `screen_row` rows above the cursor.
    pub(crate) fn ensure_window(&mut self) {
        let visible_len = self.document.visible().len();
        let needed = !widgets::fileview::window_holds(
            visible_len,
            self.view.window_start(),
            self.view.window_end(),
            self.view.cursor_visible_row(),
            usize::from(self.file_view_screen_row()),
        );
        if needed {
            self.apply_view(self.cursor_source());
        }
    }

    /// Which row of the file view pane the cursor is currently drawn on.
    pub(crate) fn file_view_screen_row(&self) -> u16 {
        self.view.cursor_screen_row()
    }

    /// Push the document's current verdicts onto the file view.
    ///
    /// `cursor_source` is where the cursor was *before* whatever changed the
    /// verdicts or the mode — it is remapped here onto the now-current
    /// visible set, since a filter or the hide toggle may have removed the
    /// exact line it was on.
    ///
    /// When nothing is hidden the buffer is the whole document and the gutter
    /// numbers itself. As soon as a line is hidden the buffer holds only the
    /// visible lines, so the gutter must be told each row's *source* number or
    /// it would renumber 1..N and the line numbers would be lies. And when
    /// hiding leaves nothing visible at all, the buffer falls back to a single
    /// blank placeholder row — which must not get a gutter number of its own,
    /// or it reads as "this file has one empty line".
    ///
    /// Rebuilding the buffer (`TextArea::set_lines`) resets the viewport's
    /// scroll position, so it only happens when the visible *set* of rows has
    /// actually changed from what the buffer was last built with. Otherwise a
    /// filter change that matches nothing — or any other no-op change to the
    /// verdicts — would still re-anchor the view to wherever the cursor's row
    /// happens to land in a freshly reset viewport, which reads as the view
    /// jumping a full page for no reason. The gutter numbers and line styles
    /// are cheap and must stay in sync regardless, so those are always
    /// reapplied.
    ///
    /// A rebuild that *does* happen still resets the viewport, so the screen
    /// row the cursor was drawn on is captured before touching anything and
    /// restored once the buffer is back in place — every caller rebuilds
    /// through here, so capturing it inside this method rather than asking
    /// each caller to pass it in (the way `cursor_source` is) means no future
    /// caller can forget and leave the view re-anchoring under it. Nothing
    /// above this point touches the textarea, so capturing before any of it
    /// runs is safe — unlike `cursor_source`, which has to be captured by the
    /// caller before `recompute_visible` destroys the *old* visible list it
    /// is measured against.
    pub(crate) fn apply_view(&mut self, cursor_source: usize) {
        let screen_row = self.file_view_screen_row();
        let cursor_source = self
            .document
            .nearest_visible(cursor_source)
            .unwrap_or(cursor_source);

        let hiding = self.document.visible().len() < self.document.lines().len();
        let nothing_visible = hiding && self.document.visible().is_empty();

        // The window `TextArea` is given, rather than the whole visible set
        // (#7). Sized from the pane height recorded on the previous frame —
        // `apply_view` runs outside `render` and has no area of its own.
        let row = self.document.visible_position(cursor_source).unwrap_or(0);
        // `screen_row` again: the window's slack is measured from the pane's
        // edges, and the cursor is `screen_row` rows below the top one (#108).
        let (window_start, window_end) = widgets::fileview::window_for(
            self.document.visible().len(),
            self.file_view_window_height(),
            row,
            usize::from(screen_row),
        );

        let styles = self
            .document
            .visible_styles_range(&self.filters, window_start, window_end);
        // **Always** supplied now, where this was gated on `hiding`. Ungated
        // the fork falls back to numbering the buffer 1..N, which is right only
        // when the buffer *is* the file — a window starting at visible row
        // 1,000 would be numbered 1, 2, 3. The reason for the old gate ("a
        // vector the length of the file, rebuilt on every navigator arrow key")
        // is gone: the vector is now the length of the window.
        let numbers: Vec<usize> = self.document.visible()[window_start..window_end].to_vec();
        // Still gated on `hiding`: with the whole file on screen every line's
        // successor is the next one, so every mark would be false anyway.
        let group_ends = if hiding {
            self.document
                .visible_group_ends_range(window_start, window_end)
        } else {
            Vec::new()
        };
        // Computed here, alongside `styles`, rather than inside the `view`
        // block below: both read `self.filters`, and grouping them keeps
        // every access to that field on the `&self` side of the borrow.
        // Re-applied on every pass rather than only when the pattern
        // changes: `load`/`preview` replace the textarea outright, dropping
        // whatever pattern it had (see `FileView::set_highlight`), and
        // switching files funnels through `refresh_view` → `apply_view` the
        // same as every filter mutation does. The pattern also tracks the
        // filter's *enabled* flag, so `!` and `space` have to reach it here
        // too, not just `/` and Esc.
        let highlight = self
            .filters
            .search()
            .filter(|search| search.enabled)
            .map(|search| search.predicate.display());

        // `CursorMove::Jump` takes a `u16`, which silently truncates past
        // 65,535 lines and lands the cursor 65,536 lines from its target on a
        // large log. `set_lines` clamps in `usize` and replaces the buffer in
        // the same call, so the row is applied directly rather than jumped to
        // afterwards. (The rendered viewport is still `u16`-limited, though —
        // see `FileView::show_lines_with_cursor`.)
        // Keyed on the window as well as the visible set: scrolling into a new
        // window leaves the visible set untouched and still needs the buffer
        // replaced.
        let rebuild = self.last_visible.as_deref() != Some(self.document.visible())
            || self.last_window != Some((window_start, window_end));
        let lines = if rebuild {
            self.last_visible = Some(self.document.visible().to_vec());
            self.last_window = Some((window_start, window_end));
            Some(self.document.visible_lines_range(window_start, window_end))
        } else {
            None
        };

        let view = &mut self.view;
        if let Some(lines) = lines {
            // `row` is an index into the visible set; the buffer now starts at
            // `window_start`, so the cursor's row *within the buffer* is the
            // difference.
            view.show_window(lines, window_start, row.saturating_sub(window_start));
        }
        view.set_line_numbers(numbers);
        view.set_group_ends(group_ends);
        view.set_line_styles(styles);
        view.set_gutter_blank(nothing_visible);
        // `highlight`, when `Some`, was `Regex::as_str()` on a pattern that
        // `ActiveFilters::set_search` already compiled once; re-parsing the same
        // string here is deterministic and cannot fail today. Even so, this
        // is the hottest path in the app — every filter mutation and every
        // navigator preview reaches it — so a future regression here should
        // cost a stale highlight, not a panic that takes the whole TUI down.
        // Swallowing it accepts that cosmetic failure mode deliberately, rather
        // than escalating it into one this codebase's own tests are better
        // suited to catch — but it is logged on the way past (#83). "Cannot
        // fail today" is a statement about today, and the symptom if it ever
        // does is a highlight that quietly stops updating, with nothing
        // anywhere to say why.
        if let Err(err) = view.set_highlight(highlight.as_deref()) {
            log::warn!(
                "search highlight not applied for {:?}: {err}",
                highlight.as_deref().unwrap_or(""),
            );
        }
        // Only a rebuild resets the viewport, so only a rebuild needs the
        // cursor nudged back onto `screen_row` — the whole point of
        // `scroll_cursor_to_row` is undoing that reset. Requesting it
        // unconditionally used to queue a no-op nudge on every call that hid
        // nothing new (every navigator arrow key goes through `Preview` →
        // `refresh_view` → here, whether or not a filter is even defined),
        // and `apply_pending_scroll` pays for that with a full scratch
        // render of the file view on the very next frame regardless of
        // whether there was anything to correct.
        if rebuild {
            view.scroll_cursor_to_row(screen_row);
        }
    }

    /// The source line the cursor is on, mapped through the *current* visible
    /// list before it is rebuilt.
    pub(crate) fn cursor_source(&self) -> usize {
        // Through the window: the textarea's own cursor row indexes the
        // buffer, which is a slice of the visible set (#7).
        let row = self.view.cursor_visible_row();
        self.document.source_at(row).unwrap_or(row)
    }

    /// The identifier-shaped word under the cursor, for `*` (#120 §13).
    /// `None` on whitespace, punctuation, past the end of the line, or when
    /// nothing is visible at all (hide mode with an empty visible set):
    /// `source_at` directly, not `cursor_source`, which falls back to row 0
    /// in that case and would silently report row 0's word instead.
    pub(crate) fn word_under_cursor(&self) -> Option<String> {
        let row = self.view.cursor_visible_row();
        let source = self.document.source_at(row)?;
        let line = self.document.lines().get(source)?;
        let col = self.view.cursor_col();
        if let Some(word) = word_around(line, col) {
            return Some(word.to_owned());
        }
        // The textarea's End puts the cursor one past the last character;
        // retry one column back so `$` then `*` finds the last word rather
        // than reporting none.
        let len = line.chars().count();
        if col == len && col > 0 {
            return word_around(line, col - 1).map(str::to_owned);
        }
        None
    }

    /// The next source line matched by an enabled including filter or by the
    /// live search, walking from the cursor and wrapping once.
    ///
    /// Line-oriented rather than span-oriented: a line with three matches is
    /// one stop. `recon` is a line-focused tool, and the alternative — three
    /// stops on a search hit but one on a filter hit — is a distinction that
    /// cannot be explained without explaining the implementation.
    ///
    /// An interesting line is always visible in both modes: `Excluded` is the
    /// only verdict that hides a line in `Dimmed`, and it is never
    /// interesting. So the caller can map through `visible_position` without
    /// a fallback for "the target is hidden".
    pub(crate) fn next_interesting(&self, backwards: bool) -> Option<usize> {
        let verdicts = self.document.verdicts();
        let len = verdicts.len();
        if len == 0 {
            return None;
        }
        let from = self.cursor_source();
        // 1..=len, so the line the cursor is on is considered last: `n` moves
        // off it if anything else matches, and stays put if it is the only
        // interesting line in the file.
        (1..=len)
            .map(|step| {
                if backwards {
                    (from + len - step) % len
                } else {
                    (from + step) % len
                }
            })
            .find(|&index| is_interesting(&verdicts[index]))
    }

    /// `next_interesting` without the wrap: `None` once the cursor is past the
    /// last interesting line (or before the first, going backwards). This is
    /// how `n` learns it has finished the file and should move to the next
    /// one rather than circle back.
    pub(crate) fn next_interesting_strict(&self, backwards: bool) -> Option<usize> {
        let verdicts = self.document.verdicts();
        let from = self.cursor_source();
        if backwards {
            (0..from)
                .rev()
                .find(|&index| is_interesting(&verdicts[index]))
        } else {
            (from + 1..verdicts.len()).find(|&index| is_interesting(&verdicts[index]))
        }
    }

    /// The first interesting line of the file — or the last, when
    /// `from_end`. Where a cross-file step lands.
    pub(crate) fn first_interesting(&self, from_end: bool) -> Option<usize> {
        let verdicts = self.document.verdicts();
        if from_end {
            (0..verdicts.len())
                .rev()
                .find(|&index| is_interesting(&verdicts[index]))
        } else {
            (0..verdicts.len()).find(|&index| is_interesting(&verdicts[index]))
        }
    }

    /// How many lines are interesting — the count `--emit`'s summary reports
    /// as "N match" (#143). The same definition `n` steps by.
    pub(crate) fn interesting_count(&self) -> usize {
        self.document
            .verdicts()
            .iter()
            .filter(|verdict| is_interesting(verdict))
            .count()
    }

    /// Put the cursor on source line `target`, bringing the window with it.
    /// Quiet when the line is not visible in the current mode.
    pub(crate) fn land_on(&mut self, target: usize) {
        let Some(row) = self.document.visible_position(target) else {
            return;
        };
        self.jump_to_visible_row(row);
    }

    /// `place_cursor_on_visible_row` for a *jump* — `n`/`N`, `g`/`G`,
    /// `{`/`}`, a cross-file landing — which also decides where on the pane
    /// the target lands. A target the pane already shows stays where
    /// it is; one it does not is centred, or with `center_jumps` off scrolled
    /// in by the minimum onto the scroll margin's edge. See
    /// `FileView::jump_landing_row`.
    ///
    /// Requested through `land_cursor_on_row`, which replaces any restore
    /// already queued — by the `apply_view` below, or by the file load that
    /// precedes a cross-file landing in the same keypress (#192). A
    /// restore's row is the pre-jump screen row, meaningful for a filter
    /// toggle and not for a jump.
    pub(crate) fn jump_to_visible_row(&mut self, row: usize) {
        if let Some(screen_row) = self.view.jump_landing_row(row, self.center_jumps) {
            self.view.land_cursor_on_row(screen_row);
        }
        self.place_cursor_on_visible_row(row);
    }

    /// Move the file view's cursor to the next interesting line, wrapping, if
    /// there is one. Quiet when there is not.
    pub(crate) fn step_to_interesting(&mut self, backwards: bool) {
        if let Some(target) = self.next_interesting(backwards) {
            self.land_on(target);
        }
    }

    /// Put the cursor on `row` of the **visible set**, bringing the window with
    /// it (#7).
    ///
    /// Every jump that can travel further than a page goes through here —
    /// `n`/`N`, and the four keys intercepted in `handle_event` (`g`, `G`, `}`,
    /// `{`). They all share one failure mode without it: `row` indexes the
    /// visible set, `FileView::set_cursor_row` indexes the *buffer*, and a
    /// windowed buffer holds three screens. Handing 50,050 to a 600-row buffer
    /// silently clamps to row 599, landing the cursor nowhere near the hit and
    /// reporting no error at all.
    ///
    /// `apply_view` is what moves the window: it sizes one around the row it is
    /// given, so calling it with the *target's* source line guarantees the
    /// buffer contains the target before the cursor is placed. The explicit
    /// `set_cursor_row` afterwards is still needed — `apply_view` only places
    /// the cursor when it actually rebuilds, and a target already inside the
    /// current window rebuilds nothing.
    pub(crate) fn place_cursor_on_visible_row(&mut self, row: usize) {
        let source = self.document.source_at(row).unwrap_or(row);
        self.apply_view(source);
        // One reach for the pane, not two. This read `window_start` through an
        // immutable scan and then wrote the cursor through a separate mutable
        // one, re-matching a variant the second scan's predicate had already
        // proven (#89).
        let start = self.view.window_start();
        self.view.set_cursor_row(row.saturating_sub(start));
    }
}

/// The maximal run of `[A-Za-z0-9_]` that contains character `col` of
/// `line`. This is vim's default `iskeyword` narrowed to ASCII: it keeps a
/// mangled `_ZN4core3fmt9Formatter3pad17hE` whole and stops at `::`, `.`
/// and `(`. `col` is a character index, matching what the textarea's
/// cursor reports, not a byte offset.
fn word_around(line: &str, col: usize) -> Option<&str> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let &(_, at) = chars.get(col)?;
    if !is_word(at) {
        return None;
    }
    let start = chars[..col]
        .iter()
        .rposition(|&(_, c)| !is_word(c))
        .map_or(0, |i| i + 1);
    let end = chars[col..]
        .iter()
        .position(|&(_, c)| !is_word(c))
        .map_or(chars.len(), |i| col + i);
    let byte_start = chars[start].0;
    let byte_end = chars.get(end).map_or(line.len(), |&(b, _)| b);
    Some(&line[byte_start..byte_end])
}

#[cfg(test)]
mod tests {
    use super::word_around;

    #[test]
    fn a_word_is_a_run_of_identifier_characters_around_the_column() {
        assert_eq!(word_around("foo::bar(x)", 0), Some("foo"));
        assert_eq!(
            word_around("foo::bar(x)", 2),
            Some("foo"),
            "last char of the word"
        );
        assert_eq!(word_around("foo::bar(x)", 5), Some("bar"));
        assert_eq!(
            word_around("foo::bar(x)", 6),
            Some("bar"),
            "middle of the word"
        );
        assert_eq!(word_around("foo::bar(x)", 9), Some("x"));
    }

    #[test]
    fn a_mangled_name_stays_whole_and_stops_at_punctuation() {
        let line = "_ZN4core3fmt9Formatter3pad17hE::call(a.b)";
        assert_eq!(
            word_around(line, 10),
            Some("_ZN4core3fmt9Formatter3pad17hE")
        );
        assert_eq!(word_around(line, 37), Some("a"), "stops at the dot");
    }

    #[test]
    fn whitespace_punctuation_and_past_the_end_have_no_word() {
        assert_eq!(word_around("foo::bar", 3), None, "on a colon");
        assert_eq!(word_around("a  b", 1), None, "on a space");
        assert_eq!(word_around("abc", 3), None, "past the end");
        assert_eq!(word_around("", 0), None);
    }

    #[test]
    fn the_column_counts_characters_not_bytes() {
        // Two multi-byte chars before the word: byte offsets would miss it.
        assert_eq!(word_around("éé foo", 3), Some("foo"));
        assert_eq!(
            word_around("éé foo", 0),
            None,
            "é is not an ASCII word char"
        );
    }
}
