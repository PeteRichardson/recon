# Opening an editor from recon

**Issue:** #42 (`o`, the project key) and #41 (`O`, the file key).
**Status:** implemented
**Depends on:** #18 (the configuration mechanism), which supplies the ladder
this hangs off.

recon started as a log viewer and is now used from inside code projects, so it
needs a way to hand the file under the cursor to an editor. `o` does that,
landing the editor on the line the cursor was on.

The deliverable is **not** "editor support". It is a small command-template
engine; every editor recon will ever support — including terminal ones — falls
out of it without another line of code.

## The shape of the thing

Four separable pieces, in the order a keypress travels through them:

```text
o  ->  project_root(file)      walk up to the enclosing project
   ->  split_template(t)       turn the template into argv, once
   ->  substitute(argv, ...)   fill {project} / {file} / {line}
   ->  Launcher::spawn(argv)   detached, reaped, failure reported
```

Everything except the first step is shared with `O`. That is what made #41 one
key, one `match` arm and one template field rather than a second copy of any of
this — `EditorScope` decides whether to climb, the template decides what the
command looks like, and nothing else differs.

## `o` and `O` are siblings, not duplicates

| Key | Hands the editor | Walk-up? | Use case |
|---|---|---|---|
| `o` | project root + file + line | yes | the file is part of a codebase and the surrounding context matters |
| `O` | file + line | no | `.zshrc`, `.ssh/config`, any one-off file you want open *fast* |

**A single auto-detecting key was rejected.** "Open the project when a marker is
found, the bare file otherwise" fails on exactly the case `O` exists for:
dotfiles kept in a git repo, where `~/.zshrc` *does* have a marker above it, so
auto-detect flings open the whole dotfiles repo. Two explicit keys never lie
about which one you asked for.

### New window vs. reuse is not a recon concern

Scope (project/file) and window (new/reuse) look like a 2×2, but the window axis
collapses out:

1. **The editor already decides it, usually correctly.** Zed keeps one window
   per project, so "reuse if that project is already open, otherwise new" falls
   out of asking for the project at all.
2. **When you want to override it, it is a word in the template** — `zed -n …`,
   `code -n …`. Since the editor is a command template and not an enum, this
   needs no recon code.

Each key carries its own window habit, because each has its own template: put
`-n` in `editor.file` if `.zshrc` should always pop its own window, and leave
`editor.project` alone. A runtime new-vs-reuse toggle — a third binding, a
modifier, or a popup menu — is out of scope. The popup especially, since it
would defeat the point of `O`.

## Finding the project root

Walk up from the file's directory, stopping at the first ancestor containing a
marker:

| Marker | Kind |
|---|---|
| `.git` | any repo — a **directory**, or a **file** in a linked worktree |
| `Cargo.toml` | Rust |
| `package.json` | Node |
| `pyproject.toml` / `setup.py` | Python |
| `go.mod` | Go |

Falling back to the file's own directory when nothing matches is what makes `o`
always do something: a stray log in `/var/log` opens `/var/log`, which is worse
than a project root and much better than a key that appears dead.

`.git` is matched as a **name**, not as a directory. In a linked worktree it is
a file holding a `gitdir:` pointer — and this repo's own `/work-issue` flow
creates one per issue, so a directory-only check would walk straight past the
tree the work is actually happening in.

### The path is absolutised first

`Path::parent` walks a *relative* path down to `""` and stops, and
`"".join("Cargo.toml")` tests the **current working directory**. Without
absolutising, `project_root("logs/today.log")` climbs out of its own path and
reports whatever project recon happened to be launched from — a plausible-looking
wrong answer, which is the worst kind. Caught by a test that fails on the naive
version.

`std::path::absolute`, not `canonicalize`: no filesystem access, no symlink
resolution. For a file reached through a symlinked directory, the path the user
navigated is the one they can find their way back to.

### Rejected: reusing lsproj

