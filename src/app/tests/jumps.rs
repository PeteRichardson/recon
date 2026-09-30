use super::*;

/// Two matching logs, one unmatched between them, scan answers in place.
/// Returns the app with `a.log` loaded and the view focused.
fn app_over_matching_logs(name: &str) -> (App<'static>, Sender<scan::Scanned>) {
    let mut app = app_over_files(
        name,
        &[
            ("a.log", "plain\nhit a1\nhit a2\n"),
            ("b.log", "plain\nplain\n"),
            ("c.log", "hit c1\nplain\nhit c2\n"),
        ],
    );
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("hit").expect("valid pattern");
    app.refresh_scan(false);
    mark(&mut app, &tx, 0, true);
    mark(&mut app, &tx, 1, false);
    mark(&mut app, &tx, 2, true);
    open_file(&mut app, 0);
    key(&mut app, KeyCode::Char('t'));
    (app, tx)
}

// ---- where a jump lands --------------------------------------

const TALL_TEXT_ROWS: u16 = 33;

/// `App` over `body` with an include filter `pattern` and the view
/// focused, rendered once so the pane knows its height.
fn app_searching(name: &str, body: &str, pattern: &str, config: Config) -> App<'static> {
    let file = fixture_path(name, body);
    let mut app = App::new(&Startup::from(Config {
        path: file.display().to_string(),
        ..config
    }));
    draw_tall(&mut app);
    key(&mut app, KeyCode::Char('t'));
    app.filters.add(pattern).expect("valid pattern");
    app.refresh_view();
    draw_tall(&mut app);
    assert_eq!(
        cursor_screen_row(&app),
        0,
        "sanity: the filter alone moved the cursor"
    );
    app
}

/// A hit the pane was not showing is put in the middle of it, so there
/// is context on both sides — the pane had to redraw anyway.
#[test]
fn n_to_a_hit_off_screen_centres_it_in_the_pane() {
    let mut app = app_searching(
        "n_centres",
        &numbered_lines(400),
        "line 150$",
        Config::default(),
    );

    key(&mut app, KeyCode::Char('n'));
    draw_tall(&mut app);

    assert_eq!(cursor_source(&app), 150, "did not reach the hit");
    assert_eq!(
        cursor_screen_row(&app),
        TALL_TEXT_ROWS / 2,
        "the hit was not centred"
    );
}

/// A hit the pane is already showing is selected where it is: nothing
/// scrolls, so the eye does not have to find the text again.
#[test]
fn n_to_a_hit_already_on_screen_does_not_scroll() {
    let mut app = app_searching(
        "n_on_screen",
        &numbered_lines(400),
        "line 10$",
        Config::default(),
    );

    key(&mut app, KeyCode::Char('n'));
    draw_tall(&mut app);

    assert_eq!(cursor_source(&app), 10, "did not reach the hit");
    assert_eq!(cursor_screen_row(&app), 10, "the view scrolled");
}

/// Centring never scrolls past the end of the file: a hit near the last
/// line lands wherever the end-of-file view puts it, with no blank rows
/// pulled in below to make it central.
#[test]
fn n_to_a_hit_near_the_end_does_not_scroll_past_the_end() {
    let mut app = app_searching(
        "n_near_end",
        &numbered_lines(400),
        "line 398$",
        Config::default(),
    );

    key(&mut app, KeyCode::Char('n'));
    draw_tall(&mut app);

    assert_eq!(cursor_source(&app), 398, "did not reach the hit");
    assert_eq!(
        cursor_screen_row(&app),
        TALL_TEXT_ROWS - 2,
        "the last lines are not on the pane's last rows"
    );
}

/// `[view] center_jumps = false`: an off-screen hit scrolls in by the
/// minimum, landing on the bottom margin's edge like a held `j` would.
#[test]
fn center_jumps_off_lands_an_off_screen_hit_on_the_bottom_margin() {
    let mut app = app_searching(
        "n_no_centre",
        &numbered_lines(400),
        "line 150$",
        Config {
            center_jumps: Some(false),
            ..Config::default()
        },
    );

    key(&mut app, KeyCode::Char('n'));
    draw_tall(&mut app);

    assert_eq!(cursor_source(&app), 150, "did not reach the hit");
    let margin = crate::widgets::fileview::SCROLL_MARGIN as u16;
    assert_eq!(cursor_screen_row(&app), TALL_TEXT_ROWS - 1 - margin);
}

