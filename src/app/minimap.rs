//! The minimap's state on `App` (#523): when the file view gets one, the
//! cache that keeps it from being built on every frame, and a click or a
//! drag on it. The widget itself is `widgets::minimap`.

use super::App;
use crate::filter::Verdict;
use crate::widgets::minimap::Minimap;
use ratatui::layout::{Margin, Rect};
use std::sync::Arc;

/// The narrowest text area, inside the pane's border, that gives columns
/// to the map. Below it the text keeps all of them.
pub(super) const MINIMAP_MIN_WIDTH: u16 = 40;

/// What a map was built from. Equal keys mean the map is still right.
/// No line count: every change to the lines moves `revision`.
#[derive(Debug, PartialEq, Eq)]
struct Key<S> {
    rows: usize,
    revision: u64,
    generation: u64,
    search: Option<S>,
}

impl Key<&str> {
    /// The same key, owned, to keep with a new map.
    fn owned(&self) -> Key<String> {
        Key {
            rows: self.rows,
            revision: self.revision,
            generation: self.generation,
            search: self.search.map(str::to_owned),
        }
    }
}

impl Key<String> {
    /// The same key, borrowed, to compare with a frame's.
    fn borrowed(&self) -> Key<&str> {
        Key {
            rows: self.rows,
            revision: self.revision,
            generation: self.generation,
            search: self.search.as_deref(),
        }
    }
}

#[derive(Debug)]
pub(super) struct State {
    /// `[view] minimap`, then `M`.
    pub(super) shown: bool,
    /// The last map built, and what it was built from. `None` after a new
    /// document, whose counters start again from 0.
    built: Option<(Key<String>, Arc<Minimap>)>,
    /// A press on the map is held: a drag moves with the pointer.
    pub(super) dragging: bool,
    /// How many maps were built, for the test that proves a frame reuses one.
    #[cfg(test)]
    pub(super) builds: usize,
}

/// On, as `Config::minimap` is when nothing says otherwise.
impl Default for State {
    fn default() -> Self {
        Self::new(true)
    }
}

impl State {
    pub(super) fn new(shown: bool) -> Self {
        Self {
            shown,
            built: None,
            dragging: false,
            #[cfg(test)]
            builds: 0,
        }
    }

    /// Forget the map: it was built for a document that is gone.
    pub(super) fn forget(&mut self) {
        self.built = None;
    }
}

impl App<'_> {
    /// Give the file view its map for a frame drawn in `view_area`, or none.
    ///
    /// Built again only when its key changed, so a cursor move or a scroll
    /// costs one compare. A build is one pass over the verdicts and, with a
    /// search, one regex pass over the visible lines.
    pub(super) fn sync_minimap(&mut self, view_area: Rect) {
        let inner = view_area.inner(Margin::new(1, 1));
        let rows = usize::from(inner.height);
        let wanted = self.minimap.shown
            && self.view.shows_raw_text()
            && inner.width >= MINIMAP_MIN_WIDTH
            // Every visible line on screen: the band would cover it all.
            && self.document.visible().len() > rows;
        if !wanted {
            self.view.set_minimap(None);
            return;
        }
        // Borrowed for the compare, so a frame that builds nothing
        // allocates nothing; owned only for a new map.
        let key = Key {
            rows,
            revision: self.document.revision(),
            generation: self.document.generation(),
            search: self.search.as_ref().map(|search| search.text.as_str()),
        };
        let kept = (self.minimap.built.as_ref()).filter(|(built, _)| built.borrowed() == key);
        let map = if let Some((_, map)) = kept {
            Arc::clone(map)
        } else {
            let key = key.owned();
            let map = Arc::new(self.build_minimap(rows));
            self.minimap.built = Some((key, Arc::clone(&map)));
            #[cfg(test)]
            {
                self.minimap.builds += 1;
            }
            map
        };
        self.view.set_minimap(Some(map));
    }

    fn build_minimap(&self, rows: usize) -> Minimap {
        let document = &self.document;
        let interesting = document
            .verdicts()
            .iter()
            .enumerate()
            .filter_map(|(line, verdict)| match verdict {
                Verdict::Included(filter) => Some((line, *filter)),
                _ => None,
            });
        let hits = self.search.iter().flat_map(|search| {
            document.visible().iter().copied().filter(|&line| {
                document
                    .lines()
                    .get(line)
                    .is_some_and(|text| search.is_hit(text))
            })
        });
        let colour = |filter| {
            self.match_style(Some(filter))
                .fg
                .unwrap_or(ratatui::style::Color::Reset)
        };
        Minimap::build(rows, document.lines().len(), interesting, hits, colour)
    }

    /// A press on the map, on screen row `row`: the start of a drag.
    pub(super) fn press_minimap(&mut self, row: u16) {
        self.minimap.dragging = true;
        self.jump_on_minimap(row);
    }

    /// Jump to the first line of the map row on screen row `row`, or the
    /// nearest visible line to it: the same jump a search makes. A row past
    /// the map's top or bottom is held at the edge, for a drag.
    pub(super) fn jump_on_minimap(&mut self, row: u16) {
        let Some((map, area)) = self.view.drawn_minimap() else {
            return;
        };
        let row = row.clamp(area.y, area.bottom().saturating_sub(1)) - area.y;
        let line = map.line_at(usize::from(row));
        let Some(target) = self.document.nearest_visible(line) else {
            return;
        };
        self.land_on(target);
    }
}
