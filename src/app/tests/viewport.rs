use super::*;

/// `key`, with Alt held.
///
/// No `keymap::DEFAULT` row carries Alt in any scope, so whatever this
/// sends is unbound by construction — which is the point at every call
/// site: an unbound *modified* key must do nothing at all.
fn alt(app: &mut App, code: KeyCode) {
    app.handle_event(event::Event::Key(event::KeyEvent::new(
        code,
        KeyModifiers::ALT,
    )));
}

/// `[` keeps paging a full screen once the window has been rebuilt at the
/// pane's real height (#108).
///
/// The window's slack was measured from the *cursor*, but a page scrolls
/// the *viewport*, whose top edge sits a full pane above the cursor when
/// `[` has parked it on the bottom row — which is exactly where `[` leaves
/// it. That left 3 buffer rows above the viewport on a 36-row terminal, so
/// `Viewport::scroll`'s `saturating_sub` clamped a 33-row page to 3, and
/// the re-anchor afterwards reproduced the same state indefinitely.
///
/// The reproduction needs a real-height window, which is why it pages down
/// so far first: the startup window is built before any render at
/// `ASSUMED_PANE_HEIGHT`, is 600 rows wide, and hides the bug for the first
/// twelve pages. `]` is unaffected throughout — it parks the cursor on the
/// pane's *top* row, where the viewport's edge and the cursor coincide.
#[test]
fn page_up_keeps_paging_a_full_screen_after_the_window_is_rebuilt() {
    let body = numbered_lines(7_000);
    // 36 rows, as reported: a 35-row file-view pane, 33 rows inside its
    // border. A page is one row short of the inner height.
    let area = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 36,
    };
    let mut app = app_over_file("page_up_full_screen.txt", &body);
    let mut buf = Buffer::empty(area);
    (&mut app).render(area, &mut buf);
    key(&mut app, KeyCode::Char('t'));
    (&mut app).render(area, &mut buf);

    let top = |app: &App| -> usize {
        let (scroll, _) = app.view.textarea().scroll_top();
        app.view.window_start() + scroll as usize
    };

    // Far enough down that the startup window has been replaced by one
    // sized to the real pane. Thirteen is the first page that does it.
    for _ in 0..13 {
        key(&mut app, KeyCode::Char(']'));
        (&mut app).render(area, &mut buf);
    }

    // Every page up moves a full page, not just the first two.
    for press in 1..=6 {
        let before = top(&app);
        key(&mut app, KeyCode::Char('['));
        (&mut app).render(area, &mut buf);
        let moved = before - top(&app);
        assert_eq!(
            moved,
            33,
            "`[` press {press} moved {moved} lines, not a full page \
             (view top {before} -> {})",
            top(&app),
        );
    }
}

/// Scrolling line by line must not rebuild the buffer on every keystroke.
///
/// This is the other half of #108 and the reason the window carries two
/// screens of slack rather than one. `window_holds` asks for a page of
/// buffer beyond each viewport edge so that a page never clamps; if
/// `window_for` laid down exactly that much, the requirement would be met
/// with zero margin and the first `j` that scrolled the viewport would owe
/// a rebuild — a `set_lines` per keystroke while holding `j`, which is the
/// cost #7 exists to avoid. The second screen is the margin.
#[test]
fn scrolling_line_by_line_does_not_rebuild_the_window_every_keystroke() {
    let body = numbered_lines(7_000);
    let area = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 36,
    };
    let mut app = app_over_file("no_rebuild_thrash.txt", &body);
    let mut buf = Buffer::empty(area);
    (&mut app).render(area, &mut buf);
    key(&mut app, KeyCode::Char('t'));
    (&mut app).render(area, &mut buf);

    // Page down far enough to replace the startup window — built before
    // any render at `ASSUMED_PANE_HEIGHT`, it is 200 screens wide and would
    // absorb every scroll below without ever rebuilding, which is a test
    // that passes for the wrong reason.
    for _ in 0..13 {
        key(&mut app, KeyCode::Char(']'));
        (&mut app).render(area, &mut buf);
    }
    // Get the cursor onto the pane's bottom row, where further `j` scrolls
    // the viewport rather than just moving the cursor down it.
    for _ in 0..40 {
        key(&mut app, KeyCode::Down);
        (&mut app).render(area, &mut buf);
    }

    let mut window = (app.view.window_start(), app.view.window_end());
    let mut rebuilds = 0;
    let presses = 60;
    for _ in 0..presses {
        key(&mut app, KeyCode::Down);
        (&mut app).render(area, &mut buf);
        let now = (app.view.window_start(), app.view.window_end());
        if now != window {
            rebuilds += 1;
            window = now;
        }
    }

    // Measured: with one screen of slack this is 30 rebuilds over 60
    // presses — every other keystroke. With two it is 0, and a third screen
    // buys nothing further, which is what fixes `SLACK_SCREENS` at 2.
    assert!(
        rebuilds <= presses / 10,
        "{rebuilds} window rebuilds over {presses} single-line scrolls — \
         the slack is not absorbing ordinary movement"
    );
}

