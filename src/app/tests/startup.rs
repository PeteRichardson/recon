use super::*;

/// The argument is held the way the explorer holds it, absolute. Two
/// spellings of one path made `check_stamps` compare unequal, so the
/// changed-on-disk badge never fired for a file opened from the command
/// line, and the title changed after the first navigation (#157).
#[test]
fn the_command_line_argument_is_held_absolute() {
    // No fixture file: `lexical_absolute` resolves against the process
    // directory whether or not the file exists, and `App::new` loads the
    // argument either way. What is under test is the spelling, not the read.
    let app = App::new(&Startup::from(Config {
        path: "a.log".to_string(),
        ..Config::default()
    }));

    assert!(
        app.view.filename().is_absolute(),
        "the view holds {} relative",
        app.view.filename().display()
    );
}

/// Launched on a directory, the view shows the first entry's contents —
/// not `<directory>`, which is what pointing the view at the argument
/// itself would have produced.
#[test]
fn a_directory_argument_previews_the_first_entry() {
    let dir = fixture_dir("arg_is_a_dir");
    fs::write(dir.join("aaa.txt"), "first file contents\n").expect("write");
    fs::write(dir.join("zzz.txt"), "last file contents\n").expect("write");

    let mut app = App::new(&Startup::from(Config {
        path: dir.display().to_string(),
        ..Config::default()
    }));

    let shown = rendered(&mut app);
    assert!(
        shown.contains("first file contents"),
        "the first entry was not previewed:\n{shown}"
    );
    assert!(
        !shown.contains("<directory>"),
        "the view was pointed at the directory itself:\n{shown}"
    );
}

/// A directory argument *selects* its first entry rather than being
/// handed it, so it is previewed — bounded — not read in full. Otherwise
/// starting recon in a directory of large logs reads one of them whole.
#[test]
fn a_directory_argument_previews_rather_than_loads() {
    let dir = fixture_dir("arg_dir_bounded");
    // Past the line cap, which is what "previewed rather than loaded" now
    // means: below the cap the two are the same thing, deliberately, since
    // reading a log-sized file whole costs well under a millisecond.
    let lines = crate::widgets::fileview::PREVIEW_LINES + 100;
    let body = numbered_lines(lines);
    fs::write(dir.join("big.log"), &body).expect("write");

    let app = App::new(&Startup::from(Config {
        path: dir.display().to_string(),
        ..Config::default()
    }));

    // Asked of the *document*, not the view. Since #7 the textarea holds
    // only a window of what is visible, so its length says how tall the
    // pane is, not how much of the file was read.
    assert_eq!(
        app.document.lines().len(),
        crate::widgets::fileview::PREVIEW_LINES,
        "the first entry was read in full instead of previewed"
    );
}

/// A duplicate fixture directory name must fail loudly and immediately,
/// not race with whatever other test already claimed it. The panic
/// happens in `claim_fixture_dir` before any filesystem work, and
/// `claim_fixture_dir` recovers the lock via `into_inner` on poison, so
/// this does not wedge the guard for every test that runs after it.
#[test]
fn a_duplicate_fixture_directory_name_is_rejected() {
    let _first = app_over("dup_fixture_name", &["a.rs"]);

    let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app_over("dup_fixture_name", &["a.rs"]);
    }));

    assert!(
        second.is_err(),
        "a duplicate fixture directory name was not rejected"
    );
}