/// The margin survives the window being rebuilt under a held `j`: the
/// rebuild's restore puts the cursor back on the row it was captured
/// on, and that row was measured before the viewport had followed the
/// keypress — one row into the margin.
#[test]
fn a_held_j_keeps_the_margin_across_a_window_rebuild() {
    let mut app = app_over_file("margin_rebuild", &numbered_lines(2000));
    draw_tall(&mut app);
    key(&mut app, KeyCode::Char('t'));
    // `G` then `g` re-windows against the real pane height rather than
    // the pre-render assumption, so a rebuild comes due within reach.
    key(&mut app, KeyCode::Char('G'));
    key(&mut app, KeyCode::Char('g'));
    draw_tall(&mut app);
    let lowest = TALL_TEXT_ROWS - 1 - crate::widgets::fileview::SCROLL_MARGIN as u16;

    let mut rebuilt = false;
    for press in 0..300u16 {
        key(&mut app, KeyCode::Char('j'));
        draw_tall(&mut app);
        rebuilt |= app.view.window_start() != 0;
        if press >= lowest {
            assert_eq!(
                cursor_screen_row(&app),
                lowest,
                "press {press} left the cursor off the margin's edge"
            );
        }
    }
    assert!(rebuilt, "sanity: the window was never rebuilt");
}

