use super::*;
use crate::views::command::{Command, Commands};
use crate::views::pool::double::Held;
use crate::views::{FileType, View};

// ---- the view pipeline and the view cycle (#455) --------------------

fn row(glob: &str, views: &[View]) -> FileType {
    FileType::with_views(glob, views)
}

/// An app over `files`, with `rows` as its `[[filetype]]` table.
fn app_with_rows(name: &str, files: &[(&str, &[u8])], rows: Vec<FileType>) -> App<'static> {
    let dir = fixture_dir(name);
    for (file, body) in files {
        fs::write(dir.join(file), body).expect("write fixture");
    }
    App::new(&Startup::from(Config {
        path: Some(dir.join("placeholder").display().to_string()),
        filetypes: rows,
        ..Config::default()
    }))
}

/// Install a held executor: every request waits for `release`.
fn hold(app: &mut App) -> Held {
    let held = Held::default();
    app.loading.executor = Box::new(held.clone());
    held
}

#[test]
fn a_row_sets_the_first_view_and_other_files_keep_the_built_in_one() {
    let mut app = app_with_rows(
        "views_row",
        &[("a.log", b"hi\n"), ("b.txt", b"hi\n")],
        vec![row("*.log", &[View::Hex, View::Raw])],
    );

    arrow_to(&mut app, "a.log");
    assert_eq!(view_lines(&app), [crate::hex::line(0, b"hi\n")]);

    arrow_to(&mut app, "b.txt");
    assert_eq!(view_lines(&app), ["hi"]);
}

/// `-` steps to the next view and wraps, and the step holds for the next
/// file of the same row: one key press shows a whole folder as hex.
#[test]
fn minus_steps_wraps_and_holds_for_the_next_file_of_the_row() {
    let mut app = app_over_files("views_sticky", &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    app.focus = Focus::Explorer;
    arrow_to(&mut app, "a.txt");

    key(&mut app, KeyCode::Char('-'));
    assert_eq!(view_lines(&app), [crate::hex::line(0, b"a\n")]);

    arrow_to(&mut app, "b.txt");
    assert_eq!(
        view_lines(&app),
        [crate::hex::line(0, b"b\n")],
        "the step did not hold for the next file"
    );

    key(&mut app, KeyCode::Char('-'));
    assert_eq!(view_lines(&app), ["b"], "the cycle did not wrap");
}

#[test]
fn the_status_line_names_the_view_when_it_is_not_raw() {
    let mut app = app_over_files("views_badge", &[("a.txt", "a\n")]);
    app.focus = Focus::Explorer;
    arrow_to(&mut app, "a.txt");
    assert!(
        !status_line(&mut app).contains(" raw "),
        "raw is named: {}",
        status_line(&mut app)
    );

    key(&mut app, KeyCode::Char('-'));

    let line = status_line(&mut app);
    assert!(
        line.starts_with(" hex "),
        "no view on the status line: {line}"
    );
}

/// A row that asks for raw on a binary file: raw fails, and hex shows at
/// once, with the failure on the status line.
#[test]
fn a_failed_view_shows_hex_and_says_why() {
    let mut app = app_with_rows(
        "views_fallback",
        &[("a.bin", b"a\0b")],
        vec![row("*.bin", &[View::Raw])],
    );

    arrow_to(&mut app, "a.bin");

    assert_eq!(view_lines(&app), [crate::hex::line(0, b"a\0b")]);
    assert_eq!(status(&app), Some("raw failed → hex: binary file"));
    assert!(status_line(&mut app).starts_with(" hex "));
}

/// A view that takes longer than the wait shows a placeholder and a
/// spinner, then the file when it comes.
#[test]
fn a_late_view_shows_a_placeholder_then_the_contents() {
    let mut app = app_over_files("views_late", &[("a.txt", "alpha\n"), ("b.txt", "bravo\n")]);
    arrow_to(&mut app, "a.txt");
    let held = hold(&mut app);

    arrow_to(&mut app, "b.txt");
    assert_eq!(view_lines(&app), [super::super::loading::PLACEHOLDER]);
    assert!(
        status_line(&mut app).starts_with(" loading "),
        "no spinner: {}",
        status_line(&mut app)
    );

    held.release();
    assert!(app.drain_views(), "the answer did not ask for a redraw");
    assert_eq!(view_lines(&app), ["bravo"]);
    assert!(!status_line(&mut app).contains("loading"));
}

/// The answer for a file the user has already left is dropped: the pane
/// shows the newest selection, whatever order the answers come in.
#[test]
fn an_answer_for_a_file_the_user_left_is_dropped() {
    let mut app = app_over_files(
        "views_generation",
        &[
            ("a.txt", "alpha\n"),
            ("b.txt", "bravo\n"),
            ("c.txt", "charlie\n"),
        ],
    );
    let held = hold(&mut app);

    arrow_to(&mut app, "b.txt");
    arrow_to(&mut app, "c.txt");
    held.release();
    app.drain_views();

    assert_eq!(view_lines(&app), ["charlie"]);
}

/// A move down the explorer prefetches the files below the selection,
/// nearest first, then the ones above it.
#[test]
fn a_move_prefetches_the_files_ahead_then_behind() {
    let files: Vec<(String, String)> = (0..12)
        .map(|n| (format!("f{n:02}.txt"), format!("{n}\n")))
        .collect();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(name, body)| (name.as_str(), body.as_str()))
        .collect();
    let mut app = app_over_files("views_prefetch", &files);
    let held = hold(&mut app);
    arrow_to(&mut app, "f02.txt");
    arrow_to(&mut app, "f03.txt");

    let names: Vec<String> = held
        .prefetched()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "f04.txt", "f05.txt", "f06.txt", "f07.txt", "f08.txt", "f09.txt", "f10.txt", "f11.txt",
            "f02.txt", "f01.txt"
        ]
    );
}

