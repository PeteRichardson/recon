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

    /// Rebuild the file view's window if the cursor has left the middle third
    /// of the one it is holding (#7).
    ///
    /// Called after every event that can move the cursor. Cheap when nothing is
    /// owed: two comparisons against bounds the view already knows. The rebuild
    /// itself goes through `apply_view`, which recomputes the window from the
    /// cursor's new position, so this decides only *whether*, never *where*.
    pub(crate) fn ensure_window(&mut self) {
        let visible_len = self.document.visible().len();
        let needed = !widgets::fileview::window_holds(
            visible_len,
            self.view.window_start(),
            self.view.window_end(),
            self.view.cursor_visible_row(),
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
        let (window_start, window_end) = widgets::fileview::window_for(
            self.document.visible().len(),
            self.file_view_window_height(),
            row,
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
            .map(|search| search.pattern.as_str().to_string());

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
        // `let _ =` accepts that cosmetic failure mode deliberately, rather
        // than escalating it into one this codebase's own tests are better
        // suited to catch.
        let _ = view.set_highlight(highlight.as_deref());
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
            .find(|&index| matches!(verdicts[index], Verdict::Included(_) | Verdict::Searched))
    }

    /// Move the file view's cursor to the next interesting line, if there is
    /// one. Quiet when there is not.
    pub(crate) fn step_to_interesting(&mut self, backwards: bool) {
        let Some(target) = self.next_interesting(backwards) else {
            return;
        };
        let Some(row) = self.document.visible_position(target) else {
            return;
        };
        self.place_cursor_on_visible_row(row);
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
