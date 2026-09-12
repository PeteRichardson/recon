//! The in-app keymap overlay, and the single table it draws (#25).
//!
//! # Why one table, and why it is tested against the source
//!
//! Before this module a binding had to be right in two places — the match arms
//! and the README — and the README's own Keybindings section admitted it,
//! naming `App::handle_event` as the authoritative source. A help view drawn
//! from a third hand-maintained list would have made that three, and the drift
//! between them is silent: nothing fails, the help just quietly starts lying.
//!
//! So `KEYMAP` below is the one list this crate keeps.
//!
//! # The drift test used to scrape source text; now it compares tables
//!
//! Earlier phases of #25 had a test, `every_bound_key_is_documented`, that
//! read the *source files* back at test time and failed when a key bound in
//! a `KeyCode::…` / `Key::…` arm — a character in `Char(..)`, or a named key
//! such as `PageDown` or `BackTab` — was not named by any row here. That
//! scan could only see a key spelled as a literal `match` arm, which is
//! exactly what this phase (moving every binding into `keymap::DEFAULT`)
//! stopped doing: after the match arms were replaced there was nothing left
//! for the scrape to find, so task 9 deleted it along with the tests that
//! only existed to prove the scrape itself worked.
//!
//! `keymap::the_table_and_the_documentation_agree` (`src/keymap.rs`) is what
//! replaces it: it compares `keymap::DEFAULT` against `KEYMAP` directly,
//! entry by entry, rather than grepping source text. That is a stronger
//! check in one respect the scrape never covered — it also verifies a name
//! sits on the *right* row (#59's `n`/`N`-in-two-scopes bug is what proved
//! that gap) — but it gives up something the scrape could see: a key bound
//! in code but missing from the table entirely, since a table-to-table
//! comparison cannot notice an absence that never became a row on either
//! side.
//!
//! Losing that is accepted, not overlooked, for three reasons. Every key a
//! user's raw keypress can resolve to now starts at `keymap::DEFAULT`, so an
//! undocumented binding — a keypress the table does not name, reaching a
//! pane some other way — cannot occur by construction. That is not the same
//! as "no `match` arm binds a key any more": `FileView::handle_events` still
//! has roughly sixteen, but each is a forwarding target `App::perform`
//! reaches with a canonical key it reconstructs from the table, not a second
//! place a raw keypress can land — the one exception, an unbound *modified*
//! key falling through to one of them directly, is a known, separately
//! tracked gap (see the comment on the `Scope::View` intercept in
//! `App::dispatch_event`), not a silent one. The other remaining non-table
//! arms — the filter pane's `h`/`l` hints, the prompt's three non-binding
//! arms, the help overlay's any-key dismissal, the bounce guard — are
//! deliberate and documented the same way. And #162 already records this
//! exact scan silently finding nothing once before, so its guarantee was
//! weaker in practice than it looked on paper.
//!
//! It still does not catch the reverse (a row describing a key that no
//! longer exists), and it deliberately says nothing about the README — that
//! stays hand-maintained. Generating the README section from `KEYMAP` is the
//! obvious next step and is not taken here.

use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style};
use ratatui::widgets::{Block, Clear, Widget};

/// Columns between two rendered columns of the overlay.
const GUTTER: usize = 3;

/// Columns between a row's keys and its description.
const KEY_GAP: usize = 2;

/// What a row shows when nothing reaches the action it documents (#61).
///
/// A `[keymap]` line can leave an action with no key at all. The row stays
/// and says so: dropping it would hide an action recon still has, and an
/// empty keys column would read as a rendering fault rather than as a fact
/// about the config. A word where a key label goes is a shape the table
/// already uses — `printable` is one.
const UNBOUND: &str = "unbound";

/// One row of the overlay: the keys that do a thing, and the thing.
pub struct Binding {
    /// One label per key that triggers this row, rendered joined by ` / `.
    ///
    /// A list rather than a pre-joined string so `codes` can derive the bound
    /// characters from the very same data that gets drawn. A separate
    /// machine-readable field would be a second thing to keep in step, which is
    /// the class of problem this module exists to remove.
    ///
    /// These are the *default* labels. What the overlay draws is each of them
    /// replaced by whatever the keymap in force binds in its place, so a
    /// rebound action shows the key the user has rather than this one — see
    /// `keys_for` (#61). A row whose `names` is empty is drawn exactly as
    /// spelled here.
    pub keys: &'static [&'static str],
    pub action: &'static str,
    /// The actions this row documents. Empty for a row that documents no
    /// single action — a chain, a reserved key, `printable` in a prompt.
    ///
    /// A list, not one name, because a row can bind more than one action at
    /// once: "Shared motions" documents `j`/`k` once for three panes, and
    /// that one row names `nav.up`, `view.up` and `filters.up` together
    /// rather than splitting into three rows and losing the "documented
    /// once" shape the module is built around.
    ///
    /// The same strings the config file will use, so the help overlay, the
    /// README and a user's `[keymap]` all spell an action one way.
    pub names: &'static [&'static str],
}

/// A key as the drift test sees it: what a `KeyCode::…` / `Key::…` arm binds
/// and what a `KEYMAP` label names, in one currency so the two can be
/// compared (#162).
///
/// `Named` carries the `KeyCode` variant's identifier — `PageDown`, `BackTab`,
/// `Esc` — which is the spelling both the source and the labels have to
/// agree on. `F` stands for every function key: `KeyCode::F(n)` is one arm
/// whichever `n` it matches.
///
/// No longer test-only (#59): `label_matches` needs it too, to answer whether
/// a `DEFAULT` label names the key that was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Char(char),
    Named(&'static str),
}

/// The `KeyCode` variants a label may name, spelled as the variants are.
/// `Shift-Tab` is the one label that maps elsewhere: crossterm reports it as
/// `BackTab`, not `Tab` with a modifier.
const NAMED_KEYS: &[&str] = &[
    "Backspace",
    "Enter",
    "Left",
    "Right",
    "Up",
    "Down",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Tab",
    "BackTab",
    "Delete",
    "Insert",
    "Esc",
];

