# Emit on Quit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `recon --emit lines|files|cwd [-n]` prints the session's result to stdout when the user quits with `q`, quits silently on `Q`, draws the TUI on stderr so stdout stays clean, and prints a one-line summary to stderr naming the mode and counts.

**Architecture:** `App` decides, `main` does I/O. A new `emit` module holds the `Emit` kind (a clap `ValueEnum`) and the `Exit` value `App::run` now returns; `App::collect` builds an `Exit` from the visible sets the widgets already hold; `main` restores the terminal and calls `Exit::deliver`, which writes stdout and stderr and returns the process exit code. The terminal backend moves from stdout to stderr unconditionally.

**Tech Stack:** Rust 2024, clap 4 (`derive`, `env`), ratatui 0.30 with `CrosstermBackend`, crossterm 0.29, color-eyre.

**Spec:** `docs/specs/2026-09-06-emit-on-quit-design.md` — read it first; every task below argues from it.

## Global Constraints

- `cargo clippy --all-targets` must stay clean under the crate's `pedantic` lint level; `cargo fmt --check` must pass. Run both before every commit.
- Every test file uses the shared fixture registry: `crate::fixtures::fixture_dir(name)` / `fixture_file(name, bytes)`; names are claimed case-insensitively and must be unique across `lib.rs`, `fileview.rs`, `filenav.rs`.
- `every_bound_key_is_documented` in `src/help.rs` fails if a `KeyCode::Char('X')` arm exists in `lib.rs` without a `KEYMAP` row; `readme_usage_block_matches_the_real_help` in `src/config.rs` fails if README's `## Usage` block differs from `recon -h` rendered at 80 columns. Both tests print the exact text to paste.
- The help overlay must still fit a 150-column terminal (`a_normal_terminal_shows_the_whole_keymap`); keep new `KEYMAP` labels and descriptions short.
- Output lines are bytes: a Unix filename is not necessarily UTF-8 and is written as-is.
- Summary lines start with `recon: emitted`. Exact strings are in the spec's "The three outputs" section and repeated in each task below.
- Commit messages follow the repo's conventional style: `feat(emit): …`, `test(emit): …`, `docs(readme): …`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/emit.rs` (new) | `Emit` kind enum, `Exit` value, `Exit::deliver`, `path_bytes`. No knowledge of `App`. |
| `src/config.rs` | `--emit` and `-n/--line-numbers` flags; the `--line-numbers` validation and its `ConfigError` variant. |
| `src/lib.rs` | `AppState::Quit { emit }`, the `Q` key, `run` returning `Exit`, `App::exit`, `App::collect` and its three helpers. |
| `src/viewport.rs` | `App::interesting_count`, beside the existing `is_interesting`. |
| `src/widgets/fileview.rs` | `showing_directory()` and `is_text()` accessors. |
| `src/widgets/filenav.rs` | `ListedFile` and `listed_files()`: the visible rows as absolute paths with their match answer. |
| `src/main.rs` | Backend on stderr; `main` returns `Result<ExitCode>` and delivers the `Exit`. |
| `src/help.rs`, `README.md` | The `Q` row; `--emit` in Usage; the "Emitting the result" section. |

---

### Task 1: The `emit` module — `Emit`, `Exit`, `deliver`

**Files:**
- Create: `src/emit.rs`
- Modify: `src/lib.rs:257-270` (module list)

**Interfaces:**
- Produces: `pub enum Emit { Lines, Files, Cwd }` (`Copy`, `clap::ValueEnum`); `pub enum Exit { Emit { lines: Vec<Vec<u8>>, summary: String }, Silent }`; `Exit::deliver(self, requested: Option<Emit>, stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode`; `pub(crate) fn path_bytes(path: &Path) -> Vec<u8>`.

- [ ] **Step 1: Write the failing tests**

Create `src/emit.rs` with only the tests and the `use` lines, so the first run fails on missing items:

```rust
//! Emit on quit (#143): what a finished session hands back, and how `main`
//! prints it.
//!
//! `App` decides what leaves the process; this module is the only place that
//! knows it leaves through stdout and stderr, and `main` is the only caller.
//! Keeping the writers as parameters is what lets every table row in
//! `Exit::deliver` be tested against a `Vec<u8>` with no terminal.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

#[cfg(test)]
mod tests {
    use super::*;

    fn deliver(exit: Exit, requested: Option<Emit>) -> (Vec<u8>, Vec<u8>, ExitCode) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = exit.deliver(requested, &mut out, &mut err);
        (out, err, code)
    }

    #[test]
    fn an_emit_writes_every_line_newline_terminated_and_the_summary_to_stderr() {
        let exit = Exit::Emit {
            lines: vec![b"one".to_vec(), b"two".to_vec()],
            summary: "recon: emitted 2 lines".to_string(),
        };

        let (out, err, code) = deliver(exit, Some(Emit::Lines));

        assert_eq!(out, b"one\ntwo\n");
        assert_eq!(err, b"recon: emitted 2 lines\n");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn an_empty_emit_is_a_success_with_nothing_on_stdout() {
        let exit = Exit::Emit {
            lines: Vec::new(),
            summary: "recon: emitted 0 files from /d, hide mode".to_string(),
        };

        let (out, err, code) = deliver(exit, Some(Emit::Files));

        assert!(out.is_empty());
        assert_eq!(err, b"recon: emitted 0 files from /d, hide mode\n");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn a_silent_quit_under_emit_writes_nothing_and_fails() {
        let (out, err, code) = deliver(Exit::Silent, Some(Emit::Cwd));

        assert!(out.is_empty());
        assert!(err.is_empty());
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn a_silent_quit_without_emit_writes_nothing_and_succeeds() {
        let (out, err, code) = deliver(Exit::Silent, None);

        assert!(out.is_empty());
        assert!(err.is_empty());
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn lines_are_written_as_bytes_not_re_encoded() {
        let exit = Exit::Emit {
            lines: vec![vec![0xff, 0xfe, b'x']],
            summary: String::new(),
        };

        let (out, _, _) = deliver(exit, Some(Emit::Files));

        assert_eq!(out, vec![0xff, 0xfe, b'x', b'\n']);
    }

    #[cfg(unix)]
    #[test]
    fn path_bytes_keeps_a_non_utf8_filename_intact() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(OsStr::from_bytes(b"/tmp/bad\xffname"));

        assert_eq!(path_bytes(path), b"/tmp/bad\xffname".to_vec());
    }
}
```

