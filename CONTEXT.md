# recon

A terminal file viewer for reading a log one filter at a time. The vocabulary
below is the one the README, the issues and the code use for what the user
sees. It is a glossary, not a spec.

## Language

<!-- ANCHOR: glossary -->
### Filters

<a id="filter"></a>**Filter**:
A regular expression the user keeps in the filter pane, with a sense and a
color. A filter decides what a line is, not where the cursor goes.
_Avoid_: pattern (that is the text of a filter), search

**Sense**:
What a match means for a line: *include* colors the line, *exclude* removes
it, *context* colors it too, but it is not an interesting line.

**Filter set**:
A named group of filters loaded together and toggled as one. The unnamed set
the user builds by hand is the *scratch set*.

**Reference**:
A part of a filter's pattern that stands for the current pattern of a named
filter in a known set, so that two filters cannot drift apart. It takes only
the pattern: never the other filter's sense, color or enabled state. A
reference whose filter cannot be found is *broken*: its filter matches
nothing in the TUI, and a batch run stops before any output.
_Avoid_: link, include, import, macro

**Advanced filter**:
A filter the user marked to use the richer regular-expression engine, which
can test what is ahead of or behind a point in the line: for example "has
`Summary` and `Updates` in either order". It can be slow on some lines, so a
line that takes too many steps counts as no match, and the filter shows how
many lines did.
_Avoid_: fancy, PCRE, lookaround filter

**Known set**:
A filter set recon found at startup, listed or not.
_Avoid_: discovered, available

**Listed / Unlisted**:
Whether a known set has a row in the filter pane. An unlisted set has no
effect on recon — not on what the user sees, and not on the pattern limit —
except that it can be listed, and that it supplies patterns to references. A
set listed again comes back as its file describes it. The scratch set is
always listed.
_Avoid_: loaded, unloaded, hidden, shelved

**Enabled / Disabled** (of a set):
Whether a listed set's filters show in the pane and can be toggled. A disabled
set shows only its header row. Only a listed set can be enabled.
_Avoid_: active, inactive, expanded, collapsed

**Autoload**:
A set's startup value of *enabled*. `listed` is its startup value of *listed*,
and an unlisted set never autoloads. `--set` enables a set whatever its
autoload says; `--unlist` unlists it whatever its `listed` and autoload say.
_Avoid_: default (that is a profile name), startup set

**Set picker**:
The list that covers the whole recon window, of every known set except the
scratch set, where the user lists and unlists sets. Its checkbox means *listed*, never *enabled*.
_Avoid_: catalogue, set list, filter list

**Filter editor**:
The screen that covers the whole recon window, where the user writes a
filter's pattern against the open file and sees every line it matches while
typing. `f I` opens it on a new filter, and `f C` on the selected one.
_Avoid_: editor alone (that is the external editor `o` and `O` open), regex builder

**Line mark**:
The act of marking a line in the filter editor: `+` marks it as one the
pattern must match, `-` as one it must not match, and `=` clears the mark.
A marked line is a check. The code calls it `Mark` and its keymap actions
`filtereditor.mark.*`.
_Avoid_: check (that is what a marked line is, not the act), tag

**Check**:
A line marked in the filter editor as one the pattern must match (`+`) or
must not match (`-`). It passes or fails under the pattern as typed. `Enter`
keeps each check with the filter as an example.
_Avoid_: example (the word for a check once it is saved with the filter), test

**Example** (of a filter):
A whole line kept with a filter that its pattern must match or must not
match: a check that `Enter` kept. A pattern that fails an example does not
replace the filter's pattern. The filter editor shows each example as a
check again, also one the open file does not have.
_Avoid_: check (that is an example while the editor is open), sample, test case

