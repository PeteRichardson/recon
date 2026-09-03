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
`Ctrl-H` removes the unmatched lines entirely, keeping the original line
numbers in the gutter so a gap tells you something was left out.
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
- [Opening an editor](#opening-an-editor)
- [Syntax colouring](#syntax-colouring)
- [Known Limitations](#known-limitations)
- [Vendored dependency](#vendored-dependency)
- [Development](#development)
- [License](#license)

---

## Features

- **A filter stack, not a single pattern** — `f` focuses the filter pane, then
  `i` adds an include filter and `x` an exclude; each one is listed, numbered,
  independently toggled with `Enter`, and editable in place with `c` — a
  near-miss regex is corrected where it stands rather than deleted and retyped
  into a different slot. The filter pane is on screen whenever
  the navigator is, showing `press f i to add` until you define your first
  filter, so the layout never shifts under you as the set grows and shrinks.
- **Dim or hide, on one keystroke** — `Ctrl-H` toggles unmatched lines between
  dimmed-but-present and removed. Toggling back returns you to the exact line
  you were on. Dimming marks unmatched lines whenever a numbered *including*
  filter is enabled; a search on its own doesn't grey the file, since its
  hits already carry a highlight — but `Ctrl-H` still collapses to them.
- **Search is just a filter** — `/` defines one in a keystroke, `Esc` throws it
  away, and `p` keeps it: it joins the numbered set with its own colour and
  frees `/` for the next probe, so a filter set gets built by trying patterns
  rather than by retyping them. In between it behaves like any other filter —
  it survives loading another file, answers to `!`, loses to an exclude, and
  feeds `Ctrl-H`. `n` and `N` step between *interesting* lines, whether the
  filters or the search made them so.
- **Line numbers stay honest** — the gutter shows original file line numbers
  even while filtered, so gaps in the numbering mark what was hidden. While
  hiding, the last number of each run of consecutive lines is underlined, so
  the boundaries are visible without reading the numbers to find them.
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
  `/` then `n`/`N` to step between matches.
- **Directories are obvious** — bright blue, bold and slash-suffixed, with
  executables in green, following yazi. `..` is the exception: it is dimmed
  grey, because it is the one row that is never what you are looking for.
- **Look before you enter** — selecting a directory lists its contents in the
  view pane, with size and modification time, so you can see what is inside
  without going in. `l` on that selection makes the listing the navigator's.
- **Cheap navigation** — moving through the navigator renders a bounded preview
  (50,000 lines / 10 MiB), so scrolling a directory of very large logs doesn't
  stutter. Ordinary files are well inside those bounds and are simply read.
- **Mouse resize** — drag either pane divider: the vertical one to set the left
  column's width, the horizontal one under the navigator to set how tall the
  filter pane is. Double-click either to return it to auto-sizing.
- **Code is coloured** — keywords, strings and comments in around 150
  languages, Swift and TOML included, using your terminal's own palette by
  default so it matches whatever theme you already run. Pick a bundled theme
  (`Dracula`, `Nord`, `gruvbox-dark`, …) or point at any `.tmTheme` file with
  `--theme` or `[syntax] theme`; `none` turns it off. Filter colours and search
  hits stay on top, and a multi-megabyte file colours only what is on screen,
  so opening one costs nothing extra.
- **Straight into your editor** — `o` opens the selected file's enclosing
  *project* at the line under the cursor; `O` opens the file *alone*, for the
  `.zshrc` you just want open fast. The editor is a command template rather than
  a hard-coded list, so zed, VS Code, Sublime, IntelliJ and even
  `nvim`-in-a-new-terminal-window are all one line of config; run
  `recon --print-editor-config` for a ready-made one.

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
| `Enter` | Toggle the selected filter off and on |
| `!` | Disable all filters — the whole file returns |
| `q` | Quit |

---

## Usage

```
Usage: recon [OPTIONS] [PATH]

Arguments:
  [PATH]  File or directory to open. A directory is listed with its first entry
          selected; a file is opened with its own directory listed alongside
          [default: .]

Options:
      --editor <TEMPLATE>
          Command template `o` runs, e.g. `zed {project} {file}:{line}` [env:
          RECON_EDITOR=]
      --file-editor <TEMPLATE>
          Command template `O` runs. Defaults to `--editor` with the `{project}`
          argument dropped, so one setting normally configures both keys [env:
          RECON_FILE_EDITOR=]
      --print-editor-config [<FLAVOUR>]
          Print a ready-to-paste `[editor]` stanza and exit. Takes a flavour —
          `zed`, `vscode`, `wezterm-nvim`, … — or `auto` to guess from
          `$TERM_PROGRAM`
      --theme <THEME>
          Colours for the file view's syntax colouring: a bundled theme name, a
          path to a `.tmTheme` file, or `none` to turn colouring off [env:
          RECON_THEME=]
  -h, --help
          Print help (see more with '--help')
  -V, --version
          Print version
```

This block is checked against the binary: `readme_usage_block_matches_the_real_help`
in `src/config.rs` renders `recon -h` at 80 columns and fails if it has drifted.
Regenerate by running the test and pasting what it prints.

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

### Logging

`recon` is quiet by default: nothing is written unless something actually goes
wrong, and warnings are the only level that reaches you unasked.

`RUST_LOG` selects what is recorded, using the usual
[`env_logger`](https://docs.rs/env_logger) syntax:

```sh
RUST_LOG=recon=debug recon app.log
```

Log output goes to **stderr** by default, which is a problem once the TUI is up:
stderr writes to the normal screen, and `recon` is holding the alternate one, so
a line logged mid-session is painted over the interface until the next redraw.
Set `RECON_LOG` to a file to avoid that, and to capture the in-session messages
at all:

```sh
RECON_LOG=/tmp/recon.log RUST_LOG=recon=debug recon app.log
```

The file is truncated on each run. If it cannot be opened, `recon` says so and
carries on logging to stderr rather than refusing to start.

What gets recorded:

| Level | What |
| --- | --- |
| `warn` | a file or directory that could not be read, and why; an editor that exited badly or could not be waited on |
| `debug` | which `config.toml` was read, or where `recon` looked and found none |

---

## Keybindings

Press `?` in the app for the same list on screen. The authoritative source is
the code, in three places rather than two:

- **`App::handle_event`** in `src/lib.rs`, for the global keys.
- **Each widget's `handle_events`**, for the keys its own pane answers.
- **`long_range_target`** in `src/viewport.rs`, for `g`, `G`, `{` and `}`.
  These are *intercepted* — `App::handle_event` resolves them against the whole
  visible set and returns before the file view sees the key, so they are bound
  in two places and the interception wins. They have to be: the file view holds
  a window of the visible set, and each of these means "the document's top" or
  "the next paragraph anywhere", not "the top of whatever is loaded" (#7).

This section and the in-app overlay both describe that code.

The four would drift silently, so they don't have to be checked by hand:
`KEYMAP` in `src/help.rs` is the table the overlay draws, and
`every_bound_key_is_documented` reads all three source locations back at test
time and fails when a key is bound in a `Char(..)` arm and named by no row.
Adding a binding without documenting it breaks the build. The test says nothing
about *this* section, which is still hand-maintained — so a new key needs a row
here too.

Global (`src/lib.rs`), handled before the focused pane sees the key:

| Key(s) | Action |
| --- | --- |
| `?` | Show the keymap overlay — every binding on one screen. Any key closes it, and that key does nothing else |
| `q` | Quit |
| `Tab` | Move focus to the next of three panes — navigator, file view, filter pane. All three are always on screen, so the cycle never skips one |
| `/` | In the navigator, search filenames. In the file view, set a live search — a filter of its own, which moves you to its next hit from the cursor exactly as `n` would |
| `p` | Promote the live search into the numbered filter set, freeing `/` for the next one |
| `Esc` | Clear the live search (an open prompt takes this key first and just cancels the prompt) |
| `e` | Focus the navigator, revealing the left column if `b` or `z` hid it |
| `t` | Focus the file view |
| `f` | Focus the filter pane — filters are then added with `i` and `x` from inside it |
| `space` | **Peek at the plain file** — drop every filter and flip the hide mode, so the code reads normally. Press again to put the filtered view back exactly as it was. See [Peeking at the plain file](#peeking-at-the-plain-file) |
| `Ctrl-H` / `H` | Toggle between dimming unmatched lines and hiding them |
| `!` | Disable every filter, remembering which were on; restores exactly that (or enables all, if none were on to remember) |
| `&` | Combine the enabled include filters with **AND** instead of OR — a line must match every one of them. Press again for OR. See [Combining filters with AND](#combining-filters-with-and) |
| `b` | Hide the left column — both the navigator and the filter pane — and focus the file view; press again to restore the split (focus stays in the file view; `e` returns it) |
| `z` | Maximise the focused pane, or restore the split — works in the navigator too, for long filenames |
| `o` | Open the selected file's enclosing **project** in your editor, at the line the cursor is on — see [Opening an editor](#opening-an-editor) |
| `O` | Open the selected **file alone**, at the same line — no project, no walk-up |
| `r` | **Refresh from disk** — re-list, re-stat and rescan the navigator's listing (so a file created since the listing was built appears), and reload the file in the view with the cursor kept on its line. The status row shows `changed on disk · r` when the open file's size or mtime has moved |

`?` used to search backward. `n`/`N` cover both directions now, which is what
freed it for the overlay — it is the conventional help key in the pagers and
file managers recon borrows the rest of its bindings from.

The overlay is a centred panel rather than a fourth pane: the three panes are
permanent and `Tab` walks between them, while help is something you glance at
and put away. Joining that cycle would mean tabbing past it forever after using
it once. It flows into as many columns as the terminal can hold, so the whole
keymap fits one screen with nothing to scroll — which is what lets *any* key
close it, rather than reserving some keys for scrolling. The status row stays
visible underneath, so the HIDE badge still tells the truth while you are
reading about `Ctrl-H`. On a terminal too small for the whole table, the bottom
border says how many rows were cut.

The overlay shows every binding, not just the focused pane's. Context-sensitive
help is a genuine improvement and deliberately deferred; see issue #25.

Mouse: drag the vertical divider between the columns to resize them;
double-click it to return to auto-sizing the left column to whichever of the
navigator or the filter pane currently needs more room.

The horizontal divider — the border between the navigator and the filter pane
below it — drags the same way, and sets how many rows the filter pane gets.
Dragging it *up* makes the pane taller, since the pane is anchored to the
bottom of the column and what moves is where it begins. Double-click it to go
back to automatic sizing. Neither drag can squeeze the navigator out of
existence: it keeps three rows whatever you ask for.

Filters colour the lines they match and dim the rest; they are regular
expressions, like search. A filter set describes a log format rather than one
file, so it survives loading another file — `!` is the single keystroke back to
an unfiltered view without discarding the set. Nothing is hidden: filters only
change how lines are presented.

#### Saved filter sets

The filters that find a bug are three or four regexes, and typing them every
session is the part recon could not help with. `~/.config/recon/filters.toml`
holds **sets**: named groups of filters, defined once and loaded at startup.

```toml
# ~/.config/recon/filters.toml

[sets.WiFi_debug]
priority = 10                            # lower is nearer the top; default 50
autoload = true                          # start with this set enabled

[sets.WiFi_debug.profiles]
default = ["assoc", "deauth"]            # applied whenever the set is enabled
WiFi_bug_32 = ["deauth", "beacon-loss", "retry"]

[[sets.WiFi_debug.filters]]
name    = "assoc"
pattern = 'wlan\d+: associated'

[[sets.WiFi_debug.filters]]
name    = "deauth"
pattern = 'deauthenticat(ed|ing)'
colour  = "red"                          # instead of the next palette colour

[[sets.WiFi_debug.filters]]
name    = "beacon-loss"
pattern = 'beacon loss'
sense   = "context"                      # include (default) | context | exclude

[[sets.WiFi_debug.filters]]
name    = "retry"
pattern = 'retry|backoff'
sense   = "exclude"
```

Single-quoted TOML strings do no escape processing, so a regex goes in
verbatim — no doubled backslashes.

**A set is enabled or disabled as a unit, and enabling it does not decide
which of its filters are on.** The set's flag means "these filters count";
each filter's own flag means "this one is on". A filter takes effect only when
both are. The filters you type with `i` and `x` live in an unnamed *scratch
set* that is always present and always first — with no `filters.toml` at all,
recon is exactly this model with one set.

- **`autoload`** — the set starts enabled. Without it a set is known and
  listed, but off until you enable it.
- **Profiles** — named permutations of a set's filters. Applying one enables
  exactly those and disables the set's others. `default` is applied whenever
  the set is enabled; without a `default`, enabling a set keeps whatever
  flags its filters had, so a re-enabled set shows the toggles you left.
- **`priority`** — where the set sits in the pane, lower first, ties by name.
  The pane is short, so the top rows are a setting rather than an accident of
  naming.
- **`name`** — what the pane calls a filter, and what profiles refer to. It
  defaults to the pattern itself.
- **`colour`** — a colour for this filter instead of the next palette entry,
  in the same spellings as the palette: a name, `#RRGGBB`, or a 256-colour
  index as a string. A palette position is a poor way to say "errors are
  red".

The file is validated before the terminal is taken, with the same
refuse-to-start policy as `config.toml`: a pattern that does not compile, a
colour that does not parse, two filters answering to one name, a profile
naming a filter the set lacks, a set with no filters, or a key the schema does
not know each stop recon with a message naming the file, the set and the
filter. A missing file is not an error. recon never writes this file.

In this release the pane lists every known filter flat, with a file filter
shown as `set/name`; the two-level pane with per-set toggling, the profile
picker, solo and reset follow (#129–#132). `!` and `space` act on filter flags
across every set and never on a set's own flag.

#### The filter palette

Successive filters take successive colours, wrapping once the list runs out.
The built-in six are fixed 256-colour shades — gold, cyan, green, magenta,
periwinkle, red — rather than `Color::Yellow` and friends, because the named
variants are ANSI slots whose actual appearance your terminal theme decides,
and recon cannot promise contrast between two colours it doesn't choose.

Override the whole list in `config.toml`:

```toml
[filters]
palette = ['#ffd700', '#00d7ff', '46', '201']
```

Each entry is a string, in one of three forms:

| Form | Example | Note |
|---|---|---|
| Hex triple | `'#00d7ff'` | Exactly `#RRGGBB`; the shade you asked for |
| 256-colour index | `'46'` | Quoted — TOML `46` unquoted is a number, not a colour |
| Colour name | `'magenta'` | One of the sixteen ANSI slots, so **your theme decides how it looks** |

The names are `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`,
`gray`, `dark-gray`, and `light-` variants of the six chromatic ones plus
`white`. `bright-` works as a synonym for `light-`, and spaces, dashes and
underscores are interchangeable. Prefer hex or an index unless you *want* the
colour to follow your theme — a name is the theme dependency this palette was
changed to avoid.

The list replaces the built-in palette **wholesale**, not slot by slot: its
length is part of the setting, since that's what decides when colours start
repeating. Four colours means the fifth filter reuses the first. Omit the key
to keep recon's own; `palette = []` is refused at startup, and so is a value
that isn't a colour — both errors name the file and the line.

The live search's colour is deliberately outside the palette — see below.

The live search set by `/` is one of these filters, not a separate mode: it
takes its own colour, answers to `!`, and loses to an exclude the same as any
numbered one. It differs in one place — dimming. A numbered *including*
filter dims the rest of the file the moment it's enabled; a search on its own
doesn't, because its hits already carry a highlight of their own and greying
the file around them would only cost the context the search was run to see.
`Ctrl-H` makes no such exception: it collapses to a search's matches exactly
as it would to a filter's. `Esc` drops the search; `p` keeps it, moving it
into the numbered set and freeing `/` for the next one.

Excluding filters (`x`) are different: their matches are removed from view
outright, in both modes. `Ctrl-H` (or `H`) toggles the remaining lines between
dimmed and hidden. `H` is kept as an alternative binding for terminals
configured with `stty erase ^H`, where the Backspace *key* itself sends
`0x08` — the same byte crossterm reports as `Ctrl-H` — so pressing Backspace
would otherwise toggle hiding instead of doing nothing in this app. The gutter
keeps the original line numbers either way, so a gap in the numbering is how
you tell something was left out — and while hiding, the last number before
each gap is underlined, since ten matched lines, a hundred hidden ones and ten
more matched lines otherwise read as twenty consecutive lines unless you stop
to compare the numbers. Toggling back returns you to the exact line
you were on, which is the point: the hidden view is for finding a line, not
for living in.

#### Combining filters with AND

Including filters are combined with **OR**: a line is coloured if *any* enabled
one matches it. That answers "show me errors or warnings" and cannot answer
"show me lines with `foo` *and* `bar`". The regex workaround does not scale —
Rust's `regex` crate has no lookaround, so the only spelling is every ordering
of the terms (`foo.*bar|bar.*foo`), which is six alternations at three terms
and 24 at four. And the natural thing to type, `foo.*bar.*baz`, silently misses
`baz bar foo`.

`&` flips the whole set to **AND**: a line is included only when *every*
enabled including filter matches it. Press `&` again to go back. The status row
carries an **AND** badge while it is on, painted like **HIDE** and for the same
reason — the mode survives file loads and is easy to forget.

What the mode does and does not touch:

- Every matching line matched every filter, so no single filter owns it: lines
  take the colour of the **first enabled** including filter. Disable that one
  and the next takes over — and stops being a term.
- Excluding filters (`x`) are unchanged. They remove lines in both modes.
- Context filters (`m`) are not terms. A line a context filter matches is still
  shown; that sense promises "also show these" and keeps its promise.
- The live search (`/`) is not a term either. A probe never narrows the set it
  is probing; `p` promotes it into one.
- The navigator follows the same rule: a file is marked when one of its lines
  matches every enabled including filter, in that first filter's colour. The
  cached scan re-answers the folder with no I/O, as any toggle does.

`&` is global and works from any pane. Groups — AND *within* a group, OR
*between* them — are the general form and are tracked separately (#40).

The hide toggle survives loading another file, exactly as the filter set does
— it describes how you are reading, not which file you are reading. That is
what makes skimming work: set a filter, press `Ctrl-H`, then walk the
navigator with `j`. Each file is drawn hidden, so the ones with no matches
come up blank and the ones worth opening are the ones with anything in them.

Because the toggle survives file loads, it is easy to forget it is on — so
while hiding, the bottom row carries a reverse-video **HIDE** badge. It tracks
the mode and only the mode: it is there the instant you press `Ctrl-H` and gone
the instant you press it again, whether or not any filter is currently removing
lines. The `▼` funnel beside it answers the other question — whether lines are
actually missing from the pane right now — which is why the two do not always
appear together. Hide mode with every filter disabled shows the badge without
the funnel, because nothing is being hidden yet; an excluding filter while
dimming shows the funnel without the badge, because lines are gone without hide
mode having anything to do with it.

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
| `Enter` | Run the search, or add the filter — or, for a prompt opened with `c`, overwrite the filter being edited |
| `Esc` | Cancel |

A prompt opened with `c` starts pre-filled with the pattern being edited, with
the cursor at the end; every other prompt starts empty. That is the tell for
which one you are in: text already there means you are changing something, an
empty prompt means you are making something new.

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
| `n` / `N` | Move to the next / previous *interesting* line |
| `Ctrl-e` / `Ctrl-y` | Scroll one line down / up |
| `Ctrl-d` / `Ctrl-u` | Scroll half a page down / up |
| `[` / `Ctrl-b` / `PageUp` | Scroll a page up |
| `]` / `Ctrl-f` / `PageDown` | Scroll a page down |

`b` and `e` are global window commands rather than vim word motions: the
trade was deliberate, since returning to the navigator from a maximised file
view is exactly when you need `e`. `w` still moves forward by word.

`n` and `N` are handled globally, the same as `Ctrl-H`, so — like that key —
they reach this table only once the file view has focus; the navigator has its
own `n`/`N`, described below. An *interesting* line here is one an enabled
including filter or the live search matches; stepping wraps at the ends of the
file and treats a line with several hits as a single stop, not one per hit.

Navigator pane (`src/widgets/filenav.rs`):

| Key(s) | Action |
| --- | --- |
| `k` / `Up` | Select the previous entry |
| `j` / `Down` | Select the next entry |
| `h` / `Left` | Go to the parent directory, landing on the directory just left |
| `l` / `Right` / `Enter` | Open the selected entry — descend into a directory, or load a file |
| `n` / `N` | Repeat the last filename search, forward / reversed — or, with no search active, move to the next / previous file the filters match |

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

Entries are *ordered* the same way `yazi`, `ls` and Finder order them:
directories first, then case-insensitively by name. A listing sorted by raw
bytes instead puts every capitalised name in a block above every lowercase one
— `Cargo.toml`, `README.md`, `app.log` — which is the half that makes a
directory hard to scan.

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
| `Enter` | Enable or disable the selected filter |
| `d` | Delete the selected filter |
| `c` | Change the selected filter's pattern — reopens the prompt over it |
| `m` | Toggle the selected filter between *include* and *context* — see below |

`c` edits in place: the filter keeps its slot, and so keeps its colour, its
sense and whether it is enabled. That matters because a slot is a precedence —
the *first* matching filter decides a line's colour — so the only way to change
a pattern before this existed, `d` followed by a full retype, put the
replacement at the end and silently reordered the set. `Esc` abandons the edit;
backspacing past the start does the same, so an edit can never leave an empty
pattern behind. A pattern that will not compile reports `E486: invalid pattern`
and leaves the prompt open over the intact filter, exactly as `i` does.

### Three senses, and which files the navigator marks

Every numbered filter has a sense, shown in its row as `inc`, `ctx` or `exc`:

| Sense | In the view | Marks the file in the navigator? |
| --- | --- | --- |
| `inc` — include | shows the line, in the filter's colour | **yes** |
| `ctx` — context | shows the line, in the filter's colour | no |
| `exc` — exclude | removes the line | no |

With at least one include filter (or a live search) enabled, the navigator marks
each file: a name drawn in a filter's colour has at least one line that filter
selected; a dimmed name has none; a plain name has not been scanned yet. In hide
mode (`Ctrl-H`), non-matching files leave the listing the way non-matching lines
leave the view. The scan runs in the background and stops each file at its first
matching line, and toggling a filter on or off usually re-answers the whole
folder without reading anything.

*Context* is for the patterns every log carries — the build's commit, the host —
that you want to see in the view but that say nothing about which logs are
interesting. Which sense a pattern has is your call, per set: `^host:
production-.*` is an include when the question is "which production logs have
errors" and context when it is "which logs have bug 57, and where did they run".
New filters are include; `m` flips the selected one.

`Enter` took the toggle over from `space`, which is now the global peek. A key
that toggled a filter in this pane and flipped hide mode in the other two is
exactly the pane-dependent meaning that change removed — and recovering from the
wrong one cost a few seconds every time it happened.

`Enter` is also the key that *commits* the prompt `i`, `x` and `c` open, one
keystroke earlier. So the `Enter` immediately after a commit is **swallowed**: a
doubled press finishes the pattern and does nothing else, rather than quietly
switching a filter off. Any other key in between and the next `Enter` toggles as
normal, so the guard costs at most one extra press when you really did mean two.

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

A live search draws as one more row, at the top, marked `/` instead of a
number — it has none, because it does not occupy a slot in the numbered set.
`Enter`, `d` and `c` reach it exactly as they reach a numbered filter: `Enter`
toggles the flag that also drives its highlight in the file view, `d` clears
it, same as `Esc`, and `c` reopens it under `/` for editing. `p` is what moves
it into the numbered set proper.

Committing an edit of the search row behaves exactly as retyping `/` does,
including moving to the first hit and switching the search back on if it had
been toggled off — it is the same operation, reached from the pane instead of
from a keystroke.

The pane never widens the left column to fit its hint: the column is sized by
the navigator's longest entry. The hint gives way instead, which is why it has
a short form and can be dropped altogether.

The left column has a floor when it sizes itself automatically, so entering a
directory of one short name no longer shrinks it to a few columns and moves
every pane on screen. It still widens for longer names, up to its cap. The
floor applies to automatic sizing only — dragging the divider is a decision
and may still take the column narrower, and `b` or `z` give the file view the
whole width outright.

The pane's *height* has a floor for the same reason, and it applies even with
no filters defined: eight rows, so the pane you define filters in is visible
before the first one exists rather than being a title and a hint wedged under
the navigator. A larger set still gets the rows it asks for, up to half the
column. Both bounds govern automatic sizing only — drag the horizontal divider
and the pane is whatever height you left it at, down to a single row.

---

## Peeking at the plain file

Filters answer "where is it?". Once you have landed on the line, the next
question is "what does this code *do*?" — and for that the filtering is in the
way. Hiding shows you the match and nothing around it; dimming leaves the
surroundings on screen but hard to read.

`space` handles that in one key. It drops every filter and flips the hide mode,
so the file reads as an ordinary file. Press it again and the filtered view
comes back **exactly** as it was — same filters, same colours, same hide mode,
same line under the cursor.

It replaces a four-key round trip you would otherwise repeat at every match:
leave hide mode, clear the filters, read, restore the filters, restore hide
mode. It works from any pane, and it always means the same thing — which is why
the filter pane's toggle moved to `Enter`.

Two neighbours it is easy to confuse:

| Key | Does |
| --- | --- |
| `space` | Filters off **and** hide mode flipped, for reading. One key back to exactly where you were |
| `!` | Filters off, hide mode untouched. Its own independent memory of what was on |
| `H` | Hide mode only, filters untouched |

`space` and `!` keep separate memories, so using one never disturbs what the
other would restore.

### Why the ` HIDE ` badge can appear over a plain file

Peeking from the dimmed view flips hiding **on** while showing you the whole
unfiltered file, so the badge lights up over a file where nothing is hidden.
That is not a contradiction, because hide mode does not mean "hide every
unmatched line". It means:

> if something is including — a filter or the search — hide unmatched lines; if
> nothing is, show everything.

So it is a standing preference, armed or not, rather than a description of what
is currently on screen. `space` turns every filter off, which leaves hiding
nothing to bite on; the badge reports that hiding is *armed*, and it will take
effect the moment the filters come back.

This is also why the flip is safe at all. Flipping into hide mode with no
filters left enabled would blank the pane, were it not for the rule above —
which is enforced in `Document::recompute_visible` and predates this key.

## Opening an editor

`o` hands the selected file to your editor, opened at the line the cursor is on,
with the **enclosing project** alongside it. It works from any pane — the file
it means is whatever the view is showing, which already follows the navigator's
selection.

The project is found by walking up from the file until a marker turns up:
`.git` (a directory, or the file a linked worktree uses), `Cargo.toml`,
`package.json`, `pyproject.toml`, `setup.py` or `go.mod`. With no marker
anywhere above, `o` opens the file's own directory, so it always does something.

`O` opens the **file alone** and skips the walk-up entirely. The two are
siblings, not duplicates:

| Key | Hands the editor | Walk-up? | For |
| --- | --- | --- | --- |
| `o` | project root + file + line | yes | a file in a codebase, where the surrounding context matters |
| `O` | file + line | no | `.zshrc`, `.ssh/config`, any one-off file you want open *fast* |

There is deliberately no single auto-detecting key. It would break on exactly
the case `O` exists for — dotfiles kept in a git repo, where `~/.zshrc` *does*
have a marker above it, so auto-detect would fling open the whole dotfiles repo.
Two explicit keys never lie about which one you asked for.

### Configuring it

The editor is a **command template**, not a list of supported editors. recon
fills in three placeholders and runs the result:

| Placeholder | Meaning |
| --- | --- |
| `{project}` | the project root found by the walk-up |
| `{file}` | absolute path of the selected file |
| `{line}` | 1-based cursor line |

The default is `zed {project} {file}:{line}`. To change it, put a stanza in
`config.toml` — `--print-editor-config` writes one for you, on stdout:

```console
$ recon --print-editor-config vscode
# recon editor templates (vscode)
# Paste into ~/.config/recon/config.toml — recon never writes it for you.
# {project} = project root, {file} = the file, {line} = the cursor's line.
[editor]
project = 'code {project} -g {file}:{line}'
file = 'code -g {file}:{line}'
```

Bare `--print-editor-config` guesses a flavour from `$TERM_PROGRAM`. The known
ones are `zed`, `vscode`, `sublime`, `idea`, `terminal-nvim`, `iterm-nvim`,
`wezterm-nvim`, `kitty-nvim` and `ghostty-nvim`. It only ever **prints** — recon
writes neither `config.toml` nor your shell rc.

`editor.file` is the template for `O`, and `{project}` is not substituted there
— there is no project. Leave it out and it is derived from `editor.project` by
dropping the `{project}` argument, so one line normally configures both. Write
both when you want them to differ — `-n` on just the file one, say.

New window versus reusing an existing one is a word in the template, not a
separate key: `zed -n {file}:{line}` always gets a fresh window, `zed
{file}:{line}` lets Zed decide. Because the two templates are separate settings,
each key carries its own window habit with no extra machinery.

Resolution order, following recon's usual chain with one deliberate exception:

```text
--editor flag                                --file-editor flag
RECON_EDITOR             recon-specific      RECON_FILE_EDITOR
config.toml              [editor] project    [editor] file
                                             derive from project, dropping {project}
$VISUAL / $EDITOR        generic, so it ranks BELOW the config file
zed {project} {file}:{line}                  zed {file}:{line}
```

`$VISUAL`/`$EDITOR` sit below the file on purpose: they are not recon's
variables, and someone with a global `EDITOR=vim` who has *also* written a recon
editor template plainly meant the template to win. A bare command name found
there gains a `{file}`, so `EDITOR=vim` still opens the file.

### Terminal editors

There is no special support for `vim`/`nvim`, and none is needed: open them in a
**new terminal window** and recon's own screen is never touched. A new window is
just another command, so it is the same template with a different string in it:

```toml
[editor]
project = 'wezterm cli spawn --cwd {project} -- nvim +{line} {file}'
file = 'wezterm cli spawn -- nvim +{line} {file}'
```

Prefer the native forms (`wezterm`, `kitty`, `ghostty`) over the `osascript`
ones — they have no nested shell string to quote. Note that `kitty @` needs
`allow_remote_control yes` and `wezterm cli spawn` needs a running mux, so
either can fail on a default install.

### Safety

The template is split into arguments **once**, before any path is put into it,
and nothing is ever handed to `sh -c`. A file path containing a space, a quote
or a `$` therefore cannot change how the command is split — there is nothing
left to split by the time it arrives.

The editor is started detached, with stdin/stdout/stderr nulled so it cannot
draw over the TUI, and reaped so no zombie is left behind. A missing or failing
command is reported on the status row; recon keeps running either way.

Full reasoning: `docs/specs/2026-08-22-opening-an-editor.md`.

---

## Syntax colouring

Source files are coloured by their grammar — keywords, strings, comments,
types — using [syntect](https://crates.io/crates/syntect) with
[bat](https://github.com/sharkdp/bat)'s grammar and theme bundles via
[two-face](https://crates.io/crates/two-face). Around 150 languages are
recognised, by extension first (`.rs`, `.swift`, `.toml`), then by file name
(`Makefile`, `Dockerfile`, `.zshrc`), then by shebang. A file no grammar
claims, a `.txt`, a binary file and a directory listing all render exactly as
before.

`.log` files are deliberately plain. bat's bundle does have a `log` grammar,
but it is generic — it colours bare numbers, dates, IPv4 octets, quoted
strings, `key=value` and URLs, whatever the log's own format — and on a
free-form log that is stray yellow numbers and green quotes rather than
structure. It is a fixed list in `syntax::PLAIN_EXTENSIONS` for now; a
per-extension setting is the intended replacement once there is a config shape
for it.

### Choosing a theme

```sh
recon --theme Dracula src/         # a bundled theme
recon --theme ~/themes/Nord.tmTheme # any TextMate theme file
recon --theme none                 # off
```

The same setting lives in `config.toml`, under the CLI and `RECON_THEME`:

```toml
[syntax]
theme = "gruvbox-dark"
```

`recon --help` lists the bundled themes; names match case-insensitively. A
theme file is Sublime's older `.tmTheme` format — an XML plist — which is what
bat's `assets/themes/`, the dracula/sublime and catppuccin/sublime-text repos,
and most VS Code and TextMate themes distribute. The newer
`.sublime-color-scheme` JSON is not read.

The default is `ansi`, which names your terminal's own sixteen colours rather
than fixed RGB values: keywords take the terminal's magenta, comments its
green, and plain text its default foreground. That is the same choice the
navigator's blue and green make — the result follows your terminal theme
instead of fighting it, works on a light background as well as a dark one, and
needs no truecolor support. `base16` is the same idea with a fixed foreground.
Every other bundled theme paints 24-bit colour and expects a background it
does not paint, so pick one that suits your terminal's.

### What stays on top

Colouring is the lowest layer. A line a filter has coloured or dimmed keeps
the filter's colour over its whole width — the filter colour is the
information, and colouring a dimmed line would un-dim it. A search hit is
black-on-yellow across the whole match, whatever it lands on. The cursor line
in the focused pane is the usual reversed bar.

### Large files

A grammar's state at line N depends on every line before it, so a file cannot
be coloured from the middle — and colouring a 10 MiB log whole would stall the
navigator for seconds on every arrow key. recon colours only the lines about
to be drawn, continuing from where the parser stopped when you scroll and
resyncing a short way above the target when you jump: `G` on a large file, or a
filter showing lines thousands apart, restarts the grammar 64 lines above each
landing point. A block comment or raw string opened further back than that is
coloured wrong until it closes — the same trade every editor makes on a jump.
Lines over 10,000 bytes are left plain; a minified file on one line is not
worth seconds of regex.

---

## Known Limitations

- **The navigator's file matching covers at most 64 patterns**, counted across
  every loaded set whether or not it is enabled, plus the live search. Above
  that the navigator's marking switches off — never wrong, just absent — while
  the view keeps filtering. A `filters.toml` with many sets can reach this.
- **Files are read entirely into memory — once, not twice.** `read_lines` in
  `src/widgets/fileview.rs` collects the whole file into a `Vec<String>`, and
  `Document` holds it. A multi-gigabyte log is fully resident, and there is
  still no streaming or memory-mapped path.

  It used to be resident *twice*: the view was handed its own `Vec<String>` of
  every visible line, so an unfiltered file cost about 3× its size on disk. The
  view now holds only a window of three screens
  (`docs/specs/2026-08-22-windowed-textarea-viewport.md`, github issue #7),
  which takes that to ~1.5×. Replacing `Document`'s own copy with a line-offset
  index is the remaining half, github issue #51.
- **Previews are bounded, full loads are not.** While the navigator has focus,
  only `PREVIEW_LINES` (50,000) lines or `MAX_PREVIEW_BYTES` (10 MiB) are read,
  whichever comes first. The full read happens once you focus the view.

  The caps are set well above ordinary files on purpose. A truncated document
  is one that filters and line counts report *wrong* answers over — the status
  bar marks it `(preview)`, but the honest fix is not to truncate a file that
  costs a millisecond to read. Read, document clone and a filter pass together
  run about 1 ms/MB, so 10 MiB bounds the worst case near 10 ms. This is
  github issue #27.
- **Binary files are not viewable.** A file with a NUL byte in its first 8 KiB
  renders as `<binary file: contains NUL bytes>` rather than as bytes. Text
  that merely holds an undecodable byte here and there is read normally: each
  bad sequence becomes a `�` in place and every other line survives intact, so
  one corrupt byte in a log costs itself and nothing else. This is github
  issue #70.
- **Almost nothing is configurable yet.** recon reads
  `$XDG_CONFIG_HOME/recon/config.toml`, falling back to
  `~/.config/recon/config.toml` on every platform including macOS, under a
  `CLI > env > file > defaults` precedence chain. The settings so far are the
  two editor templates below, `[filters] palette` and `[syntax] theme`; every other key in the
  file is reported as an unknown key. Settings land one issue at a time
  against github issue #18; see
  `docs/specs/2026-08-22-configuration-mechanism.md` for the rules and the list
  of candidates.
- **Nothing is persisted.** Filter sets live only for the session — there is no
  way to save or reload a filter set. Re-typing them is the only option after a
  restart.   This is github issue #8
- `n` and `N` are line-oriented: a line matching the search three times is one
  stop, not three. `recon` is a line-focused tool, and one rule for filter hits
  and search hits alike beats two.
- Only the live search highlights the matched text within a line. Numbered
  filters colour the whole line — the vendored `TextArea` holds one search
  pattern, so extending spans to every filter needs more work in the fork.
- **Bundled themes other than `ansi` and `base16` emit 24-bit colour.** recon
  does not detect `COLORTERM` and never approximates an RGB value with the
  nearest of 256, so on a terminal without truecolor those themes render
  wrong. Stay on `ansi`, `base16` or `base16-256` there.
- **Syntax colouring resyncs on a jump** — see [Large files](#large-files).
  Colour after `G` on a big file can be wrong inside a multi-line construct
  until it closes; scrolling there from the top is always exact.

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
cargo test               # recon's suite: 713 unit + 13 integration tests
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
| `src/filter.rs` | `ActiveFilters` — the filter stack and its evaluation |
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
