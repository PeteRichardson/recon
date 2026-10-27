use super::*;

// ---- the command palette opens and closes (#520) ---------------------

/// `:` opens the palette on the bottom row from each of the three panes,
/// and Esc closes it with the panes as they were.
#[test]
fn colon_opens_the_palette_from_every_pane_and_esc_changes_nothing() {
    let mut app = app_over_file("palette_open", "a\nb\nc\n");
    for target in [Focus::Explorer, Focus::View, Focus::Filters] {
        match target {
            Focus::Explorer => key(&mut app, KeyCode::Char('e')),
            Focus::View => key(&mut app, KeyCode::Char('t')),
            Focus::Filters => key(&mut app, KeyCode::Char('f')),
        };
        assert_eq!(app.focus, target, "sanity: the pane has focus");
        let (dir, cursor) = (app.explorer.dir().to_path_buf(), cursor_source(&app));

        key(&mut app, KeyCode::Char(':'));
        assert_eq!(prompt(&app).kind, PromptKind::Command);
        assert_eq!(prompt_line(&mut app), ":");

        typed(&mut app, "e /");
        key(&mut app, KeyCode::Esc);
        assert!(app.prompt.is_none(), "Esc did not close the palette");
        assert_eq!(app.focus, target, "Esc moved the focus");
        assert_eq!(app.explorer.dir(), dir, "Esc changed the folder");
        assert_eq!(cursor_source(&app), cursor, "Esc moved the cursor");
    }
}

/// In a `/` prompt, `:` is a character of the pattern; in the set picker
/// and the filter editor it opens nothing either.
#[test]
fn colon_does_not_open_the_palette_in_a_prompt_or_a_modal_screen() {
    let mut app = app_over_file("palette_modal", "a:b\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char(':'));
    assert_eq!(prompt(&app).kind, PromptKind::Search);
    assert_eq!(prompt(&app).pattern, ":");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char('L'));
    assert!(app.set_picker.is_some(), "sanity: the set picker is open");
    key(&mut app, KeyCode::Char(':'));
    assert!(
        app.prompt.is_none(),
        "`:` opened the palette over the set picker"
    );
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('I'));
    assert!(
        app.filter_editor.is_some(),
        "sanity: the filter editor is open"
    );
    key(&mut app, KeyCode::Char(':'));
    assert!(
        app.prompt.is_none(),
        "`:` opened the palette over the filter editor"
    );
}

/// The palette is a keymap action like any other: `:` is only its
/// default key.
#[test]
fn the_palette_can_be_bound_to_another_key() {
    let mut app = app_over_file("palette_rebind", "a\n");
    rebind(&mut app, &[("global.palette", &["F2"])]);
    key(&mut app, KeyCode::Char(':'));
    assert!(app.prompt.is_none(), "`:` still opens the palette");
    key(&mut app, KeyCode::F(2));
    assert_eq!(prompt(&app).kind, PromptKind::Command);
}

// ---- `e` and its alias `cd` (#520) ------------------------------------

/// Run `line` in the palette, as the user types it and pushes Enter.
pub(super) fn run(app: &mut App, line: &str) {
    key(app, KeyCode::Char(':'));
    typed(app, line);
    key(app, KeyCode::Enter);
}

/// The path of the file the file view shows.
fn open_path(app: &App) -> std::path::PathBuf {
    app.view.filename().to_path_buf()
}

/// `e` with a directory goes there, selects the first entry after `..`,
/// and gives the explorer focus — as a step into it with the keys would.
#[test]
fn e_with_a_directory_goes_there_and_focuses_the_explorer() {
    let (mut app, dir) = app_over_nested("palette_e_dir");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('E'));
    assert!(!app.panes.is_shown(Focus::Explorer), "sanity: hidden");

    run(
        &mut app,
        &format!(
            "e {}",
            crate::path::lexical_absolute(&dir.join("sub")).display()
        ),
    );
    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
    assert_eq!(selected_name(&app), "inner_a.log");
    assert_eq!(app.focus, Focus::Explorer);
    assert!(
        app.panes.is_shown(Focus::Explorer),
        "the explorer is still hidden"
    );
}

/// `e` with a file goes to its folder, selects it and opens it in the
/// file view, as the path on the shell command line does.
#[test]
fn e_with_a_file_goes_to_its_folder_selects_it_and_opens_it() {
    let (mut app, dir) = app_over_nested("palette_e_file");
    let file = dir.join("sub/inner_b.log");
    run(
        &mut app,
        &format!("e {}", crate::path::lexical_absolute(&file).display()),
    );
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
    assert_eq!(selected_name(&app), "inner_b.log");
    assert_eq!(open_path(&app), crate::path::lexical_absolute(&file));
    assert_eq!(app.focus, Focus::Explorer);
}

