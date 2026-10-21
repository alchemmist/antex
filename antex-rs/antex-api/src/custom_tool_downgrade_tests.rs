use super::*;
use antex_protocol::models::FunctionCallOutputBody;
use antex_protocol::models::FunctionCallOutputPayload;
use pretty_assertions::assert_eq;
use serde_json::json;

fn request(input: Vec<ResponseItem>, tools: Value) -> ResponsesApiRequest {
    let raw = RawValue::from_string(tools.to_string()).expect("tools");
    ResponsesApiRequest {
        model: "qwen".to_string(),
        stream: true,
        service_tier: None,
        input,
        tools: Some(ResponsesApiTools::from(Arc::from(raw))),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: false,
        stream_options: None,
        include: Vec::new(),
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
        access_programs: None,
    }
}

#[test]
fn custom_history_and_tools_become_functions() {
    let mut request = request(
        vec![ResponseItem::CustomToolCall {
            encrypted_content: None,
            id: None,
            status: None,
            call_id: "call-1".to_string(),
            name: "exec".to_string(),
            namespace: None,
            input: "echo hi".to_string(),
            internal_chat_message_metadata_passthrough: None,
        }],
        json!([
            {"type":"custom","name":"exec","description":"Run","format":{"type":"grammar"}},
            {"type":"web_search"},
            {"type":"namespace","name":"ns","tools":[{"type":"custom","name":"lookup","format":{"type":"text"}}]}
        ]),
    );
    downgrade_custom_tools(&mut request);
    assert_eq!(
        request.input,
        vec![ResponseItem::FunctionCall {
            encrypted_content: None,
            status: None,
            id: None,
            name: "exec".to_string(),
            namespace: None,
            arguments: json!({"input":"echo hi"}).to_string(),
            encrypted_function_args: None,
            call_id: "call-1".to_string(),
            internal_chat_message_metadata_passthrough: None,
        }]
    );
    let tools: Value =
        serde_json::from_str(request.tools.as_ref().expect("tools").as_raw_value().get())
            .expect("json");
    assert_eq!(
        tools,
        json!([
            {
                "type":"function",
                "name":"exec",
                "description":"Run",
                "parameters":{
                    "type":"object",
                    "properties":{"input":{"type":"string","description":"Exact freeform tool input"}},
                    "required":["input"],
                    "additionalProperties":false
                }
            },
            {
                "type":"namespace",
                "name":"ns",
                "tools":[{
                    "type":"function",
                    "name":"lookup",
                    "parameters":{
                        "type":"object",
                        "properties":{"input":{"type":"string","description":"Exact freeform tool input"}},
                        "required":["input"],
                        "additionalProperties":false
                    }
                }]
            }
        ])
    );
}

#[test]
fn custom_output_becomes_function_output() {
    let mut request = request(
        vec![ResponseItem::CustomToolCallOutput {
            id: None,
            call_id: "call-1".to_string(),
            name: Some("exec".to_string()),
            output: FunctionCallOutputPayload {
                body: FunctionCallOutputBody::Text("ok".to_string()),
                success: Some(true),
            },
            internal_chat_message_metadata_passthrough: None,
        }],
        json!([]),
    );
    downgrade_custom_tools(&mut request);
    assert!(matches!(
        request.input.first(),
        Some(ResponseItem::FunctionCallOutput { call_id: Some(call_id), .. }) if call_id == "call-1"
    ));
}
