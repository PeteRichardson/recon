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

**Note (#557):** a user alias from `[aliases]` is not a short name of one
command. It runs a chain, which has no single command whose row it can share.
So each user alias has a row of its own, after the built-in commands, with its
chain as the help. Its name cannot be the name of a command, so a user alias
never hides a command.

**Note (#559):** a `[keymap]` line can name a user alias, so the keymap, the
palette and the aliases share one namespace, as this ADR expected. That rule
from #557 is what makes a `[keymap]` line name exactly one thing. An alias's
key is only in the global scope.

## Considered options

- **Short palette names beside the keymap names.** Rejected: two names for
  each command, and an alias author must know which one a chain takes.
- **Short names everywhere, with the keymap names renamed to match.**
  Rejected: it breaks every keymap config, and the scope prefix
  (`explorer.`, `view.`) is what tells `explorer.down` from `view.down`.
