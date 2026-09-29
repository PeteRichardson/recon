use super::*;
use crate::app::filter_editor::NO_PATTERN;
use crate::filter::Sense;
use regex::Regex;

// ---- the filter editor (#312) --------------------------------------------

const BODY: &str = "ERROR timeout\nINFO ok\nERROR disk\nINFO timeout\n";

/// The shared screen, four rows taller: the editor's panel is eight rows
/// (#317), and on the shared ten the file would get none. These shadow the
/// shared `AREA`, `draw`, `rendered` and `status_line` in this file.
const AREA: Rect = Rect {
    height: super::AREA.height + 4,
    ..super::AREA
};

fn draw(app: &mut App) {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
}

fn rendered(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    (0..AREA.height)
        .map(|y| {
            (0..AREA.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_line(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let y = AREA.height - 1;
    (0..AREA.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn editor<'a>(app: &'a App) -> &'a FilterEditor {
    app.filter_editor
        .as_ref()
        .expect("the filter editor should be open")
}

/// `f I` from wherever focus is.
fn open_editor(app: &mut App) {
    key(app, KeyCode::Char('f'));
    key(app, KeyCode::Char('I'));
}

#[test]
fn f_big_i_opens_the_editor_on_the_open_file_with_an_empty_pattern() {
    let mut app = app_over_file("editor_opens", BODY);
    key(&mut app, KeyCode::Char('t'));
    open_editor(&mut app);

    let editor = editor(&app);
    assert_eq!(editor.field.pattern, "");
    assert_eq!(editor.lines.len(), 4, "not the open file's lines");
    assert!(editor.regex.is_none());
}

#[test]
fn the_count_follows_each_key() {
    let mut app = app_over_file("editor_count", BODY);
    open_editor(&mut app);
    assert_eq!(status_line(&mut app), "4 lines");

    typed(&mut app, "E");
    assert_eq!(editor(&app).matches, 2);
    assert_eq!(status_line(&mut app), "2 of 4 lines match");

    typed(&mut app, "RROR d");
    assert_eq!(status_line(&mut app), "1 of 4 lines match");

    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(status_line(&mut app), "2 of 4 lines match");

    ctrl(&mut app, KeyCode::Char('u'));
    assert_eq!(editor(&app).field.pattern, "");
    assert_eq!(
        status_line(&mut app),
        "4 lines",
        "an empty pattern counts nothing"
    );
}

/// A half-typed pattern keeps the last highlight and count, and says why
/// it does not compile.
#[test]
fn a_pattern_that_does_not_compile_shows_its_error_and_keeps_the_count() {
    let mut app = app_over_file("editor_error", BODY);
    open_editor(&mut app);
    typed(&mut app, "timeout(");

    let editor = editor(&app);
    assert_eq!(editor.error.as_deref(), Some("error: unclosed group"));
    assert_eq!(editor.matches, 2, "the count of `timeout` went");
    assert_eq!(editor.regex.as_ref().map(Regex::as_str), Some("timeout"));
    let screen = rendered(&mut app);
    assert!(screen.contains("error: unclosed group"), "{screen}");

    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_some(), "Enter added a broken pattern");
    assert_eq!(app.filters.len(), 0);

    typed(&mut app, ")");
    assert!(self::editor(&app).error.is_none());
}

#[test]
fn enter_on_an_empty_pattern_says_so_and_stays_open() {
    let mut app = app_over_file("editor_empty", BODY);
    open_editor(&mut app);
    key(&mut app, KeyCode::Enter);

    assert_eq!(editor(&app).error.as_deref(), Some(NO_PATTERN));
    assert_eq!(app.filters.len(), 0);
}

/// Enter gives exactly what `f i` with the same pattern gives: one include
/// filter in the scratch set, in the same colour, with the same verdicts.
#[test]
fn enter_adds_the_same_filter_as_f_i() {
    let mut by_editor = app_over_file("editor_enter", BODY);
    key(&mut by_editor, KeyCode::Char('t'));
    open_editor(&mut by_editor);
    typed(&mut by_editor, "timeout");
    key(&mut by_editor, KeyCode::Enter);

    let mut by_prompt = app_over_file("editor_enter_prompt", BODY);
    key(&mut by_prompt, KeyCode::Char('t'));
    key(&mut by_prompt, KeyCode::Char('f'));
    key(&mut by_prompt, KeyCode::Char('i'));
    typed(&mut by_prompt, "timeout");
    key(&mut by_prompt, KeyCode::Enter);

    assert!(by_editor.filter_editor.is_none(), "Enter did not close");
    assert_eq!(by_editor.filters.len(), 1);
    let (made, expected) = (&by_editor.filters.filters(), &by_prompt.filters.filters());
    let made = made.iter().find(|f| f.set == 0).expect("a scratch filter");
    let expected = expected
        .iter()
        .find(|f| f.set == 0)
        .expect("a scratch filter");
    assert_eq!(made.predicate.display(), "timeout");
    assert_eq!(made.sense, Sense::Include);
    assert_eq!(made.style, expected.style, "a different colour");
    assert_eq!(by_editor.document.verdicts(), by_prompt.document.verdicts());
    assert_eq!(by_editor.focus, by_prompt.focus, "the chain did not return");
}

/// The highlight is drawn in the colour the new filter will take.
#[test]
fn a_matched_line_is_drawn_in_the_new_filter_colour() {
    let mut app = app_over_file("editor_colour", BODY);
    open_editor(&mut app);
    typed(&mut app, "disk");
    let colour = app.filters.next_style();

    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    // Row 0 is the border; the file starts on row 1, after the three-column
    // gutter. `ERROR disk` is the third line, and `disk` starts at its
    // seventh column.
    let (x, y) = (1 + 3 + 6, 1 + 2);
    assert_eq!(buf[(x, y)].symbol(), "d");
    assert_eq!(buf[(x, y)].fg, colour.fg.expect("a palette colour"));
    assert!(
        buf[(x, y)].modifier.contains(Modifier::REVERSED),
        "the match is not marked"
    );
    assert!(
        !buf[(1 + 3, 1)].modifier.contains(Modifier::REVERSED),
        "a missed line is marked"
    );
}

#[test]
fn esc_changes_nothing() {
    let mut app = app_over_file("editor_esc", BODY);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    let (cursor, before) = (cursor_source(&app), app.document.verdicts().to_vec());

    open_editor(&mut app);
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Esc);

    assert!(app.filter_editor.is_none());
    assert_eq!(app.filters.len(), 0);
    assert_eq!(app.document.verdicts(), before.as_slice());
    assert_eq!(cursor_source(&app), cursor);
    assert_eq!(
        app.focus,
        Focus::Filters,
        "Esc ends the chain where it is, as in `f i`"
    );
    assert!(app.chain_origin.is_none());
}

/// The editor takes every key: `q` is typed, not a quit, and a key the
/// panes bind does nothing under it.
#[test]
fn the_editor_takes_every_key() {
    let mut app = app_over_file("editor_modal", BODY);
    open_editor(&mut app);
    typed(&mut app, "q?Lf");

    assert_eq!(editor(&app).field.pattern, "q?Lf");
    assert_eq!(app.state, AppState::default(), "q quit");
    assert!(!app.help);
    assert!(app.set_picker.is_none());
}

/// Backspace on an empty pattern does not close the editor, as it does a
/// prompt: the file under it is still worth reading.
#[test]
fn backspace_on_an_empty_pattern_stays_open() {
    let mut app = app_over_file("editor_backspace", BODY);
    open_editor(&mut app);
    key(&mut app, KeyCode::Backspace);
    assert!(app.filter_editor.is_some());
}

#[test]
fn the_arrows_and_page_keys_scroll_the_file() {
    let mut app = app_over_file("editor_scroll", &numbered_lines(100));
    open_editor(&mut app);
    draw(&mut app);

    key(&mut app, KeyCode::Down);
    assert_eq!(editor(&app).top, 1);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);
    assert_eq!(editor(&app).top, 0, "scrolled above the file");

    let page = editor(&app).page;
    key(&mut app, KeyCode::PageDown);
    assert_eq!(editor(&app).top, page);
    for _ in 0..100 {
        key(&mut app, KeyCode::PageDown);
    }
    assert_eq!(editor(&app).top, 99, "scrolled past the file");
    assert!(rendered(&mut app).contains("line 99"));
}

/// The count is a claim about the whole file, so a truncated preview is
/// loaded in full before the editor opens.
#[test]
fn the_editor_counts_past_a_truncated_preview() {
    let dir = fixture_dir("editor_truncated");
    let total = crate::widgets::fileview::PREVIEW_LINES + 100;
    fs::write(dir.join("big.log"), numbered_lines(total)).expect("write fixture");
    let mut app = App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    });
    key(&mut app, KeyCode::Down);
    assert!(app.file_view_truncated(), "sanity: only a preview");

    open_editor(&mut app);
    typed(&mut app, "line");

    assert_eq!(editor(&app).lines.len(), total);
    assert_eq!(editor(&app).matches, total);
}

#[test]
fn big_i_outside_the_filter_pane_names_the_chain() {
    let mut app = app_over_file("editor_hint", BODY);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('I'));

    assert!(app.filter_editor.is_none());
    assert_eq!(message(&app), Some("I opens the filter editor · f I"));
}

#[test]
fn a_rebound_key_opens_the_editor() {
    let file = fixture_path("editor_rebound", BODY);
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert("filters.editor.new".to_string(), vec!["W".to_string()]);
    let (keymap, _) = crate::keymap::Keymap::new(&crate::config::KeymapConfig { bindings })
        .expect("valid keymap");
    let mut app = App::new(&Config {
        path: file.display().to_string(),
        bindings: keymap,
        ..Config::default()
    });
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('W'));
    assert!(app.filter_editor.is_some());
}

// ---- f C: the selected filter in the editor (#313) -----------------------

/// Select the pane row that `row` is, and focus the pane.
fn select_row(app: &mut App, row: widgets::filterlist::Row) {
    let rows = widgets::filterlist::rows(&app.filters);
    let at = rows
        .iter()
        .position(|seen| *seen == row)
        .expect("the row is drawn");
    focus_filter_pane(app);
    app.filters_pane.select(at);
}

/// Scratch filter 0 is `ERROR`, filter 1 is `INFO`, as an excluding one.
fn app_with_two_filters(name: &str) -> App<'static> {
    let mut app = app_over_file(name, BODY);
    app.add_filter("ERROR").expect("valid");
    app.add_excluding_filter("INFO").expect("valid");
    app
}

fn open_selected(app: &mut App) {
    key(app, KeyCode::Char('f'));
    key(app, KeyCode::Char('C'));
}

#[test]
fn f_big_c_opens_the_selected_filter_with_its_matches_showing() {
    let mut app = app_with_two_filters("editor_open_selected");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);

    let editor = editor(&app);
    assert_eq!(editor.field.pattern, "ERROR");
    assert_eq!(
        editor.field.cursor, 5,
        "the cursor is not at the end, as `c` puts it"
    );
    assert_eq!(editor.target, Some(0));
    assert_eq!(editor.matches, 2, "the count did not show at once");
    assert_eq!(
        editor.style,
        app.filters.filters()[0].style,
        "not the filter's colour"
    );
    assert!(rendered(&mut app).contains("Enter change"));
}

/// Enter changes only the pattern, as `c` does: the sense, the enabled
/// state, the colour and the index stay.
#[test]
fn enter_changes_only_the_pattern() {
    let mut app = app_with_two_filters("editor_change");
    app.filters.toggle_enabled(1);
    let before = app.filters.filters()[1].clone();
    select_row(&mut app, widgets::filterlist::Row::Filter(1));
    open_selected(&mut app);
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    typed(&mut app, "timeout");
    key(&mut app, KeyCode::Enter);

    assert!(app.filter_editor.is_none());
    assert_eq!(app.filters.len(), 2, "a filter was added, not changed");
    let after = &app.filters.filters()[1];
    assert_eq!(after.predicate.display(), "INtimeout");
    assert_eq!(after.sense, before.sense);
    assert_eq!(after.enabled, before.enabled);
    assert_eq!(after.style, before.style);
    assert_eq!(app.filters.filters()[0].predicate.display(), "ERROR");
}

#[test]
fn esc_leaves_the_selected_filter_as_it_was() {
    let mut app = app_with_two_filters("editor_change_esc");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    typed(&mut app, "X");
    key(&mut app, KeyCode::Esc);

    assert!(app.filter_editor.is_none());
    assert_eq!(app.filters.filters()[0].predicate.display(), "ERROR");
}

/// `f C … Enter` returns as `f c … Enter` does.
#[test]
fn f_big_c_enter_returns_like_f_c() {
    let mut app = app_with_two_filters("editor_change_returns");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('C'));
    assert!(
        app.filter_editor.is_some(),
        "sanity: j selected the first filter"
    );
    // Still matches the cursor line, `ERROR timeout`, which the editor
    // marks (#314): a pattern that fails the mark is refused (#318).
    typed(&mut app, "|timeout");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::View);
}

