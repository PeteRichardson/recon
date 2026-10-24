//! Filetype icons for the directory listings (#512).
//!
//! A Nerd Font glyph, coloured by file type, leads each row of the explorer
//! and of the file view's directory listing.
//!
//! # Whether to draw them
//!
//! No terminal tells a child process which font it draws with, so `auto`
//! decides from what the host terminal is and what its own preferences
//! name: environment variables and files, read once at startup and never a
//! subprocess or a query to the terminal. Nothing is drawn before the first
//! frame, and nothing waits on an answer.

use std::path::{Path, PathBuf};

use ratatui::style::Color;

use crate::filter::Background;
use crate::widgets::explorer::{Entry, Kind};

/// The user's setting for filetype icons: `--icons`, `RECON_ICONS` or the
/// `[view] icons` key (#512).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Setting {
    /// Draw them when the host terminal is known to be able to.
    #[default]
    Auto,
    /// Always draw them. A font without the glyphs shows boxes, in column.
    On,
    /// Never draw them.
    Off,
}

/// Whether listings draw icons, and in which background's colours:
/// resolved once at startup and pushed into the panes and the read jobs,
/// the way `Background` is. Nothing is decided at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Icons {
    /// No icons: every row exactly as it was before #512.
    #[default]
    Off,
    /// Icons, coloured for this background.
    On(Background),
}

/// One row's icon: the glyph, and the colour it takes when nothing else
/// styles the row. A kind the explorer already colours lends its icon that
/// colour, so a blue directory wears a blue folder; `None` is a plain file
/// no rule knows, drawn in the terminal's own foreground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Icon {
    pub(crate) glyph: char,
    pub(crate) colour: Option<Color>,
}

/// The columns an icon takes at the head of a row: the glyph, then one
/// space. Both panes measure their name column with it.
pub(crate) const ICON_COLUMNS: usize = 2;

/// A directory: a folder.
const DIR: char = '\u{f07b}'; // fa-folder
/// `..`: an arrow up and out.
const PARENT: char = '\u{f148}'; // fa-level-up
/// An executable no name rule knows: a bolt, for "this runs".
const EXECUTABLE: char = '\u{f0e7}'; // fa-bolt
/// A FIFO, socket or device: a plug.
const SPECIAL: char = '\u{f1e6}'; // fa-plug
/// A plain file no name rule knows, in place of devicons' `*`.
const PLAIN: char = '\u{f15b}'; // fa-file

impl Icons {
    /// The icon `entry` leads with, or `None` when icons are off.
    pub(crate) fn of(self, entry: &Entry) -> Option<Icon> {
        let Self::On(background) = self else {
            return None;
        };
        let own = |glyph| Icon {
            glyph,
            colour: entry.style(background.dim_style()).fg,
        };
        Some(match entry.kind {
            Kind::Dir => own(DIR),
            Kind::Parent => own(PARENT),
            Kind::Special => own(SPECIAL),
            Kind::Executable => by_name(&entry.name, background).unwrap_or(own(EXECUTABLE)),
            Kind::Plain => by_name(&entry.name, background).unwrap_or(own(PLAIN)),
        })
    }
}

/// devicons' icon for a file named `name`, or `None` when no rule knows
/// it.
///
/// Its whole-name rules are case-sensitive (`Dockerfile`, but `license`),
/// so a name it misses is asked again lowercased: `LICENSE` finds the key.
///
/// devicons asks the filesystem whether the path is a directory when no
/// name rule matches. The entry's kind already answers that, so the name
/// is passed behind a NUL, which no path can hold: the check fails before
/// any system call, and a directory in the working directory that happens
/// to share the name cannot answer for the file.
fn by_name(name: &std::ffi::OsStr, background: Background) -> Option<Icon> {
    let theme = || match background {
        Background::Dark => devicons::Theme::Dark,
        Background::Light => devicons::Theme::Light,
    };
    let name = name.to_string_lossy();
    let lookup = |name: &str| {
        let path = PathBuf::from(format!("\0/{name}"));
        let icon = devicons::icon_for_file(path.as_path(), &Some(theme()));
        (icon != devicons::FileIcon::default()).then_some(icon)
    };
    let lower = || {
        let lower = name.to_lowercase();
        (lower != name).then(|| lookup(&lower)).flatten()
    };
    let icon = lookup(&name).or_else(lower)?;
    // A glyph the layout would measure at anything but one column would
    // shift the row (#97); the table has none today, and a new release
    // that adds one falls back to the generic glyph rather than misalign.
    if unicode_width::UnicodeWidthChar::width(icon.icon) != Some(1) {
        return None;
    }
    Some(Icon {
        glyph: icon.icon,
        colour: icon.color.parse().ok(),
    })
}

