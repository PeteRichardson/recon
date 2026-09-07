# Headless Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `recon --emit …` with stdin that is not a terminal runs without the TUI: files from stdin or `PATH`, saved sets from `--set NAME[:PROFILE]`, `--hide` for hide mode, `-q` for no summary, output on stdout, read failures warned and exit 2.

**Architecture:** A new `src/headless.rs` composes the pieces `App` already uses — `ActiveFilters`, `Document`, `scan::scan` — over an input list, with no navigator, view or terminal, and returns the same `emit::Exit` a TUI session does, so `main` prints both through one `Exit::deliver`. The file reader leaves `FileView` for `Document::read` so both paths read a file the same way. `--set`, `--hide` and `-q` are ordinary `Config` flags applied by both paths.

**Tech Stack:** Rust 2024, clap 4 derive (`Vec<String>` for a repeatable flag), color-eyre, `std::io::IsTerminal`, `std::process::Command` for the integration test. No new dependencies.

**Spec:** `docs/specs/2026-09-06-headless-mode-design.md` — the authority when this plan and the spec disagree. It builds on `docs/specs/2026-09-06-emit-on-quit-design.md` (merged as PR #217) and the plan `docs/plans/2026-09-06-emit-on-quit.md`, whose conventions this plan repeats.

## Global Constraints

- **Lints:** `cargo clippy --all-targets` must report 0 warnings (the crate is `clippy::pedantic` at warn). `cargo fmt --check` clean. The vendored `tui-textarea-2` fork is untouched.
- **Full suite green after every task:** `cargo test` (unit + integration). Doc-consistency tests that must keep passing: `every_bound_key_is_documented`, the help-overlay 150-column budget, `readme_usage_block_matches_the_real_help` (README `## Usage` block must equal `recon -h` rendered at 80 columns — the test prints the block to paste when it drifts).
- **Fixtures:** unit tests make files with `crate::fixtures::{fixture_dir, fixture_file, fixture_path}`; names are case-insensitively unique crate-wide, so every new fixture name in this plan is distinct and prefixed `headless_`, `document_read_`, or `startup_`.
- **Command line (verbatim from the spec):** `recon [--emit lines|files|cwd] [-n] [-q] [--hide] [--set NAME[:PROFILE]]... [PATH]`. `--set` splits at the **first** colon; no colon means the set's `default` profile.
- **Headless predicate (verbatim):** `config.emit.is_some() && !io::stdin().is_terminal()`.
- **Errors (verbatim):** unknown set → `unknown set "Foo"; filters.toml defines: BugFilters, WiFi_debug`; read failure → `recon: cannot read PATH: reason` with reasons `no such file`, `permission denied`, `is a directory`, `binary file`; the file is skipped, the run continues, exit code **2**. Refusals before reading exit 1 through the existing error path.
- **Multi-file line format (verbatim):** `path<TAB>[N<TAB>]line` when there is more than one input; one input is exactly the TUI's output.
- **Summary wording (verbatim):** `recon: emitted 27 lines of app.log, hide mode`; `recon: emitted 27 lines of 3 files, hide mode`; `recon: emitted 812 lines of app.log, dim mode (27 match) — pass --hide to emit matches only`; `recon: emitted 3 files from /var/log, hide mode`; `recon: emitted 3 files of 14 inputs, hide mode`; `recon: emitted 14 files from /var/log, dim mode (3 match) — pass --hide to emit matches only`; `recon: emitted 14 files from /var/log, dim mode, no filter`; `recon: emitted /dir`. The dash is U+2014 `—`, as in the TUI's summaries.
- **`-q` suppresses the summary only.** Warnings still print.
- **Nothing is read twice**, and no file is read in `cwd` mode.
- **Output lines are bytes** (`emit::path_bytes` for paths; `String::as_bytes` for text), never re-encoded.
- **The TUI's behaviour from a terminal does not change:** every emit-on-quit summary, exit code, key and `filters.toml` rule stays; `FileView` shows the same messages it showed before.
- **Commits:** one per task, message in the repo's `type(scope): summary (#143)` style; never commit to `main`.

---

## File Structure

| File | Change | Responsibility after this plan |
| --- | --- | --- |
| `src/document.rs` | modify | Owns reading a file into lines: `read_lines(path) -> io::Result<Vec<String>>`, `Document::read(path)`, the NUL sniff and lossy line reader (moved from `fileview.rs`), `BINARY_FILE`, `is_binary`. |
| `src/widgets/fileview.rs` | modify | Keeps its `Contents` and messages; its `read_lines` becomes a thin wrapper over `document::read_lines`. |
| `src/config.rs` | modify | `--set`, `--hide`, `-q` flags; `sets_to_enable()`, `check_sets()`, `ConfigError::{UnknownSet, UnknownProfile}`; README Usage block regenerated. |
| `src/filter.rs` | modify | `ActiveFilters::enable_named(set, profile)` and `EnableError`. |
| `src/lib.rs` | modify | `App::new` applies `--set` and `--hide`; `pub mod headless`; `Exit::Emit` constructors gain `failed: 0`. |
| `src/viewport.rs` | modify | `is_interesting` becomes `pub(crate)` so headless counts matches by the TUI's definition. |
| `src/emit.rs` | modify | `Exit::Emit { failed }`, `deliver(requested, quiet, …)`, exit 2 on failures. |
| `src/headless.rs` | create | `run(config)`, `inputs`, `collect`, per-kind collectors, warnings. |
| `src/main.rs` | modify | `check_sets` after `load_file`; the headless predicate; `deliver` with `quiet`. |
| `tests/headless.rs` | create | Real-process integration test over the built binary. |
| `README.md` | modify | Usage block; "Headless mode" subsection; `--set`/`--hide` in Saved filter sets. |

---

### Task 1: `Document::read` — the file reader leaves `FileView`

**Files:**
- Modify: `src/document.rs` (imports at top; new items after `Document::for_file`)
- Modify: `src/widgets/fileview.rs:197-207` (`BINARY_MESSAGE` stays, `BINARY_SNIFF_BYTES` moves), `:1057-1158` (`sniff_binary`, `read_lossy_line`, `read_lines`)
- Test: `src/document.rs` (new tests module), existing `src/widgets/fileview.rs` tests keep passing

**Interfaces:**
- Consumes: `Document::for_file(path, lines)`, `crate::fixtures`.
- Produces: `pub fn document::read_lines(path: &Path) -> io::Result<Vec<String>>`; `impl Document { pub fn read(path: &Path) -> io::Result<Self> }`; `pub(crate) const document::BINARY_SNIFF_BYTES: usize`; `pub(crate) const document::BINARY_FILE: &str = "binary file"`; `pub(crate) fn document::is_binary(err: &io::Error) -> bool`; `pub(crate) fn document::sniff_binary<R: Read>(&mut R) -> io::Result<(bool, Vec<u8>)>`; `pub(crate) fn document::read_lossy_line<R: BufRead>(&mut R, &mut Vec<u8>) -> io::Result<Option<String>>`. Errors: a directory → `ErrorKind::IsADirectory` with message `is a directory`; a NUL in the first 8 KiB → `ErrorKind::InvalidData` with message `binary file`; anything else the OS error verbatim.

- [ ] **Step 1: Write the failing tests**

Append to `src/document.rs` (the file has no tests module yet; add one at the end):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{fixture_dir, fixture_file};

    #[test]
    fn read_lines_strips_line_ends_and_decodes_lossily() {
        let file = fixture_file("document_read_lossy.log", b"one\r\ntwo\xff\nthree");

        let lines = read_lines(&file).expect("readable");

        assert_eq!(lines, ["one", "two\u{FFFD}", "three"]);
    }

    #[test]
    fn read_builds_a_document_over_the_file() {
        let file = fixture_file("document_read_document.log", b"a\nb\n");

        let document = Document::read(&file).expect("readable");

        assert_eq!(document.lines(), ["a", "b"]);
        assert_eq!(document.verdicts().len(), 2, "one verdict slot per line");
    }

    #[test]
    fn read_lines_refuses_a_binary_file() {
        let file = fixture_file("document_read_binary.bin", b"abc\0def\n");

        let err = read_lines(&file).expect_err("a NUL in the head is binary");

        assert!(is_binary(&err), "not the binary error: {err}");
        assert_eq!(err.to_string(), BINARY_FILE);
    }

    #[test]
    fn read_lines_refuses_a_directory_up_front() {
        let dir = fixture_dir("document_read_dir");

        let err = read_lines(&dir).expect_err("a directory is not a file");

        assert_eq!(err.kind(), io::ErrorKind::IsADirectory);
        assert_eq!(err.to_string(), "is a directory");
        assert!(!is_binary(&err));
    }

    #[test]
    fn read_lines_reports_a_missing_file_as_not_found() {
        let err = read_lines(Path::new("target/document_read_no_such_file.log"))
            .expect_err("missing");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib document::tests`
Expected: compile error — `read_lines`, `is_binary`, `BINARY_FILE`, `Document::read` not found.

- [ ] **Step 3: Move the reader into `document.rs`**

Replace the imports at the top of `src/document.rs` with:

```rust
use crate::filter::{ActiveFilters, Verdict};
use crate::syntax::{self, KindSet};
use ratatui::style::Style;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::path::Path;
use std::sync::Arc;
```

Add, directly after `Document::for_file` (inside `impl Document`):

```rust
    /// A document over the whole of `path`, read the way the file view reads
    /// a file (#143): the same NUL sniff, the same lossy decoding, the same
    /// line-end stripping — and the error instead of a placeholder message,
    /// which is what headless mode needs and the widget wraps.
    pub fn read(path: &Path) -> io::Result<Self> {
        Ok(Self::for_file(path, read_lines(path)?))
    }
```

Add, after the `impl Document` block (before the tests module):

```rust
/// How much of a file's head is examined for a NUL before it is read as text.
pub(crate) const BINARY_SNIFF_BYTES: usize = 8 << 10;

/// The message on the `InvalidData` error `read_lines` returns for a file
/// whose head holds a NUL. `is_binary` recognises it; the file view turns it
/// into its own `<binary file>` message and headless mode prints it as is.
pub(crate) const BINARY_FILE: &str = "binary file";

/// Whether `err` is `read_lines`' own binary-file refusal rather than an OS
/// error.
#[must_use]
pub(crate) fn is_binary(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::InvalidData && err.to_string() == BINARY_FILE
}

/// Whether the head of `reader` looks like binary rather than text, along with
/// the bytes that had to be read to decide — they are the file's first bytes
/// and belong back in front of the stream.
///
/// A NUL byte is the signal, not a decode error. A decode error says one byte
/// in the file is not UTF-8, which is routine in a log; a NUL in the first few
/// KiB says the file is not a document at all.
pub(crate) fn sniff_binary<R: Read>(reader: &mut R) -> io::Result<(bool, Vec<u8>)> {
    let mut head = Vec::new();
    (&mut *reader)
        .take(BINARY_SNIFF_BYTES as u64)
        .read_to_end(&mut head)?;
    Ok((head.contains(&0), head))
}

/// Read one newline-terminated line, decoded lossily. `None` at end of file.
///
/// Lossy, not fatal: one bad byte in a two-gigabyte log must not cost the
/// other two gigabytes. U+FFFD marks the spot in place and the read carries
/// on, which is the whole difference from `lines()` — that short-circuits the
/// entire file on its first undecodable byte.
pub(crate) fn read_lossy_line<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> io::Result<Option<String>> {
    buf.clear();
    if reader.read_until(b'\n', buf)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(buf)
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    ))
}