impl Binding {
    /// The keys this row documents, for the drift test.
    ///
    /// A label is a character binding when it is a single character, or
    /// `Ctrl-` plus one — `Ctrl-e` documents `Key::Char('e')` with the modifier
    /// held. `space` is spelled out because a bare ` ` in a table reads as an
    /// empty cell. A label spelled like a `KeyCode` variant (`Tab`, `Enter`,
    /// `PageDown`) documents that named key; `Shift-Tab` documents `BackTab`,
    /// which is what the terminal actually reports; `F1`…`F12` document the
    /// one `F(_)` arm. Everything else — `printable`, a chain such as `f i` —
    /// names no single key and yields nothing.
    ///
    /// Still test-only (#59 review): `label_matches` calls `keys_for_label`
    /// directly rather than through a `Binding`, so this stays exactly what
    /// it was — the drift test's own parser, now shared with `label_matches`
    /// only at the `keys_for_label` level.
    #[cfg(test)]
    fn codes(&self) -> impl Iterator<Item = Key> + '_ {
        self.keys.iter().flat_map(|label| keys_for_label(label))
    }
}

/// The keys one label documents, by the grammar `Binding::codes` and
/// `label_matches` both need.
///
/// Pulled out of `Binding::codes` (#59) so `label_matches` can parse a single
/// label without a `Binding` to hang it on — a label handed to `resolve` at
/// runtime has no `'static` home to put it in, and this needs none.
fn keys_for_label(label: &str) -> Vec<Key> {
    if label == "space" {
        return vec![Key::Char(' ')];
    }
    if label == "Shift-Tab" {
        return vec![Key::Named("BackTab")];
    }
    let bare = label
        .strip_prefix("Ctrl-")
        .or_else(|| label.strip_prefix("Alt-"))
        .unwrap_or(label);
    if let Some(named) = NAMED_KEYS.iter().find(|named| **named == bare) {
        return vec![Key::Named(named)];
    }
    if bare.len() > 1 && bare.starts_with('F') && bare[1..].chars().all(|c| c.is_ascii_digit()) {
        return vec![Key::Named("F")];
    }
    let chars: Vec<char> = bare.chars().collect();
    match chars.as_slice() {
        [c] => vec![Key::Char(*c)],
        // `1-9`: one label, nine keys. Only for a bare range — a `Ctrl-`
        // prefix was stripped above, so `Ctrl-d` is `d`.
        [a, '-', b] if a < b => (*a..=*b).map(Key::Char).collect(),
        _ => Vec::new(),
    }
}

/// Whether the label grammar can read this spelling at all.
///
/// `label_matches` answers "does this label name that key", and a label the
/// grammar cannot read answers `false` for every key — indistinguishable
/// there from a key that simply does not match. A `[keymap]` line spelled
/// `Mod-Q` has to be refused rather than bound to nothing at all (#61), so
/// whether the grammar reads it is asked separately, here.
pub(crate) fn label_is_readable(label: &str) -> bool {
    !keys_for_label(label).is_empty()
}

/// Whether a `KEYMAP` label names this key.
///
/// The label grammar is the one `Binding::codes` already parses, so the
/// documentation, the table and a user's config file all spell a key the same
/// way. `Ctrl-` and `Alt-` prefixes set the two modifiers the table keeps;
/// everything else is the bare key.
pub(crate) fn label_matches(label: &str, key: crate::keymap::Key) -> bool {
    let ctrl = label.starts_with("Ctrl-");
    let alt = label.starts_with("Alt-");
    if ctrl != key.ctrl || alt != key.alt {
        return false;
    }
    keys_for_label(label)
        .into_iter()
        .any(|documented| match (documented, key.code) {
            (Key::Char(c), crossterm::event::KeyCode::Char(pressed)) => c == pressed,
            (Key::Named(name), code) => named_matches(name, code),
            _ => false,
        })
}

/// Whether `code` is the `KeyCode` variant `NAMED_KEYS` spells as `name`.
///
/// Mechanical: one arm per `NAMED_KEYS` entry, plus `F` for every `F(_)`
/// function key regardless of which number.
fn named_matches(name: &str, code: crossterm::event::KeyCode) -> bool {
    use crossterm::event::KeyCode;
    matches!(
        (name, code),
        ("Backspace", KeyCode::Backspace)
            | ("Enter", KeyCode::Enter)
            | ("Left", KeyCode::Left)
            | ("Right", KeyCode::Right)
            | ("Up", KeyCode::Up)
            | ("Down", KeyCode::Down)
            | ("Home", KeyCode::Home)
            | ("End", KeyCode::End)
            | ("PageUp", KeyCode::PageUp)
            | ("PageDown", KeyCode::PageDown)
            | ("Tab", KeyCode::Tab)
            | ("BackTab", KeyCode::BackTab)
            | ("Delete", KeyCode::Delete)
            | ("Insert", KeyCode::Insert)
            | ("Esc", KeyCode::Esc)
            | ("F", KeyCode::F(_))
    )
}

/// A headed group of bindings — one per pane, plus the global set.
pub struct Section {
    pub title: &'static str,
    pub bindings: &'static [Binding],
}

