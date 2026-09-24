//! Filters decide how each line of the viewed file is presented.
//!
//! A filter set describes a *log format* rather than a document, so it outlives
//! any one file. Matching is by regular expression, the same as search, so
//! `^foo` anchors to the start of a line.

use crate::syntax::{Kind, KindSet};
use ratatui::style::{Color, Modifier, Style};
use regex::{Regex, RegexSet};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
/// selects a file in the navigator.
///
/// A variant rather than a flag on `Include`: an `Exclude` already never
/// selects a file, so "selects?" is not orthogonal to sense but one more value
/// of it — and the compiler then finds every `match` that needs to know.
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
    /// steps over it, and the navigator's scan already leaves context
    /// filters out of the mask that marks a file as matching — this variant
    /// is what lets the view agree with it.
    Context(usize),
    /// Matched no including filter.
    Unmatched,
    /// Removed by an excluding filter.
    Excluded,
}

/// How the enabled include filters combine to include a line (#39).
///
/// Global, not per filter: a flag on the set needs no per-filter state, no
/// nested pane, and leaves `Enter`, `d` and `!` untouched. Filter groups
/// (#40) are the general form and subsume this if it proves too blunt.
///
/// Only `Sense::Include` filters are terms. `Exclude` is AND-NOT in both
/// modes, as it always was. `Context` stays OR-ed: its promise is "also show
/// these lines", and a context filter that *narrowed* the view would break
/// the one workflow it exists for. The search is not a term at all: it is a
/// motion over the visible lines (ADR 0001), and `p` is how a pattern
/// crosses into the set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Combine {
    /// A line is included when *any* enabled include filter matches it.
    #[default]
    Or,
    /// A line is included only when *every* enabled include filter matches
    /// it. No single filter then owns the line, so the colour is the first
    /// enabled include filter's — one colour for one kind of line.
    And,
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
        }
    }

    /// The regex, when there is one. The navigator's `Matcher` runs regexes
    /// over files it never parses, so it has nothing to ask a definition.
    #[must_use]
    pub fn as_regex(&self) -> Option<&Regex> {
        match self {
            Self::Regex(regex) => Some(regex),
            Self::Definition(_) => None,
        }
    }

    /// The pattern this predicate contributes to the compiled `RegexSet`.
    fn source(&self) -> &str {
        match self {
            Self::Regex(regex) => regex.as_str(),
            Self::Definition(_) => NEVER,
        }
    }

    /// The scan cache's name for this predicate. Distinct from `source` only
    /// in that two definitions of different kinds must not share [`NEVER`].
    /// The leading control character cannot appear in a typed pattern.
    fn key(&self) -> String {
        match self {
            Self::Regex(regex) => regex.as_str().to_string(),
            Self::Definition(kind) => format!("\u{1}{kind}"),
        }
    }

    /// Whether the predicate holds for a line with these `kinds`, by direct
    /// evaluation — the scanning path's counterpart to a `RegexSet` hit.
    fn holds(&self, line: &str, kinds: KindSet) -> bool {
        match self {
            Self::Regex(regex) => regex.is_match(line),
            Self::Definition(kind) => kinds.contains(*kind),
        }
    }
}

#[derive(Debug)]
pub struct Filter {
    pub predicate: Predicate,
    pub sense: Sense,
    pub enabled: bool,
    pub style: Style,
    /// The file's `name` for this filter, if it gave one (#128). `None` for
    /// every typed filter; see `display_name`.
    pub name: Option<String>,
    /// Index into `ActiveFilters::sets`. 0 is the scratch set.
    pub set: usize,
}

impl Filter {
    /// What the pane calls this filter: the file's `name`, else the
    /// predicate's own display. Profiles refer to filters by this string,
    /// so it is also the filter's handle — `filtersets::parse` rejects two
    /// filters in one set that would answer to the same one.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| self.predicate.display())
    }
}

/// Where a set came from, which decides what may be done to it (#128).
///
/// `File` carries its path from day one so that "unload this file" (#46) is
/// a filter over origins later, not a new field then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The unnamed set typed filters land in. Always index 0, never in a file.
    Scratch,
    File(PathBuf),
    /// A set recon ships (#127): always present, its filters never in the
    /// file. A `[sets.<name>]` table may set its `priority`, `autoload`,
    /// `listed`, `description` and `profiles` and nothing else. Its filters take no number and no palette colour.
    BuiltIn,
}

/// The name of the one built-in set: definition filters — functions,
/// classes, structs, enums — answered by the grammar pass in
/// `syntax::definitions` (#127).
pub const DEFINITIONS_SET: &str = "definitions";

/// What the set picker says about the built-in set when the file's
/// `[sets.definitions]` table gives no `description` of its own (#284).
pub const DEFINITIONS_DESCRIPTION: &str =
    "Functions, types and other definitions, found by the syntax grammar";

/// Whether `name` is a set recon ships, which the file may position and
/// switch but not fill.
#[must_use]
pub fn is_builtin_name(name: &str) -> bool {
    name == DEFINITIONS_SET
}

/// A named group of filters, toggled as a unit (#128).
///
/// The filters themselves are not here: they are in `ActiveFilters::filters`,
/// the flat known list, each carrying the index of its set. Keeping the list
/// flat is what leaves `Verdict::Included(index)`, `Matcher` and the scan
/// cache untouched by sets. A filter takes effect only when it is enabled
/// *and* its set is — see `ActiveFilters::effective`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterSet {
    pub name: String,
    pub origin: Origin,
    /// Pane position, lower first. The scratch set ignores it and is always first.
    pub priority: i32,
    /// Enabled at startup, and what a reset returns the flag to — for a
    /// set that is listed.
    pub autoload: bool,
    /// Has a row in the pane (#282). An unlisted set has none and is never
    /// enabled: `enabled` implies `listed`, and the methods that set either
    /// flag keep it so. The file's `listed` is the startup value only (ADR
    /// 0002); `set_listed` changes it for the session.
    pub listed: bool,
    pub enabled: bool,
    /// One line for the set picker (#284), or `None` for a blank.
    pub description: Option<String>,
    /// Named subsets of this set's filters, by `Filter::display_name`.
    pub profiles: BTreeMap<String, Vec<String>>,
}

impl FilterSet {
    fn scratch() -> Self {
        Self {
            name: String::new(),
            origin: Origin::Scratch,
            priority: i32::MIN,
            autoload: true,
            listed: true,
            enabled: true,
            description: None,
            profiles: BTreeMap::new(),
        }
    }
}

/// A solo in force: which set, and what every set's flag was before it.
///
/// From audio mixers. The snapshot is aligned to `ActiveFilters::sets` by
/// index and is taken once, on the first `s`; moving the solo to another set
/// keeps it, so un-soloing returns to the world before the *first* `s` and
/// not to an intermediate one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Solo {
    set: usize,
    snapshot: Vec<bool>,
}

/// One filter as read from `filters.toml`, before it has a colour or a
/// position. The loader's output and the model's input; the model owns the
/// type because the model decides what a set *is*.
#[derive(Debug, Clone)]
pub struct LoadedFilter {
    pub name: String,
    pub predicate: Predicate,
    pub sense: Sense,
    /// The file's `colour`, or `None` for the next palette colour.
    pub colour: Option<Color>,
}

/// One set as read from `filters.toml`.
#[derive(Debug, Clone)]
pub struct LoadedSet {
    pub name: String,
    pub path: PathBuf,
    pub priority: i32,
    pub autoload: bool,
    /// The file's `listed`, `true` when absent. Wins over `autoload`.
    pub listed: bool,
    /// The file's `description`, `None` when absent (#284).
    pub description: Option<String>,
    pub profiles: BTreeMap<String, Vec<String>>,
    pub filters: Vec<LoadedFilter>,
    /// A `[sets.<name>]` table naming a built-in set: `priority`,
    /// `autoload` and `profiles` are the file's, the filters are recon's,
    /// and `filters` above is empty.
    pub builtin: bool,
}

impl LoadedSet {
    /// The built-in definitions set as it is when `filters.toml` has no
    /// table for it: collapsed, at the default priority, no profiles.
    ///
    /// The loader appends this to every file that does not name the set,
    /// so the list `Config::check_sets` validates `--set` against and the
    /// list `with_sets` builds from are the same list (#220). `with_sets`
    /// falls back to it too, for a caller that never ran the loader.
    #[must_use]
    pub fn builtin_default() -> Self {
        Self {
            name: DEFINITIONS_SET.to_string(),
            path: PathBuf::new(),
            priority: crate::filtersets::DEFAULT_PRIORITY,
            autoload: false,
            listed: true,
            description: None,
            profiles: BTreeMap::new(),
            filters: Vec::new(),
            builtin: true,
        }
    }
}

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

/// Every enabled flag in an [`ActiveFilters`], captured so it can be restored.
///
/// Opaque on purpose: it is a token to hand back to
/// [`ActiveFilters::apply_enabled_flags`], not a structure to read or build.
/// Positions in it are meaningless without the set it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledFlags {
    filters: Vec<bool>,
}

/// The bitset width. Up to 64 patterns in total; past this the navigator's
/// file matching switches off rather than shifting out of range.
const MAX_PATTERNS: usize = 64;

/// Which filter selected a file, for its colour in the navigator: the
/// filter's index. The lowest index wins — the view's "first matching filter
/// wins", applied per file.
///
/// Used to be an enum with a `Search` variant that outranked every numbered
/// filter. A search is a motion now (ADR 0001) and marks no file, so only
/// the index is left.
pub type Owner = usize;

