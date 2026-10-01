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
    /// The explorer's listed files, one absolute path per line
    Files,
    /// The directory the explorer is showing
    Cwd,
}

/// What a finished session hands back for `main` to print.
#[derive(Debug)]
pub enum Exit {
    /// `q` with `--emit`: the output and the one-line summary for stderr.
    ///
    /// Lines are bytes, not `String`: a Unix filename is not necessarily
    /// UTF-8, and a consumer hands it straight back to the filesystem, so a
    /// lossy conversion would name a file that does not exist.
    Emit {
        lines: Vec<Vec<u8>>,
        summary: String,
        /// Inputs a headless run could not read (#143): warned about as
        /// they were met and skipped, and the reason the exit code is 2.
        /// The TUI always passes 0.
        failed: usize,
    },
    /// `Emit`, with the output in a spool file rather than in memory: the
    /// lines of a file too large to hold, read to its end after `q` (#351).
    /// The file is already unlinked, so nothing is left behind whatever
    /// happens to the process.
    Spooled {
        spool: std::fs::File,
        summary: String,
    },
    /// `Emit`, with the output already written: a headless run (#219,
    /// #379) writes each line as it is made, so `| head` stops the work
    /// and no input's output waits in memory for the others. What is left
    /// to deliver is the end — the summary and the exit code — and how the
    /// writing went: `written` is the first write error, which stopped the
    /// run, or `Ok`.
    Streamed {
        summary: String,
        failed: usize,
        written: std::io::Result<()>,
    },
    /// `Q`, or any quit without `--emit`.
    Silent,
    /// The user cancelled the work `q` started under `--emit` (#351, #352):
    /// nothing on stdout, `recon: cancelled` on stderr, exit 130 — the code
    /// a shell gives a process Ctrl-C stopped. A script never gets partial
    /// output without a sign. Ctrl-c in the TUI ends the session the same
    /// way, with or without `--emit` (#382).
    Cancelled,
    /// A SIGTERM ended the session (#382): nothing on stdout,
    /// `recon: terminated` on stderr, exit 143 — the code a shell gives a
    /// process SIGTERM stopped. recon catches the signal only to restore the
    /// terminal first.
    Terminated,
}

/// Written out rather than derived, for `Spooled`: a file handle has no
/// equality short of reading it, so two spools compare by their summary.
/// `Streamed` compares its write result by error kind, since `io::Error`
/// has no equality. Every other variant compares as a derive would.
impl PartialEq for Exit {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Emit {
                    lines,
                    summary,
                    failed,
                },
                Self::Emit {
                    lines: other_lines,
                    summary: other_summary,
                    failed: other_failed,
                },
            ) => lines == other_lines && summary == other_summary && failed == other_failed,
            (Self::Spooled { summary, .. }, Self::Spooled { summary: other, .. }) => {
                summary == other
            }
            (
                Self::Streamed {
                    summary,
                    failed,
                    written,
                },
                Self::Streamed {
                    summary: other_summary,
                    failed: other_failed,
                    written: other_written,
                },
            ) => {
                summary == other_summary
                    && failed == other_failed
                    && written.as_ref().map_err(std::io::Error::kind)
                        == other_written.as_ref().map_err(std::io::Error::kind)
            }
            (Self::Silent, Self::Silent)
            | (Self::Cancelled, Self::Cancelled)
            | (Self::Terminated, Self::Terminated) => true,
            _ => false,
        }
    }
}

impl Eq for Exit {}