/// Every binding recon has, in the order the overlay draws them.
///
/// Flat rather than context-sensitive: the overlay shows all of it whichever
/// pane has focus. Narrowing it to the focused pane is a real improvement and
/// deliberately deferred — see #25, where it was weighed and put off.
///
/// The sections after Global are the layers of #120, in the order they are
/// checked, and a motion shared by more than one pane is documented once in
/// "Shared motions" rather than once per pane.
pub const KEYMAP: &[Section] = &[
    Section {
        title: "Global",
        bindings: &[
            Binding {
                keys: &["?"],
                action: "This help — any key closes it",
                names: &["global.help"],
            },
            Binding {
                keys: &["q"],
                action: "Quit",
                names: &["global.quit"],
            },
            Binding {
                keys: &["Q"],
                action: "Quit, emitting nothing",
                names: &["global.quit.silent"],
            },
            Binding {
                keys: &["Tab", "Shift-Tab"],
                action: "Focus the next / previous pane",
                names: &["global.focus.next", "global.focus.prev"],
            },
            Binding {
                keys: &["e"],
                action: "Focus the navigator",
                names: &["global.focus.nav"],
            },
            Binding {
                keys: &["t"],
                action: "Focus the file view",
                names: &["global.focus.view"],
            },
            Binding {
                keys: &["f"],
                action: "Focus the filter pane; f i / f x / f c return on commit",
                names: &["global.focus.filters"],
            },
            Binding {
                keys: &["/"],
                action: "Search — filenames, or file contents",
                names: &["global.search"],
            },
            Binding {
                keys: &["p"],
                action: "Promote the live search into the filter set",
                names: &["global.search.promote"],
            },
            Binding {
                keys: &["Esc"],
                action: "Clear the pane's search, else the live search",
                names: &["global.escape"],
            },
            Binding {
                keys: &["space"],
                action: "Peek at the plain file; press again to restore",
                names: &["global.peek"],
            },
            Binding {
                keys: &[".", ","],
                action: "Next / previous file the filters match",
                names: &["global.file.next", "global.file.prev"],
            },
            Binding {
                keys: &["[", "]"],
                action: "Page the file view up / down, from any pane",
                names: &["global.page.up", "global.page.down"],
            },
            Binding {
                keys: &["1-9"],
                action: "Toggle the filter with that number",
                names: &["global.filters.toggle"],
            },
            Binding {
                keys: &["u", "Ctrl-h", "H"],
                action: "Dim unmatched lines, or hide them",
                names: &["global.toggle.hide"],
            },
            Binding {
                keys: &["!"],
                action: "Disable every filter, or put them back",
                names: &["global.filters.disable"],
            },
            Binding {
                keys: &["&"],
                action: "AND the include filters instead of OR, or back",
                names: &["global.filters.and"],
            },
            Binding {
                keys: &["b"],
                action: "Hide the left column, and focus the file view",
                names: &["global.zoom.view"],
            },
            Binding {
                keys: &["z"],
                action: "Maximise the focused pane, or restore the split",
                names: &["global.zoom.focused"],
            },
            Binding {
                keys: &["o"],
                action: "Open the file's project in your editor",
                names: &["global.editor.project"],
            },
            Binding {
                keys: &["O"],
                action: "Open the file alone in your editor",
                names: &["global.editor.file"],
            },
            Binding {
                keys: &["r"],
                action: "Refresh from disk — rescan the listing, reload the file",
                names: &["global.reload"],
            },
            // `names: &[]`: reserved for 1.1 (#242), bound to nothing, so
            // there is no action to name — see `keymap::RESERVED`.
            Binding {
                keys: &["-"],
                action: "Reserved — the hex view, in a later release",
                names: &[],
            },
            Binding {
                keys: &[":"],
                action: "Reserved — a command palette, in a later release",
                names: &[],
            },
        ],
    },
    Section {
        title: "Chains",
        bindings: &[
            Binding {
                keys: &["f i", "f x"],
                action: "Add an including / excluding filter; returns on commit",
                names: &[],
            },
            Binding {
                keys: &["f c"],
                action: "Change the selected filter — returns on commit",
                names: &[],
            },
            Binding {
                keys: &["f d", "f Enter"],
                action: "Delete / toggle the selected filter — focus stays",
                names: &[],
            },
            Binding {
                keys: &["f f"],
                action: "Stay in the filter pane",
                names: &[],
            },
            Binding {
                keys: &["e n"],
                action: "Navigator's n — search hit, else next matching file",
                names: &[],
            },
            Binding {
                keys: &["t *"],
                action: "Search the word under the view's cursor",
                names: &[],
            },
        ],
    },
    Section {
        title: "Shared motions",
        bindings: &[
            // The arrows are in `keys`, not the action text, so the drift
            // test can see them (#162): `Up`/`Down` are bound in every pane
            // and were named by no row. The row is narrower for it.
            Binding {
                keys: &["j", "k", "Down", "Up"],
                action: "Down / up a row",
                names: &[
                    "nav.up",
                    "nav.down",
                    "view.up",
                    "view.down",
                    "filters.up",
                    "filters.down",
                ],
            },
            Binding {
                keys: &["g", "G"],
                action: "First / last row (also Home / End)",
                names: &[
                    "nav.goto.start",
                    "nav.goto.end",
                    "view.goto.start",
                    "view.goto.end",
                    "filters.goto.start",
                    "filters.goto.end",
                ],
            },
            Binding {
                keys: &["Ctrl-d", "Ctrl-u"],
                action: "Half a page down / up",
                names: &[
                    "nav.halfpage.down",
                    "nav.halfpage.up",
                    "view.halfpage.down",
                    "view.halfpage.up",
                    "filters.halfpage.down",
                    "filters.halfpage.up",
                ],
            },
            Binding {
                keys: &["PageDown", "PageUp"],
                action: "A page down / up",
                names: &[
                    "nav.page.down",
                    "nav.page.up",
                    "view.page.down",
                    "view.page.up",
                    "filters.page.down",
                    "filters.page.up",
                ],
            },
            Binding {
                keys: &["n", "N"],
                action: "Next / previous hit, or matching file in the navigator",
                names: &["hit.next", "hit.prev", "nav.hit.next", "nav.hit.prev"],
            },
            Binding {
                keys: &["Enter"],
                action: "Open entry; toggle filter, set or search; not the view",
                names: &["nav.open", "filters.toggle"],
            },
        ],
    },
    Section {
        title: "Navigator",
        bindings: &[
            Binding {
                keys: &["h", "Left"],
                action: "Up to the parent directory",
                names: &["nav.parent"],
            },
            Binding {
                keys: &["l", "Right"],
                action: "Open the entry",
                names: &["nav.open"],
            },
        ],
    },
    Section {
        title: "File view",
        bindings: &[
            Binding {
                keys: &["h", "Left"],
                action: "Cursor back",
                names: &["view.left"],
            },
            Binding {
                keys: &["l", "Right"],
                action: "Cursor forward",
                names: &["view.right"],
            },
            Binding {
                keys: &["w"],
                action: "Next word",
                names: &["view.word.forward"],
            },
            Binding {
                keys: &["0", "^"],
                action: "Start of the line",
                names: &["view.line.start"],
            },
            Binding {
                keys: &["$"],
                action: "End of the line",
                names: &["view.line.end"],
            },
            Binding {
                keys: &["{", "}"],
                action: "Previous / next paragraph",
                names: &["view.paragraph.prev", "view.paragraph.next"],
            },
            Binding {
                keys: &["#"],
                action: "Toggle the line-number gutter",
                names: &["view.toggle.linenumbers"],
            },
            Binding {
                keys: &["*"],
                action: "Search for the word under the cursor",
                names: &["global.search.word"],
            },
            Binding {
                keys: &["v", "V"],
                action: "Select by character / by line; again to end",
                names: &["global.visual.char", "global.visual.line"],
            },
            Binding {
                keys: &["y"],
                action: "Copy the selection to the clipboard",
                names: &["global.yank"],
            },
            Binding {
                keys: &["Esc"],
                action: "End the selection",
                names: &["global.escape"],
            },
            Binding {
                keys: &["Ctrl-e", "Ctrl-y"],
                action: "Scroll one line down / up",
                names: &["view.scroll.down", "view.scroll.up"],
            },
            Binding {
                keys: &["Ctrl-f", "Ctrl-b"],
                action: "A page down / up (aliases)",
                names: &["view.page.down", "view.page.up"],
            },
        ],
    },
    Section {
        title: "Filter pane",
        bindings: &[
            Binding {
                keys: &["i"],
                action: "Add an including filter",
                names: &["filters.include"],
            },
            Binding {
                keys: &["x"],
                action: "Add an excluding filter",
                names: &["filters.exclude"],
            },
            Binding {
                keys: &["c"],
                action: "Change the selected filter's pattern",
                names: &["filters.edit"],
            },
            Binding {
                keys: &["d"],
                action: "Delete the selected filter",
                names: &["filters.delete"],
            },
            Binding {
                keys: &["m"],
                action: "Toggle the selected filter between include and context",
                names: &["filters.context"],
            },
            Binding {
                keys: &["a"],
                action: "Pick a profile for the selected set",
                names: &["filters.profile"],
            },
            Binding {
                keys: &["s"],
                action: "Solo the selected set — or un-solo it",
                names: &["filters.solo"],
            },
            Binding {
                keys: &["R"],
                action: "Reset every set to its startup state",
                names: &["filters.reset"],
            },
            Binding {
                keys: &["S"],
                action: "Save the scratch filters as a named set",
                names: &["filters.save.set"],
            },
        ],
    },
    Section {
        // The picker's own j/k/Up/Down/Enter/Esc were bound but undocumented
        // until the table's per-action agreement test (#59) caught it — the
        // old drift test only compared keys, and those same keys already
        // appeared in rows for other panes, so nothing looked missing.
        title: "Profile picker",
        bindings: &[
            Binding {
                keys: &["j", "k", "Down", "Up"],
                action: "Down / up a row",
                names: &["picker.up", "picker.down"],
            },
            Binding {
                keys: &["Enter"],
                action: "Choose the profile set",
                names: &["picker.choose"],
            },
            Binding {
                keys: &["Esc"],
                action: "Cancel",
                names: &["picker.cancel"],
            },
        ],
    },
    Section {
        title: "While a prompt is open",
        bindings: &[
            Binding {
                keys: &["printable"],
                action: "Insert at the cursor",
                names: &[],
            },
            Binding {
                keys: &["Left", "Right"],
                action: "Move the cursor",
                names: &["prompt.left", "prompt.right"],
            },
            Binding {
                keys: &["Home", "Ctrl-a"],
                action: "Start of the pattern",
                names: &["prompt.start"],
            },
            Binding {
                keys: &["End", "Ctrl-e"],
                action: "End of the pattern",
                names: &["prompt.end"],
            },
            Binding {
                keys: &["Backspace"],
                action: "Delete before the cursor; cancel when empty",
                names: &["prompt.delete.back"],
            },
            Binding {
                keys: &["Delete"],
                action: "Delete under the cursor",
                names: &["prompt.delete.forward"],
            },
            Binding {
                keys: &["Ctrl-w"],
                action: "Delete the word before the cursor",
                names: &["prompt.delete.word"],
            },
            Binding {
                keys: &["Ctrl-u"],
                action: "Delete everything before the cursor",
                names: &["prompt.delete.start"],
            },
            Binding {
                keys: &["Enter"],
                action: "Run the search, or add the filter",
                names: &["prompt.commit"],
            },
            Binding {
                keys: &["Esc"],
                action: "Cancel",
                names: &["prompt.cancel"],
            },
        ],
    },
];

