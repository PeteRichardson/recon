# ADR 0021: a pack can carry aliases

**Status:** accepted
**Date:** 2026-11-22

## Context

ADR 0018 lets a set file carry `[commands]` and `[[filetype]]`, so that one
file gives recon all it must know about a tool. But a palette alias that
runs the workflow of the tool, such as `dlogs = "e $1 ; set
DeployLogs:outcome ; unlist code"`, could be only in each user's
`config.toml`. A team could not share it with the set file (#627).

Recon built the keymap before it read the set files, so that
`--print-keymap` worked when a set file was broken. A `[keymap]` line can
give an alias a key, so the keymap could not see an alias from a set file.

## Decision

- **A set file can have `[aliases]`,** in the format of `config.toml`. The
  file is then a pack, as when it has `[commands]` or `[[filetype]]`.
- **The rules of ADR 0018 apply.** The first definition wins, in the order
  the files are read, `config.toml` first. A later alias of the same name
  is kept as `pack:name`, with a warning. So a shared file cannot hide a
  user's own alias.
- **An alias name cannot contain `:`,** in `config.toml` too, as for a
  command name, because `pack:name` holds the `:`.
- **A pack alias gets the checks of #557 and #558,** with the file in the
  message. A chain cannot run another alias from any file.
- **The keymap is built after the set files are read and merged,** so a
  `[keymap]` line can give a pack alias a key. Plain `--print-keymap` moves
  with it. `--print-keymap defaults` and `--print-editor-config` stay above
  every file read.
- **Aliases have no listed state.** The file is the unit, as for commands.

## Considered options

- **Pack aliases in the palette only, and keys only for `config.toml`
  aliases.** This kept the keymap build above the set files. Rejected: a
  pack alias that no key can run is a surprise. The reason for
  the old order was too wide. The rule of #191 and #366 is only that a
  command must not fail on a file that it does not read. Plain
  `--print-keymap` prints the map in force, and that map now depends on the
  set files.

## Consequences

- A set file that recon cannot read stops plain `--print-keymap`. The
  error names the file.
- A `--set` flag that names no set does not stop `--print-keymap`, because
  the keymap does not depend on it.
- The merged aliases go to the session beside the config, as the merged
  commands and rows do. The palette and the keymap see one list, and the
  config keeps only what `config.toml` said.
