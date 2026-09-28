use super::*;

/// The pattern the file view is highlighting on `needle`'s row: the
/// style of every cell `needle` occupies in a fresh frame. `None` when
/// no row shows it.
fn styles_of(app: &mut App, needle: &str) -> Option<Vec<Style>> {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    (0..AREA.height).find_map(|y| {
        let row: String = (0..AREA.width).map(|x| buf[(x, y)].symbol()).collect();
        let start = row.find(needle)?;
        let start = u16::try_from(row[..start].chars().count()).ok()?;
        Some(
            (start..start + u16::try_from(needle.chars().count()).ok()?)
                .map(|x| buf[(x, y)].style())
                .collect(),
        )
    })
}

// ---- the explorer's filename search moves as you type (#272) --------

/// Each keystroke in the explorer's prompt moves the selection before
/// Enter, and each re-runs from the origin row rather than from the
/// entry the last keystroke reached: `b2` then Backspace lands on
/// `b1.log`, the first `b` after the origin, not on `b3.log`, the first
/// `b` after `b2.log`.
#[test]
fn typing_in_the_explorer_moves_the_selection_and_rescans_from_the_origin() {
    let mut app = app_over("type_explorer", &["a.log", "b1.log", "b2.log", "b3.log"]);
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(selected_name(&app), "a.log", "sanity: the origin");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "b2");
    assert!(app.prompt.is_some(), "sanity: the prompt is open");
    assert_eq!(selected_name(&app), "b2.log", "did not move before Enter");
    assert!(
        app.search.is_none(),
        "a filename probe became the file search"
    );

    key(&mut app, KeyCode::Backspace);
    assert_eq!(
        selected_name(&app),
        "b1.log",
        "did not re-run from the origin row"
    );

    typed(&mut app, "3");
    assert_eq!(selected_name(&app), "b3.log");

    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none(), "Enter did not close the prompt");
    assert_eq!(selected_name(&app), "b3.log", "Enter moved the selection");
    assert!(
        app.explorer.has_search(),
        "Enter did not set the filename search"
    );
}

/// The origin row's own name is considered first, as the file search
/// considers the origin line first: a pattern the origin matches stays
/// put rather than stepping to the next match.
#[test]
fn a_filename_pattern_the_origin_matches_stays_on_the_origin() {
    let mut app = app_over("type_explorer_own", &["alpha.log", "alps.log"]);
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(selected_name(&app), "alpha.log", "sanity: the origin");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "al");

    assert_eq!(selected_name(&app), "alpha.log", "stepped past the origin");
}

/// The view pane shows the file under the moving selection, as it does
/// for a `j` onto that row, and Esc puts back both the selected row and
/// the preview it had.
#[test]
fn the_view_follows_the_explorer_selection_while_typing_and_esc_restores_both() {
    let mut app = app_over_files(
        "type_explorer_preview",
        &[
            ("alpha.log", "ALPHA MARKER\n"),
            ("gamma.log", "GAMMA MARKER\n"),
        ],
    );
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(selected_name(&app), "alpha.log", "sanity: the origin");
    draw(&mut app);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "gamma");
    let frame = rendered(&mut app);
    assert!(
        frame.contains("GAMMA MARKER"),
        "the view did not follow the selection while typing:\n{frame}"
    );

    key(&mut app, KeyCode::Esc);
    assert!(app.prompt.is_none(), "Esc did not close the prompt");
    assert_eq!(
        selected_name(&app),
        "alpha.log",
        "Esc did not restore the row"
    );
    assert!(!app.explorer.has_search(), "Esc left a filename search set");
    let frame = rendered(&mut app);
    assert!(
        frame.contains("ALPHA MARKER") && !frame.contains("GAMMA MARKER"),
        "Esc did not restore the preview:\n{frame}"
    );
}

/// Esc puts back the filename search that was set before `/` opened, so
/// `n` repeats it, and a probe that stays on the origin row does not
/// reload the pane on the way out.
#[test]
fn esc_in_the_explorer_prompt_restores_the_previous_filename_search() {
    let mut app = app_over("type_explorer_prev", &["a.log", "b1.log", "b2.log"]);
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "b");
    key(&mut app, KeyCode::Enter);
    assert_eq!(selected_name(&app), "b1.log", "sanity: the search is set");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zzz");
    assert_eq!(selected_name(&app), "b1.log", "a dead end moved the row");
    key(&mut app, KeyCode::Esc);

    assert!(app.explorer.has_search(), "Esc dropped the previous search");
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(
        selected_name(&app),
        "b2.log",
        "n did not repeat the search Esc put back"
    );
}

