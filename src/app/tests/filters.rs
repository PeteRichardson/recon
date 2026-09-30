use super::*;

#[test]
fn i_opens_a_filter_prompt_in_the_filter_pane() {
    let mut app = app_over("filter_prompt", &["a.rs"]);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "foo");

    assert_eq!(prompt_line(&mut app), "filter: foo");
}

#[test]
fn committing_a_filter_adds_it() {
    let mut app = app_over("filter_add", &["a.rs"]);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "foo");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_none(), "prompt stayed open");
    assert_eq!(app.filters.len(), 1);
}

#[test]
fn an_invalid_filter_pattern_keeps_the_prompt_open() {
    let mut app = app_over("filter_bad", &["a.rs"]);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "[");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_some(), "prompt closed on an invalid pattern");
    assert!(prompt_line(&mut app).contains("E486"));
    assert_eq!(app.filters.len(), 0, "a rejected pattern must not be added");
}

/// An empty regex matches every line (#347): as an including filter it
/// colours the whole file, and as an excluding one it hides it. The prompt
/// refuses it, and stays open, as it does for an invalid pattern.
#[test]
fn an_empty_filter_pattern_keeps_the_prompt_open() {
    for (sense, name) in [('i', "filter_empty_in"), ('x', "filter_empty_ex")] {
        let mut app = app_over(name, &["a.rs"]);

        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char(sense));
        key(&mut app, KeyCode::Enter);

        assert!(
            app.prompt.is_some(),
            "`f {sense}` closed on an empty pattern"
        );
        assert_eq!(
            app.prompt.as_ref().unwrap().error.as_deref(),
            Some(EMPTY_PATTERN)
        );
        assert_eq!(app.filters.len(), 0, "`f {sense}` added an empty filter");
    }
}

#[test]
fn esc_cancels_a_filter_prompt_without_adding() {
    let mut app = app_over("filter_esc", &["a.rs"]);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "foo");
    key(&mut app, KeyCode::Esc);

    assert!(app.prompt.is_none());
    assert_eq!(app.filters.len(), 0);
}

/// The prompt swallows keys, so `q` types rather than quits — as for search.
#[test]
fn q_while_filtering_is_typed_not_quit() {
    let mut app = app_over("filter_q", &["a.rs"]);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "q");

    assert!(app.is_running());
    assert_eq!(prompt_line(&mut app), "filter: q");
}

#[test]
fn successive_filters_take_different_colours() {
    let mut app = app_over("filter_colours", &["a.rs"]);

    for pattern in ["foo", "bar"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }

    let styles: Vec<_> = app.filters.filters().iter().map(|f| f.style.fg).collect();
    assert_ne!(styles[0], styles[1]);
}

/// #62: `[filters] palette` is parsed and merged in `config`, but the value
/// is only worth anything if `App::new` actually hands it to the filter set.
/// This is the seam where a correctly-parsed setting would otherwise be
/// silently dropped.
#[test]
fn a_configured_palette_colours_the_filters() {
    let dir = fixture_dir("configured_palette");
    fs::write(dir.join("a.rs"), "x").expect("write fixture");

    let mut app = App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        filter_palette: Some(vec![Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6)]),
        ..Config::default()
    });

    for pattern in ["foo", "bar"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }

    let styles: Vec<_> = app
        .filters
        .filters()
        .iter()
        .enumerate()
        .filter(|(i, _)| app.filters.is_user_authored(*i))
        .map(|(_, f)| f.style.fg)
        .collect();
    assert_eq!(
        styles,
        vec![Some(Color::Rgb(1, 2, 3)), Some(Color::Rgb(4, 5, 6))],
        "the configured palette never reached the filter set"
    );
}

#[test]
fn committing_a_filter_styles_the_view() {
    let mut app = app_over_file("restyle", "alpha\nbeta\ngamma\n");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    let styles = view_line_styles(&app);
    assert_eq!(styles.len(), 3, "a style slot per line");
    assert!(styles[1].is_some(), "matching line unstyled");
    assert!(
        styles[0]
            .expect("unmatched line unstyled")
            .add_modifier
            .contains(Modifier::DIM),
        "unmatched line not dimmed"
    );
}

#[test]
fn an_unfiltered_view_has_no_styles() {
    let app = app_over_file("restyle_none", "alpha\nbeta\n");

    assert!(view_line_styles(&app).iter().all(Option::is_none));
}

