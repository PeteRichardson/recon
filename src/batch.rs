//! Batch mode (#143): `--emit` with stdin that is not a terminal.
//!
//! The pieces `App` composes — `ActiveFilters`, `Document`, `scan::scan` —
//! with no explorer, no view and no terminal. Files come from stdin or
//! from the `PATH` argument. The output goes to stdout as it is made
//! (#219, #379); the end of the run — summary, failure count, how the
//! writing went — leaves through the same `Exit` a TUI session hands back,
//! so `main` finishes both the same way.

use crate::app::viewport::is_interesting;
use crate::document::{self, Document, Mode};
use crate::emit::{Emit, Exit, path_bytes};
use crate::filter::{ActiveFilters, Matcher};
use crate::path::lexical_absolute;
use crate::scan::{self, Progress};
use crate::startup::Startup;
use crate::widgets::explorer::sorted_entries;
use color_eyre::{Result, eyre::eyre};
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// The files a batch run reads, and where the list came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Inputs {
    /// Absolute, in the order they were given.
    pub files: Vec<PathBuf>,
    pub from: Source,
}

/// Where the input list came from — the summary and `cwd` differ by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// One path per line on stdin.
    Stdin,
    /// `PATH` named a directory: its files, in the explorer's order.
    Directory(PathBuf),
    /// `PATH` named a file — or nothing that exists: that path alone.
    File,
}

/// Run in batch mode: read the input list, build the filters `--set` asks for,
/// and write what `--emit` names to `out`. Read-failure warnings go to
/// stderr as they are met; the `Exit` carries the summary, the failure
/// count and any write error for `main` to deliver.
pub fn run(startup: &Startup, out: &mut impl Write) -> Result<Exit> {
    let config = &startup.config;
    let Some(what) = config.emit else {
        return Err(eyre!("batch mode needs --emit"));
    };
    let inputs = inputs(io::stdin().lock(), Path::new(&config.path))?;
    let filters = filters_for(startup)?;
    let mode = if config.hide {
        Mode::FilteredOnly
    } else {
        Mode::Dimmed
    };
    Ok(collect(
        what,
        &inputs,
        &filters,
        mode,
        config.line_numbers,
        out,
        &mut io::stderr(),
    ))
}

/// The startup filter set: the loaded sets, then each `--set` enabled and
/// each `--unlist` unlisted — the same steps `App::new` takes.
fn filters_for(startup: &Startup) -> Result<ActiveFilters> {
    let config = &startup.config;
    let mut filters = ActiveFilters::with_sets(Some(config.filter_palette()), &startup.filter_sets);
    filters.set_background(config.background());
    for (set, profile) in config.sets_to_enable() {
        filters
            .enable_named(&set, profile.as_deref())
            .map_err(|err| eyre!("--set {set}: {err}"))?;
    }
    for set in &config.unlist {
        filters
            .unlist_named(set)
            .map_err(|err| eyre!("--unlist {set}: {err}"))?;
    }
    Ok(filters)
}

/// What `--emit` names, over `inputs`, written to `out` one line at a time
/// as each input is answered. `warnings` gets one line per input that could
/// not be read.
///
/// The first write error stops the run: no later input is read, so
/// `recon --emit lines big.log | head -2` ends when `head` does (#219). The
/// error goes back in `Exit::Streamed` for `deliver` to judge, and the
/// summary counts the lines handed to `out` before it.
pub(crate) fn collect(
    what: Emit,
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    line_numbers: bool,
    out: &mut impl Write,
    warnings: &mut impl Write,
) -> Exit {
    match what {
        Emit::Lines => collect_lines(inputs, filters, mode, line_numbers, out, warnings),
        Emit::Files => collect_files(inputs, filters, mode, out, warnings),
        Emit::Cwd => collect_cwd(inputs, out),
    }
}

/// Read the input list: every non-blank line of `stdin` as a path, or, when
/// stdin held none, what `path` names — a directory's files in the
/// explorer's order, or the file itself.
///
/// Lines are bytes, not `String`s, for the reason `Entry::name` is an
/// `OsString`: a Unix filename need not be UTF-8, and `ls -1` writes it
/// verbatim. A trailing `\r` is dropped so a CRLF list works; nothing else
/// is trimmed, since a name can end in a space. A line that is only
/// whitespace is skipped.
pub(crate) fn inputs(mut stdin: impl BufRead, path: &Path) -> io::Result<Inputs> {
    let mut files = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if stdin.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let bytes = strip_line_end(&line);
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        files.push(lexical_absolute(&path_from_bytes(bytes)));
    }
    if !files.is_empty() {
        return Ok(Inputs {
            files,
            from: Source::Stdin,
        });
    }

    let path = lexical_absolute(path);
    if path.is_dir() {
        let files = sorted_entries(&path)?
            .into_iter()
            .filter(|entry| entry.kind.is_readable())
            .map(|entry| path.join(entry.name))
            .collect();
        return Ok(Inputs {
            files,
            from: Source::Directory(path),
        });
    }
    Ok(Inputs {
        files: vec![path],
        from: Source::File,
    })
}

