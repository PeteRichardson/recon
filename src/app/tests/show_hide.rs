use super::*;

// ---- #300: show and hide each pane ------------------------------------

/// An app over a one-line file, with `global.hide.view` bound to `T` as
/// a user would in `config.toml`: it has no default key.
fn app_with_hide_view_key(name: &str) -> App<'static> {
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert("global.hide.view".to_string(), vec!["T".to_string()]);
    let (keymap, _) = crate::keymap::Keymap::new(&crate::config::KeymapConfig { bindings })
        .expect("a valid keymap");
    let file = fixture_path(name, "alpha\n");
    App::new(&Config {
        path: file.display().to_string(),
        bindings: keymap,
        ..Config::default()
    })
}

#[test]
fn e_and_f_hide_their_panes_and_lowercase_shows_them_again() {
    let mut app = app_over_file("hide_ef", "alpha\n");

    key(&mut app, KeyCode::Char('E'));
    assert!(
        !app.panes.is_shown(Focus::Explorer),
        "E did not hide the explorer"
    );
    key(&mut app, KeyCode::Char('F'));
    assert!(
        !app.panes.is_shown(Focus::Filters),
        "F did not hide the filter pane"
    );
    assert_eq!(app.focus, Focus::View);

    key(&mut app, KeyCode::Char('e'));
    assert!(app.panes.is_shown(Focus::Explorer));
    assert_eq!(app.focus, Focus::Explorer);
    key(&mut app, KeyCode::Char('f'));
    assert!(app.panes.is_shown(Focus::Filters));
    assert_eq!(app.focus, Focus::Filters);
}

#[test]
fn the_file_view_hides_by_a_bound_key_and_t_shows_it_again() {
    let mut app = app_with_hide_view_key("hide_view");
    key(&mut app, KeyCode::Char('t'));

    key(&mut app, KeyCode::Char('T'));
    assert!(
        !app.panes.is_shown(Focus::View),
        "the bound key did not hide the view"
    );
    assert_eq!(
        app.focus,
        Focus::Filters,
        "focus goes to the next shown pane"
    );

    key(&mut app, KeyCode::Char('t'));
    assert!(app.panes.is_shown(Focus::View));
    assert_eq!(app.focus, Focus::View);
}

/// A hidden pane is not drawn, and the others take its width.
#[test]
fn a_hidden_pane_is_not_drawn() {
    let mut app = app_over_file("hide_draw", "alpha\n");
    key(&mut app, KeyCode::Char('E'));

    let text = rendered(&mut app);

    // See `b_hides_the_left_column` for why `../` is the probe.
    assert!(!text.contains("../"), "the explorer is still on screen");
    assert_eq!(
        app.view_area.x, 0,
        "the view did not take the explorer's place"
    );
    assert_eq!(app.explorer_area.width, 0);
    assert_eq!(app.divider, u16::MAX, "a hidden explorer left a divider");
}

#[test]
fn the_last_shown_pane_cannot_be_hidden() {
    let mut app = app_with_hide_view_key("hide_last");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('z'));

    key(&mut app, KeyCode::Char('T'));

    assert!(app.panes.is_shown(Focus::View), "the last pane was hidden");
    assert_eq!(app.focus, Focus::View);
    let status = status_line(&mut app);
    assert!(status.contains(LAST_PANE), "no message: {status}");
}

/// Hiding the focused pane moves focus to the file view.
#[test]
fn hiding_the_focused_pane_moves_focus_to_the_file_view() {
    let mut app = app_over_file("hide_focus", "alpha\n");
    assert_eq!(app.focus, Focus::Explorer);

    key(&mut app, KeyCode::Char('E'));

    assert_eq!(app.focus, Focus::View);
}

/// A hidden pane keeps its state, and its global keys still work.
#[test]
fn a_hidden_filter_pane_keeps_its_global_keys() {
    let mut app = app_with_two_filters("hide_global_keys");
    key(&mut app, KeyCode::Char('F'));

    key(&mut app, KeyCode::Char('1'));
    assert!(
        !app.filters.filters()[0].enabled,
        "1 did not toggle the hidden pane's filter"
    );
    key(&mut app, KeyCode::Char('!'));
    assert!(
        !app.filters.any_enabled(),
        "! did not reach the hidden pane's filters"
    );
    assert!(
        !app.panes.is_shown(Focus::Filters),
        "a global key showed the pane"
    );
}

