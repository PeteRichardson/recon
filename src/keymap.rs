//! Turning a keypress into a named action.
//!
//! One table, one rule for modifiers, and one place that decides what a key
//! means. Before this, eight `match` sites each decided for themselves and
//! three different modifier idioms disagreed at the edges — which is what
//! #250 was.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) code: KeyCode,
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
}

/// Put a keypress in the table's currency.
#[allow(dead_code)]
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
#[allow(dead_code)]
// Unused until task 4 resolves through the table; the allow goes with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Scope {
    /// A search or filter prompt is open. `handle_search_key` owns every key.
    Prompt,
    /// The help overlay is up, and any key closes it.
    Help,
    /// The profile picker is open.
    Picker,
    /// Checked for every key that no modal scope claimed.
    Global,
    Nav,
    View,
    Filters,
}

impl Scope {
    /// The pane scope for the focused pane.
    #[allow(dead_code)]
    // Unused until task 4 resolves through the table; the allow goes with it.
    pub(crate) fn for_focus(focus: crate::widgets::Focus) -> Self {
        match focus {
            crate::widgets::Focus::Nav => Self::Nav,
            crate::widgets::Focus::View => Self::View,
            crate::widgets::Focus::Filters => Self::Filters,
        }
    }
}

/// Every action a key can name.
///
/// The string form is what a user writes in `config.toml` and what the help
/// overlay shows, so the two cannot drift: a test compares this table against
/// `help::KEYMAP` by name.
#[allow(dead_code)]
// Unused until task 4 resolves through the table; the allow goes with it.
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
    GlobalFilterToggleNumbered,
    GlobalFileNext,
    GlobalFilePrev,
    GlobalHitNext,
    GlobalHitPrev,
    GlobalHideToggle,
    GlobalZoomView,
    GlobalZoomFocused,
    GlobalEditorProject,
    GlobalEditorFile,
    GlobalReload,
    GlobalVisualChar,
    GlobalVisualLine,
    GlobalYank,
    GlobalViewPageDown,
    GlobalViewPageUp,
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
    ViewLineNumbers,
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
    PromptDeleteToStart,
    // Picker
    PickerUp,
    PickerDown,
    PickerChoose,
    PickerCancel,
    // The help overlay itself has no ActionId: any key dismisses it, so
    // there is nothing to bind or rebind, and Scope::Help carries no DEFAULT
    // rows for the same reason (#59).
}

impl ActionId {
    /// The name a user writes, and the name the documentation shows.
    #[allow(dead_code)]
    // Unused until task 4 resolves through the table; the allow goes with it.
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
            Self::GlobalFilterToggleNumbered => "global.filter.toggle.numbered",
            Self::GlobalFileNext => "global.file.next",
            Self::GlobalFilePrev => "global.file.prev",
            Self::GlobalHitNext => "global.hit.next",
            Self::GlobalHitPrev => "global.hit.prev",
            Self::GlobalHideToggle => "global.hide.toggle",
            Self::GlobalZoomView => "global.zoom.view",
            Self::GlobalZoomFocused => "global.zoom.focused",
            Self::GlobalEditorProject => "global.editor.project",
            Self::GlobalEditorFile => "global.editor.file",
            Self::GlobalReload => "global.reload",
            Self::GlobalVisualChar => "global.visual.char",
            Self::GlobalVisualLine => "global.visual.line",
            Self::GlobalYank => "global.yank",
            Self::GlobalViewPageDown => "global.view.page.down",
            Self::GlobalViewPageUp => "global.view.page.up",
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
            Self::ViewLineNumbers => "view.linenumbers.toggle",
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
            Self::FiltersSaveSet => "filters.saveset",
            Self::PromptCommit => "prompt.commit",
            Self::PromptCancel => "prompt.cancel",
            Self::PromptLeft => "prompt.left",
            Self::PromptRight => "prompt.right",
            Self::PromptStart => "prompt.start",
            Self::PromptEnd => "prompt.end",
            Self::PromptDeleteBack => "prompt.delete.back",
            Self::PromptDeleteForward => "prompt.delete.forward",
            Self::PromptDeleteWord => "prompt.delete.word",
            Self::PromptDeleteToStart => "prompt.delete.tostart",
            Self::PickerUp => "picker.up",
            Self::PickerDown => "picker.down",
            Self::PickerChoose => "picker.choose",
            Self::PickerCancel => "picker.cancel",
        }
    }
}

