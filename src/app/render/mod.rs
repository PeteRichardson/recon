//! Drawing `App`: the panes, the status row, and the panels over them.

mod filter_editor;
pub(super) mod status;

use super::App;
use super::prompt::SearchPrompt;
use crate::document::Mode;
use crate::help;
use crate::widgets::Focus;
use ratatui::prelude::{Buffer, Color, Constraint, Layout, Rect, Style, Widget};
use status::{
    AND_BADGE_TEXT, HIDE_BADGE_STYLE, HIDE_BADGE_TEXT, search_badge_text, stale_badge_text,
};
use std::borrow::Cow;
use unicode_width::UnicodeWidthStr;

impl Widget for &mut App<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use Constraint::{Length, Min};
        // The `assert!(self.widgets.len() == 3)` that used to open this method
        // is gone (#87). It ran 60 times a second, in release, to re-check an
        // invariant established once in `App::new`; three named fields make
        // "there are exactly three panes" unrepresentable rather than merely
        // checked, so there is nothing left to assert.

        // The bottom row is reserved unconditionally. It used to be taken only
        // when there was something to put there, which meant the panes resized
        // under the user the moment the first filter appeared and resized back
        // when the last one went. The row now always has the current directory
        // to show, so the objection that answered — spending a row to say
        // nothing — no longer applies.
        let [area, prompt_area] = Layout::vertical([Min(0), Length(1)]).areas(area);
        self.panes_area = area;
        self.status_area = prompt_area;

        // The badge mirrors the mode and nothing else. `▼` answers a
        // different question — "are lines missing from the pane right now?" —
        // and is deliberately false when hide mode is armed with nothing
        // including, because the #36 guard in `Document::recompute_visible` is
        // showing the whole file. Both facts are worth reporting, so they get
        // an indicator each; conflating them means one of them lies.
        //
        // Painting the badge here rather than inside `status_text` is what
        // makes that unconditional structurally: `status_text` returns early
        // with an empty string when there are no filters, which is exactly
        // the state the issue was reported from. A badge threaded
        // through that function would need a second conditional to dodge the
        // early return, and a conditional can go stale.
        // `Cow` rather than `&str` (task 8 fix round 2, #199): every other
        // badge is a fixed string, but the stale badge names a key generated
        // from the table, and the search badge carries the pattern, so those
        // own a freshly-built `String` when present.
        //
        // The search badge is what says a search is set and what `n` will
        // step by, now that the filter pane has no row for it (ADR 0001).
        let badges: Vec<Cow<'static, str>> = [
            (self.document.mode() == Mode::FilteredOnly).then_some(Cow::Borrowed(HIDE_BADGE_TEXT)),
            self.filters
                .is_and()
                .then_some(Cow::Borrowed(AND_BADGE_TEXT)),
            self.search
                .as_ref()
                .map(|search| Cow::Owned(search_badge_text(&search.text))),
            self.visual_badge().map(Cow::Borrowed),
            self.view_stale
                .then(|| stale_badge_text(&self.keymap).map(Cow::Owned))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .collect();
        // One column of separation, so the colour block never abuts the text
        // beside it. Taken out of the status text's budget rather than added
        // to the row, so `status_bar_text`'s existing priority order — filter
        // state first, then whatever the path can elide itself into — still
        // has an accurate width to work with on a narrow terminal.
        let badge_width: usize = badges.iter().map(|text| text.chars().count() + 1).sum();
        let room = (prompt_area.width as usize).saturating_sub(badge_width);
        // The filter editor's count takes the row's text (#312): the
        // panes it describes are all under the editor.
        let status = match &self.filter_editor {
            Some(editor) => editor.status(),
            None => self.status_bar_text(room),
        };

        // Three columns, `[explorer | file view | filter pane]` (#300). A
        // hidden pane gets a zero-wide rectangle and is not drawn; the rest
        // share the width by `layout::columns`.
        let [explorer_area, view_area, filter_area] = self.pane_rects(area);

        // Remember the boundaries so mouse events landing before the next
        // frame can be tested against them. A divider exists only between
        // two shown panes: the explorer's where anything is to its right,
        // the filter pane's only where the file view is to its left — with
        // the view hidden, the filter pane fills what the explorer leaves
        // and the explorer's divider is the one that moves. A missing one is
        // parked at `u16::MAX`, past any real terminal width.
        self.divider = if explorer_area.width > 0 && explorer_area.right() < area.right() {
            explorer_area.right()
        } else {
            u16::MAX
        };
        self.filter_divider = if filter_area.width > 0 && view_area.width > 0 {
            filter_area.x
        } else {
            u16::MAX
        };
        self.explorer_area = explorer_area;
        self.view_area = view_area;
        self.filter_area = filter_area;

        self.set_active_pane();
        self.view.set_title_accent(self.crossing.is_some());
        self.view.set_selection(self.painted_selection());
        for (pane, pane_area) in [
            (Focus::Explorer, explorer_area),
            (Focus::View, view_area),
            (Focus::Filters, filter_area),
        ] {
            if self.panes.is_shown(pane) {
                self.render_pane(pane, pane_area, buf);
            }
        }
        if self.panes.is_shown(Focus::View) {
            self.render_crossing(view_area, buf);
        }

        // Last, so it covers whatever the panes just drew — and over `area`,
        // which is everything above the status row rather than the whole
        // frame. The row below carries the HIDE badge and the current
        // directory, and both are still true while the keymap is up; hiding
        // them would mean the one screen that explains `Ctrl-H` is also the one
        // screen that stops showing whether it is on.
        if let Some(picker) = &self.picker {
            picker.render(area, buf);
        }
        if let Some(picker) = self.set_picker.as_mut() {
            picker.render(area, buf);
        }
        if let Some(editor) = self.filter_editor.as_mut() {
            let title = self.view.filename().display().to_string();
            editor.render(&title, self.filters.dim_style(), &self.keymap, area, buf);
        }
        if self.help {
            help::render(area, buf, &self.keymap);
        }
        self.render_keymap_warnings(area, buf);

        // An open prompt takes the rest of the row; nothing but the badge
        // competes with it. The badge stays because the mode it reports is
        // still armed while a filter is being typed — which is precisely when
        // the pane is about to change underfoot.
        //
        // A transient message sits between the prompt and the derived status
        // text: it is more urgent than a filter count (it reports something
        // that just happened, and only lives until the next keypress) and less
        // urgent than a prompt (which the user is actively typing into).
        //
        // A message that arrives while a prompt is open — the wrap a search
        // reports as it is typed — is drawn after the prompt's text rather
        // than dropped: the prompt row is the only row there is, and the
        // message is about what the last keystroke just did.
        let (text, style) = match (self.prompt.as_ref(), self.status_message.as_ref()) {
            (Some(prompt), _) if prompt.error.is_some() => {
                (prompt.line(), Style::default().fg(Color::Red))
            }
            (Some(prompt), Some(message)) => (
                format!("{}   {}", prompt.line(), message.text),
                Style::default(),
            ),
            (Some(prompt), None) => (prompt.line(), Style::default()),
            (None, Some(message)) if message.error => {
                (message.text.clone(), Style::default().fg(Color::Red))
            }
            // Not dimmed like the derived row below: this one is a report the
            // user is meant to notice, and it is gone by the next keystroke.
            (None, Some(message)) => (message.text.clone(), Style::default()),
            (None, None) => (status, Style::default().fg(Color::DarkGray)),
        };
        // Two writes at two styles, rather than converting the row to
        // `Line`/`Span`s: a bigger diff through the prompt path that shares
        // this row, for no gain until something wants a third style here.
        let mut x = prompt_area.x;
        for badge in &badges {
            buf.set_stringn(
                x,
                prompt_area.y,
                badge.as_ref(),
                prompt_area.width as usize,
                HIDE_BADGE_STYLE,
            );
            x += u16::try_from(badge.chars().count() + 1).unwrap_or(u16::MAX);
        }
        buf.set_stringn(
            prompt_area.x + badge_width as u16,
            prompt_area.y,
            text,
            room,
            style,
        );
        // The prompt's cursor, drawn as the file view draws its own: the
        // cell in reversed video. The terminal cursor is hidden for the
        // whole session, so without this an edit in the middle of a pattern
        // (#206) would have nothing on screen to say where the middle is.
        // One blank past the text when the cursor is at the end.
        if let Some(column) = self.prompt.as_ref().and_then(SearchPrompt::cursor_column)
            && column < room
        {
            let x = prompt_area.x + badge_width as u16 + column as u16;
            buf[(x, prompt_area.y)]
                .set_style(style.add_modifier(ratatui::style::Modifier::REVERSED));
        }
    }
}

