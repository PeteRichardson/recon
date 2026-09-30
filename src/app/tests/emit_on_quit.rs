use super::*;

// ---- emit on quit (#143): which quit emits --------------------------

fn app_emitting(name: &str, emit: Option<emit::Emit>) -> App<'static> {
    let file = fixture_path(name, "alpha\n");
    App::new(&Startup::from(Config {
        path: file.display().to_string(),
        emit,
        ..Config::default()
    }))
}

#[test]
fn q_quits_emitting_and_big_q_quits_silently() {
    let mut app = app_emitting("quit_q_emits", Some(emit::Emit::Cwd));
    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.state, AppState::Quit { emit: true });
    assert!(matches!(app.exit(), emit::Exit::Emit { .. }));

    let mut app = app_emitting("quit_big_q_silent", Some(emit::Emit::Cwd));
    key(&mut app, KeyCode::Char('Q'));
    assert_eq!(app.state, AppState::Quit { emit: false });
    assert_eq!(app.exit(), emit::Exit::Silent);
}

/// A terminal reports `Q` with Shift set; the arm must not be guarded on
/// an empty modifier set (the `?`/`N`/`S` trap).
#[test]
fn big_q_quits_with_shift_reported() {
    let mut app = app_emitting("quit_big_q_shift", Some(emit::Emit::Cwd));
    app.handle_event(event::Event::Key(event::KeyEvent::new(
        KeyCode::Char('Q'),
        KeyModifiers::SHIFT,
    )));
    assert_eq!(app.state, AppState::Quit { emit: false });
}

#[test]
fn without_emit_both_quits_are_silent() {
    let mut app = app_emitting("quit_no_emit_q", None);
    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.exit(), emit::Exit::Silent);

    let mut app = app_emitting("quit_no_emit_big_q", None);
    key(&mut app, KeyCode::Char('Q'));
    assert_eq!(app.exit(), emit::Exit::Silent);
}

#[test]
fn a_running_app_has_not_exited() {
    let mut app = app_emitting("quit_still_running", Some(emit::Emit::Cwd));
    assert_eq!(app.exit(), emit::Exit::Silent);
}

/// Characterises the precedence `dispatch_event` already has, before the
/// refactor that resolves it through the table (#199): `q` is a global
/// binding and the explorer does not bind it, so the pane must never get
/// a chance to swallow it first.
#[test]
fn the_global_scope_is_consulted_before_the_pane() {
    let (mut app, _root) = app_over_project("resolve_global", "alpha\n");
    app.focus = Focus::Explorer;

    // `q` is a global binding and the explorer does not bind it.
    key(&mut app, KeyCode::Char('q'));

    // `emit: true` pins the action, not just the scope: `Quit { .. }`
    // alone would also pass if `q` had resolved to `GlobalQuitSilent`.
    assert_eq!(app.state, AppState::Quit { emit: true });
}

// ---- --set and --hide at startup (#143, headless) ----------------------

/// `recon --set Bugs:only_hit --hide app.log` opens the TUI with the set
/// on, the profile applied, and hide mode live on the loaded file — the
/// same flags headless mode takes, applied the same way.
#[test]
fn set_and_hide_flags_apply_at_startup() {
    let file = fixture_file("startup_set_hide.log", b"hit\nmiss\n");
    let mut set = filter::test_support::loaded("Bugs", 50, false, &["hit", "miss"]);
    set.profiles
        .insert("only_hit".to_string(), vec!["hit".to_string()]);

    let app = App::new(&Startup {
        filter_sets: vec![set],
        ..Startup::from(Config {
            path: file.display().to_string(),
            set: vec!["Bugs:only_hit".to_string()],
            hide: true,
            ..Config::default()
        })
    });

    assert!(app.filters.sets()[1].enabled, "the set is on");
    let enabled: Vec<String> = app
        .filters
        .filters_in(1)
        .filter(|(_, filter)| filter.enabled)
        .map(|(_, filter)| filter.display_name())
        .collect();
    assert_eq!(enabled, ["hit"], "the profile was applied, not default");
    assert_eq!(app.document.mode(), Mode::FilteredOnly);
    assert_eq!(
        app.document.visible_lines(),
        ["hit"],
        "hide mode is live on the loaded file"
    );
}

