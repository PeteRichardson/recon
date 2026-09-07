# Headless mode

**Issue:** #143 (second half). **Status:** design, approved in discussion.
**Builds on:** `docs/specs/2026-09-06-emit-on-quit-design.md` (merged as
PR #217), which this spec extends rather than repeats.

Emit on quit made the TUI's result leave with the process. Headless mode is
the same result with the TUI never started: files named on stdin or by the
argument, filters and profile named on the command line, output on stdout.

```sh
ls -1 *.log | recon --emit files --set BugFilters:Bug57 --hide
find . -name '*.log' | recon --emit lines -n --set BugFilters --hide | cut -f1,2
recon --emit files --hide /var/log < /dev/null
```

## Decisions

1. **Headless is inferred, not flagged:** `--emit` given and stdin is not a
   terminal. A piped file list, a cron job, a script with stdin closed all get
   it without asking; `recon --emit lines app.log` from a terminal still gets
   the TUI. A TUI cannot run without a terminal on stdin anyway, since that
   is where its keys come from. An explicit `--batch` was rejected for the
   first version: a cron job that forgot it would try to start the TUI and
   die on raw mode. `< /dev/null` forces headless from a terminal.
2. **Files come from stdin, one path per line, else from `PATH`.** What
   `ls -1` and `find` produce. A `PATH` directory means its files,
   non-recursive, in the navigator's own order; a `PATH` file means itself.
3. **Several files prefix each line with `path<TAB>`;** `-n` puts `N<TAB>`
   after it. One file is exactly the TUI's output. grep's `path:N:line` was
   rejected: a path can contain a colon and the text certainly can, and the
   tab rule was already the decision for `-n`.
4. **`--set NAME[:PROFILE]`, repeatable, replaces a separate `--profile`.**
   The first colon splits set from profile; without one the set's `default`
   applies. Several sets with several profiles then fall out for free. A set
   name containing a colon is misparsed, and the error for an unknown set
   lists the real names, so it is discovered rather than hidden.
5. **`--set`, `--hide` and `-q` are startup flags for both paths.** Same
   code, one behaviour: `recon --set BugFilters --hide /var/log` opens the
   TUI with that set live in hide mode. Headless-only flags would have made
   the two paths disagree about what a run means.
6. **No ad-hoc patterns and no live search in the first version.** `-i
   ERROR` is `grep`; the example uses saved sets. Both are easy later.
7. **Read failures warn, skip, and exit 2.** grep's convention: 0 ran, 2 an
   input failed. Exit 1 for "no matches" was rejected: empty output with
   exit 0 is what emit on quit settled, and the summary carries the count.
8. **`-q` silences the summary only.** Warnings still print; `2>/dev/null` is
   there for anyone who wants total silence. The name collides with grep's
   `-q` (exit status only); `--help` says exactly what recon's does.
9. **A `headless` module composes the pieces `App` uses, without `App`.**
   Driving a real `App` with no terminal was rejected: it owns a navigator
   listing, a scanner thread and a pane-sized view, none of which a stdin
   file list has. Extending `main` inline was rejected as untestable.

## The command line

```
recon [--emit lines|files|cwd] [-n] [-q] [--hide] [--set NAME[:PROFILE]]... [PATH]
```

New flags on `Config`, all `#[arg(skip)]`-free ordinary clap flags:

```
--set <NAME[:PROFILE]>  Enable a saved set at startup, with a profile or its
                        default. Repeatable.
--hide                  Start in hide mode: only matching lines and files
-q, --quiet             Suppress the summary line on stderr
```

`--set` is `Vec<String>` on `Config`, parsed into `(set, Option<profile>)`
at the first colon by `Config::sets_to_enable()`. `--hide` is a bool mapped to
`Mode::FilteredOnly`. `-q` is a bool.

Validation, in order, before anything is read:

- `-n` without `--emit lines`: the existing refusal.
- `--set` naming a set `filters.toml` does not define, or a profile the set
  does not define: `ConfigError::UnknownSet { name, known }` /
  `UnknownProfile { set, name, known }`, rendered as
  `unknown set "Foo"; filters.toml defines: BugFilters, WiFi_debug`. These
  need the loaded sets, so `Config::check_sets(&[LoadedSet])` runs in `main`
  right after `filtersets::load_file`, where `check_flags` already does not
  need them.

`main` then decides:

```rust
let headless = config.emit.is_some() && !io::stdin().is_terminal();
let exit = if headless {
    recon::headless::run(&config)?
} else {
    let terminal = init_terminal()?;
    let exit = App::new(&config).run(terminal)?;
    restore_terminal()?;
    exit
};
Ok(exit.deliver(config.emit, config.quiet, &mut io::stdout(), &mut io::stderr()))
```

`Exit::deliver` gains `quiet: bool`: when set, the summary is not written.
Everything else about delivery — the exit-code table, the broken-pipe rule —
is unchanged, except that a headless run's own read failures set the code:
see Errors.

## The shared flags in the TUI

`App::new` applies them after `ActiveFilters::with_sets`:

```rust
for (set, profile) in config.sets_to_enable() {
    filters.enable_named(&set, profile.as_deref());
}
if config.hide { self.set_mode(Mode::FilteredOnly); }
```

`ActiveFilters::enable_named(&mut self, set: &str, profile: Option<&str>) -> Result<(), UnknownSet|UnknownProfile>`
finds the set by name, calls `set_enabled_set(index, true)` (which applies
`default`), then `apply_profile(index, name)` when one was given. The
validation in `main` calls the same lookup, so a name that passed validation
cannot fail here.

## The `headless` module

`src/headless.rs`, `pub fn run(config: &Config) -> Result<Exit>`. Four steps,
each a function with its own tests:

### 1. `inputs`

```rust
pub(crate) fn inputs(stdin: impl BufRead, path: &Path) -> io::Result<Inputs>
pub(crate) struct Inputs { files: Vec<PathBuf>, from: Source }
enum Source { Stdin, Directory(PathBuf), File }
```

Read stdin to the end. Each non-blank line is a path, absolutised against
the current directory the way `FileNav::new` absolutises its argument
(`path::absolute`). If stdin yielded nothing: `PATH` a file → that file;
`PATH` a directory → `sorted_entries(dir)` minus `Dir` and `Parent` entries,
joined onto the directory — the navigator's listing, in its order. `Source`
is remembered for the summary and for `cwd`.

### 2. The filters

`ActiveFilters::with_sets(config.filter_palette.clone(), &config.filter_sets)`,
then `enable_named` per `--set`, then `matcher()` — `None` when no including
filter is enabled, which is the navigator's "nothing to mark" state.

### 3. Per file

`read_lines` leaves `fileview.rs` for `document.rs` as
`Document::read(path: &Path) -> io::Result<Document>`: the same NUL sniff,
the same lossy decoding and newline stripping, the same size behaviour, but
returning the error instead of a placeholder message. `FileView` keeps its
`Contents::message` handling on top of it, so the widget's behaviour does not
change. The read failure cases are: not found, permission, a directory, a
binary file (`sniff_binary` hit) — the last reported as `binary file`.

For each input, in order:

- `lines`: `Document::read`, `evaluate(&filters)`, `set_mode`, then
  `visible()` mapped to output lines with the prefixes of decision 3.
- `files`: dim mode lists every readable file. Hide mode with a matcher runs
  `scan::scan(BufReader::new(File::open(path)?), &matcher, Progress::default(), &AtomicBool::new(false))`
  and lists the file when `Record::answer`'s rule says yes (`seen` holds a
  selecting bitmask). Hide mode with no matcher lists every readable file —
  the navigator hides nothing it cannot mark. The scan stops at the first
  match, so a directory of large logs costs what the navigator's scan costs.