/// `--emit lines`: every input's visible lines in `mode`, in input order,
/// each prefixed by `path<TAB>` when there is more than one input and by
/// `N<TAB>` under `-n` — so `path<TAB>N<TAB>line`, and with one input
/// exactly what the TUI emits. The match count is the TUI's: interesting
/// verdicts, summed over the files that were read.
///
/// Each file's lines are written once it is evaluated, so memory holds one
/// file at a time, not the whole output.
fn collect_lines(
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    line_numbers: bool,
    out: &mut impl Write,
    warnings: &mut impl Write,
) -> Exit {
    let several = inputs.files.len() > 1;
    let mut emitted = 0;
    let mut read = 0;
    let mut failed = 0;
    let mut interesting = 0;
    let mut written = Ok(());
    'files: for path in &inputs.files {
        let mut document = match Document::read(path) {
            Ok(document) => document,
            Err(err) => {
                warn(warnings, path, &err);
                failed += 1;
                continue;
            }
        };
        // The mode first: `evaluate` derives the visible set from the
        // verdicts *and* the mode, so setting it afterwards would need a
        // second pass.
        document.set_mode(mode);
        document.evaluate(filters);
        read += 1;
        // The TUI's definition, `App::interesting_count` in viewport.rs.
        interesting += document
            .verdicts()
            .iter()
            .filter(|v| is_interesting(v))
            .count();
        let text = document.lines();
        let prefix = several.then(|| path_bytes(path));
        for &source in document.visible() {
            written = write_line(out, prefix.as_deref(), line_numbers, source, &text[source]);
            if written.is_err() {
                break 'files;
            }
            emitted += 1;
        }
    }
    let written = written.and_then(|()| out.flush());
    let subject = match inputs.files.as_slice() {
        [only] => only.file_name().map_or_else(
            || only.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        _ => count(read, "file"),
    };
    let summary = match mode {
        Mode::Dimmed => format!(
            "recon: emitted {emitted} lines of {subject}, dim mode ({interesting} match) — pass --hide to emit matches only"
        ),
        Mode::FilteredOnly => format!("recon: emitted {emitted} lines of {subject}, hide mode"),
    };
    Exit::Streamed {
        summary,
        failed,
        written,
    }
}

/// One `--emit lines` line: `path<TAB>` when there is a `path`, `N<TAB>`
/// under `-n`, the text, a newline. Straight to `out`, with no line built
/// in between.
fn write_line(
    out: &mut impl Write,
    path: Option<&[u8]>,
    line_numbers: bool,
    source: usize,
    text: &str,
) -> io::Result<()> {
    if let Some(path) = path {
        out.write_all(path)?;
        out.write_all(b"\t")?;
    }
    if line_numbers {
        write!(out, "{}\t", source + 1)?;
    }
    out.write_all(text.as_bytes())?;
    out.write_all(b"\n")
}

/// `--emit files`: every readable input in dim mode; in hide mode, the
/// inputs the matcher selects — or every readable input when nothing
/// selects, which is the explorer's rule: it hides nothing it cannot mark.
///
/// One scan per file, stopping at the first selecting line, which is what
/// the explorer's scan costs. With nothing to scan, each input is still
/// opened, so an unreadable one is warned about and skipped in every mode.
/// The summary's `from <dir>` / `of N inputs` follows where the list came
/// from; there is no `unscanned` here, since every scan runs to its answer
/// before its file's line prints.
///
/// A definition filter takes a parse, not a scan (#224): a scan reads only
/// text, so the matcher skips it. When one is effective, every file is
/// answered by `file_defines` instead — every filter, regex ones too, from
/// one read and one grammar pass per file. The navigator's marks cannot
/// afford that per frame; a batch run can, so batch mode differs from `q`
/// here.
fn collect_files(
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    out: &mut impl Write,
    warnings: &mut impl Write,
) -> Exit {
    let parse = filters.needs_kinds();
    let matcher = if parse { None } else { filters.matcher() };
    // Whether some filter decides the answer. `false` is the explorer's
    // "nothing to mark", and the summary's `no filter`.
    let live = parse || matcher.is_some();
    // An include asked for matching and there is none, so every file reads
    // as unmatched. Said once, on the warnings channel, so a script does not
    // take the result for a real answer (#306). Only a scan has the limit.
    if !parse && let Some(off) = filters.scan_off() {
        let _ = writeln!(warnings, "recon: {off}; no file is matched");
    }
    let mut emitted = 0;
    let mut matched = 0;
    let mut failed = 0;
    let mut written = Ok(());
    for path in &inputs.files {
        let answer = match &matcher {
            _ if parse => file_defines(path, filters),
            Some(matcher) => file_matches(path, matcher),
            None => open_input(path).map(|_| false),
        };
        let yes = match answer {
            Ok(yes) => yes,
            Err(err) => {
                warn(warnings, path, &err);
                failed += 1;
                continue;
            }
        };
        if yes {
            matched += 1;
        }
        let listed = match mode {
            Mode::Dimmed => true,
            Mode::FilteredOnly => yes || !live,
        };
        if listed {
            written = out
                .write_all(&path_bytes(path))
                .and_then(|()| out.write_all(b"\n"));
            if written.is_err() {
                break;
            }
            emitted += 1;
        }
    }
    let written = written.and_then(|()| out.flush());
    let origin = match &inputs.from {
        Source::Directory(dir) => format!("from {}", dir.display()),
        Source::Stdin | Source::File => format!("of {}", count(inputs.files.len(), "input")),
    };
    let summary = match (live, mode) {
        (true, Mode::FilteredOnly) => {
            format!("recon: emitted {emitted} files {origin}, hide mode")
        }
        (true, Mode::Dimmed) => format!(
            "recon: emitted {emitted} files {origin}, dim mode ({matched} match) — pass --hide to emit matches only"
        ),
        (false, Mode::Dimmed) => {
            format!("recon: emitted {emitted} files {origin}, dim mode, no filter")
        }
        (false, Mode::FilteredOnly) => {
            format!("recon: emitted {emitted} files {origin}, hide mode, no filter")
        }
    };
    Exit::Streamed {
        summary,
        failed,
        written,
    }
}

/// Open an input for scanning. A directory is refused up front: `File::open`
/// accepts one on Unix and only the read fails, and `scan` swallows a read
/// error as end of file. So is a FIFO, socket or device (#221), whose open
/// would block or whose read has no end — `document::refuse_unreadable` is
/// the one stat both readers share.
fn open_input(path: &Path) -> io::Result<File> {
    document::refuse_unreadable(path)?;
    File::open(path)
}