/// `cd` is the same command as `e`.
#[test]
fn cd_is_an_alias_of_e() {
    let (mut app, dir) = app_over_nested("palette_cd");
    run(
        &mut app,
        &format!(
            "cd {}",
            crate::path::lexical_absolute(&dir.join("sub/inner_b.log")).display()
        ),
    );
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
    assert_eq!(selected_name(&app), "inner_b.log");
}

/// A relative path and `..` resolve against the explorer's folder, not
/// against the working directory of recon.
#[test]
fn a_relative_path_resolves_against_the_explorers_folder() {
    let (mut app, dir) = app_over_nested("palette_relative");
    run(&mut app, "e sub");
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
    run(&mut app, "e ../z.log");
    assert_eq!(app.explorer.dir(), crate::path::lexical_absolute(&dir));
    assert_eq!(selected_name(&app), "z.log");
}

/// `~` is the home folder, and `e` alone goes there too.
#[test]
fn tilde_and_no_argument_go_to_the_home_folder() {
    let (mut app, dir) = app_over_nested("palette_home");
    app.home = Some(
        crate::path::lexical_absolute(&dir.join("sub"))
            .display()
            .to_string(),
    );
    run(&mut app, "e ~/../z.log");
    assert_eq!(selected_name(&app), "z.log");
    run(&mut app, "cd");
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
    run(&mut app, "e ~");
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
}

/// `e -` goes back to the folder before the last `e`.
#[test]
fn dash_goes_back_to_the_last_folder() {
    let (mut app, dir) = app_over_nested("palette_dash");
    let start = app.explorer.dir().to_path_buf();
    run(&mut app, "e sub");
    run(&mut app, "e -");
    assert_eq!(app.explorer.dir(), start);
    run(&mut app, "e -");
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
}

/// A path that does not exist is an error: the palette stays open with
/// the line, and nothing moves.
#[test]
fn a_missing_path_keeps_the_palette_open_with_an_error() {
    let (mut app, _) = app_over_nested("palette_missing");
    let start = app.explorer.dir().to_path_buf();
    run(&mut app, "e nowhere");
    assert_eq!(prompt(&app).kind, PromptKind::Command, "the palette closed");
    assert_eq!(prompt(&app).pattern, "e nowhere", "the line is gone");
    assert!(
        prompt(&app)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("nowhere")),
        "{:?}",
        prompt(&app).error
    );
    assert_eq!(app.explorer.dir(), start);

    // The next key edits the line again.
    key(&mut app, KeyCode::Backspace);
    assert_eq!(prompt(&app).pattern, "e nowher");
    assert!(prompt(&app).error.is_none());
}

/// A word that names no command is an error too.
#[test]
fn an_unknown_command_keeps_the_palette_open_with_an_error() {
    let (mut app, _) = app_over_nested("palette_unknown");
    run(&mut app, "frobnicate now");
    assert_eq!(prompt(&app).pattern, "frobnicate now");
    assert!(
        prompt(&app)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("frobnicate")),
        "{:?}",
        prompt(&app).error
    );
}

/// `e` does not change the working directory of recon.
#[test]
fn e_leaves_the_working_directory_alone() {
    let (mut app, dir) = app_over_nested("palette_cwd");
    let cwd = std::env::current_dir().expect("cwd");
    run(
        &mut app,
        &format!(
            "e {}",
            crate::path::lexical_absolute(&dir.join("sub")).display()
        ),
    );
    assert_eq!(std::env::current_dir().expect("cwd"), cwd);
}

// ---- `explorer.set` and `explorer.focus`, the steps of `e` (#556) -----

/// `explorer.set` sets the explorer's folder as `e` does, but the focus
/// stays where it was, and a hidden explorer stays hidden.
#[test]
fn explorer_set_goes_there_and_leaves_the_focus_alone() {
    let (mut app, dir) = app_over_nested("palette_explorer_set");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('E'));
    let file = crate::path::lexical_absolute(&dir.join("sub/inner_b.log"));

    run(&mut app, &format!("explorer.set {}", file.display()));
    assert!(
        app.prompt.is_none(),
        "{:?}",
        app.prompt.as_ref().map(|p| &p.error)
    );
    assert_eq!(
        app.explorer.dir(),
        crate::path::lexical_absolute(&dir.join("sub"))
    );
    assert_eq!(selected_name(&app), "inner_b.log");
    assert_eq!(open_path(&app), file);
    assert_eq!(app.focus, Focus::View);
    assert!(
        !app.panes.is_shown(Focus::Explorer),
        "the explorer was shown"
    );
}

