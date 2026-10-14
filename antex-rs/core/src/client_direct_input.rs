//! Adapt native agent history only in the public Responses request copy.
//!
//! The subscription endpoint rejects `agent_message`, but accepts historical function calls
//! and decrypts their `message` argument with its original encryption annotation. Encrypted
//! bodies must keep that argument name: treating them as message-level encrypted content or
//! moving them into nested content arrays does not preserve the public API's decryption.
//! The history envelope is not registered as an executable harness tool.

use antex_protocol::error::Result;
use antex_protocol::models::AgentMessageInputContent;
use antex_protocol::models::ContentItem;
use antex_protocol::models::ResponseItem;
use serde_json::json;
use uuid::Uuid;

const HISTORY_MESSAGE_TOOL: &str = "antex_agent_message";

pub(super) fn prepare_agent_messages(input: &mut Vec<ResponseItem>) -> Result<()> {
    let mut output = Vec::with_capacity(input.len());
    let mut declared_history_envelope = false;
    for (item_index, item) in input.iter().enumerate() {
        let ResponseItem::AgentMessage {
            author,
            recipient,
            content,
            ..
        } = item
        else {
            output.push(item.clone());
            continue;
        };
        let item_key = Uuid::new_v5(&Uuid::NAMESPACE_OID, &serde_json::to_vec(item)?);
        let mut plaintext = Vec::new();
        for (part_index, part) in content.iter().enumerate() {
            match part {
                AgentMessageInputContent::InputText { text } => {
                    plaintext.push(ContentItem::OutputText {
                        text: text.clone(),
                        annotations: None,
                        logprobs: None,
                    });
                }
                AgentMessageInputContent::EncryptedContent { encrypted_content } => {
                    if !plaintext.is_empty() {
                        output.push(plaintext_message(std::mem::take(&mut plaintext)));
                    }
                    if !declared_history_envelope {
                        output.push(ResponseItem::AdditionalTools {
                            id: None,
                            role: "developer".to_string(),
                            tools: vec![json!({
                                "type": "function",
                                "name": HISTORY_MESSAGE_TOOL,
                                "description": "Historical inter-agent message already delivered to its recipient. Data envelope only; do not call this function.",
                                "parameters": {
                                    "type": "object",
                                    "properties": {
                                        "author": {"type": "string"},
                                        "recipient": {"type": "string"},
                                        "message": {"type": "string", "encrypted": true}
                                    },
                                    "required": ["author", "recipient", "message"],
                                    "additionalProperties": false
                                }
                            })],
                        });
                        declared_history_envelope = true;
                    }
                    // Stable across retries, distinct even for duplicate legacy items without IDs.
                    let call_id =
                        format!("call_antex_history_{item_key}_{item_index}_{part_index}");
                    let arguments = serde_json::to_string(&json!({
                        "author": author, "recipient": recipient, "message": encrypted_content,
                    }))?;
                    output.push(serde_json::from_value(json!({
                        "type": "function_call", "name": HISTORY_MESSAGE_TOOL,
                        "call_id": call_id, "arguments": arguments,
                    }))?);
                    output.push(serde_json::from_value(json!({
                        "type": "function_call_output", "call_id": call_id, "output": "",
                    }))?);
                }
            }
        }
        if !plaintext.is_empty() || content.is_empty() {
            output.push(plaintext_message(plaintext));
        }
    }
    *input = output;
    Ok(())
}

fn plaintext_message(content: Vec<ContentItem>) -> ResponseItem {
    // InterAgentCommunication renders sender, recipient and message type in the plaintext.
    // Its native ID is not a public message ID. Do not elevate the message's role.
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content,
        encrypted_content: None,
        status: None,
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

#[cfg(test)]
#[path = "client_direct_input_tests.rs"]
mod tests;