/// The compiled-in bindings: scope, key label, action.
///
/// The label grammar is `help::Binding`'s — `q`, `G`, `Ctrl-d`, `space`,
/// `Shift-Tab`, `PageDown`, `Home`. `Binding::codes` parses it, and plan 2b
/// lets a user write the same spellings in `config.toml`.
#[allow(dead_code)]
// Unused until task 4 resolves through the table; the allow goes with it.
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
    (Scope::Global, "1-9", ActionId::GlobalFilterToggleNumbered),
    (Scope::Global, ".", ActionId::GlobalFileNext),
    (Scope::Global, ",", ActionId::GlobalFilePrev),
    (Scope::Global, "n", ActionId::GlobalHitNext),
    (Scope::Global, "N", ActionId::GlobalHitPrev),
    (Scope::Global, "u", ActionId::GlobalHideToggle),
    (Scope::Global, "H", ActionId::GlobalHideToggle),
    (Scope::Global, "Ctrl-h", ActionId::GlobalHideToggle),
    (Scope::Global, "b", ActionId::GlobalZoomView),
    (Scope::Global, "z", ActionId::GlobalZoomFocused),
    (Scope::Global, "o", ActionId::GlobalEditorProject),
    (Scope::Global, "O", ActionId::GlobalEditorFile),
    (Scope::Global, "r", ActionId::GlobalReload),
    (Scope::Global, "v", ActionId::GlobalVisualChar),
    (Scope::Global, "V", ActionId::GlobalVisualLine),
    (Scope::Global, "y", ActionId::GlobalYank),
    (Scope::Global, "]", ActionId::GlobalViewPageDown),
    (Scope::Global, "[", ActionId::GlobalViewPageUp),
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
    (Scope::View, "#", ActionId::ViewLineNumbers),
    (Scope::View, "Ctrl-e", ActionId::ViewScrollDown),
    (Scope::View, "Ctrl-y", ActionId::ViewScrollUp),
    (Scope::View, "Ctrl-d", ActionId::ViewHalfPageDown),
    (Scope::View, "Ctrl-u", ActionId::ViewHalfPageUp),
    (Scope::View, "Ctrl-f", ActionId::ViewPageDown),
    (Scope::View, "PageDown", ActionId::ViewPageDown),
    (Scope::View, "Ctrl-b", ActionId::ViewPageUp),
    (Scope::View, "PageUp", ActionId::ViewPageUp),
    (Scope::Filters, "k", ActionId::FiltersUp),
    (Scope::Filters, "Up", ActionId::FiltersUp),
    (Scope::Filters, "j", ActionId::FiltersDown),
    (Scope::Filters, "Down", ActionId::FiltersDown),
    (Scope::Filters, "g", ActionId::FiltersGotoStart),
    (Scope::Filters, "Home", ActionId::FiltersGotoStart),
    (Scope::Filters, "G", ActionId::FiltersGotoEnd),
    (Scope::Filters, "End", ActionId::FiltersGotoEnd),
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
    (Scope::Prompt, "Ctrl-u", ActionId::PromptDeleteToStart),
    (Scope::Picker, "k", ActionId::PickerUp),
    (Scope::Picker, "Up", ActionId::PickerUp),
    (Scope::Picker, "j", ActionId::PickerDown),
    (Scope::Picker, "Down", ActionId::PickerDown),
    (Scope::Picker, "Enter", ActionId::PickerChoose),
    (Scope::Picker, "Esc", ActionId::PickerCancel),
];

/// The action a key names in a scope, or `None` when the scope does not bind
/// it.
///
/// A linear scan on purpose: the table is under 130 entries and this runs once
/// per keypress, which is an event a human produced. A map would be faster and
/// would have to be built, held and kept in step for no measurable gain.
#[allow(dead_code)]
// Unused until task 4 resolves through the table; the allow goes with it.
pub(crate) fn resolve(scope: Scope, key: Key) -> Option<ActionId> {
    DEFAULT
        .iter()
        .find(|(entry_scope, label, _)| {
            *entry_scope == scope && crate::help::label_matches(label, key)
        })
        .map(|(_, _, action)| *action)
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
        assert!(Scope::Picker < Scope::Global);
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

    /// Every documented binding resolves, and every entry in the table is
    /// documented. This replaces the two text-scraping drift tests in
    /// `help.rs`, which compared labels against `Char('x')` literals grepped
    /// out of seven source files (#162 records how that scan silently stopped
    /// working once). Comparing the table against the documentation is exact.
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
    }

    #[test]
    fn a_real_terminals_capital_g_resolves_in_the_view() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(
            resolve(Scope::View, normalise(event)),
            Some(ActionId::ViewGotoEnd),
            "this is #250: G carries SHIFT on a real terminal"
        );
    }

    #[test]
    fn an_unbound_key_resolves_to_nothing() {
        let event = KeyEvent::new(KeyCode::Char('~'), KeyModifiers::empty());
        assert_eq!(resolve(Scope::Global, normalise(event)), None);
    }
}
