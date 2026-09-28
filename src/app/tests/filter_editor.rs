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
    // Row 0 is the border; the file starts on row 1. `ERROR disk` is the
    // third line, and `disk` starts at its seventh column.
    let (x, y) = (1 + 6, 1 + 2);
    assert_eq!(buf[(x, y)].symbol(), "d");
    assert_eq!(buf[(x, y)].fg, colour.fg.expect("a palette colour"));
    assert!(
        buf[(x, y)].modifier.contains(Modifier::REVERSED),
        "the match is not marked"
    );
    assert!(
        !buf[(1, 1)].modifier.contains(Modifier::REVERSED),
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
