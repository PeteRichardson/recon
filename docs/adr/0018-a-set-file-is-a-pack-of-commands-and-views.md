# ADR 0018: a set file is a pack of commands and views

**Status:** accepted
**Date:** 2026-11-15

## Context

A tool such as an MPEG dumper needs three things in recon: filters for its
output, a `[commands]` entry that runs it, and a `[[filetype]]` row that
shows it for its files. Before this, only the filters could go in a set
file on the filter path. The command and the row had to go in each user's
`config.toml`, so a team could not share a tool with one file.

## Decision

- **A set file can also have `[commands]` and `[[filetype]]`,** in the
  format of `config.toml`. Such a file is a *pack*. Its name is the file
  name without `.filters.toml`; a bare `filters.toml` is the pack
  `filters`. There is no key for the name.
- **The first definition wins,** in the order the set files are read:
  `config.toml` first, then the personal set file, then the path in order,
  then file-name order. A later command of the same name is kept as
  `pack:name`, with a warning. Of two pack rows for the same `glob` and
  `mime`, the first is used, with a warning for the other.
- **`pack:view` always means that pack's view.** A bare name in a pack's
  own row means its own command first, then the first definition. A command
  name in a pack cannot contain `:`.
- **A pack never changes the first view of the user's row.** A pack row for
  the same files as a `config.toml` row adds its views to that row, after
  the user's views and before the built-in views at the end of the row. A
  user who names `pack:view` in their row puts it where they wrote it. A
  pack row with new criteria goes after every `config.toml` row and before
  the built-in rows. A pack's `handoff` fills a row that has none.
- **The merge is done once, at startup,** by `views::pack::merge`, before
  the view table is built. The merge rewrites each view name to the
  command's key, so the table knows nothing of packs.
- **Commands and rows have no listed state.** The file is the unit: while
  it is on the path, its commands and rows are in force. Filter sets keep
  *listed* as before.

## Considered options

- **Pack views before the user's views.** Rejected: installing a team file
  would change what the user sees first for a file type they set up.
- **A refusal when two files define one command.** Rejected: two teams can
  pick one name by chance, and a user cannot edit a shared file. A warning
  and a `pack:name` keep both usable.
- **A `name =` key in the file.** Rejected: the file name is already
  unique in a directory, and a second name could disagree with it.

## Consequences

- A pack row can merge only into a `config.toml` row. The built-in rows
  have no `glob` or `mime`, so a pack row for pictures is a new row before
  them, not a change to them.
- The merge is the one check of view names, for `config.toml` rows too:
  `Config::load` no longer checks them, because it cannot see the packs. A
  name no file defines refuses at startup, with the file and the row.
- Of two packs with one name (two directories that each hold
  `filters.toml`), the first wins and the other is ignored with a warning,
  so that `pack:view` names one pack.
- A command name with a `:` is refused in `config.toml` too.
- The include and ignore files of #603 are the control for which packs are
  in force.