Add the module to `src/lib.rs`, in the alphabetical list after `pub mod editor;`:

```rust
pub mod emit;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib emit::tests 2>&1 | grep -E "^error|test result" | head`
Expected: compile errors naming `Exit`, `Emit`, `path_bytes` as not found.

- [ ] **Step 3: Write the implementation**

Insert between the `use` lines and `#[cfg(test)]` in `src/emit.rs`:

```rust
/// What `--emit` asks for. A clap `ValueEnum`: the variant doc comments are
/// the `--help` text for each value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Emit {
    /// The file view's visible lines, in the current mode
    Lines,
    /// The navigator's listed files, one absolute path per line
    Files,
    /// The directory the navigator is showing
    Cwd,
}

/// What a finished session hands back for `main` to print.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    /// `q` with `--emit`: the output and the one-line summary for stderr.
    ///
    /// Lines are bytes, not `String`: a Unix filename is not necessarily
    /// UTF-8, and a consumer hands it straight back to the filesystem, so a
    /// lossy conversion would name a file that does not exist.
    Emit { lines: Vec<Vec<u8>>, summary: String },
    /// `Q`, or any quit without `--emit`.
    Silent,
}

impl Exit {
    /// Print the session's result and say how the process should exit.
    ///
    /// | Exit | `--emit` given | stdout | stderr | code |
    /// |---|---|---|---|---|
    /// | `Emit` | yes | every line, newline-terminated | the summary | 0 |
    /// | `Silent` | yes | nothing | nothing | 1 |
    /// | `Silent` | no | nothing | nothing | 0 |
    ///
    /// `Silent` under `--emit` fails because the caller asked for output and
    /// got none: `dir=$(recon --emit cwd) && cd "$dir"` then skips the `cd`
    /// with no test on `$dir`. Empty output from a real emit is a success —
    /// the summary is what tells the two apart.
    ///
    /// A write error on stdout — a closed pipe, most likely — is reported on
    /// stderr and is a failure. Nothing here can panic on it: the terminal has
    /// already been restored, and a panic's backtrace would be the last thing
    /// the user saw.
    pub fn deliver(
        self,
        requested: Option<Emit>,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> ExitCode {
        match (self, requested) {
            (Self::Emit { lines, summary }, _) => {
                let written = lines
                    .iter()
                    .try_for_each(|line| stdout.write_all(line).and_then(|()| stdout.write_all(b"\n")))
                    .and_then(|()| stdout.flush());
                if let Err(err) = written {
                    let _ = writeln!(stderr, "recon: could not write the output: {err}");
                    return ExitCode::FAILURE;
                }
                let _ = writeln!(stderr, "{summary}");
                ExitCode::SUCCESS
            }
            (Self::Silent, Some(_)) => ExitCode::FAILURE,
            (Self::Silent, None) => ExitCode::SUCCESS,
        }
    }
}

/// A path as the bytes the filesystem holds, for an output line.
///
/// On Unix that is the `OsStr` verbatim. Elsewhere paths are not bytes at
/// all, and the lossy string is the only honest rendering.
#[cfg(unix)]
pub(crate) fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
pub(crate) fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib emit::tests 2>&1 | grep -E "^error|panicked|test result"`
Expected: `test result: ok. 6 passed` (5 on non-Unix).

Run: `cargo clippy --all-targets 2>&1 | grep -cE "^(warning|error)"` — expected `0`. If clippy asks for `#[must_use]` on `deliver` or `path_bytes`, add it.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/emit.rs src/lib.rs
git commit -m "feat(emit): Exit value and its delivery — lines to stdout, summary to stderr, exit code (#143)"
```

---

### Task 2: `--emit` and `-n/--line-numbers` on `Config`

**Files:**
- Modify: `src/config.rs` (`Config` struct after `theme`; `Default` impl; `ConfigError`; `Config::load`; tests)
- Modify: `README.md:186-216` (the `## Usage` block)

**Interfaces:**
- Consumes: `crate::emit::Emit` from Task 1.
- Produces: `Config.emit: Option<Emit>`, `Config.line_numbers: bool`, `ConfigError::LineNumbersNeedLines`, `Config::check_flags(&self) -> Result<(), ConfigError>` (called by `load`).

- [ ] **Step 1: Write the failing tests**

In `src/config.rs`'s `mod tests`, after `default_matches_the_parsed_defaults`:

```rust
    // ---- --emit and --line-numbers (#143) --------------------------------

    #[test]
    fn emit_parses_its_three_values_and_is_unset_by_default() {
        use crate::emit::Emit;
        assert_eq!(Config::try_parse_from(["recon"]).unwrap().emit, None);
        for (flag, kind) in [("lines", Emit::Lines), ("files", Emit::Files), ("cwd", Emit::Cwd)] {
            let config = Config::try_parse_from(["recon", "--emit", flag]).unwrap();
            assert_eq!(config.emit, Some(kind), "{flag}");
        }
        assert!(Config::try_parse_from(["recon", "--emit", "filters"]).is_err());
    }

    #[test]
    fn line_numbers_has_a_short_and_a_long_spelling() {
        assert!(!Config::try_parse_from(["recon"]).unwrap().line_numbers);
        assert!(Config::try_parse_from(["recon", "-n", "--emit", "lines"]).unwrap().line_numbers);
        assert!(
            Config::try_parse_from(["recon", "--line-numbers", "--emit", "lines"])
                .unwrap()
                .line_numbers
        );
    }

    /// `-n` with anything but `--emit lines` is a mistake worth stopping on:
    /// a script that meant `lines` should find out, not get a path list.
    #[test]
    fn line_numbers_is_refused_unless_emitting_lines() {
        use crate::emit::Emit;
        let refused = |emit: Option<Emit>| {
            let config = Config {
                line_numbers: true,
                emit,
                ..Config::default()
            };
            config.check_flags().expect_err("must be refused")
        };
        for emit in [None, Some(Emit::Files), Some(Emit::Cwd)] {
            let err = refused(emit);
            assert!(
                matches!(err, ConfigError::LineNumbersNeedLines),
                "{emit:?}: {err}"
            );
            assert_eq!(err.to_string(), "--line-numbers applies to --emit lines");
        }

        let accepted = Config {
            line_numbers: true,
            emit: Some(Emit::Lines),
            ..Config::default()
        };
        assert!(accepted.check_flags().is_ok());
        assert!(Config::default().check_flags().is_ok(), "neither flag is fine");
    }
```

Also extend `default_matches_the_parsed_defaults` with two lines:

```rust
        assert_eq!(parsed.emit, default.emit);
        assert_eq!(parsed.line_numbers, default.line_numbers);
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib config::tests 2>&1 | grep -E "^error" | head -5`
Expected: errors for the missing fields `emit`, `line_numbers`, `check_flags`, `LineNumbersNeedLines`.

- [ ] **Step 3: Write the implementation**

In the `Config` struct, after the `theme` field:

```rust
    /// Print the session's result to stdout on `q`; `Q` quits without it.
    ///
    /// The TUI draws on stderr, so stdout carries only this — pipe it or
    /// capture it. Every emit also prints one summary line to stderr naming
    /// the mode and the counts.
    #[arg(long, value_name = "WHAT", value_enum)]
    pub emit: Option<crate::emit::Emit>,

    /// With `--emit lines`: prefix each line with its line number and a tab.
    ///
    /// The number is the line's position in the file, so in hide mode the
    /// numbers are the real ones, not 1..N of the output.
    #[arg(short = 'n', long)]
    pub line_numbers: bool,
```

In `impl Default for Config`, add to the struct literal:

```rust
            emit: None,
            line_numbers: false,
```

Add the `ConfigError` variant and its `Display` arm:

```rust
    /// `--line-numbers` without `--emit lines`. Refused rather than ignored:
    /// a script that meant `lines` should find out (#143).
    LineNumbersNeedLines,
```

```rust
            Self::LineNumbersNeedLines => write!(f, "--line-numbers applies to --emit lines"),
```

Add `check_flags` to `impl Config`, directly above `load`, and call it from `load`:

```rust
    /// Refuse flag combinations clap cannot express: `-n` is meaningful only
    /// with `--emit lines`. Here rather than as a clap `requires`, because
    /// `requires` can name a flag but not a flag's *value*.
    pub fn check_flags(&self) -> Result<(), ConfigError> {
        if self.line_numbers && self.emit != Some(crate::emit::Emit::Lines) {
            return Err(ConfigError::LineNumbersNeedLines);
        }
        Ok(())
    }

    pub fn load() -> Result<Self, ConfigError> {
        let mut config = Self::parse();
        config.check_flags()?;
        config.apply(&load_file()?);
        Ok(config)
    }
```

(Keep `load`'s existing doc comment; only the `check_flags()?` line is new.)

- [ ] **Step 4: Run the tests, then regenerate the README usage block**

Run: `cargo test --lib config::tests 2>&1 | grep -E "FAILED|panicked|test result"`
Expected: every test passes except `readme_usage_block_matches_the_real_help`, whose failure message prints the new block. Replace the fenced block under `## Usage` in `README.md` (lines 188–216 today) with exactly what the test printed, then rerun until `test result: ok`.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/config.rs README.md
git commit -m "feat(config): --emit lines|files|cwd and -n/--line-numbers, refused without --emit lines (#143)"
```

---

### Task 3: Two quit keys and a `run` that returns `Exit`

**Files:**
- Modify: `src/lib.rs` — `AppState` (line ~543), `App` fields (~289–437), `App::new` (~582–613), `run` (~968), the `q` arm (~1113), tests
- Modify: `src/help.rs` — the Global section (~line 98)
- Modify: `README.md` — the two Global key tables (`| \`q\` | Quit |` at ~182 and ~311)

**Interfaces:**
- Consumes: `Emit`, `Exit` from Task 1; `Config.emit`, `Config.line_numbers` from Task 2.
- Produces: `AppState::Quit { emit: bool }`; `App.emit: Option<Emit>`; `App.line_numbers: bool`; `App::run(...) -> Result<Exit>`; `pub(crate) fn exit(&self) -> Exit`; a placeholder `fn collect(&self, kind: Emit) -> Exit` that Task 4 fills in. Tests in later tasks drive `exit()` directly rather than `run`.

- [ ] **Step 1: Write the failing tests**

In `src/lib.rs`'s `mod tests`, next to the existing quit test (search for `"q did not quit"`), add:

```rust
    // ---- emit on quit (#143): which quit emits --------------------------

    fn app_emitting(name: &str, emit: Option<emit::Emit>) -> App<'static> {
        let file = fixture_path(name, "alpha\n");
        App::new(&Config {
            path: file.display().to_string(),
            emit,
            ..Config::default()
        })
    }

    #[test]
    fn q_quits_emitting_and_big_q_quits_silently() {
        let mut app = app_emitting("quit_q_emits", Some(emit::Emit::Cwd));
        key(&mut app, KeyCode::Char('q'));
        assert_eq!(app.state, AppState::Quit { emit: true });
        assert!(matches!(app.exit(), emit::Exit::Emit { .. }));

        let mut app = app_emitting("quit_big_q_silent", Some(emit::Emit::Cwd));
        key(&mut app, KeyCode::Char('Q'));
        assert_eq!(app.state, AppState::Quit { emit: false });
        assert_eq!(app.exit(), emit::Exit::Silent);
    }

    /// A terminal reports `Q` with Shift set; the arm must not be guarded on
    /// an empty modifier set (the `?`/`N`/`S` trap).
    #[test]
    fn big_q_quits_with_shift_reported() {
        let mut app = app_emitting("quit_big_q_shift", Some(emit::Emit::Cwd));
        app.handle_event(event::Event::Key(event::KeyEvent::new(
            KeyCode::Char('Q'),
            KeyModifiers::SHIFT,
        )));
        assert_eq!(app.state, AppState::Quit { emit: false });
    }

    #[test]
    fn without_emit_both_quits_are_silent() {
        let mut app = app_emitting("quit_no_emit_q", None);
        key(&mut app, KeyCode::Char('q'));
        assert_eq!(app.exit(), emit::Exit::Silent);

        let mut app = app_emitting("quit_no_emit_big_q", None);
        key(&mut app, KeyCode::Char('Q'));
        assert_eq!(app.exit(), emit::Exit::Silent);
    }

    #[test]
    fn a_running_app_has_not_exited() {
        let app = app_emitting("quit_still_running", Some(emit::Emit::Cwd));
        assert_eq!(app.exit(), emit::Exit::Silent);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- q_quits_emitting big_q_quits without_emit_both a_running_app_has_not 2>&1 | grep -E "^error" | head -5`
Expected: errors on `AppState::Quit { emit: .. }`, `emit` field, `exit()`.

- [ ] **Step 3: Write the implementation**

`AppState` — replace the enum:

```rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum AppState {
    #[default]
    Running,
    /// The user has asked to quit. `emit` is whether they pressed `q` (yes)
    /// or `Q` (no); what that means depends on whether `--emit` was given —
    /// see `App::exit`.
    Quit { emit: bool },
}
```

`App` fields — after `center_jumps: bool,` add:

```rust
    /// What `--emit` asked for, or `None`. Read once, when the session ends.
    emit: Option<emit::Emit>,
    /// `-n`: prefix each emitted line with its source line number and a tab.
    line_numbers: bool,
```

`App::new` — in the struct literal next to `center_jumps: config.center_jumps(),`:

```rust
            emit: config.emit,
            line_numbers: config.line_numbers,
```

`run` — change the signature and the tail:

```rust
    pub fn run<B>(mut self, mut terminal: Terminal<B>) -> Result<emit::Exit>
    where
        B: Backend,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut dirty = true;
        while self.is_running() {
            if dirty {
                terminal.draw(|frame| {
                    let area = frame.area();
                    frame.render_widget(&mut self, area);
                })?;
            }
            dirty = self.handle_events()?;
        }
        Ok(self.exit())
    }

    /// What this session hands back, given how it ended (#143). `Emit`
    /// only when `q` ended it *and* `--emit` was given; a `Q`, a missing
    /// `--emit`, or a session still running is `Silent`.
    pub(crate) fn exit(&self) -> emit::Exit {
        match (self.state, self.emit) {
            (AppState::Quit { emit: true }, Some(kind)) => self.collect(kind),
            _ => emit::Exit::Silent,
        }
    }

    /// The output `--emit <kind>` asks for, from what the panes are showing.
    /// Filled in per kind by the emit tasks; until then nothing is emitted.
    fn collect(&self, kind: emit::Emit) -> emit::Exit {
        let _ = kind;
        emit::Exit::Emit {
            lines: Vec::new(),
            summary: String::from("recon: emitted"),
        }
    }
```

The `q` arm — replace it and add `Q` beside it:

```rust
                KeyCode::Char('q') if key.modifiers.is_empty() => {
                    self.state = AppState::Quit { emit: true };
                    return;
                }
                // `Q` quits without emitting (#143). Without `--emit` it is
                // `q`. Not guarded on an empty modifier set: a terminal
                // reports the Shift that makes it uppercase — the same trap
                // `?`, `N` and `S` document.
                KeyCode::Char('Q')
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.state = AppState::Quit { emit: false };
                    return;
                }
```

Fix every other `AppState::Quit` reference the compiler reports (there is one production site, the `q` arm above, and tests comparing `app.state`).

`src/help.rs` — in the Global section, after the `q` binding:

```rust
            Binding {
                keys: &["Q"],
                action: "Quit, emitting nothing",
            },
```

`README.md` — in both Global tables, after the `q` row:

```markdown
| `Q` | Quit without emitting — the same as `q` unless `--emit` was given |
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test 2>&1 | grep -E "FAILED|panicked|^test result"`
Expected: all pass, including `every_bound_key_is_documented` and the help layout tests. If `a_normal_terminal_shows_the_whole_keymap` fails, shorten the `Q` action text; it must not widen the Global column.

Run: `cargo clippy --all-targets 2>&1 | grep -cE "^(warning|error)"` — expected `0`.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/lib.rs src/help.rs README.md
git commit -m "feat(emit): q quits emitting and Q quits silently; run returns the session's Exit (#143)"
```

---

### Task 4: `--emit lines`

**Files:**
- Modify: `src/lib.rs` — `collect`; tests
- Modify: `src/viewport.rs` — add `interesting_count` after `first_interesting`
- Modify: `src/widgets/fileview.rs` — add `showing_directory()` and `is_text()` accessors near `filename()` (~line 367)

**Interfaces:**
- Consumes: `Exit`, `Emit` (Task 1); `App.line_numbers` (Task 3); `Document::visible()`, `Document::lines()`, `Document::mode()`; `FileView::filename()`.
- Produces: `App::collect_lines(&self) -> Exit`; `App::interesting_count(&self) -> usize`; `FileView::showing_directory(&self) -> bool`; `FileView::is_text(&self) -> bool`.

- [ ] **Step 1: Write the failing tests**

In `src/lib.rs` tests, after the Task 3 tests:

```rust
    // ---- --emit lines --------------------------------------------------

    fn emitted(app: &App) -> (Vec<String>, String) {
        match app.exit() {
            emit::Exit::Emit { lines, summary } => (
                lines
                    .into_iter()
                    .map(|line| String::from_utf8(line).expect("utf-8 fixture"))
                    .collect(),
                summary,
            ),
            emit::Exit::Silent => panic!("the session was silent"),
        }
    }

    fn app_emitting_lines(name: &str, body: &str, line_numbers: bool) -> App<'static> {
        let file = fixture_path(name, body);
        let mut app = App::new(&Config {
            path: file.display().to_string(),
            emit: Some(emit::Emit::Lines),
            line_numbers,
            ..Config::default()
        });
        key(&mut app, KeyCode::Char('t'));
        app
    }

    #[test]
    fn lines_in_dim_mode_emits_every_visible_line_and_counts_the_matches() {
        let mut app = app_emitting_lines("emit_lines_dim", "hit one\nplain\nhit two\n", false);
        app.add_filter("hit").expect("valid");
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        assert_eq!(lines, vec!["hit one", "plain", "hit two"]);
        let name = app.view.filename().display().to_string();
        assert_eq!(
            summary,
            format!("recon: emitted 3 lines of {name}, dim mode (2 match) — Ctrl-H to emit matches only")
        );
    }

    #[test]
    fn lines_in_hide_mode_emits_only_the_matches() {
        let mut app = app_emitting_lines("emit_lines_hide", "hit one\nplain\nhit two\n", false);
        app.add_filter("hit").expect("valid");
        ctrl(&mut app, KeyCode::Char('h'));
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        assert_eq!(lines, vec!["hit one", "hit two"]);
        let name = app.view.filename().display().to_string();
        assert_eq!(summary, format!("recon: emitted 2 lines of {name}, hide mode"));
    }

    /// `-n` numbers are the *source* line numbers — the gutter's — so hide
    /// mode gives `1` and `3`, not `1` and `2`.
    #[test]
    fn line_numbers_are_source_numbers_with_a_tab() {
        let mut app = app_emitting_lines("emit_lines_numbered", "hit one\nplain\nhit two\n", true);
        app.add_filter("hit").expect("valid");
        ctrl(&mut app, KeyCode::Char('h'));
        key(&mut app, KeyCode::Char('q'));

        let (lines, _) = emitted(&app);

        assert_eq!(lines, vec!["1\thit one", "3\thit two"]);
    }

    #[test]
    fn lines_with_no_filter_still_counts_zero_matches() {
        let mut app = app_emitting_lines("emit_lines_nofilter", "a\nb\n", false);
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        assert_eq!(lines, vec!["a", "b"]);
        assert!(summary.contains("dim mode (0 match)"), "{summary}");
    }

    #[test]
    fn lines_over_a_directory_listing_emits_nothing_and_says_so() {
        let dir = fixture_dir("emit_lines_directory");
        fs::write(dir.join("a.txt"), "x\n").expect("write");
        let mut app = App::new(&Config {
            path: dir.display().to_string(),
            emit: Some(emit::Emit::Lines),
            ..Config::default()
        });
        // The navigator starts on `a.txt`; select `..` so the view shows a
        // listing rather than a file.
        key(&mut app, KeyCode::Char('g'));
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        assert!(lines.is_empty());
        assert_eq!(summary, "recon: emitted 0 lines — the view is showing a directory");
    }

    #[test]
    fn lines_over_an_unreadable_file_emits_nothing_and_says_so() {
        let dir = fixture_dir("emit_lines_missing");
        let mut app = App::new(&Config {
            path: dir.join("nope.log").display().to_string(),
            emit: Some(emit::Emit::Lines),
            ..Config::default()
        });
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        assert!(lines.is_empty());
        assert_eq!(summary, "recon: emitted 0 lines — the view is showing an error, not a file");
    }