/// The example in #300: `z` in the file view, then `f`, then `z`. Two
/// panes are shown at the second `z`, so it zooms the filter pane. It
/// does not restore.
#[test]
fn z_after_f_zooms_the_filter_pane_rather_than_restoring() {
    let mut app = app_over_file("zoom_example", "alpha\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('z'));
    assert_eq!(app.panes.shown(), PaneSet::only(Focus::View));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('z'));

    assert_eq!(app.panes.shown(), PaneSet::only(Focus::Filters));
    assert_eq!(app.focus, Focus::Filters);
}

/// `f i … Enter` with the filter pane hidden: the pane shows for the
/// chain, and hides again when focus goes back. A chain does not change
/// the layout.
#[test]
fn a_chain_with_the_filter_pane_hidden_leaves_it_hidden() {
    let mut app = app_over_file("chain_hidden", "plain\nfn one\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('F'));

    key(&mut app, KeyCode::Char('f'));
    assert!(
        app.panes.is_shown(Focus::Filters),
        "f did not show the pane for the chain"
    );
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "fn");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.filters.row_count(), 1, "the filter was not added");
    assert_eq!(app.focus, Focus::View, "focus did not go back");
    assert!(
        !app.panes.is_shown(Focus::Filters),
        "the chain changed the layout"
    );
}

/// `f` alone, with the pane hidden, shows it and stays: that is not a
/// chain that returns, so the pane stays shown.
#[test]
fn f_alone_shows_a_hidden_filter_pane_to_stay() {
    let mut app = app_over_file("chain_hidden_stay", "alpha\n");
    key(&mut app, KeyCode::Char('F'));

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('j'));

    assert!(app.panes.is_shown(Focus::Filters));
    assert_eq!(app.focus, Focus::Filters);
}

/// #306: more patterns than the scan's bits turns the explorer's file
/// matching off, and the status line says so, with the count and the
/// limit. A delete that brings the count back under the limit takes the
/// note away.
#[test]
fn the_status_line_says_when_file_matching_is_off() {
    let mut app = app_over_file("scan_off_note", "alpha\n");
    let room = filter::MAX_PATTERNS - crate::syntax::Kind::ALL.len();
    for i in 0..room {
        app.filters.add(&format!("p{i}")).expect("valid pattern");
    }
    assert!(
        !status_line_at(&mut app, 200).contains("file matching off"),
        "at the limit, matching still runs"
    );

    app.filters.add("one too many").expect("valid pattern");
    let status = status_line_at(&mut app, 200);
    assert!(
        status.contains("file matching off: 129 patterns, limit 128"),
        "no note: {status}"
    );

    app.filters.remove(0);
    assert!(!status_line_at(&mut app, 200).contains("file matching off"));
}

/// With the pane hidden, the status line says how many filters are on,
/// so a user who hid it knows why lines are coloured or gone.
#[test]
fn the_status_line_counts_filters_on_while_the_pane_is_hidden() {
    let mut app = app_with_two_filters("hide_status");
    assert!(
        !status_line(&mut app).contains("filters: "),
        "shown pane, no indicator"
    );

    key(&mut app, KeyCode::Char('F'));
    let status = status_line(&mut app);
    assert!(status.contains("filters: 2 on"), "no indicator: {status}");

    key(&mut app, KeyCode::Char('1'));
    let status = status_line(&mut app);
    assert!(
        status.contains("filters: 1 on"),
        "the count did not follow: {status}"
    );

    key(&mut app, KeyCode::Char('!'));
    let status = status_line(&mut app);
    assert!(
        !status.contains(" on"),
        "nothing is on, so no indicator: {status}"
    );
}

