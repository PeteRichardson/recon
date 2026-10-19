# About filters

A filter is the main idea in recon. This page tells you how filters, search
and the two display modes operate together, and why they are different.

## The three senses

Each filter has a *sense*. The sense tells recon what a match means.

| Sense | In the view | A stop for `n`? | Marks the file in the explorer? |
| --- | --- | --- | --- |
| include | shows the line in the filter's color | yes | yes |
| context | shows the line in the filter's color | no | no |
| exclude | removes the line | no | no |

An *include* filter finds results. An *exclude* filter removes noise. A
*context* filter is for lines that you want to read near a result, but that
are not a result. Examples are the commit of the build, or the name of the
host. Every log has these lines, so they do not tell you which logs are
important, but they do supply important context when you are analyzing a log.

The same pattern can have a different sense for a different question.
`^host: production-.*` is an include filter when you ask "which production
logs have errors?". It is a context filter when you ask "which logs have
bug 57, and where did they run?".

## Dim mode and hide mode

When an include or context filter is on, a line that no include or context
filter matches is *dimmed*. You can still read it, but it is gray. The `u` key changes to *hide
mode*, which removes these lines. Press `u` again to show them.

Each mode has a use. Dim mode shows a result with the text around it. Hide
mode shows only the results, so that you can count them or read them quickly.

The gutter always shows the original line numbers. In hide mode, a gap in
the numbers shows where recon removed lines. recon also underlines the last
number of each group of adjacent lines, so that you can see the boundaries
quickly.

## Search is not a filter

A filter decides what a line *is*. A search decides where the cursor *goes*.

A search with `/` moves the cursor to the next hit and highlights the hits.
It does not change which lines are visible, and it searches only the visible
lines. In hide mode, it does not find the lines that the filters removed.

An earlier design made search a type of filter. In use, it added hidden lines
back into a filtered view, which was the opposite of what people wanted. For
the history, see
[ADR 0001](https://github.com/PeteRichardson/recon/blob/main/docs/adr/0001-search-is-a-motion-not-a-filter.md).

A search is a good way to try a pattern. When the hits are correct, `p`
changes the search into a new include filter.

## Two ways to see the plain file

Sometimes the filters are in the way. You found the line, and now you want to
read the code around it.

- `space` *peeks*: it turns off all filters and changes the display mode.
  Press `space` again, and recon puts back the same filters, the same mode and
  the same cursor line.
- `!` turns off all filters, but does not change the display mode. Press `!`
  again to turn the same filters on.

The two keys keep different records, so one key does not change what the
other key puts back.