/// Whether any line of `path` selects under `matcher` — `Record::answer`'s
/// rule, over a scan run to its answer. The same read as the explorer's, so a
/// UTF-16 log is decoded here too (#357).
fn file_matches(path: &Path, matcher: &Matcher) -> io::Result<bool> {
    let progress = scan::scan_file(
        open_input(path)?,
        matcher,
        Progress::default(),
        &AtomicBool::new(false),
    );
    // The positive half of `scan::Record::answer`: a selecting bitmask was seen.
    Ok(progress.seen.iter().any(|&bits| matcher.selects(bits)))
}

/// Whether any line of `path` is included by `filters`, read and evaluated
/// as `--emit lines` does it — the rule its match count uses. The whole file
/// is read: a kind the grammar never names anywhere in the file falls back
/// to keywords (`syntax::definitions`), so no line can answer early.
///
/// A file `Document::read` refuses as binary has no grammar to parse, so
/// it defines nothing: `Ok(false)`, not a failure.
fn file_defines(path: &Path, filters: &ActiveFilters) -> io::Result<bool> {
    let mut document = match Document::read(path) {
        Ok(document) => document,
        Err(err) if document::is_binary(&err) => return Ok(false),
        Err(err) => return Err(err),
    };
    document.evaluate(filters);
    Ok(document.verdicts().iter().any(is_interesting))
}

/// `--emit cwd`: the directory `PATH` named, or the first input's. Nothing
/// is read.
fn collect_cwd(inputs: &Inputs, out: &mut impl Write) -> Exit {
    let dir = match &inputs.from {
        Source::Directory(dir) => dir.clone(),
        Source::Stdin | Source::File => inputs
            .files
            .first()
            .and_then(|file| file.parent())
            .map_or_else(|| PathBuf::from("/"), Path::to_path_buf),
    };
    let written = out
        .write_all(&path_bytes(&dir))
        .and_then(|()| out.write_all(b"\n"))
        .and_then(|()| out.flush());
    Exit::Streamed {
        summary: format!("recon: emitted {}", dir.display()),
        failed: 0,
        written,
    }
}

/// `recon: cannot read PATH: reason`, written as the failure is met.
fn warn(warnings: &mut impl Write, path: &Path, err: &io::Error) {
    let _ = writeln!(
        warnings,
        "recon: cannot read {}: {}",
        path.display(),
        reason(err)
    );
}

/// The reason in the words a person reads, where the kind is plain; the OS
/// message, `(os error N)` and all, where it is not.
fn reason(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::NotFound => "no such file".to_string(),
        io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        io::ErrorKind::IsADirectory => "is a directory".to_string(),
        _ if document::is_binary(err) => document::BINARY_FILE.to_string(),
        _ => err.to_string(),
    }
}

