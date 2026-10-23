//! Filetype glyphs for the directory listings (#512).
//!
//! One Nerd Font glyph per file kind or common extension, drawn at the left
//! of each row in the explorer and in the file view's directory listing.
//! The table below is the single source of truth; both draw sites compose it
//! through [`glyph`] so the two panes cannot drift apart.
//!
//! # Codepoints
//!
//! Every codepoint here was verified against a real installed Nerd Font
//! (MonaspiceArNerdFont-Regular.otf, 13 288 cmap entries) before landing, as
//! #512 requires. Glyph *names* in an installed font do not match the cheat
//! sheet one-to-one — Nerd Font releases move aliases between codepoints —
//! so the names in this file's comments record what the probe saw, and the
//! codepoints are the contract.
//!
//! # Graceful failure
//!
//! No terminal reports its configured font to a child process, so recon
//! cannot know that it can draw these glyphs. [`Icons`] carries the decision,
//! resolved once at startup (see [`resolve`]) and pushed into the panes the
//! way [`crate::filter::Background`] is. With icons off, every row is drawn
//! exactly as it was before #512 — which is what a machine with only base
//! fonts gets by default.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::widgets::explorer::Entry;

/// The user's setting for filetype glyphs: the `[view] icons` key,
/// `--icons`, or `RECON_ICONS` (#512).
///
/// `Auto` is the default, and the one that keeps a base-fonts machine
/// untouched: icons come on only when the host terminal is positively
/// identified as able to draw them (see [`resolve`]). `On` forces them;
/// `Off` is today's exact behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Setting {
    /// Decide from what the terminal and its font are found to be.
    #[default]
    Auto,
    /// Draw the glyphs whatever the probe finds; a glyph that advances the
    /// wrong number of columns is replaced by a space and logged.
    On,
    /// Never draw the glyphs: exactly the pane #512 leaves behind.
    Off,
}

/// Whether filetype glyphs are drawn, and which ones the startup probe found
/// drawing at the wrong width.
///
/// Resolved once at startup and pushed into the panes and the read jobs;
/// nothing here is decided at render time. `dropped` is only ever non-empty
/// for `icons = "on"` — in `auto`, one corrupt glyph turns the whole feature
/// off instead (see [`resolve`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Icons {
    /// Whether rows lead with a glyph at all.
    pub on: bool,
    /// Glyphs the terminal drew at zero or two columns instead of one, to be
    /// drawn as a space instead. A zero-width advance overlaps rows; a
    /// double-width advance shifts every column after it (#97) — both are
    /// corrupting, and both are only discoverable by asking the terminal.
    pub dropped: Vec<char>,
}

impl Icons {
    /// Icons off: today's exact behavior, unchanged.
    pub(crate) fn off() -> Self {
        Self::default()
    }

    /// Whether the flag pushed in says icons are on, for the tests that pin
    /// the push-in wiring.
    #[cfg(test)]
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }
}

/// The glyph for `entry`, or `None` when icons are off.
///
/// Every row gets exactly one single-column glyph — directories, `..`,
/// executables and specials included — so a caller that draws the glyph
/// always draws it in a two-column prefix: glyph plus one separating space.
/// A dropped glyph comes back as a space, keeping every row the same width.
pub(crate) fn glyph(entry: &Entry, icons: &Icons) -> Option<char> {
    if !icons.on {
        return None;
    }
    let g = glyph_for(entry);
    Some(if icons.dropped.contains(&g) { ' ' } else { g })
}

/// The table, without the off/dropped decisions around it.
fn glyph_for(entry: &Entry) -> char {
    match entry.kind {
        Kind::Dir => DIR,
        Kind::Parent => PARENT,
        Kind::Executable => EXECUTABLE,
        Kind::Special => SPECIAL,
        Kind::Plain => glyph_for_name(&entry.name),
    }
}

