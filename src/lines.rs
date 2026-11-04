//! A file's lines as one text buffer and the offset where each line starts
//! (#593).

/// The lines of a file, kept as one buffer.
#[derive(Debug, Clone, Default)]
pub struct Lines {
    text: String,
    starts: Vec<u64>,
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
        Self { text, starts }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.starts.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove the last line, if there is one.
    pub fn pop(&mut self) {
        if let Some(start) = self.starts.pop() {
            self.text.truncate(start as usize);
        }
    }

    /// Add `lines` after the last line. A line must not end in `\n` or
    /// `\r`: the line end is not part of a line.
    pub fn extend<S: AsRef<str>>(&mut self, lines: impl IntoIterator<Item = S>) {
        for line in lines {
            self.starts.push(self.text.len() as u64);
            self.text.push_str(line.as_ref());
            self.text.push('\n');
        }
    }

    /// Keep only the first `len` lines, and free the rest.
    pub fn truncate(&mut self, len: usize) {
        if let Some(&start) = self.starts.get(len) {
            self.text.truncate(start as usize);
            self.starts.truncate(len);
            self.text.shrink_to_fit();
            self.starts.shrink_to_fit();
        }
    }

    /// The bytes of text the lines hold, line ends included.
    #[must_use]
    pub fn text_len(&self) -> usize {
        self.text.len()
    }

    /// The bytes the lines take to keep, for the view cache's limit: the
    /// text and one start for each line, with the room kept for more.
    #[must_use]
    pub fn memory(&self) -> usize {
        self.text.capacity() + self.starts.capacity() * size_of::<u64>()
    }

    /// Each line in order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator + Clone {
        self.range(0..self.len())
    }

    /// Lines `rows`, in order.
    pub fn range(
        &self,
        rows: std::ops::Range<usize>,
    ) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator + Clone {
        rows.map(|row| &self[row])
    }

    /// Line `row`, or `None` past the last line.
    #[must_use]
    pub fn get(&self, row: usize) -> Option<&str> {
        (row < self.len()).then(|| &self[row])
    }

    #[must_use]
    pub fn first(&self) -> Option<&str> {
        self.get(0)
    }
}

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

impl std::ops::Index<usize> for Lines {
    type Output = str;

    fn index(&self, row: usize) -> &str {
        let start = self.starts[row] as usize;
        let end = self
            .starts
            .get(row + 1)
            .map_or(self.text.len(), |&end| end as usize);
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &Lines) -> Vec<&str> {
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

    /// Batch mode streams a file a line at a time with `read_lossy_line`.
    /// The two readers must agree, or `--emit lines` writes other lines
    /// than the TUI shows.
    #[test]
    fn a_buffer_gives_the_lines_the_stream_reader_gives() {
        let inputs: [&[u8]; 10] = [
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
        for bytes in inputs {
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
