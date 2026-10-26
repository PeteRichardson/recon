//! Filters decide how each line of the viewed file is presented.
//!
//! A filter set describes a *log format* rather than a document, so it outlives
//! any one file. Matching is by regular expression, the same as search, so
//! `^foo` anchors to the start of a line.

//! One file per job the module does: this one holds the palette and the
//! per-line model — [`ActiveFilters`], [`Predicate`], [`Verdict`] — `sets`
//! holds the set, solo, reset and adopt state machine, `matcher` the `Send`
//! snapshot a scan thread matches with, `reference` the `${set:name}`
//! references, and `advanced` the Oniguruma engine of an advanced filter.
//! Each adds its own `impl ActiveFilters` block; everything is re-exported
//! here, so callers name `crate::filter::X` whichever file `X` lives in.

mod advanced;
mod matcher;
mod reference;
mod require;
mod sets;

pub use advanced::{DEFAULT_STEP_LIMIT, Engine, LineRegex, Lookaround, OverLimit, PatternError};
pub use matcher::{Bits, MAX_PATTERNS, Matcher, Owner, ScanOff, ScanStamp};
pub use reference::{Broken, Expander};
pub use require::{RequireError, label};
pub use sets::{
    ALL_ITEM, DEFINITIONS_DESCRIPTION, DEFINITIONS_SET, EnableError, FilterSet, LoadedFilter,
    LoadedSet, Origin, SetSpec, is_builtin_name, unknown_item,
};

use crate::syntax::{Kind, KindSet};
use ratatui::style::{Color, Modifier, Style};
use regex::{Regex, RegexSet};
use sets::Solo;
use std::collections::HashMap;

/// Colours assigned to successive filters, so two filters are never
/// indistinguishable. Wraps once exhausted, and is replaced wholesale by
/// `[filters] palette` in `config.toml` — see [`crate::config::FiltersConfig`].
///
/// **Fixed 256-colour indices rather than `Color::Yellow` and friends.** The
/// named variants are ANSI slots 0–15, whose actual appearance the terminal's
/// theme decides; recon cannot promise contrast between two colours it does not
/// choose. That is what #62 hit — the palette's yellow and green were slots 3
/// and 2, which a great many themes render as near-neighbours, and reordering
/// them would only have delayed the collision to the fourth filter.
///
/// The six below are spaced around the hue wheel with no pair closer than 180
/// in RGB distance, which `every_default_palette_pair_is_visibly_distinct`
/// pins. Change one and that test is what tells you whether the replacement
/// still reads as its own colour. The same greyscale-ramp reasoning applies
/// here as to [`DIM_GREY`].
pub const DEFAULT_PALETTE: [Color; 13] = [
    Color::Indexed(220), // gold        #ffd700
    Color::Indexed(51),  // cyan        #00ffff
    Color::Indexed(46),  // pure green  #00ff00
    Color::Indexed(201), // magenta     #ff00ff
    Color::Indexed(105), // periwinkle  #8787ff
    Color::Indexed(196), // red         #ff0000
    // The tail (#231): the first six are the long-standing set and stay in
    // front; these follow in the order a greedy pass chose, each the
    // candidate furthest from everything before it. Snapped from the
    // issue's list to the nearest of the 256 so no truecolor is needed, and
    // every one at least 150 from black, from white and from `DIM_GREY`,
    // so no filter reads as background or dimmed text. Pairwise separation falls from 170 to 95 down the tail — less
    // distinct than the first six, far better than wrapping to a repeat.
    Color::Indexed(120), // light green  #87ff87
    Color::Indexed(203), // salmon       #ff5f5f
    Color::Indexed(42),  // sea green    #00d787
    Color::Indexed(33),  // azure        #0087ff
    Color::Indexed(161), // raspberry    #d7005f
    Color::Indexed(170), // orchid       #d75fd7
    Color::Indexed(202), // orange       #ff5f00
];

/// The palette for a light terminal background (#231), where `DEFAULT_PALETTE`'s
/// gold, cyan and green vanish into white.
///
/// Chosen the same way as the dark tail, against a white background and the
/// lighter `LIGHT_DIM_GREY`: each entry at least 150 from white, 120 from the
/// dim grey and 100 from black, so a filter is never mistaken for the page,
/// dimmed text or plain text; no greys, which on white *are* plain text.
/// Snapped to the 256-colour cube from the issue's list. The first six are
/// at least 120 apart, the rest at least 80.
pub const LIGHT_PALETTE: [Color; 13] = [
    Color::Indexed(33),  // azure        #0087ff
    Color::Indexed(124), // dark red     #af0000
    Color::Indexed(28),  // green        #008700
    Color::Indexed(55),  // indigo       #5f00af
    Color::Indexed(167), // coral        #d75f5f
    Color::Indexed(24),  // teal blue    #005f87
    Color::Indexed(136), // ochre        #af8700
    Color::Indexed(65),  // sage         #5f875f
    Color::Indexed(18),  // navy         #000087
    Color::Indexed(69),  // cornflower   #5f87ff
    Color::Indexed(35),  // jade         #00af5f
    Color::Indexed(126), // plum         #af0087
    Color::Indexed(64),  // olive        #5f8700
];

/// Whether the terminal's background is dark or light (#231): the one fact
/// that decides both which built-in palette filters draw from and which grey
/// dimmed text takes. recon cannot ask the terminal — see the README — so
/// this is `background` in `config.toml`, `--background`, or
/// `RECON_BACKGROUND`, and dark when unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    /// Light text on a dark background: the compiled-in default.
    #[default]
    Dark,
    /// Dark text on a light background.
    Light,
}

impl Background {
    /// The built-in filter palette for this background.
    #[must_use]
    pub fn palette(self) -> &'static [Color] {
        match self {
            Self::Dark => &DEFAULT_PALETTE,
            Self::Light => &LIGHT_PALETTE,
        }
    }

    /// How dimmed text is drawn on this background — see `DIM_STYLE`.
    #[must_use]
    pub fn dim_style(self) -> Style {
        match self {
            Self::Dark => DIM_STYLE,
            Self::Light => LIGHT_DIM_STYLE,
        }
    }
}

/// How lines matching no including filter are rendered.
///
/// `Modifier::DIM` alone is not enough: it emits the terminal's "faint"
/// attribute, which many terminals ignore outright, leaving dimmed lines
/// indistinguishable from matched ones. An explicit grey is what actually
/// produces the contrast; the modifier is kept for terminals that do honour it.
///
/// The colour is the 256-colour greyscale ramp rather than `DarkGray`, so the
/// shade does not depend on the terminal's theme. **Lower is darker** — adjust
/// `DIM_GREY` to taste: 244 is subtle, 240 clear, 236 heavy. The ramp runs from
/// 232 (near-black) to 255 (near-white).
const DIM_GREY: u8 = 240;

/// `DIM_GREY`'s counterpart on a light background (#231). 240 is `#585858`,
/// near-black on white; this is `#bcbcbc`, the same distance short of the
/// page. **Higher is lighter** here: 248 is clear, 250 subtle, 252 faint.
const LIGHT_DIM_GREY: u8 = 250;

/// The dark-background dim style. `Background::dim_style` is what the app
/// reads; this constant is the default it resolves to, and what a test
/// with no background in play compares against.
pub(crate) const DIM_STYLE: Style = Style::new()
    .fg(Color::Indexed(DIM_GREY))
    .add_modifier(Modifier::DIM);

pub(crate) const LIGHT_DIM_STYLE: Style = Style::new()
    .fg(Color::Indexed(LIGHT_DIM_GREY))
    .add_modifier(Modifier::DIM);

/// Whether a filter selects lines, removes them, or shows them without
/// counting them.
///
/// `Context` is the third kind (#119). A realistic set for a folder of logs
/// holds patterns that *discriminate* — part of a bug's signature — and
/// patterns that pick out metadata every log carries: the commit, the host.
/// The second kind is wanted in the view and useless for choosing files. A
/// `Context` filter is an `Include` for every purpose except one: it never
/// selects a file in the explorer.
///
/// A variant rather than a flag on `Include`: an `Exclude` already never
/// selects a file, so "selects?" is not orthogonal to sense but one more value
/// of it — and the compiler then finds every `match` that needs to know.
///
/// `Require` is the fourth kind (#547, ADR 0013). In the view it is a
/// `Context` filter. In the explorer it is a condition on the whole file: a
/// file is marked only when each enabled `Require` filter hits one of its
/// lines. It never adds a file; with no `Include` enabled, the `Require`
/// filters alone decide.
///
/// Sense is the user's choice, per filter, in this set. It is not a property
/// of the pattern: `^host: production-.*` is `Include` when the question is
/// "which production logs have errors" and `Context` when it is "which logs
/// have bug 57, and where did they run". Nothing derives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sense {
    Include,
    Context,
    Exclude,
    Require,
}

impl Sense {
    /// Every sense, in the order the filter editor's sense row shows and
    /// cycles them: require beside context, which it is in the view.
    pub const ALL: [Self; 4] = [Self::Include, Self::Context, Self::Require, Self::Exclude];

    /// What a set file writes after `sense =`, and the word the filter
    /// editor and the log use.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::Context => "context",
            Self::Require => "require",
            Self::Exclude => "exclude",
        }
    }
}

/// What the filter set decided about one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Matched an including filter; carries its index, for colouring.
    Included(usize),
    /// Matched a context filter and no including one; carries its index,
    /// for colouring.
    ///
    /// Shown and coloured exactly like `Included`, but not *interesting*: a
    /// context line is there to be read around a hit, not to be a hit. `n`
    /// steps over it, and the explorer's scan already leaves context
    /// filters out of the mask that marks a file as matching — this variant
    /// is what lets the view agree with it.
    Context(usize),
    /// Matched no including filter.
    Unmatched,
    /// Removed by an excluding filter.
    Excluded,
}

/// What a filter tests a line against (#123).
///
/// A filter used to *be* a regex. It is now a regex *or* a question about
/// what the line starts — a function, a class — answered by the grammar
/// pass in `syntax::definitions` rather than by the line's text. `Sense`,
/// `enabled` and the colour apply to both unchanged, which is the point:
/// "everything except functions" and "functions with context" cost nothing.
///
/// `Regex` predicates keep the `RegexSet` fast path. A `Definition` occupies
/// a slot in that set too — compiled as [`NEVER`], a pattern that matches
/// nothing — so that the set's indices stay the filters' indices and no
/// mapping between the two has to be kept in step.
#[derive(Debug, Clone)]
pub enum Predicate {
    Regex(Regex),
    /// The line starts a definition of this kind. Answered from the
    /// `KindSet` the document supplies alongside the line; on a file with no
    /// grammar every set is empty and the predicate never holds.
    Definition(Kind),
    /// A pattern with a reference to another filter's pattern (#473):
    /// `typed` as the user wrote it, and its expansion, or why there is
    /// none. See `reference`.
    Referring {
        typed: String,
        expanded: Result<Regex, Broken>,
    },
    /// An advanced filter (#474): `typed` as the user wrote it, read by
    /// Oniguruma, references and all, or why it cannot be. Its slot in the
    /// compiled set is `NEVER`; the verdict tests it on its own, with the
    /// step limit. See `advanced`.
    ///
    /// `also` is a require row's extra text (#548): the line must match
    /// `typed` and `also`, in either order. Two fields, so the pane edits
    /// the text and never parses a pattern. `None` for every other filter;
    /// empty means `typed` alone. See `joined`.
    Advanced {
        typed: String,
        also: Option<String>,
        compiled: Result<Lookaround, Broken>,
    },
}

/// What an advanced predicate compiles (#548): `typed`, or with a
/// require row's extra text `also`, a line that has both, in either order.
/// Lookaheads, not `typed.*also`: that fails when `typed` can match the
/// text too, and takes it before `also` can. An empty `also` is `typed`
/// alone.
///
/// Anchored at `^`, so the lookaheads run once, from the start of the
/// line. Unanchored, Oniguruma tries them again at every position, and
/// each `.*` reads to the end of the line: a line that fails costs the
/// square of its length, and the step limit is per position, so it does
/// not stop that. One 130,000-character line took 30 seconds.
pub(crate) fn joined<'a>(typed: &'a str, also: Option<&str>) -> std::borrow::Cow<'a, str> {
    match also {
        Some(also) if !also.is_empty() => format!("^(?=.*(?:{typed}))(?=.*(?:{also}))").into(),
        _ => typed.into(),
    }
}