/// `explorer.focus` shows the explorer and gives it the focus, from the
/// palette and from a key bound to it in `[keymap]`.
#[test]
fn explorer_focus_shows_and_focuses_the_explorer() {
    let mut app = app_over_file("palette_explorer_focus", "a\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('E'));
    run(&mut app, "explorer.focus");
    assert!(
        app.prompt.is_none(),
        "{:?}",
        app.prompt.as_ref().map(|p| &p.error)
    );
    assert_eq!(app.focus, Focus::Explorer);
    assert!(
        app.panes.is_shown(Focus::Explorer),
        "the explorer is hidden"
    );

    rebind(&mut app, &[("explorer.focus", &["F3"])]);
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('E'));
    key(&mut app, KeyCode::F(3));
    assert_eq!(app.focus, Focus::Explorer);
    assert!(
        app.panes.is_shown(Focus::Explorer),
        "the explorer is hidden"
    );
}

// ---- `goto <n>` and `:<n>` (#520) ------------------------------------

/// `goto 3` and `:3` go to line 3 of the file — the number the line
/// numbers show — and give the file view focus.
#[test]
fn goto_and_a_bare_number_go_to_that_line_of_the_file() {
    let mut app = app_over_file("palette_goto", &numbered_lines(50));
    run(&mut app, "goto 30");
    assert!(
        app.prompt.is_none(),
        "{:?}",
        app.prompt.as_ref().map(|p| &p.error)
    );
    assert_eq!(cursor_source(&app), 29);
    assert_eq!(app.focus, Focus::View);

    key(&mut app, KeyCode::Char('e'));
    run(&mut app, "3");
    assert_eq!(cursor_source(&app), 2);
    assert_eq!(app.focus, Focus::View);
}

/// The number counts the lines of the file, not the visible lines: with a
/// filter that hides line 4, `:4` goes to the next visible line after it,
/// and the status row says so.
#[test]
fn goto_a_hidden_line_lands_on_the_next_visible_line_and_says_so() {
    let mut app = app_over_file(
        "palette_goto_hidden",
        "keep 1\nkeep 2\ndrop 3\ndrop 4\nkeep 5\n",
    );
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('x'));
    typed(&mut app, "drop");
    key(&mut app, KeyCode::Enter);

    run(&mut app, "4");
    assert_eq!(cursor_source(&app), 4, "not on the next visible line");
    assert!(
        status(&app).is_some_and(|text| text.contains("line 4") && text.contains("line 5")),
        "{:?}",
        status(&app)
    );
}

/// A number past the end goes to the last line, as in vim; a word that is
/// not a number is an error.
#[test]
fn goto_past_the_end_goes_to_the_last_line_and_a_bad_number_is_an_error() {
    let mut app = app_over_file("palette_goto_end", &numbered_lines(5));
    run(&mut app, "999");
    assert_eq!(cursor_source(&app), 4);

    run(&mut app, "goto ten");
    assert!(
        prompt(&app)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("ten")),
        "{:?}",
        prompt(&app).error
    );
}

// ---- every keymap action, by its keymap name (#520, ADR 0011) ---------

/// A global action runs by the name a `[keymap]` line uses.
#[test]
fn a_global_action_runs_by_its_keymap_name() {
    let mut app = app_over_file("palette_global", "a\n");
    run(&mut app, "global.hide.explorer");
    assert!(
        app.prompt.is_none(),
        "{:?}",
        app.prompt.as_ref().map(|p| &p.error)
    );
    assert!(!app.panes.is_shown(Focus::Explorer));
}

/// A pane's action works on its own pane, whichever pane has focus, and
/// that pane shows and takes the focus.
#[test]
fn a_pane_action_works_on_its_own_pane_and_focuses_it() {
    let (mut app, _) = app_over_nested("palette_pane_action");
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(selected_name(&app), "sub", "sanity: the first entry");
    run(&mut app, "explorer.down");
    assert_eq!(app.focus, Focus::Explorer);
    assert_eq!(selected_name(&app), "z.log");

    let mut app = app_over_file("palette_view_action", &numbered_lines(20));
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('T'));
    run(&mut app, "view.goto.end");
    assert_eq!(app.focus, Focus::View);
    assert!(
        app.panes.is_shown(Focus::View),
        "the file view is still hidden"
    );
    assert_eq!(cursor_source(&app), 19);
}

/// An action that opens a prompt leaves that prompt open: the palette
/// closes first.
#[test]
fn an_action_that_opens_a_prompt_leaves_it_open() {
    let mut app = app_over_file("palette_opens_prompt", "a\n");
    run(&mut app, "global.search");
    assert_eq!(prompt(&app).kind, PromptKind::Search);
    key(&mut app, KeyCode::Esc);

    run(&mut app, "filters.include");
    assert_eq!(prompt(&app).kind, PromptKind::Filter);
    assert_eq!(app.focus, Focus::Filters);
}

