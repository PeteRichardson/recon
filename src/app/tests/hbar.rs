use super::*;
use crate::widgets::minimap;

// ---- #579: the widest visible line --------------------------------

/// Long lines to measure: the widest is 60 columns.
fn varied_body() -> String {
    format!("short\n{}\nmedium line here\n", "x".repeat(60))
}

#[test]
fn the_widest_visible_line_is_measured_when_a_file_opens() {
    let mut app = app_over_file("hbar_widest", &varied_body());
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert_eq!(app.hbar_widest(), Some(60));
}

#[test]
fn a_filter_that_hides_the_long_lines_narrows_the_measurement() {
    let mut app = app_over_file("hbar_filter", &varied_body());
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert_eq!(app.hbar_widest(), Some(60));

    // Only the short lines stay visible — `H` hides the unmatched, which
    // leaves the dimmed include filter as the visible set. The bar must
    // shrink with them, not leave a thumb stranded for a line the user
    // cannot see.
    app.add_filter("^short$|^medium").expect("a pattern");
    key(&mut app, KeyCode::Char('H'));
    draw_tall(&mut app);
    assert_eq!(app.hbar_widest(), Some(16));
}

#[test]
fn a_frame_that_changes_nothing_reuses_the_measurement() {
    let mut app = app_over_file("hbar_cache", &varied_body());
    focus_file_view(&mut app);
    draw_tall(&mut app);
    let builds = app.hbar.builds;
    assert!(builds >= 1, "the first frame measures");

    key(&mut app, KeyCode::Char('j'));
    draw_tall(&mut app);
    key(&mut app, KeyCode::Char('j'));
    draw_tall(&mut app);
    assert_eq!(
        app.hbar.builds, builds,
        "a cursor move costs one compare, not a remeasure"
    );
}

#[test]
fn wide_glyphs_cost_the_two_columns_the_renderer_gives_them() {
    let body = "ascii\n日本語のテキスト行\n";
    let mut app = app_over_file("hbar_wide", body);
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert_eq!(app.hbar_widest(), Some(18), "9 ideographs, two columns each");
}

#[test]
fn a_tab_costs_the_stop_it_reaches() {
    // `"a\tb"` renders at columns 0..5: `a`, the tab to stop 4, `b` after it.
    let body = "a\tb\nplain\n";
    let mut app = app_over_file("hbar_tab", body);
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert_eq!(app.hbar_widest(), Some(5));
}

#[test]
fn a_short_file_reports_its_longest_line() {
    // `numbered_lines(5)` is at most `"line 4"` — five letters plus digits.
    let mut app = app_over_file("hbar_short", &numbered_lines(5));
    focus_file_view(&mut app);
    draw_tall(&mut app);
    assert_eq!(app.hbar_widest(), Some(6));
}

#[test]
fn a_wide_file_draws_the_bar_and_a_fitting_one_does_not() {
    let wide = "y".repeat(400);
    let mut app = app_over_file("hbar_drawn_wide", &format!("{wide}\nshort\n"));
    focus_file_view(&mut app);
    let buf = draw_tall(&mut app);
    let track_rows = (0..TALL.height).filter(|&y| {
        (0..TALL.width).any(|x| {
            let s = buf[(x, y)].symbol();
            s == "▁" || s == "█"
        })
    });
    assert_eq!(
        track_rows.count(),
        1,
        "exactly one bar row expected across the screen"
    );

    let mut app = app_over_file("hbar_drawn_fit", &varied_body());
    focus_file_view(&mut app);
    let buf = draw_tall(&mut app);
    assert!(
        !(0..TALL.width).any(|x| (0..TALL.height).any(|y| buf[(x, y)].symbol() == "▁")),
        "60-column text fits a 120-wide screen; no bar should be drawn"
    );
}

// ---- #580 fix: the bar row and the map's columns share a block ----

/// `Block::padding` replaces rather than adds, so the bar's bottom row
/// once wiped out the minimap's right padding: the map slid over the
/// pane's border, its grey thumb block intruding toward the filters pane,
/// and the map was built a row taller than the area it drew into.
fn wide_long_app(name: &str) -> App<'static> {
    let wide = "z".repeat(400);
    let body: String = (0..500)
        .map(|i| format!("line {i} {}", if i % 50 == 0 { &wide } else { "x" }))
        .collect::<Vec<_>>()
        .join("\n");
    app_over_file(name, &body)
}

#[test]
fn the_bar_reserves_its_row_without_eating_the_maps_columns() {
    let mut app = wide_long_app("hbar_map_pad");
    focus_file_view(&mut app);
    let buf = draw_tall(&mut app);
    let (map, map_area) = app.view.drawn_minimap().expect("the map is drawn");
    let (_bar, bar_area) = app.view.drawn_hbar().expect("the bar is drawn");

    // The map kept all three of its columns, immediately right of the
    // bar's, and stops at the pane's border instead of riding over it.
    assert_eq!(map_area.width, minimap::WIDTH, "the map lost a column");
    assert_eq!(map_area.x, bar_area.right(), "the bar and map overlap");
    assert_eq!(map_area.bottom(), bar_area.y, "the map covers the bar's row");

    // The map was built for the height the text — and the map — get, so
    // the thumb is not one row short at the bottom.
    assert_eq!(map.rows(), usize::from(map_area.height));

    // And the bar's row beside the map stays blank, not a thumb rule.
    assert_ne!(buf[(map_area.x, bar_area.y)].symbol(), "┃");
}

