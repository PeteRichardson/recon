//! Opening a file in an editor, and reporting on the status row.

use super::render::status::StatusMessage;
use super::{App, EditorScope};
use crate::{editor, path};

impl App<'_> {
    /// Hand the selected file to an editor.
    ///
    /// Every step after the walk-up is shared by both bindings, which is what
    /// made `O` one key and one `match` arm rather than a second copy of this:
    /// `scope` decides whether to climb, and the template decides what the
    /// command looks like. Nothing else differs.
    ///
    /// Failures are reported on the status row and swallowed. recon is a
    /// viewer; a missing editor is not a reason to bring the TUI down over a
    /// key the user may have pressed by accident.
    pub(super) fn open_in_editor(&mut self, template: &str, scope: EditorScope) {
        let relative = self.view.filename().to_path_buf();
        if relative.as_os_str().is_empty() {
            self.report("nothing to open", true);
            return;
        }

        // `filename` is set even when the read failed — the pane shows the
        // error in place of the file's text — so a path that is not there is
        // the ordinary "the argument was a typo" case, not an impossible one.
        if !relative.exists() {
            self.report(
                &format!("cannot open {}: no such file", relative.display()),
                true,
            );
            return;
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
        // one the user can find their way back to.
        //
        // That claim was false until #78. `Explorer::set_dir` canonicalized, so
        // the explorer had *already* resolved the link before this ran and
        // there was nothing left here to preserve. All three sites share this
        // one function now, which is what makes the sentence above true.
        let file = path::lexical_absolute(&relative);

        let project = match scope {
            EditorScope::Project => editor::project_root(&file),
            // No walk-up. `O` exists precisely for `~/.zshrc` kept inside a
            // dotfiles repo, where climbing would fling open the whole repo —
            // and the file template has no `{project}` in it to receive this
            // anyway, so it is only ever a fallback for a hand-written one.
            EditorScope::File => file.parent().unwrap_or(&file).to_path_buf(),
        };

        // 1-based: `cursor_source` indexes the document's lines, and every
        // editor's `:line` argument counts from one.
        let line = self.cursor_source() + 1;
        let argv = match editor::editor_command(template, &project, &file, line) {
            Ok(argv) => argv,
            // A template error is the user's typo in a setting, and it can only
            // be reported here: it is not caught at startup, precisely so a bad
            // template does not stop recon opening a log.
            Err(err) => {
                self.report(&err.to_string(), true);
                return;
            }
        };

        // `editor_command` rejects an empty template, so there is always a
        // program — read defensively anyway rather than indexing, since this
        // runs inside a TUI where a panic takes the terminal with it.
        let program = argv
            .first()
            .map(|program| program.to_string_lossy().into_owned())
            .unwrap_or_default();
        match self.launcher.spawn(&argv) {
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
