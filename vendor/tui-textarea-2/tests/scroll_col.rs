// Tests for fork change #3: a horizontal scroll offset a caller owns,
// independent of the cursor (TextArea::set_scroll_col and friends).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use tui_textarea::{CursorMove, CursorRenderMode, TextArea};

/// A line of 80 distinguishable cells: column `i` holds the letter
/// `(b'a' + i % 26)`, so a rendered row's first character names the column
/// the view is scrolled to.
fn alphabet_line() -> String {
    (0..80)
        .map(|i| char::from(b'a' + (i % 26)))
        .collect()
}

fn plain(lines: Vec<String>) -> TextArea<'static> {
    let mut textarea = TextArea::new(lines);
    textarea.set_cursor_render_mode(CursorRenderMode::Hidden);
    textarea.set_cursor_line_style(Style::default());
    textarea
}

fn render(textarea: &TextArea<'_>, width: u16, height: u16) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    Widget::render(textarea, area, &mut buf);
    buf
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn set_scroll_col_scrolls_without_moving_the_cursor() {
    let mut textarea = plain(vec![alphabet_line()]);
    let width = 20;
    render(&textarea, width, 3); // establish the viewport width

    textarea.set_scroll_col(30);

    assert_eq!(textarea.cursor(), (0, 0), "the cursor must not move");
    assert_eq!(textarea.scroll_top(), (0, 30));
    let buf = render(&textarea, width, 3);
    // Column 30 of the line is 'e' (30 % 26).
    assert!(
        row_text(&buf, 0).starts_with('e'),
        "expected the view to start at column 30, got {:?}",
        row_text(&buf, 0)
    );
}

#[test]
fn pin_survives_renders_with_the_cursor_off_to_the_left() {
    let mut textarea = plain(vec![alphabet_line()]);
    let width = 20;
    render(&textarea, width, 3);

    textarea.set_scroll_col(30);
    let first = render(&textarea, width, 3);
    let second = render(&textarea, width, 3);

    // Without the pin, the cursor at column 0 would snap the view back to 0
    // on the first re-render (next_scroll_top: cursor < prev_top -> cursor).
    assert_eq!(textarea.scroll_top(), (0, 30));
    assert_eq!(row_text(&first, 0), row_text(&second, 0));
    assert!(row_text(&second, 0).starts_with('e'));
}

#[test]
fn set_beyond_max_clamps_to_the_widest_line() {
    let mut textarea = plain(vec![alphabet_line()]);
    let width = 20;
    render(&textarea, width, 3);

    // 80 columns of text in a 20-column window: 60 columns of overhang.
    assert_eq!(textarea.max_scroll_col(), 60);
    textarea.set_scroll_col(100);
    assert_eq!(textarea.scroll_top(), (0, 60));

    let buf = render(&textarea, width, 3);
    // The last text column (79) lands at the right edge of the window.
    let row = row_text(&buf, 0);
    assert_eq!(row.chars().last(), Some(char::from(b'a' + 79 % 26)));
}

#[test]
fn scrolling_back_to_zero_shows_the_line_start() {
    let mut textarea = plain(vec![alphabet_line()]);
    let width = 20;
    render(&textarea, width, 3);

    textarea.set_scroll_col(30);
    render(&textarea, width, 3);
    textarea.set_scroll_col(0);

    assert_eq!(textarea.scroll_top(), (0, 0));
    let buf = render(&textarea, width, 3);
    assert!(row_text(&buf, 0).starts_with('a'));
}

#[test]
fn clear_scroll_pin_returns_to_cursor_driven_scrolling() {
    let mut textarea = plain(vec![alphabet_line()]);
    let width = 20;
    render(&textarea, width, 3);

    textarea.set_scroll_col(30);
    assert!(textarea.is_scroll_col_pinned());
    textarea.clear_scroll_col_pin();

    assert!(!textarea.is_scroll_col_pinned());
    assert_eq!(textarea.scroll_top(), (0, 0));
    // Cursor-driven pinning is back: parking the cursor at the line end
    // scrolls the view so the cursor is visible again. The upstream rule
    // lands the cursor cell one past the last character in the last column
    // (cursor + 1 - width = 61), one column further right than the bar's
    // own clamp of 60 — that is upstream behaviour, untouched.
    textarea.move_cursor(CursorMove::End);
    render(&textarea, width, 3);
    assert_eq!(textarea.scroll_top(), (0, 61));
}

#[test]
fn cursor_leaving_the_right_edge_follows_clamped_to_max() {
    let mut textarea = plain(vec![alphabet_line()]);
    let width = 20;
    render(&textarea, width, 3);
    textarea.set_scroll_col(30);

    // Land the cursor past the window's right edge (col 50 > 30 + 20).
    textarea.move_cursor(CursorMove::Jump(0, 50));
    render(&textarea, width, 3);
    assert_eq!(textarea.scroll_top(), (0, 31));

    // And the follow never passes the clamp: the line's end pins to the max.
    textarea.move_cursor(CursorMove::End);
    render(&textarea, width, 3);
    assert_eq!(textarea.scroll_top(), (0, 60));
}

#[test]
fn vertical_scroll_keeps_the_cursor_column_while_pinned() {
    let lines = vec![alphabet_line(); 10];
    let mut textarea = plain(lines);
    let width = 20;
    render(&textarea, width, 3);
    textarea.move_cursor(CursorMove::Jump(0, 10));
    textarea.set_scroll_col(30);
    render(&textarea, width, 3);

    // A plain vertical scroll (what a j/k motion ends up as) clamps the
    // cursor into the viewport; the horizontal component must not apply
    // while the pin holds the caller's column.
    textarea.scroll((1, 0));

    assert_eq!(textarea.cursor(), (1, 10));
    assert_eq!(textarea.scroll_top().1, 30);
}

#[test]
fn horizontal_scroll_deltas_move_the_pin() {
    let lines = vec![alphabet_line(); 10];
    let mut textarea = plain(lines);
    let width = 20;
    render(&textarea, width, 3);
    textarea.set_scroll_col(30);

    textarea.scroll((0, 5));

    // The pin follows the explicit delta instead of snapping back on render.
    assert_eq!(textarea.scroll_top().1, 35);
    render(&textarea, width, 3);
    assert_eq!(textarea.scroll_top().1, 35);
}

#[test]
fn set_cursor_position_releases_the_pin() {
    let lines = vec![alphabet_line(); 10];
    let mut textarea = plain(lines);
    let width = 20;
    render(&textarea, width, 3);
    textarea.set_scroll_col(30);

    // An explicit jump (a search hit, a motion App owns) takes the view over.
    textarea.set_cursor_position((4, 5));

    assert!(!textarea.is_scroll_col_pinned());
    assert_eq!(textarea.scroll_top(), (0, 0));
    render(&textarea, width, 3);
    // Cursor-driven pinning resumes: the cursor's column is on screen.
    assert_eq!(textarea.cursor(), (4, 5));
    assert!(textarea.scroll_top().1 <= 5);
}

#[test]
fn set_lines_resets_the_pin() {
    let mut textarea = plain(vec![alphabet_line()]);
    let width = 20;
    render(&textarea, width, 3);
    textarea.set_scroll_col(30);

    textarea.set_lines(vec!["short".to_string()], (0, 0));

    assert!(!textarea.is_scroll_col_pinned());
    assert_eq!(textarea.scroll_top(), (0, 0));
    render(&textarea, width, 3); // re-establish the viewport width
    assert_eq!(textarea.max_scroll_col(), 0); // "short" fits the window
}

#[test]
fn the_gutter_counts_in_the_limit() {
    let mut textarea = plain(vec![alphabet_line(); 3]);
    textarea.set_line_number_style(Style::default());
    let width = 20;
    render(&textarea, width, 3);

    // One digit + two margins = 3 columns of gutter, on top of the 80 of text.
    assert_eq!(textarea.gutter_width(), 3);
    assert_eq!(textarea.max_scroll_col(), 80 + 3 - 20);

    textarea.set_scroll_col(100);
    assert_eq!(textarea.scroll_top().1, 63);
}

#[test]
fn short_lines_have_nothing_to_scroll() {
    let mut textarea = plain(vec!["short line".to_string()]);
    let width = 20;
    render(&textarea, width, 3);

    assert_eq!(textarea.max_scroll_col(), 0);
    textarea.set_scroll_col(10);
    assert_eq!(textarea.scroll_top().1, 0);
}