/// `--hide-pane` (or `[layout] hide_panes`) hides panes at startup, and
/// focus starts on a shown pane.
#[test]
fn hide_pane_at_startup_hides_those_panes() {
    let file = fixture_path("hide_startup", "alpha\n");
    let app = App::new(&Config {
        path: file.display().to_string(),
        hide_pane: Some(vec![
            crate::panes::Pane::Explorer,
            crate::panes::Pane::Filters,
        ]),
        ..Config::default()
    });

    assert_eq!(app.panes.shown(), PaneSet::only(Focus::View));
    assert_eq!(app.focus, Focus::View, "focus started on a hidden pane");
}

#[test]
fn tab_reaches_the_filter_pane_once_a_filter_exists() {
    let mut app = app_over_file("pane_focus_on", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);
    draw(&mut app);

    let mut seen = vec![app.focus];
    for _ in 0..3 {
        key(&mut app, KeyCode::Tab);
        seen.push(app.focus);
    }

    assert!(
        seen.contains(&Focus::Filters),
        "the filter pane never took focus: {seen:?}"
    );
}

/// The README documents the exact order `Tab` cycles the panes in
/// (explorer, file view, filter pane), which follows `Focus::next`
/// rather than anything visual — the filter pane sits *above* the file
/// view on screen but *after* it in the cycle. Nothing else pins that
/// order, so a reshuffle of `Focus::next` would otherwise leave the
/// README quietly wrong with an otherwise green suite.
///
/// Asserting against named `Focus` variants is what makes a reordering
/// break this test loudly. The bare `0`/`1`/`2` indices this used to
/// compare would have gone on passing while meaning something new (#73).
#[test]
fn tab_cycles_explorer_then_file_view_then_filter_pane() {
    let mut app = app_over_file("tab_cycle_order", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);
    // Creating a filter now *leaves* focus on the filter pane — `f` moved
    // it there. This test is about the cycle, not about where creating a
    // filter lands, so come back to the explorer deliberately rather than
    // assuming the setup left focus untouched.
    key(&mut app, KeyCode::Char('e'));
    draw(&mut app);

    assert_eq!(
        app.focus,
        Focus::Explorer,
        "should start focused on the explorer"
    );

    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.focus,
        Focus::View,
        "one Tab from the explorer should reach the file view"
    );

    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.focus,
        Focus::Filters,
        "two Tabs from the explorer should reach the filter pane"
    );

    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.focus,
        Focus::Explorer,
        "three Tabs should cycle back to the explorer"
    );
}