- `cwd`: nothing per file.

Nothing is read twice.

### 4. The `Exit`

`lines`:
```
recon: emitted 27 lines of app.log, hide mode
recon: emitted 27 lines of 3 files, hide mode
recon: emitted 812 lines of app.log, dim mode (27 match) — pass --hide to emit matches only
```
`N match` is the interesting-line count summed over the files, the same
definition the TUI uses.

`files`:
```
recon: emitted 3 files from /var/log, hide mode
recon: emitted 3 files of 14 inputs, hide mode
recon: emitted 14 files from /var/log, dim mode (3 match) — pass --hide to emit matches only
recon: emitted 14 files from /var/log, dim mode, no filter
```
`from <dir>` when the inputs came from a `PATH` directory, `of N inputs` when
they came from stdin. In dim mode `files` still scans, so the match count is
real; there is no `unscanned` in headless because every scan runs to its
answer before anything prints.

`cwd`: the directory of `PATH`, or of the first input; `recon: emitted /dir`.

Output lines are bytes through `emit::path_bytes`, as in emit on quit.

## Errors and the exit code

A file that cannot be read prints, as it is met,

```
recon: cannot read /var/log/secure: permission denied
recon: cannot read ./logs: is a directory
recon: cannot read core.dump: binary file
```

and is skipped: not emitted, not counted, not listed. The run continues.
`Exit::Emit` gains a field `failed: usize`; `deliver` returns exit **2** when
it is non-zero, after writing the output and the summary for what was read.
The TUI path always passes 0.

