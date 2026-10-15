//! Views of a file, and the pipeline that makes them (#455, ADR 0007).
//!
//! A view is a job: (file, view) → contents. Every job runs off the main
//! thread, on `pool::Pool`, so a slow view never stops browsing. The main
//! thread sends a `Job` and adopts the `Done` that comes back.

pub(crate) mod cache;
pub mod cycle;
pub(crate) mod pool;
pub(crate) mod read;

pub use cycle::{FileType, Glob};

use cache::{Cache, Key};
use cycle::{Steps, Table};
use read::{Contents, MAX_PREVIEW_BYTES, PREVIEW_LINES};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One way the file view shows a file. Stage 1 has the built-in views only.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum View {
    /// The file's own text.
    Raw,
    /// The file as a hex dump (#242).
    Hex,
}

impl View {
    /// The name the config and the status line use.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Hex => "hex",
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
    if let Some((cache, key)) = &cached
        && let Some(contents) = cache.get(key, view, job.extent)
    {
        return done(contents, shown);
    }
    let contents = read_view(path, view, job.extent);
    if let Some((cache, key)) = cached {
        cache.put(key, view, job.extent, &contents);
    }
    done(contents, shown)
}

/// `path` in `view`, to `extent`.
fn read_view(path: &Path, view: View, extent: Extent) -> Contents {
    match extent {
        Extent::Preview => {
            read::read_preview_with_caps(path, PREVIEW_LINES, MAX_PREVIEW_BYTES, view)
        }
        Extent::Full => read::read_lines(path, view),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{fixture_dir, fixture_file};

    fn job(path: &Path, table: Table) -> Job {
        let steps = Steps::new(&table);
        Job {
            path: path.to_path_buf(),
            extent: Extent::Preview,
            table: Arc::new(table),
            steps,
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
        assert!(!done.contents.fallback);
    }

    /// A row that asks for raw on a binary file: raw fails, and the dump
    /// shows in its place at once.
    #[test]
    fn raw_on_a_binary_file_falls_back_to_hex() {
        let binary = fixture_file("views_run_fallback.bin", b"a\0b");
        let table = Table::new(&[FileType::with_views("*.bin", &[View::Raw])]);

        let done = run(&job(&binary, table), None);

        assert_eq!(done.shown.map(|s| s.view), Some(View::Raw));
        assert!(done.contents.fallback);
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
}
