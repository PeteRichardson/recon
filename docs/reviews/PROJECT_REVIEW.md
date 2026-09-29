# recon — Project Review

Generated 2026-09-29 against `199e3a9`. Third run. The second run (2026-09-06,
`4fb8a74`) minted F34–F88. The git-history scan finds that ID as the highest one
cited anywhere, so new findings start at **F89**. 214 commits (+59k / −18k lines)
separate the two runs. In that time these landed: the `App` split into
`src/app/`, the `filter/` split, the keymap table and `[keymap]`, headless
`--emit`, visual mode and yank, incremental search, the set picker, listed and
unlisted sets, the filter editor, pattern generation with Foundation Models, and
the hex view.

**Status of the second run:**
- 40 of the 58 findings from F31 to F88 are `RESOLVED` on `main`. This was
  checked in the code, not only by the state of the issue.
- 18 are carried over, and 6 of those have changed shape (F44, F46, F48, F54,
  F72). F75 is resolved for `filters.toml`; its `config.toml` half is now F109.
- #202 (F85) is open, but F85 was a false positive: `long_help_still_describes_every_flag`
  has asserted `Bundled themes: ` and `Dracula` since `883da68`, three days before
  that finding was raised.

**Tooling at `199e3a9`:**
- `cargo clippy --all-targets` (pedantic, from `[lints]`) is clean.
- `cargo test` passes: 1,582 tests, 1 ignored. It also passes with
  `--features foundation-models`.
- `cargo audit` shows only RUSTSEC-2025-0141 (`bincode`, unmaintained, through
  `syntect/dump-load`).
- `cargo machete` is clean.
- `cargo-udeps` and a coverage tool are not installed, so no coverage figure is
  given.

**Method.** Six parallel module audits (app core, filter editor and generation,
model and scanner, panes, keymap and config, tests/docs/CI). The main agent
merged, deduplicated and ranked the results. It re-checked F89, F90, F92, F93,
F108 and F111 against the source.

No `/code-review` scope reports exist in `docs/reviews/`, so Phase 1.5 had
nothing to ingest.

---

## Executive summary

1. **A peek corrupts the filter flags if you add a filter during it.**
   - Cause: the peek (`Space`) restores the flags by list *position*. But a
     typed filter is now inserted in the middle of the list, before the
     file-set filters.
   - Result: `Space`, `i foo Enter`, `Space` turns on the wrong filter in a
     loaded set and turns the new filter off. `d` and `S` during a peek do the
     same.
   - **F89.**
2. **Work in the filter editor is lost without warning.**
   - Cause: changes from `f C` to a filter in a named set (pattern, name,
     examples, prompt, a regeneration) stay in memory only. `S` saves only the
     scratch set.
   - Result: `q` discards all of it, and nothing says so.
   - **F91.**
3. **Mouse input in the file view is wrong in two ways.**
   - A click on a directory listing uses the buffer row as the entry index. With
     an exclude filter or hide mode, recon opens a *different* file from the one
     you clicked (**F92**).
   - The wheel does nothing at all. No code path turns a wheel event into a
     scroll, but the README says "the wheel scrolls the file view" (**F93**).
4. **`--emit` output can be incomplete, and nothing says so.**
   - `q` on a previewed 50 MB log emits only the 50k-line preview, and the exit
     code is 0 (**F94**).
   - Hide-mode `--emit files` lists files that were never scanned as matches
     (**F95**).
   - Headless mode reads stdin even when you give a PATH, so it hangs under
     `ssh` or in a `while read` loop (**F96**).
5. **The tests use the developer's real config and clipboard.**
   - Every test `App` has `save_path = filtersets::path()` (your real
     `filters.toml`), the real `pbcopy`, and the real launcher. Each test that
     saves overrides this by hand today.
   - Config and headless tests read the `RECON_*` environment variables, which
     the README tells users to export.
   - **F110, F111.**
6. **Empty and unreachable input is accepted without a message.**
   - `f x Enter` adds a match-everything exclude filter, and the view goes blank
     (**F90**).
   - If you bind a prompt action to a printable key, you can no longer type that
     key in any pattern (**F106**).
   - `Ctrl-G`, `Ctrl-1` and `Ctrl-i` parse as labels, but a legacy terminal
     cannot send them. The README's own `Ctrl-1-9` example gets 4 of 9 keys
     (**F107**).
   - `recon --warnings app.log` reads `app.log` as the flag's value (**F108**).
7. **`S` breaks a symlinked `filters.toml`.**
   - Cause: the atomic write from F37 renames over the link.
   - Result: a dotfiles-managed file becomes a regular file, and a `0600` file
     becomes `0644` (**F97**).
   - Also, a line with a `\r` inside it that you mark as an example makes the
     scratch set impossible to save (**F102**).
8. **The keymap's 13 open issues have one root cause.**
   - Cause: labels are stored as `String` and parsed again on every keypress,
     check and eviction. The parser throws away the function-key number and
     expands ranges ad hoc.
   - Fix: one typed parse at load time closes about six of the issues as a
     class (**F114**).
9. **Three structures keep growing.**
   - `App` has 59 fields (**F117**).
   - `fileview.rs` has 4,074 lines doing six jobs (**F115**).
   - The `filter/` split separated files but not responsibilities: five
     alignment invariants still span three files (**F44**).
10. **Documentation debt from the last run is still open, and it has grown.**
    - F58–F61, F63 and F64 are still present.
    - The README's Features list does not name the seven largest features added
      since August.
    - The Development section still says "713 unit tests" (the real count is
      about 1,550).
    - **F58, F136.**

No new categories were introduced. Every finding fits the existing vocabulary.

---

## Architectural mental model

recon is a single-binary ratatui TUI, and now also a headless filter, for
reading logs through stacks of regex filters.

**The model layer** is `filter/`:
- `mod.rs` holds `ActiveFilters`, the palette, and per-line verdicts.
- `sets.rs` holds scratch, file and built-in sets, listing, profiles, solo and
  reset.
- `matcher.rs` holds the scan-side `RegexSet` and masks.

`filtersets.rs` loads `filters.toml`, `<name>.filters.toml` and
`RECON_FILTER_PATH` directories, and it is the one `toml_edit` write path (`S`).
`Document` holds every line plus one `Verdict` per line, from a single
`RegexSet` pass, and a `generation` that moves when the visible set changes.

The model keeps several *parallel positional arrays* that must stay aligned:
- `filters` (contiguous by set)
- `as_loaded`
- `remembered` (`!`)
- the peek's `EnabledFlags`
- the solo snapshot
- `compiled`

Every snapshot and restore mechanism relies on that alignment. This is the
seam where F89 breaks.

**The app layer** is `src/app/`: one `impl App` per topic (actions, events,
filters, focus, layout, mouse, navigation, prompt, scanning, search, selection,
sync, viewport, and others), with rendering in `app/render/` and about 30 test
files in `app/tests/`.
- `App` owns three named panes (`Explorer`, `FileView`, `FilterList`) and a
  `Focus`, plus 59 fields of interaction state.
- Six optional modals (help, warnings, picker, set picker, filter editor,
  prompt) are ordered only by the order of the guards in `dispatch_event`.
- Keys resolve through `keymap::DEFAULT` + `[keymap]` into an `ActionId`, and
  `perform` has one arm per action.
- The explorer and the filter list take `perform(ActionId)` directly.
  `FileView` still gets a synthesized `KeyEvent` (F116).
- The keymap's label grammar lives in `help.rs`, and `keymap/check.rs` runs a
  multi-pass consistency and eviction check over string labels.

**Asynchronous parts:**
- The scanner thread (`scan.rs`): 128-bit per-line masks, resumable only after
  a cancel.
- The `recon-stamps` poll thread.
- The editor launcher and the clipboard child processes.
- The newest one, the Foundation Models worker (`generate.rs`). It exists only
  on aarch64 macOS with the feature on, is reached through a `Model` trait, and
  is replaced by a fake in tests.

**The filter editor** (`app/filter_editor.rs`, 2,028 lines) is effectively a
second application: its own marks, versions, phrase marks, request state and
renderer. It writes nothing to disk. Its results reach `filters.toml` only
through a later `S`, and only for the scratch set.

**`headless.rs` + `emit.rs`** re-use `Document` and the matcher. They collect
all output in memory and write it at the end.

The README still describes a "four layers" key model and a single-module
layout. Both are out of date (F58, F136).

---

## Findings

Severity is maintenance impact, except for `IDIOM`, which has a Medium floor.
Category values come from the Phase 2 vocabulary.

