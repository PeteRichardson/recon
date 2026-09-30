//! The tests of `App`, one file for each topic, and the harness they share.
//!
//! They are in `src/`, not in `tests/`, because they read the private state
//! of `App`. Each file starts with `use super::*;`, and so it gets this
//! harness and everything that `app` imports.

use super::*;
// The pane-geometry constants moved to `layout` with the methods that use
// them (#74), and are deliberately not re-exported from the crate root —
// production code outside that module has no business reading them. The
// layout tests still assert against them by name, so they are imported
// here rather than through `use super::*`.
use super::layout::{
    DOUBLE_CLICK, MAX_EXPLORER_WIDTH, MAX_FILTER_WIDTH, MIN_AUTO_EXPLORER_WIDTH,
    MIN_AUTO_FILTER_WIDTH, MIN_FILE_VIEW_WIDTH, MIN_PANE_WIDTH,
};
use crate::filter::Verdict;
use crate::fixtures::{fixture_dir, fixture_file, fixture_path as fixture_dir_path};
use crate::panes::PaneSet;
use clipboard::double::RecordingClipboard;
use crossterm::event::{
    self, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use editor::double::RecordingLauncher;
use ratatui::layout::Margin;
use ratatui::prelude::{Buffer, Color, Style, Widget};
use ratatui::style::Modifier; // the tests assert on Modifier::DIM
use scan::double::RecordingScanner;
use std::fmt::Write as _;
use std::fs;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Duration;
use unicode_width::UnicodeWidthStr;
// What `App` keeps in its topic modules, and the tests read.
use super::focus::LAST_PANE;
use super::prompt::{HISTORY_CAP, INVALID_PATTERN, PromptKind};
use super::render::status::{AND_BADGE_TEXT, HIDE_BADGE_STYLE, HIDE_BADGE_TEXT, elide_left};
use super::search::{WRAPPED_TO_BOTTOM, WRAPPED_TO_TOP};
use super::viewport::Step;
use crate::widgets::Action;
use crate::widgets::explorer::Match;

/// `n` newline-terminated lines, `line 0` through `line n-1`.
///
/// One buffer appended to, not `(0..n).map(|i| format!(...)).collect()`:
/// the latter allocates and drops a `String` per line, which is quadratic
/// and showed up across ~15 fixtures in this file and `widgets/fileview.rs`
/// (#90). `write!` into a `String` cannot fail, hence the discarded result.
fn numbered_lines(n: usize) -> String {
    (0..n).fold(String::new(), |mut body, i| {
        let _ = writeln!(body, "line {i}");
        body
    })
}

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 120,
    height: 10,
};

/// An app listing a directory with known entry names.
fn app_over(name: &str, files: &[&str]) -> App<'static> {
    let dir = fixture_dir(name);
    for file in files {
        fs::write(dir.join(file), "x").expect("write fixture");
    }
    App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    })
}

/// `app_over`, with real contents. The app starts on a placeholder path
/// that does not exist, so nothing is loaded until `open_file`.
fn app_over_files(name: &str, files: &[(&str, &str)]) -> App<'static> {
    let dir = fixture_dir(name);
    for (file, body) in files {
        fs::write(dir.join(file), body).expect("write fixture");
    }
    App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    })
}

/// Select the `row`th file in the explorer and load it into the view,
/// the way `Enter` in the explorer would.
fn open_file(app: &mut App, row: usize) {
    let (index, path) = app.explorer.files()[row].clone();
    app.explorer.select_entry(index);
    app.perform_widget_action(Action::Load(path));
}

/// Mark the `row`th file as matching (`yes`) or not, through the scan
/// result channel — the path the real scanner uses.
///
/// `scanned_to` advances past whatever is already held for the file:
/// `drain_scan_results` (#119) keeps a finished record's answer unless a
/// later message reads further, by design (`a_result_is_kept_only_if_it_read_further`).
/// A fixture that re-marks the same row — this is the only one that does,
/// to flip a file from matching to not — has to advance too, or the
/// second answer is silently dropped as a stale duplicate.
fn mark(app: &mut App, tx: &Sender<scan::Scanned>, row: usize, yes: bool) {
    let seen = if yes { vec![0b1] } else { vec![0] };
    let mut result = scanned(app, row, seen, true);
    result.progress.scanned_to = app
        .scan_cache
        .records
        .get(&result.path)
        .map_or(1, |held| held.progress.scanned_to + 1);
    tx.send(result).expect("send");
    app.drain_scan_results();
}