impl App<'_> {
    /// Draw one pane into `area`.
    ///
    /// The filter pane is handed the `ActiveFilters` it needs; the other two
    /// need nothing beyond themselves. This replaced a free `render_widget`
    /// function that existed only to route around a `Widget` impl one variant
    /// could never satisfy (#75) — each pane is now called directly with the
    /// arguments it actually takes.
    fn render_pane(&mut self, pane: Focus, area: Rect, buf: &mut Buffer) {
        match pane {
            Focus::Explorer => self.explorer.render(area, buf),
            Focus::View => self.view.render(area, buf),
            Focus::Filters => {
                let add = self.keymap.keys_for(&[
                    crate::keymap::ActionId::GlobalFocusFilters,
                    crate::keymap::ActionId::FiltersInclude,
                ]);
                self.filters_pane
                    .render(&self.filters, add.as_deref(), area, buf);
            }
        }
    }

    /// The one-line notice a cross-file step leaves over the file view until
    /// the next keypress. Centred, bordered, and cleared underneath so it
    /// reads over any text. Nothing is drawn when there was no crossing.
    fn render_crossing(&self, view_area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Block, Clear, Paragraph};
        let Some(crossing) = &self.crossing else {
            return;
        };
        let text = format!(
            "{} {} · {}",
            if crossing.backwards { "▲" } else { "▼" },
            crossing.label(),
            crossing.name
        );
        let width = u16::try_from(UnicodeWidthStr::width(text.as_str()) + 4)
            .unwrap_or(u16::MAX)
            .min(view_area.width);
        if width < 5 || view_area.height < 3 {
            return;
        }
        let x = view_area.x + (view_area.width - width) / 2;
        let y = view_area.y + (view_area.height - 3) / 2;
        let area = Rect {
            x,
            y,
            width,
            height: 3,
        };
        Clear.render(area, buf);
        Paragraph::new(text)
            .centered()
            .block(Block::bordered().border_style(Style::default().fg(Color::Yellow)))
            .render(area, buf);
    }

    /// The keymap warnings, over everything, until the first key.
    ///
    /// Modelled on `render_crossing`: cleared underneath, bordered, and
    /// silent when the area cannot hold it. Not on the help overlay's column
    /// flow — that exists to fit ninety short key rows into columns, and
    /// these are a few long sentences.
    ///
    /// Nothing scrolls. "Any key closes it" and a scroll key cannot both be
    /// true, which is the rule the help overlay already keeps. When the
    /// warnings outrun the space, the body says how many were cut and names
    /// `--print-keymap`, which writes every one of them to stderr (#259).
    /// The body and not the title (#252): a title is cut at the border, and
    /// a narrow terminal kept the count and lost the command.
    fn render_keymap_warnings(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Block, Clear, Padding, Paragraph};
        if !self.keymap_warnings_open || self.keymap_warnings.is_empty() {
            return;
        }
        let width = area.width.min(76);
        if width < 20 || area.height < 5 {
            return;
        }
        // Two columns of border and one of padding each side.
        let inner_width = usize::from(width - 4);
        // The rows inside the borders.
        let budget = usize::from(area.height.min(20)) - 2;
        let Some(lines) = warning_panel_lines(&self.keymap_warnings, inner_width, budget) else {
            return;
        };

        let height = u16::try_from(lines.len() + 2)
            .unwrap_or(u16::MAX)
            .min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };

        Clear.render(rect, buf);
        // Wrapped here, not by `Wrap`: the rows are counted before they are
        // drawn, and a row `Wrap` adds is a row the count did not see.
        Paragraph::new(lines.join("\n"))
            .block(
                Block::bordered()
                    .title(" Keymap warnings ")
                    .padding(Padding::horizontal(1))
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .render(rect, buf);
    }
}

