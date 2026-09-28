//! The status row: its badges, its text, and the one-off messages.

use crate::app::App;
use crate::document::Mode;
use crate::widgets::Focus;
use ratatui::prelude::{Color, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The badge saying hide-unmatched-lines mode is armed, and the style that
/// makes it loud enough to notice mid-skim.
///
/// Six columns rather than a spelled-out `[HIDE MODE ON]`, on a row that also
/// has to carry a filter count and the directory on a narrow terminal. A
/// filled colour block reads louder than a glyph at a quarter of the width —
/// which is the point, since issue #36 rejected a dim status-bar icon by name.
///
/// Both are config-schema candidates for #18, text and style alike.
///
/// **Filled block means state; brackets or a border mean action.** That is the
/// established TUI idiom — vim's statusline mode indicator, tmux status
/// segments and powerline segments are all filled blocks nobody clicks, while
/// buttons get `[ OK ]` or `< Cancel >`. Mouse control is planned (click a
/// file to view it, click a filter to toggle it), so the rule is recorded here
/// rather than re-derived: anything painted like this badge is not clickable.
pub(in crate::app) const HIDE_BADGE_TEXT: &str = " HIDE ";

pub(in crate::app) const HIDE_BADGE_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::LightYellow)
    .add_modifier(ratatui::style::Modifier::BOLD);

/// The badge saying the file on screen is not the file on disk (#119).
///
/// Raised by `poll_stamps` when the *active* file's stamp moves, cleared by
/// reloading. The explorer's answer for that file updates on its own; the
/// view does not reload on its own. That is a real inconsistency between the
/// panes, and the badge exists so it is never a silent one: one key resolves
/// it.
///
/// A function rather than a `const` (task 8 fix round 2, #199): the badge
/// names the key that reloads, so — like the on-screen hints — it is
/// generated from `DEFAULT` rather than hard-coded, and stays correct after
/// a rebind. The default keymap renders this byte-identically to the old
/// constant, `" changed on disk · r "`, leading and trailing spaces
/// included — see `badges` in `Widget::render`, which pads each badge with
/// exactly the same column of separation regardless of where its text came
/// from.
///
/// `None` when a `[keymap]` line has left `global.reload` with no key at all
/// (#61): the badge exists to name the key that resolves the inconsistency,
/// and " changed on disk · " with nothing after it names none — a badge the
/// user cannot act on is worse than no badge.
pub(in crate::app) fn stale_badge_text(keymap: &crate::keymap::Keymap) -> Option<String> {
    let key = keymap.label_for(crate::keymap::ActionId::GlobalReload)?;
    Some(format!(" changed on disk · {key} "))
}

/// The badge saying the include filters are combined with AND (#39). Same style
/// as `HIDE`, and for the same reason: the mode changes what the pane shows
/// and is easy to forget while moving fast.
pub(in crate::app) const AND_BADGE_TEXT: &str = " AND ";

/// The longest pattern the search badge shows before eliding the tail. The
/// badge shares the status row with the filter summary and the path; a
/// regex this long is still recognisable from its head.
const SEARCH_BADGE_MAX: usize = 32;

/// The badges saying a selection is in progress (#67), character-wise and
/// line-wise. Same style as `HIDE`, and for the same reason: the mode
/// changes what the next keys do — `y` copies, `Esc` ends it — and is easy
/// to forget mid-scroll. Vim's own words for the two, shortened to fit.
pub(in crate::app) const VISUAL_BADGE_TEXT: &str = " VISUAL ";

pub(in crate::app) const VLINE_BADGE_TEXT: &str = " V-LINE ";

/// A one-off message shown on the status row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct StatusMessage {
    pub(in crate::app) text: String,
    /// Drawn red. Not derivable from the text — "opened src/lib.rs" and
    /// "zed: No such file or directory" are the same shape.
    pub(in crate::app) error: bool,
}

/// Shorten `text` to `width` columns by dropping characters from the *left*,
/// marking the cut with a leading `…`.
///
/// The tail is what identifies a path: `…/projects/recon/src` still says where
/// you are, where the same cut taken from the right would not.
/// Measured in terminal columns, not `char`s. A CJK ideograph or an emoji
/// occupies two columns, so counting chars over-filled the budget by one column
/// per wide glyph and the row then overran the terminal it was sized for (#97).
///
/// The tail is accumulated from the right — the one place where columns and
/// chars genuinely differ in *how* the cut is taken, not merely in what it
/// measures, since a wide glyph can no longer be assumed to cost one.
pub(in crate::app) fn elide_left(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    // One column goes to the ellipsis itself.
    let budget = width - 1;
    let mut taken = 0;
    let mut start = text.len();
    for (index, ch) in text.char_indices().rev() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if taken + w > budget {
            break;
        }
        taken += w;
        start = index;
    }
    format!("…{}", &text[start..])
}

/// Shorten `text` to `width` columns by dropping characters from the
/// *right*, marking the cut with a trailing `…`: the start of a sentence is
/// what says what it is about. Columns, not `char`s, as in `elide_left`.
pub(in crate::app) fn elide_right(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let budget = width - 1;
    let mut taken = 0;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if taken + w > budget {
            break;
        }
        taken += w;
        end = index + ch.len_utf8();
    }
    format!("{}…", &text[..end])
}

/// The search badge: `/pattern`, padded like the other badges, with the
/// tail elided past `SEARCH_BADGE_MAX` characters.
pub(in crate::app) fn search_badge_text(pattern: &str) -> String {
    let shown: String = pattern.chars().take(SEARCH_BADGE_MAX).collect();
    let ellipsis = if pattern.chars().count() > SEARCH_BADGE_MAX {
        "…"
    } else {
        ""
    };
    format!(" /{shown}{ellipsis} ")
}

