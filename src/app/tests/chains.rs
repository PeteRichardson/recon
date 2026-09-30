use super::*;

// ---- chains return focus (#120 §8, decision (a)) ---------------------

/// `f i fn Enter` from the file view: the filter is added, focus comes
/// back, and the cursor lands on the first `fn` as if `n` were pressed.
#[test]
fn f_i_enter_from_the_view_returns_and_steps() {
    let mut app = app_over_file("chain_fi_view", "plain\nfn one\nplain\nfn two\n");
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(cursor_source(&app), 0, "sanity");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "fn");
    key(&mut app, KeyCode::Enter);

    // `len()`, not `filters().len()`: the built-in definitions set (#127)
    // adds eleven rows of its own, and this is asking whether the user's
    // one filter landed, not counting recon's.
    assert_eq!(app.filters.len(), 1, "the filter was not added");
    assert_eq!(app.focus, Focus::View, "focus did not return");
    assert_eq!(cursor_source(&app), 1, "did not step to the first hit");
    assert!(app.chain_origin.is_none(), "origin not consumed");
}

/// From the explorer, the return acts as the explorer's `n`. A
/// filename search is the observable form: adding a filter changes the
/// scan cache key, so match marks cannot be pre-sent in a test, but the
/// explorer's search-repeat needs no scan at all.
#[test]
fn f_i_enter_from_the_explorer_returns_and_repeats_its_n() {
    let mut app = app_over("chain_fi_explorer", &["a.log", "b.log", "c.log"]);
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "log");
    key(&mut app, KeyCode::Enter);
    // The explorer starts on `a.log`, and a typed search considers the
    // origin row first, as the file search does (#272): `a.log` matches,
    // so `/log` stays on it. The `n` the return presses is the step —
    // the first match strictly after the selection.
    assert_eq!(
        app.explorer.selected_name().as_deref(),
        Some("a.log"),
        "sanity"
    );

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "x");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::Explorer, "focus did not return");
    assert_eq!(
        app.explorer.selected_name().as_deref(),
        Some("b.log"),
        "the return did not act as the explorer's n"
    );
}

/// `f c … Enter` (edit) returns too; `f d`, `f Enter` and plain `f` do not.
#[test]
fn f_c_returns_but_f_d_and_f_enter_stay() {
    let mut app = app_over_file("chain_fc", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("alpha").expect("valid pattern");
    app.filters.add("beta").expect("valid pattern");
    app.refresh_view();

    key(&mut app, KeyCode::Char('f'));
    // Filters added directly leave the pane with no selection; `j`
    // selects the first row, which is what a user would do before `c`.
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('c'));
    typed(&mut app, "x");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::View, "f c … Enter did not return");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Filters, "f Enter returned");

    key(&mut app, KeyCode::Char('d'));
    assert_eq!(app.focus, Focus::Filters, "f d returned");
    // `len()`, not `filters().len()` — see the comment above.
    assert_eq!(app.filters.len(), 1, "d did not delete");
}

/// A focus change inside the chain ends it: `f Tab i … Enter` stays put.
#[test]
fn a_focus_change_after_f_ends_the_chain() {
    let mut app = app_over_file("chain_tab", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Filters, "sanity: back on the pane");
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        app.focus,
        Focus::Filters,
        "returned after a Tab broke the chain"
    );
}

/// `f f` is the sticky gesture: a second `f` keeps focus in the pane.
#[test]
fn f_f_makes_the_pane_sticky() {
    let mut app = app_over_file("chain_ff", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::Filters);
}

/// A cancelled prompt ends the chain: `f i … Esc`, then `i … Enter`, stays.
#[test]
fn a_cancelled_prompt_ends_the_chain() {
    let mut app = app_over_file("chain_cancel", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "al");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::Filters);
}

/// A search prompt commit is not a chain step: `f / … Enter` stays.
///
/// Originally named `search_and_save_prompts_do_not_return` and asserted
/// only this; the `S` and search-row `c` cases below need their own
/// setup (a non-empty scratch set) so they are
/// now separate tests rather than more steps bolted onto this one.
#[test]
fn f_slash_search_commit_does_not_return() {
    let mut app = app_over_file("chain_search", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::Filters, "a search commit returned");
}

/// `S … Enter` is not a chain step either, even when the scratch set it
/// saves was itself added through a chain (`f i … Enter`, which does
/// return — that part is `f_i_enter_returns`'s job, not this test's).
#[test]
fn big_s_save_commit_does_not_return() {
    let path = save_fixture("chain_save_set");
    let mut app = app_over_file("chain_save_set_file", "alpha\n");
    app.save_path = Some(path.clone());
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.focus,
        Focus::View,
        "sanity: the filter add chained back"
    );

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "chain_save_set");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::Filters, "a save-set commit returned");
}

/// Backspacing past the start of the prompt abandons it, the same as
/// `Esc` — and, like `Esc`, ends the chain: a fresh `f i … Enter`
/// afterwards is judged as its own chain, not a continuation.
#[test]
fn backspacing_out_of_the_prompt_ends_the_chain() {
    let mut app = app_over_file("chain_backspace", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "ab");
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    assert!(
        app.prompt.is_none(),
        "sanity: backspacing past empty closed it"
    );

    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        app.focus,
        Focus::Filters,
        "the abandoned prompt should not chain"
    );
}

/// `f x … Enter` returns, same as `f i … Enter`.
#[test]
fn f_x_enter_returns_too() {
    let mut app = app_over_file("chain_fx", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::View, "an exclude-filter commit returned");
    assert_eq!(app.filters.len(), 1);
}

