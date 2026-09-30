use super::*;

// ---- the set picker (#284) ---------------------------------------------

/// Rows: `a` (enabled), `b` (disabled), `c` (disabled), `definitions`.
/// In the picker, alphabetically: a, b, c, definitions.
fn set_index(app: &App, name: &str) -> usize {
    app.filters
        .sets()
        .iter()
        .position(|meta| meta.name == name)
        .expect("known set")
}

#[test]
fn big_l_opens_the_set_picker_from_every_pane() {
    let mut app = app_with_three_sets("set_picker_opens");
    for focus in [Focus::Explorer, Focus::View, Focus::Filters] {
        app.reveal_and_focus(focus);
        key(&mut app, KeyCode::Char('L'));
        assert!(app.set_picker.is_some(), "L did not open from {focus:?}");
        key(&mut app, KeyCode::Esc);
        assert!(app.set_picker.is_none());
        assert_eq!(app.focus, focus, "Esc did not return to {focus:?}");
    }
}

#[test]
fn big_l_in_a_prompt_is_typed() {
    let mut app = app_with_three_sets("set_picker_prompt");
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char('L'));
    assert!(app.set_picker.is_none());
    assert_eq!(prompt(&app).pattern, "L");
}

#[test]
fn the_open_set_picker_covers_the_panes() {
    let mut app = app_with_three_sets("set_picker_draw");
    key(&mut app, KeyCode::Char('L'));
    let screen = rendered(&mut app);
    assert!(screen.contains("Filter sets"), "{screen}");
    assert!(screen.contains("[x] definitions"), "{screen}");
    assert!(
        !screen.contains("Filters"),
        "the filter pane shows through:\n{screen}"
    );
}

/// Unlisting the enabled set `a` on Enter disables it: its line stops
/// matching and its header leaves the pane.
#[test]
fn enter_applies_an_unlisting_and_the_view_updates() {
    let mut app = app_with_three_sets("set_picker_apply");
    assert_eq!(included(&app), 3, "sanity: alpha, beta, scratch");
    let a = set_index(&app, "a");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char(' '));
    assert!(app.filters.sets()[a].listed, "staged, not applied yet");
    key(&mut app, KeyCode::Enter);
    assert!(app.set_picker.is_none());
    assert!(!app.filters.sets()[a].listed);
    assert!(!app.filters.sets()[a].enabled);
    assert_eq!(included(&app), 2, "alpha stopped matching");
    assert!(
        !widgets::filterlist::rows(&app.filters).contains(&widgets::filterlist::Row::Header(a)),
        "a still has a row"
    );
}

#[test]
fn esc_discards_every_change() {
    let mut app = app_with_three_sets("set_picker_esc");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Esc);
    assert!(app.filters.sets().iter().all(|meta| meta.listed));
    assert_eq!(included(&app), 3);
}

/// A set listed again comes back disabled, as its file describes it:
/// the flags it had when it was unlisted are not kept (#305).
#[test]
fn a_set_listed_again_comes_back_as_its_file_describes_it() {
    let mut app = app_with_three_sets("set_picker_relist");
    let a = set_index(&app, "a");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert!(app.filters.sets()[a].listed);
    assert!(!app.filters.sets()[a].enabled);
    assert!(
        app.filters.filters_in(a).all(|(_, filter)| !filter.enabled),
        "the file's state, not the state before the unlist"
    );
}

/// The picker's apply ends a peek first (#305), so the other sets'
/// flags come back to the filters they belong to.
#[test]
fn listing_during_a_peek_ends_the_peek_first() {
    let mut app = app_with_three_sets("set_picker_peek");
    assert_eq!(included(&app), 3);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(included(&app), 0, "sanity: peeking");

    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);

    assert!(app.peek.is_none(), "the peek survived a list change");
    assert_eq!(included(&app), 2, "the other two sets' flags came back");
}

#[test]
fn the_set_picker_takes_every_key() {
    let mut app = app_with_three_sets("set_picker_swallow");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('q'));
    assert!(app.set_picker.is_some(), "q closed the picker");
    assert!(
        matches!(app.state, AppState::Running),
        "q quit from inside the picker"
    );
}

