use super::*;

/// Move the cursor to `row` without going through `CursorMove::Jump`,
/// whose `u16` argument would silently truncate on the large-file test
/// below.
fn move_cursor_to_visible_row(app: &mut App, row: usize) {
    let lines = app.view.textarea().lines().to_vec();
    app.view.textarea_mut().set_lines(lines, (row, 0));
}

/// `H` always rebuilds the buffer — unlike a filter change, there is no
/// "visible set happens to be unchanged" case to skip it — so it is the
/// most literal instance of the rebuild this whole task is about. It
/// must hold the cursor's screen row exactly as `!` does, via the same
/// `apply_view` path.
#[test]
fn h_holds_the_cursor_on_the_same_screen_row() {
    // Lines below 100 are never matched, so `FilteredOnly` drops them
    // and keeps only 100..199 — a large, predictable rebuild — while the
    // cursor, parked inside the matched range, stays visible throughout.
    let body: String = (0..200)
        .map(|i| {
            if i < 100 {
                format!("plain {i}\n")
            } else {
                format!("match {i}\n")
            }
        })
        .collect();
    let mut app = app_over_file("h_scroll_hold", &body);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "^match ");
    key(&mut app, KeyCode::Enter);
    draw(&mut app);

    for _ in 0..120 {
        focus_file_view(&mut app);
        key(&mut app, KeyCode::Char('j'));
    }
    draw(&mut app);
    let pinned_row = cursor_screen_row(&app);
    // The pane's last text row — the one a reset viewport re-anchors to.
    let last_row = app.view.window_height() - 3;

    for _ in 0..3 {
        focus_file_view(&mut app);
        key(&mut app, KeyCode::Char('k'));
    }
    draw(&mut app);
    let before_row = cursor_screen_row(&app);
    let before_source = cursor_source(&app);
    let before_len = view_lines(&app).len();
    assert!(
        before_row < last_row,
        "test setup did not move the cursor off the pane's last row \
         (pinned_row = {pinned_row}, last_row = {last_row}, \
         before_row = {before_row}) — this test would pass whether or \
         not the fix exists"
    );

    key(&mut app, KeyCode::Char('H'));
    draw(&mut app);

    assert_ne!(
        view_lines(&app).len(),
        before_len,
        "the buffer did not change size, so `H` did not force a rebuild \
         here — this test would pass whether or not the fix exists"
    );
    assert_eq!(
        cursor_screen_row(&app),
        before_row,
        "the view re-anchored instead of holding the line in place"
    );
    assert_eq!(
        cursor_source(&app),
        before_source,
        "the cursor changed line"
    );
}

#[test]
fn h_hides_lines_that_match_no_filter() {
    let mut app = app_over_file("toggle_hide", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert_eq!(view_lines(&app).len(), 3, "nothing hidden yet");

    key(&mut app, KeyCode::Char('H'));

    assert_eq!(view_lines(&app), vec!["beta".to_string()]);
}

#[test]
fn ctrl_h_toggles_the_same_way() {
    let mut app = app_over_file("toggle_ctrl_h", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    app.handle_event(event::Event::Key(event::KeyEvent::new(
        KeyCode::Char('h'),
        KeyModifiers::CONTROL,
    )));

    assert_eq!(view_lines(&app), vec!["beta".to_string()]);
}

/// The workflow: filter, hide, scroll to a match, show everything again,
/// and land on that exact line with its context around it.
#[test]
fn the_round_trip_returns_to_the_chosen_line() {
    let body = numbered_lines(20);
    let mut app = app_over_file("round_trip", &body);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "line 1[0-9]");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('H'));
    // Visible rows are now source lines 10..=19; pick the third of them.
    move_cursor_to_visible_row(&mut app, 2);
    assert_eq!(cursor_source(&app), 12);

    key(&mut app, KeyCode::Char('H'));

    assert_eq!(cursor_source(&app), 12, "did not return to the same line");
    assert_eq!(view_lines(&app).len(), 20, "context did not come back");
}

