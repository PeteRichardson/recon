# recon

<p align="center">
  <a href="../../actions/workflows/ci.yml"><img src="https://github.com/PeteRichardson/recon/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/PeteRichardson/recon" alt="License"></a>
</p>

<!-- 🖊 TODO: A release badge belongs here too, but would 404 until the first
     tagged release exists:
  <a href="../../releases/latest"><img src="https://img.shields.io/github/v/release/PeteRichardson/recon" alt="Latest release"></a>
-->

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

- **A filter stack, not a single pattern** — `f` focuses the filter pane, then
  `i` adds an include filter and `x` an exclude; each one is listed, numbered,
  and independently toggled with `space`. The filter pane is on screen whenever
  the navigator is, showing `press f i to add` until you define your first
  filter, so the layout never shifts under you as the set grows and shrinks.
- **Dim or hide, on one keystroke** — `Ctrl-H` toggles unmatched lines between
  dimmed-but-present and removed. Toggling back returns you to the exact line
  you were on.
- **Line numbers stay honest** — the gutter shows original file line numbers
  even while filtered, so gaps in the numbering mark what was hidden.
- **Filters outlive the file** — load another log and the filter set stays put,
  along with the `Ctrl-H` hide toggle. `!` disables everything at once and
  remembers what was on, so it's one keystroke back to an unfiltered view
  without discarding your work.
- **Skim a directory for hits** — with a filter set and `Ctrl-H` hiding, move
  down the navigator and each file draws only its matching lines. A file that
  comes up blank has none, which makes a directory of logs answerable by
  arrow key rather than by four chained `grep`s.
- **One key per pane, and you can see which has it** — `e`, `t` and `f` jump
  straight to the navigator, the file view and the filter pane, so focus is
  something you set rather than something you hunt for with `Tab`. The focused
  pane draws its border green *and* heavy, so it reads at a glance on a theme
  with weak colour and in a terminal with none.
- **Regex everywhere** — filters and searches are both regular expressions. An
  invalid pattern reports `E486: invalid pattern` and leaves the prompt open.
- **Vim motions** — `hjkl`, `w`, `0`/`^`/`$`, `{`/`}`, `g`/`G`, `Ctrl-D`/`Ctrl-U`,
  `/` and `?` with `n`/`N` repeat.
- **Directories are obvious** — bright blue, bold and slash-suffixed, with
  executables in green, following yazi. `..` is the exception: it is dimmed
  grey, because it is the one row that is never what you are looking for.
- **Look before you enter** — selecting a directory lists its contents in the
  view pane, with size and modification time, so you can see what is inside
  without going in. `l` on that selection makes the listing the navigator's.
- **Cheap navigation** — moving through the navigator renders a bounded preview
  (50,000 lines / 10 MiB), so scrolling a directory of very large logs doesn't
  stutter. Ordinary files are well inside those bounds and are simply read.
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
| `f` `i` `ERROR` `Enter` | Colour every line matching `ERROR`, dim the rest |
| `i` `WARN` `Enter` | Add a second filter, in its own colour — `f` already moved focus, so `i` alone is enough |
| `x` `healthcheck` `Enter` | Drop `healthcheck` lines from view entirely |
| `Ctrl-H` | Hide the dimmed lines — only `ERROR` and `WARN` remain |
| `space` | Toggle the selected filter off and on |
| `!` | Disable all filters — the whole file returns |
| `q` | Quit |

---

## Usage

```
Usage: recon [PATH]

Arguments:
  [PATH]  File or directory to open. A directory is listed with its first
          entry selected; a file is opened with its own directory listed
          alongside [default: .]

Options:
  -h, --help     Print help
  -V, --version  Print version
```

The argument is optional, and takes either a file or a directory:

| Command | Navigator lists | Cursor starts on | View shows |
| --- | --- | --- | --- |
| `recon app.log` | the file's parent | `app.log` | the file, read in full |
| `recon /var/log` | `/var/log` | its first entry | that entry, previewed |
| `recon` | the current directory | its first entry | that entry, previewed |

