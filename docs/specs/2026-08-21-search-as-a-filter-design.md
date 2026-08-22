# Search as a filter — design

**Status:** implemented
**Date:** 2026-08-21

## Motivation

`recon` has two ways to say "this line is interesting" and they do not know
about each other.

**Filters** are line-oriented, live in `FilterSet`, survive loading a new file,
colour what they match and dim what they don't, and drive `Ctrl-H`. They cannot
be stepped through — reaching the next match means hiding, pressing `j`, and
unhiding.

**Search** is span-oriented, lives inside the vendored `TextArea`, dies whenever
the buffer is rebuilt, highlights matched substrings, and drives `n`/`N`. It
cannot hide anything, and because it runs against the *buffer* rather than the
document, in `Mode::FilteredOnly` it cannot even see the lines that are hidden.

Both are useful. The goal is to have both available at all times without asking
the user to hold two mental models.

## The idea

Collapse the two notions into one:

> A line is **interesting** if it matches an enabled include filter, or if it
> matches the live search.

`Ctrl-H` uses it. `n`/`N` use it. Dimming uses a deliberately narrower
predicate — see "Dimming takes a narrower predicate than hiding" below.

The mechanism is to stop treating search as a separate concept. `/` becomes a
fast way to add a filter, and `Esc` a fast way to remove it. Everything filters
already do — outliving the file, honouring exclusion, responding to `!`, feeding
`Ctrl-H` — then applies to search for free, because search *is* a filter.

This is a smaller change than moving search into the document layer would have
been. `FilterSet` already sees every line of the file, hidden or not.

### What is deliberately given up

`n` becomes **line-oriented**. Today, three occurrences of `foo` on one line are
three stops. After this change they are one.

This is a real departure from vim, and it is accepted deliberately: `recon` is a
line-focused tool, and the alternative — `n` stopping three times on a
search-matched line but once on a filter-matched one — is a distinction that
cannot be explained without explaining the implementation. Simplicity wins.

Sub-line *highlighting* is not given up. Only sub-line *stopping*.

## Model — `src/filter.rs`

The search filter gets its own slot. It is emphatically **not** an element of
`filters`:

```rust
pub struct FilterSet {
    filters: Vec<Filter>,
    /// The live search: at most one, never numbered, replaced by each `/`.
    search: Option<Filter>,
    remembered: Option<Vec<bool>>,
    /// The search slot's enabled flag, captured alongside `remembered`.
    remembered_search: Option<bool>,
}
```

`Verdict::Included(usize)` is a **position** in `filters`. The doc comment on
`FilterSet::remove` already warns that positions invalidate cached verdicts. If
the search filter lived in that vector, every `/` and every `Esc` would renumber
the user's filters — filter 2 becoming filter 3 and back again as a side effect
of typing a search. The separate slot makes that structurally impossible.

A new verdict variant carries the distinction:

```rust
pub enum Verdict {
    Included(usize),
    /// Matched the live search rather than a numbered filter.
    Searched,
    Unmatched,
    Excluded,
}
```

### Evaluation order

`verdict()` gains one step, between exclusion and inclusion:

1. Any enabled **excluding** filter matches → `Excluded`. Unchanged, and this is
   what makes exclusion beat search without a new rule.
2. The **search** slot is present, enabled, and matches → `Searched`.
3. First matching enabled **including** filter → `Included(index)`.
4. Otherwise → `Unmatched`.

Search sits above the numbered filters so the pattern being actively probed wins
the colour on a line that several things match. The user's attention is on what
they just typed.

### Everything that learns the new variant

| Site | Change |
|---|---|
| `style_for` | `Searched` → the reserved search style |
| `any_including` | counts the search slot; becomes `pub` |
| `match_count` (`document.rs`) | counts `Searched` alongside `Included` |
| `recompute_visible` | `Searched` is visible in `FilteredOnly` |

`recompute_visible` still runs no regex, so `Ctrl-H` remains O(lines) and
instant. That property is load-bearing and must survive.

### Dimming takes a narrower predicate than hiding

Dimming is a **contrast** mechanism: unmatched lines recede so that coloured
matches stand out. Its value scales with how many things are being told apart.
A search on its own is one thing, and its hits are already marked loudly by the
span highlight — so dimming the rest of the file buys nothing and costs the
readability of the context the user searched in order to reach.

Two predicates therefore, each with one job:

```rust
/// Anything at all is marking lines — a numbered include filter, or the
/// live search. Drives Ctrl-H (including the issue #36 guard) and n/N.
pub fn any_including(&self) -> bool;

/// A *numbered* include filter is enabled. Drives dimming alone. The search
/// by itself does not dim: there is nothing to contrast it against, and its
/// hits already carry the span highlight.
fn any_numbered_including(&self) -> bool;
```

`style_for`'s `Verdict::Unmatched` guard moves to `any_numbered_including`.
Everything else keeps `any_including`.

The consequence, and it is deliberate: `/foo` on an unfiltered file highlights
and navigates but does not grey anything out, while `Ctrl-H` still collapses the
file to the matching lines. Adding a numbered filter switches dimming on,
because at that point there really are two things to distinguish.

