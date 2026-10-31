# ADR 0014: the scroll bar drags the view, not the cursor

**Status:** accepted
**Date:** 2026-10-31

## Context

Long lines in the file view did not wrap; scrolling sideways existed only
implicitly, as the textarea keeping the cursor on screen. There was no way
to look at the right-hand end of a line without moving the cursor there —
and the cursor is not decoration. It is the anchor of meaning: what `y`
yanks, the word under the cursor, and where a search started from all hang
off it. The first shape of a scroll bar — pointer down, drag the cursor
along the line — would turn a glance sideways into a silent change of that
anchor, so a yank after a look would take the wrong word.

## Decision

- **The drag scrolls the view and never moves the cursor.** The vendored
  `tui-textarea-2` fork gains a caller-owned horizontal offset
  (`set_scroll_col`, #577): while pinned, the viewport column is the
  caller's — clamped so the widest line still reaches the edge — and the
  cursor's screen position, yank and word semantics are untouched. The
  fork was already patched and unpublishable, so a setter there is the
  cheap place; `PATCH.md` records it, and the fork's tripwire holds:
  `cursor.rs` movement logic, `screen_map.rs` and `wrap.rs` are not
  touched.
- **The cursor takes the view back.** Explicit landings (a click on text,
  a search hit) release the pin, because `set_cursor_position` belongs to
  the cursor. Column motions (`h`, `l`, word, `0`, `$`) release it in the
  file view: the cursor has spoken about where it is, so the view follows
  it as it always did. Vertical moves (`j`, `k`, the wheel) keep it:
  reading down a file preserves where sideways you left it.
- **The bar tells the truth about the visible lines.** The thumb's extent
  comes from the widest *visible* line, cached per generation of the
  visible set (#579): hide or filter the long lines and the bar shrinks
  with them rather than stranding a thumb near the left edge.
- **No overhang, no bar.** When nothing is cut off the bar is not drawn
  and its row stays the text's, so files that fit the width never lose a
  row.

## Considered options

- **Drag moves the cursor** (the cheapest implementation, no fork change).
  Rejected: it silently re-points the yank/word/search anchor to make a
  view change, confusing looking with selecting.
- **Keep the pin through cursor motion.** Rejected: after a drag the
  cursor can sit off screen; a motion key would then do something the view
  cannot show. Returning the view to the cursor on any column motion is
  what the implicit cursor-pinned scroll did before the bar existed.
- **Clamp the cursor to the window edge on drag.** Rejected: it moves the
  cursor by hundreds of columns, which is the same anchor breakage as the
  first option, just bounded.

## Consequences

- The cursor can be off screen while a pinned column is held. That is the
  point, not an oversight: the user is looking somewhere the cursor is
  not, and any column motion or landing brings the view back. The terminal
  cursor simply does not paint in that frame.
- The pinned column is one more piece of view state: the thumb is drawn
  from `scroll_top().1` as its only source, so search landing, clicks and
  drags all agree on where the view is without extra bookkeeping.
- A future keyboard scroll (`zs`-style, or configured keys) is a second
  caller of the same setter and needs no new mechanism.
