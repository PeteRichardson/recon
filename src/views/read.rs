//! The read jobs for the built-in views: raw and hex, and a directory's
//! listing (#455). They run on a worker, never on the main thread.

use super::View;
use crate::document::{self, Sniff, read_lossy_line, read_utf16_lines, sniff};
use crate::hex;
use crate::widgets::explorer::Entry;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

/// Lines read for a preview.
///
/// Not "enough to fill the pane", which is what this was: at 500 lines it
/// truncated essentially every real file recon is pointed at, and a truncated
/// document is one that filters and counts report wrong answers over — the
/// defect in #27. The cap exists to bound work on a genuinely enormous log,
/// not to avoid reading a log-sized log.
///
/// 50,000 is ~5x the largest file in the corpus this was measured against
/// (a few thousand real log files: p50 1,084 lines, max 10,000), and
/// `MAX_PREVIEW_BYTES` is the binding limit well before it at any realistic
/// line length.
///
/// `pub(crate)` for the tests in `app/tests/`: reaching the truncated branch
/// through `App` means a fixture past this cap, and one that hard-coded the
/// number would quietly stop testing truncation the next time it moves.
pub(crate) const PREVIEW_LINES: usize = 50_000;

/// Byte ceiling for a preview. A file with no newlines is a single enormous
/// line, which the line cap alone would happily read in full.
///
/// 10 MiB is ~10x the largest measured file and bounds the worst-case blocking
/// read at roughly 10 ms — read, `Document` clone and a filter pass together
/// run about 1 ms/MB, so this stays far inside the ~100 ms a delay would have
/// to reach before anyone noticed it.
pub(crate) const MAX_PREVIEW_BYTES: u64 = 10 << 20;

/// Shown when the explorer's selection is a directory with nothing in it.
///
/// A directory now renders as its listing, so this survives only for the case
/// where there is no listing to show. Distinguished from `<directory>`, which
/// this used to be: "I gave you a directory and got nothing" is the one case
/// that could otherwise read as a bug rather than as an answer.
pub(crate) const EMPTY_DIRECTORY_MESSAGE: &str = "<empty directory>";

/// What a read job found: the lines it read, whether more
/// remain, and how many lines the whole file probably has.
#[derive(Debug, Clone, Default)]
pub(crate) struct Contents {
    pub(crate) lines: Arc<Vec<String>>,
    pub(crate) truncated: bool,
    /// `None` when the file was read whole (the count is not a guess then, it
    /// is `lines.len()`) or when there was nothing to estimate from.
    pub(crate) estimated_lines: Option<usize>,
    /// Whether `lines` are the file's own text, as opposed to a directory
    /// listing or a message standing in for a file that could not be shown.
    /// Only text is syntax-coloured (#122): `<binary file>` beside a `.rs`
    /// path is not Rust, and a listing is not whatever its directory is
    /// named after.
    pub(crate) text: bool,
    /// Whether `lines` are a hex dump of the file (#242). A dump is the
    /// file's own content, so `text` is true for it too, but it is never
    /// syntax-coloured and never numbered: the offset in each line is its
    /// position.
    pub(crate) hex: bool,
    /// Whether `lines` are a directory's listing.
    pub(crate) directory: bool,
    /// Whether the view asked for could not run on this file, and these
    /// lines are the fallback: hex for a binary file asked for as raw.
    pub(crate) fallback: bool,
}

impl Contents {
    /// Contents that are really an error or a placeholder. Not truncated —
    /// there is nothing better to re-read later — and nothing to estimate.
    pub(crate) fn message(text: String) -> Self {
        Self {
            lines: Arc::new(vec![text]),
            ..Contents::default()
        }
    }
}

