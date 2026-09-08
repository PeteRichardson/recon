//! The loaded file and what the filters made of it.

use crate::filter::{ActiveFilters, Verdict};
use crate::syntax::{self, KindSet};
use ratatui::style::Style;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::path::Path;
use std::sync::Arc;

/// Which lines the file view shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Every line that no excluding filter removed; unmatched lines are dimmed.
    #[default]
    Dimmed,
    /// Only lines an including filter selected.
    FilteredOnly,
}

/// A loaded file, with a cached verdict per line.
///
/// Evaluating a filter set is not free on a large log, so verdicts are computed
/// when the lines or the filters change rather than once per frame.
///
/// There is deliberately no cached `match_count`. One used to sit here,
/// documented as read by the status line every frame — it was not: the status
/// row reports lines *shown*, counted from `visible`, and the two comments in
/// `lib.rs` that mention `match_count` both say why it is the wrong number
/// (it counts `Included` and `Searched` verdicts, so it read "0 matched" with
/// only excluding filters active). Nothing outside this file ever called the
/// getter (#77).
#[derive(Debug, Default)]
pub struct Document {
    /// Shared with the `FileView` that read them (#122): the view keeps the
    /// file's lines to colour them lazily, and sharing is what stops that
    /// being a third copy of every large file.
    lines: Arc<Vec<String>>,
    verdicts: Vec<Verdict>,
    /// Whether anything was marking lines at the last `evaluate` — a numbered
    /// including filter, or the live search.
    ///
    /// Cached rather than asked of the `ActiveFilters` inside
    /// `recompute_visible`, so that method keeps taking no arguments and stays
    /// independent of the filter set. That independence is what makes the
    /// `Ctrl-H` path cheap: the toggle re-derives `visible` from the verdicts
    /// alone, with no borrow and no regex.
    anything_including: bool,
    mode: Mode,
    visible: Vec<usize>,
    /// The buffer `recompute_visible` builds the next visible set into, kept
    /// so the rebuild is allocation-free after the first: it is compared
    /// against `visible` and swapped in only if it differs.
    scratch: Vec<usize>,
    /// Moves exactly when `visible` changes. The viewport's rebuild-skip key
    /// (#159): comparing two `u64`s per `apply_view` — every arrow key —
    /// instead of two index vectors the length of the file, and no copy of
    /// that vector on every rebuild. `sync_document` replaces the whole
    /// document, so a fresh one starting at 0 is never mistaken for the
    /// previous one at 0: the viewport clears its key alongside.
    generation: u64,
    /// Where `lines` came from, for the grammar lookup a definition filter
    /// needs (#123). `None` for a document with no file behind it.
    path: Option<std::path::PathBuf>,
    /// The definition kinds each line starts, computed by the first
    /// `evaluate` that has a definition filter to answer and kept for the
    /// document's life. `None` until then, and `Some(empty)` for a file no
    /// grammar claims — that answer is worth caching too.
    kinds: Option<Vec<KindSet>>,
}

impl Document {
    #[must_use]
    pub fn new(lines: impl Into<Arc<Vec<String>>>) -> Self {
        let lines = lines.into();
        let verdicts = vec![Verdict::Unmatched; lines.len()];
        Self {
            lines,
            verdicts,
            anything_including: false,
            mode: Mode::default(),
            visible: Vec::new(),
            scratch: Vec::new(),
            generation: 0,
            path: None,
            kinds: None,
        }
    }

    /// A document over `lines` read from `path`, so that a definition filter
    /// can ask which grammar to parse them with.
    #[must_use]
    pub fn for_file(path: &Path, lines: impl Into<Arc<Vec<String>>>) -> Self {
        Self {
            path: Some(path.to_path_buf()),
            ..Self::new(lines)
        }
    }

    /// A document over the whole of `path`, read the way the file view reads
    /// a file (#143): the same NUL sniff, the same lossy decoding, the same
    /// line-end stripping — and the error instead of a placeholder message,
    /// which is what headless mode needs and the widget wraps.
    pub fn read(path: &Path) -> io::Result<Self> {
        Ok(Self::for_file(path, read_lines(path)?))
    }

    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    #[must_use]
    pub fn verdicts(&self) -> &[Verdict] {
        &self.verdicts
    }

    /// Recompute every line's verdict. Call when the lines or the filters change.
    ///
    /// The verdicts are overwritten in place (#185): `clear` keeps the
    /// allocation, so a 1M-line file no longer pays a 16 MB allocation and
    /// free on every toggle. The vector is the length of `lines` before and
    /// after, so nothing that indexes it by row sees a size change.
    pub fn evaluate(&mut self, filters: &ActiveFilters) {
        // The whole-file grammar pass, once, and only when a filter will
        // read its answer. A set of regex filters never pays for it.
        if filters.needs_kinds() && self.kinds.is_none() {
            self.kinds = Some(
                self.path
                    .as_deref()
                    .and_then(|path| syntax::definitions(path, &self.lines))
                    .unwrap_or_default(),
            );
        }
        let kinds = self.kinds.as_deref().unwrap_or(&[]);
        self.verdicts.clear();
        self.verdicts
            .extend(self.lines.iter().enumerate().map(|(row, line)| {
                filters.verdict(line, kinds.get(row).copied().unwrap_or(KindSet::EMPTY))
            }));
        self.anything_including = filters.any_including();
        self.recompute_visible();
    }

