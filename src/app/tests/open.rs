use super::*;

// ---- `o`: open the enclosing project (#42) ---------------------------

/// Swap in the recording double and hand back a handle the test can read
/// afterwards. `Rc`, not a clone: the app and the test must see the same
/// recording, and `Launcher` takes `&self` so no mutability is shared.
fn record_launches(app: &mut App, launcher: RecordingLauncher) -> Rc<RecordingLauncher> {
    let launcher = Rc::new(launcher);
    app.launcher = Box::new(Rc::clone(&launcher));
    launcher
}

/// The headline acceptance criterion: `o` opens the *enclosing project* of
/// the selected file, at the line the cursor is on.
#[test]
fn o_opens_the_enclosing_project_at_the_cursors_line() {
    let (mut app, root) = app_over_project("o_project", "alpha\nbeta\ngamma\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    key(&mut app, KeyCode::Char('o'));

    assert_eq!(
        launcher.only_command(),
        [
            "zed".to_string(),
            absolute(&root),
            format!("{}:1", absolute(&root.join("logs/log.txt"))),
        ]
    );
}

/// The line is the cursor's, not always 1 — and it is 1-based, because
/// every editor's `:line` argument is.
#[test]
fn the_editor_lands_on_the_line_the_cursor_is_on() {
    let (mut app, root) = app_over_project("o_line", "alpha\nbeta\ngamma\ndelta\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    focus_file_view(&mut app);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('o'));

    let argv = launcher.only_command();
    assert_eq!(
        argv.last().expect("a file argument"),
        &format!("{}:3", absolute(&root.join("logs/log.txt")))
    );
}

/// Global, not pane-scoped. The explorer is where `o` is most natural to
/// press, so requiring the file view to be focused first would break it in
/// the one place it matters most.
#[test]
fn o_works_from_the_explorer_pane() {
    let (mut app, _root) = app_over_project("o_from_explorer", "alpha\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    assert!(
        app.focus == Focus::Explorer,
        "the explorer should have focus at startup"
    );
    key(&mut app, KeyCode::Char('o'));

    assert!(!launcher.is_empty(), "`o` did nothing from the explorer");
}

/// The template is a setting, so a configured one has to actually reach the
/// command — this is the whole ladder in `config.rs` proved end to end.
#[test]
fn a_configured_template_is_what_runs() {
    let root = fixture_dir("o_template");
    fs::write(root.join("go.mod"), "module fixture\n").expect("write marker");
    let file = root.join("log.txt");
    fs::write(&file, "alpha\n").expect("write fixture");

    let mut app = App::new(&Config {
        path: file.display().to_string(),
        editor: Some("code {project} -g {file}:{line}".to_string()),
        ..Config::default()
    });
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    key(&mut app, KeyCode::Char('o'));

    assert_eq!(
        launcher.only_command(),
        [
            "code".to_string(),
            absolute(&root),
            "-g".to_string(),
            format!("{}:1", absolute(&file)),
        ]
    );
}

/// A missing or failing command must say so, and recon must keep running.
#[test]
fn a_failing_command_is_reported_on_the_status_row() {
    let (mut app, _root) = app_over_project("o_fail", "alpha\n");
    record_launches(&mut app, RecordingLauncher::failing("no such file"));

    key(&mut app, KeyCode::Char('o'));

    let row = status_line(&mut app);
    assert!(
        row.contains("zed"),
        "the row does not name the command: {row}"
    );
    assert!(
        row.contains("no such file"),
        "the row does not say why: {row}"
    );
    assert!(app.is_running(), "a failed launch brought the app down");
}

/// A typo in the template is the user's, and it can only be caught here:
/// templates are deliberately not validated at startup, so that a bad one
/// never stops recon opening a log.
#[test]
fn a_broken_template_is_reported_rather_than_run() {
    let root = fixture_dir("o_broken_template");
    let file = root.join("log.txt");
    fs::write(&file, "alpha\n").expect("write fixture");

    let mut app = App::new(&Config {
        path: file.display().to_string(),
        editor: Some("zed 'unclosed".to_string()),
        ..Config::default()
    });
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    key(&mut app, KeyCode::Char('o'));

    assert!(launcher.is_empty(), "a broken template still ran something");
    let row = status_line(&mut app);
    assert!(row.contains("unclosed"), "the row does not explain: {row}");
}

/// The startup argument can name a file that is not there — the pane shows
/// the error in place of its text — and `o` must not hand that path to an
/// editor as though it existed.
#[test]
fn o_refuses_a_file_that_is_not_there() {
    let dir = fixture_dir("o_missing");

    let mut app = App::new(&Config {
        path: dir.join("nope.log").display().to_string(),
        ..Config::default()
    });
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    key(&mut app, KeyCode::Char('o'));

    assert!(
        launcher.is_empty(),
        "a missing file was handed to an editor"
    );
    // The startup argument is held absolute (#157), so this message
    // carries a full path rather than the short relative one the
    // default 120-column `AREA` was fitting before. The width here is
    // incidental to what is under test — the message's content — so it
    // is widened rather than the shared `status_line`/`AREA` that 47
    // other tests render through.
    assert!(status_line_at(&mut app, 300).contains("no such file"));
}

// An editor exits on its own schedule, so its report is the one change
// that arrives with no keypress behind it. That makes `drain_editor_outcomes`
// the only thing standing between "redraw when something happened" and a
// status message that sits invisible until the user happens to press a key
// — which is why #85's conditional loop rests entirely on these three.

/// Nothing arrived, so nothing on screen changed.
#[test]
fn draining_an_empty_editor_channel_is_not_a_change() {
    let (mut app, _root) = app_over_project("drain_empty", "alpha\n");
    let (_tx, rx) = std::sync::mpsc::channel::<String>();
    app.editor_outcomes = Some(rx);

    assert!(!app.drain_editor_outcomes());
}

/// A drained message rewrites the status row, so the frame is stale.
#[test]
fn draining_an_editor_outcome_is_a_change() {
    let (mut app, _root) = app_over_project("drain_message", "alpha\n");
    let (tx, rx) = std::sync::mpsc::channel();
    app.editor_outcomes = Some(rx);
    tx.send("zed: No such file or directory".to_string())
        .expect("send outcome");

    assert!(app.drain_editor_outcomes());
    assert!(status_line(&mut app).contains("No such file"));
}

/// Only the last message survives the drain — but the drain still happened,
/// so it still reports a change. Both halves matter: a loop told "no change"
/// here would leave the surviving message unpainted.
#[test]
fn draining_several_outcomes_keeps_the_last_and_still_reports_a_change() {
    let (mut app, _root) = app_over_project("drain_several", "alpha\n");
    let (tx, rx) = std::sync::mpsc::channel();
    app.editor_outcomes = Some(rx);
    tx.send("first".to_string()).expect("send first");
    tx.send("second".to_string()).expect("send second");

    assert!(app.drain_editor_outcomes());
    let row = status_line(&mut app);
    assert!(row.contains("second"), "the newest report is the one shown");
    assert!(!row.contains("first"));
}

/// Transient means transient: the next keypress takes the row back.
#[test]
fn the_status_message_lasts_until_the_next_keypress() {
    let (mut app, _root) = app_over_project("o_transient", "alpha\n");
    record_launches(&mut app, RecordingLauncher::failing("boom"));

    key(&mut app, KeyCode::Char('o'));
    assert!(status_line(&mut app).contains("boom"));

    key(&mut app, KeyCode::Tab);
    assert!(
        !status_line(&mut app).contains("boom"),
        "the message outlived the keypress after it"
    );
}

/// A mouse move must *not* clear it. Mouse capture is on, so anything less
/// deliberate than a keypress would wipe the message before it was read.
#[test]
fn a_mouse_event_does_not_clear_the_status_message() {
    let (mut app, _root) = app_over_project("o_mouse", "alpha\n");
    record_launches(&mut app, RecordingLauncher::failing("boom"));

    key(&mut app, KeyCode::Char('o'));
    mouse(&mut app, MouseEventKind::Moved, 60);

    assert!(status_line(&mut app).contains("boom"));
}

/// An open prompt outranks the message: the user is typing into that row.
#[test]
fn a_prompt_outranks_the_status_message() {
    let (mut app, _root) = app_over_project("o_prompt", "alpha\n");
    record_launches(&mut app, RecordingLauncher::failing("boom"));

    key(&mut app, KeyCode::Char('o'));
    key(&mut app, KeyCode::Char('/'));

    assert_eq!(prompt_line(&mut app), "/");
}

/// `o` is only claimed unmodified, so Ctrl-O still reaches the focused
/// widget — the same guard `q`, `f` and `p` use.
#[test]
fn ctrl_o_is_not_the_open_key() {
    let (mut app, _root) = app_over_project("o_ctrl", "alpha\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    ctrl(&mut app, KeyCode::Char('o'));

    assert!(launcher.is_empty(), "Ctrl-O was swallowed as the open key");
}

// ---- `O`: open the file alone (#41) ----------------------------------

/// `key`, with Shift held — what a real terminal sends for an uppercase
/// character. `key(Char('O'))` alone is the harness-only case; both have to
/// work, which is why `O` is guarded on CONTROL/ALT rather than on
/// `.is_empty()`.
fn shift(app: &mut App, code: KeyCode) {
    app.handle_event(event::Event::Key(event::KeyEvent::new(
        code,
        KeyModifiers::SHIFT,
    )));
}

/// The headline acceptance criterion: `O` opens the file *alone*, at the
/// cursor's line, with no project argument — and the fixture is inside a
/// project, so this also proves no walk-up happened.
#[test]
fn shift_o_opens_the_file_alone_at_the_cursors_line() {
    let (mut app, root) = app_over_project("shift_o_file", "alpha\nbeta\ngamma\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    shift(&mut app, KeyCode::Char('O'));

    assert_eq!(
        launcher.only_command(),
        [
            "zed".to_string(),
            format!("{}:1", absolute(&root.join("logs/log.txt"))),
        ]
    );
}

/// Stated separately from the argv assertion above because it is the point
/// of the key: `~/.zshrc` inside a dotfiles repo has a marker above it, and
/// `O` exists so that marker is never consulted.
#[test]
fn shift_o_performs_no_walk_up_even_inside_a_project() {
    let (mut app, root) = app_over_project("shift_o_no_walk_up", "alpha\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    shift(&mut app, KeyCode::Char('O'));

    let argv = launcher.only_command();
    assert!(
        !argv.contains(&absolute(&root)),
        "`O` climbed to the project root anyway: {argv:?}"
    );
    assert_eq!(argv.len(), 2, "`O` passed more than a program and a file");
}

/// The cursor's line, 1-based, on this path too — the shared half of
/// `open_in_editor` proved through the other key.
#[test]
fn shift_o_lands_on_the_line_the_cursor_is_on() {
    let (mut app, root) = app_over_project("shift_o_line", "alpha\nbeta\ngamma\ndelta\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    focus_file_view(&mut app);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    shift(&mut app, KeyCode::Char('O'));

    assert_eq!(
        launcher.only_command().last().expect("a file argument"),
        &format!("{}:3", absolute(&root.join("logs/log.txt")))
    );
}

/// Global, not pane-scoped, exactly like `o`.
#[test]
fn shift_o_works_from_the_explorer_pane() {
    let (mut app, _root) = app_over_project("shift_o_from_explorer", "alpha\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    assert!(
        app.focus == Focus::Explorer,
        "the explorer should have focus at startup"
    );
    shift(&mut app, KeyCode::Char('O'));

    assert!(!launcher.is_empty(), "`O` did nothing from the explorer");
}

/// A harness that sends no modifier at all must still reach the key. The
/// tests above pin the real terminal's SHIFT; this pins the other case, so
/// neither guard can be tightened into breaking the other.
#[test]
fn shift_o_is_reached_with_no_modifier_attached() {
    let (mut app, _root) = app_over_project("shift_o_bare", "alpha\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    key(&mut app, KeyCode::Char('O'));

    assert!(!launcher.is_empty(), "a bare `O` did not reach the key");
}

/// The derive rung, end to end: one line of config for `o` makes `O` work,
/// by dropping the `{project}` entry rather than by string surgery.
#[test]
fn the_file_template_is_derived_from_the_project_template() {
    let root = fixture_dir("shift_o_derived");
    fs::write(root.join("go.mod"), "module fixture\n").expect("write marker");
    let file = root.join("log.txt");
    fs::write(&file, "alpha\n").expect("write fixture");

    let mut app = App::new(&Config {
        path: file.display().to_string(),
        editor: Some("code {project} -g {file}:{line}".to_string()),
        ..Config::default()
    });
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    shift(&mut app, KeyCode::Char('O'));

    assert_eq!(
        launcher.only_command(),
        [
            "code".to_string(),
            "-g".to_string(),
            format!("{}:1", absolute(&file)),
        ]
    );
}

/// An explicit `editor.file` outranks the derived one — the rung above it
/// on the ladder, proved through the key rather than in isolation.
#[test]
fn an_explicit_file_template_beats_the_derived_one() {
    let root = fixture_dir("shift_o_explicit");
    let file = root.join("log.txt");
    fs::write(&file, "alpha\n").expect("write fixture");

    let mut app = App::new(&Config {
        path: file.display().to_string(),
        editor: Some("code {project} -g {file}:{line}".to_string()),
        file_editor: Some("subl -n {file}:{line}".to_string()),
        ..Config::default()
    });
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    shift(&mut app, KeyCode::Char('O'));

    assert_eq!(
        launcher.only_command(),
        [
            "subl".to_string(),
            "-n".to_string(),
            format!("{}:1", absolute(&file)),
        ]
    );
}

/// `o` must keep its own template when the two differ — the one assertion
/// that catches the arms being wired to the same field.
#[test]
fn the_two_keys_do_not_share_a_template() {
    let root = fixture_dir("shift_o_distinct");
    let file = root.join("log.txt");
    fs::write(&file, "alpha\n").expect("write fixture");

    let mut app = App::new(&Config {
        path: file.display().to_string(),
        editor: Some("zed {project} {file}:{line}".to_string()),
        file_editor: Some("subl {file}:{line}".to_string()),
        ..Config::default()
    });
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    key(&mut app, KeyCode::Char('o'));

    assert_eq!(
        launcher.only_command().first().expect("a program"),
        "zed",
        "`o` ran the file template"
    );
}

/// A failing launch reports and recon keeps running on this path too.
#[test]
fn a_failing_shift_o_is_reported_on_the_status_row() {
    let (mut app, _root) = app_over_project("shift_o_fail", "alpha\n");
    record_launches(&mut app, RecordingLauncher::failing("no such file"));

    shift(&mut app, KeyCode::Char('O'));

    let row = status_line(&mut app);
    assert!(
        row.contains("no such file"),
        "the row does not say why: {row}"
    );
    assert!(app.is_running(), "a failed launch brought the app down");
}

/// Ctrl-O and Alt-O stay unclaimed, so they fall through to the focused
/// widget — the same tolerance `H` and `n`/`N` use.
#[test]
fn ctrl_shift_o_is_not_the_open_key() {
    let (mut app, _root) = app_over_project("shift_o_ctrl", "alpha\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    ctrl(&mut app, KeyCode::Char('O'));

    assert!(
        launcher.is_empty(),
        "Ctrl-Shift-O was swallowed as the open key"
    );
}

/// An open prompt outranks every binding, and an uppercase global is the
/// one most likely to break that: `O` is an ordinary character to type into
/// a search. The guard is the early return `handle_event` already makes for
/// `self.prompt`, so this pins the behaviour rather than adding to it.
#[test]
fn shift_o_typed_into_a_prompt_is_text_not_the_open_key() {
    let (mut app, _root) = app_over_project("shift_o_prompt", "alpha\n");
    let launcher = record_launches(&mut app, RecordingLauncher::default());

    key(&mut app, KeyCode::Char('/'));
    shift(&mut app, KeyCode::Char('O'));

    assert!(launcher.is_empty(), "`O` fired from inside a prompt");
    assert_eq!(prompt_line(&mut app), "/O");
}