/// `App::render` has two branches that draw a pane — the ordinary split
/// and the zoom special case — and only the split branch is exercised by
/// the tests above. Both go through `render_pane`, which is what hands
/// the filter pane the `ActiveFilters` it cannot hold itself; a zoom
/// branch that bypassed it would focus an invisible pane showing a blank
/// screen.
#[test]
fn zooming_the_filter_pane_shows_its_contents() {
    let mut app = app_over_file("zoom_filter_pane", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    // Bounded, like `focus_file_view`, rather than an unbounded `while`:
    // if the filter pane ever stopped being reachable, a `while` here
    // would hang the test instead of failing it.
    for _ in 0..PANE_COUNT {
        if app.focus == Focus::Filters {
            break;
        }
        key(&mut app, KeyCode::Tab);
    }
    assert_eq!(
        app.focus,
        Focus::Filters,
        "could not reach the filter pane by tabbing"
    );
    key(&mut app, KeyCode::Char('z'));

    let after = rendered(&mut app);
    assert!(
        after.contains("Filters"),
        "the filter pane's border is not on screen: {after}"
    );
    // The full row, not a bare `contains("alpha")`: the file view (whose
    // gutter would also print a bare `alpha`-free line number) is not
    // drawn while zoomed, which is what let `contains("alpha")` alone
    // pass here — incidentally, not because it actually pinned the
    // filter pane's own content. `the_filter_pane_lists_the_patterns`
    // already caught and fixed this exact trap once.
    assert!(
        after.contains("1[x] inc alpha"),
        "the filter pattern's row is not on screen: {after}"
    );
}

fn add_filters(app: &mut App, patterns: &[&str]) {
    for pattern in patterns {
        key(app, KeyCode::Char('f'));
        key(app, KeyCode::Char('i'));
        typed(app, pattern);
        key(app, KeyCode::Enter);
    }
}

/// #300: the filter pane sizes itself to its longest row, capped at
/// `MAX_FILTER_WIDTH` as the explorer is at `MAX_EXPLORER_WIDTH` — and
/// it no longer shares a column with the explorer, so a long pattern
/// does not widen the explorer at all.
#[test]
fn a_long_filter_pattern_is_capped_and_leaves_the_explorer_alone() {
    let mut app = app_over_file("wide_filter", "alpha\n");
    add_filters(&mut app, &[&"a".repeat(200)]);
    draw(&mut app);

    assert_eq!(app.filter_pane_width(AREA), MAX_FILTER_WIDTH);
    assert_eq!(app.explorer_width(AREA), MIN_AUTO_EXPLORER_WIDTH);
}

/// A row longer than the pane is cut at the pane's edge, not wrapped.
#[test]
fn a_long_filter_pattern_is_cut_at_the_panes_edge() {
    let mut app = app_over_file("wide_filter_cut", "alpha\n");
    add_filters(&mut app, &[&format!("{}zzz", "a".repeat(60))]);
    let text = rendered(&mut app);

    assert!(
        text.contains("1[x] inc aaaa"),
        "the row is not drawn: {text}"
    );
    assert!(!text.contains("zzz"), "the row was not cut: {text}");
}

/// The pane is a column of its own, the full height of the panes, so a
/// large set is on the screen at once.
#[test]
fn the_filter_pane_is_a_full_height_column_right_of_the_view() {
    let mut app = app_over("filter_column", &["a.rs"]);
    draw(&mut app);

    assert_eq!(app.filter_area.y, 0);
    assert_eq!(
        app.filter_area.height,
        AREA.height - 1,
        "all but the status row"
    );
    assert_eq!(app.filter_area.right(), AREA.right());
    assert_eq!(app.filter_area.x, app.view_area.right());
    assert_eq!(app.view_area.x, app.explorer_area.right());
}

/// An empty pane opens at `MIN_AUTO_FILTER_WIDTH`, which holds the whole
/// hint (#44's point, on the new axis).
#[test]
fn an_empty_filter_pane_opens_wide_enough_for_its_hint() {
    let mut app = app_over("empty_filter_width", &["a.rs"]);

    let text = rendered(&mut app);

    assert_eq!(app.filter_area.width, MIN_AUTO_FILTER_WIDTH);
    assert!(text.contains("press f i to add"), "no full hint: {text}");
}

/// On a narrow terminal the filter pane gives way first, then the
/// explorer, and the file view keeps `MIN_FILE_VIEW_WIDTH`. No pane is
/// hidden to make room.
#[test]
fn a_narrow_terminal_shrinks_the_filter_pane_then_the_explorer() {
    let mut app = app_over_file("narrow_order", "alpha\n");
    add_filters(&mut app, &[&"a".repeat(50)]);
    let area = |width| Rect {
        x: 0,
        y: 0,
        width,
        height: 12,
    };

    // 60 columns: 30 for the view, 20 for the explorer, 10 left over.
    let [explorer, view, filters] = app.pane_rects(area(60));
    assert_eq!(
        (explorer.width, view.width, filters.width),
        (MIN_AUTO_EXPLORER_WIDTH, MIN_FILE_VIEW_WIDTH, 10)
    );

    // 45 columns: the filter pane is at its floor, so the explorer gives
    // way next.
    let [explorer, view, filters] = app.pane_rects(area(45));
    assert_eq!(
        (explorer.width, view.width, filters.width),
        (
            45 - MIN_FILE_VIEW_WIDTH - MIN_PANE_WIDTH,
            MIN_FILE_VIEW_WIDTH,
            MIN_PANE_WIDTH
        )
    );
}

fn app_with_two_filters(name: &str) -> App<'static> {
    let mut app = app_over_file(name, "alpha\nbeta\ngamma\n");
    for pattern in ["alpha", "beta"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }
    settle(&mut app);
    app
}

