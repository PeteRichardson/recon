//! Putting yanked text on the system clipboard (#67).
//!
//! The same shape as `editor.rs`, for the same reasons: the clipboard is a
//! **command template** the user can change, split once by
//! [`editor::split_template`] so nothing is ever handed to a shell, and run
//! behind a trait so every test asserts on what *would* have been copied
//! rather than clobbering the developer's real pasteboard.
//!
//! Not an OSC 52 escape. Terminal.app does not honour it, and several
//! terminals ship it disabled, so it fails silently exactly where a command
//! fails loudly — and a failure here has a status row to land on. A user on a
//! terminal that does support it can put a small script in the template.

use std::io::Write as _;
use std::process::{Command, Stdio};

use crate::editor::{self, TemplateError};

/// Where yanked text goes.
///
/// A trait so the `y` path can be tested end to end without a process ever
/// running, exactly as [`editor::Launcher`] lets `o` be.
pub trait Clipboard {
    /// Put `text` on the clipboard, replacing what was there.
    fn copy(&self, text: &str) -> std::io::Result<()>;
}

/// So `App` can hold a `Box<dyn Clipboard>` without the field becoming an
/// `Option` — the same argument as `Box<dyn Launcher>`'s default. The default
/// runs the platform's command; `App::new` replaces it with the configured
/// template.
impl Default for Box<dyn Clipboard> {
    fn default() -> Self {
        Box::new(ProcessClipboard::default())
    }
}

/// The compiled-in bottom of the clipboard ladder: what this platform
/// conventionally has.
///
/// Wayland is detected by its display variable rather than assumed from the
/// platform: a Linux desktop can be either, and `xclip` under Wayland copies
/// to a clipboard only X clients can see.
#[must_use]
pub fn default_template() -> String {
    if cfg!(target_os = "macos") {
        "pbcopy".to_string()
    } else if cfg!(windows) {
        "clip".to_string()
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        "wl-copy".to_string()
    } else {
        "xclip -selection clipboard".to_string()
    }
}

/// The real clipboard: a command with the text on its stdin.
///
/// Holds the template rather than a split argv, and splits on every `copy`:
/// a typo in the template is then reported by the `y` that uses it, on the
/// status row, rather than refusing to start a log viewer over a setting
/// most sessions never touch — the same call `editor::Templates` makes.
#[derive(Debug, Default)]
pub struct ProcessClipboard {
    /// The command template. Empty means the platform default.
    template: String,
}

impl ProcessClipboard {
    #[must_use]
    pub fn new(template: &str) -> Self {
        Self {
            template: template.to_string(),
        }
    }

    /// The command that will run: the template, or the platform's default
    /// when there is none.
    fn argv(&self) -> Result<Vec<String>, TemplateError> {
        if self.template.trim().is_empty() {
            editor::split_template(&default_template())
        } else {
            editor::split_template(&self.template)
        }
    }
}

impl Clipboard for ProcessClipboard {
    fn copy(&self, text: &str) -> std::io::Result<()> {
        let argv = self
            .argv()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
        let Some((program, args)) = argv.split_first() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "empty clipboard command",
            ));
        };
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            // Both, or the child draws over the alternate screen recon is
            // holding — the same reasoning `ProcessLauncher` gives.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
            // Dropped here, so the child sees EOF before `wait` below.
        }
        // Waited on, unlike an editor: `pbcopy`, `xclip` and `wl-copy` all
        // return as soon as they have read their input, and a non-zero exit
        // is the only way they report a clipboard they could not reach.
        let status = child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "{program} exited with {status}"
            )))
        }
    }
}

/// The test double the [`Clipboard`] trait exists for. Outside `mod tests`
/// so `lib.rs`'s tests can reach it, as `editor::double` is.
#[cfg(test)]
pub(crate) mod double {
    use super::Clipboard;
    use std::sync::Mutex;

    /// Records every text it was handed, and can be told to fail.
    #[derive(Default)]
    pub(crate) struct RecordingClipboard {
        pub copies: Mutex<Vec<String>>,
        pub fail_with: Option<String>,
    }

    impl RecordingClipboard {
        pub(crate) fn failing(message: &str) -> Self {
            Self {
                fail_with: Some(message.to_string()),
                ..Self::default()
            }
        }

        /// Everything copied so far, in order.
        pub(crate) fn copies(&self) -> Vec<String> {
            self.copies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        /// The single text recorded, or a panic naming what was there
        /// instead.
        pub(crate) fn only_copy(&self) -> String {
            let copies = self.copies();
            assert_eq!(copies.len(), 1, "expected one copy: {copies:?}");
            copies[0].clone()
        }
    }

    impl Clipboard for RecordingClipboard {
        fn copy(&self, text: &str) -> std::io::Result<()> {
            self.copies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(text.to_string());
            match &self.fail_with {
                Some(message) => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    message.clone(),
                )),
                None => Ok(()),
            }
        }
    }

    /// So a test can keep a handle on the recording while `App` owns the
    /// clipboard, as `Rc<RecordingLauncher>` does for the editor.
    impl Clipboard for std::rc::Rc<RecordingClipboard> {
        fn copy(&self, text: &str) -> std::io::Result<()> {
            (**self).copy(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_a_single_program_the_platform_has() {
        let argv = ProcessClipboard::default()
            .argv()
            .expect("the default splits");
        let program = argv.first().expect("the default names a program");
        assert!(
            ["pbcopy", "clip", "wl-copy", "xclip"].contains(&program.as_str()),
            "unexpected default clipboard command {argv:?}"
        );
    }

    #[test]
    fn a_template_is_split_once_without_a_shell() {
        let clipboard = ProcessClipboard::new("xclip -selection 'clip board'");
        assert_eq!(
            clipboard.argv().expect("splits"),
            ["xclip", "-selection", "clip board"]
        );
    }

    #[test]
    fn a_bad_template_is_reported_by_the_copy_not_at_startup() {
        let clipboard = ProcessClipboard::new("pbcopy 'unterminated");
        let err = clipboard
            .copy("x")
            .expect_err("an unclosed quote is an error");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("quote"), "{err}");
    }

    /// The whole path through a real process, with `cat` standing in for the
    /// clipboard tool: the text arrives on stdin, and a command that is not
    /// there reports rather than panics.
    #[cfg(unix)]
    #[test]
    fn the_text_is_written_to_the_commands_stdin() {
        let dir = std::path::Path::new("target/test-clipboard");
        std::fs::create_dir_all(dir).expect("create clipboard fixture dir");
        let out = dir.join("copied.txt");
        let _ = std::fs::remove_file(&out);
        let clipboard = ProcessClipboard::new(&format!("sh -c 'cat > {}'", out.display()));
        clipboard.copy("alpha\nbeta\n").expect("cat runs");
        assert_eq!(
            std::fs::read_to_string(&out).expect("cat wrote"),
            "alpha\nbeta\n"
        );

        let missing = ProcessClipboard::new("recon-no-such-clipboard-tool");
        assert!(
            missing.copy("x").is_err(),
            "a missing tool is an error, not a panic"
        );

        let failing = ProcessClipboard::new("sh -c 'exit 3'");
        let err = failing.copy("x").expect_err("a non-zero exit is an error");
        assert!(err.to_string().contains("exit"), "{err}");
    }
}