/// An action still runs from the palette when the user took its key away:
/// the palette is then the only way to run it.
#[test]
fn an_action_with_no_key_still_runs() {
    let mut app = app_over_file("palette_unbound", "a\n");
    rebind(&mut app, &[("global.hide.explorer", &[])]);
    run(&mut app, "global.hide.explorer");
    assert!(!app.panes.is_shown(Focus::Explorer));
}

/// `global.filters.toggle` needs the number a digit key gives it.
#[test]
fn the_filter_toggle_takes_its_number_as_the_argument() {
    let mut app = app_over_file("palette_toggle", "a\nb\n");
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('i'));
    typed(&mut app, "a");
    key(&mut app, KeyCode::Enter);
    let enabled = |app: &App| {
        let index = crate::widgets::filterlist::numbered(&app.filters)[0];
        app.filters.filters()[index].enabled
    };
    assert!(enabled(&app), "sanity: a new filter is enabled");

    run(&mut app, "global.filters.toggle 1");
    assert!(!enabled(&app), "filter 1 is still enabled");

    run(&mut app, "global.filters.toggle");
    assert!(prompt(&app).error.is_some(), "no error without a number");
}

/// The actions of a modal screen are not commands: the palette never
/// opens over their screen, so they have nothing to act on.
#[test]
fn an_action_of_a_modal_screen_is_not_a_command() {
    let mut app = app_over_file("palette_modal_action", "a\n");
    for name in ["filtereditor.mark.match", "prompt.cancel", "sets.toggle"] {
        run(&mut app, name);
        assert!(
            prompt(&app)
                .error
                .as_deref()
                .is_some_and(|e| e.contains(name)),
            "{name}: {:?}",
            prompt(&app).error
        );
        key(&mut app, KeyCode::Esc);
    }
}

/// `q` is vim's alias of `global.quit`.
#[test]
fn q_quits_as_global_quit_does() {
    let mut app = app_over_file("palette_q", "a\n");
    run(&mut app, "q");
    assert!(
        matches!(app.state, AppState::Quit { .. }),
        "{:?}",
        app.state
    );
}

// ---- the list above the line (#520) -----------------------------------

/// The rows of the screen the palette's list takes, top to bottom.
fn list_rows(app: &mut App) -> Vec<String> {
    let screen = screen(app);
    let rows: Vec<&str> = screen.lines().collect();
    let height = app.palette_list_height(HELP_AREA.height - 1);
    rows[rows.len() - 1 - height..rows.len() - 1]
        .iter()
        .map(|row| row.trim_end().to_string())
        .collect()
}

/// An empty palette lists the commands, each with its argument, its key
/// and its line of help; an alias is on its command's row.
#[test]
fn an_empty_palette_lists_the_commands_with_keys_and_help() {
    let mut app = app_over_file("palette_list", "a\n");
    key(&mut app, KeyCode::Char(':'));
    let rows = list_rows(&mut app);
    let first = rows
        .iter()
        .find(|row| row.contains("e, cd"))
        .expect("no e row");
    assert!(first.contains("<path>"), "{first}");
    assert!(first.contains("Go to a folder"), "{first}");
    assert!(
        rows.iter()
            .any(|row| row.contains("goto") && row.contains("<n>")),
        "{rows:#?}"
    );
}

/// Typing a word puts the best match at the top of the list, with the
/// key that also runs it.
#[test]
fn typing_narrows_the_list_and_the_row_shows_the_key() {
    let mut app = app_over_file("palette_narrow", "a\n");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "gfol");
    let rows = list_rows(&mut app);
    let top = rows
        .iter()
        .find(|row| !row.trim().is_empty())
        .expect("a row");
    assert!(top.contains("global.follow"), "{rows:#?}");
    assert!(top.contains("Ctrl-t"), "{top}");
}

/// An action whose key the user took away shows no key, but is listed.
#[test]
fn an_action_with_no_key_is_listed_with_an_empty_key() {
    let mut app = app_over_file("palette_list_unbound", "a\n");
    rebind(&mut app, &[("global.follow", &[])]);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "global.follow");
    let rows = list_rows(&mut app);
    let row = rows
        .iter()
        .find(|row| row.contains("global.follow"))
        .expect("listed");
    assert!(!row.contains("Ctrl-t"), "{row}");
}

