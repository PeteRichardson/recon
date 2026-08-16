use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use tui_textarea::{CursorRenderMode, TextArea};

fn plain(lines: &[&str]) -> TextArea<'static> {
    let mut textarea = TextArea::new(lines.iter().map(|s| s.to_string()).collect());
    // The cursor is drawn as a styled cell over column 0 of its line, which
    // would mask the styles under test. Hide it and neutralise the cursor
    // line so assertions isolate the feature.
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
fn line_styles_apply_to_the_named_line_only() {
    let mut textarea = plain(&["alpha", "beta", "gamma"]);
    textarea.set_line_styles(vec![None, Some(Style::default().fg(Color::Yellow)), None]);

    let buf = render(&textarea, 10, 3);

    assert_eq!(row_text(&buf, 1), "beta", "styled the row we think we styled");
    assert_eq!(buf[(0, 1)].style().fg, Some(Color::Yellow), "beta not styled");
    assert_ne!(buf[(0, 2)].style().fg, Some(Color::Yellow), "gamma wrongly styled");
}

#[test]
fn lines_past_the_end_of_the_styles_are_unstyled() {
    let mut textarea = plain(&["alpha", "beta", "gamma"]);
    // Deliberately shorter than the buffer: must not panic or misapply.
    // Row 0 is avoided deliberately — the cursor starts there, and the
    // cursor-line style legitimately overrides per-line styles.
    textarea.set_line_styles(vec![None, Some(Style::default().fg(Color::Yellow))]);

    let buf = render(&textarea, 10, 3);

    assert_eq!(buf[(0, 1)].style().fg, Some(Color::Yellow), "beta not styled");
    assert_ne!(buf[(0, 2)].style().fg, Some(Color::Yellow), "gamma styled past the end");
}

#[test]
fn line_styles_are_empty_by_default() {
    let textarea = plain(&["alpha"]);
    assert!(textarea.line_styles().is_empty());
}

#[test]
fn clear_line_styles_restores_the_default_look() {
    let mut textarea = plain(&["alpha"]);
    textarea.set_line_styles(vec![Some(Style::default().fg(Color::Yellow))]);
    textarea.clear_line_styles();

    let buf = render(&textarea, 10, 1);

    assert!(textarea.line_styles().is_empty());
    assert_ne!(buf[(0, 0)].style().fg, Some(Color::Yellow));
}

#[test]
fn the_cursor_line_style_still_wins() {
    let mut textarea = plain(&["alpha", "beta"]);
    textarea.set_cursor_line_style(Style::default().fg(Color::Green));
    textarea.set_line_styles(vec![Some(Style::default().fg(Color::Yellow)); 2]);

    let buf = render(&textarea, 10, 2);

    // The cursor sits on row 0, which must keep the cursor-line colour.
    assert_eq!(buf[(0, 0)].style().fg, Some(Color::Green), "cursor line lost its style");
    assert_eq!(buf[(0, 1)].style().fg, Some(Color::Yellow), "other line lost its style");
}

#[test]
fn line_numbers_can_be_overridden() {
    let mut textarea = plain(&["beta", "delta"]);
    textarea.set_line_number_style(Style::default());
    // 0-based source rows 1 and 3 render as 2 and 4.
    textarea.set_line_numbers(vec![1, 3]);

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_start().starts_with("2 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("4 "), "got {:?}", row_text(&buf, 1));
}

/// The gutter pads with `lnum_len - num_digits(row + 1)`, an unsigned
/// subtraction. An override wider than the buffer's own line count would
/// underflow and panic unless the gutter is sized from the overrides.
#[test]
fn the_gutter_widens_for_overridden_numbers() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_line_numbers(vec![9997, 9998]);

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_start().starts_with("9998 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("9999 "), "got {:?}", row_text(&buf, 1));
}

#[test]
fn without_overrides_numbering_is_unchanged() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_start().starts_with("1 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("2 "), "got {:?}", row_text(&buf, 1));
}

#[test]
fn rows_without_an_override_fall_back_to_their_position() {
    let mut textarea = plain(&["a", "b", "c"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_line_numbers(vec![41]);

    let buf = render(&textarea, 20, 3);

    assert!(row_text(&buf, 0).trim_start().starts_with("42 "), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_start().starts_with("2 "), "got {:?}", row_text(&buf, 1));
}

#[test]
fn clear_line_numbers_restores_natural_numbering() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_line_numbers(vec![100, 101]);
    textarea.clear_line_numbers();

    let buf = render(&textarea, 20, 2);

    assert!(textarea.line_numbers().is_empty());
    assert!(row_text(&buf, 0).trim_start().starts_with("1 "), "got {:?}", row_text(&buf, 0));
}
