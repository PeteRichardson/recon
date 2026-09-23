# Incremental search over the visible lines — design

**Status:** proposed
**Date:** 2026-09-23
**Decision record:** [ADR 0001](../adr/0001-search-is-a-motion-not-a-filter.md)
**Supersedes:** [Search as a filter](2026-08-21-search-as-a-filter-design.md)

## Problem Statement

`/` does two things the user does not want.

First, it waits. Nothing moves, highlights or reports until Enter. A pattern
cannot be judged while it is typed, so a near-miss regex costs a full
Enter, look, Esc, retype cycle.

Second, it adds. Because a search is a filter, a searched line counts as
interesting, and in hide mode a search *pulls in* every line the filters
had removed. The user hid those lines on purpose. They want to find a
pattern among the lines they can see, the way `/` works in a vim buffer,
not to widen the view.

The same model gives the search a row in the filter pane, a colour, and a
place in `!`, `u` and the navigator marks. Each of those is one more thing
to reason about for what is, in use, a cursor motion.

## Solution

A search is a motion with a highlight. It is not a filter.

- `/` moves as you type, to the first hit at or after your line, and
  highlights the hits in the window. Esc puts you back where you were.
  Enter keeps the position.
- The search looks only at the visible lines. In hide mode that is the
  survivors. Dimmed lines count; hidden lines do not.
- A search never changes which lines are visible. It does not dim, does
  not mark files in the navigator, and does not answer to `!` or `u`.
- `n` and `N` step hit lines while a search is set. With no search they
  step interesting lines, as today.
- `p` is the bridge: it turns the search into a numbered include filter.
- The filter pane loses its search row. The status row shows the pattern.
- Up and Down in the prompt recall earlier patterns.
- The navigator's filename search takes the same incremental rules.

The vocabulary is in `CONTEXT.md`: *search*, *hit*, *origin*, *promote*,
*visible lines*, *window*, *interesting line*.

## User Stories

### Typing a search

1. As a log reader, I want the cursor to move to the first hit while I type
   the pattern, so that I can see whether the pattern is right before I
   commit it.
2. As a log reader, I want the hits in the window highlighted while I type,
   so that I can see how many lines nearby the pattern touches.
3. As a log reader, I want Esc in the prompt to return the cursor and the
   scroll to where I pressed `/`, so that a bad probe costs nothing.
4. As a log reader, I want Enter to keep the position the search reached,
   so that a good probe is also the jump.
5. As a log reader, I want the search to start on the line I am on, so
   that a hit on my own line is found first and not after a wrap.
6. As a log reader, I want the search to wrap to the top when nothing is
   below me, and to say so in the status row, so that a jump upward is
   never a surprise.
7. As a log reader, I want a half-typed regex like `foo(` to be silent, with
   the cursor at the origin and no highlight, so that I am not shouted at
   between keystrokes.
8. As a log reader, I want Enter on an invalid pattern to report `E486` and
   keep the prompt open, so that I can fix the pattern where it stands.
9. As a log reader, I want Enter on an empty prompt, or Backspace past the
   first character, to cancel and return me to the origin, so that an empty
   search does nothing surprising.
10. As a log reader, I want the cursor to land on the column of the first
    occurrence in the hit line, so that a long line does not hide where the
    hit is.
11. As a log reader, I want the landing row placed by the existing
    `center_jumps` setting, so that `/` and `n` land the same way.

### What the search looks at

12. As a log reader in hide mode, I want the search to skip hidden lines,
    so that I find a pattern among the lines I kept and not among the ones
    I removed.
13. As a log reader in dim mode, I want the search to find dimmed lines,
    so that I can reach the context around a filter hit.
14. As a log reader, I want an excluded line never to be a hit, in either
    mode, so that `x` means gone.
15. As a log reader, I want a search to leave the visible lines exactly as
    they were, so that the gutter numbers and the gaps do not shift under
    me while I probe.
16. As a log reader, I want the search to run over a preview of a large
    file the same as over a whole file, so that skimming stays the same.

### Highlight and status

17. As a log reader, I want only the hit text painted, with the line keeping
    its filter colour or its dim grey, so that the context I searched into
    stays readable.
18. As a log reader, I want the hit highlight to stay on top of syntax
    colour, so that a hit inside a string is still visible.
19. As a log reader, I want the status row to show the pattern, marked `/`,
    while a search is set, so that I know what `n` will do.
20. As a log reader, I want no hit count in the status row, so that a huge
    file costs nothing extra per keystroke.

### Stepping

21. As a log reader, I want `n` and `N` to step between hit lines while a
    search is set, so that the search owns the motion I just made.
22. As a log reader, I want one stop per hit line, so that a line with three
    occurrences is one press and not three.
23. As a log reader, I want `n` to wrap within the current file, and to say
    so, so that a search never jumps me to another file.
24. As a log reader, I want `n` with no search set to step interesting lines
    and cross files, as today, so that the filter workflow is unchanged.
25. As a log reader, I want `n` to report `no hit for /pattern` and stay put
    when the file has no hit, so that the key does not silently do
    something else.

