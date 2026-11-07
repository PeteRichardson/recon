//! `>` and `<` (#609, ADR 0017): reveal the hidden lines up to the next
//! visible line in hide mode, and hide them again.

use super::palette::run;
use super::*;
use crate::app::reveals::REVEAL_IN_DIM_MODE;

const GAPS: &str = "hit a\nx\ny\nhit b\nz\n";

/// `body` in the focused view, with `hit` as a filter and hide mode on.
fn app_hiding(name: &str, body: &str) -> App<'static> {
    let mut app = app_over_file(name, body);
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("hit").expect("valid");
    app.refresh_view();
    key(&mut app, KeyCode::Char('u'));
    app
}

#[test]
fn greater_than_reveals_the_gap_and_less_than_hides_it_again() {
    let mut app = app_hiding("reveal_gap", GAPS);
    assert_eq!(view_lines(&app), ["hit a", "hit b"]);

    key(&mut app, KeyCode::Char('>'));
    assert_eq!(view_lines(&app), ["hit a", "x", "y", "hit b"]);

    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(cursor_source(&app), 2);
    key(&mut app, KeyCode::Char('<'));
    assert_eq!(view_lines(&app), ["hit a", "hit b"]);
    assert_eq!(cursor_source(&app), 0, "the cursor goes to the first line");
}

#[test]
fn in_dim_mode_the_keys_do_nothing_and_say_why() {
    let mut app = app_hiding("reveal_dim", GAPS);
    key(&mut app, KeyCode::Char('u'));
    let shown = view_lines(&app);
    for code in ['>', '<'] {
        key(&mut app, KeyCode::Char(code));
        assert_eq!(view_lines(&app), shown);
        assert!(status_line(&mut app).contains(REVEAL_IN_DIM_MODE));
    }
}

#[test]
fn the_palette_closes_every_reveal() {
    let mut app = app_hiding("reveal_close_all", "hit\na\nhit\nb\nhit\n");
    key(&mut app, KeyCode::Char('>'));
    key(&mut app, KeyCode::Char('G'));
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(cursor_source(&app), 2);
    key(&mut app, KeyCode::Char('>'));
    assert_eq!(view_lines(&app).len(), 5);

    run(&mut app, "view.reveal.close.all");
    assert_eq!(view_lines(&app), ["hit", "hit", "hit"]);
}

/// `:w` writes what the user reads: the revealed lines too (ADR 0017).
#[test]
fn w_writes_the_revealed_lines() {
    let mut app = app_hiding("reveal_write", GAPS);
    key(&mut app, KeyCode::Char('>'));
    run(&mut app, "w out.txt");
    let out = app.explorer.dir().join("out.txt");
    let written = std::fs::read_to_string(out).expect("the file was written");
    assert_eq!(written, "hit a\nx\ny\nhit b\n");
}

/// `r` on a file that has not changed keeps its reveals; on a file that
/// has, it removes them, since their rows may be other lines now.
#[test]
fn a_reload_keeps_the_reveals_unless_the_file_changed() {
    let mut app = app_hiding("reveal_reload", GAPS);
    key(&mut app, KeyCode::Char('>'));
    key(&mut app, KeyCode::Char('r'));
    assert_eq!(view_lines(&app), ["hit a", "x", "y", "hit b"]);

    let path = fixture_dir_path("reveal_reload").join("log.txt");
    std::fs::write(&path, "hit a\nx\ny\nhit b\nz\nmore\n").expect("writable");
    key(&mut app, KeyCode::Char('r'));
    assert_eq!(view_lines(&app), ["hit a", "hit b"]);
}

/// Each file keeps its reveals for the session: a visit to another file
/// and back finds them where they were.
#[test]
fn a_file_keeps_its_reveals_while_another_file_is_open() {
    let mut app = app_hiding("reveal_away", GAPS);
    let dir = fixture_dir_path("reveal_away");
    let other = dir.join("other.txt");
    std::fs::write(&other, "hit\nplain\n").expect("writable");
    key(&mut app, KeyCode::Char('>'));

    app.perform_widget_action(Action::Load(other));
    assert_eq!(view_lines(&app), ["hit"], "the other file has no reveal");
    app.perform_widget_action(Action::Load(dir.join("log.txt")));
    assert_eq!(view_lines(&app), ["hit a", "x", "y", "hit b"]);
}