/// No list while no palette is open, and none over a `/` prompt.
#[test]
fn only_the_palette_has_a_list() {
    let mut app = app_over_file("palette_no_list", "a\n");
    assert_eq!(app.palette_list_height(HELP_AREA.height - 1), 0);
    key(&mut app, KeyCode::Char('/'));
    assert_eq!(app.palette_list_height(HELP_AREA.height - 1), 0);
}

// ---- Tab and Shift-Tab (#520) -----------------------------------------

/// The main names of the rows the palette lists for `word`, best first.
fn listed(word: &str) -> Vec<&'static str> {
    let catalog = crate::commands::catalog();
    crate::commands::matches(catalog, word)
        .into_iter()
        .map(|index| catalog[index].names[0].as_ref())
        .collect()
}

/// Tab writes the chosen row's name into the line — with a blank after a
/// command that takes an argument — so a word from the help is enough to
/// find a command: `folder`, Tab, and the path.
#[test]
fn tab_writes_the_chosen_rows_name_into_the_line() {
    let mut app = app_over_file("palette_tab_apropos", "a\n");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "folder");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "e ");
    let rows = list_rows(&mut app);
    assert!(
        rows[0].contains("e, cd"),
        "the chosen row left the list: {rows:#?}"
    );
}

/// Each Tab chooses the next row and Shift-Tab the one before, through the
/// rows the typed word matched.
#[test]
fn tab_and_shift_tab_step_through_the_rows() {
    let mut app = app_over_file("palette_tab_cycle", "a\n");
    let rows = listed("gl");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "gl");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, rows[0]);
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, rows[1]);
    key(&mut app, KeyCode::BackTab);
    assert_eq!(prompt(&app).pattern, rows[0]);
}

/// A typed character ends the choice: the line is the user's again, and
/// the list matches what it now says.
#[test]
fn typing_after_tab_ends_the_choice() {
    let mut app = app_over_file("palette_tab_type", "a\n");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "gfol");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "global.follow");
    typed(&mut app, "x");
    assert_eq!(prompt(&app).pattern, "global.followx");
    assert!(prompt(&app).completion.is_none());
}

/// On a path, Tab completes the name of an entry of the folder: all of it
/// when one entry fits, what the entries share when more than one does,
/// and then each entry in turn.
#[test]
fn tab_completes_a_path_against_the_explorers_folder() {
    let (mut app, _) = app_over_nested("palette_tab_path");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "e su");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "e sub/");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "e sub/inner_");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "e sub/inner_a.log");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "e sub/inner_b.log");
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        prompt(&app).pattern,
        "e sub/inner_a.log",
        "the choice wraps"
    );
}

/// `cd` completes as `e` does, and `~/` completes in the home folder.
#[test]
fn tab_completes_after_cd_and_in_the_home_folder() {
    let (mut app, dir) = app_over_nested("palette_tab_home");
    app.home = Some(
        crate::path::lexical_absolute(&dir.join("sub"))
            .display()
            .to_string(),
    );
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "cd ~/inner_b");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "cd ~/inner_b.log");
}

/// Tab in a `/` prompt does nothing: a pattern is not a command.
#[test]
fn tab_does_nothing_in_a_search_prompt() {
    let mut app = app_over_file("palette_tab_search", "a\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "a");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "a");
    assert_eq!(app.focus, Focus::View, "Tab moved the focus");
}

// ---- history (#520) ---------------------------------------------------

/// Up recalls the command lines that ran, newest first, and Down goes
/// back towards the newest and then to an empty line.
#[test]
fn up_and_down_recall_the_lines_that_ran() {
    let mut app = app_over_file("palette_history", &numbered_lines(9));
    run(&mut app, "goto 3");
    run(&mut app, "5");
    key(&mut app, KeyCode::Char(':'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt(&app).pattern, "5");
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt(&app).pattern, "goto 3");
    key(&mut app, KeyCode::Down);
    assert_eq!(prompt(&app).pattern, "5");
    key(&mut app, KeyCode::Down);
    assert_eq!(prompt(&app).pattern, "");
}

/// Only a line that ran goes into the history: not one that failed, and
/// not one Esc closed.
#[test]
fn only_a_line_that_ran_is_kept() {
    let mut app = app_over_file("palette_history_failed", "a\n");
    run(&mut app, "frobnicate");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "goto 1");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char(':'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt(&app).pattern, "", "a line that did not run was kept");
}

/// The palette's history is its own: a search pattern is not offered in
/// it, and a command line is not offered in a `/` prompt.
#[test]
fn the_palette_and_the_search_keep_separate_histories() {
    let mut app = app_over_file("palette_history_apart", "a\nb\n");
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Char('/'));
    typed(&mut app, "b");
    key(&mut app, KeyCode::Enter);
    run(&mut app, "goto 1");

    key(&mut app, KeyCode::Char(':'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt(&app).pattern, "goto 1");
    key(&mut app, KeyCode::Up);
    assert_eq!(
        prompt(&app).pattern,
        "goto 1",
        "the search pattern is offered"
    );
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Up);
    assert_eq!(prompt(&app).pattern, "b");
}