/// Arrowing onto a large log only previews it (bounded by
/// `PREVIEW_LINES` in `fileview.rs`), so a filter added at that
/// point is evaluated against the truncated slice. `FileView` upgrades
/// itself to a full load the moment it is actually used — inside its own
/// `handle_events`, invisible to `perform` — which rebuilds the textarea
/// and clears its line styles. Without a resync, the view is left
/// unfiltered and the style vector stuck at the stale preview length.
#[test]
fn upgrading_a_truncated_preview_resyncs_styles_without_reloading() {
    let dir = fixture_dir("preview_upgrade_resync");
    // Past PREVIEW_LINES, so the first preview is truncated. The match
    // sits inside the preview too, so it is visible both before and after
    // the upgrade to a full load.
    let body: String = (0..crate::widgets::fileview::PREVIEW_LINES + 100)
        .map(|i| {
            if i == 10 {
                "MATCH line\n".to_string()
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    fs::write(dir.join("big.log"), &body).expect("write fixture");

    let mut app = App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    });

    // The explorer pane previews the log rather than reading the whole
    // 600-line file. The startup argument names a file that does not
    // exist, so the explorer falls back to the first real entry — which
    // is the log — and `Down` holds it there.
    key(&mut app, KeyCode::Down);

    // Add a filter while the view still only holds the preview.
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "MATCH");
    key(&mut app, KeyCode::Enter);

    let preview_styles = view_line_styles(&app);
    // Length asked of the document; alignment asked of the view. Since #7
    // the style vector covers the *window*, so it is the document that says
    // whether the preview was capped, and the buffer that says whether the
    // styles line up with what is drawn.
    assert_eq!(
        app.document.lines().len(),
        crate::widgets::fileview::PREVIEW_LINES,
        "sanity: preview is capped"
    );
    assert_eq!(
        preview_styles.len(),
        view_lines(&app).len(),
        "styles do not line up with the buffer"
    );
    assert!(
        preview_styles[10].is_some(),
        "match line unstyled in the preview"
    );

    // Tab into the file view and press a key: this is exactly what
    // upgrades the truncated preview to a full load inside
    // `FileView::handle_events`. A filter is already defined here, so a
    // bare `Tab` would no longer land on the file view once the filter
    // pane joins the cycle — `focus_file_view` tabs however many times
    // that takes.
    focus_file_view(&mut app);
    key(&mut app, KeyCode::Char('j'));

    let styles = view_line_styles(&app);
    assert_eq!(
        app.document.lines().len(),
        crate::widgets::fileview::PREVIEW_LINES + 100,
        "the preview did not upgrade to a full load"
    );
    assert_eq!(
        styles.len(),
        view_lines(&app).len(),
        "style vector was not resynced to the fully loaded buffer"
    );
    assert!(
        styles[10].is_some(),
        "matching line lost its style after the preview upgraded to a full load"
    );
}

