use super::*;

// ---- #58: mouse clicks ---------------------------------------------

/// Left-click `line` rows below `pane`'s top border, one column in.
fn click_pane(app: &mut App, pane: Focus, line: u16) {
    let area = app.pane_area(pane);
    mouse_at(
        app,
        MouseEventKind::Down(MouseButton::Left),
        area.x + 1,
        area.y + 1 + line,
    );
}

#[test]
fn clicking_a_file_in_the_explorer_loads_it_and_focuses_the_pane() {
    let mut app = app_over_files("click_explorer_file", &[("a.log", "a\n"), ("b.log", "b\n")]);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::View);
    draw(&mut app);

    // Rows: `..`, `a.log`, `b.log`.
    click_pane(&mut app, Focus::Explorer, 2);

    assert_eq!(shown(&app), "b.log");
    assert_eq!(app.explorer.selected(), Some(2));
    assert_eq!(app.focus, Focus::Explorer);
}

#[test]
fn a_single_click_on_a_directory_previews_it_and_a_double_click_descends() {
    let (mut app, dir) = app_over_nested("click_explorer_dir");
    draw(&mut app);

    click_pane(&mut app, Focus::Explorer, 1);
    assert!(app.view.showing_directory(), "one click looks ahead");
    assert!(app.explorer.dir().ends_with(&dir), "one click stays put");

    click_pane(&mut app, Focus::Explorer, 1);
    assert!(
        app.explorer.dir().ends_with(dir.join("sub")),
        "two clicks descend, got {}",
        app.explorer.dir().display()
    );
}

#[test]
fn a_double_click_on_the_parent_entry_climbs_out() {
    let (mut app, dir) = app_over_nested("click_explorer_parent");
    draw(&mut app);

    click_pane(&mut app, Focus::Explorer, 0);
    click_pane(&mut app, Focus::Explorer, 0);

    assert!(
        app.explorer
            .dir()
            .ends_with(dir.parent().expect("fixture parent"))
    );
    assert!(!app.explorer.dir().ends_with(&dir));
}

#[test]
fn two_clicks_on_different_rows_are_not_a_double_click() {
    let (mut app, dir) = app_over_nested("click_explorer_two_rows");
    draw(&mut app);

    click_pane(&mut app, Focus::Explorer, 1);
    click_pane(&mut app, Focus::Explorer, 2);

    assert!(app.explorer.dir().ends_with(&dir), "no descent");
    assert_eq!(shown(&app), "z.log");
}

#[test]
fn a_click_lands_on_the_row_drawn_there_once_the_list_has_scrolled() {
    let names: Vec<String> = (0..30).map(|i| format!("f{i:02}")).collect();
    let files: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut app = app_over("click_explorer_scrolled", &files);
    key(&mut app, KeyCode::Char('G'));
    draw(&mut app);
    // 31 rows including `..`, scrolled so the last is on the bottom
    // inner row; the first inner row shows the entry `offset` rows in.
    let inner = usize::from(app.explorer_area.height - 2);
    let offset = 31 - inner;

    click_pane(&mut app, Focus::Explorer, 0);

    assert_eq!(app.explorer.selected(), Some(offset));
    assert_eq!(shown(&app), format!("f{:02}", offset - 1));
}

#[test]
fn a_click_on_a_pane_border_only_moves_focus() {
    let mut app = app_over_files(
        "click_explorer_border",
        &[("a.log", "a\n"), ("b.log", "b\n")],
    );
    key(&mut app, KeyCode::Tab);
    draw(&mut app);
    let before = app.explorer.selected();
    let showing = shown(&app);

    // The explorer's top border, well away from either divider.
    let explorer_area = app.explorer_area;
    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        explorer_area.x + 1,
        explorer_area.y,
    );

    assert_eq!(app.focus, Focus::Explorer);
    assert_eq!(app.explorer.selected(), before);
    assert_eq!(shown(&app), showing, "nothing was opened");
}

#[test]
fn a_click_below_the_last_entry_does_nothing() {
    let mut app = app_over_files("click_explorer_blank", &[("a.log", "a\n")]);
    draw(&mut app);
    let before = app.explorer.selected();
    let showing = shown(&app);

    // Rows: `..`, `a.log`; the third inner row is blank.
    click_pane(&mut app, Focus::Explorer, 2);

    assert_eq!(app.explorer.selected(), before);
    assert_eq!(shown(&app), showing, "nothing was opened");
}

#[test]
fn clicking_a_filter_row_toggles_it_and_focuses_the_pane() {
    let mut app = app_over_files("click_filter_row", &[("a.log", "x\ny\n")]);
    open_file(&mut app, 0);
    app.add_filter("x").expect("valid pattern");
    draw(&mut app);
    let rows = widgets::filterlist::rows(&app.filters);
    let (line, index) = rows
        .iter()
        .enumerate()
        .find_map(|(line, row)| match row {
            widgets::filterlist::Row::Filter(index) => Some((line, *index)),
            _ => None,
        })
        .expect("the filter has a row");
    assert!(app.filters.filters()[index].enabled);

    click_pane(&mut app, Focus::Filters, u16::try_from(line).unwrap());
    assert!(
        !app.filters.filters()[index].enabled,
        "one click switches it off"
    );
    assert_eq!(app.focus, Focus::Filters);

    click_pane(&mut app, Focus::Filters, u16::try_from(line).unwrap());
    assert!(
        app.filters.filters()[index].enabled,
        "another switches it back on"
    );
}

