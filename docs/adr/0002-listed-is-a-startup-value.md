# ADR 0002: Listed is a startup value, and it wins over autoload

**Status:** accepted
**Date:** 2026-09-24

## Context

A filter set can now be *listed* (it has a row in the filter pane) or
*unlisted* (it has no row and no effect). A listed set is enabled or disabled
as before. The rule is: enabled ⇒ listed.

Whether a set is relevant depends on what the user reads. A set for
INFO/WARNING/ERROR lines helps with a log and is noise with source code. One
global flag that follows every change the user makes would be wrong for the
other kind of file.

## Decision

- **`listed` in `filters.toml` is a startup value, the same as `autoload`.**
  The default is `true`. The set picker (`L`) changes the listed state for
  the session only. It never writes back to the file. To change the startup
  state, the user edits the file. `--unlist NAME` does the same thing as the
  picker for a headless run.
- **`listed = false` wins over `autoload = true`.** The file is not refused.
  The most common edit is "I do not use this set much": the user changes one
  key. If recon refused the pair, the user would also have to change
  `autoload` and would find out only on the next launch. This is the one
  contradiction in `filters.toml` that recon resolves and does not refuse.
- `--set NAME` lists and enables a set, whatever `listed` and `autoload` say.
  `--set X` together with `--unlist X` is refused.
- Snapshots do not bring a set back. Un-solo restores only the sets that are
  still listed, and `R` does not change the listed state.

## Considered options

- **The picker writes to the file.** Rejected: see the context. Per-context
  startup states are left to later work (named groups of sets,
  recommendations).
- **Refuse `autoload = true` with `listed = false`.** Rejected: this makes a
  one-key edit a two-key edit, and the user finds out only after a restart.