/// One line of the laid-out overlay.
enum Row<'a> {
    Heading(&'a str),
    Blank,
    Entry { keys: String, action: &'a str },
}

/// Flatten `KEYMAP` into the lines the overlay draws, in order.
fn rows(keymap: &crate::keymap::Keymap) -> Vec<Row<'static>> {
    // The compiled-in bindings, built once for the whole pass: `keys_for`
    // needs them to know what a literal label has been replaced by, and
    // asking the same `labels_for` for both sides is what makes a default
    // keymap render byte for byte as the table spells it.
    let defaults = crate::keymap::Keymap::default();
    let mut rows = Vec::new();
    for (i, section) in KEYMAP.iter().enumerate() {
        if i > 0 {
            rows.push(Row::Blank);
        }
        rows.push(Row::Heading(section.title));
        for binding in section.bindings {
            rows.push(Row::Entry {
                keys: keys_for(binding, keymap, &defaults),
                action: binding.action,
            });
        }
    }
    rows
}

/// The keys one row shows: its own labels, each replaced by whatever the
/// keymap in force put in its place (#61).
///
/// Substituting label by label, rather than listing everything the row's
/// actions are now bound to, because `keys` is a curated list and the overlay
/// depends on it staying curated. `nav.goto.start` also binds `Home`, which
/// the shared row deliberately spells in prose instead of in its keys to stay
/// inside the layout budget, and `global.toggle.hide` orders its three keys
/// differently here than `keymap::DEFAULT` does. Rebuilding a row's keys from
/// the keymap would silently undo both.
fn keys_for(
    binding: &Binding,
    keymap: &crate::keymap::Keymap,
    defaults: &crate::keymap::Keymap,
) -> String {
    // A row that documents no action — a chain, `printable`, a reserved key
    // (#242) — has nothing to look up, and its label is the whole point of
    // the row.
    if binding.names.is_empty() {
        return binding.keys.join(" / ");
    }

    let mut shown: Vec<String> = Vec::new();
    for key in binding.keys {
        for label in replacements(key, binding.names, keymap, defaults) {
            if !shown.contains(&label) {
                shown.push(label);
            }
        }
    }
    if shown.is_empty() {
        return UNBOUND.to_string();
    }
    shown.join(" / ")
}

