# Listed and unlisted filter sets — design

**Status:** proposed
**Date:** 2026-09-24
**Issue:** #281
**Decision record:** [ADR 0002](../adr/0002-listed-is-a-startup-value.md)
**Builds on:** [Saved filter sets](2026-09-03-saved-filter-sets-design.md)

## Problem Statement

Every known filter set has a row in the filter pane. A set that is not
relevant to the file the user reads still uses a row. An INFO/WARNING/ERROR
set is useful for a log and is noise for source code. The only way to remove
a set now is to quit, delete it from `filters.toml`, and start again.

Only the user knows which sets are relevant, so the user needs a fast way to
remove a set from the pane and to add it back.

## Solution

A second state for a set, separate from *enabled*:

| State | Meaning |
|---|---|
| **Known** | Recon found the set at startup. |
| **Listed** | The set has a row in the filter pane. |
| **Unlisted** | The set has no row and no effect on what the user sees. |
| **Enabled** | A listed set whose filters show in the pane and can be toggled. |
| **Disabled** | A listed set that shows only its header row. |

The rule is **enabled ⇒ listed**. There is no unlisted set that is enabled.

A **set picker** (`L`) lists every known set. The user checks or unchecks a
set there to list or unlist it.

## The states

- The scratch set is always listed. It is not in the picker.
- The built-in `definitions` set is a normal set. The user can unlist it.
- **Unlisting** a set also disables it. The visible lines, the highlights and
  the counts change at once. Its filter flags are kept.
- **Listing a set again** during a session gives a disabled set, with its
  filter flags as they were, in its `priority` position. `autoload` does not
  apply: it is a startup value.

## The file

Two new optional keys in `[sets.<name>]`:

| Key | Default | Meaning |
|---|---|---|
| `listed` | `true` | listed at startup |
| `description` | none | one line of plain text for the picker |

- The `[sets.definitions]` table now also accepts `listed` and
  `description`. Recon supplies a description for `definitions`, and the
  table can override it.
- **`listed = false` wins over `autoload = true`.** The file is not refused.
  The reason is in ADR 0002: to unlist a set, the user changes one key.
- `S` (save the scratch set) writes no `listed` and no `description`.
- The default `true` keeps the current behaviour for every existing file.

## The command line

- `--set NAME[:PROFILE]` lists the set and enables it, whatever `listed` and
  `autoload` say. This is its current meaning plus the listing.
- `--unlist NAME` unlists a set at startup. The user can repeat it. It is for
  headless runs ("my usual sets, without this one") and also works for the
  TUI. An unknown name is refused the same way `--set` refuses one.
- `--set X` together with `--unlist X` is refused before the terminal is
  taken.

## The set picker

- `L` opens it from the filter pane, the file view and the navigator. It does
  not open from inside a prompt. `?` documents it.
- It covers the whole recon window. It does not cover only the file view,
  because the filter pane under it would not change while the user toggles,
  and that would look like a bug.
- It has one row for each known set except the scratch set, **in
  alphabetical order**: `[x]` or `[ ]`, the name, and the description (blank
  if there is none). A long description is truncated. The list scrolls.
- **The checkbox means listed.** It does not show enabled. A listed set that
  is disabled is also checked.
- `j`/`k` and the arrows move the selection. Space toggles the selected row.
- `/` is a **regex** search over the name and the description, the same as
  every other `/` in recon. It moves the selection as the user types and
  never hides a row (ADR 0001). The picker has its own **history** for
  Up and Down.
- **Changes are staged.** Enter applies all of them and closes the picker.
  Esc discards them and closes the picker. After the picker closes, the user
  is where they were before `L`.
- The picker is never empty, because `definitions` is always known.

## Interactions

- **Solo (`s`)**: un-solo restores the snapshot only for sets that are still
  listed. An unlisted set stays unlisted and disabled.
- **Reset (`R`)**: does not change the listed state. It enables an autoload
  set only if the set is listed now.
- **`--emit`**: no pane, but the same rules. `--unlist` removes a set from
  the filters that decide the output.
- Restoring a state is not a global undo. Unlisting is rare, and it has
  consequences. The user can list a set again with `L`.

## Out of scope

- Named groups of sets that load as one (`recon --fss rust-code`)
- Set recommendations from the file type or the context
- Sharing and downloading sets
- #46, `RECON_FILTER_PATH`. This feature becomes more useful with more
  known sets, but it does not depend on #46.

## Testing

- Loader: `listed` and `description` parse, have correct defaults, are
  accepted on the built-in table, and `listed = false` with `autoload = true`
  gives an unlisted, disabled set.
- Flags: `--unlist`, `--set` on an unlisted set, the refused pair, an unknown
  name.
- Model: unlisting an enabled set disables it and changes the visible lines;
  listing again gives a disabled set with its flags; un-solo and `R` keep
  enabled ⇒ listed.
- Picker: alphabetical order, the staged apply on Enter and discard on Esc,
  the regex search over the description, the history, the return to the
  origin.
- Headless: `--emit` with `--unlist` does not use the unlisted set's filters.
