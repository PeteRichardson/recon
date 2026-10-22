//! Follow mode (#493): `Ctrl-T` on a growing file.
//!
//! The tail runs in the test's own thread here: `poll_follow` reads the file
//! when the test calls it, so each step sees exactly what was written.

use super::*;

/// Add `text` to the end of `path`, as a program writing a log does.
fn grow(path: &std::path::Path, text: &str) {
    crate::fixtures::append(path, text.as_bytes());
}

/// An app showing `body` in `<name>/log.txt`, focused on the view, drawn
/// once so the view knows its height.
fn app_following(name: &str, body: &str) -> (App<'static>, std::path::PathBuf) {
    let path = fixture_path(name, body);
    let mut app = App::new(&Startup::from(Config {
        path: Some(path.display().to_string()),
        ..Config::default()
    }));
    draw(&mut app);
    focus_file_view(&mut app);
    (app, path)
}

#[test]
fn ctrl_t_shows_new_lines_and_keeps_the_last_one_in_view() {
    let (mut app, path) = app_following("follow_basic", "one\ntwo\n");
    key(&mut app, KeyCode::Char('G'));
    ctrl(&mut app, KeyCode::Char('t'));
    assert!(status_line(&mut app).contains(" FOLLOW "));

    grow(&path, "three\nfour\n");
    assert!(app.poll_follow());

    assert_eq!(app.document.lines(), ["one", "two", "three", "four"]);
    assert_eq!(cursor_source(&app), 3);
    assert!(view_lines(&app).contains(&"four".to_string()));
}

#[test]
fn moving_the_cursor_up_pauses_autoscroll_and_g_resumes_it() {
    let (mut app, path) = app_following("follow_pause", &numbered_lines(20));
    key(&mut app, KeyCode::Char('G'));
    ctrl(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(cursor_source(&app), 18);

    grow(&path, "new 1\nnew 2\nnew 3\n");
    app.poll_follow();

    assert_eq!(cursor_source(&app), 18, "the cursor stays where it was");
    assert!(
        status_line(&mut app).contains(" FOLLOW · 3 new lines "),
        "{}",
        status_line(&mut app)
    );
    key(&mut app, KeyCode::Char('G'));
    assert_eq!(cursor_source(&app), 22);
    assert!(!status_line(&mut app).contains("new lines"));

    grow(&path, "new 4\n");
    app.poll_follow();
    assert_eq!(cursor_source(&app), 23, "autoscroll is on again");
}

/// In hide mode the last line is the last visible one, and a new line the
/// filters hide does not move the view.
#[test]
fn new_lines_are_filtered_and_hidden_ones_do_not_scroll() {
    let (mut app, path) = app_following("follow_filtered", "ERROR one\nfine\nERROR two\n");
    app.add_filter("ERROR").expect("valid pattern");
    app.set_mode(Mode::FilteredOnly);
    app.refresh_view();
    key(&mut app, KeyCode::Char('G'));
    ctrl(&mut app, KeyCode::Char('t'));

    grow(&path, "fine again\nERROR three\nfine at the end\n");
    app.poll_follow();

    assert_eq!(app.document.visible(), &[0, 2, 4]);
    assert_eq!(cursor_source(&app), 4);
    assert_eq!(app.document.verdicts()[4], Verdict::Included(0));
}

#[test]
fn follow_mode_belongs_to_the_file_and_the_explorer_marks_each_one() {
    let mut app = app_over_files(
        "follow_two",
        &[("a.log", "a\n"), ("b.log", "b\n"), ("c.log", "c\n")],
    );
    draw(&mut app);
    open_file(&mut app, 0);
    ctrl(&mut app, KeyCode::Char('t'));
    open_file(&mut app, 1);
    assert!(
        !status_line(&mut app).contains(" FOLLOW "),
        "b.log is not followed"
    );
    ctrl(&mut app, KeyCode::Char('t'));
    open_file(&mut app, 2);
    assert!(!status_line(&mut app).contains(" FOLLOW "));

    arrow_to(&mut app, "a.log");
    assert!(status_line(&mut app).contains(" FOLLOW "), "back on a.log");
    let text = rendered(&mut app);
    assert!(
        text.contains(&format!("a.log{}", crate::widgets::explorer::FOLLOW_MARK)),
        "{text}"
    );
    assert!(
        text.contains(&format!("b.log{}", crate::widgets::explorer::FOLLOW_MARK)),
        "{text}"
    );
    assert!(
        !text.contains(&format!("c.log{}", crate::widgets::explorer::FOLLOW_MARK)),
        "{text}"
    );

    ctrl(&mut app, KeyCode::Char('t'));
    assert!(
        !status_line(&mut app).contains(" FOLLOW "),
        "a second Ctrl-t stops it"
    );
}

#[test]
fn a_truncated_file_is_read_again_and_stays_followed() {
    let (mut app, path) = app_following("follow_truncate", "one\ntwo\nthree\n");
    key(&mut app, KeyCode::Char('G'));
    ctrl(&mut app, KeyCode::Char('t'));

    fs::write(&path, "fresh\n").expect("truncate");
    app.poll_follow();
    assert_eq!(app.document.lines(), ["fresh"]);
    assert!(message(&app).is_some_and(|text| text.contains("read again from the start")));
    assert!(status_line(&mut app).contains(" FOLLOW "));

    grow(&path, "after\n");
    app.poll_follow();
    assert_eq!(app.document.lines(), ["fresh", "after"]);
    assert_eq!(cursor_source(&app), 1);
}

/// The scan sees the followed file change, and updates its mark; the view
/// is current, so there is no badge.
#[test]
fn a_followed_file_never_shows_the_changed_on_disk_badge() {
    let mut app = app_over_files("follow_no_badge", &[("a.log", "ERROR one\n")]);
    let (_scanner, tx) = record_scans(&mut app);
    draw(&mut app);
    open_file(&mut app, 0);
    app.add_filter("ERROR").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0b1], true)).expect("send");
    app.drain_scan_results();
    ctrl(&mut app, KeyCode::Char('t'));

    let path = app.view.filename().to_path_buf();
    grow(&path, "ERROR two\n");
    app.poll_follow();
    app.check_stamps();

    assert_eq!(app.document.lines().len(), 2);
    assert!(!status_line(&mut app).contains("changed on disk"));
}

