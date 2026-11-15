# ADR 0019: include and ignore lists select the set files of a directory

**Status:** accepted
**Date:** 2026-11-15

## Context

recon reads every `filters.toml` and `*.filters.toml` in each directory on
the filter path. A shared repository of set files can have hundreds of
files. recon then parses all of them at startup, and the filter pane shows
all of their sets. A pack (ADR 0018) adds commands and `[[filetype]]` rows,
which have no listed state, so `--unlist` and the set picker cannot remove
them. The control must select whole files, before they are read (#603).

## Decision

- **A directory selects its files with `.include` and `.ignore`.** Each line
  is one glob, in the `[[filetype]]` glob format, matched against the file
  name. Blank lines and `#` lines are skipped. The files travel with a
  shared repository, so it can ship its own defaults.
- **`[filter_dirs."<dir>"]` in `config.toml` adds personal patterns.** Its
  `include` and `ignore` arrays are added to the directory's own lists, not
  put in their place. A user can add one pack to a shared list, or drop one,
  without a copy of the list. The key is the directory as the path names
  it, with `~` expanded.
- **An include list, when there is one, is the whole selection.** A
  `.include` file or an `include` key makes one. An empty list reads no
  file. With no list, every set file is read, as before.
- **Ignore wins.** A file that both lists match is not read. To change a
  pack, a user edits the pack file.
- **Every directory, and the config directory too.** But
  `~/.config/recon/filters.toml` is always read: it is the file `S` writes,
  and a saved set must be found at the next start.
- **The gate is at startup only.** A file that is not read is not opened,
  so an error in it does not stop recon, and its sets are not in the set
  picker (`L`). `L` stays the control for the sets of the files that were
  read.

## Considered

- **Full gitignore syntax** (the `ignore` crate). Rejected: a new
  dependency, and `!` negation conflicts with "ignore wins". recon reads
  only the top level of a directory, so path anchors add nothing.
- **Personal lists that replace the directory's lists.** Rejected: to change
  one line, the user must copy the full shared list and keep it current.
- **An environment variable.** Rejected: a list of patterns per directory
  does not fit in one variable.