### Lifetime

26. As a log reader, I want the pattern to survive loading another file, so
    that `n` finds its hits in the next log without retyping.
27. As a log reader, I want loading a file to leave the cursor alone, so
    that a file load is not a jump.
28. As a log reader, I want Esc, outside a prompt, to clear the pattern and
    the highlight, so that one key ends a search.
29. As a log reader, I want Esc to clear the navigator's filename search
    before the file search when the navigator has focus, as today, so that
    a stale filename search cannot keep driving `n`.

### Filters and the search

30. As a log reader, I want `u` with a search and no include filter to do
    nothing, so that a search alone cannot hide the file.
31. As a log reader, I want `!` to leave the search alone, so that toggling
    the filters off does not lose my place.
32. As a log reader, I want the navigator to mark files from filters only,
    so that a search does not repaint the directory.
33. As a log reader, I want `--emit lines` to ignore the search, so that
    what leaves recon is what the filters chose.
34. As a log reader, I want `p` to promote the search into a numbered
    include filter and clear the search, so that a probe I like becomes a
    filter with one key.
35. As a log reader, I want `/foo` `p` `u` to collapse to the `foo` lines, so
    that the old `/foo` `u` result is still one key away.
36. As a log reader, I want `*` to set the search to the word under the
    cursor and move to its next hit, with the same scope and the same `p`,
    so that `*` is `/` with the typing done.
37. As a log reader, I want the filter pane to have no search row, so that
    the pane lists only filters.

### History

38. As a log reader, I want `/` to open an empty prompt, so that a new
    pattern starts fast.
39. As a log reader, I want Up (or Ctrl-P) in the prompt to recall the last
    pattern for editing, and again for the one before, so that a near-miss
    is corrected rather than retyped.
40. As a log reader, I want Down (or Ctrl-N) to walk back toward the newest
    pattern and then to an empty prompt, so that recall is reversible.
41. As a log reader, I want the file search and the filename search to keep
    separate histories, so that a filename never turns up in the file
    prompt.
42. As a log reader, I want the history capped at a small number and not
    saved across sessions, so that it costs nothing to keep.
43. As a log reader, I want the filter prompts (`i`, `x`, `c`) unchanged by
    this work, so that adding a filter is exactly as it was.

### The navigator

44. As a log reader, I want `/` in the navigator to move the selection as I
    type, so that a filename is found without Enter.
45. As a log reader, I want the view pane to follow the selection during a
    filename search, so that I see the file the pattern found.
46. As a log reader, I want Esc in the navigator's prompt to return the
    selection and the view to the origin row, so that a bad filename probe
    costs nothing.
47. As a log reader, I want the navigator's `n` and `N` to repeat the
    filename search as today, so that nothing else in the navigator
    changes.

### Selections

48. As a log reader, I want `/` during a `v` or `V` selection to grow the
    selection to the hit as I type, so that "from here to the next ERROR"
    is one search and one yank.
49. As a log reader, I want Esc in that prompt to restore the selection as
    it was, so that a bad probe does not lose the selection.

### Keys and configuration

50. As a user with a custom keymap, I want `global.search`,
    `global.search.promote`, `global.search.word`, `hit.next`, `hit.prev`
    and `prompt.commit` to keep their names, so that my `[keymap]` still
    applies.
51. As a user with a custom keymap, I want new actions for history recall
    to be rebindable under the prompt scope, so that they follow the same
    rule as every other key.
52. As a reader of the in-app help, I want the `/`, `p`, `Esc` and `n` rows
    to describe the new behaviour, so that the help does not lie.

## Implementation Decisions

### The search state

- The reserved search slot in the active filters, the `Searched` verdict,
  and the remembered-search flag that `!` captured are removed. Every site
  that read them (any-including, visible recomputation, match counts,
  navigator marks, style lookup, the emit pipeline) reads numbered filters
  only.
- The application holds the search as its own state: an optional compiled
  pattern plus its source text. It is not part of the filter set and is not
  saved with a filter set.
- The prompt keeps the origin: the file view's cursor and scroll, or the
  navigator's selected row, captured when `/` opens. While the prompt is
  open, every edit re-runs the search from the origin, never from the
  position the last keystroke reached. Esc restores the origin. Enter drops
  it.
- The prompt's kind already distinguishes the file search from the filter
  prompts. The navigator's search is a third kind, or the same kind
  resolved by focus, whichever the existing code does with the least
  change. Filter prompts are untouched.

### The scan

- A hit is a visible line the pattern matches. The scan walks the visible
  lines from the origin row forward, wraps once, and stops at the first
  hit. It never walks the file's source lines directly.
- The scan returns the visible row and the column of the first occurrence.
  The cursor is placed there, and the landing row obeys `center_jumps`.
- The highlight is the window's job. The file view already takes a pattern
  and paints span highlights for the rows it renders; that mechanism stays
  and now takes the search pattern directly, not the enabled search filter.
- No hit count is kept. The scan's early stop is a property to preserve.