A directory *selects* its first entry rather than being handed one, so that
entry is previewed — bounded, like arrowing onto it — rather than read whole.
Starting in a directory of large logs does not read one of them in full.

A file argument that does not exist still reports itself in the view pane,
rather than silently opening whichever file happens to sort first.

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
| `Tab` | Move focus to the next of three panes — navigator, file view, filter pane. All three are always on screen, so the cycle never skips one |
| `/` | Search forward in the focused pane |
| `?` | Search backward in the focused pane |
| `e` | Focus the navigator, revealing the left column if `b` or `z` hid it |
| `t` | Focus the file view |
| `f` | Focus the filter pane — filters are then added with `i` and `x` from inside it |
| `Ctrl-H` / `H` | Toggle between dimming unmatched lines and hiding them |
| `!` | Disable every filter, remembering which were on; restores exactly that (or enables all, if none were on to remember) |
| `b` | Hide the left column — both the navigator and the filter pane — and focus the file view; press again to restore the split (focus stays in the file view; `e` returns it) |
| `z` | Maximise the focused pane, or restore the split — works in the navigator too, for long filenames |

Mouse: drag the divider between the panes to resize them; double-click it to
return to auto-sizing the left column to whichever of the navigator or the
filter pane currently needs more room.

Filters colour the lines they match and dim the rest; they are regular
expressions, like search. A filter set describes a log format rather than one
file, so it survives loading another file — `!` is the single keystroke back to
an unfiltered view without discarding the set. Nothing is hidden: filters only
change how lines are presented.

Excluding filters (`x`) are different: their matches are removed from view
outright, in both modes. `Ctrl-H` (or `H`) toggles the remaining lines between
dimmed and hidden. `H` is kept as an alternative binding for terminals
configured with `stty erase ^H`, where the Backspace *key* itself sends
`0x08` — the same byte crossterm reports as `Ctrl-H` — so pressing Backspace
would otherwise toggle hiding instead of doing nothing in this app. The gutter
keeps the original line numbers either way, so a gap in the numbering is how
you tell something was left out. Toggling back returns you to the exact line
you were on, which is the point: the hidden view is for finding a line, not
for living in.

The hide toggle survives loading another file, exactly as the filter set does
— it describes how you are reading, not which file you are reading. That is
what makes skimming work: set a filter, press `Ctrl-H`, then walk the
navigator with `j`. Each file is drawn hidden, so the ones with no matches
come up blank and the ones worth opening are the ones with anything in them.

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
| `h` / `Left` | Go to the parent directory, landing on the directory just left |
| `l` / `Right` / `Enter` | Open the selected entry — descend into a directory, or load a file |
| `n` / `N` | Repeat the last search, forward / reversed |

`h` and `l` act on the pane rather than on the row: `h` climbs out whatever is
selected, and `l` is `Enter` in every case, including on a file. They mean
here what they mean in a file manager, which is why they differ from the file
view, where they are vim character motions — the same deliberate trade as `b`
and `e`.

The cursor lands somewhere useful rather than on `..`. Entering a directory
selects its first entry and previews it, since you went in to get at something
inside; climbing out selects the directory you just left, so going up and into
a sibling is not a scroll through the whole parent listing. "First entry"
means the first one of any kind, not the first *file* — skipping over
directories would put the cursor somewhere different in every directory,
depending on where its files happened to sort.