// ---- `set` and `unlist` (#525) ----------------------------------------

/// The index of the set called `name`.
fn set_named(app: &App, name: &str) -> usize {
    app.filters
        .sets()
        .iter()
        .position(|meta| meta.name == name)
        .expect("known set")
}

/// `set` on an unlisted set lists it first, then enables it with its
/// `default` profile, as `--set` does: its lines match at once.
#[test]
fn set_lists_and_enables_an_unlisted_set() {
    let mut app = app_with_three_sets("palette_set");
    let c = set_named(&app, "c");
    app.filters.set_listed(c, false);
    app.refresh_view();
    assert_eq!(included(&app), 3, "sanity: alpha, beta, scratch");

    run(&mut app, "set c:gamma");
    assert!(app.prompt.is_none(), "the palette is still open");
    assert!(app.filters.sets()[c].listed);
    assert!(app.filters.sets()[c].enabled);
    assert_eq!(flags_of(&app, c), [true]);
    assert_eq!(included(&app), 4, "gamma does not match");
}

/// `list` lists an unlisted set and leaves it disabled, as `--list` does
/// (#545): its header comes back and its lines do not match yet. A listed
/// set does not change: `list` does not turn off a set that is on.
#[test]
fn list_lists_an_unlisted_set_disabled() {
    let mut app = app_with_three_sets("palette_list_set");
    let c = set_named(&app, "c");
    app.filters.set_listed(c, false);
    app.refresh_view();
    let before = included(&app);

    run(&mut app, "list c");
    assert!(app.prompt.is_none(), "the palette is still open");
    assert!(app.filters.sets()[c].listed);
    assert!(!app.filters.sets()[c].enabled);
    assert!(
        widgets::filterlist::rows(&app.filters).contains(&widgets::filterlist::Row::Header(c)),
        "c has no row"
    );
    assert_eq!(included(&app), before, "c decides nothing yet");

    let a = set_named(&app, "a");
    let enabled = app.filters.sets()[a].enabled;
    run(&mut app, "list a");
    assert!(app.filters.sets()[a].listed);
    assert_eq!(
        app.filters.sets()[a].enabled,
        enabled,
        "a listed set stays as it is"
    );
}

/// `unlist` unlists a set, as `--unlist` does: it is disabled, its header
/// leaves the pane and its lines stop matching.
#[test]
fn unlist_unlists_a_set() {
    let mut app = app_with_three_sets("palette_unlist");
    let a = set_named(&app, "a");
    run(&mut app, "unlist a");
    assert!(app.prompt.is_none(), "the palette is still open");
    assert!(!app.filters.sets()[a].listed);
    assert!(!app.filters.sets()[a].enabled);
    assert_eq!(included(&app), 2, "alpha still matches");
    assert!(
        !widgets::filterlist::rows(&app.filters).contains(&widgets::filterlist::Row::Header(a)),
        "a still has a row"
    );
}

/// An unknown set, or a list item the set does not have, keeps the palette
/// open with an error that names it, and no set changes.
#[test]
fn a_bad_set_or_item_keeps_the_palette_open_with_an_error() {
    let mut app = app_with_three_sets("palette_set_bad");
    let c = set_named(&app, "c");
    for (line, named) in [
        ("set nope", "nope"),
        ("set c:gamma,nope", "nope"),
        ("unlist nope", "nope"),
        ("list nope", "nope"),
        ("set", "set name"),
        ("unlist", "set name"),
        ("list", "set name"),
    ] {
        run(&mut app, line);
        assert_eq!(prompt(&app).pattern, line, "the line is gone");
        assert!(
            prompt(&app)
                .error
                .as_deref()
                .is_some_and(|e| e.contains(named)),
            "{line}: {:?}",
            prompt(&app).error
        );
        key(&mut app, KeyCode::Esc);
    }
    assert!(!app.filters.sets()[c].enabled, "c was enabled");
    assert!(app.filters.sets().iter().all(|meta| meta.listed));
}

/// `set +a:ITEMS` changes the flags the pane shows now for `a`; without
/// the `+`, the list replaces them (#538).
#[test]
fn set_with_plus_starts_from_the_flags_the_pane_shows() {
    let mut app = app_with_profiles("palette_set_plus");
    let a = set_named(&app, "a");
    run(&mut app, "set a:beta");
    assert_eq!(flags_of(&app, a), [false, true], "sanity: beta alone");

    run(&mut app, "set +a:alpha");
    assert_eq!(flags_of(&app, a), [true, true], "alpha added to beta");

    run(&mut app, "set a:alpha");
    assert_eq!(flags_of(&app, a), [true, false], "no `+`: alpha alone");
}