/// What `auto` may look at, so a test can pass its own: an environment
/// variable's value, a file's bytes, and the names in a directory.
pub(crate) struct Host<'a> {
    pub(crate) env: &'a dyn Fn(&str) -> Option<String>,
    pub(crate) read: &'a dyn Fn(&Path) -> Option<Vec<u8>>,
    pub(crate) list: &'a dyn Fn(&Path) -> Vec<String>,
}

/// The icons for this session: `setting` resolved against the real
/// environment and filesystem, in `background`'s colours. Logs the answer,
/// and how to force icons when `auto` leaves them off.
#[must_use]
pub fn resolve_startup(setting: Setting, background: Background) -> Icons {
    let env = |name: &str| std::env::var(name).ok();
    let read = |path: &Path| std::fs::read(path).ok();
    let list = |dir: &Path| {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        Some(entry.ok()?.file_name().to_string_lossy().into_owned())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let host = Host {
        env: &env,
        read: &read,
        list: &list,
    };
    let on = resolve(setting, &host);
    match (setting, on) {
        (Setting::Auto, false) => log::debug!(
            "icons: off — no Nerd Font found for this terminal; set icons = \"on\" to force them"
        ),
        _ => log::debug!("icons: {} ({setting:?})", if on { "on" } else { "off" }),
    }
    if on {
        Icons::On(background)
    } else {
        Icons::Off
    }
}

/// Whether listings draw icons under `setting`, on `host`.
pub(crate) fn resolve(setting: Setting, host: &Host<'_>) -> bool {
    match setting {
        Setting::On => true,
        Setting::Off => false,
        Setting::Auto => {
            let home = (host.env)("HOME").map(PathBuf::from);
            let prefs = |domain: &str| {
                home.as_ref().is_some_and(|home| {
                    let path = home.join(format!("Library/Preferences/{domain}.plist"));
                    (host.read)(&path)
                        .is_some_and(|bytes| names_nerd_font(&String::from_utf8_lossy(&bytes)))
                })
            };
            let installed = || {
                let user = home.as_ref().map(|home| home.join("Library/Fonts"));
                user.into_iter()
                    .chain([PathBuf::from("/Library/Fonts")])
                    .any(|dir| (host.list)(&dir).iter().any(|name| names_nerd_font(name)))
            };
            match terminal(host) {
                Some(Terminal::Bundles) => true,
                Some(Terminal::ITerm2) => prefs("com.googlecode.iterm2") || installed(),
                Some(Terminal::Apple) => prefs("com.apple.Terminal"),
                None => false,
            }
        }
    }
}

/// The host terminals `auto` knows how to answer for.
enum Terminal {
    /// Ghostty, `WezTerm` and kitty (0.36 and later), which ship the Nerd
    /// Font symbols and fall back to them whatever font is configured.
    Bundles,
    /// iTerm2, which falls back through every installed font: a Nerd Font
    /// that a profile names, or one that is merely installed, will do.
    ITerm2,
    /// Terminal.app, which draws a glyph from the profile's own font or not
    /// at all: only a profile that names a Nerd Font will do.
    Apple,
}

/// Whether `text` holds a Nerd Font's name, in any of the spellings its
/// family and PostScript names take: `Meslo Nerd Font`,
/// `MesloLGMNerdFont-Regular`, `JetBrainsMonoNF-Regular`,
/// `JetBrainsMonoNFM-Bold`. The `NF` forms want a lowercase letter before
/// and a `-` after, so `conf`, `NFKC` and `INFO-` are not names.
fn names_nerd_font(text: &str) -> bool {
    static NAME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i:nerd ?font)|[a-z]NF[MP]?-").expect("a valid pattern")
    });
    NAME.is_match(text)
}

