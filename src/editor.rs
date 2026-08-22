//! Handing the selected file off to an editor.
//!
//! Four separable pieces, in the order a keypress travels through them:
//!
//! 1. [`project_root`] — walk up from the file to the enclosing project.
//! 2. [`split_template`] — turn the user's command template into argv, **once**.
//! 3. [`substitute`] — fill `{project}` / `{file}` / `{line}` into whole argv
//!    entries.
//! 4. [`Launcher`] — spawn the result, detached, and report how it went.
//!
//! Steps 2 and 3 are separate on purpose, and their order is the whole security
//! story. Splitting happens before any path is in the string, so a path
//! containing a space, a quote or a `$` cannot influence how the command is
//! split — there is nothing left to split by the time it arrives. Nothing is
//! ever handed to `sh -c`, so there is no second parser to get this wrong
//! either. See `docs/specs/2026-08-22-opening-an-editor.md`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// Files and directories that mark the root of a project.
///
/// `.git` is listed as a name rather than a directory: in a linked worktree it
/// is a *file* holding a `gitdir:` pointer, and a check for a directory alone
/// would walk straight past every worktree this repo's own `/work-issue` flow
/// creates.
///
/// Ordered cheapest-and-commonest first, though the walk tests every marker at
/// each level before climbing, so the order is presentation only.
const PROJECT_MARKERS: [&str; 6] = [
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "go.mod",
];

/// The compiled-in bottom of the `o` ladder.
pub const DEFAULT_PROJECT_TEMPLATE: &str = "zed {project} {file}:{line}";

/// Find the project enclosing `path`.
///
/// Walks up from `path`'s directory — or from `path` itself when it is one —
/// and stops at the first ancestor holding any of [`PROJECT_MARKERS`]. Falls
/// back to that starting directory, so `o` always has somewhere to point: a
/// stray log in `/var/log` opens `/var/log`, which is a worse answer than a
/// project root and a much better one than nothing happening.
///
/// Deliberately hand-rolled rather than delegating to `git2::Repository::discover`.
/// That pulls libgit2 in for one function and only ever finds `.git`, which
/// misses every non-git project in the table above.
pub fn project_root(path: &Path) -> PathBuf {
    // Absolutised first, and that is a correctness step rather than a tidying
    // one. `Path::parent` walks a *relative* path down to `""` and stops, and
    // `"".join("Cargo.toml")` tests the **current working directory** — so a
    // relative argument would silently climb out of its own path and into
    // whatever project recon happened to be launched from. `open_in_editor`
    // already passes an absolute path; this makes the function safe for anyone
    // who does not.
    //
    // `absolute` rather than `canonicalize`: no filesystem access, no symlink
    // resolution, and the project reported is the one on the path the user
    // navigated. Falls back to the path as given if the cwd is unreadable,
    // which is the only way it can fail.
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let start = if absolute.is_dir() {
        absolute.as_path()
    } else {
        absolute.parent().unwrap_or(absolute.as_path())
    };

    let mut dir = start;
    loop {
        if PROJECT_MARKERS
            .iter()
            .any(|marker| dir.join(marker).exists())
        {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            // The filesystem root, with no marker anywhere on the way up.
            None => return start.to_path_buf(),
        }
    }
}

/// Why a command template could not be turned into argv.
#[derive(Debug, PartialEq, Eq)]
pub enum TemplateError {
    /// A `'` or `"` was opened and never closed.
    UnterminatedQuote { quote: char },
    /// The template is empty, or is nothing but whitespace, so there is no
    /// program to run.
    Empty,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedQuote { quote } => {
                write!(f, "editor template has an unclosed {quote} quote")
            }
            Self::Empty => write!(f, "editor template is empty"),
        }
    }
}

impl std::error::Error for TemplateError {}

/// Split a command template into argv, honouring quotes.
///
/// The rules are POSIX-shell **grouping** and nothing else — no variable
/// expansion, no globbing, no command substitution, no operators. `'`…`'` is
/// literal throughout; `"`…`"` allows `\` to escape a `"` or a `\`; a `\`
/// outside quotes escapes whatever follows it.
///
/// Grouping alone is what the `osascript -e '…'` terminal-editor templates
/// need: the whole AppleScript has to survive as a single argv entry, and a
/// plain whitespace split shatters it into fifteen. Everything else a shell
/// does is not merely unnecessary here but actively unwanted — see the module
/// docs on why substitution deliberately happens *after* this.
///
/// A returned error is a startup or keypress failure, not something to paper
/// over: a template with an unclosed quote is a typo, and running the
/// truncated-but-plausible command it would otherwise produce is how a
/// half-quoted path ends up as three arguments to an editor.
pub fn split_template(template: &str) -> Result<Vec<String>, TemplateError> {
    let mut argv = Vec::new();
    let mut current = String::new();
    // Distinct from `current.is_empty()`: `zed ''` is an empty *argument*, which
    // has been started and must be emitted, while the whitespace before it
    // started nothing.
    let mut started = false;
    let mut chars = template.chars();

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    argv.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => return Err(TemplateError::UnterminatedQuote { quote: '\'' }),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        // Only `"` and `\` are escapable inside double quotes,
                        // matching the shell. A `\n` stays two characters, so a
                        // Windows-style path in a template survives verbatim
                        // rather than growing a newline.
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '\\')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err(TemplateError::UnterminatedQuote { quote: '"' }),
                        },
                        Some(c) => current.push(c),
                        None => return Err(TemplateError::UnterminatedQuote { quote: '"' }),
                    }
                }
            }
            '\\' => {
                started = true;
                match chars.next() {
                    Some(escaped) => current.push(escaped),
                    // A trailing backslash escapes nothing. Kept literal rather
                    // than erroring: it is unambiguous, unlike an open quote.
                    None => current.push('\\'),
                }
            }
            c => {
                started = true;
                current.push(c);
            }
        }
    }

    if started {
        argv.push(current);
    }
    if argv.is_empty() {
        return Err(TemplateError::Empty);
    }
    Ok(argv)
}

