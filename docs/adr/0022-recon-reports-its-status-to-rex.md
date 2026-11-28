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
  is `app.log · 88k lines`. A directory is `name/`. Other states
  (`working`, `done`) are #639–#641.
- **A count past four digits is short,** rounded down: `12k`, `1.2M`. A
  previewed total is the estimate, with `~`, as on the status bar.
- **While a filter pass runs, the counts are not in the message.** The
  shown count goes up with each block, so a pass sends two reports: one
  as it starts and one as it ends.
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
- **A `--status` flag.** Rejected: a terminal setting does not change from
  one run to the next, so `config.toml` is the place.
