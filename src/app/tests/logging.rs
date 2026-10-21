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