```

If `g` in the navigator does not land on `..` in the directory test (check `selected_name` after the key), replace it with `for _ in 0..8 { key(&mut app, KeyCode::Up); }` — the point is only that the view ends up showing a directory listing; assert `app.view.showing_directory()` first if in doubt.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- lines_in_dim lines_in_hide line_numbers_are lines_with_no_filter lines_over_a 2>&1 | grep -E "^error|panicked|left|right|test result" | head -12`
Expected: the tests compile (the Task 3 placeholder `collect` exists) and fail on the assertions — empty `lines`, summary `recon: emitted`.

- [ ] **Step 3: Write the implementation**

`src/widgets/fileview.rs`, after `filename()`:

```rust
    /// Whether the pane is showing a directory's listing rather than a file.
    pub(crate) fn showing_directory(&self) -> bool {
        self.showing_directory
    }

    /// Whether the pane is showing a file's own text — as opposed to a
    /// listing, or a message standing in for a file that could not be shown.
    pub(crate) fn is_text(&self) -> bool {
        self.text
    }
```

`src/viewport.rs`, after `first_interesting`:

```rust
    /// How many lines are interesting — the count `--emit`'s summary reports
    /// as "N match" (#143). The same definition `n` steps by.
    pub(crate) fn interesting_count(&self) -> usize {
        self.document
            .verdicts()
            .iter()
            .filter(|verdict| is_interesting(verdict))
            .count()
    }
```

`src/lib.rs` — replace the placeholder `collect` with the dispatcher and add `collect_lines`:

```rust
    /// The output `--emit <kind>` asks for, from what the panes are showing
    /// (#143). Reads the visible sets; computes nothing new.
    fn collect(&self, kind: emit::Emit) -> emit::Exit {
        match kind {
            emit::Emit::Lines => self.collect_lines(),
            emit::Emit::Files => self.collect_files(),
            emit::Emit::Cwd => self.collect_cwd(),
        }
    }

    /// `--emit lines`: the file view's visible lines in the current mode,
    /// with `-n` prefixing each by its 1-based source line number and a tab.
    fn collect_lines(&self) -> emit::Exit {
        if self.view.showing_directory() {
            return emit::Exit::Emit {
                lines: Vec::new(),
                summary: "recon: emitted 0 lines — the view is showing a directory".to_string(),
            };
        }
        if !self.view.is_text() {
            return emit::Exit::Emit {
                lines: Vec::new(),
                summary: "recon: emitted 0 lines — the view is showing an error, not a file"
                    .to_string(),
            };
        }
        let text = self.document.lines();
        let visible = self.document.visible();
        let lines = visible
            .iter()
            .map(|&source| {
                let mut line = Vec::new();
                if self.line_numbers {
                    line.extend_from_slice(format!("{}\t", source + 1).as_bytes());
                }
                line.extend_from_slice(text[source].as_bytes());
                line
            })
            .collect();
        let name = self.view.filename().display();
        let summary = match self.document.mode() {
            Mode::Dimmed => format!(
                "recon: emitted {} lines of {name}, dim mode ({} match) — Ctrl-H to emit matches only",
                visible.len(),
                self.interesting_count(),
            ),
            Mode::FilteredOnly => {
                format!("recon: emitted {} lines of {name}, hide mode", visible.len())
            }
        };
        emit::Exit::Emit { lines, summary }
    }

    /// `--emit files`. Filled in by the next task.
    fn collect_files(&self) -> emit::Exit {
        emit::Exit::Emit {
            lines: Vec::new(),
            summary: String::from("recon: emitted"),
        }
    }

    /// `--emit cwd`. Filled in by the next task.
    fn collect_cwd(&self) -> emit::Exit {
        emit::Exit::Emit {
            lines: Vec::new(),
            summary: String::from("recon: emitted"),
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test 2>&1 | grep -E "FAILED|panicked|^test result"`
Expected: all pass. Then `cargo clippy --all-targets 2>&1 | grep -cE "^(warning|error)"` — `0`.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/lib.rs src/viewport.rs src/widgets/fileview.rs
git commit -m "feat(emit): --emit lines — the visible lines in the current mode, -n for source numbers (#143)"
```

---

### Task 5: `--emit files` and `--emit cwd`

**Files:**
- Modify: `src/widgets/filenav.rs` — `ListedFile` and `listed_files()` after `files()` (~line 563); tests
- Modify: `src/lib.rs` — `collect_files`, `collect_cwd`; tests

**Interfaces:**
- Consumes: `path_bytes` (Task 1); `FileNav::dir()`; `Match`; `ActiveFilters::any_enabled()`; `Document::mode()`.
- Produces: `pub(crate) struct ListedFile { pub path: PathBuf, pub matched: Option<bool> }`; `FileNav::listed_files(&self) -> Vec<ListedFile>`.

- [ ] **Step 1: Write the failing tests**

`src/widgets/filenav.rs` tests, after `files_lists_every_non_directory_with_its_index_and_absolute_path`:

```rust
    /// `listed_files` is `files` restricted to the rows on screen: hide
    /// mode drops the `No` answers, directories and `..` are never files, and
    /// the answer travels with the path so the caller can count matches
    /// without a second pass over the entries (#143).
    #[test]
    fn listed_files_follows_the_visible_rows_and_carries_the_answer() {
        let mut nav = nav_over("listed_files", &["no.log", "unk.log", "yes.log"]);
        std::fs::create_dir_all(nav.dir().join("sub")).expect("subdir");
        let mut nav = FileNav::new(nav.dir().join("placeholder").display().to_string());
        let idx = |nav: &FileNav<'_>, name: &str| {
            nav.entries.iter().position(|e| e.name == name).expect(name)
        };
        nav.set_answer(idx(&nav, "no.log"), Match::No);
        nav.set_answer(idx(&nav, "yes.log"), Match::Yes(Style::default()));
        nav.restyle();

        let dim: Vec<(String, Option<bool>)> = nav
            .listed_files()
            .into_iter()
            .map(|f| (f.path.file_name().unwrap().to_string_lossy().into_owned(), f.matched))
            .collect();
        assert_eq!(
            dim,
            vec![
                ("no.log".to_string(), Some(false)),
                ("unk.log".to_string(), None),
                ("yes.log".to_string(), Some(true)),
            ],
            "dim mode lists every file, with its answer"
        );
        assert!(
            nav.listed_files().iter().all(|f| f.path.is_absolute()),
            "paths are absolute"
        );

        nav.set_mode(Mode::FilteredOnly);
        let hidden: Vec<String> = nav
            .listed_files()
            .into_iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            hidden,
            vec!["unk.log".to_string(), "yes.log".to_string()],
            "hide mode drops the No, keeps the unscanned"
        );
    }