// ---- #581: the pointer works the bar --------------------------------

/// Press the bar at `eighths`/8 of its width; returns the cell hit.
fn press_bar(app: &mut App<'_>, area: Rect, eighths: u32) -> usize {
    let cell = (u32::from(area.width) * eighths / 8) as usize;
    assert!(
        mouse_at(app, MouseEventKind::Down(MouseButton::Left), area.x + cell as u16, area.y),
        "the press on the bar was not consumed"
    );
    cell
}

#[test]
fn a_press_on_the_bar_scrolls_the_view_and_not_the_cursor() {
    let mut app = wide_long_app("hbar_press");
    focus_file_view(&mut app);
    draw_tall(&mut app);
    let (bar, area) = app.view.drawn_hbar().expect("the bar is drawn");
    let cursor_before = app.view.cursor_col();

    // A quarter of the way along the bar.
    let cell = press_bar(&mut app, area, 2);
    let scrolled = app.view.scroll_col();
    assert_eq!(scrolled, bar.col_at(cell, area.width));
    assert!(scrolled > 0, "a quarter-bar click should scroll right");
    assert_eq!(
        app.view.cursor_col(),
        cursor_before,
        "a bar click moved the cursor"
    );
    assert!(app.view.is_scroll_pinned());
}

#[test]
fn a_drag_tracks_the_pointer_and_leaves_the_cursor_put() {
    let mut app = wide_long_app("hbar_drag");
    focus_file_view(&mut app);
    draw_tall(&mut app);
    let (bar, area) = app.view.drawn_hbar().expect("the bar is drawn");
    let cursor_before = app.view.cursor_col();
    press_bar(&mut app, area, 2);

    for eighths in [3, 5, 7] {
        let cell = (u32::from(area.width) * eighths / 8) as usize;
        assert!(
            mouse_at(&mut app, MouseEventKind::Drag(MouseButton::Left), area.x + cell as u16, area.y),
            "the drag was not consumed"
        );
        let col = app.view.scroll_col();
        assert_eq!(col, bar.col_at(cell, area.width), "the drag lagged the pointer");
    }
    assert_eq!(app.view.cursor_col(), cursor_before, "a drag moved the cursor");

    // Release ends the drag and disturbs nothing: the view keeps the
    // column, the cursor stays where it was.
    let last = app.view.scroll_col();
    assert!(mouse_at(&mut app, MouseEventKind::Up(MouseButton::Left), area.x, area.y));
    assert_eq!(app.view.scroll_col(), last, "the release moved the view");
    assert_eq!(app.view.cursor_col(), cursor_before, "the release moved the cursor");

    // And a drag after the release is a fresh press, not a live drag:
    // outside the bar it belongs to the text, so it does not scroll.
    let before = app.view.scroll_col();
    mouse_at(&mut app, MouseEventKind::Drag(MouseButton::Left), area.x + 1, area.y);
    assert_eq!(app.view.scroll_col(), before, "a stale drag kept scrolling");
}

#[test]
fn a_column_key_after_a_drag_gives_the_view_back_to_the_cursor() {
    let mut app = wide_long_app("hbar_release");
    focus_file_view(&mut app);
    draw_tall(&mut app);
    let (_, area) = app.view.drawn_hbar().expect("the bar is drawn");
    press_bar(&mut app, area, 6);
    let scrolled = app.view.scroll_col();
    assert!(app.view.is_scroll_pinned());

    // `l`: the cursor walks as it always has, and the view returns to it.
    let cursor_before = app.view.cursor_col();
    key(&mut app, KeyCode::Char('l'));
    draw_tall(&mut app);
    assert_eq!(app.view.cursor_col(), cursor_before + 1, "l moved the cursor oddly");
    assert!(!app.view.is_scroll_pinned(), "l left the drag owning the column");
    assert!(app.view.scroll_col() < scrolled, "the view stayed away from the cursor");
}

#[test]
fn a_vertical_move_keeps_the_pinned_column() {
    let mut app = wide_long_app("hbar_hold");
    focus_file_view(&mut app);
    draw_tall(&mut app);
    let (_, area) = app.view.drawn_hbar().expect("the bar is drawn");
    press_bar(&mut app, area, 4);
    let scrolled = app.view.scroll_col();

    // `j` scrolls down the file; the sideways offset is nobody's business
    // until a column motion or a landing says otherwise.
    key(&mut app, KeyCode::Char('j'));
    draw_tall(&mut app);
    assert!(app.view.is_scroll_pinned(), "j dropped the pin");
    assert_eq!(app.view.scroll_col(), scrolled, "j moved the view sideways");
}

#[test]
fn a_click_on_text_releases_the_pinned_column() {
    let mut app = wide_long_app("hbar_text_release");
    focus_file_view(&mut app);
    draw_tall(&mut app);
    let (_, area) = app.view.drawn_hbar().expect("the bar is drawn");
    press_bar(&mut app, area, 4);
    assert!(app.view.is_scroll_pinned());

    // A row above the bar's is text; a click there lands the cursor, and
    // the landing owns the view.
    let row = area.y - 1;
    assert!(mouse_at(&mut app, MouseEventKind::Down(MouseButton::Left), area.x + 5, row));
    assert!(!app.view.is_scroll_pinned(), "a text click left the drag owning the column");
}
