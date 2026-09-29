//! A file as a hex dump: `<offset>: <hex bytes>  <ASCII>`, sixteen bytes a
//! line (#242).
//!
//! The dump is formatted into ordinary lines at read time, which is the whole
//! reason it is cheap: everything downstream — filters, search, hide mode,
//! the windowed viewport, visual mode and `y`, `--emit lines` — already works
//! on lines and needs no change. The cost of that is that a filter matches the
//! formatted line, hex digits and all, and cannot see a match that straddles
//! two rows. The README says so rather than pretending otherwise.
//!
//! The offset is in the line, not in the gutter: the vendored textarea numbers
//! its gutter in decimal, and an offset in the text can be searched and yanked.

use crate::document;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// Bytes on one line of the dump.
pub(crate) const BYTES_PER_LINE: usize = 16;

/// Most of a file a full hex load reads.
///
/// A dump line is about 75 characters for 16 bytes of file, so with each
/// line's `String` overhead the view costs roughly six times the file's own
/// size. 16 MiB of file is a million lines and about 100 MB resident, which is
/// already far past what anyone reads by hand; past it the dump ends with a
/// line that says how much was left out, rather than a core dump costing
/// gigabytes.
pub(crate) const MAX_HEX_BYTES: u64 = 16 << 20;

/// What `read` found.
pub(crate) struct Dump {
    /// The dump's lines, one per sixteen bytes read.
    pub(crate) lines: Vec<String>,
    /// Bytes past `max_bytes` that were not read, when the file has any. The
    /// file's size less what was read, or at least one when the size could
    /// not be found.
    pub(crate) unread: Option<u64>,
}

/// One line of the dump: `offset`, then `chunk` as hex, then as ASCII.
///
/// A short last chunk is padded in the hex column, so its ASCII column lines
/// up with the rows above. An extra space after the eighth byte splits the
/// row into halves, as `hexdump -C` does, which is what makes a byte's column
/// countable at a glance. Only printable ASCII shows as itself; every other
/// byte is a `.`, so the column is always exactly one character per byte.
pub(crate) fn line(offset: u64, chunk: &[u8]) -> String {
    // By hand rather than with `write!`: a preview formats 50,000 of these
    // on every explorer arrow, and the formatting machinery made that four
    // times the cost of a text preview of the same length.
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    debug_assert!(chunk.len() <= BYTES_PER_LINE);
    let mut line = Vec::with_capacity(80);
    let width = (u64::BITS - offset.leading_zeros()).div_ceil(4).max(8);
    for shift in (0..width).rev() {
        line.push(DIGITS[((offset >> (shift * 4)) & 0xf) as usize]);
    }
    line.push(b':');
    for index in 0..BYTES_PER_LINE {
        if index == BYTES_PER_LINE / 2 {
            line.push(b' ');
        }
        match chunk.get(index) {
            Some(&byte) => {
                line.extend_from_slice(&[
                    b' ',
                    DIGITS[usize::from(byte >> 4)],
                    DIGITS[usize::from(byte & 0xf)],
                ]);
            }
            None => line.extend_from_slice(b"   "),
        }
    }
    line.extend_from_slice(b"  ");
    line.extend(chunk.iter().map(|&byte| {
        if byte.is_ascii_graphic() || byte == b' ' {
            byte
        } else {
            b'.'
        }
    }));
    String::from_utf8(line).expect("a dump line is ASCII")
}

/// `bytes` as dump lines, the first at offset zero.
pub(crate) fn lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(BYTES_PER_LINE)
        .zip((0u64..).step_by(BYTES_PER_LINE))
        .map(|(chunk, offset)| line(offset, chunk))
        .collect()
}

/// How many dump lines a file of `len` bytes has. Exact, not estimated: a
/// dump's line count is a function of the size alone.
pub(crate) fn line_count(len: u64) -> usize {
    usize::try_from(len.div_ceil(BYTES_PER_LINE as u64)).unwrap_or(usize::MAX)
}

/// The line that ends a dump cut short by `MAX_HEX_BYTES`.
pub(crate) fn stop_line(unread: u64) -> String {
    format!(
        "<hex view stops at {} MiB: {unread} more bytes not shown>",
        MAX_HEX_BYTES >> 20
    )
}