Refused before reading, exit 1 through the existing error path: `-n` without
`--emit lines`, an unknown set or profile, an unreadable `filters.toml`.

`-q` suppresses the summary only. An empty input list (stdin held only blank
lines and `PATH` is a file that does not exist) is a read failure for `PATH`
and an empty emit.

## Testing

- `headless.rs`, over `fixtures::fixture_dir` directories: each kind in both
  modes; one file and several; stdin list versus `PATH` directory versus
  `PATH` file; `-n` prefixes with one and several files; an unreadable input
  (message, skipped, `failed` counted); a binary input; hide mode with no
  matcher lists everything; `--set` with and without a profile using
  `filter::test_support::loaded` sets; input order preserved.
- `filter.rs`: `enable_named` for a set with and without `default`, with a
  named profile, unknown set, unknown profile.
- `config.rs`: `--set` parsing with and without the colon and repeated;
  `--hide`; `-q`; `check_sets` errors name the known sets and profiles;
  README Usage regenerated.
- `emit.rs`: `deliver` with `quiet`, and with `failed > 0` → exit 2.
- `lib.rs`: `--set X:P --hide` opens the TUI with the set enabled, the
  profile applied, and hide mode on.
- `tests/headless.rs`: a real-process integration test — the built binary
  run with a piped stdin over a fixture directory, asserting stdout, the
  summary on stderr, and the exit code, for `files --hide`, `lines -n` over
  two files, `-q`, and an unreadable input. Needs no tty, so it runs in CI.

## Documentation

- README: a "Headless mode" subsection under "Emitting the result": the
  three examples above, the flags, when headless is inferred and how to
  force it (`< /dev/null`), the multi-file prefix rule, the summary
  variants, exit codes, `-q`. The `--set`/`--hide` flags also documented
  for the TUI in the saved-filter-sets section. Usage block regenerated.
- `--help`: the long help above.

## What does not change

- Emit on quit's behaviour, summaries and exit codes from a terminal.
- Every key, every `filters.toml` rule, the navigator's listing order.
- `FileView`'s reading behaviour: it calls `Document::read` and wraps the
  error as it wrapped `read_lines`' message before.

## Follow-ups

- `--batch` to force headless from a terminal without `< /dev/null`, if the
  redirect proves annoying.
- Ad-hoc patterns (`-i PATTERN`, `-x PATTERN`) and a live search (`-s`).
- `filters` output and `--format json`, from the emit spec's follow-ups.
- A `SET:PROFILE` spelling for the TUI's profile picker, for symmetry.
