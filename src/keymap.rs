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
}
