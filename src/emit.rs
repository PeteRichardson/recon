//! Emit on quit (#143): what a finished session hands back, and how `main`
//! prints it.
//!
//! `App` decides what leaves the process; this module is the only place that
//! knows it leaves through stdout and stderr, and `main` is the only caller.
//! Keeping the writers as parameters is what lets every table row in
//! `Exit::deliver` be tested against a `Vec<u8>` with no terminal.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

/// What `--emit` asks for. A clap `ValueEnum`: the variant doc comments are
/// the `--help` text for each value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Emit {
    /// The file view's visible lines, in the current mode
    Lines,
    /// The navigator's listed files, one absolute path per line
    Files,
    /// The directory the navigator is showing
    Cwd,
}

/// What a finished session hands back for `main` to print.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    /// `q` with `--emit`: the output and the one-line summary for stderr.
    ///
    /// Lines are bytes, not `String`: a Unix filename is not necessarily
    /// UTF-8, and a consumer hands it straight back to the filesystem, so a
    /// lossy conversion would name a file that does not exist.
    Emit {
        lines: Vec<Vec<u8>>,
        summary: String,
    },
    /// `Q`, or any quit without `--emit`.
    Silent,
}

impl Exit {
    /// Print the session's result and say how the process should exit.
    ///
    /// | Exit | `--emit` given | stdout | stderr | code |
    /// |---|---|---|---|---|
    /// | `Emit` | yes | every line, newline-terminated | the summary | 0 |
    /// | `Silent` | yes | nothing | nothing | 1 |
    /// | `Silent` | no | nothing | nothing | 0 |
    ///
    /// `Silent` under `--emit` fails because the caller asked for output and
    /// got none: `dir=$(recon --emit cwd) && cd "$dir"` then skips the `cd`
    /// with no test on `$dir`. Empty output from a real emit is a success —
    /// the summary is what tells the two apart.
    ///
    /// A write error on stdout is reported on stderr and is a failure, with
    /// one exception: `BrokenPipe`, which means the consumer closed its end
    /// (`recon --emit lines big.log | head`) and already got what it asked
    /// for. That is not this process's failure, so the summary is still
    /// written to stderr and the exit code is still success. Nothing here can
    /// panic on any of it: the terminal has already been restored, and a
    /// panic's backtrace would be the last thing the user saw.
    pub fn deliver(
        self,
        requested: Option<Emit>,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> ExitCode {
        match (self, requested) {
            (Self::Emit { lines, summary }, _) => {
                let written = lines
                    .iter()
                    .try_for_each(|line| {
                        stdout
                            .write_all(line)
                            .and_then(|()| stdout.write_all(b"\n"))
                    })
                    .and_then(|()| stdout.flush());
                if let Err(err) = written
                    && err.kind() != std::io::ErrorKind::BrokenPipe
                {
                    let _ = writeln!(stderr, "recon: could not write the output: {err}");
                    return ExitCode::FAILURE;
                }
                let _ = writeln!(stderr, "{summary}");
                ExitCode::SUCCESS
            }
            (Self::Silent, Some(_)) => ExitCode::FAILURE,
            (Self::Silent, None) => ExitCode::SUCCESS,
        }
    }
}

/// A path as the bytes the filesystem holds, for an output line.
///
/// On Unix that is the `OsStr` verbatim. Elsewhere paths are not bytes at
/// all, and the lossy string is the only honest rendering.
#[cfg(unix)]
pub(crate) fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
pub(crate) fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deliver(exit: Exit, requested: Option<Emit>) -> (Vec<u8>, Vec<u8>, ExitCode) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = exit.deliver(requested, &mut out, &mut err);
        (out, err, code)
    }

    #[test]
    fn an_emit_writes_every_line_newline_terminated_and_the_summary_to_stderr() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec(), b"two".to_vec()],
            summary: "recon: emitted 2 lines".to_string(),
        };

        let (out, err, code) = deliver(exit, Some(Emit::Lines));

        assert_eq!(out, b"one\ntwo\n");
        assert_eq!(err, b"recon: emitted 2 lines\n");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn an_empty_emit_is_a_success_with_nothing_on_stdout() {
        let exit = Exit::Emit {
            lines: Vec::new(),
            summary: "recon: emitted 0 files from /d, hide mode".to_string(),
        };

        let (out, err, code) = deliver(exit, Some(Emit::Files));

        assert!(out.is_empty());
        assert_eq!(err, b"recon: emitted 0 files from /d, hide mode\n");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn a_silent_quit_under_emit_writes_nothing_and_fails() {
        let (out, err, code) = deliver(Exit::Silent, Some(Emit::Cwd));

        assert!(out.is_empty());
        assert!(err.is_empty());
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn a_silent_quit_without_emit_writes_nothing_and_succeeds() {
        let (out, err, code) = deliver(Exit::Silent, None);

        assert!(out.is_empty());
        assert!(err.is_empty());
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn lines_are_written_as_bytes_not_re_encoded() {
        let exit = Exit::Emit {
            lines: vec![vec![0xff, 0xfe, b'x']],
            summary: String::new(),
        };

        let (out, _, _) = deliver(exit, Some(Emit::Files));

        assert_eq!(out, vec![0xff, 0xfe, b'x', b'\n']);
    }

    #[cfg(unix)]
    #[test]
    fn path_bytes_keeps_a_non_utf8_filename_intact() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(OsStr::from_bytes(b"/tmp/bad\xffname"));

        assert_eq!(path_bytes(path), b"/tmp/bad\xffname".to_vec());
    }

    /// A stdout stand-in whose every write fails with a fixed error kind —
    /// there is no way to close a real pipe from within a unit test.
    struct FailingWriter {
        kind: std::io::ErrorKind,
    }

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(self.kind))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_broken_pipe_on_stdout_is_a_success_and_the_summary_still_lands() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec()],
            summary: "recon: emitted 1 line".to_string(),
        };
        let mut stdout = FailingWriter {
            kind: std::io::ErrorKind::BrokenPipe,
        };
        let mut stderr = Vec::new();

        let code = exit.deliver(Some(Emit::Lines), &mut stdout, &mut stderr);

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(stderr, b"recon: emitted 1 line\n");
    }

    #[test]
    fn a_different_write_error_on_stdout_still_fails_and_drops_the_summary() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec()],
            summary: "recon: emitted 1 line".to_string(),
        };
        let mut stdout = FailingWriter {
            kind: std::io::ErrorKind::Other,
        };
        let mut stderr = Vec::new();

        let code = exit.deliver(Some(Emit::Lines), &mut stdout, &mut stderr);

        assert_eq!(code, ExitCode::FAILURE);
        let stderr = String::from_utf8(stderr).expect("utf-8 message");
        assert!(
            stderr.starts_with("recon: could not write the output:"),
            "unexpected stderr: {stderr}"
        );
    }
}