/// Read `path` whole, as lines.
///
/// `File::open` succeeds on a directory on Unix and only fails when read, so
/// that case is refused up front with `IsADirectory`. A file whose head holds
/// a NUL is refused as [`BINARY_FILE`]; one that merely holds undecodable
/// bytes is read anyway, a U+FFFD per bad sequence. Anything else the OS
/// refuses comes back verbatim.
pub fn read_lines(path: &Path) -> io::Result<Vec<String>> {
    if path.is_dir() {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "is a directory"));
    }
    let mut reader = BufReader::new(File::open(path)?);
    let (binary, head) = sniff_binary(&mut reader)?;
    if binary {
        return Err(io::Error::new(io::ErrorKind::InvalidData, BINARY_FILE));
    }
    // The sniffed bytes are content, so they go back in front of the rest.
    let mut reader = Cursor::new(head).chain(reader);
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    while let Some(line) = read_lossy_line(&mut reader, &mut buf)? {
        lines.push(line);
    }
    Ok(lines)
}
```

- [ ] **Step 4: Make `FileView` a wrapper**

In `src/widgets/fileview.rs`:

1. Delete `const BINARY_SNIFF_BYTES: usize = 8 << 10;` (line ~207, with its doc comment). Keep `BINARY_MESSAGE`.
2. Delete the functions `sniff_binary` and `read_lossy_line` (lines ~1057-1091, with their doc comments).
3. Add to the imports at the top: `use crate::document::{self, BINARY_SNIFF_BYTES, read_lossy_line, sniff_binary};` — `read_preview_with_caps` (line ~1178) still calls `sniff_binary` and `read_lossy_line`, and the tests use `BINARY_SNIFF_BYTES`, so the names must stay in scope.
4. Replace the whole of `fn read_lines(path: &Path) -> Contents` (lines ~1093-1158) with:

```rust
/// Read `path` whole, or a single-line message describing why it could not
/// be read. The reading itself is `document::read_lines` (#143); this wraps
/// its error the way the pane shows it.
///
/// Never `truncated`, and never estimating: the whole file is here.
fn read_lines(path: &Path) -> Contents {
    // See `read_preview`: a directory opens fine and then fails to read, so
    // it is recognised up front rather than surfacing an OS error string.
    if path.is_dir() {
        return directory_listing(path, usize::MAX);
    }
    match document::read_lines(path) {
        Ok(lines) => Contents {
            lines,
            truncated: false,
            estimated_lines: None,
            text: true,
        },
        Err(err) if document::is_binary(&err) => Contents::message(BINARY_MESSAGE.to_string()),
        // Logged as well as shown (#83). The pane gets `<{err}>` in place of
        // the file, which tells the user *that* it failed; the log is where
        // the full path lives, and the pane's title is elided when the pane
        // is narrow.
        Err(err) => {
            log::warn!("cannot read {}: {err}", path.display());
            Contents::message(format!("<{err}>"))
        }
    }
}
```

5. Run `cargo clippy --all-targets`; if `Cursor` or `Read` in `fileview.rs`'s `std::io` import is now unused, drop it from that `use` line (leave any that `read_preview_with_caps` still needs).

The three distinct log messages the old function wrote (`cannot open`, `cannot read the start of`, `cannot read … at line N`) collapse into one `cannot read {path}: {err}`. `tests/logging.rs` asserts only that a warning mentioning the file is logged, which this keeps.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib document::tests` — 5 passed.
Run: `cargo test` — full suite green (the `fileview` tests over binary files, missing files and directory listings, and `tests/logging.rs`, exercise the wrapper).
Run: `cargo clippy --all-targets && cargo fmt --check` — clean.

- [ ] **Step 6: Commit**

```bash
git add src/document.rs src/widgets/fileview.rs
git commit -m "refactor(document): Document::read — the file reader leaves FileView, returning the error instead of a message (#143)"
```

---

### Task 2: `--set`, `--hide`, `-q` on `Config`, with `check_sets`

**Files:**
- Modify: `src/config.rs` (fields after `line_numbers` ~line 165; `impl Default` ~190; `ConfigError` ~376-410; `impl Config` next to `check_flags` ~513; tests after `default_matches_the_parsed_defaults` ~977)
- Modify: `README.md:189-233` (the `## Usage` block)
- Test: `src/config.rs`

**Interfaces:**
- Consumes: `crate::filter::LoadedSet { name: String, profiles: BTreeMap<String, Vec<String>>, .. }` (the type `Config.filter_sets` already holds); `crate::filter::test_support::loaded(name, priority, autoload, patterns) -> LoadedSet` (profiles empty).
- Produces: `Config.set: Vec<String>`, `Config.hide: bool`, `Config.quiet: bool`; `Config::sets_to_enable(&self) -> Vec<(String, Option<String>)>`; `Config::check_sets(&self, sets: &[LoadedSet]) -> Result<(), ConfigError>`; `ConfigError::UnknownSet { name: String, known: Vec<String> }`, `ConfigError::UnknownProfile { set: String, name: String, known: Vec<String> }`.

- [ ] **Step 1: Write the failing tests**

In `src/config.rs`'s tests module, extend `default_matches_the_parsed_defaults` with three lines after `assert_eq!(parsed.line_numbers, default.line_numbers);`:

```rust
        assert_eq!(parsed.set, default.set);
        assert_eq!(parsed.hide, default.hide);
        assert_eq!(parsed.quiet, default.quiet);
```

Then add, after the `--emit and --line-numbers (#143)` tests:

```rust
    // ---- --set, --hide, -q (#143, headless) -------------------------------

    #[test]
    fn set_splits_at_the_first_colon_and_repeats() {
        let config = Config::try_parse_from(["recon", "--set", "Bugs", "--set", "WiFi:bug:32"])
            .expect("parses");

        assert_eq!(
            config.sets_to_enable(),
            vec![
                ("Bugs".to_string(), None),
                ("WiFi".to_string(), Some("bug:32".to_string())),
            ]
        );
    }

    #[test]
    fn hide_and_quiet_parse_and_default_off() {
        let config = Config::try_parse_from(["recon", "--hide", "-q"]).expect("parses");
        assert!(config.hide);
        assert!(config.quiet);

        let config = Config::try_parse_from(["recon", "--quiet"]).expect("parses");
        assert!(config.quiet);

        assert!(!Config::default().hide);
        assert!(!Config::default().quiet);
        assert!(Config::default().set.is_empty());
    }

    /// A loaded set named `name` with one filter `x` and one profile.
    fn set_with_profile(name: &str, profile: &str) -> crate::filter::LoadedSet {
        let mut set = crate::filter::test_support::loaded(name, 50, false, &["x"]);
        set.profiles
            .insert(profile.to_string(), vec!["x".to_string()]);
        set
    }

    #[test]
    fn check_sets_accepts_a_known_set_with_and_without_a_profile() {
        let sets = [set_with_profile("Bugs", "p")];

        let plain = Config {
            set: vec!["Bugs".to_string()],
            ..Config::default()
        };
        assert!(plain.check_sets(&sets).is_ok());

        let with_profile = Config {
            set: vec!["Bugs:p".to_string()],
            ..Config::default()
        };
        assert!(with_profile.check_sets(&sets).is_ok());

        assert!(Config::default().check_sets(&[]).is_ok(), "no --set: nothing to check");
    }

    #[test]
    fn an_unknown_set_names_the_known_ones() {
        let sets = [set_with_profile("Bugs", "p"), set_with_profile("WiFi", "q")];
        let config = Config {
            set: vec!["Foo".to_string()],
            ..Config::default()
        };

        let err = config.check_sets(&sets).expect_err("refused");

        assert!(matches!(err, ConfigError::UnknownSet { .. }), "{err:?}");
        assert_eq!(
            err.to_string(),
            "unknown set \"Foo\"; filters.toml defines: Bugs, WiFi"
        );
    }

    #[test]
    fn an_unknown_set_with_no_file_says_none() {
        let config = Config {
            set: vec!["Foo".to_string()],
            ..Config::default()
        };

        let err = config.check_sets(&[]).expect_err("refused");

        assert_eq!(err.to_string(), "unknown set \"Foo\"; filters.toml defines: none");
    }

    #[test]
    fn an_unknown_profile_names_the_set_s_profiles() {
        let sets = [set_with_profile("Bugs", "p")];
        let config = Config {
            set: vec!["Bugs:nope".to_string()],
            ..Config::default()
        };

        let err = config.check_sets(&sets).expect_err("refused");

        assert!(matches!(err, ConfigError::UnknownProfile { .. }), "{err:?}");
        assert_eq!(
            err.to_string(),
            "unknown profile \"nope\"; set \"Bugs\" defines: p"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib config::tests`
Expected: compile error — no field `set`/`hide`/`quiet`, no method `sets_to_enable`/`check_sets`.

- [ ] **Step 3: Add the flags**

In `src/config.rs`, after the `line_numbers` field (inside `pub struct Config`):

```rust
    /// Enable a saved filter set at startup, as `NAME` for its `default`
    /// profile or `NAME:PROFILE` for another. Repeatable.
    #[arg(long = "set", value_name = "NAME[:PROFILE]")]
    pub set: Vec<String>,

    /// Start in hide mode: only matching lines and files.
    #[arg(long)]
    pub hide: bool,

    /// Suppress the summary line on stderr; warnings still print.
    #[arg(short = 'q', long)]
    pub quiet: bool,
```

In `impl Default for Config`, after `line_numbers: false,`:

```rust
            set: Vec::new(),
            hide: false,
            quiet: false,
```

- [ ] **Step 4: Add the errors**

In `pub enum ConfigError`, after `LineNumbersNeedLines`:

```rust
    /// `--set` naming a set `filters.toml` does not define (#143). `known`
    /// is every set it does define, for the message.
    UnknownSet { name: String, known: Vec<String> },
    /// `--set SET:NAME` naming a profile `set` does not define (#143).
    UnknownProfile {
        set: String,
        name: String,
        known: Vec<String>,
    },
```

In `impl fmt::Display for ConfigError`, after the `LineNumbersNeedLines` arm:

```rust
            Self::UnknownSet { name, known } => write!(
                f,
                "unknown set {name:?}; filters.toml defines: {}",
                known_list(known)
            ),
            Self::UnknownProfile { set, name, known } => write!(
                f,
                "unknown profile {name:?}; set {set:?} defines: {}",
                known_list(known)
            ),
```

After the `impl std::error::Error for ConfigError {}` line:

```rust
/// A list of names for an error message, or `none` for an empty one —
/// "defines: " followed by nothing reads as a truncated message.
fn known_list(known: &[String]) -> String {
    if known.is_empty() {
        "none".to_string()
    } else {
        known.join(", ")
    }
}
```

- [ ] **Step 5: Add the methods**

In `impl Config`, directly after `check_flags`:

```rust
    /// `--set` as the pairs `App::new` and headless mode apply: the set's
    /// name and, after the first colon, the profile to apply instead of
    /// `default`. A set name holding a colon is misparsed here; the
    /// unknown-set error then lists the real names, so it is found rather
    /// than hidden.
    #[must_use]
    pub fn sets_to_enable(&self) -> Vec<(String, Option<String>)> {
        self.set
            .iter()
            .map(|spec| match spec.split_once(':') {
                Some((set, profile)) => (set.to_string(), Some(profile.to_string())),
                None => (spec.clone(), None),
            })
            .collect()
    }

    /// Refuse a `--set` naming a set `sets` does not hold, or a profile its
    /// set does not define. Needs the loaded sets, so it runs in `main`
    /// right after `filtersets::load_file`, where `check_flags` did not
    /// have to wait.
    pub fn check_sets(&self, sets: &[crate::filter::LoadedSet]) -> Result<(), ConfigError> {
        for (name, profile) in self.sets_to_enable() {
            let Some(set) = sets.iter().find(|set| set.name == name) else {
                return Err(ConfigError::UnknownSet {
                    name,
                    known: sets.iter().map(|set| set.name.clone()).collect(),
                });
            };
            if let Some(profile) = profile
                && !set.profiles.contains_key(&profile)
            {
                return Err(ConfigError::UnknownProfile {
                    set: name,
                    name: profile,
                    known: set.profiles.keys().cloned().collect(),
                });
            }
        }
        Ok(())
    }
```