#[test]
fn f_big_c_on_a_header_row_says_why() {
    let mut app = app_with_three_sets("editor_header");
    let a = app
        .filters
        .sets()
        .iter()
        .position(|set| set.name == "a")
        .expect("set a");
    select_row(&mut app, widgets::filterlist::Row::Header(a));
    key(&mut app, KeyCode::Char('C'));

    assert!(app.filter_editor.is_none());
    assert_eq!(
        message(&app),
        Some("sets are defined in filters.toml; edit the file to change one")
    );
}

#[test]
fn f_big_c_on_a_built_in_filter_says_why() {
    let mut app = app_over_file("editor_builtin", BODY);
    let definitions = app
        .filters
        .sets()
        .iter()
        .position(|set| set.name == filter::DEFINITIONS_SET)
        .expect("the built-in set");
    app.filters.set_enabled_set(definitions, true);
    let (index, _) = app
        .filters
        .filters_in(definitions)
        .next()
        .expect("a built-in filter");
    select_row(&mut app, widgets::filterlist::Row::BuiltIn(index));
    key(&mut app, KeyCode::Char('C'));

    assert!(app.filter_editor.is_none());
    assert_eq!(
        message(&app),
        Some("built-in filters can be switched off or collapsed, not deleted or edited")
    );
}

/// A definition filter in a file's set has no regex to show.
#[test]
fn f_big_c_on_a_definition_filter_says_why() {
    let mut set = filter::test_support::loaded("defs", 10, true, &["functions"]);
    set.filters[0].predicate = filter::Predicate::Definition(crate::syntax::Kind::Function);
    let mut app = app_over_file("editor_definition", BODY);
    app.filters = ActiveFilters::with_sets(None, &[set]);
    app.refresh_view();
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    key(&mut app, KeyCode::Char('C'));

    assert!(app.filter_editor.is_none());
    assert_eq!(
        message(&app),
        Some("a definition filter has no pattern to edit; c turns it into one")
    );
}

#[test]
fn big_c_outside_the_filter_pane_names_the_chain() {
    let mut app = app_over_file("editor_hint_c", BODY);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('C'));

    assert!(app.filter_editor.is_none());
    assert_eq!(
        message(&app),
        Some("C opens the selected filter in the filter editor · f C")
    );
}

// ---- line marks (#314) ---------------------------------------------------

use crate::app::filter_editor::{EditorFocus, Mark};
use ratatui::prelude::Color;

fn marks(app: &App) -> Vec<(usize, Mark)> {
    editor(app)
        .marks
        .iter()
        .map(|(&line, &mark)| (line, mark))
        .collect()
}

/// `f I` with the focus already in the filter pane, so the chain has no
/// origin: a first `f I` cancelled with Esc leaves the focus there.
fn open_editor_from_the_filter_pane(app: &mut App) {
    open_editor(app);
    key(app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::Filters);
    open_editor(app);
}

#[test]
fn opened_from_the_file_view_the_cursor_line_is_must_match() {
    let mut app = app_over_file("marks_from_view", BODY);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    open_editor(&mut app);

    assert_eq!(marks(&app), [(1, Mark::MustMatch)]);
    assert_eq!(editor(&app).cursor, 1);
    assert_eq!(
        status_line(&mut app),
        "4 lines · 1 check fails",
        "an empty pattern matches nothing"
    );
}

#[test]
fn f_big_c_from_the_file_view_marks_the_cursor_line_too() {
    let mut app = app_with_two_filters("marks_c_from_view");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('C'));

    assert_eq!(marks(&app), [(0, Mark::MustMatch)]);
}

#[test]
fn opened_from_the_filter_pane_or_the_explorer_no_line_is_marked() {
    let mut app = app_over_file("marks_from_pane", BODY);
    open_editor_from_the_filter_pane(&mut app);
    assert!(marks(&app).is_empty());

    let mut app = app_over_file("marks_from_explorer", BODY);
    assert_eq!(app.focus, Focus::Explorer);
    open_editor(&mut app);
    assert!(marks(&app).is_empty());
    assert_eq!(status_line(&mut app), "4 lines", "no checks, no count");
}

#[test]
fn plus_minus_and_equals_mark_and_clear_the_cursor_line() {
    let mut app = app_over_file("marks_keys", BODY);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    assert_eq!(editor(&app).focus, EditorFocus::Lines);

    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-'));
    assert_eq!(marks(&app), [(0, Mark::MustMatch), (1, Mark::MustNotMatch)]);

    key(&mut app, KeyCode::Char('+'));
    assert_eq!(
        marks(&app),
        [(0, Mark::MustMatch), (1, Mark::MustMatch)],
        "a second mark replaces the first"
    );

    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Char('='));
    assert_eq!(marks(&app), [(1, Mark::MustMatch)]);
    assert_eq!(editor(&app).field.pattern, "", "a mark key was typed");
}

/// `+` and `-` are pattern characters, so in the pattern they are typed.
/// On the lines, a character is not typed at all.
#[test]
fn the_mark_keys_are_typed_in_the_pattern() {
    let mut app = app_over_file("marks_typed", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "a+-=V");
    assert_eq!(editor(&app).field.pattern, "a+-=V");
    assert!(marks(&app).is_empty());

    key(&mut app, KeyCode::Tab);
    typed(&mut app, "x");
    key(&mut app, KeyCode::Backspace);
    assert_eq!(editor(&app).field.pattern, "a+-=V");

    key(&mut app, KeyCode::BackTab);
    assert_eq!(editor(&app).focus, EditorFocus::Pattern);
    typed(&mut app, "x");
    assert_eq!(editor(&app).field.pattern, "a+-=Vx");
}

#[test]
fn a_visual_range_gets_one_mark_on_each_line() {
    let mut app = app_over_file("marks_range", BODY);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-'));

    assert_eq!(
        marks(&app),
        [
            (1, Mark::MustNotMatch),
            (2, Mark::MustNotMatch),
            (3, Mark::MustNotMatch)
        ]
    );
    assert!(editor(&app).anchor.is_none(), "the range stayed open");

    // Upwards, and cleared the same way.
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Char('='));
    assert_eq!(marks(&app), [(1, Mark::MustNotMatch)]);
}

/// Esc closes an open range first, as in the file view, and then the
/// editor, with its marks.
#[test]
fn esc_closes_a_range_and_then_discards_the_marks() {
    let mut app = app_over_file("marks_esc", BODY);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Esc);
    assert!(editor(&app).anchor.is_none());
    assert!(app.filter_editor.is_some(), "Esc closed the editor");

    key(&mut app, KeyCode::Esc);
    assert!(app.filter_editor.is_none());
    open_editor(&mut app);
    assert!(marks(&app).is_empty(), "the marks came back");
}

#[test]
fn the_failed_count_follows_each_key() {
    let mut app = app_over_file("marks_count", BODY);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('+')); // ERROR timeout
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-')); // INFO ok
    key(&mut app, KeyCode::BackTab);
    assert_eq!(status_line(&mut app), "4 lines · 1 check fails");

    typed(&mut app, "ERROR");
    assert_eq!(status_line(&mut app), "2 of 4 lines match · 0 checks fail");

    ctrl(&mut app, KeyCode::Char('u'));
    typed(&mut app, "INFO");
    assert_eq!(status_line(&mut app), "2 of 4 lines match · 2 checks fail");

    // A pattern that does not compile keeps the last count, as the match
    // count does.
    typed(&mut app, "(");
    assert_eq!(editor(&app).failures, 2);
}

/// The four states, one on each line: must-match passes, must-not-match
/// passes, must-match fails, must-not-match fails. A failed check fills its
/// row with red; a passed one only colours its mark.
#[test]
fn the_four_states_are_drawn_four_ways() {
    let mut app = app_over_file("marks_styles", BODY);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    for mark in ['+', '-', '+', '-'] {
        key(&mut app, KeyCode::Char(mark));
        key(&mut app, KeyCode::Down);
    }
    // Back to the first line, so all four are on the screen.
    for _ in 0..4 {
        key(&mut app, KeyCode::Up);
    }
    key(&mut app, KeyCode::BackTab);
    typed(&mut app, "timeout");

    let area = Rect { height: 20, ..AREA };
    let mut buf = Buffer::empty(area);
    app.render(area, &mut buf);
    // Row 0 is the border. Column 1 is the cursor, column 2 the mark, and
    // the text starts at column 4.
    let row = |line: u16| {
        let y = 1 + line;
        let text: Vec<_> = (4..20)
            .map(|x| (buf[(x, y)].fg, buf[(x, y)].bg, buf[(x, y)].modifier))
            .collect();
        (buf[(2, y)].symbol().to_string(), buf[(2, y)].bg, text)
    };
    let rows: Vec<_> = (0..4).map(row).collect();

    assert_eq!(rows[0].0, "+");
    assert_eq!(rows[1].0, "-");
    assert_eq!(rows[2].0, "+");
    assert_eq!(rows[3].0, "-");
    for (line, passes) in [(0, true), (1, true), (2, false), (3, false)] {
        assert_eq!(
            rows[line].1 == Color::Red,
            !passes,
            "line {line}'s mark is not in the style of its check"
        );
        assert_eq!(
            rows[line].2.iter().all(|&(_, bg, _)| bg == Color::Red),
            !passes,
            "line {line}'s text is not in the style of its check"
        );
    }
    for a in 0..4 {
        for b in a + 1..4 {
            assert_ne!(rows[a], rows[b], "lines {a} and {b} look the same");
        }
    }
    // The must-not-match failure shows what the pattern wrongly matched.
    assert!(
        rows[3]
            .2
            .iter()
            .any(|&(_, _, modifier)| modifier.contains(Modifier::REVERSED))
    );
}

/// The cursor line opened from far down the file is on the screen.
#[test]
fn the_marked_line_is_on_the_screen() {
    let mut app = app_over_file("marks_reveal", &numbered_lines(100));
    key(&mut app, KeyCode::Char('t'));
    for _ in 0..60 {
        key(&mut app, KeyCode::Char('j'));
    }
    open_editor(&mut app);
    let cursor = editor(&app).cursor;
    assert_eq!(marks(&app), [(cursor, Mark::MustMatch)]);
    assert!(cursor > 50, "sanity: the cursor is far down");

    draw(&mut app);
    let (top, page) = (editor(&app).top, editor(&app).page);
    assert!(
        (top..top + page).contains(&cursor),
        "line {cursor} is not in {top}..{}",
        top + page
    );
    assert!(rendered(&mut app).contains(&format!("+ line {cursor}")));
}

/// On the lines, the arrows move the cursor line and the screen follows it.
#[test]
fn on_the_lines_the_arrows_move_the_cursor() {
    let mut app = app_over_file("marks_cursor", &numbered_lines(100));
    open_editor_from_the_filter_pane(&mut app);
    draw(&mut app);
    key(&mut app, KeyCode::Tab);
    let page = editor(&app).page;
    for _ in 0..page + 2 {
        key(&mut app, KeyCode::Down);
    }
    draw(&mut app);

    let editor = editor(&app);
    assert_eq!(editor.cursor, page + 2);
    assert_eq!(editor.top, 3, "the screen did not follow the cursor");
}

// ---- jumps and matches only (#315) ---------------------------------------

/// `ERROR timeout` +, `INFO ok` -, `ERROR disk` +, `INFO timeout` - under
/// `timeout`: the first two checks pass and the last two fail. The cursor
/// ends on line 0 and the keys on the lines.
fn app_with_four_checks(name: &str) -> App<'static> {
    let mut app = app_over_file(name, BODY);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    for mark in ['+', '-', '+', '-'] {
        key(&mut app, KeyCode::Char(mark));
        key(&mut app, KeyCode::Down);
    }
    for _ in 0..4 {
        key(&mut app, KeyCode::Up);
    }
    key(&mut app, KeyCode::BackTab);
    typed(&mut app, "timeout");
    key(&mut app, KeyCode::Tab);
    assert_eq!(editor(&app).failures, 2, "sanity: two checks fail");
    assert_eq!(editor(&app).cursor, 0);
    app
}

#[test]
fn f_and_big_f_go_to_each_failed_check_and_wrap() {
    let mut app = app_with_four_checks("jump_failures");

    key(&mut app, KeyCode::Char('f'));
    assert_eq!(editor(&app).cursor, 2);
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(editor(&app).cursor, 3);
    assert_eq!(message(&app), None);
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(editor(&app).cursor, 2, "a passed check stopped the jump");
    assert_eq!(message(&app), Some("wrapped to the top"));

    key(&mut app, KeyCode::Char('F'));
    assert_eq!(editor(&app).cursor, 3);
    assert_eq!(message(&app), Some("wrapped to the bottom"));
    key(&mut app, KeyCode::Char('F'));
    assert_eq!(editor(&app).cursor, 2);
    assert_eq!(message(&app), None, "the message outlived its key");
}

