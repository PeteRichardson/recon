//! What a session writes to the log (#490). Each record is read back from
//! `crate::logging::capture`, on this test's own thread.

use super::*;
use crate::logging::capture;

use capture::here_at as logged;

// ---- the shutdown summary ---------------------------------------------

/// A session that ends writes one summary, and names how it ended.
#[test]
fn a_session_that_ends_writes_its_summary() {
    capture::install();
    let terminate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let file = fixture_path("log_summary", "alpha\n");
    let app = App::new(&Startup::from(Config {
        path: Some(file.display().to_string()),
        emit: Some(emit::Emit::Cwd),
        ..Config::default()
    }))
    .with_terminate(terminate);
    let terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).expect("a terminal");

    app.run(terminal).expect("the loop");

    let summary = logged(log::Level::Debug, "session ended: Terminated");
    assert_eq!(summary.len(), 1, "{:?}", capture::here());
    assert!(summary[0].contains("emit Cwd"), "{summary:?}");
    assert!(summary[0].contains("files opened"), "{summary:?}");
    assert!(summary[0].contains("view cache"), "{summary:?}");
}

/// The quit keys and Ctrl-c each name their own ending.
#[test]
fn each_way_out_names_its_ending() {
    use crate::logging::Ending;
    for (name, press, ending) in [
        ("log_ending_q", KeyCode::Char('q'), Ending::Quit),
        ("log_ending_big_q", KeyCode::Char('Q'), Ending::QuitSilent),
    ] {
        let mut app = app_over_file(name, "alpha\n");
        key(&mut app, press);
        assert_eq!(app.ending(), ending, "{press:?}");
    }
    let mut app = app_over_file("log_endings_ctrl_c", "alpha\n");
    ctrl(&mut app, KeyCode::Char('c'));
    assert_eq!(app.ending(), Ending::Cancelled);
}

/// Each file shown counts toward the summary's "files opened".
#[test]
fn a_file_shown_is_counted_as_opened() {
    let mut app = app_over_files("log_opened", &[("a.log", "a\n"), ("b.log", "b\n")]);
    let before = app.opened;

    open_file(&mut app, 1);

    assert_eq!(app.opened, before + 1);
}

// ---- filter state changes ---------------------------------------------

/// A two-line file with one including filter on `beta`, added by keys.
fn app_filtering(name: &str) -> App<'static> {
    capture::install();
    let mut app = app_over_file(name, "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    app
}