/// `set a:-alpha` turns off a filter the empty start has off already: it
/// runs, closes the palette, and the status row names the forms the user
/// probably meant, as a hint and not as an error (#538).
#[test]
fn a_list_that_changes_nothing_closes_the_palette_with_a_hint() {
    let mut app = app_with_profiles("palette_set_hint");
    let a = set_named(&app, "a");

    run(&mut app, "set a:-alpha");

    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(flags_of(&app, a), [false, false]);
    let message = app.status_message.as_ref().expect("a hint");
    assert!(!message.error, "a hint is not an error");
    assert!(message.text.contains("+a:-alpha"), "{}", message.text);
}

/// After `set` and `unlist`, Tab completes the name of a known set, listed
/// or not, as a path completes: one that fits is written whole, and more
/// that fit are stepped through.
#[test]
fn tab_completes_a_set_name() {
    let mut app = app_with_three_sets("palette_tab_set");
    let c = set_named(&app, "c");
    app.filters.set_listed(c, false);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "set ");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set a");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set b");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set c", "an unlisted set is known");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "unlist de");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "unlist definitions");
}

/// After `set NAME:` and after each `,`, Tab completes a profile or a
/// named filter of that set; `unlist` takes no list, so it completes none.
#[test]
fn tab_completes_a_profile_or_filter_after_the_colon() {
    let mut app = app_with_profiles("palette_tab_items");
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "set a:");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set a:default");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set a:only-beta");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set a:alpha");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set a:beta");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "set a:alpha,on");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "set a:alpha,only-beta");
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "unlist a:");
    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "unlist a:");
}

// ---- help (#520) ------------------------------------------------------

/// The `?` overlay names the palette's own commands on the row of its key,
/// and Tab on the prompt's rows.
#[test]
fn the_help_overlay_documents_the_palette() {
    let mut app = app_over_file("palette_help", "a\n");
    key(&mut app, KeyCode::Char('?'));
    let screen = screen(&mut app);
    for text in ["Command palette", "e / cd <path>", "goto <n>", "Tab"] {
        assert!(
            screen.contains(text),
            "the overlay has no {text:?}:\n{screen}"
        );
    }
}

// ---- `include`, `exclude` and `context` (#527) -------------------------

/// The patterns and senses of the scratch set's filters, in pane order.
fn scratch_filters(app: &App) -> Vec<(String, filter::Sense)> {
    app.filters
        .filters_in(0)
        .map(|(_, filter)| (filter.predicate.display(), filter.sense))
        .collect()
}

/// `include RE` adds an including filter to the scratch set, as `i` and
/// Enter do in the filter pane: its lines match at once, and the focus
/// stays where it was.
#[test]
fn include_adds_an_including_filter_to_the_scratch_set() {
    let mut app = app_over_file("palette_include", "alpha\nbeta\n");
    key(&mut app, KeyCode::Char('t'));
    run(&mut app, "include al.ha");
    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(
        scratch_filters(&app),
        [("al.ha".to_string(), filter::Sense::Include)]
    );
    assert_eq!(included(&app), 1);
    assert_eq!(app.focus, Focus::View, "the focus moved");
}

/// `exclude RE` adds an excluding filter, whose lines leave the view, and
/// `context RE` a context filter, which colours its lines but does not
/// make them interesting.
#[test]
fn exclude_and_context_add_a_filter_of_their_sense() {
    let mut app = app_over_file("palette_exclude", "alpha\nbeta\ngamma\n");
    run(&mut app, "exclude beta");
    run(&mut app, "context gamma");
    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(
        scratch_filters(&app),
        [
            ("beta".to_string(), filter::Sense::Exclude),
            ("gamma".to_string(), filter::Sense::Context),
        ]
    );
    assert_eq!(
        app.document.verdicts()[1..],
        [Verdict::Excluded, Verdict::Context(1)]
    );
}

/// A pattern that does not compile, or no pattern, keeps the palette open
/// with the line and an error, and adds no filter (the #520 rule).
#[test]
fn a_bad_or_missing_pattern_keeps_the_palette_open_with_an_error() {
    let mut app = app_over_file("palette_include_bad", "alpha\n");
    for (line, says) in [
        ("include a(b", INVALID_PATTERN),
        ("exclude [z", INVALID_PATTERN),
        ("context", EMPTY_PATTERN),
    ] {
        run(&mut app, line);
        assert_eq!(prompt(&app).pattern, line, "the line is gone");
        assert_eq!(prompt(&app).error.as_deref(), Some(says), "{line}");
        key(&mut app, KeyCode::Esc);
    }
    assert_eq!(scratch_filters(&app), []);
}