/// Read `path` whole, or a single-line message describing why it could not
/// be read. The reading itself is `document::read_lines` (#143); this wraps
/// its error the way the pane shows it.
///
/// Never `truncated`, and never estimating: the whole file is here.
///
/// `view` is `Raw` for the file's text, or `Hex` for its dump (#242). A
/// binary file asked for as raw falls back to its dump.
pub(crate) fn read_lines(path: &Path, view: View) -> Contents {
    // See `read_preview_with_caps`: a directory opens fine and then fails to read, so
    // it is recognised up front rather than surfacing an OS error string.
    if path.is_dir() {
        return directory_listing(path, usize::MAX);
    }
    if view == View::Hex {
        return read_hex(path, None);
    }
    match document::read_lines(path) {
        Ok(lines) => Contents {
            lines: Arc::new(lines),
            text: true,
            ..Contents::default()
        },
        Err(err) if document::is_binary(&err) => binary(path, None),
        // Logged as well as shown (#83). The pane gets `<{err}>` in place of
        // the file, which tells the user *that* it failed; the log is where
        // the full path lives, and the pane's title is elided when the pane
        // is narrow.
        Err(err) => {
            log::warn!("cannot read {}: {err}", path.display());
            Contents::message(format!("<{err}>"))
        }
    }
}

/// Read at most a screenful of `path`, reporting whether anything was left.
///
/// Bounded on both axes: `PREVIEW_LINES` lines and `MAX_PREVIEW_BYTES` bytes.
/// Because the reader stops as soon as either is reached, the cost does not
/// grow with the size of the file and there is no long-running work to cancel
/// when the selection moves on.
///
/// A file that cannot be read reports the reason and is *not* marked truncated
/// — there is nothing better to re-read later.
/// Both caps are injected rather than read from the constants.
///
/// The preview job is the only caller outside tests and always passes
/// `PREVIEW_LINES` and `MAX_PREVIEW_BYTES`. The parameters exist because those
/// constants are now 50,000 lines and 10 MiB: exercising either cap against
/// them means building a multi-megabyte fixture per test, and the byte cap
/// alone cost more than the entire rest of the suite. With the caps injectable
/// a handful of bytes is enough, and a test states the cap it is testing
/// instead of deriving it from a constant it does not control.
///
/// `view` is as `read_lines` takes it. A dump's caps are in bytes, sixteen to
/// a line, so the line cap binds long before the byte cap does: 50,000 lines
/// is only 800 KB of file.
pub(crate) fn read_preview_with_caps(
    path: &Path,
    max_lines: usize,
    max_bytes: u64,
    view: View,
) -> Contents {
    // Checked before opening, not after failing to read. `File::open` on a
    // directory *succeeds* on macOS and the read then fails `EISDIR`, so
    // falling through to the error path below would display
    // `<Is a directory (os error 21)>` — platform-specific and meaningless
    // to a reader.
    if path.is_dir() {
        return directory_listing(path, max_lines);
    }
    let hex_bytes = (max_lines as u64)
        .saturating_mul(hex::BYTES_PER_LINE as u64)
        .min(max_bytes);
    if view == View::Hex {
        return read_hex(path, Some(hex_bytes));
    }
    // The same stat guards a FIFO (#221): its open would block until a
    // writer appeared, and this runs on every selection move.
    let file = match document::refuse_unreadable(path).and_then(|()| File::open(path)) {
        Ok(file) => file,
        // Logged as well as shown, as `read_lines` does (#83, #189). The
        // preview is the arrow-key case: the pane's title is elided when
        // the pane is narrow, so the log is where the path survives.
        Err(err) => {
            log::warn!("cannot preview {}: {err}", path.display());
            return Contents::message(format!("<{err}>"));
        }
    };
    // Read before the bytes are consumed; a file that cannot be stat'd simply
    // gets no estimate rather than failing the preview.
    let file_bytes = file.metadata().ok().map(|meta| meta.len());

    let mut reader = BufReader::new(file.take(max_bytes));
    let head = match sniff(&mut reader) {
        Ok((Sniff::Text, head)) => head,
        Ok((Sniff::Binary, _)) => return binary(path, Some(hex_bytes)),
        // UTF-16 is decoded whole, so the byte cap is the only cap the read
        // itself knows; the line cap is applied to the result below.
        Ok((Sniff::Utf16(endian), head)) => {
            let mut lines = match read_utf16_lines(head, &mut reader, endian) {
                Ok(lines) => lines,
                // Logged as well as shown, as `read_lines` does (#83, #189).
                // The preview is the arrow-key case: the pane's title is
                // elided when the pane is narrow, so the log is where the
                // path survives.
                Err(err) => {
                    log::warn!("cannot preview {}: {err}", path.display());
                    return Contents::message(format!("<{err}>"));
                }
            };
            let over_the_line_cap = lines.len() > max_lines;
            lines.truncate(max_lines);
            let remaining = reader.into_inner().limit();
            return capped(
                lines,
                over_the_line_cap || remaining == 0,
                file_bytes,
                max_bytes - remaining,
            );
        }
        // Logged as well as shown, as `read_lines` does (#83, #189). The
        // preview is the arrow-key case: the pane's title is elided when
        // the pane is narrow, so the log is where the path survives.
        Err(err) => {
            log::warn!("cannot preview {}: {err}", path.display());
            return Contents::message(format!("<{err}>"));
        }
    };

    // The sniffed bytes are content, so they go back in front of the rest.
    let mut reader = Cursor::new(head).chain(reader);
    let mut lines = Vec::new();
    let mut buf = Vec::new();

    while lines.len() < max_lines {
        match read_lossy_line(&mut reader, &mut buf) {
            Ok(Some(line)) => lines.push(line),
            Ok(None) => break,
            // Logged as well as shown, as `read_lines` does (#83, #189). The
            // preview is the arrow-key case: the pane's title is elided when
            // the pane is narrow, so the log is where the path survives.
            Err(err) => {
                log::warn!("cannot preview {}: {err}", path.display());
                return Contents::message(format!("<{err}>"));
            }
        }
    }

    // Either the line budget ran out, or the byte allowance did. A file that
    // ends exactly on a cap is reported as truncated, which only costs a
    // redundant re-read the first time the view is used.
    let remaining = reader.into_inner().1.into_inner().limit();
    let truncated = lines.len() == max_lines || remaining == 0;
    capped(lines, truncated, file_bytes, max_bytes - remaining)
}

