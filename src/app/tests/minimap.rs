use super::*;
use crate::app::minimap::MINIMAP_MIN_WIDTH;

// ---- #523: the minimap ----------------------------------------------

/// `TALL` has 33 rows of text: 1,000 lines put about 30 on each map row.
const LINES: usize = 1000;

fn long_app(name: &str) -> App<'static> {
    let mut app = app_over_file(name, &numbered_lines(LINES));
    focus_file_view(&mut app);
    app
}

/// Where the map was drawn in the last frame, if it was.
fn map_area(app: &App) -> Option<Rect> {
    app.view.drawn_minimap().map(|(_, area)| area)
}

/// Press on map row `row`, as `kind`.
fn on_map(app: &mut App, kind: MouseEventKind, row: u16) {
    let area = map_area(app).expect("a map to click");
    mouse_at(app, kind, area.x + 1, area.y + row);
}

#[test]
fn a_long_file_draws_the_map_inside_the_border_at_the_right() {
    let mut app = long_app("minimap_long");
    app.add_filter("^line 9[0-9][0-9]$").expect("a pattern");
    let buf = draw_tall(&mut app);

    let area = map_area(&app).expect("a map");
    let inner = app.view_area.inner(Margin::new(1, 1));
    assert_eq!(area.right(), inner.right(), "the map ends at the border");
    assert_eq!(area.height, inner.height);
    assert_eq!(buf[(area.x, area.y + 10)].symbol(), "│", "the rule");
    // Lines 900 to 999 are the last rows of the map, and only they have bars.
    let last = area.bottom() - 1;
    assert_eq!(buf[(area.x + 2, last)].symbol(), "█");
    assert_eq!(buf[(area.x + 2, area.y + 10)].symbol(), " ");
}

#[test]
fn the_text_leaves_the_map_its_columns() {
    let body = format!("{}\n", "x".repeat(300)).repeat(LINES);
    let mut app = app_over_file("minimap_text_width", &body);
    focus_file_view(&mut app);
    let buf = draw_tall(&mut app);
    let area = map_area(&app).expect("a map");
    for row in area.y..area.bottom() {
        assert_ne!(buf[(area.x + 1, row)].symbol(), "x", "row {row}");
        assert_eq!(
            buf[(area.x - 1, row)].symbol(),
            "x",
            "the text reaches the map"
        );
    }
}

#[test]
fn a_file_the_pane_shows_whole_has_no_map() {
    let mut app = app_over_file("minimap_short", &numbered_lines(5));
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert_eq!(map_area(&app), None);
}

#[test]
fn m_hides_the_map_and_shows_it_again() {
    let mut app = long_app("minimap_toggle");
    draw_tall(&mut app);
    assert!(map_area(&app).is_some());

    key(&mut app, KeyCode::Char('M'));
    draw_tall(&mut app);
    assert_eq!(map_area(&app), None, "hidden");

    key(&mut app, KeyCode::Char('M'));
    draw_tall(&mut app);
    assert!(map_area(&app).is_some(), "shown again");
}

#[test]
fn the_config_can_turn_the_map_off() {
    let file = fixture_path("minimap_config_off", &numbered_lines(LINES));
    let mut app = App::new(&Startup::from(Config {
        path: Some(file.display().to_string()),
        minimap: Some(false),
        ..Config::default()
    }));
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert_eq!(map_area(&app), None);
}

#[test]
fn a_hex_dump_has_no_map() {
    let file = fixture_file("minimap_hex.bin", &[0_u8; 4096]);
    let mut app = App::new(&Startup::from(Config {
        path: Some(file.display().to_string()),
        ..Config::default()
    }));
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert!(app.view.is_hex());
    assert_eq!(map_area(&app), None);
}

#[test]
fn a_narrow_pane_has_no_map() {
    let mut app = long_app("minimap_narrow");
    let narrow = Rect { width: 40, ..TALL };
    let mut buf = Buffer::empty(narrow);
    app.render(narrow, &mut buf);
    assert!(
        app.view_area.width - 2 < MINIMAP_MIN_WIDTH,
        "the fixture is narrow"
    );
    assert_eq!(map_area(&app), None);
}

#[test]
fn a_click_on_the_map_jumps_to_that_part_of_the_file() {
    let mut app = long_app("minimap_click");
    draw_tall(&mut app);

    // 1,000 lines on 33 rows: row 16 starts at line ⌈16,000 / 33⌉ = 485.
    on_map(&mut app, MouseEventKind::Down(MouseButton::Left), 16);
    assert_eq!(cursor_source(&app), 485);
}

#[test]
fn a_drag_that_starts_on_the_map_follows_the_pointer() {
    let mut app = long_app("minimap_drag");
    draw_tall(&mut app);

    on_map(&mut app, MouseEventKind::Down(MouseButton::Left), 0);
    draw_tall(&mut app);
    on_map(&mut app, MouseEventKind::Drag(MouseButton::Left), 30);
    // Row 30 starts at line ⌈30,000 / 33⌉ = 910.
    assert_eq!(cursor_source(&app), 910);
    assert!(app.visual.is_none(), "a drag on the map selects nothing");
    on_map(&mut app, MouseEventKind::Up(MouseButton::Left), 30);
}

#[test]
fn a_click_lands_on_the_nearest_visible_line_in_hide_mode() {
    let mut app = long_app("minimap_click_hidden");
    app.add_filter("^line [0-9]*0$").expect("a pattern");
    key(&mut app, KeyCode::Char('u'));
    draw_tall(&mut app);

    // Row 16 starts at line 485, which is hidden; the next shown is 490.
    on_map(&mut app, MouseEventKind::Down(MouseButton::Left), 16);
    assert_eq!(cursor_source(&app), 490);
}

#[test]
fn moving_and_redrawing_do_not_build_the_map_again() {
    let mut app = long_app("minimap_cache");
    app.add_filter("^line 5").expect("a pattern");
    draw_tall(&mut app);
    let built = app.minimap.builds;

    for _ in 0..20 {
        key(&mut app, KeyCode::Char('j'));
        draw_tall(&mut app);
    }
    ctrl(&mut app, KeyCode::Char('f'));
    draw_tall(&mut app);
    assert_eq!(
        app.minimap.builds, built,
        "the cursor and the scroll build nothing"
    );

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "line 7");
    key(&mut app, KeyCode::Enter);
    draw_tall(&mut app);
    assert!(
        app.minimap.builds > built,
        "a new search builds the map again"
    );
}

#[test]
fn a_new_search_marks_its_hits() {
    let mut app = long_app("minimap_search");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "^line 99[0-9]$");
    key(&mut app, KeyCode::Enter);
    let buf = draw_tall(&mut app);

    let area = map_area(&app).expect("a map");
    let last = area.bottom() - 1;
    assert_ne!(
        buf[(area.x + 1, last)].symbol(),
        " ",
        "dots on the last row"
    );
    assert_eq!(buf[(area.x + 1, area.y)].symbol(), " ", "none at the top");
}
