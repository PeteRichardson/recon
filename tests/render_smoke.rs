use crossterm::event::{Event, KeyCode, KeyEvent};
use recon::{App, Config};
use ratatui::prelude::{Buffer, Rect, Widget};

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

/// Read the right-hand file view back out of a freshly rendered buffer.
fn view_pane(app: &mut App) -> String {
    let mut buf = Buffer::empty(AREA);
    app.render(AREA, &mut buf);
    (0..AREA.height)
        .map(|y| {
            (40..AREA.width)
                .map(|x| buf[(x, y)].symbol())
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
fn nav_pane_rows(buf: &Buffer, area: Rect, width: u16) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..width)
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

    let rows = nav_pane_rows(&buf, area, 40);
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
    let row = nav_pane_rows(&buf, AREA, 40)
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
    let previewed = view_pane(&mut app);
    press(&mut app, KeyCode::Down);
    assert_eq!(highlighted_name(&mut app), "src", "expected to land on src");

    assert_eq!(
        previewed,
        view_pane(&mut app),
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
    let view_before = view_pane(&mut app);
    press(&mut app, KeyCode::Enter);

    let mut buf = Buffer::empty(AREA);
    (&mut app).render(AREA, &mut buf);
    let nav = nav_pane_rows(&buf, AREA, 40).join("\n");

    assert!(nav.contains("lib.rs"), "nav did not descend into src:\n{nav}");
    assert_eq!(
        view_before,
        view_pane(&mut app),
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
