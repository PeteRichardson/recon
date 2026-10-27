use super::palette::run;
use super::*;

// ---- `:w` writes the visible lines to a file (#528) ------------------

/// The text of `name` in the explorer's folder, where a relative `:w`
/// path goes.
fn written(app: &App, name: &str) -> String {
    fs::read_to_string(app.explorer.dir().join(name)).expect("the file was written")
}

/// `:w PATH` writes every visible line, as `--emit lines` would print
/// it, to a path relative to the explorer's folder. The palette closes.
#[test]
fn w_writes_the_visible_lines_to_a_path_in_the_explorer_s_folder() {
    let mut app = app_over_file("write_dim", "hit one\nplain\nhit two\n");
    app.add_filter("hit").expect("valid");

    run(&mut app, "w out.txt");

    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(written(&app, "out.txt"), "hit one\nplain\nhit two\n");
}

/// Hide mode writes only the matches; `-n` puts each source line number
/// and a tab before it, as `--emit lines` does. The status row says how
/// many lines went where.
#[test]
fn w_writes_hide_mode_s_lines_with_their_numbers_and_says_so() {
    let file = fixture_path("write_hide_numbered", "hit one\nplain\nhit two\n");
    let mut app = App::new(&Startup::from(Config {
        path: Some(file.display().to_string()),
        line_numbers: true,
        ..Config::default()
    }));
    app.add_filter("hit").expect("valid");
    ctrl(&mut app, KeyCode::Char('h'));

    run(&mut app, "w out.txt");

    assert_eq!(written(&app, "out.txt"), "1\thit one\n3\thit two\n");
    let out = app.explorer.dir().join("out.txt");
    assert_eq!(
        status(&app),
        Some(format!("wrote 2 lines to {}", out.display()).as_str())
    );
}

/// `:w` never replaces a file: the palette stays open with the error, and
/// the file keeps its text. `:w!` replaces it.
#[test]
fn w_refuses_a_file_that_exists_and_w_bang_replaces_it() {
    let mut app = app_over_file("write_exists", "new\n");
    let out = app.explorer.dir().join("out.txt");
    fs::write(&out, "old\n").expect("write fixture");

    run(&mut app, "w out.txt");

    assert_eq!(
        prompt(&app).error.as_deref(),
        Some("out.txt: the file exists; w! replaces it")
    );
    assert_eq!(written(&app, "out.txt"), "old\n");

    key(&mut app, KeyCode::Esc);
    run(&mut app, "w! out.txt");

    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(written(&app, "out.txt"), "new\n");
}

/// `:w` needs a path: alone it does not write to the home folder, as `e`
/// alone would go there. A folder is not a file to write, even with `w!`.
#[test]
fn w_refuses_no_path_and_a_folder() {
    let mut app = app_over_file("write_no_path", "a\n");
    fs::create_dir(app.explorer.dir().join("sub")).expect("create fixture");

    for (line, error) in [("w", "w: give a path"), ("w! sub", "sub: is a folder")] {
        run(&mut app, line);
        assert_eq!(prompt(&app).error.as_deref(), Some(error), "{line}");
        key(&mut app, KeyCode::Esc);
    }
}

/// With no file's lines in the view — a folder's listing, or an error —
/// `:w` writes no file, rather than an empty one.
#[test]
fn w_writes_nothing_when_the_view_has_no_file_s_lines() {
    let dir = fixture_dir("write_listing");
    fs::write(dir.join("a.txt"), "x\n").expect("write");
    let mut listing = App::new(&Startup::from(Config {
        path: Some(dir.display().to_string()),
        ..Config::default()
    }));
    // `..` is selected, so the view shows a listing.
    key(&mut listing, KeyCode::Char('g'));
    let missing = fixture_dir("write_error");
    let mut error = App::new(&Startup::from(Config {
        path: Some(missing.join("nope.log").display().to_string()),
        ..Config::default()
    }));

    for (app, message) in [
        (&mut listing, "w: the view is showing a folder, not a file"),
        (&mut error, "w: the view is showing an error, not a file"),
    ] {
        run(app, "w out.txt");
        assert_eq!(prompt(app).error.as_deref(), Some(message));
        assert!(!app.explorer.dir().join("out.txt").exists(), "{message}");
    }
}

/// A file past the preview cap, whose last line is the only hit, with
/// hide mode on.
fn app_over_a_large_file(name: &str) -> App<'static> {
    let lines = crate::views::read::PREVIEW_LINES + 100;
    let body: String = (0..lines)
        .map(|i| {
            if i == lines - 1 {
                "hit at the end\n".to_string()
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    // A directory argument previews its first entry.
    let dir = fixture_dir(name);
    fs::write(dir.join("big.log"), &body).expect("write fixture");
    let mut app = App::new(&Startup::from(Config {
        path: Some(dir.display().to_string()),
        line_numbers: true,
        ..Config::default()
    }));
    assert!(app.view.is_truncated(), "the fixture must be past the cap");
    app.add_filter("hit").expect("valid");
    ctrl(&mut app, KeyCode::Char('h'));
    app
}

/// Poll the write `:w` started until it ends.
fn finish_write(app: &mut App) {
    let start = std::time::Instant::now();
    while app.writing.is_some() {
        app.poll_write();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "the write never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Over a preview, `:w` reads the rest of the file in the background, as
/// `q` under `--emit lines` does (#351), and writes the lines past the
/// preview too. The session goes on; the status row shows how far the
/// read is, and the key that stops it.
#[test]
fn w_over_a_preview_reads_the_whole_file_in_the_background() {
    let mut app = app_over_a_large_file("write_large");

    run(&mut app, "w out.txt");

    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(app.state, AppState::Running);
    let row = status_line(&mut app);
    assert!(row.contains("writing out.txt: "), "{row}");
    assert!(row.ends_with(" — Esc to cancel"), "{row}");

    finish_write(&mut app);

    let last = crate::views::read::PREVIEW_LINES + 100;
    assert_eq!(
        written(&app, "out.txt"),
        format!("{last}\thit at the end\n")
    );
    let out = app.explorer.dir().join("out.txt");
    assert_eq!(
        status(&app),
        Some(format!("wrote 1 lines to {}", out.display()).as_str())
    );
}

/// Esc stops the read and writes no part of the file. While a read runs,
/// a second `:w` is refused, so one file cannot get two writers.
#[test]
fn esc_stops_the_read_and_a_second_w_waits_for_it() {
    let mut app = app_over_a_large_file("write_large_cancel");
    run(&mut app, "w out.txt");

    run(&mut app, "w other.txt");
    assert_eq!(
        prompt(&app).error.as_deref(),
        Some("w: a write is running; Esc stops it")
    );
    key(&mut app, KeyCode::Esc);
    assert!(
        app.writing.is_some(),
        "the Esc that closed the palette stopped the read"
    );

    key(&mut app, KeyCode::Esc);

    assert!(app.writing.is_none(), "Esc did not stop the read");
    assert_eq!(status(&app), Some("w: stopped; nothing written"));
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!app.explorer.dir().join("out.txt").exists());
    assert!(!app.explorer.dir().join("other.txt").exists());
}
