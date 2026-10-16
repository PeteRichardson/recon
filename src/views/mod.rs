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
use cycle::{Head, Steps, Table};
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
    /// The file as a picture (#458).
    Image,
    /// The output of the `i`th command of the table (#457).
    Command(usize),
}

/// The names of the built-in views, which a command cannot take.
/// `quicklook` is a built-in command, on macOS only (#458).
pub(crate) const BUILT_IN: [&str; 4] = ["raw", "hex", "image", "quicklook"];

impl View {
    /// The built-in view called `name`, if this version can show it.
    pub(crate) fn built_in(name: &str) -> Option<Self> {
        match name {
            "raw" => Some(Self::Raw),
            "hex" => Some(Self::Hex),
            "image" => Some(Self::Image),
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
    /// The file's head, if a row asked for it: `o` asks only when this
    /// is `None` (#465).
    pub(crate) head: Option<Head>,
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
    let mut read_head = None;
    let row = job.table.row_for(path, || {
        let head = if let Some((cache, key)) = &cached
            && let Some(head) = cache.head(key)
        {
            head
        } else {
            let head = read::head(path);
            if let Some((cache, key)) = &cached {
                cache.set_head(key, head);
            }
            head
        };
        *read_head.insert(head)
    });
    let view = job.steps.view(&job.table, row);
    let shown = Some(Shown {
        row,
        view,
        head: read_head,
    });
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
        let contents = command::run(command, path, job.px).and_then(|output| {
            if command.is_image() {
                read::image_bytes(&output)
            } else {
                Ok(read::command_output(&output, command.output.as_deref()))
            }
        });
        return match contents {
            Ok(contents) => (contents, Extent::Full),
            Err(reason) => fall_back(job, reason),
        };
    }
    if view == View::Image {
        return match read::image_file(path) {
            Ok(contents) => (contents, Extent::Full),
            Err(reason) => fall_back(job, reason),
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

/// Raw, which is hex for a binary file, in place of a view that failed
/// for `reason` (#457).
fn fall_back(job: &Job, reason: String) -> (Contents, Extent) {
    let (contents, extent) = read_view(job, View::Raw);
    let contents = Contents {
        fallback: Some(reason),
        ..contents
    };
    (contents, extent)
}

#[cfg(test)]
pub(crate) mod tests {
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

    /// `o` asks the head again only when the job did not read it.
    #[test]
    fn a_job_keeps_the_head_it_read_and_a_glob_row_reads_none() {
        let binary = fixture_file("views_run_head.bin", b"a\0b");
        let done = run(&job(&binary, Table::default()), None);
        let head = done.shown.and_then(|shown| shown.head);
        assert_eq!(
            head,
            Some(Head {
                binary: true,
                mime: None
            })
        );

        let table = Table::new(&[FileType::with_views("*.bin", &[View::Hex])]);
        let done = run(&job(&binary, table), None);
        assert_eq!(done.shown.and_then(|shown| shown.head), None);
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

    /// A black PNG `width` × `height` pixels, as a file would hold it.
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        png_filled(width, height, [0, 0, 0])
    }

    /// A PNG `width` × `height` pixels of one colour.
    pub(crate) fn png_filled(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(width, height, image::Rgb(rgb))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn the_image_view_decodes_the_file() {
        let file = fixture_file("views_run_image.png", &png(4, 2));
        let table = Table::new(&[FileType::with_views("*.png", &[View::Image])]);

        let done = run(&job(&file, table), None);

        assert_eq!(done.shown.map(|s| s.view), Some(View::Image));
        let image = done.contents.image.expect("an image");
        assert_eq!((image.width(), image.height()), (4, 2));
        assert!(done.contents.lines.is_empty(), "an image has no lines");
        assert!(done.contents.fallback.is_none());
    }

    /// A photo is far larger than a pane. It is kept no larger than the cap,
    /// in the same shape, so the cache and the draw do not pay for pixels
    /// that no terminal shows.
    #[test]
    fn a_large_image_is_shrunk_to_the_cap() {
        let side = read::MAX_IMAGE_SIDE;
        let file = fixture_file("views_run_large.png", &png(side * 2, side / 2));
        let table = Table::new(&[FileType::with_views("*.png", &[View::Image])]);

        let image = run(&job(&file, table), None).contents.image.unwrap();

        assert_eq!((image.width(), image.height()), (side, side / 4));
    }

    /// A `.png` that holds text: the view fails, and the text shows with
    /// the reason, as a failed command's does.
    #[test]
    fn a_file_that_is_not_an_image_falls_back_to_raw() {
        let file = fixture_file("views_run_not_image.png", b"not a picture\n");
        let table = Table::new(&[FileType::with_views("*.png", &[View::Image])]);

        let done = run(&job(&file, table), None);

        assert!(done.contents.image.is_none());
        assert_eq!(*done.contents.lines, ["not a picture"]);
        assert!(done.contents.fallback.is_some());
    }

    fn command_job(path: &Path, argv: &[&str], prefetch: bool) -> Job {
        let command = command::Command::of(argv).with_prefetch(prefetch);
        let commands = command::Commands::of([("cmd", command)]);
        let row = FileType::with_view_names("*.cmd", &["cmd", "raw"]);
        let table = Table::build(&[row], &commands).0;
        job(path, table)
    }

    /// `mmdc` and `qlmanage` write a picture: it goes to the image view.
    /// Output that does not decode fails the view, as a non-zero exit does.
    #[test]
    fn a_command_with_image_output_shows_an_image() {
        let dir = fixture_dir("views_run_image_command");
        let file = dir.join("a.cmd");
        std::fs::write(&file, png(3, 5)).unwrap();
        let picture = |argv: &[&str]| {
            let command = command::Command::of(argv).with_output("image/png");
            let commands = command::Commands::of([("cmd", command)]);
            let row = FileType::with_view_names("*.cmd", &["cmd", "hex"]);
            job(&file, Table::build(&[row], &commands).0)
        };

        let done = run(&picture(&["cat"]), None);
        let image = done.contents.image.expect("an image");
        assert_eq!((image.width(), image.height()), (3, 5));
        assert_eq!(done.shown.map(|s| s.view), Some(View::Command(0)));

        let done = run(&picture(&["echo", "not a picture"]), None);
        assert!(done.contents.image.is_none());
        assert!(done.contents.fallback.is_some());
    }

    /// The built-in `quicklook` runs the real `qlmanage`: a row that names
    /// it shows the thumbnail, no larger than `{px}`.
    #[cfg(target_os = "macos")]
    #[test]
    fn quicklook_shows_the_thumbnail() {
        let file = fixture_file("views_run_quicklook.md", b"# A heading\n\nSome text.\n");
        let row = FileType::with_view_names("*.md", &["quicklook", "raw"]);
        let mut job = job(&file, Table::new(&[row]));
        job.px = 64;

        let done = run(&job, None);

        assert_eq!(done.contents.fallback, None);
        let image = done.contents.image.expect("a thumbnail");
        assert!(image.width().max(image.height()) <= 64, "{image:?}");
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