/// Enter sets the filename search as before, so `n` and `N` repeat it
/// from the row it reached.
#[test]
fn n_and_big_n_repeat_a_typed_filename_search() {
    let mut app = app_over("type_explorer_repeat", &["a.log", "b1.log", "b2.log"]);
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "b");
    key(&mut app, KeyCode::Enter);
    assert_eq!(selected_name(&app), "b1.log");

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(selected_name(&app), "b2.log", "n did not step");
    key(&mut app, KeyCode::Char('N'));
    assert_eq!(selected_name(&app), "b1.log", "N did not step back");
}

/// A pattern no name matches is silent while typed, with the selection
/// on the origin row; Enter on it reports the dead end (#243) and leaves
/// the row where it is.
#[test]
fn a_filename_pattern_with_no_match_sits_at_the_origin_and_enter_says_so() {
    let mut app = app_over("type_explorer_dead_end", &["alpha.log", "beta.log"]);
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ERROR");
    assert_eq!(selected_name(&app), "alpha.log", "a dead end moved the row");
    assert_eq!(status(&app), None, "a dead end was reported while typing");

    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none(), "Enter did not close the prompt");
    assert_eq!(selected_name(&app), "alpha.log", "Enter moved the row");
    assert_eq!(status(&app), Some("no filenames match \"ERROR\""));
}

/// A half-typed regex is silent in the explorer too: no error, the
/// selection on the origin row. Enter on it reports `E486` and keeps the
/// prompt open.
#[test]
fn a_half_typed_invalid_filename_regex_is_silent_in_the_explorer() {
    let mut app = app_over("type_explorer_invalid", &["alpha.log", "beta.log"]);
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    assert_eq!(selected_name(&app), "beta.log", "sanity: moved");
    typed(&mut app, "[");
    assert_eq!(
        selected_name(&app),
        "alpha.log",
        "an invalid pattern did not return to the origin row"
    );
    assert!(
        !prompt_line(&mut app).contains("E486"),
        "an error showed while typing: {}",
        prompt_line(&mut app)
    );
    assert_eq!(status(&app), None);

    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_some(), "Enter closed the prompt");
    assert!(
        prompt_line(&mut app).contains("E486"),
        "no error shown: {}",
        prompt_line(&mut app)
    );
    assert_eq!(selected_name(&app), "alpha.log");
}

/// Enter on an empty explorer prompt cancels, as in the file view: it
/// does not step to the next entry as a search for `""` would.
#[test]
fn enter_on_an_empty_explorer_prompt_cancels() {
    let mut app = app_over("type_explorer_empty", &["alpha.log", "beta.log"]);
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_none(), "the prompt stayed open");
    assert_eq!(selected_name(&app), "alpha.log", "an empty search moved");
    assert!(!app.explorer.has_search(), "an empty search was set");
}

/// The cursor lands on the column of the first occurrence, so a long
/// line does not hide where the hit is.
#[test]
fn the_search_lands_on_the_first_occurrence_column() {
    let mut app = app_over_file("slash_column", "xx beta beta\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.view.cursor_col(), 3);
}

/// A search with no hit in the file says so and moves nothing.
#[test]
fn a_search_with_no_hit_says_so_and_stays_put() {
    let mut app = app_over_file("slash_no_hit", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zzz");
    key(&mut app, KeyCode::Enter);

    assert_eq!(cursor_source(&app), 1, "the cursor moved");
    assert_eq!(status(&app), Some("no hit for /zzz"));
    assert_eq!(search_text(&app), "zzz", "the pattern is still set for n");
}

/// In hide mode the search looks only at the visible lines: a hidden
/// line is never a hit, and the search pulls nothing back in. The
/// visible lines, the gutter numbers and the gaps are what they were.
#[test]
fn in_hide_mode_a_search_finds_no_hidden_line_and_changes_nothing() {
    let mut app = app_over_file("slash_hidden", "alpha\nbeta\ngamma\nbeta again\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha|gamma").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.document.visible(), &[0, 2], "sanity: hiding");
    // Everything but the status row, which gains the search badge.
    let panes = |app: &mut App| {
        let frame = screen(app);
        frame
            .rsplit_once('\n')
            .map(|(panes, _)| panes.to_owned())
            .unwrap_or(frame)
    };
    let before = panes(&mut app);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        app.document.visible(),
        &[0, 2],
        "the search widened the view"
    );
    assert_eq!(cursor_source(&app), 0, "the cursor moved to a hidden line");
    assert_eq!(status(&app), Some("no hit for /beta"));
    assert_eq!(
        panes(&mut app),
        before,
        "the panes changed under the search"
    );
}

