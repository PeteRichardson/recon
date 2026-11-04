//! A file's lines as one text buffer and the offset where each line starts
//! (#593) — or, for a large file, only the offsets, with the text read
//! again from the file when it is asked for (#51).

use std::borrow::Cow;
use std::fs::File;
use std::io;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// How much an index reads at a time: when it is made, and for a range of
/// lines.
const CHUNK: usize = 1 << 20;

/// The lines of a file, kept as one buffer.
#[derive(Debug, Clone, Default)]
pub struct Lines {
    /// For a large file (#51), the lines at the start, as offsets into the
    /// file. The lines in `text` come after them.
    file: Option<Indexed>,
    text: String,
    starts: Vec<u64>,
}

/// Where each line of a file starts, and the file to read them from.
#[derive(Debug, Clone)]
struct Indexed {
    file: Arc<File>,
    starts: Vec<u64>,
    /// How many bytes a range reads at a time.
    chunk: usize,
    /// The bytes of the file the lines hold.
    end: u64,
    /// Whether a read found the file shorter than `end`, or failed: the
    /// lines are not the file's any more. Shared with each clone, which
    /// reads the same file.
    changed: Arc<AtomicBool>,
    /// The lines around the last one `Lines::line` read: a caller that asks
    /// for one line after another, in either order, reads a chunk at a
    /// time. It gives the text it read until a line outside it is asked
    /// for: only a read from the file finds the file changed.
    near: Near,
}

/// `Indexed::near`. A clone starts with none.
#[derive(Debug, Default)]
struct Near(Mutex<Chunk>);

