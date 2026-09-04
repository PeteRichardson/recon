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
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
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

/// Extensions left plain even though the bundle has a grammar for them
/// (#125).
///
/// bat's `log` grammar is generic by design: it colours whatever most logs
/// contain — bare numbers, dates, IPv4 octets, quoted strings, `key=value`,
/// URLs, lines that mention `error` or `warn` — rather than any one format.
/// On a free-form log the visible result is stray yellow numbers and green
/// quotes, which is noise on exactly the files recon is for. Hard-coded until
/// there is a per-extension setting to hang it on; compared case-insensitively,
/// so `.LOG` is a log too.
const PLAIN_EXTENSIONS: [&str; 1] = ["log"];

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
/// [`PLAIN_EXTENSIONS`] are refused before any lookup. Otherwise the whole
/// file name is tried before the extension, which is what catches
/// `Makefile`, `Dockerfile` and `.zshrc`; the first line is the fallback for
/// an extensionless script with a shebang. Plain text is a grammar the set
/// does define — `.txt` maps to it — and is reported as `None`, since running
/// a parser that colours nothing is pure cost.
fn detect(path: &Path, first_line: Option<&str>) -> Option<&'static SyntaxReference> {
    let extension = path.extension().and_then(|ext| ext.to_str());
    if extension.is_some_and(|ext| {
        PLAIN_EXTENSIONS
            .iter()
            .any(|plain| plain.eq_ignore_ascii_case(ext))
    }) {
        return None;
    }
    let set = syntaxes();
    let by_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| set.find_syntax_by_extension(name));
    let by_extension = || extension.and_then(|ext| set.find_syntax_by_extension(ext));
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

// ---- definitions (#123) ----------------------------------------------------

/// A kind of definition a source line can start.
///
/// The four kinds every bundled grammar has some notion of. Traits, impls,
/// modules and typedefs are real too and are left for a later row each; the
/// scope table and keyword map below are the only two places a kind is
/// described, so adding one is two lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Function,
    Class,
    Struct,
    Enum,
    /// Type aliases, typedefs, Go `type` declarations, TypeScript types.
    Type,
    /// Traits, interfaces and protocols: one concept to a reader, three
    /// names across languages (#141).
    Trait,
    /// Modules, namespaces and packages.
    Module,
    /// Rust `impl` blocks and Swift extensions.
    Impl,
    Constant,
    Macro,
    /// Headings in Markdown and `AsciiDoc`: a document outline, from the
    /// same mechanism for free.
    Section,
}

impl Kind {
    pub const ALL: [Self; 11] = [
        Self::Function,
        Self::Class,
        Self::Struct,
        Self::Enum,
        Self::Type,
        Self::Trait,
        Self::Module,
        Self::Impl,
        Self::Constant,
        Self::Macro,
        Self::Section,
    ];

