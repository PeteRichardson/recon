use super::*;

// ---- `&` is not a key (#474) ------------------------------------------

/// The global AND mode (#39) is gone: an advanced filter is the way to AND
/// (ADR 0008), and two AND features at one time are a trap. `&` is not a
/// key any more, so the filters stay OR-ed and the row has no badge.
#[test]
fn ampersand_no_longer_ands_the_filters() {
    let mut app = app_over_file("and_mode", "foo\nbar\nfoo bar\n");
    for pattern in ["foo", "bar"] {
        key(&mut app, KeyCode::Char('f'));
        key(&mut app, KeyCode::Char('i'));
        typed(&mut app, pattern);
        key(&mut app, KeyCode::Enter);
    }
    let included = |app: &App| {
        app.document
            .verdicts()
            .iter()
            .filter(|v| matches!(v, filter::Verdict::Included(_)))
            .count()
    };
    assert_eq!(included(&app), 3, "sanity: OR includes every line");

    key(&mut app, KeyCode::Char('&'));

    assert_eq!(included(&app), 3, "still OR");
    assert!(
        !status_line(&mut app).contains(" AND "),
        "{}",
        status_line(&mut app)
    );
}
