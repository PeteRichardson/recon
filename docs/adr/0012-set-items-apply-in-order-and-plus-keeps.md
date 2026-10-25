# ADR 0012: `--set` items apply in order, and `+NAME` keeps the earlier state

**Status:** accepted
**Date:** 2026-10-25

## Context

`--set NAME:LIST` (#441) turns on the union of the profiles and named filters
of the list, and for one set the last `--set` wins. The user cannot turn a
filter off, so "the `default` profile without `noise`" needs a new profile in
the file. A later `--set` cannot change an earlier one either: it replaces it.

#538 asks for both, from the command line and the palette's `set`, with a
syntax that is fast to type.

## Decision

- **One list, read from left to right.** A profile turns on its members, a
  filter name turns on that filter, and `-` before an item turns its filters
  off. `+` before an item only marks it as one that turns on, so every name
  can be reached (`+-x` is the filter `-x`). `@all` is every filter of the
  set.
- **`NAME:LIST` replaces; `+NAME:LIST` changes.** Without `+`, the list
  starts with all filters of the set off, as today, so every list that worked
  before keeps its meaning. With `+`, it starts from the set's state: the
  flags the earlier `--set` flags gave it (in the palette, the flags the pane
  shows now), or its `default` profile when the set is not on.
- **A set name cannot start with `+`, and no profile or filter can be named
  `@all`.** The loader refuses them, the same as a comma in a name.

## Considered options

- **A third field, `NAME:BASE:CHANGES`.** Rejected: the base and the changes
  are one list once an item can have a sign, and `::` for "keep" was not
  clear.
- **A sign on every change.** Rejected: too slow to type, and a name that
  starts with `-` is rare; the `+` escape covers it.
- **Reserved profile names `None` and `All`.** Rejected: a user can already
  have a profile or a filter with those names. `NAME:LIST` already starts
  from none, and `@all` cannot be a name.
- **`*` for all.** Rejected: zsh globs it, so it needs quotes.