/// `recon --unlist Bugs` opens the TUI with the autoload set unlisted:
/// no row, and no effect on the loaded file (#283).
#[test]
fn unlist_flag_applies_at_startup() {
    let file = fixture_file("startup_unlist.log", b"hit\nmiss\n");
    let mut set = filter::test_support::loaded("Bugs", 50, true, &["hit"]);
    set.profiles
        .insert("default".to_string(), vec!["hit".to_string()]);

    let app = App::new(&Startup {
        filter_sets: vec![set],
        ..Startup::from(Config {
            path: file.display().to_string(),
            unlist: vec!["Bugs".to_string()],
            hide: true,
            ..Config::default()
        })
    });

    assert!(!app.filters.sets()[1].listed, "the set is unlisted");
    assert!(!app.filters.sets()[1].enabled, "and so disabled");
    assert_eq!(
        app.document.visible_lines(),
        ["hit", "miss"],
        "its filter hides nothing"
    );
}

// ---- --emit lines --------------------------------------------------

fn emitted(app: &mut App) -> (Vec<String>, String) {
    match app.exit() {
        emit::Exit::Emit { lines, summary, .. } => (
            lines
                .into_iter()
                .map(|line| String::from_utf8(line).expect("utf-8 fixture"))
                .collect(),
            summary,
        ),
        emit::Exit::Spooled { mut spool, summary } => {
            use std::io::{Read, Seek, SeekFrom};
            let mut text = String::new();
            spool.seek(SeekFrom::Start(0)).expect("seek the spool");
            spool.read_to_string(&mut text).expect("utf-8 fixture");
            (text.lines().map(ToString::to_string).collect(), summary)
        }
        other => panic!("the session emitted nothing: {other:?}"),
    }
}

fn app_emitting_lines(name: &str, body: &str, line_numbers: bool) -> App<'static> {
    let file = fixture_path(name, body);
    let mut app = App::new(&Startup::from(Config {
        path: file.display().to_string(),
        emit: Some(emit::Emit::Lines),
        line_numbers,
        ..Config::default()
    }));
    key(&mut app, KeyCode::Char('t'));
    app
}

#[test]
fn lines_in_dim_mode_emits_every_visible_line_and_counts_the_matches() {
    let mut app = app_emitting_lines("emit_lines_dim", "hit one\nplain\nhit two\n", false);
    app.add_filter("hit").expect("valid");
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    assert_eq!(lines, vec!["hit one", "plain", "hit two"]);
    let name = app.view.filename().display().to_string();
    assert_eq!(
        summary,
        format!("recon: emitted 3 lines of {name}, dim mode (2 match) — u to emit matches only")
    );
}

