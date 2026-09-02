# Navigator filter matches

Which files in the listed directory would show at least one line under the active
filters — answered in the navigator, without opening each file.

Tracks #119. #6 — the original ask, for match *counts* — stays open for the count; this
ships the boolean and the hide half first.

## The problem

A folder of logs and a filter set that describes a bug. The question is "which of these
logs has it?" Today the answer costs one keypress per file: enable the filters, enter
hide mode, step through the folder, and look for a non-empty pane. A terminal `grep -l`
answers it in one line, but the patterns are already *in recon* — they are the filter
set — and leaving the tool to re-type them is the workflow this feature removes.

## What it does

With at least one selecting filter enabled, the navigator marks each file:

| State | Dimmed mode | FilteredOnly mode |
|---|---|---|
| matches | **the colour of the filter that matched**, as the view draws the line | same |
| unknown — not yet scanned, or the feature is off | plain | plain, shown |
| does not match | dimmed | **removed from the listing** |

A file **matches** when at least one of its lines is selected by an enabled `Include` filter
or by the live search, and not removed by an enabled `Exclude` filter. That is the view's
own rule for showing a line, minus one sense — see *Three senses* — so the two panes cannot
disagree about which filter picked a file.

The name is drawn in the same style the view would give the line: the search's white bold if
a search line hit, otherwise the colour of the lowest-numbered `Include` filter with a hit
in the file — the view's "first matching filter wins", applied per file. In a multi-filter
set that answers *which* signature a log carries without opening it.

The scan runs in the background, stops reading a file at its first matching line, and
streams answers into the navigator as each file finishes. Toggling a filter on or off
usually re-answers the whole folder with no I/O at all (see *The cache*). Hide mode is one
mode for both panes: `Ctrl-H`/`H` dims or hides non-matching lines in the view and
non-matching files in the navigator. `n`/`N` in the navigator step between matching
files. `r` refreshes everything from disk.

Directories are never scanned, dimmed, hidden or marked. No recursion.

## Three senses

A realistic filter set for a folder of logs has two kinds of numbered filter in it: patterns
that *discriminate* — part of a bug's signature, present in some logs and not others — and
patterns that pick out *metadata* every log carries: the commit the build came from, the
host it ran on. The second kind is wanted in the view and useless for choosing files; with
one enabled, every file matches and the navigator says nothing.

So `Sense` gains a third value:

| Sense | In the view | Selects the file? | Pane glyph |
|---|---|---|---|
| `Include` | shows the line, in its colour | **yes** | `inc` |
| `Context` | shows the line, in its colour | no | `ctx` |
| `Exclude` | removes the line | no | `exc` |

A variant rather than a flag on `Include`: an `Exclude` filter already never selects a file,
so "selects?" is not orthogonal to sense but one more value of it — and the compiler then
finds every `match` that needs to know.

`verdict` treats `Context` exactly as `Include` for a line: same `Verdict::Included(index)`,
same colour, and it still triggers dimming of the unmatched rest. The only thing a `Context`
filter does not do is pick files. New filters are `Include`; a filter is opted *out* of
selecting with `m` in the filter pane (see *Keys*), which toggles between the two.

**Sense is the user's choice, per filter, in this set — never inferred.** It is not a
property of the pattern: `^host: production-.*` is `Include` when the question is "which
production logs have errors" and `Context` when it is "which logs have bug 57, and where
did they run". Only the person asking knows which. So nothing here derives sense from the
pattern's text or from what it matches, and the scan never adjusts it from results. (That is
the deeper reason the "a filter that hits every file isn't discriminating" alternative was
rejected: it answers the wrong question, not merely a fragile one.)

