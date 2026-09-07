//! Headless mode (#143): `--emit` with stdin that is not a terminal.
//!
//! The pieces `App` composes — `ActiveFilters`, `Document`, `scan::scan` —
//! with no navigator, no view and no terminal. Files come from stdin or
//! from the `PATH` argument; the result leaves through the same `Exit` a
//! TUI session hands back, so `main` prints both the same way.

use crate::config::Config;
use crate::document::{self, Document, Mode};
use crate::emit::{Emit, Exit, path_bytes};
use crate::filter::{ActiveFilters, Matcher};
use crate::path::lexical_absolute;
use crate::scan::{self, Progress};
use crate::viewport::is_interesting;
use crate::widgets::filenav::{Kind, sorted_entries};
use color_eyre::{Result, eyre::eyre};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// The files a headless run reads, and where the list came from.
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
    /// `PATH` named a directory: its files, in the navigator's order.
    Directory(PathBuf),
    /// `PATH` named a file — or nothing that exists: that path alone.
    File,
}

/// Run headless: read the input list, build the filters `--set` asks for,
/// and collect what `--emit` names. Read-failure warnings go to stderr as
/// they are met; the `Exit` carries the output, the summary and the
/// failure count for `main` to deliver.
pub fn run(config: &Config) -> Result<Exit> {
    let Some(what) = config.emit else {
        return Err(eyre!("headless mode needs --emit"));
    };
    let inputs = inputs(io::stdin().lock(), Path::new(&config.path))?;
    let filters = filters_for(config)?;
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
        &mut io::stderr(),
    ))
}

/// The startup filter set: the loaded sets, then each `--set` enabled — the
/// same two steps `App::new` takes.
fn filters_for(config: &Config) -> Result<ActiveFilters> {
    let mut filters = ActiveFilters::with_sets(config.filter_palette.clone(), &config.filter_sets);
    for (set, profile) in config.sets_to_enable() {
        filters
            .enable_named(&set, profile.as_deref())
            .map_err(|err| eyre!("--set {set}: {err}"))?;
    }
    Ok(filters)
}

/// What `--emit` names, over `inputs`. `warnings` gets one line per input
/// that could not be read.
pub(crate) fn collect(
    what: Emit,
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    line_numbers: bool,
    warnings: &mut impl Write,
) -> Exit {
    match what {
        Emit::Lines => collect_lines(inputs, filters, mode, line_numbers, warnings),
        Emit::Files => collect_files(inputs, filters, mode, warnings),
        Emit::Cwd => collect_cwd(inputs),
    }
}

/// Read the input list: every non-blank line of `stdin` as a path, or, when
/// stdin held none, what `path` names — a directory's files in the
/// navigator's order, or the file itself.
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
            .filter(|entry| !matches!(entry.kind, Kind::Dir | Kind::Parent))
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
fn collect_lines(
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    line_numbers: bool,
    warnings: &mut impl Write,
) -> Exit {
    let several = inputs.files.len() > 1;
    let mut lines = Vec::new();
    let mut read = 0;
    let mut failed = 0;
    let mut interesting = 0;
    for path in &inputs.files {
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
        interesting += document
            .verdicts()
            .iter()
            .filter(|v| is_interesting(v))
            .count();
        let text = document.lines();
        for &source in document.visible() {
            let mut line = Vec::new();
            if several {
                line.extend_from_slice(&path_bytes(path));
                line.push(b'\t');
            }
            if line_numbers {
                line.extend_from_slice(format!("{}\t", source + 1).as_bytes());
            }
            line.extend_from_slice(text[source].as_bytes());
            lines.push(line);
        }
    }
    let subject = match inputs.files.as_slice() {
        [only] => only.file_name().map_or_else(
            || only.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        _ => count(read, "file"),
    };
    let emitted = lines.len();
    let summary = match mode {
        Mode::Dimmed => format!(
            "recon: emitted {emitted} lines of {subject}, dim mode ({interesting} match) — pass --hide to emit matches only"
        ),
        Mode::FilteredOnly => format!("recon: emitted {emitted} lines of {subject}, hide mode"),
    };
    Exit::Emit {
        lines,
        summary,
        failed,
    }
}

/// `--emit files`: every readable input in dim mode; in hide mode, the
/// inputs the matcher selects — or every readable input when nothing
/// selects, which is the navigator's rule: it hides nothing it cannot mark.
///
/// One scan per file, stopping at the first selecting line, which is what
/// the navigator's scan costs. With nothing to scan, each input is still
/// opened, so an unreadable one is warned about and skipped in every mode.
/// The summary's `from <dir>` / `of N inputs` follows where the list came
/// from; there is no `unscanned` here, since every scan runs to its answer
/// before anything prints.
fn collect_files(
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    warnings: &mut impl Write,
) -> Exit {
    let matcher = filters.matcher();
    let mut lines = Vec::new();
    let mut matched = 0;
    let mut failed = 0;
    for path in &inputs.files {
        let answer = match &matcher {
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
            Mode::FilteredOnly => yes || matcher.is_none(),
        };
        if listed {
            lines.push(path_bytes(path));
        }
    }
    let emitted = lines.len();
    let origin = match &inputs.from {
        Source::Directory(dir) => format!("from {}", dir.display()),
        Source::Stdin | Source::File => format!("of {}", count(inputs.files.len(), "input")),
    };
    let summary = match (matcher.is_some(), mode) {
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
    Exit::Emit {
        lines,
        summary,
        failed,
    }
}

/// Open an input for scanning. A directory is refused up front: `File::open`
/// accepts one on Unix and only the read fails, and `scan` swallows a read
/// error as end of file.
fn open_input(path: &Path) -> io::Result<File> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "is a directory",
        ));
    }
    File::open(path)
}