/// Line-oriented, not span-oriented: three hits on one line is one stop.
/// `recon` is a line-focused tool, and the alternative cannot be explained
/// without explaining the implementation.
#[test]
fn n_stops_once_on_a_line_with_several_matches() {
    let mut app = app_over_file("n_once", "foo foo foo\nbar\nfoo\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("foo").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 2, "stopped more than once on line 0");
}

/// The cursor starts *past* the only hit, so landing back on it requires
/// an actual wrap through index 0 — not merely "the cursor never moved",
/// which is what the previous version of this test asserted (cursor
/// already sat on the sole hit, so it passed even with `n` unbound).
#[test]
fn n_wraps_at_the_end_of_the_file() {
    let mut app = app_over_file("n_wrap", "hit\nplain\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();
    assert_eq!(cursor_source(&app), 2, "sanity: cursor starts past the hit");

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(cursor_source(&app), 0, "did not wrap around to the hit");
    assert_eq!(
        status(&app),
        Some(WRAPPED_TO_TOP),
        "the wrap went unreported"
    );
}

/// `step_visible` is the one walk `n`/`N` and the search share (#269).
/// It says whether it landed, wrapped, or found nothing, so the caller
/// can say so; and its predicate sees rows of the *visible* set, so in
/// hide mode a hidden line is never offered to it.
#[test]
fn step_visible_reports_the_wrap_and_walks_only_visible_rows() {
    let mut app = app_over_file("step_visible", "hit a\nplain\nhit b\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();
    let hit = |document: &Document, row: usize| {
        document
            .source_at(row)
            .is_some_and(|source| document.lines()[source].starts_with("hit"))
    };

    assert_eq!(app.step_visible(false, hit), Step::Landed);
    assert_eq!(cursor_source(&app), 2, "forward from 0");
    assert_eq!(app.step_visible(false, hit), Step::Wrapped);
    assert_eq!(cursor_source(&app), 0, "past the end, back to the top");
    assert_eq!(app.step_visible(true, hit), Step::Wrapped);
    assert_eq!(cursor_source(&app), 2, "before the start, back to the end");
    assert_eq!(app.step_visible(true, hit), Step::Landed);
    assert_eq!(cursor_source(&app), 0, "backward from 2");

    assert_eq!(
        app.step_visible(false, |_, _| false),
        Step::Nothing,
        "nothing accepted"
    );
    assert_eq!(cursor_source(&app), 0, "moved with nothing accepted");

    // The only accepted row is the cursor's own: a wrap onto itself.
    let only_a = |document: &Document, row: usize| document.source_at(row) == Some(0);
    assert_eq!(app.step_visible(false, only_a), Step::Wrapped);
    assert_eq!(cursor_source(&app), 0, "stays put on the only hit");

    // Hide mode: the visible set is the two hits, rows 0 and 1. A
    // predicate that accepts every row it is shown never sees `plain`.
    key(&mut app, KeyCode::Char('u'));
    let offered = std::cell::RefCell::new(Vec::new());
    let _ = app.next_visible_row(false, |document, row| {
        offered.borrow_mut().push(document.source_at(row));
        false
    });
    assert_eq!(
        offered.into_inner(),
        vec![Some(2), Some(0)],
        "hidden lines were offered"
    );
    assert_eq!(app.step_visible(false, |_, _| true), Step::Landed);
    assert_eq!(cursor_source(&app), 2, "next visible row in hide mode");
}

/// The strict step is what lets `n` know it has run out of hits in this
/// file: unlike `step_to_interesting` it refuses to wrap.
#[test]
fn next_interesting_strict_does_not_wrap() {
    let mut app = app_over_file("strict_no_wrap", "hit\nplain\nhit\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();

    assert_eq!(
        app.next_interesting_strict(false),
        Some(2),
        "forward from 0"
    );
    assert_eq!(app.next_interesting_strict(true), None, "nothing before 0");

    app.land_on(2);
    assert_eq!(cursor_source(&app), 2, "land_on moved the cursor");
    assert_eq!(app.next_interesting_strict(false), None, "nothing after 2");
    assert_eq!(
        app.next_interesting_strict(true),
        Some(0),
        "backward from 2"
    );
}

#[test]
fn first_interesting_finds_either_end() {
    let mut app = app_over_file("first_either_end", "plain\nhit\nplain\nhit\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();

    assert_eq!(app.first_interesting(false), Some(1));
    assert_eq!(app.first_interesting(true), Some(3));
}

#[test]
fn first_interesting_is_none_without_hits() {
    let mut app = app_over_file("first_none", "plain\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();

    assert_eq!(app.first_interesting(false), None);
    assert_eq!(app.next_interesting_strict(false), None);
}

/// Three hits, cursor on the middle one: forward and backward from there
/// land on different lines (4 and 0 respectively), so this actually
/// exercises direction. The previous version started at row 0 with only
/// two hits, where forward and backward both wrap to the same place —
/// it would have passed even if `N` were wired to walk forward.
#[test]
fn capital_n_walks_backwards() {
    let mut app = app_over_file("n_back", "hit a\nplain\nhit b\nplain\nhit c\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();
    assert_eq!(
        cursor_source(&app),
        2,
        "sanity: cursor starts on the middle hit"
    );

    key(&mut app, KeyCode::Char('N'));

    assert_eq!(
        cursor_source(&app),
        0,
        "N did not walk backwards to the earlier hit (forward would reach 4)"
    );
}

/// Quiet, not a panic and not a jump to line 0.
#[test]
fn n_with_nothing_interesting_does_nothing() {
    let mut app = app_over_file("n_empty", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    let before = cursor_source(&app);

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(cursor_source(&app), before);
}

/// `n` belongs to the file view. Hoisting it into `App` must not make it
/// global — in the explorer it is still the explorer's key.
///
/// `app_over` writes a single-line "x" into every fixture file, which
/// made the previous version of this test vacuous: with one line, the
/// only hit is already on row 0, so a leaked, fully global `n` binding
/// would still be a no-op and the test would pass regardless. This fixture
/// puts the hit on row 1, so a leak is observable as the cursor moving.
#[test]
fn n_in_the_explorer_does_not_move_the_file_view_cursor() {
    let dir = fixture_dir("n_explorer");
    fs::write(dir.join("alpha.log"), "alpha\nx\n").expect("write fixture");

    let mut app = App::new(&Startup::from(Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    }));
    // The startup argument names a file that does not exist, so `App::new`
    // loads that (an error message, not real content) and the explorer
    // falls back to selecting the one real entry. `Down` is what actually
    // previews it into the file view — the same two-step construction
    // `n_promotes_a_truncated_preview_before_stepping` uses, for the same
    // reason.
    key(&mut app, KeyCode::Down);
    app.filters.add("x").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('e'));
    let before = cursor_source(&app);
    assert_eq!(before, 0, "sanity: cursor starts above the hit on row 1");

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(cursor_source(&app), before, "n leaked out of the file view");
}

/// The loop-collapsing change (#120 §1): `n` past the last hit in a file
/// goes to the first hit of the next file the filters selected, skipping
/// files the scan said no to.
#[test]
fn n_at_the_last_hit_crosses_to_the_next_matching_file() {
    let (mut app, _tx) = app_over_matching_logs("cross_next");
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 2, "sanity: on the last hit of a.log");

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(
        shown(&app),
        "c.log",
        "did not cross, or stopped on the unmatched b.log"
    );
    assert_eq!(cursor_source(&app), 0, "did not land on the first hit");
    assert_eq!(
        app.explorer.selected_name().as_deref(),
        Some("c.log"),
        "the explorer's selection did not follow"
    );
    assert_eq!(
        app.status_message.as_ref().map(|m| m.text.as_str()),
        Some("next file · c.log")
    );
    assert!(app.crossing.is_some());
}

#[test]
fn capital_n_at_the_first_hit_crosses_to_the_previous_files_last_hit() {
    let (mut app, _tx) = app_over_matching_logs("cross_prev");
    open_file(&mut app, 2);
    assert_eq!(cursor_source(&app), 0, "sanity: on c.log's first hit");

    key(&mut app, KeyCode::Char('N'));

    assert_eq!(shown(&app), "a.log");
    assert_eq!(cursor_source(&app), 2, "did not land on the last hit");
    assert_eq!(
        app.status_message.as_ref().map(|m| m.text.as_str()),
        Some("previous file · a.log")
    );
}

/// With no other matching file, `n` wraps within the file as it always
/// has, and nothing claims a crossing happened.
#[test]
fn n_wraps_within_the_file_when_no_other_file_matches() {
    let (mut app, tx) = app_over_matching_logs("cross_alone");
    mark(&mut app, &tx, 2, false);
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 2);

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(shown(&app), "a.log");
    assert_eq!(cursor_source(&app), 1, "did not wrap to the first hit");
    assert!(app.crossing.is_none());
}

/// With no interesting line anywhere in the file — and no other file to
/// cross to — `step_to_interesting`'s wrap would be a silent no-op. Report
/// the dead end instead of leaving the cursor sitting there unexplained.
#[test]
fn n_with_nothing_to_step_through_says_so() {
    let mut app = app_over_file("n_dead_end", "plain\nplain\n");
    focus_file_view(&mut app);
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();
    let before = cursor_source(&app);

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(
        cursor_source(&app),
        before,
        "cursor moved with nothing to step through"
    );
    assert_eq!(
        app.status_message.as_ref().map(|m| m.text.as_str()),
        Some("no interesting line")
    );
}

/// A filename search in the explorer does not redirect the content loop.
#[test]
fn n_crossing_ignores_the_explorer_filename_search() {
    let (mut app, _tx) = app_over_matching_logs("cross_ignores_search");
    app.explorer.search("b", false).expect("valid pattern");
    open_file(&mut app, 0);
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('n'));

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(
        shown(&app),
        "c.log",
        "followed the filename search to b.log"
    );
}

/// A context filter's lines are shown *around* the hits, not as hits:
/// `n` skips them, hide mode keeps them, and they keep their colour.
/// The explorer already treats context this way when it marks files
/// (`Sense::Context` is left out of the scan's selecting mask), so the
/// view's `n` now agrees with it.
#[test]
fn n_skips_lines_that_only_a_context_filter_matches() {
    let mut app = app_over_file(
        "ctx_not_interesting",
        "ctx before\nERROR one\nctx after\nplain\nERROR two\n",
    );
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("ctx").expect("valid pattern");
    app.filters.toggle_context(0);
    app.filters.add("ERROR").expect("valid pattern");
    app.refresh_view();
    assert_eq!(
        app.filters.filters()[0].sense,
        filter::Sense::Context,
        "sanity"
    );

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 1, "stopped on a context line");
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 4, "stopped on a context line");

    // Hide mode still shows the context lines, in the filter's colour.
    key(&mut app, KeyCode::Char('H'));
    let visible: Vec<usize> = (0..4)
        .filter_map(|row| app.document.source_at(row))
        .collect();
    assert_eq!(visible, [0, 1, 2, 4], "hide mode dropped a context line");
    assert!(
        app.filters.style_for(app.document.verdicts()[0]).is_some(),
        "a context line lost its colour"
    );
}

/// `.`/`,` ignore the explorer's filename search too, same as `n`/`N`.
#[test]
fn dot_ignores_the_explorer_filename_search() {
    let (mut app, _tx) = app_over_matching_logs("dot_ignores_search");
    app.explorer.search("b", false).expect("valid pattern");
    open_file(&mut app, 0);

    key(&mut app, KeyCode::Char('.'));

    assert_eq!(
        shown(&app),
        "c.log",
        "followed the filename search to b.log"
    );
}

/// The in-file step still comes first: `n` with hits remaining in this
/// file must not cross.
#[test]
fn n_prefers_the_next_hit_in_this_file() {
    let (mut app, _tx) = app_over_matching_logs("cross_prefers_local");

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(shown(&app), "a.log");
    assert_eq!(cursor_source(&app), 1);
    assert!(app.crossing.is_none());
}

/// #120 §2 decision (b): the filter pane forwards `n` to the view.
#[test]
fn n_from_the_filter_pane_acts_on_the_file_view() {
    let (mut app, _tx) = app_over_matching_logs("cross_via_filter_pane");
    key(&mut app, KeyCode::Char('f'));

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(cursor_source(&app), 1, "n was swallowed by the filter pane");
    assert_eq!(app.focus, Focus::Filters, "focus moved");
}

/// The notice and the status line are cleared by the next keypress, like
/// every other status message.
#[test]
fn a_crossing_is_forgotten_on_the_next_key() {
    let (mut app, _tx) = app_over_matching_logs("cross_forgotten");
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('n'));
    assert!(app.crossing.is_some(), "sanity");

    key(&mut app, KeyCode::Char('j'));

    assert!(app.crossing.is_none());
    assert!(app.status_message.is_none());
}