### Stepping

- With a search set, `n` and `N` step to the next or previous hit line
  among the visible lines, wrap within the file, and say so. With no
  search, they step interesting lines and cross files exactly as today.
  The two paths share the wrap message.
- One stop per line. The cursor lands on the first occurrence's column.
- When the file has no hit, the status row reports it and nothing moves.

### Lifetime

- The pattern survives a file load. The load neither scans nor moves.
- The global Esc clears the pattern and the highlight. Focus order is
  unchanged: with the navigator focused, its filename search clears first.
- `p` builds an include filter from the pattern text, appends it to the
  scratch set, clears the search, and re-evaluates. That is the existing
  promote path minus the slot.
- `*` sets the pattern to the escaped word under the cursor and runs the
  same scan from the cursor row.

### Drawing

- No whole-line search style. The line keeps the style the filters gave it.
  The span highlight is unchanged in colour and stays above syntax colour.
- The filter pane's search row and its hint text are removed. The pane's
  row list, key handling, mouse hit-testing and tests lose that variant.
- The status row shows `/pattern` while a search is set, alongside the
  HIDE and AND badges.

### History

- Two ring histories, one per prompt kind that has a search: file patterns
  and filename patterns. Newest first, capped at 50, in memory only. A
  pattern already in the history moves to the front rather than
  duplicating.
- Up / Ctrl-P and Down / Ctrl-N walk it while the prompt is open. Walking
  past the newest returns to an empty prompt. Each recall re-runs the
  incremental scan like any edit. Only Enter adds to the history.
- The two recall actions get keymap ids in the prompt scope and rows in the
  in-app help.

### The navigator

- The navigator's search uses the same prompt, origin and history
  mechanism. Its scan is over the listed entries, as today, and moves the
  selection. Moving the selection triggers the same preview load as a `j`.
  Esc restores the selected row and the preview.

### Selections

- A selection anchored before `/` extends to the hit because the cursor
  moves; nothing special is needed beyond restoring the selection's cursor
  on Esc together with the origin.

### Documentation

- README: the "Search is just a filter" feature bullet, the key table rows
  for `/`, `p`, `Esc` and `n`, the filter pane's search-row section, the
  "live search" paragraphs in the filter colour section, and the Known
  Limitations entry on line-oriented `n` are rewritten in the new
  vocabulary. The `--emit lines` text gains the line that a search does not
  affect it.
- The in-app help follows the key table.

## Testing Decisions

A good test presses keys and reads what the user reads: the frame, the
status row, the prompt line, the cursor's source line, and the visible
lines. It does not inspect the search state or the scan's internals.

**Seam:** the key-driven `App` tests in the library's test module. The
helpers that build an app over a fixture file, press a key, type text, draw
a frame, and read the prompt line and the screen are the prior art; the
existing `/` tests there (prompt opens, `p` promotes, Esc clears) are the
ones to rewrite first. Cases to cover:

- Move while typing; Esc returns cursor and scroll; Enter keeps them.
- The origin line's own hit is found first; wrap reports itself.
- Hide mode skips hidden lines; dim mode finds dimmed ones; an excluded
  line is never a hit.
- A search changes no visible line, no gutter number, no `u` result, no `!`
  result, no navigator mark, and no emitted line.
- `n`/`N` step hit lines, one stop per line, wrap within the file, and
  report no hit; with no search they cross files as before.
- Pattern survives a load and does not move the cursor; Esc clears it.
- `p` promotes and the filter colour replaces the highlight; `*` behaves as
  `/` with the word typed.
- History: Up recalls, Up again recalls the one before, Down returns to
  empty; the two prompts do not share.
- Navigator: selection moves while typing, the preview follows, Esc restores
  both.
- A selection grows to the hit and is restored on Esc.
- The filter pane shows no search row; the status row shows the pattern.

**Adjusted, not added:** the filter and document unit tests that cover the
search slot, the `Searched` verdict and remembered-search are removed or
rewritten. The viewport stepping tests gain the search case. The render
smoke tests cover the highlight-only line styling and the status row.

**Not added:** a direct unit seam on the scan. Every rule it has is
observable through keys.

## Out of Scope

- Smartcase or any case option. It applies to search and filters together,
  and is its own issue.
- A hit count or "3 of 41" indicator.
- Per-occurrence stepping within a line.
- Crossing files with a search `n`.
- Persisting history across sessions.
- History for the filter prompts.
- `?` backward search.
- Changes to the headless `--emit` pipeline beyond ignoring the search,
  which it already does once the slot is gone.
- Regex-set fusion of the search with the filters (issue #168 territory);
  the scan compiles the search on its own.

## Further Notes

- This reverses the 2026-08-21 design, which is marked superseded. Its
  motivation section is still the clearest statement of why two models
  had to become one; ADR 0001 records why this is the one.
- What the user gives up is exactly one keystroke: `/foo` `u` becomes
  `/foo` `p` `u`.
- The bounded preview of a large file is a visible-lines question, not a
  search question: the scan sees whatever is loaded, as `n` does today.
