use super::*;

// ---- explorer filter matches (#119) ----------------------------------

/// The file on the command line is coloured once: the theme is set
/// before the load, not after it, so there is no second grammar lookup
/// at startup (#186).
#[test]
fn startup_builds_one_highlighter() {
    let dir = fixture_dir("startup_highlighter");
    let file = dir.join("main.rs");
    fs::write(&file, "fn main() {}\n").expect("write fixture");

    let app = App::new(&Startup::from(Config {
        path: file.display().to_string(),
        ..Config::default()
    }));

    assert_eq!(app.view.highlighters_built, 1);
}

fn app_over_logs(name: &str) -> App<'static> {
    app_over(name, &["a.log", "b.log"])
}

#[test]
fn adding_a_filter_starts_a_scan_of_every_file() {
    let mut app = app_over_logs("scan_start");
    let (scanner, _tx) = record_scans(&mut app);

    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);

    let requests = scanner.requests();
    assert_eq!(requests.len(), 1);
    let names: Vec<_> = requests[0]
        .files
        .iter()
        .map(|file| file.path.file_name().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["a.log", "b.log"]);
}

#[test]
fn nothing_selecting_means_no_scan_and_no_answers() {
    let mut app = app_over_logs("scan_off");
    let (scanner, _tx) = record_scans(&mut app);

    app.refresh_scan(false);
    assert!(scanner.requests().is_empty(), "empty set");

    app.filters.add_excluding("noise").expect("valid pattern");
    app.refresh_scan(false);
    assert!(scanner.requests().is_empty(), "exclude only");
}

/// The guard: an unchanged state is free. `j` in the file view must not
/// stat the folder.
#[test]
fn an_unchanged_state_issues_nothing() {
    let mut app = app_over_logs("scan_guard");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    assert_eq!(scanner.requests().len(), 1);

    app.refresh_scan(false);
    app.refresh_scan(false);

    assert_eq!(scanner.requests().len(), 1);
}

#[test]
fn force_bypasses_the_guard() {
    let mut app = app_over_logs("scan_force");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);

    app.refresh_scan(true);

    assert_eq!(scanner.requests().len(), 2);
}

/// The point of the bitset cache: with every answer known, a toggle issues
/// no request and touches no thread.
#[test]
fn a_toggle_with_every_answer_cached_issues_no_scan() {
    let mut app = app_over_logs("scan_cached");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.add_filter("beta").expect("valid pattern");
    app.refresh_scan(false);
    let request = &scanner.requests()[0];
    // Pretend the scan finished: every file read to EOF, one matched beta.
    for (i, scan::FileToScan { index, path, .. }) in request.files.iter().enumerate() {
        let seen = if i == 0 { vec![0, 0b10] } else { vec![0] };
        app.scan_cache.records.insert(
            path.clone(),
            scan::Record {
                stamp: scan::stamp(path).ok(),
                progress: scan::Progress {
                    seen,
                    scanned_to: 1,
                    eof: true,
                },
            },
        );
        let _ = index;
    }

    app.filters.set_enabled(1, false);
    app.refresh_scan(false);
    app.filters.set_enabled(1, true);
    app.refresh_scan(false);
    app.filters.toggle_context(0);
    app.refresh_scan(false);

    assert_eq!(scanner.requests().len(), 1, "a toggle re-scanned");
    assert!(matches!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        widgets::explorer::Match::Yes(_)
    ));
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[1].0].matched,
        widgets::explorer::Match::No
    );
}

/// An edit keeps the pattern count, so only the pattern generation
/// tells the guard that the bits now mean something else (#186).
#[test]
fn editing_a_pattern_scans_again() {
    let mut app = app_over_logs("scan_edit");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);

    app.filters.set_pattern(0, "beta").expect("valid pattern");
    app.refresh_scan(false);

    assert_eq!(scanner.requests().len(), 2);
}

