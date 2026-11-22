# ADR 0020: the built-in set is named `code`

**Status:** accepted
**Date:** 2026-11-22

## Context

recon has one built-in filter set. Its filters select the lines that start
a function, a struct, a module or one of eight other kinds, from the syntax
grammar of the file. The set was named `definitions`, from syntect's word
for these items. A user types the set name in `--set`, in the palette and
in `[sets.<name>]`, so a long name costs time each day. "Definitions" also
says nothing about code to a user who does not know syntect (#625).

## Decision

- **The set is named `code`.** The flag is `--set code`, and the table is
  `[sets.code]`. In prose, one item is a *code item*, and a filter of the
  set is a *code filter*. "Definitions" stays only as a note on syntect's
  word.
- **No alias.** `[sets.definitions]` in a set file stops the load with a
  message that gives the new table name. `--set definitions`, and
  `set`/`unlist` in the palette, give "unknown set" with the new name. recon
  has few users, and one name is simpler to read and to test than two.
- **`code` is now a reserved set name.** A set file that defines its own
  `[sets.code]` with filters is refused, as a `definitions` table with
  filters was before.
- **The Rust names follow.** `Predicate::CodeItem`, `syntax::code_items`,
  `CodeItems` and `CODE_SET`. `Kind`, `KindSet` and the profile names do
  not change.

## Considered

- **Keep `definitions` as an alias.** Rejected: two names for one set must
  both work in each place that takes a set name, and the old name stays in
  the help and in saved files for ever.
- **`outline` or `symbols`.** Rejected: `sections` in Markdown is an
  outline, but a struct is not, and "symbol" is the word of a language
  server, which recon does not use.
