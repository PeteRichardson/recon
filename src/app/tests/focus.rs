use super::*;

// ---- #263: opening a file sends the focus after it -------------------

/// `l`, `Right` and `Enter` on a *file* land the cursor in the view.
///
/// All three in one loop because they share `explorer.open`: a change that
/// reached only the arm one of them took would still pass a test that
/// pressed only that one.
///
/// Without this, `l` on a file changed almost nothing on screen — the
/// explorer already previews every row the cursor passes over — and
/// reading the file needed a second, differently-shaped keystroke.
#[test]
fn opening_a_file_moves_the_focus_to_the_file_view() {
    // One fixture, three apps: the registry refuses a name claimed twice,
    // and each key needs an explorer that has not already moved.
    let dir = fixture_dir("explorer_open_focus");
    fs::write(dir.join("a.log"), "alpha\n").expect("write fixture");

    for code in [KeyCode::Char('l'), KeyCode::Right, KeyCode::Enter] {
        let mut app = App::new(&Config {
            path: dir.join("placeholder").display().to_string(),
            ..Config::default()
        });
        draw(&mut app);
        assert_eq!(app.focus, Focus::Explorer, "{code:?}");

        // Rows: `..`, `a.log`. The cursor opens on the first real entry,
        // not on `..`, so `a.log` is already under it.
        assert_eq!(app.explorer.selected(), Some(1), "{code:?}");
        key(&mut app, code);

        assert_eq!(app.focus, Focus::View, "{code:?}");
        assert_eq!(shown(&app), "a.log", "{code:?}");
    }
}

/// A directory is the other half of "one level deeper", and the work is
/// still in the explorer once you are inside it — so the focus stays.
#[test]
fn opening_a_directory_keeps_the_focus_in_the_explorer() {
    let (mut app, dir) = app_over_nested("explorer_open_focus_dir");
    draw(&mut app);

    // Rows: `..`, `sub/`, `z.log`, and the cursor opens on `sub/`.
    assert_eq!(app.explorer.selected(), Some(1));
    key(&mut app, KeyCode::Char('l'));

    assert!(
        app.explorer.dir().ends_with(dir.join("sub")),
        "did not descend, got {}",
        app.explorer.dir().display()
    );
    assert_eq!(app.focus, Focus::Explorer);
}

/// `..` climbs out, which is a directory move like any other.
#[test]
fn opening_the_parent_entry_keeps_the_focus_in_the_explorer() {
    let (mut app, _dir) = app_over_nested("explorer_open_focus_parent");
    draw(&mut app);
    let before = app.explorer.dir().to_path_buf();

    // The cursor opens on `sub/`; `Up` backs it onto `..`.
    key(&mut app, KeyCode::Up);
    assert_eq!(app.explorer.selected(), Some(0));
    key(&mut app, KeyCode::Char('l'));

    assert_eq!(
        app.explorer.dir(),
        before.parent().expect("the fixture has a parent"),
        "did not climb out"
    );
    assert_eq!(app.focus, Focus::Explorer);
}

/// The focus goes through `reveal_and_focus`, not a bare assignment, so
/// `z` on the explorer cannot leave the cursor on a pane that is not
/// drawn. Same reason the focus *keys* do not just set the field.
#[test]
fn opening_a_file_reveals_a_hidden_file_view() {
    let mut app = app_over_files("explorer_open_focus_zoom", &[("a.log", "alpha\n")]);
    draw(&mut app);
    key(&mut app, KeyCode::Char('z'));
    assert!(
        !app.panes.is_shown(Focus::View),
        "the explorer should be zoomed"
    );

    key(&mut app, KeyCode::Char('l'));

    assert!(
        app.panes.is_shown(Focus::View),
        "the file view is still hidden"
    );
    assert_eq!(app.focus, Focus::View);
}