`sets` is what `filtersets::load_file` returns: the file's sets, including a `[sets.definitions]` override when the file has one. The built-in `definitions` set with no override is therefore refused by `--set definitions`; it has no profiles and no use headless, and the spec scopes `--set` to what `filters.toml` defines.

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib config::tests`
Expected: the six new tests and `default_matches_the_parsed_defaults` pass; `readme_usage_block_matches_the_real_help` **fails** and prints the regenerated block.

- [ ] **Step 7: Regenerate the README Usage block**

Replace the fenced block under `## Usage` in `README.md` (lines 191-233, from `Usage: recon [OPTIONS] [PATH]` to `Print version`) with exactly what the failing test printed. It should be the existing block with these three entries after `-n, --line-numbers` and before `-h, --help`:

```
      --set <NAME[:PROFILE]>
          Enable a saved filter set at startup, as `NAME` for its `default`
          profile or `NAME:PROFILE` for another. Repeatable
      --hide
          Start in hide mode: only matching lines and files
  -q, --quiet
          Suppress the summary line on stderr; warnings still print
```

The test's output is authoritative over the rendering above — paste what it prints.

- [ ] **Step 8: Run the full suite**

Run: `cargo test` — green, including `readme_usage_block_matches_the_real_help`.
Run: `cargo clippy --all-targets && cargo fmt --check` — clean.

- [ ] **Step 9: Commit**

```bash
git add src/config.rs README.md
git commit -m "feat(config): --set NAME[:PROFILE], --hide and -q, with check_sets naming the sets filters.toml defines (#143)"
```

---

### Task 3: `ActiveFilters::enable_named`, applied by `App::new`

**Files:**
- Modify: `src/filter.rs` (new `EnableError` after `LoadedSet` ~line 336; `enable_named` after `apply_profile` ~line 862; tests in the existing tests module, which already imports `test_support::loaded`)
- Modify: `src/lib.rs` (`App::new` ~line 602-625; new test after the `emit on quit` tests ~line 6390)
- Test: `src/filter.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: `Config::sets_to_enable()` (Task 2), `Config.hide` (Task 2), `ActiveFilters::set_enabled_set(usize, bool) -> bool`, `ActiveFilters::apply_profile(usize, &str) -> bool`, `ActiveFilters::sets() -> &[FilterSet]` (index 0 is the scratch set, name `""`), `ActiveFilters::filters_in(usize)`, `Filter::display_name()`, `App::set_mode(Mode)`, `Document::mode()`, `Document::visible_lines()` (cfg(test)).
- Produces: `pub enum filter::EnableError { UnknownSet(String), UnknownProfile { set: String, profile: String } }` (Debug, Clone, PartialEq, Eq, Display, Error); `pub fn ActiveFilters::enable_named(&mut self, set: &str, profile: Option<&str>) -> Result<(), EnableError>`. `App::new` enables every `--set` and applies `--hide`.

- [ ] **Step 1: Write the failing `filter.rs` tests**

In `src/filter.rs`'s tests module, after `loaded_filters_are_coloured_by_position_or_by_the_file`:

```rust
    // ---- enable_named (#143) ----------------------------------------------

    /// One file set `a` with filters `x`, `y`, `z`; `default` = `x`,
    /// `p` = `y`, `z`.
    fn with_profiles() -> ActiveFilters {
        let mut set = loaded("a", 50, false, &["x", "y", "z"]);
        set.profiles
            .insert("default".to_string(), vec!["x".to_string()]);
        set.profiles
            .insert("p".to_string(), vec!["y".to_string(), "z".to_string()]);
        ActiveFilters::with_sets(None, &[set])
    }

    fn enabled_names(set: &ActiveFilters) -> Vec<String> {
        set.filters_in(1)
            .filter(|(_, filter)| filter.enabled)
            .map(|(_, filter)| filter.display_name())
            .collect()
    }

    #[test]
    fn enable_named_turns_the_set_on_and_applies_default() {
        let mut set = with_profiles();

        set.enable_named("a", None).expect("known set");

        assert!(set.sets()[1].enabled);
        assert_eq!(enabled_names(&set), ["x"]);
    }

    #[test]
    fn enable_named_without_a_default_moves_no_flag() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);

        set.enable_named("a", None).expect("known set");

        assert!(set.sets()[1].enabled);
        assert!(
            enabled_names(&set).is_empty(),
            "no default profile: the set comes on with the toggles it had"
        );
    }

    #[test]
    fn enable_named_applies_the_named_profile_instead_of_default() {
        let mut set = with_profiles();

        set.enable_named("a", Some("p")).expect("known set and profile");

        assert!(set.sets()[1].enabled);
        assert_eq!(enabled_names(&set), ["y", "z"]);
    }

    #[test]
    fn enable_named_refuses_an_unknown_name_and_changes_nothing() {
        let mut set = with_profiles();

        assert_eq!(
            set.enable_named("b", None),
            Err(EnableError::UnknownSet("b".to_string()))
        );
        assert_eq!(
            set.enable_named("a", Some("nope")),
            Err(EnableError::UnknownProfile {
                set: "a".to_string(),
                profile: "nope".to_string(),
            })
        );
        assert_eq!(
            set.enable_named("", None),
            Err(EnableError::UnknownSet(String::new())),
            "the scratch set has no name and is never enabled this way"
        );
        assert!(!set.sets()[1].enabled, "a refused call enables nothing");
        assert!(enabled_names(&set).is_empty());
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --lib filter::tests::enable_named`
Expected: compile error — `enable_named`, `EnableError` not found.

- [ ] **Step 3: Implement `enable_named`**

In `src/filter.rs`, after the `LoadedSet` struct:

```rust
/// Why [`ActiveFilters::enable_named`] could not apply a `--set` (#143).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnableError {
    /// No set of that name — or the scratch set, which has none.
    UnknownSet(String),
    /// The set exists but defines no such profile.
    UnknownProfile { set: String, profile: String },
}

impl std::fmt::Display for EnableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSet(name) => write!(f, "unknown set {name:?}"),
            Self::UnknownProfile { set, profile } => {
                write!(f, "unknown profile {profile:?} for set {set:?}")
            }
        }
    }
}

impl std::error::Error for EnableError {}
```

In `impl ActiveFilters`, after `apply_profile`:

```rust
    /// Enable the set called `set` — `default` profile and all, exactly as
    /// `set_enabled_set` does — then apply `profile` when one is named
    /// (#143). Both names are checked before anything moves, so a refused
    /// call changes nothing. `Config::check_sets` refuses the same names in
    /// `main` before the terminal comes up; this is the same lookup, so a
    /// name that passed there cannot fail here.
    pub fn enable_named(&mut self, set: &str, profile: Option<&str>) -> Result<(), EnableError> {
        let index = self
            .sets
            .iter()
            .position(|meta| meta.name == set)
            .filter(|&index| index != 0)
            .ok_or_else(|| EnableError::UnknownSet(set.to_string()))?;
        if let Some(profile) = profile
            && !self.sets[index].profiles.contains_key(profile)
        {
            return Err(EnableError::UnknownProfile {
                set: set.to_string(),
                profile: profile.to_string(),
            });
        }
        self.set_enabled_set(index, true);
        if let Some(profile) = profile {
            self.apply_profile(index, profile);
        }
        Ok(())
    }
```

- [ ] **Step 4: Run the filter tests**

Run: `cargo test --lib filter::tests::enable_named` — 4 passed.

- [ ] **Step 5: Write the failing `App::new` test**

In `src/lib.rs`'s tests module, after `a_running_app_has_not_exited` (the last of the `which quit emits` tests):

```rust
    // ---- --set and --hide at startup (#143, headless) ----------------------

    /// `recon --set Bugs:only_hit --hide app.log` opens the TUI with the set
    /// on, the profile applied, and hide mode live on the loaded file — the
    /// same flags headless mode takes, applied the same way.
    #[test]
    fn set_and_hide_flags_apply_at_startup() {
        let file = fixture_file("startup_set_hide.log", b"hit\nmiss\n");
        let mut set = filter::test_support::loaded("Bugs", 50, false, &["hit", "miss"]);
        set.profiles
            .insert("only_hit".to_string(), vec!["hit".to_string()]);

        let app = App::new(&Config {
            path: file.display().to_string(),
            filter_sets: vec![set],
            set: vec!["Bugs:only_hit".to_string()],
            hide: true,
            ..Config::default()
        });

        assert!(app.filters.sets()[1].enabled, "the set is on");
        let enabled: Vec<String> = app
            .filters
            .filters_in(1)
            .filter(|(_, filter)| filter.enabled)
            .map(|(_, filter)| filter.display_name())
            .collect();
        assert_eq!(enabled, ["hit"], "the profile was applied, not default");
        assert_eq!(app.document.mode(), Mode::FilteredOnly);
        assert_eq!(
            app.document.visible_lines(),
            ["hit"],
            "hide mode is live on the loaded file"
        );
    }
```

The tests module imports `crate::fixtures::{fixture_dir, fixture_path as fixture_dir_path}` (~line 2973); add `fixture_file` to that line. (`fixture_path` inside `lib.rs`'s tests is a local helper with a different signature, hence the alias — leave it alone.)

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test --lib set_and_hide_flags_apply_at_startup`
Expected: FAIL — `the set is on` (the flags are parsed but nothing applies them).

- [ ] **Step 7: Apply the flags in `App::new`**

In `src/lib.rs`, `App::new`: before the `let mut app = Self { … }` struct literal, add:

```rust
        let mut filters = ActiveFilters::with_sets(config.filter_palette.clone(), &config.filter_sets);
        for (set, profile) in config.sets_to_enable() {
            // `Config::check_sets` refused an unknown name in `main` before
            // the terminal came up; a failure here is a hand-built `Config`
            // in a test, and the set is left off rather than the app brought
            // down over it.
            if let Err(err) = filters.enable_named(&set, profile.as_deref()) {
                log::warn!("--set {set}: {err}");
            }
        }
```

In the struct literal, replace
`filters: ActiveFilters::with_sets(config.filter_palette.clone(), &config.filter_sets),`
with `filters,`.

After the literal and before `app.sync_document();`, add:

```rust
        // `--hide`: on the document before `sync_document`, which carries
        // the mode across into the document it builds for the loaded file.
        if config.hide {
            app.set_mode(Mode::FilteredOnly);
        }
```

- [ ] **Step 8: Run the tests**

Run: `cargo test --lib set_and_hide_flags_apply_at_startup` — passes.
Run: `cargo test` — green. `cargo clippy --all-targets && cargo fmt --check` — clean.

- [ ] **Step 9: Commit**

```bash
git add src/filter.rs src/lib.rs
git commit -m "feat(filter): enable_named applies --set NAME[:PROFILE]; App::new honours --set and --hide (#143)"
```

---

### Task 4: `Exit` carries a failure count; `deliver` takes `quiet` and exits 2

**Files:**
- Modify: `src/emit.rs` (`Exit::Emit` ~line 33; `deliver` ~line 62-90; tests)
- Modify: `src/lib.rs` (five `emit::Exit::Emit {` constructors in `collect_lines`, `collect_files`, `collect_cwd` ~lines 1020-1112; the `emitted` test helper ~line 6403)
- Modify: `src/main.rs:41` (the `deliver` call)
- Test: `src/emit.rs`

