use super::*;

// ---- saved filter sets (#128) -------------------------------------------

/// An `autoload` set's `default` profile is what the app starts with,
/// and the explorer has something to scan for from the first frame.
#[test]
fn an_autoload_set_is_live_at_startup() {
    let mut set = filter::test_support::loaded("a", 50, true, &["alpha", "beta"]);
    set.profiles.insert("default".into(), vec!["beta".into()]);
    let app = App::new(&Config {
        filter_sets: vec![set],
        ..Config::default()
    });
    assert!(app.filters.sets()[1].enabled);
    let flags: Vec<bool> = app.filters.filters_in(1).map(|(_, f)| f.enabled).collect();
    assert_eq!(flags, vec![false, true]);
    assert!(
        app.filters.matcher().is_some(),
        "beta selects, so the explorer scans"
    );
}

/// A set without `autoload` is known — listed, and coloured — but off.
#[test]
fn a_set_without_autoload_is_known_but_off() {
    let app = App::new(&Config {
        filter_sets: vec![filter::test_support::loaded("a", 50, false, &["alpha"])],
        ..Config::default()
    });
    assert!(!app.filters.sets()[1].enabled);
    assert_eq!(app.filters.len(), 1);
    assert!(app.filters.matcher().is_none());
}

// ---- the built-in definitions set (#127) ---------------------------------

/// Like `app_over_file`, but the fixture file is named `file` so that a
/// grammar can be found for it.
fn app_over_named_file(dir: &str, file: &str, body: &str) -> App<'static> {
    let dir = fixture_dir(dir);
    let path = dir.join(file);
    fs::write(&path, body).expect("write fixture");
    App::new(&Config {
        path: path.display().to_string(),
        ..Config::default()
    })
}

fn definitions_index(app: &App) -> usize {
    app.filters
        .sets()
        .iter()
        .position(|s| s.origin == filter::Origin::BuiltIn)
        .expect("present")
}

/// On a Rust file: expand the set, turn on `functions`, and only the
/// lines that start a function are included — with real line numbers,
/// and without a grammar pass until the row is on.
#[test]
fn functions_narrows_a_rust_file_to_its_fn_lines() {
    let mut app = app_over_named_file(
        "defs_rust",
        "main.rs",
        "// fn in a comment\nfn one() {}\nstruct S;\nfn two() {}\n",
    );
    assert!(!app.filters.needs_kinds(), "nothing effective yet");
    let set = definitions_index(&app);
    key(&mut app, KeyCode::Char('f'));
    // Rows: the `f i` hint, then Header(definitions), with an empty scratch set.
    app.filters_pane.select(1);
    key(&mut app, KeyCode::Enter);
    assert!(app.filters.sets()[set].enabled);
    assert_eq!(included(&app), 0, "expanded, but every row is off");
    app.filters_pane.select(2); // functions
    key(&mut app, KeyCode::Enter);
    let verdicts = app.document.verdicts().to_vec();
    assert_eq!(
        verdicts,
        [
            filter::Verdict::Unmatched,
            filter::Verdict::Included(0),
            filter::Verdict::Unmatched,
            filter::Verdict::Included(0),
        ]
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(included(&app), 0, "off again: the full file");
}

/// On a log file the rows exist and are inert.
#[test]
fn definition_rows_are_inert_on_a_log_file() {
    let mut app = app_over_named_file("defs_log", "app.log", "fn looks_like_one() {}\n");
    let set = definitions_index(&app);
    app.filters.set_enabled_set(set, true);
    app.filters.set_enabled(0, true); // functions
    app.refresh_view();
    assert_eq!(included(&app), 0);
}

/// User filters keep their numbers and colours regardless of the set.
#[test]
fn user_filters_keep_numbers_and_colours_beside_the_definitions_set() {
    let mut app = app_over_named_file("defs_numbers", "main.rs", "fn one() {}\nERROR\n");
    app.add_filter("ERROR").unwrap();
    let set = definitions_index(&app);
    app.filters.set_enabled_set(set, true);
    app.refresh_view();
    let error = app
        .filters
        .filters()
        .iter()
        .find(|f| f.display_name() == "ERROR")
        .expect("typed filter");
    assert_eq!(error.style, Style::default().fg(filter::DEFAULT_PALETTE[0]));
    let rows = widgets::filterlist::rows(&app.filters);
    assert_eq!(
        rows.len(),
        2 + crate::syntax::Kind::ALL.len(),
        "ERROR, header, one row per kind"
    );
}

// ---- saving the scratch set (#131) ---------------------------------------

#[test]
fn big_s_saves_the_scratch_set_and_adopts_it() {
    let path = save_fixture("save_scratch");
    let mut app = app_over_file("save_scratch_file", "ERROR\nDEBUG\n");
    app.save_path = Some(path.clone());
    app.add_filter("ERROR").unwrap();
    app.add_excluding_filter("DEBUG").unwrap();
    app.filters.set_enabled(1, false);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    assert!(app.prompt.is_some(), "the prompt is open");
    assert_eq!(
        app.prompt.as_ref().map(SearchPrompt::sigil),
        Some("save as: ")
    );
    typed(&mut app, "bug 57");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none(), "committed");

    let text = fs::read_to_string(&path).expect("written");
    assert!(text.contains("[sets.\"bug 57\"]"), "{text}");
    assert!(text.contains("pattern = 'ERROR'"), "{text}");
    assert!(text.contains("default = [\"ERROR\"]"), "{text}");
    assert_eq!(app.filters.filters_in(0).count(), 0, "scratch is empty");
    assert_eq!(app.filters.sets()[1].name, "bug 57");
    assert!(app.filters.sets()[1].enabled);
    let flags: Vec<bool> = app.filters.filters_in(1).map(|(_, f)| f.enabled).collect();
    assert_eq!(flags, vec![true, false], "the same flags");
    assert!(
        app.status_message
            .as_ref()
            .is_some_and(|m| m.text.contains("saved set")),
        "no confirmation"
    );
    // Rows: Header(bug 57), ERROR, DEBUG, Header(definitions).
    assert_eq!(widgets::filterlist::rows(&app.filters).len(), 4);
}

