//! CSV alignment for the table view (#575): one aligned line for each CSV
//! record, so a column can be read down the rows.
//!
//! The method is `xsv table`'s and `tabwriter`'s: pad each cell with spaces
//! to the width of its column. The output is plain text — no borders, no
//! wrapped cells, one line per record — so filters, search, yank and `:w`
//! work on it as they do on any other view.
//!
//! Parsing is the `csv` crate, not a split on `,`: a quoted field can hold
//! a comma, and a quoted field can hold a new line, which makes one record
//! more than one line of the file. In the aligned view it is one line.

use unicode_width::UnicodeWidthStr;

/// How many records give the column widths.
///
/// `tabwriter` reads all rows before it knows the widths, which the 10 MiB
/// cap makes cheap but spooled output (#573) would not. Sampled widths also
/// fix a smaller problem first: a preview and a full read of one file
/// compute the same widths whenever the file's first `SAMPLE_RECORDS`
/// records are both in view, so columns do not shift under the cursor when
/// a preview is read again whole.
pub(crate) const SAMPLE_RECORDS: usize = 1_000;

/// At least this many spaces between two columns, as in `xsv table`.
const MIN_GAP: usize = 2;

/// The most columns a column reserves. A cell longer than the cap goes
/// past its column rather than being cut: a cut cell hides data from the
/// filters. The cap is only for the *padding*, so one enormous cell — an
/// array written into one field, say — misaligns its own row instead of
/// pushing every column after it off screen for every row.
const MAX_COLUMN_WIDTH: usize = 64;

/// `text` as aligned lines: one per CSV record, each cell padded to the
/// width of its column.
///
/// Widths come from the first `sample` records. A cell wider than its
/// column pushes the columns after it along that row only; `sample`
/// widths are capped at [`MAX_COLUMN_WIDTH`].
///
/// Blank lines are not records and do not appear: that is what the `csv`
/// grammar says a blank line is. A record whose fields are ragged keeps
/// its own length; the columns it lacks simply get no cell.
pub(crate) fn align(text: &str, delimiter: u8, sample: usize) -> Vec<String> {
    let records: Vec<csv::StringRecord> = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        // Every row is data: the header of a CSV is a row like any other,
        // and must land in the view where the filters can see it.
        .has_headers(false)
        .from_reader(text.as_bytes())
        .records()
        .filter_map(Result::ok)
        .collect();
    let columns = records
        .iter()
        .map(csv::StringRecord::len)
        .max()
        .unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for record in records.iter().take(sample.max(1)) {
        for (column, field) in record.iter().enumerate() {
            let width = UnicodeWidthStr::width(field).min(MAX_COLUMN_WIDTH);
            widths[column] = widths[column].max(width);
        }
    }
    records
        .iter()
        .map(|record| {
            let last = record.len().saturating_sub(1);
            let mut line = String::new();
            for (column, field) in record.iter().enumerate() {
                // A field's own new lines are content a line-based view
                // cannot show: a record is one line here, so they become
                // spaces.
                let field = field.replace(['\n', '\r'], " ");
                line.push_str(&field);
                if column != last {
                    let pad = widths
                        .get(column)
                        .copied()
                        .unwrap_or(0)
                        .saturating_sub(UnicodeWidthStr::width(field.as_str()))
                        .max(MIN_GAP);
                    line.push_str(&" ".repeat(pad));
                }
            }
            line
        })
        .collect()
}

/// The delimiter for a file: a tab for `.tsv` and `.tab`, a comma for
/// every other name.
pub(crate) fn delimiter_for_path(path: &std::path::Path) -> u8 {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("tsv" | "tab") => b'\t',
        _ => b',',
    }
}