**Interfaces:**
- Consumes: `Config.quiet` (Task 2).
- Produces: `Exit::Emit { lines: Vec<Vec<u8>>, summary: String, failed: usize }`; `Exit::deliver(self, requested: Option<Emit>, quiet: bool, stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode` — `failed > 0` → `ExitCode::from(2)` after writing the output and (unless `quiet`) the summary.

- [ ] **Step 1: Write the failing tests**

In `src/emit.rs`'s tests module, change the `deliver` helper to pass `quiet = false`:

```rust
    fn deliver(exit: Exit, requested: Option<Emit>) -> (Vec<u8>, Vec<u8>, ExitCode) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = exit.deliver(requested, false, &mut out, &mut err);
        (out, err, code)
    }
```

Add `failed: 0,` to every existing `Exit::Emit { … }` literal in the tests (five of them), and change the two direct `exit.deliver(Some(Emit::Lines), &mut stdout, &mut stderr)` calls to `exit.deliver(Some(Emit::Lines), false, &mut stdout, &mut stderr)`. Then add:

```rust
    #[test]
    fn quiet_drops_the_summary_and_nothing_else() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec()],
            summary: "recon: emitted 1 lines of a.log, hide mode".to_string(),
            failed: 0,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());

        let code = exit.deliver(Some(Emit::Lines), true, &mut out, &mut err);

        assert_eq!(out, b"one\n");
        assert!(err.is_empty(), "stderr: {}", String::from_utf8_lossy(&err));
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn a_failed_input_exits_2_after_the_output_and_the_summary() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec()],
            summary: "recon: emitted 1 lines of 1 file, hide mode".to_string(),
            failed: 1,
        };

        let (out, err, code) = deliver(exit, Some(Emit::Lines));

        assert_eq!(out, b"one\n");
        assert_eq!(err, b"recon: emitted 1 lines of 1 file, hide mode\n");
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn quiet_does_not_hide_the_failure_exit_code() {
        let exit = Exit::Emit {
            lines: Vec::new(),
            summary: "recon: emitted 0 lines of a.log, hide mode".to_string(),
            failed: 1,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());

        let code = exit.deliver(Some(Emit::Lines), true, &mut out, &mut err);

        assert!(err.is_empty());
        assert_eq!(code, ExitCode::from(2));
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --lib emit::tests`
Expected: compile error — no field `failed`; `deliver` takes 3 arguments.

- [ ] **Step 3: Implement**

In `src/emit.rs`, the `Emit` variant becomes:

```rust
    Emit {
        lines: Vec<Vec<u8>>,
        summary: String,
        /// Inputs a headless run could not read (#143): warned about as
        /// they were met and skipped, and the reason the exit code is 2.
        /// The TUI always passes 0.
        failed: usize,
    },
```

Replace `deliver`'s doc table and signature and body with:

```rust
    /// Print the session's result and say how the process should exit.
    ///
    /// | Exit | `--emit` given | stdout | stderr | code |
    /// |---|---|---|---|---|
    /// | `Emit`, `failed == 0` | yes | every line, newline-terminated | the summary, unless `quiet` | 0 |
    /// | `Emit`, `failed > 0` | yes | every line, newline-terminated | the summary, unless `quiet` | 2 |
    /// | `Silent` | yes | nothing | nothing | 1 |
    /// | `Silent` | no | nothing | nothing | 0 |
    ///
    /// `Silent` under `--emit` fails because the caller asked for output and
    /// got none: `dir=$(recon --emit cwd) && cd "$dir"` then skips the `cd`
    /// with no test on `$dir`. Empty output from a real emit is a success —
    /// the summary is what tells the two apart. Exit 2 is grep's code for an
    /// input that could not be read: the output for what *was* read is
    /// complete and the summary describes it, so both are still written.
    ///
    /// `quiet` (`-q`) drops the summary and nothing else — the read-failure
    /// warnings were written as they happened, before this runs.
    ///
    /// A write error on stdout is reported on stderr and is a failure, with
    /// one exception: `BrokenPipe`, which means the consumer closed its end
    /// (`recon --emit lines big.log | head`) and already got what it asked
    /// for. That is not this process's failure, so the summary is still
    /// written to stderr and the exit code is unchanged. Nothing here can
    /// panic on any of it: the terminal has already been restored, and a
    /// panic's backtrace would be the last thing the user saw.
    pub fn deliver(
        self,
        requested: Option<Emit>,
        quiet: bool,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> ExitCode {
        match (self, requested) {
            (
                Self::Emit {
                    lines,
                    summary,
                    failed,
                },
                _,
            ) => {
                let written = lines
                    .iter()
                    .try_for_each(|line| {
                        stdout
                            .write_all(line)
                            .and_then(|()| stdout.write_all(b"\n"))
                    })
                    .and_then(|()| stdout.flush());
                if let Err(err) = written
                    && err.kind() != std::io::ErrorKind::BrokenPipe
                {
                    let _ = writeln!(stderr, "recon: could not write the output: {err}");
                    return ExitCode::FAILURE;
                }
                if !quiet {
                    let _ = writeln!(stderr, "{summary}");
                }
                if failed > 0 {
                    ExitCode::from(2)
                } else {
                    ExitCode::SUCCESS
                }
            }
            (Self::Silent, Some(_)) => ExitCode::FAILURE,
            (Self::Silent, None) => ExitCode::SUCCESS,
        }
    }
```

In `src/lib.rs`: add `failed: 0,` to each of the five `emit::Exit::Emit { … }` constructors in `collect_lines` (two early returns and the final one), `collect_files` and `collect_cwd` — find them with `grep -n "emit::Exit::Emit {" src/lib.rs`. In the tests' `emitted` helper, change the pattern `emit::Exit::Emit { lines, summary } =>` to `emit::Emit { lines, summary, .. } =>` (keeping the `emit::Exit::` path as it is written there: `emit::Exit::Emit { lines, summary, .. } =>`). Any other `Exit::Emit { lines, .. }` patterns in lib.rs tests already use `..` and need no change.

In `src/main.rs`, the last line of `main` becomes:

```rust
    Ok(exit.deliver(
        config.emit,
        config.quiet,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib emit::tests` — all pass, including the three new ones.
Run: `cargo test` — green. `cargo clippy --all-targets && cargo fmt --check` — clean.

- [ ] **Step 5: Commit**

```bash
git add src/emit.rs src/lib.rs src/main.rs
git commit -m "feat(emit): Exit carries a failure count — exit 2 when an input could not be read; deliver takes quiet (#143)"
```

---

### Task 5: `headless::inputs` — the file list

**Files:**
- Create: `src/headless.rs`
- Modify: `src/lib.rs:257-271` (module list: add `pub mod headless;` between `pub mod filtersets;` and the `fixtures` line)
- Test: `src/headless.rs`

**Interfaces:**
- Consumes: `crate::path::lexical_absolute(&Path) -> PathBuf` (private module `path`, `pub(crate)` fn — reachable from any module in the crate); `crate::widgets::filenav::{sorted_entries(dir) -> io::Result<Vec<Entry>>, Kind::{Dir, Parent, Plain, Executable}}` with `Entry { name: OsString, kind: Kind, .. }`.
- Produces: `pub(crate) struct headless::Inputs { pub files: Vec<PathBuf>, pub from: Source }`; `pub(crate) enum headless::Source { Stdin, Directory(PathBuf), File }`; `pub(crate) fn headless::inputs(stdin: impl BufRead, path: &Path) -> io::Result<Inputs>`.

- [ ] **Step 1: Create the module with its failing tests**

Create `src/headless.rs`:

```rust
//! Headless mode (#143): `--emit` with stdin that is not a terminal.
//!
//! The pieces `App` composes — `ActiveFilters`, `Document`, `scan::scan` —
//! with no navigator, no view and no terminal. Files come from stdin or
//! from the `PATH` argument; the result leaves through the same `Exit` a
//! TUI session hands back, so `main` prints both the same way.

use crate::path::lexical_absolute;
use crate::widgets::filenav::{Kind, sorted_entries};
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

/// The files a headless run reads, and where the list came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Inputs {
    /// Absolute, in the order they were given.
    pub files: Vec<PathBuf>,
    pub from: Source,
}

/// Where the input list came from — the summary and `cwd` differ by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// One path per line on stdin.
    Stdin,
    /// `PATH` named a directory: its files, in the navigator's order.
    Directory(PathBuf),
    /// `PATH` named a file — or nothing that exists: that path alone.
    File,
}

/// Read the input list: every non-blank line of `stdin` as a path, or, when
/// stdin held none, what `path` names — a directory's files in the
/// navigator's order, or the file itself.
///
/// Lines are bytes, not `String`s, for the reason `Entry::name` is an
/// `OsString`: a Unix filename need not be UTF-8, and `ls -1` writes it
/// verbatim. A trailing `\r` is dropped so a CRLF list works; nothing else
/// is trimmed, since a name can end in a space. A line that is only
/// whitespace is skipped.
pub(crate) fn inputs(mut stdin: impl BufRead, path: &Path) -> io::Result<Inputs> {
    let mut files = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if stdin.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let bytes = strip_line_end(&line);
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        files.push(lexical_absolute(&path_from_bytes(bytes)));
    }
    if !files.is_empty() {
        return Ok(Inputs {
            files,
            from: Source::Stdin,
        });
    }

    let path = lexical_absolute(path);
    if path.is_dir() {
        let files = sorted_entries(&path)?
            .into_iter()
            .filter(|entry| !matches!(entry.kind, Kind::Dir | Kind::Parent))
            .map(|entry| path.join(entry.name))
            .collect();
        return Ok(Inputs {
            files,
            from: Source::Directory(path),
        });
    }
    Ok(Inputs {
        files: vec![path],
        from: Source::File,
    })
}

fn strip_line_end(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{fixture_dir, fixture_file};
    use std::fs;
    use std::io::Cursor;

    // ---- inputs ------------------------------------------------------------

    #[test]
    fn stdin_lines_are_paths_absolutised_in_order_with_blanks_skipped() {
        let got = inputs(
            Cursor::new(&b"b.log\n\n  \n/abs/a.log\r\nc.log"[..]),
            Path::new("."),
        )
        .expect("reads");

        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(got.from, Source::Stdin);
        assert_eq!(
            got.files,
            vec![
                cwd.join("b.log"),
                PathBuf::from("/abs/a.log"),
                cwd.join("c.log"),
            ]
        );
    }

    #[test]
    fn a_path_directory_lists_its_files_in_navigator_order_without_directories() {
        let dir = fixture_dir("headless_inputs_dir");
        fs::write(dir.join("b.log"), "x").expect("write");
        fs::write(dir.join("A.log"), "x").expect("write");
        fs::create_dir(dir.join("sub")).expect("mkdir");

        let got = inputs(Cursor::new(&b""[..]), &dir).expect("reads");

        let dir = lexical_absolute(&dir);
        assert_eq!(got.from, Source::Directory(dir.clone()));
        assert_eq!(got.files, vec![dir.join("A.log"), dir.join("b.log")]);
    }

    #[test]
    fn a_path_file_is_the_one_input_even_when_it_does_not_exist() {
        let file = fixture_file("headless_inputs_file.log", b"x\n");

        let got = inputs(Cursor::new(&b"\n"[..]), &file).expect("reads");

        assert_eq!(got.from, Source::File);
        assert_eq!(got.files, vec![lexical_absolute(&file)]);

        let missing = Path::new("target/headless_inputs_no_such_file.log");
        let got = inputs(Cursor::new(&b""[..]), missing).expect("reads");
        assert_eq!(got.from, Source::File);
        assert_eq!(got.files, vec![lexical_absolute(missing)]);
    }

    #[test]
    fn stdin_wins_over_the_path_argument() {
        let dir = fixture_dir("headless_inputs_stdin_wins");
        fs::write(dir.join("ignored.log"), "x").expect("write");

        let got = inputs(Cursor::new(&b"/only/this.log\n"[..]), &dir).expect("reads");

        assert_eq!(got.from, Source::Stdin);
        assert_eq!(got.files, vec![PathBuf::from("/only/this.log")]);
    }
}
```

