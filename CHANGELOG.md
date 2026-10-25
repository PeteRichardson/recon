# Changelog

## v0.6.0 — 2026-10-25

### Breaking changes

- **batch:** Read the lines on stdin with `-`, list files with --files-from

### Features

- **tui:** Read the lines on stdin with `-`
- **tui:** Add a command palette on `:`, with `e`/`cd` and `goto`
- **tui:** Add a minimap of search hits and filter density

## v0.5.0 — 2026-10-24

### Features

- **follow:** Watch a growing file with Ctrl-t and --follow
- **icons:** Coloured filetype icons in the explorer and listing
- **batch:** Always emit in hide mode, with or without --hide

### Fixes

- **filter:** Count the built-in slots apart in the pattern-limit note

## v0.4.0 — 2026-10-21

### Features

- **views:** Decode a large JPEG at reduced scale in the image view
- **keys:** Hide the file view with T
- **logging:** Add startup snapshot, filter changes, programs, summary
- **logging:** Add view job timings, scan passes and navigation
- **logging:** Add keys and actions, frame times, clipboard and theme
- **logging:** Log to a rotating file by default in the TUI
- **logging:** Log the full version and a session-started record
- **config:** Move the log settings to [log] and expand ~ in paths

### Fixes

- **clipboard:** Report the exit status when the tool breaks the pipe
- **explorer:** Make the file view follow a selection hide mode moves
- **explorer:** Keep the selection on a file a peek from dim mode shows
- **keys:** Let a pane binding win over the filter-verb hint
- **views:** Give quicklook a 10 s timeout for a cold qlmanage

## v0.3.0 — 2026-10-19

### Breaking changes

- **filters:** Add advanced filters with lookaround and a step limit

### Features

- **views:** Match a [[filetype]] row by MIME type from the content
- **views:** Show date, size and MIME type in the file view title
- **keys:** Open a listing entry with Enter in the file view
- **views:** Right-align the file facts in the file view title
- **keys:** Step to any file with . and , when no filter is on
- **filters:** Use another filter's pattern with ${set:name}

## v0.2.0 — 2026-10-15

### Breaking changes

- **views:** Read every view on a background pipeline
- **views:** Hand the file off on o, open the project on O
- **keys:** Toggle filters 01 to 99 with two digits

### Features

- **views:** Show a file as a command's output from [commands]
- **views:** Show pictures in the file pane, with Quick Look on macOS

### Fixes

- **editor:** Name only the file when `o` cannot find it
- **views:** Draw pictures in iTerm2, and fill the pane

## v0.1.1 — 2026-10-13

### Fixes

- **batch:** Read stdin only when no PATH is given
- **filter-editor:** Count display columns in text fields and badges
- **editor:** Say why a terminal $EDITOR fails on `o`
- **editor:** Do not start a terminal editor from $EDITOR
- **chains:** Take the next-hit step on return, not a literal `n`

### Performance

- **events:** Do not redraw for mouse motion

## v0.1.0 — 2026-10-13

Baseline: the first tagged state. The changes before this tag are in the git history.
