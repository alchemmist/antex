use std::fs;

use pretty_assertions::assert_eq;
use antex_core::Content;
use antex_core::Message;
use antex_core::RawToolCall;
use antex_core::ToolOutcome;
use antex_core::ToolOutput;

use super::MigrationPlan;

#[test]
fn codex_migration_is_deterministic_and_never_changes_the_source() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let original = "model = 'gpt-test'\nsandbox_mode = 'workspace-write'\nsecret = 'do-not-echo'\n[mcp_servers.docs]\ncommand = 'docs-server'\nargs = ['--stdio']\n[mcp_servers.remote]\nurl = 'https://example.test/mcp'\n";
    fs::write(source.path().join("config.toml"), original).unwrap();
    fs::create_dir_all(source.path().join("skills/review/scripts")).unwrap();
    fs::write(source.path().join("skills/review/SKILL.md"), "# Review\n").unwrap();
    fs::write(
        source.path().join("skills/review/scripts/check.py"),
        "print('ok')\n",
    )
    .unwrap();
    let first = MigrationPlan::codex(source.path(), destination.path()).unwrap();
    let second = MigrationPlan::codex(source.path(), destination.path()).unwrap();
    assert_eq!(first.describe(), second.describe());
    assert!(!first.describe().join("\n").contains("do-not-echo"));
    assert!(
        first
            .describe()
            .iter()
            .any(|line| line == "skip MCP remote: streamable HTTP/OAuth is not implemented")
    );
    first.apply().unwrap();
    assert_eq!(
        fs::read_to_string(source.path().join("config.toml")).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(destination.path().join("skills/review/SKILL.md")).unwrap(),
        "# Review\n"
    );
    assert_eq!(
        fs::read_to_string(destination.path().join("skills/review/scripts/check.py")).unwrap(),
        "print('ok')\n"
    );
    assert_eq!(
        fs::read_to_string(destination.path().join("config.toml")).unwrap(),
        "model = \"gpt-test\"\npermissions = \"workspace\"\n"
    );
    let definition: serde_json::Value = serde_json::from_slice(
        &fs::read(
            destination
                .path()
                .join("extensions/mcp-docs/extension.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        definition,
        serde_json::json!({
            "name":"docs",
            "program":"antex_ext_mcp.py",
            "arguments":["--name","docs","--","docs-server","--stdio"],
            "capabilities":["network","shell"]
        })
    );
}

#[test]
fn portable_sessions_and_plain_stashes_are_imported_without_provider_state() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4();
    let session_directory = source.path().join("sessions/2026/09/07");
    fs::create_dir_all(&session_directory).unwrap();
    let records = [
        serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":workspace.path(),"base_instructions":"not imported"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"hidden"}]}}),
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"request"}]}}),
        serde_json::json!({"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"summary"}],"encrypted_content":"private"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
        serde_json::json!({"type":"response_item","payload":{"type":"function_call","call_id":"call-1","name":"read","arguments":"{\"path\":\"file\"}"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"call-1","output":"result"}}),
    ];
    let text = records
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(session_directory.join(format!("rollout-{id}.jsonl")), text).unwrap();
    fs::create_dir(source.path().join("prompt-stashes")).unwrap();
    fs::write(
        source.path().join(format!("prompt-stashes/{id}.json")),
        serde_json::json!({"version":1,"composer":{"text":"draft","local_images":[],"remote_image_urls":[],"text_elements":[],"mention_bindings":[],"pending_pastes":[]}}).to_string(),
    )
    .unwrap();
    let plan = MigrationPlan::codex(source.path(), destination.path()).unwrap();
    assert!(plan.describe().iter().any(|line| line == "import sessions: 1 portable sessions"));
    plan.apply().unwrap();
    let store = antex_runtime::SessionStore::new(destination.path(), workspace.path()).unwrap();
    let mut session = store.open(id).unwrap();
    assert_eq!(
        session
            .active_path()
            .unwrap()
            .into_iter()
            .map(|entry| entry.message)
            .collect::<Vec<_>>(),
        vec![
            Message::User("request".into()),
            Message::Assistant {
                content: vec![Content::Reasoning("summary".into())],
                tool_calls: Vec::new(),
            },
            Message::Assistant {
                content: vec![Content::Text("answer".into())],
                tool_calls: Vec::new(),
            },
            Message::Assistant {
                content: Vec::new(),
                tool_calls: vec![RawToolCall {
                    id: "call-1".into(),
                    name: "read".into(),
                    arguments: "{\"path\":\"file\"}".into(),
                }],
            },
            Message::Tool(ToolOutput::new(
                "call-1".into(),
                ToolOutcome::Success,
                "result".into(),
            )),
        ]
    );
    assert_eq!(
        session.load_ui_state("promptStash").unwrap(),
        Some(serde_json::json!({"version":1,"text":"draft","elements":[],"images":[]}))
    );
}