/// The two directions are deliberately not symmetrical (#263). The view
/// needs `h` for horizontal scroll of long lines, and a key whose job
/// changes at column 0 is both hard to learn and easy to trip. `Tab`,
/// `Shift-Tab` and `e` are the documented ways back.
#[test]
fn h_in_the_file_view_does_not_send_the_focus_back() {
    let mut app = app_over_files("explorer_open_focus_h", &[("a.log", "alpha\n")]);
    draw(&mut app);
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.focus, Focus::View);

    key(&mut app, KeyCode::Char('h'));

    assert_eq!(app.focus, Focus::View);
}

/// `f` reaches the filter pane rather than opening a filter prompt.
///
/// Creating a filter moves inside the pane (`i` / `x`), which costs a
/// keystroke from elsewhere and buys one focus key per pane.
#[test]
fn f_focuses_the_filter_pane() {
    let mut app = app_over_file("focus_f", "alpha\n");
    assert_ne!(app.focus, Focus::Filters);

    key(&mut app, KeyCode::Char('f'));

    assert_eq!(app.focus, Focus::Filters);
    assert!(
        app.prompt.is_none(),
        "f opened a prompt instead of moving focus"
    );
}

/// `x` is the exclude half of the pair. `e` would read better and cannot
/// be used — the global match runs first, so a bare `e` never reaches the
/// filter pane at all.
#[test]
fn x_opens_an_excluding_filter_prompt_in_the_filter_pane() {
    let mut app = app_over("exclude_prompt", &["a.rs"]);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "noise");

    assert_eq!(prompt_line(&mut app), "exclude: noise");
}

/// `i` and `x` belong to the filter pane, not to the app. Bound globally
/// they would swallow a keystroke from every other pane — which is exactly
/// what `f` and `F` used to do, and the reason they moved.
#[test]
fn i_and_x_do_nothing_outside_the_filter_pane() {
    for (code, name) in [
        (KeyCode::Char('i'), "filter_keys_scoped_i"),
        (KeyCode::Char('x'), "filter_keys_scoped_x"),
    ] {
        // A fixture directory per iteration: `claim_fixture_dir` panics on
        // a reused name, deliberately, so tests cannot race over one.
        let mut app = app_over_file(name, "alpha\n");

        // The explorer has focus at startup.
        key(&mut app, code);
        assert!(
            app.prompt.is_none(),
            "{code:?} opened a prompt from the explorer"
        );

        key(&mut app, KeyCode::Char('t'));
        key(&mut app, code);
        assert!(
            app.prompt.is_none(),
            "{code:?} opened a prompt from the file view"
        );
    }
}

/// `F` created an excluding filter and is retired; `x` in the pane does it
/// now. Pinned so the old binding cannot quietly come back alongside the
/// new one and leave two ways to do the same thing.
#[test]
fn capital_f_no_longer_opens_a_prompt() {
    let mut app = app_over_file("capital_f_retired", "alpha\n");

    key(&mut app, KeyCode::Char('F'));

    assert!(app.prompt.is_none(), "F still opens a prompt");
}

/// The explorer's own top-left corner, as symbol and style.
///
/// The corner is the probe because it is drawn by the border and by
/// nothing else — a row's highlight, a title, and the pane's contents all
/// stay out of it.
fn explorer_corner(app: &mut App) -> (String, Style) {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    (buf[(0, 0)].symbol().to_string(), buf[(0, 0)].style())
}

/// Focus has to be visible on the pane, not just on the row inside it.
///
/// Colour *and* weight, deliberately: this is the argument #19 makes about
/// the selection marker. A single channel fails on a theme with weak
/// contrast and for a colour-blind reader, and the cue this replaces —
/// a green foreground on one already-reversed row — was exactly that.
#[test]
fn the_focused_pane_border_differs_in_colour_and_weight() {
    let mut app = app_over_file("focus_border", "alpha\n");
    // The explorer holds focus at startup.
    let (focused_symbol, focused_style) = explorer_corner(&mut app);

    key(&mut app, KeyCode::Char('t'));
    let (unfocused_symbol, unfocused_style) = explorer_corner(&mut app);

    assert_ne!(
        focused_style.fg, unfocused_style.fg,
        "the border colour is the same focused and unfocused"
    );
    assert_ne!(
        focused_symbol, unfocused_symbol,
        "the border weight is the same focused and unfocused, so the cue \
         is colour alone"
    );
}