This is the attribute a saved filter set (#8) has to carry — a "Bug 57" set is one
discriminating pattern and four context ones — and it makes a saved set a set of *roles*,
not of patterns: the same pattern legitimately appears in two sets with different senses.
If #8 takes the "activation profiles over one list" route its comments describe, a profile
that only toggles `enabled` will not be enough; it has to carry sense too.

## Components

Four pieces, three of them additive.

### `src/scan.rs` — the scanner (new)

Two layers, kept apart so the interesting behaviour is testable without a thread.

**The core** is a pure function:

```rust
pub fn scan(
    reader: impl BufRead,
    matcher: &Matcher,
    from: Progress,
    cancel: &AtomicBool,
) -> Progress
```

It reads lines from `from.scanned_to`, ORs each line's `matcher.bits(line)` into
`from.seen`, and stops at EOF, at the first line `matcher.shows(bits)` accepts, or when
`cancel` is set — checked per line, an atomic load rather than a syscall. Lines are read as
bytes and matched through `String::from_utf8_lossy`, a `Cow` that allocates only on an
invalid line, so one bad byte does not blank a file's answer (the tolerance `read_lines`
got in 7d6e587).

**The driver**, `Scanner`, owns at most one worker thread. `start(Request)` sets the previous
worker's cancel flag, then spawns a new thread that walks the request's file list — open,
`seek(progress.scanned_to)`, `BufReader`, core, send — over an `mpsc` channel. On cancel the
core returns whatever partial `Progress` it has, and the driver **still sends it** before
exiting: a toggle mid-way through a 2 GB file keeps the bitsets from the part already read.
The driver holds no cache and no state between requests.

### `src/filter.rs` — `Matcher` (new type)

`ActiveFilters` is neither `Send` nor `Clone`. `Matcher` is the snapshot a thread holds
instead:

```rust
pub struct Matcher {
    set: RegexSet,     // ActiveFilters::compiled, cloned (Arc inside)
    selects: u64,      // bit i set: filters[i] is enabled and Sense::Include; plus the search
    exclude: u64,      // bit i set: filters[i] is enabled and Sense::Exclude
}

impl Matcher {
    /// Which patterns hit this line, enabled or not.
    pub fn bits(&self, line: &str) -> u64 { /* RegexSet::matches → bitset */ }
    /// Whether a line with these hits selects its file.
    pub fn selects(&self, bits: u64) -> bool {
        bits & self.selects != 0 && bits & self.exclude == 0
    }
    /// The lowest selecting bit in `bits`, for the file's colour. Search first.
    pub fn owner(&self, bits: u64) -> Option<Owner> { /* Owner::Search | Owner::Filter(i) */ }
}
```

The search, when present and enabled, sits in `selects` at position `filters.len()` — the
same position `compiled` gives it. `Context` filters are in neither mask: they neither select
nor exclude, so a line that only a context filter hit contributes nothing to the answer.

