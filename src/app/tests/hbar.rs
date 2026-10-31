use super::*;

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
