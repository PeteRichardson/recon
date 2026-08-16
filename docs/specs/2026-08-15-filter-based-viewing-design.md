# Filter-based viewing — design

**Status:** proposed
**Date:** 2026-08-15

## Motivation

`recon` is a log viewer. The workflow it does not yet support is the one
TextAnalysisTool.NET is built around:

1. Define a filter for the lines you care about (e.g. containing `foo`).
2. Hide everything else.
3. Scroll to the line you want.
4. Show everything again — and land on **that exact line, in full context**.

Step 4 is the point. The filtered view is a *navigation aid*, not a destination:
you use it to find a position in the file, then return to the unfiltered file at
that position. Nothing on macOS does this well.

## What TextAnalysisTool.NET actually does

Confirmed from <https://textanalysistool.github.io/> and the author's notes:

- Lines pass through a set of user-defined filters before display. Lines
  matching no filter are **dimmed** (not hidden) by default.
- **Ctrl+H** — "Show Only Filtered Lines" — flips dimmed to hidden, and back.
  A funnel indicator in the status bar shows which mode is active.
- Filters match a **substring**, a **regular expression**, or a **marker type**.
- Each filter carries **text and background colours**, so lines matching
  different filters are distinguishable at a glance.
- **Excluding** filters are configured like including ones but applied
  afterwards, removing matching lines from the result.
- Filters can be **saved and loaded** as sets (e.g. one set per log format).
- Lines can be tagged with one of **eight markers**, navigated between, and
  filtered on.
- Number keys toggle individual filters on and off.

## The constraint that drives the design

The file view is currently a `tui_textarea::TextArea`. Its public API exposes
only whole-area style, cursor-line style, search style and line-number style.
`LineHighlighter` — which does have per-line styling — sits behind a private
`mod highlight`.

**There is no way to dim or colour an individual line through `TextArea`.**

Since dimming with per-filter colours *is* the feature, that gap has to be
closed. Two options were weighed: reimplement the viewer, or patch the crate.

Reading `TextArea::line_spans_segment` settles it. The per-line primitive is
already there and already used — for the cursor line:

```rust
if wrapped.row == self.cursor.0 {
    hl.set_line_style(self.cursor_line_style);   // per-line styling, internal
```

and line numbers already come from the **source** row, not the screen row:

```rust
hl.line_number(wrapped.row, lnum_len, style);
```

So the crate will be **forked with a minimal patch** — two public setters —
rather than replaced:

| Addition | Drives | Change |
|---|---|---|
| `set_line_styles(Vec<Option<Style>>)` | dimming, filter colours | feed the existing `hl.set_line_style` from user data |
| `set_line_numbers(Vec<usize>)` | source numbers while filtered | override the argument to `hl.line_number` |

**Hiding needs no change to the crate at all.** In `FilteredOnly` mode the
textarea is rebuilt from just the matching lines, and their source numbers are
supplied via the second setter. The cursor stays an ordinary index into
whichever buffer is loaded; `Document` owns the source↔visible mapping. Cursor
movement, the screen map and the wrap logic — the parts that would be
genuinely risky to touch — are untouched.

Kept for free: every motion, scrolling, search with wrap-around and match
highlighting, line wrapping, and viewport management. Reimplementing those was
the largest risk in this design, and forking removes it.

**Fork hygiene.** `tui-textarea-2` is itself a fork of rhysd's `tui-textarea`,
so this adds a third link to the chain. To keep that from festering: hold the
patch to these two setters, and offer it upstream — if accepted, the fork
disappears. If the patch ever needs to grow beyond per-line presentation, that
is the signal to revisit the from-scratch viewer instead of deepening the fork.

The fork is carried as a **vendored copy** at `vendor/tui-textarea-2`, wired in
with `[patch.crates-io]`. That keeps the dependency line in `Cargo.toml`
unchanged, makes the diff from upstream reviewable in-repo, and needs no
published fork to exist before work can start. Switching to a git dependency
later, once the fork is pushed, is a one-line change.

