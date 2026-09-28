use super::*;
use crate::app::filter_editor::NO_PATTERN;
use crate::filter::Sense;
use regex::Regex;

// ---- the filter editor (#312) --------------------------------------------

const BODY: &str = "ERROR timeout\nINFO ok\nERROR disk\nINFO timeout\n";

/// The shared screen, three rows taller: the editor's panel is seven rows
/// (#317), and on the shared ten the file would get none. These shadow the
/// shared `AREA`, `draw`, `rendered` and `status_line` in this file.
const AREA: Rect = Rect {
    height: super::AREA.height + 3,
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
    // seven rows above the status row.
    let corners = |app: &mut App| {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        let panel = AREA.height - 1 - 7;
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

/// Type `text` into the field that `tabs` presses of Shift-Tab reach from
/// the pattern: 1 the prompt, 2 the description, 3 the name.
fn type_in_field(app: &mut App, tabs: usize, text: &str) {
    for _ in 0..tabs {
        key(app, KeyCode::BackTab);
    }
    typed(app, text);
    for _ in 0..tabs {
        key(app, KeyCode::Tab);
    }
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
        EditorFocus::Pattern,
    ] {
        key(&mut app, KeyCode::Tab);
        assert_eq!(focus(&app), expected);
    }
    for expected in [
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
    type_in_field(&mut app, 3, "slow");
    type_in_field(&mut app, 2, "why it exists");
    type_in_field(&mut app, 1, "lines that time out");

    let screen = rendered(&mut app);
    for row in [
        "name:        slow",
        "description: why it exists",
        "prompt:      lines that time out",
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
    key(&mut app, KeyCode::BackTab);
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
    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::BackTab);
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
    type_in_field(&mut app, 3, "slow");
    type_in_field(&mut app, 2, "  why it exists ");
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
        },
    );
    select_row(&mut app, widgets::filterlist::Row::Filter(0));
    open_selected(&mut app);
    let editor_now = editor(&app);
    assert_eq!(editor_now.name.pattern, "errors");
    assert_eq!(editor_now.description.pattern, "old");
    assert_eq!(editor_now.prompt.pattern, "error lines");

    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::BackTab);
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
    type_in_field(&mut app, 3, "renamed");
    type_in_field(&mut app, 2, "why");
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
    type_in_field(&mut app, 3, "ERROR");
    key(&mut app, KeyCode::Enter);

    assert!(app.filter_editor.is_some(), "Enter closed the editor");
    assert_eq!(
        editor(&app).error.as_deref(),
        Some("another filter in this set is named \"ERROR\"")
    );
    assert_eq!(app.filters.len(), 2, "a filter was added");

    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::BackTab);
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
    type_in_field(&mut app, 3, "slow");
    type_in_field(&mut app, 2, "Instances of bug #57");
    type_in_field(&mut app, 1, "Timeouts, except in 'DEMO' runs");
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
