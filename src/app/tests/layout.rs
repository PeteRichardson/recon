use super::*;

fn drag_to(app: &mut App, from: u16, to: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), from);
    mouse(app, MouseEventKind::Drag(MouseButton::Left), to);
    mouse(app, MouseEventKind::Up(MouseButton::Left), to);
}

/// The name is comfortably longer than `MIN_AUTO_EXPLORER_WIDTH` on purpose:
/// below the floor the column reports the floor, so a short name would
/// make this pass without the snapping it is named for ever happening.
#[test]
fn auto_width_snaps_to_the_longest_entry() {
    const LONGEST: &str = "a_twenty_five_char_name.rs";
    let mut app = app_over("snap", &["a.rs", LONGEST]);
    draw(&mut app);

    // Name plus two borders; the `>>` marker used to add two more.
    assert!(
        LONGEST.len() as u16 + 2 > MIN_AUTO_EXPLORER_WIDTH,
        "fixture no longer exercises snapping"
    );
    assert_eq!(app.explorer_width(AREA), LONGEST.len() as u16 + 2);
}

#[test]
fn auto_width_is_capped_at_the_default() {
    let long = "a".repeat(200);
    let mut app = app_over("capped", &[long.as_str()]);
    draw(&mut app);

    assert_eq!(app.explorer_width(AREA), MAX_EXPLORER_WIDTH);
}

/// A directory of short names gets the floor, not the width of its
/// longest entry.
///
/// Replaces `auto_width_has_no_floor`, which asserted the opposite —
/// "a directory of short names gets a narrow pane". Snapping that tight
/// is what made entering a one-entry directory move every pane on
/// screen; see #33. Automatic sizing exists to stop the column being
/// uselessly wide, not to win back every column it can.
#[test]
fn auto_width_does_not_shrink_below_the_floor() {
    let mut app = app_over("tiny", &["a"]);
    draw(&mut app);

    assert_eq!(app.explorer_width(AREA), MIN_AUTO_EXPLORER_WIDTH);
}

/// The floor is not a fixed width: a directory of longer names still
/// widens past it, up to the cap.
#[test]
fn auto_width_still_grows_past_the_floor() {
    let name = "a".repeat(MIN_AUTO_EXPLORER_WIDTH as usize + 5);
    let mut app = app_over("above_floor", &[name.as_str()]);
    draw(&mut app);

    assert!(
        app.explorer_width(AREA) > MIN_AUTO_EXPLORER_WIDTH,
        "the floor became a fixed width, got {}",
        app.explorer_width(AREA)
    );
}

/// The floor governs automatic sizing only. A drag is a decision, and it
/// may still take the column down to `MIN_PANE_WIDTH`.
#[test]
fn the_floor_does_not_apply_to_a_dragged_width() {
    let mut app = app_over("drag_below_floor", &["a.rs"]);
    draw(&mut app);
    let divider = app.divider;

    drag_to(&mut app, divider, MIN_PANE_WIDTH);

    assert_eq!(app.explorer_width(AREA), MIN_PANE_WIDTH);
}

/// On a terminal too narrow to honour both, the file view's floor wins:
/// the explorer giving up columns it would like is better than the pane
/// the app exists for becoming unusable.
#[test]
fn the_file_views_floor_outranks_the_explorers() {
    let mut app = app_over("narrow_term", &["a"]);
    let narrow = Rect {
        x: 0,
        y: 0,
        width: MIN_FILE_VIEW_WIDTH + MIN_AUTO_EXPLORER_WIDTH - 5,
        height: 10,
    };
    draw(&mut app);

    assert!(
        app.explorer_width(narrow) < MIN_AUTO_EXPLORER_WIDTH,
        "the floor starved the file view, got {}",
        app.explorer_width(narrow)
    );
}

#[test]
fn dragging_the_divider_pins_the_width() {
    let mut app = app_over("drag", &["a.rs"]);
    draw(&mut app);
    let divider = app.divider;

    drag_to(&mut app, divider, 60);

    assert_eq!(app.explorer_width, PaneWidth::Pinned(60));
    assert_eq!(app.explorer_width(AREA), 60);
}

#[test]
fn a_pinned_width_survives_navigating_to_another_directory() {
    let mut app = app_over("pinned", &["a.rs"]);
    fs::create_dir_all(fixture_dir_path("pinned").join("subdir")).expect("subdir");
    draw(&mut app);
    let divider = app.divider;
    drag_to(&mut app, divider, 55);

    // Walk onto the subdirectory and descend into it.
    for _ in 0..8 {
        app.handle_event(event::Event::Key(KeyCode::Down.into()));
    }
    app.handle_event(event::Event::Key(KeyCode::Enter.into()));
    draw(&mut app);

    assert_eq!(app.explorer_width(AREA), 55, "navigation overrode the drag");
}

#[test]
fn double_clicking_the_divider_restores_automatic_sizing() {
    let mut app = app_over("dblclick", &["a.rs"]);
    draw(&mut app);
    let divider = app.divider;
    drag_to(&mut app, divider, 70);
    assert_eq!(app.explorer_width(AREA), 70);
    draw(&mut app);

    let divider = app.divider;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);

    assert_eq!(app.explorer_width, PaneWidth::Auto);
    // Back to automatic sizing, which for a name this short is the floor
    // rather than the name's own width — see `MIN_AUTO_EXPLORER_WIDTH`.
    assert_eq!(app.explorer_width(AREA), MIN_AUTO_EXPLORER_WIDTH);
}