The editing machinery (insert, delete, undo) remains unused in a read-only
viewer. It is dead weight, but harmless, and cheaper than replacing what it
comes bundled with.

## Architecture

### `Document`

Owns the file's lines and everything derived from them.

```rust
struct Document {
    path: PathBuf,
    lines: Vec<String>,
    /// Per-line filter outcome, recomputed only when lines or filters change.
    verdicts: Vec<Verdict>,
    /// Source line indices currently visible, in order.
    visible: Vec<usize>,
}

enum Verdict {
    /// Matched an including filter; carries which one, for colouring.
    Included(FilterId),
    /// Matched no including filter.
    Unmatched,
    /// Removed by an excluding filter; never visible in either mode.
    Excluded,
}
```

`verdicts` is a cache: evaluating filters is O(lines × filters), which is
noticeable on a large log, and toggling Ctrl+H must not re-evaluate anything.
It is invalidated when the file is loaded or the filter set changes.

### `Filter`

```rust
struct Filter {
    id: FilterId,
    kind: FilterKind,      // Substring(String) | Regex(Regex) | Marker(MarkerId)
    sense: Sense,          // Include | Exclude
    enabled: bool,
    style: Style,          // fg/bg, as in TextAnalysisTool
    hits: usize,           // matching line count, shown in the filter pane
}
```

Evaluation order, matching TextAnalysisTool exactly:

1. If no enabled including filters exist, every line is `Unmatched`
   (i.e. an empty filter set shows a normal file, undimmed).
2. Otherwise a line is `Included` if it matches any enabled including filter,
   and `Unmatched` if it matches none.
3. Enabled excluding filters are applied afterwards; a line matching any of
   them becomes `Excluded` regardless of step 2.

A line matching **several** including filters takes the colours of the first
one in list order, and `Included` carries that filter's id. Order in the
filter pane is therefore meaningful, and reordering is a natural later
addition.

### `LineView` — the Ctrl+H round trip

This is the heart of the feature and deserves being explicit.

**The cursor is always stored as a source line index**, never as a position in
the visible list. Everything else is derived:

```rust
enum Mode { Dimmed, FilteredOnly }   // Ctrl+H toggles
```

- `Mode::Dimmed` — `visible` is every non-`Excluded` line. `Unmatched` lines
  render dim; `Included` lines render in their filter's colours.
- `Mode::FilteredOnly` — `visible` is only `Included` lines.

Toggling recomputes `visible` and re-derives the screen position from the
unchanged cursor. Because the cursor is a source index, step 4 of the workflow
is exact by construction: you return to the line you were on.

Toggling *into* `FilteredOnly` while the cursor sits on an unmatched line is
the one ambiguous case: the cursor snaps to the nearest `Included` line at or
after it (falling back to the one before, if none follows). Toggling back then
lands on that line — which is the behaviour you want, since the match is what
you were navigating towards.

The scroll offset is chosen to keep the cursor on the **same screen row**
across a toggle where possible, so the view does not jump under you.

### `FileView` widget

`FileView` keeps its `TextArea`, now driven by `Document`:

- **`Dimmed`** — the textarea holds every non-`Excluded` line. `set_line_styles`
  supplies one entry per line: the matching filter's colours for `Included`,
  `DIM` for `Unmatched`. Line numbers are the natural ones.
- **`FilteredOnly`** — the textarea is rebuilt from `Included` lines only, and
  `set_line_numbers` supplies their source numbers so the gutter reads
  2, 4, 9… rather than 1, 2, 3.

Search keeps working as it does today, since it is the crate's own. It
therefore operates on whatever is loaded, which gives the right behaviour for
free: in `FilteredOnly` mode it cannot jump to a hidden line, because hidden
lines are not in the buffer.

Rebuilding on toggle clones the matching lines. That is bounded but not free
on a large log, and should be measured the way preview reads were; if it
bites, the fix is to keep both buffers and swap between them.