impl Exit {
    /// Print the session's result and say how the process should exit.
    ///
    /// | Exit | `--emit` given | stdout | stderr | code |
    /// |---|---|---|---|---|
    /// | `Emit`, `failed == 0` | yes | every line, newline-terminated | the summary, unless `quiet` | 0 |
    /// | `Emit`, `failed > 0` | yes | every line, newline-terminated | the summary, unless `quiet` | 2 |
    /// | `Spooled` | yes | the spool, byte for byte | the summary, unless `quiet` | 0 |
    /// | `Streamed` | yes | nothing more: already written | the summary, unless `quiet` | 0, or 2 when `failed > 0` |
    /// | `Silent` | yes | nothing | nothing | 1 |
    /// | `Silent` | no | nothing | nothing | 0 |
    /// | `Cancelled` | either | nothing | `recon: cancelled`, even under `quiet` | 130 |
    /// | `Terminated` | either | nothing | `recon: terminated`, even under `quiet` | 143 |
    ///
    /// `Silent` under `--emit` fails because the caller asked for output and
    /// got none: `dir=$(recon --emit cwd) && cd "$dir"` then skips the `cd`
    /// with no test on `$dir`. Empty output from a real emit is a success —
    /// the summary is what tells the two apart. Exit 2 is grep's code for an
    /// input that could not be read: the output for what *was* read is
    /// complete and the summary describes it, so both are still written.
    ///
    /// `quiet` (`-q`) drops the summary and nothing else — the read-failure
    /// warnings were written as they happened, before this runs.
    ///
    /// A write error on stdout — met here, or met by a headless run and
    /// carried in `Streamed` — is reported on stderr and is a failure, with
    /// one exception: `BrokenPipe`, which means the consumer closed its end
    /// (`recon --emit lines big.log | head`) and already got what it asked
    /// for. That is not this process's failure, so the summary is still
    /// written to stderr and the exit code is unchanged. Nothing here can
    /// panic on any of it: the terminal has already been restored, and a
    /// panic's backtrace would be the last thing the user saw.
    pub fn deliver(
        self,
        requested: Option<Emit>,
        quiet: bool,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> ExitCode {
        match (self, requested) {
            (
                Self::Emit {
                    lines,
                    summary,
                    failed,
                },
                _,
            ) => {
                let written = lines
                    .iter()
                    .try_for_each(|line| {
                        stdout
                            .write_all(line)
                            .and_then(|()| stdout.write_all(b"\n"))
                    })
                    .and_then(|()| stdout.flush());
                finish(written, &summary, failed, quiet, stderr)
            }
            (Self::Spooled { mut spool, summary }, _) => {
                use std::io::{Seek, SeekFrom};
                let written = spool
                    .seek(SeekFrom::Start(0))
                    .and_then(|_| std::io::copy(&mut spool, stdout))
                    .and_then(|_| stdout.flush());
                finish(written, &summary, 0, quiet, stderr)
            }
            (
                Self::Streamed {
                    summary,
                    failed,
                    written,
                },
                _,
            ) => finish(written, &summary, failed, quiet, stderr),
            (Self::Cancelled, _) => {
                let _ = writeln!(stderr, "recon: cancelled");
                ExitCode::from(130)
            }
            (Self::Terminated, _) => {
                let _ = writeln!(stderr, "recon: terminated");
                ExitCode::from(143)
            }
            (Self::Silent, Some(_)) => ExitCode::FAILURE,
            (Self::Silent, None) => ExitCode::SUCCESS,
        }
    }
}

/// A new, empty spool file for `Exit::Spooled`: read and write, in the
/// temporary directory, and unlinked at once, so that it takes disk only
/// while this process holds it open and is gone however the process ends.
///
/// # Errors
/// Whatever creating the file reports: no temporary directory, no space.
pub(crate) fn spool() -> std::io::Result<std::fs::File> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("recon-emit-{}-{n}", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    // Where an open file cannot be unlinked, it stays until the next boot
    // clears the temporary directory; that is the whole cost.
    let _ = std::fs::remove_file(&path);
    Ok(file)
}

/// The end of an emit, once its output is written or has failed: the
/// summary and the exit code, by `Exit::deliver`'s table.
fn finish(
    written: std::io::Result<()>,
    summary: &str,
    failed: usize,
    quiet: bool,
    stderr: &mut impl Write,
) -> ExitCode {
    if let Err(err) = written
        && err.kind() != std::io::ErrorKind::BrokenPipe
    {
        let _ = writeln!(stderr, "recon: could not write the output: {err}");
        return ExitCode::FAILURE;
    }
    if !quiet {
        let _ = writeln!(stderr, "{summary}");
    }
    if failed > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
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
        let code = exit.deliver(requested, false, &mut out, &mut err);
        (out, err, code)
    }

