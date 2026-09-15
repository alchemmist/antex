use super::*;

#[test]
fn paste_image_shortcut_matches_raw_ctrl_v_control_character() {
    for key_event in [
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SUPER),
        KeyEvent::new(KeyCode::Char('\u{16}'), KeyModifiers::NONE),
    ] {
        assert!(is_paste_image_key_event(key_event));
    }
}
