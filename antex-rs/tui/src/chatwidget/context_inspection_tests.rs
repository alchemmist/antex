use super::*;
use antex_protocol::models::BaseInstructions;
use antex_protocol::protocol::TokenUsage;
use antex_protocol::protocol::TokenUsageInfo;
use pretty_assertions::assert_eq;

#[test]
fn summarizes_context_items_and_limits_developer_labels() {
    let mut items = vec![
        message("user", "first question"),
        message("assistant", "answer"),
    ];
    for index in 0..7 {
        items.push(message("developer", &format!("section {index}\nbody")));
    }
    items.push(ResponseItem::FunctionCall {
        id: None,
        name: "shell".to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        encrypted_function_args: None,
        call_id: "call-1".to_string(),
        internal_chat_message_metadata_passthrough: None,
    });

    let counts = summarize_items(&items);

    assert_eq!(counts.user_messages, 1);
    assert_eq!(counts.assistant_messages, 1);
    assert_eq!(counts.developer_messages, 7);
    assert_eq!(counts.tool_calls, 1);
    assert_eq!(
        counts.developer_labels,
        vec![
            "section 0",
            "section 1",
            "section 2",
            "section 3",
            "section 4"
        ]
    );
}

#[test]
fn writes_the_complete_model_request() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("system prompt.md");
    let request = r#"{"instructions":"You are Antex.","input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"tools":[{"type":"function","name":"shell"}]}"#;
    let document = model_request_document(request).expect("model request document");

    write_system_prompt(&path, &document).expect("write prompt");

    insta::assert_snapshot!(std::fs::read_to_string(path).expect("read prompt"), @r#"
# Latest model request

This is the complete logical Responses API request from the latest model call. WebSocket transport may send it incrementally using `previous_response_id`.

```json
{
  "instructions": "You are Antex.",
  "input": [
    {
      "role": "user",
      "content": [
        {
          "type": "input_text",
          "text": "hello"
        }
      ]
    }
  ],
  "tools": [
    {
      "type": "function",
      "name": "shell"
    }
  ]
}
```
"#);
}

#[test]
fn system_prompt_window_targets_the_resolved_window_not_the_pane() {
    assert_eq!(
        new_window_args("@4", "sp-01a02451", "nvim /tmp/prompt.md"),
        [
            "new-window",
            "-a",
            "-t",
            "@4",
            "-n",
            "sp-01a02451",
            "nvim /tmp/prompt.md",
        ]
    );
}

#[test]
fn context_summary_snapshot() {
    let inspection = ContextInspection {
        base_instructions: BaseInstructions {
            text: "system prompt".to_string(),
            provenance: None,
        },
        items: vec![
            message("developer", "# AGENTS.md instructions\nDetails"),
            message("user", "question"),
            message("assistant", "answer"),
        ],
        token_info: Some(TokenUsageInfo {
            total_token_usage: TokenUsage {
                total_tokens: 25_000,
                ..TokenUsage::default()
            },
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(100_000),
        }),
        latest_model_request: None,
    };

    let rendered = context_summary_lines(&inspection)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    insta::assert_snapshot!(rendered, @r"
Context
  Tokens: 25000 / 100000 tokens (25%)
  System prompt: 13 characters
  Conversation: 1 user · 1 assistant
  Developer context: 1
    - # AGENTS.md instructions
  Tools: 0 calls · 0 outputs
");
}

fn message(role: &str, text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}