/// In dim mode a dimmed line is visible, so it is a hit.
#[test]
fn in_dim_mode_a_search_finds_a_dimmed_line() {
    let mut app = app_over_file("slash_dimmed", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.refresh_view();
    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: dimming");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(cursor_source(&app), 1);
    assert_eq!(
        app.document.verdicts()[1],
        Verdict::Unmatched,
        "the hit line is still dimmed: the search is not a filter"
    );
}

/// An excluded line is gone in both modes, so it is never a hit.
#[test]
fn an_excluded_line_is_never_a_hit() {
    let mut app = app_over_file("slash_excluded", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add_excluding("beta").expect("valid pattern");
    app.refresh_view();

    for mode in [Mode::Dimmed, Mode::FilteredOnly] {
        app.set_mode(mode);
        app.refresh_view();
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        key(&mut app, KeyCode::Enter);

        assert_eq!(
            cursor_source(&app),
            0,
            "{mode:?}: moved to an excluded line"
        );
        assert_eq!(status(&app), Some("no hit for /beta"), "{mode:?}");
    }
}

/// A hit line keeps the style the filters gave it; only the hit text
/// is painted, black on yellow, and that stays on top of everything.
#[test]
fn a_hit_line_keeps_its_filter_colour_and_only_the_hit_is_highlighted() {
    let mut app = app_over_file("slash_styles", "alpha beta\nplain\nother\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.refresh_view();
    let colour = app
        .filters
        .style_for(Verdict::Included(0))
        .expect("filter 0 has a colour")
        .fg;
    let dim = app.filters.dim_style().fg;

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    // Off the rows under test: the cursor's cell and line have styles of
    // their own.
    key(&mut app, KeyCode::Char('G'));

    let alpha = styles_of(&mut app, "alpha").expect("alpha on screen");
    assert!(
        alpha
            .iter()
            .all(|style| style.fg == colour && style.bg != Some(Color::Yellow)),
        "the filter colour was lost on the hit line: {alpha:?}"
    );
    let beta = styles_of(&mut app, "beta").expect("beta on screen");
    assert!(
        beta.iter()
            .all(|style| style.bg == Some(Color::Yellow) && style.fg == Some(Color::Black)),
        "the hit text is not highlighted: {beta:?}"
    );
    let plain = styles_of(&mut app, "plain").expect("plain on screen");
    assert!(
        plain
            .iter()
            .all(|style| style.fg == dim && style.bg != Some(Color::Yellow)),
        "a search changed how an unmatched line is drawn: {plain:?}"
    );
}

/// `n` and `N` step hit lines while a search is set: one stop per line,
/// wrapping within the file and saying so.
#[test]
fn n_steps_hit_lines_one_stop_per_line_and_wraps_within_the_file() {
    let mut app = app_over_file("n_hits", "hit hit\nplain\nhit\nplain\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "hit");
    key(&mut app, KeyCode::Enter);
    assert_eq!(cursor_source(&app), 0, "sanity");

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 2, "stopped more than once on line 0");
    assert_eq!(status(&app), None);

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 0, "did not wrap");
    assert_eq!(status(&app), Some(WRAPPED_TO_TOP));

    key(&mut app, KeyCode::Char('N'));
    assert_eq!(cursor_source(&app), 2, "N did not wrap backwards");
    assert_eq!(status(&app), Some(WRAPPED_TO_BOTTOM));
}

/// With a search set, `n` steps hits and not the filters' interesting
/// lines: the search owns the motion the user just made.
#[test]
fn n_steps_hits_and_not_interesting_lines_while_a_search_is_set() {
    let mut app = app_over_file(
        "n_hits_only",
        "alpha\nERROR one\nbeta\ntimeout two\ngamma\n",
    );
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("ERROR").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "timeout");
    key(&mut app, KeyCode::Enter);
    assert_eq!(cursor_source(&app), 3, "sanity");

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 3, "n stepped to the filter's line");
    assert_eq!(status(&app), Some(WRAPPED_TO_TOP));

    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(
        cursor_source(&app),
        1,
        "with no search, n steps interesting lines"
    );
}