impl Clone for Near {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Lines {
    /// The lines of `bytes`.
    ///
    /// Decoded lossily: a U+FFFD for each sequence that is not UTF-8, as
    /// `read_lossy_line` decodes one line. Text that is all UTF-8 is not
    /// copied.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_text(
            String::from_utf8(bytes)
                .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned()),
        )
    }

    /// The lines of `text`: one for each `\n`, without its line end, and
    /// no empty line after a last `\n`.
    #[must_use]
    pub fn from_text(mut text: String) -> Self {
        let mut starts = Vec::new();
        if !text.is_empty() {
            starts.push(0);
        }
        starts.extend(
            memchr::memchr_iter(b'\n', text.as_bytes())
                .map(|end| end as u64 + 1)
                .filter(|&start| start < text.len() as u64),
        );
        // A capped read grows its buffer by doubling, and so does `starts`.
        text.shrink_to_fit();
        starts.shrink_to_fit();
        Self {
            file: None,
            text,
            starts,
        }
    }

    /// The lines of `file`, as `from_bytes` splits them, without its text:
    /// only where each line starts (#51). A line is read from the file
    /// each time it is asked for.
    ///
    /// The file's length from its metadata is not used: a file can say a
    /// size it does not give. The index is of the bytes read.
    ///
    /// # Errors
    /// Any read of the file.
    pub fn index(file: File) -> io::Result<Self> {
        Self::index_in_chunks(file, CHUNK)
    }

    /// `index`, reading `chunk` bytes at a time: the tests make a line
    /// cross from one read to the next with a few bytes.
    pub(crate) fn index_in_chunks(file: File, chunk: usize) -> io::Result<Self> {
        let chunk = chunk.max(1);
        let mut buf = vec![0; chunk];
        let mut starts = Vec::new();
        let mut end = 0_u64;
        loop {
            let read = read_up_to_at(&file, &mut buf, end)?;
            if read == 0 {
                break;
            }
            if end == 0 {
                starts.push(0);
            }
            starts.extend(memchr::memchr_iter(b'\n', &buf[..read]).map(|at| end + at as u64 + 1));
            end += read as u64;
        }
        if starts.last() == Some(&end) {
            starts.pop();
        }
        starts.shrink_to_fit();
        Ok(Self {
            file: Some(Indexed {
                file: Arc::new(file),
                starts,
                chunk,
                end,
                changed: Arc::default(),
                near: Near::default(),
            }),
            ..Self::default()
        })
    }

    /// Whether a read found the file changed under its index (#51): it is
    /// shorter than the lines, or it cannot be read. The lines read since
    /// are empty. Always `false` for lines kept in memory.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.file
            .as_ref()
            .is_some_and(|index| index.changed.load(Ordering::Relaxed))
    }

    /// Whether the text is read from the file rather than kept (#51).
    #[cfg(test)]
    pub(crate) fn is_index(&self) -> bool {
        self.file.is_some()
    }

    /// The lines in the index, before the ones kept in memory.
    fn indexed(&self) -> usize {
        self.file.as_ref().map_or(0, |index| index.starts.len())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.indexed() + self.starts.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove the last line, if there is one.
    pub fn pop(&mut self) {
        if let Some(start) = self.starts.pop() {
            self.text.truncate(start as usize);
        } else if let Some(index) = &mut self.file
            && let Some(start) = index.starts.pop()
        {
            index.end = start;
            index.near = Near::default();
        }
    }

    /// Add `lines` after the last line. A line must not end in `\n` or
    /// `\r`: the line end is not part of a line.
    ///
    /// They are kept in memory, also after an index: they are the lines
    /// follow mode read.
    pub fn extend<S: AsRef<str>>(&mut self, lines: impl IntoIterator<Item = S>) {
        for line in lines {
            self.starts.push(self.text.len() as u64);
            self.text.push_str(line.as_ref());
            self.text.push('\n');
        }
    }

    /// Keep only the first `len` lines, and free the rest.
    pub fn truncate(&mut self, len: usize) {
        let indexed = self.indexed();
        if let Some(index) = &mut self.file
            && let Some(&start) = index.starts.get(len)
        {
            index.starts.truncate(len);
            index.starts.shrink_to_fit();
            index.end = start;
            index.near = Near::default();
            self.text = String::new();
            self.starts = Vec::new();
        } else if let Some(kept) = len.checked_sub(indexed)
            && let Some(&start) = self.starts.get(kept)
        {
            self.text.truncate(start as usize);
            self.starts.truncate(kept);
            self.text.shrink_to_fit();
            self.starts.shrink_to_fit();
        }
    }

    /// The bytes of text the lines hold, line ends included.
    #[must_use]
    pub fn text_len(&self) -> usize {
        let indexed = self.file.as_ref().map_or(0, |index| index.end as usize);
        indexed + self.text.len()
    }

    /// The bytes the lines take to keep, for the view cache's limit: the
    /// text and one start for each line, with the room kept for more. An
    /// index keeps no text.
    #[must_use]
    pub fn memory(&self) -> usize {
        let indexed = self
            .file
            .as_ref()
            .map_or(0, |index| index.starts.capacity());
        self.text.capacity() + (indexed + self.starts.capacity()) * size_of::<u64>()
    }

    /// Each line in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Cow<'_, str>> + Clone {
        self.range(0..self.len())
    }

    /// Lines `rows`, in order. An index reads them a chunk at a time, and
    /// gives each line as a copy: `each` lends them.
    pub fn range(
        &self,
        rows: std::ops::Range<usize>,
    ) -> impl ExactSizeIterator<Item = Cow<'_, str>> + Clone {
        Range {
            lines: self,
            rows,
            chunk: Chunk::default(),
        }
    }

    /// Give each line of `rows` to `line`, in order. Unlike `range`, an
    /// index lends each line from the chunk it read, with no copy: what a
    /// filter pass reads every line with.
    pub fn each(&self, rows: std::ops::Range<usize>, mut line: impl FnMut(&str)) {
        let until = rows.end;
        let _ = self.walk(rows, until, |_, text| {
            line(text);
            ControlFlow::Continue(())
        });
    }

    /// Give each of `rows`, which go up, to `line` with its text, as `each`
    /// does: for the visible rows, which have gaps.
    pub fn each_of(
        &self,
        rows: impl IntoIterator<Item = usize>,
        mut line: impl FnMut(usize, &str),
    ) {
        self.try_each_of(rows, |row, text| {
            line(row, text);
            ControlFlow::Continue(())
        });
    }

    /// `each_of`, until `line` says to stop: a write that failed, or one
    /// the user cancelled.
    pub fn try_each_of(
        &self,
        rows: impl IntoIterator<Item = usize>,
        line: impl FnMut(usize, &str) -> ControlFlow<()>,
    ) {
        let _ = self.walk(rows, self.len(), line);
    }

    /// `each` and `each_of`: a chunk at a time, of rows before `until`.
    fn walk(
        &self,
        rows: impl IntoIterator<Item = usize>,
        until: usize,
        mut line: impl FnMut(usize, &str) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let indexed = self.indexed();
        let mut chunk = Chunk::default();
        for row in rows {
            match &self.file {
                Some(index) if row < indexed => {
                    if !chunk.rows.contains(&row) {
                        chunk = index.read(index.from(row, until));
                    }
                    line(row, &chunk.line(index, row))?;
                }
                _ => line(row, self.kept(row - indexed))?,
            }
        }
        ControlFlow::Continue(())
    }

    /// Line `row`, or `None` past the last line.
    #[must_use]
    pub fn get(&self, row: usize) -> Option<Cow<'_, str>> {
        (row < self.len()).then(|| self.line(row))
    }

    /// Line `row`.
    ///
    /// # Panics
    /// Past the last line, as an index into a slice does.
    #[must_use]
    pub fn line(&self, row: usize) -> Cow<'_, str> {
        let indexed = self.indexed();
        match &self.file {
            Some(index) if row < indexed => {
                let mut near = index.near.0.lock().unwrap_or_else(PoisonError::into_inner);
                if !near.rows.contains(&row) {
                    // One line after another reads on from there; a jump
                    // reads the lines on both sides.
                    let rows = if row == near.rows.end {
                        index.from(row, indexed)
                    } else {
                        index.around(row)
                    };
                    *near = index.read(rows);
                }
                Cow::Owned(near.line(index, row).into_owned())
            }
            _ => Cow::Borrowed(self.kept(row - indexed)),
        }
    }

    /// Line `row` of the lines kept in memory.
    fn kept(&self, row: usize) -> &str {
        let start = self.starts[row] as usize;
        let end = self
            .starts
            .get(row + 1)
            .map_or(self.text.len(), |&end| end as usize);
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

impl Indexed {
    /// Where line `row` starts, or the end of the last line.
    fn start(&self, row: usize) -> u64 {
        self.starts.get(row).copied().unwrap_or(self.end)
    }

    /// The rows from `row` that fit in a chunk, and `row` itself however
    /// long it is, up to row `until` at most.
    fn from(&self, row: usize, until: usize) -> std::ops::Range<usize> {
        let until = until.min(self.starts.len());
        let base = self.start(row);
        let size = self.chunk as u64;
        let fits = self.starts[row + 1..until].partition_point(|&start| start - base < size);
        row..row + 1 + fits
    }

    /// The rows near `row`, for `near`: about half a chunk of bytes on each
    /// side, and `row` itself however long it is.
    fn around(&self, row: usize) -> std::ops::Range<usize> {
        let half = (self.chunk / 2) as u64;
        let (from, to) = (self.start(row), self.start(row + 1));
        let first = self.starts[..row].partition_point(|&start| from - start > half);
        let after = self.starts[row + 1..].partition_point(|&start| start - to < half);
        first..row + 1 + after
    }

    /// The bytes of lines `rows`, line ends included. Short, and `changed`
    /// set, when the file no longer holds them.
    fn read(&self, rows: std::ops::Range<usize>) -> Chunk {
        let base = self.start(rows.start);
        let to = self.start(rows.end);
        let mut bytes = vec![0; (to - base) as usize];
        // The file can be shorter and grow again before the read: the
        // length is what says a copytruncate happened.
        let filled = if self.file.metadata().is_ok_and(|meta| meta.len() >= to) {
            read_up_to_at(&self.file, &mut bytes, base).unwrap_or(0)
        } else {
            0
        };
        if filled < bytes.len() {
            self.changed.store(true, Ordering::Relaxed);
            bytes.truncate(filled);
        }
        let body =
            String::from_utf8(bytes).map_or_else(|err| Body::Bytes(err.into_bytes()), Body::Text);
        Chunk { rows, base, body }
    }
}

/// One line's bytes as text: without its line end, decoded lossily as
/// `from_bytes` decodes. An index's chunk that is not all UTF-8 decodes
/// each line with it, and so does `read_lossy_line`.
pub(crate) fn decode(bytes: &[u8]) -> Cow<'_, str> {
    let end = bytes
        .iter()
        .rposition(|&byte| byte != b'\n' && byte != b'\r')
        .map_or(0, |last| last + 1);
    // `from_utf8` first: the lossy decode is several times slower on text
    // that is all UTF-8, which nearly every line of a log is.
    std::str::from_utf8(&bytes[..end])
        .map_or_else(|_| String::from_utf8_lossy(&bytes[..end]), Cow::Borrowed)
}

