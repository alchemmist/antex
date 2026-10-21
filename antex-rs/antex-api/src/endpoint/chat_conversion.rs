use crate::ApiError;
use antex_protocol::models::ResponseItem;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::hash::Hash;
use std::hash::Hasher;

pub(super) struct ChatRequest {
    pub body: Value,
    tools: BTreeMap<String, Value>,
}

pub(super) fn invalid(message: &str) -> ApiError {
    ApiError::Stream(format!("Eliza: {message}"))
}

fn tool_name(tool: &Value) -> Result<String, ApiError> {
    let name = tool["name"]
        .as_str()
        .ok_or_else(|| invalid("tool name is missing"))?;
    let namespace = tool["namespace"].as_str().unwrap_or_default();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (namespace, name).hash(&mut hash);
    let label: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(/*n*/ 40)
        .collect();
    Ok(format!("{label}_{:016x}", hash.finish()))
}

fn text(content: &Value) -> Result<String, ApiError> {
    if let Some(text) = content.as_str() {
        return Ok(text.to_string());
    }
    let parts = content
        .as_array()
        .ok_or_else(|| invalid("expected text content"))?;
    parts
        .iter()
        .map(|part| match part["type"].as_str() {
            Some("input_text" | "output_text" | "text") => part["text"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid("text content is missing")),
            _ => Err(invalid("this provider supports text content only")),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("\n"))
}

impl ChatRequest {
    pub(super) fn new(request: Value) -> Result<Self, ApiError> {
        let mut tools = BTreeMap::new();
        let mut pending: Vec<Value> = request["tools"].as_array().cloned().unwrap_or_default();
        let mut functions = Vec::new();
        while let Some(tool) = pending.pop() {
            if tool["type"] == "namespace" {
                for mut child in tool["tools"].as_array().cloned().unwrap_or_default() {
                    child["namespace"] = tool["name"].clone();
                    pending.push(child);
                }
                continue;
            }
            let kind = tool["type"].as_str().unwrap_or_default();
            if kind != "function" && kind != "custom" {
                return Err(invalid(&format!("unsupported hosted tool: {kind}")));
            }
            let name = tool_name(&tool)?;
            let parameters = if kind == "custom" {
                json!({"type":"object","properties":{"input":{"type":"string","description":"Exact freeform tool input"}},"required":["input"],"additionalProperties":false})
            } else {
                tool["parameters"].clone()
            };
            functions.push(json!({"type":"function","function":{"name":name,"description":tool["description"],"parameters":parameters}}));
            if tools.insert(name, tool).is_some() {
                return Err(invalid("duplicate tool name"));
            }
        }
        let mut messages = Vec::new();
        for item in request["input"]
            .as_array()
            .ok_or_else(|| invalid("input is missing"))?
        {
            match item["type"].as_str() {
                Some("agent_message") => messages.push(json!({"role":"assistant","content":text(&item["content"])?})),
                Some("message") => {
                    let role = item["role"].as_str().ok_or_else(|| invalid("message role is missing"))?;
                    messages.push(json!({"role":if role == "developer" { "system" } else { role },"content":text(&item["content"])?}));
                }
                Some("function_call" | "custom_tool_call") => {
                    if item["encrypted_function_args"].as_array().is_some_and(|args| !args.is_empty()) {
                        return Err(invalid("encrypted tool arguments cannot be sent to this provider"));
                    }
                    let arguments = if item["type"] == "custom_tool_call" { json!({"input":item["input"]}).to_string() } else { item["arguments"].as_str().ok_or_else(|| invalid("tool arguments are missing"))?.to_string() };
                    let call = json!({"id":item["call_id"],"type":"function","function":{"name":tool_name(item)?,"arguments":arguments}});
                    if let Some(previous) = messages.last_mut().filter(|m| m["role"] == "assistant" && m["tool_calls"].is_array()) {
                        if let Some(calls) = previous["tool_calls"].as_array_mut() { calls.push(call); }
                    } else {
                        messages.push(json!({"role":"assistant","content":null,"tool_calls":[call]}));
                    }
                }
                Some("function_call_output" | "custom_tool_call_output") => messages.push(json!({"role":"tool","tool_call_id":item["call_id"],"content":text(&item["output"])?})),
                Some("reasoning" | "additional_tools" | "tool_search_call" | "tool_search_output") => {}
                _ => return Err(invalid("unsupported history item; start a new conversation")),
            }
        }
        let mut body = json!({"model":request["model"],"messages":messages,"stream":false});
        if !functions.is_empty() {
            body["tools"] = json!(functions);
            body["tool_choice"] = json!("auto");
        }
        if let Some(format) = request
            .pointer("/text/format")
            .filter(|value| !value.is_null())
        {
            body["response_format"] = json!({"type":"json_schema","json_schema":{"name":format["name"],"schema":format["schema"],"strict":format["strict"]}});
        }
        Ok(Self { body, tools })
    }

    pub(super) fn output(&self, response: &Value) -> Result<Vec<ResponseItem>, ApiError> {
        let choice = response
            .pointer("/choices/0")
            .ok_or_else(|| invalid("response has no choices"))?;
        if !matches!(
            choice["finish_reason"].as_str(),
            Some("stop" | "tool_calls")
        ) {
            return Err(invalid("generation did not finish successfully"));
        }
        let message = &choice["message"];
        let mut output = Vec::new();
        if let Some(content) = message["content"].as_str().filter(|text| !text.is_empty()) {
            output.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":content}],"id":format!("msg_{}",uuid::Uuid::new_v4())}));
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            if calls.len() > 64 {
                return Err(invalid("too many tool calls"));
            }
            for call in calls {
                let name = call["function"]["name"]
                    .as_str()
                    .ok_or_else(|| invalid("tool name is missing"))?;
                let tool = self
                    .tools
                    .get(name)
                    .ok_or_else(|| invalid("model returned an unknown tool"))?;
                let arguments = call["function"]["arguments"]
                    .as_str()
                    .ok_or_else(|| invalid("tool arguments are missing"))?;
                let mut item = json!({"type":"function_call","name":tool["name"],"namespace":tool["namespace"],"arguments":arguments,"call_id":call["id"],"id":format!("fc_{}",uuid::Uuid::new_v4())});
                if tool["type"] == "custom" {
                    let args: Value = serde_json::from_str(arguments)
                        .map_err(|_| invalid("invalid freeform tool wrapper"))?;
                    if !args["input"].is_string() {
                        return Err(invalid("missing freeform tool input"));
                    }
                    item["type"] = json!("custom_tool_call");
                    item["input"] = args["input"].clone();
                }
                output.push(item);
            }
        }
        output
            .into_iter()
            .map(|item| serde_json::from_value(item).map_err(|_| invalid("malformed output item")))
            .collect()
    }
}

#[cfg(test)]
#[path = "chat_conversion_tests.rs"]
mod tests;
