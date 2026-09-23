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