/// The filter pane's cursor stays on the row it addressed, although
/// unlisting `a` above it moves that row up.
#[test]
fn the_filter_pane_cursor_follows_its_row_across_an_apply() {
    let mut app = app_with_three_sets("set_picker_cursor");
    let b = set_index(&app, "b");
    focus_filter_pane(&mut app);
    let rows = widgets::filterlist::rows(&app.filters);
    let at = rows
        .iter()
        .position(|row| *row == widgets::filterlist::Row::Header(b))
        .expect("b has a header");
    app.filters_pane.select(at);
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    let rows = widgets::filterlist::rows(&app.filters);
    let now = app.filters_pane.selected().expect("a selection");
    assert_eq!(rows[now], widgets::filterlist::Row::Header(b));
    assert!(now < at, "sanity: the row moved");
    assert_eq!(app.focus, Focus::Filters);
}

#[test]
fn the_file_view_cursor_is_where_it_was_before_big_l() {
    let mut app = app_with_three_sets("set_picker_view_cursor");
    focus_file_view(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    let before = cursor_source(&app);
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert_eq!(cursor_source(&app), before);
    assert_eq!(app.focus, Focus::View);
}

// ---- search in the set picker (#285) -----------------------------------

/// The picker's selected row. Rows: a, b, c, definitions.
fn picker_row(app: &App) -> usize {
    app.set_picker
        .as_ref()
        .expect("the picker is open")
        .selected()
}

/// The typing moves the selection from the origin, over the name and
/// the description; Esc returns to the origin and the picker stays.
#[test]
fn the_set_picker_search_moves_as_it_is_typed_and_esc_returns() {
    let mut app = app_with_three_sets("sets_search_type");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('/'));
    assert_eq!(prompt_line(&mut app), "/");
    typed(&mut app, "c");
    assert_eq!(picker_row(&app), 2, "c is the first hit at or after b");
    typed(&mut app, "$");
    assert_eq!(picker_row(&app), 2);
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    typed(&mut app, "syntax grammar");
    assert_eq!(picker_row(&app), 3, "the description was not searched");
    key(&mut app, KeyCode::Esc);
    assert!(app.prompt.is_none());
    assert!(
        app.set_picker.is_some(),
        "Esc in the prompt closed the picker"
    );
    assert_eq!(picker_row(&app), 1, "Esc did not return to the origin");
}

/// Enter keeps the row; the next Enter applies the picker, and is not
/// swallowed as a bounce.
#[test]
fn enter_keeps_the_set_picker_search_row() {
    let mut app = app_with_three_sets("sets_search_enter");
    let c = set_index(&app, "c");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "^c$");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none());
    assert_eq!(picker_row(&app), 2);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert!(
        app.set_picker.is_none(),
        "the Enter after the search was lost"
    );
    assert!(!app.filters.sets()[c].listed);
}

#[test]
fn n_and_big_n_step_through_the_set_picker_hits() {
    let mut app = app_with_three_sets("sets_search_n");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "^[bd]");
    key(&mut app, KeyCode::Enter);
    assert_eq!(picker_row(&app), 1);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(picker_row(&app), 3);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(picker_row(&app), 1, "n did not wrap");
    key(&mut app, KeyCode::Char('N'));
    assert_eq!(picker_row(&app), 3, "N did not wrap");
}

#[test]
fn a_set_picker_pattern_with_no_hit_is_reported() {
    let mut app = app_with_three_sets("sets_search_none");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zzz");
    key(&mut app, KeyCode::Enter);
    assert_eq!(status(&app), Some("no sets match \"zzz\""));
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(status(&app), Some("no more matches"));
    assert_eq!(picker_row(&app), 0);
}

#[test]
fn an_invalid_set_picker_pattern_is_reported_like_the_others() {
    let mut app = app_with_three_sets("sets_search_invalid");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "c(");
    assert_eq!(picker_row(&app), 1, "a half-typed regex moved");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_some(), "prompt closed on an invalid pattern");
    assert_eq!(prompt_line(&mut app), INVALID_PATTERN);
}

/// The picker's history is its own: a pattern committed there is not
/// offered in the file search or the filename search, nor theirs there.
#[test]
fn the_set_picker_keeps_its_own_history() {
    let mut app = app_with_three_sets("sets_search_history");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt(&app).pattern,
        "",
        "the file search's history leaked in"
    );
    typed(&mut app, "^b$");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    assert!(app.set_picker.is_none());

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt(&app).pattern, "beta");
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt(&app).pattern,
        "beta",
        "the picker's history leaked out"
    );
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt(&app).pattern, "^b$");
    assert_eq!(picker_row(&app), 1, "a recall did not move");
}
