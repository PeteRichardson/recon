use super::*;

/// `app_over`, with a `[keymap]` that costs `explorer.down` its `j` — the
/// smallest config that produces a warning.
fn app_with_warnings(name: &str, files: &[&str]) -> App<'static> {
    app_over_keymap(name, files, false)
}

/// `app_with_warnings`, with the warnings switched off.
fn app_with_warnings_silenced(name: &str, files: &[&str]) -> App<'static> {
    app_over_keymap(name, files, true)
}

/// A sibling of `app_over` and `app_over_files`, not a refactor of either.
/// `app_over` has 58 callers, and `app_over_files` beside it already
/// repeats this shape rather than sharing a body — so repeating it once
/// more is the convention here, and rewriting `app_over` would be 58 call
/// sites of churn for nothing.
fn app_over_keymap(name: &str, files: &[&str], no_warnings: bool) -> App<'static> {
    let dir = fixture_dir(name);
    for file in files {
        fs::write(dir.join(file), "x").expect("write fixture");
    }
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert("global.quit".to_string(), vec!["j".to_string()]);
    let config = Config {
        path: dir.join("placeholder").display().to_string(),
        no_warnings,
        ..Config::default()
    };
    let (map, warnings) = crate::keymap::config::build(
        &crate::keymap::config::KeymapConfig { bindings },
        config.warnings(),
    )
    .expect("valid");
    App::new(&Startup {
        bindings: map,
        keymap_warnings: warnings,
        ..Startup::from(config)
    })
}

/// A config that costs a key must say so where the user is looking, not
/// on a stderr line the alternate screen covers a moment later.
#[test]
fn a_keymap_warning_opens_a_panel() {
    let mut app = app_with_warnings("warning_panel_opens", &["a.rs"]);

    let screen = screen(&mut app);
    assert!(
        screen.contains("Keymap warnings"),
        "the panel did not draw:\n{screen}"
    );
    assert!(screen.contains("explorer.down"), "{screen}");
}

#[test]
fn any_key_dismisses_the_panel() {
    let mut app = app_with_warnings("warning_panel_any_key", &["a.rs"]);

    key(&mut app, KeyCode::Char('k'));

    let screen = screen(&mut app);
    assert!(
        !screen.contains("Keymap warnings"),
        "the panel outlived its key:\n{screen}"
    );
}

/// The dismissing key must not also do its usual job, or the panel costs
/// the user a keystroke they did not mean to spend. `q` is the sharpest
/// case.
#[test]
fn the_dismissing_key_does_not_also_act() {
    let mut app = app_with_warnings("warning_panel_not_acted", &["a.rs"]);

    key(&mut app, KeyCode::Char('q'));

    assert_eq!(
        app.state,
        AppState::Running,
        "'q' dismissed the panel and also quit"
    );
}

/// Mouse capture is on, so a mouse crossing the terminal must not wipe a
/// panel mid-read. The help overlay makes the same distinction.
#[test]
fn a_mouse_event_does_not_dismiss_the_panel() {
    let mut app = app_with_warnings("warning_panel_mouse", &["a.rs"]);

    app.handle_event(event::Event::Mouse(event::MouseEvent {
        kind: event::MouseEventKind::Moved,
        column: 1,
        row: 1,
        modifiers: KeyModifiers::empty(),
    }));

    let screen = screen(&mut app);
    assert!(screen.contains("Keymap warnings"), "{screen}");
}

#[test]
fn a_clean_config_opens_no_panel() {
    let mut app = app_over("no_warnings_clean", &["a.rs"]);

    let screen = screen(&mut app);
    assert!(!screen.contains("Keymap warnings"), "{screen}");
}

#[test]
fn no_warnings_opens_no_panel() {
    let mut app = app_with_warnings_silenced("warning_panel_silenced", &["a.rs"]);

    let screen = screen(&mut app);
    assert!(!screen.contains("Keymap warnings"), "{screen}");
}