#[test]
fn a_pattern_change_drops_the_cache_and_bumps_its_id() {
    let mut app = app_over_logs("scan_key");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    let first = scanner.requests()[0].cache_id;

    app.add_filter("beta").expect("valid pattern");
    app.refresh_scan(false);

    assert_ne!(scanner.requests()[1].cache_id, first);
}

/// Peek drops every filter and puts them back; the round trip is free.
#[test]
fn a_peek_round_trip_issues_no_scan() {
    let mut app = app_over_logs("scan_peek");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    for scan::FileToScan { path, .. } in &scanner.requests()[0].files {
        app.scan_cache.records.insert(
            path.clone(),
            scan::Record {
                stamp: scan::stamp(path).ok(),
                progress: scan::Progress {
                    seen: vec![0],
                    scanned_to: 1,
                    eof: true,
                },
            },
        );
    }
    app.refresh_scan(true);
    let before = scanner.requests().len();

    key(&mut app, KeyCode::Char(' '));
    assert!(
        app.explorer
            .entries()
            .iter()
            .all(|e| e.matched == widgets::explorer::Match::Unknown),
        "peek must un-dim"
    );
    key(&mut app, KeyCode::Char(' '));

    assert_eq!(scanner.requests().len(), before);
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        widgets::explorer::Match::No
    );
}

#[test]
fn a_matched_file_takes_the_colour_of_the_filter_that_selected_it() {
    let mut app = app_over_logs("scan_colour");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.add_filter("beta").expect("valid pattern");
    app.refresh_scan(false);
    let path = scanner.requests()[0].files[0].path.clone();
    app.scan_cache.records.insert(
        path.clone(),
        scan::Record {
            stamp: scan::stamp(&path).ok(),
            progress: scan::Progress {
                seen: vec![0b10],
                scanned_to: 1,
                eof: true,
            },
        },
    );

    app.refresh_scan(true);

    let expected = app.filters.filters()[1].style;
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        widgets::explorer::Match::Yes(expected)
    );
}

#[test]
fn a_result_answers_its_row_and_asks_for_a_redraw() {
    let mut app = app_over_logs("drain_basic");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);

    tx.send(scanned(&app, 0, vec![0b1], false)).expect("send");
    tx.send(scanned(&app, 1, vec![0], true)).expect("send");

    assert!(
        app.drain_scan_results(),
        "answers arrived, the frame is stale"
    );
    assert!(matches!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::Yes(_)
    ));
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[1].0].matched,
        Match::No
    );
    assert!(!app.drain_scan_results(), "nothing new");
}

#[test]
fn a_result_from_a_replaced_cache_is_dropped() {
    let mut app = app_over_logs("drain_stale");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    let mut stale = scanned(&app, 0, vec![0b1], false);
    stale.cache_id += 1;

    tx.send(stale).expect("send");

    assert!(!app.drain_scan_results());
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::Unknown
    );
}

/// A result that reaches further than the record held replaces it; one
/// that does not — a cancelled worker's partial — is ignored.
#[test]
fn a_result_is_kept_only_if_it_read_further() {
    let mut app = app_over_logs("drain_further");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    let mut far = scanned(&app, 0, vec![0], true);
    far.progress.scanned_to = 50;
    let mut near = scanned(&app, 0, vec![0b1], false);
    near.progress.scanned_to = 10;

    tx.send(far).expect("send");
    tx.send(near).expect("send");
    app.drain_scan_results();

    let (_, path) = &app.explorer.files()[0];
    assert_eq!(app.scan_cache.records[path].progress.scanned_to, 50);
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::No
    );
}

/// The row a result names may no longer be the file it was for.
#[test]
fn a_result_for_a_row_that_now_holds_another_file_updates_the_cache_only() {
    let mut app = app_over_logs("drain_moved");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    let mut moved = scanned(&app, 0, vec![0b1], false);
    moved.index = app.explorer.files()[1].0;

    tx.send(moved.clone()).expect("send");
    app.drain_scan_results();

    assert!(app.scan_cache.records.contains_key(&moved.path));
    assert_eq!(
        app.explorer.entries()[moved.index].matched,
        Match::Unknown,
        "applied to the wrong row"
    );
}

