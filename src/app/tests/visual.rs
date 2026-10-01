use super::*;

// ---- visual mode and the yank (#67) ---------------------------------

/// Swap in the recording clipboard and hand back a handle the test can
/// read afterwards — the shape `record_launches` uses for the editor,
/// and for the same reason: `Clipboard::copy` takes `&self`, so the
/// shared reference needs no mutability beyond the `Mutex` inside.
fn record_copies(app: &mut App) -> Rc<RecordingClipboard> {
    let clipboard = Rc::new(RecordingClipboard::default());
    app.clipboard = Box::new(Rc::clone(&clipboard));
    clipboard
}

/// An app over one file, focused on the view with a recording clipboard
/// in place: what nearly every test below starts from.
fn app_for_yank(name: &str, body: &str) -> (App<'static>, Rc<RecordingClipboard>) {
    let mut app = app_over_file(name, body);
    let clipboard = record_copies(&mut app);
    key(&mut app, KeyCode::Char('t'));
    (app, clipboard)
}

/// The headline acceptance criterion, use case 2 in the issue: a run of
/// log lines selected with `V` and `j`, copied with `y`.
#[test]
fn v_shift_j_y_copies_the_selected_lines() {
    let (mut app, clipboard) = app_for_yank("yank_lines", "alpha\nbeta\ngamma\ndelta\n");
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('y'));

    assert_eq!(clipboard.only_copy(), "alpha\nbeta\ngamma\n");
    assert_eq!(message(&app), Some("yanked 3 lines"));
    assert!(app.visual.is_none(), "the yank left visual mode on");
}

/// Use case 1: a symbol selected character-wise and copied, ready to be
/// pasted into an `f i` prompt. `l` grows the selection a character at a
/// time, and the character under the cursor is included, as vim's `v` is.
#[test]
fn v_l_y_copies_the_characters_under_the_selection() {
    let (mut app, clipboard) = app_for_yank("yank_chars", "_ZN4core3fmt::pad\n");
    key(&mut app, KeyCode::Char('v'));
    for _ in 0..4 {
        key(&mut app, KeyCode::Char('l'));
    }
    key(&mut app, KeyCode::Char('y'));

    assert_eq!(clipboard.only_copy(), "_ZN4c");
    assert_eq!(message(&app), Some("yanked 5 characters"));
}