/// The file the view is showing, by name.
fn shown(app: &App) -> String {
    app.view
        .filename()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn draw(app: &mut App) {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
}

/// A mouse event of `kind` at `column`, `row`.
///
/// The click clock is frozen on the first event, so two clicks are a
/// double-click however slow the machine is (#391). A test of a slow second
/// click moves it on with `later`.
fn mouse_at(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    if !app.click_clock.is_set() {
        app.click_clock.set(std::time::Instant::now());
    }
    app.handle_event(event::Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    }));
}

/// Move the click clock `by` on from where it is.
fn later(app: &mut App, by: Duration) {
    let now = app.click_clock.now();
    app.click_clock.set(now + by);
}

/// Row 3 is inside the panes on every fixture area used here. Both
/// dividers run the full height, so any such row hits them.
fn mouse(app: &mut App, kind: MouseEventKind, column: u16) {
    mouse_at(app, kind, column, 3);
}

/// Press a key, as a real terminal would report it.
///
/// `SHIFT` on an uppercase letter is not decoration: crossterm attaches it
/// in legacy mode, and a helper that omitted it hid #250 for the life of
/// the test suite.
fn key(app: &mut App, code: KeyCode) {
    let modifiers = match code {
        KeyCode::Char(c) if c.is_uppercase() => KeyModifiers::SHIFT,
        _ => KeyModifiers::empty(),
    };
    app.handle_event(event::Event::Key(KeyEvent::new(code, modifiers)));
}

/// `key`, with Control held.
fn ctrl(app: &mut App, code: KeyCode) {
    app.handle_event(event::Event::Key(event::KeyEvent::new(
        code,
        KeyModifiers::CONTROL,
    )));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

/// How many `Tab` presses can be needed to reach any pane from any other.
///
/// A bound for the tab-until-focused helpers below, not an invariant the
/// production code checks — `Focus` has three variants, so a full cycle is
/// three presses. It replaces the `app.widgets.len()` those loops used to
/// read, which is one of the things the vec was doing that a `Focus` does
/// not need to (#73).
const PANE_COUNT: usize = 3;

/// Put focus on the file view, however many `Tab` presses that takes.
///
/// A fixed `key(Tab); ...; key(Tab);` pair only reaches the file view
/// because `Tab` used to be a two-state toggle; the filter pane joining
/// the cycle once a filter exists already broke that assumption for
/// three tests. Tabbing until the target is reached, rather than a fixed
/// number of times, is robust to wherever focus started.
fn focus_file_view(app: &mut App) {
    for _ in 0..PANE_COUNT {
        if app.focus == Focus::View {
            return;
        }
        key(app, KeyCode::Tab);
    }
    panic!("could not reach the file view by tabbing");
}

/// Put focus on the filter pane, however many `Tab` presses that takes.
/// Bounded the same way as `focus_file_view`, for the same reason: a
/// fixed count would break the moment a fourth pane joined the cycle.
fn focus_filter_pane(app: &mut App) {
    for _ in 0..PANE_COUNT {
        if app.focus == Focus::Filters {
            return;
        }
        key(app, KeyCode::Tab);
    }
    panic!("could not reach the filter pane by tabbing");
}

fn prompt_line(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let y = AREA.height - 1;
    (0..AREA.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}
fn prompt<'a>(app: &'a App) -> &'a SearchPrompt {
    app.prompt.as_ref().expect("the prompt should be open")
}

/// Returns the styles the file view is currently rendering with.
fn view_line_styles(app: &App) -> Vec<Option<Style>> {
    app.view.textarea().line_styles().to_vec()
}

/// Create (or recreate) `target/test-appdirs/<name>/log.txt` with `body`,
/// claiming the fixture directory name first so a duplicate is rejected
/// loudly rather than racing another test for the same path.
fn fixture_path(name: &str, body: &str) -> std::path::PathBuf {
    let dir = fixture_dir(name);
    let file = dir.join("log.txt");
    fs::write(&file, body).expect("write fixture");
    file
}

