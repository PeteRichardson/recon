# About browsing with filters

In most viewers, the filters belong to one file. In recon, the filters stay
when you select a different file. This small decision makes recon a tool for
a full directory, not only for one file.

## A directory of answers

With an include filter on, the explorer marks each file:

- A name in the color of a filter has at least one line that the filter
  includes.
- A dim name has no matches.
- A plain name is not examined yet.

In hide mode (`u`), the files with no matches leave the list, as the lines
with no matches leave the view. When you move through the explorer, the file
view shows only the matching lines of each file. A file that shows nothing
has no matches.

Thus a question such as "which of these logs contain bug 57?" has an answer
in the explorer. You do not need a pipe of `grep` commands.

## Why it is fast

Browsing is only useful when it is fast. recon reads files on background
threads, never on the thread that reads your keys. It reads the files near
the selection before you get to them. For a large file, the view shows a
bounded preview, so that a directory of very large logs does not stop the
cursor.

The scan that marks the explorer stops at the first matching line of each
file. When you turn a filter on or off, recon can usually mark the full
directory again without reading the files again.

For the design, see
[ADR 0007](https://github.com/PeteRichardson/recon/blob/main/docs/adr/0007-one-background-view-pipeline.md).

## Only include filters mark files

A context filter or an exclude filter does not mark a file. Only an include
filter finds a result, so only an include filter makes a file important.
The `n` key uses the same rule, so the explorer and the file view agree about
which lines and files are important.
