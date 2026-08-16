use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use recon::{App, Config};
use ratatui::prelude::{Buffer, Rect, Widget};

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
        .find(|&x| buf[(x, 0)].symbol() == "┐")
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
            line.trim_matches(|c| "┌┐└┘│─".contains(c))
                .trim_end()
                .chars()
                .take(COMPARABLE)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::from(code))).unwrap();
}

#[test]
fn renders_file_contents_into_buffer() {
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);

    (&mut app).render(area, &mut buf);

    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("tui-textarea-2"), "textarea did not render file contents:\n{text}");
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
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);
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
        pane.contains("src"),
        "nav pane missing src entry:\n{pane}"
    );
    assert!(
        pane.contains(">>"),
        "nav pane drew no selection highlight:\n{pane}"
    );
}

/// The name on the currently highlighted nav row, with the border glyphs and
/// the `>>` marker stripped off.
fn highlighted_name(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    let row = nav_pane_rows(&buf)
        .into_iter()
        .find(|row| row.contains(">>"))
        .expect("no highlighted row");
    row.split(">>")
        .nth(1)
        .unwrap_or_default()
        .trim_end_matches('│')
        .trim()
        .to_string()
}

/// Walk the nav selection down until `name` is highlighted. Keeps the tests
/// independent of how many entries the working tree happens to contain.
fn highlight(app: &mut App, name: &str) {
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
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);
    assert!(
        view_pane(&mut app).contains("[dependencies]"),
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
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);

    highlight(&mut app, "Cargo.lock");

    let pane = view_pane(&mut app);
    assert!(
        pane.contains("[[package]]"),
        "Cargo.lock was not previewed on selection:\n{pane}"
    );
}

/// Stepping onto a directory must not blank the view: it keeps the last file.
#[test]
fn moving_onto_a_directory_leaves_the_view_alone() {
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);

    // `Cargo.toml` sorts immediately before `src`, so a single Down lands on
    // the directory without passing over any other file.
    highlight(&mut app, "Cargo.toml");
    let previewed = view_text(&mut app);
    press(&mut app, KeyCode::Down);
    assert_eq!(highlighted_name(&mut app), "src", "expected to land on src");

    assert_eq!(
        previewed,
        view_text(&mut app),
        "the view changed on a directory"
    );
}

#[test]
fn enter_on_a_directory_relists_the_nav_pane_without_touching_the_view() {
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);

    // Capture the view once the cursor is already on `src`: getting there
    // passes over files, which now preview as the selection moves.
    highlight(&mut app, "src");
    let view_before = view_text(&mut app);
    press(&mut app, KeyCode::Enter);

    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);
    let nav = nav_pane_rows(&buf).join("\n");

    assert!(nav.contains("lib.rs"), "nav did not descend into src:\n{nav}");
    assert_eq!(
        view_before,
        view_text(&mut app),
        "descending should leave the file view alone"
    );
}

#[test]
fn tab_moves_focus_to_the_file_view() {
    // A long file, so that a page-down actually has somewhere to scroll to.
    let config = Config {
        file: "src/widgets/filenav.rs".to_string(),
    };
    let mut app = App::new(&config);

    press(&mut app, KeyCode::Tab);
    let before = view_pane(&mut app);
    press(&mut app, KeyCode::Enter);
    let after = view_pane(&mut app);

    assert_ne!(before, after, "Enter did not reach the focused file view");
}

/// The panes size themselves to the longest entry name, capped at a default.
#[test]
fn nav_pane_snaps_to_its_contents() {
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);
    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);

    let longest = nav_pane_rows(&buf)
        .iter()
        .filter_map(|row| row.split(['│', '┌', '┐']).nth(1).map(str::trim).map(str::len))
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
    }))
    .unwrap();
}

#[test]
fn dragging_the_divider_resizes_the_panes_on_screen() {
    let config = Config {
        file: "Cargo.toml".to_string(),
    };
    let mut app = App::new(&config);
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