/// `v` twice ends the selection, as vim does; `V` on a character-wise
/// selection switches it rather than ending it, keeping the anchor.
#[test]
fn v_toggles_and_shift_v_switches_without_losing_the_anchor() {
    let (mut app, clipboard) = app_for_yank("yank_toggle", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('v'));
    assert!(app.visual.is_some());
    key(&mut app, KeyCode::Char('v'));
    assert!(app.visual.is_none(), "a second v did not end the selection");

    key(&mut app, KeyCode::Char('v'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('V'));
    assert_eq!(
        app.visual.map(|v| (v.anchor, v.linewise)),
        Some((0, true)),
        "V lost the anchor or did not switch"
    );
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(clipboard.only_copy(), "alpha\nbeta\n");
}

/// `Esc` ends the selection, and — the reason it is layered above the
/// search arms — leaves the search alone.
#[test]
fn esc_ends_the_selection_and_keeps_the_search() {
    let (mut app, _) = app_for_yank("yank_esc", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "beta");
    key(&mut app, KeyCode::Enter);
    assert!(app.search.is_some(), "sanity: a search is set");

    key(&mut app, KeyCode::Char('v'));
    key(&mut app, KeyCode::Esc);
    assert!(app.visual.is_none(), "Esc did not end the selection");
    assert!(
        app.search.is_some(),
        "Esc dropped the search as well as the selection"
    );

    // A second Esc, with no selection, reaches the search as before.
    key(&mut app, KeyCode::Esc);
    assert!(app.search.is_none(), "Esc no longer clears the search");
}

/// The rule the issue settles: a yank copies the lines that are on
/// screen. `u` mid-selection reveals the rest, and the selection grows
/// to include them, because the anchor is a document line.
#[test]
fn a_yank_copies_the_visible_lines_and_u_grows_the_selection() {
    let (mut app, clipboard) = app_for_yank("yank_hidden", "hit a\nmiss\nhit b\nmiss\nhit c\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "hit");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('u'));
    assert_eq!(
        view_lines(&app),
        vec![
            "hit a".to_string(),
            "hit b".to_string(),
            "hit c".to_string()
        ],
        "sanity: hiding"
    );

    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('G'));
    // `u` while the selection is live: the hidden lines come back and
    // are inside `[anchor, cursor]`, so they join the yank.
    key(&mut app, KeyCode::Char('u'));
    assert!(app.visual.is_some(), "u invalidated the selection");
    key(&mut app, KeyCode::Char('y'));

    assert_eq!(clipboard.only_copy(), "hit a\nmiss\nhit b\nmiss\nhit c\n");
}

/// The same selection yanked *while* hiding takes only the visible
/// lines — the other half of the rule above.
#[test]
fn a_yank_while_hiding_skips_the_hidden_lines() {
    let (mut app, clipboard) =
        app_for_yank("yank_hidden_only", "hit a\nmiss\nhit b\nmiss\nhit c\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "hit");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('u'));

    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('G'));
    key(&mut app, KeyCode::Char('y'));

    assert_eq!(clipboard.only_copy(), "hit a\nhit b\nhit c\n");
}

/// A yank in dimmed view includes the dimmed lines: they are visible,
/// and the issue says so explicitly.
#[test]
fn a_yank_in_dimmed_view_includes_the_dimmed_lines() {
    let (mut app, clipboard) = app_for_yank("yank_dimmed", "hit a\nmiss\nhit b\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "hit");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(app.document.mode(), Mode::Dimmed, "sanity: dimming");

    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('G'));
    key(&mut app, KeyCode::Char('y'));

    assert_eq!(clipboard.only_copy(), "hit a\nmiss\nhit b\n");
}

/// Leaving the view drops the selection, by whichever route. Parking it
/// would leave a selection live in a pane where no key can act on it.
#[test]
fn leaving_the_view_ends_the_selection() {
    for leave in [KeyCode::Char('e'), KeyCode::Char('f'), KeyCode::Tab] {
        let (mut app, _) = app_for_yank(
            &format!("yank_focus_{}", format!("{leave:?}").to_lowercase()),
            "alpha\nbeta\n",
        );
        key(&mut app, KeyCode::Char('v'));
        assert!(app.visual.is_some(), "sanity: selecting");
        key(&mut app, leave);
        assert!(
            app.visual.is_none(),
            "{leave:?} left the selection live outside the view"
        );
    }
}

/// Loading another file ends it too: the anchor names a line of a
/// document that no longer exists.
#[test]
fn loading_another_file_ends_the_selection() {
    let mut app = app_over_file("yank_load", "alpha\nbeta\n");
    record_copies(&mut app);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('V'));

    let dir = fixture_dir_path("yank_load");
    fs::write(dir.join("other.txt"), "gamma\n").expect("write");
    app.perform_widget_action(Action::Load(dir.join("other.txt")));

    assert!(app.visual.is_none(), "the load left the selection live");
}

/// `y` with nothing selected says how to select something, rather than
/// doing nothing at all (#120 §9's rule for a key that has no effect
/// where it was pressed).
#[test]
fn y_with_nothing_selected_says_so_and_copies_nothing() {
    let (mut app, clipboard) = app_for_yank("yank_nothing", "alpha\n");
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(
        message(&app),
        Some("nothing selected · v starts a selection")
    );
    let copies = clipboard.copies();
    assert!(copies.is_empty(), "{copies:?}");
}

/// `v`, `V` and `y` outside the view hint rather than acting: there is no
/// cursor column in the other panes to anchor a selection to.
#[test]
fn v_and_y_outside_the_view_hint() {
    let (mut app, clipboard) = app_for_yank("yank_hint", "alpha\n");
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('v'));
    assert_eq!(message(&app), Some("v selects text in the file view · t v"));
    assert!(app.visual.is_none());
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(
        message(&app),
        Some("y copies a selection in the file view · t v")
    );
    let copies = clipboard.copies();
    assert!(copies.is_empty(), "{copies:?}");
}

/// A clipboard that fails reports on the status row in red, the way a
/// missing editor does, and the selection still ends — the user's next
/// `y` should not silently re-copy an old range.
#[test]
fn a_failing_clipboard_reports_and_still_ends_the_selection() {
    let mut app = app_over_file("yank_fail", "alpha\nbeta\n");
    app.clipboard = Box::new(RecordingClipboard::failing("pbcopy: not found"));
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('y'));

    let message = app.status_message.as_ref().expect("a message");
    assert!(
        message.text.contains("pbcopy: not found"),
        "{}",
        message.text
    );
    assert!(
        message.error,
        "a clipboard failure was not reported as an error"
    );
    assert!(app.visual.is_none());
}

