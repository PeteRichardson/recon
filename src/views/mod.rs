//! Views of a file, and the pipeline that makes them (#455, ADR 0007).
//!
//! A view is a job: (file, view) → contents. Every job runs off the main
//! thread, on `pool::Pool`, so a slow view never stops browsing. The main
//! thread sends a `Job` and adopts the `Done` that comes back.

pub(crate) mod cache;
pub mod command;
pub mod cycle;
pub(crate) mod pool;
pub(crate) mod read;

pub use cycle::{FileType, Glob};

use cache::{Cache, Key};
use cycle::{Steps, Table};
use read::{Contents, MAX_PREVIEW_BYTES, PREVIEW_LINES};
use std::path::PathBuf;
use std::sync::Arc;

/// One way the file view shows a file. `Table::view_name` gives its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// The file's own text.
    Raw,
    /// The file as a hex dump (#242).
    Hex,
    /// The output of the `i`th command of the table (#457).
    Command(usize),
}

/// The names of the built-in views, which a command cannot take. `image`
/// and `quicklook` come with the image view (stage 4); a row may name them
/// now, and the table skips them with a warning.
pub(crate) const BUILT_IN: [&str; 4] = ["raw", "hex", "image", "quicklook"];

impl View {
    /// The built-in view called `name`, if this version can show it.
    pub(crate) fn built_in(name: &str) -> Option<Self> {
        match name {
            "raw" => Some(Self::Raw),
            "hex" => Some(Self::Hex),
            _ => None,
        }
    }
}

/// How much of the file a job reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Extent {
    /// Up to `PREVIEW_LINES` lines and `MAX_PREVIEW_BYTES` bytes: what the
    /// explorer's selection shows as it moves.
    Preview,
    /// The whole file: what the view reads once it is used (#383).
    Full,
}

/// A file to show, with what decides its view: the table and the steps
/// `-` had taken when the job was sent.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub(crate) path: PathBuf,
    pub(crate) extent: Extent,
    pub(crate) table: Arc<Table>,
    pub(crate) steps: Steps,
    /// The view pane's width in pixels, for `{px}` (#457).
    pub(crate) px: u32,
    /// Whether the job is a prefetch: a command with `prefetch = false`
    /// does not run for one.
    pub(crate) prefetch: bool,
}

/// The file's row in the table and the view of its cycle that the job ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) row: usize,
    pub(crate) view: View,
}

/// A job's result.
#[derive(Debug, Clone)]
pub(crate) struct Done {
    pub(crate) path: PathBuf,
    pub(crate) contents: Contents,
    /// `None` for a directory, which has no row and no cycle.
    pub(crate) shown: Option<Shown>,
}

/// Run `job` in the caller: the work a worker does. With a cache, a file
/// already read in this view is not read again.
pub(crate) fn run(job: &Job, cache: Option<&Cache>) -> Done {
    let path = &job.path;
    let done = |contents, shown| Done {
        path: path.clone(),
        contents,
        shown,
    };
    if path.is_dir() {
        let max_lines = match job.extent {
            Extent::Preview => PREVIEW_LINES,
            Extent::Full => usize::MAX,
        };
        return done(read::directory_listing(path, max_lines), None);
    }
    let cached = cache.zip(Key::of(path));
    let row = job.table.row_for(path, || {
        if let Some((cache, key)) = &cached
            && let Some(binary) = cache.binary(key)
        {
            return binary;
        }
        let binary = read::is_binary(path);
        if let Some((cache, key)) = &cached {
            cache.set_binary(key, binary);
        }
        binary
    });
    let view = job.steps.view(&job.table, row);
    let shown = Some(Shown { row, view });
    if job.prefetch
        && job
            .table
            .command(view)
            .is_some_and(|command| !command.prefetch)
    {
        return done(Contents::default(), shown);
    }
    if let Some((cache, key)) = &cached
        && let Some(contents) = cache.get(key, view, job.extent)
    {
        return done(contents, shown);
    }
    let (contents, extent) = read_view(job, view);
    if let Some((cache, key)) = cached {
        cache.put(key, view, extent, &contents);
    }
    done(contents, shown)
}

