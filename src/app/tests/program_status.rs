//! The status recon reports to Rex after each frame (#638).

use super::*;
use base64::Engine as _;
use std::cell::RefCell;

/// A writer that keeps what the app reports, for the test to read.
#[derive(Clone, Default)]
pub(super) struct Reports(Rc<RefCell<Vec<u8>>>);

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
    /// Each report written so far, as `state` (with `:<progress>` when it
    /// has one) and the decoded `msg`, and empties the record.
    pub(super) fn take(&self) -> Vec<(String, String)> {
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
                let state = field("state").expect("a state");
                let state = match field("progress") {
                    Some(progress) => format!("{state}:{progress}"),
                    None => state,
                };
                (state, msg)
            })
            .collect()
    }
}

/// `app`, reporting to a record the test reads.
pub(super) fn reporting(app: App<'static>) -> (App<'static>, Reports) {
    let reports = Reports::default();
    (app.with_status_reports(Box::new(reports.clone())), reports)
}

/// One pass of the event loop's frame: draw, then report.
pub(super) fn frame(app: &mut App) {
    draw(app);
    app.report_status();
}

/// Wait for the explorer scan that a new filter starts, so that the next
/// frame reports the view, not the scan.
fn wait_for_scan(app: &mut App) {
    app.refresh_scan(false);
    let start = std::time::Instant::now();
    loop {
        app.drain_scan_results();
        let (answered, files) = app.explorer.answered();
        if answered == files {
            return;
        }
        assert!(start.elapsed().as_secs() < 20, "the scan never ended");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

pub(super) fn idle(msg: &str) -> (String, String) {
    ("idle".to_string(), msg.to_string())
}

pub(super) fn working(msg: &str) -> (String, String) {
    ("working".to_string(), msg.to_string())
}

/// The state of the last report in `reports`.
pub(super) fn last_state(reports: &Reports) -> String {
    reports.take().pop().expect("a report").0
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
    wait_for_scan(&mut app);
    frame(&mut app);
    assert_eq!(reports.take(), [idle("log.txt · 1 filter · 40/40 lines")]);

    key(&mut app, KeyCode::Char('u'));
    frame(&mut app);
    assert_eq!(reports.take(), [idle("log.txt · 1 filter · 11/40 lines")]);

    app.add_filter("line 2").expect("valid pattern");
    wait_for_scan(&mut app);
    frame(&mut app);
    assert_eq!(reports.take(), [idle("log.txt · 2 filters · 22/40 lines")]);
}

/// Big counts go short, as in the Activity popup there is room for a name
/// and a few words only.
#[test]
fn counts_past_four_digits_are_short() {
    let (mut app, reports) = reporting(app_over_file("status_short", &numbered_lines(12_345)));
    app.add_filter("line 1").expect("valid pattern");
    wait_for_scan(&mut app);
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

/// A pass reports `working` with how far it has got, in whole percent,
/// so a pass of any length sends at most about 100 reports. When it ends,
/// the report is `idle` with the counts again.
#[test]
fn a_filter_pass_reports_its_progress_then_idle() {
    let (mut app, reports) = reporting(app_over_file("status_pass", &numbered_lines(1000)));
    app.filtering.threshold = 0;
    app.filtering.block = 3;
    app.set_mode(document::Mode::FilteredOnly);
    frame(&mut app);
    reports.take();

    app.add_filter("line 1").expect("valid pattern");
    frame(&mut app);
    while app.filtering.is_running() {
        app.poll_filtering();
        wait_for_scan(&mut app);
        frame(&mut app);
    }

    let mut sent = reports.take();
    // "line 1" matches 1, 10–19 and 100–199.
    assert_eq!(
        sent.pop(),
        Some(idle("log.txt · 1 filter · 111/1000 lines"))
    );
    assert!(sent.len() <= 101, "{} reports", sent.len());
    let mut last = None;
    for (state, msg) in sent {
        assert_eq!(msg, "filtering log.txt");
        let percent: u8 = state
            .strip_prefix("working:")
            .and_then(|percent| percent.parse().ok())
            .unwrap_or_else(|| panic!("a working report with progress: {state}"));
        assert!(Some(percent) > last, "{percent} after {last:?}");
        last = Some(percent);
    }
    assert!(last.is_some_and(|percent| percent >= 90), "{last:?}");
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

/// A view that is late is busy work with no percentage: the name is the
/// file on its way, not the one the view still holds.
#[test]
fn a_late_view_reports_loading_then_idle() {
    let app = app_over_files("status_late", &[("a.txt", "alpha\n"), ("b.txt", "bravo\n")]);
    let (mut app, reports) = reporting(app);
    arrow_to(&mut app, "a.txt");
    let held = crate::views::pool::double::Held::default();
    app.loading.executor = Box::new(held.clone());
    frame(&mut app);
    reports.take();

    arrow_to(&mut app, "b.txt");
    frame(&mut app);
    assert_eq!(reports.take(), [working("loading b.txt")]);

    held.release();
    app.drain_views();
    frame(&mut app);
    assert_eq!(reports.take(), [idle("b.txt · 1 line")]);
}

/// `q` that waits for work to end blocks every key but the cancel, so it
/// outranks the scan that it waits for.
#[test]
fn q_that_waits_for_the_scan_reports_finishing() {
    let (mut app, reports, tx) = scanning("status_finishing");
    app.emit = Some(emit::Emit::Files);
    mark(&mut app, &tx, 0, true);
    frame(&mut app);
    reports.take();

    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.state, AppState::Finishing);
    frame(&mut app);
    assert_eq!(reports.take(), [working("finishing")]);
}

/// An app over three files with a filter, whose explorer scan answers only
/// when the test sends an answer on the `Sender`.
fn scanning(name: &str) -> (App<'static>, Reports, Sender<scan::Scanned>) {
    let app = app_over_files(
        name,
        &[("a.log", "hit\n"), ("b.log", "plain\n"), ("c.log", "hit\n")],
    );
    let (mut app, reports) = reporting(app);
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("hit").expect("valid");
    app.refresh_scan(false);
    (app, reports, tx)
}

/// The explorer scan runs in the background, so it is busy work, with the
/// count of the files in the listing. When each file has its answer, the
/// report is `idle` again.
#[test]
fn an_explorer_scan_reports_scanning_until_each_file_has_its_answer() {
    let (mut app, reports, tx) = scanning("status_scan");
    frame(&mut app);
    assert_eq!(reports.take(), [working("scanning 3 files")]);

    mark(&mut app, &tx, 0, true);
    mark(&mut app, &tx, 1, false);
    frame(&mut app);
    assert_eq!(reports.take(), []);

    mark(&mut app, &tx, 2, true);
    frame(&mut app);
    assert_eq!(last_state(&reports), "idle");
}

/// A file with no answer is not work until a scan starts: at start-up,
/// the scan waits for the first event, and Rex must not see `scanning`
/// with no scan.
#[test]
fn files_that_no_scan_has_read_yet_are_not_work() {
    let app = app_over_files(
        "status_no_scan_yet",
        &[("a.log", "hit\n"), ("b.log", "plain\n")],
    );
    let (mut app, reports) = reporting(app);
    app.add_filter("hit").expect("valid");
    frame(&mut app);
    assert_eq!(last_state(&reports), "idle");
}