/// Filters describe a log format, so they outlive the file they were
/// defined against — and must be re-applied after a load clears them.
#[test]
fn filters_survive_loading_another_file() {
    let mut app = app_over_file("restyle_reload", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    let dir = fixture_dir_path("restyle_reload");
    fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
    app.perform_widget_action(Action::Load(dir.join("other.txt")));

    let styles = view_line_styles(&app);
    assert_eq!(styles.len(), 2, "styles not re-applied to the new file");
    assert!(styles[0].is_some(), "match in the new file unstyled");
}

/// The hide toggle describes how you are reading, not which file you are
/// reading — so it outlives a load exactly as the filter set does.
///
/// `sync_document` replaces `self.document` wholesale, and `Document::new`
/// starts at `Mode::default()`; the filters survived only because `App`
/// owns them separately. The mode had no such owner.
#[test]
fn the_hide_mode_survives_loading_another_file() {
    let mut app = app_over_file("hide_mode_load", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('H'));
    assert_eq!(app.document.mode(), Mode::FilteredOnly, "sanity: hiding");
    assert_eq!(view_lines(&app), vec!["beta".to_string()]);

    let dir = fixture_dir_path("hide_mode_load");
    fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
    app.perform_widget_action(Action::Load(dir.join("other.txt")));

    assert_eq!(
        app.document.mode(),
        Mode::FilteredOnly,
        "the load reset the hide toggle"
    );
    assert_eq!(
        view_lines(&app),
        vec!["beta again".to_string()],
        "the new file came back unhidden"
    );
}

/// The workflow in the issue is cursor movement in the explorer, which
/// fires `Preview`, not `Load` — so previews must hold the mode too, or
/// the whole point (skimming a directory for files that are not blank) is
/// lost on every keystroke.
#[test]
fn the_hide_mode_survives_a_explorer_preview() {
    let mut app = app_over_file("hide_mode_preview", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('H'));

    let dir = fixture_dir_path("hide_mode_preview");
    fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
    app.perform_widget_action(Action::Preview(dir.join("other.txt")));

    assert_eq!(
        app.document.mode(),
        Mode::FilteredOnly,
        "the preview reset the hide toggle"
    );
    assert_eq!(view_lines(&app), vec!["beta again".to_string()]);
}

/// The payoff the issue asks for: skimming a directory while hiding, a
/// file with no matches shows an empty pane, which is the signal to move
/// on. Without the mode surviving, every such file came back full of
/// unmatched text and there was nothing to skim.
#[test]
fn a_file_with_no_matches_shows_an_empty_view_while_hiding() {
    let mut app = app_over_file("hide_mode_no_match", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('H'));

    let dir = fixture_dir_path("hide_mode_no_match");
    fs::write(dir.join("quiet.txt"), "nothing\nhere\n").expect("write");
    app.perform_widget_action(Action::Preview(dir.join("quiet.txt")));

    assert_eq!(app.document.mode(), Mode::FilteredOnly);
    assert!(
        app.document.visible().is_empty(),
        "a file with no matches still had visible lines while hiding"
    );
    assert!(
        view_lines(&app).iter().all(String::is_empty),
        "expected a blank pane, got {:?}",
        view_lines(&app)
    );
}

/// The rebuild guard's empty-set collision, reachable with no hide toggle
/// involved at all: an excluding filter that removes every line leaves
/// `visible()` legitimately empty, and that used to compare equal to the
/// `last_visible` that `sync_document` had just cleared. The guard read
/// "nothing changed" and left the freshly loaded file on screen in full —
/// showing exactly the lines the filter existed to remove.
///
/// This is why the key (`last_generation` now, `last_visible` then) is an
/// `Option`: "the buffer holds no rows" and "what the buffer holds is
/// unknown" are different claims, and only the second one may force a
/// rebuild.
#[test]
fn loading_a_file_that_every_filter_excludes_leaves_a_blank_view() {
    let mut app = app_over_file("exclude_all_load", "noise one\nnoise two\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "noise");
    key(&mut app, KeyCode::Enter);
    assert!(
        app.document.visible().is_empty(),
        "sanity: the filter excluded every line"
    );

    let dir = fixture_dir_path("exclude_all_load");
    fs::write(dir.join("other.txt"), "noise three\nnoise four\n").expect("write");
    app.perform_widget_action(Action::Load(dir.join("other.txt")));

    assert_eq!(
        app.document.mode(),
        Mode::Dimmed,
        "sanity: the hide toggle plays no part in this one"
    );
    assert!(app.document.visible().is_empty());
    assert!(
        view_lines(&app).iter().all(String::is_empty),
        "the excluded lines came back on screen: {:?}",
        view_lines(&app)
    );
}

/// Toggling back after a load must still restore the whole file — the
/// mode surviving must not leave the document unable to leave it.
#[test]
fn the_hide_mode_can_still_be_toggled_off_after_a_load() {
    let mut app = app_over_file("hide_mode_untoggle", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('H'));

    let dir = fixture_dir_path("hide_mode_untoggle");
    fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
    app.perform_widget_action(Action::Load(dir.join("other.txt")));
    key(&mut app, KeyCode::Char('H'));

    assert_eq!(app.document.mode(), Mode::Dimmed);
    assert_eq!(
        view_lines(&app),
        vec!["beta again".to_string(), "nothing".to_string()],
        "toggling off did not restore the whole file"
    );
}

/// `sync_document` replaces `self.document` wholesale, so whatever
/// `last_generation` (finding 1's rebuild-skip guard) held is meaningless
/// afterwards — it describes a buffer built from the *previous*
/// document. Reloading the *same* file while an excluding filter is
/// active reproduces an identical generation every time (the document
/// is genuinely equal), which the guard alone cannot tell apart from
/// "nothing changed". `Explorer` fires `Action::Load` unconditionally
/// on `Enter`, even over the entry that is already open, so this is not a
/// contrived path.
#[test]
fn reloading_the_same_file_reapplies_an_active_excluding_filter() {
    let mut app = app_over_file("reload_same_file", "alpha\nnoise\ngamma\n");
    let path = fixture_dir_path("reload_same_file").join("log.txt");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "noise");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        view_lines(&app),
        vec!["alpha".to_string(), "gamma".to_string()],
        "sanity: the excluding filter hid a line"
    );

    app.perform_widget_action(Action::Load(path));

    assert_eq!(
        view_lines(&app),
        vec!["alpha".to_string(), "gamma".to_string()],
        "the reload brought back the hidden line: the buffer kept the \
         freshly loaded, unfiltered content instead of being rebuilt"
    );
    assert_eq!(
        view_line_styles(&app).len(),
        view_lines(&app).len(),
        "styles/numbers were sized for the filtered subset but the buffer came back full-length"
    );
}