/// `Ctrl-f` must reach the file view's own page-down binding, not the
/// global `f` handler that moves focus to the filter pane.
#[test]
fn ctrl_f_scrolls_the_file_view_instead_of_focusing_the_filter_pane() {
    let body = numbered_lines(100);
    let mut app = app_over_file("ctrl_f_scroll", &body);
    draw(&mut app); // establish the file view's rendered size
    focus_file_view(&mut app);

    let before = view_cursor_row(&app);
    app.handle_event(event::Event::Key(KeyEvent::new(
        KeyCode::Char('f'),
        KeyModifiers::CONTROL,
    )));

    assert!(app.prompt.is_none(), "Ctrl-f opened a filter prompt");
    assert!(
        view_cursor_row(&app) > before,
        "Ctrl-f did not scroll the file view"
    );
}

fn view_line_numbers(app: &App) -> Vec<usize> {
    app.view.textarea().line_numbers().to_vec()
}

// ---- windowed viewport (#7) -----------------------------------------

/// Long enough to be windowed at any plausible pane height, short enough
/// to stay under `PREVIEW_LINES` so nothing here is also testing
/// truncation. Every tenth line is blank, giving `{` and `}` something to
/// find.
fn app_over_long_file(name: &str) -> App<'static> {
    let body: String = (0..LONG_FILE_LINES)
        .map(|i| {
            if i % 10 == 0 {
                "\n".to_string()
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    let mut app = app_over_file(name, &body);
    // Gives the view a real pane height to size its window against;
    // before the first render it assumes `ASSUMED_PANE_HEIGHT`.
    draw(&mut app);
    focus_file_view(&mut app);
    app
}

const LONG_FILE_LINES: usize = 5_000;

/// The buffer `load` seeds is a window, not the file (#151), and every
/// production path replaces it through `apply_view` before the first
/// frame — this pins that a fresh load of a long file still draws from
/// its first line and can reach its last.
#[test]
fn a_fresh_load_of_a_long_file_renders_its_first_line_and_reaches_its_last() {
    let mut app = app_over_long_file("fresh_load_renders");
    draw_tall(&mut app);
    assert!(
        view_lines(&app).iter().any(|line| line == "line 1"),
        "the first screen of a fresh load is not the file's start"
    );

    key(&mut app, KeyCode::Char('G'));
    draw_tall(&mut app);

    assert_eq!(
        cursor_source(&app),
        LONG_FILE_LINES - 1,
        "G did not reach the last line"
    );
    assert!(
        view_lines(&app).iter().any(|line| line == "line 4999"),
        "the window around the last line does not hold it"
    );
}

/// **The structural form of #7's memory acceptance criterion.**
///
/// The win is not measured in bytes — that is allocator- and
/// platform-dependent, and the classic flaky test. What causes the win is
/// directly assertable instead: the buffer holds a window, however long the
/// document is. Before this change the two numbers below were equal, and
/// the file was resident twice.
#[test]
fn the_view_holds_a_window_not_the_whole_document() {
    let mut app = app_over_long_file("window_bounded");

    // `G` forces a re-window against the real pane height, so this is not
    // just measuring the pre-render assumption.
    key(&mut app, KeyCode::Char('G'));

    assert_eq!(
        app.document.visible().len(),
        LONG_FILE_LINES,
        "sanity: the document still holds every line"
    );
    let held = view_lines(&app).len();
    // Against the constant, not a literal: the window grew from three
    // screens to five in #108, and a hard-coded 3 here made that read as a
    // regression in the memory criterion rather than the deliberate
    // widening of the slack that it was.
    let span = crate::widgets::fileview::WINDOW_SCREENS * AREA.height as usize;
    assert!(
        held <= span,
        "the buffer holds {held} lines for a {}-row pane — not a window",
        AREA.height
    );
}

/// `g` means the document's first line, not the first line of whichever
/// window happens to be loaded. Without interception this lands on
/// `window_start`, which looks entirely plausible and is wrong.
#[test]
fn g_jumps_to_the_documents_first_line() {
    let mut app = app_over_long_file("window_g_top");
    key(&mut app, KeyCode::Char('G'));
    assert_ne!(cursor_source(&app), 0, "sanity: moved away from the top");

    key(&mut app, KeyCode::Char('g'));

    assert_eq!(cursor_source(&app), 0);
}

#[test]
fn capital_g_jumps_to_the_documents_last_line() {
    let mut app = app_over_long_file("window_g_bottom");

    key(&mut app, KeyCode::Char('G'));

    assert_eq!(cursor_source(&app), LONG_FILE_LINES - 1);
}

#[test]
fn capital_g_goes_to_the_end_of_the_document_not_the_buffer() {
    // The file is longer than one window, so "the end of what is loaded"
    // and "the end of the document" are different rows.
    let body = (0..500).fold(String::new(), |mut body, i| {
        writeln!(body, "line {i}").unwrap();
        body
    });
    let (mut app, _root) = app_over_project("goto_end", &body);
    app.focus = Focus::View;

    key(&mut app, KeyCode::Char('G'));

    assert_eq!(
        cursor_source(&app),
        499,
        "G must reach the document's last line, not the buffer's (#250)"
    );
}

/// The half of #250 the normalising intercept left open. `Scope::View`
/// normalises a key before resolving it, but an *unresolved* key used to
/// fall through to `FileView::handle_events`, which matches the character
/// with the modifier fields ignored — so `Alt-j` moved the cursor as
/// though the modifier had never been pressed. The explorer and the
/// filter pane have always dropped what their own scope does not
/// resolve; the view does too now.
#[test]
fn an_unbound_modified_key_does_not_move_the_view_cursor() {
    let mut app = app_over_long_file("alt_j_moves_nothing");
    let before = cursor_source(&app);

    alt(&mut app, KeyCode::Char('j'));

    assert_eq!(
        cursor_source(&app),
        before,
        "Alt-j moved the cursor, so an unbound modified key still reaches the widget"
    );
}

/// The second-order cost of the same gap, and the one a user feels on a
/// large file: `FileView::handle_events` promotes a truncated preview on
/// entry, before it matches anything at all. So an unbound modified key
/// did not merely move the cursor — it forced a full load of a file the
/// user had never asked to load.
#[test]
fn an_unbound_modified_key_does_not_promote_a_truncated_preview() {
    let dir = fixture_dir("alt_j_keeps_the_preview");
    // Past PREVIEW_LINES, so the first preview of this file is truncated.
    let body = numbered_lines(crate::widgets::fileview::PREVIEW_LINES + 100);
    fs::write(dir.join("big.log"), &body).expect("write fixture");

    let mut app = App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    });
    // The startup argument names a file that does not exist, so the
    // explorer falls back to the first real entry — the log — and
    // previews it rather than reading the whole thing.
    key(&mut app, KeyCode::Down);
    focus_file_view(&mut app);
    assert!(app.view.is_truncated(), "sanity: the preview is truncated");

    alt(&mut app, KeyCode::Char('j'));

    assert!(
        app.view.is_truncated(),
        "Alt-j promoted the preview to a full load the user never asked for"
    );
}