#[test]
fn f_says_when_no_check_fails() {
    let mut app = app_with_four_checks("jump_no_failures");
    key(&mut app, KeyCode::BackTab);
    ctrl(&mut app, KeyCode::Char('u'));
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Tab);
    assert_eq!(editor(&app).failures, 0, "sanity");

    key(&mut app, KeyCode::Char('f'));
    assert_eq!(editor(&app).cursor, 0);
    assert_eq!(message(&app), Some("no failed check"));
}

#[test]
fn n_and_big_n_do_not_stop_on_a_marked_line() {
    let mut app = app_over_file("jump_unmarked", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR|INFO");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Up);

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(
        editor(&app).cursor,
        2,
        "the jump stopped on the marked line"
    );
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(editor(&app).cursor, 3);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(editor(&app).cursor, 0);
    assert_eq!(message(&app), Some("wrapped to the top"));

    key(&mut app, KeyCode::Char('N'));
    assert_eq!(editor(&app).cursor, 3);
    assert_eq!(message(&app), Some("wrapped to the bottom"));
    key(&mut app, KeyCode::Char('N'));
    key(&mut app, KeyCode::Char('N'));
    assert_eq!(
        editor(&app).cursor,
        0,
        "the jump stopped on the marked line"
    );
}

#[test]
fn n_says_when_there_is_no_unmarked_match() {
    let mut app = app_over_file("jump_no_unmarked", BODY);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(message(&app), Some("no unmarked match"), "no pattern");
}

/// Tab moves the thick green frame of a focused pane between the lines and
/// the pattern.
#[test]
fn the_focused_part_has_the_focused_frame() {
    let mut app = app_over_file("editor_frame", BODY);
    open_editor_from_the_filter_pane(&mut app);
    // The top-left corners: of the file's lines at row 0, and of the panel
    // eight rows above the status row.
    let corners = |app: &mut App| {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        let panel = AREA.height - 1 - 8;
        (buf[(0, 0)].clone(), buf[(0, panel)].clone())
    };
    let thick = |cell: &ratatui::buffer::Cell| cell.symbol() == "┏" && cell.fg == Color::Green;

    let (lines, pattern) = corners(&mut app);
    assert!(!thick(&lines) && thick(&pattern), "{lines:?} {pattern:?}");

    key(&mut app, KeyCode::Tab);
    let (lines, pattern) = corners(&mut app);
    assert!(thick(&lines) && !thick(&pattern), "{lines:?} {pattern:?}");
}

/// A line the pattern does not match is not an unmarked match.
#[test]
fn n_skips_a_line_the_pattern_misses() {
    let mut app = app_over_file("jump_misses", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "timeout");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(editor(&app).cursor, 3);
}

/// In the pattern, the jump keys and `u` are pattern characters.
#[test]
fn the_jump_keys_are_typed_in_the_pattern() {
    let mut app = app_with_four_checks("jump_typed");
    key(&mut app, KeyCode::BackTab);
    typed(&mut app, "|fFnNu");
    let editor = editor(&app);
    assert_eq!(editor.field.pattern, "timeout|fFnNu");
    assert_eq!(editor.cursor, 0);
    assert!(!editor.matches_only);
}

const WORDS: &str = "alpha match\nbravo\ncharlie\ndelta match\necho\n";

/// `match` under `WORDS`, with `bravo` marked must-match (a failed check),
/// and the keys on the lines at line 0.
fn app_over_words(name: &str) -> App<'static> {
    let mut app = app_over_file(name, WORDS);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "match");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Up);
    app
}

#[test]
fn u_shows_only_the_matched_and_the_marked_lines() {
    let mut app = app_over_words("matches_only");
    key(&mut app, KeyCode::Char('u'));

    let screen = rendered(&mut app);
    for shown in ["alpha match", "bravo", "delta match"] {
        assert!(screen.contains(shown), "{shown} is hidden:\n{screen}");
    }
    assert!(!screen.contains("charlie"), "charlie is drawn:\n{screen}");
    assert_eq!(editor(&app).rows(), 3, "echo is not hidden");
    assert!(
        screen.contains(" + bravo"),
        "the failed check is not drawn as one:\n{screen}"
    );
    assert_eq!(
        status_line(&mut app),
        "2 of 5 lines match · 1 check fails · matches only"
    );

    key(&mut app, KeyCode::Char('u'));
    assert!(rendered(&mut app).contains("charlie"));
    assert_eq!(editor(&app).rows(), 5);
    assert_eq!(status_line(&mut app), "2 of 5 lines match · 1 check fails");
}

/// The arrows step over a hidden line, and a range marks only the lines
/// drawn in it.
#[test]
fn with_matches_only_the_keys_act_on_the_lines_drawn() {
    let mut app = app_over_words("matches_only_keys");
    key(&mut app, KeyCode::Char('u'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    assert_eq!(
        editor(&app).cursor,
        3,
        "the cursor stopped on a hidden line"
    );

    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Char('-'));
    assert_eq!(
        marks(&app),
        [
            (0, Mark::MustNotMatch),
            (1, Mark::MustNotMatch),
            (3, Mark::MustNotMatch)
        ],
        "a hidden line in the range got a mark"
    );
}

/// A mark removed from a line the pattern misses hides the line, and the
/// cursor goes to the next line drawn.
#[test]
fn a_line_that_loses_its_mark_is_hidden() {
    let mut app = app_over_words("matches_only_unmark");
    key(&mut app, KeyCode::Char('u'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('='));

    assert!(!rendered(&mut app).contains("bravo"));
    assert_eq!(editor(&app).cursor, 3);
}

/// With no pattern nothing is highlighted, so matches only hides nothing.
#[test]
fn matches_only_with_no_pattern_shows_every_line() {
    let mut app = app_over_file("matches_only_empty", WORDS);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('u'));
    assert!(rendered(&mut app).contains("charlie"));
    assert_eq!(status_line(&mut app), "5 lines · matches only");
}

/// The editor's `u` is its own: the main window's hide mode, and all it
/// draws, are as they were when the editor opened.
#[test]
fn closing_the_editor_leaves_the_main_window_as_it_was() {
    let mut app = app_over_file("matches_only_close", WORDS);
    open_editor_from_the_filter_pane(&mut app);
    key(&mut app, KeyCode::Esc);
    for hide in [false, true] {
        if hide {
            key(&mut app, KeyCode::Char('u'));
        }
        let before = rendered(&mut app);

        open_editor(&mut app);
        typed(&mut app, "match");
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Char('u'));
        key(&mut app, KeyCode::Char('n'));
        key(&mut app, KeyCode::Esc);

        assert!(app.filter_editor.is_none());
        assert_eq!(rendered(&mut app), before, "hide mode {hide}");
    }
}

#[test]
fn the_help_overlay_shows_the_jump_keys() {
    let named = |name: &str| {
        crate::help::KEYMAP
            .iter()
            .flat_map(|section| section.bindings)
            .any(|binding| binding.names.contains(&name))
    };
    for name in [
        "filtereditor.failure.next",
        "filtereditor.failure.prev",
        "filtereditor.unmarked.next",
        "filtereditor.unmarked.prev",
        "filtereditor.toggle.matchesonly",
    ] {
        assert!(named(name), "{name} has no row in the help overlay");
    }
}

// ---- undo and redo (#316) ------------------------------------------------

/// As if the typing stopped for `VERSION_PAUSE` after the last edit.
fn pause(app: &mut App) {
    let editor = app
        .filter_editor
        .as_mut()
        .expect("the filter editor should be open");
    if let Some(at) = editor.edited_at.as_mut() {
        *at -= crate::app::filter_editor::VERSION_PAUSE;
    }
}

fn versions<'a>(app: &'a App) -> Vec<&'a str> {
    editor(app).versions.iter().map(String::as_str).collect()
}

fn pattern<'a>(app: &'a App) -> &'a str {
    &editor(app).field.pattern
}

/// `ERROR`, `ERROR d` and `INFO`, each typed after a pause, so three
/// versions; the pattern is left at `INFO`, not yet kept.
fn app_with_three_versions(name: &str) -> App<'static> {
    let mut app = app_over_file(name, BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    pause(&mut app);
    typed(&mut app, " d");
    pause(&mut app);
    ctrl(&mut app, KeyCode::Char('u'));
    typed(&mut app, "INFO");
    assert_eq!(versions(&app), ["ERROR", "ERROR d"]);
    app
}

#[test]
fn a_version_is_kept_at_a_pause_and_not_on_each_key() {
    let mut app = app_over_file("undo_pause", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    assert!(versions(&app).is_empty(), "a key made a version");

    pause(&mut app);
    typed(&mut app, " d");
    assert_eq!(versions(&app), ["ERROR"], "only the pattern at the pause");
}

#[test]
fn ctrl_z_goes_back_through_each_version_and_stops_at_the_first() {
    let mut app = app_with_three_versions("undo_back");

    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(pattern(&app), "ERROR d");
    assert_eq!(status_line(&mut app), "1 of 4 lines match");
    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(pattern(&app), "ERROR");
    assert_eq!(status_line(&mut app), "2 of 4 lines match");

    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(pattern(&app), "ERROR", "undo went past the first version");
    assert_eq!(message(&app), Some("no older version of the pattern"));
    assert_eq!(versions(&app), ["ERROR", "ERROR d", "INFO"]);
}

#[test]
fn ctrl_y_goes_forward_again() {
    let mut app = app_with_three_versions("undo_redo");
    ctrl(&mut app, KeyCode::Char('z'));
    ctrl(&mut app, KeyCode::Char('z'));

    ctrl(&mut app, KeyCode::Char('y'));
    assert_eq!(pattern(&app), "ERROR d");
    ctrl(&mut app, KeyCode::Char('y'));
    assert_eq!(pattern(&app), "INFO", "the pattern undo left came back");
    assert_eq!(editor(&app).regex.as_ref().map(Regex::as_str), Some("INFO"));
    ctrl(&mut app, KeyCode::Char('y'));
    assert_eq!(message(&app), Some("no newer version of the pattern"));
}

#[test]
fn an_edit_after_an_undo_removes_the_redo_history() {
    let mut app = app_with_three_versions("undo_edit");
    ctrl(&mut app, KeyCode::Char('z'));
    ctrl(&mut app, KeyCode::Char('z'));
    typed(&mut app, "x");

    assert_eq!(versions(&app), ["ERROR"]);
    ctrl(&mut app, KeyCode::Char('y'));
    assert_eq!(pattern(&app), "ERRORx");
    assert_eq!(message(&app), Some("no newer version of the pattern"));

    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(pattern(&app), "ERROR");
    ctrl(&mut app, KeyCode::Char('y'));
    assert_eq!(pattern(&app), "ERRORx", "the edit is itself a version");
}

/// The failed-check count follows the version, as the match count does.
#[test]
fn the_checks_agree_with_each_version() {
    let mut app = app_with_three_versions("undo_checks");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('+')); // ERROR timeout
    assert_eq!(status_line(&mut app), "2 of 4 lines match · 1 check fails");

    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(pattern(&app), "ERROR d");
    assert_eq!(status_line(&mut app), "1 of 4 lines match · 1 check fails");
    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(status_line(&mut app), "2 of 4 lines match · 0 checks fail");
}

/// Tab away from the pattern keeps it, as a pause does.
#[test]
fn tab_keeps_the_pattern_as_a_version() {
    let mut app = app_over_file("undo_tab", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Tab);
    assert_eq!(versions(&app), ["ERROR"]);
}

