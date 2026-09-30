use super::*;

// ---- the search moves as you type (#271) -----------------------------

/// Each keystroke moves the cursor before Enter, and each re-scans from
/// the origin rather than from the hit the last keystroke reached: a
/// pattern edited back to one that hits the origin's own line lands
/// there without a wrap.
#[test]
fn typing_a_pattern_moves_before_enter_and_rescans_from_the_origin() {
    let mut app = app_over_file("type_moves", "x1\nx2\nx3\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "x2");
    assert!(app.prompt.is_some(), "sanity: the prompt is open");
    assert_eq!(cursor_source(&app), 1, "did not move before Enter");
    assert_eq!(
        search_text(&app),
        "x2",
        "the highlight is not set while typing"
    );

    key(&mut app, KeyCode::Backspace);
    typed(&mut app, "1");
    assert_eq!(cursor_source(&app), 0, "did not re-scan from the origin");
    assert_eq!(
        status(&app),
        None,
        "a scan from the last hit would have wrapped"
    );

    key(&mut app, KeyCode::Enter);
    assert_eq!(cursor_source(&app), 0, "Enter moved the cursor");
    assert!(app.prompt.is_none());
}

/// Esc puts the cursor and the scroll back where `/` opened; Enter keeps
/// the position the search reached.
#[test]
fn esc_restores_the_origin_and_enter_keeps_the_position() {
    for commit in [false, true] {
        let mut app = app_over_file(&format!("type_origin_{commit}"), &numbered_lines(400));
        draw_tall(&mut app);
        key(&mut app, KeyCode::Char('t'));
        for _ in 0..40 {
            key(&mut app, KeyCode::Char('j'));
        }
        draw_tall(&mut app);
        let (row, screen_row) = (cursor_source(&app), cursor_screen_row(&app));
        assert!(screen_row > 0, "sanity: the cursor is off the top row");

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "line 300$");
        draw_tall(&mut app);
        assert_eq!(cursor_source(&app), 300, "sanity: moved while typing");

        key(&mut app, if commit { KeyCode::Enter } else { KeyCode::Esc });
        draw_tall(&mut app);

        if commit {
            assert_eq!(cursor_source(&app), 300, "Enter did not keep the position");
            assert_eq!(search_text(&app), "line 300$");
        } else {
            assert_eq!(cursor_source(&app), row, "Esc did not restore the cursor");
            assert_eq!(
                cursor_screen_row(&app),
                screen_row,
                "Esc did not restore the scroll"
            );
            assert!(app.search.is_none(), "Esc left the search set");
            assert!(
                app.file_view_highlight().is_none(),
                "Esc left the highlight"
            );
        }
        assert!(app.prompt.is_none(), "the prompt is still open");
    }
}

/// A half-typed regex is silent: the highlight clears and the cursor
/// sits at the origin, with no error shown. Enter on it reports `E486`
/// and keeps the prompt open, with the cursor still at the origin.
#[test]
fn a_half_typed_invalid_regex_is_silent_and_sits_at_the_origin() {
    let mut app = app_over_file("type_invalid", "alpha\nfoo(x\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "foo");
    assert_eq!(cursor_source(&app), 1, "sanity: moved to the hit");
    typed(&mut app, "(");

    assert_eq!(
        cursor_source(&app),
        0,
        "an invalid pattern did not return to the origin"
    );
    assert!(
        app.search.is_none(),
        "an invalid pattern kept the highlight"
    );
    assert!(app.file_view_highlight().is_none());
    assert_eq!(
        prompt_line(&mut app),
        "/foo(",
        "an error is shown while typing"
    );

    key(&mut app, KeyCode::Enter);
    assert_eq!(prompt_line(&mut app), INVALID_PATTERN);
    assert!(
        app.prompt.is_some(),
        "Enter closed the prompt on an invalid pattern"
    );
    assert_eq!(cursor_source(&app), 0);
}

/// Enter on an empty prompt, and Backspace past the first character,
/// both cancel and return to the origin.
#[test]
fn enter_on_an_empty_prompt_and_backspace_past_empty_cancel_to_the_origin() {
    for backspace in [false, true] {
        let mut app = app_over_file(&format!("type_cancel_{backspace}"), "alpha\nx\nbeta\n");
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Char('j'));

        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "beta");
        assert_eq!(cursor_source(&app), 2, "sanity: moved to the hit");
        for _ in 0..4 {
            key(&mut app, KeyCode::Backspace);
        }
        assert!(app.prompt.is_some(), "an empty pattern closed the prompt");
        assert_eq!(
            cursor_source(&app),
            1,
            "an empty pattern did not return to the origin"
        );
        assert!(app.search.is_none(), "an empty pattern kept a search");

        key(
            &mut app,
            if backspace {
                KeyCode::Backspace
            } else {
                KeyCode::Enter
            },
        );

        assert!(
            app.prompt.is_none(),
            "backspace {backspace}: the prompt did not close"
        );
        assert_eq!(
            cursor_source(&app),
            1,
            "backspace {backspace}: not at the origin"
        );
        assert!(
            app.search.is_none(),
            "backspace {backspace}: a search was set"
        );
    }
}

