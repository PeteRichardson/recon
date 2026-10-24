# ADR 0011: The keymap name is the command name

**Status:** accepted
**Date:** 2026-10-24

## Context

#520 adds the command palette: `:` opens a line where the user types a
command by name. Every keymap action is a command there, and each one already
has a name the keymap config uses, such as `global.follow`. A palette usually
gives its commands short names, such as `follow`.

Later, the user will write aliases that chain commands with their arguments
(`explorer.set <dir> ; explorer.focus`) and bind keys to them. The keymap,
the palette and the aliases must then all name a command the same way.

## Decision

- **A command that a key can run has one name: its keymap name.** The
  palette shows `global.follow`, not `follow`. Fuzzy matching on the name and
  on its help text makes the long name fast to type and easy to find.
- **A short name is only for a command that takes arguments** (`e`, `goto`),
  which no key can run, **or for a well-known Vim alias** of a command
  (`cd` for `e`, `q` for `global.quit`). An alias shares its command's row in
  the palette.

So a rename of a keymap name is now a break in the palette and in aliases
too, not only in keymap configs.

## Considered options

- **Short palette names beside the keymap names.** Rejected: two names for
  each command, and an alias author must know which one a chain takes.
- **Short names everywhere, with the keymap names renamed to match.**
  Rejected: it breaks every keymap config, and the scope prefix
  (`explorer.`, `view.`) is what tells `explorer.down` from `view.down`.
