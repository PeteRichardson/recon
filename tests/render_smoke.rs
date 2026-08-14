use recon::{App, Config};
use ratatui::prelude::{Buffer, Rect, Widget};

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

    let rows = nav_pane_rows(&buf, area, 20);
    let pane = rows.join("\n");

    assert!(pane.contains("List"), "nav block title missing:\n{pane}");
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