/// The pattern the open prompt holds, for tests.
fn prompt_pattern(app: &App) -> String {
    app.prompt
        .as_ref()
        .map(|prompt| prompt.pattern.clone())
        .expect("a prompt is open")
}

/// Up (or Ctrl-p) and Down (or Ctrl-n) in the prompt (#274).
fn recall_older(app: &mut App, with_ctrl: bool) {
    if with_ctrl {
        ctrl(app, KeyCode::Char('p'));
    } else {
        key(app, KeyCode::Up);
    }
}

fn recall_newer(app: &mut App, with_ctrl: bool) {
    if with_ctrl {
        ctrl(app, KeyCode::Char('n'));
    } else {
        key(app, KeyCode::Down);
    }
}

/// Commit `pattern` in a `/` prompt over whichever pane has focus.
fn commit_search(app: &mut App, pattern: &str) {
    key(app, KeyCode::Char('/'));
    typed(app, pattern);
    key(app, KeyCode::Enter);
    assert!(app.prompt.is_none(), "sanity: {pattern:?} was committed");
}

/// After two committed searches, Up recalls the newer, Up again the
/// older and then stays there; Down walks back to the newer and then
/// to an empty prompt. Each recall is an edit: the cursor moves as
/// typing the pattern would, the cursor sits at the pattern's end, and
/// Esc still returns to the origin. Ctrl-p and Ctrl-n are the same
/// keys under other names.
#[test]
fn up_recalls_the_newer_then_the_older_and_down_walks_back_to_an_empty_prompt() {
    for with_ctrl in [false, true] {
        let mut app = app_over_file(&format!("recall_{with_ctrl}"), "x1\nx2\nx3\n");
        key(&mut app, KeyCode::Char('t'));
        commit_search(&mut app, "x3");
        commit_search(&mut app, "x2");
        key(&mut app, KeyCode::Char('g'));
        assert_eq!(cursor_source(&app), 0, "sanity: the origin");

        key(&mut app, KeyCode::Char('/'));
        assert_eq!(prompt_pattern(&app), "", "a new prompt starts empty");

        recall_older(&mut app, with_ctrl);
        assert_eq!(prompt_pattern(&app), "x2", "the first Up is the newer");
        assert_eq!(cursor_source(&app), 1, "the recall did not move");
        assert_eq!(search_text(&app), "x2", "the recall set no highlight");
        assert_eq!(
            app.prompt.as_ref().map(|prompt| prompt.cursor),
            Some(2),
            "the cursor is not at the end of the recalled pattern"
        );

        recall_older(&mut app, with_ctrl);
        assert_eq!(prompt_pattern(&app), "x3", "the second Up is the older");
        assert_eq!(cursor_source(&app), 2);

        recall_older(&mut app, with_ctrl);
        assert_eq!(prompt_pattern(&app), "x3", "Up past the oldest moved");
        assert_eq!(cursor_source(&app), 2);

        recall_newer(&mut app, with_ctrl);
        assert_eq!(prompt_pattern(&app), "x2", "Down did not walk back");
        assert_eq!(cursor_source(&app), 1);

        recall_newer(&mut app, with_ctrl);
        assert_eq!(
            prompt_pattern(&app),
            "",
            "Down past the newest is not empty"
        );
        assert_eq!(cursor_source(&app), 0, "an empty prompt sits at the origin");
        assert!(app.search.is_none(), "an empty prompt kept the highlight");
        assert!(app.prompt.is_some(), "Down closed the prompt");

        recall_newer(&mut app, with_ctrl);
        assert_eq!(
            prompt_pattern(&app),
            "",
            "Down on an empty prompt did something"
        );
        assert!(app.prompt.is_some());

        recall_older(&mut app, with_ctrl);
        assert_eq!(
            prompt_pattern(&app),
            "x2",
            "Up after emptying did not start over"
        );
        assert_eq!(cursor_source(&app), 1);

        key(&mut app, KeyCode::Esc);
        assert!(app.prompt.is_none(), "Esc did not close the prompt");
        assert_eq!(cursor_source(&app), 0, "Esc did not restore the origin");
        assert_eq!(
            search_text(&app),
            "x2",
            "Esc did not restore the search set before `/` opened"
        );
    }
}