    /// The rebuild-skip key: unchanged for as long as `visible` is, moved
    /// every time `recompute_visible` (and so `evaluate`) changes it.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Recompute `visible` from the existing verdicts and the current mode,
    /// without re-running the filters.
    ///
    /// A verdict depends on the lines and the filters; `visible` depends only
    /// on the verdicts and the mode. So toggling the mode (`H` / `Ctrl-H`)
    /// only needs this, not a full `evaluate` — which matters, since
    /// `evaluate` is O(lines × filters) and this is O(lines).
    ///
    /// Built into `scratch` and swapped in only when it differs from the
    /// current set, so `generation` moves exactly when the rows on screen
    /// do. A filter change that leaves the same rows visible — an including
    /// filter swapped for another in dimmed mode, a search with no hits —
    /// therefore keeps the viewport's buffer, cursor column and scroll
    /// exactly where they were, which is what the old whole-vector compare
    /// in `apply_view` bought and what the `u64` key keeps (#159). The
    /// compare is O(visible), paid once per filter change here rather than
    /// once per arrow key there, and nothing is allocated or copied.
    pub fn recompute_visible(&mut self) {
        let mode = self.mode;
        let anything_including = self.anything_including;
        self.scratch.clear();
        self.scratch.extend(
            self.verdicts
                .iter()
                .enumerate()
                .filter(|(_, verdict)| match (mode, verdict) {
                    // Excluded lines are gone in both modes; the toggle governs
                    // unmatched lines only.
                    (_, Verdict::Excluded) => false,
                    (Mode::Dimmed, _) => true,
                    // A context line stays in hide mode: that is what the sense
                    // is for. Only `n` treats it differently from an include.
                    (
                        Mode::FilteredOnly,
                        Verdict::Included(_) | Verdict::Context(_) | Verdict::Searched,
                    ) => true,
                    // Issue #36: with nothing including, there is nothing to hide
                    // *against*, so hiding shows the file rather than blanking the
                    // pane. Dimming has always had this guard in `style_for`;
                    // hiding never did, which made `Ctrl-H` with no filters — and
                    // with only excluding filters — produce an empty view that read
                    // as "this file is empty".
                    (Mode::FilteredOnly, Verdict::Unmatched) => !anything_including,
                })
                .map(|(index, _)| index),
        );
        if self.scratch != self.visible {
            std::mem::swap(&mut self.scratch, &mut self.visible);
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// One style slot per line, for `FileView::set_line_styles`.
    ///
    /// Always covers every line, so a shorter vector can never leave trailing
    /// lines wearing styles computed for a previously loaded file.
    #[cfg(test)]
    #[must_use]
    pub fn line_styles(&self, filters: &ActiveFilters) -> Vec<Option<Style>> {
        self.verdicts
            .iter()
            .map(|verdict| filters.style_for(*verdict))
            .collect()
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Change which lines are shown. The caller must re-`evaluate` (or, if
    /// the verdicts have not changed, just `recompute_visible`) afterwards.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Source line indices currently on screen, in order.
    #[must_use]
    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// The text of the visible lines, for rebuilding the view's buffer.
    #[cfg(test)]
    #[must_use]
    pub fn visible_lines(&self) -> Vec<String> {
        self.visible_lines_range(0, self.visible.len())
    }

    /// The text of visible rows `start..end`, for rebuilding the view's buffer
    /// from a window rather than the whole visible set.
    ///
    /// The range is clamped rather than panicking on a stale bound: the window
    /// is computed from a pane height captured on the previous frame, and a
    /// filter change between frames can shorten the visible set under it. A
    /// short buffer for one frame is a cosmetic glitch; an index panic takes
    /// the whole TUI down.
    #[must_use]
    pub fn visible_lines_range(&self, start: usize, end: usize) -> Vec<String> {
        self.window(start, end)
            .iter()
            .map(|&source| self.lines[source].clone())
            .collect()
    }

    /// One style slot per *visible* line, aligned with `visible_lines`.
    #[cfg(test)]
    #[must_use]
    pub fn visible_styles(&self, filters: &ActiveFilters) -> Vec<Option<Style>> {
        self.visible_styles_range(filters, 0, self.visible.len())
    }

    /// One style slot per row of visible `start..end`, aligned with
    /// `visible_lines_range` over the same bounds.
    ///
    /// Windowing this matters more than it looks: unlike the gutter numbers,
    /// the style vector was never gated on hiding, so an unfiltered million-line
    /// file rebuilt a million-entry vector on every navigator arrow key.
    #[must_use]
    pub fn visible_styles_range(
        &self,
        filters: &ActiveFilters,
        start: usize,
        end: usize,
    ) -> Vec<Option<Style>> {
        self.window(start, end)
            .iter()
            .map(|&source| filters.style_for(self.verdicts[source]))
            .collect()
    }

    /// `visible[start..end]`, with both bounds clamped to the visible set.
    fn window(&self, start: usize, end: usize) -> &[usize] {
        let end = end.min(self.visible.len());
        let start = start.min(end);
        &self.visible[start..end]
    }

    /// One flag per *visible* line, aligned with `visible_lines`: whether the
    /// source line after it is hidden.
    ///
    /// Hiding unmatched lines collapses a file into groups of consecutive
    /// matches with nothing on screen to say how much was skipped between
    /// them, so ten matched lines either side of a hundred hidden ones read as
    /// twenty consecutive lines (issue #2). A set flag is where the view draws
    /// the boundary.
    ///
    /// "The next source line is hidden" rather than "another group follows",
    /// so a group running into trailing hidden lines is marked like any other
    /// — the file really does continue below it. The last line of the file has
    /// no next line and is never marked, which is what keeps an unfiltered
    /// document unmarked throughout.
    #[cfg(test)]
    #[must_use]
    pub fn visible_group_ends(&self) -> Vec<bool> {
        self.visible_group_ends_range(0, self.visible.len())
    }

    /// `visible_group_ends` over visible rows `start..end` only.
    ///
    /// **Peeks one row past `end`**, which is the whole reason this is not a
    /// slice of the full vector. A row's mark asks "is the next source line
    /// hidden?", and for the final *visible* row it means "does the file
    /// continue below?". Sliced naively, the window's last row would be
    /// mistaken for the document's last row and marked wrong — a gap marker
    /// appearing or vanishing purely because of where the window happens to
    /// stop. Reading `visible[end]` when it exists keeps every mark
    /// independent of the window.
    #[must_use]
    pub fn visible_group_ends_range(&self, start: usize, end: usize) -> Vec<bool> {
        let end = end.min(self.visible.len());
        let start = start.min(end);
        (start..end)
            .map(|row| {
                let source = self.visible[row];
                match self.visible.get(row + 1) {
                    Some(&next) => next != source + 1,
                    // Nothing visible after it: a gap only if the file continues.
                    None => source + 1 < self.lines.len(),
                }
            })
            .collect()
    }

    /// Where a source line sits in the visible list, if it is shown at all.
    #[must_use]
    pub fn visible_position(&self, source: usize) -> Option<usize> {
        self.visible.binary_search(&source).ok()
    }

    /// The source index of the visible row at `visible_row`.
    #[must_use]
    pub fn source_at(&self, visible_row: usize) -> Option<usize> {
        self.visible.get(visible_row).copied()
    }

    /// The nearest visible source line at or after `source`, falling back to
    /// the last one before it.
    ///
    /// Used when a mode change hides the line the cursor was on: snapping
    /// forward lands on the match the user was navigating towards, and the
    /// backward fallback stops the cursor being lost when nothing follows.
    #[must_use]
    pub fn nearest_visible(&self, source: usize) -> Option<usize> {
        match self.visible.binary_search(&source) {
            Ok(_) => Some(source),
            Err(index) => self
                .visible
                .get(index)
                .copied()
                .or_else(|| self.visible.last().copied()),
        }
    }
}

/// How much of a file's head is examined for a NUL before it is read as text.
pub(crate) const BINARY_SNIFF_BYTES: usize = 8 << 10;

/// The message on the `InvalidData` error `read_lines` returns for a file
/// whose head holds a NUL. `is_binary` recognises it; the file view turns it
/// into its own `<binary file>` message and headless mode prints it as is.
pub(crate) const BINARY_FILE: &str = "binary file";

/// The message for a FIFO, socket or device named as a file (#221).
pub(crate) const NOT_A_FILE: &str = "not a regular file";

/// Refuse, before opening, what an open would get wrong: a directory opens
/// fine on macOS and only the read fails `EISDIR`, and a FIFO's open blocks
/// until a writer appears — for ever, in a scanner thread or a cron job.
/// One `stat`, following symlinks, so a link to a file is a file. A path
/// that cannot be stat'd is left for `File::open` to report in its own
/// words: not found, permission denied.
pub(crate) fn refuse_unreadable(path: &Path) -> io::Result<()> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(());
    };
    let file_type = meta.file_type();
    if file_type.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "is a directory",
        ));
    }
    if !file_type.is_file() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, NOT_A_FILE));
    }
    Ok(())
}

