//! `>` and `<` (#609, ADR 0017): reveal the hidden lines after the cursor
//! in hide mode, and hide them again. A reveal is view state, not a
//! verdict, so neither key runs the filters: each costs what `u` costs.

use super::App;
use crate::document::Mode;
use crate::scan::Stamp;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// What `>` and `<` say in dim mode, where every line already shows.
pub(super) const REVEAL_IN_DIM_MODE: &str = "every line shows in dim mode: u for hide mode";
/// What `>` says when no hidden line follows the cursor's line.
pub(super) const NOTHING_TO_REVEAL: &str = "no hidden line after this one";
/// What `<` says when no reveal holds the cursor's line.
pub(super) const NO_REVEAL_HERE: &str = "no reveal holds this line";
/// What the palette says when it closes every reveal and none is open.
pub(super) const NO_REVEALS: &str = "no reveal is open";

/// The reveals of each file, for the session: a load of a file finds
/// them again, unless the file changed since they were made, when their
/// rows may be other lines.
#[derive(Debug, Default)]
pub(super) struct Kept {
    by_path: HashMap<PathBuf, (Stamp, Vec<Range<usize>>)>,
    /// The open file, by `path::lexical_absolute`, and its stamp from the
    /// load: the version of it that its reveals are rows of. `None` for
    /// what is not a file, such as a placeholder or a command's output.
    open: Option<(PathBuf, Stamp)>,
}

impl Kept {
    /// A load of `path`, whose stamp the load read before the file:
    /// its reveals, when it is the version they were made on. A changed
    /// file loses them. A load with no stamp keeps them for the next.
    pub(super) fn loaded(&mut self, path: &Path, stamp: Option<Stamp>) -> Vec<Range<usize>> {
        self.open = stamp.map(|stamp| (crate::path::lexical_absolute(path), stamp));
        let Some((key, stamp)) = &self.open else {
            return Vec::new();
        };
        match self.by_path.get(key) {
            Some((kept, reveals)) if kept == stamp => reveals.clone(),
            Some(_) => {
                self.by_path.remove(key);
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    /// The open file has `reveals` now.
    fn remember(&mut self, reveals: &[Range<usize>]) {
        let Some((key, stamp)) = &self.open else {
            return;
        };
        if reveals.is_empty() {
            self.by_path.remove(key);
        } else {
            self.by_path.insert(key.clone(), (*stamp, reveals.to_vec()));
        }
    }
}

impl App<'_> {
    /// `>`: reveal the hidden lines from the cursor's line to the next
    /// visible line. The cursor stays on its line.
    pub(super) fn reveal_gap(&mut self) {
        if self.refuse_reveal_in_dim_mode() {
            return;
        }
        let source = self.cursor_source();
        if self.document.reveal(source).is_none() {
            self.report(NOTHING_TO_REVEAL, false);
            return;
        }
        self.reveals_changed(source);
    }

    /// `<`: close the smallest reveal that holds the cursor's line, and
    /// put the cursor on its first line.
    pub(super) fn close_reveal(&mut self) {
        if self.refuse_reveal_in_dim_mode() {
            return;
        }
        let Some(first) = self.document.close_reveal(self.cursor_source()) else {
            self.report(NO_REVEAL_HERE, false);
            return;
        };
        self.reveals_changed(first);
    }

    /// `view.reveal.close.all`: close every reveal of the open file. The
    /// cursor stays on its line, or goes to the nearest one left. `Err`
    /// is what the palette shows when no reveal is open.
    pub(super) fn close_reveals(&mut self) -> Result<(), String> {
        let source = self.cursor_source();
        if !self.document.close_reveals() {
            return Err(NO_REVEALS.to_string());
        }
        self.reveals_changed(source);
        Ok(())
    }

    /// Keep the open file's reveals for its next load, and show them with
    /// the cursor on `source`.
    fn reveals_changed(&mut self, source: usize) {
        self.kept_reveals.remember(self.document.reveals());
        self.apply_view(source);
    }

    fn refuse_reveal_in_dim_mode(&mut self) -> bool {
        let dim = self.document.mode() == Mode::Dimmed;
        if dim {
            self.report(REVEAL_IN_DIM_MODE, false);
        }
        dim
    }
}
