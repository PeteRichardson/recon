//! `-` in the TUI (#517): the file stdin is copied into, on screen.
//!
//! The view shows it as any followed file; this adds what only stdin has:
//! a title that says whether more may come, and a word on the status line
//! as it grows past each tenfold size.

use super::App;
use super::finish::size;
use crate::stdin::{LABEL, State, StdinFile};

/// The file stdin is copied into, and the state the title last showed.
pub(super) struct Stdin {
    file: StdinFile,
    shown: State,
}

/// The view's title for the file stdin is copied into.
fn title(state: &State) -> String {
    match state {
        State::Open => format!("{LABEL} (open)"),
        State::Closed => format!("{LABEL} (closed)"),
        State::Stopped(reason) => format!("{LABEL} (stopped: {reason})"),
    }
}

impl App<'_> {
    /// Show `file`, which stdin is being copied into, as `<stdin>`. `App`
    /// keeps it, so that `[stdin] delete_temp_file` acts when the session
    /// ends.
    #[must_use]
    pub fn with_stdin(mut self, file: Option<StdinFile>) -> Self {
        if let Some(file) = file {
            let shown = file.state();
            self.view.set_title_alias(file.path(), title(&shown));
            self.stdin = Some(Stdin { file, shown });
        }
        self
    }

    /// Take what the copy did since the last call, and say whether the
    /// screen changed: a new title, or a size passed. Once the copy is
    /// over, nothing more can change, and it is not asked again.
    pub(super) fn poll_stdin(&mut self) -> bool {
        let Some(stdin) = &mut self.stdin else {
            return false;
        };
        if stdin.shown != State::Open {
            return false;
        }
        let state = stdin.file.state();
        let milestones = stdin.file.take_milestones();
        let changed = state != stdin.shown || !milestones.is_empty();
        if state != stdin.shown {
            let text = title(&state);
            self.view.set_title_alias(stdin.file.path(), text.clone());
            let stopped = matches!(state, State::Stopped(_));
            stdin.shown = state;
            if stopped {
                log::error!("stdin: {text}");
                self.report(&text, true);
            } else {
                log::debug!("stdin: {text}");
            }
        }
        for bytes in milestones {
            let text = format!("{LABEL}: {} so far", size(bytes));
            log::info!("{text}");
            self.report(&text, false);
        }
        changed
    }
}