// ---- user aliases (#557) ----------------------------------------------

/// Give `app` the aliases of an `[aliases]` table, as `config.toml` does at
/// startup.
fn alias(app: &mut App, aliases: &[(&str, &str)]) {
    let aliases = aliases
        .iter()
        .map(|(name, chain)| ((*name).to_string(), (*chain).to_string()))
        .collect();
    app.commands = crate::commands::with_aliases(&aliases);
}

/// An alias runs each command line of its chain, in order, as if it was
/// typed in the palette: all of a line after the command name is one
/// argument, blanks and all.
#[test]
fn an_alias_runs_its_chain_in_order_with_whole_arguments() {
    let mut app = app_over_file("palette_alias_chain", "al ha\nbeta\n");
    alias(&mut app, &[("both", "include al ha ; context be.a")]);
    run(&mut app, "both");
    assert!(
        app.prompt.is_none(),
        "{:?}",
        app.prompt.as_ref().map(|p| &p.error)
    );
    assert_eq!(
        scratch_filters(&app),
        [
            ("al ha".to_string(), filter::Sense::Include),
            ("be.a".to_string(), filter::Sense::Context),
        ]
    );
}

/// A line of the chain that fails stops it there: the lines before it stay
/// done, the lines after it do not run, and the palette stays open with
/// the alias and an error that names the alias and the line.
#[test]
fn a_failed_line_stops_the_alias_with_an_error_that_names_it() {
    let mut app = app_over_file("palette_alias_fails", "alpha\nbeta\n");
    alias(
        &mut app,
        &[("bad", "include alpha ; goto nope ; include beta")],
    );
    run(&mut app, "bad");
    assert_eq!(prompt(&app).pattern, "bad");
    let error = prompt(&app).error.clone().unwrap_or_default();
    assert!(error.contains("bad"), "{error}");
    assert!(error.contains("goto nope"), "{error}");
    assert!(error.contains("not a line number"), "{error}");
    assert_eq!(
        scratch_filters(&app),
        [("alpha".to_string(), filter::Sense::Include)]
    );
}

/// An alias of one `set` line does what the line does when it is typed.
#[test]
fn an_alias_of_a_set_line_works_as_the_typed_line() {
    let mut typed_line = app_with_three_sets("palette_alias_set_typed");
    let mut aliased = app_with_three_sets("palette_alias_set");
    alias(&mut aliased, &[("gam", "set c:gamma")]);
    for app in [&mut typed_line, &mut aliased] {
        let c = set_named(app, "c");
        app.filters.set_listed(c, false);
        app.refresh_view();
    }
    run(&mut typed_line, "set c:gamma");
    run(&mut aliased, "gam");
    assert!(aliased.prompt.is_none(), "the palette is still open");
    let c = set_named(&aliased, "c");
    assert!(aliased.filters.sets()[c].enabled);
    assert_eq!(flags_of(&aliased, c), flags_of(&typed_line, c));
    assert_eq!(included(&aliased), included(&typed_line));
}

/// An alias has a row of its own in the list, with its chain as the help,
/// and Tab writes its name, with no blank after it: it takes no argument.
#[test]
fn an_alias_is_listed_with_its_chain_and_tab_writes_it() {
    let mut app = app_over_file("palette_alias_list", "a\n");
    alias(&mut app, &[("proj", "e ~/projects/recon ; explorer.focus")]);
    key(&mut app, KeyCode::Char(':'));
    typed(&mut app, "proj");
    let rows = list_rows(&mut app);
    let top = rows
        .iter()
        .find(|row| !row.trim().is_empty())
        .expect("a row");
    assert!(top.contains("proj"), "{rows:#?}");
    assert!(top.contains("e ~/projects/recon ; explorer.focus"), "{top}");

    key(&mut app, KeyCode::Tab);
    assert_eq!(prompt(&app).pattern, "proj");
}

/// The aliases come from the `[aliases]` of the config the app starts
/// with.
#[test]
fn the_app_runs_the_aliases_of_its_config() {
    let file = fixture_path("palette_alias_config", "alpha\n");
    let mut app = App::new(&Startup::from(Config {
        path: Some(file.display().to_string()),
        aliases: [("al".to_string(), "include alpha".to_string())].into(),
        ..Config::default()
    }));
    run(&mut app, "al");
    assert!(app.prompt.is_none(), "the palette is still open");
    assert_eq!(
        scratch_filters(&app),
        [("alpha".to_string(), filter::Sense::Include)]
    );
}
