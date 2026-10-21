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

// ---- #481: the file view follows a selection that hide mode moves --------

/// An app over `files` (each with a body), with an including filter and
/// the scan answers `matches` gives, row by row, and the explorer focused.
fn app_with_answers(
    name: &str,
    files: &[(&str, &str)],
    matches: &[bool],
) -> (App<'static>, Sender<scan::Scanned>) {
    let mut app = app_over_files(name, files);
    let (_scanner, tx) = record_scans(&mut app);
    key(&mut app, KeyCode::Char('e'));
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_view();
    app.refresh_scan(false);
    for (row, &yes) in matches.iter().enumerate() {
        mark(&mut app, &tx, row, yes);
    }
    (app, tx)
}

#[test]
fn u_moves_the_view_with_a_selection_it_hides() {
    let (mut app, _tx) = app_with_answers(
        "hide_follow_view",
        &[("a.log", "beta\n"), ("b.log", "alpha\n")],
        &[false, true],
    );
    arrow_to(&mut app, "a.log");
    assert_eq!(shown(&app), "a.log", "sanity");

    key(&mut app, KeyCode::Char('u'));

    assert_eq!(selected_name(&app), "b.log");
    assert_eq!(shown(&app), "b.log", "the view kept the hidden file");
}

/// With no file left that matches, the selection can only land on `..` or
/// a directory. Their listings are not filtered, so the view shows nothing
/// rather than a page of text that does not answer the question.
#[test]
fn u_with_no_file_left_blanks_the_view_and_says_why() {
    let (mut app, _tx) = app_with_answers(
        "hide_none_left",
        &[("a.log", "beta\n"), ("sub/inner.log", "beta\n")],
        &[false],
    );
    arrow_to(&mut app, "a.log");

    key(&mut app, KeyCode::Char('u'));

    assert_ne!(selected_name(&app), "a.log", "sanity: hide mode moved it");
    assert_ne!(shown(&app), "a.log", "the view kept the hidden file");
    // What the view holds, not what it draws: hide mode would draw a
    // listing as blank too, since none of its lines match.
    assert!(
        app.view.source().is_empty(),
        "the view holds a listing: {:?}",
        app.view.source()
    );
    assert_eq!(status(&app), Some(NO_FILE_MATCHES));
}

/// The blank is only for a move hide mode made. A move the user makes onto
/// a directory shows its listing, as it always did.
#[test]
fn after_the_blank_a_move_to_a_directory_shows_its_listing() {
    let (mut app, _tx) = app_with_answers(
        "hide_blank_then_k",
        &[("a.log", "beta\n"), ("sub/inner.log", "beta\n")],
        &[false],
    );
    arrow_to(&mut app, "a.log");
    key(&mut app, KeyCode::Char('u'));
    assert!(app.view.source().is_empty(), "sanity: blank");

    key(&mut app, KeyCode::Char('k'));

    assert!(
        !app.view.source().is_empty(),
        "no listing for {}",
        selected_name(&app)
    );
}

/// Back to dim mode, the selection stays where hide mode put it, and so
/// does the view: the file that was hidden is listed again, but nothing
/// moves back to it.
#[test]
fn u_again_stays_on_the_file_hide_mode_moved_to() {
    let (mut app, _tx) = app_with_answers(
        "hide_follow_back",
        &[("a.log", "beta\n"), ("b.log", "alpha\n")],
        &[false, true],
    );
    arrow_to(&mut app, "a.log");
    key(&mut app, KeyCode::Char('u'));

    key(&mut app, KeyCode::Char('u'));

    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity");
    assert_eq!(selected_name(&app), "b.log");
    assert_eq!(shown(&app), "b.log");
}