/// `Ctrl-y` still scrolls: `y` claims the bare key only.
#[test]
fn ctrl_y_still_scrolls_the_view() {
    let (mut app, clipboard) = app_for_yank("yank_ctrl", &numbered_lines(60));
    for _ in 0..20 {
        key(&mut app, KeyCode::Char('j'));
    }
    draw(&mut app);
    let before = app.view.textarea().scroll_top().0;
    ctrl(&mut app, KeyCode::Char('y'));
    assert!(
        app.view.textarea().scroll_top().0 < before,
        "Ctrl-y did not scroll up"
    );
    assert!(clipboard.copies().is_empty(), "Ctrl-y reached the yank");
}

/// The badge names the mode, so a selection is never invisible while
/// the cursor is off screen.
#[test]
fn the_status_row_badges_the_visual_mode() {
    let (mut app, _) = app_for_yank("yank_badge", "alpha\nbeta\n");
    assert!(!rendered(&mut app).contains("VISUAL"));
    key(&mut app, KeyCode::Char('v'));
    assert!(rendered(&mut app).contains("VISUAL"), "no badge for v");
    key(&mut app, KeyCode::Char('V'));
    assert!(rendered(&mut app).contains("V-LINE"), "no badge for V");
    key(&mut app, KeyCode::Esc);
    assert!(
        !rendered(&mut app).contains("V-LINE"),
        "the badge outlived the mode"
    );
}

/// The selection is drawn, not merely held: the selected characters wear
/// the reversed bar the cursor line wears.
#[test]
fn the_selection_is_painted_over_the_selected_text() {
    let (mut app, _) = app_for_yank("yank_paint", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('j'));

    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let row_of = |needle: &str| {
        (0..AREA.height)
            .find(|&y| {
                (0..AREA.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .contains(needle)
            })
            .unwrap_or_else(|| panic!("{needle} is not on screen"))
    };
    let reversed = |y: u16| {
        (0..AREA.width).any(|x| {
            buf[(x, y)]
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)
        })
    };
    assert!(reversed(row_of("alpha")), "the anchor row is not painted");
    assert!(reversed(row_of("beta")), "the cursor row is not painted");
}

// ---- `/` grows a selection to the hit (#273) -------------------------

/// With `v` active, `/` keeps the anchor and moves the cursor with the
/// incremental search, so the selection reaches the hit line while the
/// pattern is typed. With `V`, it covers whole lines.
#[test]
fn a_search_grows_the_selection_to_the_hit_while_typing() {
    for linewise in [false, true] {
        let (mut app, _) = app_for_yank(
            &format!("search_grows_{linewise}"),
            "alpha one\nbeta two\ngamma ERROR here\ndelta\n",
        );
        key(&mut app, KeyCode::Char('l'));
        key(&mut app, KeyCode::Char('l'));
        key(&mut app, KeyCode::Char(if linewise { 'V' } else { 'v' }));
        key(&mut app, KeyCode::Char('/'));
        typed(&mut app, "ERR");
        draw(&mut app);

        assert!(app.prompt.is_some(), "sanity: the prompt is open");
        assert_eq!(
            cursor_source(&app),
            2,
            "the search did not move while typing"
        );
        assert_eq!(
            app.visual.map(|v| (v.anchor, v.col, v.linewise)),
            Some((0, 2, linewise)),
            "the prompt lost or moved the anchor"
        );
        // `gamma ` is six characters, so the hit is at column 6 and a
        // character-wise selection includes it: the end is exclusive.
        let expected = if linewise {
            ((0, 0), (2, "gamma ERROR here".len()))
        } else {
            ((0, 2), (2, 7))
        };
        assert_eq!(
            app.painted_selection(),
            Some(expected),
            "the selection is not painted from the anchor to the hit"
        );
    }
}

