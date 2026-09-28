use super::*;

// ---- AND mode (#39) -----------------------------------------------------

/// `&` flips the set to AND, the view re-evaluates under the new rule,
/// and the status row says so — the mode is easy to forget while moving
/// fast, which is the same reason `HIDE` has a badge.
#[test]
fn ampersand_ands_the_filters_and_shows_a_badge() {
    let mut app = app_over_file("and_mode", "foo\nbar\nfoo bar\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "foo");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "bar");
    key(&mut app, KeyCode::Enter);
    assert!(
        !status_line(&mut app).contains(AND_BADGE_TEXT.trim()),
        "sanity: no badge before &"
    );
    let included = |app: &App| {
        app.document
            .verdicts()
            .iter()
            .filter(|v| matches!(v, filter::Verdict::Included(_)))
            .count()
    };
    assert_eq!(included(&app), 3, "sanity: OR includes every line");

    key(&mut app, KeyCode::Char('&'));

    assert!(app.filters.is_and());
    assert_eq!(included(&app), 1, "only `foo bar` matches both");
    assert!(
        status_line(&mut app).contains(AND_BADGE_TEXT.trim()),
        "AND mode on with nothing on the row saying so: {}",
        status_line(&mut app)
    );

    key(&mut app, KeyCode::Char('&'));

    assert!(!app.filters.is_and());
    assert_eq!(included(&app), 3);
    assert!(!status_line(&mut app).contains(AND_BADGE_TEXT.trim()));
}

/// The badge is painted like `HIDE`, so a reader of one recognises the other.
#[test]
fn the_and_badge_wears_the_badge_style() {
    let mut app = app_over_file("and_badge_style", "alpha\n");
    key(&mut app, KeyCode::Char('&'));
    let width = 60;
    let area = Rect::new(0, 0, width, 6);
    let mut buf = Buffer::empty(area);
    app.render(area, &mut buf);
    let y = area.height - 1;
    let row: String = (0..width).map(|x| buf[(x, y)].symbol()).collect();
    let start = u16::try_from(row.find(AND_BADGE_TEXT).expect("badge on the row")).unwrap();
    for x in start..start + u16::try_from(AND_BADGE_TEXT.chars().count()).unwrap() {
        let style = buf[(x, y)].style();
        assert_eq!(style.fg, HIDE_BADGE_STYLE.fg, "column {x} foreground");
        assert_eq!(style.bg, HIDE_BADGE_STYLE.bg, "column {x} background");
    }
}

/// An open prompt takes every key, so `&` inside a pattern is typed, not
/// acted on — `foo&bar` is a legitimate thing to search for.
#[test]
fn ampersand_in_a_prompt_is_typed() {
    let mut app = app_over_file("and_in_prompt", "alpha\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "a&b");
    assert!(!app.filters.is_and());
    assert_eq!(app.prompt.as_ref().map(|p| p.pattern.as_str()), Some("a&b"));
}

/// Issue #36's remaining half. `▼` answers "are lines missing right now?",
/// which is deliberately false here — with nothing including, the #36 guard
/// in `Document::recompute_visible` shows the whole file. But hide mode
/// *is* armed: define a filter and the pane visibly starts hiding, with no
/// second keypress. This is the state the issue was reported against, and
/// the one the funnel can never cover without lying.
#[test]
fn the_badge_shows_hide_mode_with_no_filters_at_all() {
    let mut app = app_over_file("badge_bare", "alpha\nbeta\n");
    assert!(
        !status_line(&mut app).contains(HIDE_BADGE_TEXT.trim()),
        "sanity: no badge before Ctrl-H"
    );

    key(&mut app, KeyCode::Char('H'));

    assert_eq!(
        app.document.mode(),
        Mode::FilteredOnly,
        "sanity: hide mode is armed"
    );
    assert!(
        status_line(&mut app).contains(HIDE_BADGE_TEXT.trim()),
        "hide mode armed with nothing on the row saying so: {}",
        status_line(&mut app)
    );
}

#[test]
fn the_badge_shows_hide_mode_alongside_enabled_filters() {
    let mut app = app_over_file("badge_enabled", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('H'));

    let row = status_line(&mut app);
    assert!(row.contains(HIDE_BADGE_TEXT.trim()), "no badge: {row}");
    assert!(
        row.contains('▼'),
        "sanity: the funnel still fires too: {row}"
    );
}

/// The badge tracks the mode, not the filter set, so `!` must not take it
/// away — the mode is still armed and re-enabling the filters resumes
/// hiding immediately. This is also where `▼` and the badge visibly
/// disagree, which is the whole reason they are two indicators.
#[test]
fn the_badge_survives_disabling_every_filter() {
    let mut app = app_over_file("badge_disabled", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('!'));
    key(&mut app, KeyCode::Char('H'));

    let row = status_line(&mut app);
    assert!(
        row.contains(HIDE_BADGE_TEXT.trim()),
        "badge went away with the filters, but the mode did not: {row}"
    );
    assert!(
        !row.contains('▼'),
        "sanity: the funnel stays honest and off here: {row}"
    );
}

#[test]
fn no_badge_while_dimming() {
    let mut app = app_over_file("badge_dimmed", "alpha\nbeta\n");
    // Set directly rather than through `f x … Enter` (#120): an
    // excluding-only set selects nothing, so the chain that commit
    // triggers always ends with nothing for the explorer to step `n`
    // to, and the resulting "no matching file" report would displace
    // the status this test means to check. That interaction belongs to
    // the keymap-chain tests, not this one.
    app.add_excluding_filter("beta").expect("valid pattern");
    app.refresh_view();

    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: still dimming");
    let row = status_line(&mut app);
    assert!(
        !row.contains(HIDE_BADGE_TEXT.trim()),
        "badge claimed hide mode while dimming: {row}"
    );
    assert!(row.contains('▼'), "sanity: the funnel is on: {row}");
}

/// The issue rejected "a dim tiny icon" by name. Reverse video is what
/// makes this louder than the `DarkGray` row it sits on, so the styling is
/// the feature, not decoration — assert on it rather than on the text.
#[test]
fn the_badge_is_painted_prominently_not_dimmed() {
    let mut app = app_over_file("badge_style", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('H'));

    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let y = AREA.height - 1;

    // Every column of the badge, not just one: a colour block with a gap
    // in it is not the thing the issue asked for. Compared field by field
    // rather than as a whole `Style` because a painted cell also carries a
    // default `underline_color` that the constant never mentions.
    for x in 0..HIDE_BADGE_TEXT.chars().count() as u16 {
        let style = buf[(x, y)].style();
        assert_eq!(style.fg, HIDE_BADGE_STYLE.fg, "column {x} foreground");
        assert_eq!(style.bg, HIDE_BADGE_STYLE.bg, "column {x} background");
        assert_eq!(
            style.add_modifier, HIDE_BADGE_STYLE.add_modifier,
            "column {x} modifiers"
        );
        assert!(
            !style.add_modifier.contains(Modifier::DIM),
            "the badge is the dim glyph the issue rejected"
        );
        assert!(
            style.bg.is_some() && style.fg.is_some(),
            "reverse video needs both a foreground and a background"
        );
    }
}

/// The badge is drawn ahead of the status text and takes columns from its
/// budget, so a narrow row has to keep eliding the path from the left
/// rather than letting anything run past the edge.
#[test]
fn a_narrow_row_keeps_the_badge_and_still_elides_the_directory() {
    let mut app = app_over_file("badge_narrow", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('H'));

    let width = 40;
    let row = status_line_at(&mut app, width);

    assert!(row.contains(HIDE_BADGE_TEXT.trim()), "badge dropped: {row}");
    assert!(
        row.chars().count() <= width as usize,
        "the row overflowed {width} columns: {row}"
    );
    assert!(
        row.contains('…'),
        "the directory stopped eliding from the left: {row}"
    );
}

/// An open prompt shares the row rather than displacing the badge. Hide
/// mode is armed while a filter is being typed, and typing one is exactly
/// when the pane is about to change under you.
#[test]
fn an_open_prompt_still_shows_the_badge() {
    let mut app = app_over_file("badge_prompt", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('H'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "foo");

    let row = status_line(&mut app);
    assert!(row.contains(HIDE_BADGE_TEXT.trim()), "badge dropped: {row}");
    assert!(row.contains("filter: foo"), "prompt lost: {row}");
}

/// An excluding-only filter set never produces an `Included` verdict, so
/// the old `match_count`-based report always read "0 matched" even while
/// visibly removing lines — indistinguishable from the filter matching
/// nothing at all. The status line must describe what is on screen.
#[test]
fn the_status_line_reports_lines_shown_not_matched() {
    let mut app = app_over_file("status_shown_not_matched", "alpha\nnoise\ngamma\n");
    // Set directly rather than through `f x … Enter` (#120): an
    // excluding-only set selects nothing, so the chain that commit
    // triggers always ends with nothing for the explorer to step `n`
    // to, and the resulting "no matching file" report would displace
    // the line-count status this test means to check. That interaction
    // belongs to the keymap-chain tests, not this one.
    app.add_excluding_filter("noise").expect("valid pattern");
    app.refresh_view();

    let status = status_line(&mut app);

    assert!(
        status.contains("2/3"),
        "expected the two lines actually shown, got: {status}"
    );
    assert!(
        !status.contains("0/3"),
        "reported the (always-zero) match count instead of lines shown: {status}"
    );
}

/// Issue #36's guard made this state impossible: with nothing including
/// (no numbered filters and no search), hiding shows the whole file
/// rather than blanking it, so the status line must not claim the two
/// are in tension.
#[test]
fn hiding_with_no_filters_does_not_claim_the_file_is_empty() {
    let mut app = app_over_file("status_no_filters", "alpha\nbeta\n");

    key(&mut app, KeyCode::Char('H'));

    let status = status_line(&mut app);
    assert!(
        !status.to_lowercase().contains("nothing to show"),
        "the status line still reports a blank pane that no longer happens: {status}"
    );
}

/// `b` gives the file its full width by hiding the left column.
#[test]
fn b_hides_the_left_column() {
    let mut app = app_over_file("zoom_b", "alpha\n");
    assert!(rendered(&mut app).contains("alpha"));
    let before = rendered(&mut app);

    key(&mut app, KeyCode::Char('b'));

    let after = rendered(&mut app);
    assert_ne!(before, after, "the layout did not change");
    assert!(after.contains("alpha"), "the file view went missing");
    // `../` is the probe for "the explorer is drawn": every listing has
    // a parent entry, and the block title cannot contain `..` because
    // `set_dir` collapses it (#78). This used to look for the `>>`
    // selection marker, which no longer exists — leaving the assertion
    // vacuously true.
    assert!(!after.contains("../"), "the explorer is still on screen");
}

/// `b` restores the split, but deliberately leaves the cursor where it
/// moved it: you pressed `b` to read the file, so being dropped back into
/// the explorer on the way out would be the surprise. `e` is the way back.
#[test]
fn b_toggles_back_but_leaves_focus_in_the_file_view() {
    let mut app = app_over_file("zoom_b_back", "alpha\n");
    assert_eq!(app.focus, Focus::Explorer);

    // Capture the baseline with focus already where `b b` will leave it,
    // so the comparison below isolates the layout claim rather than also
    // depending on the active/inactive distinction being style-only
    // (which `rendered`, collecting `symbol()` alone, cannot see).
    focus_file_view(&mut app);
    let before = rendered(&mut app);
    key(&mut app, KeyCode::Tab); // back to the real starting point

    key(&mut app, KeyCode::Char('b'));
    key(&mut app, KeyCode::Char('b'));

    assert_eq!(rendered(&mut app), before, "b did not restore the split");
    assert_eq!(
        app.focus,
        Focus::View,
        "focus was dragged back to the explorer"
    );
    assert_eq!(app.panes.shown(), PaneSet::ALL);
}

/// Hiding the column the cursor is in must move focus somewhere visible,
/// or the user is left typing into a pane that is not on screen.
#[test]
fn b_moves_focus_out_of_the_hidden_column() {
    let mut app = app_over_file("zoom_b_focus", "alpha\n");
    assert_eq!(app.focus, Focus::Explorer, "starts in the explorer");

    key(&mut app, KeyCode::Char('b'));

    assert_eq!(app.focus, Focus::View);
}

/// `e` is how you get back, so it must work from a hidden state. It
/// shows the explorer and nothing else (#300): a zoom is only a hide, so
/// the filter pane `b` hid stays hidden.
#[test]
fn e_shows_the_explorer_and_focuses_it() {
    let mut app = app_over_file("zoom_e", "alpha\n");
    key(&mut app, KeyCode::Char('b'));

    key(&mut app, KeyCode::Char('e'));

    assert!(
        app.panes.is_shown(Focus::Explorer),
        "the explorer is still hidden"
    );
    assert!(app.panes.is_shown(Focus::View));
    assert!(
        !app.panes.is_shown(Focus::Filters),
        "e showed more than the explorer"
    );
    assert_eq!(app.focus, Focus::Explorer);
}

#[test]
fn e_focuses_the_explorer_even_when_nothing_is_hidden() {
    let mut app = app_over_file("zoom_e_visible", "alpha\n");
    focus_file_view(&mut app);
    assert_ne!(app.focus, Focus::Explorer);

    key(&mut app, KeyCode::Char('e'));

    assert_eq!(app.focus, Focus::Explorer);
}

/// `t` reaches the text pane directly, the way `e` reaches the explorer.
#[test]
fn t_focuses_the_file_view() {
    let mut app = app_over_file("focus_t", "alpha\n");
    assert_ne!(app.focus, Focus::View);

    key(&mut app, KeyCode::Char('t'));

    assert_eq!(app.focus, Focus::View);
}