/// `n` with a search and no hit reports it and stays put.
#[test]
fn n_with_a_search_and_no_hit_says_so_and_stays_put() {
    let mut app = app_over_file("n_no_hit", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zzz");
    key(&mut app, KeyCode::Enter);
    app.status_message = None;

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(cursor_source(&app), 1, "n moved");
    assert_eq!(status(&app), Some("no hit for /zzz"));
}

/// A search `n` wraps within the file and never crosses to another,
/// even when the filters would have carried `n` there.
#[test]
fn n_with_a_search_does_not_cross_files() {
    let mut app = app_over_files(
        "n_search_stays",
        &[("alpha.log", "hit\nplain\n"), ("zebra.log", "hit\n")],
    );
    open_file(&mut app, 0);
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "hit");
    key(&mut app, KeyCode::Enter);
    let before = app.explorer.selected_entry();

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(
        app.explorer.selected_entry(),
        before,
        "n crossed to another file"
    );
    assert_eq!(cursor_source(&app), 0);
    assert_eq!(status(&app), Some(WRAPPED_TO_TOP));
}

/// The pattern survives loading another file, so `n` finds its hits in
/// the next log; the load itself scans nothing and moves nothing.
#[test]
fn the_search_survives_a_file_load_and_the_load_moves_nothing() {
    let mut app = app_over_files(
        "slash_survives",
        &[("alpha.log", "x\n"), ("beta.log", "plain\nplain\nx\n")],
    );
    open_file(&mut app, 0);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "x");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('j'));

    assert_eq!(
        search_text(&app),
        "x",
        "the search did not outlive the load"
    );
    assert_eq!(cursor_source(&app), 0, "the load moved the cursor");
    assert!(
        app.file_view_highlight().is_some(),
        "the highlight did not survive the load"
    );

    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(
        cursor_source(&app),
        2,
        "n did not find the hit in the next file"
    );
}

/// `u` with a search and no include filter does nothing: a search alone
/// cannot hide the file, because it is not something to hide against.
#[test]
fn u_with_a_search_and_no_include_filter_does_nothing() {
    let mut app = app_over_file("u_search_only", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('u'));

    assert_eq!(
        app.document.visible(),
        &[0, 1, 2],
        "a bare search hid lines"
    );
}