/// #120 §4: with every filter disabled by the peek, a step would find no
/// interesting line and cross files at once. Put the filters back first.
#[test]
fn n_while_peeked_restores_the_peek_before_moving() {
    let (mut app, _tx) = app_over_matching_logs("peek_then_n");
    key(&mut app, KeyCode::Char(' '));
    assert!(app.peek.is_some(), "sanity: peeking");

    key(&mut app, KeyCode::Char('n'));

    assert!(app.peek.is_none(), "still peeking");
    assert_eq!(shown(&app), "a.log", "crossed files instead of stepping");
    assert_eq!(cursor_source(&app), 1);
}

/// The restore has to reach the explorer's answers too, or `n` at the
/// last hit finds no file to cross to and wraps in silence.
#[test]
fn n_at_the_last_hit_while_peeked_still_crosses_files() {
    let (mut app, _tx) = app_over_matching_logs("peek_then_n_crosses");
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 2, "sanity: on the last hit of a.log");
    key(&mut app, KeyCode::Char(' '));
    assert!(app.peek.is_some(), "sanity: peeking");

    key(&mut app, KeyCode::Char('n'));

    assert!(app.peek.is_none());
    assert_eq!(
        shown(&app),
        "c.log",
        "did not cross after restoring the peek"
    );
    assert_eq!(cursor_source(&app), 0);
}

