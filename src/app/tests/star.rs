use super::*;

// ---- `*` (#120 §13) ----------------------------------------------------

#[test]
fn word_under_cursor_follows_the_view_cursor() {
    let mut app = app_over_file("word_cursor", "foo bar\nbaz qux\n");
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(app.word_under_cursor().as_deref(), Some("foo"));

    key(&mut app, KeyCode::Char('w'));
    assert_eq!(app.word_under_cursor().as_deref(), Some("bar"));

    key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.word_under_cursor().as_deref(), Some("qux"));
}

/// `*` is vim's two-key version of #67's first use case: a long,
/// possibly mangled symbol under the cursor — where else does it appear?
#[test]
fn star_searches_for_the_word_under_the_cursor_and_steps() {
    let mut app = app_over_file("star_basic", "foo bar\nbar\nfoo\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('*'));

    let search = app.search.as_ref().expect("a search was set");
    assert_eq!(search.text, "foo");
    assert_eq!(
        cursor_source(&app),
        2,
        "did not step to the next occurrence"
    );

    key(&mut app, KeyCode::Char('k'));
    key(&mut app, KeyCode::Char('k'));
    key(&mut app, KeyCode::Char('w'));
    key(&mut app, KeyCode::Char('*'));
    assert_eq!(search_text(&app), "bar");
    assert_eq!(cursor_source(&app), 1);
}

#[test]
fn star_keeps_a_mangled_name_whole() {
    let body = "_ZN4core3fmt9Formatter3pad17hE::x\nplain\n_ZN4core3fmt9Formatter3pad17hE\n";
    let mut app = app_over_file("star_mangled", body);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('l'));

    key(&mut app, KeyCode::Char('*'));

    assert_eq!(search_text(&app), "_ZN4core3fmt9Formatter3pad17hE");
    assert_eq!(cursor_source(&app), 2);
}