/// `CursorMove::Jump` takes a `u16`, so a restore that went through it
/// would silently truncate a row above 65,535 and land 65,536 lines from
/// the chosen one instead of on it.
#[test]
fn the_round_trip_survives_more_than_65535_lines() {
    const TOTAL: usize = 70_000;
    const TARGET: usize = 66_000;
    let body: String = (0..TOTAL)
        .map(|i| {
            if i == TARGET {
                "MATCH\n".to_string()
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    let mut app = app_over_file("large_round_trip", &body);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "MATCH");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('H'));
    assert_eq!(
        cursor_source(&app),
        TARGET,
        "did not land on the sole match"
    );

    key(&mut app, KeyCode::Char('H'));

    assert_eq!(
        cursor_source(&app),
        TARGET,
        "did not return to the same line past the u16 boundary"
    );
    // The *document* is whole again. The view holds a window of it since
    // #7, so its length measures the pane rather than the file.
    assert_eq!(
        app.document.visible().len(),
        TOTAL,
        "context did not come back"
    );
}

/// Toggling into hidden mode from a line that is not a match snaps forward
/// to the next one, and toggling back lands on that.
#[test]
fn hiding_from_an_unmatched_line_snaps_to_the_next_match() {
    let mut app = app_over_file("snap_to_match", "alpha\nbeta\ngamma\nbeta two\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    move_cursor_to_visible_row(&mut app, 2); // gamma, unmatched
    assert_eq!(cursor_source(&app), 2);

    key(&mut app, KeyCode::Char('H'));

    assert_eq!(cursor_source(&app), 3, "did not snap to the next match");
}

/// Hiding with nothing to show must not panic or lose the cursor.
#[test]
fn hiding_with_no_matches_is_survivable() {
    let mut app = app_over_file("no_matches", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "zzz");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('H'));
    assert!(view_lines(&app).iter().all(String::is_empty) || view_lines(&app).is_empty());

    key(&mut app, KeyCode::Char('H'));
    assert_eq!(view_lines(&app).len(), 2, "did not come back");
}

/// When hiding leaves nothing visible, the buffer falls back to a single
/// blank placeholder row. Before this, an empty `line_numbers` override
/// fell back to natural 1..N numbering, so the gutter rendered "1" next
/// to that blank row — reading as "this file has one empty line" when
/// really both of its lines are just hidden.
#[test]
fn hiding_everything_does_not_show_a_phantom_line_number() {
    let mut app = app_over_file("no_matches_gutter", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "zzz");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('H'));

    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);

    // Row 0 is the file view's top border, so row 1 is its first content
    // row — where the blank placeholder for "nothing visible" is drawn.
    // (The row still ends in the pane's own right-hand border, hence
    // checking for digits rather than requiring the whole row blank.)
    let view = app.view_area;
    let content_row: String = (view.x + 1..view.right())
        .map(|x| buf[(x, 1)].symbol())
        .collect();
    assert!(
        !content_row.chars().any(|c| c.is_ascii_digit()),
        "expected no gutter number on the placeholder row, got: {content_row:?}"
    );
}

#[test]
fn the_status_line_shows_a_funnel_while_hiding() {
    let mut app = app_over_file("funnel", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert!(!status_line(&mut app).contains('▼'));

    key(&mut app, KeyCode::Char('H'));

    assert!(
        status_line(&mut app).contains('▼'),
        "no indication that lines are hidden: {}",
        status_line(&mut app)
    );
}

/// An excluding filter (`x`) removes lines in `Dimmed` mode too — that is
/// the entire point of it — so the funnel must not be gated on
/// `FilteredOnly` alone. Before this, `x` matching every line rendered a
/// blank pane with no indication anything was going on.
#[test]
fn the_status_line_shows_a_funnel_for_an_excluding_filter_while_dimmed() {
    let mut app = app_over_file("funnel_dimmed_exclude", "alpha\nnoise\ngamma\n");
    // Set directly rather than through `f x … Enter` (#120): an
    // excluding-only set selects nothing, so the chain that commit
    // triggers always ends with nothing for the explorer to step `n`
    // to, and the resulting "no matching file" report would displace
    // the funnel status this test means to check. That interaction
    // belongs to the keymap-chain tests, not this one.
    app.add_excluding_filter("noise").expect("valid pattern");
    app.refresh_view();

    assert_eq!(
        app.document.mode(),
        Mode::Dimmed,
        "sanity: the mode never changed, only the filter set"
    );
    assert!(
        status_line(&mut app).contains('▼'),
        "no funnel shown for an excluding filter while dimmed: {}",
        status_line(&mut app)
    );
}