/// A regex that matches nothing: an empty character class. Stands in for a
/// `Definition` predicate in the compiled set.
///
/// Empty by construction, not by contradiction. This used to be `\b\B` — a
/// word boundary and a non-boundary at one position — which is just as
/// impossible but is answered by running the `\b` machinery, and the regex
/// crate's fast DFA cannot run a Unicode `\b` over non-ASCII text: every
/// line with so much as an arrow in it fell back to a far slower engine,
/// eleven times over, for a question whose answer is fixed (#265). A class
/// that contains no character compiles to a dead state and costs nothing
/// per line.
const NEVER: &str = r"[^\s\S]";

impl Predicate {
    /// What the pane shows: the regex's source, or the kind's plural noun.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Regex(regex) => regex.as_str().to_string(),
            Self::Definition(kind) => kind.to_string(),
            Self::Referring { typed, .. } | Self::Advanced { typed, .. } => typed.clone(),
        }
    }

    /// The regex, when there is one: for a referring pattern, its
    /// expansion, and none while a reference is broken. The explorer's `Matcher` runs regexes
    /// over files it never parses, so it has nothing to ask a definition.
    #[must_use]
    pub fn as_regex(&self) -> Option<&Regex> {
        match self {
            Self::Regex(regex)
            | Self::Referring {
                expanded: Ok(regex),
                ..
            } => Some(regex),
            Self::Definition(_) | Self::Referring { .. } | Self::Advanced { .. } => None,
        }
    }

    /// The Oniguruma regex of an advanced filter, and none while a
    /// reference in it is broken.
    #[must_use]
    pub fn as_lookaround(&self) -> Option<&Lookaround> {
        match self {
            Self::Advanced {
                compiled: Ok(lookaround),
                ..
            } => Some(lookaround),
            _ => None,
        }
    }

    /// Which engine reads the pattern. A definition has none, and says
    /// `Standard`.
    #[must_use]
    pub fn engine(&self) -> Engine {
        match self {
            Self::Advanced { .. } => Engine::Advanced,
            Self::Regex(_) | Self::Definition(_) | Self::Referring { .. } => Engine::Standard,
        }
    }

    /// Whether the predicate is a test of the line's text that can hold:
    /// a regex, a referring pattern that expanded, or an advanced filter
    /// that compiled. What the explorer's scan can evaluate.
    #[must_use]
    pub fn reads_text(&self) -> bool {
        self.as_regex().is_some() || self.as_lookaround().is_some()
    }

    /// Whether the predicate is a definition kind: a [`NEVER`] slot in the
    /// compiled `RegexSet`, answered from the grammar pass instead.
    #[must_use]
    pub fn is_definition(&self) -> bool {
        matches!(self, Self::Definition(_))
    }

    /// The pattern this predicate contributes to the compiled `RegexSet`.
    fn source(&self) -> &str {
        self.as_regex().map_or(NEVER, Regex::as_str)
    }

    /// The scan cache's name for this predicate. Distinct from `source` only
    /// in that two definitions of different kinds must not share [`NEVER`].
    /// The leading control character cannot appear in a typed pattern.
    fn key(&self) -> String {
        match self {
            Self::Definition(kind) => format!("\u{1}{kind}"),
            // Not `NEVER`: the scan tests an advanced filter on its own,
            // so its bits change with its pattern.
            Self::Advanced {
                typed,
                also,
                compiled,
            } => match compiled {
                Ok(lookaround) => format!("\u{2}{}", lookaround.as_str()),
                Err(_) => format!("\u{3}{}", joined(typed, also.as_deref())),
            },
            Self::Regex(_) | Self::Referring { .. } => self.source().to_string(),
        }
    }

    /// Whether the predicate holds for a line with these `kinds`, by direct
    /// evaluation — the scanning path's counterpart to a `RegexSet` hit.
    /// Never for an advanced filter: `ActiveFilters::advanced_hits` answers
    /// for it, with the step limit, once a line.
    fn holds(&self, line: &str, kinds: KindSet) -> bool {
        match self {
            Self::Definition(kind) => kinds.contains(*kind),
            Self::Advanced { .. } => false,
            Self::Regex(_) | Self::Referring { .. } => {
                self.as_regex().is_some_and(|regex| regex.is_match(line))
            }
        }
    }
}

/// A filter's name for itself, which no insert, remove or reorder of the
/// list changes (#341). What a capture of the enabled flags is keyed on, so
/// a restore finds each filter wherever it is now.
///
/// Drawn from one process-wide counter, so no two filters ever share one —
/// not even a filter and the copy that a relisted set brings back, which is
/// a new filter as far as a capture is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FilterId(u64);

impl FilterId {
    fn fresh() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

#[derive(Debug, Clone)]
pub struct Filter {
    /// Private: only this module makes a filter, and each one gets a fresh
    /// id when it does.
    id: FilterId,
    pub predicate: Predicate,
    pub sense: Sense,
    pub enabled: bool,
    pub style: Style,
    /// The file's `name` for this filter, if it gave one (#128), or the one
    /// typed in the filter editor (#317). `None` for a typed filter until
    /// then; see `display_name`.
    pub name: Option<String>,
    /// Why the filter exists, for people only (#317). Never sent to a model.
    pub description: Option<String>,
    /// What the filter's lines look like, in plain language (#317): the
    /// text a model will be given to write the pattern from.
    pub prompt: Option<String>,
    /// Lines the pattern must match or must not match (#318): the filter
    /// editor's checks, kept as regression tests. Empty for most filters.
    pub examples: Vec<Example>,
    /// The `generated_hash` of the prompt and the pattern the model wrote
    /// from it (#321), as the filter editor or `filters.toml` gave it. Kept
    /// as it is when the prompt or the pattern changes: `is_generated`
    /// compares it with the two as they are now.
    pub generated_from: Option<String>,
    /// Index into `ActiveFilters::sets`. 0 is the scratch set.
    pub set: usize,
}

impl Filter {
    /// A new, enabled scratch filter with nothing but its pattern, sense
    /// and colour: what a prompt, the palette or `&` adds.
    fn scratch(predicate: Predicate, sense: Sense, style: Style) -> Self {
        Self {
            id: FilterId::fresh(),
            predicate,
            sense,
            enabled: true,
            style,
            name: None,
            description: None,
            prompt: None,
            examples: Vec::new(),
            generated_from: None,
            set: 0,
        }
    }

    /// Take the filter editor's fields (#317, #318, #321).
    fn apply(&mut self, details: Details) {
        self.name = details.name;
        self.description = details.description;
        self.prompt = details.prompt;
        self.examples = details.examples;
        self.generated_from = details.generated_from;
    }

    /// Whether the two say the same thing in a set file: the pattern, its
    /// engine, the sense and the editor's fields. The switch and the colour are not
    /// part of it: a switch is session state, and a colour the file did not
    /// give is the palette's (#348).
    fn same_text(&self, other: &Self) -> bool {
        self.predicate.display() == other.predicate.display()
            && self.predicate.engine() == other.predicate.engine()
            && self.predicate.also() == other.predicate.also()
            && self.sense == other.sense
            && self.name == other.name
            && self.description == other.description
            && self.prompt == other.prompt
            && self.examples == other.examples
            && self.generated_from == other.generated_from
    }

    /// What the pane calls this filter: the file's `name`, else the
    /// predicate's own display. Profiles refer to filters by this string,
    /// so it is also the filter's handle — `filtersets::parse` rejects two
    /// filters in one set that would answer to the same one.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| require::label(&self.predicate.display(), self.predicate.also()))
    }

    /// Whether the model wrote this filter's pattern from its prompt
    /// (#321): `generated_from` agrees with the prompt and the pattern as
    /// they are now. A change to either one, in recon or in the file, makes
    /// the filter an ordinary one, and nothing asks a model anything.
    #[must_use]
    pub fn is_generated(&self) -> bool {
        match (&self.generated_from, &self.prompt, self.predicate.typed()) {
            (Some(hash), Some(prompt), Some(pattern)) => *hash == generated_hash(prompt, pattern),
            _ => false,
        }
    }
}

/// What `generated_from` records for a pattern the model wrote from
/// `prompt` (#321): 16 hex digits of the 64-bit FNV-1a hash of the two.
///
/// FNV-1a, not `std`'s `DefaultHasher`, whose output can change from one
/// Rust release to the next: the hash is kept in `filters.toml` and must
/// give the same value on each platform and in each build. It is a check
/// that the two did not change, not a secret, so a short hash with no
/// crate is enough. Each string goes in after its length, so moving text
/// from the prompt to the pattern changes the hash.
#[must_use]
pub fn generated_hash(prompt: &str, pattern: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = OFFSET;
    for text in [prompt, pattern] {
        let length = u64::try_from(text.len()).unwrap_or(u64::MAX);
        for byte in length.to_le_bytes().iter().chain(text.as_bytes()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:016x}")
}

/// A line kept with a filter as a regression test (#318): the pattern must
/// match it, or must not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    /// The whole line, as the file had it.
    pub line: String,
    /// `true` for a line the pattern must match, `false` for one it must
    /// not match.
    pub must_match: bool,
}

impl Example {
    /// Whether `regex` does what the example says, with `limit` steps for
    /// an advanced pattern.
    #[must_use]
    pub fn passes(&self, regex: &LineRegex, limit: u32) -> bool {
        regex.is_match(&self.line, limit) == self.must_match
    }
}

/// What the filter editor edits beside the pattern (#317), and the checks
/// it keeps as examples (#318). `None` is a field left empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub name: Option<String>,
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub examples: Vec<Example>,
    /// The hash of the prompt and the pattern when the model wrote the
    /// pattern from the prompt (#321), else `None`.
    pub generated_from: Option<String>,
}

/// Every enabled flag in an [`ActiveFilters`], captured so it can be restored.
///
/// Opaque on purpose: it is a token to hand back to
/// [`ActiveFilters::apply_enabled_flags`], not a structure to read or build.
///
/// Keyed by [`FilterId`], not by position (#346). A filter added, deleted or
/// moved to another set since the capture moves every position after it,
/// and a capture by position then put each flag on the wrong filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledFlags {
    filters: HashMap<FilterId, bool>,
}

/// The colours successive filters are drawn from, in order.
///
/// A newtype rather than a bare `Vec<Color>` so that [`ActiveFilters`] can keep
/// deriving `Default`: the derive would give an empty vector, and an empty
/// palette is the one value `Palette::colour`'s modulo cannot survive.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Palette(Vec<Color>);

impl Default for Palette {
    fn default() -> Self {
        Self(DEFAULT_PALETTE.to_vec())
    }
}

impl Palette {
    /// Build a palette from configured colours, falling back to the built-in
    /// list when there are none.
    ///
    /// The fallback exists because [`Self::colour`]'s modulo divides by the
    /// length, so an empty list panics on the *first* filter added — after
    /// startup, with a file already open. `config::FiltersConfig` rejects
    /// `palette = []` earlier and with a message that names the file, so this
    /// arm should be unreachable in practice; it is here because a panic is
    /// the wrong way to find out otherwise.
    fn new(colours: Vec<Color>) -> Self {
        if colours.is_empty() {
            Self::default()
        } else {
            Self(colours)
        }
    }

    /// The colour for the filter at `position`, wrapping once the list runs
    /// out. A user who configures two colours gets them alternating, which is
    /// a legitimate thing to want and not an error.
    fn colour(&self, position: usize) -> Color {
        self.0[position % self.0.len()]
    }
}