/// Fill the placeholders in each argv entry.
///
/// Substitution is a **single left-to-right pass** rather than three
/// `str::replace` calls, and that is load-bearing twice over:
///
/// - Text that has just been substituted in is never re-scanned, so a file
///   literally named `{line}` is passed through instead of being replaced by
///   the cursor's line number.
/// - An unrecognised `{placeholder}` is copied verbatim, because only the three
///   known names are ever consumed. Silently emptying it would turn
///   `code --goto {ln}` into `code --goto` and leave the user hunting for a
///   typo the tool had already noticed.
///
/// Entries are substituted whole, never re-split, so `{file}:{line}` stays one
/// argument no matter what the path contains.
pub fn substitute(argv: &[String], project: &Path, file: &Path, line: usize) -> Vec<String> {
    let project = project.display().to_string();
    let file = file.display().to_string();
    let line = line.to_string();

    argv.iter()
        .map(|entry| {
            let mut out = String::with_capacity(entry.len());
            let mut rest = entry.as_str();
            while let Some(open) = rest.find('{') {
                out.push_str(&rest[..open]);
                rest = &rest[open..];
                // An unclosed `{` is not a placeholder at all; the rest of the
                // entry is ordinary text and the loop must end rather than spin.
                let Some(close) = rest.find('}') else { break };
                match &rest[1..close] {
                    "project" => out.push_str(&project),
                    "file" => out.push_str(&file),
                    "line" => out.push_str(&line),
                    // Unknown: copied through braces and all.
                    _ => out.push_str(&rest[..=close]),
                }
                rest = &rest[close + 1..];
            }
            out.push_str(rest);
            out
        })
        .collect()
}

/// Derive the file-only template from the project one by dropping the
/// `{project}` entry.
///
/// A filter over split argv, not string surgery: `{project}` is only ever a
/// whole argument in the templates this supports, so removing it is exact and
/// cannot disturb `{file}:{line}` sitting beside it.
///
/// ```text
/// zed {project} {file}:{line}      ->  zed {file}:{line}
/// code {project} -g {file}:{line}  ->  code -g {file}:{line}
/// idea --line {line} {file}        ->  unchanged
/// ```
///
/// **Known limit:** a template where `{project}` is the *value* of a flag —
/// `wezterm cli spawn --cwd {project} -- nvim {file}` — loses the value and
/// keeps the dangling `--cwd`. Deriving cannot know which entries are flags
/// without knowing every editor's CLI, which is exactly the enum this design
/// refuses to hard-code. Anyone writing such a template writes `editor.file`
/// explicitly; `--print-editor-config` emits both keys for that reason.
pub fn drop_project(argv: &[String]) -> Vec<String> {
    argv.iter()
        .filter(|entry| entry.as_str() != "{project}")
        .cloned()
        .collect()
}

/// The whole pipeline for one keypress: split, then substitute.
///
/// This is the seam the tests aim at. It is pure — no process is spawned and no
/// filesystem is touched — so every documented editor gets a table-driven unit
/// test asserting its exact argv, which is the only way to test editor support
/// at all in CI.
///
/// Returns `Result` rather than the bare `Vec<String>` the issue sketched: a
/// template with an unclosed quote has no correct argv, and the alternatives
/// are a panic in the TUI or a silently truncated command.
pub fn editor_command(
    template: &str,
    project: &Path,
    file: &Path,
    line: usize,
) -> Result<Vec<String>, TemplateError> {
    Ok(substitute(&split_template(template)?, project, file, line))
}

/// The two templates, resolved.
///
/// Held as strings rather than pre-split argv so an error in one of them is
/// reported when the key is actually pressed, naming the key that failed.
/// Failing at startup instead would refuse to run a log viewer over a typo in a
/// setting most sessions never use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Templates {
    /// `o` — the project template.
    pub project: String,
    /// `O` — the file-only template (#41).
    pub file: String,
}

/// Nothing configured anywhere: the bottom rung of both ladders. Written
/// through `resolve` rather than restating the two strings, so the default can
/// never drift from what an unconfigured startup actually produces.
impl Default for Templates {
    fn default() -> Self {
        Self::resolve(None, None, None, None)
    }
}