The original issue claimed [lsproj](https://github.com/PeteRichardson/lsproj)
had reusable "identify the project for a path" code. It does not, in a form
worth borrowing:

- lsproj's criteria lists are for walking **downward** — enumerating every
  project under a scan root. They are traversal skip-rules, not an upward lookup.
- Its upward lookup is three lines calling `git2::Repository::discover`, in
  `main.rs` rather than its `lib.rs`. There is no exported helper.

A hand-rolled walk-up is ~20 lines and adds no dependency. Pulling in
`git2`/libgit2 for one function is a heavy trade, and `Repository::discover`
only ever finds `.git` — missing every non-git project in the table above.

## The editor is a command template, not an enum

No hard-coded list of supported editors. The user supplies an argv template with
three placeholders:

| Placeholder | Meaning |
|---|---|
| `{project}` | project root found by the walk-up |
| `{file}` | absolute path of the selected file |
| `{line}` | 1-based cursor line |

Default: `zed {project} {file}:{line}`.

An unrecognised `{placeholder}` is **left alone**, braces and all. Emptying it
would turn `code --goto {ln}` into a dangling flag and send the user hunting for
a typo recon had already spotted.

`{file}` is absolute because recon's working directory is not the editor's — a
GUI editor launched from a dock or a launcher agent inherits neither.

### Splitting and substitution, in that order

Split into argv **once**, up front; then substitute placeholders as whole argv
entries. Never re-parse after substitution, and never run the template through
`sh -c`.

This ordering is the whole security story. A path containing a space, a quote or
a `$` cannot influence how the command is split, because there is nothing left
to split by the time it arrives. And with no shell in the loop there is no
second parser to get it wrong. Pinned by tests over `/My Projects/app` and
`/p/we"ird/$HOME.log`.

Splitting honours POSIX **grouping** and nothing else — no expansion, no
globbing, no operators. Grouping is needed because the `osascript -e '…'`
templates have to survive as a single argv entry; a plain whitespace split
shatters an AppleScript into fifteen arguments.

An unclosed quote is an **error**, reported on the status row when the key is
pressed. Running the truncated-but-plausible command instead is how a
half-quoted path ends up as three arguments to an editor.

### Resolution order

```text
--editor flag
RECON_EDITOR           env, recon-specific
config file            [editor] project
$VISUAL / $EDITOR      generic, NOT recon-specific — see below
zed {project} {file}:{line}
```

This follows #18's `CLI > env > config file > defaults` ladder, with one
deliberate deviation: **`$VISUAL`/`$EDITOR` sit *below* the config file**, not
with the other environment variables. They are not recon's variables. Someone
with a global `EDITOR=vim` who has *also* written an editor template into
recon's config plainly meant the config to win. `RECON_EDITOR` is
recon-specific and keeps its normal env-level precedence above the file.

`$VISUAL` before `$EDITOR`, per the long-standing Unix convention: `$EDITOR` may
name a line editor for a dumb terminal, `$VISUAL` a full-screen one, and recon
is unambiguously the full-screen case.

**A placeholderless `$VISUAL`/`$EDITOR` gains a `{file}`.** These hold a
*command*, not a recon template, so a bare `vim` would otherwise be spawned over
no file at all. `{file}` and not `{project} {file}`: recon knows nothing about a
command it did not define, and handing an unknown editor a directory it did not
ask for opens a second buffer full of nothing. A value that already carries a
placeholder is a deliberate template and is left exactly as written.

Templates are resolved at startup but **split per keypress**, so a typo in one
of them is reported by the key that uses it rather than refusing to start a log
viewer over a setting most sessions never touch.

### Two templates, but the user normally writes one

```toml
[editor]
project = 'zed {project} {file}:{line}'   # o  — the project key
file    = 'zed {file}:{line}'             # O  — the file key
```

If `file` is unset it is **derived from `project` by dropping the `{project}`
argv entry** — a filter over the split template, not string surgery:

```text
zed {project} {file}:{line}      ->  zed {file}:{line}
code {project} -g {file}:{line}  ->  code -g {file}:{line}
idea --line {line} {file}        ->  unchanged (no {project} to drop)
```

So one line of config makes both keys work. A user who wants them to differ
writes both. `editor.file` gets its own rung on the same ladder:
`--file-editor` → `RECON_FILE_EDITOR` → config → derive-from-`project` →
`$VISUAL`/`$EDITOR` → default.

The derive sits **above** `$VISUAL`, and that ordering is tested. Deriving from
a `$VISUAL` fallback would drop a `{project}` that was never there, and deriving
from the compiled-in default would outrank `$VISUAL` — putting a rung out of
order in the one case the ladder was written to settle.

**Known limit:** a template where `{project}` is the *value* of a flag —
`wezterm cli spawn --cwd {project} -- nvim {file}` — loses the value and keeps
the dangling `--cwd`. Deriving cannot know which entries are flags without
knowing every editor's CLI, which is exactly the enum this design refuses to
hard-code. `--print-editor-config` therefore always prints **both** keys, which
costs one line and removes the trap.

## Terminal editors: solved by the same template

An earlier draft put `vim`/`nvim` out of scope, because running them in place
means tearing down the alternate screen and raw mode, running the child on the
inherited terminal, then restoring and forcing a full redraw.

That work is avoidable. Open the terminal editor **in a new terminal window**
and recon's own terminal is never touched — and a new window is just another
command, so it needs no new code at all:

```text
wezterm cli spawn --cwd {project} -- nvim +{line} {file}
kitty @ launch --type=window --cwd {project} nvim +{line} {file}
ghostty -e nvim +{line} {file}
osascript -e 'tell app "Terminal" to do script "cd \"{project}\" && nvim +{line} \"{file}\""'
```

In-place suspend-and-restore remains out of scope, and may never be needed.

**One caveat.** The `osascript` forms nest a shell command inside an AppleScript
string. recon passes the `-e` argument through as a single argv entry and never
re-parses it, so recon's own layer is safe — but the inner string *is* re-parsed
by the shell in the new window, which is why the printed templates quote the
paths inside it. The native forms (`wezterm`, `kitty`, `ghostty`) have no
nesting and no quoting hazard, so prefer them — noting that `kitty @` needs
`allow_remote_control yes` and `wezterm cli spawn` needs a running mux, so both
can fail on a default install.

## `--print-editor-config`

Rather than making people derive these, `--print-editor-config <flavour>` prints
a ready-to-paste `[editor]` stanza. Bare, it means `auto`, which guesses from
`$TERM_PROGRAM` so the snippet matches the terminal the user is actually in.

**It prints; it does not install.** recon does not write `.zshrc` or any other
shell rc: silently editing a shell config is a surprising thing for a file
viewer to do, it is hard to undo, and it has to guess between `.zshrc`,
`.zprofile`, `.bashrc` and fish. And it does not write `config.toml` either —
that is #18's standing decision, enforced in `Cargo.toml` by dropping `toml`'s
serializer so a write path fails to compile rather than merely being discouraged.

A `config.toml` stanza rather than a shell `export`, because an editor template
is a recon setting and not a shell setting, and #18 has landed.

**Rough edge, accepted:** a template containing `'` cannot use TOML's literal
string form, so the `osascript` flavours print as escaped basic strings and read
badly. They are correct — a test round-trips every flavour back through the
config parser and the splitter — and pasteable, which is what the feature is
for. The native terminal forms have no quotes and print cleanly.

## Launching

recon had never spawned a subprocess; there was no `std::process::Command`
anywhere in `src/`.

- **Detached.** stdin, stdout and stderr are all nulled. A GUI editor's startup
  warning on stderr would otherwise land in the middle of the file view and stay
  there until the next full redraw, and a terminal editor launched by mistake
  would fight recon for the keyboard.
- **Reaped, on a thread.** A child that is never waited on becomes a zombie for
  the life of the process, and recon is a long-running TUI where `o` may be
  pressed fifty times. Waiting on the main thread instead would freeze the UI
  for as long as the editor runs — for a terminal editor, the whole session.
- **Failures are reported, never fatal.** A missing editor is not a reason to
  bring the TUI down over a key that may have been pressed by accident.

### The status row grew a message slot

The bottom row was derived purely from filter state and had nowhere to put "zed:
No such file or directory". It now carries an optional transient message,
ranking between the prompt (which the user is actively typing into) and the
derived status text (which is never urgent).

**Cleared by the next keypress, not by the next event.** Mouse capture is on, so
clearing on any event would let a mouse moving across the terminal wipe the
message before it could be read.

A **non-zero exit** arrives long after the keypress — `spawn` only reports that
the process *started*. The reaper thread sends the outcome down an `mpsc`
channel, which the render loop drains on its next tick; it already wakes 60
times a second, which is exactly the property this needs.

## Testing

A real editor cannot be launched in CI, so the seam is at **command
construction**, which is pure:

```rust
fn editor_command(template: &str, project: &Path, file: &Path, line: usize)
    -> Result<Vec<String>, TemplateError>
```

`Result`, not the bare `Vec<String>` the issue sketched: a template with an
unclosed quote has no correct argv, and the alternatives are a panic inside a
TUI or a silently truncated command.

Every documented editor is a table-driven unit test asserting exact argv. Spawn
itself is behind the `Launcher` trait, so a recording double captures what
*would* have run; one `#[ignore]`d test exercises real spawning over `true` and
`false`.

**The `App`-level tests pin the template explicitly.** `Config::editor_templates`
reads the real `$VISUAL`/`$EDITOR`, so on any machine whose developer has one
set — `hx`, on the machine this was written on — an unpinned test asserts
against *their* editor and fails. The ladder itself is unit-tested with the
environment injected as arguments, which is the same rule `config_path_from`
follows and for the same reason: `std::env::set_var` is process-global and
`unsafe` in edition 2024, and these tests run in parallel.

## What is deliberately not here

- **In-place terminal-editor handover.** Superseded by new-window templates.
- **A runtime new-window toggle.** It is a word in the template.
- **Writing any config, anywhere.** Print only.
- **Validating templates at startup.** A bad template must not stop recon
  opening a log; it is reported by the key that uses it.