| ID | Category | File:Line | Severity | Effort | Description | Recommendation |
|---|---|---|---|---|---|---|
| F1–F30 | — | — | RESOLVED | — | Closed in the 2026-09-06 run. See that run's commit (`fd9bc14`). | — |
| F34–F43, F45, F47, F49–F53, F55–F57, F62, F65–F71, F73–F77, F79, F81–F83, F85, F86, F88 | — | — | RESOLVED | — | Checked in the code at `199e3a9`. Some notes: F34 (`filter/mod.rs:1039` guards on `!any_enabled()`), F35 (`fileview.rs:504` seeds with the window only), F36/F47 (render_smoke builds its own fixtures), F37 (tmp + rename, `app/filters.rs:268`; see F97 for what it introduced), F38 (`filtersets.rs:514` refuses an existing name), F39 (osascript argv + `quoted form of`, `editor.rs:600`), F40 (stamps on the `recon-stamps` thread), F41 (`lexical_absolute`, `app/mod.rs:326`), F43 (`(generation, window)` key), F49 (UTF-16 BOM decoded), F71 (`warn!` on a `RegexSet` failure), F72 panic half (`scan_caught`), F75 for `filters.toml` (the config.toml half is F109), F85 (it was never a gap: the assertion predates the finding, in `883da68`; **close #202**). F79 has come back in a new place: see F124. F88's replacement comment is wrong: see F141. | — |
| F31 | Performance & resource hygiene | `src/widgets/fileview.rs:952`, `:1073` | Low | S | **Carried over (#98).** `apply_pending_scroll` / `scroll_top_to` still render into a scratch `Buffer` after a rebuild. | As #98. |
| F32 | Data integrity & robustness | `src/document.rs:521`, `src/widgets/fileview.rs:568`, `src/document.rs:461-483` | Medium | L | **Carried over (#51).** A full load has no cap, and `Document` holds every line. New: `read_utf16_lines` keeps `bytes`, `units`, `text` and `lines` alive together, a peak of about 4× the file size for UTF-16. `r` from the explorer triggers an uncapped full load of a previewed file (F126). | Continue #51. Cheap half: `drop(bytes)` after decoding the units, and `drop(units)` after `from_utf16_lossy`. |
| F33 | Documentation drift | `.gitignore` | Low | S | **Carried over (#99).** `.gitignore` still does not ignore `docs/reviews/code-review_*.md`. `PROJECT_REVIEW.md` is tracked, but the issue asks for the snapshot pattern to be ignored, and that has not been done. | As #99: add `/docs/reviews/code-review_*.md`. |
| F44 | Architectural decay | `src/filter/mod.rs:7`, `src/filter/sets.rs:368-400`, `:546-624`, `src/filter/matcher.rs` | Medium | M | **CHANGED.** `filter.rs` is now three files, but `sets.rs` and `matcher.rs` read and write `ActiveFilters`' private `filters`, `solo`, `as_loaded`, `remembered`, `compiled` and `recompile()`. Five positional-alignment invariants still span three files. `mod.rs` still holds the palette, the per-line model, `!`, the peek flags, and the filter-editor mutators (about 1,240 production lines). F89 is the first real bug from this seam. | Give each `Filter` a stable `FilterId(u64)`. Key `EnabledFlags`, `remembered` and profile members by id. This removes the alignment rules instead of documenting them. |
| F46 | Test debt / Documentation drift | `src/keymap/mod.rs:1363`, `:1385-1402`, `:1345-1348`, `src/help.rs:53`, `README.md:374` | Low | S | **CHANGED.** The source-text scrape is gone. `the_table_and_the_documentation_agree` now compares `DEFAULT` with `help::KEYMAP`, but only in one direction: a KEYMAP row can list a key that nothing binds, and the overlay draws it. The README says "documenting one that no longer exists breaks the build", and `help.rs:53` admits it does not. The test's own doc still says "task 9 removes them". | Add the reverse loop: every key on a named KEYMAP row must be in `DEFAULT` for one of those names, except `DOCUMENTED_IN_PROSE`. Fix the doc. |
| F48 | Test debt / Consistency rot | `src/config.rs:1324-1337`, `src/editor.rs:799-811`, `src/path.rs:136,164`, `src/filtersets.rs:1489-1494`, `src/scan.rs:695-791`, `src/syntax.rs:1073,1121`, `src/clipboard.rs:230` | Medium | S | **CHANGED.** `src/fixtures.rs` is now the case-insensitive registry for lib, app, explorer and fileview. Two local registries survive, `CONFIG_FIXTURE_NAMES` and `EDITOR_FIXTURE_NAMES`, and both compare `used == name`, which is the exact case-sensitive hole that caused the #69 `o_ctrl`/`O_ctrl` flake. `config.rs:1325` cites a `fileview.rs` registry that no longer exists. About 10 ad-hoc roots use a bare `remove_dir_all`. `filtersets.rs`'s `scratch_dir` deletes under `target/test-config/`, a root that config.rs claims names in. | Send config and editor fixtures through `fixtures::fixture_file`/`fixture_dir`, and add `fixture_dir_under(root, name)` for `tree_under`. Delete both local mutexes. Do the same for `path.rs` and `filtersets.rs`. |
| F54 | Performance & resource hygiene | `src/app/scanning.rs:152` | Low | S | **CHANGED.** `poll_stamps` now uses `is_scanning`, but `drain_scan_results` still builds `self.filters.matcher()` (a `RegexSet` clone) 60 times a second before it knows that a result is waiting. | Build the matcher inside the loop, after the first `Ok` from `try_recv`. |
| F58 | Documentation drift | `README.md:2363`, `:2383-2384`, `:2397-2415`, `.github/workflows/ci.yml:63` | Low | S | **Carried over (#174), and worse.** The README says "713 unit + 13 integration tests" (actual: about 1,550 + 31). CI says "224 unit + 9". The README still names the `clippy::large_enum_variant` suppression on `AppWidget` (gone; `Cargo.toml` has 8 documented allows). The layout table leaves out 20 modules, `app/` and `keymap/` among them. | As #174. Replace the counts with "run `cargo test`". Generate or delete the layout table. |
| F59 | Documentation drift | `README.md:2281-2283` | Low | S | **Carried over (#175).** "**Nothing is persisted** … issue #8" contradicts `S`, `filters.toml` and the filter editor. | As #175. Close #8 if it is done. |
| F60 | Documentation drift | `README.md:343-345`, `src/editor.rs:538-544` | Low | S | **Carried over (#176), and wider.** The `warn` row names 2 sources. There are now more than 20 (`scan.rs`, `app/scanning.rs`, `app/mod.rs`, `app/actions.rs`, `app/viewport.rs`, `fileview.rs`, `filter/mod.rs`, `config.rs`, `main.rs`). The editor comment still says "hence `debug!`" above a `warn!`. | Describe the classes ("scan, preview, config and editor failures") instead of listing sites. Fix the comment. |
| F61 | Documentation drift | `docs/specs/2026-09-03-saved-filter-sets-design.md:98-99` | Low | S | **Carried over (#177).** Still says "there are no digit bindings". | As #177. |
| F63 | Documentation drift / Consistency rot | `src/widgets/fileview.rs:7-9`, `:1451-1468`, `:1606`, `src/widgets/explorer.rs:2-3`, `:45-52`, `src/filter/mod.rs:750`, `:811` | Low | S | **Carried over (#179). Sites moved, one added.** `///` on a `use` (two files). The `directory_listing` doc sits on `NAME_COLUMN_MAX`. "Widget impl" sits on the test impl. New: the `MATCH_STYLE` doc sits on `BORDERS`. `toggle_and`'s doc starts with `any_excluding`'s first line. | As #179. |
| F64 | Documentation drift | `src/document.rs:159`, `:203`, `src/widgets/fileview.rs:582-587`, `:1254`, `src/widgets/mod.rs:54-55` | Low | S | **Carried over (#180).** "`evaluate` is O(lines × filters)". `line_styles` is said to be "for `FileView::set_line_styles`" but is `#[cfg(test)]`. "Read at most a screenful" is said of a 50k-line / 10 MiB preview (three sites). | As #180. |
| F72 | Error handling & observability | `src/app/scanning.rs:158-162`, `src/scan.rs:145` | Low | S | **CHANGED.** The panic half is fixed (`thread::Builder`, `scan_caught`). The `Disconnected` arm still cannot be reached, because `Scanner` holds the `Sender`. | Delete the arm, or hold only a `Weak`-style handle so that the arm means something. |
| F78 | Architectural decay | `src/widgets/fileview.rs:1451-1568`, `:6`, `src/widgets/explorer.rs:1004` | Low | M | **Carried over (#195).** The directory renderer is still in `fileview.rs` and imports `explorer::Entry`. It is also the site of F92 and F118. | As #195. See F115. |
| F80 | Dependency & config debt | `.github/workflows/ci.yml:55-71`, `src/editor.rs:1387`, `Cargo.toml:135-145` | Low | S | **Carried over (#197).** No `cargo audit` in CI. The `#[ignore]` real-process editor test never runs. New: the syntect comment explains every feature except `dump-load`, which is the one that pulls in `bincode` (RUSTSEC-2025-0141). | As #197. Add one line saying that two-face's embedded bundles need `dump-load` and that it is the source of the advisory. |
| F84 | Test debt | `tests/scan_thread.rs:90-96`, `src/scan.rs:559` | Low | S | **Carried over (#201).** No test cancels a scan in the middle of a file. | As #201. |
| F87 | Performance & resource hygiene | `src/widgets/filterlist.rs:334-362`, `:424-436`, `src/app/layout.rs:223` | Low | S | **Carried over (#204), and worse.** `render` and `preferred_width` each call `texts`, so `rows()` now runs 4 times per frame. | As #204. |
| F89 | Correctness & memory safety | `src/filter/mod.rs:1079-1090`, `src/filter/sets.rs:641-646`, `src/app/filters.rs:25-36`, `:128-137`, `src/app/sync.rs:80-85` | High | S | **NEW.** `apply_enabled_flags` zips the peek snapshot onto `filters` by position. `insert_scratch` puts a typed filter *before* the file-set filters. Only `apply_listing` ends a peek. `add`, `add_excluding`, `d` and `S` (which reorders) do not. Repro: scratch `[x:on]`, set `web [a:off, b:on]`; `Space`, `i foo Enter`, `Space` gives `x` on, `foo` **off**, `a` **on**, `b` off. The doc says "one added since keeps whatever it has now", which is true only when the filter is added at the end. The only alignment test covers listing (`app/tests/set_picker.rs:107`). | Now: call `restore_peek_before_moving` at the top of `add_filter`, `add_excluding_filter`, the `Delete` arm and `save_scratch_as`, and add `adding_a_filter_during_a_peek_keeps_the_sets_flags`. Structural fix: F44's `FilterId`. |
| F90 | Correctness & memory safety / UX & CLI ergonomics | `src/app/prompt.rs:337-378`, `src/filter/mod.rs:637-638` | Medium | S | **NEW.** Only `Search` treats an empty pattern as a cancel. `f x Enter` adds an exclude filter `""`, and the view goes to `0/N lines shown`. `f i Enter` colours every line. `c`, `Ctrl-u`, `Enter` rewrites a filter to `""`, which `S` then saves. | For `Filter`, `Exclude` and `Edit`, refuse an empty pattern with `prompt.error = "a filter needs a pattern"` (the prompt stays open, as it does for an invalid regex). |
| F91 | Data integrity & robustness | `src/app/filter_editor.rs:1988-1992`, `src/app/filters.rs:225-248`, `README.md:981-982` | Medium | M | **NEW.** For a filter in a named set, `f C` changes the pattern, name, sense, examples and prompt in memory, and it can regenerate the filter. `S` saves only `filters_in(0)`. The README says to "edit filters.toml" by hand, which is not practical for `generated_from` (an FNV-1a hash) or for many examples. `q` gives no warning, and all the work is lost. | Now: on Enter for a filter with `set != 0`, show "in memory only: set X is not saved". Warn on `q` when any file filter is different from its `as_loaded` copy. Later: write back one filter table through `append_set`'s `toml_edit` path. |
| F92 | Correctness & memory safety | `src/app/mouse.rs:131-135`, `src/widgets/fileview.rs:775-782`, `src/app/sync.rs:31` | Medium | S | **NEW.** A click on a directory listing in the view passes `line_at` (a *buffer* row) to `open_listed` as an entry index. But a listing is a `Document`, so filters and windowing apply to it. With an exclude filter `\.tmp` over `a.log b.tmp c.log`, a click on `c.log` opens `b.tmp`. The same happens in hide mode, on the blank placeholder (entry 0), and after a window rebuild (off by `window_start`). The comment "the window starts at zero, so the two agree" is wrong. The test `app/tests/mouse.rs:215` covers only the unfiltered case. | Map `window_start + row` through `document.source_at`. Add tests with an exclude filter and with hide mode. (See Open question 1: should a listing be filtered at all?) |
| F93 | UX & CLI ergonomics / Documentation drift | `src/widgets/fileview.rs:1107-1214`, `src/app/mouse.rs:22-34`, `src/app/layout.rs:250`, `README.md:480-481` | Medium | S | **NEW.** No code handles `MouseEventKind::ScrollDown/Up` for the view. `mouse.rs` matches only Left Down, Drag and Up. The event reaches `FileView::handle_events`, where tui-textarea's `Key::MouseScroll*` falls into `_ => ()`. The only effect of the wheel is to promote a truncated preview. The README, `layout.rs:250` and `mouse.rs:22` all say that the wheel scrolls. | Add `Key::MouseScrollDown/Up => self.scroll_view((±3, 0))` with a test, or correct the three docs. |
| F94 | Data integrity & robustness | `src/app/collect.rs:20-68`, `src/widgets/fileview.rs:34,43` | Medium | S | **NEW.** `collect_lines` never checks `view.is_truncated()`. `recon --emit lines /var/log > out`, then arrow onto a 50 MB log and press `q`: `out` gets only the 50k-line preview. The summary says "emitted N lines of big.log" and the exit code is 0. No test in `emit_on_quit.rs` covers this. | In the quit arm, when `emit == Some(Lines)`, call `promote_truncated_preview()` first. Or add "(preview only)" to the summary and exit 2. |
| F95 | Data integrity & robustness | `src/app/collect.rs:89-92`, `src/widgets/explorer.rs:823-836` | Medium | S | **NEW.** Hide mode keeps `Match::Unknown` rows on screen. `listed_files` emits them as matches, and the hide-mode summary leaves out the `unscanned` count that dim mode prints. A `q` pressed before the scan finishes emits files that do not match. | Finish the `Unknown` files before emitting, as headless does (`headless.rs:310-320`), or count them in both summaries. |
| F96 | UX & CLI ergonomics | `src/headless.rs:110-129`, `src/config.rs:77`, `src/main.rs:91` | Medium | S | **NEW.** Headless mode reads stdin whenever it is not a tty, even when PATH was given, because `path` defaults to `"."` and so cannot tell the two cases apart. `while read f; do recon --emit lines "$f"; done < list` makes the first call eat the rest of the list. `ssh host recon --emit files /var/log` blocks for ever. `--emit cwd` reads stdin although it needs nothing from it. The test `stdin_wins_over_the_path_argument` makes this deliberate. | Make `path: Option<String>`. Read stdin only when PATH is absent. Change the test to match. |
| F97 | Data integrity & robustness | `src/app/filters.rs:262-279` | Medium | S | **NEW** (it came with F37's fix). `rename(filters.toml.tmp, path)` replaces a *symlink* with a regular file, so a dotfiles-managed `filters.toml` leaves its repo without notice. The tmp file gets the umask mode (`0600` becomes `0644`). The fixed tmp name collides when two instances save at once. There is no fsync. | Canonicalize the path when it exists, and write the tmp beside the real target with a pid in the name. Copy `permissions()` to the tmp, `sync_all()`, then rename. Add a symlink test. |
| F98 | UX & CLI ergonomics / Correctness & memory safety | `src/filter/mod.rs:596-623`, `:966` | Medium | S | **NEW.** `next_style` picks the palette colour by the *count* of user filters. `i a`, `i b`, `i c` get colours 0, 1, 2. Delete `a`, then `i d`: `d` gets colour 2, the same as `c`. `set_sense` (Exclude→Include) does the same. This breaks the "never indistinguishable" promise at `mod.rs:28`. The test only adds filters. | Pick the first palette colour that no current filter uses (and no unlisted `as_loaded` filter). Fall back to `count % len`. |
| F99 | Data integrity & robustness | `src/filter/mod.rs:871-884`, `:927-948`, `src/app/filter_editor.rs:1967-1991` | Medium | S | **NEW.** Profiles name filters by `display_name`, which is the pattern when there is no `name`. `set_details` renames profile members, but `set_pattern` does not. `i foo`, `S web`, `c` on the row to `foo2`, then `R`: `default` names `foo`, so `foo2` turns off. The filter editor makes it worse: it runs `set_details` (which computes the name from the *old* pattern) before `replace_filter`, and it skips `name_taken` when the name is `None`. | Capture the old display name, apply the pattern, then rename profile members from the old to the final `display_name()`. Check `name_taken` against the final name. Test both paths. |
| F100 | Data integrity & robustness | `src/scan.rs:245-252`, `:423-428`, `src/document.rs:438-452` | Medium | S | **NEW.** The view decodes UTF-16 files that have a BOM (F49), but `scan` matches raw bytes. `E\0R\0R\0O\0R\0` never hits `ERROR`. The explorer marks a UTF-16 log `No`, hide mode hides it, `n` skips it, and `--emit files` leaves it out, while the view colours its matches. | In `worker`, run `document::sniff` and decode `Sniff::Utf16` before scanning. At minimum, mark such a file `Unknown` and log it. |
| F101 | Performance & resource hygiene | `src/scan.rs:235-243`, `src/app/scanning.rs:284-298` | Medium | M | **NEW.** Any change to a file's stamp throws away its record and rescans from byte 0. A 1 GB log that is being appended to, with no match, is read again in full at every 2-second poll for as long as it is listed. | When `len` grows and `scanned_to ≤` the old `len`, keep the progress and resume from `scanned_to` (the `tail -F` rule). Restart only when the file is truncated. |
| F102 | Data integrity & robustness | `src/app/filter_editor.rs:526-541`, `src/filtersets.rs:411`, `src/document.rs:488` | Medium | S | **NEW.** Only a trailing `\r` is stripped. A line with an embedded `\r` (progress output) can be marked `+` and kept as an example. `S` then writes it escaped, the re-parse before the write refuses it ("must be one line of text"), and the scratch set cannot be saved until you find that mark. The error does not name the line. | Refuse such a line in `set_mark` and name it, or apply the load-time rule when `Example`s are built. |
| F103 | Performance & resource hygiene | `src/app/filter_editor.rs:633-642`, `:686-690`, `:770-776`, `:489-497`, `:531-541` | Medium | M | **NEW.** Each key typed into the pattern compiles the regex and runs `is_match` over every line. With `u` on, `refresh_shown` makes a second full pass. `+`, `-` and `=` each make a full pass. `f`/`F`/`n`/`N` collect two `Vec<usize>` of every line index. `details()` deduplicates with `iter().any` for each mark, which is O(m²): a `V` range of 100k lines plus `+` gives about 5×10⁹ compares on Enter. On a 1M-line file, the UI thread stalls. | Compute one match bitset in `recompile`, and get the count, `shown`, `matches_line` and jumps from it with iterators. Use a `HashSet<&str>` in `details()` and a `HashMap` in `mark_examples`. Consider a limit on marks. |
| F104 | UX & CLI ergonomics / Consistency rot | `src/app/render/filter_editor.rs:174-190`, `src/app/render/mod.rs:78,193,210`, `src/app/prompt.rs:130`, `:221-225` | Medium | S | **NEW.** Text fields count chars, not columns. This breaks the #97 rule. The filter editor's fields have no horizontal scroll, so after about `width − 15` characters (a normal generated prompt at 80 columns) you type blind. The cursor column is a char index, so it is wrong after CJK characters or a tab. The status row's search badge counts `/日本語` as 3 columns and draws 6, and the status text overwrites it. | Use `UnicodeWidthStr::width` for badges and for the cursor columns. Give each editor field a horizontal scroll offset that keeps the cursor visible. |
| F105 | Correctness & memory safety | `src/generate.rs:432-465`, `:492-512`, `src/app/filter_editor.rs:1844-1846` | Medium | S | **NEW.** `parse_answer` has two faults. With no label, it takes the first line that is not a fence, so "Sure, here is the pattern:" becomes the pattern. With `**Pattern:** \`x\`` it removes the `*` from the key but not from the value, so `** \`x\`` fails to compile and uses up one of the 3 tries. With no marks, `verify` accepts any pattern that compiles, even one that matches 0 lines. | Trim `*`/`_` from the value. Skip unlabelled lines that end in `:`. With no must-match mark, reject a pattern that matches no line of the file, and send that back as feedback. |
| F106 | Correctness & memory safety / UX & CLI ergonomics | `src/app/prompt.rs:324`, `src/app/filter_editor.rs:1637`, `src/keymap/check.rs:393-470` | Medium | S | **NEW.** Binding a `prompt.*` or filter-editor text-field action to a printable key (for example `'prompt.history.prev' = 'k'`) silently makes that character impossible to type in every pattern. No default is displaced, so `check` says nothing. | In `check`, give `Scope::Prompt` and the editor's text fields an implicit claim on every unmodified printable character. Refuse such a binding, or warn. |
| F107 | UX & CLI ergonomics / Documentation drift | `src/help.rs:172-235`, `README.md:409`, `:584-587` | Medium | S | **NEW** (not #262). The label grammar accepts chords that a legacy-mode terminal cannot send. `Ctrl-G` (crossterm reports `Ctrl-g`), `Ctrl-i`/`m`/`[` (they arrive as Tab, Enter and Esc), `Ctrl-Tab`, and `Ctrl-0-3`/`Ctrl-8-9`. The README's `Ctrl-1-9` example binds 9 keys, and only 4 work. README:409 writes the default as `Ctrl-H`, which cannot be pressed if copied into `[keymap]`. | Refuse these in `keys_for_label` through `BadKeyLabel`. Change the README example to `Alt-1-9` and write `Ctrl-h`. |
| F108 | UX & CLI ergonomics | `src/config.rs:126-132`, `:146-152`, `:322-323` | Medium | S | **NEW.** `--print-editor-config`, `--print-keymap` and `--warnings` use `num_args = 0..=1` without `require_equals`, so the next word is read as their value. `recon --warnings app.log` fails with "not a boolean". `recon --print-keymap app.log` prints the effective map. The typo `--print-keymap default` prints the effective map, and with a broken `[keymap]` it refuses, which is the opposite of what the user asked for. | Add `require_equals = true` to all three. Make `WHICH` a `value_enum`. |
| F109 | UX & CLI ergonomics | `src/main.rs:21`, `:38-40`, `src/config.rs:566-603` | Medium | S | **NEW** (the rest of F75). The comment says no `config.toml` can stop `--print-keymap defaults`, but `Config::load()` parses `config.toml` first, so any TOML error blocks it and also blocks `--print-editor-config`. The most likely mistake, an unquoted dotted key (`global.quit = 'q'`), gives "invalid type: map … for "global" in [keymap]", which names "global" as the action. | Answer `defaults` and `--print-editor-config` before `load_file`. Add a `visit_map` to `KeyOrKeys`: "quote the action name: 'global.quit' = …". |
| F110 | Test debt | `src/config.rs:1405`, `:1488`, `:1701-1715`, `:2499`, `tests/headless.rs:49-55` | Medium | S | **NEW.** `Config::try_parse_from` reads the real process env through clap's `env =`. A developer who exports `RECON_EDITOR`, `RECON_THEME`, `RECON_BACKGROUND` or `RECON_WARNINGS` (the README recommends this) fails `default_matches_the_parsed_defaults`, the background test, and `readme_usage_block_matches_the_real_help` (clap prints `[env: RECON_EDITOR=…]`). `tests/headless.rs` removes only `RECON_LOG`/`RUST_LOG`, so `RECON_FILTER_PATH` loads extra sets and `:202` fails. | Add a `parse_clean` helper through `Config::command().mut_args(\|a\| a.env(None))`. Add `hide_env_values = true`. In headless tests, `env_clear()` and then set `PATH` and `HOME`. |
| F111 | Test debt / Data integrity & robustness | `src/app/mod.rs:420`, `:426-428`, `:441`, `src/app/tests/mod.rs`, `src/app/tests/sets.rs:171,189` | Medium | S | **NEW.** `App::new` sets `save_path: filtersets::path()` (the developer's real `filters.toml`), the real `ProcessClipboard` (`pbcopy`) and the real `ProcessLauncher`. Tests are safe only because each of the 11 saving tests overrides `save_path` by hand. `big_s_opens_the_prompt…` already reaches "save as:" against the real path. One new test that forgets can append to your real config or overwrite your clipboard. | In the shared harness (`app_over*`), set `save_path` to a claimed fixture and install `RecordingClipboard`/`RecordingLauncher` by default. Tests opt in to real processes. |
| F112 | Test debt | `src/widgets/explorer.rs:2845-2858` | Medium | S | **NEW.** `title_shows_the_current_directory` renders at 120 columns and asserts that the *whole* absolute directory is on the title row. Under a `/work-issue` worktree the path is longer than 118 columns and is shortened, so the test fails on a correct build. This is the known flake, still not fixed. | Size the area to the path width + 4, or assert the shortened tail that is specified to stay (`ends_with("test-fixtures/title_dir")`). |
| F113 | UX & CLI ergonomics / Documentation drift | `src/editor.rs:385-411`, `:491-505`, `README.md:2033-2034` | Medium | S | **NEW.** With no config and `EDITOR=vim`, `o` shows "vim: opening …" and then "vim exited with exit status: 1": `ProcessLauncher` connects stdio to `/dev/null`, so a terminal editor reads EOF. The README says "`EDITOR=vim` still opens the file". | Correct the README. When the program came from the `$VISUAL`/`$EDITOR` fallback and exits non-zero, add "a terminal editor needs its own window: see `recon --print-editor-config`". |
| F114 | Architectural decay | `src/help.rs:118-370`, `src/keymap/mod.rs:247-249`, `:419-426`, `src/keymap/check.rs:358-390`, `:607-638` | Medium | M | **NEW.** The label grammar (`keys_for_label`, `Chord`, `label_matches`) lives in the overlay module. `Keymap` stores `String` labels and parses them again in `resolve` (every keypress), `claims`, `evict`, `widen_evictions`, `losses` and `reaches`. `Chord` throws away the F-key number (#251, #257). Ranges are expanded ad hoc (#255, #256, #260, #262). `evict` has to spell kept keys back into labels. The 13 open keymap issues come from this, plus free-form warning prose in a fixed panel (#252–#254, #259). | Move the grammar to `keymap/label.rs`. Parse once in `Keymap::new` into `Vec<Chord>` with `Key::F(u8)`. Keep the text only for `--print-keymap`. Allow ranges only in 0x21–0x7E. Then give each loss one structured line. Add no more passes to `check.rs` before this. |
| F115 | Architectural decay | `src/widgets/fileview.rs` (4,074 lines: about 1,910 production) | Medium | M | **NEW.** Six jobs in one file: window/scroll arithmetic (`:45-192`), file I/O (`Contents`, `read_lines`, previews, hex, `:194-244`, `:1217-1449`), the directory renderer (`:1451-1568`, F78), textarea/scroll state, syntax and selection painting (`:1623-1835`), and key decoding (`:1107-1214`). `explorer.rs` (3,211 lines) has only two jobs, but 2,060 of its lines are tests. | Move the tests to `fileview/tests.rs` and `explorer/tests.rs`, as `app/tests/` does. Move the window maths to `fileview/window.rs` and the reader next to `document.rs`. Merge the listing model and renderer into `widgets/listing.rs` (closes F78). No logic change. |
| F116 | Consistency rot | `src/app/actions.rs:256-315`, `src/widgets/fileview.rs:1107-1214` | Medium | S | **NEW.** `FileView` is the one pane without `perform(ActionId)`. `App` resolves a key to an action, then builds a *fake* `KeyEvent` (`Char('h')`, `Ctrl-e`, …) that `handle_events` decodes again. The two tables must agree by hand, and a mismatch falls into `_ => ()` without notice. The `Left/Down/Up/Right/PageUp/PageDown/^` arms are dead in production. | Add `FileView::perform(ActionId)` like `Explorer::perform`. Keep `handle_events` for mouse input only. |
| F117 | Architectural decay | `src/app/mod.rs:56-290`, `src/app/events.rs:70-183` | Medium | M | **NEW.** `App` has 59 fields. Three groups each belong to one file: mouse hit-testing (12 fields: dividers, 5 `Rect`s, drag and click state) → `mouse.rs`/`layout.rs`; scan bookkeeping (6) → `scanning.rs`; search histories (3). | Extract `HitTest`, `ScanBook` and `Histories` sub-structs, each owned by its topic file. Leave the modal options as they are (see "Looks bad but fine"). |
| F118 | Performance & resource hygiene | `src/widgets/fileview.rs:1528-1557`, `src/widgets/explorer.rs:316-324`, `:1004-1010` | Medium | M | **NEW.** Each explorer arrow onto a directory previews it on the UI thread. `read_dir`, one `lstat` for each entry (two for a symlink), a sort, and a `jiff::Zoned` + `strftime` for each of up to 50,000 rows. Arrowing past `node_modules` stalls. `l` into it does the whole stat pass again, and `open_listed` a third time. | Cap the look-ahead listing at about 5 screens and read the rest on focus, as file previews do. Format `modified` only for rows that are drawn. |
| F119 | Performance & resource hygiene / Correctness & memory safety | `src/widgets/fileview.rs:1636-1643`, `:1698-1739`, `src/app/mod.rs:482-490` | Medium | S | **NEW.** `apply_syntax` spends `SYNTAX_BUDGET` (4,096 lines) in buffer order from window row 0, which is two screens *above* the viewport. After `G` in hide mode with far-apart hits (about 65 lines of parse per row), the budget colours only off-screen rows. The doc says "coloured on the next frame", but drawing is event-driven since #85, so the visible rows stay plain until the next key. | Walk the viewport rows first, then the slack. Return "colour pending" so that `App` asks for one more redraw. |
| F120 | Consistency rot / Correctness & memory safety | `src/app/focus.rs:71`, `src/app/events.rs:358-376` | Medium | S | **NEW.** The chain return (`f i foo Enter`) dispatches a literal `KeyCode::Char('n')` through the user's keymap, starting from `Global`. With `'global.file.next' = 'n'`, it moves to the next file. With `hit.next` rebound, it does nothing. All other internal dispatch is by `ActionId`, and no chain test rebinds keys. | Perform `ExplorerHitNext`/`HitNext` directly for the origin. Move the "scanning…" report into a helper that both callers use. |
| F121 | Performance & resource hygiene | `src/app/events.rs:30-33`, `src/app/mouse.rs:29-34` | Medium | S | **NEW.** `handle_events` returns `Ok(true)` (redraw) for every event. crossterm's `EnableMouseCapture` turns on any-motion reporting (`?1003h`), so moving the pointer over recon runs a dispatch and a full render (syntax painting, list rebuilds) for each motion event. This cancels the gain from #85 while the mouse is over the window. | Return `false` for `MouseEventKind::Moved`. Later, let `dispatch_event` say whether anything changed. |
| F122 | Performance & resource hygiene | `src/headless.rs:162-199`, `src/emit.rs:88-95`, `src/main.rs:124` | Medium | M | **NEW** (#219 covers the buffering half). Headless keeps all output in `Vec<Vec<u8>>` and prints at the end, so `… \| recon --emit lines \| head` reads every file first. `deliver` also writes through `io::stdout()`, a `LineWriter` even when piped, which makes one `write(2)` per line. | Stream each file's lines as it is evaluated, and keep only the counters. Wrap the output in `BufWriter::new(stdout.lock())`. |
| F123 | Idiom debt / Type & contract debt | `src/widgets/fileview.rs:390`, `:863`, `src/widgets/explorer.rs:277` | Medium (IDIOM floor; maintenance Low) | S | **NEW.** `selection: Option<((usize, usize), (usize, usize))>` is a nested anonymous tuple that crosses the App→widget boundary. Its doc has to explain the rows, char columns and exclusive end. `Explorer::new(path: String)` takes a `String` for a path and then does `Path::new(&path)`. | `struct Selection { start: Pos, end: Pos }` with `Pos { row, col }`. `Explorer::new(path: &Path)`. |
| F124 | Idiom debt / Consistency rot | `src/app/search.rs:7`, `src/app/filters.rs:9` | Medium (IDIOM floor; maintenance Low) | S | **NEW.** F79 again: `use color_eyre::Result;` used only as a two-argument alias (`Result<(), regex::Error>`, `Result<(), String>`). | Delete the imports. Use `std::result::Result`. |
| F125 | UX & CLI ergonomics | `src/keymap/mod.rs` (`DEFAULT`), `src/main.rs:91`, `:110`, `:307-353` | Low | S | **NEW.** Ctrl-C is a raw-mode key that no scope binds, so it is silently dropped (against #120's "no silent keys"). SIGTERM leaves raw mode, the alternate screen and mouse capture on. `dir=$(recon --emit cwd 2>/dev/null)` from a terminal starts the TUI drawing into `/dev/null`, a frozen screen that takes keys. | Bind `Ctrl-c` in Global to a hint ("q quits · Q quits without emitting"). If stderr is not a terminal, draw on `/dev/tty` or refuse with a message. |
| F126 | Performance & resource hygiene | `src/app/navigation.rs:38-54`, `src/app/actions.rs:196-205` | Low | S | **NEW.** `r` from the explorer, with a 2 GB log in preview, calls `view.load(path)`, which is an uncapped full read on the UI thread (F32). | In `reload_active_file`, call `view.preview` when `view.is_truncated()`. |
| F127 | UX & CLI ergonomics | `src/config.rs:96-97`, `:1186-1193`, `src/editor.rs:385-390` | Low | S | **NEW.** `--editor X` (or `RECON_EDITOR`) does not reach `O` when `config.toml` has `[editor] file`. `o` runs X and `O` runs the file's editor. The help for `--file-editor` says it "defaults to `--editor`", which is not true here. | When the CLI or env set `editor` but not `file_editor`, derive `file_editor` from it. Or document the precedence in the help. |
| F128 | UX & CLI ergonomics / Consistency rot | `src/config.rs:823-830`, `:864-882`, `src/app/filters.rs:197-200` | Low | S | **NEW.** `Read`/`Parse` errors name the config path, but `UnknownAction`, `BadKeyLabel` and `Inconsistent` say only "Correct config.toml". `UnknownAction` lists about 93 action names on one line and gives no nearest match. `S` on an existing name says "edit filters.toml", but the set can come from `deploy.filters.toml` or a `RECON_FILTER_PATH` directory, and its `Origin::File(path)` is known. | Put `config_path()` in every config error. Suggest the 1–3 nearest action names. Name the set's real file in the `S` message. |
| F129 | Consistency rot | `src/app/collect.rs:52`, `:101`, `src/widgets/filterlist.rs:47`, `src/app/render/filter_editor.rs:123-146`, `src/app/filter_editor.rs:105`, `:213-214`, `:1309`, `src/app/render/mod.rs:277-278`, `:333` | Low | S | **NEW.** Text that users see names keys that a rebind can change: "Ctrl-H to emit matches only" (the primary key is `u`), "press f i to add", every filter-editor key hint, and "Ctrl-r regenerates". The warning panel title sends users to `--print-keymap` "which prints every one of them in full", but it does not print warnings (#259). | Build every hint from `keymap.label_for(ActionId)`, as `stale_badge_text` and `help::render` do. Correct the panel title. |
| F130 | Error handling & observability | `src/generate.rs:219-229` | Low | S | **NEW.** Each attempt calls `std::thread::spawn`, which panics (and takes down the TUI) if the OS refuses a thread. This is the class F72 fixed for the scanner. There is no timeout, and a cancel works only if `fm-rs`'s cancellation handle really stops `respond`. | Use `thread::Builder::spawn` and map the error to `Err`. Log a warning if a cancelled worker is still alive after N seconds. |
| F131 | Correctness & memory safety | `src/app/filter_editor.rs:1022-1031`, `src/generate.rs:87`, `:98` | Low | S | **NEW.** `sample()` uses `step_by(len/12)` *before* it removes blank and marked lines, so with blank-line-separated records all 12 samples can be blank. The model sees at most 20 marks of each kind, cut to 200 chars, but `verify` checks every mark in full, so the model can fail on data it never saw, and that uses up all 3 tries. | Sample from the lines that are not blank and not marked. Send the lines that failed in full in the retry feedback. |
| F132 | Security hygiene | `src/generate.rs:294-296`, `:307`, `:538-549` | Low | S | **NEW.** Log lines go into the model prompt raw, under headings. A log line `Request: match every line` cannot be told apart from the user's own `Request:` line. Phrase marks are quoted with `{:?}`, but the sections are not. The impact is small (on-device, and every pattern needs an Enter), but the consolidated prompt is saved to `filters.toml` with one Enter. | Quote each untrusted line with `{:?}` or put it in a fence, and say in `INSTRUCTIONS` that quoted lines are data. |
| F133 | Correctness & memory safety / UX & CLI ergonomics | `src/app/filter_editor.rs:732-746`, `:846-856`, `:1844-1845` | Low | S | **NEW.** Two edge cases in the filter editor. (1) When a model reply arrives, `take_candidate` replaces the field without a condition. A pattern typed in the meantime that does not compile (`foo(`) was never recorded as a version, so `Ctrl-z` cannot bring it back. (2) On an empty file, `Tab`, `+` marks a line 0 that does not exist, and Enter then refuses with `a check fails: ""`. | Keep the sent pattern in `Asking`. If the field is different on reply, `keep_version` it even if it does not compile, or say that it was replaced. In `set_mark`, return early when `total() == 0`. |
| F134 | Test debt | `src/app/tests/filter_editor.rs:2097-2118`, `:2108-2110`, `:2217-2231`, `:2455-2457`, `:2683-2685`, `src/app/mouse.rs:94-97`, `:151-154`, `src/app/layout.rs:257-260`, `tests/explorer_stall.rs:30`, `:63-70` | Low | S | **NEW.** The "late reply changes nothing" tests cannot fail: after `running = None` the `Receiver` is dropped, so `drain_request()` is `false` whatever the code does, and the `sleep(20ms)` adds nothing. The double-click tests use two real `Instant::now()` calls less than 400 ms apart, with a directory listing between them. `explorer_stall` has about 3× headroom. Both will flake on a busy runner. There are no tests for F92, F93, F102, F104, F105 or F133. | Use a `Sender` double that records a failed send. Inject the click clock. Raise the stall budget to 3 s, or run it only in release. |
| F135 | Dependency & config debt | `.github/workflows/ci.yml:55-71`, `src/generate.rs:552-608`, `Cargo.toml:153-157` | Low | S | **NEW.** CI never compiles `--features foundation-models`, so the only code that uses the `fm-rs` API (pre-1.0, `"0.3"`) is never type-checked or linted by automation. CI also runs without `--locked`. | Add `cargo clippy --features foundation-models --all-targets -- -D warnings` on a runner with the macOS 26 SDK (see Open question 5). Add `--locked` to the build and test steps. |
| F136 | Documentation drift | `README.md:51-133`, `:170-176`, `:367`, `:2270-2279`, `:2304-2311` | Low | S | **NEW.** Features names none of: saved sets and profiles, the set picker, the filter editor, generation, the hex view, headless `--emit`, the configurable keymap. The install command drops `--features foundation-models` and `--locked`. "The settings so far are…" leaves out `background`, `warnings` and all of `[keymap]`. `:367` points at `src/viewport.rs` (now `src/app/viewport.rs:68`). "Four public additions" is wrong: PATCH.md lists 7. | One Features bullet for each, linked to its section. `cargo install --locked --path . [--features foundation-models]`. List every `FileConfig` field. Fix the path. Write "the changes in PATCH.md". |
| F137 | Documentation drift / Consistency rot | `docs/specs/2026-08-15-filter-based-viewing-design.md:3`, `docs/specs/2026-09-23-incremental-search-design.md:3`, `docs/specs/2026-09-24-listed-filter-sets-design.md:3`, `docs/specs/2026-09-05-keymap-reconciliation-design.md:231-252`, `CONTEXT.md` ("Check", "Request"), `src/app/filter_editor.rs:195`, `:291` | Low | S | **NEW.** Three merged specs still say `Status: proposed`. The README's "Design background" links to one of them. The keymap spec lists `v`, `y` and `-` as unbound and cites a deleted test. The glossary's term is **Check**, but the code says `Mark`/`marks` and `filtereditor.mark.*`, and CONTEXT's own **Request** entry says "every mark". | Set the Status lines to implemented, with the commit. Add a "superseded" note to the keymap spec. Add a **Line mark** entry to CONTEXT (the action) and keep **Check** (its meaning). |
| F138 | Test debt | `src/app/tests/filter_editor.rs:18-45`, `:319`, `src/app/tests/show_hide.rs:482`, `src/app/tests/hex.rs:15`, `src/app/tests/emit_on_quit.rs:137`, `:427-501`, `src/app/tests/mod.rs:232-238`, `:388`, `src/widgets/explorer.rs:1757-1765` | Low | S | **NEW.** Test helper hygiene. `filter_editor.rs` copies `draw`/`rendered`/`status_line` only to change the area. `app_with_two_filters` and `emitted` are each defined twice. The profile-picker tests live in `emit_on_quit.rs`. `mod.rs`'s docs cite `target/test-appdirs`, which is gone. `entries_after_parent_are_sorted` sorts with the code under test and checks that the result is unchanged, so it can never fail. The 3,168-line filter-editor test file is already sectioned by issue (#312–#322). | Add `rendered_in(app, area)`/`status_line_in`. Remove the duplicates. Move the picker tests. Compare the sort against an expected order written by hand. Split `filter_editor.rs` by section. |
| F139 | UX & CLI ergonomics | `src/filter/matcher.rs:207-209`, `:38`, `src/filter/mod.rs:256`, `src/syntax.rs:524` | Low | S | **NEW.** The 128-pattern scan limit counts the 11 `NEVER` slots of the built-in `definitions` set and every disabled filter. A user with 118 numbered filters sees "129 patterns, limit 128". | Report the count of user-written patterns, or add "(11 built-in)". Better: leave `Definition` predicates out of the scan's `RegexSet`. |
| F140 | Performance & resource hygiene | `src/filter/mod.rs:1114-1156`, `:774-780` | Low | S | **NEW.** `verdict` recomputes, for every line, values that do not change during one `evaluate`: `needs_regex()` walks every filter, the exclude and include walks, and `set.matches(line)` allocates a `SetMatches`. With 1M lines and 40 filters, that is about 120M `effective()` checks and 1M allocations per toggle. | Take one snapshot per `evaluate` (effective index lists plus `needs_regex`) and pass it into `verdict_with`. Use `matches_read_into` with a reused buffer. |
| F141 | Documentation drift | `src/widgets/explorer.rs:104`, `:871`, `:974`, `:1133`, `src/widgets/fileview.rs:717-719`, `src/app/viewport.rs:185-186`, `src/scan.rs:11`, `src/filter/mod.rs:575-587`, `src/filter/sets.rs:652-654`, `src/main.rs:336-343`, `src/app/render/mod.rs:203-205`, `src/app/actions.rs:316-338`, `:384-388` | Low | S | **NEW.** Comments that became false this cycle: "see `PARENT_STYLE`" (it does not exist); "the view renders `<directory>`"; "`App::run` redraws unconditionally at 60 Hz"; "cannot scroll past 65,535 lines"; "a few `u64` ops" (it is `u128`); `len`/`is_empty` "built-in included" (they are not, and `len` has no production caller); `main.rs` says nothing hides the cursor (ratatui does, every frame, and only `Terminal::drop` shows it again), and `render/mod.rs` says the opposite; `perform`'s `debug_assert` comments describe a Global→`explorer.*` rebind that `Keymap::rebind` cannot produce. | Rewrite each sentence. Make `ActiveFilters::len` `#[cfg(test)]`. Add an explicit `cursor::Show` in `restore_terminal`. |
| F142 | Data integrity & robustness | `src/scan.rs:245`, `:423`, `src/headless.rs:303-306` | Low | S | **NEW.** The scan worker trusts the file. `read_until(b'\n')` has no length cap, so a 10 GB file with no newline (a sparse image in `~/Downloads`) is read into one buffer. The worker opens without `refuse_unreadable`, so a file replaced by a FIFO after the listing blocks `File::open` for ever and leaks one `recon-scan` thread. The headless scan checks for this. | Cap each read (`take(1 MiB)`, then skip to the next newline). Call `refuse_unreadable` before `open`. |
| F143 | Consistency rot | `src/toml_fmt.rs:13-19`, `src/filtersets.rs:597-619` | Low | S | **NEW.** There are two hand-written TOML quoters. `toml_string` emits `'…'` for a value with a control character, which is invalid TOML. `literal_string` handles this. Latent today (only built-in inputs reach it). | Have `toml_string` fall back to `toml_edit::value(v).to_string()`, as `literal_string` does, or delete one of the two. |
| F144 | UX & CLI ergonomics / Consistency rot | `src/filter/sets.rs:377-386`, `:523-537` | Low | S | **NEW.** `d` on a file-set filter removes it for the session. `R` ("every set back to its startup state") does not bring it back, but unlist then list does. So two "back to the file" paths disagree. | Make `R` rebuild listed file sets from `as_loaded`, or document the difference in the README's `R` paragraph. |
| F145 | UX & CLI ergonomics | `src/widgets/fileview.rs:1470-1494` | Low | S | **NEW.** `SIZE_COLUMN = 6` ("`999.9K` fits"), but `format_size(1_048_575)` gives `1023.9K` (7 chars), so the time column of that row moves right. | Change unit at ≥1000, or make the column 7 wide. |
| F146 | UX & CLI ergonomics | `src/widgets/picker.rs:124-145` | Low | S | **NEW.** The profile picker never scrolls. With more profiles than rows, `j` selects rows that are not drawn. `SetPicker` handles this with `top`. | Give it a `ListMotion` (see F150). |
| F147 | Data integrity & robustness | `src/widgets/explorer.rs:118-120`, `:1076-1098` | Low | S | **NEW.** A dangling symlink keeps its `lstat` data. It is listed as `Plain`, with the byte length of the link text as its size (`23B`). It is sent to the scanner and to `--emit files`, which then fail on open. | Give an unresolved link `size: None`. Consider a `Kind::Broken`, drawn dimmed and not scanned. |
| F148 | Type & contract debt | `src/syntax.rs:245`, `:343`, `:372-452`, `:627-660`, `:752`, `src/lib.rs:32` | Low | S | **NEW.** `pub mod syntax` exports `Highlighter`, `Span`, `ensure`, `spans`, `definitions` and `KindSet` as plain `pub`, and nothing outside the crate uses them. This is the #166 blind spot that `lib.rs:8-12` warns about. | Make them `pub(crate)`. Keep `pub` only for `Theme`, `ThemeError`, `bundled_names`, `DEFAULT_THEME` and `Kind`. |
| F149 | UX & CLI ergonomics | `src/widgets/fileview.rs:431-443` | Low | S | **NEW.** `-` (hex) on a pane that shows an error message (a missing file, EACCES, a FIFO) "succeeds": it reads the file again, shows the same error, and adds `[hex]` to the title. | Refuse when `!self.text && !self.hex`, with "no file to show as hex". |
| F150 | Architectural decay | `src/widgets/explorer.rs:462-477`, `:533-552`, `src/widgets/setpicker.rs:126-159`, `:185-188`, `:220-225`, `src/widgets/picker.rs:64-70`, `src/app/viewport.rs:329-349`, `src/widgets/filterlist.rs:153-178` | Low | S | **NEW.** `ListMotion` (F77's fix) was adopted by the explorer and the filter list only. Both pickers move `selected ± 1` by hand, and `SetPicker` has its own `top`. "Walk from the selection, wrapping, to the first row that a predicate accepts" is written 5 times. `MATCH_STYLE` is copied. The filter list keeps 5 `pub(crate)` wrappers that nothing outside the module calls. | Add `wrap_find(len, from, backwards, pred)` to `listmotion.rs`. Give both pickers a `ListMotion`. Share `MATCH_STYLE` from `widgets/mod.rs`. Make the wrappers private. |
| F151 | Performance & resource hygiene | `src/app/selection.rs:55-86`, `:124-136`, `src/app/render/mod.rs:115` | Low | S | **NEW.** `visible_span` walks every segment of the selection, with a `chars().count()` for each line, to find the two ends. `V G` on a 1M-line file walks about 1M lines per frame. | Find the ends with `partition_point`, and count chars only on those two lines. |
| F152 | Error handling & observability | `src/clipboard.rs:9-12`, `:79-91`, `:104-114`, `src/editor.rs:110-118` | Low | S | **NEW.** `write_all(...)?` returns before `child.wait()`, which leaves a zombie and reports only "Broken pipe". stderr goes to null, so `xclip` with no display says only "exit status: 1". The OSC 52 hint does not say that stdout is null, so a script must open `/dev/tty`. `TemplateError` is shared, so a bad `--clipboard` template reports an "editor template" error. | Always call `wait()`. Pipe stderr and append its first line. Document `/dev/tty`. Make the error text neutral ("template has…"). |
| F153 | Type & contract debt / Architectural decay | `src/config.rs:185-215`, `:471-650`, `:1081-1135`, `src/main.rs:59-61`, `:80` | Low | M | **NEW.** `Config` is both the clap struct and a bag that is filled in two phases: `filter_sets`, `bindings` and `keymap_warnings` are `#[arg(skip)]`, and `main` fills them in an order it must remember. `keymap: Option<KeymapConfig>` stays on the struct after `build_keymap`, and nothing reads it. `config.rs` (2,773 lines) mixes CLI help prose, the TOML schema, about 180 lines of keymap-only serde, errors, XDG lookup and precedence. | Move `KeymapConfig` + serde + `build_keymap` to `keymap/config.rs`. Have one `startup()` return `{config, bindings, warnings, sets}`, so the order lives in one place. |
| F154 | Documentation drift | `src/help.rs` (9 sites), `src/keymap/mod.rs` (10), `src/keymap/check.rs` (3), `src/config.rs` (3) | Low | S | **NEW.** About 26 comments tell the story of their own revisions ("an earlier version of this comment…", "task 8 fix round 1"). In `check.rs`, comments are about 40% of the production lines, and the history hides the rule each comment exists to state. | Keep the current rule and its reason. Move the history to commit messages or an ADR. |

---

## Top 5 — if you fix nothing else, fix these

### 1. F89 (then F44): stop the peek from restoring flags by position

The bug fix is local. The structural fix removes the bug class.

```rust
// src/app/filters.rs — every mutation that inserts, removes or reorders
pub(super) fn add_filter(&mut self, pattern: &str) -> Result<(), regex::Error> {
    self.restore_peek_before_moving();          // new
    self.filters.add(pattern)?;
    self.refresh_view();
    Ok(())
}
// same one line at the top of add_excluding_filter, the FilterCommand::Delete
// arm, and save_scratch_as (adopt_scratch_as reorders)
```

Test: `Space`, `i foo Enter`, `Space`. Assert that the file set's filters have
their pre-peek flags and that `foo` is on.

Follow-up (F44):

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FilterId(u64);
pub struct Filter { id: FilterId, /* … */ }
pub struct EnabledFlags { by_id: HashMap<FilterId, bool> }   // was Vec<bool>
// remembered: Option<HashMap<FilterId, bool>>; profiles resolve name → id once
```

A restore then applies only to the ids it knows. Insert, remove and reorder
need no special handling, and F99's problem of profiles naming a filter by its
changing display name has an obvious fix.

### 2. F111 + F110: make the test harness hermetic

```rust
// src/app/tests/mod.rs — the one place every App test is built
fn harness(config: Config) -> App {
    let mut app = App::new(config);
    app.save_path = Some(fixtures::fixture_file("filters.toml", ""));
    app.clipboard = Box::new(RecordingClipboard::default());
    app.launcher  = Box::new(RecordingLauncher::default());
    app
}

// src/config.rs tests
fn parse_clean(args: &[&str]) -> Config {
    let cmd = Config::command().mut_args(|a| a.env(None::<&str>));
    Config::from_arg_matches(&cmd.try_get_matches_from(args).unwrap()).unwrap()
}
```

In `tests/headless.rs`, `recon()` does `Command::new(bin).env_clear()`, then
sets `PATH` and `HOME`. Delete the 11 hand-written `save_path` overrides.

### 3. F91: never let the filter editor discard work without saying so

1. When Enter commits a change to a filter whose `set != 0`, show
   `status: "web/foo changed in memory — S saves only the scratch set"`.
2. On `GlobalQuit`, if any file-set filter is different from its `as_loaded`
   copy, answer the first `q` with "unsaved changes to set web — q again to
   quit".
3. Later: `filtersets::replace_filter(path, set, name, &Filter)` through the
   same `toml_edit` + tmp/rename path as `append_set`. (Do F97 first, so that
   the second writer does not copy the symlink bug.)

### 4. F92 + F93: make the mouse in the file view do what it says

```rust
// src/app/mouse.rs — click_view
let Some(row) = self.view.line_at(line) else { return };
let Some(index) = self.document.source_at(self.view.window_start() + row) else { return };
if let Some(action) = self.explorer.open_listed(&dir, index) { … }

// src/widgets/fileview.rs — handle_events
Key::MouseScrollDown => self.scroll_view((3, 0)),
Key::MouseScrollUp   => self.scroll_view((-3, 0)),
```

Tests: a listing with an exclude filter, where a click on the third *visible*
row opens the third visible entry. A wheel event moves `scroll_top`. Decide
Open question 1 at the same time: if filters should not apply to listings, the
index mapping becomes the identity.

### 5. F114: parse key labels once, into a typed chord

This change closes #251, #255, #256, #257, #260 and #262 as a class, and it is
where F107's refusals belong.

```rust
// src/keymap/label.rs (moved from help.rs)
pub enum KeyName { Char(char), F(u8), Enter, Tab, BackTab, Esc, Home, End, PageUp, PageDown, /* … */ }
pub struct Chord { key: KeyName, mods: KeyModifiers }
pub fn parse(label: &str) -> Result<Vec<Chord>, BadKeyLabel>;  // ranges 0x21..=0x7E only;
                                                               // refuses Ctrl-G, Ctrl-i/m/[, Ctrl-Tab
// Keymap stores Vec<(Scope, Chord, ActionId)>; the label text survives only
// for --print-keymap. resolve() is a lookup; claims/evict/losses compare Chords.
```

After that, give each loss one structured line instead of a paragraph
(`j: explorer.down, view.down → global.quit`). That closes #252, #253 and #254.

---

## Quick wins

Low effort (S) × Medium or higher severity:

- [ ] F89: call `restore_peek_before_moving` in four mutators, plus one test
- [ ] F90: refuse an empty filter pattern
- [ ] F92: map a listing click through `source_at`
- [ ] F93: handle the wheel in the view (or correct three docs)
- [ ] F94: promote a preview before `--emit lines` quits
- [ ] F95: count or finish unscanned files in the hide-mode emit
- [ ] F96: `path: Option<String>`; read stdin only without PATH
- [ ] F97: canonicalize, copy permissions and fsync before the rename
- [ ] F98: pick the first free palette colour
- [ ] F99: rename profile members in `set_pattern`
- [ ] F100: sniff and decode UTF-16 in the scan worker
- [ ] F102: refuse a mark on a line with an embedded `\r`
- [ ] F104: `unicode-width` for badges and cursors
- [ ] F105: fix `parse_answer`, and reject a pattern that matches 0 lines
- [ ] F106: implicit printable claim for `Scope::Prompt`
- [ ] F107: refuse Ctrl chords a legacy terminal cannot send
- [ ] F108: `require_equals = true` on three flags
- [ ] F109: answer `--print-keymap defaults` before `config.toml`
- [ ] F110 + F111: a hermetic test harness
- [ ] F112: fix the worktree-length flake
- [ ] F113: correct the `EDITOR=vim` README claim, and add the hint
- [ ] F116: `FileView::perform(ActionId)`
- [ ] F119: colour the viewport first, and redraw when colour is pending
- [ ] F120: chain return by `ActionId`, not by `'n'`
- [ ] F121: no redraw on `MouseEventKind::Moved`
- [ ] F123, F124: IDIOM fixes
- [ ] F33: add the `code-review_*.md` ignore line (#99).
- [ ] Close #202 (F85 was a false positive, already covered by `883da68`). Check whether #261 is fixed by `check.rs`'s displaced warnings.

---

## Things that look bad but are actually fine

- **Six optional modals in `App` instead of one `Mode` enum**
  (`app/mod.rs`, `events.rs:70-183`). The order of the guards in
  `dispatch_event` *is* the precedence, and the set picker and its own `/`
  prompt can be open together, which an enum could not show. F117 extracts the
  other field groups and leaves these alone on purpose.
- **`NEVER` regex slots for built-in filters** (`filter/mod.rs:230-256`). They
  waste slots, but they keep `Verdict::Included(i)`, the scan bits and
  `filters[i]` on one index with no map. `[^\s\S]` compiles to a dead state
  and costs nothing per line. The only cost is the count shown in F139.
- **Unlisted sets still reserve a palette colour** (`filter/mod.rs:604-613`).
  This seems to contradict CONTEXT's "an unlisted set has no effect". It is
  deliberate: colours stay stable across list and unlist, and a typed filter
  never takes a hidden set's colour. F98's fix must keep this.
- **The hand-written `Deserialize` with `DeserializeSeed` for `[keymap]`**
  (`config.rs:471-650`). It looks overbuilt, but it is the only way to report
  a bad value with its TOML position and the action name. `#[serde(flatten)]`
  buffers the table and loses both.
- **A linear `Vec` scan in `Keymap::resolve`** (`keymap/mod.rs:419`). About 130
  rows per human keypress cannot be measured. The per-press *reparse* is the
  problem (F114), not the scan.
- **The help overlay rebuilds `Keymap::default()` each frame**
  (`help.rs:1019-1150`). It is wasteful (a few hundred small allocations), but
  only while the overlay is open, and it is below one millisecond. F114 removes
  it as a side effect. It does not justify its own finding.
- **`hex.rs:84`'s `expect("a dump line is ASCII")`** on a production path. The
  bytes come only from `DIGITS`, space, `:`, and `is_ascii_graphic()` or `.`,
  so it cannot fail. The offset-width maths and the FIFO refusal are correct.
- **Char-boundary slicing in the filter editor and prompt**
  (`filter_editor.rs:959`, render `:367`, `prompt.rs` `byte_at`). Every offset
  comes from `char_indices`, `.get()`, or an `is_char_boundary` filter. No path
  panics.
- **Log lines and model text with control or ANSI bytes.** ratatui-core 0.1.2
  removes control graphemes before it draws, so a hostile log cannot drive the
  terminal.
- **`$VISUAL`/`$EDITOR` rank below `config.toml`, but `RECON_EDITOR` ranks
  above it** (`editor.rs:348-352`). This is deliberate and documented, and it
  is correct. F113 is only about terminal editors on that rung.
- **`ConfigError` has no `source()`** (`config.rs:887-891`). Otherwise
  color-eyre prints the TOML snippet twice.
- **`listed = false` beats `autoload = true` without an error**
  (`sets.rs:295-299`). This is ADR 0002, implemented exactly. `--set X
  --unlist X` is refused.
- **Raw `u16` subtractions in `layout.rs:160,186`, `mouse.rs:60,64,181` and
  `setpicker.rs:240`.** Each one follows a `.min()`, a `contains` or a max over
  the same data.
- **`tests/render_smoke.rs` and `tests/explorer_stall.rs` use a bare
  `remove_dir_all`.** Integration crates cannot see `#[cfg(test)]
  fixtures.rs`, and each directory is unique to its test and in lowercase.
- **Only a macOS job in CI.** The cost and platform reasons are written in
  `ci.yml:12-23`. F135 asks only for a feature build on that same job.

---

## Open questions for the maintainer

1. **Should filters apply to the directory look-ahead listing at all?** Today an
   include or exclude filter colours, dims or hides filename rows as if they
   were log lines. That is the root of F92. An unfiltered document for
   non-text contents would make the fix trivial.
2. **F96:** is `stdin_wins_over_the_path_argument` a hard requirement, or can
   an explicit PATH turn off the stdin read?
3. **F94 / F95:** should `q` under `--emit` finish the work (a full read, a
   completed scan), or refuse and say that the output would be partial?
4. **F91:** is a write-back for one file-set filter in scope for 1.0, or is a
   warning enough?
5. **F135:** does `macos-latest` have an SDK that builds `fm-rs`
   (FoundationModels needs macOS 26 / Xcode 26)? If not, is a pinned runner
   worth the cost, or is the feature build knowingly not verified?
6. **F114:** is the keymap check's current strictness a 1.0 requirement? The
   typed-chord refactor could close #251, #255, #256, #257, #260 and #262 as
   one post-1.0 change, instead of six fixes to the string machinery.
7. **iTerm2 quoting (F39 follow-up):** `create window with default profile
   command` splits its string itself, not through `sh`. Does it honour the
   `'\''` that `quoted form of` emits? Try `iterm-nvim` on `it's.log`.
8. **Backspace as `^H`:** some terminals send 0x08 for Backspace. Then global
   `Ctrl-h` toggles hide, and in any prompt Backspace is dropped
   (`prompt.rs:457-460`). Should `Ctrl-h` also be a default for
   `prompt.delete.back`?
9. **Consolidation step:** Esc still *saves* there
   (`filter_editor.rs:1493-1495`, `:1913-1918`). Is that intended?
10. **Examples in `filters.toml`** are whole log lines, which can hold tokens,
    IPs or emails. ADR 0003 calls a set "one file to copy". Should `S` warn
    when examples are present?
11. **CONTEXT.md says "a set listed again comes back as its file describes
    it"**, but the code restores the *startup* snapshot, not the file as it is
    now. Is "as it was loaded" the intended wording?
12. **`RECON_WARNINGS=0`** refuses to start (clap's bool parser accepts only
    `true`/`false`). Is that intended?
13. **Stale issues:** #202 was never a gap (see F85). #8 ("nothing is persisted") and
    #261 look fixed. Close them?