**The cost.** The README describes dimmed lines as flipping "from dimmed to
gone", which makes dimming a preview of hiding. This weakens that in exactly one
case — search-alone, where nothing is dimmed yet `Ctrl-H` hides plenty. Accepted:
a user pressing a key that means "hide unmatched" is not surprised to get it.
The README sentence needs a corresponding rewrite.

### New methods

```rust
/// Set the live search, replacing any previous one. One search at a time.
pub fn set_search(&mut self, pattern: &str) -> Result<(), regex::Error>;

/// Drop the live search. No-op if there is none. Reports whether there was
/// one to drop, the same shape as `promote_search`: the `Esc` binding uses
/// this to skip the `refresh_view` a no-op press would otherwise pay for.
pub fn clear_search(&mut self) -> bool;

/// Move the live search into the numbered set, preserving its enabled
/// state, and empty the slot. Reports whether there was one to promote.
pub fn promote_search(&mut self) -> bool;
```

`set_search`, `clear_search` and `promote_search` all drop a pending `!`
capture — **both** `remembered` and `remembered_search` — matching what `add`
and `add_excluding` already do and for the same reason recorded there: a capture
that describes a set which no longer exists strands the `!` toggle. Dropping
only one of the two fields would leave the capture half-valid, which is worse
than dropping neither.

## Appearance

The search filter takes a **reserved style** — white and bold — outside
`PALETTE`. Two reasons:

- Drawing from `PALETTE` would make the search's colour depend on how many
  filters exist, so it would shift as filters come and go.
- A fixed colour gives the user one rule: **white means what I just typed.**

The `TextArea`'s existing `set_search_pattern` continues to paint matched spans
black-on-yellow. These compose: the line takes the search filter's foreground,
the matched substrings take the highlight background.

Only the search filter highlights spans, because `TextArea` holds exactly one
search pattern. This is treated as a feature rather than an inconsistency — the
live probe is visually distinct from the settled filters. Extending span
highlighting to every filter would require further work in the fork and is out
of scope.

**The span highlight follows the search filter's enabled flag.** Disabling the
search — with `space` on its row, or with `!` — must clear the `TextArea`'s
search pattern, and re-enabling must restore it. Without this, `!` leaves yellow
highlights glowing on a view where nothing is meant to be active, breaking the
"one keystroke back to an unfiltered view" the README promises. The pattern
itself is retained on the `Filter` throughout, so nothing has to be retyped;
only the `TextArea`'s copy is set and cleared.

```
FILTERS
  /  timeout          white, bold — the live one
  1  ERROR            yellow
  2  retry            cyan

VIEW (dimmed mode)
  1041  ERROR disk full             yellow line
  1042  conn timeout after 30s      white line, "timeout" on yellow
  1043  heartbeat ok                dim grey
  1044  ERROR timeout on socket     white line — search outranks filter 1
```

## Keys

| Key | Behaviour | Notes |
|---|---|---|
| `/` | Prompt, set the search filter, then behave exactly as `n` | |
| `n` | Next interesting line, wrapping | |
| `N` | Previous interesting line, wrapping | |
| `p` | Promote the search filter into the numbered set | No-op if no search |
| `Esc` | Clear the search filter | No-op if none; prompt-cancel still wins |
| `?` | **Unbound** | Reserved for the help view, issue #25 |

`/` is defined as "set the search, then do what `n` does" so there is one
movement path rather than two. It also means `/` correctly handles the buffer
rebuild that adding a filter triggers in `Mode::FilteredOnly`.

`n` and `N` wrap; `j` and `k` do not. In hide mode `n` is otherwise identical to
`j`, and wrapping is the only difference left between them.

`Esc` departs from vim, where it leaves the search pattern alone. Here it
removes the filter outright, because a filter that cannot be turned off is a
leak — a search typed ten minutes ago would silently keep changing what is on
screen with no way to stop it.

### Where the keys live

`n` and `N` move out of `FileView::handle_events` and into `App`, because they
now need `Document::verdicts()`. `H` and the filter-pane keys are already
handled at that level, so the pattern exists.

`FileView::search_reverse` is deleted. With no backward search there is no
direction to remember: `N` simply means previous.

Handling `n`/`N` in `App` must not make them global. They stay scoped to the
file view: pressed while the navigator or the filter pane has focus, they must
reach that pane's own handler as they do today, not move the file view's cursor
behind the user's back. `Esc` and `p`, by contrast, are deliberately global.

### Promotion

`p` appends the search filter to `filters` with the next palette colour,
preserving its enabled state, and empties the slot. It is global rather than
pane-scoped: the user has just searched and should not have to go find the
filter pane to keep the result.

The workflow it exists for is building a set by probing:

```
/timeout  →  look  →  p   (becomes filter 3, cyan)
/retry    →  look  →  p   (becomes filter 4, green)
```

Nothing is retyped, and the result is a set worth saving under issue #8.
Promotion preserves the enabled state rather than forcing `true`, so a search
toggled off with `space` is not silently switched back on.