#[test]
fn bang_disables_every_filter_and_restores_them() {
    let mut app = app_over_file("restyle_bang", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('!'));
    assert!(
        view_line_styles(&app).iter().all(Option::is_none),
        "! did not clear the styling"
    );

    key(&mut app, KeyCode::Char('!'));
    assert!(
        view_line_styles(&app)[1].is_some(),
        "! did not restore the filters"
    );
}

/// `!` must put back what the user had, not switch everything on.
#[test]
fn bang_restores_the_per_filter_state_it_captured() {
    let mut app = app_over_file("bang_restore", "alpha\nbeta\n");
    for pattern in ["alpha", "beta"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }
    app.filters.set_enabled(1, false);

    key(&mut app, KeyCode::Char('!'));
    assert!(!app.filters.any_enabled());

    key(&mut app, KeyCode::Char('!'));

    assert!(app.filters.filters()[0].enabled);
    assert!(
        !app.filters.filters()[1].enabled,
        "a filter the user had disabled was switched back on"
    );
}

/// With every filter disabled by hand and nothing captured, `!` has no
/// prior state to restore — so it enables everything, rather than
/// capturing all-disabled and becoming inert.
#[test]
fn bang_re_enables_when_everything_was_disabled_by_hand() {
    let mut app = app_over_file("bang_escape", "alpha\nbeta\n");
    for pattern in ["alpha", "beta"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }
    app.filters.set_enabled(0, false);
    app.filters.set_enabled(1, false);

    key(&mut app, KeyCode::Char('!'));

    assert!(
        app.filters.any_enabled(),
        "! left the user with no way back"
    );
}

/// Adding a filter while `!` has a capture pending must not strand it.
/// The capture describes a set that no longer exists, so it is dropped and
/// the next `!` captures afresh — otherwise `!` sees an enabled filter,
/// finds a capture already pending, and silently does nothing forever.
#[test]
fn adding_a_filter_after_bang_does_not_leave_bang_inert() {
    let mut app = app_over_file("bang_after_add", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('!'));
    assert!(!app.filters.any_enabled());

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('!'));
    assert!(
        !app.filters.any_enabled(),
        "! did not disable the new filter"
    );

    key(&mut app, KeyCode::Char('!'));
    assert!(
        app.filters.any_enabled(),
        "! went inert - nothing came back"
    );
}

/// The sequence from #150: `!`, then `Enter` on a filter row to switch
/// it back on by hand, then `!` again. The second `!` sees an enabled
/// filter and must disable it — it used to find the capture still
/// pending and do nothing, and every `!` after it did nothing too.
#[test]
fn toggling_a_filter_by_hand_after_bang_does_not_leave_bang_inert() {
    let mut app = app_over_file("bang_after_toggle", "alpha\nbeta\n");
    for pattern in ["alpha", "beta"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }
    key(&mut app, KeyCode::Char('!'));
    assert!(!app.filters.any_enabled(), "sanity: ! disabled both");

    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Enter);
    assert!(app.filters.any_enabled(), "sanity: Enter re-enabled a row");

    key(&mut app, KeyCode::Char('!'));
    assert!(
        !app.filters.any_enabled(),
        "! went inert after a filter was toggled by hand"
    );

    key(&mut app, KeyCode::Char('!'));
    assert!(app.filters.any_enabled(), "! did not restore anything");
}
