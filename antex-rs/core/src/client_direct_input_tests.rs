use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn agent_messages_preserve_text_order_and_leave_saved_history_unchanged() -> anyhow::Result<()> {
    let saved: Vec<ResponseItem> = serde_json::from_value(json!([
        {"type":"message","role":"user","content":[{"type":"input_text","text":"Continue."}]},
        {"type":"agent_message","author":"/root/worker","recipient":"/root","content":[
            {"type":"input_text","text":"Message Type: MESSAGE\nTask name: /root\nSender: /root/worker\nPayload:\n"},
            {"type":"input_text","text":"Result ✓"}
        ]},
        {"type":"function_call_output","call_id":"call_1","output":"done"}
    ]))?;
    let original = saved.clone();
    let mut request = saved.clone();
    prepare_agent_messages(&mut request)?;
    let mut expected = original.clone();
    expected[1] = serde_json::from_value(json!({
        "type":"message","role":"assistant","content":[
            {"type":"output_text","text":"Message Type: MESSAGE\nTask name: /root\nSender: /root/worker\nPayload:\n"},
            {"type":"output_text","text":"Result ✓"}
        ]
    }))?;
    assert_eq!(request, expected);
    assert_eq!(saved, original);
    prepare_agent_messages(&mut request)?;
    assert_eq!(request, expected);
    Ok(())
}

#[test]
fn repeated_mixed_agent_messages_keep_distinct_call_ids_and_all_parts() -> anyhow::Result<()> {
    let item: ResponseItem = serde_json::from_value(json!({
        "type":"agent_message","author":"/root/a","recipient":"/root","content":[
            {"type":"input_text","text":"Before"},
            {"type":"encrypted_content","encrypted_content":"opaque-one"},
            {"type":"input_text","text":"Between"},
            {"type":"encrypted_content","encrypted_content":"opaque-two"},
            {"type":"input_text","text":"After"}
        ]
    }))?;
    let mut request = vec![item.clone(), item];
    prepare_agent_messages(&mut request)?;
    let mut recovered = Vec::new();
    let mut call_ids = std::collections::HashSet::new();
    let mut schemas = 0;
    for item in &request {
        match item {
            ResponseItem::Message { content, .. } => {
                for part in content {
                    if let ContentItem::OutputText { text, .. } = part {
                        recovered.push(text.clone());
                    }
                }
            }
            ResponseItem::FunctionCall {
                call_id, arguments, ..
            } => {
                assert!(call_ids.insert(call_id.clone()));
                let args: serde_json::Value = serde_json::from_str(arguments)?;
                assert_eq!(
                    (&args["author"], &args["recipient"]),
                    (&json!("/root/a"), &json!("/root"))
                );
                recovered.push(args["message"].as_str().unwrap().to_string());
            }
            ResponseItem::AdditionalTools { .. } => schemas += 1,
            ResponseItem::FunctionCallOutput { .. } => {}
            _ => panic!("unexpected history item"),
        }
    }
    assert_eq!(schemas, 1);
    assert_eq!(
        recovered,
        [
            "Before",
            "opaque-one",
            "Between",
            "opaque-two",
            "After",
            "Before",
            "opaque-one",
            "Between",
            "opaque-two",
            "After"
        ]
    );
    Ok(())
}

#[test]
fn encrypted_agent_task_preserves_every_part_in_order_without_rewriting_history()
-> anyhow::Result<()> {
    let mut request: Vec<ResponseItem> = serde_json::from_value(json!([{
        "type":"agent_message","author":"/root","recipient":"/root/worker","content":[
            {"type":"input_text","text":"Task metadata"},
            {"type":"encrypted_content","encrypted_content":"private-opaque-task"}
        ]
    }]))?;
    let original = request.clone();
    prepare_agent_messages(&mut request)?;
    assert_eq!(request.len(), 4);
    let body = serde_json::to_value(&request)?;
    assert_eq!(body[0]["content"][0]["text"], "Task metadata");
    assert_eq!(body[1]["type"], "additional_tools");
    assert_eq!(
        body[1]["tools"][0]["parameters"]["properties"]["message"]["encrypted"],
        true
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body[2]["arguments"].as_str().unwrap())?,
        json!({"author":"/root","recipient":"/root/worker","message":"private-opaque-task"})
    );
    assert_eq!(body[2]["call_id"], body[3]["call_id"]);
    assert_eq!(body[3]["output"], "");
    let mut retry = original.clone();
    prepare_agent_messages(&mut retry)?;
    assert_eq!(retry, request);
    prepare_agent_messages(&mut retry)?;
    assert_eq!(retry, request);
    assert!(matches!(original[0], ResponseItem::AgentMessage { .. }));
    Ok(())
}
