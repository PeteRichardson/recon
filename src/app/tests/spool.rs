//! A command view's output past the spool threshold (#573): copied into a
//! file as the command writes it, and followed as a growing file is.
//!
//! The threshold is one byte here, so that a command of a few lines takes
//! the spool path. The command and the copy run on threads of their own,
//! so each step waits, briefly, for what they have done.

use super::*;
use crate::views::FileType;
use crate::views::command::{Command, Commands};
use std::time::Instant;

/// An app over `a.txt` and `b.txt`, whose view is the command `argv`, with
/// a spool threshold of one byte, on `a.txt`.
fn app_over_command(name: &str, argv: &[&str]) -> (App<'static>, std::path::PathBuf) {
    app_over_command_with(name, argv, |_| {})
}

/// `app_over_command`, with `setup` done before the view of `a.txt` runs.
fn app_over_command_with(
    name: &str,
    argv: &[&str],
    setup: impl FnOnce(&mut App),
) -> (App<'static>, std::path::PathBuf) {
    let dir = fixture_dir(name);
    fs::write(dir.join("a.txt"), "x\n").expect("write fixture");
    fs::write(dir.join("b.txt"), "plain\n").expect("write fixture");
    let mut app = App::new(&Startup::from(Config {
        path: Some(dir.join("placeholder").display().to_string()),
        filetypes: vec![FileType::with_view_names("a.txt", &["gen"])],
        commands: Commands::of([("gen", Command::of(argv))]),
        ..Config::default()
    }));
    app.loading.spooling.bytes = 1;
    app.loading.spooling.dir = dir.join("spools");
    fs::create_dir(&app.loading.spooling.dir).expect("spool dir");
    setup(&mut app);
    draw(&mut app);
    arrow_to(&mut app, "a.txt");
    focus_file_view(&mut app);
    (app, dir)
}

/// Take what the command, the copy and the tail did until `done`, or fail
/// after 10 s.
fn until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
    let start = Instant::now();
    while !done(app) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(5));
        app.drain_views();
        app.poll_follow();
        app.poll_spool();
    }
}

#[test]
fn lines_come_while_the_command_runs_and_the_status_line_counts_them() {
    let (mut app, _) = app_over_command(
        "spool_lines",
        &["sh", "-c", "echo one; echo two; sleep 0.5; echo three"],
    );

    until(&mut app, |app| *app.document.lines() == ["one", "two"]);
    let line = status_line(&mut app);
    assert!(line.contains(" RUNNING · 2 lines "), "{line}");

    until(&mut app, |app| {
        *app.document.lines() == ["one", "two", "three"]
    });
    until(&mut app, |app| !status_line(app).contains("RUNNING"));
}

/// `Ctrl-c` while the command runs stops it and keeps its lines, as in a
/// shell. The status line says so. The next `Ctrl-c` quits (#382).
#[test]
fn ctrl_c_stops_the_command_and_keeps_its_lines() {
    let (mut app, _) = app_over_command("spool_ctrl_c", &["sh", "-c", "echo one; sleep 10"]);
    until(&mut app, |app| *app.document.lines() == ["one"]);

    ctrl(&mut app, KeyCode::Char('c'));
    assert!(!matches!(app.state, AppState::Cancelled), "Ctrl-c quit");
    until(&mut app, |app| !status_line(app).contains("RUNNING"));

    assert_eq!(*app.document.lines(), ["one"]);
    assert_eq!(status(&app), Some("gen stopped: by the user"));
    ctrl(&mut app, KeyCode::Char('c'));
    assert!(
        matches!(app.state, AppState::Cancelled),
        "Ctrl-c did not quit"
    );
}

/// The spool is for the file on screen: a move to another file kills the
/// command and deletes the spool.
#[test]
fn leaving_the_file_deletes_its_spool() {
    let (mut app, dir) = app_over_command("spool_leave", &["sh", "-c", "echo one; sleep 10"]);
    until(&mut app, |app| *app.document.lines() == ["one"]);
    let spools = dir.join("spools");
    assert_eq!(fs::read_dir(&spools).unwrap().count(), 1, "no spool");

    arrow_to(&mut app, "b.txt");

    assert_eq!(*app.document.lines(), ["plain"]);
    assert_eq!(
        fs::read_dir(&spools).unwrap().count(),
        0,
        "the spool was left"
    );
    assert!(!status_line(&mut app).contains("RUNNING"));
}

/// The command stops when the user moves, not when the next view comes:
/// that view can be slow too.
#[test]
fn the_spool_goes_before_the_next_view_comes() {
    let (mut app, dir) = app_over_command("spool_leave_late", &["sh", "-c", "echo one; sleep 10"]);
    until(&mut app, |app| *app.document.lines() == ["one"]);
    let held = crate::views::pool::double::Held::default();
    app.loading.executor = Box::new(held.clone());

    arrow_to(&mut app, "b.txt");

    assert_eq!(fs::read_dir(dir.join("spools")).unwrap().count(), 0);
    held.release();
}

/// When the command is over, the spool is read again whole: from its index
/// when it is large (#51), so the lines are not in memory. The cursor keeps
/// its line.
#[test]
fn a_large_spool_is_read_again_from_its_index_at_the_end() {
    let (mut app, _) = app_over_command_with(
        "spool_index",
        &["sh", "-c", "echo one; echo two; sleep 0.5; echo three"],
        |app| app.loading.index_bytes = 1,
    );
    until(&mut app, |app| *app.document.lines() == ["one", "two"]);
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(cursor_source(&app), 0);
    assert!(!app.document.lines().is_index());

    until(&mut app, |app| !status_line(app).contains("RUNNING"));

    assert!(app.document.lines().is_index(), "not read from its index");
    assert_eq!(*app.document.lines(), ["one", "two", "three"]);
    assert_eq!(view_lines(&app), ["one", "two", "three"]);
    assert_eq!(cursor_source(&app), 0);
}