impl Templates {
    /// Run both ladders.
    ///
    /// `recon_project` and `recon_file` are what `clap` already resolved from
    /// `--editor` / `RECON_EDITOR` and `--file-editor` / `RECON_FILE_EDITOR`,
    /// folded together with the config file by `Config::apply`. Everything
    /// below them is decided here:
    ///
    /// ```text
    /// o:  --editor / RECON_EDITOR / config  >  $VISUAL / $EDITOR  >  default
    /// O:  --file-editor / RECON_FILE_EDITOR / config
    ///       >  derive from the o template  >  $VISUAL / $EDITOR  >  default
    /// ```
    ///
    /// **`$VISUAL`/`$EDITOR` sit below the config file**, unlike `RECON_EDITOR`
    /// above it. They are not recon's variables: someone with a global
    /// `EDITOR=vim` who has *also* written an editor template into recon's
    /// config plainly meant the config to win.
    ///
    /// The environment arrives as arguments rather than being read here, the
    /// same rule `config_path_from` follows — `std::env::set_var` is
    /// process-global and `unsafe` in edition 2024, and these tests run in
    /// parallel.
    pub fn resolve(
        recon_project: Option<&str>,
        recon_file: Option<&str>,
        visual: Option<&str>,
        editor: Option<&str>,
    ) -> Self {
        // `$VISUAL` before `$EDITOR` is the long-standing Unix convention:
        // `$EDITOR` may name a line editor for a dumb terminal, `$VISUAL` a
        // full-screen one, and recon is unambiguously the full-screen case.
        let generic = visual
            .or(editor)
            .filter(|value| !value.trim().is_empty())
            .map(generic_editor_template);

        let project = recon_project
            .map(str::to_string)
            .or_else(|| generic.clone())
            .unwrap_or_else(|| DEFAULT_PROJECT_TEMPLATE.to_string());

        let file = recon_file
            .map(str::to_string)
            // Derived from what the user configured for `o`, not from `project`
            // above: deriving from a `$VISUAL` fallback would drop a
            // `{project}` that is not there, and deriving from the compiled-in
            // default would outrank `$VISUAL` — putting a rung of the ladder
            // out of order in the one case it was written to settle.
            .or_else(|| recon_project.and_then(derive_file_template))
            .or_else(|| generic.clone())
            .unwrap_or_else(|| {
                derive_file_template(DEFAULT_PROJECT_TEMPLATE)
                    .unwrap_or_else(|| DEFAULT_PROJECT_TEMPLATE.to_string())
            });

        Self { project, file }
    }
}

/// Turn a `$VISUAL`/`$EDITOR` value into a template.
///
/// These variables hold a *command*, not a recon template, so a bare `vim` has
/// no placeholder and would be spawned with no file at all. Appending `{file}`
/// is what makes the fallback rung actually work.
///
/// `{file}` and not `{project} {file}`: recon knows nothing about a command it
/// did not define, and handing an unknown editor a directory it did not ask for
/// opens a second buffer full of nothing in `vim`. A value that already carries
/// a placeholder is a deliberate template and is left exactly as written.
fn generic_editor_template(value: &str) -> String {
    if value.contains('{') {
        value.to_string()
    } else {
        format!("{value} {{file}}")
    }
}

/// `drop_project` at the template level, for the derive rung of the `O` ladder.
///
/// `None` when the template cannot be split — a broken template must not turn
/// into a *differently* broken derived one; the ladder falls through and the
/// user gets the error from the `o` key, once, where they can act on it.
fn derive_file_template(template: &str) -> Option<String> {
    let argv = split_template(template).ok()?;
    Some(shell_join(&drop_project(&argv)))
}

/// Re-quote argv back into a single template string.
///
/// Only ever applied to entries that came out of [`split_template`], and only
/// so a derived template can travel as a `String` like every other rung of the
/// ladder. Single quotes because they are literal throughout; an entry
/// containing one is wrapped in double quotes instead, and an entry containing
/// both is escaped the double-quoted way.
fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|entry| {
            if !entry.is_empty() && !entry.contains([' ', '\t', '\'', '"', '\\']) {
                entry.clone()
            } else if !entry.contains('\'') {
                format!("'{entry}'")
            } else {
                format!("\"{}\"", entry.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How recon actually starts an editor.
///
/// A trait so the whole path above it can be tested without a process ever
/// running: a test double records the argv it *would* have executed. There is
/// no way to launch a real editor in CI, and asserting on a recorded command is
/// the entire testable surface of "spawn".
pub trait Launcher {
    /// Start `argv`, detached. `Ok(())` means the process started, not that it
    /// succeeded — an editor that exits non-zero reports that later, out of
    /// band, because recon must not block waiting for one.
    fn spawn(&self, argv: &[String]) -> std::io::Result<()>;
}

/// So `App` can hold a `Box<dyn Launcher>` in a `#[derive(Default)]` struct
/// without the field becoming an `Option` — a `None` launcher would mean `o`
/// silently doing nothing, which is the one failure mode this whole path exists
/// to avoid. Legal because `Box` is `#[fundamental]` and `dyn Launcher` is
/// local. The default reports nowhere; `App::new` replaces it with one wired to
/// the status row.
impl Default for Box<dyn Launcher> {
    fn default() -> Self {
        Box::new(ProcessLauncher::default())
    }
}

/// The real launcher: `std::process::Command`, detached and reaped.
#[derive(Default)]
pub struct ProcessLauncher {
    /// Where a non-zero exit is reported, once the child has actually exited.
    ///
    /// `None` in the tests that only exercise construction. When present, the
    /// reaper thread sends a ready-to-display message and `App` drains it onto
    /// the status row on its next poll.
    outcomes: Option<Sender<String>>,
}

impl ProcessLauncher {
    pub fn new(outcomes: Sender<String>) -> Self {
        Self {
            outcomes: Some(outcomes),
        }
    }
}

impl Launcher for ProcessLauncher {
    fn spawn(&self, argv: &[String]) -> std::io::Result<()> {
        let Some((program, args)) = argv.split_first() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "empty editor command",
            ));
        };

        let mut child = std::process::Command::new(program)
            .args(args)
            // All three, or the child draws over the alternate screen recon is
            // holding — a GUI editor's startup warning on stderr lands in the
            // middle of the file view and stays there until the next full
            // redraw. Null stdin as well: a terminal editor launched by mistake
            // would otherwise fight recon for the keyboard.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let outcomes = self.outcomes.clone();
        let name = program.clone();
        // Reaped on a thread rather than left alone. A child that is never
        // waited on becomes a zombie for the life of the process, and recon is
        // a long-running TUI someone may press `o` in fifty times. Waiting on
        // the main thread instead would freeze the UI for as long as the editor
        // runs, which for a terminal editor is the whole session.
        std::thread::spawn(move || {
            let Ok(status) = child.wait() else { return };
            if !status.success()
                && let Some(outcomes) = outcomes
            {
                // A closed receiver means recon is shutting down, which is not
                // worth reporting to anyone.
                let _ = outcomes.send(format!("{name} exited with {status}"));
            }
        });
        Ok(())
    }
}