/// Fill `buf` from `offset`, or as much of it as the file holds, and say
/// how many bytes: `scan::read_up_to` at an offset. The file's position
/// does not move, so that a pass on another thread can read the same file.
fn read_up_to_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let at = offset + filled as u64;
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(file, &mut buf[filled..], at);
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(file, &mut buf[filled..], at);
        match read {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}

/// Some lines of an index, read at once.
#[derive(Debug, Clone, Default)]
struct Chunk {
    rows: std::ops::Range<usize>,
    base: u64,
    body: Body,
}

/// What a chunk read: text when all of it is UTF-8, checked once for the
/// chunk rather than once for each line; otherwise bytes, each line
/// decoded alone.
#[derive(Debug, Clone)]
enum Body {
    Text(String),
    Bytes(Vec<u8>),
}

impl Default for Body {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl Chunk {
    /// Line `row`, which `rows` holds, without its line end. Empty when a
    /// short read lost it.
    fn line(&self, index: &Indexed, row: usize) -> Cow<'_, str> {
        let start = (index.start(row) - self.base) as usize;
        let end = (index.start(row + 1) - self.base) as usize;
        match &self.body {
            Body::Text(text) => Cow::Borrowed(
                text.get(start..end.min(text.len()))
                    .unwrap_or_default()
                    .trim_end_matches(['\n', '\r']),
            ),
            Body::Bytes(bytes) => {
                decode(bytes.get(start..end.min(bytes.len())).unwrap_or_default())
            }
        }
    }
}