/// `z` maximises whatever has focus — including the explorer, for long
/// filenames.
#[test]
fn z_zooms_the_explorer_when_it_has_focus() {
    // A distinctive marker, not "alpha": the explorer titles its block
    // with the absolutised checkout path, which could itself contain
    // "alpha" on some checkout — the negative assertion below would then
    // pass or fail depending on where the repo happens to be checked
    // out, rather than on what the test claims to check.
    let mut app = app_over_file("zoom_z_explorer", "ZOOMMARKER\n");

    key(&mut app, KeyCode::Char('z'));

    let after = rendered(&mut app);
    // See `b_hides_the_left_column` for why `../` is the probe.
    assert!(after.contains("../"), "the explorer is not on screen");
    assert!(
        !after.contains("ZOOMMARKER"),
        "the file view is still showing"
    );
}

/// With focus in the file view, `z` and `b` do the same thing.
#[test]
fn z_in_the_file_view_matches_b() {
    // Both apps must point at the exact same file: the file view's
    // border includes its full path as a title, so two different
    // fixture directories would make `rendered` disagree on the title
    // text alone, regardless of whether the zoom layouts truly match.
    let file = fixture_path("zoom_view_parity", "alpha\n");
    let config = Config {
        path: file.display().to_string(),
        ..Config::default()
    };

    let mut with_z = App::new(&config);
    focus_file_view(&mut with_z);
    key(&mut with_z, KeyCode::Char('z'));

    let mut with_b = App::new(&config);
    key(&mut with_b, KeyCode::Char('b'));

    assert_eq!(rendered(&mut with_z), rendered(&mut with_b));
    // `rendered` only sees symbols, so two layouts that differ solely in
    // which pane is focused — a thicker border, a title glyph — could
    // still render identically today. These pin the claim the symbols
    // alone cannot: `z` and `b` leave the app in the exact same state,
    // not just looking the same.
    assert_eq!(with_z.focus, with_b.focus);
    assert_eq!(with_z.panes, with_b.panes);
}

#[test]
fn z_toggles_back() {
    let mut app = app_over_file("zoom_z_back", "alpha\n");
    let before = rendered(&mut app);

    key(&mut app, KeyCode::Char('z'));
    key(&mut app, KeyCode::Char('z'));

    assert_eq!(rendered(&mut app), before);
}

/// A drag started on the divider must not survive into a zoom: there is
/// no divider to drag while zoomed, but the `Drag` arm in
/// `handle_divider` only checks `self.dragging`, so without
/// `toggle_zoom` cancelling it, moving the mouse mid-drag would silently
/// re-pin `explorer_width` while nothing is drawn to explain why.
#[test]
fn zooming_mid_drag_cancels_the_drag() {
    let mut app = app_over_file("zoom_drag_cancel", "alpha\n");
    draw(&mut app);
    let divider = app.divider;
    let before = app.explorer_width(AREA);

    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);
    assert_eq!(
        app.dragging,
        Some(Divider::Explorer),
        "sanity: the divider click started a drag"
    );

    key(&mut app, KeyCode::Char('b'));
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 60);

    assert_eq!(app.dragging, None, "the drag survived into the zoom");
    assert_eq!(
        app.explorer_width,
        PaneWidth::Auto,
        "explorer_width changed from a drag that continued while zoomed"
    );
    key(&mut app, KeyCode::Char('b'));
    assert_eq!(app.explorer_width(AREA), before);
}

/// Tab skips hidden panes (#300), so with one pane shown it has nowhere
/// to go: focus never moves onto a pane that is not on the screen.
#[test]
fn tab_while_zoomed_stays_on_the_one_shown_pane() {
    let mut app = app_over_file("zoom_tab", "alpha\n");
    key(&mut app, KeyCode::Char('z'));

    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Explorer);
    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::Explorer);
    assert_eq!(app.panes.shown(), PaneSet::only(Focus::Explorer));
}