/// `cross_file` promotes a truncated preview between `perform` and the
/// landing step; without it, a jump onto a large file whose only hit sits
/// past `PREVIEW_LINES` would land on the bounded preview and miss it.
#[test]
fn comma_lands_on_the_last_hit_of_a_file_that_was_only_previewed() {
    let dir = fixture_dir("comma_lands_on_truncated");
    fs::write(dir.join("a.log"), "hit a\n").expect("write fixture");
    // Past PREVIEW_LINES, so the first preview of this file is truncated;
    // the hit sits beyond the preview boundary, reachable only once
    // `cross_file` promotes it.
    let hit_at = crate::widgets::fileview::PREVIEW_LINES + 50;
    let body: String = (0..crate::widgets::fileview::PREVIEW_LINES + 100)
        .map(|i| {
            if i == hit_at {
                "HIT\n".to_string()
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    fs::write(dir.join("big.log"), &body).expect("write fixture");

    let mut app = App::new(&Startup::from(Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    }));
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("HIT|hit a").expect("valid pattern");
    app.refresh_scan(false);
    mark(&mut app, &tx, 0, true);
    mark(&mut app, &tx, 1, true);
    open_file(&mut app, 0);
    focus_file_view(&mut app);

    key(&mut app, KeyCode::Char(','));

    assert_eq!(shown(&app), "big.log");
    assert_eq!(
        cursor_source(&app),
        hit_at,
        "did not reach the hit past the truncated preview"
    );
}

/// `,` into the previous file lands on its last hit where a jump lands:
/// centred when there is file enough on both sides, and with the whole
/// visible set on screen when there is not — never on the top row with
/// blank rows below (#192).
///
/// The landing used to lose to the file load that precedes it in the
/// same keypress: the load's window rebuild queued a restore to the row
/// the *previous* file's cursor was drawn on, and `scroll_cursor_to_row`
/// keeps the first request. So `,` landed the hit at the top margin of
/// a pane that had never shown it. `land_cursor_on_row` is the jump's
/// request, and it replaces the restore.
#[test]
fn comma_centres_the_previous_file_s_last_hit_or_shows_every_hit() {
    let dir = fixture_dir("comma_lands_centred");
    let body: String = (0..400)
        .map(|i| {
            if [100, 150, 200].contains(&i) {
                "HIT\n".to_string()
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    fs::write(dir.join("a.log"), &body).expect("write");
    fs::write(dir.join("b.log"), "HIT\n").expect("write");
    let mut app = App::new(&Startup::from(Config {
        path: dir.join("b.log").display().to_string(),
        ..Config::default()
    }));
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("HIT").expect("valid");
    app.refresh_scan(false);
    mark(&mut app, &tx, 0, true);
    mark(&mut app, &tx, 1, true);
    open_file(&mut app, 1);
    focus_file_view(&mut app);
    draw_tall(&mut app);

    // Dimmed: 400 lines and the last hit at 200, with room on both sides.
    key(&mut app, KeyCode::Char(','));
    draw_tall(&mut app);
    assert_eq!(shown(&app), "a.log");
    assert_eq!(cursor_source(&app), 200);
    assert_eq!(
        cursor_screen_row(&app),
        TALL_TEXT_ROWS / 2,
        "the last hit was not centred"
    );

    // Hide mode: three visible lines, so all three are on screen and the
    // target is the third row, not the first with blank rows under it.
    key(&mut app, KeyCode::Char('.'));
    draw_tall(&mut app);
    assert_eq!(shown(&app), "b.log", "sanity: back on the later file");
    ctrl(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char(','));
    draw_tall(&mut app);
    assert_eq!(shown(&app), "a.log");
    assert_eq!(cursor_source(&app), 200);
    assert_eq!(
        app.view.textarea().scroll_top().0,
        0,
        "the last hit is on the top row with blank rows below"
    );
    assert_eq!(cursor_screen_row(&app), 2);
}

/// In-file motions leave the peek alone: that is what peeking is for.
#[test]
fn j_while_peeked_keeps_the_peek() {
    let (mut app, _tx) = app_over_matching_logs("peek_then_j");
    key(&mut app, KeyCode::Char(' '));

    key(&mut app, KeyCode::Char('j'));

    assert!(app.peek.is_some());
}

/// `.`/`,` are global: the same step from all three panes, with focus
/// left where it was.
#[test]
fn dot_and_comma_step_files_from_every_pane() {
    let (mut app, _tx) = app_over_matching_logs("skip_every_pane");

    key(&mut app, KeyCode::Char('.'));
    assert_eq!(shown(&app), "c.log", "from the view");
    assert_eq!(cursor_source(&app), 0);

    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('.'));
    assert_eq!(shown(&app), "a.log", "from the explorer (wrapped)");
    assert_eq!(app.focus, Focus::Explorer);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char(','));
    assert_eq!(shown(&app), "c.log", "from the filter pane, backwards");
    assert_eq!(cursor_source(&app), 2, "`,` lands on the last hit");
    assert_eq!(app.focus, Focus::Filters);
}

/// `.` does not wait for the current file to be exhausted.
#[test]
fn dot_skips_the_rest_of_the_current_file() {
    let (mut app, _tx) = app_over_matching_logs("skip_rest");
    assert_eq!(cursor_source(&app), 0, "sanity: hits remain below");

    key(&mut app, KeyCode::Char('.'));

    assert_eq!(shown(&app), "c.log");
    assert!(app.crossing.is_some());
}

#[test]
fn dot_with_no_other_matching_file_says_so() {
    let (mut app, tx) = app_over_matching_logs("skip_alone");
    mark(&mut app, &tx, 2, false);

    key(&mut app, KeyCode::Char('.'));

    assert_eq!(shown(&app), "a.log");
    assert_eq!(
        app.status_message.as_ref().map(|m| m.text.as_str()),
        Some("no other file matches")
    );
}

#[test]
fn dot_while_peeked_restores_the_peek_first() {
    let (mut app, _tx) = app_over_matching_logs("skip_peeked");
    key(&mut app, KeyCode::Char(' '));

    key(&mut app, KeyCode::Char('.'));

    assert!(app.peek.is_none());
    assert_eq!(shown(&app), "c.log");
}

/// Log files look alike. A crossing paints a notice over the view and
/// accents its title; the next key clears both (#120 §1).
#[test]
fn a_crossing_paints_a_notice_over_the_file_view() {
    let (mut app, _tx) = app_over_matching_logs("notice_paint");
    key(&mut app, KeyCode::Char('.'));

    let screen = rendered(&mut app);
    assert!(
        screen.contains("▼ next file · c.log"),
        "no notice on screen:\n{screen}"
    );

    key(&mut app, KeyCode::Char('j'));
    let screen = rendered(&mut app);
    assert!(
        !screen.contains("next file"),
        "notice survived a keypress:\n{screen}"
    );
}

#[test]
fn a_backwards_crossing_points_up() {
    let (mut app, _tx) = app_over_matching_logs("notice_up");
    key(&mut app, KeyCode::Char(','));

    let screen = rendered(&mut app);
    assert!(screen.contains("▲ previous file · c.log"), "{screen}");
}

#[test]
fn a_crossing_accents_the_view_title() {
    let (mut app, _tx) = app_over_matching_logs("notice_title");
    key(&mut app, KeyCode::Char('.'));
    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);

    // The title sits on the view's top border; find the first cell of
    // the file name and read its style. Search from the divider column:
    // the explorer's own border title is the fixture directory name,
    // which contains `notice_title` and would otherwise match `c` first.
    let title_cell = (app.divider..AREA.width)
        .map(|x| buf[(x, 0)].clone())
        .find(|cell| cell.symbol() == "c")
        .expect("the title is drawn on the top border");
    assert_eq!(title_cell.fg, Color::Yellow, "title not accented");

    key(&mut app, KeyCode::Char('j'));
    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);
    let title_cell = (app.divider..AREA.width)
        .map(|x| buf[(x, 0)].clone())
        .find(|cell| cell.symbol() == "c")
        .expect("title");
    assert_ne!(title_cell.fg, Color::Yellow, "accent survived a keypress");
}

/// `[`/`]` are global so a peeked file can be paged without leaving the
/// pane the loop is being driven from (#120 §3).
#[test]
fn brackets_page_the_file_view_from_the_explorer_and_filter_pane() {
    let body = numbered_lines(400);
    let mut app = app_over_file("brackets_global", &body);
    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(cursor_source(&app), 0, "sanity");

    key(&mut app, KeyCode::Char(']'));
    (&mut app).render(AREA, &mut buf);
    let after_page = cursor_source(&app);
    assert!(after_page > 0, "] from the explorer did not page the view");
    assert_eq!(app.focus, Focus::Explorer, "focus moved");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('['));
    (&mut app).render(AREA, &mut buf);
    assert!(
        cursor_source(&app) < after_page,
        "[ from the filter pane did not page up"
    );
    assert_eq!(app.focus, Focus::Filters);
}