### Layout

The left column splits horizontally: file nav on top, filters beneath.

```
┌ ~/logs ────────┐┌ app.log ──────────────────────────┐
│   ..           ││   1 2026-08-15 starting up        │  ← dim
│ >>app.log      ││   2 2026-08-15 foo connected      │  ← yellow
│   old.log      ││   3 2026-08-15 unrelated chatter  │  ← dim
├ Filters ───────┤│   4 2026-08-15 foo disconnected   │  ← yellow
│1[x] inc "foo"  ││                                   │
│2[ ] exc "hb"   ││                                   │
└────────────────┘└───────────────────────────────────┘
▼ 2/4 shown   app.log   line 2 of 4
```

- The column width snaps to whichever stacked pane needs more, still capped at
  `MAX_NAV_WIDTH`, and the drag/double-click behaviour is unchanged.
- The filter pane sizes to its contents and **collapses entirely when no
  filters are defined**, so it costs nothing to users who never define one.
- The horizontal divider between the two is draggable, reusing the existing
  divider machinery generalised to both axes.
- `Tab` cycles three panes; the filter pane is skipped while collapsed.

### Filters persist across files

Filters survive loading a different file, because a filter set describes a
*log format*, not a document. `!` toggles the whole set off and back on: it
disables every enabled filter, remembering which were enabled, and the next
press restores exactly those. That is the single keystroke back to an
unfiltered file, without losing the work of defining them.

### Status line

The bottom row becomes a persistent status line (currently the search prompt
borrows it transiently, which continues to take precedence):

`▼ 2/4 shown   app.log   line 2 of 4`

The `▼` funnel is the Ctrl+H indicator, mirroring TextAnalysisTool. Without
this there is no feedback about *why* lines vanished, which makes the mode
confusing.

## Line numbers

Independent of everything above, and worth doing first as it is trivial today:

`#` toggles the line-number gutter, via `remove_line_number()` /
`set_line_number_style()`. **Already implemented**, ahead of the rest of this
design, since it needed nothing from the fork.

## Keybindings

New, chosen to avoid the existing set:

| Key | Action |
|-----|--------|
| `#` | toggle line numbers |
| `Ctrl-h` | toggle dimmed ↔ filtered-only |
| `f` | add filter prompt (`f` then pattern, as `/` works today) |
| `F` | focus the filter pane |
| `1`–`9` | toggle filter *n* (filter pane only — see below) |
| `x` | flip include/exclude (filter pane) |
| `d` | delete filter (filter pane) |
| `space` | enable/disable filter (filter pane) |
| `!` | disable all filters / restore them |

Unchanged: `h j k l w b e 0 ^ $ { } g G n N / ? Ctrl-f Ctrl-b Ctrl-d Ctrl-u
Ctrl-e Ctrl-y Tab q`.

Two collisions to settle before implementing:

- **`Ctrl-h` vs Backspace.** Some terminals send `Ctrl-h` for Backspace, and
  the search prompt uses Backspace. They must be disambiguated by `KeyCode`
  rather than by byte, and `Ctrl-h` must be inert while a prompt is open.
- **Digits.** `0` is already bound to start-of-line, so binding `1`–`9` to
  filter toggles makes the digit row mean two unrelated things. Options:
  accept it (vim itself overloads `0` against counts); move filter toggles to
  the filter pane only; or use `Alt-1`–`Alt-9` globally. **Recommended:**
  digits toggle filters only while the filter pane has focus, leaving the
  file view's `0` untouched.

## Suggested additional features

Asked for; each is independently useful and independently droppable.

**Recommended**

- **Markers** — mark lines with `m1`–`m4`, jump between same-marked lines, and
  filter on a marker. Pairs naturally with filters and gives a way to collect
  lines that no single pattern describes. TextAnalysisTool offers eight; four
  is the starting point here, since a terminal has far fewer reliably
  distinguishable colours. Adding more later is trivial.