/// The known-good templates `--print-editor-config` can emit.
///
/// The GUI four come straight from each editor's documented "open at line" CLI.
/// The terminal entries open a *new window* and so need no terminal handover at
/// all: recon's own screen is never touched, which is why terminal editors need
/// no special support beyond a different string here.
const FLAVOURS: &[(&str, &str, &str)] = &[
    // (name, project template, file template)
    ("zed", "zed {project} {file}:{line}", "zed {file}:{line}"),
    (
        "vscode",
        "code {project} -g {file}:{line}",
        "code -g {file}:{line}",
    ),
    (
        "sublime",
        "subl {project} {file}:{line}",
        "subl {file}:{line}",
    ),
    (
        "idea",
        "idea --line {line} {file}",
        "idea --line {line} {file}",
    ),
    // Nested quoting, and the one hazard worth calling out: recon passes the
    // `-e` argument through as a single argv entry and never re-parses it, but
    // the shell in the new window *does* parse the string inside it. Hence the
    // escaped inner quotes around the paths — that is the layer where a space
    // in a path would otherwise split.
    (
        "terminal-nvim",
        r#"osascript -e 'tell app "Terminal" to do script "cd \"{project}\" && nvim +{line} \"{file}\""'"#,
        r#"osascript -e 'tell app "Terminal" to do script "nvim +{line} \"{file}\""'"#,
    ),
    (
        "iterm-nvim",
        r#"osascript -e 'tell app "iTerm2" to create window with default profile command "nvim +{line} \"{file}\""'"#,
        r#"osascript -e 'tell app "iTerm2" to create window with default profile command "nvim +{line} \"{file}\""'"#,
    ),
    // The native forms: no nesting, no shell in the middle, nothing to quote.
    // Prefer these where the terminal offers them — noting that `wezterm cli`
    // needs a running mux and `kitty @` needs `allow_remote_control yes`, so
    // both can fail on a default install.
    (
        "wezterm-nvim",
        "wezterm cli spawn --cwd {project} -- nvim +{line} {file}",
        "wezterm cli spawn -- nvim +{line} {file}",
    ),
    (
        "kitty-nvim",
        "kitty @ launch --type=window --cwd {project} nvim +{line} {file}",
        "kitty @ launch --type=window nvim +{line} {file}",
    ),
    (
        "ghostty-nvim",
        "ghostty -e nvim +{line} {file}",
        "ghostty -e nvim +{line} {file}",
    ),
];

/// A `--print-editor-config` argument naming no known flavour.
#[derive(Debug, PartialEq, Eq)]
pub struct UnknownFlavour(pub String);

impl fmt::Display for UnknownFlavour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let known = FLAVOURS
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            f,
            "unknown editor flavour {:?}\nknown flavours: auto, {known}",
            self.0
        )
    }
}

impl std::error::Error for UnknownFlavour {}

/// The flavour to print when the user asks for `auto`, guessed from the
/// terminal recon is running in.
///
/// A guess, and only ever a starting point — the printed snippet is text to
/// read and edit, not something applied. Falls back to `zed`, which is the
/// compiled-in default and so the least surprising thing to show.
fn guess_flavour(term_program: Option<&str>) -> &'static str {
    match term_program {
        Some("WezTerm") => "wezterm-nvim",
        Some("ghostty") => "ghostty-nvim",
        Some("iTerm.app") => "iterm-nvim",
        Some("Apple_Terminal") => "terminal-nvim",
        _ => "zed",
    }
}

/// Render the `config.toml` stanza for one flavour.
///
/// A `String` returned to the caller to print, and **nothing else**. recon
/// never writes `config.toml` — that is a decision, recorded in `Cargo.toml`,
/// which drops `toml`'s serializer so a write path cannot be added by accident
/// — and it will not write a shell rc either: silently editing `.zshrc` is a
/// surprising thing for a file viewer to do, hard to undo, and it would have to
/// guess between `.zshrc`, `.zprofile`, `.bashrc` and fish.
///
/// A stanza rather than an `export` because an editor template is a recon
/// setting, not a shell setting, and #18 has landed.
pub fn print_editor_config(
    flavour: &str,
    term_program: Option<&str>,
) -> Result<String, UnknownFlavour> {
    let wanted = if flavour == "auto" {
        guess_flavour(term_program)
    } else {
        flavour
    };
    let Some((name, project, file)) = FLAVOURS.iter().find(|(name, _, _)| *name == wanted) else {
        return Err(UnknownFlavour(flavour.to_string()));
    };

    // Both keys, always. `editor.file` would otherwise be derived by dropping
    // `{project}`, which is right for the GUI editors and wrong for the
    // `--cwd {project}` forms — see `drop_project`'s known limit. Printing the
    // pair costs one line and removes the trap.
    Ok(format!(
        "# recon editor templates ({name})\n\
         # Paste into ~/.config/recon/config.toml — recon never writes it for you.\n\
         # {{project}} = project root, {{file}} = the file, {{line}} = the cursor's line.\n\
         [editor]\n\
         project = {}\n\
         file = {}\n",
        toml_string(project),
        toml_string(file),
    ))
}

