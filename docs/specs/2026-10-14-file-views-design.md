# File views and hand-offs — design

**Status:** accepted, not implemented
**Date:** 2026-10-14
**Decision records:** [ADR 0006](../adr/0006-filters-run-on-the-view.md),
[ADR 0007](../adr/0007-one-background-view-pipeline.md)
**Related issues:** #10 (images), #422 (pretty JSON), #433 (open with other
apps), #372 (fileview.rs), #375, #383 (reads on the UI thread)

## Problem Statement

recon shows every file in one of two ways: as raw text, or as hex for a
binary file. For many files neither one helps when the user browses:

- A one-line JSON file is one very long line. The filters match the whole
  file or nothing.
- A photo, a PDF or a PSD shows as hex. To find one photo among 100, the
  user must open each one in a different app, and every app opens a window.
- A file with structure the user cares about (EXIF data, an incident
  report) needs a program to make it readable, and recon cannot run one.
- `o` and `O` send every file to the editor. A `.md` file cannot go to
  Marked 2, and a `.png` file cannot go to Preview.

Every file is also read on the main thread when the explorer selection
moves. A slower way to show a file would stop browsing, which is the reason
recon exists.

## Solution

A file has a **view cycle**: an ordered list of **views**. The first view is
the default, and `-` steps to the next one. A view is *raw*, *hex*,
*image*, or a **command view**: the output of an external program that the
user names in the config (`jq .`, `exiftool`, `qlmanage`, `mmdc`).

- Filters and search run on the lines of the view (ADR 0006). An image has
  no lines.
- Every view runs on one background pipeline, with prefetch of the nearby
  files (ADR 0007). The main thread never reads a file.
- `o` is the **hand-off**: it sends the file to the app that its file type
  names. With no rule, a text file goes to the editor and any other file
  goes to the operating system's default app. `O` opens the project in the
  editor.
- One `[[filetype]]` table in the config sets the cycle and the hand-off
  for each file type. With no config, recon works as it does now, except
  for the `o`/`O` swap, and on macOS binary files show a Quick Look
  thumbnail.

The vocabulary is in `CONTEXT.md`: *view*, *command view*, *view cycle*,
*highlighting*, *hand-off*.

## User Stories

### Browsing

1. As a user who browses, I want a raw file to show as fast as it does now,
   so that the new views cost me nothing when I do not use them.
2. As a user who browses, I want a slow view to never stop the explorer,
   so that I can move past a file before its command is complete.
3. As a user who browses, I want the files near the selection prepared
   before I arrive, so that a command view is usually ready when I see it.
4. As a user who browses photos, I want each photo to show in the pane, so
   that I can find the correct one with no window for each photo.
5. As a user on macOS, I want a PDF, PSD or HEIC file to show as a
   thumbnail in the pane, with no config.

### Views and the cycle

6. As a user, I want `-` to step to the next view of the file, so that I
   can see a JSON file as `jq` output, then as raw text, then as hex.
7. As a user, I want the status line to name the view, so that I always
   know when the lines are not the lines of the file.
8. As a user, I want my `-` step to hold for the next file of the same
   type, so that I can browse 50 photos as EXIF rows with one key press.
9. As a user, I want to set the order of a cycle and leave a view out of it,
   so that my `.mmd` files show the rendered diagram, then the source, and
   never hex.
10. As a user, I want a failed view to show the raw text (or hex, for a
    binary file) with the error on the status line, so that a bad file
    never gives an empty pane.

### Filters and search

11. As a user, I want filters and `/` to run on the lines I see, so that a
    filter on a key in pretty-printed JSON matches one line, not the whole
    file.
12. As a user, I want an image view to tell me that filters and search have
    no effect on it, so that I do not think that my filter failed.

### Command views

13. As a user, I want to name a program in the config as a view, so that
    recon can show any file type that a command-line tool can read.
14. As a user, I want a command to get the file on stdin or as a path, so
    that both `jq .` and `exiftool {file}` work.
15. As a user, I want a command's output to be highlighted for its type, so
    that `jq` output gets JSON colours.
16. As a user, I want a command that can produce an image (`mmdc`,
    `qlmanage`) to show in the image view.
17. As a user, I want to stop prefetch for a command that is expensive, so
    that an LLM summary runs only for the file I am on.
18. As a user, I want a missing command to give one warning at startup, not
    an error on each file.

### Hand-off

19. As a user, I want `o` to open a `.md` file in Marked 2 and a `.png` in
    Preview, so that I can use the best app for each type.
20. As a user, I want `o` on a source file with no rule to open it in my
    editor at the cursor line, as `O` does now.
21. As a user, I want `O` to open the project in my editor, as `o` does now.

## Implementation Decisions

### The view pipeline (stage 1)