/// `r` on a preview reads a preview again, not the whole file: a 2 GB log
/// in the explorer must not be read whole because of `r` (#383).
#[test]
fn r_on_a_truncated_preview_reads_a_preview_again() {
    let body = numbered_lines(crate::views::read::PREVIEW_LINES + 10);
    let mut app = app_over_files("views_r_preview", &[("big.log", &body)]);
    arrow_to(&mut app, "big.log");
    app.focus = Focus::Explorer;
    assert!(app.view.is_truncated(), "the fixture is not past the cap");

    key(&mut app, KeyCode::Char('r'));

    assert!(app.view.is_truncated(), "`r` read the whole file");
}

// ---- command views (#457) --------------------------------------------

/// `[commands]`: each name, its argv, and the MIME type of its output.
fn commands(entries: &[(&'static str, &[&str], Option<&str>)]) -> Commands {
    Commands::of(entries.iter().map(|(name, argv, output)| {
        let command = Command::of(argv);
        (
            *name,
            output.map_or(command.clone(), |output| command.with_output(output)),
        )
    }))
}

/// An app over `files`, with `rows` and `commands`.
fn app_with_commands(
    name: &str,
    files: &[(&str, &[u8])],
    rows: Vec<FileType>,
    commands: Commands,
) -> App<'static> {
    let dir = fixture_dir(name);
    for (file, body) in files {
        fs::write(dir.join(file), body).expect("write fixture");
    }
    App::new(&Startup::from(Config {
        path: Some(dir.join("placeholder").display().to_string()),
        filetypes: rows,
        commands,
        ..Config::default()
    }))
}

/// One JSON line, and a command that puts each key on its own line, as
/// `jq .` does, with no `jq` needed to run the test.
fn split_json_app(name: &str) -> App<'static> {
    app_with_commands(
        name,
        &[("a.json", br#"{"a":1,"b":2}"#)],
        vec![FileType::with_view_names("*.json", &["split", "raw"])],
        commands(&[("split", &["tr", ",", "\n"], Some("application/json"))]),
    )
}

#[test]
fn a_command_view_shows_its_output_and_the_status_line_names_it() {
    let mut app = split_json_app("views_command");
    arrow_to(&mut app, "a.json");

    assert_eq!(view_lines(&app), [r#"{"a":1"#, r#""b":2}"#]);
    let line = status_line(&mut app);
    assert!(line.starts_with(" split "), "no view name: {line}");

    key(&mut app, KeyCode::Char('-'));
    assert_eq!(
        view_lines(&app),
        [r#"{"a":1,"b":2}"#],
        "`-` did not step to raw"
    );
}

#[test]
fn a_command_view_is_coloured_by_its_output_type() {
    let mut app = app_with_commands(
        "views_command_mime",
        &[("a.txt", br#"{"a":1}"#)],
        vec![FileType::with_view_names("*.txt", &["json"])],
        commands(&[("json", &["cat"], Some("application/json"))]),
    );
    app.view.set_theme(crate::syntax::Theme::builtin());
    arrow_to(&mut app, "a.txt");
    assert_eq!(app.view.syntax_name(), Some("JSON"));
}

/// ADR 0006: a filter runs on the lines of the view, so a key in the
/// split output matches one line, not the whole file.
#[test]
fn a_filter_on_a_command_view_matches_its_lines() {
    let mut app = split_json_app("views_command_filter");
    arrow_to(&mut app, "a.json");

    app.add_filter("\"b\"").expect("valid");

    assert_eq!(app.document.verdicts().len(), 2);
    assert_eq!(included(&app), 1);
}

#[test]
fn a_failed_command_shows_raw_and_says_why() {
    let mut app = app_with_commands(
        "views_command_fail",
        &[("a.json", b"{}\n")],
        vec![FileType::with_view_names("*.json", &["bad", "hex"])],
        commands(&[("bad", &["sh", "-c", "echo 'parse error' >&2; exit 4"], None)]),
    );
    arrow_to(&mut app, "a.json");

    assert_eq!(view_lines(&app), ["{}"]);
    assert_eq!(status(&app), Some("bad failed → raw: parse error"));
    let line = status_line(&mut app);
    assert!(!line.contains(" bad "), "the failed view is named: {line}");
}

// ---- the image view (#458) -------------------------------------------

/// A picture shows as one, by its extension, with no row of the user's.
/// `-` steps to its dump.
#[test]
fn a_picture_shows_as_an_image_and_minus_steps_to_hex() {
    let png = crate::views::tests::png(4, 2);
    let mut app = app_with_rows("views_image", &[("a.png", &png)], vec![]);
    app.focus = Focus::Explorer;

    arrow_to(&mut app, "a.png");

    assert!(app.view.has_image(), "no image in the pane");
    assert!(view_lines(&app).iter().all(String::is_empty));
    let line = status_line(&mut app);
    assert!(line.starts_with(" image "), "no view named: {line}");

    key(&mut app, KeyCode::Char('-'));
    assert!(!app.view.has_image());
    assert_eq!(view_lines(&app)[0], crate::hex::line(0, &png[..16]));
}

/// With a picture in the focused view, the keys that act on lines say so,
/// and do nothing: a filter or a search cannot fail on lines that are not
/// there.
#[test]
fn keys_on_lines_say_an_image_has_none() {
    let png = crate::views::tests::png(4, 2);
    let mut app = app_with_rows("views_image_keys", &[("a.png", &png)], vec![]);
    app.focus = Focus::Explorer;
    arrow_to(&mut app, "a.png");
    app.focus = Focus::View;

    for code in ['/', 'n', 'N', '1', '!', 'u', 'v', 'V', '*'] {
        app.status_message = None;
        key(&mut app, KeyCode::Char(code));
        assert_eq!(status(&app), Some("this view has no lines"), "{code}");
        assert!(app.prompt.is_none(), "{code} opened a prompt");
        assert!(app.visual.is_none(), "{code} started a selection");
        assert!(app.pending_digit.is_none(), "{code} took a digit");
    }
}

/// In the explorer, `/` searches the names and the filter keys work: a
/// filter also marks the text files near the picture.
#[test]
fn an_image_leaves_the_explorer_keys_alone() {
    let png = crate::views::tests::png(4, 2);
    let mut app = app_with_rows("views_image_explorer", &[("a.png", &png)], vec![]);
    app.focus = Focus::Explorer;
    arrow_to(&mut app, "a.png");

    key(&mut app, KeyCode::Char('1'));
    assert_eq!(status(&app), Some("filter 1_"));
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('/'));
    assert!(app.prompt.is_some(), "no search in the explorer");
}

/// The picker's cell size is 10 × 20 px for half-blocks.
fn picture_app(name: &str, width: u32, height: u32) -> App<'static> {
    let png = crate::views::tests::png_filled(width, height, [255, 0, 0]);
    let mut app = app_with_rows(name, &[("a.png", &png)], vec![]);
    app.focus = Focus::Explorer;
    arrow_to(&mut app, "a.png");
    app
}

/// The columns and rows of the screen that show the picture's red.
fn red_extent(app: &mut App) -> (usize, usize) {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let red = ratatui::style::Color::Rgb(255, 0, 0);
    let cells: Vec<(u16, u16)> = (0..AREA.height)
        .flat_map(|y| (0..AREA.width).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].bg == red || buf[(x, y)].fg == red)
        .collect();
    let columns: std::collections::BTreeSet<u16> = cells.iter().map(|c| c.0).collect();
    let rows: std::collections::BTreeSet<u16> = cells.iter().map(|c| c.1).collect();
    (columns.len(), rows.len())
}

/// A picture smaller than the pane grows to fill it, but to no more than
/// three times its own size: a 20 px icon is 2 × 1 cells, and shows as
/// 6 × 3, not as large as the pane.
#[test]
fn a_small_picture_grows_but_no_more_than_three_times() {
    let mut app = picture_app("views_image_grows", 20, 20);
    assert_eq!(red_extent(&mut app), (6, 3));
}

/// A photo larger than three times the pane fills it, in its own shape:
/// the pane is 7 rows tall, so a square photo is 7 rows and 14 columns.
#[test]
fn a_large_picture_fills_the_pane() {
    let mut app = picture_app("views_image_fills", 400, 400);
    assert_eq!(red_extent(&mut app), (14, 7));
}

/// `{px}` is the pane's width in the pixels of the terminal's own cells,
/// as it answered at startup: on a Retina display a cell is 22 px, and a
/// Quick Look thumbnail made for 8 px cells shows at a third of the pane.
#[test]
fn px_counts_the_cells_the_terminal_reported() {
    use ratatui_image::picker::{Picker, ProtocolType};
    let app = app_with_commands(
        "views_px_cells",
        &[("a.cmd", b"x\n")],
        vec![FileType::with_view_names("*.cmd", &["px"])],
        commands(&[("px", &["echo", "{px}"], None)]),
    );
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize((22, 40).into());
    picker.set_protocol_type(ProtocolType::Iterm2);
    let mut app = app.with_image_picker(picker);
    app.focus = Focus::Explorer;
    draw(&mut app);

    arrow_to(&mut app, "a.cmd");

    let columns = u32::from(app.view_area.width - 2);
    assert_eq!(view_lines(&app), [(columns * 22).to_string()]);
}