/// Read at most `max_bytes` of `path` as a dump.
///
/// No binary refusal — the bytes are the point — and no decoding either: a
/// UTF-16 file shows its raw bytes, byte-order mark and all. The same stat
/// guard as a text read comes first, so a directory or a FIFO is refused
/// before it is opened.
pub(crate) fn read(path: &Path, max_bytes: u64) -> io::Result<Dump> {
    document::refuse_unreadable(path)?;
    let file = File::open(path)?;
    let size = file.metadata().ok().map(|meta| meta.len());
    let mut bytes = Vec::new();
    // One byte past the cap, to learn whether there is more without trusting
    // a size that a growing log has already outrun.
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let unread = if bytes.len() as u64 > max_bytes {
        bytes.truncate(usize::try_from(max_bytes).unwrap_or(usize::MAX));
        Some(size.map_or(1, |size| size.saturating_sub(max_bytes).max(1)))
    } else {
        None
    };
    Ok(Dump {
        lines: lines(&bytes),
        unread,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture_file;

    #[test]
    fn a_full_line_has_the_offset_the_hex_and_the_ascii() {
        let bytes: Vec<u8> = (0x41..0x51).collect();

        assert_eq!(
            line(0x10, &bytes),
            "00000010: 41 42 43 44 45 46 47 48  49 4a 4b 4c 4d 4e 4f 50  ABCDEFGHIJKLMNOP"
        );
    }

    /// The ASCII column of a short last line starts in the same column as
    /// the full lines above it.
    #[test]
    fn a_short_line_pads_the_hex_so_the_ascii_lines_up() {
        let full = line(0, &[b'a'; BYTES_PER_LINE]);
        let short = line(16, b"hi");

        assert_eq!(short, format!("00000010: 68 69{}  hi", " ".repeat(43)));
        assert_eq!(
            full.find("aaaa"),
            short.find("hi"),
            "the ASCII columns must line up"
        );
    }

    #[test]
    fn only_printable_ascii_shows_as_itself() {
        let dump = line(0, b"\0\x07\t\n ~\x7f\xff");

        assert!(dump.ends_with("  .... ~.."), "{dump}");
    }

    /// Eight digits, and more only when the offset needs them.
    #[test]
    fn the_offset_widens_past_eight_digits() {
        assert!(line(0x1_0000_0000, b"x").starts_with("100000000: 78"));
        assert!(line(0xabc, b"x").starts_with("00000abc: 78"));
    }

    #[test]
    fn lines_count_their_offsets_up_in_sixteens() {
        let bytes = vec![0u8; 40];

        let dump = lines(&bytes);

        let offsets: Vec<&str> = dump.iter().map(|line| &line[..9]).collect();
        assert_eq!(offsets, ["00000000:", "00000010:", "00000020:"]);
        assert_eq!(line_count(40), 3);
        assert_eq!(line_count(48), 3);
        assert_eq!(line_count(0), 0);
    }

    #[test]
    fn read_shows_a_binary_file() {
        let file = fixture_file("hex_read_binary.bin", b"ab\0cd");

        let dump = read(&file, MAX_HEX_BYTES).expect("readable");

        assert_eq!(dump.unread, None);
        assert_eq!(dump.lines, [line(0, b"ab\0cd")]);
    }

    /// Hex shows the bytes, not the decoded text: a UTF-16 file keeps its
    /// byte-order mark and its NULs.
    #[test]
    fn read_shows_utf16_as_its_raw_bytes() {
        let file = fixture_file("hex_read_utf16.txt", b"\xff\xfeh\0i\0");

        let dump = read(&file, MAX_HEX_BYTES).expect("readable");

        assert!(dump.lines[0].starts_with("00000000: ff fe 68 00 69 00"));
    }

    #[test]
    fn read_stops_at_the_cap_and_counts_what_is_left() {
        let file = fixture_file("hex_read_capped.txt", &[b'x'; 40]);

        let dump = read(&file, 32).expect("readable");

        assert_eq!(dump.lines.len(), 2);
        assert_eq!(dump.unread, Some(8));
    }

    #[test]
    fn read_at_exactly_the_cap_leaves_nothing_unread() {
        let file = fixture_file("hex_read_exact.txt", &[b'x'; 32]);

        let dump = read(&file, 32).expect("readable");

        assert_eq!(dump.unread, None);
    }

    #[test]
    fn read_refuses_a_directory() {
        let dir = crate::fixtures::fixture_dir("hex_read_dir");

        let err = read(&dir, MAX_HEX_BYTES)
            .err()
            .expect("a directory is refused");

        assert_eq!(err.kind(), io::ErrorKind::IsADirectory);
    }
}
