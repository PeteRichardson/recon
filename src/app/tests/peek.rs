use super::*;

// ---- `<space>`: peek at the plain file (#48) -------------------------

/// Every enabled flag — the state a peek has to put back untouched.
fn enabled_flags(app: &App) -> Vec<bool> {
    app.filters.filters().iter().map(|f| f.enabled).collect()
}

/// A two-line file with one enabled filter on `beta`, hiding unmatched
/// lines — the state the issue describes pressing `<space>` from.
fn app_hiding(name: &str) -> App<'static> {
    let mut app = app_over_file(name, "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('H'));
    app
}

/// The headline acceptance criterion: one keypress replaces the four-key
/// cycle in the issue — mode flips and every filter comes off, leaving the
/// plain file.
#[test]
fn space_shows_the_plain_unfiltered_file() {
    let mut app = app_hiding("space_plain");
    assert_eq!(
        view_lines(&app),
        vec!["beta".to_string()],
        "sanity: not hiding to begin with"
    );

    key(&mut app, KeyCode::Char(' '));

    assert_eq!(app.document.mode(), Mode::Dimmed, "still hiding");
    assert_eq!(
        view_lines(&app),
        vec!["alpha".to_string(), "beta".to_string()],
        "the whole file is not on screen"
    );
    assert!(
        view_line_styles(&app).iter().all(Option::is_none),
        "the peek left filter colouring behind"
    );
}

/// The property the issue states outright: *"Hitting `<space>` twice in a
/// row gets you back to exactly where you were."*
#[test]
fn space_twice_restores_everything_exactly() {
    let mut app = app_hiding("space_round_trip");
    let mode = app.document.mode();
    let lines = view_lines(&app);
    let styles = view_line_styles(&app);
    let flags = enabled_flags(&app);
    let cursor = app.cursor_source();

    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char(' '));

    assert_eq!(app.document.mode(), mode, "the mode did not come back");
    assert_eq!(view_lines(&app), lines, "the visible set did not come back");
    assert_eq!(view_line_styles(&app), styles, "the colouring did not");
    assert_eq!(enabled_flags(&app), flags, "the filter flags did not");
    assert_eq!(app.cursor_source(), cursor, "the cursor moved");
}

