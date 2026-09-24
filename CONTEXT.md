# recon

A terminal file viewer for reading a log one filter at a time. The vocabulary
below is the one the README, the issues and the code use for what the user
sees. It is a glossary, not a spec.

## Language

### Filters

**Filter**:
A regular expression the user keeps in the filter pane, with a sense and a
colour. A filter decides what a line is, not where the cursor goes.
_Avoid_: pattern (that is the text of a filter), search

**Sense**:
What a match means for a line: *include* colours the line, *exclude* removes
it, *context* keeps it on screen without colouring it.

**Filter set**:
A named group of filters loaded together and toggled as one. The unnamed set
the user builds by hand is the *scratch set*.

**Known set**:
A filter set recon found at startup, listed or not.
_Avoid_: discovered, available

**Listed / Unlisted**:
Whether a known set has a row in the filter pane. An unlisted set has no
effect on what the user sees. The scratch set is always listed.
_Avoid_: loaded, unloaded, hidden, shelved

**Enabled / Disabled** (of a set):
Whether a listed set's filters show in the pane and can be toggled. A disabled
set shows only its header row. Only a listed set can be enabled.
_Avoid_: active, inactive, expanded, collapsed

**Autoload**:
A set's startup value of *enabled*. `listed` is its startup value of *listed*,
and an unlisted set never autoloads. `--set` enables a set whatever its
autoload says.
_Avoid_: default (that is a profile name), startup set

**Set picker**:
The list that covers the whole recon window, of every known set except the
scratch set, where the user lists and unlists sets. Its checkbox means *listed*, never *enabled*.
_Avoid_: catalogue, set list, filter list

**Interesting line**:
A line an enabled including filter matches. `n` and `N` step between
interesting lines when no search is set, and hide mode keeps only them.

### Search

**Search**:
A regular expression the user types after `/` (or takes from the word under
the cursor with `*`). It moves the cursor to its next hit and highlights the
hits in the window. A search is a motion, not a filter: it never changes which
lines are visible, does not dim, and does not answer to `!` or `u`.
_Avoid_: live search, probe, search filter

**Hit**:
A line the search matches. A hit is always a visible line, because the search
runs over the visible lines only.
_Avoid_: match (used for filters)

**Origin**:
Where the user was when they pressed `/`: the cursor and scroll in the file
view, or the selected row in the navigator. Esc in the prompt returns there;
Enter keeps the position the search reached.

**Promote**:
Turn the current search into a numbered include filter with `p`. The pattern
crosses from search to filter; the search itself is cleared.

**History**:
The patterns Enter committed in a `/` prompt, newest first, that Up and Down
recall in the next one. The file search and the filename search each keep
their own; a filter prompt keeps none.
_Avoid_: recent searches, last patterns

### The view

**Visible lines**:
The lines the file view can show, in file order: every line an exclude filter
does not remove, and in hide mode only the interesting lines. Dimmed lines are
visible.
_Avoid_: on screen, shown lines

**Window**:
The slice of the visible lines that fits on the screen right now.
_Avoid_: viewport, visible (when the window is meant)

**Dim mode / Hide mode**:
The two ways to treat a line that is not interesting: keep it and grey it, or
remove it from the visible lines. `u` toggles between them.