Add `pub mod headless;` to `src/lib.rs`'s module list after `pub mod filtersets;`.

- [ ] **Step 2: Run the tests**

Run: `cargo test --lib headless::tests`
Expected: 4 passed. (The tests and the code land together in this task because the module does not exist before it; the test names pin the behaviour — if any fails, fix `inputs`, not the test. `a_path_directory_lists…` fails if `sorted_entries` returns a `Parent` row and it is not filtered, or if `Kind::Executable` files are dropped.)

- [ ] **Step 3: Lint and full suite**

Run: `cargo clippy --all-targets && cargo fmt --check && cargo test` — clean and green.

- [ ] **Step 4: Commit**

```bash
git add src/headless.rs src/lib.rs
git commit -m "feat(headless): inputs — one path per stdin line, else PATH's directory listing or PATH itself (#143)"
```

---

### Task 6: `--emit lines` and `--emit cwd` headless, with read-failure warnings

**Files:**
- Modify: `src/headless.rs` (imports; new functions after `inputs`; tests)
- Modify: `src/viewport.rs:31` (`fn is_interesting` → `pub(crate) fn is_interesting`)
- Test: `src/headless.rs`

**Interfaces:**
- Consumes: `Document::read(&Path) -> io::Result<Document>` (Task 1), `document::{is_binary, BINARY_FILE}` (Task 1), `Document::{set_mode(Mode), evaluate(&ActiveFilters), verdicts(), lines(), visible()}`, `Exit::Emit { lines, summary, failed }` (Task 4), `emit::path_bytes(&Path) -> Vec<u8>`, `ActiveFilters::{new(), add(&str)}`, `Inputs`/`Source` (Task 5).
- Produces: `fn headless::collect_lines(inputs: &Inputs, filters: &ActiveFilters, mode: Mode, line_numbers: bool, warnings: &mut impl Write) -> Exit`; `fn headless::collect_cwd(inputs: &Inputs) -> Exit`; `fn headless::warn(warnings: &mut impl Write, path: &Path, err: &io::Error)`; `fn headless::count(n: usize, noun: &str) -> String` (`1 file` / `3 files`); `pub(crate) fn viewport::is_interesting(&Verdict) -> bool`.

- [ ] **Step 1: Write the failing tests**

In `src/headless.rs`'s tests module, add to the imports `use crate::filter::ActiveFilters;` and `use crate::document::Mode;` and `use crate::emit::Exit;`, then append after the `inputs` tests:

```rust
    // ---- shared helpers ----------------------------------------------------

    fn filters_matching(pattern: &str) -> ActiveFilters {
        let mut filters = ActiveFilters::new();
        filters.add(pattern).expect("valid pattern");
        filters
    }

    fn strings(lines: &[Vec<u8>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| String::from_utf8(line.clone()).expect("utf-8 fixture"))
            .collect()
    }

    /// The parts of an `Exit::Emit`, as strings.
    fn emitted(exit: Exit) -> (Vec<String>, String, usize) {
        match exit {
            Exit::Emit {
                lines,
                summary,
                failed,
            } => (strings(&lines), summary, failed),
            Exit::Silent => panic!("headless never returns Silent"),
        }
    }

    fn one_file(name: &str, body: &[u8]) -> Inputs {
        let file = fixture_file(name, body);
        Inputs {
            files: vec![lexical_absolute(&file)],
            from: Source::File,
        }
    }

    /// A directory of `files`, listed as stdin would give them: in the
    /// order of `files`, not the navigator's.
    fn from_stdin(name: &str, files: &[(&str, &str)]) -> Inputs {
        let dir = fixture_dir(name);
        let files = files
            .iter()
            .map(|(file, body)| {
                let path = dir.join(file);
                fs::write(&path, body).expect("write fixture");
                lexical_absolute(&path)
            })
            .collect();
        Inputs {
            files,
            from: Source::Stdin,
        }
    }

    fn warnings_of(buf: &[u8]) -> String {
        String::from_utf8(buf.to_vec()).expect("utf-8 warnings")
    }

    // ---- lines -------------------------------------------------------------

    #[test]
    fn lines_over_one_file_in_dim_mode_is_the_whole_file_with_the_match_count() {
        let inputs = one_file("headless_lines_dim.log", b"hit\nmiss\nhit again\n");
        let mut warnings = Vec::new();

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            false,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
        assert_eq!(lines, ["hit", "miss", "hit again"]);
        assert_eq!(
            summary,
            "recon: emitted 3 lines of headless_lines_dim.log, dim mode (2 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 0);
        assert!(warnings.is_empty());
    }

    #[test]
    fn lines_over_one_file_in_hide_mode_is_the_matches() {
        let inputs = one_file("headless_lines_hide.log", b"hit\nmiss\nhit again\n");

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            false,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, ["hit", "hit again"]);
        assert_eq!(
            summary,
            "recon: emitted 2 lines of headless_lines_hide.log, hide mode"
        );
    }

    #[test]
    fn line_numbers_are_the_file_s_own_with_a_tab() {
        let inputs = one_file("headless_lines_numbered.log", b"hit\nmiss\nhit again\n");

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            true,
            &mut Vec::new(),
        );

        let (lines, _, _) = emitted(exit);
        assert_eq!(lines, ["1\thit", "3\thit again"]);
    }

    #[test]
    fn several_files_prefix_each_line_with_its_path_in_input_order() {
        let inputs = from_stdin(
            "headless_lines_several",
            &[("b.log", "miss\nhit\n"), ("a.log", "hit\n")],
        );
        let [b, a] = inputs.files.as_slice() else {
            panic!("two inputs")
        };

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            true,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
        assert_eq!(
            lines,
            [
                format!("{}\t2\thit", b.display()),
                format!("{}\t1\thit", a.display()),
            ],
            "b before a: the input order, not the navigator's"
        );
        assert_eq!(summary, "recon: emitted 2 lines of 2 files, hide mode");
    }

    #[test]
    fn several_files_without_n_still_prefix_the_path() {
        let inputs = from_stdin(
            "headless_lines_several_no_n",
            &[("a.log", "hit\n"), ("b.log", "hit\n")],
        );

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            false,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
        assert_eq!(
            lines,
            [
                format!("{}\thit", inputs.files[0].display()),
                format!("{}\thit", inputs.files[1].display()),
            ]
        );
        assert_eq!(
            summary,
            "recon: emitted 2 lines of 2 files, dim mode (2 match) — pass --hide to emit matches only"
        );
    }

    #[test]
    fn an_unreadable_input_is_warned_about_skipped_and_counted() {
        let mut inputs = from_stdin("headless_lines_unreadable", &[("a.log", "hit\n")]);
        let missing = lexical_absolute(&fixture_path("headless_lines_unreadable").join("missing.log"));
        inputs.files.insert(0, missing.clone());
        let mut warnings = Vec::new();

        let exit = collect_lines(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            false,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
        assert_eq!(
            warnings_of(&warnings),
            format!("recon: cannot read {}: no such file\n", missing.display())
        );
        assert_eq!(lines, [format!("{}\thit", inputs.files[1].display())]);
        assert_eq!(summary, "recon: emitted 1 lines of 1 file, hide mode");
        assert_eq!(failed, 1);
    }

    #[test]
    fn a_binary_input_and_a_directory_input_are_read_failures() {
        let dir = fixture_dir("headless_lines_binary_and_dir");
        let binary = dir.join("core.bin");
        fs::write(&binary, b"ab\0cd\n").expect("write");
        let inputs = Inputs {
            files: vec![lexical_absolute(&binary), lexical_absolute(&dir)],
            from: Source::Stdin,
        };
        let mut warnings = Vec::new();

        let exit = collect_lines(
            &inputs,
            &filters_matching("x"),
            Mode::Dimmed,
            false,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
        assert_eq!(
            warnings_of(&warnings),
            format!(
                "recon: cannot read {}: binary file\nrecon: cannot read {}: is a directory\n",
                lexical_absolute(&binary).display(),
                lexical_absolute(&dir).display(),
            )
        );
        assert!(lines.is_empty());
        assert_eq!(
            summary,
            "recon: emitted 0 lines of 0 files, dim mode (0 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 2);
    }

    // ---- cwd ---------------------------------------------------------------

    #[test]
    fn cwd_is_the_path_directory_or_the_first_input_s_parent() {
        let dir = fixture_dir("headless_cwd");
        let dir = lexical_absolute(&dir);

        let from_dir = Inputs {
            files: Vec::new(),
            from: Source::Directory(dir.clone()),
        };
        let (lines, summary, failed) = emitted(collect_cwd(&from_dir));
        assert_eq!(lines, [dir.display().to_string()]);
        assert_eq!(summary, format!("recon: emitted {}", dir.display()));
        assert_eq!(failed, 0);

        let from_stdin = Inputs {
            files: vec![dir.join("a.log"), dir.join("b.log")],
            from: Source::Stdin,
        };
        let (lines, _, _) = emitted(collect_cwd(&from_stdin));
        assert_eq!(lines, [dir.display().to_string()], "the first input's directory");

        let from_file = Inputs {
            files: vec![dir.join("a.log")],
            from: Source::File,
        };
        let (lines, _, _) = emitted(collect_cwd(&from_file));
        assert_eq!(lines, [dir.display().to_string()]);
    }
```

Add `fixture_path` to the tests' `use crate::fixtures::{…}` line.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --lib headless::tests`
Expected: compile error — `collect_lines`, `collect_cwd` not found.

- [ ] **Step 3: Expose `is_interesting`**

In `src/viewport.rs`, change `fn is_interesting(verdict: &Verdict) -> bool {` to `pub(crate) fn is_interesting(verdict: &Verdict) -> bool {` — the doc comment above it stays.

- [ ] **Step 4: Implement**

In `src/headless.rs`, replace the `use` block with:

```rust
use crate::document::{self, Document, Mode};
use crate::emit::{Exit, path_bytes};
use crate::filter::ActiveFilters;
use crate::path::lexical_absolute;
use crate::viewport::is_interesting;
use crate::widgets::filenav::{Kind, sorted_entries};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
```

After `inputs` (before `strip_line_end`), add:

