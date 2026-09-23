# ADR 0001: A search is a motion, not a filter

**Status:** accepted
**Date:** 2026-09-23
**Supersedes:** `docs/specs/2026-08-21-search-as-a-filter-design.md`

## Context

The 2026-08-21 design collapsed search into the filter set: `/` set a filter
in a reserved slot, so a search survived file loads, answered to `!` and `u`,
lost to an exclude, marked files in the navigator, and got a row in the filter
pane. The gain was one mental model and a cheap "probe, then keep with `p`"
workflow.

In use, the cost showed up in hide mode. A search *adds* lines: a searched
line counts as interesting, so `/foo` on a filtered view pulls in every `foo`
line the filters had removed. The user wanted the opposite: to find `foo`
among the lines already on view, the way `/` works in a vim buffer. The
filter model cannot express that, because a filter decides what a line *is*,
and a search is about where the cursor *goes*.

The search was also not incremental. Nothing happened until Enter, so a
pattern could not be judged while it was typed.

## Decision

A search is a cursor motion with a highlight. It is not a filter.

- `/` (and `*`) runs over the **visible lines** of the current file only. A
  hit is always a line the user can reach. Dimmed lines are visible; hidden
  lines are not.
- The search is **incremental**: each keystroke moves the cursor and the
  window to the first hit at or after the origin line, and highlights the
  hits in the window. Esc returns to the origin. Enter keeps the position.
- A search never changes which lines are visible, never dims, never marks a
  file in the navigator, and does not answer to `!` or `u`. `--emit lines`
  ignores it.
- `n` and `N` step hits while a search is set, one stop per line, wrapping
  within the file. With no search they step interesting lines and cross
  files as before.
- The pattern survives a file load, so `n` works in the next file. Esc after
  Enter clears the pattern and the highlight.
- **`p` is the bridge.** It turns the search into a numbered include filter
  and clears the search. The probe-then-keep workflow stays; it costs one key
  more in the case that used `u` to collapse to a bare search.
- The filter pane has no search row. The status row shows the pattern.
- Matching stays case-sensitive, as for filters. Smartcase is a separate
  decision and applies to both or neither.

## Consequences

- One meaning for `/` in every pane: find, in what you see. The navigator's
  filename search takes the same incremental rules.
- `/foo` then `u` no longer collapses to the `foo` lines. `/foo` `p` `u` does.
- The reserved search slot, the `Searched` verdict and the pane's search row
  go. Everything that read them (`any_including`, `recompute_visible`, the
  navigator's marks) reads only numbered filters.
- Per-keystroke cost is bounded by an early stop at the first hit and by
  highlighting window rows only. No count of hits is kept; that is a later
  issue if it is missed.
- The 2026-08-21 spec is marked superseded. Its reasoning about the two old
  models is still the best account of why one of them had to go; this ADR
  picks the other one.
