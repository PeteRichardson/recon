# Emit on quit

**Issue:** #143 (first of its two halves). **Status:** design, approved in discussion.

recon is good at finding things: a directory, the lines a filter set selects,
the files it matches. Today the only way to get any of those out is to read
them off the screen. This spec makes a session's result leave with the
process: `recon --emit lines app.log | sort`, or a shell function that runs
recon and `cd`s to where you ended up.

The issue's discussion split #143 into two features. This is the first,
**emit on quit**: the TUI runs as it does now and prints its result when you
leave. The second, **headless mode** (filters named on the command line, files
from arguments or stdin, no TUI), is a separate spec. The decisions here that
it inherits are listed at the end.

## Decisions

Each was settled in the issue discussion; the reasoning is kept so the
alternative does not get re-proposed.

1. **Three outputs: `lines`, `files`, `cwd`.** Each is a plain list of
   strings and each is "the result of the session". The fourth output the
   issue names, `filters`, is a debugging dump with a different shape and
   waits for a second filter source to debug (#46).
2. **The flag says what, the key says whether.** `--emit <what>` at launch;
   `q` quits and emits, `Q` quits silently. Without `--emit` both quit
   silently and nothing changes for anyone. This is yazi's and fzf's shape,
   it makes a shell function one line, and it is what headless mode becomes
   when the TUI is never started. The alternative, quit chords that pick the
   output, costs three bindings and cannot be scripted.
3. **`files` is paths only.** One absolute path per line, navigator order.
   Consumers get a list `xargs` and `while read` take unchanged. The matching
   filter per file waits for a structured format.
4. **A summary line on stderr guards against the wrong mode.** Dim mode emits
   every file or every line; hide mode emits only matches. The output cannot
   say which without ceasing to be a plain list, so every emit prints one
   fixed-form line to stderr naming the mode and the counts, which reaches the
   terminal whether stdout is a pipe, a capture or the screen. Marking
   non-matching entries with `#` was rejected: a `#/path` line is still a line
   to every consumer. Emitting matches regardless of mode was rejected: dim
   mode would then have no way to emit what is on screen.
5. **The TUI draws on stderr, always.** One backend change; stdout is clean
   whether or not it is piped; one code path. Conditional switching on
   `stdout.is_terminal()` would add a path nobody can see.
6. **`Q` under `--emit` exits 1.** Asked for output and giving none is a
   failure to the caller, so `dir=$(recon --emit cwd) && cd "$dir"` skips the
   `cd` with no test on `$dir`, and a pipeline under `set -e` stops. Empty
   output from a real emit exits 0; the summary is what tells the two apart.
7. **`App` decides, `main` does I/O.** `run` returns a value; `main` prints it
   after the terminal is restored. Every emit is testable with no terminal.

## The command line

```
recon [--emit lines|files|cwd] [PATH]
```

`--emit` is a clap `ValueEnum` on `Config`, `Option<Emit>`, `None` when absent.
No environment variable and no `config.toml` key: what to emit is a per-run
decision, like `path`. The long help:

```
--emit <WHAT>  Print the session's result to stdout on `q`; `Q` quits without it.
               lines  the file view's visible lines, in the current mode
               files  the navigator's listed files, one absolute path per line
               cwd    the directory the navigator is showing
```

`Q` is a new global key: quit without emitting. Without `--emit` it is a
synonym for `q`. Bare `Q` only, with the modifier guard the other uppercase
keys use (`!intersects(CONTROL | ALT)`, since a terminal reports the Shift).

## The terminal

`init_terminal` builds `CrosstermBackend` on `io::stderr()`; `restore_terminal`
sends `LeaveAlternateScreen` and `DisableMouseCapture` to stderr too. The eyre
and panic hooks already call `restore_terminal` first, so an error still
lands on a normal screen; eyre's report goes to stderr after it, as now.

Nothing else moves. `--print-editor-config` prints to stdout and returns
before the terminal is touched. Logging goes wherever `RECON_LOG` points; when
that is unset, `env_logger` writes to stderr, which was already the wrong
place while a TUI was up and is no more wrong now.

`main` becomes:

```rust
let exit = App::new(&config).run(terminal)?;
restore_terminal()?;
Ok(exit.deliver(config.emit, &mut io::stdout(), &mut io::stderr()))
```

with `main` returning `Result<ExitCode>`, so the code `deliver` computes is
the process's.

## What `run` returns

```rust
/// What a finished session hands back for `main` to print.
pub enum Exit {
    /// `q` with `--emit`: the output and the one-line summary.
    Emit { lines: Vec<Vec<u8>>, summary: String },
    /// `Q`, or any quit without `--emit`.
    Silent,
}
```

`AppState::Quit` becomes `Quit { emit: bool }`: `q` sets `true`, `Q` sets
`false`. `run` loops until the state is not `Running`, then:

- `Quit { emit: true }` and a kind was configured → `Exit::Emit` from
  `collect(kind)`;
- otherwise → `Exit::Silent`.

`collect(kind)` is the only new logic in `App`: three arms, each over
accessors that mostly exist. Lines are `Vec<u8>` rather than `String` because
a filename is bytes on Unix, and a consumer will pass it straight back to the
filesystem; a lossy path would name a file that does not exist. `lines` and
`cwd` are text and convert trivially.

`deliver` is a method on `Exit`:

| Exit | `--emit` given | stdout | stderr | code |
|---|---|---|---|---|
| `Emit` | yes | every line, each newline-terminated | the summary, newline-terminated | 0 |
| `Silent` | yes | nothing | nothing | 1 |
| `Silent` | no | nothing | nothing | 0 |

`Emit` without `--emit` cannot occur: `run` only collects when a kind was
configured. It takes the two writers as parameters so the tests hand it
buffers.

## The three outputs

### `lines`

`document.visible_lines()`: exactly the rows the file view would draw, in the
current mode — every line in dim mode, matches only in hide mode. Verbatim,
one per line, no line numbers, no styling. The summary:

```
recon: emitted 812 lines of app.log, dim mode (27 match) — Ctrl-H to emit matches only
recon: emitted 27 lines of app.log, hide mode
```

`app.log` is the file's name as the view's title shows it. The match count is
the number of *interesting* lines, the definition `n` steps by: included by
an enabled filter or hit by the live search. The hint is printed only in dim
mode.

When the view is not showing a file — a directory listing, a read error, the
binary-file message — the output is empty and the summary says why:

```
recon: emitted 0 lines — the view is showing a directory
recon: emitted 0 lines — the view is showing an error, not a file
```

### `files`

The rows the navigator is currently listing, minus `..` and directories, as
absolute paths (`nav.dir().join(name)`) in navigator order. Hide mode has
already dropped the rows answered `No`, so the list is the matches; dim mode
lists every file. The navigator needs one new accessor, `listed_files()`,
that reads the *visible* row list rather than every entry — `files()` reads
every entry, because the scanner needs them all.

```
recon: emitted 14 files from /abs/dir, dim mode (3 match, 2 unscanned) — Ctrl-H to emit matches only
recon: emitted 3 files from /abs/dir, hide mode
```

`N match` counts rows answered `Yes`. `N unscanned` appears only when some
listed file has no answer yet, because those files might match; it is
omitted when the scan has finished. With no filter defined at all the summary
says `no filter` in place of the counts, and no hint.

### `cwd`

One line: `nav.dir()`, already absolute (`FileNav::new` absolutises its
argument). Mode is irrelevant and not mentioned:

```
recon: emitted /abs/dir
```

### Rules common to all three

- Every line, including the last, ends with `\n`.
- Nothing reaches stdout on `Silent`, and nothing reaches stdout before
  `restore_terminal` has run.
- Empty output is legitimate: hide mode with no matches emits nothing and
  exits 0.
- The summary is one line, starts with `recon: emitted`, and is the only thing
  written to stderr after the terminal is restored.

## The shell function

Documented in the README, under a new "Emitting the result" section:

```sh
rcn() {
  local dir
  dir="$(recon --emit cwd "$@")" && cd "$dir"
}
```

`Q` inside recon is how an `rcn` browse ends without a `cd`.

## Testing

- **`collect`, through `App` with no terminal.** An app over a fixture
  directory, keys to set the mode and position, then assert on the returned
  `Exit` for each kind in both modes, plus the directory-listing and
  read-error cases for `lines`, the unscanned and no-filter cases for
  `files`, and non-UTF-8 filenames for `files` on Unix.
- **`q` versus `Q`.** Three state tests: `q` under `--emit` collects, `Q`
  under `--emit` is `Silent`, either without `--emit` is `Silent`.
- **`deliver`, on its own.** Two `Vec<u8>` sinks; assert the bytes written to
  each and the exit code for all three table rows above.
- **The backend on stderr.** One render_smoke test builds the terminal the
  way `main` does and proves a frame draws.
- **Keymap.** `Q` in the help overlay and README; `every_bound_key_is_documented`
  enforces the overlay.

## Documentation

- README: `--emit` in Usage; the "Emitting the result" section with the
  three outputs, the summary line, the mode trap and `Ctrl-H`, the exit
  codes, and `rcn`; `Q` in the Global keybindings table.
- `--help`: the long help above.
- Help overlay: a `Q` row in Global.

## What does not change

- Every existing key, including `q` for anyone who never passes `--emit`.
- What the file view and navigator show. Emit reads the visible sets; it
  does not compute new ones.
- `--print-editor-config`, logging, the config file.

## Follow-ups that inherit from this spec

- **Headless mode** (#143, second half): `--emit` with the TUI never
  started, files from arguments or stdin, filters and profile from new flags.
  It reuses `collect`'s output kinds, `deliver`, the summary line and the exit
  codes; the summary's mode field becomes whatever `--hide` was passed.
- **`filters` and a structured format** (`--format json`): the per-file
  matching filter, the mode, and the counts move into the output itself,
  which is the durable answer to the mode trap the summary line guards
  against now.
- **#205** (the commented-out `show_cursor`): unrelated, but `restore_terminal`
  is touched here and the comment should be resolved in the same place.