```rust
/// `--emit lines`: every input's visible lines in `mode`, in input order,
/// each prefixed by `path<TAB>` when there is more than one input and by
/// `N<TAB>` under `-n` — so `path<TAB>N<TAB>line`, and with one input
/// exactly what the TUI emits. The match count is the TUI's: interesting
/// verdicts, summed over the files that were read.
fn collect_lines(
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    line_numbers: bool,
    warnings: &mut impl Write,
) -> Exit {
    let several = inputs.files.len() > 1;
    let mut lines = Vec::new();
    let mut read = 0;
    let mut failed = 0;
    let mut interesting = 0;
    for path in &inputs.files {
        let mut document = match Document::read(path) {
            Ok(document) => document,
            Err(err) => {
                warn(warnings, path, &err);
                failed += 1;
                continue;
            }
        };
        // The mode first: `evaluate` derives the visible set from the
        // verdicts *and* the mode, so setting it afterwards would need a
        // second pass.
        document.set_mode(mode);
        document.evaluate(filters);
        read += 1;
        interesting += document.verdicts().iter().filter(|v| is_interesting(v)).count();
        let text = document.lines();
        for &source in document.visible() {
            let mut line = Vec::new();
            if several {
                line.extend_from_slice(&path_bytes(path));
                line.push(b'\t');
            }
            if line_numbers {
                line.extend_from_slice(format!("{}\t", source + 1).as_bytes());
            }
            line.extend_from_slice(text[source].as_bytes());
            lines.push(line);
        }
    }
    let subject = match inputs.files.as_slice() {
        [only] => only.file_name().map_or_else(
            || only.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        _ => count(read, "file"),
    };
    let emitted = lines.len();
    let summary = match mode {
        Mode::Dimmed => format!(
            "recon: emitted {emitted} lines of {subject}, dim mode ({interesting} match) — pass --hide to emit matches only"
        ),
        Mode::FilteredOnly => format!("recon: emitted {emitted} lines of {subject}, hide mode"),
    };
    Exit::Emit {
        lines,
        summary,
        failed,
    }
}

/// `--emit cwd`: the directory `PATH` named, or the first input's. Nothing
/// is read.
fn collect_cwd(inputs: &Inputs) -> Exit {
    let dir = match &inputs.from {
        Source::Directory(dir) => dir.clone(),
        Source::Stdin | Source::File => inputs
            .files
            .first()
            .and_then(|file| file.parent())
            .map_or_else(|| PathBuf::from("/"), Path::to_path_buf),
    };
    Exit::Emit {
        lines: vec![path_bytes(&dir)],
        summary: format!("recon: emitted {}", dir.display()),
        failed: 0,
    }
}

/// `recon: cannot read PATH: reason`, written as the failure is met.
fn warn(warnings: &mut impl Write, path: &Path, err: &io::Error) {
    let _ = writeln!(
        warnings,
        "recon: cannot read {}: {}",
        path.display(),
        reason(err)
    );
}

/// The reason in the words a person reads, where the kind is plain; the OS
/// message, `(os error N)` and all, where it is not.
fn reason(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::NotFound => "no such file".to_string(),
        io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        io::ErrorKind::IsADirectory => "is a directory".to_string(),
        _ if document::is_binary(err) => document::BINARY_FILE.to_string(),
        _ => err.to_string(),
    }
}

/// `1 file`, `3 files`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib headless::tests` — all pass (4 from Task 5 + 8 new).
Run: `cargo clippy --all-targets && cargo fmt --check && cargo test` — clean and green. (`collect_lines`, `collect_cwd`, `warn`, `reason`, `count` are called only from tests until Task 7 adds `collect` and `run`; if clippy reports them dead, add `#[allow(dead_code)]` **on each of the five functions**, with the comment `// Wired into `run` by the next task.`, and remove the allows in Task 7.)

- [ ] **Step 6: Commit**

```bash
git add src/headless.rs src/viewport.rs
git commit -m "feat(headless): --emit lines and cwd over the input list — path<TAB>[N<TAB>]line for several files, read failures warned and counted (#143)"
```

---

### Task 7: `--emit files` headless, `collect`, and `run`

**Files:**
- Modify: `src/headless.rs` (imports; `run`, `filters_for`, `collect`, `collect_files`, `open_input`, `file_matches`; tests)
- Test: `src/headless.rs`

**Interfaces:**
- Consumes: `ActiveFilters::{with_sets(palette, &[LoadedSet]), enable_named(&str, Option<&str>) -> Result<(), EnableError>, matcher() -> Option<Matcher>, add_excluding(&str)}`; `Matcher::selects(u64) -> bool`; `scan::scan(reader: impl BufRead, &Matcher, Progress, &AtomicBool) -> Progress` with `Progress { seen: Vec<u64>, .. }` — the scan stops at the first selecting line; `Config::{emit, path, hide, line_numbers, filter_palette, filter_sets, sets_to_enable()}`; `emit::Emit`.
- Produces: `pub fn headless::run(config: &Config) -> color_eyre::Result<Exit>`; `pub(crate) fn headless::collect(what: Emit, inputs: &Inputs, filters: &ActiveFilters, mode: Mode, line_numbers: bool, warnings: &mut impl Write) -> Exit`; `fn headless::collect_files(inputs, filters, mode, warnings) -> Exit`.

- [ ] **Step 1: Write the failing tests**

Append to `src/headless.rs`'s tests module (add `use crate::emit::Emit;` to its imports):

```rust
    // ---- files -------------------------------------------------------------

    fn three_logs(name: &str) -> Inputs {
        from_stdin(
            name,
            &[("a.log", "hit\n"), ("b.log", "miss\n"), ("c.log", "x\nhit\n")],
        )
    }

    fn displayed(inputs: &Inputs) -> Vec<String> {
        inputs
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    #[test]
    fn files_in_hide_mode_lists_the_inputs_the_matcher_selects() {
        let inputs = three_logs("headless_files_hide");
        let mut warnings = Vec::new();

        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            &mut warnings,
        );

        let (lines, summary, failed) = emitted(exit);
        let all = displayed(&inputs);
        assert_eq!(lines, [all[0].clone(), all[2].clone()]);
        assert_eq!(summary, "recon: emitted 2 files of 3 inputs, hide mode");
        assert_eq!(failed, 0);
        assert!(warnings.is_empty());
    }

    #[test]
    fn files_in_dim_mode_lists_every_input_with_the_match_count() {
        let inputs = three_logs("headless_files_dim");

        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::Dimmed,
            &mut Vec::new(),
        );

        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, displayed(&inputs));
        assert_eq!(
            summary,
            "recon: emitted 3 files of 3 inputs, dim mode (2 match) — pass --hide to emit matches only"
        );
    }

    #[test]
    fn files_from_a_path_directory_says_from() {
        let mut inputs = three_logs("headless_files_from_dir");
        let dir = lexical_absolute(&fixture_path("headless_files_from_dir"));
        inputs.from = Source::Directory(dir.clone());

        let exit = collect_files(
            &inputs,
            &filters_matching("hit"),
            Mode::FilteredOnly,
            &mut Vec::new(),
        );

        let (_, summary, _) = emitted(exit);
        assert_eq!(
            summary,
            format!("recon: emitted 2 files from {}, hide mode", dir.display())
        );
    }

    #[test]
    fn files_in_hide_mode_with_no_matcher_lists_everything() {
        let inputs = three_logs("headless_files_no_matcher");
        let mut exclude_only = ActiveFilters::new();
        exclude_only.add_excluding("x").expect("valid pattern");
        assert!(exclude_only.matcher().is_none(), "sanity: nothing selects");

        let exit = collect_files(&inputs, &exclude_only, Mode::FilteredOnly, &mut Vec::new());
        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, displayed(&inputs), "nothing to hide against");
        assert_eq!(summary, "recon: emitted 3 files of 3 inputs, hide mode, no filter");

        let exit = collect_files(&inputs, &ActiveFilters::new(), Mode::Dimmed, &mut Vec::new());
        let (lines, summary, _) = emitted(exit);
        assert_eq!(lines, displayed(&inputs));
        assert_eq!(summary, "recon: emitted 3 files of 3 inputs, dim mode, no filter");
    }

    #[test]
    fn files_warns_about_and_skips_an_unreadable_input_in_both_modes() {
        let mut inputs = three_logs("headless_files_unreadable");
        let dir = lexical_absolute(&fixture_path("headless_files_unreadable"));
        let missing = dir.join("missing.log");
        inputs.files.insert(1, missing.clone());
        inputs.files.push(dir.clone());
        let expected_warnings = format!(
            "recon: cannot read {}: no such file\nrecon: cannot read {}: is a directory\n",
            missing.display(),
            dir.display(),
        );

        let mut warnings = Vec::new();
        let exit = collect_files(&inputs, &filters_matching("hit"), Mode::Dimmed, &mut warnings);
        let (lines, summary, failed) = emitted(exit);
        assert_eq!(warnings_of(&warnings), expected_warnings);
        assert_eq!(lines.len(), 3, "the three readable files: {lines:?}");
        assert_eq!(
            summary,
            "recon: emitted 3 files of 5 inputs, dim mode (2 match) — pass --hide to emit matches only"
        );
        assert_eq!(failed, 2);

        let mut warnings = Vec::new();
        let exit = collect_files(&inputs, &ActiveFilters::new(), Mode::Dimmed, &mut warnings);
        let (_, _, failed) = emitted(exit);
        assert_eq!(warnings_of(&warnings), expected_warnings, "checked even with nothing to scan");
        assert_eq!(failed, 2);
    }

    // ---- collect -----------------------------------------------------------

    #[test]
    fn collect_dispatches_on_the_emit_kind() {
        let inputs = one_file("headless_collect.log", b"hit\n");
        let filters = filters_matching("hit");
        let mut warnings = Vec::new();

        let (lines, _, _) = emitted(collect(
            Emit::Lines,
            &inputs,
            &filters,
            Mode::FilteredOnly,
            false,
            &mut warnings,
        ));
        assert_eq!(lines, ["hit"]);

        let (lines, _, _) = emitted(collect(
            Emit::Files,
            &inputs,
            &filters,
            Mode::FilteredOnly,
            false,
            &mut warnings,
        ));
        assert_eq!(lines, [inputs.files[0].display().to_string()]);

        let (lines, _, _) = emitted(collect(
            Emit::Cwd,
            &inputs,
            &filters,
            Mode::FilteredOnly,
            false,
            &mut warnings,
        ));
        assert_eq!(
            lines,
            [inputs.files[0].parent().expect("has a parent").display().to_string()]
        );
        assert!(warnings.is_empty());
    }

    /// `run` reads real stdin, so its wiring is exercised by
    /// `tests/headless.rs`; the filter construction it delegates to is
    /// checked here.
    #[test]
    fn filters_for_enables_each_set_and_refuses_an_unknown_one() {
        let mut set = crate::filter::test_support::loaded("Bugs", 50, false, &["hit"]);
        set.profiles
            .insert("p".to_string(), vec!["hit".to_string()]);
        let config = crate::config::Config {
            filter_sets: vec![set],
            set: vec!["Bugs:p".to_string()],
            ..crate::config::Config::default()
        };

        let filters = filters_for(&config).expect("known set");
        assert!(filters.sets()[1].enabled);
        assert!(filters.matcher().is_some(), "the profile enabled `hit`");

        let config = crate::config::Config {
            set: vec!["Nope".to_string()],
            ..crate::config::Config::default()
        };
        let err = filters_for(&config).expect_err("unknown set");
        assert!(err.to_string().contains("unknown set \"Nope\""), "{err}");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --lib headless::tests`
Expected: compile error — `collect_files`, `collect`, `filters_for` not found.

- [ ] **Step 3: Implement**

Replace the `use` block at the top of `src/headless.rs` with:

```rust
use crate::config::Config;
use crate::document::{self, Document, Mode};
use crate::emit::{Emit, Exit, path_bytes};
use crate::filter::{ActiveFilters, Matcher};
use crate::path::lexical_absolute;
use crate::scan::{self, Progress};
use crate::viewport::is_interesting;
use crate::widgets::filenav::{Kind, sorted_entries};
use color_eyre::{Result, eyre::eyre};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
```

Insert before `inputs` (after the `Source` enum):

