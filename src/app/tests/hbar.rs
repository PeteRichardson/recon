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
