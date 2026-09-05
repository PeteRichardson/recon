# Keymap reconciliation

One layered key model across the three panes, `n` that crosses file boundaries, and the
review loop driven from the file view without ever changing focus.

Tracks #120. Resolves every **decision** that issue left open, and replaces the three
"additional requirements" in its thread with the two that survived the ergonomics pass.
Shapes #59 (named actions) and #61 (keymap overrides); reserves keys for #67 (copy).

## The problem

Recon's central workflow is a double loop:

```
enter hide mode
for each interesting file in the navigator
    for each interesting line in the file
        peek · read · unpeek
```

Today the outer loop runs in the navigator and the inner loop runs in the file view, so
every iteration of the outer loop costs two focus keys: `e n t` to reach the next file, then
`j`/`k`/`n` to read it. The keys themselves are fine. The focus switching is the friction,
and #120's list of silent keys is mostly its symptom: `n` in the filter pane, `g` in the
navigator, `i` in the file view are all "the key I wanted, in the pane I happened to be in".

Two fixes, in order of weight:

1. **Collapse the loop.** In the file view, `n` at the last interesting line of a file moves
   to the first interesting line of the next interesting file. The loop becomes `u`, then
   `n n n …`, with `space` to peek. Focus stays in the file view throughout. This is vim's
   quickfix model (`:cnext`) and the shape of `grep -n` output.
2. **Give every key one layer.** Global, chain, or pane. A pane key pressed in another pane
   says so instead of doing nothing.

## The model

Four layers, checked in this order. A key belongs to exactly one layer per pane, and where a
key is bound in more than one pane it means the same thing in each.

1. **Prompt** — an open prompt takes every key. Unchanged.
2. **Global** — one meaning everywhere, handled in `App::handle_event` before any pane sees
   the key. Quit, help, focus movement, peek, hide, the filter-set toggles, editor launch,
   search on/off, and the two new file-stepping keys.
3. **Chain** — a focus key followed by a pane key. `f i` is "add an include filter" from
   anywhere; from inside the filter pane the `f` is a no-op and `i` alone does it. Chains
   are documented as commands (`fi`, `fx`, `fc`, `fd`, `en`), not as a side effect.
4. **Pane** — two kinds:
   - **Shared motions**, bound in every pane where the concept exists, identical meaning:
     `j`/`k`, `g`/`G`, `Home`/`End`, `Ctrl-d`/`Ctrl-u`, `PageUp`/`PageDown`, `n`/`N`, `Enter`.
   - **Pane verbs** that only make sense in one place: `i`/`x`/`c`/`d`/`m`/`a`/`s`/`R`/`S`
     in the filter pane, `h`/`l` as ascend/descend in the navigator, `w`/`0`/`^`/`$`/`{`/`}`/
     `#`/`*` in the view.

Two rules across all of it. **Opposites pair**: a key with a direction has a partner
(`Tab`/`Shift-Tab`, `n`/`N`, `.`/`,`, `[`/`]`, `Ctrl-d`/`Ctrl-u`). **Vim where vim has an
opinion**, and the README records the reason wherever recon departs from it.

## The loop, after

| Step | Key | Hand | Layer |
|---|---|---|---|
| Enter hide mode | `u` | right | global |
| Next / previous interesting line, crossing files | `n` / `N` | right | shared motion |
| Walk this file's visible lines | `j` / `k` | right | shared motion |
| Skip to the next / previous interesting file | `.` / `,` | right | global |
| Peek at the plain file, and back | `space` | thumb | global |
| Page the file view, from any pane | `]` / `[` | right | global |
| Read while peeked | `j k { } g G Ctrl-d Ctrl-u` | right | view |
| Keep what you found | `p`, `*`, `f i …` | | global, view, chain |

Everything is unmodified and on the right hand except `space`, which is the thumb. That is
deliberate: peek is the key interleaved most often with `n`, `j` and `k`, and the thumb is the
only key that alternates hands against a right-hand vim vocabulary without leaving the home
position.

## Changes

Each item names the #120 section it resolves and the decision taken.

### 1. `n`/`N` cross file boundaries in the file view (§2, new)

In the file view, `n` looks for the next interesting line **after the cursor** in the current
file. If there is one, it goes there, as today. If there is not, it asks the navigator for the
next interesting file, loads it, and puts the cursor on that file's first interesting line.
`N` mirrors: previous interesting line, else the previous interesting file's last interesting
line.