```rust
/// Run headless: read the input list, build the filters `--set` asks for,
/// and collect what `--emit` names. Read-failure warnings go to stderr as
/// they are met; the `Exit` carries the output, the summary and the
/// failure count for `main` to deliver.
pub fn run(config: &Config) -> Result<Exit> {
    let Some(what) = config.emit else {
        return Err(eyre!("headless mode needs --emit"));
    };
    let inputs = inputs(io::stdin().lock(), Path::new(&config.path))?;
    let filters = filters_for(config)?;
    let mode = if config.hide {
        Mode::FilteredOnly
    } else {
        Mode::Dimmed
    };
    Ok(collect(
        what,
        &inputs,
        &filters,
        mode,
        config.line_numbers,
        &mut io::stderr(),
    ))
}

/// The startup filter set: the loaded sets, then each `--set` enabled — the
/// same two steps `App::new` takes.
fn filters_for(config: &Config) -> Result<ActiveFilters> {
    let mut filters = ActiveFilters::with_sets(config.filter_palette.clone(), &config.filter_sets);
    for (set, profile) in config.sets_to_enable() {
        filters
            .enable_named(&set, profile.as_deref())
            .map_err(|err| eyre!("--set {set}: {err}"))?;
    }
    Ok(filters)
}

/// What `--emit` names, over `inputs`. `warnings` gets one line per input
/// that could not be read.
pub(crate) fn collect(
    what: Emit,
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    line_numbers: bool,
    warnings: &mut impl Write,
) -> Exit {
    match what {
        Emit::Lines => collect_lines(inputs, filters, mode, line_numbers, warnings),
        Emit::Files => collect_files(inputs, filters, mode, warnings),
        Emit::Cwd => collect_cwd(inputs),
    }
}
```

Insert after `collect_lines` (before `collect_cwd`):

```rust
/// `--emit files`: every readable input in dim mode; in hide mode, the
/// inputs the matcher selects — or every readable input when nothing
/// selects, which is the navigator's rule: it hides nothing it cannot mark.
///
/// One scan per file, stopping at the first selecting line, which is what
/// the navigator's scan costs. With nothing to scan, each input is still
/// opened, so an unreadable one is warned about and skipped in every mode.
/// The summary's `from <dir>` / `of N inputs` follows where the list came
/// from; there is no `unscanned` here, since every scan runs to its answer
/// before anything prints.
fn collect_files(
    inputs: &Inputs,
    filters: &ActiveFilters,
    mode: Mode,
    warnings: &mut impl Write,
) -> Exit {
    let matcher = filters.matcher();
    let mut lines = Vec::new();
    let mut matched = 0;
    let mut failed = 0;
    for path in &inputs.files {
        let answer = match &matcher {
            Some(matcher) => file_matches(path, matcher),
            None => open_input(path).map(|_| false),
        };
        let yes = match answer {
            Ok(yes) => yes,
            Err(err) => {
                warn(warnings, path, &err);
                failed += 1;
                continue;
            }
        };
        if yes {
            matched += 1;
        }
        let listed = match mode {
            Mode::Dimmed => true,
            Mode::FilteredOnly => yes || matcher.is_none(),
        };
        if listed {
            lines.push(path_bytes(path));
        }
    }
    let emitted = lines.len();
    let origin = match &inputs.from {
        Source::Directory(dir) => format!("from {}", dir.display()),
        Source::Stdin | Source::File => format!("of {}", count(inputs.files.len(), "input")),
    };
    let summary = match (matcher.is_some(), mode) {
        (true, Mode::FilteredOnly) => {
            format!("recon: emitted {emitted} files {origin}, hide mode")
        }
        (true, Mode::Dimmed) => format!(
            "recon: emitted {emitted} files {origin}, dim mode ({matched} match) — pass --hide to emit matches only"
        ),
        (false, Mode::Dimmed) => {
            format!("recon: emitted {emitted} files {origin}, dim mode, no filter")
        }
        (false, Mode::FilteredOnly) => {
            format!("recon: emitted {emitted} files {origin}, hide mode, no filter")
        }
    };
    Exit::Emit {
        lines,
        summary,
        failed,
    }
}

/// Open an input for scanning. A directory is refused up front: `File::open`
/// accepts one on Unix and only the read fails, and `scan` swallows a read
/// error as end of file.
fn open_input(path: &Path) -> io::Result<File> {
    if path.is_dir() {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "is a directory"));
    }
    File::open(path)
}

/// Whether any line of `path` selects under `matcher` — `Record::answer`'s
/// rule, over a scan run to its answer.
fn file_matches(path: &Path, matcher: &Matcher) -> io::Result<bool> {
    let reader = BufReader::new(open_input(path)?);
    let progress = scan::scan(reader, matcher, Progress::default(), &AtomicBool::new(false));
    Ok(progress.seen.iter().any(|&bits| matcher.selects(bits)))
}
```