Entries are coloured by what they are, following
[yazi](https://github.com/sxyazi/yazi): directories are bright blue and bold
and wear a trailing `/`, files with the executable bit set are green, and
everything else takes the terminal's own foreground. Three cues on a
directory rather than one is deliberate — the colour is caught first, the
bold survives a theme with weak colour contrast, and the slash survives no
colour at all.

Colour reports the file's *mode*, not whether it can be viewed: plenty of
executable scripts are readable text. What can be viewed is answered by the
view pane, which reads the actual bytes. Moving the selection onto a
directory lists that directory there — it used to keep the previous file on
screen, which read as though the directory contained that text. The
executable bit is Unix-only, so nothing is green on Windows, and a FAT or
network mount that reports every file as executable will turn the pane green.

### The directory listing in the view

Selecting a directory renders its contents in the view pane rather than a
placeholder — name, size and last-modified time, one row per entry:

```
src/                    -  2026-08-20 14:32
Cargo.toml           1.2K  2026-08-19 09:14
README.md           18.4K  2026-08-20 11:02
```

It is a look-ahead, not a pane you act in. `l` (or `Enter`) on the selected
directory makes that listing the navigator's own, which is the one keystroke
between seeing and being there. `..` is deliberately absent — it is the
navigator's way back out and nothing here could act on it.

A directory reports `-` for size: the number `stat` gives is the size of the
directory file, not of what is in it. Times are local, resolved from the
system time zone. Line numbers are suppressed while a listing is shown, since
numbering filenames says nothing; your `#` preference is untouched and applies
again as soon as a file is selected. An empty directory reads
`<empty directory>`, and one that cannot be read reports why.

The metadata sits to the right of the name on purpose: when the navigator is
wide the view narrows, and a clipped row loses the time first, the size next,
and the name last.

There is no selection marker; the selected row is drawn in reverse video.
A `>>` marker used to sit in the gutter, and it is coming back as an opt-in
setting once there is somewhere to configure it.

Filter pane (`src/widgets/filterlist.rs`), reached with `f` or `Tab` — the
pane sizes to its contents, and with no filters defined it holds a single
dimmed row reading `press f i to add` (shortened to `press f i`, then to
`f i`, or dropped entirely, if the column is too narrow for it). It is on
screen whenever the navigator is:

| Key(s) | Action |
| --- | --- |
| `i` | Add an include filter — opens the prompt at the bottom row |
| `x` | Add an exclude filter — its matches leave the view entirely |
| `k` / `Up` | Select the previous filter |
| `j` / `Down` | Select the next filter |
| `space` | Enable or disable the selected filter |
| `d` | Delete the selected filter |

`i` and `x` work only while this pane has focus, which is what `f` is for —
`f i` and `f x` reach them from anywhere, and `f` is a no-op when the pane
already has focus, so the pair is always correct. They are deliberately not
global: bound app-wide they would swallow a keystroke from every other pane,
which is exactly what `f` and `F` used to do and the reason they moved.

Each row shows the filter's number, whether it is enabled, whether it
includes or excludes, and its pattern — e.g. `1[x] inc foo`. Including
filters are drawn in their own colour so the pane and the file view agree at
a glance; excluding filters have no colour, which is why the sense is
spelled out. A disabled filter is dimmed. Toggling or deleting re-evaluates
the whole document but holds the line under the cursor on the same screen
row, so lines appear and disappear around a fixed point rather than the view
lurching. Deleting the last filter returns the pane to its `press f i to add`
row and leaves focus where it was — the pane is still on screen, so there is
nothing to move focus off.

The pane never widens the left column to fit its hint: the column is sized by
the navigator's longest entry. The hint gives way instead, which is why it has
a short form and can be dropped altogether.

The left column has a floor when it sizes itself automatically, so entering a
directory of one short name no longer shrinks it to a few columns and moves
every pane on screen. It still widens for longer names, up to its cap. The
floor applies to automatic sizing only — dragging the divider is a decision
and may still take the column narrower, and `b` or `z` give the file view the
whole width outright.

---

## Known Limitations

- **Files are read entirely into memory.** `read_lines` in
  `src/widgets/fileview.rs` collects the whole file into a `Vec<String>`.
  A multi-gigabyte log will be fully resident. There is no streaming or
  memory-mapped path.  This is github issue #7.
- **Previews are bounded, full loads are not.** While the navigator has focus,
  only `PREVIEW_LINES` (50,000) lines or `MAX_PREVIEW_BYTES` (10 MiB) are read,
  whichever comes first. The full read happens once you focus the view.

  The caps are set well above ordinary files on purpose. A truncated document
  is one that filters and line counts report *wrong* answers over — the status
  bar marks it `(preview)`, but the honest fix is not to truncate a file that
  costs a millisecond to read. Read, document clone and a filter pass together
  run about 1 ms/MB, so 10 MiB bounds the worst case near 10 ms. This is
  github issue #27.
- **Non-UTF-8 files are not viewable.** A file that isn't valid UTF-8 renders
  as `<binary file: not valid UTF-8>` rather than as bytes.
- **Nothing is persisted.** Filter sets live only for the session — there is no
  config file and no way to save or reload a filter set. Re-typing them is the
  only option after a restart.   This is github issue #8

---

## Vendored dependency

`vendor/tui-textarea-2` is a patched copy of
[tui-textarea-2](https://github.com/srothgan/tui-textarea) 0.12.1, wired in
via `[patch.crates-io]`. The patch adds four public additions — setters for
per-line styles and gutter number overrides, which the file view needs in
order to dim lines that do not match a filter and to show original line
numbers while filtered; a `scroll_top` getter that reports the viewport's
position so the cursor's screen row can be held steady across a buffer
rebuild; and a minimum gutter width, so the line-number column can be sized
for a file's estimated length while only a bounded preview of it is loaded
rather than widening when the rest arrives.

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

`PATCH.md` is the authoritative record of what this fork changes — every
entry names its file and its anchor, which is what a rebase or an upstream
submission actually works from.

There used to be an `upstream.patch` beside it, a `diff -ruN` against pristine
0.12.1. It was deleted, because it was never submittable and could only stay
correct if every future fork change remembered to regenerate it — which was
missed the first time it mattered. Three things made it unsubmittable as it
stood: its `Cargo.toml` hunk patched the *generated* manifest, while
upstream's pre-publish tree (`Cargo.toml.orig`) has no `[[test]]` entries and
no `autotests` key for it to apply to; the `[profile.bench]` removal is needed
only because this crate sits inside `recon`'s workspace; and its headers
referenced `/tmp/tui-pristine/...`, so it would not `git apply` against an
upstream checkout regardless.

None of that is lost. Regenerating an equivalent diff at any time is one
command, against the copy of 0.12.1 cargo has already unpacked:

```sh
rm -rf /tmp/tui-pristine
cp -R "$(ls -d ~/.cargo/registry/src/*/tui-textarea-2-0.12.1)" /tmp/tui-pristine
chmod -R u+w /tmp/tui-pristine && rm -f /tmp/tui-pristine/.cargo-ok
diff -ruN --exclude=PATCH.md --exclude=target /tmp/tui-pristine vendor/tui-textarea-2
```

If upstream submission is ever pursued, the work is rebuilding these changes
as commits against upstream's own tree and dropping the two local-only hunks —
a job that starts from `PATCH.md`, not from a checked-in diff. If a suitable
subset is ever accepted upstream, this directory and the `[patch.crates-io]`
entry can both be deleted.

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

### CI

`.github/workflows/ci.yml` runs on pushes to `main` and on every pull request:
`cargo fmt -p recon --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace`, then `cargo build --release`. Running
those four locally reproduces CI exactly.

Clippy is a hard gate — `-D warnings` — so a new warning fails the build. The
one standing suppression is `clippy::large_enum_variant` on `AppWidget`, which
carries its reasoning in `src/widgets/mod.rs`.

**Toolchain: stable, unpinned.** There is no `rust-toolchain.toml`, so local
builds use whatever you have and CI uses current stable. The floor that matters
is the 1.88.0 in Prerequisites, set by the vendored fork; stable is always past
it. The trade is that a stable release can break the build on a day nobody
touched the repo — acceptable in exchange for seeing new lints as they land,
and for not forcing a toolchain download on everyone who clones the repo.

**Platform: macOS only.** Linux would very likely pass, but that is unverified
— see the note under Prerequisites.

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