- **Interesting file** means `Match::Yes` in the navigator's #119 marking: at least one line
  selected by an enabled include filter or the live search and not excluded. The navigator's
  filename search is *not* consulted here, even when one is active. The loop is about
  content, and a filename hit with no interesting lines has nowhere to land. `Match::Unknown`
  entries (not yet scanned, or scanning off) are skipped, so with #119's scan disabled `n`
  stays in-file exactly as today.
- **Wrapping** is the navigator's: `FileNav::step_to` already walks the visible rows from
  the selection and wraps. If the current file is the only interesting one, `n` wraps within
  it as today. If the listing has no interesting file at all, `n` wraps within the current
  file as today. Nothing ever goes silent: the status row reports when a step crossed a file
  and which file it landed in.
- **Crossing a file is visible.** Log files look alike, so a crossing needs more than a
  status-row line. Three cues, all cleared by the next keypress rather than by a timer (the
  event loop redraws only on events, #85, and a timed fade would need a tick source that
  does not exist): a one-line centred notice over the file view (`▼ next file · foo.log`,
  `▲ previous file · bar.log`), the view's border title in the accent colour for that
  keypress, and the status-row line. The border highlight is what registers when the same
  `n` that triggered the notice is also the key that dismisses it.
- **The navigator's selection follows.** Crossing a file is `FileNav::step_to` with the
  `Match::Yes` predicate, which selects the entry and returns `Action::Preview`; `App`
  performs it, promotes the truncated preview as `n` already does, and then steps. So the
  navigator always shows which file the view is in, and `e` lands where you expect.
- **First interesting line from the top** is a new `Viewport` query: `next_interesting`
  considers the cursor's own line last, which is right for stepping and wrong for landing.
  Landing wants "the first interesting line at or after row 0" (or at or before the last row,
  for `N`).
- **In the navigator**, `n`/`N` are unchanged: filename-search hit if a search is active,
  else next matching file (`FileNav::repeat_search`). This is the "skip the rest of this
  file" gesture from inside the navigator, and `e n` from anywhere.
- **In the filter pane** — decision **(b)**: `n`/`N` act on the file view, exactly as if
  pressed there, including crossing files. The pane has no "next" of its own and the user
  wants to see the effect of the filter they just touched.

### 2. `.` and `,` step files from any pane (new, replaces the thread's `[`/`]` request)

`.` = next interesting file, `,` = previous. Global. The same operation as the cross-file
step in §1, without first exhausting the current file: select the next `Match::Yes` entry,
load it, land on its first (for `,`: last) interesting line. From the navigator with no
filename search active this is the same as `n`. The keycaps carry the mnemonic: `<` and `>`.

Both keys are unbound in every pane today. Vim's `.` (repeat) and `,` (repeat-motion
backward) have no counterpart in recon.

### 3. `[` and `]` page the file view from any pane (thread requirement, kept)

Today `[`/`]` are `PageUp`/`PageDown` in the view only. They become global, so the file view
pages while the navigator or filter pane has focus. `Ctrl-b`/`Ctrl-f`/`PageUp`/`PageDown`
stay view-only aliases.

### 4. `space` stays the peek; toggling a filter set is `Enter` (thread requirement, reversed)

