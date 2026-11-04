//! A large file read from its index (#51): the view reads its text from
//! the file again, and a file that became shorter is read again from the
//! start.
//!
//! The index threshold is one byte here, so that a fixture of a few lines
//! takes the large-file path.

use super::*;
use crate::app::loading::Landing;
use crate::views::Extent;

/// An app showing `body` from its index, focused on the view.
fn app_over_index(name: &str, body: &str) -> (App<'static>, std::path::PathBuf) {
    let path = fixture_path(name, body);
    let mut app = App::new(&Startup::from(Config {
        path: Some(path.display().to_string()),
        ..Config::default()
    }));
    app.loading.index_bytes = 1;
    app.open_view(&path, Extent::Full, Landing::Top);
    draw(&mut app);
    focus_file_view(&mut app);
    (app, path)
}

#[test]
fn a_large_file_is_shown_from_its_index() {
    let (app, _) = app_over_index("index_shown", "one\ntwo\nthree\n");
    assert!(app.document.lines().is_index());
    assert_eq!(view_lines(&app), ["one", "two", "three"]);
}

/// logrotate's copytruncate: the next read finds the file shorter, and
/// recon reads it again, as follow mode does. The cursor keeps its line,
/// or the nearest one.
#[test]
fn a_large_file_that_became_shorter_is_read_again() {
    let (mut app, path) = app_over_index("index_shorter", &numbered_lines(20));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(cursor_source(&app), 2);

    fs::write(&path, "new 1\nnew 2\nnew 3\nnew 4\n").expect("truncate");
    // A filter reads every line again.
    app.add_filter("new").expect("valid pattern");
    app.refresh_view();
    assert!(app.document.lines().changed());
    assert!(app.poll_changed());

    assert_eq!(*app.document.lines(), ["new 1", "new 2", "new 3", "new 4"]);
    assert!(!app.document.lines().changed());
    assert_eq!(cursor_source(&app), 2);
    let status = status_line(&mut app);
    assert!(
        status.contains("became shorter or was replaced: read again from the start"),
        "{status}"
    );
    assert!(!app.poll_changed(), "one change, one read");
}
