//! Turning a keypress into a named action.
//!
//! One table, one rule for modifiers, and one place that decides what a key
//! means. Before this, eight `match` sites each decided for themselves and
//! three different modifier idioms disagreed at the edges — which is what
//! #250 was.

use crate::toml_fmt::toml_string;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fmt::Write as _;

pub(crate) mod check;

/// A key as the table stores it: the code, and the two modifiers that carry
/// meaning.
///
/// `SHIFT` is deliberately absent. In the legacy key-reporting mode — which is
/// the only mode recon runs in, because it never pushes the keyboard
/// enhancement flags — crossterm attaches `SHIFT` exactly when the character
/// is uppercase (`char_code_to_event` in crossterm's unix parser). So for a
/// letter the modifier repeats what the `char` already says, and for
/// punctuation it is never set at all: `{` is not "shifted `[`" as far as the
/// terminal is concerned, it is the character `{`.
///
/// Keeping it would mean every table entry had to guess which of the two
/// spellings a terminal would send. That guess is what #250 was: `G` arrived
/// carrying `SHIFT`, failed an `is_empty()` guard written for keys that never
/// carry it, and fell through to a handler that answered the wrong question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) code: KeyCode,
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
}

/// Put a keypress in the table's currency.
pub(crate) fn normalise(event: KeyEvent) -> Key {
    Key {
        code: event.code,
        ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
        alt: event.modifiers.contains(KeyModifiers::ALT),
    }
}

/// Where a key is looked up, in the order `dispatch_event` already checks.
///
/// The first four are modal: while one is active it owns the whole keyboard
/// and nothing below it is consulted. The last three are the panes, chosen by
/// which one has focus.
///
/// `Ord` is derived and load-bearing: the discriminant order *is* the
/// precedence, and a test asserts it, so a variant reordered for tidiness
/// cannot silently change which handler sees a key first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Scope {
    /// A search or filter prompt is open. `handle_search_key` owns every key.
    Prompt,
    /// The help overlay is up, and any key closes it.
    ///
    /// Never constructed, and that is permanent rather than pending (#199):
    /// the overlay's dismissal has no `ActionId` — see the note on
    /// `ActionId` below — so nothing ever resolves a key *against* this
    /// scope. This variant exists only so the discriminant order matches the
    /// precedence `dispatch_event` runs: `the_modal_scopes_come_before_the_pane`
    /// checks that order, and needs this rung present to do it.
    #[allow(dead_code)]
    Help,
    /// The profile picker is open.
    Picker,
    /// The set picker is open (#284).
    Sets,
    /// Checked for every key that no modal scope claimed.
    Global,
    Nav,
    View,
    Filters,
}

impl Scope {
    /// The pane scope for the focused pane.
    ///
    /// `App`'s per-focus dispatch derives `Scope::Nav` from this at the
    /// navigator's key site (task 6, #199): the focus is the thing that
    /// decides, so reading it from the focus is the honest spelling.
    pub(crate) fn for_focus(focus: crate::widgets::Focus) -> Self {
        match focus {
            crate::widgets::Focus::Nav => Self::Nav,
            crate::widgets::Focus::View => Self::View,
            crate::widgets::Focus::Filters => Self::Filters,
        }
    }

    /// The scope's name, for a message that must say where a key was bound.
    ///
    /// An action's own name usually carries its scope — `nav.up`, `global.quit`
    /// — so a message can read the scope off the name. `hit.next` and
    /// `hit.prev` cannot: they are bare names living in both `View` and
    /// `Filters`, so a collision involving one has no scope to read and needs
    /// this instead.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Help => "help",
            Self::Picker => "picker",
            Self::Sets => "sets",
            Self::Global => "global",
            Self::Nav => "nav",
            Self::View => "view",
            Self::Filters => "filters",
        }
    }
}

/// Every action a key can name.
///
/// The string form is what a user writes in `config.toml` and what the help
/// overlay shows, so the two cannot drift: a test compares this table against
/// `help::KEYMAP` by name.
///
/// The grammar: `scope.verb[.object]` for a command (`global.quit`,
/// `filters.include`, `prompt.delete.word`), `scope.target[.direction]` for a
/// motion (`nav.goto.start`, `view.halfpage.down`). A multi-word token never
/// gets a dot of its own — `halfpage` and `linenumbers` are one token each,
/// so `nav.halfpage.down` and `view.toggle.linenumbers` are exactly three
/// segments, not four. A **bare** `verb.object` with no scope prefix —
/// `hit.next`, `hit.prev` — means the action is bound the same way in more
/// than one pane scope, so there is no single scope to prefix it with (#59
/// review: `n`/`N` bind identically in the file view and the filter pane).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ActionId {
    // Global
    GlobalQuit,
    GlobalQuitSilent,
    GlobalFocusNext,
    GlobalFocusPrev,
    GlobalFocusNav,
    GlobalFocusView,
    GlobalFocusFilters,
    GlobalHelp,
    GlobalSearch,
    GlobalSearchPromote,
    GlobalSearchWord,
    GlobalEscape,
    GlobalPeek,
    GlobalFiltersAnd,
    GlobalFiltersDisable,
    GlobalFiltersToggle,
    GlobalFileNext,
    GlobalFilePrev,
    GlobalToggleHide,
    GlobalZoomView,
    GlobalZoomFocused,
    GlobalEditorProject,
    GlobalEditorFile,
    GlobalReload,
    GlobalVisualChar,
    GlobalVisualLine,
    GlobalYank,
    GlobalPageDown,
    GlobalPageUp,
    /// Open the set picker (#284).
    GlobalSets,
    // Bound the same way in more than one pane scope — see the bare-name
    // rule in the doc comment above.
    HitNext,
    HitPrev,
    // Navigator
    NavUp,
    NavDown,
    NavParent,
    NavOpen,
    NavGotoStart,
    NavGotoEnd,
    NavHalfPageDown,
    NavHalfPageUp,
    NavPageDown,
    NavPageUp,
    NavHitNext,
    NavHitPrev,
    // File view
    ViewLeft,
    ViewRight,
    ViewUp,
    ViewDown,
    ViewWordForward,
    ViewLineStart,
    ViewLineEnd,
    ViewGotoStart,
    ViewGotoEnd,
    ViewParagraphNext,
    ViewParagraphPrev,
    ViewToggleLineNumbers,
    ViewScrollDown,
    ViewScrollUp,
    ViewHalfPageDown,
    ViewHalfPageUp,
    ViewPageDown,
    ViewPageUp,
    // Filter pane
    FiltersUp,
    FiltersDown,
    FiltersGotoStart,
    FiltersGotoEnd,
    FiltersHalfPageDown,
    FiltersHalfPageUp,
    FiltersPageDown,
    FiltersPageUp,
    FiltersToggle,
    FiltersInclude,
    FiltersExclude,
    FiltersEdit,
    FiltersDelete,
    FiltersContext,
    FiltersProfile,
    FiltersSolo,
    FiltersReset,
    FiltersSaveSet,
    // Prompt
    PromptCommit,
    PromptCancel,
    PromptLeft,
    PromptRight,
    PromptStart,
    PromptEnd,
    PromptDeleteBack,
    PromptDeleteForward,
    PromptDeleteWord,
    PromptDeleteStart,
    /// Up / Ctrl-p and Down / Ctrl-n in a `/` prompt: the earlier committed
    /// patterns, older and newer (#274). Bound in the prompt scope like the
    /// editing keys, and a no-op in a filter prompt, which keeps no history.
    PromptHistoryPrev,
    PromptHistoryNext,
    // Picker
    PickerUp,
    PickerDown,
    PickerChoose,
    PickerCancel,
    // Set picker (#284)
    SetsUp,
    SetsDown,
    SetsToggle,
    SetsApply,
    SetsCancel,
    /// `/` in the set picker (#285).
    SetsSearch,
    SetsHitNext,
    SetsHitPrev,
    // The help overlay itself has no ActionId: any key dismisses it, so
    // there is nothing to bind or rebind, and Scope::Help carries no DEFAULT
    // rows for the same reason (#59).
}

