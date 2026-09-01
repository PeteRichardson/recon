pub mod filenav;
pub mod fileview;
pub mod filterlist;

use ratatui::prelude::{Color, Style};
use ratatui::widgets::{Block, BorderType};
use std::path::PathBuf;

/// The bordered block every pane draws, carrying whether it has focus.
///
/// Focus used to be signalled by one thing: the selected row's foreground
/// turning green inside an already reverse-video highlight. That is a
/// low-contrast shift on a single row, while the border — the largest element
/// the pane owns — said nothing at all.
///
/// Colour *and* weight, not colour alone. This is the argument #19 makes about
/// the selection marker: a single visual channel fails on a theme with weak
/// contrast and for a colour-blind reader, and border weight survives a
/// terminal with no colour whatsoever. Green because it is already this app's
/// focus colour, so nothing new is introduced.
///
/// One helper rather than four call sites styling their own blocks — the
/// filter pane alone builds two, and they are exactly where a copy would drift.
pub fn pane_block<'a>(title: impl Into<ratatui::text::Line<'a>>, active: bool) -> Block<'a> {
    let block = Block::bordered().title(title);
    if active {
        block
            .border_type(BorderType::Thick)
            .border_style(Style::new().fg(Color::Green))
    } else {
        block
    }
}

/// A request raised by a widget that only `App` can carry out, because it
/// needs to reach a sibling widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Show this file in the file view, reading all of it.
    Load(PathBuf),
    /// Show enough of this file to fill the pane, as the selection passes over
    /// it. Bounded so that holding a cursor key stays responsive.
    Preview(PathBuf),
}

/// Which of the three panes is receiving input.
///
/// The panes themselves are three named fields on `App`, so this names one
/// rather than indexing a collection. That is the whole difference from the
/// `Vec<AppWidget>` this replaced: "the file view" is a value you can write
/// down, not a position you have to go and find, so the twenty linear scans
/// that used to search for a variant are field reads (#73).
///
/// `Nav` is the default because the navigator is where a session starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Nav,
    View,
    Filters,
}

impl Focus {
    /// The next pane in the `Tab` cycle, wrapping.
    ///
    /// Written out rather than derived from a count: the cycle order is a
    /// deliberate left-to-right, top-to-bottom reading of the layout, and an
    /// arithmetic version would silently reorder if the variants were ever
    /// rearranged for an unrelated reason.
    pub fn next(self) -> Self {
        match self {
            Self::Nav => Self::View,
            Self::View => Self::Filters,
            Self::Filters => Self::Nav,
        }
    }
}

/// What a keypress in the filter pane asks `App` to do.
///
/// `FilterList` cannot mutate the `ActiveFilters` it only borrows for rendering,
/// so it reports what the user asked for and lets `App` — the set's owner —
/// carry it out. This is not carried on `Action`: that enum is about a
/// widget asking `App` to show a *file* (`Load`/`Preview`), and a filter
/// request is a different kind of thing that would only muddy it.
///
/// The line this enum draws is **"needs the pane's selection"**, not "mutates
/// the set". `i` and `x` are not variants here even though they do end in a
/// mutation, because they address no row — they are `App`'s own keys that
/// happen to be typed while this pane has focus, and `App` handles them
/// directly. `Edit` is a variant despite only opening a prompt, because it
/// asks about *the selected row*, and the row-to-filter mapping lives in this
/// module (see `filter_index_for_row`, which is deliberately the single place
/// that translation happens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCommand {
    Toggle(usize),
    Delete(usize),
    /// Reopen the prompt over this filter's existing pattern, to overwrite it
    /// in place. The one variant `App` answers by opening a prompt rather than
    /// by changing the set — nothing is mutated until that prompt commits.
    Edit(usize),
    /// The search row, which carries no index: the live search lives in its
    /// own slot on the `ActiveFilters`, not at a position in `filters`.
    ToggleSearch,
    DeleteSearch,
    EditSearch,
}

// There is deliberately no `Widget` impl covering all three panes.
//
// One of them could never satisfy it: `FilterList::render` needs a borrowed
// `ActiveFilters`, `App` owns the only true set, and a copy held beside the
// pane could go stale the moment a filter changed. The old `AppWidget` enum
// implemented `Widget` anyway and left that variant drawing nothing behind a
// `debug_assert!(false)`, with a free `render_widget` function existing purely
// to route around the impl it could not use (#75).
//
// `App::render` now calls each pane directly with the arguments that pane
// actually needs, so the routing is structural rather than a convention a
// runtime assertion has to police.