#[test]
fn a_disconnected_scanner_is_survived() {
    let mut app = app_over_logs("drain_gone");
    let (_scanner, tx) = record_scans(&mut app);
    drop(tx);

    assert!(!app.drain_scan_results());
}

/// The explorer's listed rows, read by rendering it — `entries` and
/// `visible` are private to `explorer`, so this is the only way a test
/// outside that module can see which rows are actually drawn.
fn explorer_rows(app: &mut App) -> Vec<String> {
    let area = Rect::new(0, 0, 40, 20);
    let mut buf = Buffer::empty(area);
    (&mut app.explorer).render(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn hide_mode_hides_non_matching_files_in_the_explorer_too() {
    let mut app = app_over_logs("hide_both");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0], true)).expect("send");
    tx.send(scanned(&app, 1, vec![0b1], false)).expect("send");
    app.drain_scan_results();

    ctrl(&mut app, KeyCode::Char('h'));
    assert!(
        !explorer_rows(&mut app).iter().any(|r| r.contains("a.log")),
        "a.log should be hidden"
    );

    ctrl(&mut app, KeyCode::Char('h'));
    assert!(explorer_rows(&mut app).iter().any(|r| r.contains("a.log")));
}

#[test]
fn peek_shows_every_file_and_restoring_hides_them_again() {
    let mut app = app_over_logs("hide_peek");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0], true)).expect("send");
    app.drain_scan_results();
    ctrl(&mut app, KeyCode::Char('h'));
    assert!(!explorer_rows(&mut app).iter().any(|r| r.contains("a.log")));

    key(&mut app, KeyCode::Char(' '));
    assert!(
        explorer_rows(&mut app).iter().any(|r| r.contains("a.log")),
        "peek must show the plain listing"
    );

    key(&mut app, KeyCode::Char(' '));
    assert!(!explorer_rows(&mut app).iter().any(|r| r.contains("a.log")));
}

/// #120 §10: hide mode is toggled often and lived behind Shift. `u`
/// ("unmatched") is the primary key now; `H` and `Ctrl-H` stay as aliases.
#[test]
fn u_toggles_hiding_like_ctrl_h() {
    let mut app = app_over_file("u_hides", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    app.filters.add("beta").expect("valid pattern");
    app.refresh_view();
    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity");

    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.document.mode(), Mode::FilteredOnly, "u did not hide");

    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.document.mode(), Mode::Dimmed, "u did not restore");

    // The aliases still work, and share the state.
    key(&mut app, KeyCode::Char('H'));
    assert_eq!(app.document.mode(), Mode::FilteredOnly);
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(app.document.mode(), Mode::Dimmed);
}

/// `Ctrl-u` must reach the panes: the global `u` arm is guarded on an
/// empty modifier set precisely so it does not swallow this.
#[test]
fn ctrl_d_and_ctrl_u_page_the_explorer_through_the_app() {
    let files: Vec<String> = (0..30).map(|i| format!("f{i:02}.log")).collect();
    let names: Vec<&str> = files.iter().map(String::as_str).collect();
    let mut app = app_over("ctrl_page_explorer", &names);
    draw(&mut app);
    key(&mut app, KeyCode::Char('e'));
    let before = app.explorer.selected_name();

    ctrl(&mut app, KeyCode::Char('d'));
    let after = app.explorer.selected_name();
    assert_ne!(after, before, "Ctrl-d did not move the explorer");
    assert_eq!(
        app.document.mode(),
        Mode::Dimmed,
        "Ctrl-d toggled hide mode"
    );

    ctrl(&mut app, KeyCode::Char('u'));
    assert_eq!(
        app.explorer.selected_name(),
        before,
        "Ctrl-u did not move back"
    );
    assert_eq!(
        app.document.mode(),
        Mode::Dimmed,
        "Ctrl-u toggled hide mode"
    );
}

