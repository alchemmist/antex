use super::*;
use crate::test_backend::VT100Backend;
use pretty_assertions::assert_eq;

#[test]
fn missing_cursor_report_keeps_existing_output() {
    let mut backend = VT100Backend::with_scrollback(40, 12, 32);
    write!(backend, "SHELL HISTORY\r\n$ antex\r\n").unwrap();
    let position = resolve_cursor_position(&mut backend, None).unwrap();
    assert_eq!(position, Position::new(0, 11));
    let mut screen = backend.vt100().screen().clone();
    let visible = screen.contents();
    screen.set_scrollback(usize::MAX);
    let retained = format!("{}\n{visible}", screen.contents());
    assert!(retained.contains("SHELL HISTORY"));
    assert!(retained.contains("$ antex"));
}

#[test]
fn cursor_report_keeps_the_shell_position() {
    let mut backend = VT100Backend::new(40, 12);
    write!(backend, "SHELL HISTORY\r\n$ antex\r\n").unwrap();
    let before = backend.vt100().screen().contents();
    assert_eq!(
        resolve_cursor_position(&mut backend, Some(Position::new(0, 2))).unwrap(),
        Position::new(0, 2)
    );
    assert_eq!(backend.vt100().screen().contents(), before);
}
