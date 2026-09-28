use super::*;
use crate::app::filter_editor::NO_PATTERN;
use crate::filter::Sense;
use regex::Regex;

// ---- the filter editor (#312) --------------------------------------------

const BODY: &str = "ERROR timeout\nINFO ok\nERROR disk\nINFO timeout\n";

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
    typed(&mut app, "!");
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

    key(&mut app, KeyCode::Tab);
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
    key(&mut app, KeyCode::Tab);
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
    key(&mut app, KeyCode::Tab);
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
