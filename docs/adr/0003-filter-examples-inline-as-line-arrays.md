# ADR 0003: A filter's examples are two arrays of lines, inline in filters.toml

**Status:** accepted
**Date:** 2026-09-28

## Context

The filter editor's checks become a filter's *examples* (#318): the lines the
pattern must match or must not match, kept as regression tests. #38 left two
questions open: where the examples live, and how to keep long log lines easy
to read in the file.

## Decision

- **Inline, on the filter, in `filters.toml`.** A filter's examples are in
  the same `[[sets.<name>.filters]]` table as its pattern. The set is one
  file to copy, and a hand edit of the pattern shows its tests beside it.
- **Two arrays of strings: `must_match` and `must_not_match`.** One string is
  one whole line of the log. recon writes each string on a line of the file
  of its own, as a single-quoted literal string when TOML allows one, so a
  line is written exactly as the log has it:

  ```toml
  must_match = [
      '2026-09-28 ERROR [net] C:\temp timeout after 30s',
  ]
  ```

  A line with a `'` or a control character other than a tab is written as a
  basic string, with escapes.
- **The load refuses** a line with a line break and a line in both arrays.
  No pattern can pass a line in both. A line twice in one array is kept once.
- **The load does not check the pattern against its examples.** A hand edit
  that breaks one still loads. The filter editor shows the failure.
- **The gate is on replacement.** `Enter` in the filter editor refuses a
  pattern while a check fails, and `f c` refuses a pattern that fails a
  stored example.

## Considered options

- **A separate file next to `filters.toml`.** Rejected: the pattern and its
  tests can then go out of step, and a set is no longer one table to copy.
- **An array of tables (`[[sets.a.filters.examples]]` with `line` and
  `match`).** Rejected: two or three rows for each example, and the key
  repeats on each one. Two arrays read as two lists.
- **Refuse to start when a pattern fails its examples.** Rejected: one
  failing test would stop recon for all filters. The editor is where the
  user can see and fix it.