/// Enter keeps the selection the search reached, and `y` copies the
/// lines it covers: "from here to the next ERROR" is one search and one
/// yank.
#[test]
fn enter_keeps_the_grown_selection_and_y_copies_it() {
    let (mut app, clipboard) =
        app_for_yank("search_grows_yank", "alpha\nbeta\ngamma ERROR\ndelta\n");
    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_none(), "Enter left the prompt open");
    assert!(app.visual.is_some(), "Enter ended the selection");
    assert_eq!(cursor_source(&app), 2, "Enter did not keep the position");

    key(&mut app, KeyCode::Char('y'));
    assert_eq!(clipboard.only_copy(), "alpha\nbeta\ngamma ERROR\n");
    assert_eq!(message(&app), Some("yanked 3 lines"));
}

/// The yank rule is unchanged by how the selection grew: the search
/// finds the hit among the visible lines, and `y` copies the visible
/// lines between the anchor and it, skipping the hidden ones.
#[test]
fn a_grown_selection_yanks_only_the_visible_lines() {
    let (mut app, clipboard) = app_for_yank(
        "search_grows_hidden",
        "keep a\nmiss\nkeep b\nmiss\nkeep ERROR\nkeep after\n",
    );
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "keep");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('u'));
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(
        cursor_source(&app),
        0,
        "sanity: the cursor starts on the first line"
    );

    key(&mut app, KeyCode::Char('V'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('y'));

    assert_eq!(clipboard.only_copy(), "keep a\nkeep b\nkeep ERROR\n");
    assert_eq!(message(&app), Some("yanked 3 lines"));
}

/// Esc in the prompt puts back the cursor, the scroll and the selection
/// exactly as they were before `/`: a bad probe does not lose the
/// selection, and does not leave it grown either.
#[test]
fn esc_in_the_prompt_restores_the_selection_with_the_origin() {
    let mut app = app_over_file("search_grows_esc", &numbered_lines(400));
    draw_tall(&mut app);
    key(&mut app, KeyCode::Char('t'));
    for _ in 0..40 {
        key(&mut app, KeyCode::Char('j'));
    }
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('l'));
    draw_tall(&mut app);
    let (row, col, screen_row) = (
        cursor_source(&app),
        app.view.cursor_col(),
        cursor_screen_row(&app),
    );
    assert!(screen_row > 0, "sanity: the cursor is off the top row");

    key(&mut app, KeyCode::Char('v'));
    let before = app.visual;
    assert!(before.is_some(), "sanity: a selection is active");

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "line 300$");
    draw_tall(&mut app);
    assert_eq!(cursor_source(&app), 300, "sanity: moved while typing");

    key(&mut app, KeyCode::Esc);
    draw_tall(&mut app);

    assert!(app.prompt.is_none(), "the prompt is still open");
    assert_eq!(
        app.visual, before,
        "Esc in the prompt changed or ended the selection"
    );
    assert_eq!(cursor_source(&app), row, "Esc did not restore the cursor");
    assert_eq!(app.view.cursor_col(), col, "Esc did not restore the column");
    assert_eq!(
        cursor_screen_row(&app),
        screen_row,
        "Esc did not restore the scroll"
    );
    // Back to a one-character selection on the origin line: the painted
    // span starts and ends on the same row, one column wide.
    let ((start_row, start_col), (end_row, end_col)) =
        app.painted_selection().expect("the selection is painted");
    assert_eq!(start_row, end_row, "the selection is still grown");
    assert_eq!((start_col, end_col), (col, col + 1));
}

