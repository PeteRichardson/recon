//! A filter pass in blocks (#597), for a file at or above the line
//! threshold. The tests set the threshold to 0 and the block to 4 lines.
//!
//! The pass runs in the test's own thread here: each `poll_filtering`
//! takes one block, so each step sees exactly one more block.

use super::*;

/// An app showing `body` in `<name>/log.txt` in hide mode, drawn once so
/// the view knows its height, with every filter change a pass in blocks.
fn app_in_blocks(name: &str, body: &str) -> App<'static> {
    let mut app = app_over_file(name, body);
    app.filtering.threshold = 0;
    app.filtering.block = 4;
    app.set_mode(document::Mode::FilteredOnly);
    draw(&mut app);
    focus_file_view(&mut app);
    app
}

fn include(app: &mut App, pattern: &str) {
    key(app, KeyCode::Char('f'));
    key(app, KeyCode::Char('i'));
    typed(app, pattern);
    key(app, KeyCode::Enter);
}

/// Take blocks until the pass ends, here or on a worker.
fn finish_pass(app: &mut App) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while app.filtering.is_running() {
        assert!(std::time::Instant::now() < deadline, "the pass did not end");
        app.poll_filtering();
    }
}

/// "hit 0", "plain 1", "hit 2", ...: a hit on each even line.
fn every_other(n: usize) -> String {
    (0..n).fold(String::new(), |mut body, i| {
        let word = if i % 2 == 0 { "hit" } else { "plain" };
        let _ = writeln!(body, "{word} {i}");
        body
    })
}

#[test]
fn rows_show_as_their_blocks_come_with_progress() {
    let mut app = app_in_blocks("pass_blocks", &every_other(10));

    include(&mut app, "^hit");
    assert!(app.document.visible().is_empty());
    assert!(
        status_line(&mut app).contains("filtering 0%"),
        "{}",
        status_line(&mut app)
    );

    app.poll_filtering();
    assert_eq!(app.document.visible(), &[0, 2]);
    assert!(
        status_line(&mut app).contains("filtering 40%"),
        "{}",
        status_line(&mut app)
    );

    finish_pass(&mut app);
    assert_eq!(app.document.visible(), &[0, 2, 4, 6, 8]);
    assert!(!status_line(&mut app).contains("filtering"));
}

/// The verdicts and rows one `evaluate` gives with the app's filters now.
fn judged_whole(app: &App) -> (Vec<Verdict>, Vec<usize>) {
    let mut whole = document::Document::for_file(app.view.filename(), app.document.lines().clone());
    whole.set_mode(app.document.mode());
    whole.evaluate(&app.filters);
    (whole.verdicts().to_vec(), whole.visible().iter().collect())
}

/// Three filter changes before any block comes: the view ends with the
/// verdicts of the last, whether the pass runs here or on a worker.
#[test]
fn a_fast_series_of_filter_changes_gives_the_last_verdicts() {
    for (on_worker, name) in [(false, "pass_series_here"), (true, "pass_series_worker")] {
        let mut app = app_in_blocks(name, &every_other(30));
        app.filtering.on_worker = on_worker;

        include(&mut app, "^hit");
        include(&mut app, "plain 1");
        key(&mut app, KeyCode::Char('!'));
        key(&mut app, KeyCode::Char('!'));
        include(&mut app, " 2");
        finish_pass(&mut app);

        let (verdicts, visible) = judged_whole(&app);
        assert_eq!(
            app.document.verdicts(),
            verdicts,
            "on a worker: {on_worker}"
        );
        assert_eq!(app.document.visible().iter().collect::<Vec<_>>(), visible);
    }
}

/// The cursor's line is below the rows judged: the cursor goes to the last
/// row shown, and down with each block, until a block judges its line.
#[test]
fn the_cursor_follows_the_blocks_to_its_line() {
    let mut app = app_in_blocks("pass_cursor", &every_other(40));
    app.place_cursor_on_visible_row(20);
    assert_eq!(cursor_source(&app), 20);

    app.add_filter("^hit").expect("valid");
    app.poll_filtering();
    assert_eq!(cursor_source(&app), 2, "the last row of the first block");
    app.poll_filtering();
    assert_eq!(cursor_source(&app), 6);
    for _ in 0..4 {
        app.poll_filtering();
    }
    assert_eq!(cursor_source(&app), 20, "a block judged its line");
    finish_pass(&mut app);
    assert_eq!(cursor_source(&app), 20);
}

/// A cursor the user moves while the pass runs stays where the user put it.
#[test]
fn a_cursor_the_user_moves_stays() {
    let mut app = app_in_blocks("pass_cursor_moved", &every_other(40));
    app.place_cursor_on_visible_row(20);

    app.add_filter("^hit").expect("valid");
    app.poll_filtering();
    app.poll_filtering();
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(cursor_source(&app), 4);
    finish_pass(&mut app);
    assert_eq!(cursor_source(&app), 4);
}

/// `f i` from the view ends with `n` (#120 §14). While the pass runs, the
/// `n` waits for a block to judge the next interesting line.
#[test]
fn the_n_after_f_i_waits_for_its_line() {
    let mut app = app_in_blocks("pass_chain_step", &every_other(40));
    app.place_cursor_on_visible_row(20);

    include(&mut app, "^hit");
    assert!(!status_line(&mut app).contains("no interesting line"));
    finish_pass(&mut app);
    assert_eq!(cursor_source(&app), 22, "the next hit after line 20");
}

/// Lines a followed file gets while a pass runs wait for the pass to end,
/// then get their verdicts as follow mode gives them.
#[test]
fn followed_lines_wait_for_the_pass() {
    let mut app = app_in_blocks("pass_follow", &every_other(10));
    let path = app.view.filename().to_path_buf();
    ctrl(&mut app, KeyCode::Char('t'));

    app.add_filter("^hit").expect("valid");
    crate::fixtures::append(&path, b"hit 10\nplain 11\n");
    app.poll_follow();
    assert_eq!(app.document.lines().len(), 10, "held until the pass ends");

    finish_pass(&mut app);
    assert_eq!(app.document.lines().len(), 12);
    let (verdicts, visible) = judged_whole(&app);
    assert_eq!(app.document.verdicts(), verdicts);
    assert_eq!(app.document.visible().iter().collect::<Vec<_>>(), visible);
}

/// `q` under `--emit lines` while a pass runs emits the lines of the whole
/// file, not only the rows judged so far.
#[test]
fn q_during_a_pass_emits_every_shown_line() {
    let mut app = app_in_blocks("pass_quit", &every_other(10));
    app.emit = Some(emit::Emit::Lines);
    app.add_filter("^hit").expect("valid");
    app.poll_filtering();

    key(&mut app, KeyCode::Char('q'));
    let emit::Exit::Emit { lines, .. } = app.exit() else {
        panic!("the session emitted no lines");
    };
    let lines: Vec<_> = lines
        .into_iter()
        .map(|line| String::from_utf8(line).expect("utf-8"))
        .collect();
    assert_eq!(lines, ["hit 0", "hit 2", "hit 4", "hit 6", "hit 8"]);
}

/// `n` while a pass runs, with no interesting line in the rows judged
/// after the cursor, waits for a block to judge one.
#[test]
fn n_during_a_pass_waits_for_its_line() {
    let mut app = app_in_blocks("pass_n", &every_other(40));
    app.place_cursor_on_visible_row(20);
    app.add_filter("^hit").expect("valid");
    app.poll_filtering();

    key(&mut app, KeyCode::Char('n'));
    finish_pass(&mut app);
    assert_eq!(cursor_source(&app), 22, "the next hit after line 20");
}
