//! `-` in the TUI (#517): stdin copied into a file, followed as it grows.
//!
//! Stdin is a real pipe the test writes into, and the copy runs on its own
//! thread, so each step waits, briefly, for what the thread has done.

use super::*;
use crate::stdin::StdinFile;
use std::io::Write as _;
use std::time::Instant;

/// An app over `-`, as `main` builds it, with stdin a pipe the test holds
/// the writing end of. Drawn once so the view knows its height.
fn app_over_stdin(name: &str) -> (App<'static>, std::io::PipeWriter, std::path::PathBuf) {
    let (reader, writer) = std::io::pipe().expect("a pipe");
    let file = StdinFile::start(&fixture_dir(name), reader, || {}, false).expect("started");
    let path = file.path().to_path_buf();
    let mut config = Config::default();
    config.read_stdin_into(&path);
    let mut app = App::new(&Startup::from(config)).with_stdin(Some(file));
    draw(&mut app);
    (app, writer, path)
}

/// Take what the copy and the tail did until `done`, or fail after 10 s.
fn until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
    let start = Instant::now();
    while !done(app) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(5));
        app.drain_views();
        app.poll_follow();
        app.poll_stdin();
    }
}

#[test]
fn lines_come_into_the_view_and_the_title_says_whether_stdin_is_open() {
    let (mut app, mut writer, _) = app_over_stdin("stdin_title_app");

    writer.write_all(b"first\nsecond\n").expect("write");
    until(&mut app, |app| app.document.lines() == ["first", "second"]);
    assert!(rendered(&mut app).contains("<stdin> (open)"), "{}", rendered(&mut app));

    drop(writer);
    until(&mut app, |app| rendered(app).contains("<stdin> (closed)"));
}

/// The explorer starts hidden, and `e` shows it on the folder the file is
/// in, with the file selected.
#[test]
fn the_explorer_starts_hidden_and_e_shows_the_file_in_its_folder() {
    let (mut app, _writer, path) = app_over_stdin("stdin_explorer");
    assert!(!app.panes.is_shown(Focus::Explorer));

    key(&mut app, KeyCode::Char('e'));

    assert!(app.panes.is_shown(Focus::Explorer));
    let name = path.file_name().expect("a name").to_string_lossy();
    assert_eq!(selected_name(&app), name);
}

/// Passing 1 MB is said on the status line.
#[test]
fn a_size_milestone_is_reported_on_the_status_line() {
    let (mut app, mut writer, _) = app_over_stdin("stdin_milestone");

    writer.write_all(&vec![b'x'; 1_000_001]).expect("write");

    until(&mut app, |app| status(app).is_some_and(|text| text.contains("1.0 MB")));
    assert_eq!(status(&app), Some("<stdin>: 1.0 MB so far"));
}