- **Filter sets, saved and loaded** — the feature that makes the tool pay off
  across sessions: one set per log format, reloaded whenever you open that
  kind of file. Suggested `~/.config/list/filters/<name>.json`.
- **Convert search to filter** — after `/foo`, a key promotes that pattern to
  a filter. The natural bridge from what exists today, and TextAnalysisTool
  has it for the same reason.
- **Goto line** (`:123`) — trivial once the cursor is a source index, and the
  obvious companion to correct line numbers in filtered mode.

**Worth considering**

- **Tail/follow** — not a TextAnalysisTool feature, but for a log viewer on a
  live file it is arguably more fundamental than anything above. Note `F` is
  taken by the filter pane above, so this would need its own key. Re-reads appended data and, when following, keeps the cursor at the
  end. Interacts well with filters: follow *only matching* lines.
- **Export visible lines** — write what you are looking at to a file or the
  clipboard. Cheap once `visible` exists, and the usual reason to want a
  filtered view is to hand it to someone else.
- **Case-insensitive / smartcase filters and search** — currently search is
  case-sensitive by regex default, which is often wrong for log spelunking.

**Deliberately excluded**

- Multiple files open at once, and timestamp-merging across files. Both are
  large, and neither serves the core workflow.
- Editing. The viewer stays read-only; the crate's editing machinery is
  carried but unused.

## Phasing

Each phase is independently shippable and leaves the app working.

1. **Fork and wire up** — fork `tui-textarea-2`, add the two setters, and
   switch the dependency over with no behaviour change. Existing tests must
   pass untouched, which is the whole point: the foundation is verified
   against known behaviour before anything is built on it. Offer the patch
   upstream at this point. (`#` for line numbers is already done.)
2. **Filters and Ctrl+H** — `Document`, `Filter`, verdict caching, dimming,
   filter colours, the filter pane, the status line. Delivers the workflow.
3. **Markers and filter sets** — persistence and marker filters.
4. **Tail/follow and export.**

## Testing

The existing approach carries over: render into a `Buffer` and assert on
cells, which already covers styling (it verified search highlighting by
inspecting `bg`).

- Filter evaluation: include-only, exclude-after-include, disabled filters,
  empty set showing an undimmed file.
- Verdict cache invalidated on file load and on any filter change.
- **Ctrl+H round trip** — the workflow, asserted directly: filter, toggle,
  move to the *n*th visible line, toggle back, assert the cursor is on the
  expected source line with the surrounding context present.
- Toggling into filtered mode from an unmatched line snaps forward, and back
  again lands on the snapped line.
- Line numbers show *source* numbers in filtered mode, not 1..N.
- Search restricted to visible lines in filtered mode.
- Filter pane collapses when empty; the left column widens for long patterns
  but respects the cap.

## Risks and open questions

- **Fork maintenance.** A third link in the tui-textarea fork chain. Held in
  check by keeping the patch to two setters and offering it upstream; growth
  beyond per-line presentation is the signal to reconsider.
- **Rebuild cost on toggle.** `FilteredOnly` clones the matching lines into a
  new textarea. Needs measuring on a large log; keeping both buffers is the
  fallback.
- **Wide characters.** The nav pane's width measurement uses `chars().count()`,
  which mis-measures CJK and emoji. Forking leaves body-text width handling
  with the crate, which already does it properly — so this stays a nav-pane
  issue rather than becoming a viewer one.
- **Very large files.** Filter evaluation is a full pass. On the 136 MB log
  measured earlier, a full read is ~4 s; a filter pass over lines already in
  memory is far cheaper, but re-evaluating on every keystroke of a filter
  prompt would not be. Evaluate on commit, not per keystroke.
- **Resolved:** filters persist across file loads — the point of a filter set
  is that it outlives one file — and `!` disables them all at once.
- **Resolved:** four marker types to start, not TextAnalysisTool's eight.