`ActiveFilters::matcher() -> Option<Matcher>` returns `None` when no `Include` filter and no
search is enabled (nothing to select with — the same guard `Document` applies for #36) or
when the pattern count exceeds 64. `ActiveFilters::pattern_key() -> Vec<String>` returns the ordered
pattern sources, search last, and is what the cache is keyed on.

`compiled` already covers every pattern whether or not it is enabled — a deliberate choice
in #86 so that toggling needs no recompile. That is what makes a line's bitset independent
of the enabled mask, and therefore reusable across toggles.

### `src/widgets/filenav.rs` — display only

`Entry` gains `matched: Match`, where `Match::{Unknown, No, Yes(Style)}` — `Unknown` is "not
yet known", and `Yes` carries the ready style so the navigator need not know what a filter
is. `FileNav` gains a `Mode` (the
same enum `Document` uses) and `visible: Vec<usize>` over `entries` — the model `Document`
already has for lines. `rebuild_visible()` derives `visible` from `mode` and `matched`;
`rebuild_list()` styles from `matched` and builds the `List` from `visible`. `visible[0]` is
always `..`.

`FileNav` never sees a filter, a file's contents, a stamp, or a thread. It is told answers
through `set_answer(index, Match)` and `set_mode(Mode)`.

### `src/lib.rs` — `App` orchestrates

Owns the cache, decides when a scan is needed, drains results, computes each file's style
with `style_for` and its `Matcher::owner`, pushes answers into the navigator, and reloads
the active file on `r`.

### `src/widgets/filterlist.rs` — one key, one glyph

`m` toggles the selected filter between `Include` and `Context`, reported to `App` as a new
`FilterCommand::ToggleContext(index)`, the same route `Toggle`/`Delete`/`Edit` take. The row
glyph shows the sense: `inc`, `ctx`, `exc`.

Not touched: `Document`, `FileView`'s reading paths, the vendored fork.

## Data model

```rust
// scan.rs
pub struct Progress {
    pub seen: Vec<u64>,   // distinct per-line bitsets; deduplicated; small
    pub scanned_to: u64,  // byte offset, always at a line boundary
    pub eof: bool,
}

pub struct Record {
    pub stamp: (SystemTime, u64),   // (mtime, len) at the moment the scan opened the file
    pub progress: Progress,
}

impl Record {
    /// `Some(answer)` if knowable from what has been read; `None` means resume.
    pub fn answer(&self, m: &Matcher) -> Option<bool> {
        if self.progress.seen.iter().any(|&b| m.selects(b)) { return Some(true); }
        if self.progress.eof { return Some(false); }
        None
    }
}

// lib.rs
struct ScanCache {
    id: u64,                          // bumped whenever `key` changes
    key: Vec<String>,                 // ActiveFilters::pattern_key() it was built for
    records: HashMap<PathBuf, Record>,
}

pub struct Request { cache_id: u64, matcher: Matcher, files: Vec<(usize, PathBuf, Progress)> }
pub struct Scanned { cache_id: u64, index: usize, path: PathBuf, stamp: (SystemTime, u64), progress: Progress }
```

`seen` is a `Vec`, not a `HashSet`: it holds distinct match *combinations*, which in a real
log is single digits, and a linear `contains` on insert beats hashing at that size.

`u64` caps the design at 63 numbered patterns plus the search. Above that `matcher()` is
`None` and the feature is off — never a wrong answer.

## The cache

The point of recording bitsets rather than answers: a file matches under a mask iff

```
∃ b ∈ seen :  (b & include) ≠ 0  ∧  (b & exclude) = 0
```

(with `include` meaning the `selects` mask) so any enable/disable toggle — and any
`Include`↔`Context` toggle, which only moves a bit between masks — is re-answered with a
few `u64` ops per file. The tension is
with early exit — `seen` is only complete if the file was read to EOF — and `Record::answer`
resolves it with three outcomes:

| `seen` has a shown bitset | `eof` | Answer |
|---|---|---|
| yes | — | `Some(true)`, no I/O |
| no | true | `Some(false)`, no I/O |
| no | false | `None` — resume from `scanned_to` |

Nothing is ever read twice. A toggle costs only the unread remainder of files whose answer
cannot be known from what was already read.

**What invalidates:**

- The pattern list changing — `add`, `remove`, `set_pattern`, `promote_search`, and
  committing or clearing a search. Positions shift, so the bitsets mean something else. The
  whole cache is dropped and `id` bumped.
- A file's `(mtime, len)` differing from its record's stamp. That record alone is dropped.
- Listing a different directory. Different files; the cache is dropped.

**Accepted limitation:** clearing a search (`Esc`) invalidates the numbered filters' records
too, because the search occupies the last position. Keying the two halves separately would
fix it; not worth it in the first cut.

## Data flow

### `App::refresh_scan(force: bool)`

Runs at the end of every `handle_event`. It is cheap-idempotent: unless `force`, it
compares `(pattern_key, selects, exclude, nav.dir())` against the tuple from its last run
and returns at once if nothing changed. That guard is what keeps `j`/`k` in the file view
from stat-ing 200 files.

When it proceeds:

1. `filters.matcher()`. `None` ⇒ feature off: every `Entry.matched` becomes `Unknown`, the
   worker is cancelled, `nav.rebuild_visible()`, return.
2. If `cache.key ≠ pattern_key` ⇒ fresh cache, `id += 1`.
3. For each file entry: stat → stamp. Drop its record on mismatch. `record.answer(&matcher)`:
   `Some(true)` ⇒ `Match::Yes(style)`, the style from `matcher.owner` over the selecting
   bitsets and `filters.style_for`; `Some(false)` ⇒ `Match::No`; `None` ⇒ `Match::Unknown`
   and push `(i, path, progress-or-default)` onto `pending`.
4. `pending` non-empty ⇒ `scanner.start(Request { .. })`. Empty ⇒ cancel the worker. A
   toggle whose every answer is cached issues no request and touches no thread.
5. `nav.rebuild_visible()`.

### `App::drain_scan_results() -> bool`

In `App::handle_events`, beside `drain_editor_outcomes`, ORed into the same "did anything
change" that the conditional redraw (#85) branches on. Per result: drop it if `cache_id ≠
cache.id`; otherwise store the record if its `scanned_to` is beyond what is held (or nothing
is held), recompute the `Match` from `record.answer(&current_matcher)`, push it to the navigator,
and report whether it changed. A result from a *cancelled* worker is accepted on the same
terms — the bitsets are valid for the pattern list regardless of which mask asked for them.

`Disconnected` on the receiver can only mean the `Scanner` itself was dropped: it holds
the `Sender` for the worker's whole life, so a panicked worker merely stops sending — the
receiver sees `Empty`, not `Disconnected` — and affected entries stay `Unknown` until the
next trigger, which is the designed recovery.

### `App::poll_stamps() -> bool`

Also in `handle_events`, rate-limited to once per 2 s, and active only while `matcher()` is
`Some`. Re-stats the listed files — a few hundred `stat` calls every two seconds — and for
any mismatch drops that record, marks the entry `Unknown`, and issues a request for it. If the
**active** file's stamp moved, sets `view_stale = true`, which the status row shows as a
badge (see *Refresh from disk*). Returns whether anything changed.

This is the tick the render loop already wakes on 60 times a second. No thread, no
file-watching API, no platform code, and it covers every listed file rather than only the
active one.

### Refresh from disk — `r`

`r` is global and does two things:

1. Re-list the directory before anything else, so a file created since the listing was
   built appears; then `refresh_scan(force: true)` — bypass the guard, re-stat everything,
   rescan what moved. Partial progress from a cancelled in-flight worker is kept (same cache
   id).
2. Reload the active file **with the cursor restored**: remember `cursor_source()`, `load`,
   `sync_document`, then `place_cursor_on_visible_row(document.nearest_visible(source))` and
   `refresh_view`. That is the machinery a filter change already uses to rebuild the buffer
   without losing the reader's place; a truncated file (logrotate) simply clamps. Clears
   `view_stale`.

**The inconsistency this is for, stated plainly.** When a file changes underneath a running
recon, the navigator's answer updates on the next poll, but the file view keeps the content
it loaded. The two panes can then disagree for as long as the user leaves them. The badge
says so the moment it becomes possible — `changed on disk · r`, beside the existing `HIDE`
badge on the status row — and one key resolves it. Automatic reload is the next work item,
not this one; with cursor restore built and tested here, it reduces to "do on the tick what
`r` does".

### Peek

`space` drops every filter ⇒ `matcher()` is `None` ⇒ the navigator un-dims. Restoring hits
the cache — same key, same masks — so the highlights return with **no I/O and no thread**.

## Navigator rendering, hide mode, selection

Progress is visible: rows flip from plain to coloured or dim as answers arrive. Unknown is
never hidden — you do not hide what you have not read. A folder of 200 logs reads as mostly
dim rows and a few coloured names: the same picture the view gives, no busier. The
filename-search highlight (`MATCH_STYLE`, yellow) composes rather than competes: a search hit
stays yellow whatever its filter state, because the user just asked for it by name.

`App` pushes `document.mode()` into `nav.set_mode()` wherever it toggles today, so the two
panes are always in the same mode.

`state.selected()` indexes `visible`. `selected_path`, `index_of`, `activate_selection`, and
`go_to_parent`'s "select the directory just left" all map through `visible`. When a rebuild
removes the selected row, the selection clamps to the nearest remaining visible row, as
`FilterList::clamp_selection` does — never to `None` while `..` exists.

## Keys

| Key | Where | Meaning |
|---|---|---|
| `r` | global | Refresh from disk: re-stat and rescan the listing; reload the active file with the cursor restored. |
| `n` / `N` | navigator | Next/previous filename-search match if a search is active; otherwise next/previous file with `Match::Yes`, wrapping, skipping directories. |
| `m` | filter pane | Toggle the selected filter between `Include` and `Context` — shown in the view either way, selects files only as `Include`. |
| `Ctrl-H` / `H` | global | Unchanged key; the mode now applies to both panes. |

`KEYMAP` in `src/help.rs` gains the `r` and `m` rows and the navigator `n`/`N` description
changes; the README's tables follow by hand. `every_bound_key_is_documented` enforces both
new rows. `r` and `m` are currently unbound in every pane.

## Error handling

- **Unreadable file** (open or stat fails) ⇒ `Match::No`, `log::warn!`. Dimmed means
  "this will show you nothing", which is true. Retried only when its stamp changes or on
  `r`, not on every trigger.
- **Bad bytes mid-file** ⇒ `from_utf8_lossy` per line; the answer survives.
- **Worker panics** ⇒ the worker simply stops sending; the receiver sees `Empty`, not
  `Disconnected` (the `Scanner` still holds the `Sender`); affected entries stay `Unknown`
  until the next trigger restarts.
- **> 64 patterns** ⇒ `matcher()` is `None`; feature off; `log::debug!` once. Never a wrong
  answer.
- **File shrinks between stat and read** ⇒ seek lands past EOF; core returns `eof: true`
  with what it had; the stamp predates the read, so the next poll rescans.
- **Active file deleted** ⇒ `load` already renders `<No such file or directory>`; cursor
  restore clamps to row 0; the navigator marks it `Match::No`.
- **`r` mid-scan** ⇒ cancel, keep partial progress, restart with the re-stat.

## Testing

**`scan.rs` core**, over `Cursor`, no threads: stops at the first shown line; resumes from
`scanned_to` and reads nothing twice; dedups `seen`; sets `eof`; cancel returns valid
partial progress; an invalid-UTF-8 line does not abort.

**`Record::answer`** truth table: seen-hit ⇒ `Some(true)`; eof-no-hit ⇒ `Some(false)`;
partial-no-hit ⇒ `None`; exclude masking each way.

**The invariant that makes the whole thing correct:** for a grid of lines × filter sets
mixing all three senses, `matcher.selects(matcher.bits(line))` equals the spec's own
definition stated directly — an enabled `Include` filter or the search hits the line, and
no enabled `Exclude` does. Deliberately *not* derived from `verdict`'s index: that is a
colouring rule (first match wins), and a context filter can win the colour of a line an
include filter also hit. Selecting and colouring are different questions. The one relation
to `verdict` that does hold, and is asserted: a selected line is always a shown line.

And `matcher.owner(bits)` names the lowest *selecting* filter with a hit — which is not
always the filter `verdict` colours the line with. A line hit by context filter 1 and
include filter 2 is drawn in filter 1's colour (first wins) but the *file* is owned by
filter 2, the one that selected it. The test says so explicitly.

**`Sense::Context` in `filter.rs`:** a context-only set shows its lines and dims the rest,
exactly as an include-only set does; `matcher()` is `None` for it; `m` round-trips a filter
`Include` → `Context` → `Include` without changing its pattern, colour or position.

**`FileNav`:** style per state, including the style carried by `Match::Yes`; `FilteredOnly`
removes `Match::No` only, never `Unknown`; `..` and directories never hidden; selection
clamps when its row vanishes; `n`/`N` precedence.

**`App`**, with a recording `Scanner` double (the `RecordingLauncher` pattern): a toggle
whose answers are cached issues **no** request, and so does `m` on a filter whose bitsets are
already known; a pattern edit issues one with a fresh cache
id; a peek round trip issues none; a stale cache id is dropped; the drain returns `true` on
change; `r` bypasses the guard; the badge appears when the active file's stamp moves; reload
puts the cursor back on its source line.

**One integration test** in `tests/`: a real `Scanner` thread over a fixture directory,
results awaited with a bounded timeout.

## Out of scope

- **Automatic reload of the file view.** The next work item; see *Refresh from disk*.
- **Match counts** beside filenames (#6's original wording). The boolean ships first; a
  count needs every matching file read to EOF, which forfeits early exit. `Progress` leaves
  the seam — a count is one more field the core increments.
- **Recursion into subdirectories.**
- **Keying the cache separately for search and numbered filters.**
- **Persisting `Context` in a saved filter set.** That is #8's job, and this sense is the
  first attribute it will need beyond pattern and colour.
- **A consistent keymap across panes.** `n`/`N` now mean "next interesting row" in two
  panes with different notions of interesting, which is defensible but was noted as a
  source of surprise. Worth its own issue.