impl ActionId {
    /// The name a user writes, and the name the documentation shows.
    ///
    /// `print_keymap`'s production caller (#61): every row's action is
    /// rendered under this name, which is also the name `config.toml` must
    /// use to rebind it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::GlobalQuit => "global.quit",
            Self::GlobalQuitSilent => "global.quit.silent",
            Self::GlobalFocusNext => "global.focus.next",
            Self::GlobalFocusPrev => "global.focus.prev",
            Self::GlobalFocusNav => "global.focus.nav",
            Self::GlobalFocusView => "global.focus.view",
            Self::GlobalFocusFilters => "global.focus.filters",
            Self::GlobalHelp => "global.help",
            Self::GlobalSearch => "global.search",
            Self::GlobalSearchPromote => "global.search.promote",
            Self::GlobalSearchWord => "global.search.word",
            Self::GlobalEscape => "global.escape",
            Self::GlobalPeek => "global.peek",
            Self::GlobalFiltersAnd => "global.filters.and",
            Self::GlobalFiltersDisable => "global.filters.disable",
            Self::GlobalFiltersToggle => "global.filters.toggle",
            Self::GlobalFileNext => "global.file.next",
            Self::GlobalFilePrev => "global.file.prev",
            Self::GlobalToggleHide => "global.toggle.hide",
            Self::GlobalZoomView => "global.zoom.view",
            Self::GlobalZoomFocused => "global.zoom.focused",
            Self::GlobalEditorProject => "global.editor.project",
            Self::GlobalEditorFile => "global.editor.file",
            Self::GlobalReload => "global.reload",
            Self::GlobalVisualChar => "global.visual.char",
            Self::GlobalVisualLine => "global.visual.line",
            Self::GlobalYank => "global.yank",
            Self::GlobalPageDown => "global.page.down",
            Self::GlobalPageUp => "global.page.up",
            Self::GlobalSets => "global.sets",
            Self::HitNext => "hit.next",
            Self::HitPrev => "hit.prev",
            Self::NavUp => "nav.up",
            Self::NavDown => "nav.down",
            Self::NavParent => "nav.parent",
            Self::NavOpen => "nav.open",
            Self::NavGotoStart => "nav.goto.start",
            Self::NavGotoEnd => "nav.goto.end",
            Self::NavHalfPageDown => "nav.halfpage.down",
            Self::NavHalfPageUp => "nav.halfpage.up",
            Self::NavPageDown => "nav.page.down",
            Self::NavPageUp => "nav.page.up",
            Self::NavHitNext => "nav.hit.next",
            Self::NavHitPrev => "nav.hit.prev",
            Self::ViewLeft => "view.left",
            Self::ViewRight => "view.right",
            Self::ViewUp => "view.up",
            Self::ViewDown => "view.down",
            Self::ViewWordForward => "view.word.forward",
            Self::ViewLineStart => "view.line.start",
            Self::ViewLineEnd => "view.line.end",
            Self::ViewGotoStart => "view.goto.start",
            Self::ViewGotoEnd => "view.goto.end",
            Self::ViewParagraphNext => "view.paragraph.next",
            Self::ViewParagraphPrev => "view.paragraph.prev",
            Self::ViewToggleLineNumbers => "view.toggle.linenumbers",
            Self::ViewScrollDown => "view.scroll.down",
            Self::ViewScrollUp => "view.scroll.up",
            Self::ViewHalfPageDown => "view.halfpage.down",
            Self::ViewHalfPageUp => "view.halfpage.up",
            Self::ViewPageDown => "view.page.down",
            Self::ViewPageUp => "view.page.up",
            Self::FiltersUp => "filters.up",
            Self::FiltersDown => "filters.down",
            Self::FiltersGotoStart => "filters.goto.start",
            Self::FiltersGotoEnd => "filters.goto.end",
            Self::FiltersHalfPageDown => "filters.halfpage.down",
            Self::FiltersHalfPageUp => "filters.halfpage.up",
            Self::FiltersPageDown => "filters.page.down",
            Self::FiltersPageUp => "filters.page.up",
            Self::FiltersToggle => "filters.toggle",
            Self::FiltersInclude => "filters.include",
            Self::FiltersExclude => "filters.exclude",
            Self::FiltersEdit => "filters.edit",
            Self::FiltersDelete => "filters.delete",
            Self::FiltersContext => "filters.context",
            Self::FiltersProfile => "filters.profile",
            Self::FiltersSolo => "filters.solo",
            Self::FiltersReset => "filters.reset",
            Self::FiltersSaveSet => "filters.save.set",
            Self::PromptCommit => "prompt.commit",
            Self::PromptCancel => "prompt.cancel",
            Self::PromptLeft => "prompt.left",
            Self::PromptRight => "prompt.right",
            Self::PromptStart => "prompt.start",
            Self::PromptEnd => "prompt.end",
            Self::PromptDeleteBack => "prompt.delete.back",
            Self::PromptDeleteForward => "prompt.delete.forward",
            Self::PromptDeleteWord => "prompt.delete.word",
            Self::PromptDeleteStart => "prompt.delete.start",
            Self::PromptHistoryPrev => "prompt.history.prev",
            Self::PromptHistoryNext => "prompt.history.next",
            Self::PickerUp => "picker.up",
            Self::PickerDown => "picker.down",
            Self::PickerChoose => "picker.choose",
            Self::PickerCancel => "picker.cancel",
            Self::SetsUp => "sets.up",
            Self::SetsDown => "sets.down",
            Self::SetsToggle => "sets.toggle",
            Self::SetsApply => "sets.apply",
            Self::SetsCancel => "sets.cancel",
            Self::SetsSearch => "sets.search",
            Self::SetsHitNext => "sets.hit.next",
            Self::SetsHitPrev => "sets.hit.prev",
        }
    }
}

/// The compiled-in bindings: scope, key label, action.
///
/// The label grammar is `help::Binding`'s — `q`, `G`, `Ctrl-d`, `space`,
/// `Shift-Tab`, `PageDown`, `Home`. `Binding::codes` parses it, and plan 2b
/// lets a user write the same spellings in `config.toml`.
pub(crate) const DEFAULT: &[(Scope, &str, ActionId)] = &[
    (Scope::Global, "q", ActionId::GlobalQuit),
    (Scope::Global, "Q", ActionId::GlobalQuitSilent),
    (Scope::Global, "Tab", ActionId::GlobalFocusNext),
    (Scope::Global, "Shift-Tab", ActionId::GlobalFocusPrev),
    (Scope::Global, "e", ActionId::GlobalFocusNav),
    (Scope::Global, "t", ActionId::GlobalFocusView),
    (Scope::Global, "f", ActionId::GlobalFocusFilters),
    (Scope::Global, "?", ActionId::GlobalHelp),
    (Scope::Global, "/", ActionId::GlobalSearch),
    (Scope::Global, "p", ActionId::GlobalSearchPromote),
    (Scope::Global, "*", ActionId::GlobalSearchWord),
    (Scope::Global, "Esc", ActionId::GlobalEscape),
    (Scope::Global, "space", ActionId::GlobalPeek),
    (Scope::Global, "&", ActionId::GlobalFiltersAnd),
    (Scope::Global, "!", ActionId::GlobalFiltersDisable),
    (Scope::Global, "1-9", ActionId::GlobalFiltersToggle),
    (Scope::Global, ".", ActionId::GlobalFileNext),
    (Scope::Global, ",", ActionId::GlobalFilePrev),
    // `i`/`x`/`c`/`d`/`m`/`a`/`s` outside the filter pane, and `h`/`l`
    // inside it, are deliberately not entries here: they are not bindings,
    // only the second key of a chain (`f i`, `f x`, …) or, inside the
    // filter pane, keys the pane simply doesn't rebind. Task 8 replaces the
    // help text that currently spells this out by hand with generated text.
    (Scope::Global, "u", ActionId::GlobalToggleHide),
    (Scope::Global, "H", ActionId::GlobalToggleHide),
    (Scope::Global, "Ctrl-h", ActionId::GlobalToggleHide),
    (Scope::Global, "b", ActionId::GlobalZoomView),
    (Scope::Global, "z", ActionId::GlobalZoomFocused),
    (Scope::Global, "o", ActionId::GlobalEditorProject),
    (Scope::Global, "O", ActionId::GlobalEditorFile),
    (Scope::Global, "r", ActionId::GlobalReload),
    (Scope::Global, "v", ActionId::GlobalVisualChar),
    (Scope::Global, "V", ActionId::GlobalVisualLine),
    (Scope::Global, "y", ActionId::GlobalYank),
    (Scope::Global, "]", ActionId::GlobalPageDown),
    (Scope::Global, "[", ActionId::GlobalPageUp),
    (Scope::Global, "L", ActionId::GlobalSets),
    // `n`/`N` are scoped away from Global (#59 review): `src/lib.rs`'s
    // global arm is guarded `self.focus != Focus::Nav`, so in the navigator
    // it never fires and the key falls through to `filenav.rs`'s own
    // `repeat_search`. Binding it here unconditionally would let this table
    // resolve the navigator's `n` to the wrong action. It binds identically
    // in the file view and the filter pane instead (see those sections
    // below), hence the bare `hit.next` / `hit.prev` name — see the grammar
    // note on `ActionId`.
    (Scope::Nav, "k", ActionId::NavUp),
    (Scope::Nav, "Up", ActionId::NavUp),
    (Scope::Nav, "j", ActionId::NavDown),
    (Scope::Nav, "Down", ActionId::NavDown),
    (Scope::Nav, "h", ActionId::NavParent),
    (Scope::Nav, "Left", ActionId::NavParent),
    (Scope::Nav, "l", ActionId::NavOpen),
    (Scope::Nav, "Right", ActionId::NavOpen),
    (Scope::Nav, "Enter", ActionId::NavOpen),
    (Scope::Nav, "g", ActionId::NavGotoStart),
    (Scope::Nav, "Home", ActionId::NavGotoStart),
    (Scope::Nav, "G", ActionId::NavGotoEnd),
    (Scope::Nav, "End", ActionId::NavGotoEnd),
    (Scope::Nav, "Ctrl-d", ActionId::NavHalfPageDown),
    (Scope::Nav, "Ctrl-u", ActionId::NavHalfPageUp),
    (Scope::Nav, "PageDown", ActionId::NavPageDown),
    (Scope::Nav, "PageUp", ActionId::NavPageUp),
    (Scope::Nav, "n", ActionId::NavHitNext),
    (Scope::Nav, "N", ActionId::NavHitPrev),
    (Scope::View, "h", ActionId::ViewLeft),
    (Scope::View, "Left", ActionId::ViewLeft),
    (Scope::View, "l", ActionId::ViewRight),
    (Scope::View, "Right", ActionId::ViewRight),
    (Scope::View, "k", ActionId::ViewUp),
    (Scope::View, "Up", ActionId::ViewUp),
    (Scope::View, "j", ActionId::ViewDown),
    (Scope::View, "Down", ActionId::ViewDown),
    (Scope::View, "w", ActionId::ViewWordForward),
    (Scope::View, "0", ActionId::ViewLineStart),
    (Scope::View, "^", ActionId::ViewLineStart),
    (Scope::View, "$", ActionId::ViewLineEnd),
    (Scope::View, "g", ActionId::ViewGotoStart),
    (Scope::View, "Home", ActionId::ViewGotoStart),
    (Scope::View, "G", ActionId::ViewGotoEnd),
    (Scope::View, "End", ActionId::ViewGotoEnd),
    (Scope::View, "}", ActionId::ViewParagraphNext),
    (Scope::View, "{", ActionId::ViewParagraphPrev),
    (Scope::View, "#", ActionId::ViewToggleLineNumbers),
    (Scope::View, "Ctrl-e", ActionId::ViewScrollDown),
    (Scope::View, "Ctrl-y", ActionId::ViewScrollUp),
    (Scope::View, "Ctrl-d", ActionId::ViewHalfPageDown),
    (Scope::View, "Ctrl-u", ActionId::ViewHalfPageUp),
    (Scope::View, "Ctrl-f", ActionId::ViewPageDown),
    (Scope::View, "PageDown", ActionId::ViewPageDown),
    (Scope::View, "Ctrl-b", ActionId::ViewPageUp),
    (Scope::View, "PageUp", ActionId::ViewPageUp),
    // See the Global section above: `n`/`N` fall through to here (and to
    // the filter pane below) rather than being bound in Global.
    (Scope::View, "n", ActionId::HitNext),
    (Scope::View, "N", ActionId::HitPrev),
    (Scope::Filters, "k", ActionId::FiltersUp),
    (Scope::Filters, "Up", ActionId::FiltersUp),
    (Scope::Filters, "j", ActionId::FiltersDown),
    (Scope::Filters, "Down", ActionId::FiltersDown),
    (Scope::Filters, "g", ActionId::FiltersGotoStart),
    (Scope::Filters, "Home", ActionId::FiltersGotoStart),
    (Scope::Filters, "G", ActionId::FiltersGotoEnd),
    (Scope::Filters, "End", ActionId::FiltersGotoEnd),
    // `Ctrl-d` used to read as plain `d` and delete the selected filter,
    // which is the whole reason `FilterList` once dropped every CONTROL key
    // outright before looking at its code. Exact matching retires that
    // guard: `Ctrl-d` and `d` are distinct rows here, so the half-page
    // motion below claims the modified key and `FiltersDelete`'s plain `d`
    // row (below) never sees it.
    (Scope::Filters, "Ctrl-d", ActionId::FiltersHalfPageDown),
    (Scope::Filters, "Ctrl-u", ActionId::FiltersHalfPageUp),
    (Scope::Filters, "PageDown", ActionId::FiltersPageDown),
    (Scope::Filters, "PageUp", ActionId::FiltersPageUp),
    (Scope::Filters, "Enter", ActionId::FiltersToggle),
    (Scope::Filters, "i", ActionId::FiltersInclude),
    (Scope::Filters, "x", ActionId::FiltersExclude),
    (Scope::Filters, "c", ActionId::FiltersEdit),
    (Scope::Filters, "d", ActionId::FiltersDelete),
    (Scope::Filters, "m", ActionId::FiltersContext),
    (Scope::Filters, "a", ActionId::FiltersProfile),
    (Scope::Filters, "s", ActionId::FiltersSolo),
    (Scope::Filters, "R", ActionId::FiltersReset),
    (Scope::Filters, "S", ActionId::FiltersSaveSet),
    (Scope::Filters, "n", ActionId::HitNext),
    (Scope::Filters, "N", ActionId::HitPrev),
    (Scope::Prompt, "Enter", ActionId::PromptCommit),
    (Scope::Prompt, "Esc", ActionId::PromptCancel),
    (Scope::Prompt, "Left", ActionId::PromptLeft),
    (Scope::Prompt, "Right", ActionId::PromptRight),
    (Scope::Prompt, "Home", ActionId::PromptStart),
    (Scope::Prompt, "Ctrl-a", ActionId::PromptStart),
    (Scope::Prompt, "End", ActionId::PromptEnd),
    (Scope::Prompt, "Ctrl-e", ActionId::PromptEnd),
    (Scope::Prompt, "Backspace", ActionId::PromptDeleteBack),
    (Scope::Prompt, "Delete", ActionId::PromptDeleteForward),
    (Scope::Prompt, "Ctrl-w", ActionId::PromptDeleteWord),
    (Scope::Prompt, "Ctrl-u", ActionId::PromptDeleteStart),
    (Scope::Prompt, "Up", ActionId::PromptHistoryPrev),
    (Scope::Prompt, "Ctrl-p", ActionId::PromptHistoryPrev),
    (Scope::Prompt, "Down", ActionId::PromptHistoryNext),
    (Scope::Prompt, "Ctrl-n", ActionId::PromptHistoryNext),
    (Scope::Picker, "k", ActionId::PickerUp),
    (Scope::Picker, "Up", ActionId::PickerUp),
    (Scope::Picker, "j", ActionId::PickerDown),
    (Scope::Picker, "Down", ActionId::PickerDown),
    (Scope::Picker, "Enter", ActionId::PickerChoose),
    (Scope::Picker, "Esc", ActionId::PickerCancel),
    (Scope::Sets, "k", ActionId::SetsUp),
    (Scope::Sets, "Up", ActionId::SetsUp),
    (Scope::Sets, "j", ActionId::SetsDown),
    (Scope::Sets, "Down", ActionId::SetsDown),
    (Scope::Sets, "space", ActionId::SetsToggle),
    (Scope::Sets, "Enter", ActionId::SetsApply),
    (Scope::Sets, "Esc", ActionId::SetsCancel),
    (Scope::Sets, "/", ActionId::SetsSearch),
    (Scope::Sets, "n", ActionId::SetsHitNext),
    (Scope::Sets, "N", ActionId::SetsHitPrev),
];