/// A pattern that does not compile is not a version: undo goes back to
/// the last one that did.
#[test]
fn a_pattern_that_does_not_compile_is_not_a_version() {
    let mut app = app_over_file("undo_invalid", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    pause(&mut app);
    typed(&mut app, "(");
    pause(&mut app);
    typed(&mut app, "x");
    assert_eq!(versions(&app), ["ERROR"]);

    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(pattern(&app), "ERROR");
    assert!(editor(&app).error.is_none());
}

#[test]
fn f_big_c_starts_with_the_filters_pattern_as_the_first_version() {
    let mut app = app_with_two_filters("undo_f_c");
    open_selected(&mut app);
    let first = pattern(&app).to_string();
    assert_eq!(versions(&app), [first.as_str()]);

    typed(&mut app, "x");
    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(pattern(&app), first);
}

/// The history lives only while the editor is open.
#[test]
fn the_history_goes_with_the_editor() {
    let mut app = app_with_three_versions("undo_close");
    key(&mut app, KeyCode::Esc);
    open_editor(&mut app);
    assert!(versions(&app).is_empty());
}

#[test]
fn the_help_overlay_shows_the_undo_keys() {
    let named = |name: &str| {
        crate::help::KEYMAP
            .iter()
            .flat_map(|section| section.bindings)
            .any(|binding| binding.names.contains(&name))
    };
    assert!(named("filtereditor.undo") && named("filtereditor.redo"));
}

// ---- the name, description and prompt (#317) -------------------------------

/// Shift-Tab from wherever the keys are until they are on `field`.
fn focus_field(app: &mut App, field: EditorFocus) {
    while editor(app).focus != field {
        key(app, KeyCode::BackTab);
    }
}

/// Type `text` into `field`, and come back to the pattern.
fn type_in_field(app: &mut App, field: EditorFocus, text: &str) {
    focus_field(app, field);
    typed(app, text);
    focus_field(app, EditorFocus::Pattern);
}

#[test]
fn tab_and_shift_tab_go_round_the_fields_and_the_lines() {
    let mut app = app_over_file("details_ring", BODY);
    open_editor_from_the_filter_pane(&mut app);
    let focus = |app: &App| editor(app).focus;
    assert_eq!(focus(&app), EditorFocus::Pattern, "it opens on the pattern");
    for expected in [
        EditorFocus::Lines,
        EditorFocus::Name,
        EditorFocus::Description,
        EditorFocus::Prompt,
        EditorFocus::Sense,
        EditorFocus::Pattern,
    ] {
        key(&mut app, KeyCode::Tab);
        assert_eq!(focus(&app), expected);
    }
    for expected in [
        EditorFocus::Sense,
        EditorFocus::Prompt,
        EditorFocus::Description,
        EditorFocus::Name,
        EditorFocus::Lines,
        EditorFocus::Pattern,
    ] {
        key(&mut app, KeyCode::BackTab);
        assert_eq!(focus(&app), expected);
    }
}

#[test]
fn the_panel_shows_the_four_fields() {
    let mut app = app_over_file("details_panel", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "timeout");
    type_in_field(&mut app, EditorFocus::Name, "slow");
    type_in_field(&mut app, EditorFocus::Description, "why it exists");
    type_in_field(&mut app, EditorFocus::Prompt, "lines that time out");

    let screen = rendered(&mut app);
    for row in [
        "name:        slow",
        "description: why it exists",
        "prompt:      lines that time out",
        "sense:       include  context  exclude",
        "pattern:     timeout",
    ] {
        assert!(screen.contains(row), "no {row:?} in\n{screen}");
    }
}

/// The fields are plain text: a character goes into the field with the
/// focus, and the pattern, its highlight and its versions do not change.
#[test]
fn typing_in_a_field_leaves_the_pattern_alone() {
    let mut app = app_over_file("details_typing", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    focus_field(&mut app, EditorFocus::Prompt);
    typed(&mut app, "(+-u");
    key(&mut app, KeyCode::Backspace);

    let editor = editor(&app);
    assert_eq!(editor.prompt.pattern, "(+-");
    assert_eq!(editor.field.pattern, "ERROR");
    assert!(editor.error.is_none(), "the prompt is not a regex");
    assert_eq!(editor.matches, 2);
    assert!(editor.marks.is_empty(), "+ and - were marks");
}

/// `Ctrl-z` is the pattern's: in a field it does nothing.
#[test]
fn ctrl_z_in_a_field_does_not_change_the_pattern() {
    let mut app = app_with_three_versions("details_undo");
    focus_field(&mut app, EditorFocus::Description);
    typed(&mut app, "why");
    ctrl(&mut app, KeyCode::Char('z'));

    assert_eq!(pattern(&app), "INFO");
    assert_eq!(editor(&app).description.pattern, "why");
}

#[test]
fn enter_gives_a_new_filter_its_name_description_and_prompt() {
    let mut app = app_over_file("details_new", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "timeout");
    type_in_field(&mut app, EditorFocus::Name, "slow");
    type_in_field(&mut app, EditorFocus::Description, "  why it exists ");
    key(&mut app, KeyCode::Enter);

    assert!(app.filter_editor.is_none());
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.predicate.display(), "timeout");
    assert_eq!(filter.name.as_deref(), Some("slow"));
    assert_eq!(filter.display_name(), "slow");
    assert_eq!(
        filter.description.as_deref(),
        Some("why it exists"),
        "not trimmed"
    );
    assert_eq!(filter.prompt, None, "an empty field is no key");
}

#[test]
fn f_big_c_shows_the_filters_fields_and_enter_changes_them() {
    let mut app = app_with_two_filters("details_change");
    app.filters.set_details(
        0,
        crate::filter::Details {
            name: Some("errors".into()),
            description: Some("old".into()),
            prompt: Some("error lines".into()),
            examples: Vec::new(),
            generated_from: None,
        },
    );
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    let editor_now = editor(&app);
    assert_eq!(editor_now.name.pattern, "errors");
    assert_eq!(editor_now.description.pattern, "old");
    assert_eq!(editor_now.prompt.pattern, "error lines");

    focus_field(&mut app, EditorFocus::Description);
    ctrl(&mut app, KeyCode::Char('u'));
    typed(&mut app, "new");
    key(&mut app, KeyCode::Enter);

    let filter = &app.filters.filters()[0];
    assert_eq!(filter.description.as_deref(), Some("new"));
    assert_eq!(filter.name.as_deref(), Some("errors"));
    assert_eq!(filter.predicate.display(), "ERROR");
}

/// A typed filter has no name; the editor does not show its pattern as one.
#[test]
fn f_big_c_on_a_typed_filter_has_an_empty_name() {
    let mut app = app_with_two_filters("details_no_name");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    assert_eq!(editor(&app).name.pattern, "");
}

#[test]
fn esc_discards_the_fields() {
    let mut app = app_with_two_filters("details_esc");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    type_in_field(&mut app, EditorFocus::Name, "renamed");
    type_in_field(&mut app, EditorFocus::Description, "why");
    key(&mut app, KeyCode::Esc);

    let filter = &app.filters.filters()[0];
    assert_eq!(filter.name, None);
    assert_eq!(filter.description, None);
}

/// Two filters in one set cannot answer to one name: the file would not
/// load.
#[test]
fn enter_refuses_a_name_another_filter_in_the_set_has() {
    let mut app = app_with_two_filters("details_taken");
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "timeout");
    type_in_field(&mut app, EditorFocus::Name, "ERROR");
    key(&mut app, KeyCode::Enter);

    assert!(app.filter_editor.is_some(), "Enter closed the editor");
    assert_eq!(
        editor(&app).error.as_deref(),
        Some("another filter in this set is named \"ERROR\"")
    );
    assert_eq!(app.filters.len(), 2, "a filter was added");

    focus_field(&mut app, EditorFocus::Name);
    typed(&mut app, "S");
    assert_eq!(editor(&app).error, None, "the reason stayed");
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none());
    assert_eq!(
        app.filters.filters()[..3]
            .iter()
            .filter(|f| f.name.as_deref() == Some("ERRORS"))
            .count(),
        1
    );
}

/// `S` writes the fields, and the file loads back to the same fields.
#[test]
fn big_s_writes_the_fields_and_they_load_again() {
    let path = save_fixture("details_save");
    let mut app = app_over_file("details_save_file", BODY);
    app.save_path = Some(path.clone());
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "timeout");
    type_in_field(&mut app, EditorFocus::Name, "slow");
    type_in_field(&mut app, EditorFocus::Description, "Instances of bug #57");
    type_in_field(
        &mut app,
        EditorFocus::Prompt,
        "Timeouts, except in 'DEMO' runs",
    );
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "bugs");
    key(&mut app, KeyCode::Enter);

    let text = std::fs::read_to_string(&path).expect("written");
    let sets = crate::filtersets::parse(&text, &path).expect("loads again");
    let bugs = sets.iter().find(|set| set.name == "bugs").expect("bugs");
    let filter = &bugs.filters[0];
    assert_eq!(filter.name, "slow");
    assert_eq!(filter.description.as_deref(), Some("Instances of bug #57"));
    assert_eq!(
        filter.prompt.as_deref(),
        Some("Timeouts, except in 'DEMO' runs")
    );
    assert_eq!(filter.predicate.display(), "timeout");
    assert_eq!(bugs.profiles["default"], ["slow"]);
}

/// Outside the editor, the status row shows the description of the filter
/// the filter pane's selection is on.
#[test]
fn the_status_row_shows_the_selected_filters_description() {
    let mut app = app_with_two_filters("details_status");
    app.filters.set_details(
        0,
        crate::filter::Details {
            description: Some("errors of every kind".into()),
            ..crate::filter::Details::default()
        },
    );
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    assert!(
        status_line(&mut app).ends_with("errors of every kind"),
        "{}",
        status_line(&mut app)
    );

    select_row(&mut app, widgets::filterlist::Row::Filter(1));
    assert!(!status_line(&mut app).contains("errors of every kind"));
}

/// A file filter without a `name` answers to its pattern. A new pattern
/// from `f C` keeps that name, as `f c` does, so the set's profile still
/// finds the filter.
#[test]
fn f_big_c_keeps_a_file_filters_name_when_the_pattern_changes() {
    let mut app = app_with_three_sets("details_file_name");
    let (alpha, _) = app.filters.filters_in(1).next().expect("alpha");
    select_row(&mut app, widgets::filterlist::Row::Filter(alpha));
    open_selected(&mut app);
    assert_eq!(editor(&app).name.pattern, "alpha");
    typed(&mut app, "!");
    key(&mut app, KeyCode::Enter);

    let filter = &app.filters.filters()[alpha];
    assert_eq!(filter.predicate.display(), "alpha!");
    assert_eq!(filter.display_name(), "alpha");
    assert_eq!(app.filters.sets()[1].profiles["default"], ["alpha"]);
}

// ---- the sense (#317) ------------------------------------------------------

#[test]
fn the_sense_keys_choose_and_cycle_the_sense() {
    let mut app = app_over_file("sense_keys", BODY);
    open_editor_from_the_filter_pane(&mut app);
    let sense = |app: &App| editor(app).sense;
    assert_eq!(sense(&app), Sense::Include, "a new filter includes");
    focus_field(&mut app, EditorFocus::Sense);

    key(&mut app, KeyCode::Char(' '));
    assert_eq!(sense(&app), Sense::Context);
    key(&mut app, KeyCode::Right);
    assert_eq!(sense(&app), Sense::Exclude);
    key(&mut app, KeyCode::Right);
    assert_eq!(sense(&app), Sense::Include, "Right wraps");
    key(&mut app, KeyCode::Left);
    assert_eq!(sense(&app), Sense::Exclude, "Left wraps");
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(sense(&app), Sense::Context);
    key(&mut app, KeyCode::Char('i'));
    assert_eq!(sense(&app), Sense::Include);
    key(&mut app, KeyCode::Char('x'));
    assert_eq!(sense(&app), Sense::Exclude);

    typed(&mut app, "qz+");
    assert_eq!(sense(&app), Sense::Exclude, "another key changed it");
    assert_eq!(editor(&app).field.pattern, "", "a key was typed");
    assert!(editor(&app).marks.is_empty());
}

/// A new excluding filter is what `f x` gives: no colour, and its lines
/// leave the view.
#[test]
fn enter_adds_an_excluding_filter_as_f_x_does() {
    let mut app = app_over_file("sense_exclude", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "INFO");
    focus_field(&mut app, EditorFocus::Sense);
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Enter);

    assert!(app.filter_editor.is_none());
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.sense, Sense::Exclude);
    assert_eq!(
        filter.style,
        Style::default(),
        "an excluding filter has a colour"
    );
    assert_eq!(
        app.document.visible().len(),
        2,
        "the INFO lines are still shown"
    );
}

#[test]
fn enter_adds_a_context_filter() {
    let mut app = app_over_file("sense_context", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "INFO");
    focus_field(&mut app, EditorFocus::Sense);
    key(&mut app, KeyCode::Char('c'));
    key(&mut app, KeyCode::Enter);

    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.sense, Sense::Context);
    assert_ne!(
        filter.style,
        Style::default(),
        "a context filter has no colour"
    );
}

/// `f C` shows the filter's sense, and Enter changes it.
#[test]
fn f_big_c_shows_and_changes_the_sense() {
    let mut app = app_with_two_filters("sense_change");
    select_row(&mut app, widgets::filterlist::Row::Filter(1));
    open_selected(&mut app);
    assert_eq!(editor(&app).sense, Sense::Exclude, "INFO is excluding");
    focus_field(&mut app, EditorFocus::Sense);
    key(&mut app, KeyCode::Char('i'));
    key(&mut app, KeyCode::Enter);

    let filter = &app.filters.filters()[1];
    assert_eq!(filter.sense, Sense::Include);
    assert_ne!(filter.style, Style::default(), "it has no colour");
    assert_eq!(filter.predicate.display(), "INFO");
    assert_eq!(
        app.document.visible().len(),
        4,
        "the INFO lines stay hidden"
    );
}

#[test]
fn esc_discards_the_sense() {
    let mut app = app_with_two_filters("sense_esc");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    focus_field(&mut app, EditorFocus::Sense);
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.filters.filters()[0].sense, Sense::Include);
}

/// `S` writes the sense the editor gave, and it loads again.
#[test]
fn big_s_writes_the_sense_from_the_editor() {
    let path = save_fixture("sense_save");
    let mut app = app_over_file("sense_save_file", BODY);
    app.save_path = Some(path.clone());
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "INFO");
    focus_field(&mut app, EditorFocus::Sense);
    key(&mut app, KeyCode::Char('c'));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "ctx");
    key(&mut app, KeyCode::Enter);

    let text = std::fs::read_to_string(&path).expect("written");
    let sets = crate::filtersets::parse(&text, &path).expect("loads again");
    let ctx = sets.iter().find(|set| set.name == "ctx").expect("ctx");
    assert_eq!(ctx.filters[0].sense, Sense::Context);
}

