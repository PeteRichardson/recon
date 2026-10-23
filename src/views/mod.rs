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
use std::time::{Duration, Instant, SystemTime};

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
    /// Whether a directory listing leads each row with a filetype glyph
    /// (#512). Resolved once at startup and pushed here with everything else
    /// the job needs that the pane cannot decide for itself.
    pub(crate) icons: crate::filetype::Icons,
}

/// The file's row in the table and the view of its cycle that the job ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) row: usize,
    pub(crate) view: View,
    /// The file's head, for `o` (#465).
    pub(crate) head: Head,
}

/// What the file view's title says about the file besides its path (#467).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) modified: Option<SystemTime>,
    /// `None` for a directory: `stat` gives the size of the directory file,
    /// not of what it holds.
    pub(crate) size: Option<u64>,
    pub(crate) mime: &'static str,
}

/// A job's result.
#[derive(Debug, Clone)]
pub(crate) struct Done {
    pub(crate) path: PathBuf,
    pub(crate) contents: Contents,
    /// `None` for a directory, which has no row and no cycle.
    pub(crate) shown: Option<Shown>,
    /// The file's, whatever the view: a command's output is not the file
    /// the title names. `None` when the file could not be read: a FIFO, or
    /// a dangling link, has only its path in the title.
    pub(crate) facts: Option<Facts>,
}

/// Run `job` in the caller: the work a worker does. With a cache, a file
/// already read in this view is not read again.
///
/// Logs one record for the job (#491): at `Debug` for a file the user asked
/// for, at `Trace` for a prefetch, which is work for each file.
pub(crate) fn run(job: &Job, cache: Option<&Cache>) -> Done {
    let path = &job.path;
    let done = |contents, shown, facts| Done {
        path: path.clone(),
        contents,
        shown,
        facts,
    };
    // One stat for all of it. `fs::metadata` follows a link, so a link's
    // facts are its target's, as its contents are.
    let meta = std::fs::metadata(path).ok();
    let modified = meta.as_ref().and_then(|meta| meta.modified().ok());
    if meta.as_ref().is_some_and(std::fs::Metadata::is_dir) {
        let max_lines = match job.extent {
            Extent::Preview => PREVIEW_LINES,
            Extent::Full => usize::MAX,
        };
        let facts = Facts {
            modified,
            size: None,
            mime: crate::mime::DIRECTORY,
        };
        let started = Instant::now();
        let contents = read::directory_listing(path, max_lines, &job.icons);
        log_job(job, "directory", "", &contents, None, started.elapsed());
        return done(contents, None, Some(facts));
    }
    let cached = cache.zip(meta.as_ref().and_then(|meta| Key::with_meta(path, meta)));
    // Read for every file, not only when a row asks: the title shows the
    // type (#467). The cache keeps it to one read per version of the file.
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
    let facts = head.mime.map(|mime| Facts {
        modified,
        size: meta.as_ref().map(std::fs::Metadata::len),
        mime,
    });
    let done = |contents, shown| done(contents, shown, facts);
    let row = job.table.row_for(path, || head);
    let view = job.steps.view(&job.table, row);
    let shown = Some(Shown { row, view, head });
    if job.prefetch
        && job
            .table
            .command(view)
            .is_some_and(|command| !command.prefetch)
    {
        return done(Contents::default(), shown);
    }
    let name = job.table.view_name(view);
    if let Some((cache, key)) = &cached {
        if let Some(contents) = cache.get(key, view, job.extent) {
            crate::logging::count(&crate::logging::CACHE_HITS);
            log::log!(
                job_level(job),
                "view job: {}, cache hit",
                job_names(job, name)
            );
            return done(contents, shown);
        }
        crate::logging::count(&crate::logging::CACHE_MISSES);
    }
    let started = Instant::now();
    let (contents, extent) = read_view(job, view);
    let cache_state = if cached.is_some() {
        ", cache miss"
    } else {
        ", no cache"
    };
    let size = meta.as_ref().map(std::fs::Metadata::len);
    log_job(job, name, cache_state, &contents, size, started.elapsed());
    if let Some((cache, key)) = cached {
        cache.put(key, view, extent, &contents);
    }
    done(contents, shown)
}

