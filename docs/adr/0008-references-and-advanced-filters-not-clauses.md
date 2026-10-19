# ADR 0008: Cross-set AND uses references and advanced filters, not clauses

**Status:** accepted
**Date:** 2026-10-18

## Context

#40 proposed filter groups: a filter set as an OR of groups, each group an
AND of filters, built with a "joins the row above" flag. The real need is
wider than that. An organization gives out a filter set (`org:Summary`), and a
team must narrow it with its own patterns (`Updates`) without copying the
org's pattern, because a copy drifts. A group that spans two sets cannot work:
`org:Summary` must be off on its own row (or every summary line is included)
and still on inside the group, and a group cannot be saved in one set's file.

## Decision

- **A reference** `${set:name}` (or `${name}` in the same set) in any pattern
  expands to the current pattern of that filter, wrapped in `(?:…)`. It takes
  only the pattern — never the sense, colour or enabled state. It can point
  into an unlisted set. It is expanded again whenever a pattern changes.
- **A broken reference** (target gone, a definition filter, a cycle) makes only
  its filter match nothing in the TUI, with the reason on its row. A batch run
  stops before any output, naming each broken reference, but only for filters
  that take effect.
- **An advanced filter** is one the user marks (`engine = "advanced"`) to use
  Oniguruma, which is already linked through syntect, so lookahead can express
  "both, in either order". It is a new `Predicate` variant; every other filter
  keeps the `RegexSet` fast path. A line that passes the step limit
  (`[filters] advanced_step_limit` in `config.toml`) counts as no match, and
  the row shows how many lines did.
- **No clauses, no global `&` mode.** The #39 global AND mode is removed.
- **Work:** pattern references #473, advanced filters #474 (blocked by #473).
  #40 is closed as superseded.
- **No versioning.** A reference means the current pattern; a copy is a pin.

## Considered options

- **Clauses within one set (#40 as written).** Rejected: it cannot reuse a
  filter from another set without a copy.
- **Clauses that link rows across sets.** Rejected: a member cannot be off on
  its own row and on in the clause at the same time.
- **Nested AND/OR layers, or an expression language.** Rejected: almost
  boolean logic, but not quite; it loses the per-filter toggle and a clear
  owner for each colour, and needs a parser.
- **Advanced regex alone.** Rejected: still needs the copy.
- **Fail at startup on a broken reference.** Rejected for the TUI: one renamed
  filter in an org file must not stop recon for everyone who uses it.

## Consequences

- `${…}` goes into shared set files, so the syntax is hard to change later.
- In an advanced filter, Oniguruma reads a referenced pattern written for the
  `regex` crate. Simple patterns mean the same in both; `\d`/`\w` Unicode
  rules, `\z`, class set operations and backreferences may not.
- Oniguruma backtracks. The step limit, not the engine, keeps a bad pattern
  from stalling a large file.
