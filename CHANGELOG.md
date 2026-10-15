# Changelog

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
