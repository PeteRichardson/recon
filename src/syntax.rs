//! Syntax colouring for the file view (#122).
//!
//! `syntect` parses each line against a Sublime grammar and hands back byte
//! ranges carrying a theme colour; `FileView` paints those through the
//! vendored textarea's `custom_highlight`. The grammar and theme bundles are
//! bat's, via `two-face`, rather than syntect's own — see `Cargo.toml` for
//! why. Everything that is a *decision* rather than glue lives here:
//!
//! * **Colour arrives lazily, in file order.** A grammar's state at line N
//!   depends on every line before it, so a file cannot be coloured from the
//!   middle; nor can it be coloured whole on open, since at roughly 13 µs a
//!   line a 10 MiB log would stall the navigator for seconds on every arrow
//!   key. [`Highlighter::ensure`] colours forward from where it left off when
//!   the wanted line is near, and otherwise *resyncs*: it restarts the grammar
//!   [`RESYNC_LOOKBACK`] lines above the wanted one and accepts that a
//!   construct opened further up — a block comment, a raw string — is
//!   coloured wrong until it closes. Every editor makes the same trade on a
//!   jump to the end of a large file; what it buys is a cost bounded by the
//!   lookback rather than by the file.
//! * **A theme is a `&'static`.** Bundled themes live in a process-wide set;
//!   one loaded from a `.tmTheme` file is leaked on purpose. A theme is
//!   process-lifetime configuration parsed once, and `'static` is what lets
//!   the per-file parser state be stored without a lifetime parameter
//!   threading through `FileView` and `App`.
//! * **Terminal colours are encoded in the alpha channel.** bat's `ansi` and
//!   `base16` themes name the terminal's own palette slots rather than RGB
//!   values, using the convention [`colour`] decodes. `ansi` is the default
//!   for the same reason the navigator's blue and green are ANSI slots: the
//!   result follows the terminal's theme instead of fighting it, and it needs
//!   no truecolor support.

use ratatui::style::{Color, Modifier, Style};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SynColor, FontStyle, Style as SynStyle, Theme as SynTheme, ThemeSet,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use two_face::theme::LazyThemeSet;

/// The theme used when neither the CLI, the environment nor `config.toml`
/// names one. See the module docs for why it is the terminal-palette theme
/// rather than a truecolor one.
pub const DEFAULT_THEME: &str = "ansi";

/// The spellings that turn colouring off, compared case-insensitively.
const OFF_SPELLINGS: [&str; 2] = ["none", "off"];

/// How far ahead of the parser a wanted line may be before colouring resyncs
/// instead of parsing every line in between.
///
/// A page is well under a hundred lines, so ordinary scrolling — `j`, `Ctrl-D`,
/// `]` — always continues from where the parser stopped and stays exactly
/// right. `G` on a large file, or a filter that shows lines thousands apart,
/// resyncs. Deliberately not larger: the worst case per frame is one gap per
/// buffer row, so this bounds a frame's parsing at rows × gap.
const RESYNC_GAP: usize = 256;

/// How many lines above a wanted line a resync starts parsing.
///
/// Enough to close most block comments and doc strings that a jump lands
/// inside; small enough that a window of scattered filter hits costs a few
/// thousand lines rather than tens of thousands.
const RESYNC_LOOKBACK: usize = 64;

/// A line longer than this is left uncoloured.
///
/// Grammar regexes are written for source lines, and a minified megabyte on
/// one line can cost seconds. Ten thousand bytes is past any line a person
/// reads and well under what a single-line data file is.
const MAX_LINE_BYTES: usize = 10_000;

/// The grammars, loaded once per process. Two milliseconds in release, so a
/// `OnceLock` rather than a field passed around.
fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// The bundled themes. `LazyThemeSet` deserialises each theme on first use,
/// so naming one costs only that one.
fn bundled() -> &'static LazyThemeSet {
    static THEMES: OnceLock<LazyThemeSet> = OnceLock::new();
    THEMES.get_or_init(|| LazyThemeSet::from(two_face::theme::extra()))
}