/// Keys 1.0 promises to 1.1, bound to nothing.
///
/// The keymap reconciliation in #120 left six keys free, and two of them are
/// spoken for. Recording them here and in the help overlay is what makes the
/// promise real: a key 1.1 adds has to be a key 1.0 already said was taken,
/// or a user who bound it loses it in an upgrade.
pub(crate) const RESERVED: &[(&str, &str)] =
    &[("-", "the hex view (#242)"), (":", "a command palette")];

/// Which reserved keys `labels` binds, each paired with what claims it.
///
/// Compared as **keys**, not as label text. One label can name many keys, and
/// no range is spelled `-` or `:`: `'*-/'` covers `-` and `'5-<'` covers `:`,
/// so a string comparison against `RESERVED` matched neither and the warning
/// this constant exists to raise never fired. The promise is that a key 1.1
/// takes is one 1.0 already said was taken, and a user who bound a reserved
/// key inside a range would have lost it at the upgrade without ever being
/// told.
///
/// A pure function of the label list one `[keymap]` line supplies for one
/// action — never of the action's scopes — which is what makes "one
/// reserved key, one warning" true regardless of how many `DEFAULT` scopes
/// the action occupies (#242). `hit.next`/`hit.prev` hold a row in both
/// `Scope::View` and `Scope::Filters` (Ruling 12), but a `[keymap]` line
/// names an action once, with one label list; this reads that list once, so
/// a two-scope action cannot make it report a key twice. Extracted from
/// `Keymap::new`'s loop so a test can call it with the same list `Keymap::new`
/// would build for `hit.next`, without a logging harness.
///
/// Walking `RESERVED` on the outside keeps that same "once each" true of the
/// labels as well: a line naming one reserved key twice, as `['-', '*-/']`
/// does, is one warning and not two.
fn reserved_hits(labels: &[String]) -> Vec<(&'static str, &'static str)> {
    let bound: Vec<crate::help::Chord> = labels
        .iter()
        .flat_map(|label| crate::help::chords_for_label(label))
        .collect();
    RESERVED
        .iter()
        .copied()
        .filter(|(reserved, _)| {
            crate::help::chords_for_label(reserved)
                .iter()
                .any(|key| bound.contains(key))
        })
        .collect()
}

/// The action `name` spells, or `None` when no action does.
///
/// `pub(crate)` rather than private (#61): the help overlay reads a row's
/// `names` — the same strings a `[keymap]` line uses — and needs the action
/// each one spells before it can ask what key reaches it.
pub(crate) fn action_named(name: &str) -> Option<ActionId> {
    DEFAULT
        .iter()
        .map(|(_, _, action)| *action)
        .find(|action| action.name() == name)
}

/// Every action, once each, in `DEFAULT`'s order.
///
/// `DEFAULT` holds 121 rows for 93 actions, because 24 actions carry a second
/// key and `hit.next`/`hit.prev` each hold a row in two scopes. This yields
/// each action one time, which is the unit `--print-keymap` prints and the
/// unit a `[keymap]` line names.
pub(crate) fn every_action() -> impl Iterator<Item = ActionId> {
    let mut seen: Vec<ActionId> = Vec::new();
    DEFAULT.iter().filter_map(move |(_, _, action)| {
        if seen.contains(action) {
            None
        } else {
            seen.push(*action);
            Some(*action)
        }
    })
}

/// Every action name, in table order, each once — the list an unknown name's
/// error offers as the fix.
fn known_action_names() -> Vec<String> {
    let mut names: Vec<&'static str> = Vec::new();
    for (_, _, action) in DEFAULT {
        if !names.contains(&action.name()) {
            names.push(action.name());
        }
    }
    names.into_iter().map(str::to_string).collect()
}

/// Every binding in force: the defaults, with the user's changes folded in.
///
/// Built once at startup and then only read. A `Vec` scanned linearly, for the
/// reason the `DEFAULT` scan gave: about 130 entries, consulted once per
/// keypress, which is an event a human produced. A map would be faster and
/// would have to be built, held and kept in step for no measurable gain.
///
/// The label is a `String` where `DEFAULT`'s is a `&'static str`: half of them
/// can now come from `config.toml`, which is read at run time (#61).
/// `pub` rather than `pub(crate)` only so that `Config` can carry one, the
/// same reason `filter::LoadedSet` is public: `main` resolves the table
/// before the terminal comes up and hands it to `App` through the config.
/// Every method on it stays `pub(crate)` — the type is carried across the
/// crate boundary, not operated on there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    entries: Vec<(Scope, String, ActionId)>,
}

/// `DEFAULT`, and nothing else: what recon binds when the file says nothing.
impl Default for Keymap {
    fn default() -> Self {
        Self {
            entries: DEFAULT
                .iter()
                .map(|(scope, label, action)| (*scope, (*label).to_string(), *action))
                .collect(),
        }
    }
}