/// `u` is global: it works with the explorer focused, not only the view.
#[test]
fn u_toggles_hiding_from_the_explorer() {
    let mut app = app_over_file("u_from_explorer", "alpha\nbeta\n");
    app.filters.add("beta").expect("valid pattern");
    app.refresh_view();
    key(&mut app, KeyCode::Char('e'));

    key(&mut app, KeyCode::Char('u'));

    assert_eq!(app.document.mode(), Mode::FilteredOnly);
    assert_eq!(app.focus, Focus::Explorer, "focus moved");
}

#[test]
fn a_file_that_changed_on_disk_is_rescanned() {
    let mut app = app_over_logs("poll_changed");
    let (scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0], true)).expect("send");
    app.drain_scan_results();
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::No
    );
    let (_, path) = app.explorer.files()[0].clone();
    fs::write(&path, "now alpha is here\nand more\n").expect("rewrite");

    assert!(app.check_stamps());

    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::Unknown
    );
    let last = scanner.requests().last().expect("a rescan").clone();
    assert!(
        last.files.iter().any(|file| file.path == path),
        "the changed file should be in the reissued request: {last:?}"
    );
}

#[test]
fn an_unchanged_listing_is_free() {
    let mut app = app_over_logs("poll_same");
    let (scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0], true)).expect("send");
    tx.send(scanned(&app, 1, vec![0], true)).expect("send");
    app.drain_scan_results();
    let before = scanner.requests().len();

    assert!(!app.check_stamps());
    assert_eq!(scanner.requests().len(), before);
}

#[test]
fn the_active_file_changing_raises_the_badge() {
    let mut app = app_over_logs("poll_badge");
    let (_scanner, tx) = record_scans(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    // `j` from the default selection (already on `a.log`) lands on
    // `b.log`, so it is row 1 — not row 0 — that becomes the active file
    // `Enter` loads. Scanning row 0 here would leave `b.log` out of the
    // cache and `check_stamps` would have nothing to compare it against.
    tx.send(scanned(&app, 1, vec![0], true)).expect("send");
    app.drain_scan_results();
    assert!(!status_line(&mut app).contains("changed on disk"));
    fs::write(app.view.filename(), "rewritten\n").expect("rewrite");

    app.check_stamps();

    assert!(
        status_line(&mut app).contains("changed on disk"),
        "{}",
        status_line(&mut app)
    );
}

/// The stale badge is generated from `DEFAULT`, not hard-coded (task 8
/// fix round 2, #199). The assertion below is a literal —
/// `" changed on disk · r "` — so what this actually pins is that the
/// badge names the reload key under the *default* keymap, which is what
/// catches the text going back to being hard-coded. It does not catch
/// the badge going stale after a rebind — that is the test below.
#[test]
fn the_stale_badge_names_the_reload_key() {
    let mut app = app_over_logs("stale_badge_key");
    app.view_stale = true;

    assert!(
        status_line(&mut app).contains(" changed on disk · r "),
        "the badge must name the key that reloads: {}",
        status_line(&mut app)
    );
}

/// The rebind half of the test above (#61). The badge names a key looked
/// up in the table `App` holds, so moving `global.reload` in `config.toml`
/// moves the key the badge names.
///
/// This is also the only test that the resolved table reaches `App` at
/// all: `Keymap::new` is exercised directly in `keymap.rs`, but nothing
/// there would notice `App::new` ignoring `startup.bindings` and keeping
/// the defaults.
#[test]
fn the_stale_badge_names_a_rebound_reload_key() {
    let dir = fixture_dir("stale_badge_rebound");
    fs::write(dir.join("a.log"), "x").expect("write fixture");
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert("global.reload".to_string(), vec!["F5".to_string()]);
    // Through `keymap::config::build`, which is what `startup::start`
    // calls, so this covers the whole path a `config.toml` line takes to
    // the screen.
    let config = Config {
        path: dir.join("placeholder").display().to_string(),
        ..Config::default()
    };
    let (bindings, _) = crate::keymap::config::build(
        &crate::keymap::config::KeymapConfig { bindings },
        config.warnings(),
    )
    .expect("valid");
    let mut app = App::new(&Startup {
        bindings,
        ..Startup::from(config)
    });
    app.view_stale = true;

    let line = status_line(&mut app);
    assert!(
        line.contains(" changed on disk · F5 "),
        "the badge must name the key the config file bound: {line}"
    );
}

/// A toggle walks the listing but stats none of it (#156): a record is
/// trusted as it stands, even when its stamp no longer matches the disk.
/// Finding that is the poll's job, off the UI thread.
#[test]
fn a_toggle_trusts_the_cached_stamp() {
    let mut app = app_over_logs("scan_trust");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.add_filter("beta").expect("valid pattern");
    app.refresh_scan(false);
    for (_, path) in app.explorer.files() {
        app.scan_cache.records.insert(
            path,
            scan::Record {
                stamp: Some((std::time::SystemTime::UNIX_EPOCH, 0)),
                progress: scan::Progress {
                    seen: vec![0],
                    scanned_to: 1,
                    eof: true,
                },
            },
        );
    }
    let before = scanner.requests().len();

    app.filters.set_enabled(1, false);
    app.refresh_scan(false);

    assert_eq!(scanner.requests().len(), before, "a toggle re-scanned");
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::No
    );
}

