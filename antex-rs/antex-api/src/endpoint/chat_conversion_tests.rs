#![allow(clippy::expect_used)]

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn eliza_namespaced_custom_tools_round_trip() -> Result<(), ApiError> {
    let request = ChatRequest::new(
        json!({"model":"test","input":[{"type":"message","role":"developer","content":[{"type":"input_text","text":"instructions"}]}],"tools":[{"type":"namespace","name":"functions","tools":[{"type":"custom","name":"exec","description":"Run JavaScript"}]}]}),
    )?;
    let name = request.body["tools"][0]["function"]["name"].clone();
    let items = request.output(&json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"call1","function":{"name":name,"arguments":"{\"input\":\"text(1)\"}"}}]}}]}))?;
    let item = serde_json::to_value(&items[0]).map_err(|_| invalid("test serialization"))?;
    assert_eq!(
        json!([item["type"], item["name"], item["namespace"], item["input"]]),
        json!(["custom_tool_call", "exec", "functions", "text(1)"])
    );
    let next = ChatRequest::new(
        json!({"model":"test","tools":[],"input":[item,{"type":"custom_tool_call_output","call_id":"call1","output":[{"type":"input_text","text":"1"}]}]}),
    )?;
    assert_eq!(
        next.body["messages"],
        json!([{"role":"assistant","content":null,"tool_calls":[{"id":"call1","type":"function","function":{"name":name,"arguments":"{\"input\":\"text(1)\"}"}}]},{"role":"tool","tool_call_id":"call1","content":"1"}])
    );
    Ok(())
}

#[test]
fn eliza_rejects_unsupported_content_and_incomplete_outputs() {
    assert!(ChatRequest::new(json!({"model":"test","input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"secret"}]}]})).is_err());
    let request = ChatRequest::new(json!({"model":"test","input":[]})).expect("valid request");
    for reason in ["length", "content_filter"] {
        assert!(
            request
                .output(
                    &json!({"choices":[{"finish_reason":reason,"message":{"content":"partial"}}]})
                )
                .is_err()
        );
    }
    assert!(request.output(&json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"call1","function":{"name":"unknown","arguments":"{}"}}]}}]})).is_err());
}

#[test]
fn eliza_replays_plaintext_agents_without_old_discovery_metadata() -> Result<(), ApiError> {
    let request = ChatRequest::new(json!({"model":"test","input":[
        {"type":"tool_search_call","arguments":{"query":"shell"}},
        {"type":"tool_search_output","tools":[]},
        {"type":"agent_message","content":[{"type":"input_text","text":"The command succeeded"}]}
    ]}))?;
    assert_eq!(
        request.body["messages"],
        json!([{"role":"assistant","content":"The command succeeded"}])
    );
    assert!(ChatRequest::new(json!({"model":"test","input":[{"type":"agent_message","content":[{"type":"encrypted_content","encrypted_content":"opaque"}]}]})).is_err());
    Ok(())
}

#[test]
fn eliza_places_system_messages_before_conversation_history() -> Result<(), ApiError> {
    let request = ChatRequest::new(json!({"model":"test","tools":[],"input":[
        {"type":"message","role":"user","content":[{"type":"input_text","text":"Earlier user message"}]},
        {"type":"message","role":"developer","content":[{"type":"input_text","text":"First instruction"}]},
        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Earlier answer"}]},
        {"type":"message","role":"system","content":[{"type":"input_text","text":"Second instruction"}]},
        {"type":"message","role":"user","content":[{"type":"input_text","text":"Current user message"}]}
    ]}))?;

    assert_eq!(
        request.body["messages"],
        json!([
            {"role":"system","content":"First instruction\n\nSecond instruction"},
            {"role":"user","content":"Earlier user message"},
            {"role":"assistant","content":"Earlier answer"},
            {"role":"user","content":"Current user message"}
        ])
    );
    Ok(())
}
