use crate::common::ResponsesApiRequest;
use crate::common::ResponsesApiTools;
use antex_protocol::models::ResponseItem;
use serde_json::Value;
use serde_json::value::RawValue;
use std::sync::Arc;

pub fn downgrade_custom_tools(request: &mut ResponsesApiRequest) {
    request.input = std::mem::take(&mut request.input)
        .into_iter()
        .map(downgrade_item)
        .collect();
    let Some(tools) = request.tools.as_ref() else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(tools.as_raw_value().get()) else {
        return;
    };
    if let Some(entries) = value.as_array_mut() {
        downgrade_tools(entries);
    }
    let Ok(raw) = RawValue::from_string(value.to_string()) else {
        return;
    };
    request.tools = Some(ResponsesApiTools::from(Arc::from(raw)));
}

fn downgrade_item(item: ResponseItem) -> ResponseItem {
    match item {
        ResponseItem::CustomToolCall {
            encrypted_content,
            id,
            status,
            call_id,
            name,
            namespace,
            input,
            internal_chat_message_metadata_passthrough,
        } => ResponseItem::FunctionCall {
            encrypted_content,
            status,
            id,
            name,
            namespace,
            arguments: serde_json::json!({ "input": input }).to_string(),
            encrypted_function_args: None,
            call_id,
            internal_chat_message_metadata_passthrough,
        },
        ResponseItem::CustomToolCallOutput {
            id,
            call_id,
            name,
            output,
            internal_chat_message_metadata_passthrough,
        } => ResponseItem::FunctionCallOutput {
            id,
            call_id: Some(call_id),
            name,
            namespace: None,
            output,
            internal_chat_message_metadata_passthrough,
        },
        other => other,
    }
}

fn downgrade_tools(tools: &mut Vec<Value>) {
    let mut index = 0;
    while index < tools.len() {
        let kind = tools[index]["type"].as_str().unwrap_or_default();
        if kind == "namespace" {
            if let Some(children) = tools[index]["tools"].as_array_mut() {
                downgrade_tools(children);
            }
            if tools[index]["tools"].as_array().is_some_and(Vec::is_empty) {
                tools.remove(index);
                continue;
            }
        } else if kind == "custom" {
            let tool = &mut tools[index];
            tool["type"] = Value::String("function".to_string());
            tool["parameters"] = serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Exact freeform tool input"
                    }
                },
                "required": ["input"],
                "additionalProperties": false
            });
            if let Some(object) = tool.as_object_mut() {
                object.remove("format");
            }
        } else if kind != "function" && kind != "tool_search" {
            tools.remove(index);
            continue;
        }
        index += 1;
    }
}

#[cfg(test)]
#[path = "custom_tool_downgrade_tests.rs"]
mod tests;