/// The keymap warning panel's rows, wrapped to `width` columns: the
/// warnings that fit whole in `budget` rows, a blank row between each, then
/// a blank row, what was not shown, and how to close the panel.
///
/// Every warning is measured against the budget, the first one too (#254):
/// admitting the first one whatever it cost is what clipped its text in a
/// short terminal while the panel said nothing was cut. So a panel can show
/// no warning at all, and then it says how many there are.
///
/// `None` when not even that fits: half a sentence of advice is worse than
/// none, which is the rule the rest of the panel keeps for a tiny area.
fn warning_panel_lines(warnings: &[String], width: usize, budget: usize) -> Option<Vec<String>> {
    let close = wrap_hanging("Any key closes this.", "", "", width);
    let note = |cut: usize, shown: usize| {
        let text = if shown == 0 {
            format!("{cut} warnings — run recon --print-keymap to read them")
        } else {
            format!("{cut} more — run recon --print-keymap to read them all")
        };
        wrap_hanging(&text, "", "", width)
    };

    let mut body: Vec<String> = Vec::new();
    let mut shown = 0;
    for (index, warning) in warnings.iter().enumerate() {
        let rows = wrap_hanging(warning, "• ", "  ", width);
        let cut = warnings.len() - index - 1;
        let footer = if cut == 0 {
            close.len()
        } else {
            note(cut, index + 1).len() + close.len()
        };
        // The blank row before this warning, and the one before the footer.
        let gap = usize::from(!body.is_empty());
        if body.len() + gap + rows.len() + 1 + footer > budget {
            break;
        }
        if gap == 1 {
            body.push(String::new());
        }
        body.extend(rows);
        shown += 1;
    }

    let cut = warnings.len() - shown;
    let mut footer = Vec::new();
    if cut > 0 {
        footer.extend(note(cut, shown));
    }
    footer.extend(close);
    if !body.is_empty() {
        body.push(String::new());
    }
    body.extend(footer);
    if body.len() > budget {
        // Only the no-warning case can land here: the loop above kept every
        // other one inside the budget. Drop the closing line before the
        // count — any key still closes the panel, and the count is the only
        // sign that anything is missing.
        body.truncate(budget);
        let says_what_is_missing = body.iter().any(|row| row.contains("--print-keymap"));
        if cut > 0 && !says_what_is_missing {
            return None;
        }
    }
    Some(body)
}

