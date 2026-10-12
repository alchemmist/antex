use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;

use super::render_html_transcript;
use crate::history_cell::AgentMarkdownCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::PlainHistoryCell;
use crate::history_cell::UserHistoryCell;
use crate::history_cell::new_proposed_plan;

#[test]
fn html_dump_groups_actions_and_sanitizes_model_markdown() {
    let user = Arc::new(UserHistoryCell {
        message: "How do I run **tests**?".to_string(),
        text_elements: Vec::new(),
        local_image_paths: Vec::new(),
        remote_image_urls: Vec::new(),
    }) as Arc<dyn HistoryCell>;
    let cells = vec![
        user,
        Arc::new(PlainHistoryCell::new(vec!["$ just test".into()])) as Arc<dyn HistoryCell>,
        Arc::new(PlainHistoryCell::new(vec!["all tests passed".into()])) as Arc<dyn HistoryCell>,
        Arc::new(AgentMarkdownCell::new(
            "Done. <script>alert('x')</script> [unsafe](javascript:alert(1))".to_string(),
            std::path::Path::new("."),
        )) as Arc<dyn HistoryCell>,
        Arc::new(new_proposed_plan(
            "1. Keep going".to_string(),
            std::path::Path::new("."),
        )) as Arc<dyn HistoryCell>,
    ];
    let exported_at = DateTime::parse_from_rfc3339("2026-08-21T12:34:56Z")
        .expect("timestamp")
        .with_timezone(&Utc);

    let html =
        render_html_transcript(&cells, "01a02451-3063-7290", exported_at).expect("HTML transcript");

    assert!(html.contains("<summary>2 actions</summary>"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("&lt;/script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("javascript:"));
    insta::assert_snapshot!("html_transcript", html);
}

#[test]
fn html_dump_rejects_an_empty_conversation() {
    let exported_at = DateTime::parse_from_rfc3339("2026-08-21T12:34:56Z")
        .expect("timestamp")
        .with_timezone(&Utc);

    assert!(render_html_transcript(&[], "thread", exported_at).is_err());
}