/// The glyph for one plain file, by its extension.
///
/// The extension is lowercased before the lookup, so `MAIN.RS` reads as the
/// Rust file it is. A leading-dot file has no extension as `Path` sees it —
/// `.gitignore` is all stem — so the handful of dotfiles worth an icon are
/// matched on the name instead.
fn glyph_for_name(name: &OsStr) -> char {
    let name = name.to_string_lossy();
    let lower = name.to_lowercase();
    if let Some(glyph) = by_name(&lower) {
        return glyph;
    }
    Path::new(&*name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .and_then(|ext| by_extension(&ext))
        .unwrap_or(PLAIN)
}

/// Glyphs for whole filenames: the dotfiles `Path::extension` cannot see, and
/// the extensionless `Dockerfile` every other tool spells out.
fn by_name(lower: &str) -> Option<char> {
    // One rule covers `.gitignore`, `.gitattributes` and `.gitmodules` with
    // the `.git*` prefix they share, and any dotfile a repo adds under it.
    if lower.starts_with(".git") {
        return Some(GIT);
    }
    if lower == "dockerfile" || lower == ".dockerignore" {
        return Some(DOCKER);
    }
    None
}

/// Glyphs by extension, lowercased. The common case first for the reader, not
/// the machine — the match order below is by family, and one linear scan per
/// row is cheap next to the `read_dir` that produced the entry.
fn by_extension(ext: &str) -> Option<char> {
    Some(match ext {
        // Rust, Python, Go.
        "rs" => RUST,
        "py" | "pyi" => PYTHON,
        "go" => GO,
        // JavaScript and TypeScript, including the module spellings and the
        // JSX/TSX files that are one or the other.
        "js" | "mjs" | "cjs" => JS,
        "ts" | "mts" | "cts" => TS,
        "jsx" | "tsx" => REACT,
        // Web.
        "html" | "htm" => HTML,
        "css" => CSS,
        // Shells.
        "sh" | "bash" | "zsh" | "fish" | "ksh" => SHELL,
        // C and C++: headers ride with their sources.
        "c" | "h" => C,
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => CPP,
        // Other languages.
        "java" => JAVA,
        "rb" => RUBY,
        "php" => PHP,
        "lua" => LUA,
        "sql" => SQL,
        "swift" => SWIFT,
        "kt" | "kts" => KOTLIN,
        "scala" => SCALA,
        "hs" => HASKELL,
        "ex" | "exs" => ELIXIR,
        "clj" | "cljs" | "cljc" | "edn" => CLOJURE,
        "elm" => ELM,
        "zig" => ZIG,
        "pl" | "pm" => PERL,
        "xml" => XML,
        // Configuration and data.
        "json" => JSON,
        "yaml" | "yml" => YAML,
        "toml" | "ini" | "conf" | "cfg" => CONFIG,
        "md" | "markdown" => MARKDOWN,
        "txt" | "text" => TEXT,
        "log" => LOG,
        "csv" | "tsv" => TABLE,
        "lock" => LOCK,
        // Archives.
        "zip" | "tar" | "tgz" | "gz" | "xz" | "zst" | "bz2" | "7z" | "rar" | "jar" => ARCHIVE,
        // Images, text formats only — a picture has its own view (#458).
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" => IMAGE,
        _ => return None,
    })
}

use std::ffi::OsStr;

use crate::widgets::explorer::Kind;

/// Directories, and the `..` row: a folder, and an up arrow for `..`.
const DIR: char = '\u{f07b}'; // fa-folder

/// `..` wears the folder glyph above; this is the arrow a caller may prefer.
const PARENT: char = '\u{f148}'; // fa-level-up

/// Files with the executable bit: a bolt, for "this thing runs".
const EXECUTABLE: char = '\u{f0e7}'; // fa-bolt

/// A FIFO, socket or device: a plug.
const SPECIAL: char = '\u{f1e6}'; // fa-plug

/// A plain file nothing more specific matched.
const PLAIN: char = '\u{f15b}'; // fa-file

/// Per-extension glyphs, `by_extension` above. Every codepoint verified in a
/// real installed Nerd Font — see the module comment. Names drift between
/// Nerd Font releases; the codepoints are the contract, and the names here
/// record what the probe saw in MonaspiceArNerdFont-Regular.otf.
const RUST: char = '\u{e7a8}'; // dev-rust
const PYTHON: char = '\u{e73c}'; // dev-python
const GO: char = '\u{e626}'; // custom-go (seti-go)
const JS: char = '\u{e74e}'; // dev-javascript_alt
const TS: char = '\u{e628}'; // seti-typescript
const REACT: char = '\u{e7ba}'; // dev-react
const HTML: char = '\u{e736}'; // dev-html5
const CSS: char = '\u{e749}'; // dev-css3
const SHELL: char = '\u{f489}'; // oct-terminal
const C: char = '\u{e61e}'; // custom-c
const CPP: char = '\u{e61d}'; // custom-cpp
const JAVA: char = '\u{e738}'; // dev-java
const RUBY: char = '\u{e739}'; // dev-ruby
const PHP: char = '\u{e73d}'; // dev-php
const LUA: char = '\u{e620}'; // seti-lua
const SQL: char = '\u{e706}'; // dev-database
const SWIFT: char = '\u{e755}'; // dev-swift
const KOTLIN: char = '\u{e634}'; // seti-kotlin
const SCALA: char = '\u{e737}'; // dev-scala
const HASKELL: char = '\u{e777}'; // dev-haskell
const ELIXIR: char = '\u{e62d}'; // seti-elixir
const CLOJURE: char = '\u{e76a}'; // dev-clojure_alt
const ELM: char = '\u{e62c}'; // seti-elm
const ZIG: char = '\u{e6a9}'; // seti-zig
const PERL: char = '\u{e769}'; // dev-perl
const XML: char = '\u{e619}'; // seti-xml
const JSON: char = '\u{e60b}'; // seti-less (seti-json)
const YAML: char = '\u{f481}'; // oct-file_symlink_file
const CONFIG: char = '\u{e615}'; // seti-config
const MARKDOWN: char = '\u{f48a}'; // oct-markdown
const TEXT: char = '\u{f15c}'; // fa-file-text
const LOG: char = '\u{f4ed}'; // oct-log
const TABLE: char = '\u{f0ce}'; // fa-table
const LOCK: char = '\u{f023}'; // fa-lock
const ARCHIVE: char = '\u{f1c6}'; // fa-file-zipper
const IMAGE: char = '\u{f1c5}'; // fa-file-image
const GIT: char = '\u{e702}'; // dev-git
const DOCKER: char = '\u{f308}'; // linux-docker
#[cfg(test)]
mod tests {
    use super::*;

    /// A `Plain` entry named `name`, for the extension table.
    fn plain(name: &str) -> Entry {
        Entry {
            name: name.into(),
            kind: Kind::Plain,
            size: Some(1),
            modified: None,
            matched: crate::widgets::explorer::Match::Unknown,
        }
    }

    /// An entry of `kind`, named `name`.
    fn named(kind: Kind, name: &str) -> Entry {
        Entry {
            name: name.into(),
            kind,
            size: None,
            modified: None,
            matched: crate::widgets::explorer::Match::Unknown,
        }
    }

    /// A directory called `name` (the trailing `/` is a drawing decision, so
    /// the stored name never has one).
    fn dir(name: &str) -> Entry {
        named(Kind::Dir, name)
    }

    /// The glyph with icons fully on and nothing dropped.
    fn g(entry: &Entry) -> char {
        glyph(
            entry,
            &Icons {
                on: true,
                dropped: Vec::new(),
            },
        )
        .expect("icons are on, so every row has a glyph")
    }

    // ---- the table ------------------------------------------------------

    #[test]
    fn directories_wear_a_folder_glyph() {
        assert_eq!(g(&dir("src")), '\u{f07b}');
    }

    #[test]
    fn the_parent_row_wears_the_up_glyph() {
        assert_eq!(g(&named(Kind::Parent, "..")), '\u{f148}');
    }

    #[test]
    fn executables_and_specials_have_glyphs_of_their_own() {
        assert_eq!(g(&named(Kind::Executable, "recon")), '\u{f0e7}');
        assert_eq!(g(&named(Kind::Special, "pipe")), '\u{f1e6}');
    }

    #[test]
    fn common_extensions_have_glyphs() {
        for name in [
            "main.rs",
            "app.py",
            "go.mod.go",
            "index.js",
            "app.ts",
            "page.jsx",
            "page.tsx",
            "index.html",
            "style.css",
            "run.sh",
            "main.c",
            "main.h",
            "main.cpp",
            "main.hpp",
            "App.java",
            "app.rb",
            "index.php",
            "init.lua",
            "query.sql",
            "App.swift",
            "Main.kt",
            "Build.scala",
            "Main.hs",
            "lib.ex",
            "lib.exs",
            "core.clj",
            "core.cljs",
            "data.edn",
            "Main.elm",
            "main.zig",
            "script.pl",
            "data.xml",
            "package.json",
            "config.yaml",
            "config.yml",
            "Cargo.toml",
            "settings.ini",
            "app.conf",
            "app.cfg",
            "README.md",
            "notes.txt",
            "app.log",
            "table.csv",
            "table.tsv",
            "Cargo.lock",
            "bundle.zip",
            "archive.tar",
            "pkg.gz",
            "img.png",
        ] {
            assert_ne!(
                g(&plain(name)),
                PLAIN,
                "{name} fell back to the generic file glyph"
            );
        }
    }

    #[test]
    fn the_glyph_is_case_insensitive_in_the_extension() {
        assert_eq!(g(&plain("MAIN.RS")), g(&plain("main.rs")));
        assert_eq!(g(&plain("ReadMe.MD")), g(&plain("readme.md")));
    }

    #[test]
    fn git_and_docker_dotfiles_are_recognised_by_name() {
        // A leading-dot file has no extension as `Path` sees it, so `.git*`
        // is matched on the name instead.
        assert_eq!(g(&plain(".gitignore")), '\u{e702}');
        assert_eq!(g(&plain(".gitattributes")), '\u{e702}');
        assert_eq!(g(&plain(".gitmodules")), '\u{e702}');
        assert_eq!(g(&plain(".dockerignore")), '\u{f308}');
        assert_eq!(g(&plain("Dockerfile")), '\u{f308}');
        assert_eq!(g(&plain("dockerfile")), '\u{f308}');
    }

    #[test]
    fn an_unknown_extension_falls_back_to_a_generic_file_glyph() {
        assert_eq!(g(&plain("data.bin")), PLAIN);
        assert_eq!(g(&plain("no_extension")), PLAIN);
        // The fallback is a real glyph, never a blank: a row must never lose
        // its icon column because the table missed.
        assert_ne!(PLAIN, ' ');
    }

    // ---- the guarantee the layout depends on -----------------------------

    /// A zero- or double-width glyph repeats the #97 failure: rows shift or
    /// overlap. PUA codepoints are East-Asian-ambiguous, which `unicode-width`
    /// counts as one column — this pins every glyph in the table to that.
    #[test]
    fn every_glyph_is_one_column_wide() {
        use unicode_width::UnicodeWidthChar;
        // Every glyph the table holds — the width test and the startup probe
        // walk the same list, so nothing can be added to one and missed by
        // the other.
        for g in GLYPH_TABLE {
            assert_eq!(
                g.width(),
                Some(1),
                "U+{:04X} is not one column wide",
                *g as u32
            );
        }
    }

    // ---- the off / dropped decisions -------------------------------------

    #[test]
    fn icons_off_means_no_glyph_at_all() {
        assert_eq!(glyph(&dir("src"), &Icons::off()), None);
        assert_eq!(glyph(&plain("main.rs"), &Icons::off()), None);
    }

    #[test]
    fn a_dropped_glyph_becomes_a_space_not_a_gap() {
        let icons = Icons {
            on: true,
            dropped: vec!['\u{f07b}'],
        };
        // The dropped glyph draws as a space, so the row keeps its two-column
        // prefix and every other row stays aligned with it.
        assert_eq!(glyph(&dir("src"), &icons), Some(' '));
        // Only the dropped glyph is replaced; the rest of the table stands.
        assert_eq!(glyph(&plain("main.rs"), &icons), Some('\u{e7a8}'));
    }
}

/// The startup entry point: detect the host from the real world, then
/// resolve. `probe` is the terminal-width probe, passed in by `main`,
/// which owns the terminal handles.
pub fn resolve_startup(setting: Setting, probe: &dyn Fn() -> Vec<char>) -> Icons {
    let detected = if std::io::stdin().is_terminal() {
        detect(&World::real())
    } else {
        // No terminal on stdin: no one to answer a query, and nothing
        // the detection could say that the probe would confirm.
        log::debug!("icons: stdin is not a terminal, so there is nothing to ask");
        Detected::Unknown
    };
    resolve(setting, detected, probe)
}

/// Every glyph the table can draw, for the probe to walk.
#[must_use]
pub fn all_glyphs() -> Vec<char> {
    GLYPH_TABLE.to_vec()
}

/// What Tier 2 found out about the host terminal and its font. Built by
/// [`detect`] from the environment, config files and command output; the
/// resolution table tests it directly, with none of that I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detected {
    /// The host is positively identified as able to draw the glyphs.
    Confirmed,
    /// The host is known, but nothing positively confirms coverage.
    NotConfirmed,
    /// The host terminal is not one we know how to ask.
    Unknown,
}