/// What a binary file shows: its dump (#242). A binary file asked for as
/// raw has no text to show, so raw fails and the dump is its fallback.
///
/// `preview` is as `read_hex` takes it.
fn binary(path: &Path, preview: Option<u64>) -> Contents {
    let mut contents = read_hex(path, preview);
    contents.fallback = contents.hex;
    contents
}

/// `path` as a hex dump (#242): whole, up to `hex::MAX_HEX_BYTES`, or no
/// more than `preview` bytes of it.
///
/// A preview cut short is `truncated`, so the rest is read as soon as the
/// view is used, exactly as a text preview's is. Its line count is not an
/// estimate — a dump's length follows from the file's size. A full read that
/// reaches `MAX_HEX_BYTES` is not truncated, since reading again would get no
/// further; it ends with a line that says how much was left out instead.
fn read_hex(path: &Path, preview: Option<u64>) -> Contents {
    let cap = preview.map_or(hex::MAX_HEX_BYTES, |bytes| bytes.min(hex::MAX_HEX_BYTES));
    let dump = match hex::read(path, cap) {
        Ok(dump) => dump,
        // Logged as well as shown, as `read_lines` does (#83).
        Err(err) => {
            log::warn!("cannot read {} as hex: {err}", path.display());
            return Contents::message(format!("<{err}>"));
        }
    };
    let mut lines = dump.lines;
    let (truncated, estimated_lines) = match dump.unread {
        None => (false, None),
        Some(unread) if cap < hex::MAX_HEX_BYTES => {
            let size = cap.saturating_add(unread);
            let stop_line = usize::from(size > hex::MAX_HEX_BYTES);
            let dumped = hex::line_count(size.min(hex::MAX_HEX_BYTES));
            (true, Some(dumped + stop_line))
        }
        Some(unread) => {
            lines.push(hex::stop_line(unread));
            (false, None)
        }
    };
    Contents {
        lines: Arc::new(lines),
        truncated,
        estimated_lines,
        text: true,
        hex: true,
        ..Contents::default()
    }
}