/// A resumed scan carries the stamp its progress was read under, so the
/// worker can tell the file changed.
#[test]
fn a_resume_carries_the_held_stamp() {
    let mut app = app_over_logs("scan_carry");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    let (_, path) = app.explorer.files()[0].clone();
    let held = Some((std::time::SystemTime::UNIX_EPOCH, 7));
    app.scan_cache.records.insert(
        path.clone(),
        scan::Record {
            stamp: held,
            progress: scan::Progress {
                seen: vec![0],
                scanned_to: 1,
                eof: false,
            },
        },
    );

    app.refresh_scan(true);

    let last = scanner.requests().last().expect("a request").clone();
    let file = last.files.iter().find(|f| f.path == path).expect("resumed");
    assert_eq!(file.stamp, held);
}

/// A restarted file reads less than the record it replaces, and is still
/// the truth: its stamp is new.
#[test]
fn a_result_with_a_new_stamp_replaces_one_that_read_further() {
    let mut app = app_over_logs("drain_restamp");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    let mut old = scanned(&app, 0, vec![0], true);
    old.stamp = Some((std::time::SystemTime::UNIX_EPOCH, 9));
    old.progress.scanned_to = 50;
    tx.send(old).expect("send");
    app.drain_scan_results();

    tx.send(scanned(&app, 0, vec![0b1], false)).expect("send");
    app.drain_scan_results();

    assert!(matches!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::Yes(_)
    ));
}

/// The check ran on a snapshot. A record that already carries the new
/// stamp by the time its answer lands is kept, not thrown away.
#[test]
fn a_moved_file_already_rescanned_is_kept() {
    let mut app = app_over_logs("poll_refreshed");
    let (scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0], true)).expect("send");
    app.drain_scan_results();
    let (_, path) = app.explorer.files()[0].clone();
    let before = scanner.requests().len();

    let changed = app.apply_moved(vec![scan::Moved {
        stamp: scan::stamp(&path).ok(),
        path,
    }]);

    assert!(!changed);
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::No
    );
    assert_eq!(scanner.requests().len(), before);
}