// ---- examples (#318) -------------------------------------------------------

use crate::filter::Example;

fn example(line: &str, must_match: bool) -> Example {
    Example {
        line: line.into(),
        must_match,
    }
}

/// `ERROR timeout` +, `INFO ok` -, under `ERROR`: both checks pass.
fn app_with_two_passing_checks(name: &str) -> App<'static> {
    let mut app = app_over_file(name, BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-'));
    assert_eq!(editor(&app).failures, 0, "sanity");
    app
}

#[test]
fn enter_keeps_the_marks_as_the_filters_examples() {
    let mut app = app_with_two_passing_checks("examples_new");
    key(&mut app, KeyCode::Enter);

    assert!(app.filter_editor.is_none());
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(
        filter.examples,
        [example("ERROR timeout", true), example("INFO ok", false)]
    );
}

#[test]
fn a_line_the_file_has_twice_is_one_example() {
    let mut app = app_over_file("examples_twice", "ERROR x\nERROR x\nINFO\n");
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Enter);

    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.examples, [example("ERROR x", true)]);
}

#[test]
fn a_filter_without_marks_has_no_examples() {
    let mut app = app_over_file("examples_none", BODY);
    open_editor_from_the_filter_pane(&mut app);
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Enter);

    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert!(filter.examples.is_empty());
}

/// Scratch filter 0 is `ERROR`, with `ERROR disk` must-match,
/// `INFO timeout` must-not-match, and `ERROR in another file` must-match,
/// which this file does not have.
fn app_with_examples(name: &str) -> App<'static> {
    let mut app = app_with_two_filters(name);
    app.filters.set_details(
        0,
        crate::filter::Details {
            examples: vec![
                example("ERROR disk", true),
                example("INFO timeout", false),
                example("ERROR in another file", true),
            ],
            ..crate::filter::Details::default()
        },
    );
    app
}

#[test]
fn f_big_c_shows_the_examples_as_marks() {
    let mut app = app_with_examples("examples_marks");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);

    assert_eq!(
        marks(&app),
        [
            (2, Mark::MustMatch),
            (3, Mark::MustNotMatch),
            (4, Mark::MustMatch)
        ],
        "the example the file does not have is line 4, after the last"
    );
    assert_eq!(editor(&app).extra, ["ERROR in another file"]);
    assert_eq!(editor(&app).failures, 0);
    assert_eq!(
        status_line(&mut app),
        "2 of 4 lines match · 1 example not in the file · 0 checks fail",
        "the file's lines are counted, not the example"
    );
    key(&mut app, KeyCode::Tab);
    for _ in 0..4 {
        key(&mut app, KeyCode::Down);
    }
    let screen = rendered(&mut app);
    assert!(
        screen.contains(">+ example, not in the file: ERROR in another file"),
        "{screen}"
    );
}

#[test]
fn an_example_not_in_the_file_is_checked_and_reached_by_f() {
    let mut app = app_with_examples("examples_extra_fails");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    typed(&mut app, " disk");
    assert_eq!(editor(&app).failures, 1, "ERROR in another file fails");
    assert!(
        status_line(&mut app).ends_with("1 check fails"),
        "{}",
        status_line(&mut app)
    );

    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(editor(&app).cursor, 4);
}

#[test]
fn enter_refuses_a_pattern_that_fails_an_example_and_names_it() {
    let mut app = app_with_examples("examples_gate");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    typed(&mut app, " disk");
    key(&mut app, KeyCode::Enter);

    assert!(app.filter_editor.is_some(), "Enter closed the editor");
    assert_eq!(
        editor(&app).error.as_deref(),
        Some("a check fails; fix the pattern or clear the mark: \"ERROR in another file\"")
    );
    assert_eq!(
        app.filters.filters()[0].predicate.display(),
        "ERROR",
        "the pattern changed"
    );

    // Clear the example, and Enter saves without it.
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('='));
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none());
    let filter = &app.filters.filters()[0];
    assert_eq!(filter.predicate.display(), "ERROR disk");
    assert_eq!(
        filter.examples,
        [example("ERROR disk", true), example("INFO timeout", false)]
    );
}

#[test]
fn enter_counts_every_failed_check() {
    let mut app = app_with_four_checks("examples_gate_count");
    let before = app.filters.len();
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        editor(&app).error.as_deref(),
        Some("2 checks fail; fix the pattern or clear the marks. The first: \"ERROR disk\"")
    );
    assert_eq!(app.filters.len(), before, "a filter was added");
}

#[test]
fn the_origin_line_keeps_its_examples_mark() {
    let mut app = app_with_two_filters("examples_origin");
    app.filters.set_details(
        0,
        crate::filter::Details {
            examples: vec![example("ERROR timeout", false)],
            ..crate::filter::Details::default()
        },
    );
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('C'));
    assert_eq!(marks(&app), [(0, Mark::MustNotMatch)]);
}

#[test]
fn esc_leaves_the_examples_as_they_were() {
    let mut app = app_with_examples("examples_esc");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('='));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.filters.filters()[0].examples.len(), 3);
}

/// `f c` edits the pattern without the editor; a stored example still
/// stops a pattern that fails it.
#[test]
fn f_c_refuses_a_pattern_that_fails_an_example() {
    let mut app = app_with_examples("examples_f_c");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    key(&mut app, KeyCode::Char('c'));
    typed(&mut app, " disk");
    key(&mut app, KeyCode::Enter);

    let prompt = app.prompt.as_ref().expect("the prompt stayed open");
    assert_eq!(
        prompt.error.as_deref(),
        Some("fails an example of the filter; f C shows it: \"ERROR in another file\"")
    );
    assert_eq!(app.filters.filters()[0].predicate.display(), "ERROR");

    ctrl(&mut app, KeyCode::Char('u'));
    typed(&mut app, "ERROR|disk");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none());
    assert_eq!(app.filters.filters()[0].predicate.display(), "ERROR|disk");
}

/// `S` writes the examples; a restart loads them, and `f C` shows them as
/// marks again.
#[test]
fn big_s_writes_the_examples_and_a_restart_shows_them_again() {
    let path = save_fixture("examples_save");
    let mut app = app_with_two_passing_checks("examples_save_file");
    app.save_path = Some(path.clone());
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "bugs");
    key(&mut app, KeyCode::Enter);

    let text = std::fs::read_to_string(&path).expect("written");
    assert!(
        text.contains("must_match = [\n    'ERROR timeout',\n]"),
        "{text}"
    );
    let sets = crate::filtersets::parse(&text, &path).expect("loads again");
    let mut app = app_over_file("examples_save_restart", BODY);
    app.filters = ActiveFilters::with_sets(None, &sets);
    app.filters.set_enabled_set(1, true);
    app.refresh_view();
    let (index, filter) = app.filters.filters_in(1).next().expect("the saved filter");
    assert_eq!(
        filter.examples,
        [example("ERROR timeout", true), example("INFO ok", false)]
    );
    select_row(&mut app, widgets::filterlist::Row::Filter(index));
    open_selected(&mut app);
    assert_eq!(marks(&app), [(0, Mark::MustMatch), (1, Mark::MustNotMatch)]);
}

// ---- a pattern from a request (#319) --------------------------------------

use crate::app::filter_editor::NO_REQUEST;
use crate::generate::{Cancel, Candidate, Model};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

/// A model double. It keeps each text it is sent, waits for the test to
/// release it, answers the next of `replies` — the last one again once it
/// is the only one left — and says it is done. It answers a consolidation
/// (#322) with `prompt` in the same way.
struct FakeModel {
    available: bool,
    replies: Mutex<Vec<Result<Candidate, String>>>,
    prompt: Arc<Mutex<Result<String, String>>>,
    sent: Arc<Mutex<Vec<String>>>,
    release: Mutex<Receiver<()>>,
    done: Mutex<Sender<()>>,
}

impl Model for FakeModel {
    fn available(&self) -> bool {
        self.available
    }

    fn generate(&self, text: &str, _cancel: &Cancel) -> Result<Candidate, String> {
        self.sent.lock().expect("sent").push(text.to_string());
        let _ = self.release.lock().expect("release").recv();
        let reply = {
            let mut replies = self.replies.lock().expect("replies");
            if replies.len() > 1 {
                replies.remove(0)
            } else {
                replies[0].clone()
            }
        };
        let _ = self.done.lock().expect("done").send(());
        reply
    }

    fn consolidate(&self, text: &str, _cancel: &Cancel) -> Result<String, String> {
        self.sent.lock().expect("sent").push(text.to_string());
        let _ = self.release.lock().expect("release").recv();
        let reply = self.prompt.lock().expect("prompt").clone();
        let _ = self.done.lock().expect("done").send(());
        reply
    }
}

/// The test's side of a `FakeModel`.
struct Harness {
    sent: Arc<Mutex<Vec<String>>>,
    /// What the model answers a consolidation with.
    prompt: Arc<Mutex<Result<String, String>>>,
    release: Sender<()>,
    done: Receiver<()>,
}

impl Harness {
    /// Let the model answer, and wait until it has.
    fn answer(&self) {
        self.release.send(()).expect("the model thread");
        self.done
            .recv_timeout(Duration::from_secs(5))
            .expect("the model did not answer");
    }

    fn sent(&self) -> Vec<String> {
        self.sent.lock().expect("sent").clone()
    }
}

/// What a `FakeModel` answers a consolidation with, unless the test says.
const MODEL_PROMPT: &str = "the timeout errors, not the DEMO runs";

fn candidate(pattern: &str) -> Candidate {
    Candidate {
        pattern: pattern.to_string(),
        explanation: format!("lines with {pattern}"),
    }
}

/// An app over `BODY` with a model that answers `reply`.
fn app_with_model(
    name: &str,
    available: bool,
    reply: Result<Candidate, String>,
) -> (App<'static>, Harness) {
    app_with_replies(name, available, vec![reply])
}

/// An app over `BODY` with a model that answers `replies` in turn.
fn app_with_replies(
    name: &str,
    available: bool,
    replies: Vec<Result<Candidate, String>>,
) -> (App<'static>, Harness) {
    let (release_tx, release_rx) = channel();
    let (done_tx, done_rx) = channel();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let prompt = Arc::new(Mutex::new(Ok(MODEL_PROMPT.to_string())));
    let model = FakeModel {
        available,
        replies: Mutex::new(replies),
        prompt: Arc::clone(&prompt),
        sent: Arc::clone(&sent),
        release: Mutex::new(release_rx),
        done: Mutex::new(done_tx),
    };
    let app = app_over_file(name, BODY).with_model(Some(Arc::new(model)));
    let harness = Harness {
        sent,
        prompt,
        release: release_tx,
        done: done_rx,
    };
    (app, harness)
}