- A view is a job: (file, view) → **contents**, where contents are lines
  (`Vec<String>`, with the truncation facts that `Contents` has now) or an
  image. Raw and hex are jobs that do what `read_preview_with_caps` and
  `hex::line` do now, with the same caps.
- Jobs run on a worker pool of `cores − 2` threads (at least 1). The main
  thread sends a request and receives results on a channel in the event
  loop.
- After the main thread sends the job for the file the user is on, it waits
  up to ~30 ms for the result. If the result arrives, it draws it at once,
  so raw and hex look as they do now. If not, it draws a placeholder and a
  spinner on the status line, and draws the contents when they arrive.
- Each request has a generation number. A result for an older generation
  of the current pane is dropped. A running external command for a file
  that is not near the selection is killed.
- The first key in the view that upgrades a truncated preview to a full
  load (#383) is also a job.
- Directory previews (#375) are also a job.
- The explorer's background scan does not change. It still reads raw files
  (ADR 0006, Consequences).
- Tests get an executor that runs each job synchronously in the caller, so
  that the existing tests keep their sequence.

### The view cache and prefetch (stage 1, or 3 if it is large)

- Results go into a cache with a byte limit (256 MiB), keyed by
  (path, mtime, size, view name). An LRU entry is removed first.
- When the explorer selection moves, the pipeline queues the current view
  of the next 8 files in the direction of travel and the 2 behind. Only the
  view that the cycle would show for each file is prefetched, not the full
  cycle.
- The job for the file the user is on goes before all prefetch jobs.
  Queued prefetch jobs that are no longer near the selection are dropped.
- A command with `prefetch = false` runs only for the file the user is on.
- Raw and hex results are cached as well, so that `,` and `.` back to a
  file is immediate.

### The view cycle

- The cycle for a file comes from the first `[[filetype]]` row that
  matches it. The user's rows go first, then the built-in rows. The first
  row that matches gives the full cycle; rows do not merge.
- Built-in rows, in order:
  1. image extensions → `["image", "hex"]`
  2. binary, macOS only → `["quicklook", "hex"]`
  3. binary → `["hex"]`
  4. all other files → `["raw", "hex"]`
- `binary` is the check that recon does now (a NUL in the first 8 KiB).
- `-` (`GlobalToggleHex` now, renamed to a "next view" action) steps to the
  next view in the cycle, and wraps. The step is kept for that row for the
  session: the next file that the same row matches opens at the same view.
  The step is not saved.
- A view that cannot run is skipped: a command that is not on `PATH`, or
  the image view in a terminal that cannot show images (not possible with
  the half-block fallback, but the rule stays).
- The status line shows the view name when it is not `raw`.

### Failure

- If a view fails (non-zero exit, a timeout, output that cannot be
  decoded), recon shows raw for a text file or hex for a binary file at
  once. It does not try the other views in the cycle.
- The status line shows `<view> failed → raw` and the first line of
  stderr. The sticky step for the row does not change.

### Command views (stage 3)

```toml
[commands]
jq        = ["jq", "."]
exif      = ["exiftool", "{file}"]
mmd       = { argv = ["mmdc", "-i", "{file}", "-o", "{out}", "-e", "png"], output = "image/png" }
summary   = { argv = ["incident-summary", "{file}"], timeout = 30, prefetch = false }

[[filetype]]
glob    = "*.json"
views   = ["jq", "raw", "hex"]

[[filetype]]
glob    = "*.mmd"
views   = ["mmd", "raw"]
handoff = ["open", "{file}"]

[[filetype]]
glob    = "*.{jpg,png,heic}"
views   = ["image", "exif"]
handoff = ["open", "-a", "Preview", "{file}"]
```

- A command is an argv array or a table with `argv`, `output`, `timeout`
  and `prefetch`. The argv is not given to a shell, as the editor templates
  are not.
- Placeholders: `{file}` (the path), `{out}` (a new temporary path that
  recon reads after the command stops, then deletes), `{px}` (the pane size
  in pixels, for thumbnails). If the argv has no `{file}`, recon writes the
  file to the command's stdin.
- Output: stdout, or the file at `{out}` when the argv has `{out}`. For
  `qlmanage -t -o`, `{out}` is a directory and recon reads the one file in
  it.
- `output` is a MIME type, default `text/plain`. `image/*` goes to the
  image view. Any other type is text: split into lines, decoded as raw text
  is (lossy UTF-8), and highlighted by the grammar for the MIME type. With
  no `output`, the grammar comes from the file's name, as it does for raw.
- `timeout` is in seconds, default 2. After the timeout, the command is
  killed and the view fails.
- `raw`, `hex`, `image` and `quicklook` are reserved names. A user command
  with one of these names is a config error. The user can use
  `quicklook` in a row.
- Built-in command, macOS only:
  `quicklook = { argv = ["qlmanage", "-t", "-s", "{px}", "-o", "{out}", "{file}"], output = "image/png" }`.
  Measured: 0.23 s for a HEIC and a `.md` together.
- The config keeps `deny_unknown_fields`.
- A row with a command that is not on `PATH` gives one warning at startup
  (through the existing warnings), and the cycle skips that view.

### Hand-off and the key swap (stage 2)

- `o` does the hand-off. The argv comes from:
  1. the `handoff` of the row that matches, else
  2. the `[editor] file` template for a text file, else
  3. `open {file}` on macOS, `xdg-open {file}` on Linux.
- `O` opens the project in the editor: the `[editor] project` template,
  which `o` does now.
- Hand-off templates take `{file}` and `{line}`. The spawn is the one that
  the editor uses now: detached, with stdio on `/dev/null`, the child
  reaped on a thread, and terminal programs refused.
- The hand-off always sends the file on disk, whatever the view.
- `x` is not used.
- This is a breaking change. It goes in the CHANGELOG with the release, and
  the README, the help screen and `--print-editor-config` change.
- Stage 2 can ship before stage 1: it needs only the `[[filetype]]` table
  with `glob` and `handoff`.

### The image view (stage 4)

- `ratatui-image`. Check its ratatui version against ours first (#10).
- It detects kitty, iTerm2 and sixel, and falls back to half-blocks. Thus
  the image view is always available. Inside tmux the fallback is
  half-blocks.
- Decoding is a job in the pipeline, with a size cap. The image is scaled
  to fit the pane and keeps its aspect ratio. It is drawn again after a
  resize or a divider drag.
- The file view holds either lines or an image. With an image: no cursor,
  no gutter, no visual mode; `/`, `n`, `N` and the filter keys say
  "this view has no lines" on the status line; the pane keys (`z`, `b`,
  `E`, `F`) work.
- Formats are the ones the `image` crate decodes. Other formats (HEIC on
  macOS) come through `quicklook`.

### Documentation

- `CONTEXT.md` has the terms. The README gets a "Views" section with the
  config table and examples for `jq`, `exiftool`, `mmdc` and an image.
- The help screen shows `-`, `o` and `O` with their new meanings.

## Stages

1. **The view pipeline, the view cycle and `-`.** Raw and hex on workers,
   the ~30 ms wait, generations, the cache, prefetch, the `[[filetype]]`
   table with `glob` and `views` (built-in views only), the sticky step,
   and the view name on the status line. No change that the user can see,
   except speed and `-` on a configured cycle.
2. **The hand-off and the `o`/`O` swap.** `handoff` in the rows, the
   editor fallback for text, `open`/`xdg-open`. Independent of stage 1.
3. **Command views.** `[commands]`, the placeholders, `output` as MIME,
   highlighting by MIME, timeouts, `prefetch = false`, the failure rule,
   and the PATH warning. Commands with `image/*` output wait for stage 4.
4. **The image view.** `ratatui-image`, the `image` view, `image/*` output
   from commands, and the built-in image and Quick Look rows.

## Testing Decisions

- The pipeline: a synchronous executor in tests, and separate unit tests
  for the real pool: generations drop old results, the job for the current
  file goes before prefetch, prefetch jobs far from the selection are
  dropped, cache keys change with mtime.
- The ~30 ms wait: a test with a job that takes longer than the wait shows
  a placeholder, then the contents.
- The cycle: row matching (first row wins, user rows before built-in rows),
  `-` wraps, the sticky step per row, skipped views.
- Commands: fixture scripts in `tests/` for stdin, `{file}`, `{out}`, a
  non-zero exit, a timeout, and a command that is not on `PATH`.
- Filters on a view: a filter and a search on `jq` output match the
  pretty-printed lines, not the raw line.
- Hand-off: the argv that each of the three sources builds, with the spawn
  replaced, as the editor tests do now.
- The image view: unit tests for the "no lines" messages. The protocols are
  checked by hand in Ghostty, kitty, iTerm2 and a terminal with no support.
- TUI checks under a pty as in `tui-verification-under-a-pty`, for the
  placeholder and the status line.

## Out of Scope

- **Explorer marks from a view.** The scan reads raw files (ADR 0006). A
  probable solution is marks from the view cache; it needs its own design.
- **Batch mode with views.** It will filter the first text view in the
  cycle, and maybe take `--view <name>`. Later.
- **A built-in pretty JSON view.** A `jq` command view does that job
  (#422).
- **More than one hand-off per type** (Preview *or* Photoshop). `O` or a
  picker can come later.
- **A hand-off as a step in the cycle.** A default hand-off would open an
  app on each explorer move.
- **User-written plugins** beyond external commands.
- **Animated images, video, and OCR.**

## Further Notes

- An LLM summary command must use `prefetch = false`, or it runs for 10
  files on each explorer move.