/// The ways a test's world can be looked at, so no OS call is made.
/// The real resolver passes closures around `std::env`, `std::fs` and
/// `std::process::Command`; a test passes its own records.
pub(crate) struct World<'a> {
    pub(crate) term_program: Option<String>,
    pub(crate) env: Env<'a>,
    /// Reads a file as text, for Ghostty's and kitty's config files.
    pub(crate) read: Read<'a>,
    /// Runs a command, captures stdout: `defaults` on macOS, `fc-list`
    /// and `wezterm` elsewhere.
    pub(crate) run: Run<'a>,
}

/// An environment lookup: a variable name, its value or `None`.
pub(crate) type Env<'a> = Box<dyn Fn(&str) -> Option<String> + 'a>;
/// A file read for the detection: the text or `None` when it cannot be read.
pub(crate) type Read<'a> = Box<dyn Fn(&Path) -> Option<String> + 'a>;
/// A subprocess run for the detection: stdout when it succeeded, else `None`.
pub(crate) type Run<'a> = Box<dyn Fn(&str, &[&str]) -> Option<String> + 'a>;

impl World<'_> {
    /// The real world: environment variables, the filesystem, and
    /// subprocesses. Called once, at startup.
    pub(crate) fn real() -> Self {
        Self {
            term_program: std::env::var("TERM_PROGRAM")
                .ok()
                .filter(|value| !value.is_empty()),
            env: Box::new(|name| std::env::var(name).ok()),
            read: Box::new(|path| std::fs::read_to_string(path).ok()),
            run: Box::new(|program, args| {
                let out = std::process::Command::new(program)
                    .args(args)
                    .output()
                    .ok()?;
                out.status
                    .success()
                    .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
            }),
        }
    }
}