#[test]
fn dragging_cannot_collapse_either_pane() {
    let mut app = app_over("clamp", &["a.rs"]);
    draw(&mut app);

    let divider = app.divider;
    drag_to(&mut app, divider, 0);
    assert!(
        app.explorer_width(AREA) >= MIN_PANE_WIDTH,
        "explorer pane collapsed"
    );

    draw(&mut app);
    let divider = app.divider;
    drag_to(&mut app, divider, AREA.width);
    draw(&mut app);
    // A drag all the way to the far edge stops at the file view's floor,
    // `MIN_FILE_VIEW_WIDTH`, not at `MIN_PANE_WIDTH`: the ceiling applies
    // to a drag just as much as to auto-sizing. The filter pane gives
    // way first, down to its own floor (#300).
    assert_eq!(
        app.view_area.width, MIN_FILE_VIEW_WIDTH,
        "a hard drag to the far edge did not stop at the file view's floor"
    );
    assert_eq!(app.filter_area.width, MIN_PANE_WIDTH);
}

/// #300: the filter pane is a column on the right, and its divider — its
/// left border and the view's right — drags like the explorer's. A width
/// rather than a column is stored: the pane is anchored to the right.
#[test]
fn dragging_the_filter_divider_pins_the_filter_panes_width() {
    let mut app = app_over("fdrag", &["a.rs"]);
    draw(&mut app);
    let divider = app.filter_divider;

    drag_to(&mut app, divider, divider - 5);

    let width = AREA.right() - (divider - 5);
    assert_eq!(app.filter_width, PaneWidth::Pinned(width));
    assert_eq!(app.filter_pane_width(AREA), width);
}

#[test]
fn double_clicking_the_filter_divider_restores_automatic_sizing() {
    let mut app = app_over("fdbl", &["a.rs"]);
    draw(&mut app);
    let divider = app.filter_divider;
    drag_to(&mut app, divider, divider - 5);
    assert_ne!(
        app.filter_width,
        PaneWidth::Auto,
        "sanity: the drag did not pin a width to restore from"
    );
    draw(&mut app);

    let divider = app.filter_divider;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), divider);

    assert_eq!(app.filter_width, PaneWidth::Auto);
    assert_eq!(app.filter_pane_width(AREA), MIN_AUTO_FILTER_WIDTH);
}

/// A drag far to the left asks for most of the terminal. The file view's
/// floor stops it, as it stops the explorer's.
#[test]
fn dragging_the_filter_divider_cannot_starve_the_file_view() {
    let mut app = app_over("fdrag_far", &["a.rs"]);
    draw(&mut app);
    let divider = app.filter_divider;

    drag_to(&mut app, divider, 0);
    draw(&mut app);

    assert_eq!(app.view_area.width, MIN_FILE_VIEW_WIDTH);
    assert!(app.explorer_area.width >= MIN_PANE_WIDTH);
}

/// And to the right edge: the pane keeps `MIN_PANE_WIDTH`.
#[test]
fn dragging_cannot_collapse_the_filter_pane() {
    let mut app = app_over("fdrag_collapse", &["a.rs"]);
    draw(&mut app);
    let divider = app.filter_divider;

    drag_to(&mut app, divider, AREA.right() + 10);

    assert_eq!(app.filter_pane_width(AREA), MIN_PANE_WIDTH);
}

/// A drag that went down on one divider keeps moving that one, and a
/// double-click on one does not reset the other.
#[test]
fn the_two_dividers_move_different_panes() {
    let mut app = app_over("fdrag_two", &["a.rs"]);
    draw(&mut app);
    let explorer = app.explorer_width(AREA);
    let divider = app.filter_divider;

    drag_to(&mut app, divider, divider - 4);

    assert_eq!(app.explorer_width, PaneWidth::Auto);
    assert_eq!(app.explorer_width(AREA), explorer);
}

/// The filter pane's divider exists only with the file view to its left.
/// With the view hidden the filter pane fills what the explorer leaves,
/// and the one boundary between them is the explorer's.
#[test]
fn with_the_view_hidden_the_only_divider_is_the_explorers() {
    let mut app = app_over("fdrag_noview", &["a.rs"]);
    app.panes = Panes::hiding([Focus::View]);
    draw(&mut app);

    assert_eq!(app.filter_divider, u16::MAX);
    assert_eq!(app.divider, app.filter_area.x);
    let divider = app.divider;
    drag_to(&mut app, divider, 30);
    assert_eq!(app.explorer_width, PaneWidth::Pinned(30));
    assert_eq!(app.filter_width, PaneWidth::Auto);
}

/// A click nowhere near the divider is still the focused widget's business.
#[test]
fn a_click_away_from_the_divider_is_not_a_drag() {
    let mut app = app_over("passthrough", &["a.rs"]);
    draw(&mut app);
    let before = app.explorer_width(AREA);

    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 90);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 60);

    assert_eq!(app.dragging, None);
    assert_eq!(app.explorer_width(AREA), before);
}