/// `/` sets the search and moves to its first hit. The filter set is
/// untouched: a search is a motion, not a filter (ADR 0001).
#[test]
fn slash_sets_the_search_and_moves_to_its_first_hit() {
    let mut app = app_over_file("slash_motion", "alpha\nbeta\ngamma\nbeta again\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(search_text(&app), "beta");
    assert_eq!(cursor_source(&app), 1);
    assert!(app.filters.is_empty(), "the search became a filter");
    assert!(
        app.document
            .verdicts()
            .iter()
            .all(|v| *v == Verdict::Unmatched),
        "a search changed a verdict"
    );
}

/// The search starts on the cursor line, that line included: a hit
/// there is found first, not after a wrap.
#[test]
fn the_search_starts_on_the_cursor_line() {
    let mut app = app_over_file("slash_own_line", "beta\nalpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        cursor_source(&app),
        2,
        "the hit on the cursor line was skipped"
    );
    assert_eq!(status(&app), None, "nothing wrapped");
}

/// Past the last hit the search wraps to the top, once, and says so:
/// beside the prompt while typing, on the status row after Enter.
#[test]
fn the_search_wraps_to_the_top_and_says_so() {
    let mut app = app_over_file("slash_wrap", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('G'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "alpha");
    assert_eq!(cursor_source(&app), 0, "did not move while typing");
    let row = status_line(&mut app);
    assert!(
        row.contains("/alpha") && row.contains(WRAPPED_TO_TOP),
        "the wrap is not shown beside the prompt: {row}"
    );

    key(&mut app, KeyCode::Enter);

    assert_eq!(cursor_source(&app), 0);
    assert_eq!(status(&app), Some(WRAPPED_TO_TOP));
}
