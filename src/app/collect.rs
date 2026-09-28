//! What recon writes to stdout when it quits (`--emit`).

use super::App;
use crate::document::Mode;
use crate::emit;

impl App<'_> {
    /// The output `--emit <kind>` asks for, from what the panes are showing
    /// (#143). Reads the visible sets; computes nothing new.
    pub(super) fn collect(&self, kind: emit::Emit) -> emit::Exit {
        match kind {
            emit::Emit::Lines => self.collect_lines(),
            emit::Emit::Files => self.collect_files(),
            emit::Emit::Cwd => self.collect_cwd(),
        }
    }

    /// `--emit lines`: the file view's visible lines in the current mode,
    /// with `-n` prefixing each by its 1-based source line number and a tab.
    fn collect_lines(&self) -> emit::Exit {
        if self.view.showing_directory() {
            return emit::Exit::Emit {
                lines: Vec::new(),
                summary: "recon: emitted 0 lines — the view is showing a directory".to_string(),
                failed: 0,
            };
        }
        if !self.view.is_text() {
            return emit::Exit::Emit {
                lines: Vec::new(),
                summary: "recon: emitted 0 lines — the view is showing an error, not a file"
                    .to_string(),
                failed: 0,
            };
        }
        let text = self.document.lines();
        let visible = self.document.visible();
        let lines = visible
            .iter()
            .map(|&source| {
                let mut line = Vec::new();
                if self.line_numbers {
                    line.extend_from_slice(format!("{}\t", source + 1).as_bytes());
                }
                line.extend_from_slice(text[source].as_bytes());
                line
            })
            .collect();
        let name = self.view.filename().display();
        let summary = match self.document.mode() {
            Mode::Dimmed => format!(
                "recon: emitted {} lines of {name}, dim mode ({} match) — Ctrl-H to emit matches only",
                visible.len(),
                self.interesting_count(),
            ),
            Mode::FilteredOnly => {
                format!(
                    "recon: emitted {} lines of {name}, hide mode",
                    visible.len()
                )
            }
        };
        emit::Exit::Emit {
            lines,
            summary,
            failed: 0,
        }
    }

    /// `--emit files`: the explorer's listed files as absolute paths. Hide
    /// mode has already dropped the non-matching rows, so the list is the
    /// matches; dim mode lists every file and the summary says how many
    /// match, and how many the scan has not answered yet.
    ///
    /// The counting branch follows what the explorer can answer, not
    /// whether any filter is switched on: `matcher()` is `None` both with no
    /// including filter enabled (an exclude-only set, or none at all) and
    /// above `MAX_PATTERNS`, and in both states `refresh_scan` never runs, so
    /// every file sits at `Match::Unknown` and a "0 match, N unscanned" line
    /// would describe a scan that will never happen.
    fn collect_files(&self) -> emit::Exit {
        let listed = self.explorer.listed_files();
        let lines = listed
            .iter()
            .map(|file| emit::path_bytes(&file.path))
            .collect();
        let dir = self.explorer.dir().display();
        let count = listed.len();
        let summary = if self.filters.is_scanning() {
            match self.document.mode() {
                Mode::FilteredOnly => format!("recon: emitted {count} files from {dir}, hide mode"),
                Mode::Dimmed => {
                    let matched = listed.iter().filter(|f| f.matched == Some(true)).count();
                    let unscanned = listed.iter().filter(|f| f.matched.is_none()).count();
                    let counts = if unscanned == 0 {
                        format!("{matched} match")
                    } else {
                        format!("{matched} match, {unscanned} unscanned")
                    };
                    format!(
                        "recon: emitted {count} files from {dir}, dim mode ({counts}) — Ctrl-H to emit matches only"
                    )
                }
            }
        } else {
            let mode = match self.document.mode() {
                Mode::Dimmed => "dim mode",
                Mode::FilteredOnly => "hide mode",
            };
            format!("recon: emitted {count} files from {dir}, {mode}, no filter")
        };
        emit::Exit::Emit {
            lines,
            summary,
            failed: 0,
        }
    }

    /// `--emit cwd`: the directory the explorer is showing, one line.
    fn collect_cwd(&self) -> emit::Exit {
        let dir = self.explorer.dir();
        emit::Exit::Emit {
            lines: vec![emit::path_bytes(dir)],
            summary: format!("recon: emitted {}", dir.display()),
            failed: 0,
        }
    }
}
