# ADR 0007: All views run on one background pipeline

**Status:** accepted
**Date:** 2026-10-14
**Spec:** [File views](../specs/2026-10-14-file-views-design.md)

## Context

recon reads each file on the main thread when the explorer selection moves:
up to 50,000 lines or 10 MiB, with no debounce and no cache (#375, #383).
Raw and hex are fast enough for that. A command view (`jq`, `exiftool`,
`qlmanage`, an LLM summary) can take from 100 ms to many seconds, and an
image must be decoded. Browsing must never wait on a view.

## Decision

Every view, raw included, is a job that makes lines (or an image) from a
file on a worker thread. Raw is the pass-through job. The main thread never
reads a file.

- **No added latency for fast views.** After it starts a job, the main
  thread waits up to ~30 ms for the result before it draws a placeholder.
  A raw or hex load shows as it does now, with no "loading" frame.
- **The newest selection wins.** A job for a file the user has left is
  cancelled or its result dropped.
- **Prefetch.** A worker pool (`cores − 2`) runs the current view of the
  next 8 files in the direction of travel and the 2 behind. The results go
  into a cache with a byte limit, keyed by (path, mtime, view). The file the
  user is on always goes first. A command can opt out with
  `prefetch = false`.

## Considered options

- **Raw and hex stay synchronous; only slow views go to the background.**
  Rejected: two load paths, and the main thread still reads files (#375,
  #383). The only cost of one pipeline is the up-front refactor and a
  synchronous executor for tests.
- **Block until the view is ready.** Rejected: a slow command would stop
  browsing.
