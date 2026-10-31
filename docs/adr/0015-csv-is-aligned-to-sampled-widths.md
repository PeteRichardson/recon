# ADR 0015: CSV is aligned to sampled widths

**Status:** accepted
**Date:** 2026-10-31

## Context

recon showed CSV as raw text: the fields of each row start at different
positions, so a column cannot be read down the rows. The motivating case is
a command view running `transmcap` with CSV output on an MCAP recording;
`output = "text/csv"` already selects the CSV grammar, so the fields get
colours, but nothing lined the columns up (#575).

Splitting on `,` is not CSV: a quoted field can hold a comma, and a quoted
field can hold a new line, which makes one record several lines of the
file. And whatever the view produces must stay readable by everything the
file view already does with lines — filters, `/`, yank, `:w`.

## Decision

- **Parse with the `csv` crate, align by padding.** Every cell is padded
  with spaces to the width of its column — `xsv table`'s and
  `tabwriter`'s method — and one record is one line. No borders and no
  wrapped cells: `comfy-table` and `tabled` were rejected because their
  wrapped output breaks filters, search and yank. A new line inside a
  quoted field becomes a space, so the record still occupies one line;
  blank lines are not records and do not show. `tabwriter` itself is not
  a dependency: it sizes columns per flush, and the view needs widths that
  do not move (below), which is a dozen lines of padding code in
  `src/csv_fmt.rs`.
- **Opt-in, two doors.** A built-in `table` view that a `[[filetype]]`
  row can put in the cycle (`views = ["table", "raw"]` for
  `*.{csv,tsv}`), and an `align = true` key for a command view beside
  `output = "text/csv"`. With no config, recon shows CSV exactly as it
  did before. `.tsv` and `.tab` align on tabs; every other name, and any
  command output that is not `text/tab-separated-values`, on commas.
- **Widths from the first 1,000 records.** Reading the whole file first
  would be affordable at today's 10 MiB read cap, but not with spooled
  output (#573), and it has a smaller defect today: a preview and a full
  read of one file would compute different widths and the columns would
  shift under the cursor when a preview is read again whole. Sampled
  widths are the same for both whenever the record sample is in both.
  The sample size is a constant, not a config key, until a real file
  shows it wants to be one.
- **Cells go past; they are never cut; the padding is capped.** A cell
  wider than its column pushes the columns after it along that row,
  because a cut cell hides data from the filters. To keep one enormous
  cell — an array written into one field — from pushing every column off
  screen, the *width* a column reserves is capped at 64 columns; the cap
  is padding-only and never truncates text.
- **The gutter counts the aligned view.** A record that spans several
  lines of the file is one line here, so the line number beside it — and
  the `{line}` a hand-off sends — is the position in the aligned view,
  not the line in the file. `raw` remains the view whose numbers are the
  file's own; a CSV row's cycle keeps `raw` beside `table` for that.
- **The table is the file's text, so raw's rules hold.** A binary file
  asked for as a table falls back to hex as raw does, and the preview
  caps are raw's caps, because the view is raw's read plus a reformat.

## Consequences

The aligned view is lossy in two documented ways — embedded newlines
become spaces, and blank lines are not records — so it is a view, not a
replacement for raw, and `table` on a non-CSV file simply shows each line
as one cell. Filters see the aligned line, which is the standing rule for
views (ADR 0006) applied to CSV.