/// Which terminal recon runs in, from `TERM_PROGRAM` or, where tmux has
/// replaced that, from the variables the terminal itself set: tmux keeps
/// those.
fn terminal(host: &Host<'_>) -> Option<Terminal> {
    let set = |name: &str| (host.env)(name).is_some_and(|value| !value.is_empty());
    let program = (host.env)("TERM_PROGRAM").unwrap_or_default();
    if matches!(program.as_str(), "ghostty" | "WezTerm")
        || set("GHOSTTY_RESOURCES_DIR")
        || set("WEZTERM_PANE")
        || set("KITTY_WINDOW_ID")
    {
        return Some(Terminal::Bundles);
    }
    match program.as_str() {
        "iTerm.app" => Some(Terminal::ITerm2),
        "Apple_Terminal" => Some(Terminal::Apple),
        _ if (host.env)("LC_TERMINAL").as_deref() == Some("iTerm2") => Some(Terminal::ITerm2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A host with these environment variables, files and font
    /// directories, and nothing else.
    #[derive(Default)]
    struct Fake {
        env: HashMap<&'static str, &'static str>,
        files: HashMap<PathBuf, Vec<u8>>,
        dirs: HashMap<PathBuf, Vec<&'static str>>,
    }

    impl Fake {
        fn env(mut self, name: &'static str, value: &'static str) -> Self {
            self.env.insert(name, value);
            self
        }

        fn file(mut self, path: &str, bytes: &[u8]) -> Self {
            self.files.insert(PathBuf::from(path), bytes.to_vec());
            self
        }

        fn fonts(mut self, dir: &str, names: Vec<&'static str>) -> Self {
            self.dirs.insert(PathBuf::from(dir), names);
            self
        }

        /// iTerm2 under `/Users/me`, its preferences naming `font`.
        fn iterm(font: &str) -> Self {
            Self::default()
                .env("TERM_PROGRAM", "iTerm.app")
                .env("HOME", "/Users/me")
                .file(ITERM_PREFS, &binary_plist_naming(font))
        }

        /// Terminal.app under `/Users/me`, its preferences naming `font`.
        fn terminal_app(font: &str) -> Self {
            Self::default()
                .env("TERM_PROGRAM", "Apple_Terminal")
                .env("HOME", "/Users/me")
                .file(TERMINAL_PREFS, &binary_plist_naming(font))
        }

        fn resolve(&self, setting: Setting) -> bool {
            let env = |name: &str| self.env.get(name).map(|value| (*value).to_string());
            let read = |path: &Path| self.files.get(path).cloned();
            let list = |dir: &Path| {
                self.dirs
                    .get(dir)
                    .map(|names| names.iter().map(|name| (*name).to_string()).collect())
                    .unwrap_or_default()
            };
            resolve(
                setting,
                &Host {
                    env: &env,
                    read: &read,
                    list: &list,
                },
            )
        }
    }

    #[test]
    fn auto_draws_icons_in_ghostty_which_bundles_the_glyphs() {
        assert!(
            Fake::default()
                .env("TERM_PROGRAM", "ghostty")
                .resolve(Setting::Auto)
        );
    }

    #[test]
    fn auto_draws_nothing_in_a_terminal_it_cannot_ask() {
        assert!(
            !Fake::default()
                .env("TERM_PROGRAM", "Hyper")
                .resolve(Setting::Auto)
        );
        assert!(!Fake::default().resolve(Setting::Auto));
    }

    #[test]
    fn auto_draws_icons_in_wezterm_and_kitty_which_bundle_them_too() {
        assert!(
            Fake::default()
                .env("TERM_PROGRAM", "WezTerm")
                .resolve(Setting::Auto)
        );
        // kitty sets no TERM_PROGRAM; its window id names it.
        assert!(
            Fake::default()
                .env("KITTY_WINDOW_ID", "1")
                .resolve(Setting::Auto)
        );
    }

    #[test]
    fn auto_sees_through_tmux_to_a_terminal_that_bundles_them() {
        // tmux replaces TERM_PROGRAM, but keeps the variables the outer
        // terminal set.
        let tmux = || Fake::default().env("TERM_PROGRAM", "tmux");
        assert!(
            tmux()
                .env("GHOSTTY_RESOURCES_DIR", "/x")
                .resolve(Setting::Auto)
        );
        assert!(tmux().env("WEZTERM_PANE", "0").resolve(Setting::Auto));
        assert!(tmux().env("KITTY_WINDOW_ID", "1").resolve(Setting::Auto));
        assert!(!tmux().resolve(Setting::Auto));
    }

    const ITERM_PREFS: &str = "/Users/me/Library/Preferences/com.googlecode.iterm2.plist";
    const TERMINAL_PREFS: &str = "/Users/me/Library/Preferences/com.apple.Terminal.plist";

    /// A binary property list as macOS writes one: `font` sits in it as
    /// plain ASCII among bytes that are not text.
    fn binary_plist_naming(font: &str) -> Vec<u8> {
        let mut bytes = b"bplist00\xd4\x01\x02\x03\x04_\x10\x1aNormal Font_".to_vec();
        bytes.extend_from_slice(font.as_bytes());
        bytes.extend_from_slice(b"\x08\x11\x00\xff");
        bytes
    }

    #[test]
    fn auto_draws_icons_in_iterm2_when_a_profile_names_a_nerd_font() {
        // The three spellings Nerd Fonts' PostScript and family names take.
        for font in [
            "MesloLGMNerdFont-Regular 13",
            "JetBrainsMonoNF-Regular 14",
            "MonaspiceNe Nerd Font Mono",
        ] {
            assert!(Fake::iterm(font).resolve(Setting::Auto), "{font}");
        }
    }

    #[test]
    fn auto_draws_icons_in_iterm2_when_a_nerd_font_is_installed_at_all() {
        // iTerm2 falls back through every installed font for a glyph its
        // own font lacks, so the configured font need not be the patched one.
        let fake = Fake::iterm("SFMono-Regular 13").fonts(
            "/Users/me/Library/Fonts",
            vec!["Inter.ttf", "MonaspaceArgonNF-Bold.otf"],
        );
        assert!(fake.resolve(Setting::Auto));
        let fake = Fake::iterm("SFMono-Regular 13")
            .fonts("/Library/Fonts", vec!["SymbolsNerdFont-Regular.ttf"]);
        assert!(fake.resolve(Setting::Auto));
    }

    #[test]
    fn auto_draws_nothing_in_iterm2_without_a_nerd_font_anywhere() {
        let fake = Fake::iterm("SFMono-Regular 13")
            .fonts("/Users/me/Library/Fonts", vec!["Inter.ttf", "Menlo.ttc"]);
        assert!(!fake.resolve(Setting::Auto));
    }

    #[test]
    fn auto_finds_iterm2_inside_tmux_by_the_variable_it_sets() {
        let fake = Fake::iterm("JetBrainsMonoNF-Regular 14")
            .env("TERM_PROGRAM", "tmux")
            .env("LC_TERMINAL", "iTerm2");
        assert!(fake.resolve(Setting::Auto));
    }

    #[test]
    fn auto_trusts_only_terminal_apps_configured_font() {
        assert!(Fake::terminal_app("MesloLGSNerdFontMono-Regular").resolve(Setting::Auto));
        // Terminal.app draws a glyph from its own font or not at all, so an
        // installed Nerd Font it is not set to use does not count.
        let fake = Fake::terminal_app("SFMonoTerminal-Regular").fonts(
            "/Users/me/Library/Fonts",
            vec!["MesloLGSNerdFontMono-Regular.ttf"],
        );
        assert!(!fake.resolve(Setting::Auto));
    }

    #[test]
    fn a_word_that_merely_contains_nf_names_no_nerd_font() {
        // `conf`, `NFKC`, `INFO-` all turn up in preference files.
        let fake = Fake::terminal_app("SFMonoTerminal-Regular Window.conf NFKC INFO-1");
        assert!(!fake.resolve(Setting::Auto));
    }

    #[test]
    fn on_and_off_are_obeyed_whatever_the_terminal() {
        let ghostty = Fake::default().env("TERM_PROGRAM", "ghostty");
        assert!(!ghostty.resolve(Setting::Off));
        assert!(Fake::default().resolve(Setting::On));
    }
}
