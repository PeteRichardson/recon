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
    ///
    /// No caller outside `the_table_and_the_documentation_agree` below as of
    /// task 4 (#199): plan 2b's `--print-keymap` is its production caller,
    /// via `action.name()`.
    #[allow(dead_code)]
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
pub(crate) fn resolve(scope: Scope, key: Key) -> Option<ActionId> {
    DEFAULT
        .iter()
        .find(|(entry_scope, label, _)| {
            *entry_scope == scope && crate::help::label_matches(label, key)
        })
        .map(|(_, _, action)| *action)
}

/// The key label `DEFAULT` binds to `action`.
///
/// An action bound to more than one key (`nav.parent` also binds `Left`)
/// takes its first `DEFAULT` row: that is the canonical spelling, listed
/// first, and the one a hint should show.
///
/// `pub(crate)` rather than private (task 8 fix round 1, #199): the "nothing
/// selected" hint in `selection.rs` names only a key, with no verb of its
/// own to attach to it — `hint_for` would say too much — so it calls this
/// directly instead of going through `hint_for`.
pub(crate) fn label_for(action: ActionId) -> &'static str {
    DEFAULT
        .iter()
        .find(|(_, _, a)| *a == action)
        .map(|(_, label, _)| *label)
        .expect("every action passed to a hint has a DEFAULT row")
}

/// Build a key hint: the key that reaches `action`, the verb describing what
/// it does, and the key that reaches `opener`.
///
/// Task 8 (#199): the callers used to spell the key inside their own hint
/// text, so a rebind that changed which key reached `action` left the hint
/// naming the wrong one. Looking the key up here instead means the hint
/// tracks a rebind for free.
///
/// The verb is not looked up here: `DEFAULT` holds no prose, and
/// `help::KEYMAP`'s `action` text is both the wrong register for a
/// status-line sentence (imperative and capitalised, for a command list —
/// not third-person lowercase, for a sentence) and, for `nav.open`,
/// ambiguous — it names two different rows. So a caller supplies its own
/// verb, and only the key and the opener are generated.
pub(crate) fn hint_for(action: ActionId, verb: &str, opener: ActionId) -> String {
    hint_for_trailing(action, verb, opener, action)
}

/// `hint_for`, widened for the one hint whose trailing key names a different
/// action than the one the hint explains (task 8 fix round 1, #199): `y`'s
/// hint reads "then press v", not "then press y", because copying needs a
/// selection first, and a selection is started with `v` (`GlobalVisualChar`),
/// not `y` (`GlobalYank`). `hint_for` is this with `trailing` pinned to
/// `action`, which is what every other hint wants.
pub(crate) fn hint_for_trailing(
    action: ActionId,
    verb: &str,
    opener: ActionId,
    trailing: ActionId,
) -> String {
    let key = label_for(action);
    let opener_key = label_for(opener);
    let trailing_key = label_for(trailing);
    format!("{key} {verb} · {opener_key} {trailing_key}")
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
