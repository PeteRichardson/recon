# Changelog

## v0.11.0 — 2026-11-07

Hide mode can now show a source file as an outline of its definitions, and you can open hidden lines in place to read one function or the lines around a match.

### Features

- **Open the hidden lines below the cursor with `>`.** Before, in hide mode, you had to change the filters or the mode to read the lines around a match. Now `>` shows every hidden line from the cursor line to the next visible line, and `<` hides the smallest open group that holds the cursor. These keys do not run the filters again. Open lines stay open when you change a filter, but an exclude filter still removes them. Each file keeps its open lines for the session, until a reload of a changed file. `view.reveal.close.all` in the command palette hides them all again. ([#609](https://github.com/PeteRichardson/recon/issues/609))
- **Definition filters show where each definition ends.** Before, with a definition filter on, hide mode showed only the first line of each function, impl or type, so you could not see where it stopped. Now hide mode also shows the end line of each matched definition, so the view reads as an outline. `--emit lines` writes the same lines. ([#610](https://github.com/PeteRichardson/recon/issues/610))
- **Open one whole definition with `>`.** Before, `>` in the outline opened only the lines up to the next visible line, so you could not read one whole function. Now `>` on the first or last line of a definition opens all of its lines. On all other lines, `>` works as before. A function that you open inside an impl stays open when you close the impl. ([#611](https://github.com/PeteRichardson/recon/issues/611))
- **The outline and `>` work for Python.** Before, recon could not find where a Python definition ends: `>` on a class opened only the lines up to its first method, and `>` on the last method also opened the module code below it. Now a Python `def` or `class` holds each later line that is blank or indented more deeply. A docstring line at column 0 or a `):` that closes a signature stays inside the definition. In a Python file, the definition filters show no end line, because that line is only the last line of the body. ([#612](https://github.com/PeteRichardson/recon/issues/612))

### Fixes

- **A broken symlink shows as a broken link.** Before, a link whose target was missing looked like a small plain file, with the length of its link text as its size. The scan and `--emit files` then tried to open it and failed. Now the row is dimmed and shows no size, and the scan and `--emit files` skip it. `l`, `Enter` and a click do nothing on it. Its preview says `broken symlink to <target>` above the error from the OS. ([#404](https://github.com/PeteRichardson/recon/issues/404))
- **A line with a carriage return can no longer block the save of a filter set.** Before, you could mark a line that held a carriage return in the middle (for example, progress output such as `50%\r100%`). After that, `S` could not save the scratch set, and the error did not tell you which line caused the problem. Now recon refuses that mark and names the line. When `f` from the file view marks the line automatically, it skips such a line without a message. ([#359](https://github.com/PeteRichardson/recon/issues/359))

### Performance

- **recon uses less CPU when it is idle.** Before, recon copied the filter matcher 60 times each second, also when no scan result was waiting. Now it builds the matcher only when a scan result needs it. ([#344](https://github.com/PeteRichardson/recon/issues/344))

## v0.10.0 — 2026-11-06

recon can now open very large logs and stay responsive with them.

### Features

- **Large command output goes to a file, with no cap.** Before, a command view kept only the first 10 MiB of output and stopped the command after 2 seconds. Now, when a command's output is larger than 10 MiB, recon writes it to a temporary file and shows it from there. You see the lines while the command runs, and the status line shows RUNNING with the line count. A command that writes output continuously can run for as long as it needs. The timeout now applies only when the command stops writing output. Press Ctrl-C to stop the command and keep the lines that it has written. To always use a file for a command view, set `spool = true`. ([#573](https://github.com/PeteRichardson/recon/issues/573))
- **The screen stays live when you change a filter on a very large file.** recon shows its progress while it checks the lines again. Before, the screen froze until recon had checked every line. ([#597](https://github.com/PeteRichardson/recon/issues/597))

### Fixes

- **Each filter keeps its own colour.** Before, after you deleted a filter, a new filter could get the same colour as a different filter, so you could not tell them apart. Now a new filter always gets a colour that no other filter uses. ([#355](https://github.com/PeteRichardson/recon/issues/355))

### Performance

- **Much less memory for large files.** A 1 GB log now needs about 0.12 GB of memory instead of about 3 GB, so you can open logs that were too large before. ([#51](https://github.com/PeteRichardson/recon/issues/51))
- **Files with hundreds of millions of lines.** recon now uses less memory for each line, so a typical computer can open such a file. ([#572](https://github.com/PeteRichardson/recon/issues/572))
- **Large files open about five times faster.** They also use less memory, so recon can keep more files in its view cache. Files of normal size open at the same speed as before. ([#593](https://github.com/PeteRichardson/recon/issues/593))
- **Faster filter matching.** recon finds filter matches in large files faster in the file view, in batch mode and in the explorer scan. The results do not change. ([#592](https://github.com/PeteRichardson/recon/issues/592))

## v0.9.0 — 2026-11-03

### Features

- **fork:** A horizontal scroll offset the caller owns ([#577](https://github.com/PeteRichardson/recon/issues/577))
- **widgets:** Hbar, the horizontal scroll bar thumb maths ([#578](https://github.com/PeteRichardson/recon/issues/578))
- **app:** Measure the widest visible line for the scroll bar ([#579](https://github.com/PeteRichardson/recon/issues/579))
- **fileview:** Reserve and draw the horizontal scroll bar row ([#580](https://github.com/PeteRichardson/recon/issues/580))
- **mouse:** Click and drag the scroll bar ([#581](https://github.com/PeteRichardson/recon/issues/581))
- **views:** Align CSV into columns ([#575](https://github.com/PeteRichardson/recon/issues/575))
- **filter:** Author and persist a per-filter neighbor count
- **filter:** Left and Right move along the neighbors row
- **view:** Show neighbor lines around include matches in hide mode
- **view:** Ctrl-n renders neighbor lines bright or dim
- **emit:** Write neighbor lines with --emit lines, --no-neighbors drops them

### Fixes

- **fileview:** One padding call, and the map sees the bar's row ([#580](https://github.com/PeteRichardson/recon/issues/580))
- **csv:** The gap comes after the fill, not inside the max

## v0.8.0 — 2026-10-29

### Breaking changes

- **palette:** Add explorer.set and rename the explorer focus action

### Features

- **filter:** Add a require sense that keeps only matching files
- **filter:** Add a Require section that makes require rows from a filter
- **filter:** Refuse a ; in a set, profile or filter name
- **palette:** Add user aliases that chain commands from [aliases]
- **palette:** Let an alias take arguments $1 to $9 and $*
- **keymap:** Let a [keymap] line bind a key to a user alias
- **palette:** Add :w and :w! to write the visible lines to a file

### Fixes

- **filter:** Anchor a require row's pattern so long lines stay fast
- **filter:** Apply the advanced step limit to the whole search of a line

## v0.7.0 — 2026-10-26

### Features

- **tui:** Add `set` and `unlist` to the command palette
- **tui:** Add `view` to the command palette
- **set:** Turn filters off in --set lists, and keep state with +NAME
- **tui:** List named filters in the profile picker
- **tui:** Scroll the profile picker and count hidden rows
- **tui:** Add `include`, `exclude` and `context` to the command palette
- **config:** Add -c / --config to read a file other than config.toml
- **set:** Add --clear to start with every filter set unlisted
- **set:** Add --list to list a filter set at startup, disabled

### Fixes

- **set:** Read @all from the file for a set that is not listed

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
