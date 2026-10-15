//! `o` and `O`: handing a file to another app, and reporting on the status
//! row.

use super::App;
use super::render::status::StatusMessage;
use crate::views::cycle::HandOff;
use crate::{editor, path};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The program that opens a file in the operating system's default app.
const DEFAULT_APP: &str = if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

impl App<'_> {
    /// `O`: open the selected file's enclosing project in the editor.
    ///
    /// Failures are reported on the status row and swallowed. recon is a
    /// viewer; a missing editor is not a reason to bring the TUI down over a
    /// key the user may have pressed by accident.
    pub(super) fn open_project(&mut self) {
        let Some(file) = self.file_to_open() else {
            return;
        };
        let template = self.editor.project.clone();
        let project = editor::project_root(&file);
        self.launch_template(&template, &project, &file);
    }

    /// `o`: the hand-off (#456). Send the selected file to the app its type
    /// names, else a text file to the editor's file template, else to the
    /// operating system's default app.
    pub(super) fn hand_off(&mut self) {
        let Some(file) = self.file_to_open() else {
            return;
        };
        let argv = match self.loading.hand_off(&file) {
            HandOff::Argv(argv) => editor::hand_off_command(&argv, &file, self.line_to_open()),
            HandOff::DefaultApp => vec![DEFAULT_APP.into(), file.clone().into_os_string()],
            HandOff::Editor => {
                let template = self.editor.file.clone();
                // No walk-up. The editor fallback exists for `~/.zshrc` kept
                // inside a dotfiles repo, where climbing would fling open the
                // whole repo — and the file template has no `{project}` in it
                // to receive this anyway, so it is only ever a fallback for a
                // hand-written one.
                let project = file.parent().unwrap_or(&file).to_path_buf();
                self.launch_template(&template, &project, &file);
                return;
            }
        };
        self.launch(&argv, &file);
    }

    /// The selected file as the far end needs it: absolute, and there. `None`,
    /// with the reason on the status row, when there is nothing to open.
    fn file_to_open(&mut self) -> Option<PathBuf> {
        let selected = self.view.filename().to_path_buf();
        if selected.as_os_str().is_empty() {
            self.report("nothing to open", true);
            return None;
        }

        // `filename` is set even when the read failed — the pane shows the
        // error in place of the file's text — so a path that is not there is
        // the ordinary "the argument was a typo" case, not an impossible one.
        //
        // The row names the file alone: the full path can be wider than the
        // row, and the reason is the part truncation would cut (#248). The
        // log keeps the full path, as `read_lines` does.
        if !selected.exists() {
            log::warn!("cannot open {}: no such file", selected.display());
            let name = selected.file_name().unwrap_or(selected.as_os_str());
            self.report(
                &format!("cannot open {}: no such file", name.display()),
                true,
            );
            return None;
        }

        // Absolute, per the `{file}` contract. recon's working directory is not
        // the editor's — a GUI editor launched from a dock or a launcher agent
        // inherits neither — so a relative path is the one input guaranteed to
        // be interpreted differently at the far end.
        //
        // `lexical_absolute` rather than `canonicalize`: it does not touch the
        // filesystem and does not resolve symlinks, so the editor opens the
        // path the explorer is showing rather than wherever it happens to
        // point. For a file reached through a symlinked directory, that is the
        // one the user can find their way back to (#78).
        Some(path::lexical_absolute(&selected))
    }

    /// The cursor's line, 1-based: `cursor_source` indexes the document's
    /// lines, and every editor's `:line` argument counts from one.
    fn line_to_open(&self) -> usize {
        self.cursor_source() + 1
    }

    /// Split and fill an editor template, then start it.
    fn launch_template(&mut self, template: &str, project: &Path, file: &Path) {
        let line = self.line_to_open();
        match editor::editor_command(template, project, file, line) {
            Ok(argv) => self.launch(&argv, file),
            // A template error is the user's typo in a setting, and it can only
            // be reported here: it is not caught at startup, precisely so a bad
            // template does not stop recon opening a log.
            Err(err) => self.report(&err.to_string(), true),
        }
    }

    /// Start `argv`, detached, and say how that went on the status row.
    fn launch(&mut self, argv: &[OsString], file: &Path) {
        // Every template has a program — `editor_command` rejects an empty
        // one — but read defensively anyway rather than indexing, since this
        // runs inside a TUI where a panic takes the terminal with it.
        let program = argv
            .first()
            .map(|program| program.to_string_lossy().into_owned())
            .unwrap_or_default();
        // A terminal editor from `$VISUAL`/`$EDITOR` cannot run without a
        // terminal, and `nvim` would stay alive, hidden (#370). One in a recon
        // template is started as written: that is the user's own choice, and
        // may be a wrapper that opens a window.
        if self.editor.fallback.as_deref() == Some(program.as_str())
            && editor::is_terminal_editor(&program)
        {
            self.report(&editor::terminal_editor_refusal(&program), true);
            return;
        }
        match self.launcher.spawn(argv) {
            // Reported rather than silent: a GUI editor can take seconds to
            // raise a window, and a key that appears to have done nothing is a
            // key that gets pressed again.
            Ok(()) => self.report(&format!("{program}: opening {}", file.display()), false),
            Err(err) => self.report(&format!("{program}: {err}"), true),
        }
    }

    /// Put a one-off message on the status row, replacing any previous one.
    pub(super) fn report(&mut self, text: &str, error: bool) {
        self.status_message = Some(StatusMessage {
            text: text.to_string(),
            error,
        });
    }

    /// Move any editor exit reports onto the status row.
    ///
    /// `try_recv` in a loop, never `recv`: this runs on the render loop and
    /// must not block. Only the last message survives — the row holds one line,
    /// and the most recent failure is the one the user is still wondering
    /// about.
    ///
    /// Returns whether it changed anything. An editor exits on its own
    /// schedule, so this is the one source of change with no keypress behind
    /// it — which makes it the reason `handle_events` cannot simply report
    /// "did an event arrive?" and be done.
    pub(super) fn drain_editor_outcomes(&mut self) -> bool {
        let Some(outcomes) = self.editor_outcomes.as_ref() else {
            return false;
        };
        let mut latest = None;
        while let Ok(message) = outcomes.try_recv() {
            latest = Some(message);
        }
        let Some(text) = latest else {
            return false;
        };
        self.report(&text, true);
        true
    }
}