```

(`nav_over`, `set_answer`, `restyle`, `set_mode` all exist; `Mode` and `Style` are already imported in that test module — check the `use` lines at the top of `mod tests` and add `use crate::document::Mode;` if it is only imported at module level.)

`src/lib.rs` tests, after the Task 4 tests:

```rust
    // ---- --emit files and --emit cwd -----------------------------------

    fn app_emitting_files(name: &str) -> (App<'static>, Sender<scan::Scanned>) {
        let mut app = app_over_files(
            name,
            &[
                ("a.log", "hit\n"),
                ("b.log", "plain\n"),
                ("c.log", "hit\n"),
            ],
        );
        app.emit = Some(emit::Emit::Files);
        let (_scanner, tx) = record_scans(&mut app);
        app.add_filter("hit").expect("valid");
        app.refresh_scan(false);
        (app, tx)
    }

    #[test]
    fn files_in_dim_mode_emits_every_listed_file_with_the_counts() {
        let (mut app, tx) = app_emitting_files("emit_files_dim");
        mark(&mut app, &tx, 0, true);
        mark(&mut app, &tx, 1, false);
        // c.log deliberately unscanned.
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        let dir = app.nav.dir().display().to_string();
        assert_eq!(
            lines,
            vec![
                format!("{dir}/a.log"),
                format!("{dir}/b.log"),
                format!("{dir}/c.log")
            ]
        );
        assert_eq!(
            summary,
            format!(
                "recon: emitted 3 files from {dir}, dim mode (1 match, 1 unscanned) — Ctrl-H to emit matches only"
            )
        );
    }

    #[test]
    fn files_omits_the_unscanned_count_once_the_scan_is_complete() {
        let (mut app, tx) = app_emitting_files("emit_files_scanned");
        mark(&mut app, &tx, 0, true);
        mark(&mut app, &tx, 1, false);
        mark(&mut app, &tx, 2, true);
        key(&mut app, KeyCode::Char('q'));

        let (_, summary) = emitted(&app);

        let dir = app.nav.dir().display().to_string();
        assert_eq!(
            summary,
            format!("recon: emitted 3 files from {dir}, dim mode (2 match) — Ctrl-H to emit matches only")
        );
    }

    #[test]
    fn files_in_hide_mode_emits_only_the_matches() {
        let (mut app, tx) = app_emitting_files("emit_files_hide");
        mark(&mut app, &tx, 0, true);
        mark(&mut app, &tx, 1, false);
        mark(&mut app, &tx, 2, true);
        ctrl(&mut app, KeyCode::Char('h'));
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        let dir = app.nav.dir().display().to_string();
        assert_eq!(lines, vec![format!("{dir}/a.log"), format!("{dir}/c.log")]);
        assert_eq!(summary, format!("recon: emitted 2 files from {dir}, hide mode"));
    }

    #[test]
    fn files_with_no_filter_says_so_instead_of_counting() {
        let mut app = app_over_files("emit_files_nofilter", &[("a.log", "x\n"), ("b.log", "y\n")]);
        app.emit = Some(emit::Emit::Files);
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        assert_eq!(lines.len(), 2);
        let dir = app.nav.dir().display().to_string();
        assert_eq!(summary, format!("recon: emitted 2 files from {dir}, dim mode, no filter"));
    }

    #[cfg(unix)]
    #[test]
    fn files_writes_a_non_utf8_name_as_its_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let dir = fixture_dir("emit_files_bytes");
        let odd = std::ffi::OsStr::from_bytes(b"bad\xffname.log");
        fs::write(dir.join(odd), "x\n").expect("write");
        let mut app = App::new(&Config {
            path: dir.join("placeholder").display().to_string(),
            emit: Some(emit::Emit::Files),
            ..Config::default()
        });
        key(&mut app, KeyCode::Char('q'));

        let emit::Exit::Emit { lines, .. } = app.exit() else {
            panic!("silent");
        };

        let expected = emit::path_bytes(&dir.join(odd));
        assert_eq!(lines, vec![expected]);
    }

    #[test]
    fn cwd_emits_the_navigator_s_directory() {
        let mut app = app_over_files("emit_cwd", &[("a.log", "x\n")]);
        app.emit = Some(emit::Emit::Cwd);
        key(&mut app, KeyCode::Char('q'));

        let (lines, summary) = emitted(&app);

        let dir = app.nav.dir().display().to_string();
        assert_eq!(lines, vec![dir.clone()]);
        assert_eq!(summary, format!("recon: emitted {dir}"));
        assert!(app.nav.dir().is_absolute());
    }
