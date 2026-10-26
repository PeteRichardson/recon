use super::*;
use crate::widgets::filterlist::{self, Row};

// ---- the Require section (#548) -----------------------------------------

/// Three logs, two hosts. The scratch set has one named include filter for
/// the host line, and the filter pane has the focus with it selected.
fn hosts(name: &str) -> App<'static> {
    let mut app = app_over_files(
        name,
        &[
            ("a.log", "Robot name: Zeus\nhit a\n"),
            ("b.log", "Robot name: Hera\nhit b\n"),
            ("c.log", "hit c\nRobot name: Zeus\n"),
        ],
    );
    app.add_filter("^Robot name: ").expect("valid pattern");
    app.filters.set_details(
        0,
        filter::Details {
            name: Some("robot".into()),
            ..filter::Details::default()
        },
    );
    focus_filter_pane(&mut app);
    select_filter(&mut app, 0);
    app
}

/// Put the filter pane's cursor on the filter at `index`.
fn select_filter(app: &mut App, index: usize) {
    let row = filterlist::rows(&app.filters)
        .iter()
        .position(|row| *row == Row::Filter(index))
        .expect("the filter has a row");
    app.filters_pane.select(row);
}

/// The scratch set's require rows, as `(typed, also)`.
fn require_rows(app: &App) -> Vec<(String, Option<String>)> {
    app.filters
        .filters_in(0)
        .filter(|(_, f)| f.sense == filter::Sense::Require)
        .map(|(_, f)| {
            (
                f.predicate.typed().unwrap_or_default().to_string(),
                f.predicate.also().map(str::to_string),
            )
        })
        .collect()
}

/// Which files the require rows keep, by a real scan of each file.
fn kept(app: &mut App) -> Vec<bool> {
    app.refresh_scan(false);
    let matcher = app.filters.matcher().expect("something selects");
    app.explorer
        .files()
        .iter()
        .map(|(_, path)| {
            let progress = scan::scan_file(
                fs::File::open(path).expect("open"),
                &matcher,
                scan::Progress::default(),
                &std::sync::atomic::AtomicBool::new(false),
            );
            matcher.marks(&progress.seen)
        })
        .collect()
}

#[test]
fn ampersand_adds_a_require_row_and_asks_for_its_text() {
    let mut app = hosts("require_ampersand");
    key(&mut app, KeyCode::Char('&'));

    assert_eq!(
        require_rows(&app),
        vec![("${robot}".to_string(), Some(String::new()))]
    );
    assert_eq!(
        prompt_line(&mut app),
        "require ${robot} &",
        "the row trims the space"
    );
    typed(&mut app, "Zeus");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_none());
    assert_eq!(
        require_rows(&app),
        vec![("${robot}".to_string(), Some("Zeus".to_string()))]
    );
    let base = app
        .filters
        .filters()
        .iter()
        .find(|f| f.name.as_deref() == Some("robot"));
    let base = base.expect("the base is still there");
    assert_eq!(
        base.sense,
        filter::Sense::Include,
        "the base does not change"
    );
    assert_eq!(base.predicate.display(), "^Robot name: ");
}

#[test]
fn ampersand_on_a_filter_with_no_name_says_so_and_changes_nothing() {
    let mut app = hosts("require_no_name");
    app.add_filter("hit").expect("valid pattern");
    let hit = app.filters.len() - 1;
    select_filter(&mut app, hit);
    key(&mut app, KeyCode::Char('&'));

    assert!(app.prompt.is_none());
    assert_eq!(require_rows(&app), vec![]);
    assert!(
        message(&app).is_some_and(|m| m.contains("name the filter first")),
        "{:?}",
        message(&app)
    );
}

#[test]
fn the_palette_command_fills_the_text() {
    let mut app = hosts("require_palette");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "require Zeus");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_none(), "no prompt: the text is given");
    assert_eq!(
        require_rows(&app),
        vec![("${robot}".to_string(), Some("Zeus".to_string()))]
    );
}