/// A real terminal reports `S` as `Char('S')` *with* `SHIFT` set;
/// `KeyEvent::from(code)` in the tests above sets no modifiers, which is
/// how an arm guarded on an empty modifier set can pass every test and
/// never fire for a user (#146 — the `?`/`N` trap, again).
#[test]
fn big_s_opens_the_prompt_with_shift_reported() {
    let mut app = app_over_file("save_shift", "ERROR\n");
    app.add_filter("ERROR").unwrap();
    key(&mut app, KeyCode::Char('f'));

    app.handle_event(event::Event::Key(event::KeyEvent::new(
        KeyCode::Char('S'),
        KeyModifiers::SHIFT,
    )));

    assert_eq!(
        app.prompt.as_ref().map(SearchPrompt::sigil),
        Some("save as: "),
        "S with Shift reported did not open the save prompt"
    );
}

#[test]
fn big_s_with_an_empty_scratch_set_reports_and_opens_nothing() {
    let mut app = app_over_file("save_empty", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    assert!(app.prompt.is_none());
    assert!(
        app.status_message
            .as_ref()
            .is_some_and(|m| m.text.contains("nothing to save")),
    );
}

#[test]
fn big_s_refuses_an_existing_name_and_keeps_the_prompt_open() {
    let path = save_fixture("save_taken");
    let mut app = app_over_file("save_taken_file", "alpha\n");
    app.save_path = Some(path.clone());
    app.filters = ActiveFilters::with_sets(
        None,
        &[filter::test_support::loaded("taken", 50, false, &["x"])],
    );
    app.add_filter("alpha").unwrap();
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "taken");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_some(), "the prompt stays open");
    assert!(
        app.prompt
            .as_ref()
            .and_then(|p| p.error.as_deref())
            .is_some_and(|e| e.contains("already exists")),
    );
    assert!(!path.exists(), "nothing written");
    assert_eq!(app.filters.filters_in(0).count(), 1, "scratch intact");
}

/// The file keeps its comments and other sets; the new set is appended.
#[test]
fn big_s_appends_to_an_existing_file_without_disturbing_it() {
    let path = save_fixture("save_append");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before = "# mine\n[sets.a]\n[[sets.a.filters]]\npattern = 'x' # keep\n";
    fs::write(&path, before).unwrap();
    let mut app = app_over_file("save_append_file", "alpha\n");
    app.save_path = Some(path.clone());
    app.filters = ActiveFilters::with_sets(None, &filtersets::parse(before, &path).unwrap());
    app.add_filter("alpha").unwrap();
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "b");
    key(&mut app, KeyCode::Enter);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with(before), "{text}");
    assert!(text.contains("[sets.b]"), "{text}");
    assert!(
        !app.filters.sets()[1].enabled,
        "a's state is untouched by the save"
    );
}

