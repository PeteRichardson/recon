# The minimap — design

**Status:** accepted
**Date:** 2026-10-24
**Related issues:** #523 (this design), #524 (the picture version, which
builds on it)

## Problem Statement

In a long file, the user cannot see where the filter matches and the
search hits are, or where the window is in the file. In dimmed mode the pane
can show 40 grey lines while all the matches are 3,000 lines further down.
Only `n`, one match at a time, finds them.

## Solution

A narrow **minimap** at the right of the file view's text. It shows the
whole file in the height of the pane:

```
 text text text │⠀▏     a thin rule, then the search column, then the filter column
 text text text ┃⣿█     the rows on screen: thick rule, grey background
 text text text │⠁▍
```

From the left, inside the pane's border, it takes 3 columns from the text:

1. **The rule.** A dark-grey `│`, and a light-grey `┃` on the rows of the
   band: the rows of the map that the window covers. It reads as a scroll
   bar, and the thick part is the thumb.
2. **The search column.** Braille: 4 dot rows in each cell, so the search
   hits are placed at 4 times the resolution of a map row. A dot row with
   hits shows 1 dot; with many hits — half or more of the busiest dot row —
   it shows 2. In the search highlight's yellow.
3. **The filter column.** A left-eighths bar, `▏` to `█`. Its width is the
   number of interesting lines in the group, compared with the busiest
   group. A group with 1 or more matches shows at least `▏`. Its colour is
   the filter with the most matches in that group; on a tie, the first
   filter.

The search and the filter columns have the dark-grey background on the
rows of the band.

A click on the map, or a drag that starts on it, moves to that part of the
file.

## Decisions

### What the map shows

- **Density, not presence.** The question the map answers is "where is the
  activity?" A bar that is wider where there are more matches answers it
  at a glance. A mark that only says "1 or more" does not.
- **Search and filters in different columns, with a different texture.**
  Two bar columns side by side merge into one shape, and a yellow filter
  bar beside a yellow search bar cannot be told apart. Dots beside a bar
  can. Search is first, beside the rule; the filter bar is at the pane's
  edge.
- **Interesting lines only.** The filter column counts the lines that `n`
  steps to: lines an include filter matched. Context lines are there to be
  read around a hit, not to be one.
- **Search hits among the visible lines.** The search looks only at the
  visible lines (ADR 0001), so the map does too.

### How lines are grouped

- **The groups are lines of the file, not visible lines.** In hide mode
  every visible line is a match, so a map of the visible lines would be
  full bars from top to bottom. A map of the file still shows where in the
  file the activity is. The band is then the rows from the first
  to the last line on screen, which in hide mode can be most of the map.
- **Line `l` of `n` is on map row `l × rows ÷ max(n, rows)`.** A file
  shorter than the map has 1 line on each row, at the top. The braille
  dot rows use the same rule with `4 × rows`.
- **The scale is the lines loaded now.** A truncated preview, follow mode
  and stdin have no known total. The map always fills its height, its
  scale changes as the file grows, and a click always goes to a line that
  exists. An estimate of the total would leave the bottom of the map empty
  and make a click there load the full file first.

### When the map is drawn

- **On by default.** `[view] minimap = false` in `config.toml` turns it
  off; `M` (`view.toggle.minimap`) shows or hides it for the session.
- **Only for raw text.** Not for hex, a directory listing, a message
  standing in for a file, or an image.
- **Not when the visible lines fit in the window.** Then the band covers the
  whole map, and the user can see every line already. This is most often
  hide mode with few matches.
- **Not in a narrow pane:** below 40 columns inside the border, the text
  keeps all of them.

### Moving with the map

- A click on map row `r` goes to the first line of that row's group, or
  the nearest visible line to it. It is a jump, the same as a search's:
  `center_jumps` decides where the line lands.
- A drag that starts on the map jumps again on each move, so the view
  follows the pointer. A drag that starts on the text still selects.

### Performance

- The map is built once and kept. It is built again only when the lines,
  the verdicts, the visible set, the search or the map's height change.
  Moving the cursor, scrolling and a click compare a key and build
  nothing, so a file of a million lines is not slower with the map on.
- A build is one pass over the verdicts and, with a search, one regex
  pass over the visible lines.

## Rejected

- **1 column with the band on the pane's border.** It fits, but a cell has
  one foreground and one background colour, so a filter colour and a search
  mark compete for the same cell.
- **Half blocks (`▀`/`▄`) for 2 coloured map rows in each cell.** Twice the
  resolution, but no density, and no room for a search mark.
- **A blank column between the text and the map.** The rule does the same
  work and reads as a scroll bar.

## Open for #524

#524 says the picture "follows the visible set when filters hide lines".
This design groups lines of the file. When #524 is worked, it must decide
if the picture groups the same way, so that the picture, the marks and the
band stay aligned.
