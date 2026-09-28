use super::*;

/// Row index of the lowest pane border, which is where the pane area ends.
///
/// A pane's own `└` is the honest probe for "the panes reach this row" —
/// it is drawn by the border, not by anything the status line writes.
fn pane_bottom_row(app: &mut App) -> u16 {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    (0..AREA.height)
        .rev()
        // Plain or thick: a focused pane draws a heavy border, so its
        // own corner is `┗` rather than `└`.
        .find(|&y| (0..AREA.width).any(|x| matches!(buf[(x, y)].symbol(), "└" | "┗")))
        .expect("no bordered pane was drawn")
}

/// The bottom row is permanent, so the panes keep the same rows whether or
/// not a filter exists. The layout shifting under the user on the way to
/// the first filter is the defect this pins.
#[test]
fn the_panes_do_not_move_when_the_first_filter_appears() {
    let mut app = app_over_file("status_stable", "alpha\nbeta\n");
    let before = pane_bottom_row(&mut app);

    app.add_filter("alpha").expect("valid pattern");

    assert_eq!(
        before,
        pane_bottom_row(&mut app),
        "the panes resized when the first filter appeared"
    );
}

/// What earns the row its permanence: the directory is there to show
/// whether or not a filter is, so the row is never blank.
///
/// Asserts on the tail because the path elides from the left — the tail is
/// what identifies where you are.
#[test]
fn the_status_line_names_the_current_directory() {
    let mut app = app_over_file("status_dir", "alpha\n");

    let bottom = status_line(&mut app);

    assert!(
        bottom.contains("status_dir"),
        "the directory is not named: {bottom}"
    );
}

/// While a preview is on screen the document holds only the preview's
/// lines, so reporting that count as the file's total is confidently
/// wrong: the file reads as exactly `PREVIEW_LINES` long, whatever its
/// real length. Report the estimate instead, and say it is one.
#[test]
fn the_status_line_marks_a_previewed_total_as_an_estimate() {
    let mut app = app_over_file("status_preview", "alpha\n");
    app.add_filter("line").expect("valid pattern");

    // Past the preview's line cap, so the view truncates and there is an
    // estimate to report.
    let body = numbered_lines(crate::widgets::fileview::PREVIEW_LINES + 100);
    let dir = fixture_dir_path("status_preview");
    fs::write(dir.join("big.txt"), &body).expect("write");
    app.perform_widget_action(Action::Preview(dir.join("big.txt")));

    let bottom = status_line(&mut app);

    assert!(
        bottom.contains("(preview)"),
        "the total is not flagged as a preview: {bottom}"
    );
    assert!(
        bottom.contains('~'),
        "the total is not marked as an estimate: {bottom}"
    );
    assert!(
        !bottom.contains(&format!("/{} ", crate::widgets::fileview::PREVIEW_LINES)),
        "reported the preview's own line count as the file's total: {bottom}"
    );
}

/// `elide_left` cuts to a column budget, not a `char` budget. Counting
/// chars over-fills by one column per wide glyph, and the status row then
/// overruns the terminal it was supposed to fit inside (#97).
#[test]
fn eliding_a_path_of_wide_glyphs_fits_the_column_budget() {
    // 6 ideographs = 12 columns, plus `/x` = 14. Asked for 10.
    let path = "日本語ロググ/x";
    assert_eq!(UnicodeWidthStr::width(path), 14);

    let elided = elide_left(path, 10);

    assert!(
        UnicodeWidthStr::width(elided.as_str()) <= 10,
        "elided to {} columns, budget was 10: {elided:?}",
        UnicodeWidthStr::width(elided.as_str())
    );
    assert!(elided.starts_with('…'), "the cut is unmarked: {elided:?}");
    assert!(
        elided.ends_with("/x"),
        "cut from the wrong end — the tail is what identifies a path: {elided:?}"
    );
}

/// A path that already fits is returned whole, wide glyphs included.
#[test]
fn a_path_of_wide_glyphs_that_fits_is_not_elided() {
    let path = "日本語/x";
    assert_eq!(UnicodeWidthStr::width(path), 8);
    assert_eq!(elide_left(path, 8), path);
}