/// A recalled pattern is a starting point: typing after it edits it,
/// and the edit re-runs from the origin as any keystroke does. The next
/// Up still steps to the entry before the one recalled.
#[test]
fn typing_after_a_recall_edits_the_recalled_pattern() {
    let mut app = app_over_file("recall_edit", "x1\nx2\nx3\nx22\n");
    key(&mut app, KeyCode::Char('t'));
    commit_search(&mut app, "x3");
    commit_search(&mut app, "x2");
    key(&mut app, KeyCode::Char('g'));

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    typed(&mut app, "2");
    assert_eq!(prompt_pattern(&app), "x22");
    assert_eq!(
        cursor_source(&app),
        3,
        "the edit did not move to the new hit"
    );

    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt_pattern(&app),
        "x3",
        "Up after an edit did not step on"
    );
    assert_eq!(cursor_source(&app), 2);
}

/// The file search and the explorer's filename search keep separate
/// histories: a filename pattern never appears in the file prompt, nor
/// the reverse.
#[test]
fn the_file_and_filename_searches_keep_separate_histories() {
    let mut app = app_over("recall_separate", &["a.log", "b.log"]);
    key(&mut app, KeyCode::Char('e'));
    assert_eq!(
        selected_name(&app),
        "a.log",
        "sanity: the explorer has focus"
    );
    commit_search(&mut app, "b");
    assert_eq!(
        selected_name(&app),
        "b.log",
        "sanity: the filename search moved"
    );

    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt_pattern(&app),
        "",
        "the file prompt recalled a filename pattern"
    );
    assert!(
        app.prompt.is_some(),
        "Up on an empty history closed the prompt"
    );
    key(&mut app, KeyCode::Esc);
    commit_search(&mut app, "x");

    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt_pattern(&app),
        "b",
        "the explorer lost its own history"
    );
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt_pattern(&app),
        "b",
        "the explorer prompt recalled a file pattern"
    );
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt_pattern(&app), "x");
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt_pattern(&app),
        "x",
        "the file prompt recalled a filename pattern"
    );
}

/// A cancelled prompt adds nothing, and a pattern committed again moves
/// to the front rather than appearing twice.
#[test]
fn a_cancelled_prompt_adds_nothing_and_a_repeat_moves_to_the_front() {
    let mut app = app_over_file("recall_dedup", "x1\nx2\nx3\n");
    key(&mut app, KeyCode::Char('t'));
    commit_search(&mut app, "x1");
    commit_search(&mut app, "x2");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "x3");
    key(&mut app, KeyCode::Esc);

    commit_search(&mut app, "x1");

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt_pattern(&app),
        "x1",
        "the repeat did not move to the front"
    );
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt_pattern(&app), "x2", "the cancelled pattern was kept");
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt_pattern(&app), "x2", "the repeat was kept twice");
}

/// Enter on a pattern that does not compile keeps the prompt open and
/// adds nothing: only a committed pattern is worth recalling.
#[test]
fn a_rejected_pattern_is_not_added_to_the_history() {
    let mut app = app_over_file("recall_invalid", "x1\n");
    key(&mut app, KeyCode::Char('t'));
    commit_search(&mut app, "x1");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "x(");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_some(), "sanity: the prompt stayed open");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt_pattern(&app), "x1", "the rejected pattern was kept");
}

/// The history holds fifty patterns: the fifty-first committed drops
/// the oldest.
#[test]
fn the_fifty_first_pattern_drops_the_oldest() {
    let mut app = app_over_file("recall_cap", "x\n");
    key(&mut app, KeyCode::Char('t'));
    for n in 1..=HISTORY_CAP + 1 {
        commit_search(&mut app, &format!("p{n}"));
    }

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt_pattern(&app), "p51", "the first Up is the newest");
    for _ in 1..HISTORY_CAP {
        key(&mut app, KeyCode::Up);
    }
    assert_eq!(
        prompt_pattern(&app),
        "p2",
        "fifty Ups did not reach the oldest kept"
    );
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt_pattern(&app),
        "p2",
        "the fifty-first pattern kept the oldest"
    );
}