/// Whether `err` is `read_lines`' own binary-file refusal rather than an OS
/// error.
#[must_use]
pub(crate) fn is_binary(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::InvalidData && err.to_string() == BINARY_FILE
}

/// The byte order a UTF-16 byte-order mark announced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Endian {
    Little,
    Big,
}

/// What the sniff made of a file's head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sniff {
    /// No NUL in the head: read as lines, decoded lossily from UTF-8.
    Text,
    /// A UTF-16 byte-order mark (#165). Half the bytes are NULs and every
    /// one of them is text; the file is decoded whole in the marked order.
    Utf16(Endian),
    /// A NUL in the head and nothing to explain it.
    Binary,
}

/// Whether the head of `reader` looks like binary rather than text, along with
/// the bytes that had to be read to decide — they are the file's first bytes
/// and belong back in front of the stream.
///
/// A NUL byte is the signal, not a decode error. A decode error says one byte
/// in the file is not UTF-8, which is routine in a log; a NUL in the first few
/// KiB says the file is not a document at all.
///
/// The one exception is UTF-16, which is text made of NULs. A byte-order mark
/// — `FF FE` or `FE FF` — is the only signal short of statistics that the
/// NULs are encoding, not content, so it is the only one read: UTF-16 without
/// a mark is still reported binary, as the README says (#165).
pub(crate) fn sniff<R: Read>(reader: &mut R) -> io::Result<(Sniff, Vec<u8>)> {
    let mut head = Vec::new();
    (&mut *reader)
        .take(BINARY_SNIFF_BYTES as u64)
        .read_to_end(&mut head)?;
    let verdict = match head.first_chunk::<2>() {
        Some([0xff, 0xfe]) => Sniff::Utf16(Endian::Little),
        Some([0xfe, 0xff]) => Sniff::Utf16(Endian::Big),
        _ if head.contains(&0) => Sniff::Binary,
        _ => Sniff::Text,
    };
    Ok((verdict, head))
}