/// Every bundled theme name, for `--theme` and the error message that lists
/// them.
pub fn bundled_names() -> impl Iterator<Item = &'static str> {
    bundled().theme_names()
}

/// Which colours the file view paints, if any.
///
/// `Copy` because the loaded theme is `'static` — see the module docs. The
/// `Off` variant exists so that `--theme none` is a *value* that beats a
/// `[syntax] theme` in the file, exactly as any other CLI setting does; an
/// `Option` could not express "set, and set to nothing".
#[derive(Clone, Copy, Default)]
pub enum Theme {
    /// No colouring: the file view renders as it did before #122.
    #[default]
    Off,
    /// Colour with this theme.
    On(&'static SynTheme),
}

impl Theme {
    /// [`DEFAULT_THEME`], resolved.
    ///
    /// # Panics
    ///
    /// If the bundled set does not contain [`DEFAULT_THEME`], which
    /// `the_default_theme_is_bundled` pins.
    #[must_use]
    pub fn builtin() -> Self {
        DEFAULT_THEME
            .parse()
            .expect("DEFAULT_THEME names a bundled theme")
    }

    /// The theme's own name, or `None` when off. A theme file without a
    /// `name` key reports its path — see `FromStr`.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Off => None,
            Self::On(theme) => theme.name.as_deref(),
        }
    }
}

impl PartialEq for Theme {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Off, Self::Off) => true,
            (Self::On(a), Self::On(b)) => std::ptr::eq(*a, *b),
            _ => false,
        }
    }
}

impl Eq for Theme {}

impl fmt::Debug for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            None => f.write_str("Off"),
            Some(name) => write!(f, "On({name:?})"),
        }
    }
}

