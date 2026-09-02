//! Filters decide how each line of the viewed file is presented.
//!
//! A filter set describes a *log format* rather than a document, so it outlives
//! any one file. Matching is by regular expression, the same as search, so
//! `^foo` anchors to the start of a line.

use ratatui::style::{Color, Modifier, Style};
use regex::{Regex, RegexSet};

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
const DEFAULT_PALETTE: [Color; 6] = [
    Color::Indexed(220), // gold        #ffd700
    Color::Indexed(51),  // cyan        #00ffff
    Color::Indexed(46),  // pure green  #00ff00
    Color::Indexed(201), // magenta     #ff00ff
    Color::Indexed(105), // periwinkle  #8787ff
    Color::Indexed(196), // red         #ff0000
];

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

pub(crate) const DIM_STYLE: Style = Style::new()
    .fg(Color::Indexed(DIM_GREY))
    .add_modifier(Modifier::DIM);

/// The colour reserved for the live search.
///
/// Deliberately outside `DEFAULT_PALETTE`: drawing from it would make the search's
/// colour depend on how many filters happen to exist, so it would shift as
/// filters come and go. A fixed colour gives the user one rule — white means
/// what you just typed.
///
/// White *and* bold, for the reason `pane_block` gives about focus: a single
/// visual channel fails on a theme with weak contrast and in a terminal with
/// no colour at all.
pub(crate) const SEARCH_STYLE: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);

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
    /// Matched the live search rather than a numbered filter.
    ///
    /// Carries no index: the search lives in its own slot, precisely so that
    /// setting and clearing it cannot renumber the filters the user built.
    Searched,
    /// Matched no including filter.
    Unmatched,
    /// Removed by an excluding filter.
    Excluded,
}

#[derive(Debug)]
pub struct Filter {
    pub pattern: Regex,
    pub sense: Sense,
    pub enabled: bool,
    pub style: Style,
}

/// Every enabled flag in an [`ActiveFilters`], captured so it can be restored.
///
/// Opaque on purpose: it is a token to hand back to
/// [`ActiveFilters::apply_enabled_flags`], not a structure to read or build.
/// Positions in it are meaningless without the set it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledFlags {
    filters: Vec<bool>,
    search: Option<bool>,
}

/// The bitset width. Up to 64 patterns in total, the search included; past
/// this the navigator's file matching switches off rather than shifting out
/// of range.
const MAX_PATTERNS: usize = 64;

/// Which filter selected a file, for its colour in the navigator.
///
/// `Search` outranks every numbered filter, as it does per line in `verdict`.
/// Among numbered filters the lowest index wins — the view's "first matching
/// filter wins", applied per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Search,
    Filter(usize),
}

