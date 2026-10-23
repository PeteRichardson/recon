use super::*;

// ---- filetype icons reach both draw sites (#512) ---------------------------

/// The resolved flag is pushed into the panes at startup, exactly as the
/// background and the mode are. These tests hold the wiring; what each pane
/// then *draws* is pinned in `filetype.rs`, `explorer.rs` and `read.rs`.
#[test]
fn with_icons_pushes_the_flag_into_both_panes() {
    let app = App::new(&Startup::from(Config::default())).with_icons(crate::filetype::Icons {
        on: true,
        dropped: vec![],
    });
    // The explorer rebuilds its rows with glyphs; the loading path stamps
    // every read job with the same answer, so a directory listing shown in
    // the view pane draws them too. One decision, both sites.
    assert!(app.explorer.has_icons());
    assert!(app.loading.has_icons());
}

#[test]
fn an_app_without_icons_has_none() {
    let app = App::new(&Startup::from(Config::default()));
    assert!(!app.explorer.has_icons());
    assert!(!app.loading.has_icons());
}

#[test]
fn the_config_ladder_ends_in_auto() {
    // `icons_setting` is what `main` resolves; the default is `auto`, the
    // setting that keeps a base-fonts machine exactly as it was.
    assert_eq!(
        Config::default().icons_setting(),
        crate::filetype::Setting::Auto
    );
}