impl App<'_> {
    /// A one-line summary of the filter state, empty when no filters exist.
    ///
    /// Dimming alone does not say *why* lines are dim, or that a filter is
    /// defined but currently disabled — the pane would just look ordinary.
    fn status_text(&self) -> String {
        // `FilteredOnly` only hides anything when something enabled is
        // including: issue #36's guard in `Document::recompute_visible`
        // shows the whole file instead once nothing is, so `any_including`
        // has to gate the funnel here too — otherwise a filter that exists
        // but is disabled would claim lines are hidden while the guard is
        // already showing everything.
        //
        // An excluding filter (`x`) is counted on its own, regardless of
        // mode: it removes its matches in `Dimmed` mode too, which is the
        // entire point of it, so gating the funnel on `FilteredOnly` alone
        // let `x` empty the pane with nothing on the status line saying so.
        let hiding = (self.document.mode() == Mode::FilteredOnly && self.filters.any_including())
            || self.filters.any_excluding();
        let funnel = if hiding { "▼ " } else { "" };
        if self.filters.is_empty() {
            // With no filters at all, `any_including` and
            // `any_excluding` are both trivially false, so `hiding` above is
            // always false too — there is nothing this early return could be
            // discarding. Filters that exist but are all disabled are a
            // different state, one the same `hiding` expression already
            // keeps honest below (see the `!any_enabled` branch): nothing
            // enabled can be including or excluding either, so the funnel
            // stays off there as well.
            return String::new();
        }
        // `row_count`, not `len`: the user-authored filters, which is what
        // the pane numbers. See `ActiveFilters::row_count`.
        let count = self.filters.row_count();
        let noun = if count == 1 { "filter" } else { "filters" };
        if !self.filters.any_enabled() {
            return format!("{funnel}{count} {noun} (disabled)");
        }
        // Report what is actually on screen (lines *shown*) rather than how
        // many matched an including filter: an excluding filter alone can
        // remove lines while matching nothing, which used to read as "0
        // matched" over a pane that had in fact lost lines.
        let (total, previewing) = self.total_lines_text();
        let note = if previewing { " (preview)" } else { "" };
        // With the filter pane hidden, nothing on the screen says which
        // filters colour or remove lines, so the count is of those in effect
        // rather than of rows a pane is not showing (#300).
        let on = self.filters.effective_count();
        let filters = if !self.panes.is_shown(Focus::Filters) && on > 0 {
            format!("filters: {on} on")
        } else {
            format!("{count} {noun}")
        };
        let off = self
            .filters
            .scan_off()
            // Permanent rather than a message, because the cause stays until
            // the user removes it: the explorer marks nothing and its `n`
            // finds nothing, and without this nothing on the screen says why.
            .map(|off| format!("   {off}"))
            .unwrap_or_default();
        format!(
            "{funnel}{filters}   {}/{total} lines shown{note}{off}",
            self.document.visible().len(),
        )
    }

    /// The file's line count as the row should report it, and whether that is
    /// a preview's estimate rather than a fact.
    ///
    /// The document holds whatever the view holds, so while the view is
    /// showing a bounded preview `document.lines().len()` is the *preview's*
    /// length — the cap, not the file's count — and reporting it unqualified is
    /// not a rounding error but a wrong answer. The estimate is a guess, but
    /// it is a guess marked as one.
    fn total_lines_text(&self) -> (String, bool) {
        let loaded = self.document.lines().len();
        match (self.view.is_truncated(), self.view.estimated_lines()) {
            // Truncated with nothing to scale from: the count is still the
            // preview's, so it stays flagged even without a better number.
            (true, None) => (loaded.to_string(), true),
            (true, Some(estimate)) => (format!("~{estimate}"), true),
            (false, _) => (loaded.to_string(), false),
        }
    }

    /// The directory the explorer is listing.
    fn explorer_dir(&self) -> &std::path::Path {
        self.explorer.dir()
    }

    /// The description of the filter the filter pane's selection is on, while
    /// the pane has the focus (#317): the place outside the filter editor
    /// where a filter's description can be read.
    fn selected_description(&self) -> Option<&str> {
        use crate::widgets::filterlist::{Row, rows};
        if self.focus != Focus::Filters {
            return None;
        }
        let row = self.filters_pane.selected()?;
        let (Row::Filter(index) | Row::BuiltIn(index)) = *rows(&self.filters).get(row)? else {
            return None;
        };
        self.filters.filters().get(index)?.description.as_deref()
    }

    /// The whole bottom row: filter state first, then the directory in
    /// whatever width is left — or, with the filter pane's selection on a
    /// filter that has one, the filter's description (#317).
    ///
    /// Filter state comes first because it cannot degrade — a count with its
    /// digits cut off is wrong rather than short — whereas the path elides
    /// from the left and stays readable. That is the priority order the row
    /// needs on a narrow terminal, where all of this cannot fit at once.
    pub(in crate::app) fn status_bar_text(&self, width: usize) -> String {
        let status = self.status_text();
        let tail = |room: usize| match self.selected_description() {
            Some(description) => elide_right(description, room),
            None => elide_left(&self.explorer_dir().display().to_string(), room),
        };
        if status.is_empty() {
            return tail(width);
        }
        // Two spaces of separation, and the path only gets what survives the
        // status text. Too narrow for any of it and the path is dropped
        // entirely rather than rendered as a lone ellipsis.
        let spent = status.chars().count() + 2;
        match width.checked_sub(spent) {
            Some(room) if room > 1 => format!("{status}  {}", tail(room)),
            _ => status,
        }
    }
}
