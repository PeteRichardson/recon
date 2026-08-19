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

/// A minimum width lets a caller reserve gutter room the buffer does not yet
/// justify — `recon` sizes the gutter for a file's estimated line count while
/// only a bounded preview is loaded, so the column does not jump when the
/// rest of the file arrives.
#[test]
fn a_minimum_width_reserves_gutter_room() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_min_line_number_width(4);

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_end().ends_with("   1 a"), "got {:?}", row_text(&buf, 0));
    assert!(row_text(&buf, 1).trim_end().ends_with("   2 b"), "got {:?}", row_text(&buf, 1));
}

/// The minimum only ever raises the width. A buffer already wider than the
/// minimum keeps its own numbering, so a stale reservation cannot truncate a
/// gutter that has outgrown it.
#[test]
fn a_minimum_narrower_than_the_content_is_ignored() {
    let mut textarea = plain(&["a", "b"]);
    textarea.set_line_number_style(Style::default());
    textarea.set_line_numbers(vec![9997, 9998]);
    textarea.set_min_line_number_width(2);

    let buf = render(&textarea, 20, 2);

    assert!(row_text(&buf, 0).trim_start().starts_with("9998 "), "got {:?}", row_text(&buf, 0));
}

#[test]
fn the_minimum_width_is_zero_by_default() {
    let textarea = plain(&["a"]);

    assert_eq!(textarea.min_line_number_width(), 0);
}

/// The gutter width feeds the cursor's screen column as well as the rendered
/// text. Sizing only the render would leave the cursor drawn `min - natural`
/// columns left of the character it is actually on.
#[test]
fn a_minimum_width_shifts_the_cursor_column_too() {
    fn cursor_column(textarea: &TextArea<'_>) -> Option<u16> {
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        Widget::render(textarea, area, &mut buf);
        (0..area.width).find(|&x| {
            buf[(x, 0)]
                .style()
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED)
        })
    }

    // Not `plain`: that hides the cursor, which is the thing under test.
    let mut narrow = TextArea::new(vec!["ab".to_string()]);
    narrow.set_line_number_style(Style::default());
    let narrow_col = cursor_column(&narrow).expect("cursor not drawn without a minimum");

    let mut wide = TextArea::new(vec!["ab".to_string()]);
    wide.set_line_number_style(Style::default());
    wide.set_min_line_number_width(6);
    let wide_col = cursor_column(&wide).expect("cursor not drawn with a minimum");

    assert_eq!(
        wide_col - narrow_col,
        5,
        "cursor column did not follow the reserved gutter ({wide_col} vs {narrow_col})"
    );
}
