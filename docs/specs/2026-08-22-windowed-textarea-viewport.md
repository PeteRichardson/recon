# Windowed `TextArea` viewport — design

**Status:** implemented
**Date:** 2026-08-22
**Issue:** #7 (Half A)

## Motivation

The file is resident **twice**, and nothing had counted the second copy.

`Document` owns a `Vec<String>` of every line. `App::apply_view` then builds a
*second* `Vec<String>` — `document.visible_lines()` — and hands it to
`TextArea::set_lines`. With no filters, "visible" is the whole file, so
`TextArea` holds its own complete copy.

`Vec<String>` costs roughly 1.5× the bytes: a 24-byte header in the vec plus a
separately allocated, allocator-rounded heap buffer per line. An 80-byte log
line costs about 112 bytes. Two copies of that is ~3×:

| File | Resident today |
|---|---|
| 10 MB | ~30 MB — fine |
| 100 MB | ~300 MB — fine |
| 1 GB | ~3 GB — painful |
| 4 GB | ~12 GB — dead |

The second copy is pure waste: only the rows actually on screen are ever drawn.

**This spec removes the second copy, taking 3× → 1.5×.** `Document` keeps its
`Vec<String>`. Replacing *that* with a line-offset index is Half B, split out as
#51 and backlogged; it requires this work to exist first, or its re-read path
would materialise the whole file and be strictly worse than today.

Two things this deliberately does **not** do, both decided in #7 and restated so
a future reader does not relitigate them:

- **No mmap, in any form.** recon holds the file open while you browse it. If
  logrotate truncates the file under a mapping, touching a page past the new EOF
  raises **SIGBUS** — a signal, not a `Result`, with no clean way to catch it in
  a TUI. `greep` gets away with mmap because it runs and exits.
- **No size threshold and no second code path.** A rarely-taken branch rots
  untested. One path serves every file; see "Small documents are unwindowed by
  construction" for why that costs nothing.

## The window

`TextArea` is fed a contiguous slice of the visible set rather than all of it.

```
visible set (13,000,000 rows)
        │
        │   ┌───────────────┐  slack — one screen above
        │   ├═══════════════┤  ◄── viewport: what is actually drawn
        │   └───────────────┘  slack — one screen below
        │
   window = 3 × pane height
```

Two numbers define it, both on `FileView`:

- **`window_start`** — the visible-set index of the buffer's first row. The
  buffer's row *r* is visible row `window_start + r`.
- **`last_height`** — the height of the area the pane last rendered into,
  recorded by `render`.

### Why three screens rather than exactly one

A window of exactly the viewport would mean **every single `j`** moves the cursor
off the buffer's end, forcing a rebuild. Each rebuild calls `set_lines`, which
resets the viewport and re-enters the whole `pending_screen_row` /
`apply_pending_scroll` dance that exists to stop the view lurching. Doing that
per keystroke would fight the most delicate machinery in the app on the hottest
path there is.

With a screen of slack either side, ordinary movement — `j`, `k`, `Ctrl-E`,
`Ctrl-Y`, and a page in either direction — happens **entirely inside the buffer**
and `TextArea` handles it exactly as it does today. Today's behaviour is
preserved by construction rather than re-implemented.

The extra memory is nothing: three screens is ~72 lines on a 24-row terminal.

### The middle-third rule

> Re-window whenever the cursor leaves the **middle third** of the window,
> unless that side of the window is already the end of the document.

This is not a heuristic; it is what makes page-sized movement correct.

The middle third is one screen tall, so the rule guarantees **at least one full
screen of buffer beyond the cursor in each direction**. Any single move of at
most a page therefore completes inside the buffer before the next re-window —
`TextArea` never clamps it short.

A looser margin breaks exactly there. With a half-screen margin, the second
consecutive `PageDown` runs into the buffer's end and is silently truncated: the
page moves less than a page, and the user sees a stutter with no explanation.

Re-windowing is otherwise cheap — it clones about 72 strings — so there is no
reason to be stingy with the trigger.

### Small documents are unwindowed by construction

When the visible set is no longer than the window, the window **is** the whole
visible set: `window_start` is 0, every row is in the buffer, and the middle-third
rule can never fire because both sides are the document's ends.

So a file shorter than 3× the pane height behaves bit-for-bit as it did before
this change — same buffer, same cursor arithmetic, same scroll machinery. That is
what lets a single code path serve every file without a threshold: the small-file
case is not a *branch*, it is the general case with the window degenerating to
everything.

It is also why the existing suite is a real safety net here rather than a
formality. Most fixtures are far shorter than 72 lines, so they exercise the
unwindowed degenerate case and would catch any arithmetic that is wrong at
`window_start == 0`.

## Four keys move further than a page

Everything in `FileView::handle_events` moves the cursor by at most one page —
except four, which can travel an unbounded distance through the buffer:

| Key | Move | Windowed, without help |
|---|---|---|
| `g` / `Home` | `CursorMove::Top` | jumps to the top of the *window* |
| `G` / `End` | `CursorMove::Bottom` | jumps to the bottom of the *window* |
| `}` | `CursorMove::ParagraphForward` | stops at the window's edge |
| `{` | `CursorMove::ParagraphBack` | stops at the window's edge |

`^`, `0`, `$` and `w` are horizontal and unaffected.

These four are **intercepted in `App::handle_event`** before the event reaches
the widget, resolved against the whole visible set, and applied through
`jump_to_visible_row`, which re-windows and places the cursor in one step.

Interception in `App` rather than in the widget follows the existing precedent:
`n`/`N` already bypass `FileView::handle_events` for the same reason — the widget
cannot see the document, and these moves are only definable against it.

**The cost is real and worth naming:** the file view's bindings now live in two
places. That is the drift hazard #25 is about, and it is why the four keys are
listed in one table in `App` next to the interception, not scattered.

Paragraph moves are resolved against `document.lines()`, which costs nothing
extra: this work removes `TextArea`'s copy, not `Document`'s. Half B is what
makes that text unavailable, and it will have to solve paragraph moves its own
way.

## What `apply_view` sends

Four per-row vectors are handed to the view on every pass. All four are now
windowed, and the styles vector is the one that mattered most — it was a
full-length `Vec<Option<Style>>` rebuilt on **every navigator arrow key**, even
unfiltered.

| Sent | Was | Now |
|---|---|---|
| lines | `visible_lines()` — whole visible set | `visible_lines_range(start, end)` |
| styles | `visible_styles()` — whole visible set | `visible_styles_range(&filters, start, end)` |
| gutter numbers | `visible().to_vec()`, only while hiding | `visible()[start..end].to_vec()`, **always** |
| group ends | `visible_group_ends()`, only while hiding | `visible_group_ends_range(start, end)`, only while hiding |

### Gutter numbers can no longer be omitted

Previously the numbers override was skipped unless hiding, because with the whole
file on screen the fork's natural 1..N numbering of the buffer is already right.

**Windowed, it is wrong.** A buffer starting at visible row 1,000 would be
numbered 1, 2, 3… by natural numbering. The override is now always supplied.

The reason the gate existed — "a vector the length of the file, rebuilt on every
navigator arrow key, to say nothing" — no longer applies: the vector is now the
length of the *window*.

### Group ends must look one row past the window

`visible_group_ends` marks a row when the next source line is hidden, and treats
the final visible row specially — a gap only if the file continues below it.

Sliced naively, the window's last row would be mistaken for the document's last
row and marked wrong. `visible_group_ends_range` therefore peeks at
`visible[end]` when it exists, so a row's mark never depends on where the window
happens to stop.

## Cursor position is a visible-set index

`textarea.cursor().0` is now an index into the **window**, not the visible set.
Everything that reads it goes through one translation:

```
visible row = window_start + textarea.cursor().0
source line = document.source_at(visible row)
```

`App::cursor_source` and the test helper of the same name both do this. Getting
it wrong is silent — it produces an off-by-`window_start` line number — so the
translation lives in `FileView::cursor_visible_row()` and nothing outside reads
`textarea.cursor().0` for a vertical position.

## The rebuild-skip cache

`apply_view` skips `set_lines` when the visible set is unchanged, so that a
filter change leaving the same rows on screen does not reset the scroll position.

That cache is now keyed on the **window as well**: an unchanged visible set with
a moved window still needs a rebuild. `last_visible` gains a companion
`last_window`, and both must match for the rebuild to be skipped.

Omitting this would be the subtlest possible bug — scrolling into a new window
would silently keep showing the old rows.

## `metadata.len()` is a hint, not a promise

Carried over from #7's reading of `greep` as the one lesson that survives the
no-mmap decision: a special file can report a size it never delivers, and a bare
`as usize` cast truncates on 32-bit targets. Nothing here newly depends on it —
`read_preview` already reads `file.metadata()` for `estimate_lines` — but any
sizing arithmetic added later should assume the number can lie.

## Testing

Per #7's acceptance, the memory win is pinned **structurally** rather than
measured. Asserting on real resident bytes needs a large fixture and is
allocator- and platform-dependent — the classic flaky test.

The property that causes the win is directly assertable: *the buffer holds at
most a window, however long the document is.* A document of several thousand
lines rendered into a known pane height must leave `textarea.lines().len()`
bounded by the window span while `document.visible().len()` stays at the full
count.

The rest of the acceptance — "cursor position, scrolling and `CursorMove::Jump`
behave as they do today" — is pinned by the existing suite, which runs almost
entirely in the unwindowed degenerate case, plus new tests that drive a document
long enough to window and check that `g`, `G`, `{`, `}` and repeated paging land
on the same rows they would without windowing.

## Deliberately not covered

- **Half B**, holding a line index instead of the lines (#51). Needs this first.
- **Background reading.** `evaluate` needs line text, so every filter change is
  already a full in-memory pass. Half B turns that into a disk re-scan per
  keystroke and forces threading; this change does not, because `Document` still
  holds the text.
- **A progress indicator** during a full read — #27's territory.