```

`emitted` is the helper from Task 4. `mark`, `record_scans`, `app_over_files`, `Sender` and `scan` are already in scope in the test module.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- listed_files_follows files_in_dim files_omits files_in_hide files_with_no_filter files_writes cwd_emits 2>&1 | grep -E "^error|panicked|test result" | head -8`
Expected: a compile error on `listed_files` / `ListedFile` for the filenav test; the `lib.rs` tests compile and fail on empty output.

- [ ] **Step 3: Write the implementation**

`src/widgets/filenav.rs`, after `files()`:

```rust
/// One row of the listing as `--emit files` sees it (#143).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedFile {
    /// Absolute.
    pub path: PathBuf,
    /// `Some(true)` for a `Yes` answer, `Some(false)` for `No`, `None` while
    /// unscanned — which hide mode keeps, since it might still match.
    pub matched: Option<bool>,
}
```

and in `impl FileNav`:

```rust
    /// The files among the rows currently on screen, in row order (#143).
    /// `files` reads every entry, because the scanner needs them all; this
    /// reads the visible rows, so hide mode has already dropped the `No`s.
    pub(crate) fn listed_files(&self) -> Vec<ListedFile> {
        self.visible
            .iter()
            .filter_map(|&index| self.entries.get(index))
            .filter(|entry| !matches!(entry.kind, Kind::Dir | Kind::Parent))
            .map(|entry| ListedFile {
                path: self.dir.join(&entry.name),
                matched: match entry.matched {
                    Match::Yes(_) => Some(true),
                    Match::No => Some(false),
                    Match::Unknown => None,
                },
            })
            .collect()
    }
```

`src/lib.rs` — replace the two placeholders:

```rust
    /// `--emit files`: the navigator's listed files as absolute paths. Hide
    /// mode has already dropped the non-matching rows, so the list is the
    /// matches; dim mode lists every file and the summary says how many
    /// match, and how many the scan has not answered yet.
    fn collect_files(&self) -> emit::Exit {
        let listed = self.nav.listed_files();
        let lines = listed
            .iter()
            .map(|file| emit::path_bytes(&file.path))
            .collect();
        let dir = self.nav.dir().display();
        let count = listed.len();
        let summary = if !self.filters.any_enabled() {
            format!("recon: emitted {count} files from {dir}, dim mode, no filter")
        } else {
            match self.document.mode() {
                Mode::FilteredOnly => format!("recon: emitted {count} files from {dir}, hide mode"),
                Mode::Dimmed => {
                    let matched = listed.iter().filter(|f| f.matched == Some(true)).count();
                    let unscanned = listed.iter().filter(|f| f.matched.is_none()).count();
                    let counts = if unscanned == 0 {
                        format!("{matched} match")
                    } else {
                        format!("{matched} match, {unscanned} unscanned")
                    };
                    format!(
                        "recon: emitted {count} files from {dir}, dim mode ({counts}) — Ctrl-H to emit matches only"
                    )
                }
            }
        };
        emit::Exit::Emit { lines, summary }
    }

    /// `--emit cwd`: the directory the navigator is showing, one line.
    fn collect_cwd(&self) -> emit::Exit {
        let dir = self.nav.dir();
        emit::Exit::Emit {
            lines: vec![emit::path_bytes(dir)],
            summary: format!("recon: emitted {}", dir.display()),
        }
    }
```

The "no filter" branch reads "dim mode, no filter" in hide mode too, which is fine: with nothing enabled, hide mode hides nothing (`Unknown` rows stay), so the listed set is the same either way. If the mode should still be named accurately, use `self.document.mode()` to pick "dim mode"/"hide mode" and append ", no filter" — either satisfies the spec; the test above pins the dim wording.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test 2>&1 | grep -E "FAILED|panicked|^test result"`
Expected: all pass. `cargo clippy --all-targets 2>&1 | grep -cE "^(warning|error)"` — `0`. If clippy flags `format!` inside `.map` or asks for `#[must_use]`, follow it.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/lib.rs src/widgets/filenav.rs
git commit -m "feat(emit): --emit files (listed files, absolute) and --emit cwd, with the mode summary (#143)"
```

---

### Task 6: `main` — the TUI on stderr, the result on stdout, the exit code

**Files:**
- Modify: `src/main.rs` — `main`, `init_terminal`, `restore_terminal`, the `use` lines

**Interfaces:**
- Consumes: `App::run -> Result<Exit>` (Task 3); `Exit::deliver` (Task 1); `Config.emit` (Task 2).

No unit test reaches `main`; this task's verification is manual and is written out below. Do not skip it.

- [ ] **Step 1: Write the implementation**

Replace the `use std::io::{self, Stdout};` line and the three functions:

```rust
use std::io::{self, Stderr};
use std::process::ExitCode;
```

```rust
fn main() -> Result<ExitCode> {
    install_error_hooks()?;

    setup_logging();
    let mut config = Config::load()?;
    config.filter_sets = recon::filtersets::load_file()?;

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

    let terminal = init_terminal()?;
    let exit = App::new(&config).run(terminal)?;
    restore_terminal()?;

    // Only now, with the alternate screen gone, does anything reach stdout:
    // the session's result, if `--emit` asked for one, and the summary that
    // names its mode on stderr (#143).
    Ok(exit.deliver(config.emit, &mut io::stdout(), &mut io::stderr()))
}
```

```rust
/// The TUI draws on **stderr** (#143), so stdout carries nothing but what
/// `--emit` asks for and can be piped or captured while the TUI is up — the
/// same arrangement fzf uses. Unconditional rather than switched on whether
/// stdout is a terminal: one code path, and a difference nobody could see.
fn init_terminal() -> Result<Terminal<CrosstermBackend<Stderr>>> {
    enable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, LeaveAlternateScreen, DisableMouseCapture)?;
    // terminal.show_cursor()?;
    Ok(())
}
```

Leave the `show_cursor` comment as it is; #205 owns it.

- [ ] **Step 2: Build and run the automated checks**

Run: `cargo build 2>&1 | grep -E "^(warning|error)" -A 5 | head`, then `cargo test 2>&1 | grep -E "FAILED|^test result"`, then `cargo clippy --all-targets 2>&1 | grep -cE "^(warning|error)"`.
Expected: clean build, all tests pass, `0` warnings.

- [ ] **Step 3: Verify by hand — the TUI on stderr and the result on stdout**

In a real terminal, from the repo root:

```bash
cargo run -q -- --emit cwd . > /tmp/recon-cwd.out; echo "exit=$?"; cat /tmp/recon-cwd.out
```

Expected: the TUI appears normally even though stdout is redirected. Press `q`. The shell prints `exit=0`, the file holds the absolute path of the repo root, and one line `recon: emitted /…/recon` appeared on the terminal.

```bash
cargo run -q -- --emit cwd . > /tmp/recon-cwd.out; echo "exit=$?"; wc -c /tmp/recon-cwd.out
```

Press `Q`. Expected: `exit=1`, the file is empty, nothing on stderr after the TUI.

```bash
cargo run -q -- --emit lines -n README.md | head -3
```

Type `/Usage` `Enter`, press `Ctrl-H`, press `q`. Expected: three numbered lines containing `Usage`, and the summary `recon: emitted N lines of README.md, hide mode` on the terminal.

```bash
cargo run -q -- -n README.md; echo "exit=$?"
```

Expected: no TUI; `Error: --line-numbers applies to --emit lines` (via color-eyre) and a non-zero exit.

```bash
cargo run -q -- README.md
```

Press `q`. Expected: exactly the pre-change behaviour — the TUI, then a clean prompt, nothing printed, `exit=0`.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add src/main.rs
git commit -m "feat(emit): the TUI draws on stderr; main delivers the session's result and exit code (#143)"
```