/// Two or more warnings, each long enough to wrap onto more than one
/// row, sized generously enough that nothing needs to be cut.
///
/// The broken comparison (`lines.len() + wrapped > budget`) is skipped
/// entirely for a single warning — the guard is `!lines.is_empty()`, so
/// the first entry is always admitted unconditionally — which is why a
/// fixture of one warning could never exercise it. `app_with_warnings`'s
/// three ~150-byte warnings both admit a second comparison and wrap to
/// more than one row each, so the two units (rows consumed vs. warnings
/// admitted) can actually disagree. Under the broken arithmetic the box
/// was sized from the *count* (3, plus a margin) regardless of how much
/// room the terminal offered, so the third warning — `filters.down` —
/// never appeared even though the panel had ample space and the title
/// claimed nothing was cut.
#[test]
fn two_wrapping_warnings_both_render_in_full() {
    let mut app = app_with_warnings("warning_panel_roomy", &["a.rs"]);
    let area = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 20,
    };
    let mut buf = Buffer::empty(area);
    app.render(area, &mut buf);
    let screen = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(screen.contains("explorer.down"), "{screen}");
    assert!(
        screen.contains("filters.down"),
        "the third warning was cut though the box had room for it:\n{screen}"
    );
    assert!(
        !screen.contains("more — run recon --print-keymap to read them all"),
        "the box reported a cut though everything fit:\n{screen}"
    );
}

/// A regression guard for a defect where the row budget was checked
/// against the *count of warnings admitted* rather than the *rows they
/// render to*. `app_with_warnings` produces three warnings (152, 156 and
/// 168 bytes); on a box this cramped they need more rows than fit, so an
/// honest panel must say it cut one. The broken arithmetic admitted all
/// three and reported nothing cut, silently truncating the last one
/// under `Wrap` instead.
#[test]
fn a_cramped_panel_reports_what_it_could_not_show() {
    let mut app = app_with_warnings("warning_panel_cramped", &["a.rs"]);
    let area = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 12,
    };
    let mut buf = Buffer::empty(area);
    app.render(area, &mut buf);
    let screen = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        screen.contains("more — run recon --print-keymap to read them all"),
        "a box too short for all three warnings claimed to have cut none:\n{screen}"
    );
}

/// The screen as rows of text, for the panel tests below.
fn warning_screen(app: &mut App<'_>, width: u16, height: u16) -> Vec<String> {
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let mut buf = Buffer::empty(area);
    app.render(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

/// The panel's text with the borders taken off and the rows joined by a
/// space, so a sentence that wraps reads as one.
fn panel_text(screen: &[String]) -> String {
    screen
        .iter()
        .filter_map(|row| {
            let start = row.find('│')?;
            let end = row.rfind('│')?;
            (end > start).then(|| row[start + '│'.len_utf8()..end].trim().to_string())
        })
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// #252: in a narrow terminal the title cut the command off and kept the
/// count, so the user saw that warnings were missing and not how to read
/// them. The command must be there in full at any width the panel draws at.
#[test]
fn a_narrow_panel_still_names_the_command_in_full() {
    let mut app = app_with_warnings("warning_panel_narrow", &["a.rs"]);
    let screen = warning_screen(&mut app, 40, 12);
    let text = panel_text(&screen);

    assert!(
        text.contains("run recon --print-keymap to read them"),
        "the command was cut:\n{}",
        screen.join("\n")
    );
}

/// #253: the rows a warning wraps to are indented under its bullet, so
/// where one warning ends and the next starts is visible.
#[test]
fn a_wrapped_warning_is_indented_under_its_bullet() {
    let mut app = app_with_warnings("warning_panel_indent", &["a.rs"]);
    let screen = warning_screen(&mut app, 40, 20);
    let rows: Vec<&str> = screen
        .iter()
        .filter_map(|row| {
            let start = row.find('│')? + '│'.len_utf8();
            let end = row.rfind('│')?;
            (end > start).then(|| &row[start..end])
        })
        .collect();
    let first = rows
        .iter()
        .position(|row| row.trim_start().starts_with('•'))
        .unwrap_or_else(|| panic!("no warning drawn:\n{}", screen.join("\n")));
    let bullet_column = rows[first].find('•').expect("bullet");

    let next = rows[first + 1];
    assert!(
        next[..bullet_column + 2].trim().is_empty() && !next.trim().is_empty(),
        "the second row of a wrapped warning is not under its text:\n{}",
        screen.join("\n")
    );
}

/// #254: at five rows for the panel — six for the terminal, less the
/// status row — the budget was one row, the first warning needs more,
/// and the panel admitted it anyway — so the text clipped and the panel
/// said it had cut nothing. It must now say what it did not show.
#[test]
fn a_very_short_panel_says_it_showed_no_warning() {
    let mut app = app_with_warnings("warning_panel_short", &["a.rs"]);
    let screen = warning_screen(&mut app, 80, 6);
    let text = panel_text(&screen);

    assert!(
        text.contains("3 warnings — run recon --print-keymap to read them"),
        "the panel hid every warning and did not say so:\n{}",
        screen.join("\n")
    );
    assert!(
        !text.contains('•'),
        "a warning was drawn in a box with no room for it:\n{}",
        screen.join("\n")
    );
}