/// A peek from `Mode::Dimmed` must not empty the pane. It doesn't, and the
/// reason is `Document::recompute_visible`'s #36 guard rather than anything
/// the peek does: with nothing including, `FilteredOnly` shows the whole
/// file. This test predates #65 and passed under the old forced-`Dimmed`
/// peek too — it is kept because the *guard* is what it is really pinning,
/// and that guard is now load-bearing for `<space>`.
#[test]
fn space_from_dimmed_mode_does_not_empty_the_pane() {
    let mut app = app_over_file("space_from_dimmed", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: dimming");

    key(&mut app, KeyCode::Char(' '));

    assert_eq!(
        view_lines(&app),
        vec!["alpha".to_string(), "beta".to_string()],
        "the peek emptied the pane"
    );
}

/// #65, the headline: #48 asked for `<space>` to **toggle** dimmed/hide, and
/// the peek forced `Mode::Dimmed` instead. From the dimmed view that made
/// `<space>` a pure filter switch — indistinguishable from `!`.
///
/// It is safe to flip because hide mode does not mean "hide every unmatched
/// line"; it means "*if* something is including, hide unmatched lines".
/// With the filters off there is nothing to hide against, so
/// `recompute_visible`'s #36 guard shows the whole file either way.
#[test]
fn space_from_dimmed_mode_arms_hiding() {
    let mut app = app_over_file("space_arms_hiding", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: dimming");

    key(&mut app, KeyCode::Char(' '));

    assert_eq!(
        app.document.mode(),
        Mode::FilteredOnly,
        "`<space>` did not toggle the mode"
    );
}

/// The other half of the toggle, and the half that already worked. Pinned
/// alongside its opposite so a future change cannot fix one direction by
/// breaking the other.
#[test]
fn space_from_hide_mode_disarms_hiding() {
    let mut app = app_hiding("space_disarms_hiding");
    assert_eq!(
        app.document.mode(),
        Mode::FilteredOnly,
        "sanity: hiding to begin with"
    );

    key(&mut app, KeyCode::Char(' '));

    assert_eq!(app.document.mode(), Mode::Dimmed, "hiding did not come off");
}

/// The mode a peek leaves armed is real, so the badge that reports it must
/// appear — even though the plain file is on screen and nothing is being
/// hidden. The badge tracks what is *armed*, which is what makes the flip
/// honest rather than a lie on the status row (see `HIDE_BADGE_TEXT`).
#[test]
fn the_hide_badge_reports_a_peek_that_armed_hiding() {
    let mut app = app_over_file("space_badge", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert!(
        !status_line(&mut app).contains(HIDE_BADGE_TEXT.trim()),
        "sanity: no badge while merely dimming"
    );

    key(&mut app, KeyCode::Char(' '));

    let row = status_line(&mut app);
    assert!(
        row.contains(HIDE_BADGE_TEXT.trim()),
        "the peek armed hiding but the badge does not say so: {row}"
    );
}

/// Global, like `!` and `o`: the file the user means is whatever the view
/// is showing, so the peek must not require focusing a particular pane.
#[test]
fn space_peeks_from_the_explorer_pane() {
    let mut app = app_hiding("space_from_explorer");
    key(&mut app, KeyCode::Char('e'));
    assert!(
        app.focus == Focus::Explorer,
        "sanity: the explorer should have focus"
    );

    key(&mut app, KeyCode::Char(' '));

    assert_eq!(app.document.mode(), Mode::Dimmed, "`space` did nothing");
}

/// The peek and `!` both turn every filter off, and they must not share one
/// slot: a peek taken while `!` is holding a capture would overwrite it,
/// and `!` would then restore all-disabled forever.
#[test]
fn the_peek_leaves_the_bang_capture_alone() {
    let mut app = app_hiding("space_vs_bang");
    let flags = enabled_flags(&app);

    key(&mut app, KeyCode::Char('!'));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Char('!'));

    assert_eq!(
        enabled_flags(&app),
        flags,
        "`!` could not restore what it captured after a peek"
    );
}

/// An open prompt outranks every binding — `space` is an ordinary
/// character to type into a pattern.
#[test]
fn space_typed_into_a_prompt_is_text_not_a_peek() {
    let mut app = app_hiding("space_prompt");
    // `/` is deliberately inert while the filter pane has focus — that pane
    // has nothing to search over — and `app_hiding` leaves it focused, so
    // the prompt has to be opened from the file view.
    key(&mut app, KeyCode::Char('t'));
    let mode = app.document.mode();

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char(' '));

    assert_eq!(app.document.mode(), mode, "the peek fired from a prompt");
    // The pattern itself, not the rendered row: the row carries the HIDE
    // badge here, and a trailing space does not survive rendering.
    assert_eq!(
        app.prompt.as_ref().map(|prompt| prompt.pattern.as_str()),
        Some(" "),
        "the space did not reach the pattern"
    );
}

// ---- `Enter`: the filter pane's toggle, and its bounce guard (#48) ----

/// The reason `Enter` was left unbound here until now: it is also the key
/// that *commits* the prompt `i`, `x` and `c` open. A doubled press would
/// otherwise commit the pattern and then silently switch a filter off.
#[test]
fn the_enter_that_commits_a_prompt_does_not_also_toggle() {
    let mut app = app_hiding("enter_bounce");
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('c'));
    key(&mut app, KeyCode::Enter);
    let flags = enabled_flags(&app);

    key(&mut app, KeyCode::Enter);

    assert_eq!(enabled_flags(&app), flags, "the bounce toggled a filter");
}

/// The guard swallows exactly one `Enter`, and only the one immediately
/// after the commit — any other key in between means the user is still
/// working the pane and meant it.
#[test]
fn an_enter_after_an_intervening_key_still_toggles() {
    let mut app = app_hiding("enter_after_key");
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('c'));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('k'));
    let flags = enabled_flags(&app);

    key(&mut app, KeyCode::Enter);

    assert_ne!(
        enabled_flags(&app),
        flags,
        "the guard outlived its keypress"
    );
}

/// Only one. A second doubled press is a deliberate toggle, not a bounce.
#[test]
fn the_guard_swallows_only_a_single_enter() {
    let mut app = app_hiding("enter_one_guard");
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('c'));
    key(&mut app, KeyCode::Enter);
    let flags = enabled_flags(&app);

    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);

    assert_ne!(
        enabled_flags(&app),
        flags,
        "the second Enter was swallowed too"
    );
}