#[derive(Debug)]
pub struct ActiveFilters {
    /// Every set, scratch first (index 0), then in pane order. Filters point
    /// into this by index; see `FilterSet`.
    sets: Vec<FilterSet>,
    /// The set being soloed, and every set's flag from before the first `s`,
    /// so a second `s` puts the world back (#132).
    solo: Option<Solo>,
    /// Where filter colours come from. Whole-list replacement, never a merge —
    /// see [`crate::config::FiltersConfig`] for why.
    palette: Palette,
    /// Which grey dimmed lines take (#231). The palette above is already the
    /// one the background chose — `Config::filter_palette` resolves that —
    /// so this carries only the half the palette cannot: `style_for`'s
    /// answer for an unmatched line.
    background: Background,
    filters: Vec<Filter>,
    /// Enabled flags captured by `disable_all_remembering`, awaiting a restore.
    ///
    /// Held separately from the filters so that a filter removed in the
    /// meantime simply drops out of the restore rather than resurrecting.
    remembered: Option<EnabledFlags>,
    /// Every pattern in one automaton: `filters` in order.
    ///
    /// `verdict` used to run one `Regex::is_match` per filter per line, so a
    /// filter change cost O(lines × filters) separate DFA walks over the same
    /// bytes (#86). `RegexSet` walks them once and reports which patterns
    /// matched, which is what this crate provides it for.
    ///
    /// `enabled` is deliberately **not** baked in. Toggling a filter is the
    /// frequent operation — `space`, `d` and `!` all drive it — and it stays a
    /// flag read at verdict time so it costs no recompile. Only the patterns
    /// themselves are here, so only the seven methods that change a pattern
    /// call `recompile`.
    ///
    /// `None` when `RegexSet::new` refused the set (its size limit is not the
    /// sum of the individual ones). `verdict` then falls back to the original
    /// per-filter scan: slower, never wrong.
    compiled: Option<RegexSet>,
    /// Moves every time `recompile` runs — that is, every time a pattern is
    /// added, removed or replaced — and never on a flag. What `refresh_scan`
    /// compares instead of building the pattern key on every event (#186).
    /// Drawn from `GENERATIONS`, so no two sets ever share one.
    generation: u64,
    /// Each set's filters as they were loaded — disabled, with their colours
    /// — aligned with `sets`; empty for the scratch set (#305).
    ///
    /// An unlisted set's filters are not in `filters` at all, so they are in
    /// no compiled set, no scan and no pattern limit: an unlisted set is
    /// known, and has no other effect. This is where they wait. Listing the
    /// set puts a fresh copy back, which is why a set listed again comes
    /// back as its file describes it, not as it was when it was unlisted.
    as_loaded: Vec<Vec<Filter>>,
    /// How many steps an advanced filter may take on one line (#474):
    /// `[filters] advanced_step_limit`. A line over it is no match.
    step_limit: u32,
    /// How many lines of the open file each advanced filter gave up on at
    /// the step limit, as the last `Document::evaluate` counted them. By
    /// id, so a filter added or removed since moves no count to another.
    over_limit: HashMap<FilterId, usize>,
}