/// Why a theme spelling could not be turned into a [`Theme`].
#[derive(Debug)]
pub enum ThemeError {
    /// Neither `none`, a bundled name, nor an existing file.
    Unknown(String),
    /// A file that exists but is not a `.tmTheme` syntect can read.
    Unreadable {
        path: PathBuf,
        source: syntect::LoadingError,
    },
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(spec) => {
                let names = bundled_names().collect::<Vec<_>>().join(", ");
                write!(
                    f,
                    "{spec:?} is not a bundled theme or a readable .tmTheme file. \
                     Bundled themes: {names}. \"none\" turns colouring off"
                )
            }
            Self::Unreadable { path, source } => {
                write!(f, "could not load theme {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ThemeError {}

impl FromStr for Theme {
    type Err = ThemeError;

    /// `none`/`off`, then a bundled name, then a path to a `.tmTheme` file.
    ///
    /// The bundled name wins over a file of the same name in the working
    /// directory, so that `--theme Dracula` means the same thing in every
    /// directory. Names match case-insensitively — `dracula` is accepted, and
    /// the theme reports its own spelling — but a path is used as written.
    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        if OFF_SPELLINGS
            .iter()
            .any(|off| off.eq_ignore_ascii_case(spec))
        {
            return Ok(Self::Off);
        }
        if let Some(name) = bundled_names().find(|name| name.eq_ignore_ascii_case(spec)) {
            let theme = bundled()
                .get(name)
                .expect("a name the set reported is in the set");
            return Ok(Self::On(theme));
        }
        let path = Path::new(spec);
        if !path.is_file() {
            return Err(ThemeError::Unknown(spec.to_string()));
        }
        let mut theme = ThemeSet::get_theme(path).map_err(|source| ThemeError::Unreadable {
            path: path.to_path_buf(),
            source,
        })?;
        // A file's `name` key is optional, and a theme with no name renders
        // as `On("")` in a debug log — the path is what the user typed and
        // what they would grep for.
        theme.name.get_or_insert_with(|| spec.to_string());
        // Leaked, see the module docs: one theme per process, parsed once.
        Ok(Self::On(Box::leak(Box::new(theme))))
    }
}

/// One coloured byte range within a line. `start..end` is half-open and
/// indexes the line's bytes, which is what `custom_highlight` compares
/// against — syntect returns `&str` slices, so no column conversion is
/// needed in either direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

/// A syntect colour as a ratatui one, decoding bat's terminal-palette
/// convention.
///
/// `#RRGGBBAA` with `AA = 00` names terminal palette slot `RR`: the sixteen
/// ANSI colours by their named variants, so they render on a 16-colour
/// terminal, and anything above by index. `AA = 01` is the terminal's
/// *default* colour — `None`, no escape at all. Everything else is the RGB
/// value it says it is.
fn colour(colour: SynColor) -> Option<Color> {
    match colour.a {
        0 => Some(match colour.r {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            15 => Color::White,
            index => Color::Indexed(index),
        }),
        1 => None,
        _ => Some(Color::Rgb(colour.r, colour.g, colour.b)),
    }
}

/// A syntect style as a ratatui one: foreground and font style only.
///
/// The background is dropped deliberately. A custom highlight *replaces* the
/// line's style within its range rather than layering on it, so a theme
/// background would punch holes in the cursor line and in any filter colour —
/// and every bundled theme's background is the one it expects the whole
/// pane to have, which recon does not paint.
fn style(style: SynStyle) -> Style {
    let mut out = Style::default();
    if let Some(fg) = colour(style.foreground) {
        out = out.fg(fg);
    }
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

/// The grammar for `path`, or `None` when the file should stay uncoloured.
///
/// The whole file name is tried before the extension, which is what catches
/// `Makefile`, `Dockerfile` and `.zshrc`; the first line is the fallback for
/// an extensionless script with a shebang. Plain text is a grammar the set
/// does define — `.txt` maps to it — and is reported as `None`, since running
/// a parser that colours nothing is pure cost.
fn detect(path: &Path, first_line: Option<&str>) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let by_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| set.find_syntax_by_extension(name));
    let by_extension = || {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| set.find_syntax_by_extension(ext))
    };
    let by_first_line = || first_line.and_then(|line| set.find_syntax_by_first_line(line));
    let syntax = by_name.or_else(by_extension).or_else(by_first_line)?;
    (!std::ptr::eq(syntax, set.find_syntax_plain_text())).then_some(syntax)
}

/// The colouring of one file, filled in as lines are asked for.
///
/// Holds the grammar's running state and one slot per source line. Built by
/// [`Highlighter::for_file`] when a file is loaded or previewed; the view asks
/// [`Highlighter::ensure`] for each row it is about to draw and reads the
/// result back with [`Highlighter::spans`].
pub struct Highlighter {
    theme: &'static SynTheme,
    syntax: &'static SyntaxReference,
    state: HighlightLines<'static>,
    /// The source line `state` is positioned to parse next.
    next: usize,
    /// One entry per source line, `None` until that line has been coloured.
    spans: Vec<Option<Vec<Span>>>,
}

/// Names rather than contents: the parser state is opaque and the spans
/// run to one entry per line of the file.
impl fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Highlighter")
            .field("syntax", &self.syntax.name)
            .field("theme", &self.theme.name)
            .field("next", &self.next)
            .field("lines", &self.spans.len())
            .finish_non_exhaustive()
    }
}

impl Highlighter {
    /// A highlighter for `lines` read from `path`, or `None` when there is
    /// nothing to colour with — the theme is off, or no grammar claims the
    /// file. Costs nothing beyond the grammar lookup: no line is parsed
    /// until [`ensure`](Self::ensure) asks for it.
    #[must_use]
    pub fn for_file(theme: Theme, path: &Path, lines: &[String]) -> Option<Self> {
        let Theme::On(theme) = theme else {
            return None;
        };
        let syntax = detect(path, lines.first().map(String::as_str))?;
        Some(Self {
            theme,
            syntax,
            state: HighlightLines::new(syntax, theme),
            next: 0,
            spans: vec![None; lines.len()],
        })
    }

