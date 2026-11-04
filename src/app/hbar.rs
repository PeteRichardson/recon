//! The widest visible line, measured for the horizontal scroll bar (#579):
//! how far right the text reaches, so the bar knows what it can scroll to
//! and whether there is anything at all off screen. The widget geometry
//! lives in `widgets::hbar` (#578); the row reservation and drawing are
//! #580.
//!
//! It has to be measured here, at the whole document, because the
//! textarea's own limit is computed from the buffer window it holds —
//! the windowed viewport keeps only the rows around the cursor, and the
//! widest line may be far outside it.
//!
//! Only *visible* lines count: the bar tells the truth about what is on
//! screen, so a `u` or a filter that hides the long lines shrinks the bar
//! with the text rather than leaving a thumb stranded near the left edge.

use super::App;

/// What a width was measured from. Equal keys mean the width is still
/// right: `generation` moves exactly when the visible set does, and the
/// set's length keeps the key honest across a swapped document whose
/// counters restart from zero — the load path's `forget` is the real guard
/// there, as it is for the minimap.
#[derive(Debug, PartialEq, Eq)]
struct Key {
    generation: u64,
    visible: usize,
}

/// The cache: the last width measured, what it was measured from, and —
/// in tests — how many measurements were made, proving a frame reuses
/// rather than remeasures.
#[derive(Debug, Default)]
pub(super) struct Width {
    built: Option<(Key, usize)>,
    /// Whether the pointer is holding the bar down (#581), kept beside the
    /// bar's own state because it is the same piece of furniture.
    pub(super) dragging: bool,
    #[cfg(test)]
    pub(super) builds: usize,
}

impl Width {
    /// Forget the width: it was measured for a document that is gone.
    pub(super) fn forget(&mut self) {
        self.built = None;
    }
}

/// `text`'s width in terminal columns: `unicode-width`, so a CJK ideograph
/// or an emoji costs the two columns the renderer gives it (#97), with
/// tabs advanced to stops `tab` apart as the view renders them — a tab
/// costs the distance to the next stop, not one cell.
fn text_width(text: &str, tab: usize) -> usize {
    let tab = tab.max(1);
    text.chars().fold(0, |col, ch| {
        if ch == '\t' {
            (col / tab + 1) * tab
        } else {
            col + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
        }
    })
}

impl App<'_> {
    /// Bring the cached width up to date for the frame about to draw.
    /// Remeasures only when its key changed, so a cursor move or a scroll
    /// costs one compare; a measurement is one pass over the visible lines.
    /// A view that shows no raw text — an image, hex — has no line widths
    /// to carry over, so the cache is dropped rather than left stale.
    pub(super) fn sync_hbar_width(&mut self) {
        if !self.view.shows_raw_text() {
            self.hbar.forget();
            return;
        }
        let visible = self.document.visible();
        let key = Key {
            generation: self.document.generation(),
            visible: visible.len(),
        };
        if self
            .hbar
            .built
            .as_ref()
            .is_some_and(|(built, _)| built == &key)
        {
            return;
        }
        let tab = self.view.tab_length();
        let lines = self.document.lines();
        let widest = visible
            .iter()
            .filter_map(|line| lines.get(line))
            .map(|text| text_width(text, tab))
            .max()
            .unwrap_or(0);
        self.hbar.built = Some((key, widest));
        #[cfg(test)]
        {
            self.hbar.builds += 1;
        }
    }

    /// The widest column a visible line reaches, in text columns — the
    /// gutter is the view's to add, as it is everywhere else a column
    /// crosses that boundary. `None` when nothing has been measured: no
    /// raw-text view, or an empty file.
    pub(super) fn hbar_widest(&self) -> Option<usize> {
        self.hbar.built.as_ref().map(|(_, width)| *width)
    }

    /// A press on the bar: the drag begins, and the view jumps to the
    /// column under the pointer (#581). `cell` counts from the bar's left
    /// edge, `cells` is the bar's width.
    pub(super) fn press_hbar(&mut self, cell: usize, cells: usize) {
        self.hbar.dragging = true;
        self.jump_on_hbar(cell, cells);
    }

    /// Scroll sideways to the column a pointer on bar cell `cell` names:
    /// the thumb's centre sits under the pointer, so a drag tracks it
    /// within about a column. Only the view moves — the cursor stays put
    /// (#576), the offset is the fork's pinned scroll (#577).
    pub(super) fn jump_on_hbar(&mut self, cell: usize, cells: usize) {
        let Some((bar, _)) = self.view.drawn_hbar() else {
            return;
        };
        let cells = u16::try_from(cells).unwrap_or(u16::MAX);
        let col = bar.col_at(cell, cells);
        self.view.scroll_to_col(col);
    }
}
