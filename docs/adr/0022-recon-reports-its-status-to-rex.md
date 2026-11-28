# ADR 0022: recon reports its status to Rex

**Status:** accepted
**Date:** 2026-11-27

## Context

Rex shows the status of the programs in its panes in an Activity popup.
A program sends its status with OSC 7501, the Rex Program Status
Protocol: `ESC ] 7501 ; state=idle:app=recon:msg=<base64> ESC \`. Each
report replaces the record of the program. With no status from recon, a
pane that has recon in it shows nothing about the file or the filters
(#638, goal #642).

recon draws on stderr, or on `/dev/tty` when stderr is not a terminal.
stdout is for `--emit`, so a report on stdout would go into a pipe.

## Decision

- **recon sends `idle` reports in v1,** with `app=recon`. The message is
  the name of the file on screen, the filter count, and the shown and
  total lines: `app.log · 3 filters · 1204/88k lines`. With no filters it
  is `app.log · 88k lines`. A directory is `name/`.
- **A count past four digits is short,** rounded down: `12k`, `1.2M`. A
  previewed total is the estimate, with `~`, as on the status bar.
- **recon sends `working` while it has busy work (#639),** and `idle`
  again when the work stops. Each piece of work has its message, with
  the values that the status bar gives:

  | Work | Message |
  |---|---|
  | `q` that waits for work to end | `finishing` |
  | A view that is late | `loading app.log` |
  | A filter pass | `filtering app.log`, with `progress` |
  | A command view that still runs | `running gen · 52k lines` |
  | stdin that is open | `reading stdin · 52k lines` |
  | An explorer scan | `scanning 412 files` |

  Only a filter pass knows how far it has got. Its `progress` is in whole
  percent, so a pass of any length sends at most about 100 reports. The
  counts are not in its message, as the shown count goes up with each
  block.
- **With more than one piece of busy work, the first in the table is
  reported.** One report holds one piece of work. The order puts the work
  that blocks the user first: `finishing` takes every key but the cancel,
  and a late view is not on screen yet. An explorer scan runs in the
  background, so it is last. A file with no answer is work only while
  a scan is out for it: with no filter that selects, or before the first
  scan starts, nothing reads it.
- **stdin that is open is `working` until it closes,** as more lines can
  come. The name in an `idle` message is `<stdin>`, as in the title.
- **Busy work that took 2 s or more ends with `done` (#640),** with the
  result of the last work in the busy period:

  | Work | Result |
  |---|---|
  | A late view | `loaded app.log: 88k lines` |
  | A filter pass | `filtered app.log: 1204 of 88k lines` |
  | A command view | `ran gen: 52k lines` |
  | stdin | `read stdin: 52k lines` |
  | An explorer scan | `scanned 412 files` |

  `done` stays until the next key, then the report is `idle`. recon
  cannot tell if its tab is on screen, so it reports `done` after each
  long period, and Rex decides what to show. The threshold is
  `LONG_WORK`, 2 s: shorter work is over before the user looks away, and
  a marker for each move in the explorer would say nothing. `finishing`
  has no `done`, as recon exits when it ends.
- **A report goes after a frame, and only when the status changed.** The
  report goes to the screen, after the draw has flushed the frame, so it
  cannot land inside one. Scrolling changes nothing that the status says,
  so it sends nothing.
- **`[terminal] status` in `config.toml` is `auto`, `on` or `off`.** `auto`
  is the default, and sends reports only when `TERM_PROGRAM` is `rex`.
  Another terminal can show the bytes of an OSC that it does not know.
  `on` is for a Rex pane that does not say it is Rex, as inside tmux.
  There is no flag.
- **recon clears its record as it gives the terminal back,** in
  `restore_terminal`, so on `q`, Ctrl-C, SIGTERM, an error and a panic.
  Rex can keep an `idle` record after the program has gone. A `SIGKILL`
  leaves the record, and Rex decides what to do with it.

## Considered options

- **Detect Rex with the protocol's query, `OSC 7501;?`.** This needs a
  reply read through the input that the event loop owns, with a timeout
  for a terminal that never replies. Rejected for v1: `TERM_PROGRAM` is
  free, and `on` covers the case that it misses.
- **Report on each frame.** Simple, but each scroll sends a report.
  Rejected: Rex gets traffic that says nothing new.
- **Keep the counts during a pass.** Rejected: one report for each block.
- **Report each piece of busy work, with an `id` for each.** Rejected for
  v1: the Activity popup has room for a few words for each pane, and one
  status says what the user waits for.
- **A `--status` flag.** Rejected: a terminal setting does not change from
  one run to the next, so `config.toml` is the place.