/// What now reaches the actions `key` reached by default on this row.
///
/// One key can stand for one action per pane: the shared row's `k` is
/// `nav.up`, `view.up` and `filters.up` at once. So every name on the row is
/// asked, and each answers with the label sitting where `key` sits in its own
/// default list — a config that rebinds one pane's motion leaves the row
/// documenting two keys, and both are true.
///
/// An action whose new list is shorter than `key`'s position, or empty,
/// contributes nothing: that key no longer reaches it. A key no name on the
/// row binds by default is kept as it is — nothing claims it, so there is
/// nothing to substitute.
fn replacements(
    key: &str,
    names: &[&str],
    keymap: &crate::keymap::Keymap,
    defaults: &crate::keymap::Keymap,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut claimed = false;
    for name in names {
        // `None` only if a row names an action that does not exist, which
        // `keymap::tests::the_table_and_the_documentation_agree` forbids.
        let Some(action) = crate::keymap::action_named(name) else {
            continue;
        };
        let Some(index) = defaults
            .labels_for(action)
            .iter()
            .position(|label| *label == key)
        else {
            continue;
        };
        claimed = true;
        if let Some(label) = keymap.labels_for(action).get(index) {
            found.push((*label).to_string());
        }
    }
    if claimed {
        found
    } else {
        vec![key.to_string()]
    }
}

/// Draw the keymap over `area`, hiding whatever was under it.
///
/// The rows flow into as many columns as it takes to fit the height, which is
/// what lets the whole keymap sit on one screen with nothing to scroll — and
/// scrolling is what would break "any key closes it", since the keys that
/// scrolled would have to be exempt from it.
///
/// A terminal too small even for the widest layout gets a truncated list and a
/// count of what was cut, in the bottom border. Silently dropping rows from a
/// reference would be the worse failure of the two.
pub fn render(area: Rect, buf: &mut Buffer, keymap: &crate::keymap::Keymap) {
    let rows = rows(keymap);
    // Laid out against everything available, then the panel shrunk to what the
    // layout actually used. Doing it the other way round — sizing the panel
    // first — would make the column count depend on a height chosen before the
    // rows were flowed into it.
    let columns = layout(&rows, Block::bordered().inner(area));
    let area = panel(area, &columns);

    // The panes are already drawn underneath; without this their borders and
    // text show through wherever the overlay writes nothing.
    Clear.render(area, buf);
    let inner = Block::bordered().inner(area);

    let mut block = Block::bordered().title(" Keys ");
    let shown = shown(&columns);
    if shown < rows.len() {
        // Rows, not bindings: a heading that fell off the end is just as much
        // a thing the reader cannot see.
        let missing = rows.len() - shown;
        block = block.title_bottom(format!(" {missing} more rows — see the README "));
    }
    block.render(area, buf);

    for column in &columns {
        let x = inner.x + u16::try_from(column.x).unwrap_or(u16::MAX);
        let action_x = x + u16::try_from(column.key_width + KEY_GAP).unwrap_or(u16::MAX);
        for (offset, row) in rows[column.start..column.start + column.len]
            .iter()
            .enumerate()
        {
            let y = inner.y + u16::try_from(offset).unwrap_or(u16::MAX);
            let room = usize::from(inner.right().saturating_sub(x));
            match row {
                Row::Blank => {}
                Row::Heading(title) => {
                    buf.set_stringn(
                        x,
                        y,
                        title,
                        room,
                        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                    );
                }
                Row::Entry { keys, action } => {
                    buf.set_stringn(x, y, keys, room, Style::new().fg(Color::Yellow));
                    let action_room = usize::from(inner.right().saturating_sub(action_x));
                    buf.set_stringn(action_x, y, action, action_room, Style::new());
                }
            }
        }
    }
}