/// Hide mode can remove the selected file without a key: the scan answers
/// it `No` while hide mode is on. The view follows the same way.
#[test]
fn a_scan_answer_that_hides_the_selection_moves_the_view_too() {
    let (mut app, tx) = app_with_answers(
        "hide_follow_scan",
        &[("a.log", "beta\n"), ("b.log", "alpha\n")],
        &[],
    );
    mark(&mut app, &tx, 1, true);
    arrow_to(&mut app, "a.log");
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(shown(&app), "a.log", "sanity: not answered yet, so listed");

    mark(&mut app, &tx, 0, false);

    assert_eq!(selected_name(&app), "b.log");
    assert_eq!(shown(&app), "b.log", "the view kept the hidden file");
}

/// Ending a peek puts hide mode back, and with it the rows the peek showed.
/// A file selected during the peek can be one of them.
#[test]
fn ending_a_peek_that_hides_the_selection_moves_the_view_too() {
    let (mut app, _tx) = app_with_answers(
        "hide_follow_peek",
        &[("a.log", "beta\n"), ("b.log", "alpha\n")],
        &[false, true],
    );
    arrow_to(&mut app, "b.log");
    key(&mut app, KeyCode::Char('u'));
    key(&mut app, KeyCode::Char(' '));
    arrow_to(&mut app, "a.log");
    assert_eq!(shown(&app), "a.log", "sanity: the peek lists it");

    key(&mut app, KeyCode::Char(' '));

    assert_eq!(selected_name(&app), "b.log");
    assert_eq!(shown(&app), "b.log", "the view kept the hidden file");
}

/// The nearest row can be a file not scanned yet. Hide mode passes it for
/// the nearest file that matches, so the view has a file to show.
#[test]
fn u_passes_an_unscanned_file_for_one_that_matches() {
    let (mut app, tx) = app_with_answers(
        "hide_follow_unscanned",
        &[
            ("a.log", "alpha\n"),
            ("b.log", "beta\n"),
            ("c.log", "beta\n"),
        ],
        &[true],
    );
    mark(&mut app, &tx, 2, false);
    arrow_to(&mut app, "c.log");

    key(&mut app, KeyCode::Char('u'));

    assert_eq!(selected_name(&app), "a.log");
    assert_eq!(shown(&app), "a.log");
}

// ---- #488: a peek leaves the explorer selection where it was -------------

/// `space` `space`, with `check` after each: a whole peek, as the user
/// makes it.
fn peek_and_back(app: &mut App, check: impl Fn(&App)) {
    for _ in 0..2 {
        key(app, KeyCode::Char(' '));
        check(app);
    }
}

/// From dim mode a peek turns hide mode on, with every filter off. The
/// file with no matches must not be hidden for the moment before the scan
/// says that nothing selects.
#[test]
fn a_peek_from_dim_mode_keeps_the_selection_on_a_file_with_no_matches() {
    let (mut app, _tx) = app_with_answers(
        "peek_dim_keeps",
        &[("a.log", "beta\n"), ("b.log", "alpha\n")],
        &[false, true],
    );
    arrow_to(&mut app, "a.log");

    peek_and_back(&mut app, |app| {
        assert_eq!(selected_name(app), "a.log", "the peek moved the selection");
        assert_eq!(shown(app), "a.log", "the peek moved the view");
    });
    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity");
}

/// From hide mode the selection is on a file that matches, and a peek
/// leaves it there too.
#[test]
fn a_peek_from_hide_mode_keeps_the_selection() {
    let (mut app, _tx) = app_with_answers(
        "peek_hide_keeps",
        &[
            ("a.log", "beta\n"),
            ("b.log", "alpha\n"),
            ("c.log", "alpha\n"),
        ],
        &[false, true, true],
    );
    arrow_to(&mut app, "c.log");
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(selected_name(&app), "c.log", "sanity");

    peek_and_back(&mut app, |app| {
        assert_eq!(selected_name(app), "c.log", "the peek moved the selection");
        assert_eq!(shown(app), "c.log", "the peek moved the view");
    });
    assert_eq!(app.document.mode(), Mode::FilteredOnly, "sanity");
}