/// The other half of dropping an unresolved key: it must not cost a key
/// that works. `j` resolves in `Scope::View`. `]` does not — it is the
/// one key `FileView::handle_events` acts on that has no `Scope::View`
/// row at all, so it is the most exposed to this change; it resolves in
/// `Scope::Global`, which is checked first, and reaches the widget
/// through `GlobalPageDown`'s rebuilt key rather than through the
/// dropped fallthrough.
#[test]
fn the_keys_the_view_acts_on_still_reach_the_widget() {
    let area = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 36,
    };
    let mut buf = Buffer::empty(area);
    let mut app = app_over_file("view_keys_still_work", &numbered_lines(7_000));
    (&mut app).render(area, &mut buf);
    key(&mut app, KeyCode::Char('t'));
    (&mut app).render(area, &mut buf);

    key(&mut app, KeyCode::Char('j'));
    assert_eq!(cursor_source(&app), 1, "j no longer moves the cursor");

    let top = |app: &App| -> usize {
        let (scroll, _) = app.view.textarea().scroll_top();
        app.view.window_start() + scroll as usize
    };
    let before = top(&app);

    key(&mut app, KeyCode::Char(']'));
    (&mut app).render(area, &mut buf);

    assert!(
        top(&app) > before,
        "] no longer pages the view down (Scope::Global -> forward_to_view)"
    );
}

