# recon

> _Read a log the way you read it in your head: one filter at a time._

`recon` is a terminal file viewer built around a **stack of regex filters** you
build up interactively. Matching lines get colour; everything else dims — and
`Ctrl-H` flips the unmatched lines from dimmed to gone entirely, keeping the
original line numbers in the gutter so a gap tells you something was left out.
Filters are individually toggled, disabled en masse with `!`, and survive
loading a different file. It's for the moment when `grep -v` has become four 
chained `grep -v`s and you've lost track of what you're excluding.

Three panes: a file navigator, the file view, and the filter list. Vim-style
motions throughout.

> **Status:** Active development — pre-1.0, no tagged releases. Keybindings and
> behaviour may change between commits.

---

## Table of Contents

- [Features](#features)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Usage](#usage)
- [Keybindings](#keybindings)
- [Known Limitations](#known-limitations)
- [Vendored dependency](#vendored-dependency)
- [Development](#development)
- [License](#license)

---

## Features

- **A filter stack, not a single pattern** — add filters with `f` (include) and
  `F` (exclude); each one is listed, numbered, and independently toggled with
  `space`. The filter pane collapses to nothing until you define your first
  filter.
- **Dim or hide, on one keystroke** — `Ctrl-H` toggles unmatched lines between
  dimmed-but-present and removed. Toggling back returns you to the exact line
  you were on.
- **Line numbers stay honest** — the gutter shows original file line numbers
  even while filtered, so gaps in the numbering mark what was hidden.
- **Filters outlive the file** — load another log and the filter set stays put.
  `!` disables everything at once and remembers what was on, so it's one
  keystroke back to an unfiltered view without discarding your work.
- **Regex everywhere** — filters and searches are both regular expressions. An
  invalid pattern reports `E486: invalid pattern` and leaves the prompt open.
- **Vim motions** — `hjkl`, `w`, `0`/`^`/`$`, `{`/`}`, `g`/`G`, `Ctrl-D`/`Ctrl-U`,
  `/` and `?` with `n`/`N` repeat.
- **Cheap navigation** — moving through the navigator renders a bounded preview
  (500 lines / 1 MiB), so scrolling a directory of large logs doesn't stutter.
- **Mouse resize** — drag the pane divider; double-click it to return to
  auto-sizing.

---

## Prerequisites

- **Rust 1.88.0 or later** — required by the vendored `tui-textarea-2` fork.
  Both it and `recon` are edition 2024, which itself needs only 1.85; the
  fork's own floor is what sets 1.88.
- **A terminal.** `recon` enters raw mode and the alternate screen; it exits
  with `Device not configured (os error 6)` if stdout isn't a TTY.

Developed and tested on macOS.

---

## Installation

### From source

```sh
git clone git@github.com:PeteRichardson/recon.git
cd recon
cargo build --release
# Binary: ./target/release/recon
```

### Install to your PATH

```sh
cargo install --path .
```

`recon` is not published to crates.io, so `cargo install recon` will not work.

### Verify

```sh
recon --version
```
```
recon 0.1.0
```

---

## Quick Start

<!-- 🖊 TODO: Add a demo GIF here
<p align="center">
  <img src="docs/images/demo.gif" alt="recon demo" width="700">
</p>
-->

```sh
recon /var/log/system.log
```

The file opens in the centre pane with its directory listed on the left. Then:

| Press | To |
| --- | --- |
| `f` `ERROR` `Enter` | Colour every line matching `ERROR`, dim the rest |
| `f` `WARN` `Enter` | Add a second filter, in its own colour |
| `F` `healthcheck` `Enter` | Drop `healthcheck` lines from view entirely |
| `Ctrl-H` | Hide the dimmed lines — only `ERROR` and `WARN` remain |
| `Tab` | Focus the filter pane; `space` toggles a filter off and on |
| `!` | Disable all filters — the whole file returns |
| `q` | Quit |

---

## Usage

```
Usage: recon <FILE>

Arguments:
  <FILE>

Options:
  -h, --help     Print help
  -V, --version  Print version
```

`<FILE>` is required. The file opens in the view pane and the navigator opens
its **parent directory**, so `recon Cargo.toml` lists the current directory.
From there, `Enter` in the navigator descends into directories or loads files.

<!-- 🖊 TODO: `<FILE>` has no help text and `--help` shows no description,
     because `Config.file` in src/lib.rs has no doc comment and Cargo.toml has
     no `description` field. Adding both would make `recon --help` self-
     explanatory. -->

### A note on stderr

`recon` logs its parsed config at DEBUG to stderr before starting the TUI, so
you'll see one line like `[DEBUG] Config { file: "app.log" }` on launch.
Redirect it if it's in your way:

```sh
recon app.log 2>/dev/null
```

---

## Keybindings

There is no separate keybindings reference beyond this section; the
authoritative source is `App::handle_event` in `src/lib.rs` for the global
keys, and each widget's `handle_events` for the rest.

Global (`src/lib.rs`), handled before the focused pane sees the key:

| Key(s) | Action |
| --- | --- |
| `q` | Quit |
| `Tab` | Move focus to the next of three panes — navigator, file view, filter pane — skipping the filter pane while it is collapsed |
| `/` | Search forward in the focused pane |
| `?` | Search backward in the focused pane |
| `f` | Add an include filter (always applies to the file view) |
| `F` | Add an exclude filter — its matches leave the view entirely |
| `Ctrl-H` / `H` | Toggle between dimming unmatched lines and hiding them |
| `!` | Disable every filter, remembering which were on; restores exactly that (or enables all, if none were on to remember) |
| `b` | Hide the left column — both the navigator and the filter pane — and focus the file view; press again to restore the split (focus stays in the file view; `e` returns it) |
| `e` | Bring the left column back and focus the navigator specifically, skipping the filter pane |
| `z` | Maximise the focused pane, or restore the split — works in the navigator too, for long filenames |

Mouse: drag the divider between the panes to resize them; double-click it to
return to auto-sizing the left column to whichever of the navigator or the
filter pane currently needs more room.

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
| `0` / `^` | Move to the start of the line |
| `$` | Move to the end of the line |
| `{` / `}` | Move by paragraph, back / forward |
| `g` / `Home` | Move to the top |
| `G` / `End` | Move to the bottom |
| `#` | Toggle the line-number gutter |
| `n` / `N` | Repeat the last search, forward / reversed |
| `Ctrl-e` / `Ctrl-y` | Scroll one line down / up |
| `Ctrl-d` / `Ctrl-u` | Scroll half a page down / up |
| `Ctrl-b` / `PageUp` / `Space` | Scroll a page up |
| `Ctrl-f` / `PageDown` / `Enter` | Scroll a page down |

`b` and `e` are global window commands rather than vim word motions: the
trade was deliberate, since returning to the navigator from a maximised file
view is exactly when you need `e`. `w` still moves forward by word.

Navigator pane (`src/widgets/filenav.rs`):

| Key(s) | Action |
| --- | --- |
| `k` / `Up` | Select the previous entry |
| `j` / `Down` | Select the next entry |
| `Enter` | Open the selected entry (descend into a directory, or load a file) |
| `n` / `N` | Repeat the last search, forward / reversed |

Filter pane (`src/widgets/filterlist.rs`), reached with `Tab` once a filter
exists — the pane sizes to its contents and collapses entirely when no
filters are defined, so it costs nothing to a user who never defines one:

| Key(s) | Action |
| --- | --- |
| `k` / `Up` | Select the previous filter |
| `j` / `Down` | Select the next filter |
| `space` | Enable or disable the selected filter |
| `d` | Delete the selected filter |

Each row shows the filter's number, whether it is enabled, whether it
includes or excludes, and its pattern — e.g. `1[x] inc foo`. Including
filters are drawn in their own colour so the pane and the file view agree at
a glance; excluding filters have no colour, which is why the sense is
spelled out. A disabled filter is dimmed. Toggling or deleting re-evaluates
the whole document but holds the line under the cursor on the same screen
row, so lines appear and disappear around a fixed point rather than the view
lurching. Deleting the last filter collapses the pane again and moves focus
off it.

---

## Known Limitations

- **Files are read entirely into memory.** `read_lines` in
  `src/widgets/fileview.rs` collects the whole file into a `Vec<String>`.
  A multi-gigabyte log will be fully resident. There is no streaming or
  memory-mapped path.  This is github issue #7.
- **Previews are bounded, full loads are not.** While the navigator has focus,
  only `PREVIEW_LINES` (500) lines or `MAX_PREVIEW_BYTES` (1 MiB) are read,
  whichever comes first. The full read happens once you focus the view.
- **Non-UTF-8 files are not viewable.** A file that isn't valid UTF-8 renders
  as `<binary file: not valid UTF-8>` rather than as bytes.
- **Nothing is persisted.** Filter sets live only for the session — there is no
  config file and no way to save or reload a filter set. Re-typing them is the
  only option after a restart.   This is github issue #8
- **The argument must be a file, not a directory.** The navigator opens the
  *parent* of whatever path you pass.

---

## Vendored dependency

`vendor/tui-textarea-2` is a patched copy of
[tui-textarea-2](https://github.com/srothgan/tui-textarea) 0.12.1, wired in
via `[patch.crates-io]`. The patch adds three public additions — setters for
per-line styles and gutter number overrides, which the file view needs in
order to dim lines that do not match a filter and to show original line
numbers while filtered, plus a `scroll_top` getter that reports the
viewport's position so the cursor's screen row can be held steady across a
buffer rebuild.

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

---

## Development

```sh
cargo build              # debug build
cargo test               # recon's suite: 224 unit + 9 integration tests
cargo test --workspace   # also runs the vendored fork's tests — see above
cargo clippy
cargo fmt -p recon       # -p recon, not plain `cargo fmt` — see below
```

Use `cargo fmt -p recon` rather than plain `cargo fmt`. At the workspace root
the latter also reformats `vendor/tui-textarea-2`, which is meant to stay
verbatim from crates.io apart from the changes listed in its `PATCH.md` —
reformatting it adds diff noise against upstream and makes a rebase harder to
verify.

Layout:

| Path | Contents |
| --- | --- |
| `src/main.rs` | Entry point: terminal setup, error hooks, logging |
| `src/lib.rs` | `App` — layout, global keys, pane focus, prompts |
| `src/document.rs` | The loaded file and its visible-line set |
| `src/filter.rs` | `FilterSet` — the filter stack and its evaluation |
| `src/widgets/filenav.rs` | Directory navigator pane |
| `src/widgets/fileview.rs` | File view pane |
| `src/widgets/filterlist.rs` | Filter list pane |
| `tests/render_smoke.rs` | End-to-end render tests |
| `docs/specs/`, `docs/plans/` | Design specs and implementation plans |

Design background lives in
[`docs/specs/2026-08-15-filter-based-viewing-design.md`](docs/specs/2026-08-15-filter-based-viewing-design.md).

---

## License

Licensed under the **MIT License** — see [LICENSE](LICENSE) for details.

`vendor/tui-textarea-2` is third-party code under its own MIT licence
(Copyright © 2022 rhysd), kept unmodified at
`vendor/tui-textarea-2/LICENSE`. It covers that directory only.