fn app_over_file(name: &str, body: &str) -> App<'static> {
    let file = fixture_path(name, body);
    App::new(&Config {
        path: file.display().to_string(),
        ..Config::default()
    })
}

/// The bottom row when no prompt is open.
fn status_line(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let y = AREA.height - 1;
    (0..AREA.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The bottom row at an arbitrary width, for the narrow-terminal cases
/// `AREA`'s 120 columns are far too generous to reach.
fn status_line_at(app: &mut App, width: u16) -> String {
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height: 10,
    };
    let mut buf = Buffer::empty(area);
    app.render(area, &mut buf);
    let y = area.height - 1;
    (0..area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn view_cursor_row(app: &App) -> usize {
    app.view.textarea().cursor().0
}

/// The text the file view is currently showing, one entry per row.
fn view_lines(app: &App) -> Vec<String> {
    app.view.textarea().lines().to_vec()
}

/// The cursor's source line, derived from where it sits in the view.
fn cursor_source(app: &App) -> usize {
    let row = app.view.cursor_visible_row();
    app.document.source_at(row).unwrap_or(row)
}

fn cursor_screen_row(app: &App) -> u16 {
    app.view.cursor_screen_row()
}
/// A `filters.toml` path under `target/` that does not exist yet.
fn save_fixture(name: &str) -> std::path::PathBuf {
    fixture_dir(name).join("recon").join("filters.toml")
}
fn app_with_three_sets(fixture: &str) -> App<'static> {
    let mut a = filter::test_support::loaded("a", 10, true, &["alpha"]);
    a.profiles.insert("default".into(), vec!["alpha".into()]);
    let mut b = filter::test_support::loaded("b", 20, true, &["beta"]);
    b.profiles.insert("default".into(), vec!["beta".into()]);
    let c = filter::test_support::loaded("c", 30, false, &["gamma"]);
    let mut app = app_over_file(fixture, "alpha\nbeta\ngamma\nscratch\n");
    app.filters = ActiveFilters::with_sets(None, &[a, b, c]);
    app.add_filter("scratch").expect("valid");
    app
}

fn included(app: &App) -> usize {
    app.document
        .verdicts()
        .iter()
        .filter(|v| matches!(v, filter::Verdict::Included(_)))
        .count()
}
fn app_with_profiles(fixture: &str) -> App<'static> {
    let mut a = filter::test_support::loaded("a", 10, true, &["alpha", "beta"]);
    a.profiles.insert("default".into(), vec!["alpha".into()]);
    a.profiles.insert("only-beta".into(), vec!["beta".into()]);
    let b = filter::test_support::loaded("b", 20, true, &["neither"]);
    let mut app = app_over_file(fixture, "alpha\nbeta\nneither\n");
    app.filters = ActiveFilters::with_sets(None, &[a, b]);
    app.refresh_view();
    app
}

fn flags_of(app: &App, set: usize) -> Vec<bool> {
    app.filters
        .filters_in(set)
        .map(|(_, f)| f.enabled)
        .collect()
}

fn rendered(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    (0..AREA.height)
        .map(|y| {
            (0..AREA.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
/// A pane tall enough that the centre row and the margin's edge are
/// different rows — on `AREA` they coincide. 36 rows is 33 of text once
/// the status line and the pane's borders are taken off.
const TALL: Rect = Rect {
    x: 0,
    y: 0,
    width: 120,
    height: 36,
};

fn draw_tall(app: &mut App) {
    let mut buf = Buffer::empty(TALL);
    app.render(TALL, &mut buf);
}

/// The pattern the search holds, for tests.
fn search_text(app: &App) -> String {
    app.search
        .as_ref()
        .map(|search| search.text.clone())
        .expect("a search was set")
}
fn selected_name(app: &App) -> String {
    app.explorer.selected_name().expect("an entry is selected")
}
fn status<'a>(app: &'a App<'a>) -> Option<&'a str> {
    app.status_message.as_ref().map(|m| m.text.as_str())
}

impl App<'_> {
    /// The pattern the file view is currently highlighting, for tests.
    fn file_view_highlight(&self) -> Option<String> {
        self.view.highlight()
    }
}

/// A fixture project: `target/test-appdirs/<name>/` with a `go.mod` marker
/// in it and `log.txt` inside a `logs/` subdirectory, so the walk-up has an
/// actual level to climb.
///
/// `go.mod` rather than `Cargo.toml` purely so nothing under `target/`
/// looks like a crate to any tool that goes wandering; every marker in the
/// table is proved equivalent by `every_marker_in_the_table_is_recognised`
/// over in `editor.rs`.
/// The template is pinned to the compiled-in default rather than left to
/// resolve, and that is not belt-and-braces: `Config::editor_templates`
/// reads the real `$VISUAL`/`$EDITOR`, so on any machine whose developer
/// has one set (`hx`, here) these tests would assert against *their*
/// editor. Pinning the top rung takes the environment out of it. The ladder
/// below is unit-tested with the environment injected, in `editor.rs`.
fn app_over_project(name: &str, body: &str) -> (App<'static>, std::path::PathBuf) {
    let root = fixture_dir(name);
    fs::create_dir_all(root.join("logs")).expect("create fixture project");
    fs::write(root.join("go.mod"), "module fixture\n").expect("write marker");
    let file = root.join("logs/log.txt");
    fs::write(&file, body).expect("write fixture");
    let app = App::new(&Config {
        path: file.display().to_string(),
        editor: Some(editor::DEFAULT_PROJECT_TEMPLATE.to_string()),
        ..Config::default()
    });
    (
        app,
        std::path::absolute(&root).expect("absolute fixture root"),
    )
}

fn absolute(path: &std::path::Path) -> String {
    std::path::absolute(path)
        .expect("absolute path")
        .display()
        .to_string()
}

/// Tall and wide on purpose: `AREA` is ten rows, and the overlay flows its
/// sections into however many columns the area allows, so a ten-row buffer
/// would clip away everything these tests assert on.
const HELP_AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 160,
    height: 44,
};

/// The whole rendered frame as text, at `HELP_AREA`.
fn screen(app: &mut App) -> String {
    let mut buf = Buffer::empty(HELP_AREA);
    app.render(HELP_AREA, &mut buf);
    (0..HELP_AREA.height)
        .map(|y| {
            (0..HELP_AREA.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Swap in the recording double and a channel the test controls.
fn record_scans(app: &mut App) -> (Rc<RecordingScanner>, Sender<scan::Scanned>) {
    let scanner = Rc::new(RecordingScanner::default());
    app.scanner = Box::new(Rc::clone(&scanner));
    let (tx, rx) = std::sync::mpsc::channel();
    app.scan_results = Some(rx);
    (scanner, tx)
}

fn scanned(app: &App, row: usize, seen: Vec<filter::Bits>, eof: bool) -> scan::Scanned {
    let (index, path) = app.explorer.files()[row].clone();
    scan::Scanned {
        cache_id: app.scan_cache.id,
        index,
        stamp: scan::stamp(&path).ok(),
        path,
        progress: scan::Progress {
            seen,
            scanned_to: 1,
            eof,
        },
    }
}

/// A directory holding `sub/inner_a.log`, `sub/inner_b.log` and `z.log`,
/// listed by a fresh app. Rows: `..`, `sub/`, `z.log`.
fn app_over_nested(name: &str) -> (App<'static>, std::path::PathBuf) {
    let dir = fixture_dir(name);
    fs::create_dir_all(dir.join("sub")).expect("create subdir");
    fs::write(dir.join("sub/inner_a.log"), "a\n").expect("write");
    fs::write(dir.join("sub/inner_b.log"), "b\n").expect("write");
    fs::write(dir.join("z.log"), "z\n").expect("write");
    let app = App::new(&Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    });
    (app, dir)
}

/// The status row's transient message, or `None`.
fn message<'a>(app: &'a App<'_>) -> Option<&'a str> {
    app.status_message.as_ref().map(|m| m.text.as_str())
}

mod and_mode;
mod chains;
mod emit_on_quit;
mod explorer_search;
mod filter_editor;
mod filters;
mod focus;
mod hex;
mod hiding;
mod jumps;
mod layout;
mod mouse;
mod open;
mod peek;
mod prompt;
mod scanning;
mod search;
mod set_picker;
mod sets;
mod show_hide;
mod star;
mod startup;
mod status_line;
mod viewport;
mod visual;
mod warnings;