/// Disarm the bounce guard the setup above leaves behind (#48).
///
/// The helper's last keystroke is the `Enter` that commits a pattern, which
/// arms the guard — so without this the *first* `Enter` a test presses is
/// swallowed as a bounce, and every `Enter`-driven test would be asserting
/// against the guard rather than the binding.
///
/// `Esc` because it moves no selection and, with no search set, is a
/// genuine no-op: its arm does nothing unless there was a search to
/// clear.
fn settle(app: &mut App) {
    key(app, KeyCode::Esc);
}

#[test]
fn enter_toggles_the_selected_filter_from_the_pane() {
    let mut app = app_with_two_filters("pane_toggle");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Enter);

    assert!(!app.filters.filters()[0].enabled);
}

/// Toggling must re-evaluate: the view is what the pane is controlling.
#[test]
fn toggling_a_filter_restyles_the_view() {
    let mut app = app_with_two_filters("pane_toggle_view");
    focus_filter_pane(&mut app);
    let before = view_line_styles(&app);

    key(&mut app, KeyCode::Enter);

    assert_ne!(before, view_line_styles(&app), "the view did not follow");
}

#[test]
fn d_deletes_the_selected_filter() {
    let mut app = app_with_two_filters("pane_delete");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('d'));

    assert_eq!(app.filters.len(), 1);
}

/// Important 1: the routing into the filter pane used to pass only
/// `key.code`, discarding the modifiers — so every global binding's
/// "no CONTROL/ALT" guard was silently bypassed once a key reached this
/// pane. `Ctrl-D` is half-page-down in the file view, documented in the
/// README, and exactly the muscle memory a vim user arrives with; here
/// it used to delete the selected filter outright, with no confirmation
/// and no undo.
#[test]
fn ctrl_d_does_not_delete_the_selected_filter() {
    let mut app = app_with_two_filters("pane_ctrl_d");
    focus_filter_pane(&mut app);

    app.handle_event(event::Event::Key(KeyEvent::new(
        KeyCode::Char('d'),
        KeyModifiers::CONTROL,
    )));

    assert_eq!(app.filters.len(), 2, "Ctrl-D deleted a filter");
}

/// `c` reopens the prompt over the selected filter's own pattern. Starting
/// it empty would be no better than `d` then `f i`, which is the retyping
/// this binding exists to remove.
#[test]
fn c_opens_the_prompt_prefilled_with_the_selected_pattern() {
    let mut app = app_with_two_filters("pane_edit_prefill");
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('j'));

    key(&mut app, KeyCode::Char('c'));

    let prompt = app.prompt.as_ref().expect("the prompt should be open");
    assert_eq!(prompt.pattern, "beta");
    assert_eq!(
        prompt.line(),
        "filter: beta",
        "an edit should read like the `i` that would have created it"
    );
}

/// The point of the whole issue: the edited filter keeps its slot, so it
/// keeps its colour and its precedence. Deleting and retyping put the
/// replacement at the end and silently reordered the set.
#[test]
fn committing_an_edit_replaces_the_pattern_in_place() {
    let mut app = app_with_two_filters("pane_edit_commit");
    focus_filter_pane(&mut app);
    let colour = app.filters.filters()[0].style;

    key(&mut app, KeyCode::Char('c'));
    typed(&mut app, "X");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.filters.len(), 2, "the edit added a filter");
    assert_eq!(app.filters.filters()[0].predicate.display(), "alphaX");
    assert_eq!(
        app.filters.filters()[0].style,
        colour,
        "the filter lost its colour, so it moved"
    );
    assert_eq!(app.filters.filters()[1].predicate.display(), "beta");
    assert!(app.prompt.is_none(), "the prompt should have closed");
}