/// A paragraph move can travel further than the window. Resolved against
/// the document, `}` finds the next blank line; left to the widget it would
/// stop at the buffer's edge.
#[test]
fn brace_moves_by_paragraph_across_the_whole_document() {
    let mut app = app_over_long_file("window_paragraph");

    key(&mut app, KeyCode::Char('}'));
    // Line 0 is blank and the cursor starts there, so the next blank below
    // is line 10.
    assert_eq!(cursor_source(&app), 10);

    key(&mut app, KeyCode::Char('}'));
    assert_eq!(cursor_source(&app), 20);

    key(&mut app, KeyCode::Char('{'));
    assert_eq!(cursor_source(&app), 10);
}

/// The off-by-`window_start` failure this change is most exposed to.
/// `textarea.cursor().0` indexes the buffer; the source line is
/// `window_start` further down. Reading it untranslated yields a line
/// number that is wrong but entirely believable.
#[test]
fn the_cursor_reports_its_source_line_from_inside_a_window() {
    let mut app = app_over_long_file("window_cursor_source");

    key(&mut app, KeyCode::Char('G'));

    let view = &app.view;
    assert!(
        view.window_start() > 0,
        "sanity: the end of the file must be a moved window"
    );
    assert_eq!(cursor_source(&app), LONG_FILE_LINES - 1);
}

/// A window starting at visible row N must number its gutter from N, not
/// from 1. This is why the numbers override became unconditional.
#[test]
fn the_gutter_numbers_a_window_by_its_source_lines() {
    let mut app = app_over_long_file("window_gutter");

    key(&mut app, KeyCode::Char('G'));

    let numbers = view_line_numbers(&app);
    assert_eq!(
        numbers.last().copied(),
        Some(LONG_FILE_LINES - 1),
        "the last row of the last window must be the file's last line"
    );
    assert!(
        numbers[0] > 0,
        "a window at the end of the file must not renumber from the top"
    );
}

/// Paging repeatedly must walk the document, not stall at a window edge.
/// This is the case the middle-third rule exists for: with a smaller margin
/// the second page runs into the buffer's end and is silently truncated.
#[test]
fn paging_down_repeatedly_walks_past_window_boundaries() {
    let mut app = app_over_long_file("window_paging");
    let mut last = cursor_source(&app);

    for page in 0..40 {
        key(&mut app, KeyCode::PageDown);
        let now = cursor_source(&app);
        assert!(now >= last, "page {page} moved backwards: {last} -> {now}");
        last = now;
    }

    assert!(
        last > 3 * AREA.height as usize,
        "paging never left the first window (reached line {last})"
    );
}

/// End to end through the real render path, against a window that has
/// *moved*. The scroll machinery is the risky part of this change — the
/// pending-scroll priming render, the viewport reset by `set_lines` — and
/// the other tests here read the buffer rather than the screen. This one
/// checks that what is actually painted is the end of the file.
#[test]
fn a_moved_window_renders_the_lines_it_holds() {
    let mut app = app_over_long_file("window_render");

    key(&mut app, KeyCode::Char('G'));
    let screen = rendered(&mut app);

    assert!(
        screen.contains(&format!("line {}", LONG_FILE_LINES - 1)),
        "the last line of the file was not painted:\n{screen}"
    );
    assert!(
        !screen.contains("line 1 "),
        "the top of the file is still on screen after jumping to the end:\n{screen}"
    );
}

