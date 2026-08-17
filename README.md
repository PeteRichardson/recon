# list

A TUI log viewer with a two-pane file navigator.

## Keybindings

There is no separate keybindings reference beyond this section; the
authoritative source is `App::handle_event` in `src/lib.rs` for the global
keys, and each widget's `handle_events` for the rest.

Global (`src/lib.rs`), handled before the focused pane sees the key:

| Key(s) | Action |
| --- | --- |
| `q` | Quit |
| `Tab` | Move focus to the next pane |
| `/` | Search forward in the focused pane |
| `?` | Search backward in the focused pane |
| `f` | Add an include filter (always applies to the file view) |
| `F` | Add an exclude filter — its matches leave the view entirely |
| `Ctrl-H` / `H` | Toggle between dimming unmatched lines and hiding them |
| `!` | Disable every filter, remembering which were on; restores exactly that (or enables all, if none were on to remember) |

Mouse: drag the divider between the panes to resize them; double-click it to
return to sizing the navigator pane automatically to its contents.

Filters colour the lines they match and dim the rest; they are regular
expressions, like search. A filter set describes a log format rather than one
file, so it survives loading another file — `!` is the single keystroke back to
an unfiltered view without discarding the set. Nothing is hidden: filters only
change how lines are presented.

Excluding filters (`F`) are different: their matches are removed from view
outright, in both modes. `Ctrl-H` (or `H`) toggles the remaining lines between
dimmed and hidden. `H` is kept as an alternative binding for terminals
configured with `stty erase ^H`, where the Backspace *key* itself sends
`0x08` — the same byte crossterm reports as `Ctrl-H` — so pressing Backspace
would otherwise toggle hiding instead of doing nothing in this app. The gutter
keeps the original line numbers either way, so a gap in the numbering is how
you tell something was left out. Toggling back returns you to the exact line
you were on, which is the point: the hidden view is for finding a line, not
for living in.

Behaviour change: `Ctrl-h` no longer moves the cursor left in the file view.
It used to — the file view matches `h` regardless of modifiers — but the
global `Ctrl-H` binding is now handled first and returns before the view ever
sees the key. Plain `h` is unaffected.

While a search or filter prompt is open it consumes every key, so `q` and
`Tab` are typed into the pattern rather than acted on:

| Key(s) | Action |
| --- | --- |
| printable characters | Append to the pattern |
| `Backspace` | Delete the last character; on an empty pattern, cancel |
| `Enter` | Run the search, or add the filter |
| `Esc` | Cancel |

Searching is by regular expression in both panes — the file view matches line
contents, the navigator matches entry names, so `^foo` anchors to the start of
a filename. An invalid pattern reports `E486: invalid pattern` and leaves the
prompt open to correct.

File view pane (`src/widgets/fileview.rs`):

| Key(s) | Action |
| --- | --- |
| `h` / `Left` | Move cursor back |
| `Ctrl-h` | Nothing here — the global `Ctrl-H` hide toggle handles it first. Plain `h` still moves the cursor back. |
| `j` / `Down` | Move cursor down |
| `k` / `Up` | Move cursor up |
| `l` / `Right` | Move cursor forward |
| `w` | Move to the next word |
| `b` | Move to the previous word |
| `e` | Move to the end of the word |
| `0` / `^` | Move to the start of the line |
| `$` | Move to the end of the line |
| `{` / `}` | Move by paragraph, back / forward |
| `g` / `Home` | Move to the top |
| `G` / `End` | Move to the bottom |
| `#` | Toggle the line-number gutter |
| `n` / `N` | Repeat the last search, forward / reversed |
| `Ctrl-e` / `Ctrl-y` | Scroll one line down / up |
| `Ctrl-d` / `Ctrl-u` | Scroll half a page down / up |
| `Ctrl-b` / `PageUp` | Scroll a page up |
| `Ctrl-f` / `PageDown` / `Space` / `Enter` | Scroll a page down |

Navigator pane (`src/widgets/filenav.rs`):

| Key(s) | Action |
| --- | --- |
| `k` / `Up` | Select the previous entry |
| `j` / `Down` | Select the next entry |
| `Enter` | Open the selected entry (descend into a directory, or load a file) |
| `n` / `N` | Repeat the last search, forward / reversed |

## Vendored dependency

`vendor/tui-textarea-2` is a patched copy of
[tui-textarea-2](https://github.com/srothgan/tui-textarea) 0.12.1, wired in
via `[patch.crates-io]`. The patch adds two public setters — per-line styles
and gutter number overrides — which the file view needs in order to dim lines
that do not match a filter and to show original line numbers while filtered.

See `vendor/tui-textarea-2/PATCH.md` for the exact changes and how to rebase
onto a new upstream release. Verify a rebase with `cargo test --workspace`,
not plain `cargo test`: the workspace's default members are `recon` alone, so
plain `cargo test` runs only `recon`'s suite and skips this crate's own tests
entirely (including `tests/line_presentation.rs`, which pins the fork's whole
contract), which would let a broken fork look green. Note that
`--workspace` also compiles the vendored crate's dev-dependencies — making it
a workspace member pulls its whole dev-dependency graph into `Cargo.lock`,
around 20 extra packages including `termion`, `termwiz`, `serde_json`, and a
second `crossterm` (0.28.1) alongside `recon`'s own 0.29.0 — so expect a
heavier build than `cargo build`/plain `cargo test`, which never compile any
of it.

`upstream.patch` is a local record of what this fork changes, not a
submission-ready patch, and it has not been submitted anywhere. It contains
two hunks that are not offerable upstream even so: the `[profile.bench]`
removal (needed only because this crate lives inside `recon`'s workspace,
noted in `PATCH.md`), and the `[[test]] name = "line_presentation"` addition,
which patches the *generated* `Cargo.toml` — upstream's actual pre-publish
tree (`Cargo.toml.orig`) has no `[[test]]` entries and no `autotests` key at
all, so that hunk doesn't apply there either. The patch headers also
reference `/tmp/tui-pristine/...` and `vendor/tui-textarea-2/...`, so as-is
it will not `git apply` cleanly against an upstream checkout regardless. If
upstream submission is ever pursued, this all needs reworking first — nothing
here is submission-ready. If a suitable subset is ever accepted upstream,
this directory and the `[patch.crates-io]` entry can both be deleted.