/// Take the reply the model gave. The worker thread sends it just after
/// the model returns, so it is waited for, not assumed.
fn take_reply(app: &mut App) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while editor(app).running.is_some() {
        assert!(std::time::Instant::now() < deadline, "no reply to take");
        app.drain_request();
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// `Ctrl-g`, the request, and Enter.
fn ask(app: &mut App, request: &str) {
    ctrl(app, KeyCode::Char('g'));
    typed(app, request);
    key(app, KeyCode::Enter);
}

#[test]
fn without_a_model_there_is_no_request_line() {
    let mut app = app_over_file("fe_no_model", BODY);
    open_editor(&mut app);
    ctrl(&mut app, KeyCode::Char('g'));
    assert_eq!(editor(&app).focus, EditorFocus::Pattern);
    key(&mut app, KeyCode::Tab);
    assert_eq!(editor(&app).focus, EditorFocus::Lines);
    let screen = rendered(&mut app);
    assert!(!screen.contains("request:"), "{screen}");
}

/// A build with the model whose model cannot take a request now is the
/// same as a build without one, and says nothing about it.
#[test]
fn a_model_that_is_not_available_shows_no_request_line_and_no_error() {
    let (mut app, _harness) = app_with_model("fe_unavailable", false, Ok(candidate("x")));
    open_editor(&mut app);
    ctrl(&mut app, KeyCode::Char('g'));
    assert_eq!(editor(&app).focus, EditorFocus::Pattern);
    let screen = rendered(&mut app);
    assert!(!screen.contains("request:"), "{screen}");
    assert!(editor(&app).error.is_none());
    assert!(app.status_message.is_none());
}

#[test]
fn a_request_gives_the_models_pattern_and_explanation() {
    let (mut app, harness) = app_with_model("fe_request", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    let screen = rendered(&mut app);
    assert!(screen.contains("request:"), "{screen}");

    ask(&mut app, "the errors");
    assert!(editor(&app).running.is_some());
    assert!(
        status_line(&mut app).contains("asking the model"),
        "{}",
        status_line(&mut app)
    );
    // The UI works while the model thinks.
    assert!(!app.drain_request() || editor(&app).running.is_some());
    key(&mut app, KeyCode::Tab);
    assert_eq!(editor(&app).focus, EditorFocus::Lines);
    key(&mut app, KeyCode::Down);
    assert_eq!(editor(&app).cursor, 1);

    harness.answer();
    take_reply(&mut app);
    let editor = editor(&app);
    assert_eq!(editor.field.pattern, "ERROR");
    assert_eq!(editor.matches, 2);
    assert_eq!(editor.explanation.as_deref(), Some("lines with ERROR"));
    assert_eq!(
        editor.request.pattern, "",
        "the request line was not cleared"
    );
    let screen = rendered(&mut app);
    assert!(screen.contains("lines with ERROR"), "{screen}");
    assert!(!status_line(&mut app).contains("asking the model"));
}

#[test]
fn esc_cancels_a_request_and_its_late_reply_changes_nothing() {
    let (mut app, harness) = app_with_model("fe_cancel", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    typed(&mut app, "INFO");
    ask(&mut app, "the errors");

    key(&mut app, KeyCode::Esc);
    assert!(app.filter_editor.is_some(), "Esc closed the editor");
    assert!(editor(&app).running.is_none());

    harness.answer();
    // Time for the late reply to be sent, were there anywhere to send it.
    std::thread::sleep(Duration::from_millis(20));
    assert!(!app.drain_request());
    assert_eq!(editor(&app).field.pattern, "INFO");
    assert!(editor(&app).explanation.is_none());

    key(&mut app, KeyCode::Esc);
    assert!(
        app.filter_editor.is_none(),
        "a second Esc closes the editor"
    );
}

/// The description is for people only (#317): the text the model gets has
/// the prompt, the marks, other lines and the request, and never it.
#[test]
fn the_description_is_never_sent_to_the_model() {
    let (mut app, harness) = app_with_model("fe_description", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    for _ in 0..3 {
        key(&mut app, KeyCode::BackTab);
    }
    assert_eq!(editor(&app).focus, EditorFocus::Description);
    typed(&mut app, "SECRET REASON");
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "timeouts");
    // Mark `INFO timeout` must-not-match: past the sense, the pattern and
    // the request line to the lines.
    for _ in 0..4 {
        key(&mut app, KeyCode::Tab);
    }
    assert_eq!(editor(&app).focus, EditorFocus::Lines);
    for _ in 0..3 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Char('-'));

    ask(&mut app, "only the errors");
    harness.answer();
    take_reply(&mut app);

    let sent = harness.sent();
    assert_eq!(sent.len(), 1);
    let text = &sent[0];
    assert!(!text.contains("SECRET"), "the description was sent: {text}");
    assert!(!crate::generate::INSTRUCTIONS.contains("SECRET"));
    assert!(text.contains("timeouts"), "no prompt: {text}");
    assert!(
        text.contains("must not match:\nINFO timeout"),
        "no mark: {text}"
    );
    assert!(text.contains("Request: only the errors"), "{text}");
}

#[test]
fn undo_goes_back_to_the_pattern_before_the_models() {
    let (mut app, harness) = app_with_model("fe_undo_model", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    typed(&mut app, "INFO");
    ask(&mut app, "the errors");
    harness.answer();
    take_reply(&mut app);
    assert_eq!(editor(&app).field.pattern, "ERROR");

    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(editor(&app).field.pattern, "INFO");
    assert_eq!(editor(&app).matches, 2);
    ctrl(&mut app, KeyCode::Char('y'));
    assert_eq!(editor(&app).field.pattern, "ERROR");
}

#[test]
fn undo_after_a_first_pattern_from_the_model_empties_the_field() {
    let (mut app, harness) = app_with_model("fe_undo_empty", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    ask(&mut app, "the errors");
    harness.answer();
    take_reply(&mut app);

    ctrl(&mut app, KeyCode::Char('z'));
    assert_eq!(editor(&app).field.pattern, "");
    assert!(editor(&app).regex.is_none());
}

#[test]
fn enter_with_no_request_sends_nothing() {
    let (mut app, harness) = app_with_model("fe_empty_request", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    ask(&mut app, "  ");
    assert!(editor(&app).running.is_none());
    assert_eq!(editor(&app).error.as_deref(), Some(NO_REQUEST));
    assert!(harness.sent().is_empty());
}

#[test]
fn a_models_error_leaves_the_pattern_and_says_why() {
    let (mut app, harness) =
        app_with_model("fe_model_error", true, Err("the model is busy".to_string()));
    open_editor(&mut app);
    typed(&mut app, "INFO");
    ask(&mut app, "the errors");
    harness.answer();
    take_reply(&mut app);
    assert_eq!(editor(&app).field.pattern, "INFO");
    let error = editor(&app).error.clone().unwrap_or_default();
    assert!(error.contains("the model is busy"), "{error}");
}

#[test]
fn closing_the_editor_cancels_the_request() {
    let (mut app, harness) = app_with_model("fe_close_cancel", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    typed(&mut app, "INFO");
    ask(&mut app, "the errors");
    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none());
    harness.answer();
    // Time for the late reply to be sent, were there anywhere to send it.
    std::thread::sleep(Duration::from_millis(20));
    assert!(!app.drain_request());
    assert_eq!(app.filters.len(), 1);
}

/// A second request goes with the first and the pattern the first gave, so
/// the pattern improves step by step.
#[test]
fn a_later_request_has_the_earlier_requests_and_the_pattern() {
    let (mut app, harness) = app_with_model("fe_rounds", true, Ok(candidate("timeout")));
    open_editor(&mut app);
    ask(&mut app, "the timeouts");
    harness.answer();
    take_reply(&mut app);
    let first = harness.sent();
    assert!(!first[0].contains("Earlier requests"), "{}", first[0]);
    assert!(!first[0].contains("Current pattern"), "{}", first[0]);

    ask(&mut app, "also exclude DEMO");
    harness.answer();
    take_reply(&mut app);

    let sent = harness.sent();
    assert!(
        sent[1].contains("Current pattern: timeout\n"),
        "{}",
        sent[1]
    );
    assert!(
        sent[1].contains("Earlier requests, oldest first:\nthe timeouts\n"),
        "{}",
        sent[1]
    );
    assert!(
        sent[1].ends_with("Request: also exclude DEMO\n"),
        "{}",
        sent[1]
    );
    assert_eq!(editor(&app).requests, ["the timeouts", "also exclude DEMO"]);
}

/// A request the model did not act on — cancelled, or failed — is not one
/// of the session's requests.
#[test]
fn a_cancelled_or_failed_request_is_not_kept() {
    let (mut app, harness) = app_with_model("fe_not_kept", true, Err("busy".to_string()));
    open_editor(&mut app);
    ask(&mut app, "the timeouts");
    key(&mut app, KeyCode::Esc);
    harness.answer();

    typed(&mut app, "the errors");
    key(&mut app, KeyCode::Enter);
    harness.answer();
    take_reply(&mut app);
    assert!(editor(&app).requests.is_empty());
}

/// A pattern that does not compile is not sent as the current pattern.
#[test]
fn a_broken_pattern_is_not_sent() {
    let (mut app, harness) = app_with_model("fe_broken_sent", true, Ok(candidate("timeout")));
    open_editor(&mut app);
    typed(&mut app, "timeout(");
    ask(&mut app, "the timeouts");
    harness.answer();
    take_reply(&mut app);
    assert!(!harness.sent()[0].contains("Current pattern"));
}

// ---- the verify loop (#320) -----------------------------------------------

use crate::generate::ATTEMPTS;

/// Let the model answer the attempt that runs, and take the reply. When
/// the loop goes on, wait until the next attempt's text is with the model.
fn answer(app: &mut App, harness: &Harness) {
    harness.answer();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !app.drain_request() {
        assert!(std::time::Instant::now() < deadline, "no reply to take");
        std::thread::sleep(Duration::from_millis(1));
    }
    if let Some(attempt) = editor(app).running.as_ref().map(|asking| asking.attempt) {
        while harness.sent().len() < attempt {
            assert!(std::time::Instant::now() < deadline, "no next attempt");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// The editor with `ERROR timeout` marked must-match and `INFO timeout`
/// must-not-match.
fn open_with_marks(app: &mut App) {
    open_editor(app);
    key(app, KeyCode::Tab);
    key(app, KeyCode::Tab);
    assert_eq!(editor(app).focus, EditorFocus::Lines);
    key(app, KeyCode::Char('+'));
    for _ in 0..3 {
        key(app, KeyCode::Down);
    }
    key(app, KeyCode::Char('-'));
    assert_eq!(marks(app), [(0, Mark::MustMatch), (3, Mark::MustNotMatch)]);
}

#[test]
fn a_pattern_that_fails_a_mark_is_sent_back_and_never_shown() {
    let (mut app, harness) = app_with_replies(
        "fe_verify_mark",
        true,
        vec![Ok(candidate("timeout")), Ok(candidate("ERROR timeout"))],
    );
    open_with_marks(&mut app);
    ask(&mut app, "the error timeouts");

    answer(&mut app, &harness);
    assert!(editor(&app).running.is_some(), "the loop stopped");
    assert_eq!(editor(&app).field.pattern, "", "a failed pattern was shown");
    assert!(editor(&app).explanation.is_none());

    let sent = harness.sent();
    assert_eq!(sent.len(), 2);
    assert!(sent[1].starts_with(&sent[0]), "{}", sent[1]);
    assert!(
        sent[1].contains(
            "Pattern: timeout\nIt matches these lines, which it must not match:\nINFO timeout\n"
        ),
        "{}",
        sent[1]
    );

    answer(&mut app, &harness);
    let editor = editor(&app);
    assert!(editor.running.is_none());
    assert_eq!(editor.field.pattern, "ERROR timeout");
    assert_eq!(editor.failures, 0);
    assert_eq!(editor.requests, ["the error timeouts"]);
}

#[test]
fn a_pattern_that_does_not_compile_is_sent_back_with_the_error() {
    let (mut app, harness) = app_with_replies(
        "fe_verify_compile",
        true,
        vec![Ok(candidate("ERROR(")), Ok(candidate("ERROR"))],
    );
    open_with_marks(&mut app);
    ask(&mut app, "the errors");
    answer(&mut app, &harness);
    assert_eq!(editor(&app).field.pattern, "");
    let sent = harness.sent();
    assert!(
        sent[1].contains("Pattern: ERROR(\nIt does not compile: unclosed group"),
        "{}",
        sent[1]
    );
    answer(&mut app, &harness);
    assert_eq!(editor(&app).field.pattern, "ERROR");
}

#[test]
fn the_loop_stops_at_the_last_attempt_and_says_why() {
    let (mut app, harness) = app_with_model("fe_verify_max", true, Ok(candidate("INFO")));
    open_with_marks(&mut app);
    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::BackTab);
    typed(&mut app, "ERROR");
    ask(&mut app, "the errors");
    for attempt in 1..=ATTEMPTS {
        assert_eq!(
            editor(&app).running.as_ref().map(|a| a.attempt),
            Some(attempt)
        );
        answer(&mut app, &harness);
    }
    assert!(editor(&app).running.is_none());
    assert_eq!(harness.sent().len(), ATTEMPTS);
    let editor = editor(&app);
    assert_eq!(editor.field.pattern, "ERROR", "the pattern changed");
    assert!(editor.requests.is_empty());
    let error = editor.error.clone().unwrap_or_default();
    assert!(
        error.contains(&format!("in {ATTEMPTS} tries"))
            && error.contains("INFO")
            && error.contains("\"ERROR timeout\""),
        "{error}"
    );
    assert!(rendered(&mut app).contains(&format!("in {ATTEMPTS} tries")));
}

#[test]
fn the_status_line_shows_the_attempt() {
    let (mut app, harness) = app_with_replies(
        "fe_verify_status",
        true,
        vec![Ok(candidate("INFO")), Ok(candidate("ERROR"))],
    );
    open_with_marks(&mut app);
    ask(&mut app, "the errors");
    let status = status_line(&mut app);
    assert!(status.contains(&format!("try 1 of {ATTEMPTS}")), "{status}");
    answer(&mut app, &harness);
    let status = status_line(&mut app);
    assert!(status.contains(&format!("try 2 of {ATTEMPTS}")), "{status}");
    answer(&mut app, &harness);
    assert!(!status_line(&mut app).contains("try "));
}

#[test]
fn esc_cancels_the_loop_at_a_later_attempt_and_keeps_the_pattern() {
    let (mut app, harness) = app_with_replies(
        "fe_verify_cancel",
        true,
        vec![Ok(candidate("INFO")), Ok(candidate("ERROR"))],
    );
    open_with_marks(&mut app);
    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::BackTab);
    typed(&mut app, "timeout");
    ask(&mut app, "the errors");
    answer(&mut app, &harness);
    assert_eq!(editor(&app).running.as_ref().map(|a| a.attempt), Some(2));

    key(&mut app, KeyCode::Esc);
    assert!(app.filter_editor.is_some(), "Esc closed the editor");
    assert!(editor(&app).running.is_none());
    harness.answer();
    // Time for the late reply to be sent, were there anywhere to send it.
    std::thread::sleep(Duration::from_millis(20));
    assert!(!app.drain_request());
    assert_eq!(editor(&app).field.pattern, "timeout");
    assert_eq!(harness.sent().len(), 2);
    assert!(editor(&app).requests.is_empty());
}

/// The `?` help says how many tries a request has.
#[test]
fn the_help_says_the_number_of_tries() {
    let action = crate::help::KEYMAP
        .iter()
        .flat_map(|section| section.bindings)
        .find(|binding| binding.names.contains(&"filtereditor.request"))
        .map(|binding| binding.action)
        .expect("a help row for the request line");
    assert!(action.contains(&format!("{ATTEMPTS} tries")), "{action}");
}

// ---- generated filters (#321) ---------------------------------------------

use crate::app::filter_editor::{
    GENERATED, Generated, NEEDS_EXAMPLES, NO_MODEL, NO_PROMPT, PATTERN_CHANGED, PROMPT_CHANGED,
    PROMPT_CHANGED_NO_MODEL,
};
use crate::filter::generated_hash;
use crate::widgets::filterlist::GENERATED_MARK;

/// Scratch filter 0 is `ERROR`, generated from the prompt `error lines`,
/// with `ERROR disk` must-match and `INFO timeout` must-not-match.
fn make_generated(app: &mut App) {
    app.filters.set_details(
        0,
        crate::filter::Details {
            prompt: Some("error lines".into()),
            examples: vec![example("ERROR disk", true), example("INFO timeout", false)],
            generated_from: Some(generated_hash("error lines", "ERROR")),
            ..crate::filter::Details::default()
        },
    );
}

fn app_with_generated(name: &str) -> App<'static> {
    let mut app = app_with_two_filters(name);
    make_generated(&mut app);
    app
}

/// Shift-Tab from the pattern to the prompt.
fn to_prompt(app: &mut App) {
    key(app, KeyCode::BackTab);
    key(app, KeyCode::BackTab);
    assert_eq!(editor(app).focus, EditorFocus::Prompt);
}

/// A request with a prompt and a mark of each kind; the model's pattern
/// and Enter make a generated filter. `S` writes `generated_from`, and a
/// restart shows the filter as generated, in the pane and in the editor.
#[test]
fn a_generated_pattern_is_saved_with_generated_from_and_shown_after_a_restart() {
    let path = save_fixture("generated_save");
    let (mut app, harness) = app_with_model("generated_save_file", true, Ok(candidate("ERROR")));
    app.save_path = Some(path.clone());
    open_editor_from_the_filter_pane(&mut app);
    to_prompt(&mut app);
    typed(&mut app, "error lines");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    ask(&mut app, "the errors");
    harness.answer();
    take_reply(&mut app);
    assert_eq!(editor(&app).generated(), Generated::Yes);
    assert!(rendered(&mut app).contains(GENERATED));
    // `ERROR timeout` must match and `INFO ok` must not.
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-'));
    key(&mut app, KeyCode::Enter);
    // The consolidation step (#322); Esc keeps `error lines`.
    key(&mut app, KeyCode::Esc);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);

    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(
        filter.generated_from,
        Some(generated_hash("error lines", "ERROR"))
    );
    assert!(filter.is_generated());
    let screen = rendered(&mut app);
    assert!(
        screen.contains(&format!("ERROR{GENERATED_MARK}")),
        "{screen}"
    );

    key(&mut app, KeyCode::Char('S'));
    // Before `definitions`, so it is set 1.
    typed(&mut app, "ai");
    key(&mut app, KeyCode::Enter);
    let text = std::fs::read_to_string(&path).expect("written");
    let hash = generated_hash("error lines", "ERROR");
    assert!(
        text.contains(&format!("pattern = 'ERROR'\ngenerated_from = \"{hash}\"\n")),
        "{text}"
    );

    let sets = crate::filtersets::parse(&text, &path).expect("loads again");
    let mut app = app_over_file("generated_restart", BODY);
    app.filters = ActiveFilters::with_sets(None, &sets);
    app.filters.set_enabled_set(1, true);
    app.refresh_view();
    let (index, filter) = app.filters.filters_in(1).next().expect("the saved filter");
    assert!(filter.is_generated());
    let screen = rendered(&mut app);
    assert!(
        screen.contains(&format!("ERROR{GENERATED_MARK}")),
        "{screen}"
    );
    select_row(&mut app, widgets::filterlist::Row::Filter(index));
    open_selected(&mut app);
    assert_eq!(editor(&app).generated(), Generated::Yes);
    assert!(rendered(&mut app).contains(GENERATED));
}

/// A change to the prompt or the pattern in the file, and the filter is
/// an ordinary one on the next load (rule 3).
#[test]
fn a_prompt_or_pattern_changed_in_the_file_removes_the_marker() {
    let hash = generated_hash("error lines", "ERROR");
    let load = |prompt: &str, pattern: &str| {
        let text = format!(
            "[[sets.a.filters]]\nprompt = \"{prompt}\"\npattern = '{pattern}'\ngenerated_from = \"{hash}\"\n"
        );
        let sets = crate::filtersets::parse(&text, std::path::Path::new("f.toml")).expect("loads");
        let filters = ActiveFilters::with_sets(None, &sets);
        let (_, filter) = filters.filters_in(1).next().expect("the filter");
        filter.is_generated()
    };
    assert!(load("error lines", "ERROR"), "the hash agrees");
    assert!(!load("error lines!", "ERROR"), "the prompt changed");
    assert!(!load("error lines", "ERROR|WARN"), "the pattern changed");
}

/// A hand edit of a generated pattern removes the generated state, and the
/// editor asks about the prompt (rule 4).
#[test]
fn a_hand_edit_removes_the_marker_and_asks_about_the_prompt() {
    let mut app = app_with_generated("generated_hand_edit");
    let screen = rendered(&mut app);
    assert!(
        screen.contains(&format!("ERROR{GENERATED_MARK}")),
        "{screen}"
    );
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    assert_eq!(editor(&app).generated(), Generated::Yes);

    typed(&mut app, "|disk");
    assert_eq!(editor(&app).generated(), Generated::PatternChanged);
    let screen = rendered(&mut app);
    assert!(screen.contains(PATTERN_CHANGED), "{screen}");
    assert!(!screen.contains(GENERATED), "{screen}");

    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let filter = &app.filters.filters()[0];
    assert_eq!(filter.predicate.display(), "ERROR|disk");
    assert_eq!(filter.generated_from, None);
    assert!(!rendered(&mut app).contains(GENERATED_MARK));
}

/// Without a model the prompt can change; the pattern does not, and the
/// filter becomes an ordinary one (rule 5). `Ctrl-r` says there is no
/// model.
#[test]
fn without_a_model_the_prompt_changes_and_the_pattern_does_not() {
    let mut app = app_with_generated("generated_no_model");
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    to_prompt(&mut app);
    typed(&mut app, " and disks");
    assert_eq!(editor(&app).generated(), Generated::PromptChanged);
    let screen = rendered(&mut app);
    assert!(screen.contains(PROMPT_CHANGED_NO_MODEL), "{screen}");

    ctrl(&mut app, KeyCode::Char('r'));
    assert_eq!(
        app.status_message.as_ref().map(|m| m.text.as_str()),
        Some(NO_MODEL)
    );
    assert_eq!(editor(&app).field.pattern, "ERROR");

    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let filter = &app.filters.filters()[0];
    assert_eq!(filter.prompt.as_deref(), Some("error lines and disks"));
    assert_eq!(filter.predicate.display(), "ERROR");
    assert!(!filter.is_generated());
}

/// With a model, a changed prompt says `Ctrl-r` regenerates the pattern.
#[test]
fn with_a_model_a_changed_prompt_names_ctrl_r() {
    let (mut app, _harness) = app_with_model("generated_prompt_model", true, Ok(candidate("x")));
    app.add_filter("ERROR").expect("valid");
    make_generated(&mut app);
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    to_prompt(&mut app);
    typed(&mut app, "!");
    let screen = rendered(&mut app);
    assert!(screen.contains(PROMPT_CHANGED), "{screen}");
}

/// Loading generated filters never asks the model anything, also when a
/// hash no longer agrees, as after a new model (rule 1).
#[test]
fn a_load_never_calls_the_model() {
    let (mut app, harness) = app_with_model("generated_load", true, Ok(candidate("x")));
    let text = format!(
        "[sets.a]\nautoload = true\n\n[[sets.a.filters]]\nprompt = \"error lines\"\npattern = 'ERROR'\ngenerated_from = \"{}\"\n\n\
         [[sets.a.filters]]\nprompt = \"info lines\"\npattern = 'INFO'\ngenerated_from = \"0000000000000000\"\n",
        generated_hash("error lines", "ERROR")
    );
    let sets = crate::filtersets::parse(&text, std::path::Path::new("f.toml")).expect("loads");
    app.filters = ActiveFilters::with_sets(None, &sets);
    app.refresh_view();
    draw(&mut app);
    std::thread::sleep(Duration::from_millis(20));
    assert!(!app.drain_request());

    assert!(harness.sent().is_empty(), "{:?}", harness.sent());
    let generated: Vec<bool> = app
        .filters
        .filters_in(1)
        .map(|(_, filter)| filter.is_generated())
        .collect();
    assert_eq!(generated, [true, false]);
    assert_eq!(
        app.filters
            .filters_in(1)
            .nth(1)
            .map(|(_, f)| f.predicate.display()),
        Some("INFO".to_string()),
        "the pattern changed on load"
    );
}

/// A generated filter needs a must-match and a must-not-match line
/// (rule 7); Enter says why it refuses.
#[test]
fn enter_refuses_a_generated_filter_without_an_example_of_each_kind() {
    let (mut app, harness) = app_with_model("generated_examples", true, Ok(candidate("ERROR")));
    open_editor_from_the_filter_pane(&mut app);
    to_prompt(&mut app);
    typed(&mut app, "error lines");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    ask(&mut app, "the errors");
    harness.answer();
    take_reply(&mut app);
    // Only `ERROR timeout`, must match.
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('+'));
    let before = app.filters.len();
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_some(), "Enter closed the editor");
    assert_eq!(editor(&app).error.as_deref(), Some(NEEDS_EXAMPLES));
    assert_eq!(app.filters.len(), before, "a filter was added");

    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-'));
    key(&mut app, KeyCode::Enter);
    // The consolidation step (#322); Esc keeps `error lines`.
    key(&mut app, KeyCode::Esc);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert!(filter.is_generated());
}

/// An ordinary filter needs no examples: the rule is for generated ones.
#[test]
fn an_ordinary_filter_with_a_prompt_needs_no_examples() {
    let mut app = app_over_file("generated_ordinary", BODY);
    open_editor_from_the_filter_pane(&mut app);
    to_prompt(&mut app);
    typed(&mut app, "error lines");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.generated_from, None);
}

/// `Ctrl-r` sends the prompt and the marks, not the pattern and not the
/// earlier requests, and its pattern goes through the verify loop, which
/// tests it against the stored examples too (#320's regression gate).
#[test]
fn ctrl_r_regenerates_the_pattern_through_the_verify_loop() {
    let (mut app, harness) = app_with_replies(
        "generated_regenerate",
        true,
        // Fails the example `INFO timeout`, then passes.
        vec![
            Ok(candidate("timeout|disk")),
            Ok(candidate("ERROR (disk|timeout)")),
        ],
    );
    app.add_filter("ERROR").expect("valid");
    make_generated(&mut app);
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    ctrl(&mut app, KeyCode::Char('r'));
    assert!(editor(&app).running.is_some());
    answer(&mut app, &harness);
    assert_eq!(
        editor(&app).running.as_ref().map(|a| a.attempt),
        Some(2),
        "the first pattern fails an example"
    );
    answer(&mut app, &harness);

    let sent = harness.sent();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].contains("What the lines to match look like: error lines"));
    assert!(sent[0].contains("Lines the pattern must match:\nERROR disk\n"));
    assert!(sent[0].contains("Lines the pattern must not match:\nINFO timeout\n"));
    assert!(!sent[0].contains("Current pattern"), "{}", sent[0]);
    assert!(sent[0].contains(crate::generate::REGENERATE));
    assert!(sent[1].contains("Pattern: timeout|disk"), "{}", sent[1]);

    assert_eq!(editor(&app).field.pattern, "ERROR (disk|timeout)");
    assert_eq!(editor(&app).generated(), Generated::Yes);
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let filter = &app.filters.filters()[0];
    assert_eq!(filter.predicate.display(), "ERROR (disk|timeout)");
    assert!(filter.is_generated());
}