/// A set added to the file by hand since startup is not in memory, so the
/// name check in `save_scratch_as` cannot see it; `append_set` must refuse
/// it rather than replace the table (#154).
#[test]
fn big_s_refuses_a_set_added_to_the_file_since_startup() {
    let path = save_fixture("save_since_startup");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before = "# my file\n[sets.bug]\n[[sets.bug.filters]]\npattern = 'ORIGINAL'\n";
    fs::write(&path, before).unwrap();
    let mut app = app_over_file("save_since_startup_file", "alpha\n");
    app.save_path = Some(path.clone());
    // Started before `[sets.bug]` was written: the file is not loaded.
    app.filters = ActiveFilters::with_sets(None, &[]);
    app.add_filter("alpha").unwrap();
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "bug");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_some(), "the prompt stays open");
    let error = app
        .prompt
        .as_ref()
        .and_then(|p| p.error.as_deref())
        .unwrap_or_default();
    assert!(error.contains("already in filters.toml"), "{error}");
    assert!(error.contains("since recon started"), "{error}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before,
        "the hand-written table and its comment survive"
    );
    assert_eq!(app.filters.filters_in(0).count(), 1, "scratch intact");
}

/// Two scratch filters with one pattern would be two file filters
/// answering to one name. The refusal names the pattern and the fix the
/// UI offers — delete one — not the file's `name` key (#190).
#[test]
fn big_s_refuses_duplicate_scratch_patterns_with_a_pane_level_message() {
    let path = save_fixture("save_dup_pattern");
    let mut app = app_over_file("save_dup_pattern_file", "foo\n");
    app.save_path = Some(path.clone());
    app.add_filter("foo").unwrap();
    app.add_excluding_filter("foo").unwrap();
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "dup");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_some(), "the prompt stays open");
    let error = app
        .prompt
        .as_ref()
        .and_then(|p| p.error.as_deref())
        .unwrap_or_default();
    assert!(
        error.contains("two scratch filters share the pattern \"foo\""),
        "{error}"
    );
    assert!(error.contains("delete one"), "{error}");
    assert!(
        !error.contains("`name`"),
        "no advice about a file key: {error}"
    );
    assert!(!path.exists(), "nothing written");
}

/// The file is written beside itself and renamed over, so a crash
/// mid-write leaves the old file, not a truncated one (#153). After a
/// save the temporary is gone; a stale one from an earlier crash is
/// simply overwritten.
#[test]
fn big_s_writes_through_a_temporary_and_leaves_none_behind() {
    let path = save_fixture("save_atomic");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp = path.with_file_name("filters.toml.tmp");
    fs::write(&tmp, "garbage from a crash").unwrap();
    let mut app = app_over_file("save_atomic_file", "alpha\n");
    app.save_path = Some(path.clone());
    app.add_filter("alpha").unwrap();
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "a");
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none(), "committed");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("[sets.a]"), "{text}");
    assert!(!tmp.exists(), "the temporary was renamed over the file");
}

// ---- solo and reset (#132) -----------------------------------------------

/// `s` on b leaves only b's rows; the view matches b alone; `s` again
/// puts every set and the scratch filter back.
#[test]
fn s_solos_a_set_and_s_again_restores() {
    let mut app = app_with_three_sets("solo_round_trip");
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(included(&app), 3, "sanity: alpha, beta, scratch");
    // Rows: scratch, Header(a), alpha, Header(b), beta, Header(c).
    app.filters_pane.select(3);
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.filters.soloed(), Some(2));
    assert_eq!(widgets::filterlist::rows(&app.filters).len(), 2);
    assert_eq!(included(&app), 1, "only beta");
    // The soloed header is now row 0.
    app.filters_pane.select(0);
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.filters.soloed(), None);
    assert_eq!(included(&app), 3);
}

#[test]
fn s_on_a_disabled_set_enables_it_with_default_and_solos() {
    let mut app = app_with_three_sets("solo_disabled");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(5); // Header(c)
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.filters.soloed(), Some(3));
    assert!(app.filters.sets()[3].enabled);
    assert_eq!(included(&app), 0, "c has no default, so gamma stays off");
}

