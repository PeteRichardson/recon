//! Keeping the document and the file view in step with the filters.

use super::App;
use crate::document::Document;
use crate::widgets;

impl App<'_> {
    /// Take the view's current contents as the document to filter.
    ///
    /// The view owns the reading — including its preview truncation and its
    /// error messages — so the document follows it rather than re-reading.
    /// Note the consequence: while a file is only previewed (the view
    /// truncates large files), the document holds just that preview, so
    /// filters see only the truncated slice until the view is focused and
    /// loads the file in full.
    pub(super) fn sync_document(&mut self) {
        let lines = self.view.source().clone();
        // The hide toggle describes how the user is reading, not which file
        // they are reading, so it outlives the document exactly as the filter
        // set does — and for the same reason. The filters survived a load
        // only because `App` owns them separately from the `Document` this
        // line replaces; the mode lives *on* the document, so without
        // carrying it across, every load and every explorer preview silently
        // reset it to `Mode::default()`.
        //
        // That made the toggle almost unusable for its main purpose: skimming
        // a directory for the files a filter actually matches means moving
        // the explorer's selection, and every move fired a `Preview` through
        // here and undid the `Ctrl-H` that made the skim possible.
        let mode = self.document.mode();
        self.document = Document::for_file(self.view.filename(), lines);
        self.set_mode(mode);
        // The anchor was a line of the document this just replaced (#67).
        self.visual = None;
        // The buffer the view is showing belongs to the *previous* document,
        // so the record of what it was built from is meaningless now.
        // Clearing it forces the next `apply_view` to rebuild: two different
        // documents can easily carry an equal generation — every fresh one
        // starts at 0, and reloading the same file with a filter active
        // lands on the same count every time — which would otherwise leave
        // the just-loaded, unfiltered buffer in place under numbers and
        // styles sized for the filtered subset.
        self.last_generation = None;
        // Both halves of the rebuild-skip key, or the surviving half could
        // still match and skip a rebuild this just decided is owed.
        self.last_window = None;
    }

    /// Re-evaluate the filters and rebuild what the view shows.
    pub(super) fn refresh_view(&mut self) {
        // Whatever changed the set may have changed how many filters there
        // are, so the pane's selection has to be pulled back into range (or
        // established at 0 on a set that just became non-empty) before
        // anything else runs. Every mutation path — `add_filter`,
        // `add_excluding_filter`, and the pane's own toggle/delete — funnels
        // through this method, so putting the call here rather than at each
        // call site means a future fourth path cannot forget it.
        let rows = widgets::filterlist::rows(&self.filters).len();
        self.filters_pane.clamp_selection(rows);

        // The cursor is a source line index for the duration of the rebuild:
        // its row in the view is only meaningful against the old visible list.
        let cursor_source = self.cursor_source();
        self.document.evaluate(&self.filters);
        self.apply_view(cursor_source);
    }

    /// List or unlist each `(set, listed)` the set picker hands back (#284),
    /// then update the view once.
    ///
    /// The filter pane's cursor stays on the row it was on when that row is
    /// still there. Its index can move — a set above it may have gained or
    /// lost its rows — so it is followed by what it addresses, not by its
    /// position. A row that went with its set leaves the cursor where
    /// `refresh_view`'s clamp puts it.
    pub(super) fn apply_listing(&mut self, changes: &[(usize, bool)]) {
        if changes.is_empty() {
            return;
        }
        // A list change ends a peek (#305): the flags come back first, so the
        // sets are listed and unlisted as they are, not as the peek shows
        // them. The capture is by filter id (#346), so it would survive the
        // change; ending the peek is what the picker promises.
        if self.peek.is_some() {
            self.toggle_peek();
        }
        let before = widgets::filterlist::rows(&self.filters);
        let selected = self
            .filters_pane
            .selected()
            .and_then(|index| before.get(index).copied());
        for &(set, listed) in changes {
            self.filters.set_listed(set, listed);
        }
        if let Some(row) = selected {
            let after = widgets::filterlist::rows(&self.filters);
            self.filters_pane.follow(row, &after);
        }
        self.refresh_view();
    }
}
