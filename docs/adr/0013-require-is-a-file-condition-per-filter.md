# ADR 0013: `require` is a fourth sense, a condition on the whole file

**Status:** accepted
**Date:** 2026-10-26

## Context

Include filters find the interesting logs, for example the logs with one
error signature. The next question is often about the whole log: "of these
logs, which ran on host `Zeus`?". Context filters show the host line, but they
do not change which files the explorer marks, so the user steps through each
marked log and skips the wrong ones by hand (#547).

Each sense works on one line, and this condition is on the file. An include
filter for `^Host: Zeus` uses OR, so it adds files. An advanced filter's
lookahead and the old global `&` (#39) both need the two matches on the same
line, and the error line does not contain the host.

## Decision

- **A fourth sense, `require`.** A file is marked only when each enabled
  require filter matches a line of it that no exclude filter removes. Include
  filters still decide which files are interesting; require filters only
  remove files from that result. With no include filter enabled, the require
  filters alone decide, so "every log from `Zeus`" works.
- **In the view, a require filter is a context filter.** Its lines show in
  its colour, they are not a stop for `n`, and hide mode keeps them.
- **The scan keeps its bitsets.** A require filter has its own mask beside
  "selects" and "exclude". A file's answer comes from the set of distinct
  bitsets its lines had, so a toggle still reads nothing again. The scan no
  longer stops at the first include line: it stops when the answer is yes,
  or at the end of the file. A file that passes stops as soon as it has a
  line for each condition; a file that fails reads to its end, as a file with
  no include line always did.
- **Batch mode removes the files that fail and does not change the lines of
  the files that pass.** `--emit lines` keeps a file's context lines when it
  has no include line, as before, so there only the require half of the rule
  applies. A require filter on stdin makes the run read stdin to its end
  first, as a definition filter does.
- **`m` stays an include ↔ context toggle** and leaves a require filter
  alone. One press too many must not hide files. The filter editor and
  `filters.toml` set the sense; the pane row shows `req`.
- **A require filter with a definition predicate does not mark files.** The
  scan reads text and never parses, which is already true for an include
  definition filter.

## Considered options

- **A global AND mode again.** Rejected by ADR 0008, and it is a line
  condition, not a file condition.
- **A flag on a context filter** ("this context filter also gates the
  file"). Rejected: sense is already the per-filter choice of what a match
  means, and a fourth value lets the compiler find each `match` that must
  know about it.
- **`m` cycles include → context → require.** Rejected: a press too many
  hides files without a clear sign. A quick way to make a require filter from
  any filter, with an editable extra pattern, is #548.

## Consequences

- This does not conflict with ADR 0008. That ADR removed an AND across the
  filters of one line; this is a condition per filter, at the file level.
- A require filter costs I/O for the files that fail it: they are read to
  their end. The per-filter cache keeps a toggle free.
- A later "reject" sense (the file must have no such line) fits the same
  mask model.
