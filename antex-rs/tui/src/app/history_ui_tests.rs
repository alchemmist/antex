//! Desktop handoff output and clear-screen history reset behavior.

use super::*;
use crate::app::tests::make_test_app_with_channels;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use crate::history_cell::PlainHistoryCell;
use pretty_assertions::assert_eq;

#[test]
fn desktop_thread_opened_history_snapshot() {
    let cell = history_cell::new_info_event(
        DESKTOP_THREAD_OPENED_MESSAGE.to_string(),
        /*hint*/ None,
    );

    insta::assert_snapshot!("desktop_thread_opened_history", render_cell(&cell));
}

#[test]
fn desktop_thread_open_error_history_snapshot() {
    let cell = history_cell::new_error_event(desktop_thread_open_error_message("launch failed"));

    insta::assert_snapshot!("desktop_thread_open_error_history", render_cell(&cell));
}

fn render_cell(cell: &impl HistoryCell) -> String {
    let lines = cell.display_lines(/*width*/ 80);
    lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn interrupted_prompt_removal_prunes_native_queue_in_both_screen_modes() -> Result<()> {
    for owned in [false, true] {
        let (mut app, _rx, _op_rx) = make_test_app_with_channels().await;
        let mut tui = crate::tui::test_support::make_test_tui()?;
        tui.set_owned_screen(owned)?;
        let prompt = |message: &str| -> Arc<dyn HistoryCell> {
            Arc::new(history_cell::UserHistoryCell {
                message: message.to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            })
        };
        let retained = prompt("keep this prompt");
        let interrupted = prompt("interrupted prompt");
        app.transcript_cells = vec![retained.clone(), interrupted.clone()];
        app.native_history.defer(&interrupted);
        app.remove_interrupted_user_prompt(
            &mut tui,
            &crate::chatwidget::ChatWidget::user_message_display_from_inputs(&[
                antex_app_server_protocol::UserInput::Text {
                    text: "interrupted prompt".to_string(),
                    text_elements: Vec::new(),
                },
            ]),
        );

        assert_eq!(app.transcript_cells.len(), 1);
        assert!(Arc::ptr_eq(&app.transcript_cells[0], &retained));
        assert!(!app.native_history.contains(&interrupted));
        app.flush_native_history(&mut tui);
        if owned {
            assert!(tui.pending_history_lines_for_test().is_empty());
        }
        tui.set_owned_screen(/*owned*/ false)?;
    }
    Ok(())
}

#[tokio::test]
async fn owned_theme_change_does_not_queue_native_reflow() -> Result<()> {
    let (mut app, _rx, _op_rx) = make_test_app_with_channels().await;
    let mut tui = crate::tui::test_support::make_test_tui()?;
    tui.set_owned_screen(/*owned*/ true)?;
    app.insert_history_cell(
        &mut tui,
        Box::new(PlainHistoryCell::new(vec!["retained".into()])),
    );
    app.schedule_theme_change_reflow(&mut tui);
    assert!(!app.transcript_reflow.has_pending_reflow());
    assert!(tui.pending_history_lines_for_test().is_empty());
    assert_eq!(app.transcript_cells.len(), 1);
    tui.set_owned_screen(/*owned*/ false)?;
    Ok(())
}

#[tokio::test]
async fn owned_clear_resets_navigation_and_retains_one_fresh_header() -> Result<()> {
    let (mut app, _rx, _op_rx) = make_test_app_with_channels().await;
    let mut tui = crate::tui::test_support::make_test_tui()?;
    tui.set_owned_screen(/*owned*/ true)?;
    app.insert_history_cell(
        &mut tui,
        Box::new(PlainHistoryCell::new(vec!["old".into()])),
    );
    app.transcript_view
        .jump_to_entry(&app.transcript_cells, /*index*/ 0);
    app.reset_transcript_state_after_clear();
    app.queue_clear_ui_header(&mut tui);
    app.queue_clear_ui_header(&mut tui);

    assert_eq!(app.transcript_cells.len(), 1);
    assert_eq!(
        app.transcript_cells[0].display_lines(/*width*/ 80),
        app.clear_ui_header_lines(/*width*/ 80),
    );
    app.chat_widget.set_raw_output_mode(/*enabled*/ true);
    assert_eq!(
        app.transcript_cells[0].raw_lines(),
        app.clear_ui_header_lines(/*width*/ 80),
    );
    assert!(app.transcript_view.is_following());
    assert!(tui.pending_history_lines_for_test().is_empty());
    tui.set_owned_screen(/*owned*/ false)?;
    Ok(())
}
