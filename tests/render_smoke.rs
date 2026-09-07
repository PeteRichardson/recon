use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::prelude::{Buffer, Rect, Widget};
use recon::{App, Config};

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

/// Column where the nav pane ends, read off its top-right corner. The panes
/// size themselves to their contents, so the split cannot be assumed.
fn divider_column(buf: &Buffer) -> u16 {
    (0..AREA.width)
        // Plain *or* thick: the focused pane draws a heavy border, so the
        // navigator's own corner is `┓` whenever it has focus. Matching only
        // the plain glyph found the *file view's* corner instead and put the
        // divider at the far edge of the screen.
        .find(|&x| matches!(buf[(x, 0)].symbol(), "┐" | "┓"))
        .expect("no nav pane border in the rendered frame")
        + 1
}

/// Read the right-hand file view back out of a freshly rendered buffer.
fn view_pane(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let divider = divider_column(&buf);
    (0..AREA.height)
        .map(|y| {
            (divider..AREA.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The file view's contents with borders and padding stripped, so comparisons
/// survive the pane being resized by auto-snapping.
///
/// Lines are also truncated to a width both panes comfortably exceed: a wider
/// pane clips long lines later, which would otherwise read as a difference.
fn view_text(app: &mut App) -> String {
    const COMPARABLE: usize = 30;

    view_pane(app)
        .lines()
        .map(|line| {
            line.trim_matches(|c| "┌┐└┘│─┏┓┗┛┃━".contains(c))
                .trim_end()
                .chars()
                .take(COMPARABLE)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::from(code)));
}

/// The one line of `Cargo.toml` the view tests look for. A fixture's, not
/// the repo's: this file used to look for `tui-textarea-2` in the real
/// `Cargo.toml`, which was on screen only because a comment happened to
/// mention it near the top.
const MARKER: &str = "name = \"render-smoke-fixture\"";

/// A directory of known files under `target/test-navdirs/render_smoke/`,
/// one per test so the tests can run in parallel.
///
/// Every test here used to run against the repo root, which made each one a
/// claim about the working tree: that `..` fits above `Cargo.toml` in a
/// 13-row navigator, that `Cargo.toml` mentions a vendored crate in its
/// first screen, that `Cargo.lock` exists, that `filenav.rs` is longer than
/// a page. An untracked directory or two in the root — a worktree, a tool's
/// cache — scrolled `..` off and failed a test that had nothing to do with
/// the change (#152, and #82 before it). What the working tree contains is
/// not any of these tests' subject.
///
/// The layout, in navigator order — directories first, then names
/// case-insensitively:
///
/// ```text
/// ..
/// beta_dir/      first.rs, second.rs
/// alpha.rs       one line, "content"
/// Cargo.lock     a `[[package]]` table
/// Cargo.toml     `[package]` first, then `MARKER`
/// long.rs        sixty numbered lines, longer than any pane here
/// ```
///
/// `beta_dir` is the only directory, so it sits directly above `alpha.rs`:
/// the directory tests step between the two.
fn fixture(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new("target/test-navdirs/render_smoke").join(name);
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("beta_dir")).expect("create fixture dir");
    let write = |rel: &str, body: &str| {
        std::fs::write(dir.join(rel), body).expect("write fixture");
    };
    write(
        "Cargo.toml",
        &format!("[package]\n{MARKER}\nversion = \"0.0.0\"\n"),
    );
    write(
        "Cargo.lock",
        "# fixture\nversion = 3\n\n[[package]]\nname = \"render-smoke-fixture\"\n",
    );
    write("alpha.rs", "content\n");
    write("beta_dir/first.rs", "the first entry's text\n");
    write("beta_dir/second.rs", "the second entry's text\n");
    let long = (0..60).fold(String::new(), |mut body, i| {
        use std::fmt::Write as _;
        let _ = writeln!(body, "line {i}");
        body
    });
    write("long.rs", &long);
    dir
}

/// An `App` opened on `file` inside a fresh fixture directory named `name`.
fn app_on(name: &str, file: &str) -> App<'static> {
    let config = Config {
        path: fixture(name).join(file).display().to_string(),
        ..Config::default()
    };
    App::new(&config)
}

#[test]
fn renders_file_contents_into_buffer() {
    let mut app = app_on("renders_file_contents", "Cargo.toml");
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);

    (&mut app).render(area, &mut buf);

    let text: String = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(
        text.contains(MARKER),
        "textarea did not render file contents:\n{text}"
    );
    assert!(text.contains("Cargo.toml"), "block title missing");
}

/// Read the left-hand nav pane back out of the buffer, row by row.
fn nav_pane_rows(buf: &Buffer) -> Vec<String> {
    let divider = divider_column(buf);
    (0..AREA.height)
        .map(|y| {
            (0..divider)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn nav_pane_renders_directory_entries() {
    let mut app = app_on("nav_pane_entries", "Cargo.toml");
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);

    (&mut app).render(area, &mut buf);

    let rows = nav_pane_rows(&buf);
    let pane = rows.join("\n");

    assert!(pane.contains(".."), "parent entry missing:\n{pane}");
    assert!(
        pane.contains("Cargo.toml"),
        "nav pane did not list real directory entries:\n{pane}"
    );
    assert!(
        pane.contains("beta_dir"),
        "nav pane missing the directory entry:\n{pane}"
    );
    assert!(
        highlighted_row_index(&buf).is_some(),
        "nav pane drew no selection highlight:\n{pane}"
    );
}

/// Row of the nav pane drawn as selected, found by its reverse-video
/// attribute.
///
/// This used to look for the `>>` marker, which no longer exists — reverse
/// video is now the only thing that says "selected", so it is what the tests
/// have to read.
fn highlighted_row_index(buf: &Buffer) -> Option<u16> {
    let divider = divider_column(buf);
    (0..AREA.height).find(|&y| {
        (0..divider).any(|x| {
            buf[(x, y)]
                .style()
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED)
        })
    })
}

/// The name on the currently highlighted nav row, with border glyphs and a
/// directory's trailing `/` stripped off.
fn highlighted_name(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let y = highlighted_row_index(&buf).expect("no highlighted row");
    let divider = divider_column(&buf);
    (0..divider)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_matches(|c| c == '\u{2502}' || c == '\u{2503}' || c == ' ')
        .trim_end_matches('/')
        .to_string()
}

/// Walk the nav selection to `name`. Keeps the tests independent of how many
/// entries the working tree happens to contain.
///
/// Rewinds to the top first: the cursor no longer starts on `..`, it starts
/// on whatever the startup argument selected, so walking only downwards
/// cannot reach an entry that sorts before it.
fn highlight(app: &mut App, name: &str) {
    for _ in 0..64 {
        press(app, KeyCode::Up);
    }
    for _ in 0..64 {
        if highlighted_name(app) == name {
            return;
        }
        press(app, KeyCode::Down);
    }
    panic!("never highlighted {name}");
}

#[test]
fn enter_on_a_file_loads_it_into_the_view() {
    let mut app = app_on("enter_on_a_file", "Cargo.toml");
    // The claim is "the file named on the CLI is in the view at startup", and
    // only the top of it is guaranteed to be on screen — which is why this
    // once broke when a comment block pushed the header it looked for below
    // the pane (#82). The fixture's file is three lines, so the whole thing
    // is on screen.
    assert!(
        view_pane(&mut app).contains("[package]"),
        "expected Cargo.toml in the view at startup"
    );

    highlight(&mut app, "Cargo.lock");
    press(&mut app, KeyCode::Enter);

    let pane = view_pane(&mut app);
    assert!(
        pane.contains("[[package]]"),
        "Cargo.lock was not loaded into the view:\n{pane}"
    );
}

/// Moving the selection is enough on its own; Enter is only for directories.
#[test]
fn moving_onto_a_file_loads_it_without_enter() {
    let mut app = app_on("moving_onto_a_file", "Cargo.toml");

    highlight(&mut app, "Cargo.lock");

    let pane = view_pane(&mut app);
    assert!(
        pane.contains("[[package]]"),
        "Cargo.lock was not previewed on selection:\n{pane}"
    );
}

/// Stepping onto a directory replaces the view with `<directory>`.
///
/// It used to keep the last file on screen, which read as though the
/// directory contained that text. The pane now always describes what is
/// actually selected.
///
#[test]
fn moving_onto_a_directory_shows_that_it_is_a_directory() {
    let mut app = app_on("moving_onto_a_directory", "alpha.rs");

    // `beta_dir` sits directly above `alpha.rs`: directories sort first (#96),
    // so the step onto it is upwards.
    highlight(&mut app, "alpha.rs");
    assert!(
        view_text(&mut app).contains("content"),
        "precondition: the file is on screen"
    );

    press(&mut app, KeyCode::Up);

    assert_eq!(highlighted_name(&mut app), "beta_dir");
    let shown = view_text(&mut app);
    // Selecting a directory previews its listing, so the probe for "a
    // directory is selected" is a name from inside it. The probe used to be
    // `<directory>`, which every directory rendered; that placeholder now
    // survives only for the empty case.
    assert!(
        shown.contains("first.rs"),
        "the view did not list the directory:\n{shown}"
    );
    assert!(
        !shown.contains("content"),
        "the previous file's text is still on screen:\n{shown}"
    );
}

/// Descending relists the nav pane *and* moves the view onto the first entry
/// of the directory entered.
///
/// It used to leave the view untouched, so you descended into a directory and
/// went on looking at a file from the one you had just left.
#[test]
fn enter_on_a_directory_relists_and_previews_its_first_entry() {
    // This once walked the repo's own `src/` and named `document.rs` as the
    // entry that sorts first, so adding `src/config.rs` broke a test about
    // pressing Enter. What sorts first in recon's source tree is not this
    // test's subject; the fixture's `beta_dir` has two entries of its own.
    let mut app = app_on("enter_on_a_directory", "alpha.rs");

    highlight(&mut app, "beta_dir");
    let view_before = view_text(&mut app);
    // Selecting a directory now previews its *contents*, so the probe for
    // "a directory is selected" is a name from inside it.
    assert!(
        view_before.contains("first.rs"),
        "precondition: a directory is selected:\n{view_before}"
    );

    press(&mut app, KeyCode::Enter);

    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);
    let nav = nav_pane_rows(&buf).join("\n");
    assert!(
        nav.contains("second.rs"),
        "nav did not descend into beta_dir:\n{nav}"
    );

    // `first.rs` sorts first, so the cursor landed on it and previewed it.
    assert_eq!(highlighted_name(&mut app), "first.rs");
    let view_after = view_text(&mut app);
    assert!(
        view_after.contains("the first entry's text"),
        "the first entry was not previewed:\n{view_after}"
    );
}

#[test]
fn tab_moves_focus_to_the_file_view() {
    // The long file, so that a page-down actually has somewhere to scroll to.
    let mut app = app_on("tab_moves_focus", "long.rs");

    press(&mut app, KeyCode::Tab);
    let before = view_pane(&mut app);
    // `]` pages down. It took that job from `Enter` in #48, when `Enter` became
    // the filter pane's toggle and `space` the global peek.
    press(&mut app, KeyCode::Char(']'));
    let after = view_pane(&mut app);

    assert_ne!(before, after, "`]` did not reach the focused file view");
}

/// The panes size themselves to the longest entry name, capped at a default.
#[test]
fn nav_pane_snaps_to_its_contents() {
    let mut app = app_on("nav_pane_snaps", "Cargo.toml");
    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);

    let longest = nav_pane_rows(&buf)
        .iter()
        .filter_map(|row| {
            row.split(['│', '┌', '┐', '┃', '┏', '┓'])
                .nth(1)
                .map(str::trim)
                .map(str::len)
        })
        .max()
        .expect("no nav rows");

    // Two borders and the two-column marker on top of the longest name.
    assert!(
        divider_column(&buf) <= longest as u16 + 6,
        "nav pane is wider than its contents need: {} for a {longest}-char name",
        divider_column(&buf)
    );
}

fn click(app: &mut App, kind: MouseEventKind, column: u16) {
    app.handle_event(Event::Mouse(MouseEvent {
        kind,
        column,
        row: 3,
        modifiers: KeyModifiers::empty(),
    }));
}

#[test]
fn dragging_the_divider_resizes_the_panes_on_screen() {
    let mut app = app_on("dragging_the_divider", "Cargo.toml");
    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);
    let before = divider_column(&buf);

    click(&mut app, MouseEventKind::Down(MouseButton::Left), before);
    click(&mut app, MouseEventKind::Drag(MouseButton::Left), 50);
    click(&mut app, MouseEventKind::Up(MouseButton::Left), 50);

    let mut after_buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut after_buf);
    let after = divider_column(&after_buf);

    assert_ne!(before, after, "divider did not move");
    assert_eq!(after, 50, "divider did not land where it was dragged");
}