/// A `Send` snapshot of the filter set, for a scan thread to match with (#119).
///
/// `ActiveFilters` is neither `Send` nor `Clone`. This is the three things a
/// scan needs from it: every pattern compiled into one `RegexSet` (cloning one
/// is an `Arc` bump), and which positions currently select or exclude. The set
/// covers every pattern whether or not it is enabled — a deliberate choice in
/// #86 — which is what makes a line's `bits` independent of the enabled mask,
/// and so reusable across toggles.
///
/// `Context` filters are in neither mask: they neither select nor exclude, so
/// a line only a context filter hit contributes nothing to a file's answer.
#[derive(Debug, Clone)]
pub struct Matcher {
    set: RegexSet,
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Include`.
    selects: u64,
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Exclude`.
    exclude: u64,
    /// How the include bits combine. In `And` mode a line selects only when
    /// every include bit in `selects` is set.
    combine: Combine,
}

impl Matcher {
    /// Which patterns hit `line`, enabled or not.
    #[must_use]
    pub fn bits(&self, line: &str) -> u64 {
        self.set
            .matches(line)
            .iter()
            .fold(0, |bits, index| bits | (1 << index))
    }

    /// Whether a line with these hits selects its file, and no enabled
    /// `Exclude` filter hits it.
    ///
    /// `Or`: an enabled `Include` filter hits it. `And`: every enabled
    /// `Include` filter hits it. With no include filter enabled, `And`
    /// selects nothing, the same as `Or`.
    #[must_use]
    pub fn selects(&self, bits: u64) -> bool {
        if bits & self.exclude != 0 {
            return false;
        }
        let includes = self.selects;
        match self.combine {
            Combine::Or => bits & includes != 0,
            Combine::And => includes != 0 && bits & includes == includes,
        }
    }

    /// Which filter selected a line with these hits, if any. In `And` mode
    /// every include filter hit, so this is the first enabled one — the same
    /// colour `verdict` gives the line.
    #[must_use]
    pub fn owner(&self, bits: u64) -> Option<Owner> {
        if !self.selects(bits) {
            return None;
        }
        Some((bits & self.selects).trailing_zeros() as usize)
    }

    /// `(selects, exclude, combine)`, for the caller that wants to know
    /// whether a toggle changed anything a scan cares about. The mode is part
    /// of it: the same bitset answers differently under each.
    #[must_use]
    pub fn masks(&self) -> (u64, u64, Combine) {
        (self.selects, self.exclude, self.combine)
    }
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
    /// How the enabled include filters combine. `Or` by default; `&` flips it.
    combine: Combine,
    filters: Vec<Filter>,
    /// Enabled flags captured by `disable_all_remembering`, awaiting a restore.
    ///
    /// Held separately from the filters so that a filter removed in the
    /// meantime simply drops out of the restore rather than resurrecting.
    remembered: Option<Vec<bool>>,
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
    /// `App` and headless mode go through `with_sets` with the loaded sets.
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
            combine: Combine::default(),
            filters: Vec::new(),
            remembered: None,
            compiled: None,
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

