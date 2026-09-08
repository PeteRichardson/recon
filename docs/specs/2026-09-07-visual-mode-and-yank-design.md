# Visual mode and yank: copying text from the file view

Resolves #67. Reserved by the keymap reconciliation (#120), which left `v` and `y`
unbound for this.

## Two use cases

1. **Symbol → filter.** A long mangled name in the code and the question "where else
   does this appear?" Select it and make it a filter: `v`, grow the selection, `y`, then
   `f i`, paste, `Enter`. Or double-click the word. (`*` already covers the common case
   of a word the `[A-Za-z0-9_]` rule can find; this is for anything it can't.)
2. **Consecutive log lines → clipboard.** Navigate to a section of a log and copy a run of
   lines: `V`, `j`/`k`, `y`. Or drag with the mouse.

## The model

**A selection is anchored to document lines, not screen rows.** `App` holds
`visual: Option<Visual>` where `Visual` is the anchor's *source* line and character
column plus whether the selection is line-wise. The cursor is the other end, as it
already is. Visibility is applied at yank time and at paint time, never stored, so:

- `u`, `!`, `space`, `1`–`9` and every other global key keep working mid-selection, and
  none of them can invalidate the selection — they only change what the yank will contain.
- **The yank is every line in `[anchor, cursor]` that is currently visible.** Hide mode
  skips the hidden lines; `u` mid-selection reveals them and the selection grows to
  include them. Dimmed lines are visible, so a yank in dimmed view includes them. One
  rule, no per-mode special case.

**Visual mode is a sub-mode of the view pane, below the global layer.** `v` changes what
`y`, `Esc` and the motions mean *in the view*, and nothing else. It sits below Global
because an open prompt takes every key and the globals are worth having while selecting.
Two consequences, decided here:

- **Leaving the view ends the selection.** `e`, `f`, `Tab`, `Shift-Tab`, a click on
  another pane: the selection is dropped, not parked. One rule enforced in one place
  (`handle_event`, after dispatch), rather than a selection that survives into a pane
  where nothing can act on it.
- **Loading another file ends it.** The anchor is a line of a document that no longer
  exists. `sync_document` clears it, so `.`/`,`, a navigator move and `r` all drop it.

## Keys

All in the file view. `v`/`V`/`y` pressed in the navigator or the filter pane get a
status-row hint (`v selects in the file view · t v`), like `*` does (#120 §9).

| Key | Outside visual mode | In visual mode |
|---|---|---|
| `v` | start a character-wise selection at the cursor | switch to character-wise; if already character-wise, end it |
| `V` | start a line-wise selection at the cursor line | switch to line-wise; if already line-wise, end it |
| `y` | hint: `v starts a selection` | yank, then end visual mode |
| `Esc` | (global: clear searches) | end visual mode. Takes the key before the global arm, so a stray `Esc` does not also drop the live search |
| motions | as now | as now — the cursor is the selection's moving end |

Character-wise is inclusive of the character under the cursor, as vim's is. Line-wise
copies whole lines with a trailing newline; character-wise has none.

The yank reports on the status row: `yanked 3 lines`, or `yanked 12 characters` for a
single-line character-wise selection. A clipboard failure reports in red, the way a
missing editor does.

## Mouse

Follows the rule in `mouse.rs`: a click is the pointing version of the keys, never a new
verb. In the view, on a file's text:

- **Click** puts the cursor there. It also ends any visual mode, as a click does in vim.
- **Drag** starts a character-wise selection at the press point and moves the cursor with
  the pointer. Releasing leaves visual mode active; `y` copies. The mouse selects, the key
  copies — the same split as `v` … `y`.
- **Double-click** selects the word under the pointer, by the `*` rule for a word.

Pointer column to character column goes through the gutter width, the horizontal scroll
and the display width of each character (tabs, wide glyphs). The gutter width is the one
number recon could not compute itself, so the vendored textarea gains a `gutter_width()`
accessor (PATCH.md §5).

## Painting the selection

`TextArea`'s own selection cannot be used: `set_lines` and `set_cursor_position` both
cancel it, and both run on ordinary movement here. The fork's `custom_highlight` is
used instead, re-applied on every render from the anchor mapped into the current window.
An anchor on a hidden line paints from the next visible line, so the painted range and
the yanked range agree.

## Clipboard

A **command template**, the same shape as the editor's: split once by
`editor::split_template` (no shell), run with the text on stdin. Defaults per platform:
`pbcopy` on macOS, `clip` on Windows, `wl-copy` when `WAYLAND_DISPLAY` is set and
otherwise `xclip -selection clipboard` elsewhere. Configurable as `--clipboard`,
`RECON_CLIPBOARD`, or `[clipboard] command` in `config.toml`, in that order.

Behind a trait (`Clipboard`) with a recording double, as `Launcher` is, so every test
asserts on what would have been copied rather than touching the real pasteboard.

Not an OSC 52 escape: Terminal.app does not honour it and several terminals ship it
disabled, so it fails silently exactly where a command fails loudly. A user on a terminal
that supports it can put a small script in the template.

## Status row

A ` VISUAL ` / ` V-LINE ` badge beside ` HIDE ` while the mode is on, in the same style:
the mode changes what the next keys do and is easy to forget mid-scroll.

## Out of scope

- `o` to swap the anchor and cursor in visual mode (`o` opens the editor).
- Block-wise selection.
- Auto-scrolling a drag past the pane's edge.
- `--emit` of a selection (#143 owns emission; the yank text function is written to be
  reusable there).
