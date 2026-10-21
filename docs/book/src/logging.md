# Logging

recon can write a log of what it does. Use the log when recon does not do
what you expect, and attach it to a bug report.

## Two settings

- `RUST_LOG` sets which records recon writes. It uses the
  [`env_logger`](https://docs.rs/env_logger) syntax. `RUST_LOG=recon=debug`
  gives the records on this page.
- `RECON_LOG` names a file for the log. recon empties the file each time it
  starts.

Without `RECON_LOG`, the log goes to stderr. recon draws its screen on
stderr, so it drops each record that comes while the screen is open. Only
the records from before the screen opens and after it closes stay. Set
`RECON_LOG` to keep all of them.

Without `RUST_LOG`, recon writes only warnings and errors. A normal run and
a batch run then write nothing new to stderr.

## The levels

| Level | When recon uses it |
| --- | --- |
| `error` | A worker thread stopped with a panic. |
| `warn` | Something went wrong: a file that recon cannot read, a command view, an editor or a clipboard copy that failed, a slow frame. |
| `info` | A fact that you must see without a setting. |
| `debug` | One time for each session, or one time for each thing that you do. |
| `trace` | One time for each file, each key or each frame. |

## What a `debug` log shows

- **At the start:** the recon version, the arguments, TUI or batch mode and
  the reason, the config file, each `RECON_*` variable that is set, the
  filter path, each filter set (listed or not, enabled or not), the number of
  keymap overrides, the syntax theme and the background (each with `set`
  or `default`), the image protocol and the reason, the screen (stderr or
  `/dev/tty`), and the terminal size. Also the time to read the config and
  the filter sets. A set that another file shadows has its own record.
- **Each change to the filters:** add, edit, delete, toggle, `!`, list,
  unlist, a set picker change, a profile, a solo, a reset, the peek and the
  hide mode. Each record gives the new state.
- **Each external program:** the arguments of each command view and each
  editor hand-off, how long it ran and its exit status. A command view that
  fails, or that recon stops at its timeout, is a `warn` record with the
  first line of its stderr. An editor's stderr goes nowhere, so its record
  has only the exit status.
- **Each file that recon loads into the file view:** a `view job` record
  with the path, the view, the extent (`preview` or `full`), and if the view
  cache had it. When recon reads the file, the record gives the bytes read
  and the time. For a picture, the record gives the decode time, and the
  scale when recon decoded a large JPEG at ½, ¼ or ⅛ of its size. For a
  picture file, the read and the decode are one step, so the read time
  includes the decode.
- **Each scan pass:** at the start, the directory, the number of files, the
  number that the scan cache answered and the number that recon must scan.
  At the end, the number of files scanned and the time. A pass that a new
  pass stops says `stopped`.
- **Navigation:** each change of directory, each file opened (with `n`, `N`,
  `.` and `,` too), each reload with `r`, and each file that changed on disk
  (it grew, or something wrote it again).
- **Each copy to the clipboard:** the number of lines and bytes that `y`
  copied. A copy that fails is a `warn` record with the reason.
- **At the end:** how long the session was, how it ended (`q`, `Q`, Ctrl-c,
  SIGTERM, or an error), the `--emit` mode, the number of files opened and
  scanned, and the view cache hits and misses.

## What a `trace` log adds

- **Each prefetch:** a `view job` record, as for a file that you open, with
  `prefetch` in it. recon prefetches the files near the selection.
- **Each file in a scan pass:** the path, the answer (`yes`, `no`, or `not
  known yet` when a pass stopped before the end of the file) and the time.
  Use `RUST_LOG=recon=debug,recon::scan=trace` to add these records and no
  other `trace` records.

- **Each key:** the key, and the action that it did, for example `key j:
  view.down`. A key that does nothing says `no action`. A character that
  you type into a prompt or the filter editor is not in the log: its
  record says `typed into a text field`.
- **Each frame:** the time that recon took to draw the screen.

## A slow frame

A frame that takes more than 50 ms to draw gives a `warn` record, `slow
frame`. You do not need `RUST_LOG` for it: set only `RECON_LOG`. recon does
not show it on the screen.

The log contains paths and filter patterns. It does not contain the
contents of your files or the text of a search.

## For a bug report

```sh
RECON_LOG=/tmp/recon.log RUST_LOG=recon=debug recon app.log
```

Do the steps that show the problem, quit, and attach `/tmp/recon.log`.

{{#include glossary-links.md}}