/// `n` targets a row in the visible set, which a windowed buffer does not
/// contain. Handing that row straight to the textarea clamps it to the
/// buffer's last line — silently, and nowhere near the match.
#[test]
fn n_reaches_a_match_far_outside_the_window() {
    let mut app = app_over_long_file("window_n_far");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "line 4321$");
    key(&mut app, KeyCode::Enter);
    focus_file_view(&mut app);

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(cursor_source(&app), 4321);
}

#[test]
fn an_excluding_filter_removes_its_lines_from_the_view() {
    let mut app = app_over_file("exclude_view", "alpha\nnoise\ngamma\n");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "noise");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        view_lines(&app),
        vec!["alpha".to_string(), "gamma".to_string()]
    );
}

/// Which rows the gutter is currently marking as ending a group.
fn view_group_ends(app: &App) -> Vec<bool> {
    app.view
        .textarea()
        .line_number_styles()
        .iter()
        .map(Option::is_some)
        .collect()
}

/// Issue #2. Hiding butts groups of matches against each other; the mark
/// on the last row of a group is what says the file did not run
/// continuously from one to the next.
#[test]
fn hiding_marks_the_last_row_of_each_group() {
    let mut app = app_over_file(
        "gap_marks",
        "beta 1\nbeta 2\nalpha\nbeta 3\nbeta 4\ntrailing\n",
    );
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    ctrl(&mut app, KeyCode::Char('h'));

    assert_eq!(view_lines(&app).len(), 4, "the gap was not hidden");
    assert_eq!(view_group_ends(&app), vec![false, true, false, true]);
}