---

### Task 7: README — "Emitting the result"

**Files:**
- Modify: `README.md` — the Table of Contents (~line 31–44); a new `## Emitting the result` section between `## Peeking at the plain file` and `## Opening an editor`

**Interfaces:**
- Consumes: the behaviour of Tasks 1–6; the summary strings are quoted verbatim from the spec.

- [ ] **Step 1: Add the ToC entry**

After `- [Peeking at the plain file](#peeking-at-the-plain-file)` (add that line too if the ToC lacks it), insert:

```markdown
- [Emitting the result](#emitting-the-result)
```

- [ ] **Step 2: Write the section**

Insert before `## Opening an editor`:

````markdown
## Emitting the result

recon is good at finding things; `--emit` is how the result leaves with the
process. The TUI draws on stderr, so stdout is free for it — pipe it, capture
it, or read it off the terminal.

```sh
recon --emit lines app.log | sort | uniq -c      # the visible lines
recon --emit files /var/log | xargs wc -l        # the listed files
dir="$(recon --emit cwd)" && cd "$dir"           # where you ended up
```

| `--emit` | Prints, on `q` | Summary on stderr |
| --- | --- | --- |
| `lines` | the file view's visible lines, verbatim, in the current mode | `recon: emitted 812 lines of app.log, dim mode (27 match) — Ctrl-H to emit matches only` |
| `files` | the navigator's listed files, one absolute path per line, in navigator order | `recon: emitted 14 files from /var/log, dim mode (3 match, 2 unscanned) — Ctrl-H to emit matches only` |
| `cwd` | the directory the navigator is showing | `recon: emitted /var/log` |

**`q` emits, `Q` doesn't.** `Q` quits without printing and exits 1 when
`--emit` was given, so an aborted browse never `cd`s anywhere and a pipeline
under `set -e` stops. Without `--emit`, `Q` is `q`.

**The mode is the trap.** Dim mode emits everything on screen — every line of
the file, every file in the listing; hide mode emits only the matches. The
output cannot say which, so every emit prints one summary line to stderr
naming the mode and the counts, and it reaches the terminal even when stdout is
piped or captured. `dim mode (3 match)` when you wanted the three is the cue to
press `Ctrl-H` and quit again. `unscanned` appears while the navigator's scan
is still running: those files might match. Empty output is legitimate — hide
mode with no matches emits nothing and exits 0.

**`-n` numbers the lines.** With `--emit lines`, `-n` (or `--line-numbers`)
prefixes each line with its line number in the file and a tab, so hide mode
gives the real numbers rather than 1..N of the output:

```sh
recon --emit lines -n app.log | cut -f1          # the matching line numbers
recon --emit lines -n app.log | cut -f2-         # the text again
```

A tab rather than `grep -n`'s colon: a line can contain colons, a tab never
appears in a line number, and the text after it is byte-for-byte the file's.
`-n` with anything but `--emit lines` is refused at startup.

The shell function the `cwd` output exists for:

```sh
rcn() {
  local dir
  dir="$(recon --emit cwd "$@")" && cd "$dir"
}
```

Filenames are written as the bytes the filesystem holds, so a name that is
not valid UTF-8 comes out unchanged. Not emitted yet: the filter set itself,
and a structured format that carries the mode and each file's matching filter
in the output — both are follow-ups to #143, along with a headless mode that
takes the same `--emit` without starting the TUI.
````

- [ ] **Step 3: Check the README against the binary**

Run: `cargo test --lib -- readme every_bound_key 2>&1 | grep -E "FAILED|test result"`
Expected: both pass (the Usage block was regenerated in Task 2; `Q` was documented in Task 3). Also check the Known Limitations section for any sentence the feature now contradicts (search for "stdout" and "quit") and amend it.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs(readme): emitting the result — --emit, -n, the summary line, Q, and rcn (#143)"
```

---

## Self-review against the spec

- **Decisions 1–8:** three kinds (Task 1), flag-says-what/key-says-whether (Tasks 2–3), paths only (Task 5), stderr summary (Tasks 4–5), stderr backend (Task 6), `Q` exits 1 (Task 1's `deliver`, Task 3's key), `App` decides/`main` prints (Tasks 3, 6), `-n` with a tab (Tasks 2, 4).
- **The command line:** `--emit` and `-n` with `--help` text from the variant docs (Task 2); `--line-numbers` refused at startup in `Config::load` (Task 2, verified by hand in Task 6).
- **The terminal:** backend and restore on stderr; eyre hooks untouched (Task 6).
- **What `run` returns:** `Exit`, `AppState::Quit { emit }`, `collect`, `deliver` table (Tasks 1, 3).
- **The three outputs and their summaries:** every string in the spec appears in a test assertion (Tasks 4–5), including the directory and error cases, unscanned, no filter, and non-UTF-8 names.
- **Common rules:** trailing newline on every line and nothing on stdout for `Silent` (Task 1 tests); nothing before `restore_terminal` (Task 6 ordering).
- **The shell function, testing, documentation:** Tasks 6–7.
- **Not covered by an automated test, by design:** that a frame actually draws on the stderr backend. The spec asked for a render_smoke test; `init_terminal` lives in the binary and needs a real tty, so Task 6 verifies it by hand instead and says so.
