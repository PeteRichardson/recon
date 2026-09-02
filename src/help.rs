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
//! and fails when a character bound in a `KeyCode::Char(..)` / `Key::Char(..)`
//! arm is not named by any row here. That is the cheapest of the three options
//! #25 weighed, and it catches the common case: a new binding added without
//! being documented.
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
}

impl Binding {
    /// The characters this row documents, for the drift test.
    ///
    /// A label is a character binding when it is a single character, or
    /// `Ctrl-` plus one — `Ctrl-e` documents `Key::Char('e')` with the modifier
    /// held. `space` is spelled out because a bare ` ` in a table reads as an
    /// empty cell. Everything else (`Tab`, `Enter`, `PageDown`, `printable`)
    /// names a key that is not a `Char`, and yields nothing.
    ///
    /// Test-only for now: nothing in the running app needs to know which
    /// characters a row covers. Generating the README's tables from `KEYMAP`
    /// would, and this is the piece that would make it possible.
    #[cfg(test)]
    fn codes(&self) -> impl Iterator<Item = char> + '_ {
        self.keys.iter().filter_map(|label| {
            if *label == "space" {
                return Some(' ');
            }
            let bare = label.strip_prefix("Ctrl-").unwrap_or(label);
            let mut chars = bare.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        })
    }
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
pub const KEYMAP: &[Section] = &[
    Section {
        title: "Global",
        bindings: &[
            Binding {
                keys: &["?"],
                action: "This help — any key closes it",
            },
            Binding {
                keys: &["q"],
                action: "Quit",
            },
            Binding {
                keys: &["Tab"],
                action: "Focus the next pane",
            },
            Binding {
                keys: &["e"],
                action: "Focus the navigator",
            },
            Binding {
                keys: &["t"],
                action: "Focus the file view",
            },
            Binding {
                keys: &["f"],
                action: "Focus the filter pane",
            },
            Binding {
                keys: &["/"],
                action: "Search — filenames, or file contents",
            },
            Binding {
                keys: &["p"],
                action: "Promote the live search into the filter set",
            },
            Binding {
                keys: &["Esc"],
                action: "Clear the live search",
            },
            Binding {
                keys: &["space"],
                action: "Peek at the plain file; press again to restore",
            },
            Binding {
                keys: &["Ctrl-h", "H"],
                action: "Dim unmatched lines, or hide them",
            },
            Binding {
                keys: &["!"],
                action: "Disable every filter, or put them back",
            },
            Binding {
                keys: &["b"],
                action: "Hide the left column, and focus the file view",
            },
            Binding {
                keys: &["z"],
                action: "Maximise the focused pane, or restore the split",
            },
            Binding {
                keys: &["o"],
                action: "Open the file's project in your editor",
            },
            Binding {
                keys: &["O"],
                action: "Open the file alone in your editor",
            },
            Binding {
                keys: &["r"],
                action: "Refresh from disk — rescan the listing, reload the file",
            },
        ],
    },
    Section {
        title: "File view",
        bindings: &[
            Binding {
                keys: &["h", "Left"],
                action: "Cursor back",
            },
            Binding {
                keys: &["j", "Down"],
                action: "Cursor down",
            },
            Binding {
                keys: &["k", "Up"],
                action: "Cursor up",
            },
            Binding {
                keys: &["l", "Right"],
                action: "Cursor forward",
            },
            Binding {
                keys: &["w"],
                action: "Next word",
            },
            Binding {
                keys: &["0", "^"],
                action: "Start of the line",
            },
            Binding {
                keys: &["$"],
                action: "End of the line",
            },
            Binding {
                keys: &["{", "}"],
                action: "Previous / next paragraph",
            },
            Binding {
                keys: &["g", "Home"],
                action: "Top of the file",
            },
            Binding {
                keys: &["G", "End"],
                action: "Bottom of the file",
            },
            Binding {
                keys: &["n", "N"],
                action: "Next / previous interesting line",
            },
            Binding {
                keys: &["#"],
                action: "Toggle the line-number gutter",
            },
            Binding {
                keys: &["Ctrl-e", "Ctrl-y"],
                action: "Scroll one line down / up",
            },
            Binding {
                keys: &["Ctrl-d", "Ctrl-u"],
                action: "Scroll half a page down / up",
            },
            Binding {
                keys: &["[", "Ctrl-b", "PageUp"],
                action: "Scroll a page up",
            },
            Binding {
                keys: &["]", "Ctrl-f", "PageDown"],
                action: "Scroll a page down",
            },
        ],
    },
    Section {
        title: "Navigator",
        bindings: &[
            Binding {
                keys: &["k", "Up"],
                action: "Previous entry",
            },
            Binding {
                keys: &["j", "Down"],
                action: "Next entry",
            },
            Binding {
                keys: &["h", "Left"],
                action: "Up to the parent directory",
            },
            Binding {
                keys: &["l", "Right", "Enter"],
                action: "Open the entry",
            },
            Binding {
                keys: &["n", "N"],
                action: "Next / previous search match, or matching file",
            },
        ],
    },
    Section {
        title: "Filter pane",
        bindings: &[
            Binding {
                keys: &["j", "k"],
                action: "Move the selection",
            },
            Binding {
                keys: &["i"],
                action: "Add an including filter",
            },
            Binding {
                keys: &["x"],
                action: "Add an excluding filter",
            },
            Binding {
                keys: &["Enter"],
                action: "Enable or disable the selected filter",
            },
            Binding {
                keys: &["c"],
                action: "Change the selected filter's pattern",
            },
            Binding {
                keys: &["d"],
                action: "Delete the selected filter",
            },
            Binding {
                keys: &["m"],
                action: "Toggle selected filter to context mode",
            },
        ],
    },
    Section {
        title: "While a prompt is open",
        bindings: &[
            Binding {
                keys: &["printable"],
                action: "Append to the pattern",
            },
            Binding {
                keys: &["Backspace"],
                action: "Delete a character; cancel when empty",
            },
            Binding {
                keys: &["Enter"],
                action: "Run the search, or add the filter",
            },
            Binding {
                keys: &["Esc"],
                action: "Cancel",
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
    ];

    /// The characters bound by `Char(..)` patterns in `source`.
    ///
    /// Scans to the file's own test module and stops: fixtures press keys by
    /// the hundred, and a `KeyCode::Char('z')` in an assertion is not a
    /// binding. Deliberately a scan rather than a regex, so `Char(c @ ('n' |
    /// 'N'))` — the shape `n`/`N` are actually written in — yields both
    /// characters instead of neither.
    fn bound_chars(source: &str) -> BTreeSet<char> {
        let code = source.split("\n#[cfg(test)]").next().unwrap_or(source);
        let mut found = BTreeSet::new();
        for (start, matched) in code.match_indices("Char(") {
            let rest = &code[start + matched.len()..];
            // The first `)` closes either the pattern itself (`Char('q')`) or
            // the or-pattern inside it (`Char(c @ ('n' | 'N'))`). Both hold
            // every character the arm binds.
            let end = rest.find(')').unwrap_or(rest.len());
            let mut chars = rest[..end].chars();
            while let Some(c) = chars.next() {
                if c == '\'' {
                    if let Some(bound) = chars.next() {
                        found.insert(bound);
                    }
                    // Skip the closing quote so `'''` cannot be misread.
                    chars.next();
                }
            }
        }
        found
    }

    #[test]
    fn every_bound_key_is_documented() {
        let documented: BTreeSet<char> = KEYMAP
            .iter()
            .flat_map(|section| section.bindings)
            .flat_map(Binding::codes)
            .collect();

        let mut missing = Vec::new();
        for (path, source) in SOURCES {
            for c in bound_chars(source) {
                if !documented.contains(&c) {
                    missing.push(format!("{c:?} bound in {path}"));
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
        let bound = bound_chars(include_str!("viewport.rs"));

        assert!(
            ['g', 'G', '{', '}'].iter().all(|c| bound.contains(c)),
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
            bound_chars(source),
            BTreeSet::from(['q', 'n', 'N']),
            "an or-pattern binding was not read"
        );
    }

    /// Keys pressed in a file's own tests are not bindings.
    #[test]
    fn the_scan_stops_at_the_test_module() {
        let source = "KeyCode::Char('q')\n#[cfg(test)]\nKeyCode::Char('\u{263a}')";

        assert_eq!(bound_chars(source), BTreeSet::from(['q']));
    }

    /// `codes` derives from the labels that get drawn, so a label shape it
    /// cannot read would quietly shrink the documented set.
    #[test]
    fn a_labels_bound_character_is_derived_from_how_it_is_drawn() {
        let binding = Binding {
            keys: &["Ctrl-e", "H", "space", "PageDown"],
            action: "irrelevant",
        };

        assert_eq!(
            binding.codes().collect::<Vec<_>>(),
            vec!['e', 'H', ' '],
            "a key label was read as the wrong character"
        );
    }

    fn inner(width: u16, height: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// The case the whole layout exists for. A 142x44 terminal is unremarkable,
    /// and the keymap is the one screen where "most of it" is not good enough —
    /// the row you cannot see is exactly the one you opened it to find.
    ///
    /// It fails when the columns are sized against the *widest* row in the
    /// whole table rather than the widest in each column: two columns of the
    /// global maximum need 159 columns, and this area has 140 to give.
    #[test]
    fn a_normal_terminal_shows_the_whole_keymap() {
        let rows = rows();

        let columns = layout(&rows, inner(140, 42));

        assert_eq!(shown(&columns), rows.len(), "the keymap did not fit");
        assert!(
            total_width(&columns) <= 140,
            "the columns overflowed the area they were fitted to"
        );
    }

    /// Each column is sized to its own content, so a column of short
    /// descriptions does not pay for a long one three columns over.
    #[test]
    fn a_column_is_sized_to_its_own_widest_row() {
        let rows = rows();

        let columns = layout(&rows, inner(140, 42));

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
        let area = inner(170, 64);
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