/// The pattern decides which lines match, so an edit owes the document a
/// full `evaluate` — not the `recompute_visible` a mode flip gets away
/// with. Without it the pane shows the new pattern over the old pattern's
/// colouring.
#[test]
fn committing_an_edit_restyles_the_view() {
    let mut app = app_over_file("pane_edit_view", "alpha\nbeta\ngamma\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);
    let before = view_line_styles(&app);

    key(&mut app, KeyCode::Char('c'));
    for _ in 0.."alpha".len() {
        key(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "gamma");
    key(&mut app, KeyCode::Enter);

    assert_ne!(
        view_line_styles(&app),
        before,
        "the view still reflects the old pattern"
    );
}

/// Same contract `f i` has: the prompt stays open over an intact filter, so
/// the typo can be corrected rather than retyped from nothing.
#[test]
fn an_invalid_edit_reports_and_leaves_the_filter_alone() {
    let mut app = app_with_two_filters("pane_edit_invalid");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('c'));
    typed(&mut app, "[");
    key(&mut app, KeyCode::Enter);

    let prompt = app.prompt.as_ref().expect("the prompt should stay open");
    assert_eq!(prompt.line(), INVALID_PATTERN);
    assert_eq!(
        app.filters.filters()[0].predicate.display(),
        "alpha",
        "a rejected pattern overwrote the filter"
    );
}

#[test]
fn escape_abandons_an_edit_and_leaves_the_filter_untouched() {
    let mut app = app_with_two_filters("pane_edit_escape");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('c'));
    typed(&mut app, "X");
    key(&mut app, KeyCode::Esc);

    assert!(app.prompt.is_none());
    assert_eq!(app.filters.filters()[0].predicate.display(), "alpha");
}

/// Backspacing past the start cancels the prompt, as in vim — and a
/// pre-filled prompt is the first place that rule is reachable by
/// *deleting what was already there*. It must abandon the edit, exactly as
/// `Esc` does, rather than commit an empty pattern: an empty regex matches
/// every line, so the filter would silently start colouring the whole file.
#[test]
fn backspacing_an_edit_away_cancels_it_rather_than_emptying_the_filter() {
    let mut app = app_with_two_filters("pane_edit_backspace");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('c'));
    for _ in 0..="alpha".len() {
        key(&mut app, KeyCode::Backspace);
    }

    assert!(app.prompt.is_none(), "the prompt should have cancelled");
    assert_eq!(
        app.filters.filters()[0].predicate.display(),
        "alpha",
        "backspacing out of the prompt emptied the filter"
    );
}

#[test]
fn m_in_the_filter_pane_flips_the_selected_filter_to_context_and_back() {
    let mut app = app_with_two_filters("ctx_toggle");
    key(&mut app, KeyCode::Char('f'));

    key(&mut app, KeyCode::Char('m'));
    assert_eq!(app.filters.filters()[0].sense, filter::Sense::Context);

    key(&mut app, KeyCode::Char('m'));
    assert_eq!(app.filters.filters()[0].sense, filter::Sense::Include);
}

/// An edit changes the pattern and nothing else. The sense in particular
/// has to survive, and it is what the prompt's sigil must report — an
/// excluding filter editing under a `filter:` prompt would read as though
/// committing were about to turn it into an including one.
#[test]
fn editing_an_excluding_filter_keeps_its_sense() {
    let mut app = app_over_file("pane_edit_exclude", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(
        app.prompt.as_ref().expect("prompt open").line(),
        "exclude: alpha"
    );
    typed(&mut app, "X");
    key(&mut app, KeyCode::Enter);

    assert_eq!(app.filters.filters()[0].sense, filter::Sense::Exclude);
    assert_eq!(app.filters.filters()[0].predicate.display(), "alphaX");
}

/// A filter the user had switched off must not come back on just because
/// its pattern was corrected.
#[test]
fn editing_preserves_the_filters_enabled_state() {
    let mut app = app_with_two_filters("pane_edit_enabled");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('c'));
    typed(&mut app, "X");
    key(&mut app, KeyCode::Enter);

    assert!(
        !app.filters.filters()[0].enabled,
        "the edit switched a disabled filter back on"
    );
}

/// The same modifier guard `Ctrl-D` and `Ctrl-Space` get. `Ctrl-C` is the
/// interrupt every terminal user has in their fingers, and it must not
/// open a prompt that then swallows every following key.
#[test]
fn ctrl_c_does_not_open_an_edit_prompt() {
    let mut app = app_with_two_filters("pane_ctrl_c");
    focus_filter_pane(&mut app);

    app.handle_event(event::Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )));

    assert!(app.prompt.is_none(), "Ctrl-C opened the edit prompt");
}