impl Keymap {
    /// The defaults with a `[keymap]` table folded in (#61).
    ///
    /// Every action the table names is rebound; anything it does not mention
    /// keeps what it had. A name no action spells is
    /// [`crate::config::ConfigError::UnknownAction`], and a key spelling the
    /// label grammar cannot read is
    /// [`crate::config::ConfigError::BadKeyLabel`] — `Config::build_keymap`
    /// runs this in `main`, before the terminal comes up, so a typo is
    /// refused on a screen the message can still reach rather than bound to
    /// nothing.
    pub(crate) fn new(
        overlay: &crate::config::KeymapConfig,
    ) -> Result<(Self, Vec<String>), crate::config::ConfigError> {
        let mut keymap = Self::default();
        let mut reserved = Vec::new();
        for (name, labels) in &overlay.bindings {
            let action =
                action_named(name).ok_or_else(|| crate::config::ConfigError::UnknownAction {
                    name: name.clone(),
                    known: known_action_names(),
                })?;
            // Checked before the rebind rather than left to `resolve`: a
            // spelling nothing can parse would otherwise bind the action to
            // no key at all, silently, which is the state a typo produces.
            if let Some(bad) = labels
                .iter()
                .find(|label| !crate::help::label_is_readable(label))
            {
                return Err(crate::config::ConfigError::BadKeyLabel {
                    action: name.clone(),
                    label: bad.clone(),
                });
            }
            // `reserved_hits` above is what keeps this to one warning per
            // reserved key rather than one per scope. Collected rather than
            // logged here: `Keymap::new` cannot read a `Config`, so it cannot
            // know whether the user asked for silence. `Config::build_keymap`
            // is the caller that can, and logs each of these with
            // `log::warn!` before `main` brings up the terminal — the same
            // reason `check_sets` is called there rather than from
            // `Config::load` — so a warning still reaches stderr rather than
            // being dropped by `Muted` (#246).
            // The reserved key that was found, not the label the line was
            // written with: a range names many keys and only one of them is
            // reserved, so `'*-/'` has to be reported as `-`.
            for (key, claim) in reserved_hits(labels) {
                reserved.push(format!(
                    "{name} binds '{key}', which is reserved for {claim}; \
                     a later release will want it back, but recon binds it anyway"
                ));
            }
            keymap.rebind(action, labels);
        }
        Ok((keymap, reserved))
    }

    /// Put `labels` in place of every key `action` currently holds.
    ///
    /// The replacement lands **once per scope the action already appeared
    /// in**, at the position of its first row there. Two things turn on that
    /// (#61). `hit.next` and `hit.prev` each hold a row in `Scope::View` and
    /// another in `Scope::Filters`, and a config line names an action, not a
    /// scope — putting the new key back in only one of them would take `n`
    /// away from the other pane, which is not what a rebind was asked to do.
    /// And leaving each action where it sat is what makes a round trip
    /// through `--print-keymap` rebuild the table it printed, rather than the
    /// same bindings in a different order.
    ///
    /// That round trip relies on `HitNext`/`HitPrev` carrying identical
    /// labels in both of their scopes: `print_keymap` deduplicates a printed
    /// action's labels *across* scopes, while this puts the whole `labels`
    /// list back in *every* scope the action held. If an action ever bound
    /// different keys per scope, printing it would lose which scope had
    /// which key, and parsing the result back in would hand every scope the
    /// union — a `--print-keymap` round trip that quietly widens the map.
    /// `the_printed_keymap_parses_back_as_the_same_bindings` would catch it
    /// loudly, so no guard lives here — this comment is the warning for
    /// whoever binds an action to different keys per scope next.
    ///
    /// Every `ActionId` holds at least one `DEFAULT` row, so there is always
    /// a position to replace.
    fn rebind(&mut self, action: ActionId, labels: &[String]) {
        let mut replaced: Vec<Scope> = Vec::new();
        let mut entries = Vec::with_capacity(self.entries.len());
        for (scope, label, entry_action) in self.entries.drain(..) {
            if entry_action != action {
                entries.push((scope, label, entry_action));
            } else if !replaced.contains(&scope) {
                replaced.push(scope);
                entries.extend(labels.iter().map(|label| (scope, label.clone(), action)));
            }
        }
        self.entries = entries;
    }

    /// Drop the rows a `check::Report` named as losing their key.
    ///
    /// This is what makes a written line win. `resolve` takes the first
    /// matching row and `rebind` keeps every action at its original position,
    /// so without this the built-in table's row order decides a contested key
    /// — an order the user cannot see and no test holds still.
    ///
    /// It also keeps `labels_for` honest, and through it the `?` overlay:
    /// a row removed here stops being offered as a key that reaches its
    /// action, because it no longer does.
    ///
    /// A row loses **the keys named**, not the label spelling them. One label
    /// can name several keys, and only `1-9` in `DEFAULT` does today: a line
    /// taking `5` from `global.filters.toggle` must leave the other eight
    /// digits toggling filters, so that row is re-spelled as the keys it keeps
    /// rather than dropped whole. A row naming one key — every other row — is
    /// dropped exactly as it was before, since the keys it keeps are none.
    pub(crate) fn evict(&mut self, rows: &[(Scope, String, ActionId)]) {
        let mut entries = Vec::with_capacity(self.entries.len());
        for (scope, label, action) in self.entries.drain(..) {
            let lost: Vec<crate::help::Chord> = rows
                .iter()
                .filter(|(row_scope, _, row_action)| *row_scope == scope && *row_action == action)
                .flat_map(|(_, key, _)| crate::help::chords_for_label(key))
                .collect();
            let held = crate::help::chords_for_label(&label);
            let kept: Vec<crate::help::Chord> = held
                .iter()
                .copied()
                .filter(|chord| !lost.contains(chord))
                .collect();
            if kept.len() == held.len() {
                // Nothing of this row was taken. Pushed back exactly as it
                // was, label and all, so a row that keeps every key it had
                // keeps the spelling the table or the user gave it.
                entries.push((scope, label, action));
            } else {
                entries.extend(kept.into_iter().map(|chord| (scope, chord.label(), action)));
            }
        }
        self.entries = entries;
    }

    /// The action a key names in a scope, or `None` when the scope does not
    /// bind it.
    pub(crate) fn resolve(&self, scope: Scope, key: Key) -> Option<ActionId> {
        self.entries
            .iter()
            .find(|(entry_scope, label, _)| {
                *entry_scope == scope && crate::help::label_matches(label, key)
            })
            .map(|(_, _, action)| *action)
    }

    /// The key label bound to `action`, or `None` when nothing is.
    ///
    /// An action bound to more than one key (`nav.parent` also binds `Left`)
    /// takes its first row: that is the canonical spelling, listed first, and
    /// the one a hint should show. Under an overlay that is the first key the
    /// user listed, which is what makes a hint track a rebind.
    ///
    /// `Option` rather than the `expect` this replaced (#61): a `[keymap]`
    /// line can leave an action with no key at all, so an absent label is a
    /// config file's doing and must not crash the TUI — the same rule
    /// `App::perform`'s `debug_assert!` arms record.
    ///
    /// `pub(crate)` rather than private (task 8 fix round 1, #199): the
    /// "nothing selected" hint in `selection.rs` names only a key, with no
    /// verb of its own to attach to it — `hint_for` would say too much — so
    /// it calls this directly instead of going through `hint_for`.
    pub(crate) fn label_for(&self, action: ActionId) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, _, a)| *a == action)
            .map(|(_, label, _)| label.as_str())
    }

    /// Every key bound to `action`, in table order, each once.
    ///
    /// `label_for` answers "which one key should a hint name"; this answers
    /// "which keys reach this at all", which is what a help row shows — a row
    /// lists `h / Left`, not just `h` (#61).
    ///
    /// Deduplicated for the reason `print_keymap` deduplicates: `hit.next`
    /// and `hit.prev` each hold a row in `Scope::View` and another in
    /// `Scope::Filters` carrying the identical label, and a row reading
    /// `n / n` would be a lie told twice.
    ///
    /// Empty when a `[keymap]` line has left the action with no key at all —
    /// the same state `label_for` reports as `None`.
    pub(crate) fn labels_for(&self, action: ActionId) -> Vec<&str> {
        let mut labels: Vec<&str> = Vec::new();
        for (_, label, entry_action) in &self.entries {
            if *entry_action == action && !labels.contains(&label.as_str()) {
                labels.push(label);
            }
        }
        labels
    }

    /// Build a key hint: the key that reaches `action`, the verb describing
    /// what it does, and the key that reaches `opener`.
    ///
    /// Task 8 (#199): the callers used to spell the key inside their own hint
    /// text, so a rebind that changed which key reached `action` left the hint
    /// naming the wrong one. Looking the key up here instead means the hint
    /// tracks a rebind for free.
    ///
    /// The verb is not looked up here: the table holds no prose, and
    /// `help::KEYMAP`'s `action` text is both the wrong register for a
    /// status-line sentence (imperative and capitalised, for a command list —
    /// not third-person lowercase, for a sentence) and, for `nav.open`,
    /// ambiguous — it names two different rows. So a caller supplies its own
    /// verb, and only the key and the opener are generated.
    ///
    /// `None` when any key the sentence needs is unbound: a hint with a hole
    /// where a key should be ("· t " with nothing after it) tells the user
    /// less than no hint at all (#61).
    pub(crate) fn hint_for(
        &self,
        action: ActionId,
        verb: &str,
        opener: ActionId,
    ) -> Option<String> {
        self.hint_for_trailing(action, verb, opener, action)
    }

    /// `hint_for`, widened for the one hint whose trailing key names a
    /// different action than the one the hint explains (task 8 fix round 1,
    /// #199): `y`'s hint reads "then press v", not "then press y", because
    /// copying needs a selection first, and a selection is started with `v`
    /// (`GlobalVisualChar`), not `y` (`GlobalYank`). `hint_for` is this with
    /// `trailing` pinned to `action`, which is what every other hint wants.
    pub(crate) fn hint_for_trailing(
        &self,
        action: ActionId,
        verb: &str,
        opener: ActionId,
        trailing: ActionId,
    ) -> Option<String> {
        let key = self.label_for(action)?;
        let opener_key = self.label_for(opener)?;
        let trailing_key = self.label_for(trailing)?;
        Some(format!("{key} {verb} · {opener_key} {trailing_key}"))
    }
}