/// The row cannot hold everything on a narrow terminal, so it has a
/// priority: the counts survive whole and the path gives way.
///
/// That order is not arbitrary. A path cut down to `…/status_narrow` still
/// says where you are, where `3/3 lines sh` is not a shorter truth but a
/// broken one — so the part that cannot degrade goes first.
#[test]
fn a_narrow_status_line_keeps_the_counts_and_elides_the_path() {
    let mut app = app_over_file("status_narrow", "alpha\nbeta\ngamma\n");
    app.add_filter("beta").expect("valid pattern");

    let bottom = status_line_at(&mut app, 44);

    assert!(
        bottom.contains("1 filter") && bottom.contains("3/3 lines shown"),
        "the counts were cut to make room for the path: {bottom}"
    );
    // The tail is the whole point: clipping the row at its width would
    // leave the *head* (`/Users/pete/pro…`), which says nothing about
    // where you are. Eliding from the left keeps the part that does.
    assert!(
        bottom.contains("status_narrow"),
        "the path was cut from the right, so its identifying tail is gone: {bottom}"
    );
}

/// With no filters the row reports no filter state — but it is still
/// surrendered, because it is permanent.
///
/// Replaces `no_row_is_surrendered_without_filters`, which asserted that
/// the panes reached the bottom row when there was nothing to report. That
/// is the conditional layout whose shifting-under-the-user is the defect
/// here; its objection was that a permanent row costs a row to say
/// nothing, and the row now always names the directory instead.
#[test]
fn no_filter_state_is_reported_without_filters() {
    let mut app = app_over_file("status_none", "alpha\n");

    let bottom = status_line(&mut app);

    // The row's filter state is always "<count> filter(s)", so look for
    // the word, not the substring: the row also names the directory,
    // and a worktree such as `.worktrees/Fix-I123-predicate-filters`
    // used to fail this test on its path alone.
    assert!(
        !bottom
            .split_whitespace()
            .any(|word| word == "filter" || word == "filters"),
        "reported filter state when no filters exist: {bottom}"
    );
    assert!(
        !bottom.contains('└') && !bottom.contains('┗'),
        "the panes still reach the bottom row, so the layout still shifts: {bottom}"
    );
}

#[test]
fn the_status_line_reports_filters_and_lines_shown() {
    let mut app = app_over_file("status_some", "alpha\nbeta\ngamma\n");
    // Set directly rather than through `f i … Enter` (#120): this
    // fixture's single-file directory has nothing else for the chain's
    // synthetic `n` to step to (the scan has not answered yet at this
    // point in the test), and the resulting "no matching file" report
    // would displace the status this test means to check. That
    // interaction belongs to the keymap-chain tests, not this one.
    app.add_filter("beta").expect("valid pattern");
    app.refresh_view();

    let status = status_line(&mut app);

    assert!(
        status.contains("1 filter") && !status.contains("1 filters"),
        "count is not singular-aware: {status}"
    );
    // An including filter alone dims rather than removes, so every line
    // is still shown even though only one of them matched.
    assert!(
        status.contains("3/3"),
        "lines-shown count missing: {status}"
    );
}

#[test]
fn the_status_line_says_when_filters_are_disabled() {
    let mut app = app_over_file("status_off", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('!'));

    assert!(
        status_line(&mut app).contains("disabled"),
        "no indication the filters are off: {}",
        status_line(&mut app)
    );
}

/// With every filter disabled, `any_including` is false, so issue #36's
/// guard in `Document::recompute_visible` shows the whole file even in
/// `FilteredOnly` mode — nothing is actually hidden. Gating `hiding` on
/// `mode == FilteredOnly` alone (rather than requiring `any_including`
/// too) would still show the funnel here, claiming lines were hidden
/// over a pane that is in fact showing everything.
#[test]
fn the_status_line_does_not_show_a_funnel_when_disabled_filters_cannot_hide_anything() {
    let mut app = app_over_file("status_off_no_funnel", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('!'));
    key(&mut app, KeyCode::Char('H'));

    assert!(
        status_line(&mut app).contains("disabled"),
        "sanity: still reports disabled: {}",
        status_line(&mut app)
    );
    assert!(
        !status_line(&mut app).contains('▼'),
        "funnel claimed lines were hidden while the #36 guard is \
         showing everything: {}",
        status_line(&mut app)
    );
}

/// An open prompt takes the row, as it already does.
#[test]
fn a_prompt_still_takes_the_bottom_row() {
    let mut app = app_over_file("status_prompt", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "foo");

    assert_eq!(status_line(&mut app), "filter: foo");
}
