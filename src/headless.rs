//! Headless mode (#143): `--emit` with stdin that is not a terminal.
//!
//! The pieces `App` composes — `ActiveFilters`, `Document`, `scan::scan` —
//! with no navigator, no view and no terminal. Files come from stdin or
//! from the `PATH` argument; the result leaves through the same `Exit` a
//! TUI session hands back, so `main` prints both the same way.

use crate::path::lexical_absolute;
use crate::widgets::filenav::{Kind, sorted_entries};
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

/// The files a headless run reads, and where the list came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// Absolute, in the order they were given.
    pub files: Vec<PathBuf>,
    pub from: Source,
}

/// Where the input list came from — the summary and `cwd` differ by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// One path per line on stdin.
    Stdin,
    /// `PATH` named a directory: its files, in the navigator's order.
    Directory(PathBuf),
    /// `PATH` named a file — or nothing that exists: that path alone.
    File,
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
pub fn inputs(mut stdin: impl BufRead, path: &Path) -> io::Result<Inputs> {
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
    use crate::fixtures::{fixture_dir, fixture_file};
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
}
