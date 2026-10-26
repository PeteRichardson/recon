# About filters

A filter is the main idea in recon. This page tells you how filters, search
and the two display modes operate together, and why they are different.

## The four senses

Each filter has a *sense*. The sense tells recon what a match means.

| Sense | In the view | A stop for `n`? | Effect in the explorer |
| --- | --- | --- | --- |
| include | shows the line in the filter's color | yes | marks the file |
| context | shows the line in the filter's color | no | none |
| require | shows the line in the filter's color | no | a file is marked only if this filter matches one of its lines |
| exclude | removes the line | no | none |

An *include* filter finds results. An *exclude* filter removes noise. A
*context* filter is for lines that you want to read near a result, but that
are not a result. Examples are the commit of the build, or the name of the
host. Every log has these lines, so they do not tell you which logs are
important, but they do supply important context when you are analyzing a log.

The same pattern can have a different sense for a different question.
`^host: production-.*` is an include filter when you ask "which production
logs have errors?". It is a context filter when you ask "which logs have
bug 57, and where did they run?".

A *require* filter is for the next question about the logs that you found:
"of these logs, which ran on host `Zeus`?". In the view, it is the same as a
context filter. In the explorer, a log is marked only when it has an include
line and also a line for each require filter, anywhere in the file. With no
include filter on, the require filters alone decide which logs are marked. A
line that an exclude filter removes does not count. Set the sense in the
filter editor (key `r` on the sense row) or in `filters.toml`
(`sense = "require"`).

## Dim mode and hide mode

When an include, context or require filter is on, a line that none of them
matches is *dimmed*. You can still read it, but it is gray. The `u` key changes to *hide
mode*, which removes these lines. Press `u` again to show them.

![Two exclude filters and an include filter, then hide mode and a peek](images/filter-stack.gif)

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
