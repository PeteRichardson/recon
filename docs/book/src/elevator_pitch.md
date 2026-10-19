# About recon: why filter and browse in one tool

recon is a terminal file viewer for one job: to find the lines that matter in
a lot of text, and then to find the files that have them. Other tools do part
of this job. recon puts the parts in one window, and this document tells why
that is useful.

## The problem: grep chains that you cannot see

When you read a log, you usually remove noise one step at a time. First you
remove the health checks, then the debug lines, then a component that you do
not care about. With the shell, each step is one more `grep -v` in a pipe.
After four steps, you cannot easily see what you removed. To change one step,
you must edit the command again. When you open a different file, you must
type the full pipe again.

A pager such as `less` gives you one search, not a set of filters. An editor
can show a file, but it does not keep a list of rules about which lines to
show. recon starts from a different idea: **the filters are a thing that you
keep, look at and change**, and the file is only the text that the filters
operate on.

## The filter pane: your reasoning, on the screen

The filter pane is a full column at the right side of the window. Each filter
is a regular expression with a sense (include, exclude or context), a colour
and a number. You can turn each filter on or off, or edit it where it is.
A near-miss pattern is corrected in its position, not deleted and typed
again.

This changes how a log investigation feels. Your hypotheses are visible:
"I think the timeouts come from the cache, so I remove the cache lines." If
the hypothesis is wrong, you turn that filter off, and you do not lose it.
One key disables all filters, and the same key restores them, so it is easy
to compare the filtered view with the original file.

Two more decisions make the filters trustworthy:

- **Dim or hide.** Lines that no filter includes are dimmed, so you still see
  their context. One key removes them fully. The gutter always shows the
  original line numbers, so a gap in the numbers tells you that recon hid
  something.
- **Filters stay when the file changes.** The filter set is not attached to
  a file. That is why recon is also a browser, which the next section
  explains.

Search is intentionally different from a filter. A filter decides what a
line *is*; a search decides where the cursor *goes*. An earlier design made
search a filter, but in use it added hidden lines back into a filtered view,
which was the opposite of what people wanted (see
[ADR 0001](../../adr/0001-search-is-a-motion-not-a-filter.md)). Now `/`
behaves as it does in vim, and a good search can become a filter with one
key.

## Browsing: filters across a directory

Because the filters stay when you load another file, the explorer becomes a
fast way to ask a question about a full directory. With the filters set and
unmatched lines hidden, each file in the explorer shows only its matching
lines when you move to it. A file that shows nothing has no matches. A
directory of logs becomes something that you can answer with the arrow keys.

This only works if browsing is fast. Thus recon reads every file on
background threads and reads the files near the selection before you get to
them (see [ADR 0007](../../adr/0007-one-background-view-pipeline.md)). A very
large log, or a slow program, does not stop the cursor.

Filters can also be saved as named sets, with profiles that turn on a chosen
few of them. Thus a team's knowledge about a log format ("these are the
lines that show a failed deploy") becomes a file that everyone can load.

## Views: a file is not always text

Not every file is a log. recon gives each file type a **view cycle**: an
ordered list of ways to show it. Built-in views show the text, a hex dump or
a picture. A **command view** shows a file as the output of a program that
you name in the configuration: `jq .` for JSON, `exiftool` for a photo, or a
script of your own. One key steps to the next view.

The important decision is that **filters and search operate on the lines of
the view**, not on the bytes on disk
([ADR 0006](../../adr/0006-filters-run-on-the-view.md)). Thus a filter on a
JSON key matches the one line that `jq` puts that key on, and a filter on an
EXIF field matches a field that only `exiftool` can show. A command view
makes recon pluggable without a plugin system: any program that writes text,
or a picture, can become a view.

## Quick Look and pictures: browsing more than logs

On macOS, recon uses Quick Look to show a preview of files that a terminal
cannot otherwise show: PDFs, Photoshop files, Keynote and Pages documents,
camera RAW files and videos. Pictures fill the pane in
terminals that can draw them (kitty, Ghostty, iTerm2 and sixel terminals),
and as half-block characters in all other terminals.

This makes recon a general file browser too. You can move through a folder
of mixed files and see each one, in the same window where you filter the
logs. When a preview is not sufficient, one key sends the file to its own
app: your editor at the current line, or Preview for a photo.

## A keymap that you control

recon uses vim motions, and a key has the same meaning in each pane where it
makes sense. The keymap has layers (global keys, two-key chains such as
"focus the filter pane, then add a filter", and pane keys) so that a key
pressed in the wrong pane tells you so, and does not do nothing.

But the defaults are only defaults. You can bind any action to a different
key in the configuration file, or unbind it. recon can print the full keymap
that is in force, in the format that the configuration file uses, so that
you start from what you have and not from an empty file. A tool that you use
all day must fit your hands, not only the hands of its author.

## In one sentence

recon keeps your filters as visible, editable things, lets them follow you
from file to file, and shows each file in the form that makes it readable,
so that "what is in these files?" becomes a question that you answer by
browsing, not by building pipes.