/// The global Esc order is unchanged by the grown selection: outside the
/// prompt, the first Esc ends the selection and keeps the search the
/// selection was grown under; the second clears the search.
#[test]
fn esc_after_a_grown_selection_ends_it_before_the_search() {
    let (mut app, _) = app_for_yank("search_grows_esc_order", "alpha\nbeta ERROR\n");
    key(&mut app, KeyCode::Char('v'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ERROR");
    key(&mut app, KeyCode::Enter);
    assert!(app.visual.is_some() && app.search.is_some(), "sanity");

    key(&mut app, KeyCode::Esc);
    assert!(app.visual.is_none(), "Esc did not end the selection first");
    assert!(
        app.search.is_some(),
        "Esc dropped the search with the selection"
    );

    key(&mut app, KeyCode::Esc);
    assert!(
        app.search.is_none(),
        "the second Esc did not clear the search"
    );
}

// ---- the mouse selects too (#67) ------------------------------------

/// A click in the file view puts the cursor where the pointer is, which
/// is the motions that would have got there.
#[test]
fn a_click_in_the_view_moves_the_cursor_to_the_character() {
    let (mut app, _) = app_for_yank("click_cursor", "alpha beta\ngamma delta\n");
    draw(&mut app);
    let inner = app.view_area.inner(Margin::new(1, 1));
    // Row 1 of the text, six columns in past the gutter.
    let gutter = app.view.textarea().gutter_width();
    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        inner.x + gutter + 6,
        inner.y + 1,
    );

    assert_eq!(view_cursor_row(&app), 1);
    assert_eq!(app.view.cursor_col(), 6, "the column missed the character");
}

/// A drag selects from the press point to the pointer, and `y` copies
/// it — the mouse selects, the key copies, as `v` … `y` does.
#[test]
fn a_drag_selects_and_y_copies_it() {
    let (mut app, clipboard) = app_for_yank("drag_select", "alpha beta\ngamma delta\n");
    draw(&mut app);
    let inner = app.view_area.inner(Margin::new(1, 1));
    let gutter = app.view.textarea().gutter_width();
    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        inner.x + gutter + 6,
        inner.y,
    );
    mouse_at(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        inner.x + gutter + 4,
        inner.y + 1,
    );
    mouse_at(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        inner.x + gutter + 4,
        inner.y + 1,
    );

    assert!(app.visual.is_some(), "the release ended the selection");
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(clipboard.only_copy(), "beta\ngamma");
}

/// A double-click selects the word under the pointer, by `*`'s rule for
/// a word — so a mangled symbol comes whole.
#[test]
fn a_double_click_selects_the_word() {
    let (mut app, clipboard) = app_for_yank("double_click", "call _ZN4core3fmt(x)\n");
    draw(&mut app);
    let inner = app.view_area.inner(Margin::new(1, 1));
    let gutter = app.view.textarea().gutter_width();
    let at = inner.x + gutter + 8;
    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        at,
        inner.y,
    );
    mouse_at(&mut app, MouseEventKind::Up(MouseButton::Left), at, inner.y);
    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        at,
        inner.y,
    );

    key(&mut app, KeyCode::Char('y'));
    assert_eq!(clipboard.only_copy(), "_ZN4core3fmt");
}

/// Two clicks further apart than `DOUBLE_CLICK` only move the cursor.
#[test]
fn a_slow_second_click_selects_nothing() {
    let (mut app, _) = app_for_yank("double_click_slow", "call _ZN4core3fmt(x)\n");
    draw(&mut app);
    let inner = app.view_area.inner(Margin::new(1, 1));
    let at = inner.x + app.view.textarea().gutter_width() + 8;
    let down = MouseEventKind::Down(MouseButton::Left);
    mouse_at(&mut app, down, at, inner.y);
    mouse_at(&mut app, MouseEventKind::Up(MouseButton::Left), at, inner.y);
    later(&mut app, DOUBLE_CLICK + Duration::from_millis(1));
    mouse_at(&mut app, down, at, inner.y);

    assert!(app.visual.is_none(), "a slow pair selected a word");
}

/// A click ends a selection in progress, as it does in vim.
#[test]
fn a_click_ends_a_selection() {
    let (mut app, _) = app_for_yank("click_ends", "alpha beta\ngamma\n");
    draw(&mut app);
    key(&mut app, KeyCode::Char('v'));
    let inner = app.view_area.inner(Margin::new(1, 1));
    let at = inner.x + app.view.textarea().gutter_width() + 2;
    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        at,
        inner.y + 1,
    );
    assert!(app.visual.is_none(), "the click left the selection live");
}

/// A click on a directory listing still opens the entry: the listing's
/// rows are entries, not text, and #58's behaviour is unchanged.
#[test]
fn a_click_on_a_listing_row_still_opens_the_entry() {
    let mut app = app_over_files(
        "click_listing",
        &[("a.log", "alpha\n"), ("b.log", "beta\n")],
    );
    record_copies(&mut app);
    // Point the view at the directory so it shows the listing.
    let dir = fixture_dir_path("click_listing");
    app.perform_widget_action(Action::Preview(dir));
    assert!(app.view.showing_directory(), "sanity: a listing");
    draw(&mut app);

    let inner = app.view_area.inner(Margin::new(1, 1));
    mouse_at(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        inner.x + 2,
        inner.y,
    );
    assert_eq!(
        shown(&app),
        "a.log",
        "the listing row did not open its entry"
    );
    assert!(app.visual.is_none(), "a listing row started a selection");
}