/// `!` toggles the filters and leaves the search alone: the pattern and
/// its highlight stay through both presses.
#[test]
fn bang_leaves_the_search_alone() {
    let mut app = app_over_file("bang_search", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    for press in 1..=2 {
        key(&mut app, KeyCode::Char('!'));
        assert_eq!(
            search_text(&app),
            "beta",
            "press {press} dropped the search"
        );
        assert!(
            app.file_view_highlight().is_some(),
            "press {press} dropped the highlight"
        );
    }
}

/// The explorer marks files from filters only. With nothing but a
/// search there is nothing to scan for, which is what `matcher` being
/// `None` means to the explorer.
#[test]
fn the_explorer_marks_files_from_filters_only() {
    let mut app = app_over_file("explorer_marks_search", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert!(
        app.filters.matcher().is_none(),
        "a search started a file scan"
    );
}

/// `--emit lines` is what the filters chose. A search changes none of it.
#[test]
fn emit_lines_output_is_unchanged_by_a_search() {
    let mut app = app_over_file("emit_search", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha|gamma").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('u'));
    let before = app.collect(emit::Emit::Lines);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.collect(emit::Emit::Lines), before);
}

/// The filter pane has no search row; the status row shows the pattern.
#[test]
fn the_filter_pane_has_no_search_row_and_the_status_row_shows_the_pattern() {
    let mut app = app_over_file("status_search", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    let rows = widgets::filterlist::rows(&app.filters);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        widgets::filterlist::rows(&app.filters),
        rows,
        "the pane gained a row"
    );
    let status = status_line(&mut app);
    assert!(
        status.contains("/beta"),
        "the status row does not show the pattern: {status}"
    );
    assert!(
        !status.contains("1 filter"),
        "a search is counted as a filter: {status}"
    );

    key(&mut app, KeyCode::Esc);
    assert!(
        !status_line(&mut app).contains("/beta"),
        "the badge outlived the search"
    );
}

#[test]
fn an_invalid_search_pattern_leaves_the_prompt_open() {
    let mut app = app_over_file("slash_bad", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "[");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_some(), "prompt closed on an invalid pattern");
    assert!(app.search.is_none(), "a rejected pattern became the search");
}

#[test]
fn escape_clears_the_search() {
    let mut app = app_over_file("esc_clears", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert!(app.search.is_some(), "sanity: search set");

    key(&mut app, KeyCode::Esc);

    assert!(app.search.is_none());
}

/// An open prompt still wins: Esc there cancels the prompt, as it always has,
/// rather than reaching past it to delete an established search.
#[test]
fn escape_in_an_open_prompt_still_cancels_the_prompt() {
    let mut app = app_over_file("esc_prompt", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "gamma");

    key(&mut app, KeyCode::Esc);

    assert!(app.prompt.is_none(), "the prompt did not close");
    assert!(app.search.is_some(), "Esc reached past the prompt");
}

#[test]
fn escape_with_no_search_does_nothing() {
    let mut app = app_over_file("esc_noop", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Esc);

    assert_eq!(app.filters.len(), 1, "Esc touched the numbered filters");
}

/// `refresh_view` runs `Document::evaluate`, which is O(lines × filters)
/// — not free — and Esc is a key people tap out of habit, so clearing an
/// empty search must not pay for it. Nothing in the filter set itself
/// tells "refreshed and found nothing new" apart from "never refreshed",
/// so this reaches for `apply_view`'s own tell instead: it only
/// overwrites `last_generation` when the key it just computed differs
/// from what is already there. Seeding a value the document can never
/// reach means a leftover mismatch after Esc is direct evidence that
/// `refresh_view` never ran.
#[test]
fn escape_with_no_search_does_not_refresh() {
    let mut app = app_over_file("esc_no_refresh", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    app.last_generation = Some(u64::MAX);

    key(&mut app, KeyCode::Esc);

    assert_eq!(
        app.last_generation,
        Some(u64::MAX),
        "Esc refreshed the view with nothing to clear"
    );
}

/// Probe, keep, probe again — a filter set assembled without retyping a
/// regex that was hard to get right. `p` is the bridge from search to
/// filter (ADR 0001): the pattern crosses, the search is cleared, and
/// the filter's colour replaces the highlight.
#[test]
fn p_promotes_the_search_into_a_filter_and_clears_it() {
    let mut app = app_over_file("p_promote", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('p'));

    assert_eq!(app.filters.len(), 1, "the search did not become a filter");
    assert!(app.search.is_none(), "the search was not cleared");
    assert!(
        app.file_view_highlight().is_none(),
        "the highlight outlived p"
    );
    assert_eq!(app.document.verdicts()[1], Verdict::Included(0));
    // Off the row under test: the cursor's cell has a style of its own.
    key(&mut app, KeyCode::Char('k'));
    let colour = app.filters.style_for(Verdict::Included(0)).unwrap().fg;
    let beta = styles_of(&mut app, "beta").expect("beta on screen");
    assert!(
        beta.iter()
            .all(|style| style.fg == colour && style.bg != Some(Color::Yellow)),
        "the filter colour did not replace the highlight: {beta:?}"
    );
}

/// `/foo` `p` `u`: the old `/foo` `u` result, one key further away.
#[test]
fn slash_p_u_collapses_to_the_pattern_lines() {
    let mut app = app_over_file("p_then_u", "alpha\nbeta\ngamma\nbeta again\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('p'));
    key(&mut app, KeyCode::Char('u'));

    assert_eq!(app.document.visible(), &[1, 3]);
}

#[test]
fn two_probes_promote_into_two_filters() {
    let mut app = app_over_file("p_twice", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    for pattern in ["beta", "gamma"] {
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('p'));
    }

    assert_eq!(app.filters.len(), 2);
    // The verdicts themselves are what depend on `p`'s `refresh_view`:
    // a search changes no verdict, and only the re-evaluate after the
    // promotion gives the line its `Verdict::Included`.
    assert_eq!(
        app.document.verdicts()[1],
        Verdict::Included(0),
        "beta's verdict was not updated after being promoted"
    );
    assert_eq!(
        app.document.verdicts()[2],
        Verdict::Included(1),
        "gamma's verdict was not updated after being promoted"
    );
}

#[test]
fn p_with_no_search_does_nothing() {
    let mut app = app_over_file("p_noop", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('p'));

    assert!(app.filters.is_empty());
}

/// Same reasoning as `ctrl_modified_letters_still_reach_the_file_view`:
/// a modified `p` must fall through rather than be swallowed by the
/// promote binding, which only claims the bare, unmodified key.
#[test]
fn ctrl_p_does_not_promote_the_search() {
    let mut app = app_over_file("p_ctrl", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    app.handle_event(event::Event::Key(event::KeyEvent::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    )));

    assert!(
        app.filters.is_empty(),
        "Ctrl-P was taken as a promote command"
    );
    assert!(app.search.is_some(), "Ctrl-P consumed the search");
}

/// `?` is reserved for the help view (#25). With n/N covering both
/// directions there is nothing left for it to do.
#[test]
fn question_mark_no_longer_opens_a_prompt() {
    let mut app = app_over_file("question_inert", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('?'));

    assert!(app.prompt.is_none(), "? still opens a prompt");
}

/// `/` in the explorer still searches filenames — that pane has its own
/// search and is untouched by this work. Asserts the selection actually
/// moved, not just that no filter was created: a `/` that did nothing at
/// all would also pass the filter-only assertion.
#[test]
fn slash_in_the_explorer_still_searches_filenames() {
    let mut app = app_over("slash_explorer", &["alpha.log", "zebra.log"]);
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zebra");
    key(&mut app, KeyCode::Enter);

    assert!(
        app.search.is_none(),
        "an explorer search became the file search"
    );
    let explorer = &app.explorer;
    assert_eq!(
        explorer.entries()[explorer.selected().unwrap()].name,
        "zebra.log",
        "the explorer search did not move the selection"
    );
}

/// #120 §8: `Esc` clears whichever search the focused pane owns first,
/// then the file search. One key, one meaning, layered.
#[test]
fn esc_clears_the_explorer_search_before_the_file_search() {
    let mut app = app_over_files(
        "esc_layers",
        &[("alpha.log", "hit\n"), ("zebra.log", "hit\n")],
    );
    open_file(&mut app, 0);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "hit");
    key(&mut app, KeyCode::Enter);
    assert!(app.search.is_some(), "sanity: file search set");

    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zebra");
    key(&mut app, KeyCode::Enter);
    assert!(app.explorer.has_search(), "sanity: explorer search set");

    key(&mut app, KeyCode::Esc);
    assert!(
        !app.explorer.has_search(),
        "Esc did not clear the explorer search"
    );
    assert!(
        app.search.is_some(),
        "Esc cleared the file search on the same press"
    );

    key(&mut app, KeyCode::Esc);
    assert!(
        app.search.is_none(),
        "second Esc did not clear the file search"
    );
}

/// From the file view, `Esc` does not reach into the explorer.
#[test]
fn esc_in_the_view_leaves_the_explorer_search_alone() {
    let mut app = app_over("esc_view_only", &["alpha.log", "zebra.log"]);
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "zebra");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Esc);

    assert!(app.explorer.has_search());
}

/// #120 §7 decision (b): the filter pane forwards `/` to the view.
/// Focus stays in the pane, and `n` works from there.
#[test]
fn slash_from_the_filter_pane_sets_the_search() {
    let mut app = app_over_file("slash_from_pane", "plain\nhit\nplain\nhit\n");
    key(&mut app, KeyCode::Char('f'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "hit");
    key(&mut app, KeyCode::Enter);

    assert!(app.search.is_some(), "no search was set");
    assert_eq!(app.focus, Focus::Filters, "focus moved");
    assert_eq!(cursor_source(&app), 1, "did not move to the first hit");

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 3);
}