/// The job's file in `view`, and how much of it the contents answer for.
///
/// A command's output is the whole view whatever the extent, so it answers
/// for both. A command that fails shows raw, which is hex for a binary file
/// (#457), read to the job's extent. The failure is kept as the contents
/// are, so that prefetch does not run a failing command again on each move.
fn read_view(job: &Job, view: View) -> (Contents, Extent) {
    let path = &job.path;
    if let Some(command) = job.table.command(view) {
        return match command::run(command, path, job.px) {
            Ok(output) => (
                read::command_output(&output, command.output.as_deref()),
                Extent::Full,
            ),
            Err(reason) => {
                let (contents, extent) = read_view(job, View::Raw);
                let contents = Contents {
                    fallback: Some(reason),
                    ..contents
                };
                (contents, extent)
            }
        };
    }
    let contents = match job.extent {
        Extent::Preview => {
            read::read_preview_with_caps(path, PREVIEW_LINES, MAX_PREVIEW_BYTES, view)
        }
        Extent::Full => read::read_lines(path, view),
    };
    (contents, job.extent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{fixture_dir, fixture_file};
    use std::path::Path;

    fn job(path: &Path, table: Table) -> Job {
        let steps = Steps::new(&table);
        Job {
            path: path.to_path_buf(),
            extent: Extent::Preview,
            table: Arc::new(table),
            steps,
            px: 0,
            prefetch: false,
        }
    }

    #[test]
    fn a_text_file_shows_raw_and_a_binary_one_hex() {
        let text = fixture_file("views_run_text.txt", b"hi\n");
        let binary = fixture_file("views_run_binary.bin", b"a\0b");

        let done = run(&job(&text, Table::default()), None);
        assert_eq!(done.shown.map(|s| s.view), Some(View::Raw));
        assert_eq!(*done.contents.lines, ["hi"]);

        let done = run(&job(&binary, Table::default()), None);
        assert_eq!(done.shown.map(|s| s.view), Some(View::Hex));
        assert!(done.contents.fallback.is_none());
    }

    /// A row that asks for raw on a binary file: raw fails, and the dump
    /// shows in its place at once.
    #[test]
    fn raw_on_a_binary_file_falls_back_to_hex() {
        let binary = fixture_file("views_run_fallback.bin", b"a\0b");
        let table = Table::new(&[FileType::with_views("*.bin", &[View::Raw])]);

        let done = run(&job(&binary, table), None);

        assert_eq!(done.shown.map(|s| s.view), Some(View::Raw));
        assert_eq!(done.contents.fallback.as_deref(), Some("binary file"));
        assert!(done.contents.hex);
        assert_eq!(*done.contents.lines, [crate::hex::line(0, b"a\0b")]);
    }

    #[test]
    fn a_directory_has_a_listing_and_no_row() {
        let dir = fixture_dir("views_run_dir");
        std::fs::write(dir.join("inner.txt"), "x\n").unwrap();
        let done = run(&job(&dir, Table::default()), None);
        assert!(done.shown.is_none());
        assert!(done.contents.directory);
    }

    fn command_job(path: &Path, argv: &[&str], prefetch: bool) -> Job {
        let command = command::Command::of(argv).with_prefetch(prefetch);
        let commands = command::Commands::of([("cmd", command)]);
        let row = FileType::with_view_names("*.cmd", &["cmd", "raw"]);
        let table = Table::build(&[row], &commands).0;
        job(path, table)
    }

    /// A command marked `prefetch = false` runs only for the file on
    /// screen: an LLM summary must not run for ten files on each move.
    #[test]
    fn a_command_with_prefetch_off_does_not_run_for_a_prefetch() {
        let dir = fixture_dir("views_run_no_prefetch");
        let file = dir.join("a.cmd");
        std::fs::write(&file, "x\n").unwrap();
        let mark = dir.join("ran");
        let script = format!("touch {}", mark.display());
        let mut job = command_job(&file, &["sh", "-c", &script], false);

        job.prefetch = true;
        run(&job, Some(&Cache::default()));
        assert!(!mark.exists(), "the command ran for a prefetch");

        job.prefetch = false;
        run(&job, Some(&Cache::default()));
        assert!(
            mark.exists(),
            "the command did not run for the file on screen"
        );
    }

    /// A command's output is kept, so `,` and `.` back to the file do not
    /// run it again. So is a failure, read to the job's extent, so that
    /// prefetch does not run a failing command on each move.
    #[test]
    fn a_command_s_output_and_its_failure_are_cached() {
        let dir = fixture_dir("views_run_command_cache");
        let file = dir.join("a.cmd");
        std::fs::write(&file, "x\n").unwrap();
        let count = dir.join("count");
        let script = format!("echo run >> {}; cat", count.display());
        let runs = || {
            std::fs::read_to_string(&count)
                .unwrap_or_default()
                .lines()
                .count()
        };
        let cache = Cache::default();

        let ok = command_job(&file, &["sh", "-c", &script], true);
        assert_eq!(*run(&ok, Some(&cache)).contents.lines, ["x"]);
        run(
            &Job {
                extent: Extent::Full,
                ..ok
            },
            Some(&cache),
        );
        assert_eq!(runs(), 1, "the output was not kept for either extent");

        let failing = format!("echo run >> {}; exit 1", count.display());
        let bad = command_job(&file, &["sh", "-c", &failing], true);
        let cache = Cache::default();
        let done = run(&bad, Some(&cache));
        assert_eq!(*done.contents.lines, ["x"], "raw is the fallback");
        assert!(done.contents.fallback.is_some());
        run(&bad, Some(&cache));
        assert_eq!(runs(), 2, "a failure was not kept");
        let full = run(
            &Job {
                extent: Extent::Full,
                ..bad
            },
            Some(&cache),
        );
        assert_eq!(
            runs(),
            3,
            "a preview's fallback answered for the whole file"
        );
        assert!(full.contents.fallback.is_some());
    }
}