/// The bordered panel `columns` needs, centred in `area` and never larger.
///
/// Centred and shrunk rather than filling the frame: the keymap is about forty
/// rows, and on a tall terminal the difference is twenty rows of empty bordered
/// box. Leaving the panes visible around the edges also keeps it obvious that
/// the overlay is something you are looking *at*, not somewhere you have gone.
///
/// The title has no say in the width. `" Keys "` is six columns and the
/// narrowest useful panel is far wider, so a clamp for it would be dead code
/// everywhere except a terminal where the keymap is unreadable anyway.
fn panel(area: Rect, columns: &[Column]) -> Rect {
    // Two for the borders on each axis.
    let width = u16::try_from(total_width(columns) + 2)
        .unwrap_or(u16::MAX)
        .min(area.width);
    let height = u16::try_from(columns.iter().map(|column| column.len).max().unwrap_or(0) + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// One rendered column: which slice of the rows it holds, and how wide that
/// slice made it.
struct Column {
    /// Index into the row list of this column's first row.
    start: usize,
    len: usize,
    /// Offset from the left of the overlay's inner area.
    x: usize,
    /// Widest key list *in this column* — what its description column is
    /// indented by.
    key_width: usize,
    width: usize,
}

/// Rows the layout can actually draw.
fn shown(columns: &[Column]) -> usize {
    columns.iter().map(|column| column.len).sum()
}

/// Columns from the left edge of the first to the right edge of the last.
fn total_width(columns: &[Column]) -> usize {
    columns.last().map_or(0, |column| column.x + column.width)
}

/// Flow `rows` into the fewest columns that fit `inner`'s height, sizing each
/// column to its own content.
///
/// Balanced by row count rather than packed section by section: greedy packing
/// leaves a column nearly empty whenever the next section is one row too tall,
/// and the sections here differ enough in size for that to be the common case
/// rather than the rare one.
///
/// Sizing each column to its own widest row, rather than every column to the
/// table's widest, is what makes the keymap fit a normal terminal at all — one
/// forty-five-character description in the global section would otherwise set
/// the width of the column holding `Cursor up`. Which rows land in which column
/// depends on the column count, so the widths cannot be known before the count
/// is chosen: the count is chosen from the height, the widths measured, and the
/// count walked back down if they do not fit. Walking back down costs rows off
/// the bottom, which is why it only happens on a terminal too narrow to hold
/// the columns the height asked for.
fn layout(rows: &[Row<'_>], inner: Rect) -> Vec<Column> {
    let height = usize::from(inner.height).max(1);
    let width = usize::from(inner.width);
    let mut count = rows.len().div_ceil(height).max(1);
    loop {
        let laid = pack(rows, count, height);
        // One column is the floor: fewer would show nothing at all, and a
        // single column too wide for the area is clipped by `set_stringn`
        // rather than being a reason to give up.
        if total_width(&laid) <= width || count == 1 {
            return laid;
        }
        count -= 1;
    }
}

/// Split `rows` into `count` balanced columns, each capped at `height` rows.
///
/// The cap is what drops rows when `count` has been walked down below what the
/// height wanted; `shown` is how the caller learns it happened.
fn pack(rows: &[Row<'_>], count: usize, height: usize) -> Vec<Column> {
    let per_column = rows.len().div_ceil(count).min(height).max(1);
    let mut columns: Vec<Column> = Vec::new();
    let mut x = 0;
    for index in 0..count {
        let start = index * per_column;
        if start >= rows.len() {
            break;
        }
        let len = per_column.min(rows.len() - start);
        let slice = &rows[start..start + len];
        let key_width = slice
            .iter()
            .filter_map(|row| match row {
                Row::Entry { keys, .. } => Some(keys.chars().count()),
                Row::Heading(_) | Row::Blank => None,
            })
            .max()
            .unwrap_or(0);
        let action_width = slice
            .iter()
            .map(|row| match row {
                Row::Entry { action, .. } => action.chars().count(),
                Row::Heading(title) => title.chars().count(),
                Row::Blank => 0,
            })
            .max()
            .unwrap_or(0);
        let width = key_width + KEY_GAP + action_width;
        columns.push(Column {
            start,
            len,
            x,
            key_width,
            width,
        });
        x += width + GUTTER;
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `codes` derives from the labels that get drawn, so a label shape it
    /// cannot read would quietly shrink the documented set.
    #[test]
    fn a_labels_bound_character_is_derived_from_how_it_is_drawn() {
        let binding = Binding {
            keys: &[
                "Ctrl-e",
                "H",
                "space",
                "PageDown",
                "Shift-Tab",
                "F3",
                "f i",
                "printable",
            ],
            action: "irrelevant",
            names: &[],
        };

        assert_eq!(
            binding.codes().collect::<Vec<_>>(),
            vec![
                Key::Char('e'),
                Key::Char('H'),
                Key::Char(' '),
                Key::Named("PageDown"),
                Key::Named("BackTab"),
                Key::Named("F"),
            ],
            "a key label was read as the wrong key"
        );
    }

    #[test]
    fn a_range_label_documents_every_key_in_it() {
        let binding = Binding {
            keys: &["1-9"],
            action: "",
            names: &[],
        };
        let codes: Vec<Key> = binding.codes().collect();
        assert_eq!(codes, ('1'..='9').map(Key::Char).collect::<Vec<_>>());

        // `Ctrl-d` is not a range: one key, `d`.
        let binding = Binding {
            keys: &["Ctrl-d"],
            action: "",
            names: &[],
        };
        assert_eq!(binding.codes().collect::<Vec<_>>(), vec![Key::Char('d')]);
    }

    /// The overlay is the model (#120 §15): one section per layer, in the
    /// order the layers are checked, and a shared motion documented once
    /// rather than once per pane.
    #[test]
    fn sections_follow_the_layer_model() {
        let titles: Vec<&str> = KEYMAP.iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            [
                "Global",
                "Chains",
                "Shared motions",
                "Navigator",
                "File view",
                "Filter pane",
                "Profile picker",
                "While a prompt is open",
            ]
        );

        // A shared motion appears in exactly one section outside Global and
        // the prompt: the shared one. Counting sections, not rows, so a key
        // that legitimately has two rows in one section is not a failure.
        //
        // `Home` and `End` are not in this list: the shared row spells them
        // out in its action text ("also Home / End") rather than the `keys`
        // array, to keep the joined key label short enough for the 150-column
        // layout budget, so `contains(&"Home")` would find nothing.
        //
        // "Profile picker" is excluded along with "Global" and the prompt: it
        // is a modal scope like the prompt, not a layered pane, and its own
        // j/k/Enter happen to reuse these labels for unrelated actions
        // (#59's agreement test is what surfaced that the picker needed a
        // section here at all).
        let shared = [
            "j", "k", "g", "G", "Ctrl-d", "Ctrl-u", "PageDown", "PageUp", "n", "N", "Enter",
        ];
        for key in shared {
            let sections: Vec<&str> = KEYMAP
                .iter()
                .filter(|s| {
                    s.title != "Global"
                        && s.title != "While a prompt is open"
                        && s.title != "Profile picker"
                })
                .filter(|s| s.bindings.iter().any(|b| b.keys.contains(&key)))
                .map(|s| s.title)
                .collect();
            assert_eq!(
                sections,
                ["Shared motions"],
                "{key} is documented in {sections:?}"
            );
        }

        // Pane verbs stay in their pane.
        let pane_only = [
            ("h", "Navigator"),
            ("l", "Navigator"),
            ("i", "Filter pane"),
            ("*", "File view"),
        ];
        for (key, pane) in pane_only {
            let sections: Vec<&str> = KEYMAP
                .iter()
                .filter(|s| s.bindings.iter().any(|b| b.keys.contains(&key)))
                .map(|s| s.title)
                .collect();
            assert!(
                sections.contains(&pane),
                "{key} is not documented in {pane}: {sections:?}"
            );
        }
    }

    fn inner(width: u16, height: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// The case the whole layout exists for. 150 and 44 are inner columns and
    /// rows — the area handed to `layout`, already inside the border, so 44
    /// inner rows is a 46-row terminal. A 150x44 terminal is unremarkable, and
    /// the keymap is the one screen where "most of it" is not good enough —
    /// the row you cannot see is exactly the one you opened it to find.
    ///
    /// It fails when the columns are sized against the *widest* row in the
    /// whole table rather than the widest in each column: two columns of the
    /// global maximum need 151 columns, while correct per-column sizing needs
    /// only 150. This area's width sits between them so the test fails on the
    /// regression but passes on the correct layout.
    ///
    /// There is no margin left below: 150 is what the correct layout fills
    /// the 150-column area with, exactly — the "Profile picker" section (#59)
    /// used up the two columns of slack this area used to have below the
    /// correct fill (it was 148). The one column of slack above, before the
    /// regression's 151, is unchanged. So a `KEYMAP` row that widens a column
    /// any further has nowhere left to go without this area's width growing
    /// past 151 — which would stop the second test here from failing on the
    /// regression it exists to catch — and a row that adds to *either* total
    /// has to be re-measured against both. The height, 44, is likewise the
    /// exact number of rows two columns hold at the current row count (88,
    /// after the two reserved-key rows of #242 raised it from 86); adding a
    /// row without raising the height would drop it off the bottom, which is
    /// a `shown(&columns) == rows.len()` failure below, not a width one.
    #[test]
    fn a_normal_terminal_shows_the_whole_keymap() {
        let rows = rows(&crate::keymap::Keymap::default());

        let columns = layout(&rows, inner(150, 44));

        assert_eq!(shown(&columns), rows.len(), "the keymap did not fit");
        assert!(
            total_width(&columns) <= 150,
            "the columns overflowed the area they were fitted to"
        );
    }

    /// Each column is sized to its own content, so a column of short
    /// descriptions does not pay for a long one three columns over.
    #[test]
    fn a_column_is_sized_to_its_own_widest_row() {
        let rows = rows(&crate::keymap::Keymap::default());

        let columns = layout(&rows, inner(150, 43));

        assert!(columns.len() > 1, "the table was not split into columns");
        assert!(
            columns.iter().map(|column| column.width).min()
                != columns.iter().map(|column| column.width).max(),
            "every column came out the same width, so one global maximum was used"
        );
    }

    /// Where the overlay's borders sit in a freshly rendered frame.
    fn border_box(buf: &Buffer, area: Rect) -> Rect {
        let corner = |glyph: &str| {
            (0..area.height)
                .flat_map(|y| (0..area.width).map(move |x| (x, y)))
                .find(|&(x, y)| buf[(x, y)].symbol() == glyph)
                .unwrap_or_else(|| panic!("no {glyph} corner in the frame"))
        };
        let (left, top) = corner("┌");
        let (right, bottom) = corner("┘");
        Rect {
            x: left,
            y: top,
            width: right - left + 1,
            height: bottom - top + 1,
        }
    }

    /// A panel sized to the keymap and centred, not a full-screen wash. The
    /// keymap is about sixty rows; on a sixty-four-row terminal the
    /// difference is still visible as empty bordered box.
    #[test]
    fn the_overlay_is_a_centred_panel() {
        // Tall enough for one column with room to spare, however many rows
        // the keymap grows to: the property under test is that the panel
        // shrinks to its content, which needs content smaller than the screen.
        let height = u16::try_from(rows(&crate::keymap::Keymap::default()).len() + 10)
            .expect("a keymap of sane size");
        let area = inner(170, height);
        let mut buf = Buffer::empty(area);

        render(area, &mut buf, &crate::keymap::Keymap::default());

        let panel = border_box(&buf, area);
        assert!(
            panel.height < area.height,
            "the overlay stretched to the full height of the screen"
        );
        assert!(
            panel.width < area.width,
            "the overlay stretched to the full width of the screen"
        );
        assert_eq!(
            panel.x,
            (area.width - panel.width) / 2,
            "the panel is not centred horizontally"
        );
        assert_eq!(
            panel.y,
            (area.height - panel.height) / 2,
            "the panel is not centred vertically"
        );
    }

    /// The panel shrinks to its content; it must never grow past the area it
    /// was handed, however small that is.
    #[test]
    fn the_overlay_never_outgrows_a_small_area() {
        let area = inner(30, 8);
        let mut buf = Buffer::empty(area);

        render(area, &mut buf, &crate::keymap::Keymap::default());

        let panel = border_box(&buf, area);
        assert_eq!(panel.width, area.width);
        assert_eq!(panel.height, area.height);
    }

    /// The narrow case: one column, and the rows past the bottom are dropped
    /// rather than squeezed. `render` reports the count in the bottom border.
    #[test]
    fn a_short_area_cannot_show_every_row() {
        let rows = rows(&crate::keymap::Keymap::default());

        let columns = layout(&rows, inner(40, 5));

        assert_eq!(columns.len(), 1, "a 40-column area fitted a second column");
        assert_eq!(
            shown(&columns),
            5,
            "more rows were kept than there are rows"
        );
    }

    // ---- the overlay shows the keys in force (#61) -----------------------

    /// The keymap a `[keymap]` stanza of these `action = keys` lines builds.
    fn keymap(lines: &[(&str, &[&str])]) -> crate::keymap::Keymap {
        let bindings = lines
            .iter()
            .map(|(action, keys)| {
                (
                    (*action).to_string(),
                    keys.iter().map(|key| (*key).to_string()).collect(),
                )
            })
            .collect();
        crate::keymap::Keymap::new(&crate::config::KeymapConfig { bindings })
            .expect("the test's own bindings are valid")
    }

    /// The keys a row shows, addressed by the description printed beside them.
    fn keys_of(rows: &[Row<'_>], action: &str) -> String {
        rows.iter()
            .find_map(|row| match row {
                Row::Entry { keys, action: text } if *text == action => Some(keys.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no row describes {action:?}"))
    }

    /// `keys_of`, narrowed to one section: "Down / up a row" describes both
    /// the shared motions and the profile picker, so the heading is part of
    /// the address.
    fn keys_under(rows: &[Row<'_>], heading: &str, action: &str) -> String {
        let start = rows
            .iter()
            .position(|row| matches!(row, Row::Heading(title) if *title == heading))
            .unwrap_or_else(|| panic!("no {heading:?} section"));
        keys_of(&rows[start..], action)
    }

    const RELOAD: &str = "Refresh from disk — rescan the listing, reload the file";
    const HIDE: &str = "Dim unmatched lines, or hide them";

    /// Why this task exists: the overlay is the only place a user can see the
    /// keys they actually have — the README is static text and
    /// `--print-keymap` prints the defaults by design — so it must read the
    /// keymap in force rather than the table it was compiled from.
    #[test]
    fn a_rebound_action_shows_the_key_the_user_bound() {
        let rows = rows(&keymap(&[
            ("global.reload", &["F5"]),
            ("global.toggle.hide", &["U"]),
        ]));

        assert_eq!(
            keys_of(&rows, RELOAD),
            "F5",
            "the overlay still showed the compiled-in key"
        );
        // Three default keys replaced by one: the two the config dropped are
        // gone rather than left on the row reaching nothing.
        assert_eq!(keys_of(&rows, HIDE), "U");
    }

    /// A row carrying several names documents several actions, and each of
    /// its keys is substituted for itself alone: rebinding one does not
    /// disturb the rest of the row.
    #[test]
    fn a_row_of_several_actions_substitutes_key_by_key() {
        let rows = rows(&keymap(&[("global.file.prev", &["<"])]));

        assert_eq!(
            keys_of(&rows, "Next / previous file the filters match"),
            ". / <",
            "only the rebound half of the row may move"
        );
    }

    /// One key can stand for one action per pane. `nav.up` alone is rebound
    /// here, so `k` still reaches the file view and the filter pane — and the
    /// row reports both keys, because both are true. Showing only the
    /// navigator's `K` would tell two thirds of the row's readers to press a
    /// key that does nothing for them.
    #[test]
    fn a_shared_row_shows_every_key_that_still_reaches_it() {
        let rows = rows(&keymap(&[("nav.up", &["K"])]));

        assert_eq!(
            keys_under(&rows, "Shared motions", "Down / up a row"),
            "j / K / k / Down / Up"
        );
    }

    /// A `[keymap]` line may leave an action with no key at all. The row says
    /// so in words: dropping it would hide an action recon still has, and a
    /// blank keys column would read as a rendering fault (#61).
    #[test]
    fn an_action_left_unbound_says_so() {
        let rows = rows(&keymap(&[("global.reload", &[])]));

        assert_eq!(keys_of(&rows, RELOAD), "unbound");
    }

    /// A row that names no action has nothing to look up, and its label is
    /// the whole point of the row: the reserved keys (#242) are documented as
    /// taken and bound to nothing, and `printable` never was a key.
    #[test]
    fn a_row_that_names_no_action_keeps_its_literal_keys() {
        let rows = rows(&keymap(&[("global.reload", &["F5"])]));

        assert_eq!(
            keys_of(&rows, "Reserved — the hex view, in a later release"),
            "-"
        );
        assert_eq!(
            keys_of(&rows, "Reserved — a command palette, in a later release"),
            ":"
        );
        assert_eq!(keys_of(&rows, "Insert at the cursor"), "printable");
        assert_eq!(keys_of(&rows, "Stay in the filter pane"), "f f");
    }

    /// The defaults must render as the table spells them, to the byte. The
    /// layout tests above are calibrated against this text, and `keys` is a
    /// curated list — `nav.goto.start` also binds `Home`, which the shared
    /// row deliberately keeps out of its keys — so a rendering derived from
    /// the keymap must not rewrite it.
    #[test]
    fn the_default_keymap_renders_the_tables_own_keys() {
        let literal: Vec<String> = KEYMAP
            .iter()
            .flat_map(|section| section.bindings)
            .map(|binding| binding.keys.join(" / "))
            .collect();

        let rendered: Vec<String> = rows(&crate::keymap::Keymap::default())
            .into_iter()
            .filter_map(|row| match row {
                Row::Entry { keys, .. } => Some(keys),
                Row::Heading(_) | Row::Blank => None,
            })
            .collect();

        assert_eq!(rendered, literal);
    }
}