/// With nothing hidden there are no gaps, so nothing may be marked — the
/// mark has to mean something, and a file shown whole has no boundaries
/// to draw.
#[test]
fn nothing_is_marked_while_the_whole_file_is_shown() {
    let mut app = app_over_file("gap_marks_off", "beta\nalpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(view_lines(&app).len(), 3, "sanity: nothing is hidden");
    assert!(
        view_group_ends(&app).iter().all(|end| !end),
        "a gap was marked with no lines hidden"
    );
}

/// The marks are indexed by buffer row, so they must be re-derived
/// whenever the visible set changes — a stale set would underline rows
/// that are no longer where a group ends.
#[test]
fn lifting_the_hiding_clears_the_marks() {
    let mut app = app_over_file("gap_marks_restore", "beta\nalpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    ctrl(&mut app, KeyCode::Char('h'));
    assert!(
        view_group_ends(&app).iter().any(|&end| end),
        "sanity: hiding marked a gap"
    );

    ctrl(&mut app, KeyCode::Char('h'));

    assert!(
        view_group_ends(&app).iter().all(|end| !end),
        "the marks survived the file coming back whole"
    );
}

/// The gutter keeps the original numbering, so a hidden line leaves a gap.
#[test]
fn the_gutter_shows_source_line_numbers_when_lines_are_hidden() {
    let mut app = app_over_file("exclude_gutter", "alpha\nnoise\ngamma\n");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "noise");
    key(&mut app, KeyCode::Enter);

    // 0-based source indices: rows 0 and 2 render as 1 and 3.
    assert_eq!(view_line_numbers(&app), vec![0, 2]);
}

#[test]
fn styles_still_line_up_with_the_rebuilt_buffer() {
    let mut app = app_over_file("exclude_styles", "alpha\nnoise\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "noise");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(view_line_styles(&app).len(), view_lines(&app).len());
}

/// With nothing excluded the gutter numbers the file straight through.
///
/// This used to assert the override was *absent*, letting the fork number
/// the buffer 1..N itself. #7 made the override unconditional, because a
/// windowed buffer starting at visible row 1,000 would otherwise be
/// numbered 1, 2, 3. The invariant worth protecting was never "no
/// override" — it was "the numbers are the file's own", which is what this
/// now checks directly.
#[test]
fn without_hiding_the_gutter_numbers_the_file_straight_through() {
    let mut app = app_over_file("no_hiding", "alpha\nbeta\n");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(view_lines(&app).len(), 2);
    assert_eq!(
        view_line_numbers(&app),
        vec![0, 1],
        "the gutter must number an unhidden file straight through"
    );
}

/// Lifting the hiding must restore the whole buffer. Leaving a stale subset
/// behind is worse than never hiding: the remaining rows would renumber
/// from 1 and claim to be the whole file.
#[test]
fn disabling_an_excluding_filter_restores_the_hidden_lines() {
    let mut app = app_over_file("exclude_restore", "alpha\nnoise\ngamma\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "noise");
    key(&mut app, KeyCode::Enter);
    assert_eq!(view_lines(&app).len(), 2, "the line was not hidden");

    key(&mut app, KeyCode::Char('!'));

    assert_eq!(
        view_lines(&app),
        vec![
            "alpha".to_string(),
            "noise".to_string(),
            "gamma".to_string()
        ],
        "the hidden line did not come back"
    );
    assert_eq!(
        view_line_styles(&app).len(),
        view_lines(&app).len(),
        "styles no longer line up with the buffer"
    );
    assert_eq!(
        view_line_numbers(&app),
        vec![0, 1, 2],
        "the gutter must renumber back to the file's own lines"
    );
}

/// The row (if any) whose rendered text contains `needle`.
fn row_containing(buf: &Buffer, needle: &str) -> Option<u16> {
    (0..buf.area.height).find(|&y| {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .contains(needle)
    })
}

/// A regression against the previous phase: `restyle` (Phase 2a) only
/// set styles and never touched the buffer, so a filter change left the
/// view exactly where it was. `refresh_view` rebuilding unconditionally
/// broke that, because rebuilding resets the textarea's viewport —
/// so any filter change re-anchored the scroll on the next render, even
/// one that changed nothing about what is on screen.
///
/// Demonstrated exactly as found: the cursor is scrolled so its line
/// sits at the top of the pane, then a filter that matches nothing is
/// added. Nothing about the visible rows changed, so the view must not
/// move.
#[test]
fn a_filter_matching_nothing_does_not_move_the_viewport() {
    // 13 rows, not 12: one goes to the permanent status row, leaving the
    // 12-row bordered pane this test's page arithmetic below depends on.
    let area = Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 13,
    };
    let body = numbered_lines(200);
    let mut app = app_over_file("no_op_filter_viewport", &body);
    focus_file_view(&mut app);

    // Render once so the textarea knows its viewport size (10 rows of
    // content inside the 12-row, bordered pane), then page down nine
    // screens so the cursor's line lands exactly at the top of the pane.
    let mut buf = Buffer::empty(area);
    (&mut app).render(area, &mut buf);
    for _ in 0..9 {
        app.handle_event(event::Event::Key(KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL,
        )));
    }

    let mut buf = Buffer::empty(area);
    (&mut app).render(area, &mut buf);
    let before = row_containing(&buf, "line 90").expect("line 90 should be on screen");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "zzz_never_matches");
    key(&mut app, KeyCode::Enter);

    let mut buf = Buffer::empty(area);
    (&mut app).render(area, &mut buf);
    let after = row_containing(&buf, "line 90").expect("line 90 should still be on screen");

    assert_eq!(
        before, after,
        "adding a filter that matched nothing moved the viewport"
    );
}

