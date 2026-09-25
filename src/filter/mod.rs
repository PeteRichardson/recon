//! Filters decide how each line of the viewed file is presented.
//!
//! A filter set describes a *log format* rather than a document, so it outlives
//! any one file. Matching is by regular expression, the same as search, so
//! `^foo` anchors to the start of a line.

//! Three files, one per job the module does: this one holds the palette
//! and the per-line model — [`ActiveFilters`], [`Predicate`], [`Verdict`] —
//! `sets` holds the set, solo, reset and adopt state machine, and `matcher`
//! the `Send` snapshot a scan thread matches with. Each adds its own
//! `impl ActiveFilters` block; everything is re-exported here, so callers
//! name `crate::filter::X` whichever file `X` lives in.

mod matcher;
mod sets;

pub use matcher::{Matcher, Owner};
pub use sets::{
    DEFINITIONS_DESCRIPTION, DEFINITIONS_SET, EnableError, FilterSet, LoadedFilter, LoadedSet,
    Origin, is_builtin_name,
};

use crate::syntax::{Kind, KindSet};
use ratatui::style::{Color, Modifier, Style};
use regex::{Regex, RegexSet};
use sets::Solo;

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

/// Every enabled flag in an [`ActiveFilters`], captured so it can be restored.
///
/// Opaque on purpose: it is a token to hand back to
/// [`ActiveFilters::apply_enabled_flags`], not a structure to read or build.
/// Positions in it are meaningless without the set it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledFlags {
    filters: Vec<bool>,
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
    use super::*;

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
}