/// `Ctrl-r` with no prompt has nothing to regenerate from.
#[test]
fn ctrl_r_without_a_prompt_says_so() {
    let (mut app, harness) = app_with_model("generated_no_prompt", true, Ok(candidate("x")));
    open_editor(&mut app);
    ctrl(&mut app, KeyCode::Char('r'));
    assert_eq!(editor(&app).error.as_deref(), Some(NO_PROMPT));
    assert!(editor(&app).running.is_none());
    assert!(harness.sent().is_empty());
}

/// A pattern the model wrote with no prompt is not generated: there is no
/// prompt to regenerate it from.
#[test]
fn a_model_pattern_with_no_prompt_is_ordinary() {
    let (mut app, harness) = app_with_model("generated_empty_prompt", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    ask(&mut app, "the errors");
    harness.answer();
    take_reply(&mut app);
    assert_eq!(editor(&app).generated(), Generated::No);
    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::Enter);
    // The consolidation step (#322); Esc keeps the empty prompt.
    key(&mut app, KeyCode::Esc);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.generated_from, None);
}

// ---- the consolidated prompt (#322) ---------------------------------------

use crate::app::filter_editor::{CONSOLIDATED, CONSOLIDATING, Phrase};

/// A model session over `BODY`: the prompt `errors`, the request `the
/// timeouts` answered with `timeout`, and `ERROR timeout` must-match and
/// `INFO ok` must-not-match. Enter opens the consolidation step.
fn app_in_a_session(name: &str) -> (App<'static>, Harness) {
    let (mut app, harness) = app_with_model(name, true, Ok(candidate("timeout")));
    open_editor_from_the_filter_pane(&mut app);
    to_prompt(&mut app);
    typed(&mut app, "errors");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    ask(&mut app, "the timeouts");
    harness.answer();
    take_reply(&mut app);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-'));
    (app, harness)
}