/// With no filters the pane draws its hint and has no selection, so `c` has
/// nothing to address. It must be inert rather than opening a prompt whose
/// `Enter` would silently do nothing.
#[test]
fn c_on_an_empty_set_opens_nothing() {
    let mut app = app_over_file("pane_edit_empty", "alpha\n");
    key(&mut app, KeyCode::Char('f'));

    key(&mut app, KeyCode::Char('c'));

    assert!(app.prompt.is_none());
}

/// Same defect, for the toggle binding.
#[test]
fn ctrl_enter_does_not_toggle_the_selected_filter() {
    let mut app = app_with_two_filters("pane_ctrl_space");
    focus_filter_pane(&mut app);

    app.handle_event(event::Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::CONTROL,
    )));

    assert!(
        app.filters.filters()[0].enabled,
        "Ctrl-Space toggled a filter"
    );
}

/// `/` from the filter pane opens the live-search prompt rather than
/// doing nothing (#120 §7).
#[test]
fn slash_from_the_filter_pane_opens_the_search_prompt() {
    let mut app = app_with_two_filters("filter_pane_slash");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('/'));

    assert!(app.prompt.is_some(), "no search prompt opened");
}

/// Deleting renumbers the filters, so every cached verdict is stale.
///
/// Two filters, deleting the first — this test's original form — cannot
/// actually distinguish a full re-evaluate from a naive patch: on a
/// naive patch (splice the `ActiveFilters` but leave cached verdicts alone),
/// the stale `Verdict::Included(1)` left over from the deleted filter's
/// own line would index *past* the now-length-1 `ActiveFilters` — `None`,
/// not a collision — and the test would fail via `.expect()` panicking,
/// before the colour assertion it advertises ever ran.
///
/// Three filters, deleting the *middle* one ("beta"), produces a genuine
/// in-range collision instead: beta's own filter is gone, so beta must
/// read as unmatched (dim) once re-evaluated — but a naive patch leaves
/// beta's stale `Verdict::Included(1)` in place, and after the splice,
/// array position 1 is no longer beta's old filter — it is gamma's,
/// shifted down from position 2. `style_for` finds a filter there
/// (`.get(1)` succeeds), so the naive patch renders beta in *gamma's*
/// colour: a real, in-range wrong-colour failure, not a panic.
#[test]
fn deleting_a_filter_re_evaluates_rather_than_patching() {
    let mut app = app_over_file("pane_delete_verdicts_mid", "alpha\nbeta\ngamma\n");
    for pattern in ["alpha", "beta", "gamma"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('j')); // move off "alpha" onto "beta", the middle filter

    key(&mut app, KeyCode::Char('d')); // removes "beta"; "gamma" becomes index 1

    let styles = view_line_styles(&app);

    // beta's own filter was just deleted, so beta must be unmatched —
    // plain dim, never coloured as if it still matched something. A
    // stale, un-re-evaluated verdict left over from beta's own
    // (deleted) filter would instead land on gamma's — see the doc
    // comment above — which is the collision this test exists to
    // catch. Checked first, and as a colour comparison rather than an
    // `.expect()`, so that exact failure surfaces directly instead of
    // being masked by a panic from the sanity check below.
    let beta = styles[1].map(|s| s.fg);
    assert_ne!(
        beta,
        Some(app.filters.filters()[1].style.fg),
        "beta is coloured with the wrong (gamma's) filter's style"
    );

    // gamma is unaffected in content and still matches its own filter,
    // now shifted down to index 1.
    let gamma = styles[2].expect("gamma still matches a filter");
    assert_eq!(
        gamma.fg,
        app.filters.filters()[1].style.fg,
        "gamma is not coloured with its own (shifted) filter's style"
    );
}

#[test]
/// Deleting the last filter used to collapse the pane, which forced focus
/// off it. The pane now stays, so focus stays too — moving it would be a
/// jump the user did not ask for, and the pane they are looking at is
/// still on screen and still the one they were working in.
fn deleting_the_last_filter_keeps_the_pane_and_the_focus() {
    let mut app = app_over_file("pane_delete_last", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('d'));
    draw(&mut app);

    assert!(app.filters.is_empty());
    let text = rendered(&mut app);
    assert!(
        text.contains("Filters"),
        "pane vanished with its last filter"
    );
    assert!(text.contains("f i"), "pane lost its empty hint");
    assert!(
        app.focus == Focus::Filters,
        "focus was moved off a pane that is still on screen"
    );
}

