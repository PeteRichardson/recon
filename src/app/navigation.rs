//! Moving between files, and between the interesting lines in one.

use super::App;
use super::loading::Landing;
use super::search::{WRAPPED_TO_BOTTOM, WRAPPED_TO_TOP};
use super::viewport::Step;
use crate::views::Extent;
use crate::widgets::Action;
use crate::widgets::explorer::Explorer;

/// A cross-file step that just happened, for the notice over the file view
/// and the accent on its title. Lives exactly as long as a `StatusMessage`:
/// until the next keypress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Crossing {
    pub(super) backwards: bool,
    pub(super) name: String,
}

impl Crossing {
    /// The direction word shared by the status report (`cross_file`) and the
    /// notice painted over the view (`render_crossing`), so the two only say
    /// "previous file" and "next file" in one place.
    pub(super) fn label(&self) -> &'static str {
        if self.backwards {
            "previous file"
        } else {
            "next file"
        }
    }
}

impl<'a> App<'a> {
    /// Re-read the active file, and put the cursor back on the line it was on.
    ///
    /// A new read rebuilds the buffer from the top, so this remembers the
    /// cursor's *source* line first and `Landing::Keep` re-places it when
    /// the read arrives — the machinery a filter change already uses to
    /// rebuild without losing the reader's place. A file that shrank underneath the cursor (logrotate) simply
    /// clamps to what is left.
    pub(super) fn reload_active_file(&mut self) {
        let path = self.view.filename().to_path_buf();
        if path.as_os_str().is_empty() {
            return;
        }
        log::debug!("navigation: reload {}", path.display());
        let source = self.cursor_source();
        self.open_view(&path, self.current_extent(), Landing::Keep(source));
        self.view_stale = false;
    }

    /// Open the entry a directory listing shows on `visible_row`, the way a
    /// click on that row (#58) and `Enter` on it (#469) both do: the
    /// explorer enters the listed directory and opens the entry. Does
    /// nothing while the view shows anything but a listing.
    ///
    /// The visible row is not the entry: filters and hide mode apply to a
    /// listing like any document. The row goes back to its source line,
    /// which is the entry's index in the listing (#349). An empty hidden
    /// listing draws one blank row that maps to none.
    pub(super) fn open_listing_row(&mut self, visible_row: usize) {
        if !self.view.showing_directory() {
            return;
        }
        let Some(index) = self.document.source_at(visible_row) else {
            return;
        };
        let dir = self.view.filename().to_path_buf();
        if let Some(action) = self.explorer.open_listed(&dir, index) {
            self.perform_widget_action(action);
        }
        self.ensure_window();
    }

    /// Carry out one of the view's own actions, whichever pane has focus,
    /// with the after-care the focused dispatch gives the other panes: a
    /// truncated preview is made whole first, and the window is checked
    /// after a page at its edge.
    pub(super) fn perform_on_view(&mut self, action: crate::keymap::ActionId) {
        // The user is interacting with the view, so a preview is no longer
        // enough: they can now scroll past the end of it.
        self.promote_truncated_preview();
        self.view.perform(action);
        self.ensure_window();
    }

    /// `n`/`N` in the file view. With a search set, the next hit line in this
    /// file — see `step_hit`. Otherwise the next interesting line in this
    /// file, else the first interesting line of the next file the filters
    /// selected, else (when this is the only such file) wrap within it as
    /// `n` always has.
    ///
    /// The in-file step comes first so the loop the key drives — every hit in
    /// every file — never skips a hit. The cross-file step is what makes it a
    /// single loop rather than one per file (#120 §1).
    pub(super) fn step_interesting(&mut self, backwards: bool) {
        if self.search.is_some() {
            self.step_hit(backwards);
            return;
        }
        // A jump that leaves the peeked context has nothing to come back to,
        // and with every filter disabled by the peek the step would find no
        // interesting line and cross files at once. Restore first (#120 §4).
        self.restore_peek_before_moving();
        // `n`/`N` bypass the widget's own `perform`, which is where a
        // truncated preview normally promotes itself on first interaction —
        // see `promote_truncated_preview`, which `apply_search` also calls
        // for the same reason.
        self.promote_truncated_preview();
        if let Some(target) = self.next_interesting_strict(backwards) {
            self.land_on(target);
            return;
        }
        if !self.cross_file(backwards, Explorer::step_to_match) {
            // `step_to_interesting` is quiet about what it did, and a key
            // whose whole job is finding a hit should not be: say when the
            // walk passed the file's edge, and say when there was nothing.
            match self.step_to_interesting(backwards) {
                Step::Nothing => self.report("no interesting line", false),
                Step::Wrapped => self.report(
                    if backwards {
                        WRAPPED_TO_BOTTOM
                    } else {
                        WRAPPED_TO_TOP
                    },
                    false,
                ),
                Step::Landed => {}
            }
        }
    }

    /// Select, load and land in the next (previous) file that `step` walks
    /// the explorer to. `false` when there is no *other* such file — the explorer
    /// wraps, so "the only match is the one we are in" comes back as an
    /// unchanged selection rather than `None`.
    ///
    /// Reports the crossing three ways, all gone by the next keypress: the
    /// status row, the notice `render` paints over the file view, and the
    /// accent on the view's title. Log files look alike, and a step that
    /// silently changed which one is on screen would be worse than no step.
    fn cross_file(
        &mut self,
        backwards: bool,
        step: fn(&mut Explorer<'a>, bool) -> Option<Action>,
    ) -> bool {
        let before = self.explorer.selected_entry();
        let Some(action) = step(&mut self.explorer, backwards) else {
            return false;
        };
        if self.explorer.selected_entry() == before {
            return false;
        }
        let (Action::Load(path) | Action::LoadAndFocus(path) | Action::Preview(path)) = action;
        let name = self.explorer.selected_name().unwrap_or_default();
        let crossing = Crossing { backwards, name };
        log::debug!(
            "navigation: open {}, the {}",
            path.display(),
            crossing.label()
        );
        self.open_view(&path, Extent::Full, Landing::FirstInteresting { backwards });
        self.prefetch_near_selection();
        self.report(&format!("{} · {}", crossing.label(), crossing.name), false);
        self.crossing = Some(crossing);
        true
    }

    /// `.`/`,`: the cross-file half of `n`/`N`, without first exhausting the
    /// current file. Global, so the loop can skip a file from any pane.
    ///
    /// With no filter selecting, every file is a match — the explorer shows
    /// them all plain, unscanned — so the step goes to the next file (#471).
    /// The peek is restored first, so its own "no filter" never counts.
    pub(super) fn skip_file(&mut self, backwards: bool) {
        self.restore_peek_before_moving();
        let step = if self.filters.is_scanning() {
            Explorer::step_to_match
        } else {
            Explorer::step_to_file
        };
        if !self.cross_file(backwards, step) {
            self.report("no other file matches", false);
        }
    }
}
