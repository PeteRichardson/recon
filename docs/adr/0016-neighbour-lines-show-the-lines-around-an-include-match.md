# ADR 0016: neighbour lines show the lines around an include match

**Status:** accepted
**Date:** 2026-11-02

## Context

A filter often names a line a little above or below the data the user wants
to read. A log says `Sanity Check Failed:` and the actual payload is the
next few lines, but no filter matches them. The line to name is easy; the
interesting data is not the match itself. grep solves the same problem with
`-A`, `-B` and `-C`, which show a number of lines around a match. recon's
*context* sense is a different thing — a colour that is not a target — and
does not show lines around a match.

## Decision

- **A per include filter *neighbour count*:** a `(before, after)` pair,
  defaulting to `0 0`, in the filter editor and kept by a saved filter. A
  filter at the default omits it from the saved form. The terms match grep
  so the analogy holds: *before* is `-B`, *after* is `-A`, and one value set
  for both is `-C`.
- **A *neighbour line*:** a line within `[match - before, match + after]` of
  a line an include filter matches, in file order, whether or not it matches
  a filter itself. An exclude filter still removes one. A neighbour line is
  not interesting: `n` and `N` step over it, and it shows in the default
  foreground, never in a filter colour — that is how it reads as "near a
  match" rather than a match.
- **A new verdict, `ContextKeep`,** marks a line kept only as a neighbour.
  `shows` and `style_for` gain its arm, and `is_interesting` stays
  `Included` only, so navigation is untouched. The new verdict is what lets
  a neighbour line render *dim* in hide mode; `style_for(Unmatched)` returns
  no style there, which would show it bright.
- **The expansion runs where the verdicts are written:** in `evaluate`, and
  again after `append` when a followed file grows. A verdict does not depend
  on the mode, so the `u` toggle still needs only `recompute_visible`, which
  writes no verdict. Hide mode keeps a `ContextKeep` line; in dim mode every
  line already shows, so only the rendering toggle applies. The pass gathers
  each `Included` match whose filter has a non-zero count and marks each
  unmatched line in `[match - before, match + after]`, so ranges that overlap
  are one band. A line with a filter of its own keeps its verdict.
- **One global, session-only bright/dim flag, default bright**, parallel to
  the `u` dim/hide toggle: a named palette command flips it, and there is no
  `config.toml` key. When dim, a neighbour line recedes among the dimmed
  lines in either mode.
- **`--emit lines` writes neighbour lines by default.** The command line
  carries the same default-on idea as the bright/dim flag, as
  `--[no-]neighbours`: neighbour lines are emitted with the matches unless
  `--no-neighbours` is given, so the opt-out expresses the setting whose
  default is "on". The two are separate surfaces with no runtime coupling
  (#587): `Ctrl-n` changes how the view draws them, not what is emitted. The
  same rule applies in batch mode and to `q` and `:w` in the TUI, so a file
  emits the same lines whichever way it is read.
- **The *context* sense is unchanged and keeps its name.** No rename; the
  feature carries its grep comprehension in a help line ("like grep
  `-A`/`-B`/`-C`") instead of in a word.

## Considered options

- **Rename the *context* sense to *metadata* to free the word
  *context* for this feature.** Rejected. The sense is behavior-anchored and
  broad; *metadata* fits attribute lines (host, date) but not the structure
  lines (section headers, step boundaries) the sense also colours. And
  *context* is already a palette command (`context <pattern>`), the `m`
  toggle key and the `ctx` row label, so both a format and a vocabulary break
  would be paid.
- **One global neighbour count shared by every include filter.**
  Rejected. grep's `-C` is per invocation, and the common use is after-biased
  (`before 0, after N`), which a single value cannot express.
- **Encode the count in the pattern text.** Rejected. The pattern is the
  regex that decides what a line *is*; a `-C` inside a live regex is fragile
  and conflates classification with extent.
- **Split the *context* sense into *fact* and *structure* senses.**
  Rejected. The engine cannot tell the two apart — their behavior is
  identical — so they are not two senses. This is the principle now stated
  in the *Sense* glossary entry.
- **A per-filter bright/dim state.** Shelved as YAGNI: neighbour lines carry
  no colour, so two filters' neighbour lines look alike anyway; one global
  flag is enough.
- **A visual `-` marker on neighbour lines (grep-style).** Shelved. Default
  foreground against the coloured matches is already a clear signal; a
  marker is the first thing to add later if it ever feels weak.

## Consequences

- **Sits on ADR 0009 (batch always hides):** `--emit lines` is already in the
  hide posture where the neighbour expansion applies, so the batch path
  reuses the same `shows()` rule and the two agree.
- **Sits on ADR 0005 (batch has a flag):** the CLI surface grows one boolean,
  `--[no-]neighbours`.
- **A neighbour line and a matching line are different verdicts:** the new
  `ContextKeep` keeps "lines to show" and "lines to land on" apart, so the
  view, batch and navigation answer one rule.
- **Room to grow:** a per-filter bright/dim state or a `config.toml` default
  for the flag can be added later if a real user asks; the session-only flag
  and the omitted-at-default count leave that room.