/// Quote a template as a TOML string.
///
/// A literal string (`'…'`) wherever the template has no single quote in it,
/// which keeps the backslashes in the `osascript` forms readable — TOML's basic
/// strings would double every one of them. Falls back to a basic string with
/// the two escapes TOML requires when the template contains a `'`.
fn toml_string(value: &str) -> String {
    if value.contains('\'') {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{value}'")
    }
}

/// The test double the [`Launcher`] trait exists for.
///
/// Outside `mod tests` so `lib.rs`'s tests can reach it too — they drive the
/// `o` key end to end and need the same "what would have run?" recording, and a
/// second copy over there is the sort of duplicate that drifts.
#[cfg(test)]
pub(crate) mod double {
    use super::Launcher;
    use std::sync::Mutex;

    /// Records the argv it would have run, and can be told to fail.
    #[derive(Default)]
    pub(crate) struct RecordingLauncher {
        pub commands: Mutex<Vec<Vec<String>>>,
        /// When set, `spawn` reports this instead of succeeding — the "editor
        /// is not installed" path, which is the whole reason the status row
        /// grew a message slot.
        pub fail_with: Option<String>,
    }

    impl RecordingLauncher {
        pub fn failing(message: &str) -> Self {
            Self {
                fail_with: Some(message.to_string()),
                ..Self::default()
            }
        }

        /// The single command recorded, or a panic naming what was there
        /// instead. Every test here expects exactly one.
        pub fn only_command(&self) -> Vec<String> {
            let commands = self.commands.lock().unwrap_or_else(|p| p.into_inner());
            assert_eq!(commands.len(), 1, "expected one command: {commands:?}");
            commands[0].clone()
        }

        pub fn is_empty(&self) -> bool {
            self.commands
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        }
    }