#[test]
fn star_on_whitespace_says_so() {
    let mut app = app_over_file("star_space", "a  b\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('l'));

    key(&mut app, KeyCode::Char('*'));

    assert!(app.search.is_none(), "a search was set from whitespace");
    assert_eq!(status(&app), Some("no word under the cursor"));
}

/// The textarea's `$` (End) puts the cursor one past the last character;
/// `*` retries one column back rather than reporting no word (#120).
#[test]
fn star_after_dollar_searches_the_last_word() {
    let mut app = app_over_file("star_eol", "foo bar\nbar\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('$'));

    key(&mut app, KeyCode::Char('*'));

    assert_eq!(search_text(&app), "bar");
    assert_eq!(cursor_source(&app), 1);
}

/// Hide mode with a matching-nothing including filter (#36's cousin, not
/// its case: `anything_including` is true here) leaves nothing visible.
/// `word_under_cursor` must not fall back to row 0 in that state.
#[test]
fn star_with_nothing_visible_says_no_word() {
    let mut app = app_over_file("star_hidden", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("zzz").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('u'));
    key(&mut app, KeyCode::Char('*'));

    assert!(app.search.is_none());
    assert_eq!(status(&app), Some("no word under the cursor"));
}

/// `*` while peeked behaves like `/`: it sets the search without first
/// clearing the peek, and searches the plain file the peek shows.
#[test]
fn star_while_peeked_behaves_like_slash() {
    let mut app = app_over_file("star_peeked", "foo\nfoo\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("nomatch").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('*'));

    assert_eq!(search_text(&app), "foo");
    assert_eq!(cursor_source(&app), 1);
    assert!(app.peek.is_some(), "* ended the peek");
}

/// A search while peeked finds its hits in the plain file the peek
/// shows, and `n` steps them there. Neither ends the peek: a search
/// never leaves the file, so there is nothing to put back first.
#[test]
fn slash_and_n_while_peeked_search_the_plain_file() {
    let mut app = app_over_file("slash_peeked", "alpha\nbeta\ngamma\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.document.visible(), &[0], "sanity: hiding");

    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert_eq!(cursor_source(&app), 1);
    assert!(app.peek.is_some(), "/ ended the peek");

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(cursor_source(&app), 3);
    assert!(app.peek.is_some(), "n ended the peek");
}

/// `* p`: the whole "symbol to filter" flow without a selection (#67).
#[test]
fn star_then_p_promotes_the_literal_word() {
    let mut app = app_over_file("star_promote", "foo bar\nfoo\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('*'));
    key(&mut app, KeyCode::Char('p'));

    assert!(app.search.is_none(), "p did not consume the search");
    assert_eq!(app.filters.len(), 1);
    let numbered = widgets::filterlist::numbered(&app.filters);
    assert_eq!(
        app.filters.filters()[numbered[0]].predicate.display(),
        "foo"
    );
}

/// Shift arrives with `*` on most layouts; the arm must not be guarded
/// on an empty modifier set (the `?`/`N` trap).
#[test]
fn star_works_with_shift_reported() {
    let mut app = app_over_file("star_shift", "foo\nfoo\n");
    key(&mut app, KeyCode::Char('t'));

    app.handle_event(event::Event::Key(event::KeyEvent::new(
        KeyCode::Char('*'),
        KeyModifiers::SHIFT,
    )));

    assert!(app.search.is_some());
}

#[test]
fn star_in_the_explorer_hints_at_t_star() {
    let mut app = app_over_file("star_hint", "foo\n");
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('*'));

    assert!(app.search.is_none());
    assert_eq!(
        status(&app),
        Some("* searches the word under the cursor · t *")
    );
    assert_eq!(app.focus, Focus::Explorer);
}

/// `*` from the filter pane acts on the view's cursor, like `n`/`N`,
/// `/`, and `[`/`]` (#120 §11) — not a hint.
#[test]
fn star_from_the_filter_pane_acts_on_the_view() {
    let mut app = app_over_file("star_from_pane", "foo bar\nfoo\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('f'));

    key(&mut app, KeyCode::Char('*'));

    assert_eq!(search_text(&app), "foo");
    assert_eq!(cursor_source(&app), 1);
    assert_eq!(app.focus, Focus::Filters);
}

/// A terminal paste arrives as individual `Char` events; a newline in
/// it must not become part of a single-line pattern (#120 §13). This
/// guards the `*`/paste interplay if bracketed paste is ever enabled
/// for #67.
#[test]
fn a_pasted_newline_is_dropped_from_the_prompt() {
    let mut app = app_over_file("paste_newline", "alpha\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "al");
    key(&mut app, KeyCode::Char('\n'));
    key(&mut app, KeyCode::Char('\r'));
    ctrl(&mut app, KeyCode::Char('j'));
    typed(&mut app, "pha");

    assert_eq!(prompt_line(&mut app).trim_end(), "/alpha");
}

/// #120 §14: `3` toggles the filter the pane labels `3`. Global, so the
/// loop can switch a filter without leaving the view.
#[test]
fn digits_toggle_filters_by_their_pane_number() {
    let mut app = app_over_file("digit_toggle", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.filters.add("beta").expect("valid pattern");
    app.refresh_view();
    assert!(app.filters.filters()[1].enabled, "sanity");

    key(&mut app, KeyCode::Char('2'));
    assert!(
        !app.filters.filters()[1].enabled,
        "2 did not toggle filter 2"
    );
    assert!(app.filters.filters()[0].enabled, "2 touched filter 1");
    assert_eq!(app.focus, Focus::View, "focus moved");

    key(&mut app, KeyCode::Char('2'));
    assert!(app.filters.filters()[1].enabled, "2 did not toggle back");
}

#[test]
fn a_digit_with_no_filter_behind_it_says_so() {
    let mut app = app_over_file("digit_missing", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('9'));

    assert!(app.filters.filters()[0].enabled, "9 toggled something");
    assert_eq!(
        app.status_message.as_ref().map(|m| m.text.as_str()),
        Some("no filter 9")
    );
}

/// The digit toggles what the *pane* numbers: with a set soloed, the
/// numbering restarts inside it, and so does the key.
#[test]
fn digits_follow_the_pane_numbering_under_a_solo() {
    // Autoload alone (`true`) only brings the *set* on; a filter needs a
    // `default` profile to come on with it — the convention every other
    // solo/reset test in this module already follows.
    let mut a = filter::test_support::loaded("a", 10, true, &["alpha"]);
    a.profiles.insert("default".into(), vec!["alpha".into()]);
    let mut b = filter::test_support::loaded("b", 20, true, &["beta"]);
    b.profiles.insert("default".into(), vec!["beta".into()]);
    let mut app = app_over_file("digit_solo", "alpha\nbeta\nneither\n");
    app.filters = ActiveFilters::with_sets(None, &[a, b]);
    app.refresh_view();
    key(&mut app, KeyCode::Char('t'));
    let before = widgets::filterlist::numbered(&app.filters);
    assert_eq!(before.len(), 2, "sanity: alpha is 1, beta is 2");
    let beta = before[1];

    // Solo set 2 (`b`): the pane now labels beta `1`, and so does the key.
    app.filters.solo(2);
    assert_eq!(
        widgets::filterlist::numbered(&app.filters),
        vec![beta],
        "sanity"
    );
    assert!(app.filters.filters()[beta].enabled, "sanity");

    key(&mut app, KeyCode::Char('1'));

    assert!(
        !app.filters.filters()[beta].enabled,
        "1 did not follow the solo numbering"
    );
    assert!(
        app.filters.filters()[before[0]].enabled,
        "1 reached the soloed-out set"
    );
}

/// `apply_search`'s ordering — `refresh_view` before `step_to_interesting`
/// — doesn't show up in `a_search_with_hiding_on_collapses_the_file_to_its_matches`
/// because the cursor starts on row 0 there: forward-from-0 and "the
/// first hit" agree by coincidence regardless of order. Here the cursor
/// starts past both hits. A swapped order runs `step_to_interesting`
/// against verdicts that don't know about the new search yet, so it finds
/// nothing and no-ops; `refresh_view` then merely re-anchors the
/// unmoved cursor to its nearest surviving neighbour (`beta2`, source 3)
/// once it finally runs — landing one hit short of forward-from-cursor's
/// real answer (`beta1`, source 1).
#[test]
fn slash_moves_forward_from_the_cursor_only_after_the_rebuild_completes() {
    let mut app = app_over_file("slash_order", "alpha\nbeta1\ngamma\nbeta2\ndelta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('H'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(
        cursor_source(&app),
        4,
        "sanity: cursor starts past both hits"
    );

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        cursor_source(&app),
        1,
        "landed on the second hit (3), not the first hit reachable forward \
         from the cursor (1) — refresh_view did not complete before \
         step_to_interesting ran"
    );
}

/// `key()` builds every event with `KeyModifiers::NONE`, which cannot
/// express a real keypress: crossterm attaches SHIFT to every uppercase
/// character a terminal actually sends. A guard written as
/// `key.modifiers.is_empty()` looks correct under `key()` and is
/// unreachable in production — this fires the event by hand, the way
/// crossterm really would, to catch exactly that class of bug.
#[test]
fn capital_n_tolerates_the_shift_modifier_a_real_terminal_sends() {
    let mut app = app_over_file("n_shift", "hit a\nplain\nhit b\nplain\nhit c\n");
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

    app.handle_event(event::Event::Key(KeyEvent::new(
        KeyCode::Char('N'),
        KeyModifiers::SHIFT,
    )));

    assert_eq!(
        cursor_source(&app),
        0,
        "N with the SHIFT modifier a real terminal sends did not walk backwards"
    );
}

/// `n`/`N` bypass `FileView::perform`, which is where a truncated
/// preview normally promotes itself to a full load on first interaction.
/// Without repeating that promotion, `n` on a large log would silently
/// wrap inside the preview and never reach a hit past it.
#[test]
fn n_promotes_a_truncated_preview_before_stepping() {
    let dir = fixture_dir("n_truncated_promote");
    // Past PREVIEW_LINES, so the first preview is truncated; the hit sits
    // beyond the preview boundary, reachable only once promoted.
    let hit_at = crate::widgets::fileview::PREVIEW_LINES + 50;
    let body: String = (0..crate::widgets::fileview::PREVIEW_LINES + 100)
        .map(|i| {
            if i == hit_at {
                "HIT line\n".to_string()
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
    // The explorer falls back to the first real entry (the log) and
    // previews it, exactly as `upgrading_a_truncated_preview_resyncs_styles_without_reloading`
    // does — the startup argument names a file that does not exist.
    key(&mut app, KeyCode::Down);
    focus_file_view(&mut app);
    app.filters.add("HIT").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(
        cursor_source(&app),
        hit_at,
        "n did not reach a hit beyond the truncated preview"
    );
}

/// `apply_search` is documented as "set it, then do exactly what `n`
/// does" — including the truncated-preview promotion above. This drives
/// `/` through the real key path (unlike `n_promotes_a_truncated_preview_before_stepping`,
/// which adds a filter directly and so never
/// exercises `apply_search` at all) against a preview truncated the same
/// way, with the only hit past the boundary. If `apply_search` skips the
/// promotion, the pattern is evaluated against the preview alone: there
/// is no hit in range, `step_to_interesting` is a no-op, and the cursor
/// stays wherever it started.
#[test]
fn slash_promotes_a_truncated_preview_before_landing_on_a_hit() {
    let dir = fixture_dir("slash_truncated_promote");
    // Same shape as the `n` fixture above: past PREVIEW_LINES, with the
    // only hit beyond the preview boundary.
    let hit_at = crate::widgets::fileview::PREVIEW_LINES + 50;
    let body: String = (0..crate::widgets::fileview::PREVIEW_LINES + 100)
        .map(|i| {
            if i == hit_at {
                "HIT line\n".to_string()
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
    key(&mut app, KeyCode::Down);
    focus_file_view(&mut app);
    assert_eq!(cursor_source(&app), 0, "sanity: cursor starts at the top");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "HIT");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        cursor_source(&app),
        hit_at,
        "/ did not reach a hit beyond the truncated preview"
    );
}

/// Toggling hide mode shrinks the visible line set, which rebuilds the
/// pane's buffer through `FileView::show_lines_with_cursor`. That call
/// does not itself clear the textarea's search pattern — `set_lines`
/// resets history, selection, custom highlights, atomic ranges and the
/// viewport, but never touches the search pattern — and `apply_view`
/// recomputes and re-applies the highlight unconditionally on every
/// pass regardless, the same as `styles`/`numbers` above. This confirms
/// a filter-driven rebuild does not disturb it; the swap that actually
/// clears the pattern is covered separately, below.
#[test]
fn the_span_highlight_survives_a_rebuild() {
    let mut app = app_over_file("hl_rebuild", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('H'));

    assert!(
        app.file_view_highlight().is_some(),
        "the highlight was lost"
    );
}

/// Unlike a filter-driven rebuild, `load`/`preview` replace the textarea
/// outright (see `FileView::load`), which drops any pattern the old one
/// held. Filters and the hide mode already had to survive that same
/// swap — `filters_survive_loading_another_file`,
/// `the_hide_mode_survives_loading_another_file` — the highlight is no
/// different.
#[test]
fn the_span_highlight_survives_loading_another_file() {
    let mut app = app_over_file("hl_reload", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    let dir = fixture_dir_path("hl_reload");
    fs::write(dir.join("other.txt"), "beta again\nnothing\n").expect("write");
    app.perform_widget_action(Action::Load(dir.join("other.txt")));

    assert!(
        app.file_view_highlight().is_some(),
        "the highlight did not survive loading another file"
    );
}

#[test]
fn clearing_the_search_clears_the_span_highlight() {
    let mut app = app_over_file("hl_esc", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Esc);

    assert!(app.file_view_highlight().is_none());
}
