use super::*;

// ---- the hex view (#242) --------------------------------------------

fn app_over_bytes(name: &str, bytes: &[u8]) -> App<'static> {
    let file = fixture_file(&format!("{name}.bin"), bytes);
    App::new(&Startup::from(Config {
        path: file.display().to_string(),
        emit: Some(emit::Emit::Lines),
        ..Config::default()
    }))
}

/// The lines `--emit lines` would write.
fn emitted(app: &App) -> Vec<String> {
    let emit::Exit::Emit { lines, .. } = app.collect(emit::Emit::Lines) else {
        panic!("silent");
    };
    lines
        .into_iter()
        .map(|line| String::from_utf8(line).expect("a dump is ASCII"))
        .collect()
}

#[test]
fn a_binary_file_opens_as_hex_and_minus_shows_the_message() {
    let mut app = app_over_bytes("hex_app_binary", b"ab\0cd");
    assert_eq!(view_lines(&app), [crate::hex::line(0, b"ab\0cd")]);

    key(&mut app, KeyCode::Char('-'));
    assert_eq!(view_lines(&app), ["<binary file: contains NUL bytes>"]);

    key(&mut app, KeyCode::Char('-'));
    assert_eq!(view_lines(&app), [crate::hex::line(0, b"ab\0cd")]);
}

/// `-` is global, so it works from the explorer as the selection moves —
/// which is where a binary file is first met — and on a text file too.
#[test]
fn minus_shows_a_text_file_as_hex_from_the_explorer() {
    let mut app = app_over_files("hex_app_text", &[("a.txt", "hi\n")]);
    open_file(&mut app, 0);
    app.focus = Focus::Explorer;

    key(&mut app, KeyCode::Char('-'));

    assert_eq!(view_lines(&app), [crate::hex::line(0, b"hi\n")]);
}

/// A dump is lines like any other, so `--emit lines` writes them.
#[test]
fn emit_lines_writes_the_dump() {
    let bytes: Vec<u8> = (0..20).collect();
    let app = app_over_bytes("hex_app_emit", &bytes);

    assert_eq!(emitted(&app), crate::hex::lines(&bytes));
}

#[test]
fn minus_on_a_directory_says_there_is_no_file() {
    let dir = fixture_dir("hex_app_dir");
    fs::create_dir(dir.join("sub")).expect("mkdir");
    let mut app = App::new(&Startup::from(Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    }));
    app.perform_widget_action(Action::Load(dir.join("sub")));

    key(&mut app, KeyCode::Char('-'));

    assert_eq!(status(&app), Some("no file to show as hex"));
}

/// An error in the pane is not a file: `-` says so rather than read it
/// again and show the same error (#405).
#[test]
fn minus_on_an_error_says_there_is_no_file() {
    let dir = fixture_dir("hex_app_error");
    let mut app = App::new(&Startup::from(Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    }));
    app.perform_widget_action(Action::Load(dir.join("absent.txt")));

    key(&mut app, KeyCode::Char('-'));

    assert_eq!(status(&app), Some("no file to show as hex"));
}