    /// The grammar's own name — `Rust`, `JSON`, `Bourne Again Shell (bash)`.
    #[must_use]
    pub fn syntax_name(&self) -> &str {
        &self.syntax.name
    }

    /// Make sure source line `row` of `lines` is coloured, spending at most
    /// `budget` lines of parsing to get there. Returns whether it is.
    ///
    /// `lines` must be the same lines `for_file` was built over. Continues
    /// from where the parser stopped when `row` is within [`RESYNC_GAP`] of
    /// it, and otherwise resyncs [`RESYNC_LOOKBACK`] lines above `row` — see
    /// the module docs. The budget is what keeps a frame's worth of scattered
    /// rows from stalling the app: a row that would overspend is left plain
    /// this time and asked for again on the next frame.
    pub fn ensure(&mut self, lines: &[String], row: usize, budget: &mut usize) -> bool {
        debug_assert_eq!(
            lines.len(),
            self.spans.len(),
            "coloured over different lines"
        );
        if row >= self.spans.len() {
            return false;
        }
        if self.spans[row].is_some() {
            return true;
        }
        let start = if self.next <= row && row - self.next <= RESYNC_GAP {
            self.next
        } else {
            row.saturating_sub(RESYNC_LOOKBACK)
        };
        let work = row + 1 - start;
        if work > *budget {
            return false;
        }
        *budget -= work;
        if start != self.next {
            self.state = HighlightLines::new(self.syntax, self.theme);
        }
        for (slot, line) in self.spans[start..=row].iter_mut().zip(&lines[start..=row]) {
            *slot = Some(Self::colour_line(&mut self.state, line));
        }
        self.next = row + 1;
        true
    }

    /// The coloured ranges of source line `row`: empty until
    /// [`ensure`](Self::ensure) has reached it, and empty for a line that
    /// needs no colour.
    #[must_use]
    pub fn spans(&self, row: usize) -> &[Span] {
        self.spans
            .get(row)
            .and_then(Option::as_deref)
            .unwrap_or(&[])
    }

