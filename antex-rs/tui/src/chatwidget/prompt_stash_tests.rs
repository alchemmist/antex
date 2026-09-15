use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;

use super::*;
use crate::chatwidget::tests::make_chatwidget_manual_with_sender;

fn ctrl_s() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
}

#[tokio::test]
async fn ctrl_s_stashes_and_appends_the_prompt_on_restore() {
    let (mut chat, _sender, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
    chat.bottom_pane
        .set_composer_text("stashed prompt".to_string(), Vec::new(), Vec::new());

    chat.handle_key_event(ctrl_s());

    assert_eq!("", chat.bottom_pane.composer_text());
    assert_eq!(
        Some("stashed prompt"),
        chat.stashed_composer
            .as_ref()
            .map(|composer| composer.text.as_str())
    );

    chat.bottom_pane
        .set_composer_text("current prompt: ".to_string(), Vec::new(), Vec::new());
    chat.handle_key_event(ctrl_s());

    assert_eq!(
        "current prompt: stashed prompt",
        chat.bottom_pane.composer_text()
    );
    assert_eq!(None, chat.stashed_composer);
}

#[tokio::test]
async fn prompt_stash_survives_chatwidget_recreation() {
    let thread_id = ThreadId::new();
    let (mut first_chat, _sender, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
    first_chat.thread_id = Some(thread_id);
    first_chat.bottom_pane.set_composer_text(
        "persisted prompt".to_string(),
        Vec::new(),
        Vec::new(),
    );

    first_chat.handle_key_event(ctrl_s());

    let expected = first_chat
        .stashed_composer
        .clone()
        .expect("in-memory stash");
    let codex_home = first_chat.config.codex_home.clone();
    let stash_path = first_chat.prompt_stash_path(thread_id);
    assert!(stash_path.exists());
    drop(first_chat);

    let (mut resumed_chat, _sender, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
    resumed_chat.config.codex_home = codex_home;
    resumed_chat.thread_id = Some(thread_id);
    resumed_chat
        .restore_persisted_prompt_stash(thread_id)
        .expect("restore persisted stash");

    assert_eq!(Some(&expected), resumed_chat.stashed_composer.as_ref());
    resumed_chat.handle_key_event(ctrl_s());
    assert_eq!("persisted prompt", resumed_chat.bottom_pane.composer_text());
    assert_eq!(None, resumed_chat.stashed_composer);
    assert!(!stash_path.exists());
}

#[tokio::test]
async fn persisted_prompt_stash_roundtrips_full_state() {
    let thread_id = ThreadId::new();
    let (mut first_chat, _sender, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
    first_chat.thread_id = Some(thread_id);
    let expected = ThreadComposerState {
        text: "persisted prompt".to_string(),
        local_images: vec![LocalImageAttachment {
            placeholder: "[Image #1]".to_string(),
            path: PathBuf::from("/tmp/persisted-image.png"),
        }],
        remote_image_urls: vec!["https://example.com/image.png".to_string()],
        text_elements: vec![TextElement::new(
            (0..9).into(),
            Some("persisted".to_string()),
        )],
        mention_bindings: vec![MentionBinding {
            sigil: '@',
            mention: "project".to_string(),
            path: "/tmp/project".to_string(),
        }],
        pending_pastes: vec![("[Pasted Content 5 chars]".to_string(), "alpha".to_string())],
    };
    first_chat.stashed_composer = Some(expected.clone());
    first_chat
        .persist_prompt_stash()
        .expect("persist full stash state");

    let codex_home = first_chat.config.codex_home.clone();
    drop(first_chat);

    let (mut resumed_chat, _sender, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
    resumed_chat.config.codex_home = codex_home;
    resumed_chat.thread_id = Some(thread_id);
    resumed_chat
        .restore_persisted_prompt_stash(thread_id)
        .expect("restore persisted stash");

    assert_eq!(Some(&expected), resumed_chat.stashed_composer.as_ref());
}

#[tokio::test]
async fn restore_remaps_colliding_large_paste_placeholders() {
    let (mut chat, _sender, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
    let placeholder = "[Pasted Content 5 chars]";
    chat.restore_composer_state(ThreadComposerState {
        text: placeholder.to_string(),
        text_elements: vec![TextElement::new(
            (0..placeholder.len()).into(),
            Some(placeholder.to_string()),
        )],
        pending_pastes: vec![(placeholder.to_string(), "alpha".to_string())],
        ..ThreadComposerState::default()
    });
    chat.handle_key_event(ctrl_s());

    chat.restore_composer_state(ThreadComposerState {
        text: placeholder.to_string(),
        text_elements: vec![TextElement::new(
            (0..placeholder.len()).into(),
            Some(placeholder.to_string()),
        )],
        pending_pastes: vec![(placeholder.to_string(), "bravo".to_string())],
        ..ThreadComposerState::default()
    });
    chat.handle_key_event(ctrl_s());

    assert_eq!(
        format!("{placeholder}{placeholder} #2"),
        chat.bottom_pane.composer_text()
    );
    assert_eq!(
        vec![
            (placeholder.to_string(), "bravo".to_string()),
            (format!("{placeholder} #2"), "alpha".to_string()),
        ],
        chat.bottom_pane.composer_pending_pastes()
    );
    assert_eq!(None, chat.stashed_composer);
}

#[tokio::test]
async fn ctrl_s_keeps_history_search_ownership() {
    let (mut chat, _sender, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
    chat.bottom_pane
        .set_composer_text("search draft".to_string(), Vec::new(), Vec::new());
    chat.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert!(chat.bottom_pane.composer_history_search_active());
    chat.handle_key_event(ctrl_s());

    assert!(chat.bottom_pane.composer_history_search_active());
    assert_eq!(None, chat.stashed_composer);
}
