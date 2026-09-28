use super::*;

// ---- editing inside the prompt (#206) --------------------------------

/// The issue's own example. A filter `load_file`, changed with `c`:
/// left over `_file`, backspace over `load`, type `print`, Enter — and
/// the result is `print_file`, not `load_fileprint` and not a prompt
/// that had to be emptied first.
#[test]
fn the_prompt_edits_at_the_cursor_not_only_at_the_end() {
    let mut app = app_over_file("prompt_cursor_edit", "print_file\nload_file\n");
    app.add_filter("load_file").unwrap();
    focus_filter_pane(&mut app);
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(prompt(&app).pattern, "load_file", "sanity: prefilled");

    for _ in 0..5 {
        key(&mut app, KeyCode::Left);
    }
    for _ in 0..4 {
        key(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "print");
    assert_eq!(prompt(&app).pattern, "print_file");

    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_none(), "committed");
    assert_eq!(app.filters.filters()[0].predicate.display(), "print_file");
}

#[test]
fn typing_inserts_at_the_cursor() {
    let mut app = app_over_file("prompt_insert", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ac");
    key(&mut app, KeyCode::Left);

    typed(&mut app, "b");

    assert_eq!(prompt(&app).pattern, "abc");
    assert_eq!(prompt(&app).cursor, 2, "the cursor follows the insertion");
}

#[test]
fn left_stops_at_the_start_and_right_at_the_end() {
    let mut app = app_over_file("prompt_left_right", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ab");
    assert_eq!(
        prompt(&app).cursor,
        2,
        "sanity: a fresh prompt types at its end"
    );

    for _ in 0..3 {
        key(&mut app, KeyCode::Left);
    }
    assert_eq!(prompt(&app).cursor, 0);
    for _ in 0..3 {
        key(&mut app, KeyCode::Right);
    }
    assert_eq!(prompt(&app).cursor, 2);
}

/// `Home`/`End` and the readline pair both jump; vim's command line
/// takes the same keys.
#[test]
fn home_end_and_the_ctrl_pair_jump_to_the_pattern_s_ends() {
    let mut app = app_over_file("prompt_home_end", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "abc");

    key(&mut app, KeyCode::Home);
    assert_eq!(prompt(&app).cursor, 0, "Home");
    key(&mut app, KeyCode::End);
    assert_eq!(prompt(&app).cursor, 3, "End");
    ctrl(&mut app, KeyCode::Char('a'));
    assert_eq!(prompt(&app).cursor, 0, "Ctrl-a");
    ctrl(&mut app, KeyCode::Char('e'));
    assert_eq!(prompt(&app).cursor, 3, "Ctrl-e");
}

#[test]
fn delete_removes_the_character_under_the_cursor() {
    let mut app = app_over_file("prompt_delete", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "abc");
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Left);

    key(&mut app, KeyCode::Delete);
    assert_eq!(prompt(&app).pattern, "ac");
    assert_eq!(prompt(&app).cursor, 1, "the cursor stays put");

    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Delete);
    assert_eq!(
        prompt(&app).pattern,
        "ac",
        "nothing under the cursor at the end"
    );
}

/// Backspace on an empty pattern still cancels — but at the *start* of
/// a pattern with text after the cursor there is nothing to delete and
/// nothing to cancel: the text the user is keeping is still there.
#[test]
fn backspace_at_the_start_of_a_pattern_deletes_nothing_and_keeps_the_prompt() {
    let mut app = app_over_file("prompt_backspace_start", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ab");
    key(&mut app, KeyCode::Home);

    key(&mut app, KeyCode::Backspace);

    assert_eq!(prompt(&app).pattern, "ab");
    assert!(app.prompt.is_some(), "the prompt was cancelled");
}

/// vim's command-line `Ctrl-w`: the word before the cursor goes, with any
/// blanks between it and the cursor; a run of punctuation counts as a
/// word of its own. The same word definition as `*`.
#[test]
fn ctrl_w_deletes_the_word_before_the_cursor() {
    let mut app = app_over_file("prompt_ctrl_w", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "foo::bar baz");

    ctrl(&mut app, KeyCode::Char('w'));
    assert_eq!(prompt(&app).pattern, "foo::bar ");
    ctrl(&mut app, KeyCode::Char('w'));
    assert_eq!(
        prompt(&app).pattern,
        "foo::",
        "the blank went with the word"
    );
    ctrl(&mut app, KeyCode::Char('w'));
    assert_eq!(
        prompt(&app).pattern,
        "foo",
        "punctuation is a word of its own"
    );
    ctrl(&mut app, KeyCode::Char('w'));
    assert_eq!(prompt(&app).pattern, "");
    assert!(app.prompt.is_some(), "emptying by word does not cancel");
}

#[test]
fn ctrl_u_deletes_everything_before_the_cursor() {
    let mut app = app_over_file("prompt_ctrl_u", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "abcd");
    key(&mut app, KeyCode::Left);

    ctrl(&mut app, KeyCode::Char('u'));

    assert_eq!(prompt(&app).pattern, "d");
    assert_eq!(prompt(&app).cursor, 0);
}

/// The terminal cursor is hidden, so the prompt row draws its own: the
/// cell under the cursor in reversed video, one blank past the text when
/// the cursor is at the end.
#[test]
fn the_prompt_row_shows_the_cursor_as_a_reversed_cell() {
    let mut app = app_over_file("prompt_cursor_cell", "x\n");
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ab");
    let y = AREA.height - 1;
    let reversed = |app: &mut App, x: u16| {
        let mut buf = Buffer::empty(AREA);
        app.render(AREA, &mut buf);
        buf[(x, y)]
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)
    };

    // `/ab` with the cursor at the end: columns 0..3 are the text, the
    // cursor cell is column 3.
    assert!(reversed(&mut app, 3), "no cursor cell after the text");
    assert!(
        !reversed(&mut app, 2),
        "the last character is not the cursor"
    );

    key(&mut app, KeyCode::Left);
    assert!(reversed(&mut app, 2), "the cursor did not move on screen");
    assert!(!reversed(&mut app, 3));
}