#[test]
fn question_mark_opens_the_help_overlay() {
    let mut app = app_over("help_open", &["a.rs"]);

    key(&mut app, KeyCode::Char('?'));

    let screen = screen(&mut app);
    assert!(
        screen.contains("Quit"),
        "the help overlay did not draw:\n{screen}"
    );
}

/// A user presses `?` as Shift-`/` on the keyboard, but crossterm only
/// attaches `SHIFT` when the character itself is uppercase (`keymap::Key`'s
/// doc comment records the rule once) — and `'?'` is not uppercase, so a
/// real terminal reports it with no modifiers at all. An `is_empty()`
/// guard would have been satisfied here; the risk such a guard actually
/// poses is the one `O` and `n`/`N` document, where the letter really does
/// carry `SHIFT`.
///
/// This test does not reproduce what a terminal sends: it hand-builds an
/// event carrying `SHIFT` on `'?'`, which `normalise` drops. It pins that
/// dropping behaviour, not real keypress handling.
#[test]
fn shift_does_not_stop_the_help_overlay_opening() {
    let mut app = app_over("help_shift", &["a.rs"]);

    app.handle_event(event::Event::Key(event::KeyEvent::new(
        KeyCode::Char('?'),
        KeyModifiers::SHIFT,
    )));

    assert!(app.help, "Shift-? did not reach the help binding");
}

#[test]
fn any_key_closes_the_help_overlay() {
    let mut app = app_over("help_close", &["a.rs"]);
    key(&mut app, KeyCode::Char('?'));

    key(&mut app, KeyCode::Char('j'));

    let screen = screen(&mut app);
    assert!(
        !screen.contains("Quit"),
        "the help overlay outlived its dismissing key:\n{screen}"
    );
}

/// The dismissing key is consumed. Otherwise the key that closes help also
/// acts, and the most likely one to be pressed is `q`.
#[test]
fn the_key_that_closes_help_does_nothing_else() {
    let mut app = app_over("help_swallow", &["a.rs"]);
    key(&mut app, KeyCode::Char('?'));

    key(&mut app, KeyCode::Char('q'));

    assert!(app.is_running(), "the key that closed help also quit");
}

/// A mouse moving across the terminal must not wipe the overlay away — the
/// same reasoning `handle_event` already applies to the status message.
#[test]
fn a_mouse_event_does_not_close_the_help_overlay() {
    let mut app = app_over("help_mouse", &["a.rs"]);
    key(&mut app, KeyCode::Char('?'));

    mouse(&mut app, MouseEventKind::Moved, 10);

    assert!(app.help, "a mouse event closed the help overlay");
}

/// An open prompt consumes every key, `?` included — it is a perfectly
/// ordinary character in a regular expression.
#[test]
fn question_mark_is_typed_into_an_open_prompt() {
    let mut app = app_over("help_prompt", &["a.rs"]);
    key(&mut app, KeyCode::Char('/'));

    typed(&mut app, "ab?");

    assert!(!app.help, "`?` opened help from inside a prompt");
    assert_eq!(prompt_line(&mut app), "/ab?");
}

/// The overlay covers the panes, not the status row: the row carries the
/// HIDE badge and the current directory, and both stay true while help is
/// up.
#[test]
fn the_help_overlay_leaves_the_status_row_alone() {
    let mut app = app_over("help_status", &["a.rs"]);
    let before = prompt_line(&mut app);

    key(&mut app, KeyCode::Char('?'));

    assert_eq!(prompt_line(&mut app), before);
}

/// `?` spends the post-commit `Enter` guard (#48) exactly as any other key
/// does — the guard's whole rule is "cleared by any key that is not
/// `Enter`". Otherwise reading the keymap between a commit and a toggle
/// leaves a guard armed with nothing left to guard against, and the next
/// deliberate `Enter` silently does nothing.
#[test]
fn reading_the_keymap_spends_the_post_commit_enter_guard() {
    let mut app = app_hiding("help_enter_guard");
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('c'));
    key(&mut app, KeyCode::Enter);
    let flags = enabled_flags(&app);

    key(&mut app, KeyCode::Char('?'));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);

    assert_ne!(
        enabled_flags(&app),
        flags,
        "the guard outlived the key that should have spent it"
    );
}