/// `Trace` for a prefetch, `Debug` for a file the user asked for.
const fn job_level(job: &Job) -> log::Level {
    if job.prefetch {
        log::Level::Trace
    } else {
        log::Level::Debug
    }
}

/// The start of a job's record: the file, the view, the extent, and
/// whether it is a prefetch.
fn job_names(job: &Job, view: &str) -> String {
    format!(
        "{}, {view}, {}{}",
        job.path.display(),
        match job.extent {
            Extent::Preview => "preview",
            Extent::Full => "full",
        },
        if job.prefetch { ", prefetch" } else { "" },
    )
}

/// The record of a job that read its file: `took` is the whole read, and
/// a picture's decode is part of it (#491). For a picture in a file the
/// decoder reads as it decodes, so the decode time is the read time too.
///
/// A reader that reads the whole file gives no byte count: `size`, from
/// the job's one stat, is the count then.
fn log_job(
    job: &Job,
    view: &str,
    cache_state: &str,
    contents: &Contents,
    size: Option<u64>,
    took: Duration,
) {
    let level = job_level(job);
    if !log::log_enabled!(level) {
        return;
    }
    let decoded = contents.decoded.map_or_else(String::new, |decoded| {
        let scale = if decoded.scale > 1 {
            format!(" at 1/{} scale", decoded.scale)
        } else {
            String::new()
        };
        format!(", decoded in {:.1?}{scale}", decoded.took)
    });
    log::log!(
        level,
        "view job: {}{cache_state}, {} bytes read in {took:.1?}{decoded}",
        job_names(job, view),
        if contents.bytes > 0 {
            contents.bytes
        } else {
            size.unwrap_or(0)
        },
    );
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
            read::read_preview_with_caps(path, PREVIEW_LINES, MAX_PREVIEW_BYTES, view, &job.icons)
        }
        Extent::Full => read::read_lines(path, view, &job.icons),
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

    /// A preview job for `path`, as the explorer's selection sends one.
    pub(crate) fn job(path: &Path, table: Table) -> Job {
        let steps = Steps::new(&table);
        Job {
            path: path.to_path_buf(),
            extent: Extent::Preview,
            table: Arc::new(table),
            steps,
            px: 0,
            prefetch: false,
            icons: crate::filetype::Icons::off(),
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

    /// `o` uses the head the job read, so it never reads the file again.
    #[test]
    fn a_job_keeps_the_head_it_read() {
        let binary = fixture_file("views_run_head.bin", b"a\0b");
        let table = Table::new(&[FileType::with_views("*.bin", &[View::Hex])]);
        let done = run(&job(&binary, table), None);
        assert_eq!(
            done.shown.map(|shown| shown.head),
            Some(Head {
                binary: true,
                mime: Some("application/octet-stream")
            })
        );
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

    /// A job writes one `Debug` record (#491): the file, the view, the
    /// extent, the cache's answer, the bytes and the time.
    #[test]
    fn a_job_logs_what_it_read_and_how_long_it_took() {
        use crate::logging::capture;
        capture::install();
        let text = fixture_file("views_log_job.txt", b"alpha\nbeta\n");
        let cache = Cache::with_limit(1 << 20);

        run(&job(&text, Table::default()), Some(&cache));

        let records = capture::here_at(log::Level::Debug, "views_log_job.txt");
        assert_eq!(records.len(), 1, "{:?}", capture::here());
        let record = &records[0];
        assert!(record.starts_with("view job: "), "{record}");
        assert!(record.contains(", raw, preview, cache miss,"), "{record}");
        assert!(record.contains(" 11 bytes read in "), "{record}");

        run(&job(&text, Table::default()), Some(&cache));

        let records = capture::here_at(log::Level::Debug, "views_log_job.txt");
        assert_eq!(records.len(), 2, "{:?}", capture::here());
        assert!(
            records[1].ends_with(", raw, preview, cache hit"),
            "{records:?}"
        );
    }

    /// A prefetch is work for each file that no one asked for: `Trace`.
    #[test]
    fn a_prefetch_job_logs_at_trace() {
        use crate::logging::capture;
        capture::install();
        let text = fixture_file("views_log_prefetch.txt", b"alpha\n");

        run(
            &Job {
                prefetch: true,
                ..job(&text, Table::default())
            },
            None,
        );

        let debug = capture::here_at(log::Level::Debug, "views_log_prefetch.txt");
        assert!(debug.is_empty(), "{debug:?}");
        let records = capture::here_at(log::Level::Trace, "views_log_prefetch.txt");
        assert_eq!(records.len(), 1, "{:?}", capture::here());
        assert!(records[0].contains(", prefetch, no cache,"), "{records:?}");
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
        encoded(
            &image::RgbImage::from_pixel(width, height, image::Rgb(rgb)),
            image::ImageFormat::Png,
        )
    }

    /// `image` as a file in `format` would hold it.
    fn encoded(image: &image::RgbImage, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
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

    /// A mid-grey baseline JPEG `width` × `height` pixels, written by hand
    /// so a picture far past the memory limit costs only a few MiB (#484).
    ///
    /// One grey component, and each Huffman table has one code, `0`: a
    /// block is two bits, a DC difference of 0 and an end of block.
    fn grey_jpeg(width: u16, height: u16) -> Vec<u8> {
        let [w1, w0] = width.to_be_bytes();
        let [h1, h0] = height.to_be_bytes();
        let mut bytes = vec![0xFF, 0xD8];
        // The quantization table: every step 1.
        bytes.extend([0xFF, 0xDB, 0x00, 0x43, 0x00]);
        bytes.extend([1; 64]);
        bytes.extend([0xFF, 0xC0, 0x00, 0x0B, 8, h1, h0, w1, w0, 1, 1, 0x11, 0]);
        // DC table 0, then AC table 0: one code of length 1, for symbol 0.
        for class in [0x00, 0x10] {
            bytes.extend([0xFF, 0xC4, 0x00, 0x14, class, 1]);
            bytes.extend([0; 15]);
            bytes.push(0);
        }
        bytes.extend([0xFF, 0xDA, 0x00, 0x08, 1, 1, 0x00, 0, 63, 0]);
        let blocks = usize::from(width.div_ceil(8)) * usize::from(height.div_ceil(8));
        bytes.resize(bytes.len() + blocks / 4, 0);
        if blocks % 4 != 0 {
            // The rest of the last byte is padded with 1 bits.
            bytes.push(0xFF >> (2 * (blocks % 4)));
        }
        bytes.extend([0xFF, 0xD9]);
        bytes
    }

    /// A scan is too large to decode at full size inside the memory limit
    /// (#484). It is decoded at a reduced scale, so it shows, and kept no
    /// larger than the cap.
    #[test]
    fn a_jpeg_too_large_to_decode_whole_shows_at_the_cap() {
        let side = read::MAX_IMAGE_SIDE;
        let file = fixture_file("views_run_huge.jpg", &grey_jpeg(32_000, 20_000));
        let table = Table::new(&[FileType::with_views("*.jpg", &[View::Image])]);

        let done = run(&job(&file, table), None);

        assert!(
            done.contents.fallback.is_none(),
            "{:?}",
            done.contents.fallback
        );
        let image = done.contents.image.expect("an image");
        assert_eq!((image.width(), image.height()), (side, side * 5 / 8));
        assert_eq!(image.to_luma8().get_pixel(1000, 500).0, [128]);
    }

    /// The scale is the smallest that still keeps the longest side at the
    /// cap: ½ here, as ¼ would leave 1024 pixels to stretch.
    #[test]
    fn a_jpeg_is_not_scaled_below_the_cap() {
        let side = read::MAX_IMAGE_SIDE;
        let file = fixture_file("views_run_half.jpg", &grey_jpeg(side as u16 * 2, 64));
        let table = Table::new(&[FileType::with_views("*.jpg", &[View::Image])]);

        let image = run(&job(&file, table), None).contents.image.unwrap();

        assert_eq!((image.width(), image.height()), (side, 32));
    }

    /// The record of a picture names its decode, and the scale when it was
    /// reduced (#491).
    #[test]
    fn a_picture_logs_its_decode_and_its_scale() {
        use crate::logging::capture;
        capture::install();
        let side = read::MAX_IMAGE_SIDE as u16;
        let file = fixture_file("views_log_half.jpg", &grey_jpeg(side * 2, 64));
        let table = Table::new(&[FileType::with_views("*.jpg", &[View::Image])]);

        run(&job(&file, table), None);

        let records = capture::here_at(log::Level::Debug, "views_log_half.jpg");
        assert_eq!(records.len(), 1, "{:?}", capture::here());
        assert!(
            records[0].contains(", image, preview, no cache,"),
            "{records:?}"
        );
        assert!(records[0].contains(", decoded in "), "{records:?}");
        assert!(records[0].ends_with(" at 1/2 scale"), "{records:?}");
    }

    /// A JPEG too small to scale decodes as it did before #484, pixel for
    /// pixel.
    #[test]
    fn a_small_jpeg_decodes_as_before() {
        let bytes = encoded(
            &image::RgbImage::from_fn(64, 32, |x, y| image::Rgb([x as u8 * 4, y as u8 * 8, 99])),
            image::ImageFormat::Jpeg,
        );
        let file = fixture_file("views_run_small.jpg", &bytes);
        let table = Table::new(&[FileType::with_views("*.jpg", &[View::Image])]);

        let image = run(&job(&file, table), None).contents.image.unwrap();

        assert_eq!(*image, image::load_from_memory(&bytes).unwrap());
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

    /// When the file changed, as its metadata says.
    fn modified(path: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).unwrap().modified().ok()
    }

    /// The title's facts are the file's (#467): its size and type, whatever
    /// row or view shows it.
    #[test]
    fn a_job_carries_the_file_s_facts() {
        let file = fixture_file("views_run_facts.rs", b"fn main() {}\n");
        let table = Table::new(&[FileType::with_views("*.rs", &[View::Hex])]);

        let facts = run(&job(&file, table), None).facts;

        assert_eq!(
            facts,
            Some(Facts {
                modified: modified(&file),
                size: Some(13),
                mime: "text/x-rust",
            })
        );
    }

    /// A command view shows the facts of the file it ran on, not of its
    /// output: the title names the file.
    #[test]
    fn a_command_view_carries_the_file_s_facts_not_the_output_s() {
        let picture = png(1, 1);
        let file = fixture_file("views_run_facts.cmd", &picture);
        let done = run(&command_job(&file, &["echo", "some output"], true), None);

        assert_eq!(done.shown.map(|s| s.view), Some(View::Command(0)));
        assert_eq!(*done.contents.lines, ["some output"]);
        let facts = done.facts.expect("facts");
        assert_eq!(
            (facts.size, facts.mime),
            (Some(picture.len() as u64), "image/png")
        );
    }

    /// A directory has a date and its type, but no size: what `stat` says
    /// is the size of the directory file, not of what it holds.
    #[test]
    fn a_directory_carries_its_date_and_type_and_no_size() {
        let dir = fixture_dir("views_run_facts_dir");
        let facts = run(&job(&dir, Table::default()), None).facts;
        assert_eq!(
            facts,
            Some(Facts {
                modified: modified(&dir),
                size: None,
                mime: crate::mime::DIRECTORY,
            })
        );
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