/// The filter prompts keep no history: Up, Down and their Ctrl twins do
/// nothing in them, as they did when they were unbound, even after a
/// `/` has committed a pattern.
#[test]
fn filter_prompts_keep_no_history() {
    let mut app = app_over_file("recall_filter", "x1\n");
    key(&mut app, KeyCode::Char('t'));
    commit_search(&mut app, "x1");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    assert!(
        matches!(&app.prompt, Some(prompt) if prompt.kind == PromptKind::Filter),
        "sanity: an include prompt is open"
    );
    typed(&mut app, "ab");
    for with_ctrl in [false, true] {
        recall_older(&mut app, with_ctrl);
        assert_eq!(
            prompt_pattern(&app),
            "ab",
            "Up recalled into a filter prompt"
        );
        recall_newer(&mut app, with_ctrl);
        assert_eq!(prompt_pattern(&app), "ab", "Down changed a filter prompt");
    }
    assert!(
        app.prompt.is_some(),
        "a recall key closed the filter prompt"
    );
    assert!(app.filters.is_empty(), "a recall key committed a filter");
    key(&mut app, KeyCode::Esc);
}

/// `prompt.history.prev` and `prompt.history.next` are keymap ids like
/// any other: a `[keymap]` line moves them, and the keys they left do
/// nothing in the prompt.
#[test]
fn the_recall_keys_can_be_rebound() {
    let file = fixture_path("recall_rebound", "x1\nx2\n");
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert(
        "prompt.history.prev".to_string(),
        vec!["Ctrl-k".to_string()],
    );
    bindings.insert(
        "prompt.history.next".to_string(),
        vec!["Ctrl-j".to_string()],
    );
    let config = Config {
        path: file.display().to_string(),
        ..Config::default()
    };
    let (map, warnings) = crate::keymap::config::build(
        &crate::keymap::config::KeymapConfig { bindings },
        config.warnings(),
    )
    .expect("valid");
    assert!(
        warnings.is_empty(),
        "moving the recall keys warned: {warnings:?}"
    );
    let mut app = App::new(&Startup {
        bindings: map,
        keymap_warnings: warnings,
        ..Startup::from(config)
    });
    key(&mut app, KeyCode::Char('t'));
    commit_search(&mut app, "x2");
    key(&mut app, KeyCode::Char('g'));

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    ctrl(&mut app, KeyCode::Char('p'));
    assert_eq!(prompt_pattern(&app), "", "the default keys still recall");

    ctrl(&mut app, KeyCode::Char('k'));
    assert_eq!(prompt_pattern(&app), "x2", "the rebound key did not recall");
    assert_eq!(cursor_source(&app), 1);

    key(&mut app, KeyCode::Down);
    ctrl(&mut app, KeyCode::Char('n'));
    assert_eq!(
        prompt_pattern(&app),
        "x2",
        "the default keys still walk back"
    );

    ctrl(&mut app, KeyCode::Char('j'));
    assert_eq!(
        prompt_pattern(&app),
        "",
        "the rebound key did not walk back"
    );
    assert_eq!(cursor_source(&app), 0);
}

/// The help overlay and the README key table both carry the two recall
/// actions, under the names a `[keymap]` line uses.
#[test]
fn the_recall_actions_are_documented() {
    for name in ["prompt.history.prev", "prompt.history.next"] {
        assert!(
            crate::help::KEYMAP
                .iter()
                .flat_map(|section| section.bindings)
                .any(|binding| binding.names.contains(&name)),
            "{name} is not in the help overlay"
        );
        let readme = include_str!("../../../README.md");
        assert!(
            readme.contains(&format!("`{name}`")),
            "{name} is not in the README key table"
        );
    }
}

/// While typing, the scan covers the visible lines only: in hide mode a
/// pattern that matches only hidden lines leaves the cursor at the
/// origin and the visible lines as they were.
#[test]
fn typing_searches_the_visible_lines_only() {
    let mut app = app_over_file("type_hidden", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha|gamma").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.document.visible(), &[0, 2], "sanity: hiding");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "bet");

    assert_eq!(app.document.visible(), &[0, 2], "typing widened the view");
    assert_eq!(cursor_source(&app), 0, "typing moved to a hidden line");

    typed(&mut app, "");
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    typed(&mut app, "gam");
    assert_eq!(cursor_source(&app), 2, "a visible hit was not found");
}

/// A probe costs nothing: Esc in the prompt puts back the search that
/// was set before `/` opened, highlight and all.
#[test]
fn esc_in_the_prompt_restores_the_previous_search() {
    let mut app = app_over_file("type_previous", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert_eq!(cursor_source(&app), 1, "sanity");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "gam");
    assert_eq!(
        search_text(&app),
        "gam",
        "sanity: the probe replaced the search"
    );
    assert_eq!(cursor_source(&app), 2);

    key(&mut app, KeyCode::Esc);

    assert_eq!(
        search_text(&app),
        "beta",
        "the previous search did not come back"
    );
    assert_eq!(cursor_source(&app), 1);
    assert_eq!(app.file_view_highlight().as_deref(), Some("beta"));
}
