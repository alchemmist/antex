use super::chat_conversion::ChatRequest;
use super::chat_conversion::invalid;
use super::session::EndpointSession;
use crate::ApiError;
use crate::ResponseEvent;
use crate::ResponseStream;
use crate::ResponsesApiRequest;
use antex_client::HttpTransport;
use antex_protocol::eliza::ElizaCatalog;
use antex_protocol::eliza::MAX_CATALOG_BYTES;
use antex_protocol::models::ResponseItem;
use antex_protocol::protocol::TokenUsage;
use http::HeaderMap;
use http::Method;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

pub(super) async fn stream<T: HttpTransport>(
    session: &EndpointSession<T>,
    request: ResponsesApiRequest,
) -> Result<ResponseStream, ApiError> {
    let response = tokio::time::timeout(
        Duration::from_secs(/*secs*/ 5),
        session.execute_with(Method::GET, "/models", HeaderMap::new(), None, |request| {
            request.response_body_limit_bytes = Some(MAX_CATALOG_BYTES);
        }),
    )
    .await
    .map_err(|_| invalid("model catalog timed out"))??;
    let catalog: ElizaCatalog =
        serde_json::from_slice(&response.body).map_err(|_| invalid("invalid model catalog"))?;
    let model = catalog
        .data
        .into_iter()
        .find(|entry| entry.id == request.model && entry.is_interactive())
        .ok_or_else(|| invalid("model is not available in the internal interactive catalog"))?;
    let mut chat =
        ChatRequest::new(serde_json::to_value(request).map_err(|_| invalid("invalid request"))?)?;
    let max_output = model
        .architecture
        .and_then(|a| a.max_output_tokens)
        .filter(|n| *n > 0)
        .unwrap_or(8192)
        .min(16_384);
    chat.body["max_tokens"] = serde_json::json!(max_output);
    let base = session
        .provider()
        .base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1");
    let url = format!("{base}/raw{}", model.endpoint);
    let response = tokio::time::timeout(
        Duration::from_secs(/*secs*/ 180),
        session.execute_with(
            Method::POST,
            "/chat/completions",
            HeaderMap::new(),
            Some(chat.body.clone()),
            |request| {
                request.url.clone_from(&url);
                request.response_body_limit_bytes = Some(4 * 1024 * 1024);
            },
        ),
    )
    .await
    .map_err(|_| invalid("generation timed out"))??;
    let body: Value = serde_json::from_slice(&response.body)
        .map_err(|_| invalid("invalid completion response"))?;
    let output = chat.output(&body)?;
    let response_id = body["id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let usage = body.get("usage").map(|usage| TokenUsage {
        input_tokens: usage["prompt_tokens"].as_i64().unwrap_or(0),
        output_tokens: usage["completion_tokens"].as_i64().unwrap_or(0),
        total_tokens: usage["total_tokens"].as_i64().unwrap_or(0),
        cached_input_tokens: usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        reasoning_output_tokens: usage
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        ..Default::default()
    });
    let mut events = vec![ResponseEvent::Created {
        response_id: Some(response_id.clone()),
    }];
    for item in output {
        let mut added = item.clone();
        if let ResponseItem::Message { content, .. } = &mut added {
            content.clear();
        }
        events.push(ResponseEvent::OutputItemAdded(added));
        if let ResponseItem::Message { content, .. } = &item {
            for part in content {
                if let antex_protocol::models::ContentItem::OutputText { text, .. } = part {
                    events.push(ResponseEvent::OutputTextDelta(text.clone()));
                }
            }
        }
        events.push(ResponseEvent::OutputItemDone(item));
    }
    events.push(ResponseEvent::Completed {
        response_id,
        token_usage: usage,
        usage_metadata: None,
        end_turn: Some(body["choices"][0]["finish_reason"] == "stop"),
    });
    let (tx, rx_event) = mpsc::channel(events.len());
    for event in events {
        tx.try_send(Ok(event))
            .map_err(|_| invalid("output channel is full"))?;
    }
    Ok(ResponseStream {
        rx_event,
        upstream_request_id: None,
        interrupt: None,
    })
}
