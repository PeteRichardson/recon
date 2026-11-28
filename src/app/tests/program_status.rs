//! The status recon reports to Rex after each frame (#638).

use super::*;
use base64::Engine as _;
use std::cell::RefCell;

/// A writer that keeps what the app reports, for the test to read.
#[derive(Clone, Default)]
struct Reports(Rc<RefCell<Vec<u8>>>);

impl std::io::Write for Reports {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Reports {
    /// Each report written so far, as `state` and the decoded `msg`, and
    /// empties the record.
    fn take(&self) -> Vec<(String, String)> {
        let bytes = std::mem::take(&mut *self.0.borrow_mut());
        let text = String::from_utf8(bytes).expect("UTF-8 reports");
        text.split_terminator("\x1b\\")
            .map(|report| {
                let pairs = report
                    .strip_prefix("\x1b]7501;")
                    .expect("an OSC 7501 report");
                let field = |key: &str| {
                    pairs
                        .split(':')
                        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
                        .map(str::to_string)
                };
                assert_eq!(field("app").as_deref(), Some("recon"), "{pairs}");
                let msg = field("msg").map_or_else(String::new, |msg| {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(msg)
                        .expect("base64");
                    String::from_utf8(bytes).expect("UTF-8 msg")
                });
                (field("state").expect("a state"), msg)
            })
            .collect()
    }
}

/// `app`, reporting to a record the test reads.
fn reporting(app: App<'static>) -> (App<'static>, Reports) {
    let reports = Reports::default();
    (app.with_status_reports(Box::new(reports.clone())), reports)
}

/// One pass of the event loop's frame: draw, then report.
fn frame(app: &mut App) {
    draw(app);
    app.report_status();
}

fn idle(msg: &str) -> (String, String) {
    ("idle".to_string(), msg.to_string())
}

#[test]
fn the_first_frame_reports_idle_with_the_file_name_and_its_lines() {
    let (mut app, reports) = reporting(app_over_file("status_first_frame", &numbered_lines(40)));
    frame(&mut app);
    assert_eq!(reports.take(), [idle("log.txt · 40 lines")]);
}

/// The status says nothing about where the view is, so a frame after a
/// scroll sends nothing: Rex sees no traffic while you read.
#[test]
fn scrolling_sends_no_report() {
    let (mut app, reports) = reporting(app_over_file("status_scroll", &numbered_lines(400)));
    frame(&mut app);
    reports.take();
    focus_file_view(&mut app);
    for code in [
        KeyCode::Char('j'),
        KeyCode::PageDown,
        KeyCode::Char('G'),
        KeyCode::Char('g'),
    ] {
        key(&mut app, code);
        frame(&mut app);
    }
    assert_eq!(reports.take(), []);
}

/// With filters, the message says how many, and how many lines are on
/// screen of how many: the numbers the status bar gives.
#[test]
fn filters_add_their_count_and_the_shown_lines() {
    let (mut app, reports) = reporting(app_over_file("status_filters", &numbered_lines(40)));
    frame(&mut app);
    reports.take();

    // "line 1" matches line 1 and lines 10 to 19.
    app.add_filter("line 1").expect("valid pattern");
    frame(&mut app);
    assert_eq!(reports.take(), [idle("log.txt · 1 filter · 40/40 lines")]);

    key(&mut app, KeyCode::Char('u'));
    frame(&mut app);
    assert_eq!(reports.take(), [idle("log.txt · 1 filter · 11/40 lines")]);

    app.add_filter("line 2").expect("valid pattern");
    frame(&mut app);
    assert_eq!(reports.take(), [idle("log.txt · 2 filters · 22/40 lines")]);
}

/// Big counts go short, as in the Activity popup there is room for a name
/// and a few words only.
#[test]
fn counts_past_four_digits_are_short() {
    let (mut app, reports) = reporting(app_over_file("status_short", &numbered_lines(12_345)));
    app.add_filter("line 1").expect("valid pattern");
    key(&mut app, KeyCode::Char('u'));
    frame(&mut app);
    // "line 1" matches 1, 10–19, 100–199, 1000–1999 and 10000–12344.
    assert_eq!(
        reports.take().last(),
        Some(&idle("log.txt · 1 filter · 3456/12k lines"))
    );
}

/// A preview holds only the head of the file, so the total is the
/// estimate, marked as one, as on the status bar.
#[test]
fn a_previewed_total_is_the_estimate() {
    let (mut app, reports) = reporting(app_over_file("status_estimate", "alpha\n"));
    let dir = fixture_dir_path("status_estimate");
    fs::write(
        dir.join("big.txt"),
        numbered_lines(crate::views::read::PREVIEW_LINES + 100),
    )
    .expect("write");
    app.perform_widget_action(Action::Preview(dir.join("big.txt")));
    frame(&mut app);
    let (_, msg) = reports.take().pop().expect("a report");
    assert!(msg.starts_with("big.txt · ~"), "{msg}");
    assert!(msg.ends_with("k lines"), "{msg}");
}

/// A pass in blocks changes the shown count with each block. The counts
/// stay out of the message until it ends, so a pass sends two reports,
/// not one per block.
#[test]
fn a_filter_pass_sends_one_report_as_it_starts_and_one_as_it_ends() {
    let (mut app, reports) = reporting(app_over_file("status_pass", &numbered_lines(40)));
    app.filtering.threshold = 0;
    app.filtering.block = 4;
    app.set_mode(document::Mode::FilteredOnly);
    frame(&mut app);
    reports.take();

    app.add_filter("line 1").expect("valid pattern");
    frame(&mut app);
    while app.filtering.is_running() {
        app.poll_filtering();
        frame(&mut app);
    }

    assert_eq!(
        reports.take(),
        [
            idle("log.txt · 1 filter"),
            idle("log.txt · 1 filter · 11/40 lines")
        ]
    );
}

/// A directory in the view has no lines to count: its name, marked as a
/// directory the way `ls -F` marks one.
#[test]
fn a_directory_in_the_view_is_its_name_and_a_slash() {
    let (app, dir) = app_over_nested("status_directory");
    let (mut app, reports) = reporting(app);
    app.perform_widget_action(Action::Preview(dir.join("sub")));
    assert!(app.view.showing_directory());
    frame(&mut app);
    assert_eq!(reports.take().last(), Some(&idle("sub/")));
}
