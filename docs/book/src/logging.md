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
| `warn` | Something went wrong: a file that recon cannot read, a command view or an editor that failed. |
| `info` | A fact that you must see without a setting. |
| `debug` | One time for each session, or one time for each thing that you do. |
| `trace` | One time for each file, each key or each frame. |

## What a `debug` log shows

- **At the start:** the recon version, the arguments, TUI or batch mode and
  the reason, the config file, each `RECON_*` variable that is set, the
  filter path, each filter set (listed or not, enabled or not), the number of
  keymap overrides, the image protocol and the reason, the screen (stderr or
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
- **At the end:** how long the session was, how it ended (`q`, `Q`, Ctrl-c,
  SIGTERM, or an error), the `--emit` mode, the number of files opened and
  scanned, and the view cache hits and misses.

The log contains paths and filter patterns. It does not contain the
contents of your files or the text of a search.

## For a bug report

```sh
RECON_LOG=/tmp/recon.log RUST_LOG=recon=debug recon app.log
```

Do the steps that show the problem, quit, and attach `/tmp/recon.log`.

{{#include glossary-links.md}}
