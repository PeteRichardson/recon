# ADR 0005: Batch mode has a flag, and "headless" is now "batch"

**Status:** accepted
**Date:** 2026-10-01

## Context

The run with no TUI started only when `--emit` was given and stdin was not a
terminal (#143). From a terminal, `recon --set errors --emit lines --hide
app.log` opened the TUI and waited for `q`. Only `< /dev/null` forced the
other path, and that was not easy to find. The 1.0 design rejected a flag as
the *only* trigger (decision 1 of
`docs/specs/2026-09-06-headless-mode-design.md`), because a pipe or a cron job
must not need one. Its Follow-ups list `--batch`.

## Decision

- **`-b, --batch` is a second trigger.** Batch mode starts on `--batch`, or
  on `--emit` with stdin that is not a terminal. The stdin rule stays, so a
  pipe or a cron job still needs no flag (#439).
- **`--batch` without `--emit` emits `lines`.** That is the common case.
  `files` and `cwd` still need `--emit`.
- **With `--batch` and stdin piped, stdin is the file list,** as before.
- **The feature is called "batch mode", not "headless mode",** in the code,
  the tests, `--help` and the README. The flag gave it a name the user types,
  and the docs use the same name.
- **Dated records keep the old word.** Specs, plans, reviews and earlier ADRs
  under `docs/` record what was decided at the time. Where they say
  "headless", read "batch".

## Considered options

- **`--batch` as the only trigger.** Rejected for the reason the 1.0 design
  gave: a pipe on stdin already means no keys can drive a TUI.
- **Rename inside the dated records.** Rejected: it rewrites history, and
  this ADR maps the old word to the new one.