/// Whether any line of `path` selects under `matcher` — `Record::answer`'s
/// rule, over a scan run to its answer.
fn file_matches(path: &Path, matcher: &Matcher) -> io::Result<bool> {
    let reader = BufReader::new(open_input(path)?);
    let progress = scan::scan(
        reader,
        matcher,
        Progress::default(),
        &AtomicBool::new(false),
    );
    Ok(progress.seen.iter().any(|&bits| matcher.selects(bits)))
}

/// `--emit cwd`: the directory `PATH` named, or the first input's. Nothing
/// is read.
fn collect_cwd(inputs: &Inputs) -> Exit {
    let dir = match &inputs.from {
        Source::Directory(dir) => dir.clone(),
        Source::Stdin | Source::File => inputs
            .files
            .first()
            .and_then(|file| file.parent())
            .map_or_else(|| PathBuf::from("/"), Path::to_path_buf),
    };
    Exit::Emit {
        lines: vec![path_bytes(&dir)],
        summary: format!("recon: emitted {}", dir.display()),
        failed: 0,
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
    fn a_path_directory_lists_its_files_in_navigator_order_without_directories() {
        let dir = fixture_dir("headless_inputs_dir");
        fs::write(dir.join("b.log"), "x").expect("write");
        fs::write(dir.join("A.log"), "x").expect("write");
        fs::create_dir(dir.join("sub")).expect("mkdir");

        let got = inputs(Cursor::new(&b""[..]), &dir).expect("reads");

        let dir = lexical_absolute(&dir);
        assert_eq!(got.from, Source::Directory(dir.clone()));
        assert_eq!(got.files, vec![dir.join("A.log"), dir.join("b.log")]);
    }

    #[test]
    fn a_path_file_is_the_one_input_even_when_it_does_not_exist() {
        let file = fixture_file("headless_inputs_file.log", b"x\n");

        let got = inputs(Cursor::new(&b"\n"[..]), &file).expect("reads");

        assert_eq!(got.from, Source::File);
        assert_eq!(got.files, vec![lexical_absolute(&file)]);

        let missing = Path::new("target/headless_inputs_no_such_file.log");
        let got = inputs(Cursor::new(&b""[..]), missing).expect("reads");
        assert_eq!(got.from, Source::File);
        assert_eq!(got.files, vec![lexical_absolute(missing)]);
    }

    #[test]
    fn stdin_wins_over_the_path_argument() {
        let dir = fixture_dir("headless_inputs_stdin_wins");
        fs::write(dir.join("ignored.log"), "x").expect("write");

        let got = inputs(Cursor::new(&b"/only/this.log\n"[..]), &dir).expect("reads");

        assert_eq!(got.from, Source::Stdin);
        assert_eq!(got.files, vec![PathBuf::from("/only/this.log")]);
    }

    // ---- shared helpers ----------------------------------------------------

    fn filters_matching(pattern: &str) -> ActiveFilters {
        let mut filters = ActiveFilters::new();
        filters.add(pattern).expect("valid pattern");
        filters
    }

    fn strings(lines: &[Vec<u8>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| String::from_utf8(line.clone()).expect("utf-8 fixture"))
            .collect()
    }

    /// The parts of an `Exit::Emit`, as strings.
    fn emitted(exit: Exit) -> (Vec<String>, String, usize) {
        match exit {
            Exit::Emit {
                lines,
                summary,
                failed,
            } => (strings(&lines), summary, failed),
            Exit::Silent => panic!("headless never returns Silent"),
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
    /// order of `files`, not the navigator's.
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
        let inputs = one_file("headless_lines_dim.log", b"hit\nmiss\nhit again\n");
        let mut warnings = Vec::new();

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            false,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
        assert_eq!(lines, ["hit", "miss", "hit again"]);
        assert_eq!(
            summary,
            "recon: emitted 3 lines of headless_lines_dim.log, dim mode (2 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 0);
        assert!(warnings.is_empty());
    }

    #[test]
    fn lines_over_one_file_in_hide_mode_is_the_matches() {
        let inputs = one_file("headless_lines_hide.log", b"hit\nmiss\nhit again\n");

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            false,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, ["hit", "hit again"]);
        assert_eq!(
            summary,
            "recon: emitted 2 lines of headless_lines_hide.log, hide mode"
        );
    }

    #[test]
    fn line_numbers_are_the_file_s_own_with_a_tab() {
        let inputs = one_file("headless_lines_numbered.log", b"hit\nmiss\nhit again\n");

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            true,
            &mut Vec::new(),
        );

        let (lines, _, _) = emitted(exit);
        assert_eq!(lines, ["1\thit", "3\thit again"]);
    }

    #[test]
    fn several_files_prefix_each_line_with_its_path_in_input_order() {
        let inputs = from_stdin(
            "headless_lines_several",
            &[("b.log", "miss\nhit\n"), ("a.log", "hit\n")],
        );
        let [b, a] = inputs.files.as_slice() else {
            panic!("two inputs")
        };

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            true,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
        assert_eq!(
            lines,
            [
                format!("{}\t2\thit", b.display()),
                format!("{}\t1\thit", a.display()),
            ],
            "b before a: the input order, not the navigator's"
        );
        assert_eq!(summary, "recon: emitted 2 lines of 2 files, hide mode");
    }

    #[test]
    fn several_files_without_n_still_prefix_the_path() {
        let inputs = from_stdin(
            "headless_lines_several_no_n",
            &[("a.log", "hit\n"), ("b.log", "hit\n")],
        );

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            false,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
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
        let mut inputs = from_stdin("headless_lines_unreadable", &[("a.log", "hit\n")]);
        let missing =
            lexical_absolute(&fixture_path("headless_lines_unreadable").join("missing.log"));
        inputs.files.insert(0, missing.clone());
        let mut warnings = Vec::new();

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            false,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
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
        let dir = fixture_dir("headless_lines_binary_and_dir");
        let binary = dir.join("core.bin");
        fs::write(&binary, b"ab\0cd\n").expect("write");
        let inputs = Inputs {
            files: vec![lexical_absolute(&binary), lexical_absolute(&dir)],
            from: Source::Stdin,
        };
        let mut warnings = Vec::new();

        let exit = collect_lines(
            &inputs,
            &filters_matching("x"),
            Mode::Dimmed,
            false,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
        assert_eq!(
            warnings_of(&warnings),
            format!(
                "recon: cannot read {}: binary file\nrecon: cannot read {}: is a directory\n",
                lexical_absolute(&binary).display(),
                lexical_absolute(&dir).display(),
            )
        );
        assert!(lines.is_empty());
        assert_eq!(
            summary,
            "recon: emitted 0 lines of 0 files, dim mode (0 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 2);
    }

    // ---- cwd ---------------------------------------------------------------

    #[test]
    fn cwd_is_the_path_directory_or_the_first_input_s_parent() {
        let dir = fixture_dir("headless_cwd");
        let dir = lexical_absolute(&dir);

        let from_dir = Inputs {
            files: Vec::new(),
            from: Source::Directory(dir.clone()),
        };
        let (lines, summary, failed) = emitted(collect_cwd(&from_dir));
        assert_eq!(lines, [dir.display().to_string()]);
        assert_eq!(summary, format!("recon: emitted {}", dir.display()));
        assert_eq!(failed, 0);

        let from_stdin = Inputs {
            files: vec![dir.join("a.log"), dir.join("b.log")],
            from: Source::Stdin,
        };
        let (lines, _, _) = emitted(collect_cwd(&from_stdin));
        assert_eq!(
            lines,
            [dir.display().to_string()],
            "the first input's directory"
        );

        let from_file = Inputs {
            files: vec![dir.join("a.log")],
            from: Source::File,
        };
        let (lines, _, _) = emitted(collect_cwd(&from_file));
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
        let inputs = three_logs("headless_files_hide");
        let mut warnings = Vec::new();

        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
        let all = displayed(&inputs);
        assert_eq!(lines, [all[0].clone(), all[2].clone()]);
        assert_eq!(summary, "recon: emitted 2 files of 3 inputs, hide mode");
        assert_eq!(failed, 0);
        assert!(warnings.is_empty());
    }

    #[test]
    fn files_in_dim_mode_lists_every_input_with_the_match_count() {
        let inputs = three_logs("headless_files_dim");

        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, displayed(&inputs));
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, dim mode (2 match) — pass --hide to emit matches only"
        );
    }

    #[test]
    fn files_from_a_path_directory_says_from() {
        let mut inputs = three_logs("headless_files_from_dir");
        let dir = lexical_absolute(&fixture_path("headless_files_from_dir"));
        inputs.from = Source::Directory(dir.clone());

        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            &mut Vec::new(),
        );

        let (_, summary, _) = emitted(exit);
        assert_eq!(
            summary,
            format!("recon: emitted 2 files from {}, hide mode", dir.display())
        );
    }

    #[test]
    fn files_in_hide_mode_with_no_matcher_lists_everything() {
        let inputs = three_logs("headless_files_no_matcher");
        let mut exclude_only = ActiveFilters::new();
        exclude_only.add_excluding("x").expect("valid pattern");
        assert!(exclude_only.matcher().is_none(), "sanity: nothing selects");

        let exit = collect_files(&inputs, &exclude_only, Mode::FilteredOnly, &mut Vec::new());
        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, displayed(&inputs), "nothing to hide against");
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, hide mode, no filter"
        );

        let exit = collect_files(
            &inputs,
            &ActiveFilters::new(),
            Mode::Dimmed,
            &mut Vec::new(),
        );
        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, displayed(&inputs));
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, dim mode, no filter"
        );
    }

    #[test]
    fn files_warns_about_and_skips_an_unreadable_input_in_both_modes() {
        let mut inputs = three_logs("headless_files_unreadable");
        let dir = lexical_absolute(&fixture_path("headless_files_unreadable"));
        let missing = dir.join("missing.log");
        inputs.files.insert(1, missing.clone());
        inputs.files.push(dir.clone());
        let expected_warnings = format!(
            "recon: cannot read {}: no such file\nrecon: cannot read {}: is a directory\n",
            missing.display(),
            dir.display(),
        );

        let mut warnings = Vec::new();
        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            &mut warnings,
        );
        let (lines, summary, failed) = emitted(exit);
        assert_eq!(warnings_of(&warnings), expected_warnings);
        assert_eq!(lines.len(), 3, "the three readable files: {lines:?}");
        assert_eq!(
            summary,
            "recon: emitted 3 files of 5 inputs, dim mode (2 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 2);

        let mut warnings = Vec::new();
        let exit = collect_files(&inputs, &ActiveFilters::new(), Mode::Dimmed, &mut warnings);
        let (_, _, failed) = emitted(exit);
        assert_eq!(
            warnings_of(&warnings),
            expected_warnings,
            "checked even with nothing to scan"
        );
        assert_eq!(failed, 2);
    }

    // ---- collect -----------------------------------------------------------

    #[test]
    fn collect_dispatches_on_the_emit_kind() {
        let inputs = one_file("headless_collect.log", b"hit\n");
        let filters = filters_matching("hit");
        let mut warnings = Vec::new();

        let (lines, _, _) = emitted(collect(
            Emit::Lines,
            &inputs,
            &filters,
            Mode::FilteredOnly,
            false,
            &mut warnings,
        ));
        assert_eq!(lines, ["hit"]);

        let (lines, _, _) = emitted(collect(
            Emit::Files,
            &inputs,
            &filters,
            Mode::FilteredOnly,
            false,
            &mut warnings,
        ));
        assert_eq!(lines, [inputs.files[0].display().to_string()]);

        let (lines, _, _) = emitted(collect(
            Emit::Cwd,
            &inputs,
            &filters,
            Mode::FilteredOnly,
            false,
            &mut warnings,
        ));
        assert_eq!(
            lines,
            [inputs.files[0]
                .parent()
                .expect("has a parent")
                .display()
                .to_string()]
        );
        assert!(warnings.is_empty());
    }

    /// `run` reads real stdin, so its wiring is exercised by
    /// `tests/headless.rs`; the filter construction it delegates to is
    /// checked here.
    #[test]
    fn filters_for_enables_each_set_and_refuses_an_unknown_one() {
        let mut set = crate::filter::test_support::loaded("Bugs", 50, false, &["hit"]);
        set.profiles
            .insert("p".to_string(), vec!["hit".to_string()]);
        let config = crate::config::Config {
            filter_sets: vec![set],
            set: vec!["Bugs:p".to_string()],
            ..crate::config::Config::default()
        };

        let filters = filters_for(&config).expect("known set");
        assert!(filters.sets()[1].enabled);
        assert!(filters.matcher().is_some(), "the profile enabled `hit`");

        let config = crate::config::Config {
            set: vec!["Nope".to_string()],
            ..crate::config::Config::default()
        };
        let err = filters_for(&config).expect_err("unknown set");
        assert!(err.to_string().contains("unknown set \"Nope\""), "{err}");
    }
}