#[test]
fn slash_opens_a_search_prompt() {
    let mut app = app_over("prompt", &["a.rs"]);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "foo");

    assert_eq!(prompt_line(&mut app), "/foo");
}

/// The prompt swallows keys that are otherwise app-wide commands.
#[test]
fn q_while_searching_is_typed_not_quit() {
    let mut app = app_over("prompt_q", &["a.rs"]);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "q");

    assert!(app.is_running(), "q closed the app while typing a pattern");
    assert_eq!(prompt_line(&mut app), "/q");
}

#[test]
fn backspace_deletes_then_cancels() {
    let mut app = app_over("prompt_bs", &["a.rs"]);
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "ab");

    key(&mut app, KeyCode::Backspace);
    assert_eq!(prompt_line(&mut app), "/a");

    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);

    assert!(app.prompt.is_none(), "backspace on empty did not cancel");
}

#[test]
fn esc_cancels_the_prompt() {
    let mut app = app_over("prompt_esc", &["a.rs"]);
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "foo");

    key(&mut app, KeyCode::Esc);

    assert!(app.prompt.is_none());
}

#[test]
fn committing_a_search_closes_the_prompt_and_moves_the_selection() {
    let mut app = app_over("prompt_commit", &["alpha.rs", "gamma.rs"]);
    draw(&mut app);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "gamma");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_none(), "prompt stayed open");
    let explorer = &app.explorer;
    assert_eq!(
        explorer.entries()[explorer.selected().unwrap()].name,
        "gamma.rs"
    );
}

#[test]
fn an_invalid_pattern_keeps_the_prompt_open_with_an_error() {
    let mut app = app_over("prompt_bad", &["a.rs"]);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "[");
    key(&mut app, KeyCode::Enter);

    assert!(app.prompt.is_some(), "prompt closed on an invalid pattern");
    assert!(
        prompt_line(&mut app).contains("E486"),
        "no error shown: {}",
        prompt_line(&mut app)
    );
}

/// Searching in the explorer pane jumps to the file *and* previews it.
#[test]
fn a_explorer_search_previews_the_matched_file() {
    let mut app = app_over("prompt_preview", &["alpha.rs", "gamma.rs"]);
    fs::write(
        fixture_dir_path("prompt_preview").join("gamma.rs"),
        "GAMMA MARKER\n",
    )
    .unwrap();
    draw(&mut app);

    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "gamma");
    key(&mut app, KeyCode::Enter);

    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);
    let text: String = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(
        text.contains("GAMMA MARKER"),
        "matched file was not previewed"
    );
}

/// From the explorer, `/` searches file names. A pattern that matches no
/// name used to close the prompt in silence, which looks the same as `Esc`.
/// Say so, the way `n` reports a dead end (#243).
#[test]
fn a_explorer_search_with_no_matching_filename_says_so() {
    let mut app = app_over("explorer_search_dead_end", &["alpha.log", "beta.log"]);

    app.run_search("ERROR").expect("valid pattern");

    assert_eq!(
        app.status_message.as_ref().map(|m| m.text.as_str()),
        Some("no filenames match \"ERROR\"")
    );
}

/// The report is for the dead end only. A name that does match stays quiet.
#[test]
fn a_explorer_search_that_matches_reports_nothing() {
    let mut app = app_over("explorer_search_hit", &["alpha.log", "beta.log"]);

    app.run_search("beta").expect("valid pattern");

    assert!(app.status_message.is_none());
}

/// The prompt only takes a row while it is open.
#[test]
fn the_prompt_row_appears_only_while_searching() {
    let mut app = app_over("prompt_layout", &["a.rs"]);
    let idle = prompt_line(&mut app);

    key(&mut app, KeyCode::Char('/'));

    assert_ne!(idle, prompt_line(&mut app));
}