/// A file past the preview cap, whose last line is the only hit.
fn app_emitting_a_large_file(name: &str, line_numbers: bool) -> App<'static> {
    let lines = crate::widgets::fileview::PREVIEW_LINES + 100;
    let body: String = (0..lines)
        .map(|i| {
            if i == lines - 1 {
                "hit at the end\n".to_string()
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    // A directory argument previews its first entry rather than reading it
    // whole, which is the state `q` has to finish from.
    let dir = fixture_dir(name);
    fs::write(dir.join("big.log"), &body).expect("write fixture");
    let mut app = App::new(&Startup::from(Config {
        path: dir.display().to_string(),
        emit: Some(emit::Emit::Lines),
        line_numbers,
        ..Config::default()
    }));
    assert!(app.view.is_truncated(), "the fixture must be past the cap");
    app.add_filter("hit").expect("valid");
    app
}

/// Poll the work `q` started until it ends.
fn finish(app: &mut App) {
    let start = std::time::Instant::now();
    while app.state == AppState::Finishing {
        app.poll_finish();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "the work never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// #351: `q` over a file the view holds a preview of reads the rest, and
/// emits a line past the preview, with a count over the whole file.
#[test]
fn lines_of_a_large_file_are_read_to_its_end() {
    let mut app = app_emitting_a_large_file("emit_lines_large", true);
    ctrl(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.state, AppState::Finishing);
    let status = app.finishing_text().expect("a progress line");
    assert!(status.starts_with("finishing "), "{status}");
    assert!(status.ends_with(" — Ctrl-c to cancel"), "{status}");

    finish(&mut app);

    let (lines, summary) = emitted(&mut app);
    let last = crate::widgets::fileview::PREVIEW_LINES + 100;
    assert_eq!(lines, vec![format!("{last}\thit at the end")]);
    let name = app.view.filename().display().to_string();
    assert_eq!(
        summary,
        format!("recon: emitted 1 lines of {name}, hide mode")
    );
}

/// Dim mode emits every line of the whole file, and counts the matches
/// over all of it.
#[test]
fn lines_of_a_large_file_in_dim_mode_count_the_whole_file() {
    let mut app = app_emitting_a_large_file("emit_lines_large_dim", false);
    key(&mut app, KeyCode::Char('q'));
    finish(&mut app);

    let (lines, summary) = emitted(&mut app);
    let total = crate::widgets::fileview::PREVIEW_LINES + 100;
    assert_eq!(lines.len(), total);
    assert_eq!(lines.last().map(String::as_str), Some("hit at the end"));
    let name = app.view.filename().display().to_string();
    assert_eq!(
        summary,
        format!(
            "recon: emitted {total} lines of {name}, dim mode (1 match) — u to emit matches only"
        )
    );
}

/// A cancel stops the reading thread and emits nothing.
#[test]
fn a_cancelled_large_file_emits_nothing() {
    let mut app = app_emitting_a_large_file("emit_lines_large_cancel", false);
    key(&mut app, KeyCode::Char('q'));
    ctrl(&mut app, KeyCode::Char('c'));

    assert_eq!(app.exit(), emit::Exit::Cancelled);
}

/// #386: the summary names the key the keymap in force gives hide mode,
/// and names none when a rebind left it without one.
#[test]
fn the_dim_summary_names_the_hide_key_in_force() {
    for (keys, name, hint) in [
        (
            &["Ctrl-u"][..],
            "emit_lines_rebound",
            Some("— Ctrl-u to emit matches only"),
        ),
        (&[][..], "emit_lines_unbound", None),
    ] {
        let mut app = app_emitting_lines(name, "hit one\nplain\n", false);
        app.add_filter("hit").expect("valid");
        rebind(&mut app, &[("global.toggle.hide", keys)]);
        key(&mut app, KeyCode::Char('q'));

        let (_, summary) = emitted(&mut app);

        match hint {
            Some(hint) => assert!(summary.ends_with(hint), "{summary}"),
            None => assert!(!summary.contains("matches only"), "{summary}"),
        }
    }
}

#[test]
fn lines_in_hide_mode_emits_only_the_matches() {
    let mut app = app_emitting_lines("emit_lines_hide", "hit one\nplain\nhit two\n", false);
    app.add_filter("hit").expect("valid");
    ctrl(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    assert_eq!(lines, vec!["hit one", "hit two"]);
    let name = app.view.filename().display().to_string();
    assert_eq!(
        summary,
        format!("recon: emitted 2 lines of {name}, hide mode")
    );
}

/// `-n` numbers are the *source* line numbers — the gutter's — so hide
/// mode gives `1` and `3`, not `1` and `2`.
#[test]
fn line_numbers_are_source_numbers_with_a_tab() {
    let mut app = app_emitting_lines("emit_lines_numbered", "hit one\nplain\nhit two\n", true);
    app.add_filter("hit").expect("valid");
    ctrl(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('q'));

    let (lines, _) = emitted(&mut app);

    assert_eq!(lines, vec!["1\thit one", "3\thit two"]);
}

#[test]
fn lines_with_no_filter_still_counts_zero_matches() {
    let mut app = app_emitting_lines("emit_lines_nofilter", "a\nb\n", false);
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    assert_eq!(lines, vec!["a", "b"]);
    assert!(summary.contains("dim mode (0 match)"), "{summary}");
}

#[test]
fn lines_over_a_directory_listing_emits_nothing_and_says_so() {
    let dir = fixture_dir("emit_lines_directory");
    fs::write(dir.join("a.txt"), "x\n").expect("write");
    let mut app = App::new(&Startup::from(Config {
        path: dir.display().to_string(),
        emit: Some(emit::Emit::Lines),
        ..Config::default()
    }));
    // The explorer starts on `a.txt`; select `..` so the view shows a
    // listing rather than a file.
    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    assert!(lines.is_empty());
    assert_eq!(
        summary,
        "recon: emitted 0 lines — the view is showing a directory"
    );
}

#[test]
fn lines_over_an_unreadable_file_emits_nothing_and_says_so() {
    let dir = fixture_dir("emit_lines_missing");
    let mut app = App::new(&Startup::from(Config {
        path: dir.join("nope.log").display().to_string(),
        emit: Some(emit::Emit::Lines),
        ..Config::default()
    }));
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    assert!(lines.is_empty());
    assert_eq!(
        summary,
        "recon: emitted 0 lines — the view is showing an error, not a file"
    );
}

// ---- --emit files and --emit cwd -----------------------------------

fn app_emitting_files(name: &str) -> (App<'static>, Sender<scan::Scanned>) {
    let mut app = app_over_files(
        name,
        &[("a.log", "hit\n"), ("b.log", "plain\n"), ("c.log", "hit\n")],
    );
    app.emit = Some(emit::Emit::Files);
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("hit").expect("valid");
    app.refresh_scan(false);
    (app, tx)
}

/// #352: `q` before the scan ends waits for it, showing how far it has
/// got, and then emits the complete answer — no "unscanned" count, since a
/// completed run has none.
#[test]
fn files_in_dim_mode_waits_for_the_scan_and_emits_the_counts() {
    let (mut app, tx) = app_emitting_files("emit_files_dim");
    mark(&mut app, &tx, 0, true);
    mark(&mut app, &tx, 1, false);
    // c.log not scanned yet.
    key(&mut app, KeyCode::Char('q'));

    assert_eq!(app.state, AppState::Finishing);
    assert!(app.is_running());
    let status = app.finishing_text().expect("a progress line");
    assert!(
        status.starts_with("finishing: 2 of 3 files scanned — Ctrl-c to cancel"),
        "{status}"
    );

    mark(&mut app, &tx, 2, true);
    assert!(app.poll_finish());
    assert_eq!(app.state, AppState::Quit { emit: true });

    let (lines, summary) = emitted(&mut app);

    let dir = app.explorer.dir().display().to_string();
    assert_eq!(
        lines,
        vec![
            format!("{dir}/a.log"),
            format!("{dir}/b.log"),
            format!("{dir}/c.log")
        ]
    );
    assert_eq!(
        summary,
        format!("recon: emitted 3 files from {dir}, dim mode (2 match) — u to emit matches only")
    );
}

/// #352's defect: hide mode keeps an unscanned row on screen, and `q` used
/// to emit it as a match. Now it waits, and a file that turns out not to
/// match is not emitted.
#[test]
fn files_in_hide_mode_never_emits_an_unscanned_file() {
    let (mut app, tx) = app_emitting_files("emit_files_hide_unscanned");
    mark(&mut app, &tx, 0, true);
    ctrl(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.state, AppState::Finishing);

    mark(&mut app, &tx, 1, false);
    mark(&mut app, &tx, 2, true);
    app.poll_finish();

    let (lines, summary) = emitted(&mut app);

    let dir = app.explorer.dir().display().to_string();
    assert_eq!(lines, vec![format!("{dir}/a.log"), format!("{dir}/c.log")]);
    assert_eq!(
        summary,
        format!("recon: emitted 2 files from {dir}, hide mode")
    );
}

/// Ctrl-c or Esc while finishing cancels: nothing is emitted, and the exit
/// says so (#351, #352). Any other key does nothing.
#[test]
fn a_cancel_while_finishing_emits_nothing() {
    for (name, cancel) in [
        ("emit_files_cancel_ctrl_c", KeyCode::Char('c')),
        ("emit_files_cancel_esc", KeyCode::Esc),
    ] {
        let (mut app, _tx) = app_emitting_files(name);
        key(&mut app, KeyCode::Char('q'));
        key(&mut app, KeyCode::Char('Q'));
        key(&mut app, KeyCode::Char('j'));
        assert_eq!(app.state, AppState::Finishing, "{name}: another key acted");

        if cancel == KeyCode::Esc {
            key(&mut app, cancel);
        } else {
            ctrl(&mut app, cancel);
        }

        assert_eq!(app.state, AppState::Cancelled, "{name}");
        assert!(!app.is_running());
        assert_eq!(app.exit(), emit::Exit::Cancelled, "{name}");
    }
}

#[test]
fn files_omits_the_unscanned_count_once_the_scan_is_complete() {
    let (mut app, tx) = app_emitting_files("emit_files_scanned");
    mark(&mut app, &tx, 0, true);
    mark(&mut app, &tx, 1, false);
    mark(&mut app, &tx, 2, true);
    key(&mut app, KeyCode::Char('q'));

    let (_, summary) = emitted(&mut app);

    let dir = app.explorer.dir().display().to_string();
    assert_eq!(
        summary,
        format!("recon: emitted 3 files from {dir}, dim mode (2 match) — u to emit matches only")
    );
}

#[test]
fn files_in_hide_mode_emits_only_the_matches() {
    let (mut app, tx) = app_emitting_files("emit_files_hide");
    mark(&mut app, &tx, 0, true);
    mark(&mut app, &tx, 1, false);
    mark(&mut app, &tx, 2, true);
    ctrl(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    let dir = app.explorer.dir().display().to_string();
    assert_eq!(lines, vec![format!("{dir}/a.log"), format!("{dir}/c.log")]);
    assert_eq!(
        summary,
        format!("recon: emitted 2 files from {dir}, hide mode")
    );
}

#[test]
fn files_with_no_filter_says_so_instead_of_counting() {
    let mut app = app_over_files("emit_files_nofilter", &[("a.log", "x\n"), ("b.log", "y\n")]);
    app.emit = Some(emit::Emit::Files);
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    assert_eq!(lines.len(), 2);
    let dir = app.explorer.dir().display().to_string();
    assert_eq!(
        summary,
        format!("recon: emitted 2 files from {dir}, dim mode, no filter")
    );
}

#[test]
fn files_with_only_an_excluding_filter_says_no_filter_rather_than_unscanned() {
    let mut app = app_over_files(
        "emit_files_exclude_only",
        &[("a.log", "x\n"), ("b.log", "y\n"), ("c.log", "z\n")],
    );
    app.emit = Some(emit::Emit::Files);
    app.add_excluding_filter("noise").expect("valid");
    app.refresh_scan(false);
    key(&mut app, KeyCode::Char('q'));

    let (_, summary) = emitted(&mut app);

    assert!(
        summary.ends_with(", no filter"),
        "an exclude-only filter set should read as unscannable, not counted: {summary}"
    );
    assert!(
        !summary.contains("unscanned"),
        "an exclude-only filter set never scans, so nothing is unscanned: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn files_writes_a_non_utf8_name_as_its_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let dir = fixture_dir("emit_files_bytes");
    let odd = std::ffi::OsStr::from_bytes(b"bad\xffname.log");
    // APFS and HFS+ enforce valid UTF-8 in filenames and reject this one
    // with EILSEQ, so on macOS there is no such file to list and nothing
    // to assert (see `non_utf8_fixture` in `widgets::explorer::tests`).
    // ext4, XFS, tmpfs and every other Unix filesystem take arbitrary
    // bytes, which is where this test actually runs.
    if let Err(err) = fs::write(dir.join(odd), "x\n") {
        eprintln!("skipping: this filesystem rejects non-UTF-8 names ({err})");
        return;
    }
    let mut app = App::new(&Startup::from(Config {
        path: dir.join("placeholder").display().to_string(),
        emit: Some(emit::Emit::Files),
        ..Config::default()
    }));
    key(&mut app, KeyCode::Char('q'));

    let emit::Exit::Emit { lines, .. } = app.exit() else {
        panic!("silent");
    };

    let expected = emit::path_bytes(&dir.join(odd));
    assert_eq!(lines, vec![expected]);
}

#[test]
fn cwd_emits_the_explorer_s_directory() {
    let mut app = app_over_files("emit_cwd", &[("a.log", "x\n")]);
    app.emit = Some(emit::Emit::Cwd);
    key(&mut app, KeyCode::Char('q'));

    let (lines, summary) = emitted(&mut app);

    let dir = app.explorer.dir().display().to_string();
    assert_eq!(lines, vec![dir.clone()]);
    assert_eq!(summary, format!("recon: emitted {dir}"));
    assert!(app.explorer.dir().is_absolute());
}

#[test]
fn esc_closes_the_picker_without_changing_a_flag() {
    let mut app = app_with_profiles("picker_esc");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(0);
    key(&mut app, KeyCode::Char('a'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Esc);
    assert!(app.picker.is_none());
    assert_eq!(flags_of(&app, 1), vec![true, false]);
}

/// A key that names no `Scope::Picker` action is swallowed right there
/// (task 7, Ruling 30): it must not fall through to `Scope::Global` the
/// way a pane scope's unresolved key does. Getting this wrong is silent
/// and severe — `q` would quit the program from inside the picker — and
/// nothing else in the suite presses an unbound key against an open
/// picker and checks the app is still running.
#[test]
fn an_unbound_key_is_swallowed_by_an_open_picker_rather_than_quitting() {
    let mut app = app_with_profiles("picker_swallow");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(0);
    key(&mut app, KeyCode::Char('a'));
    assert!(app.picker.is_some());

    key(&mut app, KeyCode::Char('q'));

    assert!(app.picker.is_some(), "q closed the picker");
    assert_eq!(
        app.state,
        AppState::Running,
        "q quit from inside the picker"
    );
}

/// A modified key must not act as the bare key: `Ctrl-j` moving the
/// selection (rather than `Ctrl-j` resolving to nothing, like every
/// other pane since #120) was the exact bug #193 fixed. `resolve` is
/// what enforces this now, not a guard inside the picker itself.
#[test]
fn a_modified_key_does_not_move_the_picker_selection() {
    let mut app = app_with_profiles("picker_modified_key");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(0);
    key(&mut app, KeyCode::Char('a'));
    let selected_before = app.picker.as_ref().unwrap().selected();

    ctrl(&mut app, KeyCode::Char('j'));

    assert_eq!(
        app.picker.as_ref().unwrap().selected(),
        selected_before,
        "Ctrl-j moved the selection"
    );
}

#[test]
fn a_on_a_set_without_profiles_reports_and_opens_nothing() {
    let mut app = app_with_profiles("picker_none");
    key(&mut app, KeyCode::Char('f'));
    // Rows: Header(a), alpha, beta, Header(b), neither — b's header is row 3.
    app.filters_pane.select(3);
    key(&mut app, KeyCode::Char('a'));
    assert!(app.picker.is_none());
    assert!(
        app.status_message
            .as_ref()
            .is_some_and(|m| m.text.contains("no profiles")),
        "no message"
    );
}

/// The picker is drawn over the panes.
#[test]
fn the_open_picker_is_visible() {
    let mut app = app_with_profiles("picker_draw");
    key(&mut app, KeyCode::Char('f'));
    app.filters_pane.select(0);
    key(&mut app, KeyCode::Char('a'));
    let area = Rect::new(0, 0, 80, 20);
    let mut buf = Buffer::empty(area);
    app.render(area, &mut buf);
    let screen: String = (0..20)
        .map(|y| (0..80).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains("Profiles"), "{screen}");
    assert!(screen.contains("only-beta"), "{screen}");
}