/// A preview's `Contents`, with a line estimate only when it was cut short —
/// there is nothing to estimate about a file that was read whole.
fn capped(
    lines: Vec<String>,
    truncated: bool,
    file_bytes: Option<u64>,
    bytes_read: u64,
) -> Contents {
    let estimated_lines = if truncated {
        estimate_lines(file_bytes, bytes_read, lines.len())
    } else {
        None
    };
    Contents {
        lines: Arc::new(lines),
        truncated,
        estimated_lines,
        text: true,
        ..Contents::default()
    }
}

/// Widest a name column grows before long names are allowed to push the
/// metadata out of line on their own row.
///
/// Same trade `MAX_EXPLORER_WIDTH` makes: one pathological 200-character filename
/// would otherwise pad *every* row out to 200 columns and push the size and
/// time off screen for all of them. Capping means that one row misaligns
/// instead of all of them going blank.
const NAME_COLUMN_MAX: usize = 40;

/// Columns a size occupies, right-aligned. `1023B` and `999.9K` both fit.
const SIZE_COLUMN: usize = 6;

/// `bytes` as something readable at a glance, in the width `SIZE_COLUMN` gives.
///
/// Binary units, since this reports what the filesystem reports. One decimal
/// above 1 KiB: `18.4K` says as much as `18841` in fewer columns, and the
/// exact byte count is not what anyone scans a listing for.
fn format_size(bytes: u64) -> String {
    const UNITS: [(u64, char); 4] = [(1 << 30, 'G'), (1 << 20, 'M'), (1 << 10, 'K'), (1, 'B')];
    for (scale, suffix) in UNITS {
        if bytes >= scale {
            return if scale == 1 {
                format!("{bytes}B")
            } else {
                // Truncating rather than rounding, so a listing never claims a
                // file reached the next unit before it did.
                let whole = bytes / scale;
                let tenth = (bytes % scale) * 10 / scale;
                format!("{whole}.{tenth}{suffix}")
            };
        }
    }
    "0B".to_string()
}

/// `time` as a local calendar datetime, or `None` if it cannot be represented.
///
/// Local, not UTC: the logs recon reads carry local timestamps, and a listing
/// disagreeing with the lines inside the files would be its own small bug.
/// `jiff` resolves the zone from the system (`/etc/localtime` on Unix), which
/// is the part that is genuinely hard to get right by hand.
fn format_modified(time: std::time::SystemTime) -> Option<String> {
    let zoned = jiff::Zoned::try_from(time).ok()?;
    Some(zoned.strftime("%Y-%m-%d %H:%M").to_string())
}

/// One row: name, then size, then when it changed.
///
/// The metadata sits to the *right* of the name deliberately. The view pane
/// narrows when the explorer is wide, and a row clipped at the pane's edge
/// then loses the time first, the size next, and the name last — which is the
/// priority order this wants, achieved by layout rather than by logic that
/// would need a width the listing does not have when it is built.
pub(crate) fn listing_row(entry: &Entry, name_width: usize) -> String {
    let size = entry.size.map_or_else(|| "-".to_string(), format_size);
    let modified = entry.modified.and_then(format_modified).unwrap_or_default();
    // Padded by hand rather than with `{:<name_width$}`, which counts `char`s.
    // `name_width` is terminal columns, so a CJK name would otherwise be padded
    // as though its ideographs were one column each and push its own size
    // column one place right per glyph (#97).
    let name = entry.display();
    let pad = " ".repeat(name_width.saturating_sub(UnicodeWidthStr::width(name.as_str())));
    format!("{name}{pad}  {size:>SIZE_COLUMN$}  {modified}")
        .trim_end()
        .to_string()
}

