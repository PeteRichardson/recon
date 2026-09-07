# recon — Project Review

Generated 2026-09-06 against `4fb8a74`. Second run. The first run (2026-08-31,
`eeb3b1b`) minted F1–F33; the git-history scan finds commit messages citing up
to F29 and the live doc carrying up to F33, so new findings start at **F34**.
101 commits and ~16,000 added lines separate the two runs: saved filter sets,
syntax colouring, the navigator scan thread, and five keymap PRs all landed in
between.

**Status of the first run:** F1–F30 are all `RESOLVED` on `main` (verified in
code, not just by issue state — the `Vec<AppWidget>` is three named fields and a
`Focus` enum, `layout.rs`/`viewport.rs` exist, redraws are event-driven,
`RegexSet` is in use, `read_lines` is lossy with a NUL sniff, logging is real).
F31, F32 and F33 are carried over unchanged as issues #98, #51 and #99.

No `/code-review` reports exist yet, so Phase 1.5 had nothing to ingest.

---

## Executive summary

1. **`!` goes inert after one toggle.** `disable_all_remembering` refuses to run
   while a capture is pending; `App` calls it whenever anything is enabled. So
   `!`, then `Enter` on any row (or a profile, or `s`), then `!` — nothing,
   forever, until an add/remove/`S`/`R` drops the capture. The four `bang_*`
   tests cover add and remove but not a flag change. Reproduced. **F34.**
2. **Every file load clones the whole file into a `TextArea` that is thrown
   away unrendered.** `adopt` seeds the textarea with `source.clone()`; every
   caller replaces that buffer via `show_window` before the first draw. A 2 GB
   log is momentarily resident three times, and every navigator arrow pays a
   full copy. **F35.**
3. **The suite is red on this machine, deterministically.** Six of nine
   `render_smoke` tests and ~15 widget unit tests use the repo root as their
   fixture. `nav_pane_renders_directory_entries` now fails because the root has
   grown past the navigator's row count and `..` scrolls off. It passes in CI
   only because CI has fewer ignored directories. **F36, F47.**
4. **The one file recon writes can be destroyed by writing it.** `S` writes
   `filters.toml` in place with `fs::write` (truncate, then write), and
   `append_set` uses `Table::insert`, which silently *replaces* a set added by
   hand since startup — comment and all. **F37, F38.**
5. **The README's safety claim is false for two templates recon itself
   emits.** `terminal-nvim` and `iterm-nvim` hand a string to a second shell;
   a filename with `"` or `$(` breaks out of it. **F39.**
6. **A `stat` per listed file, on the UI thread, on every filter toggle** and
   again every two seconds. Fine at a few hundred files; a 20k-file directory or
   a network mount turns each keystroke into thousands of syscalls. **F40.**
7. **Fixture management has fragmented into three conventions** since #69: a
   registry in `lib.rs`, a second one in `fileview.rs`, and bare
   `remove_dir_all` in `filenav.rs` plus twelve `lib.rs` tests that bypass the
   registry. The APFS race is one new `O_*` fixture away from returning. **F48.**
8. **The keymap drift test has blind spots the README doesn't admit:** it does
   not scan `picker.rs`, and it only sees `Char(..)` arms, so `Home`, `End`,
   `PageUp`, `BackTab` and every mouse binding are outside its reach. **F46.**
9. **`filter.rs` is 3,702 lines doing four jobs** — palette, per-line verdicts,
   the set/solo/reset state machine, and the scan-side `Matcher` — with 72
   `pub` items and several doc comments now attached to the wrong function.
   **F44, F63.**
10. **The README's Development section describes the August codebase:** wrong
    test counts, a clippy suppression on a type that no longer exists, and a
    layout table missing nine of the eighteen modules. **F58, F59.**

No new categories were introduced; every finding fits the existing vocabulary.

---

## Architectural mental model

recon is a single-binary ratatui TUI for reading logs through a stack of regex
filters. The model layer is still the well-factored core it was in August, but
it has grown a second axis. `ActiveFilters` (`filter.rs`) now owns *sets*: an
always-present scratch set plus named sets loaded from `filters.toml`
(`filtersets.rs`) and one built-in `definitions` set whose filters are
syntect-scope predicates rather than regexes. A filter takes effect only when
both its own flag and its set's flag are on; solo, reset, `!` and profiles are
four independent snapshot-and-restore mechanisms over those flags. `Document`
(`document.rs`) holds every line and one `Verdict` per line, evaluated in a
single `RegexSet` pass, and derives the visible set from a `Mode`.

Around that, `App` (`lib.rs`) owns three named panes (`FileNav`, `FileView`,
`FilterList`), a `Focus`, and 33 fields of interaction state: divider drags,
the prompt, zoom, the peek, the cross-file `Crossing` notice, the chain origin
for `f i … Enter`, the profile picker, and the scan bookkeeping. `dispatch_event`
is a 520-line flat match over the global keys; pane keys live in three widget
handlers plus `viewport::long_range_target`. The README's "four layers" model
(prompt, global, chain, pane) is real and the code follows it, but it is spread
across six sites and only the help table is machine-checked against them.

Two subsystems are new since the last run and both are asynchronous. The
**scanner** (`scan.rs`) is a worker thread fed over mpsc: for each listed file
it records, per line, a 64-bit mask of which patterns hit, so toggling a filter
re-answers "which files match" from cache with no I/O. The 64-pattern ceiling
counts every loaded set, enabled or not. The **highlighter** (`syntax.rs`)
wraps syntect with two-face's grammar bundle, parses forward lazily, and resyncs
approximately after a jump; the same scope pass feeds the definition filters.
`FileView` still holds a three-screen window of the visible set with `App`
translating between source index, visible row and buffer row — that translation
is now in `viewport.rs` and reads much better than it did.

The README's architecture claims match reality except in the Development
section (F58). What I did not expect: `lib.rs`'s *production* half grew from
~2,400 to 2,651 lines despite the #73/#74 split, because the chain, crossing,
peek, picker, save and scan features each added state to `App` rather than to a
type of their own. That is the shape to watch. The comment discipline noted last
time has held, and several August findings are now recorded in the code as
"this used to be X, and here is why it was wrong" — which is exactly how a few
of this run's findings were found.

---

## Findings

Severity is maintenance impact except for `IDIOM`, which carries a Medium floor.
Category values are the Phase 2 vocabulary. F1–F30 are listed once, collapsed,
as `RESOLVED`; F31–F33 are carried over; everything from F34 is `NEW`.