/// Toggling a filter changes the visible set, so the buffer is rebuilt —
/// but the line under the cursor must stay on the same screen row rather
/// than the view re-anchoring beneath it.
///
/// An *excluding* filter is used deliberately: an including filter in
/// the default (`Dimmed`) mode never changes `visible` at all — it only
/// changes styling — so `!` would trigger no rebuild and the test would
/// pass trivially, before any fix exists. Excluded lines are dropped
/// from `visible` in every mode, so toggling one genuinely forces the
/// rebuild this test is about.
#[test]
fn toggling_a_filter_leaves_the_cursor_on_the_same_screen_row() {
    let body = numbered_lines(200);
    let mut app = app_over_file("scroll_hold", &body);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "line 1[5-9][0-9]"); // excludes 150..=199, well below the cursor
    key(&mut app, KeyCode::Enter);
    draw(&mut app);

    // Put the cursor well down the file. Moving down one line at a time
    // like this pins it to the lowest screen row the scroll margin
    // allows — the viewport scrolls to keep `SCROLL_MARGIN` rows of
    // context below it, landing it on the margin's edge every time.
    for _ in 0..120 {
        focus_file_view(&mut app);
        key(&mut app, KeyCode::Char('j'));
    }
    draw(&mut app);
    let pinned_row = cursor_screen_row(&app);
    // The pane's last text row — the one a reset viewport re-anchors to.
    let last_row = app.view.window_height() - 3;

    // Pull it back off that row: the pane's last row is exactly where a
    // reset viewport would re-anchor the cursor after a rebuild, so
    // parking there would make the bug and the fix indistinguishable.
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

    key(&mut app, KeyCode::Char('!'));
    draw(&mut app);

    // A structural guard on the rebuild itself: if a future change (e.g.
    // swapping `x` for `i`) stopped the buffer from actually changing
    // size, the assertions below would pass trivially whether or not the
    // fix exists — the same failure mode correction (a) already covers,
    // pinned here so it can't quietly regress.
    assert_ne!(
        view_lines(&app).len(),
        before_len,
        "the buffer did not change size, so `!` did not force a rebuild \
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

/// Companion to the test above: there the excluded block sits *below*
/// the cursor, so the cursor's own absolute buffer row never moves — only
/// the viewport reset is exercised. Here the excluded block sits
/// *above* it, so restoring the excluded lines shifts the cursor's
/// buffer row itself (lines appear above it), which is the case
/// `scroll_cursor_to_row`'s `desired_top` / `saturating_sub` clamping
/// exists for. The screen row must still hold.
#[test]
fn toggling_a_filter_above_the_cursor_also_leaves_the_cursor_on_the_same_screen_row() {
    let body = numbered_lines(200);
    let mut app = app_over_file("scroll_hold_above", &body);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    // Excludes source lines 0..=49, anchored so e.g. "line 100" is not
    // also matched as a substring of "line 10".
    typed(&mut app, "^line ([0-9]|[1-4][0-9])$");
    key(&mut app, KeyCode::Enter);
    draw(&mut app);

    // The excluded block snaps the initial cursor forward to line 50
    // (the nearest remaining visible line), then this drives it further
    // down — well clear of the excluded block either way.
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

    key(&mut app, KeyCode::Char('!'));
    draw(&mut app);

    assert_ne!(
        view_lines(&app).len(),
        before_len,
        "the buffer did not change size, so `!` did not force a rebuild \
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

/// The two tests above drive this same screen-row criterion through `!`
/// and (below) `H` — both global bindings — but never through the
/// filter pane's own `space` key, even though the spec calls pane
/// toggling "the dominant interaction" once filters exist. Without this,
/// a future `handle_filter_key` change that bypassed `refresh_view` —
/// patching the cached verdicts in place instead of re-evaluating, say
/// — would pass every screen-row test in this file while breaking the
/// one interaction the pane exists for.
#[test]
fn toggling_a_filter_from_the_pane_leaves_the_cursor_on_the_same_screen_row() {
    let body = numbered_lines(200);
    let mut app = app_over_file("pane_scroll_hold", &body);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "line 1[5-9][0-9]"); // excludes 150..=199, well below the cursor
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

    // The pane's own key, not `!` — this is the criterion this test adds.
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Enter);
    draw(&mut app);

    assert_ne!(
        view_lines(&app).len(),
        before_len,
        "the buffer did not change size, so the pane's toggle did not \
         force a rebuild here — this test would pass whether or not the \
         fix exists"
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