/// `text` word-wrapped to `width` columns, with `first` before the first
/// row and `rest` before every row after it (#253): a bullet, and an indent
/// as wide as the bullet, so a warning that wraps stays visibly one warning.
/// A word wider than a row is broken where the row ends.
fn wrap_hanging(text: &str, first: &str, rest: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = first.to_string();
    let mut prefix = first.width();
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        let room = width.saturating_sub(prefix).max(1);
        let mut word = word;
        loop {
            let used = row.width() - prefix;
            let gap = usize::from(used > 0);
            if used + gap + word.width() <= room {
                if gap == 1 {
                    row.push(' ');
                }
                row.push_str(word);
                break;
            }
            if used > 0 {
                rows.push(std::mem::replace(&mut row, rest.to_string()));
                prefix = rest.width();
                continue;
            }
            // Wider than a whole row: take what fits and go on with the rest.
            let room = width.saturating_sub(prefix).max(1);
            let mut taken = 0;
            let split = word
                .char_indices()
                .find(|&(_, c)| {
                    taken += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    taken > room
                })
                .map_or(word.len(), |(at, _)| at);
            let split = if split == 0 {
                word.chars().next().map_or(word.len(), char::len_utf8)
            } else {
                split
            };
            row.push_str(&word[..split]);
            word = &word[split..];
            if word.is_empty() {
                break;
            }
            rows.push(std::mem::replace(&mut row, rest.to_string()));
            prefix = rest.width();
        }
    }
    rows.push(row);
    rows
}

#[cfg(test)]
mod tests {
    use super::{warning_panel_lines, wrap_hanging};

    #[test]
    fn wrapped_rows_keep_the_indent_and_the_width() {
        let rows = wrap_hanging("one two three four five", "• ", "  ", 10);
        assert_eq!(rows, ["• one two", "  three", "  four", "  five"]);
    }

    #[test]
    fn a_word_wider_than_a_row_is_broken_where_the_row_ends() {
        let rows = wrap_hanging("abcdefghij xy", "• ", "  ", 6);
        assert_eq!(rows, ["• abcd", "  efgh", "  ij", "  xy"]);
    }

    #[test]
    fn every_row_the_panel_returns_fits_its_budget() {
        let warnings = vec!["w ".repeat(40), "x ".repeat(40)];
        for budget in 0..12 {
            if let Some(lines) = warning_panel_lines(&warnings, 20, budget) {
                assert!(lines.len() <= budget, "{budget}: {lines:?}");
                assert!(lines.iter().all(|row| row.chars().count() <= 20));
            }
        }
    }
}
