use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn desktop_question_reply_accepts_single_and_batched_envelopes() {
    let reply = json!({"questionItemId": "[\"request_user_input_async\",\"item\",0]", "question": "Which environment?", "answer": "Staging", "extra": true});
    for payload in [reply.clone(), json!([reply])] {
        let text = format!(
            " \n<send_user_message_question_reply>\n{payload}\n</send_user_message_question_reply>\n "
        );
        assert_eq!(
            display_text(&text).as_deref(),
            Some("> Which environment?\n\nStaging")
        );
    }
    let text = "<send_user_message_question_reply>[{\"questionItemId\":\"one\",\"question\":\"First?\",\"answer\":\"Yes\"},{\"questionItemId\":\"two\",\"question\":\"Second?\",\"answer\":\"No\"}]</send_user_message_question_reply>";
    assert_eq!(
        display_text(text).as_deref(),
        Some("> First?\n\nYes\n\n> Second?\n\nNo")
    );
}

#[test]
fn ide_prefixed_replies_accept_current_and_legacy_delimiters() {
    for brand in ["Antex", "Codex"] {
        let answer = format!("Keep literal\n## My request for {brand}:\ntext");
        let reply = json!({"questionItemId": "one", "question": "Where?", "answer": answer});
        let text = format!(
            "# Context from my IDE setup:\n\n## Active file: lib.rs\n\n## My request for {brand}:\n<send_user_message_question_reply>{reply}</send_user_message_question_reply>"
        );
        assert_eq!(display_text(&text), Some(format!("> Where?\n\n{answer}")));
    }
}

#[test]
fn malformed_or_embedded_question_envelopes_remain_ordinary_text() {
    for text in [
        "> Which environment?\n\nStaging",
        "<send_user_message_question_reply>[]</send_user_message_question_reply>",
        "<send_user_message_question_reply>[{\"questionItemId\":\"one\",\"question\":\"First?\",\"answer\":\"Yes\"},null]</send_user_message_question_reply>",
        "Quoted: <send_user_message_question_reply>{\"questionItemId\":\"one\",\"question\":\"First?\",\"answer\":\"Yes\"}</send_user_message_question_reply>",
        "<send_user_message_question_reply>{\"questionItemId\":\"one\",\"question\":\"First?\",\"answer\":\"Yes\"}</send_user_message_question_reply> trailing text",
    ] {
        assert_eq!(parse(text), None, "{text}");
    }
}