/// The lines of a UTF-16 file whose head `sniff` read into `head` and whose
/// remainder is still in `reader`, decoded in `endian`'s order.
///
/// Whole, not streamed: a line ends at a two-byte `0A 00` (or `00 0A`), and a
/// byte-at-a-time `read_until` would cut it in half, so the file is decoded
/// first and split after. The byte-order mark is dropped, an unpaired
/// surrogate becomes U+FFFD as an undecodable UTF-8 sequence would, and an
/// odd byte at the end — a file cut mid-unit by a preview's byte cap, or
/// simply damaged — is one more U+FFFD rather than an error.
pub(crate) fn read_utf16_lines<R: Read>(
    head: Vec<u8>,
    reader: &mut R,
    endian: Endian,
) -> io::Result<Vec<String>> {
    let mut bytes = head;
    reader.read_to_end(&mut bytes)?;
    let mut units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            let pair = [pair[0], pair[1]];
            match endian {
                Endian::Little => u16::from_le_bytes(pair),
                Endian::Big => u16::from_be_bytes(pair),
            }
        })
        .collect();
    if bytes.len() % 2 == 1 {
        units.push(0xfffd);
    }
    let text = String::from_utf16_lossy(&units);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    Ok(lines_of(text))
}

/// `text` split as `read_lossy_line` would have read it: one line per `\n`,
/// line ends stripped, and no phantom empty line after a final newline.
fn lines_of(text: &str) -> Vec<String> {
    text.split_inclusive('\n')
        .map(|line| line.trim_end_matches(['\n', '\r']).to_string())
        .collect()
}

/// Read one newline-terminated line, decoded lossily. `None` at end of file.
///
/// Lossy, not fatal: one bad byte in a two-gigabyte log must not cost the
/// other two gigabytes. U+FFFD marks the spot in place and the read carries
/// on, which is the whole difference from `lines()` — that short-circuits the
/// entire file on its first undecodable byte.
pub(crate) fn read_lossy_line<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> io::Result<Option<String>> {
    buf.clear();
    if reader.read_until(b'\n', buf)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(buf)
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    ))
}