## Filter pane

The pane gains a `/` row at the top, shown only while a search exists, with `/`
where a number would be. `space` toggles it and `d` deletes it, the same as any
other row.

The pane's selection index now addresses a list with one row that is not an
element of `filters()`. That offset — in `clamp_selection`, in `handle_key`, and
in the `FilterCommand` indices the pane reports back — is where bugs will hide,
and it gets dedicated tests.

## Folding in issue #36

Issue #36 reports that `Ctrl-H` with no filters shows a blank pane. The cause is
an asymmetry between the two modes.

Dimming guards itself (`src/filter.rs:269`):

```rust
Verdict::Unmatched if self.any_including() => Some(DIM_STYLE),
```

Hiding does not (`src/document.rs:86`):

```rust
(Mode::FilteredOnly, Verdict::Unmatched) => false,
```

Dimming asks "is anything actually including?" before dimming. Hiding never
asks. Verified: with no filters, and also with **only excluding filters**, hide
mode produces an empty `visible`. The exclude-only case is a second, unreported
instance of the same bug — `filter.rs`'s own
`excluding_filters_alone_do_not_dim` test shows dimming already handles it.

**This design makes the bug far easier to hit.** Today it takes deleting every
filter or pressing `!`. Afterwards, `Esc` in hide mode blanks the pane
instantly, and `!` does too now that it disables the search. The fix therefore
ships with this change rather than after it.

The fix is to give hiding the guard dimming already has:

```rust
(Mode::FilteredOnly, Verdict::Unmatched) => !filters.any_including(),
```

`recompute_visible` will need the `FilterSet` passed in, or the predicate cached
on the `Document` at `evaluate` time. The latter is preferred: it keeps
`recompute_visible` free of borrows and preserves its independence from the
filter set, which is what makes the `Ctrl-H` path cheap.

In the new model `any_including()` already means "any enabled include filter, or
the live search", so this is the same predicate in both modes and "matched"
finally means one thing throughout.

Two consequences worth stating:

- **Directory skim is preserved.** With filters enabled, a file with no matches
  still renders blank. Verified.
- **Blank becomes unambiguous.** Today a blank pane means either "no hits" or
  "no filters". Afterwards it means only "no hits" — which is exactly what the
  skim feature needs it to mean.

Issue #36 stays open for its other half: a visible indicator that hide mode is
on. That is a separate want and survives this fix.

## Out of scope

| Deferred | Where |
|---|---|
| In-app help view | #25 — this change only frees `?` |
| Editing an existing filter's pattern | #37 |
| Interactive regex builder with history | #38 |
| Saved filter sets | #8 — promotion feeds it |
| Hide-mode indicator | #36, remaining half |
| Span highlighting for every filter | Needs more work in the `tui-textarea` fork |

## Testing

- **`filter.rs`** — evaluation order (exclude beats search beats include);
  `set_search` replaces rather than stacks; `promote_search` preserves the
  enabled state, takes the next palette colour, empties the slot, and leaves
  existing filter *indices untouched*; the reserved style is not a `PALETTE`
  member; `!` round-trips the search slot's enabled flag via `remembered_search`.
- **The two predicates** — `any_including` counts the search slot,
  `any_numbered_including` does not; a search alone leaves `style_for` returning
  `None` for `Unmatched` while `Ctrl-H` still hides those lines; adding a
  numbered filter switches dimming on. This pair is the subtlest thing in the
  design and needs the most direct coverage.
- **Highlight follows enabled** — disabling the search clears the `TextArea`
  search pattern; re-enabling restores it; `!` and `space` both go through that
  path; the pattern itself survives on the `Filter` and never has to be retyped.
- **`document.rs`** — `Searched` is visible in `FilteredOnly` and counted by
  `match_count`; the #36 guard, covering no-filters, exclude-only, and the
  skim case that must stay blank; `recompute_visible` still runs no regex.
- **`lib.rs`** — `/` sets the search and moves like `n`; `n`/`N` wrap and visit
  both filter and search hits in source order; `n` with nothing interesting is a
  quiet no-op; `Esc` clears the search but a prompt-cancel `Esc` still cancels
  the prompt; `p` promotes; `?` is inert; the search filter survives a file load
  and a navigator preview.
- **`filterlist.rs`** — the `/` row appears only when a search exists;
  selection, `space` and `d` address the right filter across the offset.

## Migration notes

On a file with no filters, `/foo` behaves as it does today: highlights, no
dimming. The narrower dimming predicate is what preserves that. What is new is
that `n`/`N` now step line-by-line rather than span-by-span, that `Ctrl-H` will
now collapse the file to the matching lines, and that `Esc` clears the search.

The README needs three edits:

- The line describing dimmed lines flipping "from dimmed to gone" overstates the
  relationship now. Dimming marks unmatched lines *when marking helps*; `Ctrl-H`
  hides what is not interesting. They coincide whenever a numbered filter is
  enabled, which is the common case, but not when a search is the only thing
  active.
- `?` no longer searches backwards; `n`/`N` cover both directions.
- The keybindings table gains `p` and `Esc`.