impl Owner {
    /// Lower is higher precedence. `Search` first, then filter order.
    #[must_use]
    pub fn rank(self) -> usize {
        match self {
            Self::Search => 0,
            Self::Filter(index) => index + 1,
        }
    }
}

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
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Include`; plus the
    /// search's bit when it is present and enabled.
    selects: u64,
    /// Bit `i` set: `filters[i]` is enabled and `Sense::Exclude`.
    exclude: u64,
    /// The search's bit alone, or zero — so `owner` can rank it first.
    search: u64,
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

    /// Whether a line with these hits selects its file. A line selects when
    /// an enabled `Include` filter or the search hits it, and no enabled
    /// `Exclude` does.
    #[must_use]
    pub fn selects(&self, bits: u64) -> bool {
        bits & self.selects != 0 && bits & self.exclude == 0
    }

    /// Which filter selected a line with these hits, if any.
    #[must_use]
    pub fn owner(&self, bits: u64) -> Option<Owner> {
        if !self.selects(bits) {
            return None;
        }
        if bits & self.search != 0 {
            return Some(Owner::Search);
        }
        Some(Owner::Filter(
            (bits & self.selects).trailing_zeros() as usize
        ))
    }

    /// `(selects, exclude)`, for the caller that wants to know whether a
    /// toggle changed anything a scan cares about.
    #[must_use]
    pub fn masks(&self) -> (u64, u64) {
        (self.selects, self.exclude)
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

#[derive(Debug, Default)]
pub struct ActiveFilters {
    /// Where filter colours come from. Whole-list replacement, never a merge —
    /// see [`crate::config::FiltersConfig`] for why.
    palette: Palette,
    filters: Vec<Filter>,
    /// The live search: at most one, replaced by each `/`, and never an
    /// element of `filters`.
    ///
    /// `Verdict::Included(usize)` is a *position* in `filters` — see
    /// `remove`'s doc comment. Were the search stored there, every `/` and
    /// every `Esc` would renumber the user's filters as a side effect of
    /// typing a search.
    search: Option<Filter>,
    /// Enabled flags captured by `disable_all_remembering`, awaiting a restore.
    ///
    /// Held separately from the filters so that a filter removed in the
    /// meantime simply drops out of the restore rather than resurrecting.
    remembered: Option<Vec<bool>>,
    /// The search slot's enabled flag, captured alongside `remembered`.
    remembered_search: Option<bool>,
    /// Every pattern in one automaton: `filters` in order, then the search.
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

impl ActiveFilters {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_palette(palette: Vec<Color>) -> Self {
        Self {
            palette: Palette::new(palette),
            ..Self::default()
        }
    }

    /// Whether there are any *numbered* filters. The live search is not one
    /// of these — see `row_count`, which counts it and is what the pane
    /// sizes itself against.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// How many *numbered* filters there are. The live search is not one of
    /// these — see `row_count`, which counts it and is what the pane sizes
    /// itself against.
    #[must_use]
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    #[must_use]
    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    /// The colour the next filter added will take.
    fn next_style(&self) -> Style {
        Style::default().fg(self.palette.colour(self.filters.len()))
    }

    /// Add an including filter, colouring it distinctly from its
    /// predecessors. A pattern that will not compile is rejected and the set
    /// left untouched.
    pub fn add(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let compiled = Regex::new(pattern)?;
        let style = self.next_style();
        self.filters.push(Filter {
            pattern: compiled,
            sense: Sense::Include,
            enabled: true,
            style,
        });
        self.recompile();
        // A pending capture describes a set that no longer exists. Keeping it
        // would strand it: `!` would see an enabled filter, try to capture,
        // find one already pending, and do nothing at all — forever.
        self.forget_capture();
        Ok(())
    }

    /// Add an excluding filter: its matches are removed from view entirely,
    /// in both display modes.
    ///
    /// Excluding filters carry no colour, since a line they match is never
    /// rendered.
    pub fn add_excluding(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.filters.push(Filter {
            pattern,
            sense: Sense::Exclude,
            enabled: true,
            style: Style::default(),
        });
        self.recompile();
        // A pending capture describes a set that no longer exists. Keeping it
        // would strand it: `!` would see an enabled filter, try to capture,
        // find one already pending, and do nothing at all — forever.
        self.forget_capture();
        Ok(())
    }

    /// Set the live search, replacing any previous one.
    ///
    /// One search at a time, like vim's search register: a second `/` is a
    /// new question, not another filter.
    pub fn set_search(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.search = Some(Filter {
            pattern,
            sense: Sense::Include,
            enabled: true,
            style: SEARCH_STYLE,
        });
        self.recompile();
        self.forget_capture();
        Ok(())
    }

    /// Drop the live search. A no-op when there is none.
    ///
    /// Reports whether there was a search to drop, the same shape as
    /// `promote_search`: a caller can skip the `refresh_view` it would
    /// otherwise pay for on every press — see the `Esc` binding.
    pub fn clear_search(&mut self) -> bool {
        let had_search = self.search.take().is_some();
        self.recompile();
        self.forget_capture();
        had_search
    }

    #[must_use]
    pub fn search(&self) -> Option<&Filter> {
        self.search.as_ref()
    }

    /// Enable or disable the search, reporting whether there was one.
    pub fn search_set_enabled(&mut self, enabled: bool) -> bool {
        match self.search.as_mut() {
            Some(search) => {
                search.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Rows the filter pane draws: one per numbered filter, plus the search
    /// row when a search exists.
    ///
    /// Distinct from `len`, which counts only the numbered filters and is
    /// what `Verdict::Included` indexes into. The pane needs the larger
    /// number; nothing else does.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.filters.len() + usize::from(self.search.is_some())
    }

    /// Move the live search into the numbered set and free the slot.
    ///
    /// This is the probe-and-keep loop `p` exists for: `/` a pattern, look at
    /// what it catches, `p` to keep it, `/` again — building a set worth
    /// saving without retyping anything.
    ///
    /// The enabled state is carried across rather than forced to `true`, so a
    /// search the user had toggled off is not silently switched back on.
    /// Reports whether there was a search to promote.
    pub fn promote_search(&mut self) -> bool {
        let Some(mut search) = self.search.take() else {
            return false;
        };
        search.style = self.next_style();
        self.filters.push(search);
        // The pattern moved from the search slot to the numbered set, which
        // changes the compiled order even though the pattern list has not.
        self.recompile();
        self.forget_capture();
        true
    }

    /// Drop a pending `!` capture, both halves together.
    ///
    /// A capture describes a set that no longer exists once the set changes.
    /// Keeping it would strand it — see `add`. Both fields go, always:
    /// dropping only one leaves the capture half-valid, which is worse than
    /// dropping neither.
    fn forget_capture(&mut self) {
        self.remembered = None;
        self.remembered_search = None;
    }

    /// Whether any enabled filter removes lines.
    #[must_use]
    pub fn any_excluding(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense == Sense::Exclude)
    }

    /// Enable or disable every filter at once, for the `!` toggle.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            filter.enabled = enabled;
        }
        if let Some(search) = self.search.as_mut() {
            search.enabled = enabled;
        }
    }

    /// Remove the filter at `index`, reporting whether it existed.
    ///
    /// Indices are positional, so this renumbers every later filter. Any
    /// cached `Verdict::Included` is invalid afterwards — callers must
    /// re-evaluate rather than patch.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.filters.len() {
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
        match self.filters.get_mut(index) {
            Some(filter) => {
                filter.pattern = compiled;
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
    /// A second call before a restore is ignored: the flags at that point are
    /// the ones this method just cleared, so capturing them again would
    /// overwrite the real state with all-disabled and lose it for good.
    pub fn disable_all_remembering(&mut self) {
        if self.remembered.is_some() {
            return;
        }
        self.remembered = Some(self.filters.iter().map(|f| f.enabled).collect());
        self.remembered_search = self.search.as_ref().map(|search| search.enabled);
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
        // Taken unconditionally, so a capture made while no search existed
        // does not linger and get applied to an unrelated later search.
        let remembered_search = self.remembered_search.take();
        if let (Some(search), Some(was_enabled)) = (self.search.as_mut(), remembered_search) {
            search.enabled = was_enabled;
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
            search: self.search.as_ref().map(|search| search.enabled),
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
        if let (Some(search), Some(was_enabled)) = (self.search.as_mut(), flags.search) {
            search.enabled = was_enabled;
        }
    }

    #[must_use]
    pub fn any_enabled(&self) -> bool {
        self.filters.iter().any(|filter| filter.enabled)
            || self.search.as_ref().is_some_and(|search| search.enabled)
    }

    /// Decide how `line` should be presented.
    ///
    /// A set with no enabled including filters leaves every line `Unmatched`,
    /// so an empty or fully disabled set renders an ordinary, undimmed file
    /// rather than a wholly dimmed one. The first matching filter wins, which
    /// is what makes the set's order meaningful.
    /// One pass over the line, not one per filter. See `compiled`.
    #[must_use]
    pub fn verdict(&self, line: &str) -> Verdict {
        let Some(set) = self.compiled.as_ref().filter(|set| self.in_step(set)) else {
            return self.verdict_by_scanning(line);
        };
        let matched = set.matches(line);
        let matched = |index: usize| matched.matched(index);

        // Exclusion is applied after inclusion and overrides it, so a line an
        // including filter selected is still removed if an excluding filter
        // also matches it.
        if self.filters.iter().enumerate().any(|(index, filter)| {
            filter.enabled && filter.sense == Sense::Exclude && matched(index)
        }) {
            return Verdict::Excluded;
        }

        // The live search outranks the numbered filters: the user's attention
        // is on the pattern they just typed, so it wins the colour on a line
        // that several things match. It is compiled last, so its index is the
        // one past the numbered filters.
        if let Some(search) = &self.search
            && search.enabled
            && matched(self.filters.len())
        {
            return Verdict::Searched;
        }

        self.filters
            .iter()
            .enumerate()
            .find(|&(index, filter)| {
                filter.enabled && filter.sense != Sense::Exclude && matched(index)
            })
            .map_or(Verdict::Unmatched, |(index, _)| Verdict::Included(index))
    }

    /// The original per-filter scan, kept as the fallback for a set that would
    /// not compile into a `RegexSet`.
    ///
    /// Also what runs if `compiled` were ever out of step with `filters` — see
    /// `in_step`. That is a bug rather than a state to support, but indexing a
    /// short `SetMatches` panics, and taking down a full-screen TUI is a much
    /// worse way to report it than being slow.
    fn verdict_by_scanning(&self, line: &str) -> Verdict {
        if self.filters.iter().any(|filter| {
            filter.enabled && filter.sense == Sense::Exclude && filter.pattern.is_match(line)
        }) {
            return Verdict::Excluded;
        }

        if let Some(search) = &self.search
            && search.enabled
            && search.pattern.is_match(line)
        {
            return Verdict::Searched;
        }

        self.filters
            .iter()
            .enumerate()
            .find(|(_, filter)| {
                filter.enabled && filter.sense != Sense::Exclude && filter.pattern.is_match(line)
            })
            .map_or(Verdict::Unmatched, |(index, _)| Verdict::Included(index))
    }

    /// Whether the compiled set still describes this filter set.
    fn in_step(&self, set: &RegexSet) -> bool {
        set.len() == self.filters.len() + usize::from(self.search.is_some())
    }

    /// Rebuild the compiled set. Called by every method that adds, removes or
    /// replaces a pattern — and by none that only flips an `enabled` flag.
    fn recompile(&mut self) {
        let patterns = self
            .filters
            .iter()
            .chain(self.search.as_ref())
            .map(|filter| filter.pattern.as_str());
        self.compiled = RegexSet::new(patterns).ok();
    }

    /// The snapshot a scan thread matches with, or `None` when there is no
    /// scan to run.
    ///
    /// `None` when nothing selects — no enabled `Include` and no enabled
    /// search, which is the same "nothing to match against" guard `Document`
    /// applies for #36 — and when the pattern count exceeds the bitset width.
    #[must_use]
    pub fn matcher(&self) -> Option<Matcher> {
        let set = self.compiled.as_ref().filter(|set| self.in_step(set))?;
        if set.len() > MAX_PATTERNS {
            return None;
        }
        let mut selects = 0u64;
        let mut exclude = 0u64;
        for (index, filter) in self.filters.iter().enumerate() {
            if !filter.enabled {
                continue;
            }
            match filter.sense {
                Sense::Include => selects |= 1 << index,
                Sense::Exclude => exclude |= 1 << index,
                Sense::Context => {}
            }
        }
        let mut search = 0u64;
        if self.search.as_ref().is_some_and(|search| search.enabled) {
            search = 1 << self.filters.len();
            selects |= search;
        }
        if selects == 0 {
            return None;
        }
        Some(Matcher {
            set: set.clone(),
            selects,
            exclude,
            search,
        })
    }

    /// Every pattern's source, in compiled order, search last. What a scan
    /// cache is keyed on: a change here shifts bit positions, so cached
    /// bitsets mean something else. Sense and enabled are deliberately not
    /// part of it — they are masks over the same bits.
    #[must_use]
    pub fn pattern_key(&self) -> Vec<String> {
        self.filters
            .iter()
            .chain(self.search.as_ref())
            .map(|filter| filter.pattern.as_str().to_string())
            .collect()
    }

    /// Whether anything at all is marking lines — a numbered including filter,
    /// or the live search.
    ///
    /// Drives hiding (including the `Ctrl-H` guard in `Document`) and `n`/`N`.
    /// Public because `Document` caches it at `evaluate` time.
    #[must_use]
    pub fn any_including(&self) -> bool {
        self.any_numbered_including() || self.search.as_ref().is_some_and(|search| search.enabled)
    }

    /// Whether a *numbered* including filter is enabled. Drives dimming alone.
    ///
    /// Dimming is a contrast mechanism: unmatched lines recede so that
    /// coloured matches stand out, and its value scales with how many things
    /// are being told apart. A search on its own is one thing, and its hits
    /// already carry the span highlight — so dimming the rest of the file buys
    /// nothing and costs the readability of the context the search was run in
    /// order to reach.
    ///
    /// The consequence is deliberate and is the one place dimming stops being
    /// a strict preview of hiding: after a bare `/foo`, nothing is grey and
    /// `Ctrl-H` still hides plenty. A user pressing a key that means "hide
    /// unmatched" is not surprised to get it.
    fn any_numbered_including(&self) -> bool {
        self.filters
            .iter()
            .any(|filter| filter.enabled && filter.sense != Sense::Exclude)
    }

    /// The style to render a line with, or `None` to leave it alone.
    ///
    /// `Unmatched` dims only when a *numbered* including filter is active. The
    /// live search does not trigger dimming on its own; see `any_numbered_including`
    /// for why.
    #[must_use]
    pub fn style_for(&self, verdict: Verdict) -> Option<Style> {
        match verdict {
            Verdict::Included(index) => self.filters.get(index).map(|f| f.style),
            Verdict::Searched => Some(SEARCH_STYLE),
            Verdict::Unmatched if self.any_numbered_including() => Some(DIM_STYLE),
            Verdict::Unmatched | Verdict::Excluded => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(patterns: &[&str]) -> ActiveFilters {
        let mut set = ActiveFilters::new();
        for pattern in patterns {
            set.add(pattern).expect("valid pattern");
        }
        set
    }

    // ---- the matcher snapshot --------------------------------------------

    /// The invariant the navigator rests on, stated the way the spec states
    /// it: a line selects its file when an enabled `Include` filter or the
    /// search hits it and no enabled `Exclude` does. Deliberately *not*
    /// derived from `verdict`'s index — that is a colouring rule (first match
    /// wins), and a context filter can win the colour of a line an include
    /// filter also hit. Selecting and colouring are different questions.
    #[test]
    fn the_matcher_agrees_with_the_spec_on_what_selects_a_file() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        set.add_excluding("noise").expect("valid pattern");
        set.set_search("gamma").expect("valid pattern");
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
                set.filters()
                    .iter()
                    .any(|f| f.enabled && f.sense == sense && f.pattern.is_match(line))
            };
            let searched = set
                .search()
                .is_some_and(|s| s.enabled && s.pattern.is_match(line));
            let expected = (hit(Sense::Include) || searched) && !hit(Sense::Exclude);

            assert_eq!(
                matcher.selects(matcher.bits(line)),
                expected,
                "matcher and the spec disagree on {line:?}"
            );
            // A selected line is always one the view shows.
            if expected {
                assert!(
                    matches!(set.verdict(line), Verdict::Included(_) | Verdict::Searched),
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
    fn the_owner_is_the_lowest_selecting_filter_and_search_outranks_them() {
        let mut set = set_with(&["alpha", "beta", "delta"]);
        set.toggle_context(1);
        set.set_search("gamma").expect("valid pattern");
        let matcher = set.matcher().expect("something selects");

        assert_eq!(matcher.owner(matcher.bits("beta")), None);
        assert_eq!(
            matcher.owner(matcher.bits("beta delta")),
            Some(Owner::Filter(2))
        );
        assert_eq!(
            set.verdict("beta delta"),
            Verdict::Included(1),
            "the line is still beta's"
        );
        assert_eq!(
            matcher.owner(matcher.bits("alpha delta")),
            Some(Owner::Filter(0))
        );
        assert_eq!(
            matcher.owner(matcher.bits("alpha gamma")),
            Some(Owner::Search)
        );
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

        let mut search_only = ActiveFilters::new();
        search_only.set_search("gamma").expect("valid pattern");
        assert!(
            search_only.matcher().is_some(),
            "the search selects on its own"
        );
    }

    /// 64 is the width of the bitset; the 65th pattern switches the feature off
    /// rather than wrapping a shift.
    #[test]
    fn no_matcher_past_sixty_four_patterns() {
        let mut set = ActiveFilters::new();
        for i in 0..64 {
            set.add(&format!("p{i}")).expect("valid pattern");
        }
        assert!(set.matcher().is_some());

        set.add("p64").expect("valid pattern");
        assert!(set.matcher().is_none());
    }

    #[test]
    fn the_pattern_key_lists_every_pattern_with_the_search_last() {
        let mut set = set_with(&["alpha", "beta"]);
        set.set_search("gamma").expect("valid pattern");

        assert_eq!(set.pattern_key(), vec!["alpha", "beta", "gamma"]);

        set.toggle_context(0);
        assert_eq!(
            set.pattern_key(),
            vec!["alpha", "beta", "gamma"],
            "sense is not part of the key"
        );
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
        for (position, colour) in DEFAULT_PALETTE.iter().enumerate() {
            assert!(
                rgb_of(*colour).is_some(),
                "palette entry {position} is {colour:?}, whose appearance the \
                 terminal theme decides; use an indexed or RGB colour"
            );
        }
    }

    /// The distance below which two filter colours read as "the same one" at a
    /// glance. Yellow-vs-green was the complaint; this is what stops any pair
    /// from drifting back into it.
    const MIN_SEPARATION: f64 = 150.0;

    #[test]
    fn every_default_palette_pair_is_visibly_distinct() {
        for (i, first) in DEFAULT_PALETTE.iter().enumerate() {
            for (j, second) in DEFAULT_PALETTE.iter().enumerate().skip(i + 1) {
                let (a, b) = (
                    rgb_of(*first).expect("palette colours have fixed values"),
                    rgb_of(*second).expect("palette colours have fixed values"),
                );
                let apart = distance(a, b);
                assert!(
                    apart >= MIN_SEPARATION,
                    "filters {} and {} are only {apart:.0} apart ({first:?} {a:?} vs \
                     {second:?} {b:?}); {MIN_SEPARATION} is the minimum",
                    i + 1,
                    j + 1,
                );
            }
        }
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

        assert_eq!(set.verdict("anything"), Verdict::Unmatched);
        assert!(set.is_empty());
    }

    #[test]
    fn a_matching_line_is_included_with_its_filter_index() {
        let set = set_with(&["foo", "bar"]);

        assert_eq!(set.verdict("a bar line"), Verdict::Included(1));
    }

    #[test]
    fn a_non_matching_line_is_unmatched() {
        let set = set_with(&["foo"]);

        assert_eq!(set.verdict("nothing here"), Verdict::Unmatched);
    }

    /// Order in the set decides the colour, so the first match wins.
    #[test]
    fn the_first_matching_filter_wins() {
        let set = set_with(&["foo", "foo.*bar"]);

        assert_eq!(set.verdict("foo and bar"), Verdict::Included(0));
    }

    // ---- the third sense ------------------------------------------------

    /// A context filter shows its lines exactly as an include filter does.
    #[test]
    fn a_context_filter_includes_its_lines() {
        let mut set = set_with(&["foo"]);
        assert!(set.toggle_context(0));

        assert_eq!(set.filters()[0].sense, Sense::Context);
        assert_eq!(set.verdict("foo"), Verdict::Included(0));
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
        assert_eq!(set.filters()[1].pattern.as_str(), "bar");
        assert_eq!(set.filters()[1].style, before);
        assert_eq!(set.verdict("bar"), Verdict::Included(1));
    }

    /// An exclude filter is never context, and an index off the end is not a filter.
    #[test]
    fn toggle_context_leaves_excludes_and_missing_indices_alone() {
        let mut set = set_with(&["foo"]);
        set.add_excluding("noise").expect("valid pattern");

        assert!(!set.toggle_context(1));
        assert_eq!(set.filters()[1].sense, Sense::Exclude);
        assert!(!set.toggle_context(7));
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
        assert_eq!(set.verdict("bar"), Verdict::Unmatched);

        set.add("bar").expect("valid pattern");

        assert_eq!(set.verdict("bar"), Verdict::Included(1));
    }

    #[test]
    fn adding_an_excluding_filter_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);
        assert_eq!(set.verdict("foo"), Verdict::Included(0));

        set.add_excluding("foo").expect("valid pattern");

        assert_eq!(set.verdict("foo"), Verdict::Excluded);
    }

    #[test]
    fn removing_a_filter_is_visible_to_verdict() {
        let mut set = set_with(&["foo", "bar"]);

        assert!(set.remove(0));

        // Not merely "no longer matches foo": everything renumbers, so a
        // stale set would answer `Included(1)` for a line it should call
        // `Included(0)`.
        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
        assert_eq!(set.verdict("bar"), Verdict::Included(0));
    }

    #[test]
    fn editing_a_pattern_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);

        assert!(set.set_pattern(0, "bar").expect("valid pattern"));

        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
        assert_eq!(set.verdict("bar"), Verdict::Included(0));
    }

    #[test]
    fn a_rejected_pattern_leaves_the_compiled_set_alone() {
        let mut set = set_with(&["foo"]);

        assert!(set.set_pattern(0, "[").is_err());

        assert_eq!(
            set.verdict("foo"),
            Verdict::Included(0),
            "a pattern that would not compile disturbed the set it was rejected from"
        );
    }

    #[test]
    fn setting_and_clearing_the_search_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);

        set.set_search("bar").expect("valid pattern");
        assert_eq!(set.verdict("bar"), Verdict::Searched);

        set.set_search("baz").expect("valid pattern");
        assert_eq!(set.verdict("bar"), Verdict::Unmatched);
        assert_eq!(set.verdict("baz"), Verdict::Searched);

        assert!(set.clear_search());
        assert_eq!(set.verdict("baz"), Verdict::Unmatched);
    }

    #[test]
    fn promoting_the_search_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);
        set.set_search("bar").expect("valid pattern");

        assert!(set.promote_search());

        // It stops being the search and becomes filter 1.
        assert_eq!(set.verdict("bar"), Verdict::Included(1));
    }

    /// Toggling `enabled` must *not* need a recompile — it is the frequent
    /// operation, and `space`, `d` and `!` all drive it. Pinned so a future
    /// change cannot quietly move the enabled flag into the compiled set.
    #[test]
    fn toggling_enabled_is_visible_to_verdict() {
        let mut set = set_with(&["foo"]);

        assert!(set.set_enabled(0, false));
        assert_eq!(set.verdict("foo"), Verdict::Unmatched);

        assert!(set.set_enabled(0, true));
        assert_eq!(set.verdict("foo"), Verdict::Included(0));
    }

    #[test]
    fn patterns_are_regular_expressions() {
        let set = set_with(&[r"^\d+ms$"]);

        assert_eq!(set.verdict("250ms"), Verdict::Included(0));
        assert_eq!(set.verdict("took 250ms"), Verdict::Unmatched);
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

        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
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

        assert_eq!(set.verdict("foo"), Verdict::Included(0));
    }

    /// A set whose filters are all disabled behaves like an empty one: an
    /// undimmed file, not a fully dimmed one.
    #[test]
    fn a_fully_disabled_set_leaves_lines_unmatched() {
        let mut set = set_with(&["foo"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("bar"), Verdict::Unmatched);
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

        assert_eq!(set.verdict("a heartbeat line"), Verdict::Excluded);
    }

    /// Excluding filters run after including ones, so exclusion wins even on a
    /// line an including filter selected.
    #[test]
    fn exclusion_beats_inclusion_on_the_same_line() {
        let mut set = set_with(&["foo"]);
        set.add_excluding("noisy").expect("valid pattern");

        assert_eq!(set.verdict("foo but noisy"), Verdict::Excluded);
        assert_eq!(set.verdict("foo alone"), Verdict::Included(0));
    }

    /// With only excluding filters, unmatched lines stay ordinary — there is
    /// nothing to dim against.
    #[test]
    fn excluding_filters_alone_do_not_dim() {
        let set = set_excluding(&["heartbeat"]);

        assert_eq!(set.verdict("something else"), Verdict::Unmatched);
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    #[test]
    fn a_disabled_excluding_filter_excludes_nothing() {
        let mut set = set_excluding(&["heartbeat"]);
        set.set_all_enabled(false);

        assert_eq!(set.verdict("a heartbeat line"), Verdict::Unmatched);
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
        assert_eq!(set.verdict("bar line"), Verdict::Included(0));
    }

    /// Indices are positional, so removing a filter renumbers the ones after
    /// it. Any verdict cached against the old numbering is now wrong, which is
    /// why callers must re-evaluate rather than patch.
    #[test]
    fn removing_a_filter_renumbers_the_rest() {
        let mut set = set_with(&["foo", "bar"]);
        assert_eq!(set.verdict("bar line"), Verdict::Included(1));

        set.remove(0);

        assert_eq!(set.verdict("bar line"), Verdict::Included(0));
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

        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
        assert_eq!(set.verdict("bar line"), Verdict::Included(1));
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

        assert_eq!(set.toggle_enabled(5), None);
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
        assert_eq!(set.verdict("foo"), Verdict::Unmatched);
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

    #[test]
    fn a_search_matches_like_an_including_filter() {
        let mut set = ActiveFilters::new();
        set.set_search("timeout").expect("valid pattern");

        assert_eq!(set.verdict("conn timeout"), Verdict::Searched);
        assert_eq!(set.verdict("all fine"), Verdict::Unmatched);
    }

    /// The user's attention is on the pattern they just typed, so it wins the
    /// colour on a line a numbered filter also matches.
    #[test]
    fn the_search_outranks_a_numbered_filter() {
        let mut set = set_with(&["ERROR"]);
        set.set_search("timeout").expect("valid pattern");

        assert_eq!(set.verdict("ERROR timeout on socket"), Verdict::Searched);
        assert_eq!(set.verdict("ERROR disk full"), Verdict::Included(0));
    }

    /// Exclusion runs first and beats everything, so search inherits the rule
    /// rather than needing one of its own.
    #[test]
    fn exclusion_beats_the_search() {
        let mut set = ActiveFilters::new();
        set.add_excluding("heartbeat").expect("valid pattern");
        set.set_search("timeout").expect("valid pattern");

        assert_eq!(set.verdict("heartbeat timeout"), Verdict::Excluded);
    }

    /// One search at a time, like vim's search register: a second `/` replaces
    /// the first rather than stacking another filter.
    #[test]
    fn setting_a_search_replaces_the_previous_one() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        set.set_search("bar").expect("valid pattern");

        assert_eq!(set.verdict("bar line"), Verdict::Searched);
        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
    }

    /// The whole point of the separate slot: `/` and `Esc` must never renumber
    /// the filters the user built, because `Verdict::Included` is a position.
    #[test]
    fn the_search_does_not_occupy_a_numbered_slot() {
        let mut set = set_with(&["alpha", "beta"]);
        set.set_search("gamma").expect("valid pattern");

        assert_eq!(set.len(), 2, "the search must not join the numbered set");
        assert_eq!(set.verdict("beta line"), Verdict::Included(1));

        set.clear_search();
        assert_eq!(set.verdict("beta line"), Verdict::Included(1));
    }

    #[test]
    fn clearing_a_search_removes_it() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        set.clear_search();

        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
        assert!(set.search().is_none());
    }

    #[test]
    fn an_invalid_search_pattern_is_reported_and_changes_nothing() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");

        assert!(set.set_search("[").is_err());
        assert_eq!(
            set.verdict("foo line"),
            Verdict::Searched,
            "the old search was lost"
        );
    }

    #[test]
    fn a_disabled_search_matches_nothing() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        set.set_all_enabled(false);

        assert_eq!(set.verdict("foo line"), Verdict::Unmatched);
    }

    /// The search carries a colour of its own, outside `DEFAULT_PALETTE`, so it never
    /// shifts as filters are added and removed.
    #[test]
    fn the_search_style_is_reserved_rather_than_drawn_from_the_palette() {
        assert!(
            !DEFAULT_PALETTE
                .iter()
                .any(|colour| SEARCH_STYLE.fg == Some(*colour)),
            "the search colour would move as the palette rotates"
        );
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        assert_eq!(set.style_for(Verdict::Searched), Some(SEARCH_STYLE));
    }

    /// `!` must round-trip the search slot too, or it stops meaning "back to an
    /// unfiltered view".
    #[test]
    fn disabling_all_remembers_the_search_slot() {
        let mut set = set_with(&["foo"]);
        set.set_search("bar").expect("valid pattern");

        set.disable_all_remembering();
        assert!(!set.any_enabled(), "the search kept the set enabled");
        assert_eq!(set.verdict("bar line"), Verdict::Unmatched);

        set.restore_remembered();
        assert_eq!(set.verdict("bar line"), Verdict::Searched);
    }

    /// A search that the user had deliberately toggled off must not come back on.
    #[test]
    fn restoring_does_not_switch_a_disabled_search_back_on() {
        let mut set = ActiveFilters::new();
        set.set_search("bar").expect("valid pattern");
        set.search_set_enabled(false);

        set.disable_all_remembering();
        set.restore_remembered();

        assert_eq!(set.verdict("bar line"), Verdict::Unmatched);
    }

    /// A capture describes a set that no longer exists once the search changes,
    /// exactly as it does when a filter is added — see `add`'s comment.
    #[test]
    fn changing_the_search_drops_a_pending_capture() {
        let mut set = set_with(&["foo"]);
        set.disable_all_remembering();

        set.set_search("bar").expect("valid pattern");
        assert!(!set.has_remembered());

        set.clear_search();
        assert!(!set.has_remembered());
    }

    #[test]
    fn row_count_includes_the_search_row() {
        let mut set = set_with(&["foo", "bar"]);
        assert_eq!(set.row_count(), 2);

        set.set_search("baz").expect("valid pattern");
        assert_eq!(set.row_count(), 3);
    }

    /// Dimming is a contrast mechanism, and a search on its own is one thing to
    /// see: its hits already carry the span highlight, so greying the rest of the
    /// file buys nothing and costs the readability of the context the user
    /// searched in order to reach.
    #[test]
    fn a_search_alone_does_not_dim() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");

        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    /// But it still counts as something to hide against, so `Ctrl-H` works after
    /// a bare search. This is the asymmetry the two predicates exist for.
    #[test]
    fn a_search_alone_still_counts_for_hiding() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");

        assert!(
            set.any_including(),
            "Ctrl-H would have nothing to hide against"
        );
    }

    /// Add a numbered filter and dimming switches on, because now there really
    /// are two things to tell apart. If we then disable the numbered filter,
    /// leaving only the search, dimming must stop — that's the asymmetry.
    /// This pins the difference between the two predicates directly rather than
    /// testing a case where they agree.
    #[test]
    fn a_numbered_filter_alongside_a_search_dims() {
        let mut set = set_with(&["ERROR"]);
        set.set_search("foo").expect("valid pattern");

        assert_eq!(set.style_for(Verdict::Unmatched), Some(DIM_STYLE));

        set.set_enabled(0, false);
        assert_eq!(
            set.style_for(Verdict::Unmatched),
            None,
            "search alone must not dim, even when one was present"
        );
    }

    #[test]
    fn a_disabled_search_counts_for_neither() {
        let mut set = ActiveFilters::new();
        set.set_search("foo").expect("valid pattern");
        set.search_set_enabled(false);

        assert!(!set.any_including());
        assert_eq!(set.style_for(Verdict::Unmatched), None);
    }

    /// The probe-and-keep loop: `/` to try a pattern, `p` to keep it, `/` again.
    /// Nothing is retyped.
    #[test]
    fn promoting_moves_the_search_into_the_numbered_set() {
        let mut set = set_with(&["alpha"]);
        set.set_search("beta").expect("valid pattern");

        assert!(set.promote_search());

        assert_eq!(set.len(), 2);
        assert!(
            set.search().is_none(),
            "the slot should be free for the next probe"
        );
        assert_eq!(set.verdict("beta line"), Verdict::Included(1));
    }

    /// `next_style` reads `self.filters.len()`, so it must be called before the
    /// push that grows `filters`, not after — call it after and it reads one
    /// too many and returns `DEFAULT_PALETTE[2]` instead of `DEFAULT_PALETTE[1]`. A mere
    /// inequality would not catch that swap: `DEFAULT_PALETTE[2]` is still unequal to
    /// both filter 0's colour and to `SEARCH_STYLE`. Pinning the exact colour
    /// is the only assertion that sees the off-by-one.
    #[test]
    fn a_promoted_search_takes_the_next_palette_colour() {
        let mut set = set_with(&["alpha"]);
        set.set_search("beta").expect("valid pattern");
        set.promote_search();

        assert_eq!(
            set.filters()[1].style.fg,
            Some(DEFAULT_PALETTE[1]),
            "index 1 is where the promoted filter actually lands"
        );
        assert_ne!(
            set.filters()[1].style,
            SEARCH_STYLE,
            "a promoted filter is a keeper, not the live probe"
        );
    }

    /// Promoting must not silently switch on a search the user had toggled off.
    #[test]
    fn promoting_preserves_the_enabled_state() {
        let mut set = ActiveFilters::new();
        set.set_search("beta").expect("valid pattern");
        set.search_set_enabled(false);

        set.promote_search();

        assert!(!set.filters()[0].enabled);
    }

    #[test]
    fn promoting_without_a_search_reports_failure_and_changes_nothing() {
        let mut set = set_with(&["alpha"]);

        assert!(!set.promote_search());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn promoting_drops_a_pending_capture() {
        let mut set = ActiveFilters::new();
        set.set_search("beta").expect("valid pattern");
        set.disable_all_remembering();

        set.promote_search();

        assert!(!set.has_remembered());
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
        assert_eq!(set.verdict("gamma line"), Verdict::Included(0));
        assert_eq!(set.verdict("beta line"), Verdict::Included(1));
        assert_eq!(set.filters()[0].style, colour, "the colour moved with it");
    }

    #[test]
    fn editing_a_pattern_replaces_the_old_one() {
        let mut set = set_with(&["alpha"]);

        set.set_pattern(0, "gamma").expect("valid pattern");

        assert_eq!(
            set.verdict("alpha line"),
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
        assert_eq!(set.verdict("a keepalive line"), Verdict::Excluded);
    }

    /// The same discipline `add` follows: compile first, mutate second, so a
    /// rejected pattern leaves the previous one intact and the prompt has
    /// something to stay open over.
    #[test]
    fn an_invalid_edit_is_reported_and_leaves_the_filter_untouched() {
        let mut set = set_with(&["alpha"]);

        assert!(set.set_pattern(0, "[").is_err());

        assert_eq!(set.verdict("alpha line"), Verdict::Included(0));
    }

    #[test]
    fn editing_out_of_range_reports_failure_and_changes_nothing() {
        let mut set = set_with(&["alpha"]);

        assert!(!set.set_pattern(5, "gamma").expect("valid pattern"));
        assert_eq!(set.len(), 1);
        assert_eq!(set.verdict("alpha line"), Verdict::Included(0));
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
