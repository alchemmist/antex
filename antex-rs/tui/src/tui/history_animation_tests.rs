use super::*;
use crate::insert_history::insert_history_lines;
use crate::terminal_hyperlinks::plain_hyperlink_lines;
use crate::test_backend::VT100Backend;
use pretty_assertions::assert_eq;
use ratatui::layout::Position;
use ratatui::text::Line;

#[test]
fn repaint_header_preserves_shell_notices_and_composer() {
    let backend = VT100Backend::with_scrollback(40, 16, 32);
    let mut terminal =
        Terminal::with_options_and_cursor_position(backend, Position::new(0, 2)).unwrap();
    write!(terminal.backend_mut(), "SHELL HISTORY\r\n$ antex\r\n").unwrap();
    terminal.set_viewport_area(Rect::new(0, 2, 40, 3));
    let previous = plain_hyperlink_lines(vec![Line::from("ant resting"), Line::from("header")]);
    let next = plain_hyperlink_lines(vec![Line::from("ANT moving"), Line::from("header")]);
    insert_history_lines(
        &mut terminal,
        vec![
            "ant resting".into(),
            "header".into(),
            "startup notice".into(),
        ],
    )
    .unwrap();
    terminal
        .draw(|frame| {
            let y = frame.area().y;
            frame
                .buffer_mut()
                .set_string(0, y, "composer draft", ratatui::style::Style::default());
        })
        .unwrap();
    let viewport = terminal.viewport_area;
    let cursor = terminal.last_known_cursor_pos;
    repaint_history_block(
        &mut terminal,
        &previous,
        &next,
        1,
        HistoryLineWrapPolicy::PreWrap,
    )
    .unwrap();
    assert_eq!(terminal.viewport_area, viewport);
    assert_eq!(terminal.last_known_cursor_pos, cursor);
    insta::assert_snapshot!(terminal.backend().vt100().screen().contents());
}

#[test]
fn clear_visible_history_preserves_shell_prefix() {
    let backend = VT100Backend::new(40, 16);
    let mut terminal =
        Terminal::with_options_and_cursor_position(backend, Position::new(0, 2)).unwrap();
    write!(terminal.backend_mut(), "SHELL HISTORY\r\n$ antex\r\n").unwrap();
    terminal.set_viewport_area(Rect::new(0, 2, 40, 3));
    insert_history_lines(
        &mut terminal,
        vec!["old header".into(), "old warning".into()],
    )
    .unwrap();
    terminal.clear_visible_history().unwrap();
    assert_eq!(
        terminal.backend().vt100().screen().contents(),
        "SHELL HISTORY\n$ antex"
    );
    assert_eq!(terminal.viewport_area.y, 2);
}
