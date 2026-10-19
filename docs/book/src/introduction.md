# About recon

recon is a terminal file viewer for one job: to find the lines that matter in
a large quantity of text, and then to find the files that contain them. You
write each idea about the text as a regular expression, and recon keeps these
expressions on the screen as a list of [filters][filter]. recon never changes
a file that it shows.

This page tells you what recon is for, which parts are familiar from other
tools, and which parts are new.

## The problem: a pipe that you cannot see

When you read a log, you usually remove the noise one step at a time. First
you remove the health checks, then the debug lines, then a component that is
not important to you. In the shell, each step is one more `grep -v` in a pipe.
After four steps, it is difficult to see what you removed. To change one
step, you must edit the full command. To read a different file, you must type
the pipe again.

A pager such as `less` gives you one search at a time, not a set of rules. An
editor shows a file, but it does not keep a list of rules about which lines
to show. recon starts from a different idea: **the filters are things that
you keep, look at and change**. The file is only the text that the filters
operate on.

## The basics

Many log viewers give you a version of the features in this section. recon
puts them together with a small number of consistent rules.

**Three panes.** The recon window has three columns. The *explorer* at the
left shows the file system. The *file view* in the center shows the selected
file. The *filter pane* at the right shows your filters. You can hide any
pane, or zoom one pane to fill the window.

**A filter is a pattern with a sense.** Each filter is a regular expression,
a color and a *sense*. The sense tells recon what a match means:

| Sense | A line that matches |
| --- | --- |
| include | shows in the color of the filter |
| context | shows in the color of the filter, but is not a result |
| exclude | is removed from the view |

You turn each filter on or off with one key, and you edit a pattern where it
is. When a pattern is almost correct, you correct it. You do not delete it
and type it again.

**Dim or hide.** A line that no filter matches is dimmed, so that you can
still see the text around a result. One key (`u`) removes the dimmed lines
fully. The gutter always shows the original line numbers, so a gap in the
numbers tells you that recon hid some lines.

**A search is a motion, not a filter.** The `/` key works as it does in vim:
it moves the cursor to the next hit, and it does not change which lines you
see. When a search finds the correct lines, one key (`p`) changes it into a
filter.

**Keys that you know.** recon uses vim motions. Each action has one key, and
`?` shows the full keymap. You can change any key in the configuration file.
The mouse also works: you can click a file or a filter, select text, and drag
the pane borders.

**Hand-off to other apps.** recon is a viewer, not an editor. One key (`o`)
sends the file to the app for its type, for example your editor at the
current line. A different key (`O`) opens the project that contains the file.

For more about filters, search and the dim and hide modes, see
[About filters](filters.md).

## What makes recon different

The basics make recon a good viewer. The features in this section are
the reasons to use recon in place of other tools. Each one comes from the
same idea: a filter is a thing that you keep.

### Filters stay when the file changes

The filters are not attached to a file. When you select a different file in
the explorer, the same filters operate on it. Thus the explorer becomes a
fast way to examine a full directory. recon reads the files in the
background and shows the name of each file that matches in the color of the
filter. A dim name has no matches. You find the important logs with the arrow
keys, not with a pipe. See [About browsing with filters](browsing.md).

### Filters that you save and share

A good set of filters contains knowledge, for example "these lines show a
failed deploy". recon saves filters as named *filter sets* in TOML files.
A set can have *profiles* that turn on a selected group of its filters. A
team can keep its sets in a shared directory, and each person loads them at
startup. See [About filter sets](filter-sets.md).

### Filters that you test while you write them

The *filter editor* shows the full file while you type a pattern, and
highlights each line that matches. You can mark lines that the pattern must
match or must not match, and recon checks these marks after each key. When
you save the filter, recon keeps the marked lines as examples. Later, a
change that fails an example does not replace the pattern. On Apple silicon,
an optional build can ask an on-device model to write the pattern from a
description in plain language. See [About the filter editor](filter-editor.md).

### Filters that use other filters

A pattern can refer to the pattern of a different filter by name. When the
other filter changes, the reference changes with it, so two filters cannot
become different by accident. For patterns that standard regular
expressions cannot write, such as "contains A and B in either order", you can
mark a filter as *advanced*. An advanced filter uses a second engine with
lookahead and lookbehind, and has a step limit so that a slow pattern cannot
stop recon. See [About references and advanced filters](advanced-filters.md).

### Filters that operate on what you see

A file is not always text. Each file type has a *view cycle*: an ordered list
of ways to show it, such as text, a hex dump or a picture. A *command view*
shows a file as the output of a program that you name, for example `jq .`
for JSON or `exiftool` for a photo. Filters and search operate on the lines
of the view, not on the bytes on disk. Thus a filter can match a JSON key on
the line where `jq` puts it. On macOS, recon also shows Quick Look previews of
PDFs, documents and videos. See [About views](views.md).

### Results that leave recon

recon is part of the shell, not a replacement for it. When you quit, recon
can print the visible lines, the matched files or the current directory to
stdout, for the next command in a pipe. In *batch mode*, recon does not open
a window: it applies saved filter sets to files and prints the result. Thus
a filter set that you made in the window also works in a script. See
[About output and batch mode](batch_mode.md).

## Where to go next

- [Panes](panes.md) tells you what each pane shows and how to hide or show it.
- [Keys](keys.md) lists the keys that this guide uses.
- [Usage](usage.md) lists the command-line options.
- [Glossary](glossary.md) defines the words that recon uses.

{{#include glossary-links.md}}