| ID | Category | File:Line | Severity | Effort | Description | Recommendation |
|---|---|---|---|---|---|---|
| F1–F30 | — | — | RESOLVED | — | All thirty findings from the 2026-08-31 run are closed on `main` (issues #69–#97). Verified in code: named panes + `Focus` (F2), `layout.rs`/`viewport.rs` (F1), no `Widget for AppWidget` (F3), test-only methods `#[cfg(test)]` + `unreachable_pub` (F4), `match_count` gone (F5), `lexical_absolute` everywhere (F6), `OsString` names (F7), `PathBuf` filename (F8), `handle_events` unwrapped (F9), private widget fields (F10), case-insensitive fixture guard (F11), pedantic gated (F12), real logging with `RUST_LOG`/`RECON_LOG` (F13/F14), machete metadata (F15), `mem::take` not clone (F16), cached width (F17), dirty-flag draw (F18), `RegexSet` (F19), lossy read + NUL sniff (F20), `debug_assert` gone with the vec (F21), `try_from` (F22), field access (F23), pedantic clusters cleared (F24), `long_help` split from rationale (F25), README usage pinned by test (F26), stderr section rewritten (F27), keymap sources named (F28), dirs-first case-insensitive sort (F29), `unicode-width` (F30). | — |
| F31 | Performance & resource hygiene | `src/widgets/fileview.rs:670` | Low | S | Carried over (#98). `apply_pending_scroll` still renders into a scratch `Buffer` to prime the viewport, now gated to rebuilds only. | As #98. |
| F32 | Data integrity & robustness | `src/widgets/fileview.rs:377`, `src/document.rs` | Medium | L | Carried over (#51). Full loads are still uncapped and `Document` holds every line. F35 makes the transient peak worse than the README's "~1.5×". | Continue with #51; F35 is the cheap half. |
| F33 | Documentation drift | `.gitignore` | Low | S | Carried over (#99). `docs/reviews/` is still untracked; this file's IDs are cited in commit messages and issues. | As #99. |
| F34 | Correctness & memory safety / UX & CLI ergonomics | `src/filter.rs:1311`, `src/lib.rs:1099` | High | S | **NEW.** `disable_all_remembering` returns early whenever `remembered.is_some()`. `App` calls it whenever `any_enabled()`. Sequence: `!` (capture, all off) → `Enter` on a row / set header with `default` / `a` profile / `s` solo (re-enables flags, none of which call `forget_capture`) → `!` → early return, nothing happens — and keeps not happening until add/remove/`S`/`R` drops the capture. Confirmed: after `!`, toggle, `!`, `!`, `!` the flags stay `[true, false]`. The `bang_*` tests cover add and remove only. | Guard on the thing the comment actually fears: `if self.remembered.is_some() && !self.any_enabled() { return; }`. Add `toggling_a_filter_during_bang_does_not_leave_bang_inert`. |
| F35 | Performance & resource hygiene | `src/widgets/fileview.rs:377` | High | S | **NEW.** `adopt` does `TextArea::new(self.source.as_ref().clone())` — a deep copy of every line. Every production caller (`perform` → `load`/`preview` → `sync_document` → `refresh_view` → `apply_view`) then calls `show_window`, which replaces that buffer via `set_lines` before it is ever rendered, because `sync_document` clears `last_visible`/`last_window`. A full load is resident three times at peak; every navigator arrow copies the preview twice. | Seed with `vec![String::new()]` and let `show_window` populate; keep an `App`-level test that a fresh load renders. |
| F36 | Test debt | `tests/render_smoke.rs:120` (also `68,185,213,329,349,387`) | High | M | **NEW. Failing now, deterministically.** Six of nine tests use the repo root: `nav_pane_renders_directory_entries` asserts `..` is on screen while the cursor sits on `Cargo.toml` in a root that now has 14 entries above it and 13 navigator rows, so `..` has scrolled off. `renders_file_contents_into_buffer` needs `tui-textarea-2` in the first ~22 lines of `Cargo.toml` (it is there only because the machete comment mentions it); two tests need `Cargo.lock` with `[[package]]`; one needs `filenav.rs` longer than a page. The two tests at `238`/`281` already moved to a private fixture after this exact breakage (#82). The file also re-implements `divider_column`/`press`/`rendered` from `lib.rs`'s test module because integration tests cannot see `#[cfg(test)]` helpers. | One `fixture()` helper writing a known file set under `target/test-navdirs/render_smoke/`, used by all nine tests. The `..` failure disappears as a side effect. |
| F37 | Data integrity & robustness | `src/lib.rs:1662` | Medium | S | **NEW.** `save_scratch_as` writes `filters.toml` with `std::fs::write` — truncate, then write. A crash, `kill`, or full disk between the two leaves the user's hand-edited file empty or partial, and the next start refuses to run on it. The read at `1652` and the write at `1662` also bracket a window in which an external editor's save is overwritten. | Write to `filters.toml.tmp` beside it and `fs::rename` over the original; keep the `parse(&after)` check before the rename. |
| F38 | Data integrity & robustness | `src/filtersets.rs:342`, `src/lib.rs:1630` | Medium | S | **NEW.** `append_set` uses `Table::insert`, which *replaces* an existing `[sets.<name>]`; `save_scratch_as` checks the name only against in-memory sets. A set added to the file by hand after startup under that name is silently overwritten by `S`, together with the comment above its header. Confirmed: `# my file` + `[sets.bug] ORIGINAL` → `[sets.bug] NEW`, comment gone, `parse` passes. | Before the insert: `if sets.contains_key(name) { return Err("a set named {name:?} is already in {path}; it was added since recon started") }`. |
| F39 | Security hygiene / Documentation drift | `src/editor.rs:569-580`, `README.md:1122-1127` | Medium | S | **NEW.** The `terminal-nvim` and `iterm-nvim` flavours embed `{file}`/`{project}` inside `do script "cd \"{project}\" && nvim +{line} \"{file}\""` — a string the spawned terminal's shell re-parses. A filename containing `"`, a backtick or `$(` breaks out of the inner quoting and runs in the user's terminal. The README's Safety section says without qualification that a quote or `$` "cannot change how the command is split"; true of recon's own split, false for these two templates recon prints via `--print-editor-config`. | Add a `{file:sh}` placeholder that single-quotes with `'\''` escaping and use it in those two flavours; or scope the README claim and say so in the printed stanza's comment. |
| F40 | Performance & resource hygiene | `src/lib.rs:1747-1762`, `src/lib.rs:1892`, `src/scan.rs:47` | Medium | M | **NEW.** `refresh_scan` calls `scan::stamp` (a `stat`) for every listed file, synchronously on the event thread, whenever the masks change — every filter toggle, digit, `!`, `&`, `Esc`, directory change. `check_stamps` repeats it for every cached record every two seconds. The comment at `1865` says "a few hundred stat calls … is nothing", which holds for small local directories only; a 20k-file log directory or a network mount pays 20k syscalls per keystroke before the frame draws. | The worker already stats each file it reads (`scan.rs:174`). Have `refresh_scan` trust the cached stamp and let the worker report a changed one; or cap the synchronous pass and defer the rest. |
| F41 | Correctness & memory safety / UX & CLI ergonomics | `src/lib.rs:444`, `src/lib.rs:1887-1897` | Medium | S | **NEW.** `App::new` loads the CLI argument as typed (relative for `recon app.log`), while `nav.files()` yields `dir.join(name)` from a `lexical_absolute` dir. `check_stamps` compares `path == active` on those two spellings, so the ` changed on disk · r ` badge never fires for a file opened from the command line — the most common way to open one — until it is re-selected through the navigator. The view title also flips from `app.log` to `/abs/app.log` after the first navigation. | Absolutise once in `App::new` with `path::lexical_absolute` before `view.load` — the rule the other three sites share. |
| F42 | UX & CLI ergonomics | `src/lib.rs:1350-1362`, `src/lib.rs:2359` | Medium | S | **NEW.** A chain commit from the navigator (`f i pat Enter`) runs `refresh_scan(false)` and immediately dispatches a synthetic `n`. Every answer is still `Unknown` — the worker was just started — so `step_to_match` finds nothing and the status row says `no matching file`. That is false: the answer is "not scanned yet". A real `n` or `.` before the first scan lands says the same. | When any listed entry is `Unknown`, report `scanning…` or say nothing and let `drain_scan_results`'s redraw speak; reserve `no matching file` for a listing with no `Unknown`s. |
| F43 | Performance & resource hygiene | `src/viewport.rs:212-215` | Medium | S | **NEW.** The rebuild-skip key is the whole visible index vector: every `apply_view` (every arrow, every filter change) does an O(visible) slice compare, and every rebuild copies the entire `Vec<usize>` — 8 MB for a 1M-line unfiltered file. | Give `Document` a `generation: u64` bumped in `evaluate`/`recompute_visible` and compare `(generation, window)`. `sync_document` already clears the key, so semantics are unchanged. |
| F44 | Architectural decay | `src/filter.rs:1-1572` | Medium | M | **NEW.** One module, 72 `pub` items in production, four jobs: the palette (`Palette`, `DEFAULT_PALETTE`, `DIM_STYLE`); the per-line model (`ActiveFilters`, `Verdict`, `Predicate`, `verdict`); the set/solo/reset/adopt state machine (`FilterSet`, `Solo`, `Origin`, `with_sets`, `solo`, `reset`, `adopt_scratch_as`); and the scan-side `Matcher`/`Owner`/`MAX_PATTERNS`, consumed only by `scan.rs` and `refresh_scan`. It is also where three doc comments have drifted off their functions (F63). | File split along the existing `impl` blocks into `filter/{mod,sets,matcher}.rs`. No signature changes. |
| F45 | UX & CLI ergonomics / Documentation drift | `src/filter.rs:768,779`, `README.md:439` | Medium | S | **NEW.** A set with `autoload = true` and no `profiles.default` starts *enabled with every filter off*: an enabled header and nothing filtering. The spec sanctions "else off" (`saved-filter-sets-design.md:154`); the README says only "the set starts enabled", and its example carries a `default` so the trap never shows. | Either start such a set's filters on (the natural reading of `autoload`), or say in the README's `autoload` and `Reset` paragraphs that an autoload set with no `default` loads with its filters off. |
| F46 | Test debt / Documentation drift | `src/help.rs:586`, `src/help.rs:611`, `src/help.rs:57`, `README.md:271-290` | Medium | S | **NEW.** `every_bound_key_is_documented` scans five files; `src/widgets/picker.rs` binds `j`/`k` (`picker.rs:60,64`) and is not in `SOURCES`. `bound_chars` reads only `Char(..)` patterns and `Binding::codes` yields only characters, so `Home`/`End`/`PageUp`/`PageDown`/`BackTab`/`Delete`, F-keys and every mouse binding are outside the test's reach. The README presents the test as the thing that stops keys drifting. | Add `picker.rs` to `SOURCES`; extend the scan to `KeyCode::<Ident>` tokens and let `codes` return `Char(c) \| Named(&str)`. |
| F47 | Test debt | `src/widgets/filenav.rs:1419,1430,1473`, `src/widgets/fileview.rs:1716-1971` (15 sites) | Medium | S | **NEW.** Unit tests construct `FileNav::new("Cargo.toml")` / `FileView::new("Cargo.toml")` against the real repo root; `bare_filename_lists_current_directory` asserts `src` and `Cargo.toml` exist in cwd. Same class of dependence as F36, one level down. | Point them at a `target/test-navdirs` fixture like the rest of `filenav.rs`'s tests, or at `env!("CARGO_MANIFEST_DIR")`. |
| F48 | Test debt | `src/lib.rs:2719`, `src/widgets/fileview.rs:1569`, `src/widgets/filenav.rs:948-1743`, `src/lib.rs:3001,3527,3713,3740,3822,3848,3877,3902,3940,3968,3994,4213` | Medium | S | **NEW.** Three fixture conventions now coexist: `claim_fixture_dir` in `lib.rs` (the #69 guard), a second `FIXTURE_NAMES` mutex in `fileview.rs` that cannot see the first, and ~12 `filenav.rs` fixtures (`kinds_width`, `keys_first_is_dir`, `keys_empty`, `outer`, …) doing bare `remove_dir_all`/`create_dir_all` with no guard at all. Twelve `lib.rs` tests also build `target/test-appdirs/<name>` by hand and never claim. A new `O_*`-spelled fixture in any of those races exactly as #69 did, with no assertion to name it. | One shared `#[cfg(test)]` `fixture_dir(name) -> PathBuf` that claims (case-insensitively) and creates; make `claim_fixture_dir` private to it and use it from all three files. |
| F49 | Data integrity & robustness | `src/widgets/fileview.rs:839-849` | Medium | S | **NEW.** `sniff_binary` declares a file binary on any NUL in the first 8 KiB. UTF-16 text (Windows logs, `.plist` exports, some CSV) is ~50 % NULs and is always rejected as `<binary file: contains NUL bytes>` — a false description of a text file. The README documents the NUL rule, so the *rule* is a decision; the message is not. | Check for a `FF FE`/`FE FF` BOM in the sniffed head and either decode with `String::from_utf16_lossy` or say `<UTF-16 file: not supported>`. |
| F50 | Type & contract debt | `src/widgets/fileview.rs:354-591` (24 methods), `src/widgets/filenav.rs:369,450,524` | Medium | S | **NEW.** Two dozen `FileView` methods and four on `FileNav` are plain `pub` where their neighbours are `pub(crate)`. `set_line_styles`, `show_window`, `window_start`, `scroll_cursor_to_row`, `set_cursor_row` are the invariant-carrying calls the `source()` doc says must only be made from `App::apply_view`. `pub` on a lib crate means `unreachable_pub` never fires — the trap #76 closed for six other methods. | `pub` → `pub(crate)` across both files; the lint then does the auditing. |
| F51 | Architectural decay | `src/widgets/fileview.rs:326`, `src/filter.rs:677`, `src/filter.rs:567`, `src/document.rs:61` | Medium | S | **NEW.** Four `pub` constructors/methods with no production caller: `FileView::new(String)` (App uses `default()` + `load`), `ActiveFilters::add_definition` (whose doc still says "nothing user-facing creates one yet — that is #127's built-in set"; #127 shipped), `with_palette`, `Document::new`. `#![warn(unreachable_pub)]` cannot see them because they are `pub` in a `pub mod`. | `#[cfg(test)]` all four (the #76 pattern); delete the stale sentence. |
| F52 | Performance & resource hygiene | `src/widgets/fileview.rs:1226-1249` | Medium | M | **NEW.** `apply_syntax` runs on every frame: clears custom highlights, runs `pattern.find_iter(line)` over every window row (≈120–200 lines) when a search is set, allocates a `Vec` of matches per row plus a `pushes` vec. Inputs (window, cursor row, styles, pattern, highlighter progress) do not change between two idle frames. Event-driven draws (#85) cap the damage, but a held `j` with syntax on and a search active does regex work over ~150 rows per keystroke. | Cache `(window_start, window_end, cursor_row, pattern ptr, styles len)` and skip when unchanged. |
| F53 | Idiom debt / Type & contract debt | `src/widgets/filterlist.rs:120-122` | Medium (IDIOM floor; maintenance Low) | S | **NEW.** `FilterList` still exposes `pub state: ListState` and `pub active: bool`; `lib.rs` writes both directly. #81 closed `FileView` and `FileNav` on exactly this argument, with `set_active` as the one writer. | Private fields plus `set_active`/`select`, matching the other two panes. |
| F54 | Idiom debt / Performance & resource hygiene | `src/lib.rs:1859`, `src/lib.rs:1884` | Medium (IDIOM floor; maintenance Low) | S | **NEW.** `poll_stamps` calls `self.filters.matcher()` sixty times a second purely to test `is_none()`; `check_stamps` calls it again on entry. `matcher()` walks every filter, builds masks and clones the `RegexSet` to answer yes/no. | `ActiveFilters::is_scanning(&self) -> bool` (the `selects != 0` test without constructing the `Matcher`). |
| F55 | Idiom debt | `src/scan.rs:102` | Medium (IDIOM floor; maintenance Low) | S | **NEW.** `Request.files: Vec<(usize, PathBuf, Progress)>` is an anonymous triple crossing a thread boundary; `worker` destructures it positionally, while `Scanned` already names the same fields. | A `FileToScan { index, path, progress }` struct. |
| F56 | Idiom debt | `src/widgets/filenav.rs:831-837` | Medium (IDIOM floor; maintenance Low) | S | **NEW.** `sort_key` returns `(bool, String, OsString)`: a lowercase `String` *and* a cloned `OsString` per entry, the third being only a tiebreak. 10k small allocations once per `set_dir`. | `sort_by_cached_key(\|e\| (e.kind != Dir, lowercase))` then a stable `sort_by` on `name`; or keep it and say so. |
| F57 | Idiom debt | `src/widgets/fileview.rs:1147` | Medium (IDIOM floor; maintenance Low) | S | **NEW.** `digits`: `n.ilog10() as u8 + 1` — the one `as` narrowing in the module without a comment, in a file that otherwise makes a point of `u16::try_from(..).unwrap_or(..)` with a rationale. Safe (max 19). | `u8::try_from(n.ilog10() + 1).unwrap_or(u8::MAX)` and a one-line note. |
| F58 | Documentation drift | `README.md:1332-1346`, `README.md:1356-1358`, `README.md:1370-1385`, `.github/workflows/ci.yml:69` | Low | S | **NEW.** The Development section says `cargo test` runs "713 unit + 13 integration tests" (actual: 920 + 11); the CI comment says "224 unit + 9". "The one standing suppression is `clippy::large_enum_variant` on `AppWidget`" — `AppWidget` no longer exists and `Cargo.toml` now has a nine-entry `[lints.clippy]` allow-list. The layout table lists nine paths and omits `config.rs`, `editor.rs`, `filtersets.rs`, `help.rs`, `layout.rs`, `path.rs`, `scan.rs`, `syntax.rs`, `viewport.rs`, `widgets/picker.rs`, `tests/logging.rs`, `tests/scan_thread.rs`. | Drop the counts (they drift every PR) or say "see `cargo test`"; point the suppression sentence at `Cargo.toml`'s `[lints]` block; regenerate the table from `src/`. |
| F59 | Documentation drift | `README.md:1251-1253` | Low | S | **NEW.** Known Limitations: "**Nothing is persisted.** Filter sets live only for the session — there is no way to save or reload a filter set… This is github issue #8" — 850 lines after a section documenting `S`, `filters.toml`, `autoload` and profiles. #8 is still open on GitHub too. | Delete the bullet; close or re-scope #8. |
| F60 | Documentation drift | `README.md:266`, `src/editor.rs:520-526` | Low | S | **NEW.** The Logging table's `warn` row lists two sources; three more exist: the scan worker disconnecting (`lib.rs:1813`), a resume-seek or read failure mid-scan (`scan.rs:190,204,316`), a highlight failing to apply (`viewport.rs:246`). In `editor.rs` the comment says a bad exit is logged "hence `debug!` rather than `warn!`" and the code two lines down is `log::warn!`; the `debug!` it describes is the send-failure branch at `531`. | Add the scan row; move the `debug!` sentence to `531`. |
| F61 | Documentation drift | `docs/specs/2026-09-03-saved-filter-sets-design.md:98-99` | Low | S | **NEW.** "Nothing in recon addresses a filter by its number — there are no digit bindings" was overtaken by keymap PR 2 (`1`–`9` toggle by pane number). Commit `7fde2a6` reconciled §14 and this sentence survived. | "Digits address the *pane's* number, which is why it is a label recomputed on every change." |
| F62 | Documentation drift / Correctness & memory safety | `src/lib.rs:975-978`, `src/lib.rs:1160-1163`, `src/lib.rs:951` | Low | S | **NEW.** Both comments say crossterm "reports the Shift" for `?` and `*` and that an `is_empty()` guard "would make the key unreachable". In legacy key mode crossterm attaches SHIFT only when `c.is_uppercase()` (`crossterm-0.29/src/event/sys/unix/parse.rs:131`); shifted punctuation arrives with no modifiers. The tolerant guard is harmless, but the stated reason is wrong and hides a real dependency: the `is_empty()` guards on `!`, `&`, `[`, `]`, `.`, `,`, `/`, `{`, `}` and `1`–`9` all rely on that legacy rule, and pushing kitty keyboard-enhancement flags would kill every one at once. (Modifier survey: #146's `S` is the only uppercase key with the trap; `O`/`H`/`N` use the CONTROL\|ALT exclusion.) | Correct the two comments; record the legacy-mode dependency once beside `q`'s guard. |
| F63 | Documentation drift / Consistency rot | `src/filter.rs:670-677`, `src/filter.rs:1115-1116`, `src/widgets/fileview.rs:1030-1035`, `src/widgets/fileview.rs:3-5`, `src/widgets/filenav.rs:1-2`, `src/widgets/fileview.rs:1149-1150` | Low | S | **NEW.** Doc comments attached to the wrong item, six sites: `set_search`'s doc sits on `add_definition` (and `set_search` at `1027` has none); `any_excluding`'s one-liner sits on `toggle_and`; `directory_listing`'s paragraph sits on `const NAME_COLUMN_MAX` (the fn at `1087` is undocumented); two files open with `///` on a `use` item instead of `//!`; "Widget impl for `FileView`" sits above the `#[cfg(test)]` impl, 140 lines from the real one. | Move each block to its function; `//!` for the two module heads. |
| F64 | Documentation drift | `src/document.rs:126`, `src/document.rs:155`, `src/filter.rs:1077`, `src/widgets/fileview.rs:941-953`, `src/lib.rs:1370-1374` | Low | S | **NEW.** Statements contradicted by adjacent code: `recompute_visible` says `evaluate` "is O(lines × filters)" (one `RegexSet` pass since #86); `line_styles` is "for `FileView::set_line_styles`" while `#[cfg(test)]`; `row_count` says `len` "is what `Verdict::Included` indexes into" (since #127 `Included(i)` indexes `filters()` incl. built-ins while `len()` counts user filters — the trap `lib.rs:8576` has to comment around); `read_preview_with_caps` reads "at most a screenful" (50k lines / 10 MiB); `Focus::Filters => None` is "Unreachable" but every mouse event reaches it. | Fix each sentence; rename `len`/`is_empty` to `numbered_count`/`has_numbered`. |
| F65 | Data integrity & robustness | `src/widgets/filenav.rs:872-876` | Low | S | **NEW.** `describe` classifies via `entry.file_type()` (does not follow symlinks) and stats via `entry.metadata()` (lstat). A symlink to a directory is `Kind::Plain`: drawn as a file, no `/`, sorted among files, and `files()` hands it to the scanner, which opens a directory. `activate_selection` re-checks `path.is_dir()` and descends, so the pane and the key disagree. | Fall back to `fs::metadata(path).is_dir()` when `file_type().is_symlink()`; keep lstat for the executable bit. |
| F66 | Data integrity & robustness | `src/editor.rs:228-229` | Low | S | **NEW.** `substitute` renders `project`/`file` with `.display().to_string()` and argv is `Vec<String>` end to end. After #71 the navigator can *reach* a non-UTF-8 filename, but `o`/`O` hand the editor a U+FFFD path that does not exist. | Argv as `Vec<OsString>`, pushing `OsStr` slices for the two path placeholders; `Launcher::spawn` takes `&[OsString]`. |
| F67 | Data integrity & robustness | `src/syntax.rs:426` | Low | S | **NEW.** `ensure` writes `spans[start..=row]` unconditionally on a resync. Lines 0–1000 coloured exactly, jump to 5000, scroll back to 1001: the resync starts at 937 and overwrites already-correct spans 937–1000 with results from a fresh parser state — exact colouring degraded to approximate. | In the resync branch, skip slots that are already `Some`, still advancing the parser over them. |
| F68 | Performance & resource hygiene | `src/syntax.rs:695` | Low | S | **NEW.** `KindScopes::note` calls `open.build_string()` (heap allocation) for each open scope, innermost first, for every token on every line while the `Function` scope is present. `definitions` is a whole-file pass, so a 50k-line source allocates tens of thousands of short strings to test a `meta.` prefix. | Pre-parse `Scope::new("meta")` in `KindScopes` and use `is_prefix_of`, as `storage_type`/`meta_block` already do. |
| F69 | Performance & resource hygiene | `src/document.rs:108` | Low | S | **NEW.** `evaluate` allocates a fresh `Vec<Verdict>` (16 B/line) on every call rather than overwriting in place; a 1M-line file pays a 16 MB alloc + free per toggle. | `self.verdicts.clear(); self.verdicts.extend(..)`. |
| F70 | Performance & resource hygiene | `src/lib.rs:1717-1732`, `src/lib.rs:448` | Low | S | **NEW.** `refresh_scan` runs after every event and, even when nothing changed, allocates `pattern_key()` (a `Vec<String>` of every pattern) and `dir.to_path_buf()` to compare against `last_scan` — per mouse-move on terminals that report motion. Separately, `App::new` loads the file (which runs `rebuild_highlighter` in `adopt`) and then `set_theme` runs it again, so the startup file pays two grammar detections. | A generation counter on `ActiveFilters` compared first; build the key only on change. Set the theme before the load. |
| F71 | Error handling & observability | `src/filter.rs:1462`, `src/filter.rs:1450` | Low | S | **NEW.** `recompile` swallows `RegexSet::new`'s error with `.ok()`; the consequences are silent and two-fold: `verdict` falls back to per-filter scanning, and `matcher()` returns `None`, so the navigator's marking switches off with no log line. `in_step` false is described as "a bug rather than a state to support" but is handled silently by the slow path. | `log::warn!` on the `Err` arm; `debug_assert!(self.in_step(set))` at the top of `verdict` and `matcher`. |
| F72 | Error handling & observability | `src/scan.rs:151`, `src/lib.rs:1813` | Low | S | **NEW.** `std::thread::spawn` panics if the OS refuses a thread (and takes the TUI with it). Because `Scanner` keeps a `Sender`, a *panicking* worker never disconnects the channel, so `drain_scan_results`'s `Disconnected` warning is unreachable in practice — files simply stay `Unknown`. | `thread::Builder::new().name("recon-scan")`, log on `Err`; wrap the per-file body in `catch_unwind` and send `eof: true` for a file that panicked. |
| F73 | Error handling & observability | `src/widgets/fileview.rs:960-967` | Low | S | **NEW.** `read_preview_with_caps` and `directory_listing` return `Contents::message(format!("<{err}>"))` on failure with no `log::warn!`, while `read_lines` twenty lines up warns at every failure point (#83). The preview is the navigator-arrow case where the title is elided and the path is lost. | Mirror `read_lines`'s three `warn!` sites. |
| F74 | UX & CLI ergonomics | `src/filtersets.rs:224` via `src/lib.rs:1658` | Low | S | **NEW.** `S` with two scratch filters sharing a pattern (`i foo` then `x foo`) is refused with "two filters named "foo"; give one a distinct `name`" — an instruction about a file key the user cannot set from the UI, for a file that was not written. | Check duplicate patterns in `save_scratch_as`: "two scratch filters share the pattern "foo"; delete one before saving". |
| F75 | UX & CLI ergonomics | `src/main.rs:31` | Low | S | **NEW.** `filtersets::load_file()?` runs before the `--print-editor-config` early return at `38`, so a malformed `filters.toml` blocks a command that only prints an `[editor]` stanza. | Move the `print_editor_config` branch above line 31. |
| F76 | UX & CLI ergonomics / Consistency rot | `src/widgets/picker.rs:56-73`, `src/widgets/picker.rs:79-84`, `src/widgets/filterlist.rs:451-453` | Low | S | **NEW.** The picker is the one modal that missed two repo-wide rules: `handle_key` ignores modifiers entirely (`Ctrl-j`/`Alt-k` move, `Ctrl-c` is swallowed) after #120's "no silent keys"; its width and `EMPTY_HINTS`' fit use `chars().count()` after #97 moved every other width to `UnicodeWidthStr`. | Drop modified keys before the match as `filterlist.rs:207-217` does; `UnicodeWidthStr::width` at both sites. |
| F77 | Consistency rot | `src/widgets/filenav.rs:24`, `src/widgets/filterlist.rs:23` | Low | S | **NEW.** `ASSUMED_PAGE = 20` and `page_rows()` are duplicated verbatim (the second says "see the same constant in `filenav.rs`"), and `move_by`/`select_first`/`select_last`/`clamp` are near-identical between the two panes. | A small `ListMotion` helper over `ListState` shared by both. |
| F78 | Architectural decay | `src/widgets/fileview.rs:1030-1130`, `src/widgets/filenav.rs:805` | Low | M | **NEW.** `directory_listing`, `listing_row`, `format_size`, `format_modified`, `NAME_COLUMN_MAX`, `SIZE_COLUMN` are a directory renderer living in the *file* view and importing `filenav::Entry`, while `sorted_entries` is `pub(crate)` "split out for the file view". Two modules each own half of "how a directory is described". | A `widgets/listing.rs` owning `Entry`, `Kind`, `describe`, `sorted_entries` and the row formatters. |
| F79 | Consistency rot | `src/widgets/fileview.rs:6`, `src/widgets/filenav.rs:6` | Low | S | **NEW.** Both files `use color_eyre::Result;` and never use it — every `Result` in them is spelled `Result<_, regex::Error>` or `std::io::Result`. It compiles because the alias shadows the prelude's with the same shape; a reader assumes eyre reports are possible. | Delete both imports. |
| F80 | Dependency & config debt | `.github/workflows/ci.yml:69`, `src/editor.rs:1283` | Low | S | **NEW.** CI never runs `cargo audit` (the present advisory — `bincode` unmaintained via `syntect/dump-load`, RUSTSEC-2025-0141 — was found only by hand) and never runs the one `#[ignore]`d test, so the real-process spawn path has no automated execution anywhere. | `cargo audit` with an `ignore` for RUSTSEC-2025-0141 and a comment; `cargo test -p recon -- --ignored` on the macOS runner. |
| F81 | Dependency & config debt | `Cargo.toml:1`, `README.md:120` | Low | S | **NEW.** `recon` declares no `rust-version`; the 1.88 floor the README documents lives only in the vendored fork's manifest. On an older toolchain `cargo install --path .` fails inside `vendor/tui-textarea-2` with a message about a *dependency*. | `rust-version = "1.88"` on the root package with a one-line comment that it mirrors the fork's. |
| F82 | Consistency rot / Documentation drift | `Cargo.toml:47-50`, `src/lib.rs:879-1399` | Low | M | **NEW.** `too_many_lines = "allow"` is justified as keeping "the one place you can currently read the whole keymap". The keymap now lives in six places — `dispatch_event` (520 lines), `handle_filter_key`, `long_range_target`, and three pane handlers — with `n`, `[`, `]`, `g`, `G`, `{`, `}` each bound in two of them. Not a call to split; the rationale no longer holds. | Update the `[lints]` comment; if the arms keep growing, group them by the layer the README already names (prompt / global / chain / pane). |
| F83 | Documentation drift | `src/filtersets.rs:100,216,222` | Low | S | **NEW.** `Error::Invalid.filter` is documented as "by its name, or its pattern", but a bad regex or colour always reports `entry.pattern` even when the entry has a `name`. | `entry.name.as_deref().unwrap_or(&entry.pattern)`. |
| F84 | Test debt | `tests/scan_thread.rs:81`, `src/scan.rs:241-337` | Low | S | **NEW.** The handoff test says in its own doc that it does not prove a second `start` cancels the first worker; the unit tests check a flag set *before* `scan` began. Nothing shows the per-line cancel check fires mid-file. | A `BufRead` double whose `read_until` blocks on a channel until the test flips the flag; assert `eof: false` and `scanned_to` at the line boundary. |
| F85 | Test debt | `src/config.rs:609`, `src/config.rs:148` | Low | S | **NEW.** `readme_usage_block_matches_the_real_help` is sound but covers only `-h`; the `--theme` long help — the one place the bundled theme names are listed and the text the README tells users to run — has no test that the bundle still yields names. | One assertion that the rendered long help contains `Bundled themes:` followed by at least `ansi` and `Dracula`. |
| F86 | Correctness & memory safety | `src/widgets/picker.rs:91-92` | Low | S | **NEW.** `area.width - width` and `area.height - height` are unchecked `u16` subtractions, safe today only because both are `.min(..)`'d two lines up; a future sizing edit reintroduces a panic on the render path. | `saturating_sub`, matching every other subtraction in `src/widgets/`. |
| F87 | Performance & resource hygiene | `src/widgets/filterlist.rs:395-411`, `src/widgets/filterlist.rs:443-458` | Low | S | **NEW.** Each frame `render` calls `texts` → `numbered` → `rows`, then `rows` again inside `texts` — two full row walks and a `String` per row per frame — and `App::nav_width` calls `preferred_width`, which calls `texts` a third time. Rows change only on filter mutation. Small lists in practice; the identical shape was #84 in `filenav.rs`. | Compute the numbering in the same walk (it already has `next`), or cache `texts` and invalidate from `refresh_view`. |
| F88 | Documentation drift | `src/main.rs:154` | Low | S | **NEW.** `// terminal.show_cursor()?;` — a commented-out call in `restore_terminal` with no rationale, in a file where every other decision carries one. `LeaveAlternateScreen` does not restore a hidden cursor. | Delete it, or state whether re-showing the cursor on exit is intended. |

---

## Top 5 — if you fix nothing else, fix these

### 1. F34 — `!` must not go inert (one line, one test)

The guard protects against capturing an all-disabled state. That cannot happen
when something is enabled, so say exactly that:

```rust
// src/filter.rs:1311
 pub fn disable_all_remembering(&mut self) {
-    if self.remembered.is_some() {
+    // A second `!` before a restore would capture the all-disabled flags it
+    // just cleared and lose the real ones. But a capture taken *before* the
+    // user re-enabled something by hand is stale, and refusing to replace it
+    // is what made `!` inert after `!`, `Enter`, `!` (F34).
+    if self.remembered.is_some() && !self.any_enabled() {
         return;
     }
```

Then the missing test, next to `bang_re_enables_when_everything_was_disabled_by_hand`:

```rust
#[test]
fn toggling_a_filter_during_bang_does_not_leave_bang_inert() {
    let mut app = app_with_two_filters("bang_toggle_inert");
    key(&mut app, KeyCode::Char('!'));          // capture, all off
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Enter);              // row 1 back on by hand
    key(&mut app, KeyCode::Char('!'));          // must disable again
    assert!(!app.filters.any_enabled());
    key(&mut app, KeyCode::Char('!'));          // and restore
    assert!(app.filters.any_enabled());
}
```

### 2. F35 — stop cloning the file into a buffer nobody renders

```rust
// src/widgets/fileview.rs:374
 fn adopt(&mut self, lines: Vec<String>, text: bool) {
     self.source = Arc::new(lines);
     self.text = text;
-    self.textarea = TextArea::new(self.source.as_ref().clone());
+    // Empty on purpose: every caller reaches `show_window` before the first
+    // draw, which replaces this buffer with the window. Seeding it with the
+    // whole file made a full load resident three times at peak (F35).
+    self.textarea = TextArea::new(vec![String::new()]);
     self.window_start = 0;
     self.rebuild_highlighter();
 }
```

Check that `FileView::new` (test-only after F51) and any test that renders a
`FileView` without going through `App` still call `show_window` first; run
`cargo test` and the three widget tests that assert on `textarea.lines()`
directly will tell you which ones need it.

### 3. F37 + F38 — write `filters.toml` like a file you care about

```rust
// src/filtersets.rs:342 — refuse, don't replace
 let sets = doc["sets"].or_insert(table()).as_table_mut()…;
+if sets.contains_key(set.name) {
+    return Err(format!(
+        "a set named {:?} is already in the file; it was added since recon started",
+        set.name
+    ));
+}
 sets.insert(set.name, …);
```

```rust
// src/lib.rs:1662 — write beside, then rename over
-std::fs::write(&path, after)
-    .map_err(|err| format!("could not write {}: {err}", path.display()))?;
+let tmp = path.with_extension("toml.tmp");
+std::fs::write(&tmp, after)
+    .map_err(|err| format!("could not write {}: {err}", tmp.display()))?;
+std::fs::rename(&tmp, &path)
+    .map_err(|err| format!("could not replace {}: {err}", path.display()))?;
```

`rename` within one directory is atomic on APFS and every Linux filesystem
recon is likely to meet. Add a test that an on-disk `[sets.bug]` added after
`App::new` makes `S bug` refuse.

### 4. F36 + F47 + F48 — one fixture helper, three files, zero cwd dependence

Create a `#[cfg(test)] pub mod testing` in `lib.rs` (or a `tests/common/`
module for the integration side) with one function:

```rust
pub fn fixture_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    claim(name);                       // case-insensitive registry, the #69 guard
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures").join(name);
    let _ = fs::remove_dir_all(&dir);
    for (rel, body) in files {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    dir
}
```

Then: `render_smoke.rs`'s nine tests over `fixture_dir("render_smoke", &[("Cargo.toml", "…tui-textarea-2…"), ("Cargo.lock", "[[package]]…"), ("src/long.rs", &numbered_lines(200))])`;
`fileview.rs`'s `FIXTURE_NAMES` and `filenav.rs`'s bare `remove_dir_all` sites
replaced with the same call; the twelve hand-built `test-appdirs` in `lib.rs`
routed through it. The red test goes green without touching its assertions.

### 5. F40 — get the `stat` storm off the event thread

The worker already stats each file it reads (`scan.rs:174`, into
`Scanned.stamp`). So:

- `refresh_scan` stops calling `scan::stamp` per file. It builds the request
  from the cache as today, but hands each cached record's *stored* stamp to the
  worker alongside `Progress`.
- The worker compares the stored stamp to a fresh `stat` before deciding
  whether to resume or restart, and reports the new stamp.
- `check_stamps` becomes a request to the worker (`Request { files, stamp_only: true }`)
  rather than a loop of syscalls in `poll_stamps`.

The UI thread then does zero filesystem work per keystroke, and the two-second
tick costs one channel send. `F54` (`is_scanning`) falls out of the same edit.

---

## Quick wins

Low effort, Medium or higher severity. Roughly in order of value per minute.

- [ ] **F34** — `&& !self.any_enabled()` on the `!` guard, plus the test. Live bug.
- [ ] **F35** — seed the textarea empty in `adopt`. Cuts peak memory by a third.
- [ ] **F37** — tmp + `rename` for `filters.toml`.
- [ ] **F38** — `contains_key` before `Table::insert`.
- [ ] **F41** — `lexical_absolute` on the CLI argument in `App::new`.
- [ ] **F46** — add `picker.rs` to `SOURCES`.
- [ ] **F51** — `#[cfg(test)]` on the four test-only `pub` fns.
- [ ] **F50** — `pub` → `pub(crate)` sweep over `fileview.rs`/`filenav.rs`.
- [ ] **F43** — `Document::generation` instead of the visible-vec key.
- [ ] **F45** — one README sentence on `autoload` without `default`.
- [ ] **F49** — say `UTF-16` when the NULs come with a BOM.
- [ ] **F54** — `is_scanning()` instead of building a `Matcher` 60×/s.
- [ ] **F53** — close `FilterList`'s two `pub` fields.
- [ ] **F55** — `FileToScan` struct.
- [ ] **F39** — scope the README Safety claim (the placeholder is S too).
- [ ] **F58 / F59** — the Development section and the "Nothing is persisted" bullet.

---

## Things that look bad but are actually fine

- **`handle_events` still waking sixty times a second** (`lib.rs:847-864`).
  Reads like #85's idle-CPU problem returning. It is a `poll` syscall with a
  timeout and no render; the draw is gated on `dirty`. The tick is what drains
  editor exits and scan results, which have no keypress behind them.

- **`return_to_chain_origin` re-entering `dispatch_event` with a synthetic key**
  (`lib.rs:2352`). Recursion from inside a key handler looks like a re-entrancy
  hazard. The prompt is `None` by then so the synthetic `n` cannot open one, the
  bounce guard is deliberately spent and re-armed, and it is what keeps "as if
  you pressed `n`" literally true per pane.

- **`sync_document` cloning `self.view.source()`** (`lib.rs:1952`). Reads as
  the "resident twice" cost the README says was removed. `source()` is an
  `&Arc<Vec<String>>`; this is a refcount bump. F35 is the real copy.

- **`Predicate::Definition` compiled as `\b\B` in the `RegexSet`**
  (`filter.rs:157`). Eleven never-matching slots look like waste; they keep set
  indices equal to filter indices so no mapping exists to drift.

- **`Matcher` covers disabled patterns too** (`filter.rs:378`). Looks like
  wasted matching. It is what makes a line's bitset reusable across every
  toggle without I/O — the entire point of the scan cache.

- **Two positional snapshots with two repair strategies** — `remembered`
  (aligned to `filters`, dropped on shape change) and `Solo.snapshot` (aligned
  to `sets`, patched by `adopt_scratch_as`). They differ because sets can only
  be added while filters can be removed; each is the cheapest correct strategy
  for its shape. F34 is a bug in the *guard*, not in this design.

- **`Record.stamp` `None == None`** (`scan.rs:57`). Two unreadable files
  comparing equal looks like a bug; it is what stops an unreadable file being
  re-stat'd and re-opened every two seconds.

- **`Progress.seen` as a `Vec` with linear `contains`** (`scan.rs:328`).
  Worst case is 2^patterns distinct bitsets; real logs have single digits, the
  comment records the measurement, and a hash set costs more per line.

- **The navigator ignoring definition filters** (`filter.rs:1483`): in AND mode
  a file can be marked *Yes* while the view shows nothing. Documented in the
  README's Definition-filters section, deliberate, and the alternative is a
  grammar pass per file per scan.

- **`theme_long_help()` deserialising the theme index on every run**
  (`config.rs:148`), not just `--help`, because clap evaluates `long_help =
  expr` while building the `Command`. Measured: release `recon --version` is
  2 ms, the same as `/usr/bin/true`. Not worth a `OnceLock`.

- **`bincode 1.3.3` unmaintained** (RUSTSEC-2025-0141). It arrives only through
  `syntect/dump-load`, which `two-face` genuinely needs to read its embedded
  grammar dumps; `cargo tree -e features` confirms it. `Cargo.toml`'s reasoning
  for `two-face` holds. Only F80 (recording the ignore in CI) is worth doing.

- **`Box::leak` on a user `.tmTheme`** (`syntax.rs:236`) and parsing the file's
  theme even when `--theme` overrides it (`config.rs:194`). A few KB once per
  process; the `'static` is what keeps `Highlighter` lifetime-free through
  `FileView` and `App`. The module doc records the trade.

- **`.expect(...)` on theme lookups** (`syntax.rs:134,220`). Both are on names
  the bundle itself just reported, never on user input — the user path goes
  through `ThemeError`.

- **`read_preview_with_caps` re-reading a file that exactly fits `max_bytes`**
  (`fileview.rs:985-1006`). One redundant read at a single byte boundary; the
  comment admits it. Not worth the branch.

- **`toml` with its serializer off plus `toml_edit` for the one write**
  (`filtersets.rs:297`). Two TOML crates for one file is not duplication: one
  parses with `serde` + `deny_unknown_fields`, the other preserves comments on
  write; `Cargo.toml` says why.

- **`lexical_absolute` collapsing `..` lexically past a symlink** (`path.rs:44`).
  Wrong in general, deliberately right for a navigator whose contract is "the
  path you walked"; documented and tested.

- **`status_bar_text` measuring `chars().count()` while `elide_left` measures
  columns** (`lib.rs:2257` vs `2450`). The status text is ASCII plus `▼` and
  `~`, all width 1, so the two agree for every string that can appear there.

- **`AppState` as a two-variant enum with `is_running`** (`lib.rs:420-436`).
  Looks like a `bool` in a costume; it is the shape a quit-with-output for #143
  would extend.

---

## Open questions for the maintainer

1. **Is the synchronous per-file `stat` in `refresh_scan` (F40) an accepted
   ceiling on directory size**, the way the 64-pattern limit is a documented
   one? If so it belongs in Known Limitations; if not, the worker already has
   the data.

2. **Is `filters.toml` meant to survive concurrent edits** — recon in two
   terminals, both pressing `S`? If yes, F37's temp-and-rename should also
   re-read immediately before writing; if no, one sentence in the README's
   "Saving" paragraph settles it.

3. **Should the CLI argument's spelling be preserved in the view title** (so
   `recon ../x.log` shows what was typed) with only `check_stamps` normalised
   (F41), or is the absolute path the intended title everywhere, as it is after
   the first navigation?

4. **`R` and the peek capture.** `reset()` drops the `!` memory and the solo,
   but `App::peek` (`lib.rs:331`) is not cleared, so `R` while peeked followed
   by `space` restores the pre-peek flags over the reset. "Flags only, one key
   to redo" — intentional, or the fifth snapshot mechanism nobody reconciled?

5. **Autoload without `default`** (F45): is "enabled header, every filter off"
   the intended reading of `autoload`, or a consequence of `enabled: false` at
   `filter.rs:768` being the simplest starting state?

6. **`MAX_PATTERNS = 64` counts every loaded set's patterns, enabled or not**
   (`filter.rs:1474`), so a `filters.toml` with a dozen sets switches navigator
   marking off permanently even with one set enabled. Is the "count everything"
   rule (bitset reuse across toggles) worth more than a per-enabled-set bitset
   that would lift the ceiling for the common case?

7. **Is the UTF-16 rejection (F49) a decision or an oversight?** The README
   documents the NUL rule without mentioning the one common text encoding it
   misclassifies.

8. **Symlinked directories drawn as plain files (F65)** — was following links
   in `describe` considered and declined (loops, cost), or not considered?

9. **A filename beginning with `-`** becomes an argv entry like `-foo.log:12`,
   which `zed`/`code` read as a flag (`editor.rs:225`). Should `substitute` or
   the shipped flavours insert `--` where the editor accepts it, or is "write
   your own template" the intended answer?

10. **`mode` is accepted in `filters.toml`** (`filtersets.rs:44`, only `"or"`)
    and never mentioned in the README's schema. Reserved seam for #40, or
    should the README list it as reserved so a user who meets it in an error
    knows why?

11. **Is `apply_syntax`'s per-frame recomputation (F52) measured?** The comment
    says "a few thousand pushes a frame, and nothing to keep in step". With
    event-driven draws that may be acceptable; if held-key scrolling with a
    search active ever stutters, this is the first place to look.

12. **Was cursor re-showing on exit dropped on purpose** (`main.rs:154`, F88)?
    The app never hides it today, so the commented line is either dead or a
    reminder.

---

Next: `/make-issues` to file the Medium+ findings as GitHub issues (`--all` to include Low).
Then `/code-review src/filter.rs` — the High bug, the module split and three drifted doc comments all live there, and this audit deliberately does not restate line-level detail.