/// The source of every `ActiveFilters::generation`. Process-wide rather than
/// per set so that a set built to replace another never repeats its number.
static GENERATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A generation no set has had yet.
fn next_generation() -> u64 {
    GENERATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Hand-written rather than derived: the scratch set must exist from the
/// start, so that every filter has a set to belong to.
impl Default for ActiveFilters {
    fn default() -> Self {
        Self::with_sets(None, &[])
    }
}

impl ActiveFilters {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A set with no file sets and a palette of its own. Test-only (#167):
    /// `App` and batch mode go through `with_sets` with the loaded sets.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_palette(palette: Vec<Color>) -> Self {
        Self::with_sets(Some(palette), &[])
    }

    /// The scratch set and nothing else — the state `with_sets` builds on.
    /// Private: every public constructor goes through `with_sets`, so the
    /// built-in set is present in every `ActiveFilters` there is.
    fn bare(palette: Option<Vec<Color>>) -> Self {
        Self {
            sets: vec![FilterSet::scratch()],
            solo: None,
            palette: palette.map_or_else(Palette::default, Palette::new),
            background: Background::default(),
            filters: Vec::new(),
            remembered: None,
            compiled: None,
            generation: 0,
            as_loaded: vec![Vec::new()],
            step_limit: DEFAULT_STEP_LIMIT,
            over_limit: HashMap::new(),
        }
    }

    /// Whether there are any filters at all, built-in ones included. See
    /// `row_count`, which counts only the user-authored ones and is what the
    /// pane sizes itself against.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.user_authored_count() == 0
    }

    /// How many filters there are, built-in ones included. See `row_count`,
    /// which counts only the user-authored ones.
    #[must_use]
    pub fn len(&self) -> usize {
        // User-authored: a built-in row is not something the user built. The
        // known list itself, built-ins included, is `filters()`.
        self.user_authored_count()
    }

    #[must_use]
    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    /// The colour the next filter added will take. Public for the filter
    /// editor (#312), which draws its matches in the colour `add` will give.
    #[must_use]
    pub fn next_style(&self) -> Style {
        Style::default().fg(self.palette.colour(self.known_user_authored_count()))
    }

    /// `user_authored_count`, plus the filters of every unlisted set (#305).
    /// What colours run over, so a colour never depends on which sets are
    /// listed: a set's filters keep the colours they had at startup when it
    /// is listed again, and a typed filter does not take one of theirs.
    fn known_user_authored_count(&self) -> usize {
        let unlisted: usize = self
            .sets
            .iter()
            .zip(&self.as_loaded)
            .filter(|(set, _)| !set.listed && set.origin != Origin::BuiltIn)
            .map(|(_, filters)| filters.len())
            .sum();
        self.user_authored_count() + unlisted
    }

    /// How many filters the user wrote — scratch and file — which is what
    /// the palette and the pane's numbering run over. A built-in filter is
    /// neither numbered nor coloured, so it does not move the next colour.
    fn user_authored_count(&self) -> usize {
        self.filters
            .iter()
            .filter(|filter| self.sets[filter.set].origin != Origin::BuiltIn)
            .count()
    }

    /// Whether the filter at `index` is one the user wrote rather than one
    /// recon ships. The pane numbers and colours only these.
    #[must_use]
    pub fn is_user_authored(&self, index: usize) -> bool {
        self.filters
            .get(index)
            .is_some_and(|filter| self.sets[filter.set].origin != Origin::BuiltIn)
    }

    /// Add an including filter, colouring it distinctly from its
    /// predecessors. A pattern that will not compile is rejected and the set
    /// left untouched.
    pub fn add(&mut self, pattern: &str) -> Result<(), PatternError> {
        self.add_as(pattern, Sense::Include, Engine::Standard)
    }

    /// Add a filter of `sense` whose pattern `engine` reads: the filter
    /// editor's Enter on a new filter (#474). An excluding filter carries
    /// no colour, as `add_excluding` says.
    pub fn add_as(
        &mut self,
        pattern: &str,
        sense: Sense,
        engine: Engine,
    ) -> Result<(), PatternError> {
        let predicate = Predicate::parse_as(pattern, engine)?;
        let style = if sense == Sense::Exclude {
            Style::default()
        } else {
            self.next_style()
        };
        self.insert_scratch(Filter::scratch(predicate, sense, style));
        Ok(())
    }

    /// Add an excluding filter: its matches are removed from view entirely,
    /// in both display modes.
    ///
    /// Excluding filters carry no colour, since a line they match is never
    /// rendered.
    pub fn add_excluding(&mut self, pattern: &str) -> Result<(), PatternError> {
        self.add_as(pattern, Sense::Exclude, Engine::Standard)
    }

    /// Add an including filter that selects lines starting a definition of
    /// `kind` (#123), coloured and numbered like any other filter. Test-only
    /// (#167): the built-in `definitions` set (#127) is how a user gets one,
    /// and this is the direct route the filter and document tests take.
    #[cfg(test)]
    pub(crate) fn add_definition(&mut self, kind: Kind) {
        let style = self.next_style();
        self.insert_scratch(Filter::scratch(
            Predicate::Definition(kind),
            Sense::Include,
            style,
        ));
    }

    /// Whether any filter is a definition predicate, and so needs the
    /// document to supply each line's kinds. What gates the whole-file
    /// grammar pass in `Document::evaluate`.
    #[must_use]
    pub fn needs_kinds(&self) -> bool {
        // Effective, not merely present: the built-in set means a definition
        // filter always exists, and a whole-file grammar pass for a row
        // nobody has turned on would be paid by every source file opened.
        self.filters
            .iter()
            .enumerate()
            .any(|(index, filter)| filter.predicate.is_definition() && self.effective(index))
    }

    /// Whether a line's verdict reads the compiled set: an effective regex
    /// filter of any sense. What gates the `RegexSet`
    /// pass in `verdict`, the counterpart of `needs_kinds` for the grammar
    /// pass.
    ///
    /// Not `matcher().is_some()`: that is `None` when nothing *selects*,
    /// and an excluding filter selects nothing yet still has to run over
    /// every line. Any sense counts here.
    #[must_use]
    pub fn needs_regex(&self) -> bool {
        self.filters
            .iter()
            .enumerate()
            .any(|(index, filter)| filter.predicate.as_regex().is_some() && self.effective(index))
    }

    /// Whether the filter at `index` currently takes effect: on, and in a
    /// set that is on. The one place the two flags meet; everything that
    /// decides a line, a file, or a dim reads this and never `enabled`
    /// alone. The flag-level operations — `!`, the peek, `toggle_enabled` —
    /// deliberately do not: they act on the filter's own flag and leave the
    /// set's to `set_enabled_set`.
    fn effective(&self, index: usize) -> bool {
        let filter = &self.filters[index];
        filter.enabled && self.sets[filter.set].enabled
    }

    /// Drop a pending `!` capture.
    ///
    /// A capture describes a set that no longer exists once the set changes.
    /// Keeping it would strand it — see `add`.
    fn forget_capture(&mut self) {
        self.remembered = None;
    }

    /// The include verdict once exclusion has had its say:
    /// which include or context filter, if any, claims a line whose hits
    /// `hit` reports: the first effective include filter that hit, else the
    /// first context one. Shared by the compiled and scanning paths so the
    /// two cannot disagree.
    ///
    /// There is no global AND (#474, ADR 0008): an advanced filter's
    /// lookahead is how one filter asks for two things at once.
    fn include_verdict(&self, hit: impl Fn(usize) -> bool) -> Verdict {
        let live = |senses: &'static [Sense]| {
            self.filters
                .iter()
                .enumerate()
                .filter(move |&(index, filter)| {
                    self.effective(index) && senses.contains(&filter.sense)
                })
        };
        // Include outranks context, whatever their order in the set: a
        // line an include filter selected is a hit, and must say so — `n`
        // stops on `Included`, not on `Context` — and its colour is then
        // the same one the explorer gives the file, whose owner is the
        // lowest *selecting* filter and never a context one.
        //
        // A require filter is context in the view: what it adds is a
        // condition on the file, which the explorer's scan applies (#547).
        live(&[Sense::Include])
            .find(|&(index, _)| hit(index))
            .map(|(index, _)| Verdict::Included(index))
            .or_else(|| {
                live(&[Sense::Context, Sense::Require])
                    .find(|&(index, _)| hit(index))
                    .map(|(index, _)| Verdict::Context(index))
            })
            .unwrap_or(Verdict::Unmatched)
    }

    /// Whether any enabled filter removes lines.
    #[must_use]
    pub fn any_excluding(&self) -> bool {
        self.filters
            .iter()
            .enumerate()
            .any(|(index, filter)| self.effective(index) && filter.sense == Sense::Exclude)
    }

    /// Enable or disable every filter at once, for the `!` toggle.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            filter.enabled = enabled;
        }
    }

    /// Remove the filter at `index`, reporting whether it existed.
    ///
    /// Indices are positional, so this renumbers every later filter. Any
    /// cached `Verdict::Included` is invalid afterwards — callers must
    /// re-evaluate rather than patch.
    pub fn remove(&mut self, index: usize) -> bool {
        // A built-in filter is recon's, not the user's: it can be switched
        // off, and its set collapsed, but not deleted (#127).
        if index >= self.filters.len() || !self.is_user_authored(index) {
            return false;
        }
        self.filters.remove(index);
        self.recompile();
        true
    }

    /// Replace one filter's pattern, keeping everything else about it —
    /// reporting whether it existed.
    ///
    /// The filter stays at `index`, which is the point: `verdict` returns the
    /// *first* match, so a position is a precedence, and `style_for` looks a
    /// colour up by the same number. Deleting and re-adding — the only way to
    /// change a pattern before this existed — put the replacement at the end
    /// and silently reordered the set.
    ///
    /// Compiles before it mutates, the same discipline `add` follows: a
    /// pattern that will not compile leaves the old one in place, so the
    /// prompt has something intact to stay open over.
    ///
    /// Deliberately does **not** `forget_capture`. An add drops a pending `!`
    /// capture because the set it describes is gone — see
    /// `adding_while_a_restore_is_pending_drops_the_capture`. An edit is the
    /// same filter with the same flag, so the capture still describes this
    /// set exactly and dropping it would strand a restore for nothing.
    ///
    /// Callers must re-evaluate: the verdicts cached against the old pattern
    /// are stale. Unlike `remove`, only *this* filter's verdicts can have
    /// changed — the numbering is untouched — but `Document::evaluate` is the
    /// only thing that recomputes them, so a full pass is what a caller owes.
    ///
    /// A filter with no `name` answers to its pattern, so a new pattern is
    /// a new name, and the profiles follow it as `set_details` says (#356).
    ///
    /// The pattern is read by the engine the filter already uses (#474).
    pub fn set_pattern(&mut self, index: usize, pattern: &str) -> Result<bool, PatternError> {
        let engine = self
            .filters
            .get(index)
            .map_or(Engine::Standard, |filter| filter.predicate.engine());
        let predicate = self.reparse(index, pattern, engine)?;
        let changed = self.change(index, |filter| {
            filter.predicate = predicate;
        });
        if changed {
            self.recompile();
        }
        Ok(changed)
    }

    /// `set_details` and `set_pattern` as one change, for the filter
    /// editor's Enter: the profiles go from the name the filter had to the
    /// name it has now in one step (#356). One after the other, the name
    /// between the two can be another filter's, and the second rename then
    /// takes that filter's member too. The caller checks the final name
    /// with `name_taken` first.
    ///
    /// `engine` reads the pattern (#474): the editor's key chooses it.
    pub fn edit(
        &mut self,
        index: usize,
        pattern: &str,
        engine: Engine,
        details: Details,
    ) -> Result<bool, PatternError> {
        let predicate = self.reparse(index, pattern, engine)?;
        let changed = self.change(index, |filter| {
            filter.predicate = predicate;
            filter.apply(details);
        });
        if changed {
            self.recompile();
        }
        Ok(changed)
    }

    /// The predicate for a new `pattern` of the filter at `index`, read by
    /// `engine`. A require row keeps its extra text while it stays advanced
    /// (#548): a new base is not a new text.
    fn reparse(
        &self,
        index: usize,
        pattern: &str,
        engine: Engine,
    ) -> Result<Predicate, PatternError> {
        match self.filters.get(index).and_then(|f| f.predicate.also()) {
            Some(also) if engine == Engine::Advanced => Predicate::parse_also(pattern, Some(also)),
            _ => Predicate::parse_as(pattern, engine),
        }
    }

    /// Whether a filter other than the one at `index` — or any filter of
    /// the scratch set, for a new filter (`None`) — answers to `name` in
    /// the set the filter is in. `parse` refuses two in one set (#128), so
    /// the filter editor refuses the name first (#317).
    #[must_use]
    pub fn name_taken(&self, index: Option<usize>, name: &str) -> bool {
        let set = index
            .and_then(|at| self.filters.get(at))
            .map_or(0, |filter| filter.set);
        self.filters.iter().enumerate().any(|(at, filter)| {
            Some(at) != index && filter.set == set && filter.display_name() == name
        })
    }

    /// The stored examples of the filter at `index` that `regex` fails
    /// (#318), in the file's order. A new pattern that fails one does not
    /// replace the old one: the examples are regression tests.
    #[must_use]
    pub fn failed_examples(&self, index: usize, regex: &LineRegex) -> Vec<&Example> {
        self.filters.get(index).map_or_else(Vec::new, |filter| {
            filter
                .examples
                .iter()
                .filter(|example| !example.passes(regex, self.step_limit))
                .collect()
        })
    }

    /// Give the filter at `index` the filter editor's name, description and
    /// prompt (#317), its examples (#318) and its generated state (#321),
    /// reporting whether it changed
    /// anything. A built-in filter is recon's and is left alone, as
    /// `set_pattern` leaves it.
    ///
    /// A profile names its filters by `display_name`, so a new name is
    /// written into every profile of the filter's set that named the old
    /// one: the set still opens the way it did. The caller checks the name
    /// with `name_taken` first.
    ///
    /// No `forget_capture`: nothing that is switched changes. A `recompile`
    /// only when a new name changes what a reference stands for (#473).
    pub fn set_details(&mut self, index: usize, details: Details) -> bool {
        let before = self.filters.get(index).map(Filter::display_name);
        let changed = self.change(index, |filter| filter.apply(details));
        if changed
            && before != self.filters.get(index).map(Filter::display_name)
            && self.expand_references()
        {
            self.recompile();
        }
        changed
    }

    /// Change the user-authored filter at `index` with `change`, and write
    /// its new `display_name` into every profile of its set that named the
    /// old one. `false`, and nothing changed, for a built-in filter or no
    /// filter.
    fn change(&mut self, index: usize, change: impl FnOnce(&mut Filter)) -> bool {
        if !self.is_user_authored(index) {
            return false;
        }
        let Some(filter) = self.filters.get_mut(index) else {
            return false;
        };
        let before = filter.display_name();
        change(filter);
        let after = filter.display_name();
        let set = filter.set;
        if before != after {
            let meta = &mut self.sets[set];
            for members in meta.profiles.values_mut() {
                for member in members.iter_mut().filter(|member| **member == before) {
                    member.clone_from(&after);
                }
            }
            // A reference names its target by name (#473), so the old name
            // is no longer a target and the new one is.
            for name in meta.filter_names.iter_mut().filter(|name| **name == before) {
                name.clone_from(&after);
            }
        }
        true
    }

    /// Give the filter at `index` `sense` (#317), from the filter editor,
    /// reporting whether it changed. A built-in filter is left alone, as
    /// `set_pattern` leaves it.
    ///
    /// An excluding filter carries no colour (`add_excluding`), so one that
    /// becomes including or context takes the next palette colour. One that
    /// becomes excluding keeps its colour, unused, for the way back.
    ///
    /// No `recompile` and no `forget_capture`, for the reasons
    /// `toggle_context` gives. Callers re-evaluate: every verdict can change.
    pub fn set_sense(&mut self, index: usize, sense: Sense) -> bool {
        if !self.is_user_authored(index) {
            return false;
        }
        let colour = self.next_style();
        let Some(filter) = self.filters.get_mut(index) else {
            return false;
        };
        if filter.sense == sense {
            return false;
        }
        filter.sense = sense;
        if sense != Sense::Exclude && filter.style == Style::default() {
            filter.style = colour;
        }
        true
    }

    /// Flip a filter between `Include` and `Context`, reporting whether it
    /// changed. An `Exclude` filter is left alone: it already selects nothing.
    /// So is a `Require` filter: one key press too many must not change
    /// which files the explorer marks (#547).
    ///
    /// No `recompile` — the pattern is untouched, so the compiled set is still
    /// right — and no `forget_capture`, for the same reason `set_pattern` gives:
    /// the set's shape is unchanged, so a pending `!` capture still describes it.
    pub fn toggle_context(&mut self, index: usize) -> bool {
        let Some(filter) = self.filters.get_mut(index) else {
            return false;
        };
        filter.sense = match filter.sense {
            Sense::Include => Sense::Context,
            Sense::Context => Sense::Include,
            Sense::Exclude | Sense::Require => return false,
        };
        true
    }

    /// Enable or disable one filter, reporting whether it existed.
    ///
    /// Test-only. Production reaches the same state through `toggle_enabled`
    /// (the `space` key) and `set_all_enabled` (`!`); this direct setter had no
    /// caller outside the tests that use it to arrange a set (#76).
    #[cfg(test)]
    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool {
        match self.filters.get_mut(index) {
            Some(filter) => {
                filter.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Flip one filter, returning its new state, or `None` if there is no
    /// such filter.
    ///
    /// Distinguishing the two matters: a caller cannot otherwise tell "turned
    /// off" from "that row is gone", and the pane's selection can lag a
    /// deletion by a frame.
    pub fn toggle_enabled(&mut self, index: usize) -> Option<bool> {
        let filter = self.filters.get_mut(index)?;
        filter.enabled = !filter.enabled;
        Some(filter.enabled)
    }

    /// Disable every filter, recording which were enabled.
    ///
    /// A second call before a restore is ignored *while everything is still
    /// off*: the flags at that point are the ones this method just cleared,
    /// so capturing them again would overwrite the real state with
    /// all-disabled and lose it for good.
    ///
    /// Guarded on that, not on the capture alone (#150). A filter switched
    /// back on by hand in between — `Enter` on its row, a profile, a solo —
    /// leaves the set with something enabled and a stale capture pending, and
    /// ignoring the call then made `!` inert until an add or remove dropped
    /// the capture. A set with something enabled is a real state, so it
    /// replaces the stale one and the next `!` restores it.
    pub fn disable_all_remembering(&mut self) {
        if self.remembered.is_some() && !self.any_enabled() {
            return;
        }
        self.remembered = Some(self.enabled_flags());
        self.set_all_enabled(false);
    }

    /// Put back exactly the state `disable_all_remembering` captured.
    ///
    /// Enabling everything instead would silently switch on filters the user
    /// had deliberately turned off.
    pub fn restore_remembered(&mut self) {
        if let Some(remembered) = self.remembered.take() {
            self.apply_enabled_flags(&remembered);
        }
    }

    #[must_use]
    pub fn has_remembered(&self) -> bool {
        self.remembered.is_some()
    }

    /// Capture every enabled flag for a caller to hold and hand back later.
    ///
    /// Deliberately *not* `disable_all_remembering`, even though the peek `App`
    /// uses this for (#48) also turns everything off. That method owns a single
    /// internal slot which `!` already uses; a peek writing to it would
    /// overwrite a capture `!` was still holding, and `!` would then restore
    /// all-disabled for the rest of the session. Two independent undo stacks
    /// need two independent captures, so this one lives with its caller.
    #[must_use]
    pub fn enabled_flags(&self) -> EnabledFlags {
        EnabledFlags {
            filters: self
                .filters
                .iter()
                .map(|filter| (filter.id, filter.enabled))
                .collect(),
        }
    }

    /// Put back what [`enabled_flags`](Self::enabled_flags) captured, on
    /// each filter wherever it is now.
    ///
    /// A filter deleted since the capture simply drops out of the restore
    /// rather than resurrecting, and one added since keeps whatever it has
    /// now. A snapshot is a convenience, not a transaction.
    pub fn apply_enabled_flags(&mut self, flags: &EnabledFlags) {
        for filter in &mut self.filters {
            if let Some(&was_enabled) = flags.filters.get(&filter.id) {
                filter.enabled = was_enabled;
            }
        }
    }

    /// How many filters take effect: enabled, in an enabled set. The count
    /// the status line gives while the filter pane is hidden (#300).
    #[must_use]
    pub fn effective_count(&self) -> usize {
        (0..self.filters.len())
            .filter(|&index| self.effective(index))
            .count()
    }

    #[must_use]
    pub fn any_enabled(&self) -> bool {
        self.filters.iter().any(|filter| filter.enabled)
    }

    /// Decide how `line` should be presented.
    ///
    /// A set with no enabled including filters leaves every line `Unmatched`,
    /// so an empty or fully disabled set renders an ordinary, undimmed file
    /// rather than a wholly dimmed one. The first matching filter wins, which
    /// is what makes the set's order meaningful.
    /// One pass over the line, not one per filter. See `compiled`.
    #[must_use]
    pub fn verdict(&self, line: &str, kinds: KindSet) -> Verdict {
        self.verdict_noting(line, kinds, |_| {})
    }

    /// `verdict`, telling `over_limit` the index of each advanced filter
    /// that took more steps on this line than the limit allows (#474).
    /// Such a filter does not match the line. Every effective advanced
    /// filter is tested on every line, so the count is exact.
    pub fn verdict_noting(
        &self,
        line: &str,
        kinds: KindSet,
        mut over_limit: impl FnMut(usize),
    ) -> Verdict {
        let advanced = self.advanced_hits(line, &mut over_limit);
        let advanced_hit = |index: usize| advanced.contains(&index);
        debug_assert!(
            self.compiled.as_ref().is_none_or(|set| self.in_step(set)),
            "the compiled set is out of step with the filters"
        );
        let Some(set) = self.compiled.as_ref().filter(|set| self.in_step(set)) else {
            return self.verdict_scanning(line, kinds, advanced_hit);
        };
        // Nothing in the compiled set can take effect, so it is not asked:
        // one pass over the line is still one regex pass too many when
        // there is no regex to run (#265). The scan is exact here — it
        // tests only effective filters, and every one of those is a
        // definition, answered from `kinds`.
        if !self.needs_regex() {
            return self.verdict_scanning(line, kinds, advanced_hit);
        }
        let matched = set.matches(line);
        let matched = |index: usize| matched.matched(index);
        // A definition's slot in the set never matches (see `NEVER`), so it
        // is answered from `kinds` instead; a regex's is the set's answer.
        let hit = |index: usize| match &self.filters[index].predicate {
            Predicate::Regex(_) | Predicate::Referring { .. } => matched(index),
            Predicate::Definition(kind) => kinds.contains(*kind),
            Predicate::Advanced { .. } => advanced_hit(index),
        };

        // Exclusion is applied after inclusion and overrides it, so a line an
        // including filter selected is still removed if an excluding filter
        // also matches it.
        if self.filters.iter().enumerate().any(|(index, filter)| {
            self.effective(index) && filter.sense == Sense::Exclude && hit(index)
        }) {
            return Verdict::Excluded;
        }

        self.include_verdict(hit)
    }

    /// The original per-filter scan, kept as the fallback for a set that would
    /// not compile into a `RegexSet`.
    ///
    /// Also what runs if `compiled` were ever out of step with `filters` — see
    /// `in_step`. That is a bug rather than a state to support, but indexing a
    /// short `SetMatches` panics, and taking down a full-screen TUI is a much
    /// worse way to report it than being slow.
    ///
    /// `advanced` answers for each advanced filter, already tested once by
    /// `advanced_hits`.
    fn verdict_scanning(
        &self,
        line: &str,
        kinds: KindSet,
        advanced: impl Fn(usize) -> bool,
    ) -> Verdict {
        let holds = |index: usize| match &self.filters[index].predicate {
            Predicate::Advanced { .. } => advanced(index),
            predicate => predicate.holds(line, kinds),
        };
        if self.filters.iter().enumerate().any(|(index, filter)| {
            self.effective(index) && filter.sense == Sense::Exclude && holds(index)
        }) {
            return Verdict::Excluded;
        }

        self.include_verdict(holds)
    }

    /// The scanning path alone, for the tests that compare it with the
    /// compiled one.
    #[cfg(test)]
    fn verdict_by_scanning(&self, line: &str, kinds: KindSet) -> Verdict {
        let advanced = self.advanced_hits(line, &mut |_| {});
        self.verdict_scanning(line, kinds, |index| advanced.contains(&index))
    }

    /// The index of each effective advanced filter that matches `line`,
    /// with `over_limit` told of each that passed the step limit. Empty,
    /// and no allocation, when none matches.
    fn advanced_hits(&self, line: &str, over_limit: &mut impl FnMut(usize)) -> Vec<usize> {
        let mut hits = Vec::new();
        for (index, filter) in self.filters.iter().enumerate() {
            let Some(lookaround) = filter.predicate.as_lookaround() else {
                continue;
            };
            if !self.effective(index) {
                continue;
            }
            match lookaround.test(line, self.step_limit) {
                Ok(true) => hits.push(index),
                Ok(false) => {}
                Err(OverLimit) => over_limit(index),
            }
        }
        hits
    }

    /// Set how many steps an advanced filter may take on one line (#474),
    /// from `[filters] advanced_step_limit`. Callers re-evaluate. It moves
    /// the generation, as a pattern change does: a cached scan bit of an
    /// advanced filter may mean something else under another limit.
    pub fn set_step_limit(&mut self, limit: u32) {
        self.step_limit = limit;
        self.generation = next_generation();
    }

    /// Keep the open file's counts of lines over the step limit, by filter
    /// index as `Document::over_limit` gives them, for the filter rows.
    pub fn note_over_limit(&mut self, counts: &[usize]) {
        self.over_limit = self
            .filters
            .iter()
            .zip(counts)
            .filter(|&(_, &count)| count > 0)
            .map(|(filter, &count)| (filter.id, count))
            .collect();
    }

    /// How many lines of the open file the filter at `index` gave up on at
    /// the step limit (#474). 0 for a filter that is not advanced.
    #[must_use]
    pub fn over_limit(&self, index: usize) -> usize {
        self.filters
            .get(index)
            .and_then(|filter| self.over_limit.get(&filter.id))
            .copied()
            .unwrap_or(0)
    }

    /// The step limit in force, for a test of a pattern outside the set:
    /// the filter editor's highlight and checks.
    #[must_use]
    pub fn step_limit(&self) -> u32 {
        self.step_limit
    }

    /// Whether the compiled set still describes this filter set.
    fn in_step(&self, set: &RegexSet) -> bool {
        set.len() == self.filters.len()
    }

    /// Rebuild the compiled set. Called by every method that adds, removes or
    /// replaces a pattern — and by none that only flips an `enabled` flag.
    ///
    /// A failure is rare and its consequences are invisible without this
    /// warning: `verdict` falls back to per-filter scanning, which is only
    /// slow, but `matcher` returns `None` and the explorer's marking
    /// switches off with nothing said (#187).
    fn recompile(&mut self) {
        self.expand_references();
        self.generation = next_generation();
        let patterns = self.filters.iter().map(|filter| filter.predicate.source());
        match RegexSet::new(patterns) {
            Ok(set) => self.compiled = Some(set),
            Err(err) => {
                log::warn!(
                    "cannot compile the filter patterns together: {err}; \
                     the explorer stops marking files until they change"
                );
                self.compiled = None;
            }
        }
    }

    /// A copy to judge lines on another thread (#351): what `q` under
    /// `--emit lines` reads the rest of a large file with.
    ///
    /// Not `Clone`: every set carries a generation no other set shares, and
    /// a derived clone would share it. This one draws its own.
    #[must_use]
    pub(crate) fn snapshot(&self) -> Self {
        Self {
            sets: self.sets.clone(),
            solo: self.solo.clone(),
            palette: self.palette.clone(),
            background: self.background,
            filters: self.filters.clone(),
            remembered: self.remembered.clone(),
            compiled: self.compiled.clone(),
            generation: next_generation(),
            as_loaded: self.as_loaded.clone(),
            step_limit: self.step_limit,
            over_limit: self.over_limit.clone(),
        }
    }

    /// Whether an including filter is enabled — anything marking lines.
    ///
    /// Drives dimming, hiding (including the `u` guard in `Document`) and
    /// `n`/`N` with no search set. Public because `Document` caches it at
    /// `evaluate` time. A search is not counted: it is a motion, not a filter
    /// (ADR 0001), and never changes which lines are visible.
    #[must_use]
    pub fn any_including(&self) -> bool {
        self.filters
            .iter()
            .enumerate()
            .any(|(index, filter)| self.effective(index) && filter.sense != Sense::Exclude)
    }

    /// The style to render a line with, or `None` to leave it alone.
    ///
    /// `Unmatched` dims only when an including filter is active.
    #[must_use]
    pub fn style_for(&self, verdict: Verdict) -> Option<Style> {
        match verdict {
            Verdict::Included(index) | Verdict::Context(index) => {
                self.filters.get(index).map(|f| f.style)
            }
            Verdict::Unmatched if self.any_including() => Some(self.dim_style()),
            Verdict::Unmatched | Verdict::Excluded => None,
        }
    }

    /// Tell the set which background it is drawn on (#231). Only the dim
    /// style follows: the palette was resolved before construction, by
    /// `Config::filter_palette`, because file filters take their colours in
    /// `with_sets` and a change afterwards would not reach them.
    pub fn set_background(&mut self, background: Background) {
        self.background = background;
    }

    /// The grey a dimmed line, row or header takes on this background.
    #[must_use]
    pub fn dim_style(&self) -> Style {
        self.background.dim_style()
    }
}

/// Builders shared by this module's tests and the pane's.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{LoadedFilter, LoadedSet, Predicate, Sense};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// A file set named `name`, one include filter per pattern, each named
    /// by its pattern.
    pub(crate) fn loaded(
        name: &str,
        priority: i32,
        autoload: bool,
        patterns: &[&str],
    ) -> LoadedSet {
        LoadedSet {
            name: name.to_string(),
            path: PathBuf::from("test/filters.toml"),
            priority,
            autoload,
            listed: true,
            description: None,
            profiles: BTreeMap::new(),
            filters: patterns
                .iter()
                .map(|pattern| LoadedFilter {
                    name: (*pattern).to_string(),
                    named: true,
                    predicate: Predicate::parse(pattern).expect("valid"),
                    sense: Sense::Include,
                    colour: None,
                    description: None,
                    prompt: None,
                    examples: Vec::new(),
                    generated_from: None,
                })
                .collect(),
            builtin: false,
        }
    }

    /// A `[sets.definitions]` override, as the loader would pass it through.
    pub(crate) fn builtin_override(priority: i32, autoload: bool) -> LoadedSet {
        LoadedSet {
            priority,
            autoload,
            path: PathBuf::from("test/filters.toml"),
            ..LoadedSet::builtin_default()
        }
    }

    /// `builtin_override` with profiles, each a list of kind names (#220).
    pub(crate) fn builtin_with_profiles(autoload: bool, profiles: &[(&str, &[&str])]) -> LoadedSet {
        let mut set = builtin_override(super::super::filtersets::DEFAULT_PRIORITY, autoload);
        set.profiles = profiles
            .iter()
            .map(|(profile, members)| {
                (
                    (*profile).to_string(),
                    members.iter().map(|m| (*m).to_string()).collect(),
                )
            })
            .collect();
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- generated filters (#321) ------------------------------------------

    /// The hash is kept in `filters.toml`, so it must not change from one
    /// build to the next. The value is FNV-1a 64 over each string after its
    /// length as 8 little-endian bytes.
    #[test]
    fn generated_hash_is_a_fixed_value() {
        assert_eq!(generated_hash("error lines", "ERROR"), "067ecc5f30502be0");
        assert_eq!(generated_hash("", "").len(), 16);
    }

    #[test]
    fn generated_hash_changes_with_the_prompt_the_pattern_and_the_split() {
        let hash = generated_hash("ab", "c");
        assert_ne!(hash, generated_hash("ab!", "c"));
        assert_ne!(hash, generated_hash("ab", "c!"));
        assert_ne!(hash, generated_hash("a", "bc"), "text moved to the pattern");
    }

    #[test]
    fn a_filter_is_generated_only_while_its_hash_agrees() {
        let mut set = set_with(&["ERROR"]);
        let details = |prompt: Option<&str>, hash: Option<String>| Details {
            prompt: prompt.map(Into::into),
            generated_from: hash,
            ..Details::default()
        };
        let hash = || Some(generated_hash("errors", "ERROR"));
        set.set_details(0, details(Some("errors"), hash()));
        assert!(set.filters()[0].is_generated());
        set.set_details(0, details(Some("errors!"), hash()));
        assert!(!set.filters()[0].is_generated(), "the prompt changed");
        set.set_details(0, details(None, hash()));
        assert!(!set.filters()[0].is_generated(), "no prompt");
        set.set_details(0, details(Some("errors"), None));
        assert!(!set.filters()[0].is_generated(), "no hash");
        set.set_details(0, details(Some("errors"), hash()));
        set.set_pattern(0, "ERROR|WARN").expect("valid");
        assert!(!set.filters()[0].is_generated(), "the pattern changed");
    }

    pub(super) fn set_with(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    // ---- predicates (#123) -------------------------------------------------

    /// A definition filter answers from the line's kinds, not its text.
    #[test]
    fn a_definition_filter_matches_by_kind_not_by_text() {
        let mut set = ActiveFilters::new();
        set.add_definition(Kind::Function);
        let functions = KindSet::of(&[Kind::Function]);
        assert_eq!(
            set.verdict("anything at all", functions),
            Verdict::Included(0)
        );
        assert_eq!(
            set.verdict("fn looks_like_one() {}", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert_eq!(set.filters()[0].predicate.display(), "functions");
        assert!(set.needs_kinds());
        assert!(!set_with(&["fn"]).needs_kinds());
    }

    /// The scanning fallback and the compiled path agree on definitions.
    #[test]
    fn definition_verdicts_agree_between_paths() {
        let mut set = set_with(&["foo"]);
        set.add_definition(Kind::Struct);
        let structs = KindSet::of(&[Kind::Struct]);
        for (line, k) in [("foo", KindSet::EMPTY), ("bar", structs), ("foo", structs)] {
            assert_eq!(
                set.verdict(line, k),
                set.verdict_by_scanning(line, k),
                "{line:?}"
            );
        }
        assert_eq!(set.verdict("bar", structs), Verdict::Included(1));
        assert_eq!(
            set.verdict("foo", structs),
            Verdict::Included(0),
            "first filter wins"
        );
    }

    /// Senses apply to a definition filter as to any other.
    #[test]
    fn definition_filters_take_every_sense() {
        let mut set = set_with(&["foo"]);
        set.add_definition(Kind::Enum);
        let enums = KindSet::of(&[Kind::Enum]);
        // Exclude: an enum line is removed even though `foo` matched it.
        set.toggle_context(1);
        assert_eq!(set.filters()[1].sense, Sense::Context);
        assert_eq!(set.verdict("plain", enums), Verdict::Context(1));
        assert_eq!(set.verdict_by_scanning("plain", enums), Verdict::Context(1));
        set.toggle_context(1);
        set.remove(1);
        set.add_excluding("never").expect("valid");
        set.filters[1].predicate = Predicate::Definition(Kind::Enum);
        set.recompile();
        assert_eq!(set.verdict("foo", enums), Verdict::Excluded);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Included(0));
    }

    /// The explorer cannot evaluate a definition, so its bit is in no mask:
    /// alone it yields no matcher, and beside a regex it neither selects nor
    /// excludes.
    #[test]
    fn the_matcher_ignores_definition_filters() {
        let mut set = ActiveFilters::new();
        set.add_definition(Kind::Function);
        assert!(set.matcher().is_none(), "nothing a scan can evaluate");
        set.add("foo").expect("valid");
        let m = set.matcher().expect("foo selects");
        assert!(m.selects(m.bits("foo")));
        assert_eq!(m.owner(m.bits("foo")), Some(1));
        assert!(
            !m.selects(m.bits("fn nothing")),
            "the definition slot never matches text"
        );
    }

    /// Two definitions of different kinds are different cache keys, and
    /// neither collides with a regex someone might type.
    #[test]
    fn definition_keys_are_distinct() {
        let mut set = ActiveFilters::new();
        set.add_definition(Kind::Function);
        set.add_definition(Kind::Struct);
        set.add("functions").expect("valid");
        let key = set.pattern_key();
        assert_eq!(
            key.len(),
            3 + Kind::ALL.len(),
            "three typed, then every built-in"
        );
        assert_ne!(key[0], key[1]);
        assert_ne!(key[0], key[2]);
    }

    /// `c` on a definition filter turns it into a regex filter, keeping its
    /// position, colour and sense — the same contract `set_pattern` has.
    #[test]
    fn set_pattern_turns_a_definition_into_a_regex() {
        let mut set = ActiveFilters::new();
        set.add_definition(Kind::Class);
        assert!(set.set_pattern(0, "class ").expect("valid"));
        assert!(set.filters()[0].predicate.as_regex().is_some());
        assert!(!set.needs_kinds());
    }

    // ---- the default palette -------------------------------------------

    /// The xterm 256-colour cube's five levels, in order. Indices 16..=231 are
    /// a 6×6×6 cube over these; 232..=255 are a separate greyscale ramp.
    const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    /// What an indexed colour actually renders as, so two of them can be
    /// compared. Only 16..=255 have fixed values — 0..=15 are the terminal's
    /// own ANSI slots and mean whatever the user's theme says they mean, which
    /// is the whole reason `DEFAULT_PALETTE` does not use them.
    fn indexed_rgb(index: u8) -> Option<(i32, i32, i32)> {
        match index {
            0..=15 => None,
            16..=231 => {
                let offset = usize::from(index - 16);
                Some((
                    i32::from(CUBE_LEVELS[offset / 36]),
                    i32::from(CUBE_LEVELS[(offset % 36) / 6]),
                    i32::from(CUBE_LEVELS[offset % 6]),
                ))
            }
            _ => {
                let level = i32::from(8 + 10 * (index - 232));
                Some((level, level, level))
            }
        }
    }

    fn rgb_of(colour: Color) -> Option<(i32, i32, i32)> {
        match colour {
            Color::Indexed(index) => indexed_rgb(index),
            Color::Rgb(r, g, b) => Some((i32::from(r), i32::from(g), i32::from(b))),
            _ => None,
        }
    }

    fn distance(a: (i32, i32, i32), b: (i32, i32, i32)) -> f64 {
        let (dr, dg, db) = (a.0 - b.0, a.1 - b.1, a.2 - b.2);
        f64::from(dr * dr + dg * dg + db * db).sqrt()
    }

    /// #62: the palette's first and third entries were `Color::Yellow` and
    /// `Color::Green`, which are ANSI slots 3 and 2 — the terminal decides what
    /// they look like. On a great many themes it decides they look nearly
    /// identical, and no amount of reordering fixes a colour recon does not
    /// choose. Fixed shades are the only way the contrast below can be a
    /// promise rather than a hope.
    #[test]
    fn the_default_palette_names_no_theme_dependent_colour() {
        for background in [Background::Dark, Background::Light] {
            for (position, colour) in background.palette().iter().enumerate() {
                assert!(
                    rgb_of(*colour).is_some(),
                    "{background:?} palette entry {position} is {colour:?}, whose \
                     appearance the terminal theme decides; use an indexed or RGB colour"
                );
            }
        }
    }

    /// What each built-in palette promises (#231): how far apart its first
    /// six are, how far apart every pair is, and how far every entry keeps
    /// from the three things a filter colour must never be mistaken for on
    /// that background — the page itself, dimmed text, and plain white
    /// text. The bars differ because the light list's source material is
    /// darker and closer together; both are what the greedy pass that
    /// ordered the lists was run with, so a hand edit that breaks one is
    /// caught here.
    struct Promise {
        background: Background,
        /// Between any two of the first six. Yellow-vs-green was the
        /// complaint (#62); this is what stops any pair from drifting back
        /// into it.
        first_six_apart: f64,
        /// Between any two entries at all.
        every_pair_apart: f64,
        /// From the page: black, or white.
        page: (i32, i32, i32),
        page_apart: f64,
        /// From the dim grey of unmatched lines.
        dim_apart: f64,
        /// From white: plain text on a dark page, the page itself on a
        /// light one.
        white_apart: f64,
    }

    const PROMISES: [Promise; 2] = [
        Promise {
            background: Background::Dark,
            first_six_apart: 150.0,
            every_pair_apart: 90.0,
            page: (0, 0, 0),
            page_apart: 150.0,
            dim_apart: 150.0,
            white_apart: 150.0,
        },
        Promise {
            background: Background::Light,
            first_six_apart: 120.0,
            every_pair_apart: 80.0,
            page: (255, 255, 255),
            page_apart: 150.0,
            dim_apart: 120.0,
            white_apart: 150.0,
        },
    ];

    fn fixed(colour: Color) -> (i32, i32, i32) {
        rgb_of(colour).expect("palette colours have fixed values")
    }

    #[test]
    fn every_palette_pair_is_visibly_distinct() {
        for promise in &PROMISES {
            let palette = promise.background.palette();
            for (i, first) in palette.iter().enumerate() {
                for (j, second) in palette.iter().enumerate().skip(i + 1) {
                    let (a, b) = (fixed(*first), fixed(*second));
                    let apart = distance(a, b);
                    let minimum = if j < 6 {
                        promise.first_six_apart
                    } else {
                        promise.every_pair_apart
                    };
                    assert!(
                        apart >= minimum,
                        "{:?} filters {} and {} are only {apart:.0} apart ({first:?} {a:?} \
                         vs {second:?} {b:?}); {minimum} is the minimum",
                        promise.background,
                        i + 1,
                        j + 1,
                    );
                }
            }
        }
    }

    /// A filter colour that reads as the page, as dimmed text or as plain
    /// white text is worse than an indistinct one: it says the wrong thing
    /// rather than nothing (#231).
    #[test]
    fn no_palette_entry_is_mistakable_for_page_dim_or_plain_text() {
        for promise in &PROMISES {
            let dim = fixed(promise.background.dim_style().fg.expect("dim has a colour"));
            let white = (255, 255, 255);
            for (position, colour) in promise.background.palette().iter().enumerate() {
                let c = fixed(*colour);
                for (name, other, minimum) in [
                    ("the page", promise.page, promise.page_apart),
                    ("dimmed text", dim, promise.dim_apart),
                    ("white", white, promise.white_apart),
                ] {
                    let apart = distance(c, other);
                    assert!(
                        apart >= minimum,
                        "{:?} filter {} ({colour:?} {c:?}) is only {apart:.0} from {name} \
                         {other:?}; {minimum} is the minimum",
                        promise.background,
                        position + 1,
                    );
                }
                if promise.background == Background::Light {
                    assert!(
                        !(c.0 == c.1 && c.1 == c.2),
                        "light filter {} is a grey, which on white is plain text",
                        position + 1
                    );
                }
            }
        }
    }

    /// The first six of the dark palette are the ones users have had since
    /// #62 (#231's decision): a saved set's colours must not change under it.
    #[test]
    fn the_dark_palette_keeps_its_original_six_in_front() {
        assert_eq!(
            &DEFAULT_PALETTE[..6],
            &[
                Color::Indexed(220),
                Color::Indexed(51),
                Color::Indexed(46),
                Color::Indexed(201),
                Color::Indexed(105),
                Color::Indexed(196),
            ]
        );
    }

    /// The dim grey follows the background (#231): 240 is near-black on
    /// white, so a light page dims with a light grey instead.
    #[test]
    fn dimming_follows_the_background() {
        let mut set = set_with(&["alpha"]);
        assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE));
        set.set_background(Background::Light);
        assert_eq!(set.style_for(Verdict::Unmatched), Some(LIGHT_DIM_STYLE));
        assert_eq!(set.dim_style(), LIGHT_DIM_STYLE);
        assert_ne!(DIM_STYLE.fg, LIGHT_DIM_STYLE.fg);
    }

    // ---- a configured palette ------------------------------------------

    /// #62's second half: the built-in shades are a default, not a decree.
    #[test]
    fn a_configured_palette_replaces_the_default() {
        let mut set = ActiveFilters::with_palette(vec![Color::Red, Color::Blue]);
        set.add("alpha").expect("valid pattern");
        set.add("beta").expect("valid pattern");

        assert_eq!(set.filters()[0].style.fg, Some(Color::Red));
        assert_eq!(set.filters()[1].style.fg, Some(Color::Blue));
    }

    /// A configured palette wraps exactly as the built-in one does, so a user
    /// who lists two colours gets them alternating rather than an error on the
    /// third filter.
    #[test]
    fn a_configured_palette_wraps_once_exhausted() {
        let mut set = ActiveFilters::with_palette(vec![Color::Red, Color::Blue]);
        for pattern in ["alpha", "beta", "gamma"] {
            set.add(pattern).expect("valid pattern");
        }

        assert_eq!(set.filters()[2].style.fg, Some(Color::Red));
    }

    /// An empty list would make `next_style`'s modulo divide by zero and panic
    /// on the first filter added. `with_palette` is the last place that can
    /// still refuse it — see `config::FiltersConfig`, which rejects it earlier
    /// and with a better message.
    #[test]
    fn an_empty_configured_palette_falls_back_to_the_default() {
        let mut set = ActiveFilters::with_palette(Vec::new());
        set.add("alpha").expect("valid pattern");

        assert_eq!(set.filters()[0].style.fg, Some(DEFAULT_PALETTE[0]));
    }

    /// With no filters at all, nothing is dimmed — a plain file reads normally.
    #[test]
    fn an_empty_set_leaves_every_line_unmatched() {
        let set = ActiveFilters::new();

        assert_eq!(set.verdict("anything", KindSet::EMPTY), Verdict::Unmatched);
        assert!(set.is_empty());
    }

    #[test]
    fn a_matching_line_is_included_with_its_filter_index() {
        let set = set_with(&["foo", "bar"]);

        assert_eq!(
            set.verdict("a bar line", KindSet::EMPTY),
            Verdict::Included(1)
        );
    }

    #[test]
    fn a_non_matching_line_is_unmatched() {
        let set = set_with(&["foo"]);

        assert_eq!(
            set.verdict("nothing here", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    /// Order in the set decides the colour, so the first match wins.
    #[test]
    fn the_first_matching_filter_wins() {
        let set = set_with(&["foo", "foo.*bar"]);

        assert_eq!(
            set.verdict("foo and bar", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    // ---- the third sense ------------------------------------------------

    /// A context filter shows its lines exactly as an include filter does —
    /// under its own verdict, so `n` can tell the two apart.
    #[test]
    fn a_context_filter_includes_its_lines() {
        let mut set = set_with(&["foo"]);
        assert!(set.toggle_context(0));

        assert_eq!(set.filters()[0].sense, Sense::Context);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Context(0));
        assert_eq!(
            set.style_for(Verdict::Context(0)),
            set.style_for(Verdict::Included(0)),
            "context keeps the filter's colour"
        );
        assert_eq!(
            set.style_for(Verdict::Unmatched),
            Some(DIM_STYLE),
            "context dims the rest"
        );
    }

    // ---- the fourth sense (#547) ------------------------------------------

    /// In the view, a require filter is a context filter: its lines show in
    /// its colour, and they are not hits, so `n` steps over them. An
    /// include filter still outranks it.
    #[test]
    fn a_require_filter_shows_its_lines_as_context() {
        let mut set = set_with(&["error"]);
        set.add_as("Host: Zeus", Sense::Require, Engine::Standard)
            .expect("valid pattern");

        for verdict in [
            set.verdict("Host: Zeus", KindSet::EMPTY),
            set.verdict_by_scanning("Host: Zeus", KindSet::EMPTY),
        ] {
            assert_eq!(verdict, Verdict::Context(1));
        }
        assert_eq!(
            set.verdict("error on Host: Zeus", KindSet::EMPTY),
            Verdict::Included(0)
        );
        assert_ne!(set.style_for(Verdict::Context(1)), None, "it has a colour");
    }

    /// `m` must not move a filter out of require, nor into it.
    #[test]
    fn toggle_context_leaves_a_require_filter_alone() {
        let mut set = ActiveFilters::new();
        set.add_as("Host", Sense::Require, Engine::Standard)
            .expect("valid pattern");

        assert!(!set.toggle_context(0));
        assert_eq!(set.filters()[0].sense, Sense::Require);
    }

    /// A require filter alone still dims the lines it does not show.
    #[test]
    fn a_require_filter_alone_dims_the_rest() {
        let mut set = ActiveFilters::new();
        set.add_as("Host", Sense::Require, Engine::Standard)
            .expect("valid pattern");

        assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE));
    }

    #[test]
    fn toggle_context_round_trips_without_touching_the_pattern() {
        let mut set = set_with(&["foo", "bar"]);
        let before = set.filters()[1].style;

        assert!(set.toggle_context(1));
        assert!(set.toggle_context(1));

        assert_eq!(set.filters()[1].sense, Sense::Include);
        assert_eq!(set.filters()[1].predicate.display(), "bar");
        assert_eq!(set.filters()[1].style, before);
        assert_eq!(set.verdict("bar", KindSet::EMPTY), Verdict::Included(1));
    }

    /// An exclude filter is never context, and an index off the end is not a filter.
    #[test]
    fn toggle_context_leaves_excludes_and_missing_indices_alone() {
        let mut set = set_with(&["foo"]);
        set.add_excluding("noise").expect("valid pattern");

        assert!(!set.toggle_context(1));
        assert_eq!(set.filters()[1].sense, Sense::Exclude);
        assert!(!set.toggle_context(99));
    }

    // ---- the compiled set stays in step --------------------------------
    //
    // `verdict` matches against a `RegexSet` compiled once per set change
    // rather than running one `Regex` per filter per line (#86). That cache is
    // the whole risk of the change: a mutator that forgets to rebuild it does
    // not fail loudly, it returns confidently wrong verdicts. One test per
    // pattern-changing method, each asserting through `verdict` — the only
    // thing that reads the cache — rather than at the cache itself.

    #[test]
    fn adding_a_filter_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);
        assert_eq!(set.verdict("bar", KindSet::EMPTY), Verdict::Unmatched);

        set.add("bar").expect("valid pattern");

        assert_eq!(set.verdict("bar", KindSet::EMPTY), Verdict::Included(1));
    }

    #[test]
    fn adding_an_excluding_filter_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Included(0));

        set.add_excluding("foo").expect("valid pattern");

        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Excluded);
    }

    #[test]
    fn removing_a_filter_is_visible_to_verdict() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.remove(0));

        // Not merely "no longer matches foo": everything renumbers, so a
        // stale set would answer `Included(1)` for a line it should call
        // `Included(0)`.
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);
        assert_eq!(set.verdict("bar", KindSet::EMPTY), Verdict::Included(0));
    }

    #[test]
    fn editing_a_pattern_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);

        assert!(set.set_pattern(0, "bar").expect("valid pattern"));

        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);
        assert_eq!(set.verdict("bar", KindSet::EMPTY), Verdict::Included(0));
    }

    #[test]
    fn a_rejected_pattern_leaves_the_compiled_set_alone() {
        let mut set = set_with(&["foo"]);

        assert!(set.set_pattern(0, "[").is_err());

        assert_eq!(
            set.verdict("foo", KindSet::EMPTY),
            Verdict::Included(0),
            "a pattern that would not compile disturbed the set it was rejected from"
        );
    }

    /// Toggling `enabled` must *not* need a recompile — it is the frequent
    /// operation, and `space`, `d` and `!` all drive it. Pinned so a future
    /// change cannot quietly move the enabled flag into the compiled set.
    #[test]
    fn toggling_enabled_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);

        assert!(set.set_enabled(0, false));
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);

        assert!(set.set_enabled(0, true));
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Included(0));
    }

    #[test]
    fn patterns_are_regular_expressions() {
        let set = set_with(&[r"^\d+ms$"]);

        assert_eq!(set.verdict("250ms", KindSet::EMPTY), Verdict::Included(0));
        assert_eq!(
            set.verdict("took 250ms", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    #[test]
    fn an_invalid_pattern_is_reported() {
        let mut set = ActiveFilters::new();

        assert!(set.add("[").is_err());
        assert!(set.is_empty(), "a rejected pattern must not be added");
    }

    #[test]
    fn a_disabled_filter_does_not_match() {
        let mut set = set_with(&["foo"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);
        assert!(!set.any_enabled());
    }

    /// `!` disables every filter, then a second press re-enables them all —
    /// not a restore of whatever per-filter state existed before, since
    /// nothing here can disable a filter individually yet.
    #[test]
    fn disabling_and_restoring_round_trips() {
        let mut set = set_with(&["foo"]);
        assert!(set.any_enabled());

        set.set_all_enabled(false);
        set.set_all_enabled(true);

        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Included(0));
    }

    /// A set whose filters are all disabled behaves like an empty one: an
    /// undimmed file, not a fully dimmed one.
    #[test]
    fn a_fully_disabled_set_leaves_lines_unmatched() {
        let mut set = set_with(&["foo"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("bar", KindSet::EMPTY), Verdict::Unmatched);
    }

    #[test]
    fn successive_filters_get_distinct_colours() {
        let mut set = ActiveFilters::new();
        set.add("a").expect("valid");
        set.add("b").expect("valid");

        assert_ne!(
            set.filters()[0].style,
            set.filters()[1].style,
            "two filters would be indistinguishable"
        );
    }

    fn set_excluding(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add_excluding(pattern).expect("valid pattern");
        }
        set
    }

    #[test]
    fn an_excluding_filter_excludes_its_matches() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(
            set.verdict("a heartbeat line", KindSet::EMPTY),
            Verdict::Excluded
        );
    }

    /// Excluding filters run after including ones, so exclusion wins even on a
    /// line an including filter selected.
    #[test]
    fn exclusion_beats_inclusion_on_the_same_line() {
        let mut set = set_with(&["foo"]);
        set.add_excluding("noisy").expect("valid pattern");

        assert_eq!(
            set.verdict("foo but noisy", KindSet::EMPTY),
            Verdict::Excluded
        );
        assert_eq!(
            set.verdict("foo alone", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    /// With only excluding filters, unmatched lines stay ordinary — there is
    /// nothing to dim against.
    #[test]
    fn excluding_filters_alone_do_not_dim() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(
            set.verdict("something else", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    #[test]
    fn a_disabled_excluding_filter_excludes_nothing() {
        let mut set = set_excluding(&["heartbeat"]);
        set.set_all_enabled(false);

        assert_eq!(
            set.verdict("a heartbeat line", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    #[test]
    fn an_invalid_excluding_pattern_is_reported() {
        let mut set = ActiveFilters::new();

        assert!(set.add_excluding("[").is_err());
        assert!(set.is_empty(), "a rejected pattern must not be added");
    }

    #[test]
    fn any_excluding_reports_whether_one_is_enabled() {
        let mut set = set_with(&["foo"]);
        assert!(!set.any_excluding());

        set.add_excluding("bar").expect("valid pattern");
        assert!(set.any_excluding());

        set.set_all_enabled(false);
        assert!(!set.any_excluding(), "a disabled filter does not count");
    }

    /// Dimming must set a foreground colour, not just the DIM attribute: many
    /// terminals ignore the attribute entirely, and on those a "dimmed" line
    /// would be indistinguishable from a matched one.
    #[test]
    fn dimming_sets_a_colour_rather_than_only_an_attribute() {
        let set = set_with(&["foo"]);

        let style = set.style_for(Verdict::Unmatched).expect("unmatched dims");

        assert!(
            style.fg.is_some(),
            "dimming relies on the DIM attribute alone, which many terminals ignore"
        );
        assert!(style.add_modifier.contains(Modifier::DIM));
    }

    /// An excluded line is never rendered, so it has no style.
    #[test]
    fn an_excluded_line_has_no_style() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.style_for(Verdict::Excluded), None);
    }

    #[test]
    fn removing_a_filter_drops_it() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.remove(0));

        assert_eq!(set.len(), 1);
        assert_eq!(
            set.verdict("bar line", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    /// Indices are positional, so removing a filter renumbers the ones after
    /// it. Any verdict cached against the old numbering is now wrong, which is
    /// why callers must re-evaluate rather than patch.
    #[test]
    fn removing_a_filter_renumbers_the_rest() {
        let mut set = set_with(&["foo", "bar"]);
        assert_eq!(
            set.verdict("bar line", KindSet::EMPTY),
            Verdict::Included(1)
        );

        set.remove(0);

        assert_eq!(
            set.verdict("bar line", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    #[test]
    fn removing_out_of_range_reports_failure_and_changes_nothing() {
        let mut set = set_with(&["foo"]);

        assert!(!set.remove(5));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn a_single_filter_can_be_disabled() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.set_enabled(0, false));

        assert_eq!(set.verdict("foo line", KindSet::EMPTY), Verdict::Unmatched);
        assert_eq!(
            set.verdict("bar line", KindSet::EMPTY),
            Verdict::Included(1)
        );
    }

    #[test]
    fn toggle_flips_one_filter_and_reports_its_new_state() {
        let mut set = set_with(&["foo"]);

        assert_eq!(
            set.toggle_enabled(0),
            Some(false),
            "was enabled, so is now disabled"
        );
        assert_eq!(set.toggle_enabled(0), Some(true), "and back on");
    }

    /// `toggle_enabled` must distinguish "turned off" from "no such filter" —
    /// both would otherwise report `false`, and the pane's selection can lag
    /// a deletion by a frame.
    #[test]
    fn toggling_a_missing_index_reports_none_and_changes_nothing() {
        let mut set = set_with(&["foo"]);

        assert_eq!(set.toggle_enabled(99), None);
        assert!(set.filters()[0].enabled, "nothing should have changed");
    }

    /// `!` must restore what was enabled before, not enable everything —
    /// otherwise it silently switches on filters the user turned off.
    #[test]
    fn disabling_all_remembers_the_previous_state() {
        let mut set = set_with(&["foo", "bar", "baz"]);
        set.set_enabled(1, false);

        set.disable_all_remembering();
        assert!(!set.any_enabled());

        set.restore_remembered();

        assert!(set.filters()[0].enabled);
        assert!(
            !set.filters()[1].enabled,
            "a filter the user had off came back on"
        );
        assert!(set.filters()[2].enabled);
    }

    #[test]
    fn has_remembered_reports_whether_a_restore_is_pending() {
        let mut set = set_with(&["foo"]);
        assert!(!set.has_remembered());

        set.disable_all_remembering();
        assert!(set.has_remembered());

        set.restore_remembered();
        assert!(!set.has_remembered());
    }

    /// Removing a filter while a restore is pending must not resurrect it or
    /// misapply the remembered flags to the wrong filters.
    #[test]
    fn removing_while_disabled_does_not_corrupt_the_restore() {
        let mut set = set_with(&["foo", "bar"]);
        set.disable_all_remembering();

        set.remove(0);
        set.restore_remembered();

        assert_eq!(set.len(), 1);
        assert!(set.filters()[0].enabled);
    }

    /// A second `disable_all_remembering` before a restore must not overwrite
    /// the capture: by then every filter reads disabled (this method just
    /// disabled them), so capturing again would replace the real prior state
    /// with all-false and lose it for good — the exact bug this task exists
    /// to prevent, reached from the other direction.
    #[test]
    fn disabling_all_twice_does_not_overwrite_the_capture() {
        let mut set = set_with(&["foo", "bar"]);
        set.set_enabled(1, false);

        set.disable_all_remembering();
        set.disable_all_remembering();

        set.restore_remembered();

        assert!(set.filters()[0].enabled);
        assert!(
            !set.filters()[1].enabled,
            "a filter the user had off came back on"
        );
    }

    #[test]
    fn removing_down_to_an_empty_set_leaves_it_empty() {
        let mut set = set_with(&["foo"]);

        assert!(set.remove(0));

        assert!(set.is_empty());
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);
    }

    #[test]
    fn removing_from_an_already_empty_set_reports_failure() {
        let mut set = ActiveFilters::new();

        assert!(!set.remove(0));
        assert!(set.is_empty());
    }

    /// With nothing captured, a restore is a no-op — the `else { return; }`
    /// path in `restore_remembered` has no other coverage, and a future
    /// refactor to `.unwrap()` should fail this test rather than panic
    /// unnoticed in production.
    #[test]
    fn restoring_with_nothing_captured_does_nothing() {
        let mut set = set_with(&["foo"]);
        set.set_enabled(0, false);

        set.restore_remembered();

        assert!(
            !set.filters()[0].enabled,
            "nothing was captured, so nothing should change"
        );
        assert!(!set.has_remembered());
    }

    /// Adding a filter while a restore is pending describes a set that no
    /// longer exists, so `add` drops the capture rather than let it strand:
    /// a later `restore_remembered` is a no-op, and every filter — the ones
    /// captured and the one just added — is left exactly as it stood right
    /// after the add.
    #[test]
    fn adding_while_a_restore_is_pending_drops_the_capture() {
        let mut set = set_with(&["foo"]);
        set.disable_all_remembering();

        set.add("bar").expect("valid pattern");
        assert!(
            set.filters()[1].enabled,
            "new filters are always added enabled"
        );
        assert!(
            !set.has_remembered(),
            "adding a filter should drop the now-stale capture"
        );

        set.restore_remembered();

        assert!(
            !set.filters()[0].enabled,
            "nothing was captured any more, so restore is a no-op"
        );
        assert!(set.filters()[1].enabled, "still enabled, exactly as added");
    }

    /// A filter switched back on by hand while a capture is pending — `Enter`
    /// on its row, a profile, a solo — leaves something enabled, so the next
    /// `!` is asked to disable everything again. It used to refuse because a
    /// capture already existed, and kept refusing until an add or remove
    /// dropped it (#150). The fear that early return guarded against was
    /// capturing an all-disabled set; a set with something enabled is a
    /// state worth capturing, so it replaces the stale one.
    #[test]
    fn toggling_a_filter_during_bang_does_not_leave_bang_inert() {
        let mut set = set_with(&["foo", "bar"]);
        set.set_enabled(1, false);
        set.disable_all_remembering();
        assert!(!set.any_enabled(), "sanity: the first ! disabled both");

        set.toggle_enabled(0);
        assert!(set.any_enabled(), "sanity: toggled back on by hand");

        set.disable_all_remembering();
        assert!(
            !set.any_enabled(),
            "! went inert: the hand-enabled filter stayed on"
        );

        set.restore_remembered();
        // The scratch pair only: `filters()` also holds every loaded set.
        let flags: Vec<bool> = set.filters()[..2].iter().map(|f| f.enabled).collect();
        assert_eq!(
            flags,
            vec![true, false],
            "restore should bring back the state the second ! captured"
        );
    }

    /// The early return still has its original job: `!` while everything is
    /// already off must not overwrite a real capture with all-disabled.
    #[test]
    fn a_second_capture_while_everything_is_off_keeps_the_first() {
        let mut set = set_with(&["foo", "bar"]);
        set.set_enabled(1, false);
        set.disable_all_remembering();

        set.disable_all_remembering();

        set.restore_remembered();
        let flags: Vec<bool> = set.filters()[..2].iter().map(|f| f.enabled).collect();
        assert_eq!(flags, vec![true, false], "the real capture was overwritten");
    }

    /// Scratch `x` on, then set `web` with `a` off and `b` on: the state
    /// #346's repro starts from.
    fn scratch_and_web() -> ActiveFilters {
        let web = test_support::loaded("web", 10, true, &["a", "b"]);
        let mut set = ActiveFilters::with_sets(None, &[web]);
        set.add("x").expect("valid pattern");
        let b = position(&set, "b");
        set.set_enabled(b, true);
        set
    }

    fn position(set: &ActiveFilters, name: &str) -> usize {
        set.filters()
            .iter()
            .position(|filter| filter.display_name() == name)
            .expect("a filter of that name")
    }

    fn flag(set: &ActiveFilters, name: &str) -> bool {
        set.filters()[position(set, name)].enabled
    }

    /// A typed filter goes in before the file sets' filters, so every one of
    /// them moves along one place. The capture is by id (#346), and each
    /// flag comes back on its own filter; the new filter keeps its own.
    #[test]
    fn captured_flags_follow_their_filters_past_an_insert() {
        let mut set = scratch_and_web();
        let flags = set.enabled_flags();
        set.set_all_enabled(false);

        set.add("foo").expect("valid pattern");
        set.apply_enabled_flags(&flags);

        assert!(flag(&set, "x"));
        assert!(
            flag(&set, "foo"),
            "the new filter lost the flag it was added with"
        );
        assert!(!flag(&set, "a"), "a took x's flag");
        assert!(flag(&set, "b"), "b took a's flag");
    }

    /// `S` moves the scratch filters into a new set and so reorders the
    /// list (#346). The adopted filters are the same filters, and a capture
    /// taken before still finds them.
    #[test]
    fn captured_flags_follow_their_filters_into_an_adopted_set() {
        let mut set = scratch_and_web();
        let flags = set.enabled_flags();
        set.set_all_enabled(false);

        assert!(set.adopt_scratch_as("zeta", "test/filters.toml".into()));
        set.apply_enabled_flags(&flags);

        assert!(flag(&set, "x"));
        assert!(!flag(&set, "a"));
        assert!(flag(&set, "b"));
    }

    /// A relisted set's filters are copies from the file, not the filters
    /// that were unlisted: a capture from before the unlist leaves them
    /// as the file describes them.
    #[test]
    fn a_capture_does_not_reach_a_relisted_sets_filters() {
        let mut set = scratch_and_web();
        let web = set
            .sets()
            .iter()
            .position(|s| s.name == "web")
            .expect("web");
        let flags = set.enabled_flags();

        set.set_listed(web, false);
        set.set_listed(web, true);
        set.apply_enabled_flags(&flags);

        assert!(!flag(&set, "b"), "the capture reached a copy from the file");
    }

    /// The whole point of editing in place rather than deleting and retyping:
    /// the filter keeps its position, so it keeps its colour and its
    /// precedence in `verdict`. Retyping put the replacement at the end and
    /// silently reordered the set.
    #[test]
    fn editing_a_pattern_keeps_the_filter_at_its_index() {
        let mut set = set_with(&["alpha", "beta"]);
        let colour = set.filters()[0].style;

        assert!(set.set_pattern(0, "gamma").expect("valid pattern"));

        assert_eq!(set.len(), 2, "editing must not grow the set");
        assert_eq!(
            set.verdict("gamma line", KindSet::EMPTY),
            Verdict::Included(0)
        );
        assert_eq!(
            set.verdict("beta line", KindSet::EMPTY),
            Verdict::Included(1)
        );
        assert_eq!(set.filters()[0].style, colour, "the colour moved with it");
    }

    #[test]
    fn editing_a_pattern_replaces_the_old_one() {
        let mut set = set_with(&["alpha"]);

        set.set_pattern(0, "gamma").expect("valid pattern");

        assert_eq!(
            set.verdict("alpha line", KindSet::EMPTY),
            Verdict::Unmatched,
            "the old pattern still matches"
        );
    }

    /// An edit changes the pattern and nothing else — a filter the user had
    /// toggled off must not come back on, and an excluding filter must not
    /// quietly become an including one.
    #[test]
    fn editing_preserves_the_sense_and_the_enabled_state() {
        let mut set = set_excluding(&["heartbeat"]);
        set.set_enabled(0, false);

        set.set_pattern(0, "keepalive").expect("valid pattern");

        assert!(!set.filters()[0].enabled, "a disabled filter came back on");
        assert_eq!(set.filters()[0].sense, Sense::Exclude);

        set.set_enabled(0, true);
        assert_eq!(
            set.verdict("a keepalive line", KindSet::EMPTY),
            Verdict::Excluded
        );
    }

    /// The same discipline `add` follows: compile first, mutate second, so a
    /// rejected pattern leaves the previous one intact and the prompt has
    /// something to stay open over.
    #[test]
    fn an_invalid_edit_is_reported_and_leaves_the_filter_untouched() {
        let mut set = set_with(&["alpha"]);

        assert!(set.set_pattern(0, "[").is_err());

        assert_eq!(
            set.verdict("alpha line", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    #[test]
    fn editing_out_of_range_reports_failure_and_changes_nothing() {
        let mut set = set_with(&["alpha"]);

        assert!(!set.set_pattern(5, "gamma").expect("valid pattern"));
        assert_eq!(set.len(), 1);
        assert_eq!(
            set.verdict("alpha line", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    /// Unlike `add` and `remove`, an edit leaves the set's *shape* alone — same
    /// length, same enabled flags — so a pending `!` capture still describes it
    /// exactly and must survive. Dropping it here would strand the restore for
    /// no reason.
    #[test]
    fn editing_keeps_a_pending_capture_valid() {
        let mut set = set_with(&["alpha", "beta"]);
        set.set_enabled(1, false);
        set.disable_all_remembering();

        set.set_pattern(0, "gamma").expect("valid pattern");
        assert!(set.has_remembered(), "the capture is still accurate");

        set.restore_remembered();

        assert!(set.filters()[0].enabled);
        assert!(
            !set.filters()[1].enabled,
            "a filter the user had off came back on"
        );
    }
}