    /// The `TextMate` scopes a well-behaved grammar gives the *name* of a
    /// definition of this kind. Rust, C, Go, Python and JavaScript all
    /// follow the convention; see `keywords` for the ones that do not.
    /// Several scopes where languages spell one concept differently:
    /// `entity.name.trait` in Rust, `.interface` in C#, `.protocol` in
    /// Elixir. Measured across the bundle in #141.
    fn scopes(self) -> &'static [&'static str] {
        match self {
            Self::Function => &["entity.name.function"],
            Self::Class => &["entity.name.class"],
            Self::Struct => &["entity.name.struct"],
            Self::Enum => &["entity.name.enum"],
            Self::Type => &["entity.name.type"],
            Self::Trait => &[
                "entity.name.trait",
                "entity.name.interface",
                "entity.name.protocol",
            ],
            Self::Module => &[
                "entity.name.module",
                "entity.name.namespace",
                "entity.name.package",
            ],
            Self::Impl => &["entity.name.impl"],
            Self::Constant => &["entity.name.constant"],
            Self::Macro => &["entity.name.macro"],
            Self::Section => &["entity.name.section"],
        }
    }

    /// The declaration keywords that start a definition of this kind, for
    /// grammars that scope the keyword (`storage.type`) but leave the name
    /// unscoped. bat's Swift grammar is the one that forced this: `class
    /// Canvas` there is a `storage.type.swift` keyword followed by plain
    /// text, and `func` names itself `entity.type.function.swift`, which is
    /// not the convention either.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Function => &["fn", "func", "def", "fun"],
            Self::Class => &["class"],
            Self::Struct => &["struct"],
            Self::Enum => &["enum"],
            Self::Type => &["type", "typealias", "typedef"],
            Self::Trait => &["trait", "interface", "protocol"],
            Self::Module => &["mod", "namespace", "package"],
            Self::Impl => &["impl", "extension"],
            // `const` only: C's `static` is storage too and is not a
            // definition keyword.
            Self::Constant => &["const"],
            Self::Macro => &["macro_rules"],
            Self::Section => &[],
        }
    }

    /// The plural noun the pane will call a filter of this kind.
    #[must_use]
    pub fn plural(self) -> &'static str {
        match self {
            Self::Function => "functions",
            Self::Class => "classes",
            Self::Struct => "structs",
            Self::Enum => "enums",
            Self::Type => "types",
            Self::Trait => "traits",
            Self::Module => "modules",
            Self::Impl => "impls",
            Self::Constant => "constants",
            Self::Macro => "macros",
            Self::Section => "sections",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.plural())
    }
}

/// The kinds of definition one line starts. A bitset, because a line can
/// start more than one — `trait Area { fn area(&self); }` on one line is
/// both a trait and a function, and that is the right answer for "lines
/// that start a definition".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindSet(u16);

impl KindSet {
    pub const EMPTY: Self = Self(0);

    /// One bit per kind, in `Kind::ALL` order. `u16` holds sixteen kinds;
    /// there are eleven.
    fn bit(kind: Kind) -> u16 {
        let position = Kind::ALL
            .iter()
            .position(|&k| k == kind)
            .expect("every kind is in ALL");
        1 << position
    }

    /// A set holding exactly `kinds`.
    #[must_use]
    pub fn of(kinds: &[Kind]) -> Self {
        kinds.iter().fold(Self::EMPTY, |mut set, &kind| {
            set.insert(kind);
            set
        })
    }

    #[must_use]
    pub fn contains(self, kind: Kind) -> bool {
        self.0 & Self::bit(kind) != 0
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn insert(&mut self, kind: Kind) {
        self.0 |= Self::bit(kind);
    }
}

/// The scopes `definitions` tests against, parsed once per call rather than
/// once per token. `Scope::new` consults the global scope repository, which
/// takes a lock.
struct KindScopes {
    /// One entry per `(kind, scope)` pair; a kind with several spellings
    /// appears several times.
    named: Vec<(Kind, Scope)>,
    storage_type: Scope,
    meta_block: Scope,
}

impl KindScopes {
    fn new() -> Self {
        let scope = |text: &str| Scope::new(text).expect("a literal scope parses");
        Self {
            named: Kind::ALL
                .iter()
                .flat_map(|&kind| kind.scopes().iter().map(move |text| (kind, scope(text))))
                .collect(),
            storage_type: scope("storage.type"),
            meta_block: scope("meta.block"),
        }
    }

    /// What one token says about its line. `text` is the token's own text;
    /// `stack` the scopes open over it, innermost last.
    fn note(&self, stack: &[Scope], text: &str, named: &mut KindSet, keyworded: &mut KindSet) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        for &(kind, scope) in &self.named {
            if !stack.iter().any(|open| scope.is_prefix_of(*open)) {
                continue;
            }
            // The Rust grammar names a closure's binding `entity.name.function`
            // too: `let f = |x| x` would otherwise start a "function". A real
            // definition's name is enclosed by `meta.function.*`; a closure's
            // by the `meta.block.*` of the body it sits in. The nearest
            // enclosing `meta.*` scope tells them apart.
            if kind == Kind::Function
                && stack
                    .iter()
                    .rev()
                    .find(|open| open.build_string().starts_with("meta."))
                    .is_some_and(|nearest| self.meta_block.is_prefix_of(*nearest))
            {
                continue;
            }
            named.insert(kind);
        }
        if stack
            .iter()
            .any(|open| self.storage_type.is_prefix_of(*open))
        {
            for kind in Kind::ALL {
                if kind.keywords().contains(&text) {
                    keyworded.insert(kind);
                }
            }
        }
    }
}