/// Take the consolidation's reply, waited for as `take_reply` waits.
fn take_prompt(app: &mut App) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while editor(app)
        .consolidation
        .as_ref()
        .is_some_and(|step| step.running.is_some())
    {
        assert!(std::time::Instant::now() < deadline, "no prompt to take");
        app.drain_request();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn enter_after_a_request_shows_the_models_prompt_to_edit_before_the_save() {
    let (mut app, harness) = app_in_a_session("consolidate_edit");
    let before = app.filters.len();
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_some(), "Enter saved at once");
    assert_eq!(app.filters.len(), before);
    assert_eq!(editor(&app).focus, EditorFocus::Prompt);
    let status = status_line(&mut app);
    assert!(status.contains(CONSOLIDATING), "{status}");

    // What the model receives: the prompt before, the request, the pattern.
    harness.answer();
    take_prompt(&mut app);
    let sent = harness.sent();
    let text = sent.last().expect("a consolidation");
    assert!(
        text.contains("The filter's description before the steps: errors"),
        "{text}"
    );
    assert!(
        text.contains("The steps, oldest first:\nthe timeouts\n"),
        "{text}"
    );
    assert!(
        text.contains("The regular expression the steps gave: timeout"),
        "{text}"
    );

    assert_eq!(editor(&app).prompt.pattern, MODEL_PROMPT);
    let status = status_line(&mut app);
    assert!(status.contains("the model's prompt"), "{status}");
    assert!(CONSOLIDATED.contains("Esc keeps the prompt as it was"));
    // `f`, `+` and the other keys of the lines are typed into the prompt.
    typed(&mut app, " of +f");
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    let prompt = format!("{MODEL_PROMPT} of +f");
    assert_eq!(filter.prompt.as_deref(), Some(prompt.as_str()));
    assert_eq!(filter.predicate.display(), "timeout");
    // The model wrote the pattern for the requests the prompt now says.
    assert_eq!(
        filter.generated_from,
        Some(generated_hash(&prompt, "timeout"))
    );
}

/// Esc keeps the prompt from before the step and still saves the pattern:
/// after the model's answer, and while the model still writes.
#[test]
fn esc_in_the_step_keeps_the_earlier_prompt_and_saves() {
    let (mut app, harness) = app_in_a_session("consolidate_esc_after");
    key(&mut app, KeyCode::Enter);
    harness.answer();
    take_prompt(&mut app);
    key(&mut app, KeyCode::Esc);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.prompt.as_deref(), Some("errors"));
    assert_eq!(filter.predicate.display(), "timeout");
    assert!(filter.is_generated());

    let (mut app, _harness) = app_in_a_session("consolidate_esc_during");
    key(&mut app, KeyCode::Enter);
    assert!(editor(&app).consolidation.is_some());
    key(&mut app, KeyCode::Esc);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.prompt.as_deref(), Some("errors"));
    assert_eq!(filter.predicate.display(), "timeout");
}

/// While the model writes, Enter and the characters do nothing: the answer
/// would replace what they did.
#[test]
fn while_the_model_writes_only_esc_acts() {
    let (mut app, harness) = app_in_a_session("consolidate_waiting");
    key(&mut app, KeyCode::Enter);
    typed(&mut app, "abc");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Tab);
    assert!(app.filter_editor.is_some());
    assert_eq!(editor(&app).prompt.pattern, "errors");
    assert_eq!(editor(&app).focus, EditorFocus::Prompt);
    harness.answer();
    take_prompt(&mut app);
    assert_eq!(editor(&app).prompt.pattern, MODEL_PROMPT);
}

#[test]
fn a_session_with_no_request_saves_at_once() {
    let (mut app, harness) = app_with_model("consolidate_none", true, Ok(candidate("x")));
    open_editor(&mut app);
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    assert!(harness.sent().is_empty(), "{:?}", harness.sent());
}

/// `Ctrl-r` writes the pattern from the prompt alone and starts the
/// session's requests again, so there is nothing to consolidate after it.
#[test]
fn after_ctrl_r_enter_saves_at_once() {
    let (mut app, harness) = app_in_a_session("consolidate_after_regenerate");
    ctrl(&mut app, KeyCode::Char('r'));
    answer(&mut app, &harness);
    let asked = harness.sent().len();
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    assert_eq!(harness.sent().len(), asked, "the model was asked again");
}

/// A model that gives no prompt leaves the prompt as it was, to edit or
/// save.
#[test]
fn a_failed_consolidation_keeps_the_prompt_as_it_was() {
    let (mut app, harness) = app_in_a_session("consolidate_failed");
    *harness.prompt.lock().expect("prompt") = Err("busy".to_string());
    key(&mut app, KeyCode::Enter);
    harness.answer();
    take_prompt(&mut app);
    let error = editor(&app).error.clone().unwrap_or_default();
    assert!(error.contains("the model gave no prompt: busy"), "{error}");
    assert_eq!(editor(&app).prompt.pattern, "errors");
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.prompt.as_deref(), Some("errors"));
}

/// A model's prompt can make a filter generated that was not: one without
/// a mark of each kind is then refused, and the next Enter, with the
/// requests spent, does not ask the model again.
#[test]
fn a_prompt_that_makes_the_filter_generated_needs_its_examples() {
    let (mut app, harness) = app_with_model("consolidate_examples", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    ask(&mut app, "the errors");
    harness.answer();
    take_reply(&mut app);
    // Off the request line, where Enter sends a request.
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    harness.answer();
    take_prompt(&mut app);
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_some(), "saved without examples");
    assert_eq!(editor(&app).error.as_deref(), Some(NEEDS_EXAMPLES));
    assert!(editor(&app).consolidation.is_none());
    assert_eq!(editor(&app).prompt.pattern, MODEL_PROMPT);

    focus_field(&mut app, EditorFocus::Lines);
    key(&mut app, KeyCode::Char('+'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('-'));
    let asked = harness.sent().len();
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    assert_eq!(harness.sent().len(), asked);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert!(filter.is_generated());
}

/// A pattern changed by hand after the request is not generated from the
/// model's prompt.
#[test]
fn a_hand_edited_pattern_is_not_generated_from_the_models_prompt() {
    let (mut app, harness) = app_in_a_session("consolidate_hand_edit");
    focus_field(&mut app, EditorFocus::Pattern);
    typed(&mut app, "|x");
    key(&mut app, KeyCode::Enter);
    harness.answer();
    take_prompt(&mut app);
    key(&mut app, KeyCode::Enter);
    assert!(app.filter_editor.is_none(), "{:?}", editor(&app).error);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert_eq!(filter.prompt.as_deref(), Some(MODEL_PROMPT));
    assert_eq!(filter.generated_from, None);
}

// ---- phrase marks (#322) --------------------------------------------------

/// The screen cell of byte `at` of line `line`'s text, as drawn.
fn cell_of(app: &mut App, line: u16, at: u16) -> (u16, u16) {
    draw(app);
    let area = editor(app).lines_area;
    (area.x + 3 + at, area.y + line)
}

/// A drag with the left button from byte `from` to byte `to` of line
/// `line`, as drawn with no scroll.
fn drag_phrase(app: &mut App, line: u16, from: u16, to: u16) {
    let (x, y) = cell_of(app, line, from);
    mouse_at(app, MouseEventKind::Down(MouseButton::Left), x, y);
    let (x, y) = cell_of(app, line, to);
    mouse_at(app, MouseEventKind::Drag(MouseButton::Left), x, y);
    mouse_at(app, MouseEventKind::Up(MouseButton::Left), x, y);
}

#[test]
fn a_drag_along_a_line_marks_a_phrase() {
    let mut app = app_over_file("phrase_drag", BODY);
    open_editor(&mut app);
    typed(&mut app, "ERROR");
    // `timeout` of `ERROR timeout`, dragged from its end to its start.
    drag_phrase(&mut app, 0, 12, 6);
    assert_eq!(
        editor(&app).phrases,
        [Phrase {
            line: 0,
            start: 6,
            end: 13
        }]
    );
    // A click is not a drag, and marks nothing.
    let (x, y) = cell_of(&mut app, 1, 2);
    mouse_at(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse_at(&mut app, MouseEventKind::Up(MouseButton::Left), x, y);
    assert_eq!(editor(&app).phrases.len(), 1);
    // A drag over it takes its place.
    drag_phrase(&mut app, 0, 0, 7);
    assert_eq!(
        editor(&app).phrases,
        [Phrase {
            line: 0,
            start: 0,
            end: 8
        }]
    );
}

/// A phrase mark is underlined in bold over the line's own style, with `~`
/// in the gutter, so it is not read as a line mark.
#[test]
fn a_phrase_mark_is_drawn_other_than_a_line_mark() {
    let mut app = app_over_file("phrase_style", BODY);
    open_editor(&mut app);
    typed(&mut app, "ERROR");
    drag_phrase(&mut app, 0, 6, 12);
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let area = editor(&app).lines_area;
    let cell = |x: u16| &buf[(area.x + x, area.y)];
    assert_eq!(cell(1).symbol(), "~");
    assert!(cell(3 + 6).modifier.contains(Modifier::UNDERLINED));
    assert!(!cell(3 + 5).modifier.contains(Modifier::UNDERLINED));
    // The match is still reversed, and the phrase does not change it.
    assert!(cell(3).modifier.contains(Modifier::REVERSED));
    assert!(!cell(3 + 6).modifier.contains(Modifier::REVERSED));
}

/// Phrase marks are hints: they go to the model and are never checks.
#[test]
fn a_phrase_mark_goes_to_the_model_and_is_not_a_check() {
    let (mut app, harness) = app_with_model("phrase_model", true, Ok(candidate("ERROR")));
    open_editor(&mut app);
    typed(&mut app, "INFO");
    drag_phrase(&mut app, 0, 6, 12);
    assert_eq!(editor(&app).failures, 0);
    assert!(editor(&app).marks.is_empty());
    assert!(!status_line(&mut app).contains("check"));
    ask(&mut app, "the timeouts");
    harness.answer();
    take_reply(&mut app);
    let sent = harness.sent();
    assert!(
        sent[0].contains("\"timeout\" in the line: ERROR timeout"),
        "{}",
        sent[0]
    );
    // Enter keeps no phrase marks as examples.
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    harness.answer();
    take_prompt(&mut app);
    assert!(
        harness.sent()[1].contains("\"timeout\" in the line: ERROR timeout"),
        "the consolidation has the phrase marks too"
    );
    // Esc: the model's prompt would make it generated, which needs marks.
    key(&mut app, KeyCode::Esc);
    let (_, filter) = app.filters.filters_in(0).next().expect("a scratch filter");
    assert!(filter.examples.is_empty(), "{:?}", filter.examples);
}

/// `=` on the lines removes a line's phrase marks with its line mark.
#[test]
fn equals_clears_the_phrase_marks_of_the_line() {
    let mut app = app_over_file("phrase_clear", BODY);
    open_editor(&mut app);
    drag_phrase(&mut app, 0, 0, 4);
    drag_phrase(&mut app, 2, 0, 4);
    focus_field(&mut app, EditorFocus::Lines);
    key(&mut app, KeyCode::Char('='));
    assert_eq!(
        editor(&app).phrases,
        [Phrase {
            line: 2,
            start: 0,
            end: 5
        }]
    );
}