/// Deleting the last filter while the pane is zoomed must not leave
/// focus on a pane that is not shown, and the pane must still be drawn.
#[test]
fn deleting_the_last_filter_while_zoomed_keeps_the_pane_on_screen() {
    let mut app = app_over_file("pane_delete_last_zoomed", "alpha\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "alpha");
    key(&mut app, KeyCode::Enter);
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('z'));
    assert_eq!(
        app.panes.shown(),
        PaneSet::only(Focus::Filters),
        "z did not zoom the filter pane"
    );

    key(&mut app, KeyCode::Char('d'));

    assert!(
        app.focus == Focus::Filters,
        "focus left the filter pane, which deleting its last filter no longer collapses"
    );

    // The focused pane is genuinely drawn, not a blank frame.
    let text = rendered(&mut app);
    assert!(
        text.contains("Filters") && text.contains("press f"),
        "the pane the zoom now names is not actually on screen: {text}"
    );
}

#[test]
fn j_and_k_move_the_filter_selection() {
    let mut app = app_with_two_filters("pane_select");
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);

    assert!(app.filters.filters()[0].enabled, "toggled the wrong filter");
    assert!(
        !app.filters.filters()[1].enabled,
        "j did not move the selection"
    );

    // `k` back up, then toggle again: if `k` did not move the selection
    // back to filter 0, this toggle would re-hit filter 1 instead — and
    // since filter 1 is already disabled, that would re-enable it rather
    // than disabling filter 0, so the assertions below would fail.
    key(&mut app, KeyCode::Char('k'));
    key(&mut app, KeyCode::Enter);

    assert!(
        !app.filters.filters()[0].enabled,
        "k did not move the selection back"
    );
    assert!(
        !app.filters.filters()[1].enabled,
        "the wrong filter was toggled after k"
    );
}

/// Task 6 review (RULING 27): before the table, a global arm guarded
/// only on `focus != Focus::Explorer` ran ahead of both scopes' own
/// `HitNext`/`HitPrev` rows and called `step_interesting` itself, so
/// neither arm had ever executed in production. Pins that the file
/// view's own arm — `Scope::View`'s bare `n`/`N` rows in `keymap.rs` —
/// is what a real `n`/`N` in the view now reaches.
#[test]
fn n_and_capital_n_resolve_through_the_view_scope() {
    let mut app = app_over_file("n_view_scope", "hit a\nplain\nhit b\nplain\nhit c\n");
    focus_file_view(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();
    assert_eq!(
        cursor_source(&app),
        2,
        "sanity: cursor starts on the middle hit"
    );

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(
        cursor_source(&app),
        4,
        "n did not resolve through Scope::View to HitNext"
    );

    key(&mut app, KeyCode::Char('N'));
    assert_eq!(
        cursor_source(&app),
        2,
        "N did not resolve through Scope::View to HitPrev"
    );
}

/// Same pin, for `Scope::Filters`: the filter pane has no "next" of its
/// own, so `n`/`N` there must reach the same `HitNext`/`HitPrev` action —
/// resolved inside `handle_filter_key` rather than at the view's own
/// call site — and make the same `step_interesting` call.
#[test]
fn n_and_capital_n_resolve_through_the_filters_scope() {
    let mut app = app_over_file("n_filters_scope", "hit a\nplain\nhit b\nplain\nhit c\n");
    focus_file_view(&mut app);
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    app.filters.add("hit").expect("valid pattern");
    app.refresh_view();
    assert_eq!(
        cursor_source(&app),
        2,
        "sanity: cursor starts on the middle hit"
    );
    focus_filter_pane(&mut app);

    key(&mut app, KeyCode::Char('n'));
    assert_eq!(
        cursor_source(&app),
        4,
        "n did not resolve through Scope::Filters to HitNext"
    );

    key(&mut app, KeyCode::Char('N'));
    assert_eq!(
        cursor_source(&app),
        2,
        "N did not resolve through Scope::Filters to HitPrev"
    );
}