/// Which definition kinds each line of `lines`, read from `path`, starts —
/// or `None` when no grammar claims the file, in which case no line starts
/// anything.
///
/// A whole-file pass, unlike colouring, which [`Highlighter`] does lazily
/// under a budget: a filter needs every line's answer at once, and the
/// answer for a source file is the kind of thing that fits in memory (one
/// byte per line). The cost is one parse of the file, on the order of ten
/// microseconds per line, paid once per load and only when a definition
/// filter exists — see `Document::evaluate`. Log files have no grammar and
/// never pay it.
///
/// Two signals, combined per file. The grammar's `entity.name.<kind>` scope
/// on a name is the primary one. A declaration keyword under `storage.type`
/// is the fallback, and it is applied **only for kinds the grammar never
/// named anywhere in this file**: C scopes `struct` in `struct point p;` as
/// `storage.type` too, and since the C grammar does name real struct
/// definitions, the fallback stays off for structs there. In a Swift file
/// nothing is ever named, so the fallback carries all four kinds.
#[must_use]
pub fn definitions(path: &Path, lines: &[String]) -> Option<Vec<KindSet>> {
    let syntax = detect(path, lines.first().map(String::as_str))?;
    let scopes = KindScopes::new();
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut named = vec![KindSet::EMPTY; lines.len()];
    let mut keyworded = vec![KindSet::EMPTY; lines.len()];

    for (row, line) in lines.iter().enumerate() {
        if line.len() > MAX_LINE_BYTES {
            continue;
        }
        let with_newline = format!("{line}\n");
        let Ok(ops) = state.parse_line(&with_newline, syntaxes()) else {
            continue;
        };
        let mut last = 0;
        let mut note = |stack: &ScopeStack, start: usize, end: usize| {
            let end = end.min(line.len());
            if start < end {
                scopes.note(
                    stack.as_slice(),
                    &line[start..end],
                    &mut named[row],
                    &mut keyworded[row],
                );
            }
        };
        for (pos, op) in &ops {
            note(&stack, last, *pos);
            if stack.apply(op).is_err() {
                break;
            }
            last = *pos;
        }
        note(&stack, last, line.len());
    }

    let seen_named: Vec<Kind> = Kind::ALL
        .into_iter()
        .filter(|&kind| named.iter().any(|set| set.contains(kind)))
        .collect();
    Some(
        named
            .into_iter()
            .zip(keyworded)
            .map(|(mut kinds, fallback)| {
                for kind in Kind::ALL {
                    if fallback.contains(kind) && !seen_named.contains(&kind) {
                        kinds.insert(kind);
                    }
                }
                kinds
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    // ---- definitions (#123) ------------------------------------------------

    /// The rows of `text` that start `kind`, by index.
    fn rows_of(kind: Kind, name: &str, text: &str) -> Vec<usize> {
        let lines = lines(text);
        let kinds = definitions(Path::new(name), &lines).expect("a bundled grammar");
        kinds
            .iter()
            .enumerate()
            .filter(|(_, set)| set.contains(kind))
            .map(|(row, _)| row)
            .collect()
    }

    const RUST: &str = "\
pub fn top() {}
struct Point { x: i32 }
enum Shape { Circle, Square }
impl Point {
    fn method(&self) -> i32 {
        let double = |v: i32| v * 2;
        double(self.x)
    }
}
";

    /// Functions, structs and enums by their `entity.name.*` scopes; a
    /// method inside an impl counts; a closure binding does not.
    #[test]
    fn rust_definitions_by_scope() {
        assert_eq!(rows_of(Kind::Function, "a.rs", RUST), vec![0, 4]);
        assert_eq!(rows_of(Kind::Struct, "a.rs", RUST), vec![1]);
        assert_eq!(rows_of(Kind::Enum, "a.rs", RUST), vec![2]);
        assert_eq!(rows_of(Kind::Class, "a.rs", RUST), Vec::<usize>::new());
    }

    const PYTHON: &str = "\
class Canvas:
    def draw(self):
        pass

async def main():
    pass
";

    #[test]
    fn python_definitions_by_scope() {
        assert_eq!(rows_of(Kind::Class, "a.py", PYTHON), vec![0]);
        assert_eq!(rows_of(Kind::Function, "a.py", PYTHON), vec![1, 4]);
        assert_eq!(rows_of(Kind::Struct, "a.py", PYTHON), Vec::<usize>::new());
    }

    const C: &str = "\
struct point { int x; };
int main(void) {
    struct point p;
    return 0;
}
";

    /// The C grammar names struct definitions, so the `storage.type`
    /// fallback stays off for structs and `struct point p;` is not one.
    #[test]
    fn c_struct_declarations_are_not_definitions() {
        assert_eq!(rows_of(Kind::Struct, "a.c", C), vec![0]);
        assert_eq!(rows_of(Kind::Function, "a.c", C), vec![1]);
    }

    const SWIFT: &str = "\
class Canvas {
    func draw() {}
    static func make() -> Canvas { Canvas() }
}
struct Point { var x: Int }
enum Shape { case circle }
func topLevel() {}
";

    /// bat's Swift grammar names nothing, so every kind comes from the
    /// keyword fallback — and nothing spurious comes with it.
    #[test]
    fn swift_definitions_by_keyword_fallback() {
        assert_eq!(rows_of(Kind::Class, "a.swift", SWIFT), vec![0]);
        assert_eq!(rows_of(Kind::Function, "a.swift", SWIFT), vec![1, 2, 6]);
        assert_eq!(rows_of(Kind::Struct, "a.swift", SWIFT), vec![4]);
        assert_eq!(rows_of(Kind::Enum, "a.swift", SWIFT), vec![5]);
    }

    // ---- the seven kinds #141 added -------------------------------------

    const RUST_MORE: &str = "\
type Alias = u32;
trait Area { fn area(&self) -> f64; }
impl Area for Point {
    fn area(&self) -> f64 { 0.0 }
}
mod inner {}
const LIMIT: u32 = 10;
macro_rules! say { () => {} }
";

    #[test]
    fn rust_types_traits_impls_modules_constants_and_macros() {
        assert_eq!(rows_of(Kind::Type, "b.rs", RUST_MORE), vec![0]);
        assert_eq!(rows_of(Kind::Trait, "b.rs", RUST_MORE), vec![1]);
        assert_eq!(rows_of(Kind::Impl, "b.rs", RUST_MORE), vec![2]);
        assert_eq!(rows_of(Kind::Module, "b.rs", RUST_MORE), vec![5]);
        assert_eq!(rows_of(Kind::Constant, "b.rs", RUST_MORE), vec![6]);
        assert_eq!(rows_of(Kind::Macro, "b.rs", RUST_MORE), vec![7]);
        // The one-line trait starts a function too; the impl's method is a
        // function and not an impl.
        assert_eq!(rows_of(Kind::Function, "b.rs", RUST_MORE), vec![1, 3]);
    }

    const GO: &str = "\
package main
type Point struct { X int }
type Shape interface { Area() float64 }
const Limit = 10
func main() {}
";

    /// Go names functions and types; structs and interfaces arrive under
    /// `type`, and the keyword fallback adds the struct and trait rows.
    /// `package` is not `storage.type` in the Go grammar, and a file's one
    /// package clause is not the kind of definition the row is for.
    #[test]
    fn go_types_cover_struct_and_interface_declarations() {
        assert_eq!(rows_of(Kind::Type, "a.go", GO), vec![1, 2]);
        assert_eq!(rows_of(Kind::Struct, "a.go", GO), vec![1]);
        assert_eq!(rows_of(Kind::Trait, "a.go", GO), vec![2]);
        assert!(rows_of(Kind::Module, "a.go", GO).is_empty());
        assert_eq!(rows_of(Kind::Constant, "a.go", GO), vec![3]);
        // The interface's method requirement is a function declaration too,
        // as a one-line Rust trait's is.
        assert_eq!(rows_of(Kind::Function, "a.go", GO), vec![2, 4]);
    }

    const SWIFT_MORE: &str = "\
protocol Drawable { func draw() }
extension Canvas { func clear() {} }
typealias Handler = (Int) -> Void
struct Point {}
";

    /// Swift names nothing, so each of these rides the keyword fallback.
    #[test]
    fn swift_protocols_extensions_and_typealiases_by_keyword() {
        assert_eq!(rows_of(Kind::Trait, "b.swift", SWIFT_MORE), vec![0]);
        assert_eq!(rows_of(Kind::Impl, "b.swift", SWIFT_MORE), vec![1]);
        assert_eq!(rows_of(Kind::Type, "b.swift", SWIFT_MORE), vec![2]);
        assert_eq!(rows_of(Kind::Struct, "b.swift", SWIFT_MORE), vec![3]);
    }

    const MARKDOWN: &str = "\
# Title
prose
## Second
- a list

### Third
";

    /// Headings are sections: a document outline for free. The blank line
    /// before the third heading matters: the grammar reads a heading right
    /// under a list item as the item's continuation, as `CommonMark`'s lazy
    /// continuation does.
    #[test]
    fn markdown_headings_are_sections() {
        assert_eq!(rows_of(Kind::Section, "README.md", MARKDOWN), vec![0, 2, 5]);
        assert!(rows_of(Kind::Function, "README.md", MARKDOWN).is_empty());
    }

    const C_MORE: &str = "\
typedef struct point point_t;
#define LIMIT 10
int main(void) { return 0; }
";

    #[test]
    fn c_typedefs_and_defines() {
        assert_eq!(rows_of(Kind::Type, "b.c", C_MORE), vec![0]);
        assert_eq!(rows_of(Kind::Constant, "b.c", C_MORE), vec![1]);
    }

    /// Eleven kinds fit the set, and the highest bit round-trips.
    #[test]
    fn kind_set_holds_every_kind() {
        let all = KindSet::of(&Kind::ALL);
        assert!(Kind::ALL.iter().all(|&kind| all.contains(kind)));
        assert!(!KindSet::of(&[Kind::Section]).contains(Kind::Function));
    }

    /// No grammar, no answer: a log file starts nothing.
    #[test]
    fn a_log_file_has_no_definitions() {
        assert!(definitions(Path::new("app.log"), &lines("fn main() {}\n")).is_none());
    }

    /// One line can start two kinds, and a `KindSet` says both.
    #[test]
    fn kind_set_holds_more_than_one_kind() {
        let mut set = KindSet::EMPTY;
        assert!(set.is_empty());
        set.insert(Kind::Function);
        set.insert(Kind::Enum);
        assert!(set.contains(Kind::Function));
        assert!(set.contains(Kind::Enum));
        assert!(!set.contains(Kind::Class));
    }

    #[test]
    fn kinds_display_as_their_plural() {
        assert_eq!(Kind::Function.to_string(), "functions");
        assert_eq!(Kind::Class.to_string(), "classes");
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

    /// The bundle *has* a `log` grammar — this pins that it is refused, and
    /// refused before the shebang fallback could claim the file either.
    #[test]
    fn a_log_file_is_left_plain_although_the_bundle_has_a_grammar_for_it() {
        assert!(
            syntaxes().find_syntax_by_extension("log").is_some(),
            "if bat dropped its log grammar, PLAIN_EXTENSIONS can drop \"log\""
        );
        let theme = Theme::builtin();
        for path in ["deploy.log", "/var/log/system.LOG", "app.log"] {
            let lines = lines("#!/bin/sh\n2024-03-22 07:10:38 error=1");
            assert!(
                Highlighter::for_file(theme, Path::new(path), &lines).is_none(),
                "{path} should be plain"
            );
        }
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