Remove any `#[allow(dead_code)]` added in Task 6.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib headless::tests` — all pass (12 from before + 7 new).
Run: `cargo clippy --all-targets && cargo fmt --check && cargo test` — clean and green.

- [ ] **Step 5: Commit**

```bash
git add src/headless.rs
git commit -m "feat(headless): --emit files by one scan per input; run composes inputs, --set filters and the collectors (#143)"
```

---

### Task 8: `main` decides headless; the real-process integration test

**Files:**
- Modify: `src/main.rs:12` (import), `:20-41` (`main`)
- Create: `tests/headless.rs`
- Test: `tests/headless.rs`

**Interfaces:**
- Consumes: `Config::check_sets` (Task 2), `recon::headless::run` (Task 7), `Exit::deliver(.., quiet, ..)` (Task 4), `std::io::IsTerminal`. The binary is `env!("CARGO_BIN_EXE_recon")` (Cargo sets it for integration tests of a package with a bin target). `filtersets::load_file` reads `$XDG_CONFIG_HOME/recon/filters.toml`.
- Produces: the finished behaviour.

- [ ] **Step 1: Write the failing integration test**

Create `tests/headless.rs`:

```rust
//! Headless mode (#143) end to end: the built binary, a piped stdin, real
//! files and a real `filters.toml`. `src/headless.rs` tests the pieces;
//! this is the one place `main`'s headless decision, the flags, the
//! summary on stderr and the exit code are exercised together — with no
//! tty, so it runs in CI.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A fresh directory for one test under `target/`, named after the test so
/// parallel tests never share one.
fn fixture(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures-headless")
        .join(name);
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

/// A config home whose `filters.toml` defines one set, `Bugs`, with the
/// include filters `hit` and `other` and a profile `only_hit`. No `default`
/// profile, so `--set Bugs` alone would enable nothing — the tests always
/// name the profile.
fn config_home(dir: &Path) -> PathBuf {
    let home = dir.join("config");
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(
        home.join("recon/filters.toml"),
        "[sets.Bugs]\n\n\
         [sets.Bugs.profiles]\n\
         only_hit = [\"hit\"]\n\n\
         [[sets.Bugs.filters]]\n\
         name = \"hit\"\n\
         pattern = \"hit\"\n\n\
         [[sets.Bugs.filters]]\n\
         name = \"other\"\n\
         pattern = \"other\"\n",
    )
    .expect("write filters.toml");
    home
}

/// Run recon with `args`, `stdin` piped in and closed, and the config home
/// at `home`. Stdin is a pipe even when empty, which is what makes the run
/// headless.
fn recon(home: &Path, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_recon"))
        .args(args)
        .env("XDG_CONFIG_HOME", home)
        .env_remove("RECON_LOG")
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn recon");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("wait for recon")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("utf-8 output")
}

#[test]
fn files_hide_lists_the_matching_inputs_from_stdin() {
    let dir = fixture("files_hide");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    let b = dir.join("b.log");
    fs::write(&a, "hit\n").expect("write");
    fs::write(&b, "miss\n").expect("write");
    let list = format!("{}\n{}\n", a.display(), b.display());

    let out = recon(
        &home,
        &["--emit", "files", "--set", "Bugs:only_hit", "--hide"],
        list.as_bytes(),
    );

    assert_eq!(text(&out.stdout), format!("{}\n", a.display()));
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 1 files of 2 inputs, hide mode\n"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn lines_n_over_two_files_prefixes_path_and_line_number() {
    let dir = fixture("lines_two_files");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    let b = dir.join("b.log");
    fs::write(&a, "hit\n").expect("write");
    fs::write(&b, "miss\nhit\n").expect("write");
    let list = format!("{}\n{}\n", b.display(), a.display());

    let out = recon(
        &home,
        &["--emit", "lines", "-n", "--set", "Bugs:only_hit", "--hide"],
        list.as_bytes(),
    );

    assert_eq!(
        text(&out.stdout),
        format!("{}\t2\thit\n{}\t1\thit\n", b.display(), a.display()),
        "b before a: input order"
    );
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 2 lines of 2 files, hide mode\n"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn a_path_file_with_empty_stdin_is_read_headless_and_quiet_drops_the_summary() {
    let dir = fixture("quiet");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\nmiss\n").expect("write");

    let out = recon(
        &home,
        &["--emit", "lines", "-q", a.to_str().expect("utf-8 path")],
        b"",
    );

    assert_eq!(text(&out.stdout), "hit\nmiss\n", "dim mode, no filter: the whole file");
    assert!(out.stderr.is_empty(), "stderr: {}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn an_unreadable_input_warns_skips_and_exits_2() {
    let dir = fixture("unreadable");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\n").expect("write");
    let missing = dir.join("missing.log");
    let list = format!("{}\n{}\n", missing.display(), a.display());

    let out = recon(
        &home,
        &["--emit", "lines", "--set", "Bugs:only_hit", "--hide"],
        list.as_bytes(),
    );

    assert_eq!(text(&out.stdout), format!("{}\thit\n", a.display()));
    assert_eq!(
        text(&out.stderr),
        format!(
            "recon: cannot read {}: no such file\nrecon: emitted 1 lines of 1 file, hide mode\n",
            missing.display()
        )
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn cwd_over_a_path_directory_prints_it_without_reading_anything() {
    let dir = fixture("cwd");
    let home = config_home(&dir);
    fs::write(dir.join("core.bin"), b"\0\0\0").expect("write");

    let out = recon(
        &home,
        &["--emit", "cwd", dir.to_str().expect("utf-8 path")],
        b"",
    );

    assert_eq!(text(&out.stdout), format!("{}\n", dir.display()));
    assert_eq!(text(&out.stderr), format!("recon: emitted {}\n", dir.display()));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn an_unknown_set_is_refused_before_anything_is_read() {
    let dir = fixture("unknown_set");
    let home = config_home(&dir);

    let out = recon(&home, &["--emit", "files", "--set", "Nope"], b"");

    assert!(out.stdout.is_empty());
    assert!(
        text(&out.stderr).contains("unknown set \"Nope\"; filters.toml defines: Bugs"),
        "stderr: {}",
        text(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test headless`
Expected: every test fails — without the `main` wiring the binary tries to start the TUI on a pipe, so `enable_raw_mode` (or the alternate-screen write) fails and the process exits 1 with an error on stderr, or the stdout assertions fail. Whichever it is, none pass. (`an_unknown_set_is_refused…` may fail on the message rather than the code: `check_sets` is not called yet.)

- [ ] **Step 3: Wire `main`**

In `src/main.rs`, change the `std::io` import to:

```rust
use std::io::{self, IsTerminal, Stderr};
```

Replace `main` with:

```rust
fn main() -> Result<ExitCode> {
    install_error_hooks()?;

    setup_logging();
    let mut config = Config::load()?;
    config.filter_sets = recon::filtersets::load_file()?;
    // Needs the loaded sets, which is why it is not inside `Config::load`
    // with `check_flags`. Still before any terminal setup: the message must
    // reach a screen that is not about to be replaced (#143).
    config.check_sets(&config.filter_sets)?;

    if let Some(flavour) = &config.print_editor_config {
        print!(
            "{}",
            recon::editor::print_editor_config(
                flavour,
                std::env::var("TERM_PROGRAM").ok().as_deref(),
            )?
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Headless (#143): `--emit` with no terminal on stdin. A TUI needs stdin
    // for its keys, so a pipe or `/dev/null` there is not a session that
    // could have been driven anyway; the result is computed and printed
    // instead. `recon --emit lines app.log` from a terminal still gets the
    // TUI, and `< /dev/null` forces headless from one.
    let headless = config.emit.is_some() && !io::stdin().is_terminal();
    let exit = if headless {
        recon::headless::run(&config)?
    } else {
        let terminal = init_terminal()?;
        let exit = App::new(&config).run(terminal)?;
        restore_terminal()?;
        exit
    };

    // Only now, with the alternate screen gone (or never entered), does
    // anything reach stdout: the result, if `--emit` asked for one, and the
    // summary that names its mode on stderr (#143).
    Ok(exit.deliver(
        config.emit,
        config.quiet,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --test headless` — 6 passed.
Run: `cargo test` — green. `cargo clippy --all-targets && cargo fmt --check` — clean.

- [ ] **Step 5: Verify the TUI path is untouched under a pty**

The automated suite never runs `init_terminal`. From the worktree root, after `cargo build`:

```sh
cd /tmp && (sleep 1; printf 'q') | script -q /dev/null sh -c \
  "$OLDPWD/target/debug/recon --emit cwd $OLDPWD > /tmp/headless-out 2>/tmp/headless-err; echo \$? > /tmp/headless-code"; cd "$OLDPWD"
cat /tmp/headless-out /tmp/headless-code
```

Expected: `/tmp/headless-out` holds the worktree path, the code is `0`, and `/tmp/headless-err` ends with `recon: emitted <path>` — stdin is the pty here, so the TUI ran and `q` emitted. Then the headless form of the same command:

```sh
target/debug/recon --emit cwd . < /dev/null; echo "exit $?"
```

Expected: the absolute path of the worktree on stdout, `recon: emitted <path>` on stderr, `exit 0`, and no screen flicker — the TUI never started.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/headless.rs
git commit -m "feat(main): headless when --emit is given and stdin is not a terminal; check --set before anything runs (#143)"
```

---

### Task 9: README — Headless mode, and `--set`/`--hide` for the TUI

**Files:**
- Modify: `README.md` (end of `## Emitting the result`, ~line 1156-1165, before `## Opening an editor`; the Saved filter sets section, after the bullet list that ends with the `name`/`colour` items ~line 470)
- Test: `cargo test` (`readme_usage_block_matches_the_real_help` still passes; nothing else reads the README)

**Interfaces:** none — prose.

- [ ] **Step 1: Replace the closing paragraph of "Emitting the result"**

Find this text (the last paragraph before `## Opening an editor`):

```
that isn't, and a trailing carriage return is stripped — paths are the only
part of the output that is byte-for-byte verbatim. Not emitted yet: the
filter set itself, and a structured format that carries the mode and each
file's matching filter in the output — both are follow-ups to #143, along
with a headless mode that takes the same `--emit` without starting the TUI.
```

Replace it with:

````
that isn't, and a trailing carriage return is stripped — paths are the only
part of the output that is byte-for-byte verbatim. Not emitted yet: the
filter set itself, and a structured format that carries the mode and each
file's matching filter in the output — both are follow-ups to #143.

### Headless mode

`--emit` with stdin that is not a terminal skips the TUI altogether: the
files come from stdin or the argument, the filters from `--set`, and the
result goes to stdout exactly as `q` would have sent it.

```sh
ls -1 *.log | recon --emit files --set BugFilters:Bug57 --hide
find . -name '*.log' | recon --emit lines -n --set BugFilters --hide | cut -f1,2
recon --emit files --hide /var/log < /dev/null
```

Headless is inferred, never flagged. A pipe on stdin, a cron job, a script
with stdin closed all get it; `recon --emit lines app.log` from a terminal
still opens the TUI, because the terminal is where its keys come from. From
a terminal, `< /dev/null` forces it.

**Inputs.** Each non-blank line of stdin is a path — what `ls -1` and `find`
print. With nothing on stdin, a `PATH` directory means its files,
non-recursive, in the navigator's order, and a `PATH` file means itself.

**Flags.** `--set NAME` enables a saved set with its `default` profile;
`--set NAME:PROFILE` applies another profile instead. Repeat it for several
sets. `--hide` starts in hide mode, so only matches are emitted; without it
the run is in dim mode and emits everything, with the match count in the
summary. `-n` numbers lines as in the TUI. `-q` drops the summary; warnings
still print. All four work in the TUI too. An unknown set or profile is
refused before anything is read, and the error lists the names
`filters.toml` defines.

**Several files.** With more than one input, `--emit lines` prefixes each
line with its path and a tab, and `-n` puts the line number and a tab after
that — `path<TAB>N<TAB>line` — so `cut -f1` is the paths and `cut -f3-` the
text. One input is exactly the TUI's output.

**Summaries and exit codes.** The summary names the mode as it does in the
TUI, with `pass --hide to emit matches only` in place of the key:

| Run | Summary |
| --- | --- |
| `lines`, one file, hide mode | `recon: emitted 27 lines of app.log, hide mode` |
| `lines`, several files, dim mode | `recon: emitted 812 lines of 3 files, dim mode (27 match) — pass --hide to emit matches only` |
| `files` from a `PATH` directory | `recon: emitted 3 files from /var/log, hide mode` |
| `files` from stdin | `recon: emitted 3 files of 14 inputs, hide mode` |
| `files` with no include filter enabled | `recon: emitted 14 files from /var/log, dim mode, no filter` |
| `cwd` | `recon: emitted /var/log` |

A file that cannot be read is reported as it is met — `recon: cannot read
/var/log/secure: permission denied`, `… is a directory`, `… binary file` —
and skipped: not emitted, not counted. The run continues and exits **2**,
grep's convention for an input that failed. Exit 0 otherwise, empty output
included; exit 1 for a refused flag or an unreadable `filters.toml`.

Not in the first version: ad-hoc patterns (`-i PATTERN`) and a live search —
`grep` covers the one-off case, and saved sets are what headless is for.
````

- [ ] **Step 2: Document the flags in "Saved filter sets"**

In the `#### Saved filter sets` section, find the paragraph that begins `A header carries `*` when its set has profiles.` and insert **before** it:

```
Sets can also be switched on from the command line: `--set WiFi_debug`
enables the set at startup with its `default` profile, `--set
WiFi_debug:WiFi_bug_32` applies that profile instead, and the flag repeats
for several sets. `--hide` alongside it starts the session in hide mode. A
set without a `default` profile comes on with every filter off, exactly as
`Enter` on its header would leave it — give it a `default` if it is meant to
be used this way. The same flags drive a run with no TUI at all; see
[Headless mode](#headless-mode).

```

- [ ] **Step 3: Check the result**

Run: `cargo test` — green (`readme_usage_block_matches_the_real_help` reads only the Usage block, which Task 2 already regenerated).
Run: `grep -n "^### Headless mode\|Headless mode\](#headless-mode)" README.md` — both the heading and the link are present.
Read the two inserted sections once in the rendered file for a broken fence or table.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs(readme): headless mode — inputs, --set/--hide/-q, the multi-file prefix, summaries and exit codes (#143)"
```

---

## Self-review

**Spec coverage.**

| Spec section | Task |
| --- | --- |
| Decision 1, headless inferred from `--emit` + non-tty stdin; `< /dev/null` forces it | 8 (`main`), integration tests `a_path_file_with_empty_stdin…`, `cwd_over_a_path_directory…` |
| Decision 2, files from stdin lines else `PATH` (dir → navigator order minus dirs; file → itself) | 5 |
| Decision 3, `path<TAB>[N<TAB>]line` for several files; one file = TUI output | 6, integration `lines_n_over_two_files…` |
| Decision 4, `--set NAME[:PROFILE]` split at the first colon, `default` otherwise, misparse discovered by the error listing real names | 2 (`sets_to_enable`, `check_sets`), 3 (`enable_named`) |
| Decision 5, flags shared by both paths | 3 (`App::new`), 7 (`filters_for`, `run`) |
| Decision 6, no ad-hoc patterns | none needed; README says so (9) |
| Decision 7, read failures warn, skip, exit 2 | 4 (`failed`, exit 2), 6 (`warn`, `reason`), 7 (files), 8 (integration) |
| Decision 8, `-q` silences the summary only | 4, integration `…quiet_drops_the_summary` |
| Decision 9, a `headless` module, no `App` | 5-7 |
| Command line and `--help` text | 2 |
| Validation order: `-n` first (existing), then `check_sets` in `main` after `load_file` | 8 |
| `main` shape and `Exit::deliver(emit, quiet, …)` | 4, 8 |
| `App::new` applies `--set` and `--hide` after `with_sets` | 3 |
| `enable_named` lookup shared with validation | 3 (same name lookup; `check_sets` on `LoadedSet`s, `enable_named` on `FilterSet`s — same names, same profile maps) |
| `inputs`, `Inputs`, `Source` | 5 |
| Filters via `with_sets` + `enable_named` + `matcher()` | 7 |
| `Document::read` moved out of `fileview.rs`; `FileView` wraps the error | 1 |
| Per-file `lines` / `files` (one scan, `Record::answer`'s rule, no-matcher lists all) / `cwd` reads nothing | 6, 7 |
| Every summary string | 6, 7 (verbatim in Global Constraints and tests) |
| Error wording and exit codes | 6 (`reason`), 4 (exit 2), 8 (exit 1 through `?`) |
| Testing list: `headless.rs`, `filter.rs`, `config.rs`, `emit.rs`, `lib.rs`, `tests/headless.rs` | 5-7, 3, 2, 4, 3, 8 |
| README subsection + saved-sets flags + Usage block | 9, 9, 2 |
| "What does not change" | 1 (wrapper keeps `FileView`'s messages), 4 (TUI passes `failed: 0`), 8 (pty check) |

Two spec details resolved by ruling, both noted where they land: `--set definitions` (the built-in set with no file override) is refused by `check_sets` — Task 2 — since the spec scopes `--set` to `filters.toml`; and the `lines` summary's `of N files` counts files actually read, not inputs, so a run that lost an input says `of 1 file` (Task 6, matched by the integration test in Task 8).

**Placeholder scan.** No TBD/TODO. Every test and implementation block is complete code. The one "run the test and paste what it prints" step (Task 2, Step 7) is the repo's established procedure for the Usage block and ships with the expected text.

**Type consistency.** `Inputs { files: Vec<PathBuf>, from: Source }` and `Source::{Stdin, Directory(PathBuf), File}` are defined in Task 5 and used unchanged in 6-7. `Exit::Emit { lines, summary, failed }` (Task 4) is what 6-7 construct and 8 delivers. `deliver(requested, quiet, stdout, stderr)` (4) matches `main` (8). `Config::{set, hide, quiet, sets_to_enable, check_sets}` (2) match `App::new` (3), `filters_for` (7) and `main` (8). `EnableError` (3) is what `filters_for` maps into an eyre error (7). `document::{read_lines, is_binary, BINARY_FILE}` (1) are what `reason` and `collect_lines` use (6). `viewport::is_interesting` is made `pub(crate)` in 6 before 6 uses it. `count(n, noun)` is defined in 6 and used in 7.

**Pre-existing, out of scope:** the color-eyre error hook calls `restore_terminal`, which writes the leave-alternate-screen sequence to stderr even when no terminal was ever entered — so a refused flag in headless mode leaves a few escape bytes on stderr before the message. The integration test uses `contains` for that reason. Fixing it (skip the restore when nothing was initialised) is a one-line follow-up outside #143.
