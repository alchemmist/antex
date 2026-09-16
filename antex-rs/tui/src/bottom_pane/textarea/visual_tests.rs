use super::TextArea;
use super::VimMode;
use super::VimVisualKind;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::WidgetRef;

fn visual_textarea(text: &str, cursor: usize) -> TextArea {
    let mut textarea = TextArea::new();
    textarea.insert_str(text);
    textarea.set_cursor(cursor);
    textarea.set_vim_enabled(/*enabled*/ true);
    textarea
}

fn keys(textarea: &mut TextArea, keys: &str) {
    for key in keys.chars() {
        textarea.input(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
    }
}

#[test]
fn character_visual_mode_moves_deletes_yanks_and_changes() {
    let mut textarea = visual_textarea("alpha beta", /*cursor*/ 0);
    keys(&mut textarea, "vwd");
    assert_eq!(textarea.text(), "eta");
    assert_eq!(textarea.kill_buffer, "alpha b");
    assert_eq!(textarea.vim_mode_label(), Some("Normal"));

    let mut textarea = visual_textarea("alpha beta", /*cursor*/ 0);
    keys(&mut textarea, "vey");
    assert_eq!(textarea.text(), "alpha beta");
    assert_eq!(textarea.kill_buffer, "alpha");
    assert_eq!(
        textarea.take_system_clipboard_yank().as_deref(),
        Some("alpha")
    );
    assert_eq!(textarea.cursor(), 0);

    keys(&mut textarea, "vwc");
    assert_eq!(textarea.text(), "eta");
    assert_eq!(textarea.vim_mode, VimMode::Insert);
}

#[test]
fn line_visual_mode_operates_on_complete_lines() {
    let mut textarea = visual_textarea("one\ntwo\nthree", /*cursor*/ 0);
    textarea.input(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT));
    keys(&mut textarea, "jd");

    assert_eq!(textarea.text(), "three");
    assert_eq!(textarea.kill_buffer, "one\ntwo\n");
    assert_eq!(textarea.take_system_clipboard_yank().as_deref(), None);
    assert_eq!(textarea.vim_mode_label(), Some("Normal"));
}

#[test]
fn block_visual_mode_operates_on_display_columns() {
    let mut textarea = visual_textarea("abc\ndef\nghi", /*cursor*/ 0);
    textarea.input(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    keys(&mut textarea, "ljd");

    assert_eq!(textarea.text(), "c\nf\nghi");
    assert_eq!(textarea.kill_buffer, "ab\nde");
    assert_eq!(textarea.vim_mode_label(), Some("Normal"));
}

#[test]
fn visual_mode_supports_russian_layout_and_escape() {
    let mut textarea = visual_textarea("alpha beta", /*cursor*/ 0);
    keys(&mut textarea, "мц");
    assert_eq!(textarea.vim_mode_label(), Some("Visual"));
    assert_eq!(textarea.cursor(), "alpha ".len());

    textarea.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(textarea.vim_mode_label(), Some("Normal"));
    assert_eq!(textarea.vim_visual, None);
}

#[test]
fn visual_selection_rendering_uses_quiet_terminal_gray() {
    let mut textarea = visual_textarea("alpha beta", /*cursor*/ 0);
    keys(&mut textarea, "ve");
    assert_eq!(
        textarea.vim_visual.map(|state| state.kind),
        Some(VimVisualKind::Character)
    );

    let area = Rect::new(0, 0, /*width*/ 16, /*height*/ 1);
    let mut buffer = Buffer::empty(area);
    WidgetRef::render_ref(&&textarea, area, &mut buffer);
    let rendered = (0..area.width)
        .take_while(|x| buffer[(*x, 0)].symbol() != " ")
        .map(|x| {
            let cell = &buffer[(x, 0)];
            if cell.style().bg == Some(Color::DarkGray) {
                format!("[{}]", cell.symbol())
            } else {
                cell.symbol().to_string()
            }
        })
        .collect::<String>();

    insta::assert_snapshot!(rendered, @"[a][l][p][h][a]");
}

#[test]
fn visual_case_commands_transform_unicode_selection_and_exit() {
    let mut textarea = visual_textarea("straße привет tail", /*cursor*/ 0);
    keys(&mut textarea, "vegU");
    assert_eq!(textarea.text(), "STRASSE привет tail");
    assert_eq!(textarea.vim_mode_label(), Some("Normal"));
    assert_eq!(textarea.cursor(), 0);
    keys(&mut textarea, "vegu");
    assert_eq!(textarea.text(), "strasse привет tail");
    keys(&mut textarea, "wvegU");
    assert_eq!(textarea.text(), "strasse ПРИВЕТ tail");
    let area = Rect::new(0, 0, /*width*/ 24, /*height*/ 1);
    let mut buffer = Buffer::empty(area);
    WidgetRef::render_ref(&&textarea, area, &mut buffer);
    let rendered = (0..area.width)
        .map(|x| buffer[(x, 0)].symbol())
        .collect::<String>();
    insta::assert_snapshot!(rendered.trim_end(), @"strasse ПРИВЕТ tail");
}

#[test]
fn visual_case_prefix_cancels_on_escape_and_unknown_key() {
    let mut textarea = visual_textarea("hello", /*cursor*/ 0);
    keys(&mut textarea, "veg");
    textarea.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    keys(&mut textarea, "0vegx");
    assert_eq!(textarea.text(), "hello");
    assert!(textarea.is_vim_visual_mode());
    keys(&mut textarea, "gU");
    assert_eq!(textarea.text(), "HELLO");
}

#[test]
fn line_and_block_visual_case_commands_preserve_unselected_text() {
    let mut textarea = visual_textarea("ab cd\nef gh\nkeep", /*cursor*/ 0);
    textarea.input(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    keys(&mut textarea, "ljgU");
    assert_eq!(textarea.text(), "AB cd\nEF gh\nkeep");
    textarea.input(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT));
    keys(&mut textarea, "jgu");
    assert_eq!(textarea.text(), "ab cd\nef gh\nkeep");
}

#[test]
fn visual_case_change_preserves_attachment_markers() {
    let mut textarea = visual_textarea("before ", /*cursor*/ 7);
    textarea.insert_element("[photo.png]");
    textarea.insert_str(" after");
    textarea.set_cursor(0);
    keys(&mut textarea, "v$g");
    textarea.input(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT));
    assert_eq!(textarea.text(), "BEFORE [photo.png] AFTER");
    assert_eq!(textarea.elements.len(), 1);
    assert_eq!(
        &textarea.text()[textarea.elements[0].range.clone()],
        "[photo.png]"
    );
}