    #[test]
    fn an_emit_writes_every_line_newline_terminated_and_the_summary_to_stderr() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec(), b"two".to_vec()],
            summary: "recon: emitted 2 lines".to_string(),
            failed: 0,
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
            failed: 0,
        };

        let (out, err, code) = deliver(exit, Some(Emit::Files));

        assert!(out.is_empty(), "{out:?}");
        assert_eq!(err, b"recon: emitted 0 files from /d, hide mode\n");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    /// #219, #379: the lines are already out, so only the end is left —
    /// the summary, and exit 2 for an input that could not be read.
    #[test]
    fn a_streamed_emit_writes_only_the_summary_and_keeps_the_failure_code() {
        let exit = Exit::Streamed {
            summary: "recon: emitted 2 lines of 1 file, hide mode".to_string(),
            failed: 1,
            written: Ok(()),
        };

        let (out, err, code) = deliver(exit, Some(Emit::Lines));

        assert!(out.is_empty(), "{out:?}");
        assert_eq!(err, b"recon: emitted 2 lines of 1 file, hide mode\n");
        assert_eq!(code, ExitCode::from(2));
    }

    /// A consumer that closed its end got what it asked for; any other
    /// write error is the process's failure, the same rule as `Emit`.
    #[test]
    fn a_streamed_write_error_fails_unless_the_pipe_was_closed() {
        let streamed = |kind| Exit::Streamed {
            summary: "recon: emitted 9 lines".to_string(),
            failed: 0,
            written: Err(std::io::Error::from(kind)),
        };

        let (_, err, code) = deliver(streamed(std::io::ErrorKind::BrokenPipe), Some(Emit::Lines));
        assert_eq!(err, b"recon: emitted 9 lines\n");
        assert_eq!(code, ExitCode::SUCCESS);

        let (_, err, code) = deliver(streamed(std::io::ErrorKind::StorageFull), Some(Emit::Lines));
        assert!(
            String::from_utf8_lossy(&err).starts_with("recon: could not write the output: "),
            "{err:?}"
        );
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn a_spooled_emit_writes_the_spool_from_its_start() {
        let mut spool = spool().expect("a spool");
        spool.write_all(b"one\ntwo\n").expect("write");
        let exit = Exit::Spooled {
            spool,
            summary: "recon: emitted 2 lines of big.log, hide mode".to_string(),
        };

        let (out, err, code) = deliver(exit, Some(Emit::Lines));

        assert_eq!(out, b"one\ntwo\n");
        assert_eq!(err, b"recon: emitted 2 lines of big.log, hide mode\n");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    /// #351, #352: a cancel writes nothing to stdout, says so on stderr
    /// even under `-q`, and exits 130.
    #[test]
    fn a_cancelled_emit_writes_nothing_says_so_and_exits_130() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = Exit::Cancelled.deliver(Some(Emit::Lines), true, &mut out, &mut err);

        assert!(out.is_empty(), "{out:?}");
        assert_eq!(err, b"recon: cancelled\n");
        assert_eq!(code, ExitCode::from(130));
    }

    /// #382: a SIGTERM writes nothing to stdout, says so on stderr even
    /// under `-q`, and exits 143, the code a shell gives a process SIGTERM
    /// stopped.
    #[test]
    fn a_terminated_session_writes_nothing_says_so_and_exits_143() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = Exit::Terminated.deliver(Some(Emit::Lines), true, &mut out, &mut err);

        assert!(out.is_empty(), "{out:?}");
        assert_eq!(err, b"recon: terminated\n");
        assert_eq!(code, ExitCode::from(143));
    }

    #[test]
    fn a_silent_quit_under_emit_writes_nothing_and_fails() {
        let (out, err, code) = deliver(Exit::Silent, Some(Emit::Cwd));

        assert!(out.is_empty(), "{out:?}");
        assert!(err.is_empty(), "{err:?}");
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn a_silent_quit_without_emit_writes_nothing_and_succeeds() {
        let (out, err, code) = deliver(Exit::Silent, None);

        assert!(out.is_empty(), "{out:?}");
        assert!(err.is_empty(), "{err:?}");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn lines_are_written_as_bytes_not_re_encoded() {
        let exit = Exit::Emit {
            lines: vec![vec![0xff, 0xfe, b'x']],
            summary: String::new(),
            failed: 0,
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
            failed: 0,
        };
        let mut stdout = FailingWriter {
            kind: std::io::ErrorKind::BrokenPipe,
        };
        let mut stderr = Vec::new();

        let code = exit.deliver(Some(Emit::Lines), false, &mut stdout, &mut stderr);

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(stderr, b"recon: emitted 1 line\n");
    }

    #[test]
    fn a_different_write_error_on_stdout_still_fails_and_drops_the_summary() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec()],
            summary: "recon: emitted 1 line".to_string(),
            failed: 0,
        };
        let mut stdout = FailingWriter {
            kind: std::io::ErrorKind::Other,
        };
        let mut stderr = Vec::new();

        let code = exit.deliver(Some(Emit::Lines), false, &mut stdout, &mut stderr);

        assert_eq!(code, ExitCode::FAILURE);
        let stderr = String::from_utf8(stderr).expect("utf-8 message");
        assert!(
            stderr.starts_with("recon: could not write the output:"),
            "unexpected stderr: {stderr}"
        );
    }

    #[test]
    fn quiet_drops_the_summary_and_nothing_else() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec()],
            summary: "recon: emitted 1 lines of a.log, hide mode".to_string(),
            failed: 0,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());

        let code = exit.deliver(Some(Emit::Lines), true, &mut out, &mut err);

        assert_eq!(out, b"one\n");
        assert!(err.is_empty(), "stderr: {}", String::from_utf8_lossy(&err));
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn a_failed_input_exits_2_after_the_output_and_the_summary() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec()],
            summary: "recon: emitted 1 lines of 1 file, hide mode".to_string(),
            failed: 1,
        };

        let (out, err, code) = deliver(exit, Some(Emit::Lines));

        assert_eq!(out, b"one\n");
        assert_eq!(err, b"recon: emitted 1 lines of 1 file, hide mode\n");
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn quiet_does_not_hide_the_failure_exit_code() {
        let exit = Exit::Emit {
            lines: Vec::new(),
            summary: "recon: emitted 0 lines of a.log, hide mode".to_string(),
            failed: 1,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());

        let code = exit.deliver(Some(Emit::Lines), true, &mut out, &mut err);

        assert!(err.is_empty(), "{err:?}");
        assert_eq!(code, ExitCode::from(2));
    }
}