/// The modifier guard: an earlier phase shipped a global key that swallowed
/// a Ctrl- binding the file view needed.
#[test]
fn ctrl_modified_letters_still_reach_the_file_view() {
    let mut app = app_over_file("zoom_ctrl", "alpha\nbeta\n");
    focus_file_view(&mut app);

    for code in [KeyCode::Char('b'), KeyCode::Char('e'), KeyCode::Char('z')] {
        app.handle_event(event::Event::Key(event::KeyEvent::new(
            code,
            KeyModifiers::CONTROL,
        )));
        assert_eq!(
            app.panes.shown(),
            PaneSet::ALL,
            "a Ctrl- key was taken as a zoom command"
        );
    }
}

/// The status/prompt row is split off above the pane split and drawn
/// below it, so a zoomed pane must not skip past that drawing.
#[test]
fn the_status_line_still_renders_while_zoomed() {
    let mut app = app_over_file("zoom_status", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('z'));

    let status = status_line(&mut app);
    assert!(
        status.contains("lines shown"),
        "expected filter status text while zoomed, got: {status}"
    );
}

/// The pane costs nothing until a filter exists.
#[test]
/// The pane is on screen whenever the explorer is, filters or not — so a
/// user who has never pressed `f i` still sees where filters will appear,
/// and the layout does not shift under them the first time they add one.
fn the_filter_pane_is_present_before_any_filter_is_defined() {
    let mut app = app_over_file("pane_absent", "alpha\n");

    let empty = rendered(&mut app);
    assert!(empty.contains("Filters"), "no filter pane before a filter");
    assert!(empty.contains("f i"), "empty pane drew no hint: {empty}");

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    let populated = rendered(&mut app);
    assert!(populated.contains("Filters"));
    assert!(
        !populated.contains("f i"),
        "hint outlived the empty pane: {populated}"
    );
}

#[test]
fn the_filter_pane_lists_the_patterns() {
    let mut app = app_over_file("pane_lists", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    // `.contains("alpha")` alone would also be satisfied by the file
    // view drawing the log's own one-line body (see
    // `b_hides_the_left_column`, which asserts exactly that over the
    // same body with no filter pane in play) — so this asserts a
    // substring only `FilterList::row_text` produces: the row's index,
    // enabled marker, sense and pattern together.
    assert!(rendered(&mut app).contains("1[x] inc alpha"));
}

/// Tab reaches the filter pane once it exists, and skips it before then.
#[test]
/// The pane is always on screen now, so `Tab` always stops on it. The
/// cycle is three panes deep whether or not a filter exists — the rule is
/// "visible means focusable", with no special case for an empty set.
fn tab_reaches_the_filter_pane_while_it_is_empty() {
    let mut app = app_over_file("pane_focus", "alpha\n");
    draw(&mut app);
    assert!(app.filters.is_empty(), "precondition: no filters");

    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);

    assert_eq!(
        app.focus,
        Focus::Filters,
        "Tab skipped the empty filter pane"
    );

    key(&mut app, KeyCode::Tab);

    assert_eq!(
        app.focus,
        Focus::Explorer,
        "focus did not return to the explorer"
    );
}

/// `Tab` finally has its opposite (#120 §1). crossterm reports Shift-Tab
/// as `KeyCode::BackTab`.
#[test]
fn shift_tab_reverses_tab() {
    let mut app = app_over_file("backtab_cycle", "alpha\n");
    draw(&mut app);
    assert_eq!(app.focus, Focus::Explorer, "sanity: starts on the explorer");

    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::Filters, "did not wrap to the filter pane");
    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::View);
    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::Explorer);

    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.focus,
        Focus::Explorer,
        "Tab then Shift-Tab is not a no-op"
    );
}

/// Tab and Shift-Tab go left to right and skip a hidden pane (#300).
#[test]
fn tab_and_shift_tab_skip_a_hidden_pane() {
    let mut app = app_over_file("tab_skip", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('E'));

    key(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.focus,
        Focus::Filters,
        "Shift-Tab reached the hidden explorer"
    );
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::View, "Tab reached the hidden explorer");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Filters);
}