**Matches only**:
The filter editor's own hide mode: it shows only the lines the pattern
matches and the marked lines. `u` on the lines toggles it. It does not change
the main window's hide mode.
_Avoid_: hide mode (that is the main window's)

**Version** (of a pattern):
A pattern the filter editor keeps so that `Ctrl-z` can go back to it: a
valid pattern at a pause in the typing, at `Tab`, or before an undo or redo.
Versions last only while the editor is open.
_Avoid_: history (that is the prompt's list of earlier patterns), revision

**Description** (of a filter):
One line that says why a filter exists. It is for people only, and is never
given to a model. A set has a description of its own, for the set picker.
_Avoid_: comment, note

**Prompt** (of a filter):
One line in plain language that says what a filter's lines look like: the
text a model will write the pattern from. Without a model it is only text
kept with the filter; it never changes the pattern.
_Avoid_: query, request (that is one turn of a talk with the model)

**Generated** (of a filter):
A filter whose pattern the model wrote from its prompt. It stays generated
only while its `generated_from` hash agrees with the prompt and the pattern:
a change to either one, in recon or in the file, makes it an ordinary
filter. Only a request or `Ctrl-r` in the filter editor changes a generated
pattern; a load never does.
_Avoid_: AI filter, model filter, regenerated (that is the act of `Ctrl-r`)

**Request**:
One turn of a talk with the model in the filter editor: what the user types
on the request line and sends with `Enter`. The model answers with a pattern
and an explanation. recon shows the pattern only when it compiles and passes
every check; otherwise it sends the failure back and asks again, up to 3
tries. The requests the model answered go to it again with
each later request while the editor is open, so the pattern improves step
by step. A request is not kept with the filter; the prompt is.
_Avoid_: prompt (that is the filter's), query

**Consolidated prompt**:
The one prompt the model writes from all of a session's requests when the
user saves, in place of the requests. The user can edit it before the save;
Esc saves with the prompt from before. After it, the session has no
requests.
_Avoid_: summary, merged prompt

**Phrase mark**:
A part of a line the user selected with the mouse in the filter editor. It
is a hint to the model and never a check: it does not pass or fail, and
Enter does not keep it with the filter.
_Avoid_: check (only a line mark is a check), highlight, selection

**Interesting line**:
A line an enabled include filter matches. `n` and `N` step between
interesting lines when no search is set. A line a context filter matches is
not interesting, but hide mode keeps it.

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
view, or the selected row in the explorer or the set picker. Esc in the
prompt returns there; Enter keeps the position the search reached.

**Promote**:
Turn the current search into a numbered include filter with `p`. The pattern
crosses from search to filter; the search itself is cleared.

**History**:
The patterns Enter committed in a `/` prompt, newest first, that Up and Down
recall in the next one. The file search, the filename search and the set
picker's search each keep their own; a filter prompt keeps none. The command
palette keeps one too, of the command lines it ran.
_Avoid_: recent searches, last patterns

### Commands

**Command**:
A thing recon can do, with a name the user types in the command palette.
Some commands take arguments. A key in the keymap runs a command that takes
none, and the keymap uses the same name for it.
_Avoid_: action (that is the code's word), function

**Command palette**:
The line that `:` opens at the bottom of the recon window from any pane (never
from a screen that covers the window, or from a prompt), where the user
types a command by name, with its arguments. Above the line, a list shows the
commands that agree with what the user typed, each with its key. Enter runs
the command; Esc closes the palette and changes nothing.
_Avoid_: palette alone (that is the filter colors), command line (that is the
shell's), ex mode

### Panes

**Pane**:
One of the three columns of the recon window, from left to right: the
**explorer**, the **file view** and the **filter pane**.
_Avoid_: panel, window (that is a slice of the visible lines), navigator (the old name of the explorer)

**Shown / Hidden** (of a pane):
Whether a pane is on the screen. Any pane can be hidden, but at least one is
always shown. Always say which pane: "hide the explorer", "show the filter
pane", never only "hide". A hidden pane keeps its state, and its global keys
still work.
_Avoid_: open, closed, collapsed

**Zoom**:
Hide every pane except the focused pane, and remember which panes were shown.
A zoom when only one pane is shown does the opposite: it shows the remembered
panes again, or all panes if none are remembered.
_Avoid_: maximise

**Folder** (of the explorer):
The directory whose entries the explorer lists. The user changes it by
stepping into or out of a directory, or with `:e` (or its alias `:cd`); it is never the working
directory of recon, which does not change.
_Avoid_: root, cwd, current directory (that is the working directory)

### The view

**Visible lines**:
The lines the file view can show, in file order: every line an exclude filter
does not remove, and in hide mode only the lines an include or context filter
matches. Dimmed lines are visible.
_Avoid_: on screen, shown lines

**Window**:
The slice of the visible lines that fits on the screen right now.
_Avoid_: viewport, visible (when the window is meant)

**Minimap**:
The narrow strip at the right of a long file's text that shows the whole file
in the height of the pane: where the search hits are, how dense the
interesting lines are, and the *band* — the rows of the map that the window
covers. It groups the lines of the file, not the visible lines. `M` hides it.
_Avoid_: scroll bar, overview ruler

**Dim mode / Hide mode**:
The two ways to treat a line that no filter matches, when an include or
context filter is enabled: keep it and grey it, or remove it from the visible
lines. `u` toggles between them.

**Follow mode**:
A property of one file, for the session: while the file is in the raw view,
the lines added to its end come into the view at once, filtered as the other
lines are. `Ctrl-t` turns it on or off. While the cursor is on the last
visible line, the view scrolls with the new lines (*autoscroll*); a cursor
above it pauses autoscroll, and `G` resumes it.
_Avoid_: tail mode, watch, live mode

### Views and hand-offs

**View** (of a file):
One way the file view shows a file: *raw*, *hex*, *command* (the output of
an external program) or *image*. The lines of the view are the lines the user
sees, and filters and search run on them, not on the bytes on disk. A view
with no lines (image) cannot be filtered or searched.
_Avoid_: mode (that is dim or hide), format, renderer

**Command view**:
A view made by an external program the user names in the config. It reads
the file and writes text or an image. The program is the user's, not part of
recon.
_Avoid_: plugin, preprocessor, filter (that is a pattern in the filter pane)

**View cycle**:
The ordered views a file type offers. The first one is the default, `-`
steps to the next, and a view that is not in the cycle is never offered. A
step with `-` holds for every file of that type until the user steps again
or recon stops.
_Avoid_: view list, view order

**Highlighting**:
Syntax color on the lines of a text view. It is a layer on a view, never a
view of its own, and it does not change the lines.
_Avoid_: syntax view, pretty view

**Hand-off**:
Sending the file to an app outside recon: the app its file type names, else
the editor for a text file, else the operating system's default app. `o`
does it. recon does not wait, and stays open. It is never a step in a view
cycle.
_Avoid_: launch, export, open in editor (that is `O`, the project)

<!-- ANCHOR_END: glossary -->

### Builds and releases

**Release version**:
The `X.Y.Z` number of recon. Only a release changes it.
_Avoid_: version on its own (a *version* is a state of a pattern in the
filter editor)

**Build stamp**:
The facts that identify one build: its release version, commit, dirty flag,
branch, checkout path and build time.
_Avoid_: build info, build ID, build version

**Release**:
A release version that has a tag `vX.Y.Z` and a GitHub release with notes.
_Avoid_: milestone (that is a GitHub issue grouping)