/// A directory rendered as its contents, bounded by `max_lines`.
///
/// The view is the widest pane on screen and was spending all of it on the
/// word `<directory>`. Listing what is actually there turns a selected
/// directory into a look-ahead — and `l` on that selection makes the listing
/// the explorer's own, which is what stops it being a navigable-looking list
/// that cannot be navigated.
///
/// `..` is absent deliberately: it is the explorer's way back out, and there
/// is nothing here that could act on it.
pub(crate) fn directory_listing(path: &Path, max_lines: usize) -> Contents {
    let entries = match crate::widgets::explorer::sorted_entries(path) {
        Ok(entries) => entries,
        // Same shape as an unreadable file: say why, verbatim from the OS.
        // Worded distinctly from the explorer's own "cannot list" (#83, in
        // explorer.rs): that is a different subsystem failing at the same
        // directory, and a reader of the log should be able to tell them
        // apart rather than seeing two identical lines.
        Err(err) => {
            log::warn!("cannot show the listing for {}: {err}", path.display());
            return Contents {
                directory: true,
                ..Contents::message(format!("<{err}>"))
            };
        }
    };
    if entries.is_empty() {
        return Contents {
            directory: true,
            ..Contents::message(EMPTY_DIRECTORY_MESSAGE.to_string())
        };
    }
    let total = entries.len();
    let shown = &entries[..entries.len().min(max_lines)];
    // Padded to the longest name actually on screen, so the columns line up
    // without every listing being as wide as the widest possible name.
    let name_width = shown
        .iter()
        .map(|entry| UnicodeWidthStr::width(entry.display().as_str()))
        .max()
        .unwrap_or(0)
        .min(NAME_COLUMN_MAX);
    let lines: Vec<String> = shown
        .iter()
        .map(|entry| listing_row(entry, name_width))
        .collect();
    let truncated = total > lines.len();
    Contents {
        lines: Arc::new(lines),
        directory: true,
        truncated,
        // Unlike a file, the real count is known exactly rather than scaled
        // from a sample — there is no guessing to do.
        estimated_lines: truncated.then_some(total),
        ..Contents::default()
    }
}

/// How many lines a file of `file_bytes` probably holds, given that its first
/// `bytes_read` bytes held `lines_read` lines.
///
/// Scaling the sample's bytes-per-line up to the whole file is a guess, but a
/// cheap one — the alternative is reading the file to count its newlines,
/// which is exactly the unbounded work a preview exists to avoid. It only
/// has to be right to the *digit*, since all it feeds is a gutter width, so
/// being off by 10% costs nothing and being off by 10x costs one redraw: the
/// same redraw as having made no estimate at all.
///
/// `None` when there is nothing to scale from — no readable size, an empty
/// read, or a preview that consumed no bytes.
fn estimate_lines(file_bytes: Option<u64>, bytes_read: u64, lines_read: usize) -> Option<usize> {
    let file_bytes = file_bytes?;
    if bytes_read == 0 || lines_read == 0 {
        return None;
    }
    // u128: `file_bytes` is a real file size and `lines_read` is at most
    // PREVIEW_LINES, but their product still overflows u64 on a file above
    // ~37 PiB. Cheap to rule out rather than reason about.
    let estimate = (u128::from(file_bytes) * lines_read as u128).div_ceil(u128::from(bytes_read));
    Some(usize::try_from(estimate).unwrap_or(usize::MAX))
}

/// Whether `path` has a NUL in its head: the check that picks the built-in
/// binary row. A file that cannot be read is not binary; its read says why.
pub(crate) fn is_binary(path: &Path) -> bool {
    document::refuse_unreadable(path)
        .and_then(|()| File::open(path))
        .and_then(|mut file| sniff(&mut file))
        .is_ok_and(|(verdict, _)| verdict == Sniff::Binary)
}