#[test]
fn c_on_a_require_row_edits_only_its_text_and_the_marks_follow() {
    let mut app = hosts("require_edit");
    app.filters.set_enabled(0, false);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "require Zeus");
    key(&mut app, KeyCode::Enter);
    assert_eq!(kept(&mut app), vec![true, false, true]);

    let row = app
        .filters
        .filters()
        .iter()
        .position(|f| f.sense == filter::Sense::Require)
        .expect("a require row");
    select_filter(&mut app, row);
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(prompt_line(&mut app), "require ${robot} & Zeus");
    for _ in 0.."Zeus".len() {
        key(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "Hera");
    key(&mut app, KeyCode::Enter);

    assert_eq!(
        require_rows(&app),
        vec![("${robot}".to_string(), Some("Hera".to_string()))]
    );
    assert_eq!(kept(&mut app), vec![false, true, false]);
}

#[test]
fn the_section_heads_the_pane_and_counts_the_files_kept() {
    let mut app = hosts("require_header");
    let (_scanner, tx) = record_scans(&mut app);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "require Zeus");
    key(&mut app, KeyCode::Enter);
    app.refresh_scan(false);
    let matcher = app.filters.matcher().expect("something selects");
    for row in 0..3 {
        let path = app.explorer.files()[row].1.clone();
        let progress = scan::scan_file(
            fs::File::open(&path).expect("open"),
            &matcher,
            scan::Progress::default(),
            &std::sync::atomic::AtomicBool::new(false),
        );
        let mut result = scanned(&app, row, progress.seen, true);
        result.progress.scanned_to = progress.scanned_to;
        tx.send(result).expect("send");
    }
    app.drain_scan_results();

    let screen = rendered(&mut app);
    let lines: Vec<&str> = screen
        .lines()
        .filter(|line| {
            line.contains("Require") || line.contains("${robot}") || line.contains("robot")
        })
        .collect();
    assert!(lines[0].contains("Require  2 of 3 files"), "{lines:#?}");
    assert!(
        lines[1].contains("  01[x] req ${robot} & Zeus"),
        "{lines:#?}"
    );
    assert!(lines[2].contains("02[x] inc robot"), "{lines:#?}");
}

#[test]
fn enter_on_the_require_head_switches_every_require_row() {
    let mut app = hosts("require_head_enter");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "require Zeus");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(filterlist::rows(&app.filters)[0], Row::Require);

    key(&mut app, KeyCode::Enter);
    let on = |app: &App| -> Vec<bool> {
        app.filters
            .require_rows()
            .map(|index| app.filters.filters()[index].enabled)
            .collect()
    };
    assert_eq!(on(&app), vec![false]);
    key(&mut app, KeyCode::Enter);
    assert_eq!(on(&app), vec![true]);
}

/// `S` writes the row's base and text as two keys, and the set loads
/// back with the same row: its reference names a filter of the same set.
#[test]
fn big_s_saves_a_require_row_as_two_keys() {
    let path = save_fixture("require_save");
    let mut app = hosts("require_save_files");
    app.save_path = Some(path.clone());
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "require Zeus");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('S'));
    typed(&mut app, "hosts");
    key(&mut app, KeyCode::Enter);

    let text = fs::read_to_string(&path).expect("written");
    assert!(
        text.contains("pattern = '${robot}'\nalso = 'Zeus'\n"),
        "{text}"
    );
    let sets = crate::filtersets::parse(&text, &path).expect("loads again");
    let hosts = sets.iter().find(|set| set.name == "hosts").expect("hosts");
    let row = &hosts.filters[0];
    assert_eq!(row.sense, filter::Sense::Require);
    assert_eq!(row.predicate.typed(), Some("${robot}"));
    assert_eq!(row.predicate.also(), Some("Zeus"));
}