    /// Parse one line through `state`, advancing it, and report the
    /// coloured ranges. A function of the state rather than `&mut self` so
    /// `ensure` can hold `spans` borrowed alongside it.
    fn colour_line(state: &mut HighlightLines<'static>, line: &str) -> Vec<Span> {
        if line.len() > MAX_LINE_BYTES {
            return Vec::new();
        }
        // The grammars are the `newlines` variants, which expect the
        // terminator they were written against; without it a construct that
        // ends at end-of-line is left open into the next.
        let with_newline = format!("{line}\n");
        let Ok(regions) = state.highlight_line(&with_newline, syntaxes()) else {
            return Vec::new();
        };
        let mut spans: Vec<Span> = Vec::new();
        let mut offset = 0;
        for (region_style, text) in regions {
            let start = offset;
            offset += text.len();
            // Drop the synthetic newline, and with it any region that was
            // only ever the newline.
            let end = offset.min(line.len());
            let style = style(region_style);
            if end <= start || style == Style::default() {
                continue;
            }
            // syntect reports a region per scope change, and most changes
            // resolve to the same colour under any theme. Merging keeps the
            // count the render loop walks per row proportional to the
            // colours on the line rather than to its tokens.
            match spans.last_mut() {
                Some(last) if last.end == start && last.style == style => last.end = end,
                _ => spans.push(Span { start, end, style }),
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn rust(theme: Theme, text: &str) -> (Highlighter, Vec<String>) {
        let lines = lines(text);
        let highlighter = Highlighter::for_file(theme, Path::new("main.rs"), &lines)
            .expect("Rust is a bundled grammar");
        (highlighter, lines)
    }

    #[test]
    fn the_default_theme_is_bundled() {
        assert!(bundled_names().any(|name| name == DEFAULT_THEME));
        assert!(matches!(Theme::builtin(), Theme::On(_)));
    }

    #[test]
    fn none_and_off_turn_colouring_off_in_any_case() {
        for spec in ["none", "NONE", "off", "Off"] {
            assert_eq!(spec.parse::<Theme>().unwrap(), Theme::Off, "{spec}");
        }
    }

    #[test]
    fn a_bundled_name_is_matched_case_insensitively() {
        let theme: Theme = "dracula".parse().unwrap();
        assert_eq!(theme.name(), Some("Dracula"));
        assert_eq!(theme, "Dracula".parse().unwrap());
    }

    #[test]
    fn an_unknown_spelling_lists_the_bundled_themes() {
        let err = "no-such-theme".parse::<Theme>().unwrap_err();
        let message = err.to_string();
        assert!(message.contains("\"no-such-theme\""), "{message}");
        assert!(message.contains("Dracula"), "{message}");
        assert!(message.contains("\"none\""), "{message}");
    }

    #[test]
    fn a_tmtheme_file_is_loaded_by_path() {
        let dir = Path::new("target/test-themes");
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("loaded-by-path.tmTheme");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>name</key><string>Test Theme</string>
  <key>settings</key>
  <array>
    <dict><key>settings</key><dict>
      <key>foreground</key><string>#00000001</string>
    </dict></dict>
    <dict>
      <key>scope</key><string>storage, keyword</string>
      <key>settings</key><dict><key>foreground</key><string>#FF0000</string></dict>
    </dict>
  </array>
</dict>
</plist>
"#,
        )
        .unwrap();
        let theme: Theme = path.to_str().unwrap().parse().unwrap();
        assert_eq!(theme.name(), Some("Test Theme"));

        let (mut highlighter, lines) = {
            let lines = lines("fn main() {}");
            let highlighter = Highlighter::for_file(theme, Path::new("x.rs"), &lines).unwrap();
            (highlighter, lines)
        };
        assert!(highlighter.ensure(&lines, 0, &mut 10));
        let spans = highlighter.spans(0);
        assert_eq!(
            spans,
            [Span {
                start: 0,
                end: 2,
                style: Style::default().fg(Color::Rgb(255, 0, 0)),
            }],
            "only `fn` is storage, and the file's default foreground is the terminal's"
        );
    }

    #[test]
    fn a_file_that_is_not_a_theme_is_reported_with_its_path() {
        let dir = Path::new("target/test-themes");
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("not-a-theme.tmTheme");
        std::fs::write(&path, "this is not a plist").unwrap();
        let err = path.to_str().unwrap().parse::<Theme>().unwrap_err();
        assert!(matches!(err, ThemeError::Unreadable { .. }), "{err}");
        assert!(err.to_string().contains("not-a-theme.tmTheme"), "{err}");
    }

    #[test]
    fn terminal_palette_slots_decode_to_named_colours() {
        let slot = |r| SynColor {
            r,
            g: 0,
            b: 0,
            a: 0,
        };
        assert_eq!(colour(slot(1)), Some(Color::Red));
        assert_eq!(colour(slot(9)), Some(Color::LightRed));
        assert_eq!(colour(slot(15)), Some(Color::White));
        assert_eq!(colour(slot(208)), Some(Color::Indexed(208)));
        assert_eq!(
            colour(SynColor {
                r: 0,
                g: 0,
                b: 0,
                a: 1
            }),
            None,
            "alpha 1 is the terminal's default colour"
        );
        assert_eq!(
            colour(SynColor {
                r: 1,
                g: 2,
                b: 3,
                a: 255
            }),
            Some(Color::Rgb(1, 2, 3))
        );
    }

    #[test]
    fn a_theme_off_or_an_unknown_file_yields_no_highlighter() {
        let lines = lines("fn main() {}");
        assert!(Highlighter::for_file(Theme::Off, Path::new("main.rs"), &lines).is_none());
        let theme = Theme::builtin();
        assert!(Highlighter::for_file(theme, Path::new("notes.txt"), &lines).is_none());
        assert!(Highlighter::for_file(theme, Path::new("data.xyz123"), &lines).is_none());
    }

    #[test]
    fn a_grammar_is_found_by_extension_by_file_name_and_by_shebang() {
        let theme = Theme::builtin();
        let name = |path: &str, first: &str| {
            Highlighter::for_file(theme, Path::new(path), &lines(first))
                .map(|highlighter| highlighter.syntax_name().to_string())
        };
        assert_eq!(name("src/lib.rs", "use std;").as_deref(), Some("Rust"));
        assert_eq!(name("Cargo.toml", "[package]").as_deref(), Some("TOML"));
        assert_eq!(
            name("App.swift", "import Foundation").as_deref(),
            Some("Swift")
        );
        assert_eq!(name("Makefile", "all:").as_deref(), Some("Makefile"));
        assert_eq!(name("Dockerfile", "FROM x").as_deref(), Some("Dockerfile"));
        assert!(name(".zshrc", "export X=1").is_some());
        assert!(name("run", "#!/bin/sh").is_some(), "found by shebang");
        assert_eq!(name("run", "no shebang here"), None);
    }

    #[test]
    fn keywords_and_comments_take_the_theme_s_colours() {
        let (mut highlighter, lines) = rust(Theme::builtin(), "fn main() {} // hi");
        assert!(highlighter.ensure(&lines, 0, &mut 10));
        let spans = highlighter.spans(0);
        let keyword = spans
            .iter()
            .find(|span| span.start == 0)
            .expect("`fn` is coloured");
        assert_eq!(keyword.end, 2);
        assert_eq!(
            keyword.style.fg,
            Some(Color::Magenta),
            "ansi maps keywords to slot 5"
        );
        let comment = spans
            .iter()
            .find(|span| span.start == 13)
            .expect("`// hi` is coloured");
        assert_eq!(
            comment.end,
            lines[0].len(),
            "the synthetic newline is dropped"
        );
        assert_eq!(
            comment.style.fg,
            Some(Color::Green),
            "ansi maps comments to slot 2"
        );
    }

    #[test]
    fn a_line_that_needs_no_colour_has_no_spans_but_counts_as_coloured() {
        let (mut highlighter, lines) = rust(Theme::builtin(), "\nmain();");
        assert!(highlighter.ensure(&lines, 0, &mut 10));
        assert!(highlighter.spans(0).is_empty());
        assert!(highlighter.ensure(&lines, 1, &mut 10));
    }

    #[test]
    fn lines_are_coloured_forward_from_the_last_one_asked_for() {
        let (mut highlighter, lines) = rust(Theme::builtin(), &"let x = 1;\n".repeat(10));
        let mut budget = 100;
        assert!(highlighter.ensure(&lines, 4, &mut budget));
        assert_eq!(budget, 95, "lines 0 through 4");
        assert!(highlighter.ensure(&lines, 6, &mut budget));
        assert_eq!(budget, 93, "lines 5 and 6, continuing");
        assert!(highlighter.ensure(&lines, 2, &mut budget));
        assert_eq!(budget, 93, "already coloured: free");
        assert!(
            highlighter
                .spans(3)
                .iter()
                .any(|span| span.style.fg.is_some())
        );
    }

    #[test]
    fn a_far_jump_resyncs_from_a_short_lookback_rather_than_parsing_the_gap() {
        let text = "let x = 1;\n".repeat(RESYNC_GAP * 4);
        let (mut highlighter, lines) = rust(Theme::builtin(), &text);
        let mut budget = usize::MAX;
        let far = RESYNC_GAP * 3;
        assert!(highlighter.ensure(&lines, far, &mut budget));
        assert_eq!(usize::MAX - budget, RESYNC_LOOKBACK + 1);
        assert!(
            highlighter
                .spans(far - RESYNC_LOOKBACK)
                .iter()
                .any(|s| s.style.fg.is_some())
        );
        assert!(
            highlighter.spans(far - RESYNC_LOOKBACK - 1).is_empty(),
            "the gap was skipped"
        );
        assert!(!highlighter.spans(far).is_empty());
    }

    #[test]
    fn scrolling_back_into_a_skipped_region_resyncs_there() {
        let text = "let x = 1;\n".repeat(RESYNC_GAP * 4);
        let (mut highlighter, lines) = rust(Theme::builtin(), &text);
        let mut budget = usize::MAX;
        assert!(highlighter.ensure(&lines, RESYNC_GAP * 3, &mut budget));
        let back = RESYNC_GAP * 2;
        assert!(highlighter.ensure(&lines, back, &mut budget));
        assert!(!highlighter.spans(back).is_empty());
        assert_eq!(usize::MAX - budget, 2 * (RESYNC_LOOKBACK + 1));
    }

    #[test]
    fn a_resync_carries_a_multi_line_construct_across_the_lookback() {
        // A block comment opened inside the lookback is still seen as one.
        let mut text = String::from("/* open\n");
        text.push_str(&"still a comment\n".repeat(RESYNC_LOOKBACK / 2));
        text.push_str("*/\nfn after() {}\n");
        let (mut highlighter, lines) = rust(Theme::builtin(), &text);
        // Colour the tail first, so the head is reached by a resync.
        let last = lines.len() - 1;
        let mut budget = usize::MAX;
        assert!(highlighter.ensure(&lines, last, &mut budget));
        let mid = RESYNC_LOOKBACK / 4;
        let spans = highlighter.spans(mid);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert_eq!(
            spans[0].style.fg,
            Some(Color::Green),
            "coloured as a comment"
        );
    }

    #[test]
    fn a_row_that_would_overspend_the_budget_is_left_for_next_time() {
        let (mut highlighter, lines) = rust(Theme::builtin(), &"let x = 1;\n".repeat(10));
        let mut budget = 3;
        assert!(
            !highlighter.ensure(&lines, 5, &mut budget),
            "six lines wanted, three allowed"
        );
        assert_eq!(budget, 3, "nothing spent");
        assert!(highlighter.spans(5).is_empty());
        assert!(highlighter.ensure(&lines, 2, &mut budget));
        assert_eq!(budget, 0);
    }

    #[test]
    fn a_row_past_the_end_is_refused() {
        let (mut highlighter, lines) = rust(Theme::builtin(), "fn a() {}");
        assert!(!highlighter.ensure(&lines, 1, &mut 10));
        assert!(highlighter.spans(1).is_empty());
    }

    #[test]
    fn an_overlong_line_is_left_plain() {
        let long = format!("let x = \"{}\";", "a".repeat(MAX_LINE_BYTES));
        let (mut highlighter, lines) = rust(Theme::builtin(), &long);
        assert!(highlighter.ensure(&lines, 0, &mut 10));
        assert!(highlighter.spans(0).is_empty());
    }

    #[test]
    fn adjacent_regions_of_one_colour_are_merged() {
        let (mut highlighter, lines) = rust(Theme::builtin(), "let s = \"a b c\";");
        assert!(highlighter.ensure(&lines, 0, &mut 10));
        let spans = highlighter.spans(0);
        for pair in spans.windows(2) {
            assert!(
                pair[0].end < pair[1].start || pair[0].style != pair[1].style,
                "{pair:?} should have been one span"
            );
        }
    }

    #[test]
    fn spans_use_byte_offsets_so_wide_glyphs_do_not_shift_them() {
        let (mut highlighter, lines) = rust(Theme::builtin(), "let s = \"日本\"; // c");
        assert!(highlighter.ensure(&lines, 0, &mut 10));
        let comment_at = lines[0].find("//").unwrap();
        let comment = highlighter
            .spans(0)
            .iter()
            .find(|span| span.start == comment_at)
            .expect("the comment starts at its byte offset");
        assert_eq!(comment.end, lines[0].len());
    }
}