/// `1 file`, `3 files`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn strip_line_end(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Mode;
    use crate::emit::{Emit, Exit};
    use crate::filter::ActiveFilters;
    use crate::fixtures::{fixture_dir, fixture_file, fixture_path};
    use std::fs;
    use std::io::Cursor;

    // ---- inputs ------------------------------------------------------------

    #[test]
    fn stdin_lines_are_paths_absolutised_in_order_with_blanks_skipped() {
        let got = inputs(
            Cursor::new(&b"b.log\n\n  \n/abs/a.log\r\nc.log"[..]),
            Path::new("."),
        )
        .expect("reads");

        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(got.from, Source::Stdin);
        assert_eq!(
            got.files,
            vec![
                cwd.join("b.log"),
                PathBuf::from("/abs/a.log"),
                cwd.join("c.log"),
            ]
        );
    }

    #[test]
    fn a_path_directory_lists_its_files_in_explorer_order_without_directories() {
        let dir = fixture_dir("batch_inputs_dir");
        fs::write(dir.join("b.log"), "x").expect("write");
        fs::write(dir.join("A.log"), "x").expect("write");
        fs::create_dir(dir.join("sub")).expect("mkdir");

        let got = inputs(Cursor::new(&b""[..]), &dir).expect("reads");

        let dir = lexical_absolute(&dir);
        assert_eq!(got.from, Source::Directory(dir.clone()));
        assert_eq!(got.files, vec![dir.join("A.log"), dir.join("b.log")]);
    }

    /// A FIFO under the directory would block the first read for ever — a
    /// cron job that never finishes (#221). The explorer's `Kind::Special`
    /// is what `inputs` skips, so the two agree on what a file is.
    #[cfg(unix)]
    #[test]
    fn a_path_directory_skips_sockets_and_fifos() {
        let dir = fixture_dir("batch_inputs_special");
        fs::write(dir.join("a.log"), "x").expect("write");
        let _listener = std::os::unix::net::UnixListener::bind(dir.join("sock")).expect("bind");
        // `mkfifo` rather than libc, which is not a dependency; a system
        // without it (none known) skips the FIFO half rather than failing.
        let fifo = std::process::Command::new("mkfifo")
            .arg(dir.join("fifo"))
            .status()
            .is_ok_and(|status| status.success());

        let got = inputs(Cursor::new(&b""[..]), &dir).expect("reads");

        let dir = lexical_absolute(&dir);
        assert_eq!(got.files, vec![dir.join("a.log")], "fifo made: {fifo}");
    }

    /// `PATH` naming a FIFO directly: refused by the stat, not read. Without
    /// the guard this test would hang rather than fail.
    #[cfg(unix)]
    #[test]
    fn a_fifo_named_directly_is_a_read_failure_not_a_hang() {
        let dir = fixture_dir("batch_fifo_direct");
        let fifo = dir.join("fifo");
        if !std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|status| status.success())
        {
            eprintln!("skipping: no mkfifo");
            return;
        }

        let err = open_input(&fifo).expect_err("a FIFO is refused");
        assert_eq!(reason(&err), "not a regular file");

        let mut warnings = Vec::new();
        let mut out = Vec::new();
        let exit = collect(
            Emit::Lines,
            &Inputs {
                files: vec![fifo.clone()],
                from: Source::File,
            },
            &ActiveFilters::new(),
            Mode::Dimmed,
            false,
            &mut out,
            &mut warnings,
        );
        let (lines, _, failed) = emitted(exit, &out);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(failed, 1);
        assert_eq!(
            String::from_utf8_lossy(&warnings),
            format!(
                "recon: cannot read {}: not a regular file\n",
                fifo.display()
            )
        );
    }

    #[test]
    fn a_path_file_is_the_one_input_even_when_it_does_not_exist() {
        let file = fixture_file("batch_inputs_file.log", b"x\n");

        let got = inputs(Cursor::new(&b"\n"[..]), &file).expect("reads");

        assert_eq!(got.from, Source::File);
        assert_eq!(got.files, vec![lexical_absolute(&file)]);

        let missing = Path::new("target/batch_inputs_no_such_file.log");
        let got = inputs(Cursor::new(&b""[..]), missing).expect("reads");
        assert_eq!(got.from, Source::File);
        assert_eq!(got.files, vec![lexical_absolute(missing)]);
    }

    #[test]
    fn stdin_wins_over_the_path_argument() {
        let dir = fixture_dir("batch_inputs_stdin_wins");
        fs::write(dir.join("ignored.log"), "x").expect("write");

        let got = inputs(Cursor::new(&b"/only/this.log\n"[..]), &dir).expect("reads");

        assert_eq!(got.from, Source::Stdin);
        assert_eq!(got.files, vec![PathBuf::from("/only/this.log")]);
    }

    /// The README's promise: a filename is carried as the bytes the
    /// filesystem holds, so a name that is not UTF-8 reaches the output
    /// unchanged. No fixture — the path need not exist to be listed.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_stdin_line_keeps_its_bytes() {
        use std::os::unix::ffi::OsStrExt;

        let got = inputs(Cursor::new(&b"/tmp/we\xffird.log\n"[..]), Path::new(".")).expect("reads");

        assert_eq!(got.files[0].as_os_str().as_bytes(), b"/tmp/we\xffird.log");
    }

    // ---- shared helpers ----------------------------------------------------

    fn filters_matching(pattern: &str) -> ActiveFilters {
        let mut filters = ActiveFilters::new();
        filters.add(pattern).expect("valid pattern");
        filters
    }

    /// The lines written to `out`, and the summary and failure count of
    /// the `Exit::Streamed` that ended the run — which wrote every line.
    fn emitted(exit: Exit, out: &[u8]) -> (Vec<String>, String, usize) {
        match exit {
            Exit::Streamed {
                summary,
                failed,
                written,
            } => {
                written.expect("every line written");
                let out = String::from_utf8(out.to_vec()).expect("utf-8 fixture");
                assert!(out.is_empty() || out.ends_with('\n'), "{out:?}");
                let lines = out.lines().map(str::to_string).collect();
                (lines, summary, failed)
            }
            other => panic!("batch returns only Streamed, not {other:?}"),
        }
    }

    fn one_file(name: &str, body: &[u8]) -> Inputs {
        let file = fixture_file(name, body);
        Inputs {
            files: vec![lexical_absolute(&file)],
            from: Source::File,
        }
    }

    /// A directory of `files`, listed as stdin would give them: in the
    /// order of `files`, not the explorer's.
    fn from_stdin(name: &str, files: &[(&str, &str)]) -> Inputs {
        let dir = fixture_dir(name);
        let files = files
            .iter()
            .map(|(file, body)| {
                let path = dir.join(file);
                fs::write(&path, body).expect("write fixture");
                lexical_absolute(&path)
            })
            .collect();
        Inputs {
            files,
            from: Source::Stdin,
        }
    }

    fn warnings_of(buf: &[u8]) -> String {
        String::from_utf8(buf.to_vec()).expect("utf-8 warnings")
    }

    // ---- lines -------------------------------------------------------------

    #[test]
    fn lines_over_one_file_in_dim_mode_is_the_whole_file_with_the_match_count() {
        let inputs = one_file("batch_lines_dim.log", b"hit\nmiss\nhit again\n");
        let mut warnings = Vec::new();

        let mut out = Vec::new();
        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            false,
            &mut out,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit, &out);
        assert_eq!(lines, ["hit", "miss", "hit again"]);
        assert_eq!(
            summary,
            "recon: emitted 3 lines of batch_lines_dim.log, dim mode (2 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 0);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn lines_over_one_file_in_hide_mode_is_the_matches() {
        let inputs = one_file("batch_lines_hide.log", b"hit\nmiss\nhit again\n");

        let mut out = Vec::new();
        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            false,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit, &out);
        assert_eq!(lines, ["hit", "hit again"]);
        assert_eq!(
            summary,
            "recon: emitted 2 lines of batch_lines_hide.log, hide mode"
        );
    }

    #[test]
    fn line_numbers_are_the_file_s_own_with_a_tab() {
        let inputs = one_file("batch_lines_numbered.log", b"hit\nmiss\nhit again\n");

        let mut out = Vec::new();
        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            true,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, _, _) = emitted(exit, &out);
        assert_eq!(lines, ["1\thit", "3\thit again"]);
    }

    #[test]
    fn several_files_prefix_each_line_with_its_path_in_input_order() {
        let inputs = from_stdin(
            "batch_lines_several",
            &[("b.log", "miss\nhit\n"), ("a.log", "hit\n")],
        );
        let [b, a] = inputs.files.as_slice() else {
            panic!("two inputs")
        };

        let mut out = Vec::new();
        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            true,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit, &out);
        assert_eq!(
            lines,
            [
                format!("{}\t2\thit", b.display()),
                format!("{}\t1\thit", a.display()),
            ],
            "b before a: the input order, not the explorer's"
        );
        assert_eq!(summary, "recon: emitted 2 lines of 2 files, hide mode");
    }

    #[test]
    fn several_files_without_n_still_prefix_the_path() {
        let inputs = from_stdin(
            "batch_lines_several_no_n",
            &[("a.log", "hit\n"), ("b.log", "hit\n")],
        );

        let mut out = Vec::new();
        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            false,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit, &out);
        assert_eq!(
            lines,
            [
                format!("{}\thit", inputs.files[0].display()),
                format!("{}\thit", inputs.files[1].display()),
            ]
        );
        assert_eq!(
            summary,
            "recon: emitted 2 lines of 2 files, dim mode (2 match) — pass --hide to emit matches only"
        );
    }

    #[test]
    fn an_unreadable_input_is_warned_about_skipped_and_counted() {
        let mut inputs = from_stdin("batch_lines_unreadable", &[("a.log", "hit\n")]);
        let missing = lexical_absolute(&fixture_path("batch_lines_unreadable").join("missing.log"));
        inputs.files.insert(0, missing.clone());
        let mut warnings = Vec::new();

        let mut out = Vec::new();
        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            false,
            &mut out,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit, &out);
        assert_eq!(
            warnings_of(&warnings),
            format!("recon: cannot read {}: no such file\n", missing.display())
        );
        assert_eq!(lines, [format!("{}\thit", inputs.files[1].display())]);
        assert_eq!(summary, "recon: emitted 1 lines of 1 file, hide mode");
        assert_eq!(failed, 1);
    }

    #[test]
    fn a_binary_input_and_a_directory_input_are_read_failures() {
        let dir = fixture_dir("batch_lines_binary_and_dir");
        let binary = dir.join("core.bin");
        fs::write(&binary, b"ab\0cd\n").expect("write");
        let inputs = Inputs {
            files: vec![lexical_absolute(&binary), lexical_absolute(&dir)],
            from: Source::Stdin,
        };
        let mut warnings = Vec::new();

        let mut out = Vec::new();
        let exit = collect_lines(
            &inputs,
            &filters_matching("x"),
            Mode::Dimmed,
            false,
            &mut out,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit, &out);
        assert_eq!(
            warnings_of(&warnings),
            format!(
                "recon: cannot read {}: binary file\nrecon: cannot read {}: is a directory\n",
                lexical_absolute(&binary).display(),
                lexical_absolute(&dir).display(),
            )
        );
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(
            summary,
            "recon: emitted 0 lines of 0 files, dim mode (0 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 2);
    }

    // ---- cwd ---------------------------------------------------------------

    #[test]
    fn cwd_is_the_path_directory_or_the_first_input_s_parent() {
        let dir = fixture_dir("batch_cwd");
        let dir = lexical_absolute(&dir);

        let from_dir = Inputs {
            files: Vec::new(),
            from: Source::Directory(dir.clone()),
        };
        let mut out = Vec::new();
        let (lines, summary, failed) = emitted(collect_cwd(&from_dir, &mut out), &out);
        assert_eq!(lines, [dir.display().to_string()]);
        assert_eq!(summary, format!("recon: emitted {}", dir.display()));
        assert_eq!(failed, 0);

        let from_stdin = Inputs {
            files: vec![dir.join("a.log"), dir.join("b.log")],
            from: Source::Stdin,
        };
        let mut out = Vec::new();
        let (lines, _, _) = emitted(collect_cwd(&from_stdin, &mut out), &out);
        assert_eq!(
            lines,
            [dir.display().to_string()],
            "the first input's directory"
        );

        let from_file = Inputs {
            files: vec![dir.join("a.log")],
            from: Source::File,
        };
        let mut out = Vec::new();
        let (lines, _, _) = emitted(collect_cwd(&from_file, &mut out), &out);
        assert_eq!(lines, [dir.display().to_string()]);
    }

    // ---- files -------------------------------------------------------------

    fn three_logs(name: &str) -> Inputs {
        from_stdin(
            name,
            &[
                ("a.log", "hit\n"),
                ("b.log", "miss\n"),
                ("c.log", "x\nhit\n"),
            ],
        )
    }

    fn displayed(inputs: &Inputs) -> Vec<String> {
        inputs
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    #[test]
    fn files_in_hide_mode_lists_the_inputs_the_matcher_selects() {
        let inputs = three_logs("batch_files_hide");
        let mut warnings = Vec::new();

        let mut out = Vec::new();
        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            &mut out,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit, &out);
        let all = displayed(&inputs);
        assert_eq!(lines, [all[0].clone(), all[2].clone()]);
        assert_eq!(summary, "recon: emitted 2 files of 3 inputs, hide mode");
        assert_eq!(failed, 0);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn files_in_dim_mode_lists_every_input_with_the_match_count() {
        let inputs = three_logs("batch_files_dim");

        let mut out = Vec::new();
        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit, &out);
        assert_eq!(lines, displayed(&inputs));
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, dim mode (2 match) — pass --hide to emit matches only"
        );
    }

    #[test]
    fn files_from_a_path_directory_says_from() {
        let mut inputs = three_logs("batch_files_from_dir");
        let dir = lexical_absolute(&fixture_path("batch_files_from_dir"));
        inputs.from = Source::Directory(dir.clone());

        let mut out = Vec::new();
        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            &mut out,
            &mut Vec::new(),
        );

        let (_, summary, _) = emitted(exit, &out);
        assert_eq!(
            summary,
            format!("recon: emitted 2 files from {}, hide mode", dir.display())
        );
    }

    /// #306: an include filter asked for matching and the set is past the
    /// limit, so no file can match. That is said on the warnings channel,
    /// not left for a script to take as a real answer. "Nothing selects" is
    /// not a failure and says nothing.
    #[test]
    fn files_warns_when_file_matching_is_off() {
        let inputs = three_logs("batch_files_scan_off");
        let mut filters = ActiveFilters::new();
        for i in 0..=crate::filter::MAX_PATTERNS {
            filters.add(&format!("p{i}")).expect("valid pattern");
        }
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        collect_files(&inputs, &filters, Mode::Dimmed, &mut out, &mut warnings);
        let text = String::from_utf8(warnings).expect("utf-8");
        assert!(
            text.contains("recon: file matching off: ") && text.contains("no file is matched"),
            "no warning: {text}"
        );

        let mut exclude_only = ActiveFilters::new();
        exclude_only.add_excluding("x").expect("valid pattern");
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        collect_files(
            &inputs,
            &exclude_only,
            Mode::Dimmed,
            &mut out,
            &mut warnings,
        );
        assert!(warnings.is_empty(), "nothing selects is not a failure");
    }

    #[test]
    fn files_in_hide_mode_with_no_matcher_lists_everything() {
        let inputs = three_logs("batch_files_no_matcher");
        let mut exclude_only = ActiveFilters::new();
        exclude_only.add_excluding("x").expect("valid pattern");
        assert!(exclude_only.matcher().is_none(), "sanity: nothing selects");

        let mut out = Vec::new();
        let exit = collect_files(
            &inputs,
            &exclude_only,
            Mode::FilteredOnly,
            &mut out,
            &mut Vec::new(),
        );
        let (lines, summary, _) = emitted(exit, &out);
        assert_eq!(lines, displayed(&inputs), "nothing to hide against");
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, hide mode, no filter"
        );

        let mut out = Vec::new();
        let exit = collect_files(
            &inputs,
            &ActiveFilters::new(),
            Mode::Dimmed,
            &mut out,
            &mut Vec::new(),
        );
        let (lines, summary, _) = emitted(exit, &out);
        assert_eq!(lines, displayed(&inputs));
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, dim mode, no filter"
        );
    }

    #[test]
    fn files_warns_about_and_skips_an_unreadable_input_in_both_modes() {
        let mut inputs = three_logs("batch_files_unreadable");
        let dir = lexical_absolute(&fixture_path("batch_files_unreadable"));
        let missing = dir.join("missing.log");
        inputs.files.insert(1, missing.clone());
        inputs.files.push(dir.clone());
        let expected_warnings = format!(
            "recon: cannot read {}: no such file\nrecon: cannot read {}: is a directory\n",
            missing.display(),
            dir.display(),
        );

        let mut warnings = Vec::new();
        let mut out = Vec::new();
        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            &mut out,
            &mut warnings,
        );
        let (lines, summary, failed) = emitted(exit, &out);
        assert_eq!(warnings_of(&warnings), expected_warnings);
        assert_eq!(lines.len(), 3, "the three readable files: {lines:?}");
        assert_eq!(
            summary,
            "recon: emitted 3 files of 5 inputs, dim mode (2 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 2);

        let mut warnings = Vec::new();
        let mut out = Vec::new();
        let exit = collect_files(
            &inputs,
            &ActiveFilters::new(),
            Mode::FilteredOnly,
            &mut out,
            &mut warnings,
        );
        let (lines, summary, failed) = emitted(exit, &out);
        assert_eq!(
            warnings_of(&warnings),
            expected_warnings,
            "checked even with nothing to scan"
        );
        assert_eq!(
            lines.len(),
            3,
            "all readable files in hide mode with no filter"
        );
        assert_eq!(
            summary,
            "recon: emitted 3 files of 5 inputs, hide mode, no filter"
        );
        assert_eq!(failed, 2);
    }

    /// A UTF-16 log is decoded before it is matched, as the explorer's scan
    /// decodes it: matched as bytes, `E\0R\0R\0O\0R\0` never hits (#357).
    #[test]
    fn a_utf16_input_that_matches_is_listed_by_files() {
        let dir = fixture_dir("batch_files_utf16");
        let log = lexical_absolute(&dir.join("app.log"));
        let bytes: Vec<u8> = std::iter::once(0xfeff_u16)
            .chain("ok\r\nhit here\r\n".encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        fs::write(&log, bytes).expect("write");
        let inputs = Inputs {
            files: vec![log.clone()],
            from: Source::Stdin,
        };

        let mut warnings = Vec::new();
        let mut out = Vec::new();
        let (lines, _, failed) = emitted(
            collect_files(
                &inputs,
                &filters_matching("hit"),
                Mode::FilteredOnly,
                &mut out,
                &mut warnings,
            ),
            &out,
        );

        assert_eq!(lines, [log.display().to_string()]);
        assert_eq!(failed, 0);
    }

    /// A NUL-bearing file is a read failure for `lines` (`Document::read`
    /// sniffs it) but an ordinary input for `files` (`scan` reads bytes, as
    /// the explorer's scan does). Pinned so the asymmetry is a decision on
    /// record, not an accident.
    #[test]
    fn a_binary_input_is_listed_by_files_but_refused_by_lines() {
        let dir = fixture_dir("batch_files_binary");
        let binary = lexical_absolute(&dir.join("core.bin"));
        fs::write(&binary, b"hit\0\n").expect("write");
        let inputs = Inputs {
            files: vec![binary.clone()],
            from: Source::Stdin,
        };
        let filters = filters_matching("hit");

        let mut warnings = Vec::new();
        let mut out = Vec::new();
        let (lines, _, failed) = emitted(
            collect_files(
                &inputs,
                &filters,
                Mode::FilteredOnly,
                &mut out,
                &mut warnings,
            ),
            &out,
        );
        assert_eq!(lines, [binary.display().to_string()], "files lists it");
        assert_eq!(failed, 0);
        assert!(warnings.is_empty(), "{warnings:?}");

        let mut warnings = Vec::new();
        let mut out = Vec::new();
        let (lines, _, failed) = emitted(
            collect_lines(
                &inputs,
                &filters,
                Mode::FilteredOnly,
                false,
                &mut out,
                &mut warnings,
            ),
            &out,
        );
        assert!(lines.is_empty(), "lines refuses it");
        assert_eq!(failed, 1);
        assert_eq!(
            warnings_of(&warnings),
            format!("recon: cannot read {}: binary file\n", binary.display())
        );
    }

    // ---- files, answered by a parse (#224) ---------------------------------

    /// The built-in `definitions` set enabled with only `functions` on —
    /// what `--set definitions:functions` gives a run.
    fn functions_only(mut filters: ActiveFilters) -> ActiveFilters {
        let builtin = filters
            .sets()
            .iter()
            .position(|meta| meta.origin == crate::filter::Origin::BuiltIn)
            .expect("the built-in set is always present");
        let functions = filters
            .filters_in(builtin)
            .find(|(_, filter)| filter.display_name() == "functions")
            .map(|(index, _)| index)
            .expect("a functions filter");
        filters.set_enabled(functions, true);
        filters.set_enabled_set(builtin, true);
        assert!(filters.needs_kinds());
        filters
    }

    /// A Rust file that defines a function, one that does not, and a log.
    fn sources(name: &str) -> Inputs {
        from_stdin(
            name,
            &[
                ("a.rs", "fn main() {}\n"),
                ("b.rs", "// only a comment\n"),
                ("c.log", "fn main() {}\nhit\n"),
            ],
        )
    }

    #[test]
    fn files_in_hide_mode_lists_only_the_inputs_that_define_a_function() {
        let inputs = sources("batch_files_defs_hide");
        let mut out = Vec::new();

        let exit = collect_files(
            &inputs,
            &functions_only(ActiveFilters::new()),
            Mode::FilteredOnly,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, summary, failed) = emitted(exit, &out);
        assert_eq!(
            lines,
            [displayed(&inputs)[0].clone()],
            "a log has no grammar"
        );
        assert_eq!(summary, "recon: emitted 1 files of 3 inputs, hide mode");
        assert_eq!(failed, 0);
    }

    #[test]
    fn files_in_dim_mode_counts_the_inputs_that_define_a_function() {
        let inputs = sources("batch_files_defs_dim");
        let mut out = Vec::new();

        let exit = collect_files(
            &inputs,
            &functions_only(ActiveFilters::new()),
            Mode::Dimmed,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit, &out);
        assert_eq!(lines, displayed(&inputs));
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, dim mode (1 match) — pass --hide to emit matches only"
        );
    }

    /// A regex filter beside a definition filter is answered by the same
    /// parse, so a file the regex alone selects is still listed.
    #[test]
    fn files_answers_a_regex_and_a_definition_filter_together() {
        let inputs = sources("batch_files_defs_regex");
        let mut out = Vec::new();

        let exit = collect_files(
            &inputs,
            &functions_only(filters_matching("hit")),
            Mode::FilteredOnly,
            &mut out,
            &mut Vec::new(),
        );

        let (lines, _, _) = emitted(exit, &out);
        let all = displayed(&inputs);
        assert_eq!(lines, [all[0].clone(), all[2].clone()]);
    }

    /// `Document::read` refuses a NUL-bearing file. On the parse path that
    /// is "no definitions", not a read failure: listed in dim mode, dropped
    /// in hide mode, no warning, no exit 2.
    #[test]
    fn files_takes_a_binary_input_as_defining_nothing() {
        let dir = fixture_dir("batch_files_defs_binary");
        let binary = lexical_absolute(&dir.join("core.rs"));
        fs::write(&binary, b"fn main() {}\0\n").expect("write");
        let inputs = Inputs {
            files: vec![binary.clone()],
            from: Source::Stdin,
        };

        for (mode, listed) in [(Mode::Dimmed, 1), (Mode::FilteredOnly, 0)] {
            let mut out = Vec::new();
            let mut warnings = Vec::new();
            let exit = collect_files(
                &inputs,
                &functions_only(ActiveFilters::new()),
                mode,
                &mut out,
                &mut warnings,
            );

            let (lines, _, failed) = emitted(exit, &out);
            assert_eq!(lines.len(), listed, "{mode:?}");
            assert_eq!(failed, 0, "{mode:?}");
            assert!(warnings.is_empty(), "{}", warnings_of(&warnings));
        }
    }

    // ---- streaming ---------------------------------------------------------

    /// A consumer that has closed its end: every write fails as
    /// `recon … | head` sees once `head` exits.
    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The parts of an `Exit::Streamed` whose writing failed.
    fn stopped(exit: Exit) -> (String, usize, io::ErrorKind) {
        match exit {
            Exit::Streamed {
                summary,
                failed,
                written: Err(err),
            } => (summary, failed, err.kind()),
            other => panic!("a stopped run, not {other:?}"),
        }
    }

    /// #219: the first failed write ends the run. The second input does not
    /// exist, so a run that went on to read it would warn about it.
    #[test]
    fn a_closed_pipe_stops_lines_before_the_next_input_is_read() {
        let mut inputs = from_stdin("batch_stream_lines", &[("a.log", "hit\nhit\n")]);
        inputs
            .files
            .push(inputs.files[0].with_file_name("missing.log"));
        let mut warnings = Vec::new();

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            false,
            &mut ClosedPipe,
            &mut warnings,
        );

        let (summary, failed, kind) = stopped(exit);
        assert_eq!(kind, io::ErrorKind::BrokenPipe);
        assert!(warnings.is_empty(), "{}", warnings_of(&warnings));
        assert_eq!(failed, 0);
        assert_eq!(summary, "recon: emitted 0 lines of 1 file, hide mode");
    }

    #[test]
    fn a_closed_pipe_stops_files_before_the_next_input_is_read() {
        let mut inputs = three_logs("batch_stream_files");
        inputs
            .files
            .insert(1, inputs.files[0].with_file_name("missing.log"));
        let mut warnings = Vec::new();

        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            &mut ClosedPipe,
            &mut warnings,
        );

        let (summary, failed, kind) = stopped(exit);
        assert_eq!(kind, io::ErrorKind::BrokenPipe);
        assert!(warnings.is_empty(), "{}", warnings_of(&warnings));
        assert_eq!(failed, 0);
        assert!(
            summary.starts_with("recon: emitted 0 files of 4 inputs"),
            "{summary}"
        );
    }

    // ---- collect -----------------------------------------------------------

    #[test]
    fn collect_dispatches_on_the_emit_kind() {
        let inputs = one_file("batch_collect.log", b"hit\n");
        let filters = filters_matching("hit");
        let mut warnings = Vec::new();

        let mut out = Vec::new();
        let (lines, _, _) = emitted(
            collect(
                Emit::Lines,
                &inputs,
                &filters,
                Mode::FilteredOnly,
                false,
                &mut out,
                &mut warnings,
            ),
            &out,
        );
        assert_eq!(lines, ["hit"]);

        let mut out = Vec::new();
        let (lines, _, _) = emitted(
            collect(
                Emit::Files,
                &inputs,
                &filters,
                Mode::FilteredOnly,
                false,
                &mut out,
                &mut warnings,
            ),
            &out,
        );
        assert_eq!(lines, [inputs.files[0].display().to_string()]);

        let mut out = Vec::new();
        let (lines, _, _) = emitted(
            collect(
                Emit::Cwd,
                &inputs,
                &filters,
                Mode::FilteredOnly,
                false,
                &mut out,
                &mut warnings,
            ),
            &out,
        );
        assert_eq!(
            lines,
            [inputs.files[0]
                .parent()
                .expect("has a parent")
                .display()
                .to_string()]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// `run` reads real stdin, so its wiring is exercised by
    /// `tests/batch.rs`; the filter construction it delegates to is
    /// checked here.
    #[test]
    fn filters_for_enables_each_set_and_refuses_an_unknown_one() {
        let mut set = crate::filter::test_support::loaded("Bugs", 50, false, &["hit"]);
        set.profiles
            .insert("p".to_string(), vec!["hit".to_string()]);
        let config = Startup {
            filter_sets: vec![set],
            ..Startup::from(crate::config::Config {
                set: vec!["Bugs:p".to_string()],
                ..crate::config::Config::default()
            })
        };

        let filters = filters_for(&config).expect("known set");
        assert!(filters.sets()[1].enabled);
        assert!(filters.matcher().is_some(), "the profile enabled `hit`");

        let config = Startup::from(crate::config::Config {
            set: vec!["Nope".to_string()],
            ..crate::config::Config::default()
        });
        let err = filters_for(&config).expect_err("unknown set");
        assert!(err.to_string().contains("unknown set \"Nope\""), "{err}");
    }

    /// `--emit` follows the same rules as the pane (#282): an unlisted set
    /// decides nothing, even with `autoload`, and `--set` lists it.
    #[test]
    fn filters_for_skips_an_unlisted_set_unless_it_is_named() {
        let mut set = crate::filter::test_support::loaded("Bugs", 50, true, &["hit"]);
        set.listed = false;
        set.profiles
            .insert("default".to_string(), vec!["hit".to_string()]);
        let config = Startup {
            filter_sets: vec![set.clone()],
            ..Startup::from(crate::config::Config {
                ..crate::config::Config::default()
            })
        };
        let filters = filters_for(&config).expect("loads");
        assert!(!filters.sets()[1].enabled);
        assert!(
            filters.matcher().is_none(),
            "the unlisted set selects nothing"
        );

        let config = Startup {
            filter_sets: vec![set],
            ..Startup::from(crate::config::Config {
                set: vec!["Bugs".to_string()],
                ..crate::config::Config::default()
            })
        };
        let filters = filters_for(&config).expect("known set");
        assert!(filters.sets()[1].listed);
        assert!(filters.sets()[1].enabled);
        assert!(filters.matcher().is_some());
    }

    /// `--unlist` wins over `listed = true` and `autoload = true` (#283):
    /// the set is out of the filters that decide the output.
    #[test]
    fn filters_for_drops_an_unlisted_autoload_set() {
        let mut set = crate::filter::test_support::loaded("Bugs", 50, true, &["hit"]);
        set.profiles
            .insert("default".to_string(), vec!["hit".to_string()]);
        let config = Startup {
            filter_sets: vec![set.clone()],
            ..Startup::from(crate::config::Config {
                ..crate::config::Config::default()
            })
        };
        assert!(filters_for(&config).expect("loads").matcher().is_some());

        let config = Startup {
            filter_sets: vec![set],
            ..Startup::from(crate::config::Config {
                unlist: vec!["Bugs".to_string()],
                ..crate::config::Config::default()
            })
        };
        let filters = filters_for(&config).expect("known set");
        assert!(!filters.sets()[1].listed);
        assert!(!filters.sets()[1].enabled);
        assert!(filters.matcher().is_none(), "the set selects nothing");
    }
}