/// A return to the explorer whose `n` finds nothing says so, rather
/// than landing silently. The commit has only just started the scan, so
/// every mark is still `Unknown` and the honest answer is "not scanned
/// yet", not "no matching file" (#158) — that one is reserved for a
/// listing the scan has finished answering.
#[test]
fn a_return_to_the_explorer_before_the_scan_answers_says_scanning() {
    let mut app = app_over("chain_explorer_scanning", &["a.log", "b.log"]);
    let (_scanner, tx) = record_scans(&mut app);
    key(&mut app, KeyCode::Char('e'));
    let before = app.explorer.selected_name();

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "zzz");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.focus, Focus::Explorer, "sanity: returned");
    assert_eq!(status(&app), Some("scanning…"));
    assert_eq!(
        app.explorer.selected_name(),
        before,
        "nothing to step to, so the selection should not have moved"
    );

    // The scan answers: neither file matches. Now the claim is true.
    mark(&mut app, &tx, 0, false);
    mark(&mut app, &tx, 1, false);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(status(&app), Some("no matching file"));
}

/// With nothing to scan for — no including filter, so no matcher and no
/// worker — the marks stay `Unknown` for ever, and `scanning…` would be
/// a promise nothing keeps. Directories are always `Unknown` too, and
/// must not count.
#[test]
fn n_with_no_scan_running_says_no_matching_file() {
    let dir = fixture_dir("n_no_scan");
    fs::write(dir.join("a.log"), "x").expect("write");
    fs::create_dir_all(dir.join("sub")).expect("mkdir");
    let mut app = App::new(&Startup::from(Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    }));
    key(&mut app, KeyCode::Char('e'));
    app.add_excluding_filter("noise").expect("valid pattern");
    app.refresh_view();
    assert!(app.filters.matcher().is_none(), "sanity: nothing selects");

    key(&mut app, KeyCode::Char('n'));

    assert_eq!(status(&app), Some("no matching file"));
}

/// A real `n` pressed while the worker is still out says the same as
/// the chain's synthetic one.
#[test]
fn n_before_the_first_answer_lands_says_scanning() {
    let mut app = app_over("n_scanning", &["a.log", "b.log"]);
    let (_scanner, tx) = record_scans(&mut app);
    key(&mut app, KeyCode::Char('e'));
    app.add_filter("zzz").expect("valid pattern");
    app.refresh_view();
    app.refresh_scan(false);

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(status(&app), Some("scanning…"));

    // Half answered is still scanning.
    mark(&mut app, &tx, 0, false);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(status(&app), Some("scanning…"));

    mark(&mut app, &tx, 1, false);
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(status(&app), Some("no matching file"));
}

/// The `Enter` that commits is still swallowed once after the return,
/// so it cannot also open the explorer's selection.
#[test]
fn the_committing_enter_is_swallowed_after_a_return() {
    let mut app = app_over("chain_swallow", &["a.log", "b.log"]);
    key(&mut app, KeyCode::Char('e'));
    let before = shown(&app);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "x");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Explorer, "sanity: returned");
    key(&mut app, KeyCode::Enter);

    assert_eq!(shown(&app), before, "the doubled Enter opened an entry");
}

// ---- wrong-pane hints (#120 §9) ---------------------------------------

#[test]
fn a_filter_verb_in_the_view_hints_at_the_chain() {
    let mut app = app_over_file("hint_view", "alpha\n");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('i'));

    assert_eq!(status(&app), Some("i adds a filter · f i"));
    assert!(app.prompt.is_none(), "a prompt opened");
    assert_eq!(app.focus, Focus::View, "focus moved");
}

#[test]
fn every_filter_verb_hints_in_the_explorer() {
    let mut app = app_over("hint_explorer", &["a.log"]);
    key(&mut app, KeyCode::Char('e'));
    let expected = [
        ('i', "i adds a filter · f i"),
        ('x', "x adds an excluding filter · f x"),
        ('c', "c changes the selected filter · f c"),
        ('d', "d deletes the selected filter · f d"),
        ('m', "m toggles include and context · f m"),
        ('a', "a picks a profile for the set · f a"),
        ('s', "s solos the set · f s"),
    ];
    for (c, text) in expected {
        key(&mut app, KeyCode::Char(c));
        assert_eq!(status(&app), Some(text), "hint for {c}");
        assert_eq!(app.focus, Focus::Explorer, "{c} moved focus");
    }
    assert!(app.filters.is_empty(), "a verb acted outside its pane");
}

#[test]
fn a_hint_lasts_one_keypress() {
    let mut app = app_over_file("hint_gone", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('i'));
    assert!(status(&app).is_some(), "sanity");

    key(&mut app, KeyCode::Char('j'));

    assert_eq!(status(&app), None);
}

#[test]
fn h_and_l_in_the_filter_pane_hint_at_the_explorer() {
    let mut app = app_over("hint_pane", &["a.log"]);
    key(&mut app, KeyCode::Char('f'));

    key(&mut app, KeyCode::Char('h'));
    assert_eq!(status(&app), Some("h goes up a directory · e h"));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(status(&app), Some("l opens the entry · e l"));
    assert_eq!(app.focus, Focus::Filters);
}

/// The hint does not fire where the verb is real.
#[test]
fn a_filter_verb_in_the_filter_pane_is_not_a_hint() {
    let mut app = app_over_file("hint_real", "alpha\n");
    key(&mut app, KeyCode::Char('f'));

    key(&mut app, KeyCode::Char('i'));

    assert!(app.prompt.is_some(), "i did not open the prompt");
    assert_eq!(status(&app), None);
}