#[test]
fn big_r_resets_every_set_and_keeps_the_scratch_filter() {
    let mut app = app_with_three_sets("reset_all");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(3);
    key(&mut app, KeyCode::Char('s')); // solo b
    app.filters.set_enabled(1, false); // beta off, contrary to default
    key(&mut app, KeyCode::Char('R'));
    assert_eq!(app.filters.soloed(), None);
    let set_flags: Vec<bool> = app.filters.sets().iter().map(|s| s.enabled).collect();
    assert_eq!(set_flags, vec![true, true, true, false, false]);
    assert!(
        app.filters.filters()[1].enabled,
        "beta follows default again"
    );
    assert_eq!(app.filters.filters_in(0).count(), 1, "scratch filter kept");
    assert_eq!(included(&app), 3);
    assert_eq!(widgets::filterlist::rows(&app.filters).len(), 7);
}

// ---- the profile picker (#130) -------------------------------------------

/// `a` on a set with profiles opens the picker; it takes every key; `Enter`
/// applies the chosen profile to that set alone and closes it.
#[test]
fn a_opens_the_picker_and_enter_applies_the_profile() {
    let mut app = app_with_profiles("picker_apply");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(0); // Header(a)
    assert_eq!(
        flags_of(&app, 1),
        vec![true, false],
        "sanity: default applied at load"
    );

    key(&mut app, KeyCode::Char('a'));
    assert!(app.picker.is_some());
    key(&mut app, KeyCode::Char('q'));
    assert!(app.picker.is_some(), "the picker takes every key");
    assert_eq!(app.state, AppState::Running, "q did not quit");

    key(&mut app, KeyCode::Char('j')); // default -> only-beta (BTreeMap order)
    key(&mut app, KeyCode::Enter);
    assert!(app.picker.is_none());
    assert_eq!(flags_of(&app, 1), vec![false, true]);
    assert_eq!(flags_of(&app, 2), vec![false], "the other set is untouched");
    let included = app
        .document
        .verdicts()
        .iter()
        .filter(|v| matches!(v, filter::Verdict::Included(_)))
        .count();
    assert_eq!(included, 1, "the view re-evaluated under the profile");
}

// ---- the two-level pane (#129) -------------------------------------------

fn app_with_two_sets(fixture: &str) -> App<'static> {
    let mut a = filter::test_support::loaded("a", 10, true, &["alpha"]);
    a.profiles.insert("default".into(), vec!["alpha".into()]);
    let b = filter::test_support::loaded("b", 20, false, &["beta"]);
    let mut app = app_over_file(fixture, "alpha\nbeta\nneither\n");
    app.filters = ActiveFilters::with_sets(None, &[a, b]);
    app.refresh_view();
    app
}

/// `Enter` on a disabled set's header enables it, applies `default`, and
/// the view and explorer follow; `Enter` again collapses it and its
/// filters stop matching.
#[test]
fn enter_on_a_header_toggles_the_set_and_the_view_follows() {
    let mut app = app_with_two_sets("pane_toggle_set");
    key(&mut app, KeyCode::Char('f'));
    let included = |app: &App| {
        app.document
            .verdicts()
            .iter()
            .filter(|v| matches!(v, filter::Verdict::Included(_)))
            .count()
    };
    assert_eq!(included(&app), 1, "sanity: a's alpha is live");
    // Rows: Header(a), Filter(alpha), Header(b) — select b's header.
    app.filters_pane.select(2);
    key(&mut app, KeyCode::Enter);
    assert!(app.filters.sets()[2].enabled);
    assert_eq!(included(&app), 1, "b has no default, so beta stays off");
    app.filters.set_enabled(1, true);
    app.refresh_view();
    assert_eq!(included(&app), 2);
    // Collapse a: select its header at row 0.
    app.filters_pane.select(0);
    key(&mut app, KeyCode::Enter);
    assert!(!app.filters.sets()[1].enabled);
    assert_eq!(included(&app), 1, "alpha stopped matching");
    assert!(
        app.filters.filters()[0].enabled,
        "alpha's own flag survives"
    );
}

#[test]
fn d_on_a_header_reports_and_changes_nothing() {
    let mut app = app_with_two_sets("pane_header_read_only");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(0);
    key(&mut app, KeyCode::Char('d'));
    assert_eq!(app.filters.sets().len(), 4);
    assert_eq!(app.filters.len(), 2);
    assert!(
        app.status_message
            .as_ref()
            .is_some_and(|m| m.text.contains("filters.toml")),
        "no message"
    );
}
