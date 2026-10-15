# ADR 0006: Filters and search run on the lines of the view

**Status:** accepted
**Date:** 2026-10-14
**Spec:** [File views](../specs/2026-10-14-file-views-design.md)

## Context

A file can now have more than one **view**: raw, hex, a **command view**
(for example `jq .` or `exiftool`), or an image. A command view can change
the lines a lot: `jq .` puts one key on each line, `exiftool` makes rows
from bytes that have no lines. The filters and `/` must run on one set of
lines: the lines the user sees, or the lines on disk.

## Decision

Filters and search run on the lines of the current view. What the user sees
is what they filter. A view with no lines (an image) cannot be filtered or
searched, and the status line says so. Highlighting is a layer on a view and
does not change its lines.

The hex view already works this way: its formatted rows are the lines that
filters see.

## Consequences

- **Explorer marks do not agree with a command view.** The background scan
  that marks files in the explorer reads the raw file. A word that `jq .`
  moves to a different line, or a field that only `exiftool` shows, gives a
  mark that does not agree with the view. This is a known problem, not
  solved here. A probable solution is to take marks from the view cache of
  ADR 0007 when it has the view. A real solution can change how recon feels
  to use, so it needs its own design.
- **Batch mode reads raw lines for now.** For the same rule, batch mode
  will later filter the first text view in the cycle.

## Considered options

- **Filter the raw file, show the view.** Rejected: a match would point at
  a line that the view does not have, and a filter on an EXIF field could
  never match.