/// The font family a macOS terminal's own preferences name, read from
/// its `defaults` domain (#512). Terminal.app does no PUA fallback, so
/// only its *configured* face counts; iTerm2 and friends fall back
/// through CoreText, but the family check is the cheap positive answer
/// for both, so it stands for the lot and says so in the log.
fn font_from_defaults(domain: &str, world: &World) -> Option<String> {
    let out = (world.run)("defaults", &["read", domain])?;
    // iTerm2: "Normal Font" = "MesloLGMNerdFont-Regular 13" in the
    // bookmarks array. Terminal.app: the archived-NSFont blobs under
    // "Window Settings" carry the family name as plain strings inside.
    // Scanning for the family markers in the whole blob is deliberately
    // loose: a false *positive* asks the width probe to have the final
    // word, and a false negative just keeps icons off.
    let marker = |needle: &str| out.to_lowercase().contains(needle);
    (marker("nerd font") || marker("nerdfont")).then(|| "a Nerd Font".to_string())
}

/// The font family the host's own config file names, for terminals that
/// keep one and fall back through the system when it lacks a glyph.
fn font_from_config(paths: &[PathBuf], world: &World) -> Option<String> {
    for path in paths {
        if let Some(text) = (world.read)(path) {
            for line in text.lines() {
                let line = line.trim();
                let after_key = line
                    .strip_prefix("font-family")
                    .or_else(|| line.strip_prefix("font_family"));
                let Some(after_key) = after_key else { continue };
                let value = after_key
                    .trim_start()
                    .strip_prefix('=')
                    .unwrap_or(after_key)
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                if value.is_empty() || value == line {
                    continue;
                }
                let lower = value.to_lowercase().replace(' ', "");
                if lower.contains("nerdfont") || lower.ends_with("nf") {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Whether any installed font covers the glyphs, on Linux, where
/// fontconfig is effectively always present on a desktop.
fn fontconfig_covers(glyphs: &[char], world: &World) -> bool {
    // One call, all glyphs: the charset pattern names the codepoints, and
    // a non-empty answer names a font that covers them.
    let mut charset = String::new();
    for g in glyphs {
        charset.push_str(&format!("{:x}", *g as u32));
    }
    (world.run)("fc-list", &[&format!(":charset={charset}")])
        .is_some_and(|out| !out.trim().is_empty())
}

/// Everything Tier 2 can find out, once, at startup.
pub(crate) fn detect(world: &World) -> Detected {
    use Detected::{Confirmed, NotConfirmed, Unknown};
    let host = world.term_program.as_deref().unwrap_or("");
    match host {
        "iTerm.app" | "Apple_Terminal" => {
            let domain = match host {
                "iTerm.app" => "com.googlecode.iterm2",
                _ => "com.apple.Terminal",
            };
            if font_from_defaults(domain, world).is_some() {
                Confirmed
            } else {
                NotConfirmed
            }
        }
        "ghostty" => {
            let mut paths = vec![PathBuf::from(".config/ghostty/config")];
            if let Some(home) = (world.env)("HOME") {
                paths.insert(0, PathBuf::from(home).join(".config/ghostty/config"));
            }
            if let Some(dir) = (world.env)("GHOSTTY_RESOURCES_DIR") {
                let dir = PathBuf::from(dir);
                if let Some(parent) = dir.parent() {
                    paths.insert(0, parent.join("config"));
                }
            }
            if font_from_config(&paths, world).is_some() {
                Confirmed
            } else {
                NotConfirmed
            }
        }
        _ if (world.env)("KITTY_WINDOW_ID").is_some() => {
            let mut paths = vec![PathBuf::from(".config/kitty/kitty.conf")];
            if let Some(home) = (world.env)("HOME") {
                paths.insert(0, PathBuf::from(home).join(".config/kitty/kitty.conf"));
            }
            if font_from_config(&paths, world).is_some() {
                Confirmed
            } else {
                NotConfirmed
            }
        }
        "WezTerm" => {
            // The strongest signal available anywhere: WezTerm's own CLI
            // answers for the *configured* stack, not for the system at
            // large. One call, all glyphs.
            let glyphs: String = GLYPH_TABLE.iter().collect();
            let confirmed =
                (world.run)("wezterm", &["ls-fonts", "--text", &glyphs]).is_some_and(|out| {
                    let lower = out.to_lowercase().replace(' ', "");
                    lower.contains("nerdfont") || lower.contains("/nf-") || lower.ends_with("nf")
                });
            if confirmed { Confirmed } else { NotConfirmed }
        }
        _ => {
            // A bare X11/Wayland session (TERM_PROGRAM unset, no kitty
            // markers): fontconfig is the one signal left on Linux, and
            // it answers for the fonts actually installed — which is
            // what a VTE terminal, Konsole or Alacritty will fall back
            // through. On macOS this call is never reached: every host
            // we know how to ask was named above, and one we cannot ask
            // (Terminal.app's fallback, tmux) stays Unknown rather than
            // guessing from a font the terminal will never use.
            if fontconfig_covers(GLYPH_TABLE, world) {
                Confirmed
            } else {
                Unknown
            }
        }
    }
}

/// Resolve the tri-state setting into the flag the panes draw with.
///
/// `probe` is Tier 1: it asks the terminal how wide each candidate glyph
/// actually drew, and names the ones that advanced zero or two columns.
/// Never a crash, never a misaligned row — and the log says why when the
/// answer is off, with how to force it.
pub(crate) fn resolve(
    setting: Setting,
    detected: Detected,
    probe: &dyn Fn() -> Vec<char>,
) -> Icons {
    match setting {
        Setting::Off => {
            log::debug!("icons: off (setting)");
            Icons::off()
        }
        Setting::On => {
            let dropped = probe();
            for g in &dropped {
                log::warn!(
                    "icon U+{:04X} drew at the wrong width and is replaced by a space \
                     (icons = \"on\"); the font in this terminal is not a Nerd Font",
                    *g as u32
                );
            }
            Icons { on: true, dropped }
        }
        Setting::Auto => match detected {
            Detected::Confirmed => {
                let dropped = probe();
                if dropped.is_empty() {
                    log::debug!("icons: on (the terminal is positively able to draw them)");
                    Icons { on: true, dropped }
                } else {
                    // The confirming font covers the codepoints, yet the
                    // terminal drew at least one at the wrong width —
                    // what the probe exists to catch. Off beats wrong.
                    for g in &dropped {
                        log::warn!(
                            "icon U+{:04X} drew at the wrong width; icons stay off \
                             (set icons = \"on\" to force them)",
                            *g as u32
                        );
                    }
                    Icons::off()
                }
            }
            Detected::NotConfirmed => {
                log::debug!(
                    "no font covering the icon codepoints was found — set icons = \"on\" to force them"
                );
                Icons::off()
            }
            Detected::Unknown => {
                log::debug!(
                    "the terminal does not say what font it draws with — set icons = \"on\" to force them"
                );
                Icons::off()
            }
        },
    }
}

/// The glyph constants, collected: the one list the width test and the
/// startup probe share.
pub(crate) const GLYPH_TABLE: &[char] = &[
    DIR, PARENT, EXECUTABLE, SPECIAL, PLAIN, RUST, PYTHON, GO, JS, TS, REACT, HTML, CSS, SHELL, C,
    CPP, JAVA, RUBY, PHP, LUA, SQL, SWIFT, KOTLIN, SCALA, HASKELL, ELIXIR, CLOJURE, ELM, ZIG, PERL,
    XML, JSON, YAML, CONFIG, MARKDOWN, TEXT, LOG, LOCK, ARCHIVE, IMAGE, GIT, DOCKER,
];

#[cfg(test)]
mod resolution_tests {
    use super::*;

    // ---- the resolution table --------------------------------------------

    /// A probe that found nothing wrong.
    fn clean_probe() -> Vec<char> {
        Vec::new()
    }

    /// A probe that found the folder glyph undrawable.
    fn broken_probe() -> Vec<char> {
        vec![DIR]
    }

    #[test]
    fn off_is_off_whatever_was_detected() {
        for detected in [
            Detected::Confirmed,
            Detected::NotConfirmed,
            Detected::Unknown,
        ] {
            assert!(!resolve(Setting::Off, detected, &clean_probe).is_on());
        }
    }

    #[test]
    fn on_is_on_even_when_the_probe_finds_nothing_to_confirm() {
        for detected in [
            Detected::Confirmed,
            Detected::NotConfirmed,
            Detected::Unknown,
        ] {
            assert!(resolve(Setting::On, detected, &clean_probe).is_on());
        }
    }

    #[test]
    fn on_keeps_the_corrupt_glyphs_but_names_them() {
        // Forced on: the row stays aligned (the dropped glyph draws as a
        // space) and the log names what happened. The user asked.
        let icons = resolve(Setting::On, Detected::NotConfirmed, &broken_probe);
        assert!(icons.on);
        assert_eq!(icons.dropped, vec![DIR]);
    }

    #[test]
    fn auto_turns_on_only_when_positively_confirmed() {
        assert!(resolve(Setting::Auto, Detected::Confirmed, &clean_probe).is_on());
        assert!(!resolve(Setting::Auto, Detected::NotConfirmed, &clean_probe).is_on());
        assert!(!resolve(Setting::Auto, Detected::Unknown, &clean_probe).is_on());
    }

    #[test]
    fn auto_turns_off_when_the_probe_contradicts_the_confirmation() {
        // The font said yes, the terminal said no: off beats wrong. This is
        // the case a base-fonts machine with a half-patched setup lands in.
        assert!(!resolve(Setting::Auto, Detected::Confirmed, &broken_probe).is_on());
    }

    // ---- Tier 2, the detection -------------------------------------------

    /// A world that records what it was asked and answers from a script.
    struct Fake {
        term_program: &'static str,
        env: Vec<(&'static str, &'static str)>,
        /// (program, args-prefix) → stdout, matched by first argument.
        runs: Vec<(&'static str, String)>,
        files: Vec<(String, String)>,
    }

    impl Fake {
        fn world(&self) -> World<'_> {
            World {
                term_program: Some(self.term_program.to_string()),
                env: Box::new(|name| {
                    self.env
                        .iter()
                        .find(|(key, _)| *key == name)
                        .map(|(_, value)| value.to_string())
                }),
                read: Box::new(|path| {
                    self.files
                        .iter()
                        .find(|(want, _)| Path::new(want) == path)
                        .map(|(_, text)| text.clone())
                }),
                run: Box::new(|program, _args| {
                    self.runs
                        .iter()
                        .find(|(want, _)| *want == program)
                        .map(|(_, out)| out.clone())
                }),
            }
        }
    }

    /// The `defaults` output iTerm2 writes when its font is patched.
    const ITERM2_WITH_NERD_FONT: &str =
        "\"Normal Font\" = \"MesloLGMNerdFont-Regular 13\";\n\"Default Bookmark Guid\" = \"X\";\n";
    const ITERM2_PLAIN_FONT: &str = "\"Normal Font\" = \"Menlo-Regular 11\";\n";

    #[test]
    fn iterm2_with_a_patched_font_is_confirmed() {
        let fake = Fake {
            term_program: "iTerm.app",
            env: vec![],
            runs: vec![("defaults", ITERM2_WITH_NERD_FONT.to_string())],
            files: vec![],
        };
        assert_eq!(detect(&fake.world()), Detected::Confirmed);
    }

    #[test]
    fn iterm2_with_a_base_font_is_not_confirmed() {
        let fake = Fake {
            term_program: "iTerm.app",
            env: vec![],
            runs: vec![("defaults", ITERM2_PLAIN_FONT.to_string())],
            files: vec![],
        };
        assert_eq!(detect(&fake.world()), Detected::NotConfirmed);
    }

    #[test]
    fn a_terminal_we_cannot_ask_is_unknown() {
        let fake = Fake {
            term_program: "tmux",
            env: vec![],
            runs: vec![],
            files: vec![],
        };
        assert_eq!(detect(&fake.world()), Detected::Unknown);
        let unnamed = Fake {
            term_program: "",
            env: vec![],
            runs: vec![],
            files: vec![],
        };
        assert_eq!(detect(&unnamed.world()), Detected::Unknown);
    }

    #[test]
    fn ghostty_is_confirmed_by_its_config() {
        let fake = Fake {
            term_program: "ghostty",
            env: vec![("HOME", "/home/tester")],
            runs: vec![],
            files: vec![(
                "/home/tester/.config/ghostty/config".to_string(),
                "theme = catppuccin\nfont-family = MonaspiceAr Nerd Font\n".to_string(),
            )],
        };
        assert_eq!(detect(&fake.world()), Detected::Confirmed);
    }

    #[test]
    fn ghostty_with_a_plain_font_is_not_confirmed() {
        let fake = Fake {
            term_program: "ghostty",
            env: vec![("HOME", "/home/tester")],
            runs: vec![],
            files: vec![(
                "/home/tester/.config/ghostty/config".to_string(),
                "font-family = JetBrains Mono\n".to_string(),
            )],
        };
        assert_eq!(detect(&fake.world()), Detected::NotConfirmed);
    }

    #[test]
    fn kitty_is_confirmed_by_its_config_when_it_is_the_host() {
        let fake = Fake {
            term_program: "",
            env: vec![("KITTY_WINDOW_ID", "1"), ("HOME", "/home/tester")],
            runs: vec![],
            files: vec![(
                "/home/tester/.config/kitty/kitty.conf".to_string(),
                "font_family MonaspiceArNerdFont-Regular\n".to_string(),
            )],
        };
        assert_eq!(detect(&fake.world()), Detected::Confirmed);
    }

    #[test]
    fn wezterm_is_confirmed_by_its_own_cli() {
        let fake = Fake {
            term_program: "WezTerm",
            env: vec![],
            runs: vec![(
                "wezterm",
                "/home/tester/.fonts/MonaspiceArNerdFont-Regular.otf\n".to_string(),
            )],
            files: vec![],
        };
        assert_eq!(detect(&fake.world()), Detected::Confirmed);
    }

    #[test]
    fn a_linux_desktop_is_confirmed_by_fontconfig() {
        // TERM_PROGRAM is unset on a bare X11/Wayland session; fontconfig is
        // the one signal left, and it answers for the fonts actually
        // installed rather than for the terminal's own stack.
        let fake = Fake {
            term_program: "",
            env: vec![],
            runs: vec![(
                "fc-list",
                "/usr/share/fonts/MonaspiceArNerdFont-Regular.otf: MonaspiceAr Nerd Font\n"
                    .to_string(),
            )],
            files: vec![],
        };
        assert_eq!(detect(&fake.world()), Detected::Confirmed);
    }
}