/// The keymap as a `[keymap]` stanza a user can paste.
///
/// Prints the map **in force**, not the defaults. Once a file can change the
/// map, the defaults describe a map the user does not have — and after
/// `Keymap::evict` the map can differ from a plain reading of their own file
/// too, which is exactly what the comments explain.
///
/// A line matching its default carries no comment, so a user who has changed
/// nothing sees what they have always seen. A comment is not data, so the
/// output still parses and still pastes.
///
/// One line **per action**, not per row: TOML has no duplicate keys, and 26
/// of the table's 93 actions bind more than one key in the same scope
/// (`GlobalToggleHide` alone has three: `u`, `H`, `Ctrl-h`). A row per
/// binding would print `'global.toggle.hide'` three times and the file the
/// parser rejects would be recon's own advice. An action's several keys
/// become a TOML array instead, in table order.
///
/// Labels are deduplicated before printing. `HitNext`/`HitPrev` are the one
/// case that needs it: each name is shared by a `View` row and a `Filters`
/// row, and both rows carry the identical label (`n`/`N`) — without
/// deduplication that is a valid but false `['n', 'n']`.
///
/// Grouped by the scope of an action's first `DEFAULT` row, in table order,
/// which is the order the help overlay shows — the same reason
/// `print_editor_config`'s FLAVOURS stays in a fixed, deliberate order.
#[must_use]
pub fn print_keymap(keymap: &Keymap, defaults: &Keymap) -> String {
    let mut out = String::from(
        "# recon's keymap in effect\n\
         # Paste into ~/.config/recon/config.toml — recon never writes it for you.\n\
         # Keep only the lines you want to change; anything absent keeps its default.\n\
         # A line with no comment matches recon's built-in default.\n\
         [keymap]\n",
    );

    let mut last_scope = None;
    for action in every_action() {
        let labels = keymap.labels_for(action);
        let scope = first_scope_of(defaults, action);
        if last_scope != Some(scope) {
            out.push('\n');
            last_scope = Some(scope);
        }

        let value = if let [only] = labels.as_slice() {
            toml_string(only)
        } else {
            format!(
                "[{}]",
                labels
                    .iter()
                    .map(|label| toml_string(label))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        // `writeln!` rather than `push_str(&format!(...))`: writing into the
        // `String` directly avoids allocating a throwaway one per row.
        // Infallible — `String`'s `Write` impl never errs.
        let _ = writeln!(
            out,
            "{} = {value}{}",
            toml_string(action.name()),
            annotation(keymap, defaults, action),
        );
    }
    out
}

/// The scope an action's first `DEFAULT` row sits in, for the blank-line
/// grouping. Read from the defaults rather than the live map: grouping must
/// not move about because a user rebound something.
fn first_scope_of(defaults: &Keymap, action: ActionId) -> Scope {
    defaults
        .entries
        .iter()
        .find(|(_, _, a)| *a == action)
        .map_or(Scope::Global, |(scope, _, _)| *scope)
}

/// Every key `action` no longer answers to, and what holds each one now.
///
/// Asked scope by scope, and in concrete keys. `Keymap::evict` removes one
/// `(scope, key, action)` row, and that is what first lets an action holding
/// a row in two scopes lose a key in one of them: before eviction existed,
/// only `rebind` could remove a row, and `rebind` replaces an action's list
/// in every scope it holds at once, so an action's scopes could never fall
/// out of step.
///
/// `labels_for` merges an action's scopes into one list, so comparing merged
/// lists cannot see any of this. It reports a key as still held when only one
/// scope still holds it, and a single search for the thief names whichever
/// comes first in the table when two scopes lost one key to two different
/// actions. Both are answered by asking per scope instead.
///
/// A key the user simply rebound away is not a loss and yields nothing here:
/// nobody else holds it, so there is no thief to name, and `annotation` says
/// "yours" instead.
fn losses(keymap: &Keymap, defaults: &Keymap, action: ActionId) -> Vec<String> {
    let mut taken: Vec<String> = Vec::new();
    for (scope, label, entry_action) in &defaults.entries {
        if *entry_action != action {
            continue;
        }
        for key in crate::help::chords_for_label(label) {
            if reaches(keymap, *scope, action, key) {
                continue;
            }
            // Only `Scope::Global` shadows a pane, which is `check`'s pass
            // two. A peer pane holding the same key by coincidence — `j`
            // defaults `nav.down`, `view.down` and `filters.down` — never
            // crosses with this one, so it must not be searched.
            let mut scopes = vec![*scope];
            if matches!(scope, Scope::Nav | Scope::View | Scope::Filters) {
                scopes.push(Scope::Global);
            }
            let Some((_, _, thief)) = keymap.entries.iter().find(|(entry_scope, entry, a)| {
                scopes.contains(entry_scope)
                    && *a != action
                    && crate::help::chords_for_label(entry).contains(&key)
            }) else {
                continue;
            };
            // Deduplicated: an action that lost one key in both of its scopes
            // to the same thief lost it once as far as a reader is concerned.
            let said = format!("'{}' taken by {}", key.label(), thief.name());
            if !taken.contains(&said) {
                taken.push(said);
            }
        }
    }
    taken
}

/// Whether `action` still answers `key` in `scope`.
fn reaches(keymap: &Keymap, scope: Scope, action: ActionId, key: crate::help::Chord) -> bool {
    keymap
        .entries
        .iter()
        .any(|(entry_scope, label, entry_action)| {
            *entry_scope == scope
                && *entry_action == action
                && crate::help::chords_for_label(label).contains(&key)
        })
}

/// What to say about a line that is no longer its default, or nothing at all.
///
/// Two things are worth saying, and they are different. An action whose keys
/// the user set says what it replaced. An action that merely *lost* a key
/// says which action took it, because that is the part a user cannot work
/// out from the line in front of them.
///
/// The losses are counted first, before the two label lists are compared at
/// all: a key lost in one scope of a two-scope action leaves the merged lists
/// equal, so a comparison of those lists returns with nothing said about a
/// key that has gone.
fn annotation(keymap: &Keymap, defaults: &Keymap, action: ActionId) -> String {
    let taken = losses(keymap, defaults, action);
    if !taken.is_empty() {
        return format!("   # {}", taken.join(", "));
    }

    let now = keymap.labels_for(action);
    let was = defaults.labels_for(action);
    if now == was {
        return String::new();
    }
    format!(
        "   # yours; default {}",
        if was.is_empty() {
            "none".to_string()
        } else {
            was.iter()
                .map(|label| format!("'{label}'"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn shift_is_dropped_from_an_uppercase_letter() {
        let real_terminal = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        let test_helper = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::empty());

        assert_eq!(
            normalise(real_terminal),
            normalise(test_helper),
            "a real terminal's G must resolve to the same entry as a synthetic one"
        );
    }

    #[test]
    fn shift_is_kept_nowhere_else_either() {
        // Punctuation never carries SHIFT in legacy mode, but a terminal that
        // sent it must not get a different answer.
        let braced = KeyEvent::new(KeyCode::Char('{'), KeyModifiers::SHIFT);
        assert!(!normalise(braced).ctrl);
        assert!(!normalise(braced).alt);
    }

    #[test]
    fn control_and_alt_are_kept() {
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(normalise(ctrl_d).ctrl);
        assert!(!normalise(ctrl_d).alt);

        let alt_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT);
        assert!(normalise(alt_d).alt);
        assert!(!normalise(alt_d).ctrl);
    }

    #[test]
    fn a_plain_key_carries_neither() {
        let plain = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty());
        assert_eq!(
            normalise(plain),
            Key {
                code: KeyCode::Char('q'),
                ctrl: false,
                alt: false
            }
        );
    }

    #[test]
    fn the_modal_scopes_come_before_the_panes() {
        // The order is the order `dispatch_event` already runs, and the table
        // must not be free to disagree with it.
        assert!(Scope::Prompt < Scope::Help);
        assert!(Scope::Help < Scope::Picker);
        assert!(Scope::Picker < Scope::Sets);
        assert!(Scope::Sets < Scope::Global);
        assert!(Scope::Global < Scope::Nav);
    }

    #[test]
    fn a_pane_scope_is_named_for_its_focus() {
        assert_eq!(Scope::for_focus(crate::widgets::Focus::Nav), Scope::Nav);
        assert_eq!(Scope::for_focus(crate::widgets::Focus::View), Scope::View);
        assert_eq!(
            Scope::for_focus(crate::widgets::Focus::Filters),
            Scope::Filters
        );
    }

    /// Labels a row documents in its action text rather than its `keys`
    /// array.
    ///
    /// `Home`/`End` alias `g`/`G` in all three panes, but adding them to the
    /// shared row's `keys` would lengthen the joined label past what the
    /// 150-column layout can afford — see
    /// `help::tests::a_normal_terminal_shows_the_whole_keymap`, which has no
    /// width slack left. The row says "also Home / End" in prose instead, so
    /// the binding is discoverable without costing a column.
    ///
    /// This is a deliberate, narrow exception for the layout budget, not a
    /// judgment that these aliases are unimportant. Anything added here has
    /// to earn its place the same way: every label outside this list still
    /// has to be named on a row whose `keys` actually list it.
    const DOCUMENTED_IN_PROSE: &[&str] = &["Home", "End"];

    /// Every documented binding resolves, and every entry in the table is
    /// documented. Will replace the two text-scraping drift tests in
    /// `help.rs` (task 9 removes them), which compared labels against
    /// `Char('x')` literals grepped out of seven source files (#162 records
    /// how that scan silently stopped working once).
    ///
    /// The set comparison alone cannot see a `names` entry sitting on the
    /// *wrong* row — two sets can agree while a name is attached to a key
    /// that never binds it. The per-entry loop below is what makes "every
    /// documented binding resolves" true rather than aspirational: for each
    /// row `DEFAULT` actually has, some `KEYMAP` binding whose `keys`
    /// contains that label has to name that exact action (#59 review: this
    /// is what would have caught `n`/`N` being bound in both `Global` and
    /// `Nav` — the set comparison alone did not).
    ///
    /// A `RESERVED` row (`-`, `:`) is documented on purpose while binding
    /// nothing: its `names` is empty, so it contributes nothing to either
    /// set and needs no exception here (#242).
    #[test]
    fn the_table_and_the_documentation_agree() {
        let documented: std::collections::BTreeSet<&str> = crate::help::KEYMAP
            .iter()
            .flat_map(|section| section.bindings)
            .flat_map(|binding| binding.names.iter().copied())
            .collect();

        let tabled: std::collections::BTreeSet<&str> =
            DEFAULT.iter().map(|(_, _, action)| action.name()).collect();

        let undocumented: Vec<&&str> = tabled.difference(&documented).collect();
        assert!(
            undocumented.is_empty(),
            "in the table but not in KEYMAP: {undocumented:?}"
        );

        let unbound: Vec<&&str> = documented.difference(&tabled).collect();
        assert!(
            unbound.is_empty(),
            "in KEYMAP but not in the table: {unbound:?}"
        );

        let mut mismatched = Vec::new();
        for (scope, label, action) in DEFAULT {
            if DOCUMENTED_IN_PROSE.contains(label) {
                continue;
            }
            let named_on_that_row = crate::help::KEYMAP
                .iter()
                .flat_map(|section| section.bindings)
                .any(|binding| {
                    binding.keys.contains(label) && binding.names.contains(&action.name())
                });
            if !named_on_that_row {
                mismatched.push(format!("{scope:?} {label:?} -> {}", action.name()));
            }
        }
        assert!(
            mismatched.is_empty(),
            "not named on any KEYMAP row whose keys include the label:\n  {}",
            mismatched.join("\n  ")
        );
    }

    #[test]
    fn a_real_terminals_capital_g_resolves_in_the_view() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(
            Keymap::default().resolve(Scope::View, normalise(event)),
            Some(ActionId::ViewGotoEnd),
            "this is #250: G carries SHIFT on a real terminal"
        );
    }

    #[test]
    fn an_unbound_key_resolves_to_nothing() {
        let event = KeyEvent::new(KeyCode::Char('~'), KeyModifiers::empty());
        assert_eq!(
            Keymap::default().resolve(Scope::Global, normalise(event)),
            None
        );
    }

    #[test]
    fn the_printed_keymap_is_a_keymap_stanza() {
        let defaults = Keymap::default();
        let printed = print_keymap(&defaults, &defaults);

        assert!(printed.contains("[keymap]"), "{printed}");
        assert!(
            printed.contains("recon never writes it for you"),
            "the header must say recon will not write the file: {printed}"
        );
        assert!(printed.contains("'q'"), "{printed}");
        assert!(printed.contains("global.quit"), "{printed}");
    }

    /// A map equal to the defaults must print with no annotation at all, so a
    /// user who has changed nothing sees exactly what they saw before.
    ///
    /// Checks for `annotation`'s own delimiter (three spaces then `#`) rather
    /// than a bare `#`: `view.toggle.linenumbers` binds the literal key `'#'`
    /// by default, so a naive `contains('#')` would flag that unannotated
    /// line as if it carried a comment.
    #[test]
    fn an_unchanged_keymap_prints_without_comments() {
        let defaults = Keymap::default();
        let printed = print_keymap(&defaults, &defaults);

        for line in printed.lines().filter(|line| line.contains(" = ")) {
            assert!(
                !line.contains("   #"),
                "an unchanged line was annotated: {line}"
            );
        }
    }

    /// The point of printing the map in effect: the line tells you what it is
    /// now *and* what it was, so you can see what your file did.
    #[test]
    fn a_changed_line_says_what_it_replaced() {
        let defaults = Keymap::default();
        let (mut keymap, _) = Keymap::new(&overlay("global.reload", &["F5"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::GlobalReload]);
        keymap.evict(report.evict());

        let printed = print_keymap(&keymap, &defaults);
        let line = printed
            .lines()
            .find(|line| line.starts_with("'global.reload'"))
            .expect("global.reload must be printed");

        assert!(line.contains("'F5'"), "{line}");
        assert!(
            line.contains('#'),
            "a changed line must be annotated: {line}"
        );
        assert!(
            line.contains("'r'"),
            "the comment must name the old key: {line}"
        );
    }

    /// A key a *different* action took must be said so, because that is the
    /// case a user cannot work out from the line alone.
    ///
    /// Cross-scope: `global.quit` is in `Scope::Global`, which shadows every
    /// pane, so all three panes that defaulted `j` to their own `down` action
    /// lose it to `global.quit` — not to each other.
    #[test]
    fn a_taken_key_names_what_took_it() {
        let defaults = Keymap::default();
        let (mut keymap, _) = Keymap::new(&overlay("global.quit", &["j"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::GlobalQuit]);
        keymap.evict(report.evict());

        let printed = print_keymap(&keymap, &defaults);
        for action in ["'nav.down'", "'view.down'", "'filters.down'"] {
            let line = printed
                .lines()
                .find(|line| line.starts_with(action))
                .unwrap_or_else(|| panic!("{action} must be printed"));
            assert!(line.contains("'j' taken by global.quit"), "{line}");
        }
    }

    /// Task 7 fix round 1: the thief lookup used to scan every scope for the
    /// lost label and take the first action that was not the one
    /// being annotated. `j` is also `nav.down`'s, `view.down`'s and
    /// `picker.down`'s default key, so a same-scope eviction — one write in
    /// `Scope::Filters` taking `Scope::Filters`'s own `j` — was misreported
    /// as `nav.down`'s doing, purely because `Scope::Nav` sorts first in the
    /// table. The thief must be searched for in the scope the key was lost
    /// in (and `Scope::Global`, which can shadow it), never a peer pane.
    #[test]
    fn a_same_scope_taken_key_names_the_real_thief() {
        let defaults = Keymap::default();
        let (mut keymap, _) = Keymap::new(&overlay("filters.exclude", &["j"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::FiltersExclude]);
        keymap.evict(report.evict());

        let printed = print_keymap(&keymap, &defaults);
        let line = printed
            .lines()
            .find(|line| line.starts_with("'filters.down'"))
            .expect("filters.down must be printed");

        assert!(
            line.contains("'j' taken by filters.exclude"),
            "the real thief is in the same scope: {line}"
        );
        assert!(
            !line.contains("nav.down"),
            "nav.down never touched this key and must not be blamed: {line}"
        );

        let nav_line = printed
            .lines()
            .find(|line| line.starts_with("'nav.down'"))
            .expect("nav.down must be printed");
        assert!(
            !nav_line.contains('#'),
            "nav.down's own 'j' is untouched and must carry no annotation: {nav_line}"
        );
    }

    /// A second reproduction of the same fault, along the axis the first one
    /// didn't cover: `k` has **no default `Global` row at all** (its only
    /// rows are `Nav`, `View`, `Filters` and `Picker`), so a buggy unscoped
    /// scan does not even need a `Global` coincidence to misfire — it walks
    /// straight past every scope to `Nav`'s untouched `k` and blames
    /// `nav.up`, which never lost anything.
    #[test]
    fn a_same_scope_contest_with_no_global_row_still_names_the_real_thief() {
        let defaults = Keymap::default();
        let (mut keymap, _) = Keymap::new(&overlay("view.word.forward", &["k"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::ViewWordForward]);
        keymap.evict(report.evict());

        let printed = print_keymap(&keymap, &defaults);
        let line = printed
            .lines()
            .find(|line| line.starts_with("'view.up'"))
            .expect("view.up must be printed");

        assert!(
            line.contains("'k' taken by view.word.forward"),
            "the real thief is in the same scope: {line}"
        );
        assert!(
            !line.contains("nav.up"),
            "nav.up never touched this key and must not be blamed: {line}"
        );

        let nav_line = printed
            .lines()
            .find(|line| line.starts_with("'nav.up'"))
            .expect("nav.up must be printed");
        assert!(
            !nav_line.contains('#'),
            "nav.up's own 'k' is untouched and must carry no annotation: {nav_line}"
        );
    }

    /// A key taken out of a *range* was the third place the label-vs-key
    /// confusion hid, and the one a user was most likely to read.
    /// `global.filters.toggle` holds `1-9`, so losing `5` left no label
    /// *string* missing from its list: the comparison of label text saw
    /// nothing taken, fell through to the "yours" branch, and told the user
    /// they had written a line they never wrote — while never naming what had
    /// taken the digit.
    #[test]
    fn a_key_taken_out_of_a_range_names_the_thief_and_is_not_called_yours() {
        let defaults = Keymap::default();
        let (mut keymap, _) =
            Keymap::new(&overlay("global.editor.project", &["5"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::GlobalEditorProject]);
        keymap.evict(report.evict());

        let printed = print_keymap(&keymap, &defaults);
        let line = printed
            .lines()
            .find(|line| line.starts_with("'global.filters.toggle'"))
            .expect("global.filters.toggle must be printed");

        assert!(
            line.contains("'5' taken by global.editor.project"),
            "the annotation must name the key lost and what took it: {line}"
        );
        assert!(
            !line.contains("yours"),
            "nobody wrote this line, so it must not be called yours: {line}"
        );
        assert!(
            line.contains("'4'") && line.contains("'6'"),
            "the eight digits it kept must still be on the line: {line}"
        );

        // And the value list must not still offer the key the comment says it
        // lost. `losses` asks `reaches` first today, so the two halves of the
        // line cannot disagree — but a change that separated them could
        // satisfy every assertion above while printing '5' in both, a line
        // claiming the action still holds the key it has just announced
        // losing. Split on the annotation's own delimiter, because '5'
        // belongs in the comment by design.
        let value = line
            .split("   #")
            .next()
            .expect("the value comes before the comment");
        assert!(
            !value.contains("'5'"),
            "the key it lost must be gone from the value list: {line}"
        );
    }

    /// Annotations are comments, so the output is still a `[keymap]` table
    /// and still parses. Without this the flag stops being paste-able, which
    /// is its whole purpose.
    #[test]
    fn an_annotated_keymap_still_parses_back_as_itself() {
        let defaults = Keymap::default();
        let (mut keymap, _) = Keymap::new(&overlay("global.reload", &["F5"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::GlobalReload]);
        keymap.evict(report.evict());

        let printed = print_keymap(&keymap, &defaults);
        let parsed: crate::config::FileConfig =
            toml::from_str(&printed).expect("recon must print what it accepts");
        let overlay = parsed.keymap.expect("a [keymap] table");

        let (rebuilt, _) = Keymap::new(&overlay).expect("valid");
        assert_eq!(rebuilt, keymap, "printing then parsing changed a binding");
    }

    /// One `[keymap]` value, either a bare string or an array of them — the
    /// two shapes `print_keymap` emits depending on how many keys an action
    /// has.
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Keys {
        One(String),
        Many(Vec<String>),
    }

    impl Keys {
        fn as_vec(&self) -> Vec<String> {
            match self {
                Self::One(key) => vec![key.clone()],
                Self::Many(keys) => keys.clone(),
            }
        }
    }

    /// A local, test-only mirror of the `[keymap]` table rather than
    /// `crate::config::FileConfig` — Task 3 has not added `FileConfig::keymap`
    /// yet, and this needs only `serde`, which the pinned `toml` build always
    /// carries (`display`, the serializer, is the one feature dropped).
    #[derive(serde::Deserialize)]
    struct Parsed {
        keymap: std::collections::BTreeMap<String, Keys>,
    }

    /// Parses as TOML (Task 2 fix round 1, #61 review) — a `contains` check
    /// cannot prove this and previously let a syntax the parser rejects ship
    /// as "printed" — and every printed action's keys match `DEFAULT`
    /// exactly: same set of names, same labels in the same order,
    /// deduplicated. A plain substring check on the action name cannot tell
    /// `global.quit` from `global.quit.silent`, so this compares the parsed
    /// structure instead.
    #[test]
    fn every_default_binding_is_printed_exactly_once() {
        let printed = print_keymap(&Keymap::default(), &Keymap::default());
        let parsed: Parsed = toml::from_str(&printed).unwrap_or_else(|err| {
            panic!("the printed keymap must parse as TOML: {err}\n{printed}")
        });

        let mut expected: std::collections::BTreeMap<&str, Vec<&str>> =
            std::collections::BTreeMap::new();
        for (_, label, action) in DEFAULT {
            let labels = expected.entry(action.name()).or_default();
            if !labels.contains(label) {
                labels.push(label);
            }
        }

        let parsed_names: std::collections::BTreeSet<&str> =
            parsed.keymap.keys().map(String::as_str).collect();
        let expected_names: std::collections::BTreeSet<&str> = expected.keys().copied().collect();
        assert_eq!(
            parsed_names, expected_names,
            "the printed action names must match DEFAULT's distinct actions exactly"
        );

        for (name, labels) in &expected {
            let actual = parsed.keymap[*name].as_vec();
            assert_eq!(
                &actual, labels,
                "{name}'s printed keys must match DEFAULT, in order and deduplicated"
            );
        }
    }

    #[test]
    fn the_printed_keymap_parses_back_as_the_same_bindings() {
        // The whole point of printing in the accepted syntax: a user pastes a
        // line and it means what it meant.
        let printed = print_keymap(&Keymap::default(), &Keymap::default());
        let parsed: crate::config::FileConfig =
            toml::from_str(&printed).expect("recon must print what it accepts");
        // Not named `overlay`: the loop below calls the helper of that name.
        let table = parsed.keymap.expect("a [keymap] table");

        let (built, _) = Keymap::new(&table).expect("the defaults must be valid");
        assert_eq!(
            built,
            Keymap::default(),
            "printing then parsing changed a binding"
        );

        // And a map that has been **evicted**, which is what `rebind`'s
        // comment above names this test as the guard for. The defaults alone
        // cannot catch that class: eviction is the only thing that removes a
        // single row, so it is the only way the printed stanza can come to
        // describe a map it cannot rebuild.
        for (action, keys) in [
            // A two-scope action as the **winner**, taking one key into both
            // of its scopes at once. Nothing held this shape, and it is the
            // one all-or-nothing eviction makes interesting.
            ("hit.next", &["j"][..]),
            // A two-scope action robbed in one of its scopes.
            ("view.line.end", &["n"][..]),
            // One key taken out of a default range.
            ("global.editor.project", &["5"][..]),
            // A global line shadowing the same key in three panes at once.
            ("global.quit", &["j"][..]),
        ] {
            let (mut keymap, _) = Keymap::new(&overlay(action, keys)).expect("valid");
            let written = vec![action_named(action).expect("a real action")];
            let report = check::check(&keymap, &written);
            assert!(report.errors().is_empty(), "{action}: {report:?}");
            keymap.evict(report.evict());

            let printed = print_keymap(&keymap, &Keymap::default());
            let parsed: crate::config::FileConfig = toml::from_str(&printed)
                .unwrap_or_else(|err| panic!("{action}: must print what it accepts: {err}"));
            let pasted = parsed.keymap.expect("a [keymap] table");
            let (rebuilt, _) = Keymap::new(&pasted).expect("valid");

            assert_eq!(
                rebuilt, keymap,
                "{action}: the printed stanza rebuilt a different map\n{printed}"
            );
        }
    }

    // ---- the overlay (#61) ----------------------------------------------

    /// The shape one `[keymap]` line parses to.
    fn overlay(action: &str, keys: &[&str]) -> crate::config::KeymapConfig {
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert(
            action.to_string(),
            keys.iter().map(|key| (*key).to_string()).collect(),
        );
        crate::config::KeymapConfig { bindings }
    }

    #[test]
    fn an_override_moves_an_action_and_frees_the_old_key() {
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("global.quit".to_string(), vec!["Ctrl-q".to_string()]);
        let overlay = crate::config::KeymapConfig { bindings };

        let (keymap, _) = Keymap::new(&overlay).expect("valid");
        let ctrl_q = normalise(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        let plain_q = normalise(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()));

        assert_eq!(
            keymap.resolve(Scope::Global, ctrl_q),
            Some(ActionId::GlobalQuit)
        );
        assert_eq!(
            keymap.resolve(Scope::Global, plain_q),
            None,
            "rebinding an action must take its default key away, or q would do two things"
        );
    }

    #[test]
    fn an_untouched_action_keeps_its_default() {
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("global.quit".to_string(), vec!["Ctrl-q".to_string()]);
        let overlay = crate::config::KeymapConfig { bindings };

        let (keymap, _) = Keymap::new(&overlay).expect("valid");
        let help = normalise(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()));

        assert_eq!(
            keymap.resolve(Scope::Global, help),
            Some(ActionId::GlobalHelp)
        );
    }

    /// `hit.next` and `hit.prev` are the only two actions bound in more than
    /// one scope, and a config line names an action, not a scope: rebinding
    /// one must move it in *both* panes, or `n` would quietly stop working in
    /// one of them (#61).
    #[test]
    fn rebinding_a_two_scope_action_keeps_both_scopes() {
        let (keymap, _) = Keymap::new(&overlay("hit.next", &["Ctrl-n"])).expect("valid");
        let ctrl_n = normalise(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        let plain_n = normalise(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()));

        for scope in [Scope::View, Scope::Filters] {
            assert_eq!(
                keymap.resolve(scope, ctrl_n),
                Some(ActionId::HitNext),
                "the new key must reach the action in {scope:?}"
            );
            assert_eq!(
                keymap.resolve(scope, plain_n),
                None,
                "the old key must be free in {scope:?} too"
            );
        }
    }

    /// The hint mechanism's reason for existing (#199, #61): the key a hint
    /// names is looked up rather than written out, so a rebind moves it.
    #[test]
    fn a_hint_names_the_rebound_keys() {
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("global.visual.char".to_string(), vec!["s".to_string()]);
        bindings.insert("global.focus.view".to_string(), vec!["T".to_string()]);
        let (keymap, _) = Keymap::new(&crate::config::KeymapConfig { bindings }).expect("valid");

        assert_eq!(
            keymap.hint_for(
                ActionId::GlobalVisualChar,
                "selects text in the file view",
                ActionId::GlobalFocusView,
            ),
            Some("s selects text in the file view · T s".to_string()),
            "the hint must name the user's keys, not v and t"
        );
    }

    /// An overlay may leave an action with no key at all, which is a config
    /// file's doing and must not crash the TUI (#61). The hint is dropped
    /// rather than rendered with a hole where its key would be.
    #[test]
    fn an_action_left_unbound_has_no_label_and_no_hint() {
        let (keymap, _) = Keymap::new(&overlay("global.reload", &[])).expect("valid");

        assert_eq!(keymap.label_for(ActionId::GlobalReload), None);
        assert_eq!(
            keymap.hint_for(
                ActionId::GlobalReload,
                "reloads the file",
                ActionId::GlobalFocusView,
            ),
            None
        );
    }

    // ---- reserved keys (#242, #61) --------------------------------------

    #[test]
    fn binding_a_reserved_key_warns_and_obeys() {
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("global.quit".to_string(), vec!["-".to_string()]);
        let overlay = crate::config::KeymapConfig { bindings };

        // A warning, not a refusal: it is the user's keyboard, and 1.0 only
        // promises that 1.1 will want the key back.
        let (keymap, warnings) = Keymap::new(&overlay).expect("a reserved key is allowed");
        let dash = normalise(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::empty()));

        assert_eq!(
            keymap.resolve(Scope::Global, dash),
            Some(ActionId::GlobalQuit)
        );
        assert_eq!(
            warnings,
            vec![
                "global.quit binds '-', which is reserved for the hex view (#242); \
                 a later release will want it back, but recon binds it anyway"
                    .to_string()
            ],
            "the reserved-key warning must come back for the caller to log"
        );
    }

    /// The case `binding_a_reserved_key_warns_and_obeys` above cannot tell
    /// apart from a bug: `global.quit` holds one scope, so it would pass
    /// even if the check fired once per scope instead of once per key.
    /// `hit.next` is the real two-scope action (Ruling 12: a `DEFAULT` row
    /// in both `Scope::View` and `Scope::Filters`), and a `[keymap]` line
    /// for it still supplies exactly one label list — the same one
    /// `reserved_hits` is handed here. A version of the check that walked
    /// `Scope`s instead of this list — say, one moved inside `rebind`'s
    /// per-scope loop — would report the key twice; this asserts it does
    /// not, without needing a logging harness to watch `Keymap::new` warn.
    #[test]
    fn a_two_scope_actions_reserved_key_is_one_hit_not_two() {
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("hit.next".to_string(), vec!["-".to_string()]);
        let overlay = crate::config::KeymapConfig { bindings };

        // The same slice `Keymap::new` would read for this config line —
        // one list, regardless of `hit.next` holding two `DEFAULT` scopes.
        let labels = &overlay.bindings["hit.next"];
        assert_eq!(
            reserved_hits(labels),
            vec![("-", "the hex view (#242)")],
            "hit.next holds two scopes, but one config line must warn once"
        );
    }

    /// A range that covers a reserved key binds it, so it has to warn about
    /// it.
    ///
    /// `reserved_hits` compared label text, and no range is spelled `-` or
    /// `:`, so `'*-/'` took the hex view's key in silence — the one failure
    /// `RESERVED` exists to prevent. The warning names the reserved key that
    /// was found rather than the label written, because `'*-/'` is not a key.
    #[test]
    fn a_range_covering_a_reserved_key_warns_about_that_key() {
        // '*' through '/' covers '-'; '5' through '<' covers ':'.
        assert_eq!(
            reserved_hits(&["*-/".to_string()]),
            vec![("-", "the hex view (#242)")],
            "a range covering '-' binds it, so it must be reported as '-'"
        );
        assert_eq!(
            reserved_hits(&["5-<".to_string()]),
            vec![(":", "a command palette")]
        );
        // And a line reaching one reserved key two ways is still one warning.
        assert_eq!(
            reserved_hits(&["-".to_string(), "*-/".to_string()]),
            vec![("-", "the hex view (#242)")]
        );
    }

    /// The guard against over-warning. `1-9` is `global.filters.toggle`'s own
    /// default, so a check that fired here would warn about the built-in
    /// table on a config that says nothing.
    #[test]
    fn a_range_covering_no_reserved_key_stays_silent() {
        for range in ["1-9", "a-f", "A-Z"] {
            assert!(
                reserved_hits(&[range.to_string()]).is_empty(),
                "{range} covers neither reserved key"
            );
        }
    }

    #[test]
    fn the_reserved_keys_are_bound_to_nothing_by_default() {
        let keymap = Keymap::default();
        // Every scope, not just the panes: the promise is that a reserved
        // key binds nothing anywhere, and `Prompt` or `Picker` growing a
        // default binding later should fail this test rather than slip past
        // it.
        for (label, _) in RESERVED {
            for scope in [
                Scope::Prompt,
                Scope::Help,
                Scope::Picker,
                Scope::Sets,
                Scope::Global,
                Scope::Nav,
                Scope::View,
                Scope::Filters,
            ] {
                assert!(
                    !keymap
                        .entries
                        .iter()
                        .any(|(s, l, _)| s == &scope && l.as_str() == *label),
                    "{label} is reserved and must bind nothing"
                );
            }
        }
    }

    /// `check`'s same-scope pass assumes a contested key always has a written
    /// claimant, because a group of two defaults cannot happen. That is a fact
    /// about `DEFAULT`, so it is pinned here rather than trusted.
    ///
    /// By concrete key, not by label text, since `check` groups by key: two
    /// labels that expand onto one key — a `1-9` beside a `5`, an `F5` beside
    /// an `F6` — are a duplicate that a comparison of strings cannot see, and
    /// would reach `check` as a contest between two defaults.
    #[test]
    fn the_defaults_hold_no_duplicate_key() {
        let mut seen: Vec<(Scope, crate::help::Chord)> = Vec::new();
        for (scope, label, _) in DEFAULT {
            for chord in crate::help::chords_for_label(label) {
                assert!(
                    !seen.contains(&(*scope, chord)),
                    "{} is bound two times in {scope:?}",
                    chord.label()
                );
                seen.push((*scope, chord));
            }
        }
    }

    /// Every key `DEFAULT` binds renders back to a label naming that same key.
    ///
    /// `check` hands `Keymap::evict` a concrete key spelled by `Chord::label`,
    /// and `evict` reads it back with `chords_for_label`. The one key with no
    /// spelling is a function key, because every `Fn` label names it; this is
    /// what keeps an eviction from ever carrying one, since the row that loses
    /// a key is always a row the config file did not write and so always one
    /// of these.
    #[test]
    fn every_default_key_renders_back_to_a_label() {
        for (scope, label, _) in DEFAULT {
            for chord in crate::help::chords_for_label(label) {
                assert_eq!(
                    crate::help::chords_for_label(&chord.label()),
                    vec![chord],
                    "{scope:?} {label:?} names a key that no label reads back"
                );
            }
        }
    }

    #[test]
    fn every_scope_has_a_name_for_a_message() {
        assert_eq!(Scope::Global.name(), "global");
        assert_eq!(Scope::Nav.name(), "nav");
        assert_eq!(Scope::View.name(), "view");
        assert_eq!(Scope::Filters.name(), "filters");
        assert_eq!(Scope::Prompt.name(), "prompt");
        assert_eq!(Scope::Picker.name(), "picker");
        assert_eq!(Scope::Help.name(), "help");
    }

    /// An action bound to several keys hints with the first one the user
    /// listed — the same rule `label_for` keeps for `DEFAULT`.
    #[test]
    fn several_keys_hint_with_the_first() {
        let (keymap, _) = Keymap::new(&overlay("global.reload", &["F5", "r"])).expect("valid");

        assert_eq!(keymap.label_for(ActionId::GlobalReload), Some("F5"));
        let f5 = normalise(KeyEvent::new(KeyCode::F(5), KeyModifiers::empty()));
        let r = normalise(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::empty()));
        assert_eq!(
            keymap.resolve(Scope::Global, f5),
            Some(ActionId::GlobalReload)
        );
        assert_eq!(
            keymap.resolve(Scope::Global, r),
            Some(ActionId::GlobalReload),
            "every key the user listed must reach the action"
        );
    }

    /// The heart of phase 2c. Before this, `resolve` took the first matching
    /// row and `rebind` left every action at its original position, so 'q'
    /// went to `global.quit` — the earlier row — however plainly the file
    /// asked for `global.reload`. Driving the real binary confirmed it: recon
    /// quit.
    #[test]
    fn a_written_line_beats_a_default_on_the_same_key() {
        let (mut keymap, _) = Keymap::new(&overlay("global.reload", &["q"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::GlobalReload]);
        keymap.evict(report.evict());

        let q = normalise(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()));
        assert_eq!(
            keymap.resolve(Scope::Global, q),
            Some(ActionId::GlobalReload),
            "the file said reload, so 'q' must reload and not quit"
        );
    }

    /// A written line takes one key out of a default range, and the other
    /// eight keys of that range go on working.
    ///
    /// Both halves were broken before the checker compared concrete keys:
    /// nothing saw the contest, so `5` went on toggling a filter however
    /// plainly the file asked for the editor — and an eviction that dropped
    /// the whole `1-9` row would have been the opposite fault, costing eight
    /// keys nobody contested.
    #[test]
    fn a_written_line_takes_one_key_out_of_a_default_range() {
        let (mut keymap, _) =
            Keymap::new(&overlay("global.editor.project", &["5"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::GlobalEditorProject]);
        keymap.evict(report.evict());

        let five = normalise(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::empty()));
        let four = normalise(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::empty()));
        assert_eq!(
            keymap.resolve(Scope::Global, five),
            Some(ActionId::GlobalEditorProject),
            "the file said the editor, so '5' must open it"
        );
        assert_eq!(
            keymap.resolve(Scope::Global, four),
            Some(ActionId::GlobalFiltersToggle),
            "the eight digits nobody asked for must still toggle their filters"
        );
    }

    /// A two-scope action that loses a key loses it in both scopes, so what
    /// `--print-keymap` prints is a stanza that pastes back.
    ///
    /// `hit.next` and `hit.prev` are the only actions holding a row in two
    /// scopes, and a `[keymap]` line names an action with no scope in it — so
    /// "n reaches `hit.next` in the filter pane but not in the file view" is a
    /// state the file format cannot express. Evicting one scope at a time
    /// built that state and printed `'hit.next' = 'n'` with no comment, and
    /// recon then refused that line when it was pasted back.
    #[test]
    fn a_two_scope_action_loses_a_key_in_both_scopes_and_still_pastes_back() {
        let defaults = Keymap::default();
        let (mut keymap, _) = Keymap::new(&overlay("view.line.end", &["n"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::ViewLineEnd]);
        keymap.evict(report.evict());

        let n = normalise(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()));
        assert_eq!(
            keymap.resolve(Scope::View, n),
            Some(ActionId::ViewLineEnd),
            "the file asked for 'n' in the file view"
        );
        assert!(
            keymap.labels_for(ActionId::HitNext).is_empty(),
            "the key goes from every scope the action held it in, or the
             printed line cannot say which scope kept it"
        );
        assert_eq!(
            keymap.resolve(Scope::Filters, n),
            None,
            "the filter pane's 'n' goes with it"
        );

        let printed = print_keymap(&keymap, &defaults);
        let line = printed
            .lines()
            .find(|line| line.starts_with("'hit.next'"))
            .expect("hit.next must be printed");
        assert!(
            line.contains("[]"),
            "an action with no key prints as []: {line}"
        );
        assert!(line.contains("'n' taken by view.line.end"), "{line}");

        // The round trip the README promises: the dump, read back and checked
        // again, is refused by nothing.
        let parsed: crate::config::FileConfig =
            toml::from_str(&printed).expect("recon must print what it accepts");
        let pasted = parsed.keymap.expect("a [keymap] table");
        let written: Vec<ActionId> = pasted
            .bindings
            .keys()
            .filter_map(|name| action_named(name))
            .collect();
        let (rebuilt, _) = Keymap::new(&pasted).expect("valid");

        let again = check::check(&rebuilt, &written);
        assert_eq!(
            again,
            check::Report::default(),
            "pasting recon's own output back must say nothing at all: {again:?}"
        );
    }

    /// The overlay defect the same eviction removes, with no change to
    /// `help.rs`: `labels_for` fed the `?` overlay a key that now quits.
    #[test]
    fn a_shadowed_pane_key_leaves_the_actions_label_list() {
        let (mut keymap, _) = Keymap::new(&overlay("global.quit", &["j"])).expect("valid");
        let report = check::check(&keymap, &[ActionId::GlobalQuit]);
        keymap.evict(report.evict());

        assert!(
            !keymap.labels_for(ActionId::NavDown).contains(&"j"),
            "the overlay must stop offering a key that quits"
        );
        assert_eq!(
            keymap.labels_for(ActionId::NavDown),
            vec!["Down"],
            "and must still offer the key that works"
        );
    }

    /// `check`'s cross-scope pass assumes two *defaults* never cross, so the
    /// "neither written" case cannot occur. That is a fact about `DEFAULT`.
    #[test]
    fn no_default_key_is_in_both_global_and_a_pane() {
        // By concrete key, for the reason `the_defaults_hold_no_duplicate_key`
        // gives: the global `1-9` shadows a pane's `5` while sharing no
        // character with the label that spells it.
        let global: Vec<crate::help::Chord> = DEFAULT
            .iter()
            .filter(|(scope, _, _)| *scope == Scope::Global)
            .flat_map(|(_, label, _)| crate::help::chords_for_label(label))
            .collect();
        for (scope, label, _) in DEFAULT {
            if matches!(scope, Scope::Nav | Scope::View | Scope::Filters) {
                for chord in crate::help::chords_for_label(label) {
                    assert!(
                        !global.contains(&chord),
                        "{} is bound both globally and in {scope:?}",
                        chord.label()
                    );
                }
            }
        }
    }
}