/// `Lines::range`: lines kept in memory are borrowed; an index reads a
/// chunk of lines at a time.
struct Range<'a> {
    lines: &'a Lines,
    rows: std::ops::Range<usize>,
    chunk: Chunk,
}

/// A clone starts with no chunk: it reads its own.
impl Clone for Range<'_> {
    fn clone(&self) -> Self {
        Self {
            lines: self.lines,
            rows: self.rows.clone(),
            chunk: Chunk::default(),
        }
    }
}

impl<'a> Iterator for Range<'a> {
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.next()?;
        let lines = self.lines;
        let Some(index) = lines.file.as_ref().filter(|index| row < index.starts.len()) else {
            return Some(Cow::Borrowed(lines.kept(row - lines.indexed())));
        };
        if !self.chunk.rows.contains(&row) {
            self.chunk = index.read(index.from(row, self.rows.end));
        }
        Some(Cow::Owned(self.chunk.line(index, row).into_owned()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}

impl ExactSizeIterator for Range<'_> {}

/// For the tests: the lines compare with a list of lines.
#[cfg(test)]
impl<S: AsRef<str>> PartialEq<[S]> for Lines {
    fn eq(&self, other: &[S]) -> bool {
        self.iter().eq(other.iter().map(AsRef::as_ref))
    }
}

#[cfg(test)]
impl<S: AsRef<str>, const N: usize> PartialEq<[S; N]> for Lines {
    fn eq(&self, other: &[S; N]) -> bool {
        *self == other[..]
    }
}

impl<S: AsRef<str>> From<Vec<S>> for Lines {
    fn from(lines: Vec<S>) -> Self {
        lines.into_iter().collect()
    }
}

impl<S: AsRef<str>> FromIterator<S> for Lines {
    fn from_iter<I: IntoIterator<Item = S>>(lines: I) -> Self {
        let mut all = Self::default();
        all.extend(lines);
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &Lines) -> Vec<Cow<'_, str>> {
        lines.iter().collect()
    }

    #[test]
    fn a_buffer_splits_at_each_line_end_without_the_line_end() {
        let lines = Lines::from_bytes(b"one\ntwo\r\nthree\r\r\n\nlast".to_vec());
        assert_eq!(rows(&lines), ["one", "two", "three", "", "last"]);
    }

    /// Follow mode (#493): the last line had no line end and has grown, so
    /// it goes, and the grown line comes back with the new ones.
    #[test]
    fn a_followed_file_drops_its_last_line_and_adds_lines_after() {
        let mut lines = Lines::from_bytes(b"a\nb".to_vec());
        lines.pop();
        lines.extend(["bc", "", "d"]);
        assert_eq!(rows(&lines), ["a", "bc", "", "d"]);
    }

    /// The lines a view makes itself — a dump, a listing, a message — keep
    /// every line, an empty last one too.
    #[test]
    fn lines_made_one_at_a_time_keep_every_line() {
        let lines = Lines::from(vec![String::new(), "x".to_string(), String::new()]);
        assert_eq!(rows(&lines), ["", "x", ""]);
        assert!(Lines::from(Vec::<String>::new()).is_empty());
    }

    /// The view cache has a byte limit: a line costs its text, its line
    /// end and its start, not a `String` of its own.
    #[test]
    fn the_memory_of_lines_is_their_text_and_their_starts() {
        let lines = Lines::from_bytes(b"abc\nde\n".to_vec());
        assert_eq!(lines.memory(), 7 + 2 * 8);
    }

    const SHAPES: [&[u8]; 10] = [
        b"",
        b"\n",
        b"\n\n",
        b"\r",
        b"no line end",
        b"a\nb\n",
        b"bad \xff byte\nok\n",
        b"cut \xe2\x82\nnext \xe2\x82\xac\n",
        b"\xf0\x9f\x98",
        b"mid\rline\r\n\r\n",
    ];

    fn indexed(name: &str, bytes: &[u8], chunk: usize) -> Lines {
        let path = crate::fixtures::fixture_file(name, bytes);
        Lines::index_in_chunks(std::fs::File::open(path).expect("fixture"), chunk)
            .expect("readable")
    }

    /// A large file keeps only where each line starts (#51), and reads the
    /// text again when it is asked for. It must read the lines the buffer
    /// holds, also where a line crosses from one read to the next.
    #[test]
    fn an_index_gives_the_lines_a_buffer_gives() {
        for (n, bytes) in SHAPES.iter().enumerate() {
            let buffer = Lines::from_bytes(bytes.to_vec());
            for chunk in [1, 3, 1 << 20] {
                let index = indexed(&format!("index_shape_{n}_{chunk}.log"), bytes, chunk);
                assert_eq!(rows(&index), rows(&buffer), "{bytes:?} in reads of {chunk}");
                let mut gaps = Vec::new();
                index.each_of((0..index.len()).step_by(2), |row, line| {
                    gaps.push((row, line.to_string()));
                });
                let every_other: Vec<_> = (0..buffer.len())
                    .step_by(2)
                    .map(|row| (row, buffer.line(row).into_owned()))
                    .collect();
                assert_eq!(
                    gaps, every_other,
                    "{bytes:?} every other in reads of {chunk}"
                );
                let mut lent = Vec::new();
                index.each(0..index.len(), |line| lent.push(line.to_string()));
                assert_eq!(lent, rows(&buffer), "{bytes:?} lent in reads of {chunk}");
                for row in (0..=buffer.len()).chain((0..buffer.len()).rev()) {
                    assert_eq!(index.get(row), buffer.get(row), "{bytes:?} row {row}");
                }
            }
        }
    }

    /// Follow mode on a large file: the last line goes, and the grown line
    /// and the new ones are kept in memory after the index.
    #[test]
    fn a_followed_index_drops_its_last_line_and_adds_lines_after() {
        let mut lines = indexed("index_follow.log", b"a\nb", 1 << 20);
        lines.pop();
        lines.extend(["bc", "", "d"]);
        assert_eq!(rows(&lines), ["a", "bc", "", "d"]);
        lines.pop();
        lines.pop();
        lines.pop();
        lines.pop();
        assert!(lines.is_empty());
        lines.extend(["z"]);
        assert_eq!(rows(&lines), ["z"]);
    }

    #[test]
    fn a_truncated_index_keeps_its_first_lines() {
        let mut lines = indexed("index_truncate.log", b"a\nb\nc\n", 1 << 20);
        lines.extend(["d"]);
        lines.truncate(4);
        assert_eq!(rows(&lines), ["a", "b", "c", "d"]);
        lines.truncate(2);
        assert_eq!(rows(&lines), ["a", "b"]);
        lines.extend(["e"]);
        assert_eq!(rows(&lines), ["a", "b", "e"]);
    }

    /// The point of an index (#51): it keeps a start for each line, and no
    /// text.
    #[test]
    fn an_index_keeps_no_text() {
        let lines = indexed(
            "index_memory.log",
            &b"some line of a log\n".repeat(1000),
            64,
        );
        assert_eq!(lines.len(), 1000);
        assert_eq!(lines.memory(), 1000 * size_of::<u64>());
    }

    /// logrotate's copytruncate: the file under the index is shorter. The
    /// lines it lost are empty, not a panic, and the lines say they
    /// changed, so that the view reads the file again.
    #[test]
    fn an_index_whose_file_became_shorter_says_it_changed() {
        let path = crate::fixtures::fixture_file("index_shorter.log", b"one\ntwo\nthree\n");
        let lines = Lines::index(std::fs::File::open(&path).expect("fixture")).expect("readable");
        assert_eq!(lines.get(1).as_deref(), Some("two"));
        assert!(!lines.changed());

        std::fs::write(&path, b"one\n").expect("truncate");

        assert_eq!(rows(&lines), ["", "", ""], "one read of all three is short");
        assert!(lines.changed());
        assert!(lines.clone().changed(), "a clone reads the same file");
    }

    /// Lines in memory never change under recon.
    #[test]
    fn lines_in_memory_never_say_they_changed() {
        assert!(!Lines::from_bytes(b"a\n".to_vec()).changed());
    }

    /// Batch mode streams a file a line at a time with `read_lossy_line`.
    /// The two readers must agree, or `--emit lines` writes other lines
    /// than the TUI shows.
    #[test]
    fn a_buffer_gives_the_lines_the_stream_reader_gives() {
        for bytes in SHAPES {
            let mut reader = std::io::Cursor::new(bytes);
            let mut buf = Vec::new();
            let mut streamed = Vec::new();
            while let Some(line) = crate::document::read_lossy_line(&mut reader, &mut buf).unwrap()
            {
                streamed.push(line);
            }
            assert_eq!(
                rows(&Lines::from_bytes(bytes.to_vec())),
                streamed,
                "{bytes:?}"
            );
        }
    }
}