    /// The colour the next filter added will take.
    fn next_style(&self) -> Style {
        Style::default().fg(self.palette.colour(self.user_authored_count()))
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
    pub fn add(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let compiled = Regex::new(pattern)?;
        let style = self.next_style();
        self.insert_scratch(Filter {
            predicate: Predicate::Regex(compiled),
            sense: Sense::Include,
            enabled: true,
            style,
            name: None,
            set: 0,
        });
        Ok(())
    }

    /// Add an excluding filter: its matches are removed from view entirely,
    /// in both display modes.
    ///
    /// Excluding filters carry no colour, since a line they match is never
    /// rendered.
    pub fn add_excluding(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.insert_scratch(Filter {
            predicate: Predicate::Regex(pattern),
            sense: Sense::Exclude,
            enabled: true,
            style: Style::default(),
            name: None,
            set: 0,
        });
        Ok(())
    }

    /// Add an including filter that selects lines starting a definition of
    /// `kind` (#123), coloured and numbered like any other filter. Test-only
    /// (#167): the built-in `definitions` set (#127) is how a user gets one,
    /// and this is the direct route the filter and document tests take.
    #[cfg(test)]
    pub(crate) fn add_definition(&mut self, kind: Kind) {
        let style = self.next_style();
        self.insert_scratch(Filter {
            predicate: Predicate::Definition(kind),
            sense: Sense::Include,
            enabled: true,
            style,
            name: None,
            set: 0,
        });
    }

    /// Whether any filter is a definition predicate, and so needs the
    /// document to supply each line's kinds. What gates the whole-file
    /// grammar pass in `Document::evaluate`.
    #[must_use]
    pub fn needs_kinds(&self) -> bool {
        // Effective, not merely present: the built-in set means a definition
        // filter always exists, and a whole-file grammar pass for a row
        // nobody has turned on would be paid by every source file opened.
        self.filters.iter().enumerate().any(|(index, filter)| {
            matches!(filter.predicate, Predicate::Definition(_)) && self.effective(index)
        })
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
        self.filters.iter().enumerate().any(|(index, filter)| {
            matches!(filter.predicate, Predicate::Regex(_)) && self.effective(index)
        })
    }

    /// Build the startup set (#128): the scratch set, then `sets` in the
    /// order given — the loader has already sorted them by priority and
    /// name — every file filter disabled, then each `autoload` set that is
    /// also listed enabled, which applies its `default` profile if it has one.
    ///
    /// A file filter's colour is its position in the known list, the same
    /// rule `add` uses, unless the file named one. Assigned here, once, and
    /// stored on the filter: toggling a set later changes what is shown and
    /// what matches, never a colour.
    #[must_use]
    pub fn with_sets(palette: Option<Vec<Color>>, sets: &[LoadedSet]) -> Self {
        let mut this = Self::bare(palette);
        // The built-in set is always present. The loader supplies it — a
        // `[sets.definitions]` table passed through with its `priority`,
        // `autoload` and `profiles` and no filters, or the default when the
        // file has no such table — so a caller that ran the loader never
        // reaches the fallback. It is here for the ones that did not:
        // `new`, `with_palette` and the tests.
        let mut ordered: Vec<&LoadedSet> = sets.iter().collect();
        let default_builtin = LoadedSet::builtin_default();
        if !ordered.iter().any(|set| set.builtin) {
            ordered.push(&default_builtin);
        }
        ordered.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.name.cmp(&b.name))
        });

        for loaded in ordered {
            let index = this.sets.len();
            if loaded.builtin {
                // The profiles are the file's, over the kinds' plural
                // names; the loader has checked every member is one.
                this.sets.push(FilterSet {
                    name: loaded.name.clone(),
                    origin: Origin::BuiltIn,
                    priority: loaded.priority,
                    autoload: loaded.autoload,
                    listed: loaded.listed,
                    enabled: false,
                    // Recon describes its own set; the table can override.
                    description: Some(
                        loaded
                            .description
                            .clone()
                            .unwrap_or_else(|| DEFINITIONS_DESCRIPTION.to_string()),
                    ),
                    profiles: loaded.profiles.clone(),
                });
                // No palette colour: a built-in filter wears the terminal's
                // default, so the pane's colours stay the user's own.
                for kind in Kind::ALL {
                    this.filters.push(Filter {
                        predicate: Predicate::Definition(kind),
                        sense: Sense::Include,
                        enabled: false,
                        style: Style::default(),
                        name: Some(kind.plural().to_string()),
                        set: index,
                    });
                }
                continue;
            }
            this.sets.push(FilterSet {
                name: loaded.name.clone(),
                origin: Origin::File(loaded.path.clone()),
                priority: loaded.priority,
                autoload: loaded.autoload,
                listed: loaded.listed,
                enabled: false,
                description: loaded.description.clone(),
                profiles: loaded.profiles.clone(),
            });
            for filter in &loaded.filters {
                let style = match filter.colour {
                    Some(colour) => Style::default().fg(colour),
                    None => this.next_style(),
                };
                this.filters.push(Filter {
                    predicate: filter.predicate.clone(),
                    sense: filter.sense,
                    enabled: false,
                    style,
                    name: Some(filter.name.clone()),
                    set: index,
                });
            }
        }
        this.recompile();
        // `listed = false` wins over `autoload = true` (ADR 0002).
        for index in 1..this.sets.len() {
            if this.sets[index].autoload && this.sets[index].listed {
                this.set_enabled_set(index, true);
            }
        }
        this
    }

    /// Every set, scratch first, then in pane order.
    #[must_use]
    pub fn sets(&self) -> &[FilterSet] {
        &self.sets
    }

    /// The filters in `set`, with their known-list indices.
    pub fn filters_in(&self, set: usize) -> impl Iterator<Item = (usize, &Filter)> {
        self.filters
            .iter()
            .enumerate()
            .filter(move |(_, filter)| filter.set == set)
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

    /// Enable or disable a named set. Enabling applies the set's `default`
    /// profile if it has one; otherwise, and on disable, no filter flag
    /// moves — a set re-enabled without a `default` shows the toggles it
    /// had.
    ///
    /// Returns `false` for the scratch set, which is never toggled by hand,
    /// for an index that names no set, and for enabling an unlisted set —
    /// enabled implies listed (#282), so `set_listed` comes first.
    pub fn set_enabled_set(&mut self, set: usize, enabled: bool) -> bool {
        if set == 0 || set >= self.sets.len() || (enabled && !self.sets[set].listed) {
            return false;
        }
        self.sets[set].enabled = enabled;
        if enabled && self.sets[set].profiles.contains_key("default") {
            self.apply_profile(set, "default");
        }
        true
    }

    /// Flip a named set, reporting its new state — or `None` for the
    /// scratch set, for an index that names no set, and for an unlisted set.
    pub fn toggle_set(&mut self, set: usize) -> Option<bool> {
        let now = !self.sets.get(set)?.enabled;
        self.set_enabled_set(set, now).then_some(now)
    }

    /// List or unlist a named set for this session (#282).
    ///
    /// Unlisting also disables the set, so it stops deciding lines at once;
    /// its filter flags are kept. Unlisting the soloed set ends the solo.
    /// Listing gives a disabled set with the flags it had, in its `priority`
    /// position, which it never left: `autoload` is a startup value and does
    /// not apply here.
    ///
    /// Returns `false`, changing nothing, for the scratch set — which is
    /// always listed — and for an index that names no set.
    pub fn set_listed(&mut self, set: usize, listed: bool) -> bool {
        if set == 0 || set >= self.sets.len() {
            return false;
        }
        self.sets[set].listed = listed;
        if !listed {
            self.sets[set].enabled = false;
            if let Some(solo) = self.solo.take_if(|solo| solo.set == set) {
                self.restore(solo.snapshot);
            }
        }
        true
    }

    /// Put every set's flag back from a solo's snapshot, except that an
    /// unlisted set stays disabled: a snapshot does not bring a set back
    /// (ADR 0002).
    fn restore(&mut self, snapshot: Vec<bool>) {
        for (meta, was) in self.sets.iter_mut().zip(snapshot) {
            meta.enabled = was && meta.listed;
        }
    }

    /// Enable exactly the profile's members within `set` and disable the
    /// set's other filters. An action, not a binding: nothing remembers that
    /// a profile was applied, and later toggles are the user's to make.
    pub fn apply_profile(&mut self, set: usize, profile: &str) -> bool {
        let Some(members) = self
            .sets
            .get(set)
            .and_then(|meta| meta.profiles.get(profile))
            .cloned()
        else {
            return false;
        };
        for filter in self.filters.iter_mut().filter(|f| f.set == set) {
            filter.enabled = members.contains(&filter.display_name());
        }
        true
    }

    /// List and enable the set called `set` — `default` profile and all,
    /// exactly as `set_enabled_set` does — then apply `profile` when one is named
    /// (#143). Both names are checked before anything moves, so a refused
    /// call changes nothing. `Config::check_sets` refuses the same names in
    /// `main` before the terminal comes up; this is the same lookup, so a
    /// name that passed there cannot fail here.
    pub fn enable_named(&mut self, set: &str, profile: Option<&str>) -> Result<(), EnableError> {
        let index = self.named(set)?;
        if let Some(profile) = profile
            && !self.sets[index].profiles.contains_key(profile)
        {
            return Err(EnableError::UnknownProfile {
                set: set.to_string(),
                profile: profile.to_string(),
            });
        }
        self.set_listed(index, true);
        self.set_enabled_set(index, true);
        if let Some(profile) = profile {
            self.apply_profile(index, profile);
        }
        Ok(())
    }

    /// Unlist the set called `set`, as `set_listed` does (#283). The same
    /// lookup as `enable_named`, so `Config::check_sets` refuses in `main`
    /// every name that would fail here.
    pub fn unlist_named(&mut self, set: &str) -> Result<(), EnableError> {
        let index = self.named(set)?;
        self.set_listed(index, false);
        Ok(())
    }

    /// The index of the named set called `set`. The scratch set has no name
    /// a flag can give, so it is never found.
    fn named(&self, set: &str) -> Result<usize, EnableError> {
        self.sets
            .iter()
            .position(|meta| meta.name == set)
            .filter(|&index| index != 0)
            .ok_or_else(|| EnableError::UnknownSet(set.to_string()))
    }

    /// Solo `set` (#132): snapshot every set's flag — the scratch set's
    /// included — and enable only `set`. On the soloed set, restore the
    /// snapshot instead. On another set while soloed, move the solo there
    /// and keep the original snapshot. Returns whether a solo is now on.
    ///
    /// Filter flags are untouched throughout: the soloed set shows exactly
    /// the toggles it had. A set that was off comes on the way `Enter`
    /// brings it on, `default` profile and all. Toggling a set by hand while
    /// soloed is drift, as toggling a filter during `!` is; un-solo restores
    /// the snapshot regardless.
    pub fn solo(&mut self, set: usize) -> bool {
        if set == 0 || set >= self.sets.len() || !self.sets[set].listed {
            return self.solo.is_some();
        }
        if let Some(current) = self.solo.take() {
            if current.set == set {
                self.restore(current.snapshot);
                return false;
            }
            self.solo = Some(Solo {
                set,
                snapshot: current.snapshot,
            });
        } else {
            self.solo = Some(Solo {
                set,
                snapshot: self.sets.iter().map(|meta| meta.enabled).collect(),
            });
        }
        let was_enabled = self.sets[set].enabled;
        for (index, meta) in self.sets.iter_mut().enumerate() {
            meta.enabled = index == set;
        }
        if !was_enabled && self.sets[set].profiles.contains_key("default") {
            self.apply_profile(set, "default");
        }
        true
    }

    /// The soloed set, if any.
    #[must_use]
    pub fn soloed(&self) -> Option<usize> {
        self.solo.as_ref().map(|solo| solo.set)
    }

    /// Every set back to its startup state (#132): enabled iff `autoload`
    /// and listed now — the listed state itself is left alone (#282) —
    /// each file filter from its set's `default` profile if there is one and
    /// off otherwise, no solo, no pending `!` capture.
    ///
    /// Scratch filters are not deleted and their flags are left alone: a
    /// reset must never destroy something the user typed. Flags only, and
    /// one key to redo from any state, so it asks no confirmation.
    pub fn reset(&mut self) {
        self.solo = None;
        self.forget_capture();
        self.sets[0].enabled = true;
        for set in 1..self.sets.len() {
            for filter in self.filters.iter_mut().filter(|f| f.set == set) {
                filter.enabled = false;
            }
            self.sets[set].enabled = false;
            if self.sets[set].autoload && self.sets[set].listed {
                self.set_enabled_set(set, true);
            }
        }
    }

    /// Turn the scratch set into a named, enabled set (#131), in memory.
    ///
    /// The new set takes the default priority and a `default` profile of
    /// the scratch filters that are on right now, so it opens the way it
    /// is. Its filters keep their colours — colour is a property of the
    /// filter — and their flags. Other sets keep whatever state they are
    /// in: this is not a reload. Returns `false`, changing nothing, when
    /// the scratch set is empty or a set of that name already exists.
    pub fn adopt_scratch_as(&mut self, name: &str, path: PathBuf) -> bool {
        let count = self.scratch_end();
        if count == 0 || self.sets.iter().any(|meta| meta.name == name) {
            return false;
        }
        let priority = crate::filtersets::DEFAULT_PRIORITY;
        let default: Vec<String> = self.filters[..count]
            .iter()
            .filter(|filter| filter.enabled)
            .map(Filter::display_name)
            .collect();
        let mut profiles = BTreeMap::new();
        if !default.is_empty() {
            profiles.insert("default".to_string(), default);
        }
        // Its place among the named sets: after every set that sorts before
        // it by (priority, name), which is the loader's order.
        let at = self
            .sets
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, meta)| (meta.priority, meta.name.as_str()) > (priority, name))
            .map_or(self.sets.len(), |(index, _)| index);
        self.sets.insert(
            at,
            FilterSet {
                name: name.to_string(),
                origin: Origin::File(path),
                priority,
                autoload: false,
                listed: true,
                enabled: true,
                // `S` writes no description (#284), so the session has none.
                description: None,
                profiles,
            },
        );
        if let Some(solo) = self.solo.as_mut() {
            solo.snapshot.insert(at, false);
            if solo.set >= at {
                solo.set += 1;
            }
        }
        // Renumber: every filter in a set at or past the insertion point
        // moves up one; the scratch filters take the new index and move to
        // the end of the sets before it, keeping the known list contiguous.
        for filter in &mut self.filters {
            if filter.set >= at {
                filter.set += 1;
            }
        }
        let mut adopted: Vec<Filter> = self.filters.drain(..count).collect();
        for filter in &mut adopted {
            filter.set = at;
        }
        let splice_at = self
            .filters
            .iter()
            .position(|filter| filter.set > at)
            .unwrap_or(self.filters.len());
        self.filters.splice(splice_at..splice_at, adopted);
        self.recompile();
        self.forget_capture();
        true
    }

    /// Where the scratch set's filters end: the first filter not in set 0.
    fn scratch_end(&self) -> usize {
        self.filters
            .iter()
            .position(|filter| filter.set != 0)
            .unwrap_or(self.filters.len())
    }

    /// Put a typed filter at the end of the scratch range.
    ///
    /// An insert, not a push: file filters follow the scratch set in the
    /// known list, and the list must stay contiguous by set. Every cached
    /// `Verdict::Included` is a position, so callers re-evaluate — which
    /// they already did, since a push had the same effect on the compiled
    /// set. A pending `!` capture describes a set that no longer exists and
    /// is dropped for the same reason it always was.
    fn insert_scratch(&mut self, filter: Filter) {
        let at = self.scratch_end();
        self.filters.insert(at, filter);
        self.recompile();
        self.forget_capture();
    }

    /// Rows the filter pane numbers: the user-authored filters. The status
    /// row's "N filters" is about what the user built, and a built-in row
    /// nobody turned on is not that.
    ///
    /// Distinct from `len`, which counts every filter and is what
    /// `Verdict::Included` indexes into.
    ///
    /// An unlisted set's filters are not counted: the pane has no row for
    /// them (#282).
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.filters
            .iter()
            .filter(|filter| {
                let set = &self.sets[filter.set];
                set.origin != Origin::BuiltIn && set.listed
            })
            .count()
    }

    /// Drop a pending `!` capture.
    ///
    /// A capture describes a set that no longer exists once the set changes.
    /// Keeping it would strand it — see `add`.
    fn forget_capture(&mut self) {
        self.remembered = None;
    }

    /// Whether any enabled filter removes lines.
    /// Flip between OR and AND (#39), reporting whether AND is now on.
    ///
    /// Only a flag read at verdict time, like `enabled`: no recompile, and
    /// nothing about the pattern list changes. Callers re-evaluate, since
    /// every cached verdict was decided under the other rule.
    pub fn toggle_and(&mut self) -> bool {
        self.combine = match self.combine {
            Combine::Or => Combine::And,
            Combine::And => Combine::Or,
        };
        self.is_and()
    }

    /// Whether the enabled include filters are combined with AND.
    #[must_use]
    pub fn is_and(&self) -> bool {
        self.combine == Combine::And
    }

    /// The include verdict once exclusion has had its say:
    /// which include or context filter, if any, claims a line whose hits
    /// `hit` reports. Shared by the compiled and scanning paths so the two
    /// cannot disagree about the mode.
    ///
    /// `Or`: the first enabled non-excluding filter that hit. `And`: every
    /// enabled `Include` filter must have hit, and the first of them takes
    /// the colour; failing that, a context filter that hit still shows the
    /// line — context is OR-ed in both modes, see [`Combine`].
    fn include_verdict(&self, hit: impl Fn(usize) -> bool) -> Verdict {
        let live = |sense: Sense| {
            self.filters
                .iter()
                .enumerate()
                .filter(move |&(index, filter)| self.effective(index) && filter.sense == sense)
        };
        // Include outranks context in both modes, whatever their order in
        // the set: a line an include filter selected is a hit, and must say
        // so — `n` stops on `Included`, not on `Context` — and its colour is
        // then the same one the navigator gives the file, whose owner is the
        // lowest *selecting* filter and never a context one.
        match self.combine {
            Combine::Or => live(Sense::Include)
                .find(|&(index, _)| hit(index))
                .map(|(index, _)| Verdict::Included(index))
                .or_else(|| {
                    live(Sense::Context)
                        .find(|&(index, _)| hit(index))
                        .map(|(index, _)| Verdict::Context(index))
                })
                .unwrap_or(Verdict::Unmatched),
            Combine::And => {
                let mut includes = live(Sense::Include).map(|(index, _)| index).peekable();
                let first = includes.peek().copied();
                if let Some(first) = first
                    && includes.all(&hit)
                {
                    return Verdict::Included(first);
                }
                live(Sense::Context)
                    .find(|&(index, _)| hit(index))
                    .map_or(Verdict::Unmatched, |(index, _)| Verdict::Context(index))
            }
        }
    }

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
        if let Some(remembered) = self.remembered.as_mut()
            && index < remembered.len()
        {
            remembered.remove(index);
        }
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
    /// Deliberately does **not** `forget_capture`. Every other mutator here
    /// drops a pending `!` capture because it changes the set's *shape* —
    /// `remembered` is a `Vec<bool>` aligned to `filters` by position, so an
    /// add or a remove invalidates it. An edit changes neither the length nor
    /// any enabled flag, so the capture still describes this set exactly and
    /// dropping it would strand a restore for nothing.
    ///
    /// Callers must re-evaluate: the verdicts cached against the old pattern
    /// are stale. Unlike `remove`, only *this* filter's verdicts can have
    /// changed — the numbering is untouched — but `Document::evaluate` is the
    /// only thing that recomputes them, so a full pass is what a caller owes.
    pub fn set_pattern(&mut self, index: usize, pattern: &str) -> Result<bool, regex::Error> {
        let compiled = Regex::new(pattern)?;
        if !self.is_user_authored(index) {
            return Ok(false);
        }
        match self.filters.get_mut(index) {
            Some(filter) => {
                filter.predicate = Predicate::Regex(compiled);
                self.recompile();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Flip a filter between `Include` and `Context`, reporting whether it
    /// changed. An `Exclude` filter is left alone: it already selects nothing.
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
            Sense::Exclude => return false,
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
        self.remembered = Some(self.filters.iter().map(|f| f.enabled).collect());
        self.set_all_enabled(false);
    }

    /// Put back exactly the state `disable_all_remembering` captured.
    ///
    /// Enabling everything instead would silently switch on filters the user
    /// had deliberately turned off.
    pub fn restore_remembered(&mut self) {
        let Some(remembered) = self.remembered.take() else {
            return;
        };
        for (filter, was_enabled) in self.filters.iter_mut().zip(remembered) {
            filter.enabled = was_enabled;
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
            filters: self.filters.iter().map(|filter| filter.enabled).collect(),
        }
    }

    /// Put back what [`enabled_flags`](Self::enabled_flags) captured.
    ///
    /// `zip` rather than an index, and the same tolerance `restore_remembered`
    /// has: a filter deleted since the capture simply drops out of the restore
    /// rather than resurrecting, and one added since keeps whatever it has now.
    /// A snapshot is a convenience, not a transaction.
    pub fn apply_enabled_flags(&mut self, flags: &EnabledFlags) {
        for (filter, was_enabled) in self.filters.iter_mut().zip(&flags.filters) {
            filter.enabled = *was_enabled;
        }
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
        debug_assert!(
            self.compiled.as_ref().is_none_or(|set| self.in_step(set)),
            "the compiled set is out of step with the filters"
        );
        let Some(set) = self.compiled.as_ref().filter(|set| self.in_step(set)) else {
            return self.verdict_by_scanning(line, kinds);
        };
        // Nothing in the compiled set can take effect, so it is not asked:
        // one pass over the line is still one regex pass too many when
        // there is no regex to run (#265). The scan is exact here — it
        // tests only effective filters, and every one of those is a
        // definition, answered from `kinds`.
        if !self.needs_regex() {
            return self.verdict_by_scanning(line, kinds);
        }
        let matched = set.matches(line);
        let matched = |index: usize| matched.matched(index);
        // A definition's slot in the set never matches (see `NEVER`), so it
        // is answered from `kinds` instead; a regex's is the set's answer.
        let hit = |index: usize| match &self.filters[index].predicate {
            Predicate::Regex(_) => matched(index),
            Predicate::Definition(kind) => kinds.contains(*kind),
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
    fn verdict_by_scanning(&self, line: &str, kinds: KindSet) -> Verdict {
        let holds = |filter: &Filter| filter.predicate.holds(line, kinds);
        if self.filters.iter().enumerate().any(|(index, filter)| {
            self.effective(index) && filter.sense == Sense::Exclude && holds(filter)
        }) {
            return Verdict::Excluded;
        }

        self.include_verdict(|index| holds(&self.filters[index]))
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
    /// slow, but `matcher` returns `None` and the navigator's marking
    /// switches off with nothing said (#187).
    fn recompile(&mut self) {
        let patterns = self.filters.iter().map(|filter| filter.predicate.source());
        match RegexSet::new(patterns) {
            Ok(set) => self.compiled = Some(set),
            Err(err) => {
                log::warn!(
                    "cannot compile the filter patterns together: {err}; \
                     the navigator stops marking files until they change"
                );
                self.compiled = None;
            }
        }
    }

    /// The snapshot a scan thread matches with, or `None` when there is no
    /// scan to run.
    ///
    /// `None` when nothing selects — no enabled `Include`, which is the same
    /// "nothing to match against" guard `Document` applies for #36 — and when
    /// the pattern count exceeds the bitset width.
    #[must_use]
    pub fn matcher(&self) -> Option<Matcher> {
        debug_assert!(
            self.compiled.as_ref().is_none_or(|set| self.in_step(set)),
            "the compiled set is out of step with the filters"
        );
        let set = self.compiled.as_ref().filter(|set| self.in_step(set))?;
        if set.len() > MAX_PATTERNS {
            return None;
        }
        let mut selects = 0u64;
        let mut exclude = 0u64;
        for (index, filter) in self.filters.iter().enumerate() {
            // A definition predicate is not a regex over the line's text,
            // and a scan reads text it never parses: its bit stays out of
            // every mask, so it neither selects nor excludes a file. The
            // view still applies it per line.
            if !self.effective(index) || filter.predicate.as_regex().is_none() {
                continue;
            }
            match filter.sense {
                Sense::Include => selects |= 1 << index,
                Sense::Exclude => exclude |= 1 << index,
                Sense::Context => {}
            }
        }
        if selects == 0 {
            return None;
        }
        Some(Matcher {
            set: set.clone(),
            selects,
            exclude,
            combine: self.combine,
        })
    }

    /// Every pattern's source, in compiled order. What a scan
    /// cache is keyed on: a change here shifts bit positions, so cached
    /// bitsets mean something else. Sense and enabled are deliberately not
    /// part of it — they are masks over the same bits.
    #[must_use]
    pub fn pattern_key(&self) -> Vec<String> {
        self.filters
            .iter()
            .map(|filter| filter.predicate.key())
            .collect()
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
    use regex::Regex;
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
                    predicate: Predicate::Regex(Regex::new(pattern).expect("valid")),
                    sense: Sense::Include,
                    colour: None,
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
    use super::test_support::{builtin_override, builtin_with_profiles, loaded};
    use super::*;

    fn set_with(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    // ---- sets (#128) -------------------------------------------------------

    fn flags(set: &ActiveFilters, of: usize) -> Vec<bool> {
        set.filters_in(of).map(|(_, f)| f.enabled).collect()
    }

    fn with_default_profile() -> ActiveFilters {
        let mut a = loaded("a", 50, false, &["x", "y", "z"]);
        a.profiles
            .insert("default".into(), vec!["x".into(), "z".into()]);
        a.profiles
            .insert("loud".into(), vec!["x".into(), "y".into(), "z".into()]);
        ActiveFilters::with_sets(None, &[a])
    }

    /// A fresh `ActiveFilters` already has one set: the scratch set, so that
    /// every filter lives in a set and nothing needs a "loose filter" case.
    #[test]
    fn a_new_set_has_the_scratch_set_and_the_built_in_one() {
        let set = ActiveFilters::new();
        assert_eq!(
            set.sets().len(),
            2,
            "scratch, and the built-in definitions set"
        );
        assert_eq!(set.sets()[0].origin, Origin::Scratch);
        assert!(set.sets()[0].enabled);
        assert_eq!(set.sets()[0].name, "");
    }

    #[test]
    fn typed_filters_belong_to_the_scratch_set_and_are_named_by_their_pattern() {
        let set = set_with(&["foo", "bar"]);
        assert_eq!(set.filters_in(0).count(), 2);
        assert_eq!(set.filters()[0].display_name(), "foo");
    }

    /// Loaded filters follow the scratch set in the known list, contiguous
    /// by set, and start disabled.
    #[test]
    fn loaded_sets_follow_scratch_in_the_known_list() {
        let set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);
        assert_eq!(set.sets().len(), 3, "scratch, a, definitions");
        assert_eq!(set.sets()[1].name, "a");
        assert_eq!(
            set.sets()[1].origin,
            Origin::File(PathBuf::from("test/filters.toml"))
        );
        assert_eq!(set.len(), 2);
        assert!(set.filters_in(1).all(|(_, f)| !f.enabled));
        assert_eq!(set.filters()[0].display_name(), "x");
    }

    /// A loaded filter's colour is its known-list position in the palette —
    /// the same rule `add` uses — unless the file named one.
    #[test]
    fn loaded_filters_are_coloured_by_position_or_by_the_file() {
        let mut one = loaded("a", 50, false, &["x", "y"]);
        one.filters[1].colour = Some(Color::Red);
        let set = ActiveFilters::with_sets(None, &[one]);
        assert_eq!(
            set.filters()[0].style,
            Style::default().fg(DEFAULT_PALETTE[0])
        );
        assert_eq!(set.filters()[1].style, Style::default().fg(Color::Red));
    }

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

        set.enable_named("a", Some("p"))
            .expect("known set and profile");

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

    #[test]
    fn autoload_sets_start_enabled() {
        let set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 50, true, &["x"]),
                loaded("b", 50, false, &["y"]),
            ],
        );
        assert!(set.sets()[1].enabled);
        assert!(!set.sets()[2].enabled);
    }

    /// A filter typed after loading is inserted at the end of the scratch
    /// range, ahead of every file filter, and takes the next colour after
    /// every known filter so it does not repeat a file filter's colour.
    #[test]
    fn a_typed_filter_lands_after_scratch_and_before_file_filters() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x", "y"])]);
        set.add("typed").expect("valid");
        set.add_excluding("second").expect("valid");
        assert_eq!(set.filters()[0].display_name(), "typed");
        assert_eq!(set.filters()[1].display_name(), "second");
        assert_eq!(set.filters()[0].set, 0);
        assert_eq!(set.filters()[2].set, 1);
        assert_eq!(
            set.filters()[0].style,
            Style::default().fg(DEFAULT_PALETTE[2])
        );
        // The compiled set follows the new order: `typed` is index 0.
        set.set_enabled(0, true);
        assert_eq!(set.verdict("typed", KindSet::EMPTY), Verdict::Included(0));
    }

    /// The rule: a filter takes effect when it is enabled *and* its set is.
    #[test]
    fn a_filter_in_a_disabled_set_matches_nothing() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["foo"])]);
        set.set_enabled(0, true);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Included(0));
        set.set_enabled_set(1, false);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);
        assert_eq!(
            set.verdict_by_scanning("foo", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert!(
            set.filters()[0].enabled,
            "the filter's own flag is untouched"
        );
    }

    /// Exclusion follows the rule too: an exclude in a disabled set removes
    /// nothing.
    #[test]
    fn an_exclude_in_a_disabled_set_removes_nothing() {
        let mut a = loaded("a", 50, true, &["foo", "skip"]);
        a.filters[1].sense = Sense::Exclude;
        let mut set = ActiveFilters::with_sets(None, &[a]);
        set.set_all_enabled(true);
        assert_eq!(set.verdict("foo skip", KindSet::EMPTY), Verdict::Excluded);
        set.set_enabled_set(1, false);
        assert_eq!(set.verdict("foo skip", KindSet::EMPTY), Verdict::Unmatched);
        assert!(!set.any_excluding());
    }

    /// The navigator's masks and dimming follow the same rule.
    #[test]
    fn the_matcher_and_dimming_ignore_filters_in_disabled_sets() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["foo"])]);
        set.set_enabled(0, true);
        assert!(set.matcher().is_some());
        assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE));
        set.set_enabled_set(1, false);
        assert!(
            set.matcher().is_none(),
            "nothing selects, so no scan to run"
        );
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    /// `!` is flag-level: it disables and restores across every known filter
    /// and never touches a set's flag.
    #[test]
    fn bang_acts_on_filter_flags_across_sets_and_leaves_set_flags_alone() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x"])]);
        set.add("typed").expect("valid");
        set.set_enabled(1, true); // x
        set.disable_all_remembering();
        assert_eq!(flags(&set, 0), vec![false]);
        assert_eq!(flags(&set, 1), vec![false]);
        assert!(set.sets()[1].enabled);
        set.restore_remembered();
        assert_eq!(flags(&set, 0), vec![true]);
        assert_eq!(flags(&set, 1), vec![true]);
    }

    /// Enabling a set with a `default` profile applies it.
    #[test]
    fn enabling_a_set_applies_its_default_profile() {
        let mut set = with_default_profile();
        assert!(set.set_enabled_set(1, true));
        assert_eq!(flags(&set, 1), vec![true, false, true]);
    }

    /// `autoload` with no `default` starts the set enabled and every filter
    /// off: `autoload` names the sets that are live at startup, `default`
    /// names their filters, and a set without one names none. Decided on
    /// #161 and documented in the README's `autoload` and `Reset`
    /// paragraphs; pinned here so the alternative that issue offered —
    /// switching every filter on — cannot arrive by accident.
    #[test]
    fn autoload_without_a_default_starts_enabled_with_every_filter_off() {
        let set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x", "y"])]);
        assert!(set.sets()[1].enabled, "the set is enabled");
        assert_eq!(
            flags(&set, 1),
            vec![false, false],
            "and its filters are off"
        );
        assert!(!set.any_enabled(), "nothing filters until a row is toggled");
    }

    /// `autoload` goes through the same path, so `default` applies at startup.
    #[test]
    fn autoload_applies_default() {
        let mut a = loaded("a", 50, true, &["x", "y"]);
        a.profiles.insert("default".into(), vec!["y".into()]);
        let set = ActiveFilters::with_sets(None, &[a]);
        assert_eq!(flags(&set, 1), vec![false, true]);
    }

    /// Without `default`, enabling keeps whatever flags the filters had, and
    /// disabling touches none.
    #[test]
    fn enabling_a_set_without_default_keeps_the_flags() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, false, &["x", "y"])]);
        set.set_enabled(1, true);
        set.set_enabled_set(1, true);
        assert_eq!(flags(&set, 1), vec![false, true]);
        set.set_enabled_set(1, false);
        assert_eq!(flags(&set, 1), vec![false, true]);
        assert_eq!(set.toggle_set(1), Some(true));
        assert_eq!(flags(&set, 1), vec![false, true]);
    }

    /// A profile enables exactly its members and disables the rest of the set.
    #[test]
    fn a_profile_is_exact_and_touches_one_set() {
        let mut set = with_default_profile();
        set.add("typed").expect("valid");
        set.set_enabled_set(1, true);
        assert!(set.apply_profile(1, "loud"));
        assert_eq!(flags(&set, 1), vec![true, true, true]);
        assert!(set.apply_profile(1, "default"));
        assert_eq!(flags(&set, 1), vec![true, false, true]);
        assert!(!set.apply_profile(1, "nope"));
        assert_eq!(flags(&set, 0), vec![true], "the scratch set is untouched");
    }

    /// The scratch set cannot be toggled through this path.
    #[test]
    fn the_scratch_set_is_not_toggleable() {
        let mut set = set_with(&["foo"]);
        assert!(!set.set_enabled_set(0, false));
        assert_eq!(set.toggle_set(0), None);
        assert_eq!(set.toggle_set(7), None);
        assert!(set.sets()[0].enabled);
    }

    // ---- solo and reset (#132) ---------------------------------------------

    fn three_sets() -> ActiveFilters {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 10, true, &["x"]),
                loaded("b", 20, true, &["y"]),
                loaded("c", 30, false, &["z"]),
            ],
        );
        set.add("scratch").expect("valid");
        set
    }

    fn set_flags(set: &ActiveFilters) -> Vec<bool> {
        set.sets().iter().map(|meta| meta.enabled).collect()
    }

    #[test]
    fn solo_enables_one_set_and_suspends_the_rest_including_scratch() {
        let mut set = three_sets();
        assert!(set.solo(2));
        assert_eq!(set_flags(&set), vec![false, false, true, false, false]);
        assert_eq!(set.soloed(), Some(2));
        assert!(
            set.filters().iter().all(|f| f.enabled || f.set != 0),
            "scratch filter flags are untouched"
        );
        assert_eq!(
            set.verdict("scratch", KindSet::EMPTY),
            Verdict::Unmatched,
            "the scratch set is suspended, not just hidden"
        );
    }

    #[test]
    fn solo_again_restores_the_snapshot() {
        let mut set = three_sets();
        set.solo(2);
        assert!(!set.solo(2));
        assert_eq!(set_flags(&set), vec![true, true, true, false, false]);
        assert_eq!(set.soloed(), None);
    }

    #[test]
    fn moving_the_solo_keeps_the_first_snapshot() {
        let mut set = three_sets();
        set.solo(2);
        assert!(set.solo(3));
        assert_eq!(set_flags(&set), vec![false, false, false, true, false]);
        assert!(!set.solo(3));
        assert_eq!(set_flags(&set), vec![true, true, true, false, false]);
    }

    #[test]
    fn soloing_a_disabled_set_applies_its_default() {
        let mut c = loaded("c", 30, false, &["z", "w"]);
        c.profiles.insert("default".into(), vec!["w".into()]);
        let mut set = ActiveFilters::with_sets(None, &[c]);
        set.solo(1);
        assert_eq!(flags(&set, 1), vec![false, true]);
    }

    #[test]
    fn solo_refuses_the_scratch_set_and_bad_indices() {
        let mut set = three_sets();
        assert!(!set.solo(0));
        assert!(!set.solo(9));
        assert_eq!(set_flags(&set), vec![true, true, true, false, false]);
        set.solo(1);
        assert!(set.solo(0), "still soloed after a refused press");
    }

    /// `!` and solo are independent: `!` inside a solo acts on filter flags,
    /// and restores them, while the set flags stay soloed.
    #[test]
    fn bang_inside_a_solo_acts_on_filter_flags_only() {
        let mut set = three_sets();
        set.set_enabled(1, true); // x
        set.solo(1);
        set.disable_all_remembering();
        assert_eq!(flags(&set, 1), vec![false]);
        assert_eq!(set.soloed(), Some(1));
        set.restore_remembered();
        assert_eq!(flags(&set, 1), vec![true]);
        assert_eq!(set_flags(&set), vec![false, true, false, false, false]);
    }

    #[test]
    fn reset_returns_every_set_to_startup_and_leaves_scratch_alone() {
        let mut a = loaded("a", 10, true, &["x", "y"]);
        a.profiles.insert("default".into(), vec!["x".into()]);
        let mut set = ActiveFilters::with_sets(None, &[a, loaded("b", 20, false, &["z"])]);
        set.add("scratch").expect("valid");
        set.set_enabled(0, false); // scratch off, by hand
        set.solo(2);
        set.set_enabled(3, true); // z
        set.set_enabled(1, false); // x, contrary to default
        set.disable_all_remembering();
        set.reset();
        assert_eq!(set_flags(&set), vec![true, true, false, false]);
        assert_eq!(flags(&set, 1), vec![true, false]);
        assert_eq!(flags(&set, 2), vec![false]);
        assert!(!set.filters()[0].enabled, "scratch flag untouched");
        assert_eq!(set.filters_in(0).count(), 1, "scratch filter not deleted");
        assert_eq!(set.soloed(), None);
        assert!(!set.has_remembered());
    }

    // ---- adopting the scratch set (#131) -----------------------------------

    #[test]
    fn adopt_moves_scratch_into_a_new_enabled_set_with_default_from_the_flags() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("m", 10, true, &["q"]),
                loaded("z", 90, false, &["r"]),
            ],
        );
        set.add("a").expect("valid");
        set.add("b").expect("valid");
        set.set_enabled(1, false); // b off
        let a_style = set.filters()[0].style;
        assert!(set.adopt_scratch_as("new", PathBuf::from("t")));
        assert_eq!(set.filters_in(0).count(), 0);
        let names: Vec<&str> = set.sets().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["", "m", "definitions", "new", "z"],
            "priority 50 sorts between 10 and 90"
        );
        let new = 3;
        assert!(set.sets()[new].enabled);
        assert_eq!(set.sets()[new].priority, 50);
        assert_eq!(set.sets()[new].profiles["default"], vec!["a".to_string()]);
        assert_eq!(flags(&set, new), vec![true, false]);
        // Known list stays contiguous by set: m's q, then a, b, then z's r.
        // The built-in set's rows sit between m's and new's; count them
        // rather than name them, since the kinds may grow.
        let order: Vec<(usize, String)> = set
            .filters()
            .iter()
            .filter(|f| set.sets()[f.set].origin != Origin::BuiltIn)
            .map(|f| (f.set, f.display_name()))
            .collect();
        assert_eq!(set.filters_in(2).count(), Kind::ALL.len());
        assert_eq!(
            order,
            vec![
                (1, "q".into()),
                (3, "a".into()),
                (3, "b".into()),
                (4, "r".into())
            ]
        );
        assert_eq!(
            set.filters()[1 + Kind::ALL.len()].style,
            a_style,
            "colour travels with the filter"
        );
        assert_eq!(
            set.verdict("a", KindSet::EMPTY),
            Verdict::Included(1 + Kind::ALL.len())
        );
        assert!(
            !set.adopt_scratch_as("new", PathBuf::from("t")),
            "name taken"
        );
        assert!(
            !set.adopt_scratch_as("other", PathBuf::from("t")),
            "scratch is empty now"
        );
    }

    /// A live solo's snapshot stays aligned with the set list.
    #[test]
    fn adopt_keeps_a_live_solo_aligned() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("z", 90, true, &["r"])]);
        set.add("a").expect("valid");
        set.solo(2); // z: the sets are "", definitions, z
        assert!(set.adopt_scratch_as("new", PathBuf::from("t")));
        assert_eq!(set.soloed(), Some(3), "z moved from 2 to 3");
        set.solo(3);
        let flags: Vec<bool> = set.sets().iter().map(|s| s.enabled).collect();
        assert_eq!(
            flags,
            vec![true, false, false, true],
            "the snapshot restored z as it was and new as off"
        );
    }

    // ---- the built-in definitions set (#127) -------------------------------

    fn builtin_index(set: &ActiveFilters) -> usize {
        set.sets()
            .iter()
            .position(|meta| meta.origin == Origin::BuiltIn)
            .expect("the built-in set is always present")
    }

    #[test]
    fn the_definitions_set_is_always_present_and_collapsed_by_default() {
        let set = ActiveFilters::new();
        assert_eq!(set.sets().len(), 2, "scratch and definitions");
        let index = builtin_index(&set);
        let meta = &set.sets()[index];
        assert_eq!(meta.name, DEFINITIONS_SET);
        assert!(!meta.enabled);
        assert!(!meta.autoload);
        assert_eq!(meta.priority, crate::filtersets::DEFAULT_PRIORITY);
        let kinds: Vec<String> = set
            .filters_in(index)
            .map(|(_, f)| f.display_name())
            .collect();
        let expected: Vec<&str> = Kind::ALL.iter().map(|kind| kind.plural()).collect();
        assert_eq!(kinds, expected);
        assert_eq!(kinds[..4], ["functions", "classes", "structs", "enums"]);
        assert!(set.filters_in(index).all(|(_, f)| !f.enabled));
        assert!(
            set.filters_in(index)
                .all(|(_, f)| matches!(f.predicate, Predicate::Definition(_)))
        );
    }

    /// Built-in filters take no palette colour and do not move the next one.
    #[test]
    fn builtin_filters_do_not_consume_the_palette() {
        let mut set = ActiveFilters::new();
        set.add("typed").expect("valid");
        assert_eq!(
            set.filters()[0].style,
            Style::default().fg(DEFAULT_PALETTE[0])
        );
        let index = builtin_index(&set);
        assert!(
            set.filters_in(index)
                .all(|(_, f)| f.style == Style::default())
        );
        assert!(set.is_user_authored(0));
        assert!(!set.is_user_authored(1));
    }

    /// The built-in set sorts among file sets by priority and name.
    #[test]
    fn the_definitions_set_sorts_among_file_sets() {
        let set = ActiveFilters::with_sets(
            None,
            &[
                loaded("alpha", 50, false, &["a"]),
                loaded("zeta", 50, false, &["z"]),
            ],
        );
        let names: Vec<&str> = set.sets().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["", "alpha", "definitions", "zeta"]);
    }

    /// A `[sets.definitions]` table positions and switches it, and nothing
    /// else: its filters are still recon's four.
    #[test]
    fn a_file_override_positions_and_switches_the_definitions_set() {
        let set = ActiveFilters::with_sets(
            None,
            &[
                loaded("alpha", 50, false, &["a"]),
                builtin_override(10, true),
            ],
        );
        let index = builtin_index(&set);
        assert_eq!(index, 1, "priority 10 sorts before alpha");
        assert!(set.sets()[index].enabled, "autoload");
        assert_eq!(set.filters_in(index).count(), Kind::ALL.len());
        assert_eq!(set.sets().len(), 3, "no second definitions set");
    }

    /// A `[sets.definitions]` table's profiles are the set's (#220): the
    /// `default` one applies on autoload, another applies by name, and the
    /// members are the kinds' plural names.
    #[test]
    fn a_file_override_gives_the_definitions_set_profiles() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[builtin_with_profiles(
                true,
                &[
                    ("default", &["functions"]),
                    ("types", &["types", "structs", "enums"]),
                ],
            )],
        );
        let index = builtin_index(&set);
        assert_eq!(set.sets()[index].profiles.len(), 2);
        let on = |set: &ActiveFilters| -> Vec<String> {
            set.filters_in(index)
                .filter(|(_, f)| f.enabled)
                .map(|(_, f)| f.display_name())
                .collect()
        };
        assert_eq!(on(&set), ["functions"], "autoload applied `default`");
        assert!(set.apply_profile(index, "types"));
        assert_eq!(
            on(&set),
            ["structs", "enums", "types"],
            "in Kind::ALL order"
        );
        assert_eq!(
            set.enable_named(DEFINITIONS_SET, Some("nope")),
            Err(EnableError::UnknownProfile {
                set: DEFINITIONS_SET.to_string(),
                profile: "nope".to_string(),
            })
        );
    }

    /// The grammar pass is paid only once a definition filter takes effect.
    #[test]
    fn needs_kinds_is_false_until_a_definition_filter_is_effective() {
        let mut set = ActiveFilters::new();
        assert!(!set.needs_kinds(), "present is not effective");
        let index = builtin_index(&set);
        set.set_enabled(0, true); // functions, but the set is collapsed
        assert!(!set.needs_kinds());
        set.set_enabled_set(index, true);
        assert!(set.needs_kinds());
    }

    /// The regex pass is paid only once a regex filter of any sense takes
    /// effect. The built-in set alone never asks for it.
    #[test]
    fn needs_regex_is_false_until_a_regex_filter_is_effective() {
        let mut set = ActiveFilters::new();
        assert!(!set.needs_regex(), "the built-in set is all definitions");

        set.add("foo").expect("valid pattern");
        assert!(set.needs_regex());
        set.set_all_enabled(false);
        assert!(!set.needs_regex(), "present is not effective");

        // An excluding filter selects nothing, and `matcher` says so with
        // `None`; it still runs over every line, so it counts here.
        set.add_excluding("noise").expect("valid pattern");
        assert!(set.needs_regex());
        set.set_all_enabled(false);
        assert!(!set.needs_regex());
    }

    /// With no regex to run, a definition filter is still answered from the
    /// kinds, and the definition filters the user left off still say
    /// nothing — the guard changes what is asked, not what is answered.
    #[test]
    fn a_definition_filter_alone_is_answered_without_the_regex_pass() {
        let mut set = ActiveFilters::new();
        let index = builtin_index(&set);
        set.set_enabled(0, true); // functions
        set.set_enabled_set(index, true);
        assert!(!set.needs_regex());

        let functions = KindSet::of(&[Kind::Function]);
        assert_eq!(
            set.verdict("anything at all", functions),
            Verdict::Included(0)
        );
        let structs = KindSet::of(&[Kind::Struct]);
        assert_eq!(set.verdict("a struct line", structs), Verdict::Unmatched);
        assert_eq!(
            set.verdict("plain text", KindSet::EMPTY),
            Verdict::Unmatched
        );
    }

    /// Solo, reset and `!` treat it as any set.
    #[test]
    fn the_definitions_set_is_an_ordinary_set_to_solo_and_reset() {
        let mut set = ActiveFilters::new();
        set.add("typed").expect("valid");
        let index = builtin_index(&set);
        assert!(set.solo(index));
        assert!(set.sets()[index].enabled);
        assert!(!set.sets()[0].enabled);
        set.reset();
        assert!(!set.sets()[index].enabled, "autoload is off");
        assert!(set.sets()[0].enabled);
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

    /// A definition filter is a term of the AND like any include filter.
    #[test]
    fn definitions_are_terms_in_and_mode() {
        let mut set = set_with(&["pub"]);
        set.add_definition(Kind::Function);
        set.toggle_and();
        let functions = KindSet::of(&[Kind::Function]);
        assert_eq!(set.verdict("pub fn x()", functions), Verdict::Included(0));
        assert_eq!(
            set.verdict("pub struct", KindSet::EMPTY),
            Verdict::Unmatched
        );
        assert_eq!(set.verdict("fn x()", functions), Verdict::Unmatched);
    }

    /// The navigator cannot evaluate a definition, so its bit is in no mask:
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
        set.toggle_and();
        let m = set.matcher().expect("still selects under AND");
        assert!(
            m.selects(m.bits("foo")),
            "the definition is not a term of the scan's AND"
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

    // ---- AND mode (#39) ----------------------------------------------------

    /// The default is today's behaviour: any enabled include filter includes.
    #[test]
    fn or_is_the_default() {
        let set = set_with(&["foo", "bar"]);
        assert!(!set.is_and());
        assert_eq!(
            set.verdict("foo only", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    /// In AND mode a line must match every enabled include filter.
    #[test]
    fn and_requires_every_enabled_include_filter() {
        let mut set = set_with(&["foo", "bar"]);
        assert!(set.toggle_and());
        assert_eq!(set.verdict("foo only", KindSet::EMPTY), Verdict::Unmatched);
        assert_eq!(set.verdict("bar only", KindSet::EMPTY), Verdict::Unmatched);
        assert_eq!(
            set.verdict("bar then foo", KindSet::EMPTY),
            Verdict::Included(0)
        );
        assert!(!set.toggle_and(), "a second press turns it back off");
        assert_eq!(
            set.verdict("foo only", KindSet::EMPTY),
            Verdict::Included(0)
        );
    }

    /// The colour is the first *enabled* include filter's, so disabling the
    /// first filter hands the colour to the next — and drops it as a term.
    #[test]
    fn and_colours_by_the_first_enabled_include_filter() {
        let mut set = set_with(&["foo", "bar", "baz"]);
        set.toggle_and();
        set.set_enabled(0, false);
        assert_eq!(set.verdict("bar baz", KindSet::EMPTY), Verdict::Included(1));
        assert_eq!(
            set.verdict("foo bar baz", KindSet::EMPTY),
            Verdict::Included(1)
        );
    }

    /// A disabled filter is not a term, and with no enabled include filter
    /// at all nothing is included — the same as OR mode.
    #[test]
    fn and_with_nothing_enabled_includes_nothing() {
        let mut set = set_with(&["foo"]);
        set.toggle_and();
        set.set_enabled(0, false);
        assert_eq!(set.verdict("foo", KindSet::EMPTY), Verdict::Unmatched);
    }

    /// Exclusion still wins, in either mode.
    #[test]
    fn and_still_excludes() {
        let mut set = set_with(&["foo", "bar"]);
        set.add_excluding("skip").expect("valid");
        set.toggle_and();
        assert_eq!(
            set.verdict("foo bar skip", KindSet::EMPTY),
            Verdict::Excluded
        );
    }

    /// Context filters are not terms of the AND: a line a context filter
    /// matches is still shown, as the sense promises, and a line matching
    /// every include filter is not also required to match the context one.
    #[test]
    fn context_filters_stay_or_ed_in_and_mode() {
        let mut set = set_with(&["foo", "bar", "ctx"]);
        set.toggle_context(2);
        set.toggle_and();
        assert_eq!(set.verdict("foo bar", KindSet::EMPTY), Verdict::Included(0));
        assert_eq!(
            set.verdict("ctx alone", KindSet::EMPTY),
            Verdict::Context(2)
        );
        assert_eq!(set.verdict("foo alone", KindSet::EMPTY), Verdict::Unmatched);
    }

    /// The scanning fallback agrees with the compiled path.
    #[test]
    fn and_agrees_between_compiled_and_scanning_paths() {
        let mut set = set_with(&["foo", "bar"]);
        set.add_excluding("skip").expect("valid");
        set.toggle_and();
        for line in ["foo", "bar foo", "foo bar skip", "nothing"] {
            assert_eq!(
                set.verdict(line, KindSet::EMPTY),
                set.verdict_by_scanning(line, KindSet::EMPTY),
                "{line:?}"
            );
        }
    }

    /// The navigator's rule follows: a file matches when one line matches
    /// every enabled include filter, and its owner is the first of them.
    #[test]
    fn the_matcher_ands_too() {
        let mut set = set_with(&["foo", "bar"]);
        set.toggle_and();
        let m = set.matcher().expect("something selects");
        assert!(!m.selects(m.bits("foo only")));
        assert!(m.selects(m.bits("foo bar")));
        assert_eq!(m.owner(m.bits("foo bar")), Some(0));
        set.set_enabled(0, false);
        let m = set.matcher().expect("bar still selects");
        assert!(m.selects(m.bits("bar only")));
        assert_eq!(m.owner(m.bits("bar only")), Some(1));
    }

    /// Flipping the mode changes what a cached bitset means, so the masks a
    /// scan is keyed on must differ between the two modes.
    #[test]
    fn toggling_and_changes_the_matcher_masks() {
        let mut set = set_with(&["foo", "bar"]);
        let before = set.matcher().expect("selects").masks();
        set.toggle_and();
        let after = set.matcher().expect("selects").masks();
        assert_ne!(before, after);
    }

    // ---- the matcher snapshot --------------------------------------------

    /// The invariant the navigator rests on, stated the way the spec states
    /// it: a line selects its file when an enabled `Include` filter hits it
    /// and no enabled `Exclude` does. Deliberately *not*
    /// derived from `verdict`'s index — that is a colouring rule (first match
    /// wins), and a context filter can win the colour of a line an include
    /// filter also hit. Selecting and colouring are different questions.
    #[test]
    fn the_matcher_agrees_with_the_spec_on_what_selects_a_file() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        set.add_excluding("noise").expect("valid pattern");
        let matcher = set.matcher().expect("something selects");

        for line in [
            "alpha",
            "beta",
            "delta",
            "alpha noise",
            "beta delta",
            "gamma",
            "gamma noise",
            "beta gamma",
            "nothing here",
            "alpha beta",
            "beta noise",
            "delta noise",
        ] {
            let hit = |sense: Sense| {
                set.filters().iter().any(|f| {
                    f.enabled && f.sense == sense && f.predicate.holds(line, KindSet::EMPTY)
                })
            };
            let expected = hit(Sense::Include) && !hit(Sense::Exclude);

            assert_eq!(
                matcher.selects(matcher.bits(line)),
                expected,
                "matcher and the spec disagree on {line:?}"
            );
            // A selected line is always one the view shows.
            if expected {
                assert!(
                    matches!(set.verdict(line, KindSet::EMPTY), Verdict::Included(_)),
                    "{line:?} selects its file but the view would not show it"
                );
            }
        }
    }

    /// The owner is the lowest *selecting* filter — which is not always the
    /// filter `verdict` colours the line with. A line hit by context filter 1
    /// and include filter 2 is drawn in filter 1's colour (first wins) but the
    /// *file* is owned by filter 2: it is the one that selected it.
    #[test]
    fn the_owner_is_the_lowest_selecting_filter() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        let matcher = set.matcher().expect("something selects");

        assert_eq!(matcher.owner(matcher.bits("beta")), None);
        assert_eq!(matcher.owner(matcher.bits("beta delta")), Some(2));
        assert_eq!(
            set.verdict("beta delta", KindSet::EMPTY),
            Verdict::Included(2),
            "the include filter outranks the context one, so the view's colour \
             is the navigator's"
        );
        assert_eq!(matcher.owner(matcher.bits("alpha delta")), Some(0));
    }

    #[test]
    fn a_disabled_filter_neither_selects_nor_excludes() {
        let mut set = set_with(&["alpha"]);
        set.add_excluding("noise").expect("valid pattern");
        set.set_enabled(1, false);
        let matcher = set.matcher().expect("alpha selects");

        assert!(matcher.selects(matcher.bits("alpha noise")));
    }

    /// Nothing selecting means nothing to match against — the #36 guard.
    #[test]
    fn no_matcher_without_a_selecting_filter() {
        assert!(ActiveFilters::new().matcher().is_none(), "empty set");

        let mut context_only = set_with(&["alpha"]);
        context_only.toggle_context(0);
        assert!(context_only.matcher().is_none(), "context only");

        let mut exclude_only = ActiveFilters::new();
        exclude_only.add_excluding("noise").expect("valid pattern");
        assert!(exclude_only.matcher().is_none(), "exclude only");

        let mut disabled = set_with(&["alpha"]);
        disabled.set_enabled(0, false);
        assert!(disabled.matcher().is_none(), "disabled");
    }

    /// 64 is the width of the bitset; the 65th pattern switches the feature off
    /// rather than wrapping a shift.
    #[test]
    fn no_matcher_past_sixty_four_patterns() {
        let mut set = ActiveFilters::new();
        // The built-in definitions set holds one of the 64 slots per kind.
        let room = 64 - Kind::ALL.len();
        for i in 0..room {
            set.add(&format!("p{i}")).expect("valid pattern");
        }
        assert!(set.matcher().is_some());

        set.add(&format!("p{room}")).expect("valid pattern");
        assert!(set.matcher().is_none());
    }

    #[test]
    fn the_pattern_key_lists_every_pattern_in_compiled_order() {
        let mut set = set_with(&["alpha", "beta"]);
        let mut expected = vec!["alpha".to_string(), "beta".to_string()];
        expected.extend(Kind::ALL.iter().map(|kind| format!("\u{1}{kind}")));

        assert_eq!(set.pattern_key(), expected, "typed, then the built-in set");

        set.toggle_context(0);
        assert_eq!(set.pattern_key(), expected, "sense is not part of the key");
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

    // ---- listed and unlisted sets (#282) -----------------------------------

    fn unlisted(mut set: LoadedSet) -> LoadedSet {
        set.listed = false;
        set
    }

    #[test]
    fn a_set_is_listed_unless_the_file_says_otherwise() {
        let set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x"])]);
        assert!(
            set.sets().iter().all(|meta| meta.listed),
            "scratch, a, definitions"
        );
        assert!(set.sets()[1].enabled);
    }

    /// `listed = false` wins over `autoload = true` (ADR 0002): the set is
    /// known, and decides nothing.
    #[test]
    fn an_unlisted_autoload_set_starts_unlisted_and_disabled() {
        let set = ActiveFilters::with_sets(None, &[unlisted(loaded("a", 50, true, &["x"]))]);
        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(set.matcher().is_none(), "its filter selects nothing");
        assert_eq!(set.row_count(), 0, "and has no row");
    }

    #[test]
    fn unlisting_an_enabled_set_disables_it_and_keeps_its_flags() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 10, true, &["x", "y"]),
                loaded("b", 20, false, &["z"]),
            ],
        );
        set.set_enabled(0, true);
        set.set_enabled(1, true);
        assert!(set.needs_regex());

        assert!(set.set_listed(1, false));
        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(!set.needs_regex(), "the visible lines change at once");
        assert_eq!(flags(&set, 1), vec![true, true], "flags kept");

        assert!(set.set_listed(1, true));
        assert_eq!(set.sets()[1].name, "a", "back in its priority position");
        assert!(set.sets()[1].listed);
        assert!(!set.sets()[1].enabled, "listing again gives a disabled set");
        assert_eq!(flags(&set, 1), vec![true, true], "with the flags it had");
    }

    #[test]
    fn an_unlisted_set_cannot_be_enabled_toggled_or_soloed() {
        let mut set = ActiveFilters::with_sets(None, &[unlisted(loaded("a", 50, false, &["x"]))]);
        assert!(!set.set_enabled_set(1, true));
        assert_eq!(set.toggle_set(1), None);
        assert!(!set.solo(1));
        assert_eq!(set.soloed(), None);
        assert!(!set.sets()[1].enabled);
        assert!(set.set_enabled_set(1, false), "disabling is always allowed");
    }

    #[test]
    fn the_scratch_set_cannot_be_unlisted() {
        let mut set = ActiveFilters::new();
        assert!(!set.set_listed(0, false));
        assert!(set.sets()[0].listed);
        assert!(!set.set_listed(99, false), "no such set");
    }

    /// `--set NAME` lists the set and enables it, `default` profile and all,
    /// whatever the file says.
    #[test]
    fn enable_named_lists_an_unlisted_set() {
        let mut a = unlisted(loaded("a", 50, false, &["x", "y"]));
        a.profiles.insert("default".into(), vec!["x".into()]);
        let mut set = ActiveFilters::with_sets(None, &[a]);

        set.enable_named("a", None).expect("known set");

        assert!(set.sets()[1].listed);
        assert!(set.sets()[1].enabled);
        assert_eq!(enabled_names(&set), ["x"]);
    }

    /// `--unlist NAME` (#283): the set has no row and decides nothing, even
    /// with `autoload`.
    #[test]
    fn unlist_named_unlists_an_autoload_set() {
        let mut a = loaded("a", 50, true, &["x"]);
        a.profiles.insert("default".into(), vec!["x".into()]);
        let mut set = ActiveFilters::with_sets(None, &[a]);
        assert!(set.matcher().is_some(), "autoload turned `x` on");

        set.unlist_named("a").expect("known set");

        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(set.matcher().is_none(), "its filter selects nothing");
        assert_eq!(set.row_count(), 0, "and has no row");
    }

    #[test]
    fn unlist_named_refuses_an_unknown_name_and_the_scratch_set() {
        let mut set = ActiveFilters::with_sets(None, &[loaded("a", 50, true, &["x"])]);
        let scratch = set.sets()[0].name.clone();

        assert_eq!(
            set.unlist_named("b"),
            Err(EnableError::UnknownSet("b".to_string()))
        );
        assert_eq!(
            set.unlist_named(&scratch),
            Err(EnableError::UnknownSet(scratch.clone()))
        );
        assert!(set.sets().iter().all(|meta| meta.listed), "nothing moved");
    }

    #[test]
    fn un_solo_restores_only_sets_that_are_still_listed() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[loaded("a", 10, true, &["x"]), loaded("b", 20, true, &["y"])],
        );
        set.solo(1);
        assert!(set.set_listed(2, false));
        assert!(!set.solo(1), "un-solo");
        assert!(set.sets()[1].enabled);
        assert!(!set.sets()[2].listed, "the snapshot does not list it");
        assert!(!set.sets()[2].enabled, "and so does not enable it");
    }

    #[test]
    fn unlisting_the_soloed_set_ends_the_solo() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[loaded("a", 10, true, &["x"]), loaded("b", 20, true, &["y"])],
        );
        set.solo(1);
        assert!(set.set_listed(1, false));
        assert_eq!(set.soloed(), None);
        assert!(!set.sets()[1].enabled);
        assert!(
            set.sets()[2].enabled,
            "the rest come back from the snapshot"
        );
    }

    /// `R` does not change the listed state, and enables an autoload set
    /// only if it is listed now.
    #[test]
    fn reset_keeps_the_listed_state_and_autoloads_only_listed_sets() {
        let mut set = ActiveFilters::with_sets(
            None,
            &[
                loaded("a", 10, true, &["x"]),
                unlisted(loaded("b", 20, true, &["y"])),
            ],
        );
        set.set_listed(1, false);
        set.set_listed(2, true);
        set.reset();
        assert!(!set.sets()[1].listed, "still unlisted");
        assert!(!set.sets()[1].enabled, "so autoload does not apply");
        assert!(set.sets()[2].listed, "still listed");
        assert!(set.sets()[2].enabled, "listed now, so autoload applies");
    }

    #[test]
    fn the_definitions_set_can_be_unlisted() {
        let set = ActiveFilters::with_sets(
            None,
            &[unlisted(builtin_override(
                crate::filtersets::DEFAULT_PRIORITY,
                true,
            ))],
        );
        assert_eq!(set.sets()[1].origin, Origin::BuiltIn);
        assert!(!set.sets()[1].listed);
        assert!(!set.sets()[1].enabled);
        assert!(!set.needs_kinds());

        let mut set = ActiveFilters::new();
        assert!(set.set_listed(1, false), "and unlisted in a session");
        assert!(!set.sets()[1].listed);
    }

    // ---- descriptions (#284) -----------------------------------------------

    #[test]
    fn the_definitions_set_has_a_builtin_description_the_table_can_override() {
        let set = ActiveFilters::new();
        assert_eq!(
            set.sets()[1].description.as_deref(),
            Some(DEFINITIONS_DESCRIPTION)
        );

        let mut table = builtin_override(crate::filtersets::DEFAULT_PRIORITY, false);
        table.description = Some("mine".into());
        let set = ActiveFilters::with_sets(None, &[table]);
        assert_eq!(set.sets()[1].description.as_deref(), Some("mine"));
    }

    #[test]
    fn a_file_set_carries_its_description() {
        let mut a = loaded("a", 50, false, &["x"]);
        a.description = Some("logs".into());
        let set = ActiveFilters::with_sets(None, &[a, loaded("b", 60, false, &["y"])]);
        let described = |name: &str| {
            let meta = set.sets().iter().find(|meta| meta.name == name);
            meta.expect("known").description.clone()
        };
        assert_eq!(described("a").as_deref(), Some("logs"));
        assert_eq!(described("b"), None);
    }
}
