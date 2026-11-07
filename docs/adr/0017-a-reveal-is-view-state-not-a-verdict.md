# ADR 0017: a reveal is view state, not a verdict

**Status:** accepted
**Date:** 2026-11-06

## Context

Hide mode with the definition filters on shows the structure of a source
file: one line for each function, impl or type. With *end lines* (the
closing brace of each match's *extent*) it reads as an outline. The user then
wants to read one function without leaving hide mode and without a new
filter. The same need exists in a log: hide mode shows the interesting
lines, and the user wants to see the hidden lines between two of them.

These are one need. The only difference is where the range comes from: the
extent of a definition, or the gap to the next visible line.

## Decision

- **A *reveal* is a range of file lines kept on the document, for the
  session.** `>` makes one for the cursor line; `<` closes the smallest one
  that holds the cursor and moves the cursor to the reveal's first line.
- **The range:** on a line that starts or ends a definition with an extent,
  the extent; on another line, every hidden line up to the next visible
  line, with no limit. A large gap is no worse than `u`, and `<` undoes it.
- **Reveals are not verdicts.** `recompute_visible` shows a line when its
  verdict shows it *or* a reveal holds it. `>` and `<` do not run the
  filters, so they cost what `u` costs. A verdict still does not depend on
  the view (ADR 0016).
- **Reveals use file line numbers,** so a change to the filters keeps them.
  A reload of a changed file removes them.
- **Reveals can nest.** `>` inside a reveal records a smaller reveal. It
  changes nothing on screen until the outer reveal closes; then the inner
  one stays. That is how the user keeps one function of a revealed impl.
- **An exclude filter still removes a revealed line,** as it removes a
  neighbor line.
- **A revealed line renders as it would with no filter enabled** (full
  syntax colour in a source file), unless a filter matches it.
- **In dim mode, `>` and `<` do nothing** and show a message: every line is
  already visible.
- **`:w` and `q` write revealed lines.** Batch mode has no reveals.

## Considered options

- **A reveal as a verdict (like `ContextKeep`).** Rejected. A verdict is a
  property of the line under the filters; a reveal is a choice the user made
  in the view. As a verdict, every `>` would run the filter pass again, and
  a filter change would have to carry the user's choices.
- **Two features: one for definitions, one for gaps.** Rejected. One range
  rule with two sources is one key pair and one state, and a Python file,
  where the grammar gives no extent, still gets a useful gap reveal.
- **Reveal at most one screen of lines per `>`.** Rejected by the user:
  one key that shows a full gap is as easy to undo as `u`.
- **`:w` and `q` ignore reveals,** so that the TUI and batch mode emit the
  same lines for a file (the ADR 0016 rule). Rejected: the user reveals lines
  to read them, and `:w` must write what the user reads.

## Consequences

- **An exception to ADR 0016:** `--emit lines` and `:w` agree only when no
  reveal is open. End lines are verdicts, so they stay the same in both.
- **End lines and reveals are separate work.** End lines change the
  verdicts; reveals change only the visible lines. A gap reveal needs no
  syntax work, and a definition reveal needs both.