/// Read `path` whole, as lines.
///
/// `File::open` succeeds on a directory on Unix and only fails when read, so
/// that case is refused up front with `IsADirectory`. A file whose head holds
/// a NUL is refused as [`BINARY_FILE`] — unless a byte-order mark says the
/// NULs are UTF-16, in which case it is decoded as such; one that merely
/// holds undecodable bytes is read anyway, a U+FFFD per bad sequence.
/// Anything else the OS refuses comes back verbatim.
pub fn read_lines(path: &Path) -> io::Result<Vec<String>> {
    refuse_unreadable(path)?;
    let mut reader = BufReader::new(File::open(path)?);
    let head = match sniff(&mut reader)? {
        (Sniff::Text, head) => head,
        (Sniff::Utf16(endian), head) => return read_utf16_lines(head, &mut reader, endian),
        (Sniff::Binary, _) => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, BINARY_FILE));
        }
    };
    // The sniffed bytes are content, so they go back in front of the rest.
    let mut reader = Cursor::new(head).chain(reader);
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    while let Some(line) = read_lossy_line(&mut reader, &mut buf)? {
        lines.push(line);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{fixture_dir, fixture_file};

    // ---- definition filters (#123) -----------------------------------------

    /// A document that knows its file answers a definition filter from the
    /// grammar; lines that start no definition are unmatched whatever they
    /// say, and the pass runs only once.
    #[test]
    fn evaluating_a_definition_filter_reads_the_grammar() {
        let mut filters = ActiveFilters::new();
        filters.add_definition(crate::syntax::Kind::Function);
        let lines = vec![
            "// fn not_a_definition".to_string(),
            "fn real() {}".to_string(),
            "struct S;".to_string(),
        ];
        let mut document = Document::for_file(Path::new("t.rs"), lines);
        document.evaluate(&filters);
        assert_eq!(
            document.verdicts(),
            [Verdict::Unmatched, Verdict::Included(0), Verdict::Unmatched]
        );
        assert!(document.kinds.is_some(), "the pass ran and was kept");
    }

    /// No file behind the lines, or a file no grammar claims: a definition
    /// filter matches nothing, and a regex filter is unaffected either way.
    #[test]
    fn without_a_grammar_a_definition_filter_matches_nothing() {
        let mut filters = ActiveFilters::new();
        filters.add_definition(crate::syntax::Kind::Function);
        filters.add("real").expect("valid");
        let lines = vec!["fn real() {}".to_string()];
        for mut document in [
            Document::new(lines.clone()),
            Document::for_file(Path::new("app.log"), lines.clone()),
        ] {
            document.evaluate(&filters);
            assert_eq!(document.verdicts(), [Verdict::Included(1)]);
        }
    }

    /// Regex-only sets never pay for the grammar pass.
    #[test]
    fn a_regex_only_set_does_not_run_the_grammar_pass() {
        let filters = {
            let mut set = ActiveFilters::new();
            set.add("fn").expect("valid");
            set
        };
        let mut document = Document::for_file(Path::new("t.rs"), vec!["fn x() {}".to_string()]);
        document.evaluate(&filters);
        assert!(document.kinds.is_none());
    }
    use ratatui::style::Modifier;

    fn doc(lines: &[&str]) -> Document {
        Document::new(
            lines
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>(),
        )
    }

    fn set_with(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    fn set_searching(pattern: &str) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        set.set_search(pattern).expect("valid pattern");
        set
    }

    #[test]
    fn a_new_document_has_a_verdict_for_every_line() {
        let document = doc(&["one", "two", "three"]);

        assert_eq!(document.verdicts().len(), document.lines().len());
    }

    #[test]
    fn evaluating_records_each_line_s_verdict() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);

        document.evaluate(&filters);

        assert_eq!(
            document.verdicts(),
            &[Verdict::Unmatched, Verdict::Included(0), Verdict::Unmatched]
        );
    }

    #[test]
    fn re_evaluating_replaces_the_previous_verdicts() {
        let mut document = doc(&["alpha", "beta"]);
        document.evaluate(&set_with(&["beta"]));

        document.evaluate(&set_with(&["alpha"]));

        assert_eq!(
            document.verdicts(),
            &[Verdict::Included(0), Verdict::Unmatched]
        );
    }

    /// A 1M-line file paid a 16 MB allocation and free on every toggle;
    /// the verdicts are overwritten in place now, so the second `evaluate`
    /// reuses the first one's buffer (#185).
    #[test]
    fn re_evaluating_reuses_the_verdict_allocation() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.evaluate(&set_with(&["beta"]));
        let before = document.verdicts().as_ptr();

        document.evaluate(&set_with(&["alpha"]));

        assert_eq!(
            document.verdicts().as_ptr(),
            before,
            "evaluate allocated a fresh Vec<Verdict>"
        );
        assert_eq!(document.verdicts().len(), 3);
    }

    /// The viewport keys its rebuild-skip on `generation`, so it must move
    /// exactly when the visible set does (#159): a filter change that leaves
    /// the same rows on screen must not disturb the view, and one that
    /// changes them must rebuild.
    #[test]
    fn the_generation_advances_only_when_the_visible_set_changes() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let start = document.generation();

        document.evaluate(&set_with(&["beta"]));
        let after_first = document.generation();
        assert_ne!(after_first, start, "the first evaluate fills `visible`");

        // Dimmed mode shows every line an excluding filter did not remove,
        // so a different including filter leaves the same rows on screen.
        document.evaluate(&set_with(&["gamma"]));
        assert_eq!(
            document.generation(),
            after_first,
            "same rows, same generation — the view must not be rebuilt"
        );

        document.set_mode(Mode::FilteredOnly);
        document.recompute_visible();
        assert_ne!(
            document.generation(),
            after_first,
            "hiding changed the rows"
        );
        assert_eq!(document.visible(), &[2]);
    }

    /// `set_mode` alone changes nothing on screen until `recompute_visible`
    /// runs, so it moves nothing here either.
    #[test]
    fn set_mode_alone_leaves_the_generation() {
        let mut document = doc(&["alpha"]);
        document.evaluate(&set_with(&["alpha"]));
        let generation = document.generation();
        document.set_mode(Mode::FilteredOnly);
        assert_eq!(document.generation(), generation);
    }

    // ---- windowed accessors (#7) ---------------------------------------

    /// Everything visible, which is what `visible_lines_range` and friends are
    /// windowing over. `Document::new` leaves `visible` empty until something
    /// evaluates, so an unfiltered pass is the "no filters yet" baseline.
    fn shown(lines: &[&str]) -> Document {
        let mut document = doc(lines);
        document.evaluate(&ActiveFilters::new());
        document
    }

    #[test]
    fn visible_lines_range_returns_only_the_window() {
        let document = shown(&["a", "b", "c", "d", "e"]);

        assert_eq!(
            document.visible_lines_range(1, 4),
            vec!["b".to_string(), "c".to_string(), "d".to_string()]
        );
    }

    /// The window is computed from a pane height captured on the previous
    /// frame, so a filter change can shorten the visible set under it. Clamping
    /// costs a short buffer for one frame; panicking takes the TUI down.
    #[test]
    fn visible_lines_range_clamps_a_stale_end() {
        let document = shown(&["a", "b"]);

        assert_eq!(document.visible_lines_range(1, 999), vec!["b".to_string()]);
    }

    #[test]
    fn visible_lines_range_clamps_a_start_past_the_end() {
        let document = shown(&["a", "b"]);

        assert!(document.visible_lines_range(9, 999).is_empty());
    }

    #[test]
    fn visible_styles_range_lines_up_with_its_window() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let windowed = document.visible_styles_range(&filters, 1, 3);

        assert_eq!(windowed.len(), 2);
        assert_eq!(windowed, document.visible_styles(&filters)[1..3].to_vec());
    }

    /// The regression this range method exists to prevent. Row 1 is followed by
    /// a *hidden* line, so it is a group end — and must stay one when the
    /// window stops right after it. Slicing a whole-set vector would give the
    /// same answer; computing the range in isolation, without peeking at
    /// `visible[end]`, would treat row 1 as the document's last row and ask the
    /// wrong question.
    #[test]
    fn visible_group_ends_range_peeks_past_the_window() {
        let mut document = doc(&["keep", "keep", "drop", "keep"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["keep"]));

        // Visible rows are sources 0, 1, 3 — source 2 is hidden.
        assert_eq!(document.visible(), &[0, 1, 3]);

        let windowed = document.visible_group_ends_range(0, 2);

        assert_eq!(
            windowed,
            vec![false, true],
            "row 1 is followed by a hidden line and must be marked a group end \
             even though the window stops there"
        );
        assert_eq!(windowed, document.visible_group_ends()[0..2].to_vec());
    }

    #[test]
    fn visible_group_ends_range_clamps_a_stale_end() {
        let document = shown(&["a", "b"]);

        assert_eq!(document.visible_group_ends_range(0, 999).len(), 2);
    }

    /// The vector handed to the textarea has one entry per line, so no line is
    /// left to fall through to whatever the previous file's styles were.
    #[test]
    fn line_styles_covers_every_line() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_eq!(styles.len(), 2);
    }

    #[test]
    fn matching_lines_take_their_filter_s_colour_and_others_dim() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_eq!(
            styles[1].expect("beta styled").fg,
            filters.filters()[0].style.fg
        );
        assert!(
            styles[0]
                .expect("alpha styled")
                .add_modifier
                .contains(Modifier::DIM),
            "unmatched line not dimmed"
        );
    }

    /// Without filters nothing is dimmed, so an ordinary file looks ordinary.
    #[test]
    fn an_unfiltered_document_styles_nothing() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = ActiveFilters::new();
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert!(styles.iter().all(Option::is_none));
    }

    #[test]
    fn two_filters_colour_their_lines_differently() {
        let mut document = doc(&["alpha", "beta"]);
        let filters = set_with(&["alpha", "beta"]);
        document.evaluate(&filters);

        let styles = document.line_styles(&filters);

        assert_ne!(styles[0].unwrap().fg, styles[1].unwrap().fg);
    }

    fn set_excluding(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    #[test]
    fn dimmed_mode_shows_every_line_that_is_not_excluded() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[0, 1, 2]);
    }

    /// Excluded lines are gone in both modes — the toggle governs unmatched
    /// lines only.
    #[test]
    fn excluded_lines_are_hidden_even_when_dimmed() {
        let mut document = doc(&["alpha", "noise", "gamma"]);
        document.evaluate(&set_excluding(&["noise"]));

        assert_eq!(document.mode(), Mode::Dimmed);
        assert_eq!(document.visible(), &[0, 2]);
    }

    /// Issue #36. With nothing including, there is nothing to hide against, so
    /// hiding shows the file rather than blanking the pane. Dimming has always
    /// had this guard (`style_for`); hiding never did.
    #[test]
    fn hiding_with_no_filters_shows_the_whole_file() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&ActiveFilters::new());

        assert_eq!(document.visible(), &[0, 1, 2]);
    }

    /// The same bug through a second door, unreported until #36 was investigated:
    /// with only excluding filters there is nothing to hide unmatched lines
    /// *against* — the user wants the file minus the noise, not an empty pane.
    #[test]
    fn hiding_with_only_excluding_filters_shows_the_rest_of_the_file() {
        let mut document = doc(&["alpha", "noise here", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_excluding(&["noise"]));

        assert_eq!(document.visible(), &[0, 2]);
    }

    /// A bare search counts as something to hide against, which is what makes
    /// `/foo` followed by `Ctrl-H` an instant grep.
    #[test]
    fn hiding_with_only_a_search_collapses_to_its_matches() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_searching("beta"));

        assert_eq!(document.visible(), &[1]);
    }

    /// The guard must not soften a real filter set: a file with no hits still
    /// renders blank, which is exactly what the directory-skim feature needs
    /// "blank" to mean.
    #[test]
    fn a_file_with_no_hits_is_still_blank_when_hiding() {
        let mut document = doc(&["alpha", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["ERROR"]));

        assert!(document.visible().is_empty());
    }

    /// The guard is cached at `evaluate` time precisely so that the mode toggle
    /// stays O(lines) and runs no regex — see `recompute_visible`.
    #[test]
    fn the_guard_survives_a_mode_toggle_without_re_evaluating() {
        let mut document = doc(&["alpha", "beta"]);
        document.evaluate(&ActiveFilters::new());

        document.set_mode(Mode::FilteredOnly);
        document.recompute_visible();

        assert_eq!(document.visible(), &[0, 1]);
    }

    #[test]
    fn filtered_only_mode_shows_matches_alone() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[1, 3]);
    }

    #[test]
    fn visible_lines_are_the_text_of_the_visible_indices() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_lines(), vec!["beta".to_string()]);
    }

    #[test]
    fn visible_styles_line_up_with_visible_lines() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);

        assert_eq!(
            document.visible_styles(&filters).len(),
            document.visible().len()
        );
    }

    /// Issue #2. Hiding collapses a file into groups of consecutive matches
    /// separated by invisible gaps; this is what marks where a group stops.
    #[test]
    fn a_group_ends_where_the_next_source_line_is_hidden() {
        let mut document = doc(&["beta", "beta", "alpha", "beta", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible(), &[0, 1, 3, 4]);
        assert_eq!(
            document.visible_group_ends(),
            vec![false, true, false, false]
        );
    }

    /// The mark means "the next source line is not shown", so a group running
    /// to the end of the file has nothing after it to mark.
    #[test]
    fn the_last_line_of_the_file_never_ends_a_group() {
        let mut document = doc(&["alpha", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_group_ends(), vec![false]);
    }

    /// Trailing hidden lines are a gap like any other — the group really does
    /// stop there, and the rest of the file is below it.
    #[test]
    fn a_group_ends_where_the_rest_of_the_file_is_hidden() {
        let mut document = doc(&["beta", "alpha"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_group_ends(), vec![true]);
    }

    #[test]
    fn nothing_ends_a_group_when_every_line_is_visible() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_group_ends(), vec![false; 3]);
    }

    #[test]
    fn group_ends_line_up_with_visible_lines() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(
            document.visible_group_ends().len(),
            document.visible().len()
        );
    }

    #[test]
    fn source_and_visible_positions_map_both_ways() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.visible_position(3), Some(1));
        assert_eq!(document.source_at(1), Some(3));
        assert_eq!(document.visible_position(0), None, "line 0 is hidden");
    }

    /// Toggling into filtered mode from a hidden line snaps forward to the
    /// next match, which is what the user was navigating towards.
    #[test]
    fn nearest_visible_snaps_forward() {
        let mut document = doc(&["alpha", "beta", "gamma", "beta again"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.nearest_visible(0), Some(1));
        assert_eq!(document.nearest_visible(2), Some(3));
    }

    /// With no match after it, fall back to the one before rather than losing
    /// the cursor entirely.
    #[test]
    fn nearest_visible_falls_back_to_the_previous_match() {
        let mut document = doc(&["beta", "alpha", "gamma"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["beta"]));

        assert_eq!(document.nearest_visible(2), Some(0));
    }

    #[test]
    fn nearest_visible_is_none_when_nothing_is_visible() {
        let mut document = doc(&["alpha", "beta"]);
        document.set_mode(Mode::FilteredOnly);
        document.evaluate(&set_with(&["zzz"]));

        assert!(document.visible().is_empty());
        assert_eq!(document.nearest_visible(0), None);
    }

    /// `set_mode` records the mode but does not recompute anything: `visible`
    /// catches up on the next `evaluate`. The task that toggles modes relies on
    /// that ordering, because it captures the cursor's source line against the
    /// *old* mapping before rebuilding.
    #[test]
    fn set_mode_alone_does_not_change_what_is_visible() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);
        assert_eq!(document.visible(), &[0, 1, 2]);

        document.set_mode(Mode::FilteredOnly);

        assert_eq!(
            document.mode(),
            Mode::FilteredOnly,
            "the mode was not recorded"
        );
        assert_eq!(
            document.visible(),
            &[0, 1, 2],
            "visible changed before evaluate was called"
        );

        document.evaluate(&filters);
        assert_eq!(document.visible(), &[1], "visible did not catch up");
    }

    /// The point of splitting `recompute_visible` out of `evaluate`: the mode
    /// toggle can refresh `visible` alone, without redoing the verdict pass
    /// (the expensive part on a large document).
    #[test]
    fn recompute_visible_updates_visible_without_rerunning_the_filters() {
        let mut document = doc(&["alpha", "beta", "gamma"]);
        let filters = set_with(&["beta"]);
        document.evaluate(&filters);
        let verdicts_before = document.verdicts().to_vec();

        document.set_mode(Mode::FilteredOnly);
        document.recompute_visible();

        assert_eq!(
            document.visible(),
            &[1],
            "visible did not pick up the new mode"
        );
        assert_eq!(
            document.verdicts(),
            verdicts_before.as_slice(),
            "recompute_visible must not touch the verdicts"
        );
    }

    // ---- reading (#143) -----------------------------------------------------

    #[test]
    fn read_lines_strips_line_ends_and_decodes_lossily() {
        let file = fixture_file("document_read_lossy.log", b"one\r\ntwo\xff\nthree");

        let lines = read_lines(&file).expect("readable");

        assert_eq!(lines, ["one", "two\u{FFFD}", "three"]);
    }

    #[test]
    fn read_builds_a_document_over_the_file() {
        let file = fixture_file("document_read_document.log", b"a\nb\n");

        let document = Document::read(&file).expect("readable");

        assert_eq!(document.lines(), ["a", "b"]);
        assert_eq!(document.verdicts().len(), 2, "one verdict slot per line");
    }

    #[test]
    fn read_lines_refuses_a_binary_file() {
        let file = fixture_file("document_read_binary.bin", b"abc\0def\n");

        let err = read_lines(&file).expect_err("a NUL in the head is binary");

        assert!(is_binary(&err), "not the binary error: {err}");
        assert_eq!(err.to_string(), BINARY_FILE);
    }

    /// UTF-16 text is half NUL bytes, and the sniff used to call every such
    /// file binary (#165). A byte-order mark says what the NULs are: the
    /// file is decoded, the mark dropped, and the lines come out as they
    /// would from the same text in UTF-8.
    #[test]
    fn read_lines_decodes_utf16_with_a_byte_order_mark() {
        let le = fixture_file(
            "document_read_utf16le.txt",
            &utf16(Endian::Little, "hi\nthere\r\n"),
        );
        let be = fixture_file(
            "document_read_utf16be.txt",
            &utf16(Endian::Big, "hi\nthere\r\n"),
        );

        assert_eq!(read_lines(&le).expect("UTF-16 LE is text"), ["hi", "there"]);
        assert_eq!(read_lines(&be).expect("UTF-16 BE is text"), ["hi", "there"]);
    }

    /// The same lenience as UTF-8: an unpaired surrogate and a stray odd byte
    /// at the end each become U+FFFD, and the rest of the file survives.
    #[test]
    fn read_lines_decodes_damaged_utf16_lossily() {
        let mut bytes = utf16(Endian::Little, "ok\n");
        bytes.extend_from_slice(&[0x00, 0xd8]); // a lone high surrogate
        bytes.extend_from_slice(&utf16_body(Endian::Little, "x\n"));
        bytes.push(0x41); // an odd trailing byte
        let file = fixture_file("document_read_utf16_damaged.txt", &bytes);

        assert_eq!(
            read_lines(&file).expect("damage is lossy, not fatal"),
            ["ok", "\u{fffd}x", "\u{fffd}"]
        );
    }

    /// Without a byte-order mark the NULs are still the verdict: the rule the
    /// README documents is unchanged, only its exception is new.
    #[test]
    fn read_lines_still_refuses_utf16_without_a_byte_order_mark() {
        let file = fixture_file(
            "document_read_utf16_no_bom.txt",
            &utf16_body(Endian::Little, "hi\n"),
        );

        let err = read_lines(&file).expect_err("no mark, so the NULs are binary");

        assert!(is_binary(&err), "not the binary error: {err}");
    }

    /// `text` in UTF-16 with `endian`'s byte order, byte-order mark first.
    fn utf16(endian: Endian, text: &str) -> Vec<u8> {
        let mut bytes = match endian {
            Endian::Little => vec![0xff, 0xfe],
            Endian::Big => vec![0xfe, 0xff],
        };
        bytes.extend(utf16_body(endian, text));
        bytes
    }

    /// `text` in UTF-16 with `endian`'s byte order and no mark.
    fn utf16_body(endian: Endian, text: &str) -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|unit| match endian {
                Endian::Little => unit.to_le_bytes(),
                Endian::Big => unit.to_be_bytes(),
            })
            .collect()
    }

    #[test]
    fn read_lines_refuses_a_directory_up_front() {
        let dir = fixture_dir("document_read_dir");

        let err = read_lines(&dir).expect_err("a directory is not a file");

        assert_eq!(err.kind(), io::ErrorKind::IsADirectory);
        assert_eq!(err.to_string(), "is a directory");
        assert!(!is_binary(&err));
    }

    /// A FIFO blocks `File::open` until a writer appears and a socket cannot
    /// be opened as a file at all; neither has an end to read to. Refused by
    /// a stat before the open, so the caller never blocks (#221).
    #[cfg(unix)]
    #[test]
    fn read_lines_refuses_a_non_regular_file_before_opening_it() {
        let dir = fixture_dir("document_read_socket");
        let sock = dir.join("sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind");

        let err = read_lines(&sock).expect_err("a socket is not a file");

        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert_eq!(err.to_string(), NOT_A_FILE);
        assert!(!is_binary(&err));
    }

    #[test]
    fn read_lines_reports_a_missing_file_as_not_found() {
        let err =
            read_lines(Path::new("target/document_read_no_such_file.log")).expect_err("missing");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