/// The whole poll, thread included: one tick starts the check, a later
/// tick applies it.
#[test]
fn polling_finds_a_changed_file_off_the_ui_thread() {
    let mut app = app_over_logs("poll_thread");
    let (_scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0], true)).expect("send");
    app.drain_scan_results();
    let (_, path) = app.explorer.files()[0].clone();
    fs::write(&path, "now alpha is here\nand more\n").expect("rewrite");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut changed = app.poll_stamps();
    while !changed && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        changed = app.poll_stamps();
    }

    assert!(changed, "the poll never reported the change");
    assert_eq!(
        app.explorer.entries()[app.explorer.files()[0].0].matched,
        Match::Unknown
    );
    assert!(
        app.stamp_check.is_none(),
        "the finished check is still held"
    );
}

/// `r` promises a re-stat. With `refresh_scan` trusting its records, the
/// key has to do it itself or a change waits for the poll.
#[test]
fn r_rescans_a_file_that_changed_on_disk() {
    let mut app = app_over_logs("r_restat");
    let (scanner, tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    tx.send(scanned(&app, 0, vec![0], true)).expect("send");
    app.drain_scan_results();
    let (_, path) = app.explorer.files()[0].clone();
    fs::write(&path, "now alpha is here\nand more\n").expect("rewrite");

    key(&mut app, KeyCode::Char('r'));

    let last = scanner.requests().last().expect("a rescan").clone();
    assert!(
        last.files.iter().any(|file| file.path == path),
        "the changed file was not rescanned: {last:?}"
    );
}

#[test]
fn polling_is_rate_limited() {
    let mut app = app_over_logs("poll_rate");
    let _seam = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);

    app.poll_stamps();
    let first = app.last_poll.expect("stamped");
    app.poll_stamps();

    assert_eq!(
        app.last_poll,
        Some(first),
        "polled again inside the interval"
    );
}

#[test]
fn r_reloads_the_file_and_keeps_the_cursor_on_its_line() {
    let mut app = app_over_file("r_reload", "one\ntwo\nthree\nfour\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(cursor_source(&app), 2);
    fs::write(app.view.filename(), "one\ntwo\nthree\nfour\nfive\n").expect("rewrite");

    key(&mut app, KeyCode::Char('r'));

    assert_eq!(app.document.lines().len(), 5, "not reloaded");
    assert_eq!(cursor_source(&app), 2, "the reader lost their place");
}

#[test]
fn r_clears_the_badge_and_forces_a_rescan() {
    let mut app = app_over_logs("r_rescan");
    let (scanner, _tx) = record_scans(&mut app);
    app.add_filter("alpha").expect("valid pattern");
    app.refresh_scan(false);
    let before = scanner.requests().len();
    app.view_stale = true;

    key(&mut app, KeyCode::Char('r'));

    assert!(!app.view_stale);
    assert_eq!(scanner.requests().len(), before + 1);
}

#[test]
fn r_on_a_truncated_file_clamps_rather_than_losing_the_cursor() {
    let mut app = app_over_file("r_shrunk", "one\ntwo\nthree\nfour\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('G'));
    fs::write(app.view.filename(), "one\n").expect("truncate");

    key(&mut app, KeyCode::Char('r'));

    assert_eq!(app.document.lines().len(), 1);
    assert_eq!(cursor_source(&app), 0);
}

#[test]
fn r_re_lists_the_directory_so_a_new_file_appears() {
    let mut app = app_over_logs("r_relist");
    let dir = app.explorer.dir().to_path_buf();
    assert!(
        !explorer_rows(&mut app).iter().any(|r| r.contains("c.log")),
        "c.log should not be listed before it exists"
    );
    fs::write(dir.join("c.log"), "x").expect("write new file");

    key(&mut app, KeyCode::Char('r'));

    assert!(
        explorer_rows(&mut app).iter().any(|r| r.contains("c.log")),
        "c.log should appear once r re-lists the directory"
    );
    assert_eq!(
        app.explorer.selected_path(),
        Some(dir.join("a.log")),
        "selection should stay on the file it was on"
    );
}