The thread proposed moving peek to `a` so `space` could toggle filter sets. Reversed, for
the reason in the loop table: peek is the loop key and toggle is a setup key, and the thumb
belongs to the loop. It is also the Finder idiom: Quick Look is "peek at the thing under the
cursor" on `space`. `Enter` already toggles the selected filter or set; from anywhere that is
`f Enter`. Mouse clicks (#58) close the remaining gap.

Consequences: `p` stays promote, `a` stays the profile picker, `* p` (§11) stays two keys.

**Peek and movement** — decision: `n`, `N`, `.` and `,` while peeked first restore the peek
(as `space` would), then move. A jump that leaves the peeked file has nothing to come back
to, and the alternative (the step running over a file with every filter disabled) would find
no interesting line and cross files immediately, which is never what was meant. In-file
motions (`j`, `k`, `g`, `{`, `[`) do not touch the peek.

### 5. Hide mode toggles on `u` (§10, decided)

`u` toggles between dimming and hiding **u**nmatched lines. `Ctrl-H` and `H` stay as
aliases; retiring them is a follow-up. `u` is free in every pane and vim's `u` (undo) has no
counterpart here.

### 6. `Shift-Tab` focuses the previous pane (§1)

`focus_next` gets a `focus_prev`. crossterm reports the key as `KeyCode::BackTab`.

### 7. Shared list motions in the navigator and the filter pane (§3)

`g`/`Home`, `G`/`End`, `Ctrl-d`/`Ctrl-u`, `PageUp`/`PageDown` in both list panes, same
meaning as the view. The filter pane already guards `Ctrl-d` from reading as `d`
(`FilterList::handle_key`), so the half-page binding slots in beside it.

### 8. Chains that commit a prompt return focus, then step (§4, decision (a))

After `f i … Enter`, `f x … Enter` or `f c … Enter` from another pane, focus returns to the
pane that had it when `f` was pressed, and then `n` runs there. `f i fn Enter` from the file
view lands on the first `fn`, crossing files if the current one has none. `f d`, `f Enter`,
`f a`, `f s`, `f m` and plain `f` leave focus in the filter pane: a toggle or delete is often
one of several. `p` keeps focus where it is, as now.

`App` records the origin pane when `f` moves focus and clears it on any other focus change,
so `f` then `Tab` then `i … Enter` does not return anywhere.

### 9. A pane verb in the wrong pane says so (§5)

`i`, `x`, `c`, `d`, `m`, `a`, `s` outside the filter pane put a one-line hint on the status
row for one keypress, in the shape `i adds a filter · f i`. `h`/`l` in the filter pane
likewise (`h` goes up a directory · `e h`). Not auto-redirect: making `i` global would
collapse `f i` and `i`, and `x`-not-`e` for exclude exists because `e` is a focus key.

### 10. `Enter` stays unbound in the file view (§6, decision (a))

Navigator: open. Filter pane: toggle. View: nothing. `Enter` as "open in the editor" would
launch too many editors by accident.

### 11. `/` in the filter pane acts on the file view (§7, decision (b))

`/` from the filter pane opens the live-search prompt as if pressed in the view: "new
search", where `c` on the search row is "edit search". Consistent with §1's rule that the
filter pane forwards view-shaped keys to the view.

### 12. `Esc` clears the focused pane's search first (§8)

With the navigator focused and a filename search active, `Esc` clears it. Otherwise `Esc`
clears the live search as now. One key, layered like the prompt-cancel case already is.

### 13. `*` searches for the word under the cursor (§11)

View key. Sets the live search to the word under the cursor, escaped with `regex::escape`,
where a word is a maximal run of `[A-Za-z0-9_]` (keeps a mangled `_ZN4core…E` whole, stops
at `::`, `.`, `(`). Then steps as `/` does. `* p` promotes it to a numbered filter. No
backward twin: `#` is the gutter and `N` covers the direction. On whitespace or punctuation
the status row says there is no word under the cursor.

### 14. `1`–`9` toggle a filter by position (new)

Global. `3` toggles the third toggleable row of the filter pane in display order: search
row and set headers excluded, user-authored and built-in filters included. The pane prints
the digit in its gutter beside each of the first nine such rows, so the mapping is visible;
rows beyond nine have no key. The numbering shifts when a set is soloed, reset or reordered,
which is exactly why the gutter shows it rather than the user counting. Implementation is
`Filters::toggle_enabled(index)`, which exists, behind a global arm; the row-to-index walk
is the one `FilterList::rows` already does.

### 15. Documentation follows the model (§9)

`KEYMAP` in `src/help.rs` and the README's *Keybindings* section regroup by layer: Global ·
Chains · Shared motions · Navigator · File view · Filter pane. A shared motion appears once,
with a note on any pane it is absent from. `every_bound_key_is_documented` keeps the overlay
and the code agreeing. The README's "three places a key can be bound" paragraph stays; it
describes the code, and that isn't changing.

The README's reasoning paragraphs gain three entries beside the existing `space` and
`h`/`l` ones: why `n` crosses files, why `space` peeks rather than toggles, and why `u`.

## What does not change

- `h`/`l` differ by pane on purpose (file-manager motion in the navigator, character motion
  in the view). `v` and `y` stay unbound, reserved for #67's visual mode.
- `b`/`e` are window commands, not word motions.
- `p`, `a`, `s`, `m`, `R`, `S`, `!`, `&`, `b`, `z`, `o`, `O`, `r`, `q`, `?`, `e`, `t`, `f`.

## Keys after this change

Unbound in every pane: `-`, `=`, `;`, `'`, `` ` ``, `\`. Every lowercase letter and every
digit is bound or reserved.

## Implementation notes

- **Cross-file step** lives in `App`, not `Viewport` or `FileNav`: it needs the navigator's
  marking, the view's verdicts and `perform`. Shape: `step_interesting(backwards)` tries
  `viewport.next_interesting_strict(backwards)` (a variant that does *not* wrap), and on
  `None` calls `nav.step_to(backwards, is_match_yes)`, performs the returned action,
  promotes the truncated preview, and lands with `viewport.first_interesting(from_end)`.
  `.`/`,` call the second half directly. The existing `n` arm at the top of
  `App::handle_event` becomes a call to this.
- **Landing on the last interesting line** for `N` and `,` needs the whole file, not the
  preview: `promote_truncated_preview` runs before the landing step, as it does for `n`
  today.
- **Status-row hints** (§9) and the crossed-file report (§1) share one transient slot:
  a message that the next keypress clears. `changed on disk · r` is the existing example of
  a status-row message; check whether it already has the one-keypress lifetime before adding
  a second mechanism.
- **Return-focus** (§8) is a field on `App`: `chain_origin: Option<Focus>`, set in the `f`
  arm when focus actually moves, cleared in `reveal_and_focus` and `focus_next`/`focus_prev`,
  consumed by the prompt-commit path of `handle_filter_key`.
- **`*`** reads the cursor's source line from `Viewport`, finds the word around the cursor
  column, and goes through `apply_search`, so it inherits the "set it, then do what `n`
  does" contract.
- **Filter pane forwarding** (§1 filter-pane clause, §11): `n`/`N`/`/` in the filter pane
  are handled by the same global arms as in the view. The `/` guard `self.focus !=
  Focus::Filters` goes; `handle_filter_key`'s comment about the search prompt is updated.

## Testing

Each change lands with tests in the style of the existing ones (`key(&mut app, …)` against
a fixture directory). The ones that matter most:

- `n` at the last interesting line of a file lands on the first interesting line of the next
  `Match::Yes` file, and the navigator's selection moved with it.
- `N` mirrors, landing on the last interesting line, including when that file was only
  previewed (truncation promoted first).
- `n` with a filename search active in the navigator still steps by content, not by name.
- `n` with no other interesting file wraps within the file, as before.
- `n` while peeked restores the peek before moving; `j` while peeked does not.
- `.` and `,` from each of the three panes.
- Crossing a file shows the notice and the highlighted title; the next key clears both.
- `[`/`]` from the navigator and filter pane page the view.
- `3` toggles the third toggleable row; `9` with eight filters does nothing and says so; the
  gutter digits match what the keys do after a solo.
- `u` toggles hiding; `H` and `Ctrl-H` still do.
- `Shift-Tab` reverses `Tab` through all three panes.
- `g`/`G`/`Ctrl-d`/`Ctrl-u` in the navigator and filter pane.
- `f i fn Enter` from the view returns focus and lands on the first `fn`; `f d` does not
  return; `f Tab i … Enter` does not return.
- `i` in the view produces the hint and no filter.
- `/` from the filter pane sets a live search.
- `Esc` with a navigator search clears it; a second `Esc` clears the live search.
- `*` on `foo::bar(` at the cursor on `bar` sets the search to `bar`; on `_ZN4core3fmt` sets
  the whole run; on whitespace sets nothing and says so; on a name containing `.` the search
  pattern is escaped.
- `every_bound_key_is_documented` passes with the regrouped `KEYMAP`.
- A pasted newline in the search prompt is dropped (guards the `*`/paste interplay if
  bracketed paste ever lands).

## Landing

Independent PRs, in this order so each is small and the loop improves first:

1. Cross-file `n`/`N`, `.`/`,`, global `[`/`]`, peek-then-move. (§1–4)
2. `u`, `Shift-Tab`, shared list motions, `Esc` layering, `/` from the filter pane, digit
   toggles. (§5–7, §11, §12, §14)
3. Chain return-focus and wrong-pane hints. (§8, §9)
4. `*`. (§13)
5. `KEYMAP` and README regroup, plus the reasoning paragraphs. (§15) Every earlier PR adds
   its rows to the existing grouping; this one rearranges.

## Out of scope

- #59 names these actions once the tables in §14 exist. #61 overrides them after that.
- #67 owns visual mode, `v`, `y` and double-click selection.
- #58 owns the mouse; nothing here changes what it does.
- Retiring `H`/`Ctrl-H` once `u` has settled.
- Match counts in the navigator (#6).
- Emitting the directory, visible lines, visible files or the filter set on quit (#143).
