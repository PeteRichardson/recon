//! `--files-from` in the TUI (#519): the listed files in the explorer.
//!
//! The explorer lists a directory of links to the files, so this only
//! keeps that listing up to date as the list comes: in the list's order,
//! with a new link listed as soon as it is made.

use super::App;
use super::loading::Landing;
use crate::files_from::LinkDir;
use crate::stdin::State;
use crate::widgets::Action;
use crate::widgets::explorer::renamed_to;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The shortest time between two reads of the links, while the list
/// comes: a long list can make thousands a second.
const RELIST_EVERY: Duration = Duration::from_millis(100);

/// How many times the last read of the links the next one waits, at the
/// least: each read lists the whole folder, on the thread that draws, so a
/// long list is read less often rather than slowing the keys.
const RELIST_FACTOR: u32 = 10;

/// The most missing paths the warning names; the log names them all.
const NAMED_MISSING: usize = 5;

/// The directory of links, and when the explorer can read it next.
pub(super) struct Links {
    dir: LinkDir,
    next: Option<Instant>,
}

/// The one warning for the listed paths that do not exist.
fn missing_warning(missing: &[PathBuf]) -> String {
    let mut text = "--files-from: no such file: ".to_string();
    let named: Vec<String> = missing
        .iter()
        .take(NAMED_MISSING)
        .map(|path| path.display().to_string())
        .collect();
    text.push_str(&named.join(", "));
    if missing.len() > NAMED_MISSING {
        let _ = write!(text, ", and {} more", missing.len() - NAMED_MISSING);
    }
    text
}

impl App<'_> {
    /// List `links`, the `--files-from` list's files, in the explorer. `App`
    /// keeps it, so that `[stdin] delete_temp_file` acts when the session
    /// ends.
    #[must_use]
    pub fn with_links(mut self, links: Option<LinkDir>) -> Self {
        let Some(dir) = links else {
            return self;
        };
        let path = dir.path().to_path_buf();
        self.links = Some(Links { dir, next: None });
        // `App::new` read the folder before its order was known, and showed
        // the first link by name, or `..` with none yet. The list's first
        // file is the one to show; with none, the empty folder. A first
        // link that comes in this poll, `poll_links` shows itself.
        let listed_by_new = !self.explorer.is_empty();
        self.poll_links();
        if self.explorer.is_empty() {
            self.open_view(&path, crate::views::Extent::Preview, Landing::Top);
        } else if listed_by_new {
            self.show_first_link();
        }
        self
    }

    /// Select the first link, and show it.
    fn show_first_link(&mut self) {
        self.explorer.select_entry(1);
        if let Some(path) = self.explorer.selected_path() {
            self.perform_widget_action(Action::Preview(path));
        }
    }

    /// List the links made since the last call, and say whether the screen
    /// changed. The first file to come is selected and shown.
    pub(super) fn poll_links(&mut self) -> bool {
        let Some(links) = &mut self.links else {
            return false;
        };
        if links.next.is_some_and(|next| Instant::now() < next) {
            return false;
        }
        let Some(changes) = links.dir.take() else {
            return false;
        };
        let started = Instant::now();
        let dir = links.dir.path().to_path_buf();
        match &changes.state {
            State::Open => {}
            State::Closed => {
                if !changes.skipped.is_empty() {
                    self.report(&missing_warning(&changes.skipped), true);
                }
            }
            State::Stopped(reason) => {
                let text = format!("--files-from: cannot read the list: {reason}");
                log::error!("{text}");
                self.report(&text, true);
            }
        }
        self.explorer.set_order(dir.clone(), &changes.order);
        if self.explorer.dir() == dir {
            let was_empty = self.explorer.is_empty();
            self.explorer.reload_renamed(&changes.renamed);
            self.follow_rename(&dir, &changes.renamed);
            if was_empty && !self.explorer.is_empty() {
                self.show_first_link();
            }
            // Forced: the folder and the filters are the same as at the
            // last scan, so an unforced one would not see the new links.
            // `r` forces it for the same reason.
            self.refresh_scan(true);
        }
        if let Some(links) = &mut self.links {
            let wait = started.elapsed() * RELIST_FACTOR;
            links.next = Some(Instant::now() + wait.max(RELIST_EVERY));
        }
        true
    }

    /// Open the view again under its new name, when it shows a link in
    /// `dir` that `renamed` changed. The cursor keeps its line.
    fn follow_rename(&mut self, dir: &Path, renamed: &[(OsString, OsString)]) {
        let shown = self.view.filename();
        if shown.parent() != Some(dir) {
            return;
        }
        let Some(name) = shown.file_name() else {
            return;
        };
        let new = renamed_to(name, renamed);
        if new == name {
            return;
        }
        let path = dir.join(new);
        log::debug!("files-from: the view's link is now {}", path.display());
        let source = self.cursor_source();
        self.open_view(&path, self.current_extent(), Landing::Keep(source));
    }
}