/// The delimiter for a command's declared `output` type: a tab for
/// `text/tab-separated-values`, a comma for anything else, which is what
/// `text/csv` — and an aligned view of an untyped output — wants.
pub(crate) fn delimiter_for_mime(mime: Option<&str>) -> u8 {
    match mime.map(str::trim) {
        Some("text/tab-separated-values") => b'\t',
        _ => b',',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `align` at the real sample size.
    fn aligned(text: &str, delimiter: u8) -> Vec<String> {
        align(text, delimiter, SAMPLE_RECORDS)
    }

    #[test]
    fn each_cell_is_padded_to_the_width_of_its_column() {
        let lines = aligned("name,qty\nwidget,3\nabacus,120\n", b',');
        assert_eq!(
            lines,
            ["name  qty", "widget  3", "abacus  120"],
            "{lines:?}"
        );
    }

    #[test]
    fn the_header_needs_no_special_case() {
        // Column widths follow the cells, and the header is a cell like
        // any other: it is what `transmcap` writes first in practice.
        let lines = aligned("t,topic\n1698000000,some/long/topic\n1,x\n", b',');
        assert_eq!(lines[0], "t         topic");
        assert_eq!(lines[1], "1698000000  some/long/topic");
    }

    #[test]
    fn a_comma_in_a_quoted_field_stays_in_its_cell() {
        let lines = aligned("a,b\n\"x,y\",2\n", b',');
        assert_eq!(lines, ["a  b", "x,y  2"]);
    }

    #[test]
    fn a_record_spanning_lines_becomes_one_line() {
        let lines = aligned("a,b\n\"multi\nline\",2\n", b',');
        assert_eq!(lines, ["a         b", "multi line  2"]);
    }

    #[test]
    fn blank_lines_are_not_records() {
        let lines = aligned("a,b\n\nc,d\n", b',');
        assert_eq!(lines, ["a  b", "c  d"]);
    }

    #[test]
    fn a_wide_cell_goes_past_its_column_rather_than_cutting() {
        // Past the cap the cell keeps its whole text and the next column
        // starts at the gap after it: the row misaligns, the data does
        // not disappear from the filters.
        let long = "L".repeat(MAX_COLUMN_WIDTH + 30);
        let lines = aligned(&format!("a,b\n{long},2\n"), b',');
        assert_eq!(lines[1], format!("{long}  2"));
    }

    #[test]
    fn one_huge_cell_does_not_widen_the_column_for_other_rows() {
        // The cap is for the padding only: short rows keep their columns
        // at the capped width whatever the huge cell holds. The huge
        // row's second column sits past the cap; every other row's does
        // not move.
        let long = "L".repeat(MAX_COLUMN_WIDTH + 30);
        let lines = aligned(&format!("a,b\n{long},2\nshort,x\ntiny,y\n"), b',');
        assert_eq!(
            lines[2],
            format!("short{}x", " ".repeat(MAX_COLUMN_WIDTH - 5))
        );
        assert_eq!(
            lines[3],
            format!("tiny{}y", " ".repeat(MAX_COLUMN_WIDTH - 4))
        );
    }

    #[test]
    fn widths_come_from_the_sample_and_later_rows_do_not_shift_them() {
        let text = format!("a,b\n{},2\nlater_is_much_longer,z\n", "x".repeat(10));
        let lines = align(&text, b',', 2);
        // The sample is the header and the `x` row: width 10 for
        // column 0. The row past the sample goes past the column.
        assert_eq!(lines[0], "a         b");
        assert_eq!(lines[1], "xxxxxxxxxx  2");
        assert_eq!(lines[2], "later_is_much_longer  z");
    }

    #[test]
    fn ragged_rows_keep_their_own_length() {
        let lines = aligned("a,b,c\n1\n2,3\n", b',');
        assert_eq!(lines, ["a  b  c", "1", "2  3"]);
    }

    #[test]
    fn padding_counts_terminal_columns_not_chars() {
        // A CJK cell is two columns a character; padding by `char` count
        // would push the next column right per glyph (#97).
        let lines = aligned("na,b\n\u{540d}\u{524d},2\nx,y\n", b',');
        assert_eq!(lines[1], "\u{540d}\u{524d}  2");
        assert_eq!(lines[2], "x   y");
    }

    #[test]
    fn a_tab_separated_file_aligns_on_tabs() {
        let lines = aligned("a\tb\nlong\tx\n", b'\t');
        assert_eq!(lines, ["a   b", "long  x"]);
    }

    #[test]
    fn empty_text_aligns_to_nothing() {
        assert_eq!(aligned("", b','), Vec::<String>::new());
    }

    #[test]
    fn tsv_is_a_tab_and_everything_else_a_comma() {
        use std::path::Path;
        assert_eq!(delimiter_for_path(Path::new("a.tsv")), b'\t');
        assert_eq!(delimiter_for_path(Path::new("A.TAB")), b'\t');
        assert_eq!(delimiter_for_path(Path::new("a.csv")), b',');
        assert_eq!(delimiter_for_path(Path::new("noext")), b',');
        assert_eq!(delimiter_for_mime(Some("text/tab-separated-values")), b'\t');
        assert_eq!(delimiter_for_mime(Some("text/csv")), b',');
        assert_eq!(delimiter_for_mime(None), b',');
    }

    #[test]
    fn one_column_needs_no_padding() {
        let lines = aligned("a\nbb\nccc\n", b',');
        assert_eq!(lines, ["a", "bb", "ccc"]);
    }
}