/// Each change is one `Debug` record that names the change and gives the
/// state after it.
fn assert_one_change(app_name: &str, change: &str, act: impl FnOnce(&mut App<'static>)) {
    let mut app = app_filtering(app_name);
    let before = logged(log::Level::Debug, change).len();
    act(&mut app);
    let records = logged(log::Level::Debug, change);
    assert_eq!(records.len(), before + 1, "{change}: {:?}", capture::here());
    let record = records.last().expect("one record");
    assert!(record.starts_with("filters: "), "{record}");
    assert!(record.contains("; now "), "no state in {record}");
}

#[test]
fn adding_a_filter_is_logged_with_its_pattern() {
    assert_one_change("log_add", "add include \"gamma\"", |app| {
        key(app, KeyCode::Char('f'));
        key(app, KeyCode::Char('i'));
        typed(app, "gamma");
        key(app, KeyCode::Enter);
    });
    assert_one_change("log_exclude", "add exclude \"delta\"", |app| {
        key(app, KeyCode::Char('f'));
        key(app, KeyCode::Char('x'));
        typed(app, "delta");
        key(app, KeyCode::Enter);
    });
}

#[test]
fn editing_toggling_and_deleting_a_filter_are_logged() {
    let index = |app: &App| {
        app.filters
            .filters()
            .iter()
            .position(|filter| filter.predicate.display() == "beta")
            .expect("the beta filter")
    };
    assert_one_change("log_edit", "edit \"beta\" to \"bet\"", |app| {
        let at = index(app);
        app.replace_filter(at, "bet").expect("valid");
    });
    assert_one_change("log_toggle", "toggle \"beta\" off", |app| {
        let at = index(app);
        app.apply_filter_command(widgets::FilterCommand::Toggle(at));
    });
    assert_one_change("log_toggle_numbered", "toggle \"beta\" off", |app| {
        typed(app, "01");
    });
    assert_one_change("log_delete", "delete \"beta\"", |app| {
        let at = index(app);
        app.apply_filter_command(widgets::FilterCommand::Delete(at));
    });
}

#[test]
fn bang_peek_and_hide_mode_are_logged() {
    assert_one_change("log_bang", "! disable all", |app| {
        key(app, KeyCode::Char('!'));
    });
    assert_one_change("log_hide", "hide mode on", |app| {
        key(app, KeyCode::Char('H'));
    });
    assert_one_change("log_peek", "peek on", |app| {
        key(app, KeyCode::Char(' '));
    });
}

#[test]
fn listing_and_unlisting_a_set_are_logged() {
    capture::install();
    let mut app = app_with_three_sets("log_unlist");
    key(&mut app, KeyCode::Char('L'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        logged(log::Level::Debug, "filters: unlist set \"a\"").len(),
        1,
        "{:?}",
        capture::here()
    );
}

// ---- scan passes --------------------------------------------------------

/// A pass starts with one `Debug` record: the directory, the files, how
/// many the scan cache answered and how many go to the worker (#491).
#[test]
fn a_scan_pass_logs_its_start_with_the_cache_hits() {
    capture::install();
    let mut app = app_over_files("log_scan_start", &[("a.log", "a\n"), ("b.log", "b\n")]);
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid");
    let dir = app.explorer.dir().display().to_string();
    let started = |app: &mut App| {
        app.refresh_scan(true);
        logged(log::Level::Debug, &format!("scan pass started in {dir}: "))
            .pop()
            .expect("a start record")
    };

    assert!(
        started(&mut app).ends_with(": 2 files, 0 from the scan cache, 2 to scan"),
        "{:?}",
        capture::here()
    );

    tx.send(scanned(&app, 0, vec![], true)).expect("send");
    app.drain_scan_results();

    let record = started(&mut app);
    assert!(
        record.ends_with(": 2 files, 1 from the scan cache, 1 to scan"),
        "{record}"
    );
}

// ---- navigation ---------------------------------------------------------

/// Each move to another directory is one `Debug` record (#491).
#[test]
fn a_change_of_directory_is_logged() {
    capture::install();
    let (mut app, _dir) = app_over_nested("log_nav_dir");
    draw(&mut app);
    let moves = || logged(log::Level::Debug, "navigation: directory ").len();
    let before = moves();

    key(&mut app, KeyCode::Char('l'));

    assert_eq!(moves(), before + 1, "{:?}", capture::here());
    let sub = app.explorer.dir().display().to_string();
    assert!(sub.ends_with("log_nav_dir/sub"), "{sub}");
    assert_eq!(
        logged(log::Level::Debug, &format!("navigation: directory {sub}")).len(),
        1
    );
}

/// A file opened is one `Debug` record with its path.
#[test]
fn a_file_opened_is_logged() {
    capture::install();
    let mut app = app_over_files("log_nav_open", &[("a.log", "a\n"), ("b.log", "b\n")]);
    let path = app.explorer.files()[1].1.display().to_string();

    open_file(&mut app, 1);

    let records = logged(log::Level::Debug, &format!("navigation: open {path}"));
    assert_eq!(records.len(), 1, "{:?}", capture::here());
}

/// `r` is one `Debug` record with the file it reads again.
#[test]
fn a_reload_is_logged() {
    capture::install();
    let mut app = app_over_file("log_nav_reload", "alpha\n");
    let file = app.view.filename().to_path_buf();

    key(&mut app, KeyCode::Char('r'));

    let records = logged(
        log::Level::Debug,
        &format!("navigation: reload {}", file.display()),
    );
    assert_eq!(records.len(), 1, "{:?}", capture::here());
}

/// A file the stamp check finds changed on disk is one `Debug` record.
#[test]
fn a_file_changed_on_disk_is_logged() {
    capture::install();
    let mut app = app_over_files("log_nav_moved", &[("a.log", "a\n")]);
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid");
    app.refresh_scan(true);
    tx.send(scanned(&app, 0, vec![], true)).expect("send");
    app.drain_scan_results();
    let path = app.explorer.files()[0].1.clone();
    fs::write(&path, "a\nmore\n").expect("grow the file");

    app.check_stamps();

    let records = logged(
        log::Level::Debug,
        &format!("navigation: {} changed on disk", path.display()),
    );
    assert_eq!(records.len(), 1, "{:?}", capture::here());
    assert!(records[0].ends_with("(it grew)"), "{records:?}");
}

// ---- keys and actions ---------------------------------------------------

/// Each key is one `Trace` record that names the key and the action it
/// dispatched.
#[test]
fn a_key_is_logged_with_its_action() {
    capture::install();
    let mut app = app_over_file("log_key_action", "alpha\n");

    key(&mut app, KeyCode::Tab);

    let records = logged(log::Level::Trace, "key Tab: ");
    assert_eq!(
        records,
        ["key Tab: global.focus.next"],
        "{:?}",
        capture::here()
    );
}

/// A key that no binding takes says so.
#[test]
fn a_key_that_does_nothing_is_logged_as_no_action() {
    capture::install();
    let mut app = app_over_file("log_key_none", "alpha\n");

    app.handle_event(event::Event::Key(KeyEvent::new(
        KeyCode::Char('%'),
        KeyModifiers::ALT,
    )));

    let records = logged(log::Level::Trace, "key Alt-%: ");
    assert_eq!(records, ["key Alt-%: no action"], "{:?}", capture::here());
}

/// A character typed into a prompt is search text, and the log does not
/// contain search text: the record names neither the character nor an
/// action, only that a character went into the text.
#[test]
fn a_character_typed_into_a_prompt_is_not_logged() {
    let app = app_filtering("log_key_typed");

    assert_eq!(
        logged(log::Level::Trace, "key "),
        [
            "key f: global.focus.filters",
            "key i: filters.include",
            "key (typed into a text field): no action",
            "key (typed into a text field): no action",
            "key (typed into a text field): no action",
            "key (typed into a text field): no action",
            "key Enter: prompt.commit",
        ],
        "{:?}",
        app.focus
    );
}

/// A key is named as `[keymap]` spells it.
#[test]
fn a_key_is_named_as_the_keymap_spells_it() {
    capture::install();
    let mut app = app_over_file("log_key_label", "alpha\n");

    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::F(5));

    assert_eq!(
        logged(log::Level::Trace, "key Shift-Tab: ").len(),
        1,
        "{:?}",
        capture::here()
    );
    assert_eq!(
        logged(log::Level::Trace, "key F5: ").len(),
        1,
        "{:?}",
        capture::here()
    );
}

// ---- the clipboard ------------------------------------------------------

/// A yank over two lines, through `clipboard`, from the file view.
fn yank_two_lines(name: &str, clipboard: RecordingClipboard) {
    capture::install();
    let mut app = app_over_file(name, "alpha\nbeta\n");
    app.clipboard = Box::new(clipboard);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('y'));
}

/// A copy is one `Debug` record with its size, and not the text.
#[test]
fn a_copy_is_logged_with_its_size() {
    yank_two_lines("log_clipboard_copy", RecordingClipboard::default());

    let records = logged(log::Level::Debug, "clipboard: ");
    assert_eq!(
        records,
        ["clipboard: copied 2 lines, 11 bytes"],
        "{:?}",
        capture::here()
    );
    assert!(
        capture::here()
            .iter()
            .all(|(_, text)| !text.contains("alpha"))
    );
}

/// A copy that fails is a `Warn` record with the reason.
#[test]
fn a_copy_that_fails_is_a_warning() {
    yank_two_lines(
        "log_clipboard_fail",
        RecordingClipboard::failing("pbcopy: not found"),
    );

    let records = logged(log::Level::Warn, "clipboard: ");
    assert_eq!(
        records,
        ["clipboard: the copy of 2 lines, 11 bytes failed: pbcopy: not found"],
        "{:?}",
        capture::here()
    );
}