    impl Launcher for RecordingLauncher {
        fn spawn(&self, argv: &[String]) -> std::io::Result<()> {
            self.commands
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(argv.to_vec());
            match &self.fail_with {
                Some(message) => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    message.clone(),
                )),
                None => Ok(()),
            }
        }
    }

    /// So a test can keep a handle on the recording while `App` owns the
    /// launcher. `Launcher::spawn` takes `&self`, so the shared reference needs
    /// no interior mutability beyond the `Mutex` already there.
    impl Launcher for std::rc::Rc<RecordingLauncher> {
        fn spawn(&self, argv: &[String]) -> std::io::Result<()> {
            (**self).spawn(argv)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::double::RecordingLauncher;
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    // ---- project root --------------------------------------------------

    /// Same guard, and same reasoning, as `config.rs`'s `CONFIG_FIXTURE_NAMES`:
    /// these tests write real directories under `target/` and run in parallel.
    static EDITOR_FIXTURE_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn claim_fixture_name(name: &str) {
        let mut names = EDITOR_FIXTURE_NAMES
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(
            !names.iter().any(|used| used == name),
            "editor fixture name {name:?} is already in use by another test — pick a unique name"
        );
        names.push(name.to_string());
    }

    /// Build `target/test-editor/<name>/` with the given relative paths in it.
    /// A path ending in `/` becomes a directory; anything else an empty file.
    ///
    /// Returns an **absolute** root, because that is what `project_root`
    /// returns and comparing the two forms would fail on the shape rather than
    /// on the answer.
    fn tree(name: &str, paths: &[&str]) -> PathBuf {
        tree_under(Path::new("target/test-editor"), name, paths)
    }

    fn tree_under(parent: &Path, name: &str, paths: &[&str]) -> PathBuf {
        claim_fixture_name(name);
        let root = std::path::absolute(parent.join(name)).expect("absolute fixture root");
        // Left over from a previous run, which would otherwise leave a marker
        // in place that this test never asked for.
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create fixture root");
        for path in paths {
            let full = root.join(path);
            if path.ends_with('/') {
                fs::create_dir_all(&full).expect("create fixture dir");
            } else {
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent).expect("create fixture parent");
                }
                fs::write(&full, "").expect("write fixture file");
            }
        }
        root
    }

    #[test]
    fn a_cargo_project_is_found_from_a_nested_file() {
        let root = tree("cargo", &["Cargo.toml", "src/widgets/fileview.rs"]);
        assert_eq!(project_root(&root.join("src/widgets/fileview.rs")), root);
    }

    #[test]
    fn a_git_directory_marks_a_project() {
        let root = tree("git-dir", &[".git/", "notes/today.log"]);
        assert_eq!(project_root(&root.join("notes/today.log")), root);
    }

    /// In a linked worktree — which this repo's own `/work-issue` flow creates
    /// on every issue — `.git` is a *file* holding a `gitdir:` pointer. A
    /// marker check that insisted on a directory would walk straight past it
    /// and open the wrong project.
    #[test]
    fn a_git_file_in_a_linked_worktree_marks_a_project() {
        let root = tree("git-file", &["src/lib.rs"]);
        fs::write(root.join(".git"), "gitdir: /elsewhere\n").expect("write gitdir pointer");
        assert_eq!(project_root(&root.join("src/lib.rs")), root);
    }

    /// The *nearest* marker wins, not the outermost. A crate inside a workspace
    /// is the project you meant when the cursor is in one of its files.
    #[test]
    fn the_nearest_marker_wins() {
        let root = tree("nested", &["Cargo.toml", "vendor/inner/Cargo.toml"]);
        fs::write(root.join("vendor/inner/lib.rs"), "").expect("write inner file");
        assert_eq!(
            project_root(&root.join("vendor/inner/lib.rs")),
            root.join("vendor/inner")
        );
    }

    #[test]
    fn every_marker_in_the_table_is_recognised() {
        for (index, marker) in PROJECT_MARKERS.iter().enumerate() {
            // `.git` is exercised as both a file and a directory above; here it
            // rides along with the rest as an ordinary file.
            let root = tree(
                &format!("marker-{index}"),
                &[marker, "deep/nested/file.txt"],
            );
            assert_eq!(
                project_root(&root.join("deep/nested/file.txt")),
                root,
                "marker {marker} was not recognised"
            );
        }
    }

    /// No marker anywhere above: `o` still has to do something, and the file's
    /// own directory is the honest answer.
    ///
    /// Built under the system temp directory, not `target/`, and that is the
    /// point of the test rather than an inconvenience: `target/` sits inside
    /// recon's own repo, so a walk-up from there finds recon's `Cargo.toml` and
    /// the fallback is never reached. The temp directory is the nearest thing
    /// to "somewhere with no project above it" that a test can rely on — and if
    /// a marker ever does appear above it on some machine, this fails loudly
    /// rather than passing for the wrong reason.
    #[test]
    fn a_file_with_no_project_falls_back_to_its_directory() {
        let root = tree_under(
            &std::env::temp_dir(),
            "recon-no-marker",
            &["logs/today.log"],
        );
        assert_eq!(
            project_root(&root.join("logs/today.log")),
            root.join("logs")
        );
    }

    /// The bug this function's absolutising step exists for. A relative path's
    /// ancestors bottom out at `""`, and `"".join("Cargo.toml")` tests the
    /// *current working directory* — so without it, `o` on a relative path
    /// would report whatever project recon was launched from, which is very
    /// often a plausible-looking wrong answer.
    #[test]
    fn a_relative_path_does_not_climb_into_the_working_directory() {
        let root = tree("relative", &["go.mod", "logs/today.log"]);
        let relative = Path::new("target/test-editor/relative/logs/today.log");
        assert_eq!(project_root(relative), root);
    }

    /// The navigator can have a directory selected, and the file view shows its
    /// listing. Walking up from the directory itself — rather than from its
    /// parent — is what makes `o` on a project's own root open that project.
    #[test]
    fn a_directory_walks_up_from_itself() {
        let root = tree("dir-arg", &["Cargo.toml", "src/"]);
        assert_eq!(project_root(&root), root);
        assert_eq!(project_root(&root.join("src")), root);
    }

    // ---- splitting ------------------------------------------------------

    fn split(template: &str) -> Vec<String> {
        split_template(template).expect("template splits")
    }

    #[test]
    fn a_plain_template_splits_on_whitespace() {
        assert_eq!(
            split("zed {project} {file}:{line}"),
            ["zed", "{project}", "{file}:{line}"]
        );
    }

    /// The whole reason splitting is quote-aware. A whitespace split turns this
    /// AppleScript into fifteen arguments, and `osascript` runs the first word
    /// of it as an entire script.
    #[test]
    fn the_osascript_form_survives_as_one_argv_entry() {
        let (_, project, _) = FLAVOURS
            .iter()
            .find(|(name, _, _)| *name == "terminal-nvim")
            .expect("terminal-nvim is a known flavour");
        let argv = split(project);
        assert_eq!(argv.len(), 3, "expected `osascript -e <script>`: {argv:?}");
        assert_eq!(argv[0], "osascript");
        assert_eq!(argv[1], "-e");
        assert!(
            argv[2].starts_with("tell app \"Terminal\""),
            "the script lost its quoting: {:?}",
            argv[2]
        );
        assert!(
            argv[2].contains("{project}") && argv[2].contains("{file}"),
            "the script lost its placeholders: {:?}",
            argv[2]
        );
    }

    #[test]
    fn double_quotes_group_and_escape() {
        assert_eq!(
            split(r#"say "hello there" and \"quoted\""#),
            ["say", "hello there", "and", "\"quoted\""]
        );
    }

    /// No escapes inside single quotes, exactly as in a shell — which is what
    /// makes them the readable way to write a template full of backslashes.
    #[test]
    fn single_quotes_are_literal_throughout() {
        assert_eq!(split(r#"e 'a \b "c" '"#), ["e", r#"a \b "c" "#]);
    }

    #[test]
    fn an_unterminated_quote_is_an_error() {
        assert_eq!(
            split_template("zed 'unclosed"),
            Err(TemplateError::UnterminatedQuote { quote: '\'' })
        );
        assert_eq!(
            split_template("zed \"unclosed"),
            Err(TemplateError::UnterminatedQuote { quote: '"' })
        );
    }

    #[test]
    fn an_empty_template_is_an_error() {
        assert_eq!(split_template("   "), Err(TemplateError::Empty));
        assert_eq!(split_template(""), Err(TemplateError::Empty));
    }

    // ---- substitution ---------------------------------------------------

    fn command(template: &str, project: &str, file: &str, line: usize) -> Vec<String> {
        editor_command(template, Path::new(project), Path::new(file), line)
            .expect("template builds a command")
    }

    /// The documented table, asserted as exact argv. This is the whole of
    /// "editor support" that can be tested without launching anything.
    #[test]
    fn every_known_good_template_produces_its_documented_argv() {
        let cases: &[(&str, &[&str])] = &[
            (
                "zed {project} {file}:{line}",
                &["zed", "/proj", "/proj/src/lib.rs:42"],
            ),
            (
                "code {project} -g {file}:{line}",
                &["code", "/proj", "-g", "/proj/src/lib.rs:42"],
            ),
            (
                "subl {project} {file}:{line}",
                &["subl", "/proj", "/proj/src/lib.rs:42"],
            ),
            (
                "idea --line {line} {file}",
                &["idea", "--line", "42", "/proj/src/lib.rs"],
            ),
        ];
        for (template, expected) in cases {
            assert_eq!(
                command(template, "/proj", "/proj/src/lib.rs", 42),
                *expected,
                "template {template:?}"
            );
        }
    }

    /// The reason splitting happens before substitution. A space in a path can
    /// only split the command if the path is present when splitting happens —
    /// so it never is.
    #[test]
    fn a_path_containing_a_space_stays_one_argv_entry() {
        let argv = command(
            "zed {project} {file}:{line}",
            "/My Projects/app",
            "/My Projects/app/src/main.rs",
            7,
        );
        assert_eq!(
            argv,
            ["zed", "/My Projects/app", "/My Projects/app/src/main.rs:7"]
        );
    }

    /// No shell ever sees these, so `"` and `$` are ordinary characters. This
    /// is the test that would fail the day someone reaches for `sh -c`.
    #[test]
    fn quotes_and_dollars_in_a_path_pass_through_verbatim() {
        let argv = command("zed {project} {file}", "/p", r#"/p/we"ird/$HOME.log"#, 1);
        assert_eq!(argv, ["zed", "/p", r#"/p/we"ird/$HOME.log"#]);
    }

    #[test]
    fn a_template_with_no_line_placeholder_still_works() {
        assert_eq!(
            command("mate {file}", "/p", "/p/a.rs", 9),
            ["mate", "/p/a.rs"]
        );
    }

    /// Emptying an unknown placeholder would turn `--goto {ln}` into a
    /// dangling flag, and the user would go hunting for a typo recon had
    /// already spotted.
    #[test]
    fn an_unknown_placeholder_is_left_alone() {
        assert_eq!(
            command("ed --at {ln} {file}", "/p", "/p/a.rs", 3),
            ["ed", "--at", "{ln}", "/p/a.rs"]
        );
    }

    /// A single pass, not three `replace` calls: text substituted in is never
    /// re-scanned, so a file whose name looks like a placeholder survives.
    #[test]
    fn substituted_text_is_not_re_scanned() {
        assert_eq!(
            command("zed {file}", "/p", "/p/{line}.log", 5),
            ["zed", "/p/{line}.log"]
        );
    }

    #[test]
    fn an_unclosed_brace_is_ordinary_text() {
        assert_eq!(command("zed {fi", "/p", "/p/a.rs", 1), ["zed", "{fi"]);
    }

    // ---- deriving the file template -------------------------------------

    #[test]
    fn dropping_project_yields_the_documented_file_argv() {
        let cases: &[(&str, &str)] = &[
            ("zed {project} {file}:{line}", "zed {file}:{line}"),
            ("code {project} -g {file}:{line}", "code -g {file}:{line}"),
            ("subl {project} {file}:{line}", "subl {file}:{line}"),
            // Nothing to drop.
            ("idea --line {line} {file}", "idea --line {line} {file}"),
        ];
        for (project, expected) in cases {
            assert_eq!(
                derive_file_template(project).as_deref(),
                Some(*expected),
                "deriving from {project:?}"
            );
        }
    }

    // ---- the resolution ladders -----------------------------------------

    #[test]
    fn nothing_configured_yields_the_compiled_in_defaults() {
        let templates = Templates::resolve(None, None, None, None);
        assert_eq!(templates.project, "zed {project} {file}:{line}");
        assert_eq!(templates.file, "zed {file}:{line}");
    }

    #[test]
    fn recon_editor_wins_over_visual_and_editor() {
        let templates = Templates::resolve(
            Some("code {project} -g {file}:{line}"),
            None,
            Some("vim"),
            Some("ed"),
        );
        assert_eq!(templates.project, "code {project} -g {file}:{line}");
    }

    /// The one deliberate deviation from #18's ladder, pinned as a test: these
    /// are not recon's variables, so a recon-specific setting outranks them.
    /// `recon_project` here stands for the merged CLI/env/**config file** layer.
    #[test]
    fn visual_and_editor_rank_below_the_config_file() {
        let from_file = Templates::resolve(Some("subl {project} {file}"), None, Some("vim"), None);
        assert_eq!(from_file.project, "subl {project} {file}");

        let without_file = Templates::resolve(None, None, Some("vim"), None);
        assert_eq!(without_file.project, "vim {file}");
    }

    #[test]
    fn visual_outranks_editor() {
        let templates = Templates::resolve(None, None, Some("nvim"), Some("ed"));
        assert_eq!(templates.project, "nvim {file}");
    }

    /// A bare command name is not a template. Without an appended `{file}` the
    /// fallback rung spawns an editor over no file at all.
    #[test]
    fn a_placeholderless_editor_variable_gains_a_file_placeholder() {
        assert_eq!(generic_editor_template("vim"), "vim {file}");
        assert_eq!(
            generic_editor_template("emacsclient -nw"),
            "emacsclient -nw {file}"
        );
    }

    /// Someone who wrote placeholders into `$EDITOR` meant them.
    #[test]
    fn an_editor_variable_that_is_already_a_template_is_untouched() {
        assert_eq!(
            generic_editor_template("nvim +{line} {file}"),
            "nvim +{line} {file}"
        );
    }

    /// The whole point of the derive rung: one line of config makes both keys
    /// work.
    #[test]
    fn the_file_template_derives_from_the_project_one() {
        let templates =
            Templates::resolve(Some("code {project} -g {file}:{line}"), None, None, None);
        assert_eq!(templates.file, "code -g {file}:{line}");
    }

    #[test]
    fn an_explicit_file_template_wins_over_the_derive() {
        let templates = Templates::resolve(
            Some("zed {project} {file}:{line}"),
            Some("zed -n {file}:{line}"),
            None,
            None,
        );
        assert_eq!(templates.file, "zed -n {file}:{line}");
    }

    /// Ladder order, not just presence: the derive sits *above* `$VISUAL`.
    #[test]
    fn the_derive_outranks_visual_for_the_file_template() {
        let templates = Templates::resolve(Some("subl {project} {file}"), None, Some("vim"), None);
        assert_eq!(templates.file, "subl {file}");
    }

    /// An empty or whitespace-only variable is the usual shell accident
    /// (`export EDITOR=$SOMETHING_UNSET`) and reads as unset, the same way
    /// `config_path_from` treats an empty `$HOME`.
    #[test]
    fn an_empty_editor_variable_is_ignored() {
        let templates = Templates::resolve(None, None, Some(""), Some("  "));
        assert_eq!(templates.project, DEFAULT_PROJECT_TEMPLATE);
    }

    // ---- --print-editor-config -------------------------------------------

    #[test]
    fn every_flavour_prints_a_stanza_the_config_parser_accepts() {
        for (name, _, _) in FLAVOURS {
            let snippet = print_editor_config(name, None).expect("known flavour");
            let parsed: crate::config::FileConfig =
                toml::from_str(&snippet).unwrap_or_else(|err| {
                    panic!("the {name} snippet is not a valid config file: {err}\n{snippet}")
                });
            let editor = parsed.editor.expect("the snippet sets [editor]");
            // Round-trip: what it prints must survive the parser *and* still
            // split, or the snippet is advice that does not work.
            for template in [editor.project, editor.file] {
                let template = template.expect("both keys are printed");
                split_template(&template)
                    .unwrap_or_else(|err| panic!("{name}: {template:?} does not split: {err}"));
            }
        }
    }

    /// The `auto` guess is a starting point, keyed off the terminal recon is
    /// running in so the printed snippet matches where the user actually is.
    #[test]
    fn auto_guesses_from_term_program_and_falls_back_to_zed() {
        assert!(
            print_editor_config("auto", Some("WezTerm"))
                .expect("auto")
                .contains("wezterm")
        );
        assert!(
            print_editor_config("auto", Some("iTerm.app"))
                .expect("auto")
                .contains("iTerm2")
        );
        assert!(
            print_editor_config("auto", Some("Apple_Terminal"))
                .expect("auto")
                .contains("Terminal")
        );
        assert!(
            print_editor_config("auto", None)
                .expect("auto")
                .contains("zed")
        );
    }

    #[test]
    fn an_unknown_flavour_is_reported_with_the_known_ones() {
        let err = print_editor_config("emacs", None).expect_err("emacs is not a flavour");
        let rendered = err.to_string();
        assert!(rendered.contains("emacs"), "{rendered}");
        assert!(
            rendered.contains("zed"),
            "the known list is missing: {rendered}"
        );
    }

    // ---- launching --------------------------------------------------------

    #[test]
    fn the_recording_launcher_captures_what_would_have_run() {
        let launcher = RecordingLauncher::default();
        launcher
            .spawn(&["zed".to_string(), "/p".to_string()])
            .expect("recording never fails");
        assert_eq!(launcher.only_command(), ["zed", "/p"]);
    }

    /// The double has to be able to fail, or the status-row error path in
    /// `lib.rs` has nothing to exercise it.
    #[test]
    fn the_recording_launcher_can_be_told_to_fail() {
        let launcher = RecordingLauncher::failing("no such file or directory");
        let err = launcher
            .spawn(&["nope".to_string()])
            .expect_err("configured to fail");
        assert!(err.to_string().contains("no such file"));
        assert_eq!(launcher.only_command(), ["nope"]);
    }

    /// The one test that really spawns, over a command every Unix has. Ignored
    /// by default: it is the only thing here that touches the process table,
    /// and CI should not depend on `/bin/true` existing at a given path.
    #[test]
    #[ignore = "spawns a real process; run with `cargo test -- --ignored`"]
    fn a_real_process_spawns_and_is_reaped() {
        let (tx, rx) = std::sync::mpsc::channel();
        let launcher = ProcessLauncher::new(tx);
        launcher
            .spawn(&["true".to_string()])
            .expect("`true` exists on every Unix");
        // A successful child reports nothing, so the channel stays empty. The
        // failing one below is where the reporting path is actually exercised.
        launcher
            .spawn(&["false".to_string()])
            .expect("`false` exists too");
        let message = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("a non-zero exit is reported");
        assert!(message.contains("false"), "{message}");
    }

    #[test]
    fn a_missing_program_is_an_error_rather_than_a_panic() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let launcher = ProcessLauncher::new(tx);
        let err = launcher
            .spawn(&["recon-definitely-not-a-real-program".to_string()])
            .expect_err("a missing program cannot spawn");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
