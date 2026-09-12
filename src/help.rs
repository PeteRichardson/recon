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
//! So `KEYMAP` below is the one list this crate keeps, and
//! `every_bound_key_is_documented` reads the *source files* back at test time
//! and fails when a key bound in a `KeyCode::…` / `Key::…` arm — a character
//! in `Char(..)`, or a named key such as `PageDown` or `BackTab` — is not
//! named by any row here. That is the cheapest of the three options #25
//! weighed, and it catches the common case: a new binding added without being
//! documented.
//!
//! It does not catch the reverse (a row describing a key that no longer
//! exists), and it deliberately says nothing about the README — that stays
//! hand-maintained. Generating the README section from `KEYMAP` is the obvious
//! next step and is not taken here.

use ratatui::prelude::{Buffer, Color, Modifier, Rect, Style};
use ratatui::widgets::{Block, Clear, Widget};

/// Columns between two rendered columns of the overlay.
const GUTTER: usize = 3;

/// Columns between a row's keys and its description.
const KEY_GAP: usize = 2;

/// One row of the overlay: the keys that do a thing, and the thing.
pub struct Binding {
    /// One label per key that triggers this row, rendered joined by ` / `.
    ///
    /// A list rather than a pre-joined string so `codes` can derive the bound
    /// characters from the very same data that gets drawn. A separate
    /// machine-readable field would be a second thing to keep in step, which is
    /// the class of problem this module exists to remove.
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
fn rows() -> Vec<Row<'static>> {
    let mut rows = Vec::new();
    for (i, section) in KEYMAP.iter().enumerate() {
        if i > 0 {
            rows.push(Row::Blank);
        }
        rows.push(Row::Heading(section.title));
        for binding in section.bindings {
            rows.push(Row::Entry {
                keys: binding.keys.join(" / "),
                action: binding.action,
            });
        }
    }
    rows
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
pub fn render(area: Rect, buf: &mut Buffer) {
    let rows = rows();
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
    use std::collections::BTreeSet;

    /// Every source file that binds a key, paired with its path for the failure
    /// message. `App::handle_event` is the big one, but the panes bind their
    /// own keys too, and a table that only covered the globals would be exactly
    /// the half-truth #25 is about.
    const SOURCES: &[(&str, &str)] = &[
        ("src/lib.rs", include_str!("lib.rs")),
        // `long_range_target`'s `g`/`G`/`{`/`}` table, which moved here out of
        // `impl App` when the viewport was split off. Keys bound in it are
        // intercepted before the file view sees them — a third source, and one
        // this list did not follow (#95).
        ("src/viewport.rs", include_str!("viewport.rs")),
        ("src/widgets/filenav.rs", include_str!("widgets/filenav.rs")),
        (
            "src/widgets/fileview.rs",
            include_str!("widgets/fileview.rs"),
        ),
        (
            "src/widgets/filterlist.rs",
            include_str!("widgets/filterlist.rs"),
        ),
        // The profile picker's own `j`/`k`/`Up`/`Down`/`Enter`/`Esc` (#162).
        ("src/widgets/picker.rs", include_str!("widgets/picker.rs")),
    ];

    /// Where a source file's own test module begins. Everything after it is
    /// fixtures pressing keys by the hundred, and a `KeyCode::Char('z')` in
    /// an assertion is not a binding.
    ///
    /// The marker is the module header, not the bare attribute: `lib.rs`
    /// carries `#[cfg(test)] pub(crate) mod fixtures;` at line 263 of twelve
    /// thousand and `fileview.rs` opens with a `#[cfg(test)] use`, so a split
    /// on the attribute alone stopped the scan before either file's first
    /// binding. Every global key went unchecked, and the test kept passing
    /// because nothing happened to be undocumented (#162).
    const TEST_MODULE: &str = "\n#[cfg(test)]\nmod tests";

    /// The keys bound in `source`: every character in a `Char(..)` pattern,
    /// and every named `KeyCode::…` / `Key::…` variant — `Enter`, `PageDown`,
    /// `BackTab`, with `F(_)` read as `F`.
    ///
    /// Deliberately a scan rather than a regex, so `Char(c @ ('n' | 'N'))` —
    /// the shape `n`/`N` are actually written in — yields both characters
    /// instead of neither.
    fn bound_keys(source: &str) -> BTreeSet<Key> {
        let code = source.split(TEST_MODULE).next().unwrap_or(source);
        let mut found = BTreeSet::new();
        for prefix in ["KeyCode::", "Key::"] {
            for (start, matched) in code.match_indices(prefix) {
                let rest = &code[start + matched.len()..];
                let ident: &str = rest
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                if !ident.starts_with(|c: char| c.is_ascii_uppercase()) {
                    continue;
                }
                if ident != "Char" {
                    if let Some(named) = NAMED_KEYS.iter().find(|named| **named == ident) {
                        found.insert(Key::Named(named));
                    } else if ident == "F" {
                        found.insert(Key::Named("F"));
                    }
                    continue;
                }
                let Some(rest) = rest[ident.len()..].strip_prefix('(') else {
                    continue;
                };
                // The first `)` closes either the pattern itself (`Char('q')`)
                // or the or-pattern inside it (`Char(c @ ('n' | 'N'))`). Both
                // hold every character the arm binds.
                let end = rest.find(')').unwrap_or(rest.len());
                let mut chars = rest[..end].chars();
                while let Some(c) = chars.next() {
                    if c == '\'' {
                        if let Some(bound) = chars.next() {
                            found.insert(Key::Char(bound));
                        }
                        // Skip the closing quote so `'''` cannot be misread.
                        chars.next();
                    }
                }
            }
        }
        found
    }

    #[test]
    fn every_bound_key_is_documented() {
        let documented: BTreeSet<Key> = KEYMAP
            .iter()
            .flat_map(|section| section.bindings)
            .flat_map(Binding::codes)
            .collect();

        let mut missing = Vec::new();
        for (path, source) in SOURCES {
            for key in bound_keys(source) {
                if !documented.contains(&key) {
                    missing.push(format!("{key:?} bound in {path}"));
                }
            }
        }

        assert!(
            missing.is_empty(),
            "these keys are bound but missing from KEYMAP in src/help.rs — \
             add a row for each, and the README's Keybindings section too:\n  {}",
            missing.join("\n  ")
        );
    }

    /// The long-range table is a *third* place a key can be bound, and the
    /// scan has to reach it.
    ///
    /// `g`, `G`, `{` and `}` are intercepted in `App::handle_event` before the
    /// file view sees them, and resolved by `long_range_target` — which moved
    /// out of `impl App` and into `src/viewport.rs` when the viewport was split
    /// off. `SOURCES` did not follow it, so a fifth long-range key added there
    /// would be bound and undocumented with nothing to say so (#95).
    #[test]
    fn the_long_range_table_is_scanned_too() {
        let bound = bound_keys(include_str!("viewport.rs"));

        assert!(
            ['g', 'G', '{', '}']
                .iter()
                .all(|c| bound.contains(&Key::Char(*c))),
            "src/viewport.rs no longer holds the long-range table; \
             this test and SOURCES both need to follow it: {bound:?}"
        );
        assert!(
            SOURCES.iter().any(|(path, _)| *path == "src/viewport.rs"),
            "src/viewport.rs binds keys, but every_bound_key_is_documented \
             does not scan it"
        );
    }

    /// The scan is the whole test's foundation, so it gets its own coverage:
    /// a `bound_chars` that silently found nothing would make
    /// `every_bound_key_is_documented` pass forever.
    #[test]
    fn the_scan_reads_both_binding_shapes() {
        let source = "KeyCode::Char('q') KeyCode::Char(c @ ('n' | 'N'))";

        assert_eq!(
            bound_keys(source),
            BTreeSet::from([Key::Char('q'), Key::Char('n'), Key::Char('N')]),
            "an or-pattern binding was not read"
        );
    }

    /// Named keys are bindings too (#162): `Home`, `PageDown` and `BackTab`
    /// used to be outside the test's reach, along with every F-key. The
    /// textarea's `Key::…` spelling counts the same as crossterm's, and a
    /// `KeyModifiers::…` or a non-key variant such as `Null` does not.
    #[test]
    fn the_scan_reads_named_keys() {
        let source = "KeyCode::PageDown | Key::Home => x, KeyCode::BackTab, KeyCode::F(5), \
                      KeyModifiers::CONTROL, KeyCode::Null";

        assert_eq!(
            bound_keys(source),
            BTreeSet::from([
                Key::Named("PageDown"),
                Key::Named("Home"),
                Key::Named("BackTab"),
                Key::Named("F"),
            ]),
            "a named key was not read, or a non-key identifier was"
        );
    }

    /// Keys pressed in a file's own tests are not bindings.
    #[test]
    fn the_scan_stops_at_the_test_module() {
        let source = "KeyCode::Char('q')\n#[cfg(test)]\nmod tests {\nKeyCode::Char('\u{263a}')";

        assert_eq!(bound_keys(source), BTreeSet::from([Key::Char('q')]));
    }

    /// A `#[cfg(test)]` on a `use` or a fixtures module is not the test
    /// module, and must not end the scan (#162): `lib.rs` has one at line 263
    /// of twelve thousand, and the old split there left every global key
    /// unchecked.
    #[test]
    fn the_scan_does_not_stop_at_an_early_cfg_test_attribute() {
        let source = "#[cfg(test)]\nuse x;\nKeyCode::Char('q')\n#[cfg(test)]\npub(crate) mod fixtures;\n\
                      KeyCode::Enter\n#[cfg(test)]\nmod tests {\nKeyCode::Char('z')";

        assert_eq!(
            bound_keys(source),
            BTreeSet::from([Key::Char('q'), Key::Named("Enter")])
        );
    }

    /// The scan reaches the global keys at all — the regression #162 found:
    /// every `lib.rs` binding sits past the fixtures module's attribute. And
    /// it reaches the picker, which `SOURCES` did not list.
    #[test]
    fn the_global_keys_and_the_picker_are_scanned() {
        let bound = bound_keys(include_str!("lib.rs"));
        // `q` and `BackTab` were this probe's canaries before task 4 (#199):
        // both now resolve through `keymap::DEFAULT` instead of a literal
        // `KeyCode::…` pattern in `lib.rs`, so the scan legitimately no
        // longer finds them here. `n` and `Esc` are still bound by a literal
        // pattern past the fixtures module — the hint arm `n`/`N` stays a
        // fallthrough until task 6, and the prompt's `Esc` until task 7 — so
        // they still prove the scan reaches this file's real bindings.
        assert!(
            bound.contains(&Key::Char('n')) && bound.contains(&Key::Named("Esc")),
            "src/lib.rs's global keys are not reached by the scan: {bound:?}"
        );
        let bound = bound_keys(include_str!("widgets/picker.rs"));
        assert!(
            bound.contains(&Key::Named("Esc")),
            "src/widgets/picker.rs is not reached by the scan: {bound:?}"
        );
        assert!(
            SOURCES
                .iter()
                .any(|(path, _)| *path == "src/widgets/picker.rs"),
            "src/widgets/picker.rs binds keys, but SOURCES does not list it"
        );
    }

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

    /// The case the whole layout exists for. 150 and 43 are inner columns and
    /// rows — the area handed to `layout`, already inside the border, so 43
    /// inner rows is a 45-row terminal. A 150x43 terminal is unremarkable, and
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
    /// has to be re-measured against both. The height, 43, is likewise the
    /// exact number of rows two columns hold at the current row count (86);
    /// adding a row without raising the height would drop it off the bottom,
    /// which is a `shown(&columns) == rows.len()` failure below, not a width
    /// one.
    #[test]
    fn a_normal_terminal_shows_the_whole_keymap() {
        let rows = rows();

        let columns = layout(&rows, inner(150, 43));

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
        let rows = rows();

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
        let height = u16::try_from(rows().len() + 10).expect("a keymap of sane size");
        let area = inner(170, height);
        let mut buf = Buffer::empty(area);

        render(area, &mut buf);

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

        render(area, &mut buf);

        let panel = border_box(&buf, area);
        assert_eq!(panel.width, area.width);
        assert_eq!(panel.height, area.height);
    }

    /// The narrow case: one column, and the rows past the bottom are dropped
    /// rather than squeezed. `render` reports the count in the bottom border.
    #[test]
    fn a_short_area_cannot_show_every_row() {
        let rows = rows();

        let columns = layout(&rows, inner(40, 5));

        assert_eq!(columns.len(), 1, "a 40-column area fitted a second column");
        assert_eq!(
            shown(&columns),
            5,
            "more rows were kept than there are rows"
        );
    }
}