#[test]
fn clicking_a_set_header_toggles_the_set() {
    let mut app = app_over_files("click_filter_header", &[("a.log", "x\n")]);
    draw(&mut app);
    let rows = widgets::filterlist::rows(&app.filters);
    let (line, set) = rows
        .iter()
        .enumerate()
        .find_map(|(line, row)| match row {
            widgets::filterlist::Row::Header(set) => Some((line, *set)),
            _ => None,
        })
        .expect("the built-in set draws a header");
    let before = app.filters.sets()[set].enabled;

    click_pane(&mut app, Focus::Filters, u16::try_from(line).unwrap());

    assert_eq!(app.filters.sets()[set].enabled, !before);
}

#[test]
fn clicking_the_status_row_opens_the_include_prompt_like_f_i() {
    let mut app = app_over_files("click_status", &[("a.log", "x\ny\n")]);
    open_file(&mut app, 0);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::View);
    draw(&mut app);

    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        AREA.width / 2,
        AREA.height - 1,
    );

    assert!(
        matches!(&app.prompt, Some(prompt) if prompt.kind == PromptKind::Filter),
        "an include prompt is open"
    );
    assert_eq!(app.focus, Focus::Filters);
    assert_eq!(app.chain_origin, Some(Focus::View));

    // Committing behaves exactly as after `f i`: the filter lands and
    // focus goes back where the click came from.
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.filters.filters_in(0).count(), 1, "one scratch filter");
    assert_eq!(app.focus, Focus::View);
}

#[test]
fn clicking_a_row_of_the_listing_in_the_view_opens_that_entry() {
    let (mut app, dir) = app_over_nested("click_view_listing");
    draw(&mut app);
    click_pane(&mut app, Focus::Explorer, 1);
    assert!(app.view.showing_directory());
    draw(&mut app);

    // The look-ahead lists `inner_a.log`, `inner_b.log`; no `..`.
    click_pane(&mut app, Focus::View, 1);

    assert!(app.explorer.dir().ends_with(dir.join("sub")));
    assert_eq!(shown(&app), "inner_b.log");
    assert!(
        app.explorer
            .selected_path()
            .is_some_and(|p| p.ends_with("inner_b.log"))
    );
    assert_eq!(app.focus, Focus::View);
}

#[test]
fn clicking_a_listed_subdirectory_in_the_view_descends_into_it() {
    let dir = fixture_dir("click_view_subdir");
    fs::create_dir_all(dir.join("outer/deeper")).expect("create dirs");
    fs::write(dir.join("outer/deeper/leaf.log"), "l\n").expect("write");
    let mut app = App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    });
    draw(&mut app);
    click_pane(&mut app, Focus::Explorer, 1); // `outer/`, previewed
    draw(&mut app);

    click_pane(&mut app, Focus::View, 0); // `deeper/` in the look-ahead

    assert!(
        app.explorer.dir().ends_with(dir.join("outer/deeper")),
        "got {}",
        app.explorer.dir().display()
    );
    assert!(app.view.showing_directory() || shown(&app) == "leaf.log");
}

#[test]
fn clicking_the_view_while_it_shows_a_file_only_focuses_it() {
    let mut app = app_over_files("click_view_file", &[("a.log", "a\nb\nc\n")]);
    open_file(&mut app, 0);
    assert_eq!(app.focus, Focus::Explorer);
    draw(&mut app);
    let dir = app.explorer.dir().to_path_buf();

    click_pane(&mut app, Focus::View, 1);

    assert_eq!(app.focus, Focus::View);
    assert_eq!(shown(&app), "a.log");
    assert_eq!(app.explorer.dir(), dir);
}

#[test]
fn clicks_are_ignored_while_a_prompt_is_open() {
    let mut app = app_over_files("click_during_prompt", &[("a.log", "a\n"), ("b.log", "b\n")]);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    draw(&mut app);
    let showing = shown(&app);

    click_pane(&mut app, Focus::Explorer, 2);

    assert!(app.prompt.is_some(), "the prompt is still open");
    assert_eq!(shown(&app), showing, "nothing was opened");
}

#[test]
fn a_click_inside_the_zoomed_pane_keeps_the_zoom() {
    let mut app = app_over_files("click_zoomed", &[("a.log", "a\n"), ("b.log", "b\n")]);
    app.zoom_focused();
    assert_eq!(app.panes.shown(), PaneSet::only(Focus::Explorer));
    draw(&mut app);

    // Full-frame pane: the third inner row is `b.log`.
    mouse_at(&mut app, MouseEventKind::Down(MouseButton::Left), 1, 3);

    assert_eq!(shown(&app), "b.log");
    assert_eq!(app.panes.shown(), PaneSet::only(Focus::Explorer));
}
