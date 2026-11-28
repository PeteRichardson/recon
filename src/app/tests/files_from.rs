//! `--files-from` in the TUI (#519): the listed files in the explorer, as a
//! directory of links that grows as the list comes.
//!
//! The list is a real pipe the test writes into, and the links are made on
//! a thread of their own, so each step waits, briefly, for what it did.

use super::*;
use crate::files_from::LinkDir;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// An app over a `--files-from` list, as `main` builds it, with the list a
/// pipe the test holds the writing end of. Drawn once so the panes know
/// their sizes.
fn app_over_list(name: &str) -> (App<'static>, std::io::PipeWriter) {
    let (reader, writer) = std::io::pipe().expect("a pipe");
    let links = LinkDir::start(
        &fixture_dir(&format!("{name}_tmp")),
        std::io::BufReader::new(reader),
        false,
    )
    .expect("started");
    let mut config = Config::default();
    config.list_files_into(links.path());
    let mut app = App::new(&Startup::from(config)).with_links(Some(links));
    draw(&mut app);
    (app, writer)
}

/// `files`, each holding its own name, in a fresh fixture directory, with
/// the absolute path of each.
fn listed_files(name: &str, files: &[&str]) -> Vec<PathBuf> {
    let dir = crate::path::lexical_absolute(&fixture_dir(name));
    files
        .iter()
        .map(|file| {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            fs::write(&path, format!("{file}\n")).expect("write");
            path
        })
        .collect()
}

/// Send `paths` down the list, one per line.
fn send(writer: &mut std::io::PipeWriter, paths: &[&Path]) {
    for path in paths {
        writeln!(writer, "{}", path.display()).expect("write");
    }
}

/// Take what the list and the loads did until `done`, or fail after 10 s.
fn until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) {
    let start = Instant::now();
    while !done(app) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(5));
        app.drain_views();
        app.poll_links();
    }
}

/// The explorer's rows, `..` first.
fn rows(app: &App) -> Vec<String> {
    app.explorer
        .entries()
        .iter()
        .map(|entry| entry.name.to_string_lossy().into_owned())
        .collect()
}

/// The paths come into the explorer as they are sent, in the order of the
/// list and not sorted by name. The first one is selected and shown.
#[test]
fn the_listed_files_come_in_the_order_of_the_list_and_the_first_is_shown() {
    let files = listed_files("files_from_app_order", &["zebra.log", "apple.log"]);
    let (mut app, mut writer) = app_over_list("files_from_app_order");

    send(&mut writer, &[&files[0]]);
    until(&mut app, |app| rows(app) == ["..", "zebra.log"]);
    send(&mut writer, &[&files[1]]);
    until(&mut app, |app| {
        rows(app) == ["..", "zebra.log", "apple.log"]
    });

    assert_eq!(selected_name(&app), "zebra.log");
    until(&mut app, |app| *app.document.lines() == ["zebra.log"]);
}

/// A later path with the same name renames the earlier link. The
/// selection and the view stay on that file, under its new name.
#[test]
fn a_rename_keeps_the_selection_and_the_view_on_the_file() {
    let files = listed_files("files_from_app_rename", &["a/log.txt", "b/log.txt"]);
    let (mut app, mut writer) = app_over_list("files_from_app_rename");
    send(&mut writer, &[&files[0]]);
    until(&mut app, |app| *app.document.lines() == ["a/log.txt"]);

    send(&mut writer, &[&files[1]]);
    until(&mut app, |app| {
        rows(app) == ["..", "a__log.txt", "b__log.txt"]
    });

    assert_eq!(selected_name(&app), "a__log.txt");
    assert_eq!(
        app.view
            .filename()
            .file_name()
            .and_then(|name| name.to_str()),
        Some("a__log.txt")
    );
    assert_eq!(*app.document.lines(), ["a/log.txt"]);
}

/// A listed path that does not exist gets no row. When the list ends, one
/// warning names each one that was skipped.
#[test]
fn the_end_of_the_list_names_the_missing_paths_in_one_warning() {
    let files = listed_files("files_from_app_missing", &["here.log"]);
    let dir = files[0].parent().expect("a parent").to_path_buf();
    let (mut app, mut writer) = app_over_list("files_from_app_missing");

    send(
        &mut writer,
        &[&dir.join("gone.log"), &files[0], &dir.join("lost.log")],
    );
    until(&mut app, |app| rows(app) == ["..", "here.log"]);
    assert_eq!(message(&app), None, "no warning while the list is open");
    drop(writer);
    until(&mut app, |app| app.status_message.is_some());

    assert!(app.status_message.as_ref().is_some_and(|m| m.error));
    assert_eq!(
        message(&app),
        Some(&*format!(
            "--files-from: no such file: {}, {}",
            dir.join("gone.log").display(),
            dir.join("lost.log").display()
        ))
    );
}

/// A list that is linked before the app starts: the first file of the
/// list is selected and shown, not the folder or the first by name.
#[test]
fn a_list_linked_before_the_start_shows_its_first_file() {
    let files = listed_files("files_from_app_early", &["zebra.log", "apple.log"]);
    let list = format!("{}\n{}\n", files[0].display(), files[1].display());
    let links = LinkDir::start(
        &fixture_dir("files_from_app_early_tmp"),
        std::io::Cursor::new(list),
        false,
    )
    .expect("started");
    let start = Instant::now();
    while !links.path().join("apple.log").exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::yield_now();
    }
    let mut config = Config::default();
    config.list_files_into(links.path());

    let mut app = App::new(&Startup::from(config)).with_links(Some(links));

    until(&mut app, |app| {
        rows(app) == ["..", "zebra.log", "apple.log"]
    });
    assert_eq!(selected_name(&app), "zebra.log");
    until(&mut app, |app| *app.document.lines() == ["zebra.log"]);
}

/// With a filter on, a link that comes later is scanned too, so its row
/// can show whether the file matches.
#[test]
fn a_link_that_comes_later_is_scanned() {
    let files = listed_files("files_from_app_scan", &["first.log", "later.log"]);
    let (mut app, mut writer) = app_over_list("files_from_app_scan");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    send(&mut writer, &[&files[0]]);
    until(&mut app, |app| rows(app) == ["..", "first.log"]);

    send(&mut writer, &[&files[1]]);
    until(&mut app, |app| {
        rows(app) == ["..", "first.log", "later.log"]
    });

    let scanned: Vec<_> = scanner
        .requests()
        .iter()
        .flat_map(|request| request.files.iter())
        .filter_map(|file| file.path.file_name().map(std::ffi::OsStr::to_os_string))
        .collect();
    assert!(
        scanned.iter().any(|name| name == "later.log"),
        "scanned: {scanned:?}"
    );
}
