# ADR 0009: Batch mode always hides

**Status:** accepted
**Date:** 2026-10-23

## Context

Decision 5 of `docs/specs/2026-09-06-headless-mode-design.md` made `--set`,
`--hide` and `-q` startup flags for both paths, with one behaviour: without
`--hide`, a batch run started in dim mode, as the TUI does.

In the TUI, dim mode is useful. You can see the grey, and `u` takes you to
hide mode. A batch run has neither. Stdout has no grey, so in dim mode the
matches look the same as the rest:

- `--emit lines` printed the whole file minus the excluded lines. The
  include and context filters had no effect on the output.
- `--emit files` listed every readable input, which is what `ls` does.

Every batch example in the README and in the design passed `--hide`. The
summary had to warn about it: `dim mode (2 match) — pass --hide to emit
matches only` (#511).

## Decision

- **Batch mode always runs in hide mode.** `--emit lines` prints only the
  lines the filters keep. `--emit files` lists only the files they select,
  or every readable file when no filter selects, which is the explorer's
  rule in hide mode.
- **`--hide` is still accepted, and does nothing in batch mode.** Scripts
  that pass it keep working and give the same output.
- **The TUI does not change.** `--emit` from a terminal still opens in dim
  mode unless `--hide` is given. `--follow` in the TUI does not change.
- **The summaries lose the `— pass --hide to emit matches only` hint.** They
  always say `hide mode`.

This reverses decision 5 for `--hide` only. `--set` and `-q` still mean the
same on both paths.

## Considered options

- **Keep dim mode as the batch default.** Rejected: on stdout, dim mode
  hides the matches among the rest, and every real use needed `--hide`.
- **A `--dim` flag to get dim mode back in batch mode.** Rejected. With an
  include filter on, dim mode gives a batch run only "apply the excludes and
  ignore the includes". To get the same output, do not enable the include
  filters.
