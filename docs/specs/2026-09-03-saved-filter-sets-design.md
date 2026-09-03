# Saved filter sets

Named groups of filters, defined once in a file and switched on together — so
that "the filters that find bug 57" is a thing recon knows by name, not a set of
regexes typed again every session.

Tracks #8. Consolidates the decisions made in that issue's thread and adds the
ones that were left for "when the code is in front of us". Split for delivery
into #128–#132 (see *Delivery*), with #39 and #123 landing first and #127 after.

## The problem

A folder of logs and a bug to find. The filters that describe the bug are three
or four regexes, and recon already colours and hides by them well. What it
cannot do is remember them. Every session starts with `f i`, type, `Enter`,
repeat — and a colleague who wants the same triage has to be sent the patterns
in a message.

The workflow this enables: open the folder, enable the set called `WiFi_debug`,
pick the profile called `WiFi_bug_32`, and read the navigator (#119) to see
which logs have it.

## The model

Three nouns, one rule.

- A **filter** is what it is today: a pattern (or, after #123, a predicate), a
  sense, an enabled flag, and a colour. It gains a **name** and belongs to
  exactly one **set**.
- A **set** is a named, ordered group of filters. A set is enabled or disabled
  as a unit. **Enabling a set makes its filters available; it does not decide
  which of them are on.** The set's `[x]` means "these rows count", and each
  row's `[x]` means "this one is on".
- A **profile** is a named permutation of a set's filters. Applying a profile
  enables exactly those filters and disables the set's others. A set called
  `WiFi_debug` might carry profiles `WiFi_triage`, `WiFi_bug_32` and
  `WiFi_disconnect_failures`, each a different subset of the same filters.

**The rule: a filter takes effect when it is enabled *and* its set is enabled.**
recon shows the union of every effective filter across every set. Nothing else
changes — `verdict`, senses, the navigator's `Matcher`, and #39's AND mode all
run over the effective filters exactly as they run over the enabled ones today.

A filter belongs to one set. A pattern two sets both want is written twice and
is two filters, with two rows and two colours; editing or toggling one does not
touch the other. Shared filters referenced by name are a possible later
extension (`ref = "error"` beside `pattern`) and are not designed here.

### The scratch set

Every filter lives in a set. The filters a user types — `i`, `x`, and `p`
promoting the live search — land in a permanent, unnamed **scratch set**: always
present, always first, never written to the file, and with no header row of its
own. It cannot be toggled by hand; the one thing that suspends it is *solo*.

This is a statement about today, not a new feature: **recon today is this model
with exactly one set.** A user with no `filters.toml` sees precisely what they
see now. The scratch set exists so that nesting, toggling, `!`, solo and the
peek need no special case for "loose" filters.

`c` edits a file-loaded filter's pattern in place. In v1 that edit is
in-session only; nothing writes it back. `d` on a file-loaded filter removes it
for the session, likewise.

### Order and priority

Sets are listed by **priority**, an integer, lower first, ties broken by name.
The default is 50. The scratch set is always first regardless, because it is
where typing lands. The filter pane is short and its top rows are the ones a
hand reaches, so which set sits there is a setting, not an accident of naming.

Built-in sets (#127) have a built-in priority and land wherever that puts them
among the file sets. They are not listed last: a set recon ships is there
because it is useful, and it competes for the top rows on the same terms.

The **known list** is every filter in every set, flat, in this order: the
scratch set's filters, then each set's filters in priority order.
`Verdict::Included(index)` keeps carrying one flat index — into the known list.
A set being disabled hides its filters from the pane and stops them matching; it
does not remove them from the list.

### Numbers and colours

**Colour is a property of the filter, assigned once.** Today `Filter.style` is
set at `add` time from the palette position and never recomputed, which is why
deleting filter 2 does not recolour filter 3. Sets inherit that rule: a
file-loaded filter takes its colour from its position in the known list at load,
unless the file names one with `colour`; a filter added to the scratch set takes
the next palette colour after every known filter. Toggling a set on or off
changes what is shown and what matches. It never changes a colour. Changing a
set's `priority` moves it in the known list and so changes palette colours at
the next start, as reordering filters by hand does today.

`colour` exists because a palette position is a poor way to say "errors are
red". Its value takes the same spellings as `[filters] palette` in
`config.toml`: a name, a hex triple, or a 256-colour index as a string.

**Numbers are labels, not addresses.** Nothing in recon addresses a filter by
its number — there are no digit bindings — so the pane numbers what it shows,
top to bottom, continuously across sets. Enabling a set renumbers the filters
below it in the pane and nothing else, the same way deleting a filter renumbers
today.

### Enabling a set

When a set becomes enabled:

- if it has a profile named `default`, that profile is applied;
- otherwise its filters keep whatever flags they had. A freshly loaded set's
  filters are all disabled, so the first enable of a set with no `default`
  shows the set's header and a column of `[ ]` rows.

Disabling a set touches no filter flag. Enable, toggle a few, disable, enable
again: with no `default` profile the toggles are still there; with one, the
profile wins. **A profile is an action, not a live binding** (#8): it writes the
flags and then stops mattering, and nothing reconciles the user's later toggles
against it.

### `autoload`

A set with `autoload = true` starts enabled, with `default` applied if it
exists. Any number of sets may autoload. Without the key a set starts disabled
and shows only its header row. `autoload` replaces the original idea of a magic
set named `default`, which overloaded that word with the profile of the same
name.

### Solo

From audio mixers: focus on one set and silence the rest, then put everything
back. `s` on a set's header row:

- snapshots every set's enabled flag, the scratch set's included, then enables
  only the selected set. If it was disabled it is enabled the normal way, so
  `default` applies. Filter flags are untouched: the soloed set shows exactly
  the toggles it had.
- `s` on the soloed set restores the snapshot. `s` on a different set while
  soloed moves the solo there; the original snapshot is kept, so un-soloing
  later returns to the world before the first `s`, not to the intermediate one.
- The scratch set is suspended too. While soloed, the pane shows the search row
  if there is one and the soloed set alone. That is what "isolated" means, and
  the scratch filters return on un-solo.
- Toggling a set by hand while soloed is drift, like toggling a filter during
  `!`. Un-solo restores the snapshot regardless.

Solo is independent of `!` and the peek, which act on filter flags. Soloing a
set and then pressing `!` inside it does what both keys say.

A dozen sets autoloaded, one `s`, one profile, and the pane is a single set's
rows: the startup-heavy configuration the priority setting is for.

### Reset

`R` returns every set to its startup state. Each set is enabled if and only if
it has `autoload`. Each file filter is set from its set's `default` profile if
there is one, else off. Any solo and any pending `!` memory are dropped.

Scratch filters are not deleted and their flags are left alone: a reset must
never destroy something the user typed. Because it touches only flags and is
one key to redo from any state, it asks no confirmation.

### `!`, the peek, and `Ctrl-H`

`!` and `space` operate on **filter flags only**, across every known filter,
and never on a set's flag. `!` with sets enabled disables every filter, remembers
which were on, and restores exactly those; the sets stay enabled throughout. The
peek captures and restores filter flags the same way. Neither needs to know sets
exist, and a set toggled during a peek is the same "drift" case a filter added
during one is today.

`Ctrl-H`, `n`/`N`, dimming and the navigator all read "is anything selecting?"
through the effective rule, so a set of enabled filters inside a disabled set is
quiet everywhere, not just in the pane.

## The file

`~/.config/recon/filters.toml`, located by the same rules as `config.toml`
(`$XDG_CONFIG_HOME`, else `$HOME/.config`; a relative `$XDG_CONFIG_HOME` is
ignored). No CLI flag and no environment variable name the file in v1; #46
covers a search path if one is ever wanted.

```toml
# ~/.config/recon/filters.toml

[sets.WiFi_debug]
priority = 10                            # lower is nearer the top; default 50
autoload = true                          # start with this set enabled
# mode = "or"                            # reserved: only "or" is accepted

[sets.WiFi_debug.profiles]
default = ["assoc", "deauth"]            # applied whenever the set is enabled
WiFi_bug_32 = ["deauth", "beacon-loss", "retry"]
WiFi_disconnect_failures = ["deauth", "beacon-loss"]

[[sets.WiFi_debug.filters]]
name    = "assoc"
pattern = 'wlan\d+: associated'

[[sets.WiFi_debug.filters]]
name    = "deauth"
pattern = 'deauthenticat(ed|ing)'
colour  = "red"                          # instead of the next palette colour

[[sets.WiFi_debug.filters]]
name    = "beacon-loss"
pattern = 'beacon loss'
sense   = "context"                      # include (default) | context | exclude

[[sets.WiFi_debug.filters]]
name    = "retry"
pattern = 'retry|backoff'
sense   = "exclude"

# A built-in set (#127) can be positioned and switched, but has no filters here.
[sets.definitions]
priority = 80
autoload = false
```

Single-quoted TOML strings do no escape processing, which is why TOML won this
file's format decision in #8: every regex goes in verbatim, with no `\\` tax.

**Keys, and what is rejected.** The schema uses `deny_unknown_fields`
throughout, as `config.toml` does, so a misspelt key is an error naming the key
rather than a setting that silently does nothing.

| Key | Required | Meaning |
|---|---|---|
| `sets.<name>` | — | one table per set; the table's key is the set's name |
| `priority` | no, default `50` | position in the pane, lower first; ties by name |
| `autoload` | no, default `false` | enabled at startup |
| `mode` | no | reserved for #40. `"or"` is accepted and means nothing; any other value is an error |
| `profiles.<name>` | no | a list of filter names in this set |
| `filters` | yes for a file set, at least one; **forbidden** for a built-in set's table | an array of tables |
| `filters[].pattern` | yes | a regex, compiled with the same `regex` crate rules as `i` |
| `filters[].name` | no | the filter's handle; defaults to `pattern` verbatim |
| `filters[].sense` | no, default `"include"` | `include`, `context`, or `exclude` |
| `filters[].colour` | no | a colour name, `#RRGGBB`, or a 256-colour index as a string; else the palette |

A table whose name is a built-in set's (#127) overrides that set's `priority`
and `autoload` and may carry nothing else. Until #127 lands there are no
built-in names and every `[sets.*]` table is a file set.

Validated at load, before the terminal is taken, with the same "refuse to start"
policy as `config.toml` and for the same reason (#18): a warning printed and then
overwritten by the alternate screen is a warning nobody reads. Each error names
the file, the set, and where it applies the filter:

- a pattern that does not compile — with `regex`'s own message;
- a `colour` that does not parse — with the same message `[filters] palette`
  gives, listing the accepted forms;
- two filters in one set with the same name, after the `pattern` fallback;
- a profile naming a filter the set does not have;
- a file set with no filters, or a built-in set's table with any;
- `mode` set to anything but `"or"`;
- a set named with the empty string.

A missing file is not an error. A file that exists and cannot be read is.

**Never written, until #131.** Loading is read-only. `Cargo.toml` still has
`toml`'s `display` feature off, so `toml::to_string` does not exist in the
build. When the save path lands (#131) it uses `toml_edit`, which preserves the
comments and ordering of a hand-edited file, and `display` stays off.

## The pane

Three kinds of row, in this order: the live search's row if there is one; the
scratch set's filters with no header; then, for each set in priority order, a
header row and — only while the set is enabled — its filters indented beneath
it.

```
Filters
 /[x] inc ETIMEDOUT              ← live search, as today
 1[x] inc ERROR                  ← scratch, no header
 2[ ] exc DEBUG
[x] WiFi_debug *                 ← a set with profiles
   3[x] inc assoc
   4[x] inc deauth
   5[ ] ctx beacon-loss
   6[ ] exc retry
[ ] bug-57                       ← disabled: header only
```

A header row shows `[x]` or `[ ]` and the set's name, a trailing `*` when the
set has at least one profile, and `solo` when it is the soloed set. Filter rows
show the filter's **name**, which is its pattern unless the file said otherwise
— so `deauth` rather than `deauthenticat(ed|ing)`. A disabled set and an enabled
set whose filters are all off look different: the first is one `[ ]` row, the
second is a `[x]` row over a column of `[ ]` rows. They mean different things
and the pane says so.

While soloed the pane shows the search row and the soloed set only; the scratch
rows and every other header are absent until `s` restores them.

Selection is one row index over this list, as today. Keys, on the selected row:

| Key | Filter row | Header row |
|---|---|---|
| `Enter` | toggle the filter | toggle the set |
| `d` | remove the filter for this session | status message: sets are defined in `filters.toml` |
| `c` | edit the pattern in place | same message |
| `m` | flip include / context | same message |
| `a` | — | open the profile picker for this set |
| `s` | — | solo this set, or un-solo it |
| `R` | reset every set to its startup state | same |
| `i` / `x` | add to the scratch set, wherever the selection is | same |

`a`, `s` and `R` are unbound today. The three inert cases say why rather than
doing nothing, following #120's "no silent keys". `R` is uppercase because `r`
is the global refresh-from-disk.

**The profile picker** is a small centred overlay listing the set's profile
names, one per row, drawn over the panes the way the `?` overlay is. `j`/`k`
move, `Enter` applies and closes, `Esc` closes. It takes every key while open,
as the search prompt does. Applying a profile enables exactly the named filters
and disables the set's others; it does not touch other sets and does not enable
the set if it was disabled (the picker only opens on an enabled set's row, since
a disabled set's filters are not shown).

**Height.** `preferred_height` counts rows, header rows included, and the
existing cap against the navigator's minimum applies unchanged. A user with many
sets sees the pane grow to the cap and scroll, as a long filter list does today.
Priority and solo are the two tools for keeping what matters in view.

## The navigator

Nothing changes in `Matcher` or the scan. Its `selects` and `exclude` masks are
built from *effective* filters rather than enabled ones, and `pattern_key`
already covers every known pattern whether or not it is enabled, so toggling a
set, soloing, and resetting all re-answer the folder from the bitset cache with
no I/O, exactly as toggling a filter does. The 64-pattern ceiling (#119) now
counts every known filter across every set; above it the navigator's matching
switches off, as before. A file with a great many sets can reach this, and the
README says so.

## Where built-in sets go (#127)

Sets have an origin: **scratch**, **file** (carrying the path it was read
from), or — reserved for #127 — **built-in**. A built-in set is like the scratch
set in that it is never in the file's filter tables, and unlike it in that it
has a name, a header row, a priority, can be disabled, and its filters are
`Predicate::Definition` rows (#123). It sorts among the file sets by priority,
and a `[sets.<its name>]` table overrides its `priority` and `autoload`.

Two rules are stated here so #127 adds a variant and nothing else:

- **Numbering and the palette cover user-authored filters only** — scratch and
  file. A built-in filter has no number and a fixed neutral style. `next_style`
  counts user-authored filters, not the known list.
- Solo, reset, `!` and the peek treat a built-in set exactly as a file set.

## Saving (#131)

The one write path. `S` in the filter pane saves the scratch set as a named set:

1. A prompt asks for the set's name. A name already loaded is refused with a
   message; so is an empty scratch set.
2. The new table is appended to `filters.toml` with `toml_edit` — one
   `[[sets.<name>.filters]]` entry per scratch filter with its `pattern` and,
   when not `include`, its `sense`; a `profiles.default` listing the filters that
   are enabled right now, so the current activation is what the set opens with;
   no `autoload`, no `priority`, no `colour`. Comments and every existing table
   survive untouched.
3. The written text is re-parsed to prove a restart would load it, then the
   scratch set becomes the new set in memory: enabled, with the same filters
   on, at priority 50. Other sets keep whatever state they are in — the model
   is not rebuilt from the file, so a save never undoes the user's toggles.

If the file does not exist it is created. If it cannot be written the save fails
with a status message and the scratch set is left alone. Names are used as
written, so `bug 57` becomes the quoted key `[sets."bug 57"]`.

Not in #131: saving a profile onto an existing set, editing a set's members or
priority from inside recon, or deleting a set. Each is a one-line hand edit to a
file that `S` has just shown the shape of.

## What #39 and #40 mean here

`&` (#39) ANDs the **union**: a line is included when every effective `Include`
filter across every enabled set matches it. #39 ships first and does not know
sets exist; this spec records the meaning so that #8 does not silently redefine
it. The reserved `mode` key is #40's seam — AND within a set, OR between sets —
and stays a rejected-unless-`"or"` key until #40 thaws.

## Seams for later

Not designed, but the model leaves a place for each so that adding it changes
nothing already written:

- **Several files, and unloading one.** `Origin::File` carries its path from
  day one. #46's search path loads more files; "unload" is then "drop every set
  with this origin", and a set that is not loaded takes no pane rows.
- **Cross-set references in profiles.** Profile members are names within one
  set. A later `other_set/name` spelling can extend the member syntax so that a
  profile pulls in, say, a context filter from another set. Versions of a set
  are deliberately not designed for.
- **Shared filter definitions.** `ref = "name"` beside `pattern`, pointing at a
  top-level `[filters.<name>]` table, if the same pattern in several sets ever
  becomes a maintenance problem.

## Testing

- **Parsing and validation are pure.** `parse(text: &str) -> Result<Vec<LoadedSet>, Error>`
  is tested with inline TOML; the filesystem is touched only by the one test
  that proves a missing file is not an error, using a fixture under
  `target/test-config/` claimed through the existing mutex pattern.
- **No environment variables in tests.** The path resolver takes `$XDG_CONFIG_HOME`
  and `$HOME` as arguments, as `config_path_from` does.
- **The model is tested at `ActiveFilters`.** Effective-enabled, set toggling,
  profile application, solo and un-solo, reset, `!` across sets, priority
  order, and colour stability across a toggle each get a test that builds sets
  in code with no file at all.
- **The pane is tested through `rendered()`**, the existing helper, asserting on
  row text and per-row style.
- **Startup is tested through `App::new`** with a `Config` that carries loaded
  sets, so a `filters.toml` fixture is not needed to prove autoload.

## Delivery

One PR each, in this order. The two prerequisites and the one follow-on are
existing issues.

| Issue | Delivers | Why this order |
|---|---|---|
| #39 | global AND mode | #8's thread asked for it first; it settles the colour question AND mode raises |
| #123 | `Predicate` enum, model only | the loader, the name fallback and `Matcher` are built on `Filter` — build them once, on the enum |
| **#128** | the model and the loader: sets, scratch, effective-enabled, priority, `filters.toml` read and validated, `autoload`, `colour`, profiles parsed and `default` applied at startup. The pane stays flat and shows every known filter | a shippable increment: write the file, get the filters at startup |
| **#129** | the two-level pane: header rows in priority order, `Enter` on a set, hidden rows for disabled sets, the three inert keys, `default` applied on enable | the pane rebuilt once, on the finished model |
| **#130** | the profile picker and the `*` marker | pure UI over a model that already applies profiles |
| **#132** | solo and reset | set-level verbs over the finished pane |
| **#131** | `S` saves the scratch set with `toml_edit` | the last piece, and the only write |
| #127 | the built-in definitions set | a third origin on a model that has two |

## Out of scope

- A search path or per-project files (#46), and unloading.
- Cross-set references and shared filter definitions (see *Seams*).
- Deleting, renaming or editing sets from inside recon beyond `S`.
- AND within a set (#40).
- Per-file automatic set selection ("open a `.rs`, get the Rust set").
- Sharing sets between machines; the file is copyable, and that is the feature.