#[test]
fn ctrl_t_in_the_hex_view_says_follow_mode_needs_the_text_view() {
    let (mut app, _) = app_following("follow_hex", "one\n");
    key(&mut app, KeyCode::Char('-'));
    ctrl(&mut app, KeyCode::Char('t'));
    assert_eq!(message(&app), Some(crate::app::follow::TEXT_VIEW_ONLY));
    assert!(!status_line(&mut app).contains(" FOLLOW "));
}

/// From the explorer: the file previewed is followed, and read whole,
/// so its tail starts at its real end and not at the preview's.
#[test]
fn ctrl_t_in_the_explorer_follows_the_previewed_file_read_whole() {
    let total = crate::views::read::PREVIEW_LINES + 5;
    let mut app = app_over_files(
        "follow_from_explorer",
        &[("a.log", &numbered_lines(total)), ("b.log", "b\n")],
    );
    draw(&mut app);
    arrow_to(&mut app, "a.log");
    assert_eq!(app.focus, Focus::Explorer);
    assert!(app.view.is_truncated(), "the explorer shows a preview");
    ctrl(&mut app, KeyCode::Char('t'));
    assert!(!app.view.is_truncated());
    assert!(status_line(&mut app).contains(" FOLLOW "));

    let path = app.view.filename().to_path_buf();
    grow(&path, "new\n");
    app.poll_follow();
    assert_eq!(app.document.lines().len(), total + 1);
    assert_eq!(app.document.lines()[total], "new");

    // Away and back: a followed file is read whole, not previewed.
    arrow_to(&mut app, "b.log");
    arrow_to(&mut app, "a.log");
    assert!(!app.view.is_truncated());
}

#[test]
fn the_follow_flag_starts_follow_mode_on_the_file() {
    let path = fixture_path("follow_flag", "one\n");
    let mut app = App::new(&Startup::from(Config {
        path: Some(path.display().to_string()),
        follow: true,
        ..Config::default()
    }));
    draw(&mut app);
    assert!(status_line(&mut app).contains(" FOLLOW "));
    grow(&path, "two\n");
    app.poll_follow();
    assert_eq!(app.document.lines(), ["one", "two"]);
}
