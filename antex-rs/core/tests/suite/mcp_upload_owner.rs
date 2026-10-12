//! Real run_turn -> MCP approval -> local-file preparation ownership regressions.
//! All credentials and HTTP endpoints are synthetic and isolated to this fixture.
use super::*;
use antex_login::AntexAuth;
use antex_login::AuthCredentialsStoreMode;
use antex_login::AuthDotJson;
use antex_login::AuthKeyringBackendKind;
use antex_login::AuthManager;
use antex_protocol::approvals::ElicitationRequestEvent;
use base64::Engine;
use pretty_assertions::assert_eq;
use std::path::Path;
use std::time::Duration;
use wiremock::MockServer;
use wiremock::matchers::path;

const FILE: &str = "cb1-private.txt";
const MARKER: &str = "CB-1 private original-operation bytes 07d5ec43";

fn save_owner(home: &Path, account: &str, token: &str) -> Result<()> {
    let mut document = AuthDotJson {
        auth_mode: Some(antex_protocol::auth::AuthMode::Chatgpt),
        openai_api_key: None,
        tokens: Some(AntexAuth::create_dummy_chatgpt_auth_for_testing().get_token_data()?),
        last_refresh: Some(chrono::Utc::now()),
        agent_identity: None,
        personal_access_token: None,
        bedrock_api_key: None,
        bedrock_access_keys: None,
    };
    // A complete stable identity makes this a genuine same-owner rotation
    // control, not the distinct sparse host-token header-provider exception.
    let claims =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account,
                "chatgpt_user_id": "cb1-user",
                "chatgpt_plan_type": "plus"
            }
        }))?);
    let tokens = document.tokens.as_mut().unwrap();
    tokens.account_id = Some(account.to_string());
    tokens.access_token = token.to_string();
    tokens.id_token =
        antex_login::token_data::parse_chatgpt_jwt_claims(&format!("header.{claims}.signature"))?;
    antex_login::save_auth(
        home,
        &document,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?;
    Ok(())
}

fn save_sparse_owner(home: &Path, token: &str) -> Result<()> {
    save_owner(home, "account-a", token)?;
    let mut document: AuthDotJson =
        serde_json::from_slice(&std::fs::read(home.join("auth.json"))?)?;
    document.auth_mode = Some(antex_protocol::auth::AuthMode::ChatgptAuthTokens);
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(
        &json!({"https://api.openai.com/auth": {
            "chatgpt_account_id": "account-a", "chatgpt_plan_type": "plus"
        }}),
    )?);
    document.tokens.as_mut().unwrap().id_token =
        antex_login::token_data::parse_chatgpt_jwt_claims(&format!("header.{claims}.signature"))?;
    antex_login::save_auth(
        home,
        &document,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?;
    Ok(())
}

async fn install_owner(
    manager: &AuthManager,
    home: &Path,
    account: &str,
    token: &str,
) -> Result<()> {
    save_owner(home, account, token)?;
    // reload's bool uses AntexAuth's mode-based equality for ChatGPT; a
    // same-mode account/token replacement returns false. Verify the actual
    // request credential below instead of treating that bool as acceptance.
    manager.reload().await;
    let accepted = manager.auth_for_request().await?.unwrap();
    assert_eq!(
        (
            accepted.get_account_id(),
            accepted.get_chatgpt_user_id(),
            accepted.get_token()?
        ),
        (
            Some(account.to_string()),
            Some("cb1-user".to_string()),
            token.to_string()
        )
    );
    assert!(accepted.uses_antex_backend());
    Ok(())
}

async fn mount_file_tool(server: &MockServer) -> Result<AppsTestServer> {
    let apps = AppsTestServer::mount(server).await?;
    Mock::given(method("POST"))
        .and(path_regex("^/api/codex/ps/mcp/?$"))
        .and(body_partial_json(json!({"method": "tools/list"})))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": body["id"],
                "result": {"tools": [{
                    "name": "calendar_create_event",
                    "description": "Create an event from a private file.",
                    "inputSchema": {"type": "object", "properties": {
                        "file": {"type": "object", "properties": {
                            "file_id": {"type": "string"},
                            "download_url": {"type": "string"}
                        }}
                    }, "required": ["file"]},
                    "annotations": {"readOnlyHint": false},
                    "_meta": {
                        "connector_id": "calendar", "connector_name": "Calendar",
                        "openai/fileParams": ["file"]
                    }
                }]}
            }))
        })
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("^/api/codex/ps/mcp/?$"))
        .and(body_partial_json(json!({"method": "tools/call"})))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": body["id"],
                "result": {"content": [{"type": "text", "text": "file accepted"}], "isError": false}
            }))
        })
        .with_priority(1)
        .mount(server)
        .await;
    Ok(apps)
}

// Valid upload responses are deliberately mounted even in zero-upload cases:
// an accidental request must not look safe just because it received a 404.
async fn mount_uploads(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "file_id": "cb1-file", "upload_url": format!("{}/blob/cb1-file", server.uri())
        })))
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/blob/cb1-file"))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/files/cb1-file/uploaded"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success", "download_url": format!("{}/download/cb1-file", server.uri()),
            "file_name": FILE, "file_size_bytes": MARKER.len()
        })))
        .mount(server)
        .await;
}

async fn upload_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| {
            request.url.path() == "/files"
                || request.url.path().starts_with("/blob/")
                || request.url.path().ends_with("/uploaded")
        })
        .collect()
}

fn file_call(id: &str) -> String {
    sse(vec![
        ev_response_created(id),
        ev_function_call_with_namespace(
            id,
            SEARCH_CALENDAR_NAMESPACE,
            SEARCH_CALENDAR_CREATE_TOOL,
            &json!({"file": FILE}).to_string(),
        ),
        ev_completed(id),
    ])
}

fn done() -> String {
    sse(vec![
        ev_assistant_message("done", "done"),
        ev_completed("done"),
    ])
}

async fn start_file_call(test: &TestAntex) -> Result<ElicitationRequestEvent> {
    submit_user_turn(
        test,
        "Use [$calendar](app://calendar) to create an event from cb1-private.txt.",
        AskForApproval::OnRequest,
        PermissionProfile::Disabled,
        /*collaboration_mode*/ None,
    )
    .await?;
    let event = wait_for_event(&test.antex, |event| {
        matches!(
            event,
            EventMsg::ElicitationRequest(_) | EventMsg::TurnComplete(_)
        )
    })
    .await;
    let EventMsg::ElicitationRequest(request) = event else {
        panic!("expected the real MCP approval gate, got {event:?}");
    };
    assert_eq!(request.server_name, "codex_apps");
    Ok(request)
}

async fn approve(test: &TestAntex, request: ElicitationRequestEvent) -> Result<()> {
    test.antex
        .submit(Op::ResolveElicitation {
            server_name: request.server_name,
            request_id: request.id,
            decision: ElicitationAction::Accept,
            content: None,
            meta: None,
        })
        .await?;
    Ok(())
}

async fn finish_file_call(test: &TestAntex, succeeds: bool) {
    let mut ended = false;
    wait_for_event(&test.antex, |event| {
        if let EventMsg::McpToolCallEnd(end) = event {
            assert_eq!(end.result.is_ok(), succeeds, "unexpected MCP result: {:?}", end.result);
            if let Err(error) = &end.result {
                assert!(error.contains("failed to resolve file upload authentication"),
                    "must reject at upload authority, not catalog staleness or file permissions: {error}");
            }
            ended = true;
        }
        matches!(event, EventMsg::TurnComplete(_))
    }).await;
    assert!(
        ended,
        "must observe preparation/execution completing, not merely model rejection"
    );
}

async fn assert_uploaded(server: &MockServer, account: &str, token: &str) -> Result<()> {
    let requests = upload_requests(server).await;
    assert_eq!(
        requests
            .iter()
            .map(|r| (r.method.as_str(), r.url.path()))
            .collect::<Vec<_>>(),
        vec![
            ("POST", "/files"),
            ("PUT", "/blob/cb1-file"),
            ("POST", "/files/cb1-file/uploaded")
        ]
    );
    for request in [&requests[0], &requests[2]] {
        assert_eq!(
            request.headers.get("authorization").unwrap().to_str()?,
            format!("Bearer {token}")
        );
        assert_eq!(
            request
                .headers
                .get("chatgpt-account-id")
                .unwrap()
                .to_str()?,
            account
        );
    }
    assert_eq!(requests[1].body, MARKER.as_bytes());
    let calls = recorded_apps_tool_calls(server).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]["params"]["arguments"]["file"],
        json!({
            "file_id": "cb1-file", "download_url": format!("{}/download/cb1-file", server.uri()),
            "file_name": "cb1-private.txt"
        })
    );
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum SwitchAt {
    Approval,
    SparseApproval,
    CreateResponse,
    Rotation,
}

async fn exercise_upload_owner(switch_at: SwitchAt) -> Result<()> {
    let server = start_mock_server().await;
    let apps = mount_file_tool(&server).await?;
    mount_uploads(&server).await;
    let home = tempfile::tempdir()?;
    if switch_at == SwitchAt::SparseApproval {
        save_sparse_owner(home.path(), "token-a")?;
    } else {
        save_owner(home.path(), "account-a", "token-a")?;
    }
    let manager = Arc::new(
        AuthManager::new(
            home.path().to_path_buf(),
            /*enable_antex_api_key_env*/ false,
            AuthCredentialsStoreMode::File,
            /*forced_chatgpt_workspace_id*/ None,
            /*chatgpt_base_url*/ None,
            AuthKeyringBackendKind::default(),
            antex_login::test_support::transport_default_auth_route_config(),
        )
        .await,
    );
    let responses = mount_sse_sequence(
        &server,
        if matches!(switch_at, SwitchAt::Approval | SwitchAt::SparseApproval) {
            vec![file_call("old-a"), file_call("new-b"), done()]
        } else if switch_at == SwitchAt::Rotation {
            vec![file_call("old-a"), done()]
        } else {
            vec![file_call("old-a")]
        },
    )
    .await;
    let test = search_capable_apps_builder(apps.chatgpt_base_url)
        .with_auth_manager(Arc::clone(&manager))
        .with_config(|config| {
            set_calendar_approval_mode(config, AppToolApproval::Prompt);
            config.approvals_reviewer = ApprovalsReviewer::User;
            config
                .features
                .enable(Feature::ToolCallMcpElicitation)
                .unwrap();
        })
        .build_with_auto_env(&server)
        .await?;
    test.fs()
        .write_file(
            &test.workspace_path_uri(FILE)?,
            MARKER.as_bytes().to_vec(),
            Default::default(),
            /*sandbox*/ None,
        )
        .await?;
    let request = start_file_call(&test).await?;
    assert!(upload_requests(&server).await.is_empty());
    assert!(recorded_apps_tool_calls(&server).await.is_empty());
    assert_eq!(responses.requests().len(), 1);
    assert_eq!(
        responses.requests()[0].header("authorization"),
        Some("Bearer token-a".to_string())
    );
    assert_eq!(
        responses.requests()[0].header("chatgpt-account-id"),
        Some("account-a".to_string())
    );

    if switch_at == SwitchAt::CreateResponse {
        // Gate a real preparation await without adding a production filesystem
        // hook. A's allocation already exists here; only finalization must stop.
        let reached = Arc::new(tokio::sync::Notify::new());
        let notify = Arc::clone(&reached);
        let (release, gate) = std::sync::mpsc::channel();
        let gate = Mutex::new(gate);
        let upload_url = format!("{}/blob/cb1-file", server.uri());
        Mock::given(method("POST"))
            .and(path("/files"))
            .respond_with(move |_request: &Request| {
                notify.notify_one();
                gate.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                ResponseTemplate::new(200).set_body_json(json!({
                    "file_id": "cb1-file", "upload_url": upload_url
                }))
            })
            .with_priority(1)
            .mount(&server)
            .await;
        approve(&test, request).await?;
        tokio::time::timeout(Duration::from_secs(5), reached.notified()).await?;
        install_owner(&manager, home.path(), "account-b", "token-b").await?;
        release.send(())?;
        finish_file_call(&test, /*succeeds*/ false).await;
        let uploads = upload_requests(&server).await;
        assert_eq!(
            uploads.iter().map(|r| r.url.path()).collect::<Vec<_>>(),
            vec!["/files", "/blob/cb1-file"]
        );
        assert_eq!(
            uploads[0].headers.get("authorization").unwrap(),
            "Bearer token-a"
        );
        assert_eq!(
            uploads[0].headers.get("chatgpt-account-id").unwrap(),
            "account-a"
        );
        assert_eq!(uploads[1].body, MARKER.as_bytes());
        assert!(recorded_apps_tool_calls(&server).await.is_empty());
        assert_eq!(responses.requests().len(), 1);
        return Ok(());
    }

    let account = if switch_at == SwitchAt::Rotation {
        "account-a"
    } else {
        "account-b"
    };
    if switch_at == SwitchAt::SparseApproval {
        let original = manager.auth_cached().unwrap();
        assert_eq!(original.get_chatgpt_user_id(), None);
        let operation_owner = manager.unauthorized_recovery();
        let header_provider =
            antex_model_provider::auth_provider_from_auth_manager(Arc::clone(&manager), &original);
        save_sparse_owner(home.path(), "replacement-token")?;
        manager.reload().await;
        let accepted = manager.auth_for_request().await?.unwrap();
        assert_eq!(
            (
                accepted.get_account_id(),
                accepted.get_chatgpt_user_id(),
                accepted.get_token()?
            ),
            (
                Some("account-a".to_string()),
                None,
                "replacement-token".to_string()
            )
        );
        assert!(operation_owner.ensure_same_owner().is_err());
        // Account-scoped headers deliberately allow this legacy rotation;
        // uploading the old operation's private file must be stricter.
        assert_eq!(
            header_provider
                .resolve_auth_headers()
                .await?
                .get("authorization")
                .unwrap(),
            "Bearer replacement-token"
        );
    } else {
        install_owner(&manager, home.path(), account, "replacement-token").await?;
    }
    // No refresh, steer, or new turn before releasing the ORIGINAL approval.
    approve(&test, request).await?;
    finish_file_call(&test, switch_at == SwitchAt::Rotation).await;
    if switch_at == SwitchAt::Rotation {
        assert_uploaded(&server, "account-a", "replacement-token").await?;
        assert_eq!(responses.requests().len(), 2);
        assert_eq!(
            responses.requests()[1].header("authorization"),
            Some("Bearer replacement-token".to_string())
        );
    } else {
        assert!(
            upload_requests(&server).await.is_empty(),
            "old A call must send ZERO file-create, blob PUT, and finalize requests"
        );
        assert!(recorded_apps_tool_calls(&server).await.is_empty());
        assert_eq!(responses.requests().len(), 1);
        if switch_at == SwitchAt::SparseApproval {
            install_owner(&manager, home.path(), "account-b", "replacement-token").await?;
        }
        // Fresh B operation is allowed to select the same readable file. Only
        // now refresh the binding and submit a genuinely new user turn.
        test.antex.submit(Op::RefreshMcpServers).await?;
        let request = start_file_call(&test).await?;
        approve(&test, request).await?;
        finish_file_call(&test, /*succeeds*/ true).await;
        assert_uploaded(&server, "account-b", "replacement-token").await?;
        assert_eq!(responses.requests().len(), 3);
        for response in &responses.requests()[1..] {
            assert_eq!(
                (
                    response.header("authorization"),
                    response.header("chatgpt-account-id")
                ),
                (
                    Some("Bearer replacement-token".to_string()),
                    Some("account-b".to_string())
                )
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_file_approval_owner_switch_blocks_all_uploads_and_new_b_turn_succeeds() -> Result<()> {
    skip_if_no_network!(Ok(()));
    exercise_upload_owner(SwitchAt::Approval).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_file_approval_sparse_host_replacement_cannot_use_account_scoped_headers() -> Result<()>
{
    skip_if_no_network!(Ok(()));
    exercise_upload_owner(SwitchAt::SparseApproval).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_file_approval_same_owner_rotation_uploads_with_rotated_auth() -> Result<()> {
    skip_if_no_network!(Ok(()));
    exercise_upload_owner(SwitchAt::Rotation).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_file_preparation_owner_switch_during_create_rejects_finalize() -> Result<()> {
    skip_if_no_network!(Ok(()));
    exercise_upload_owner(SwitchAt::CreateResponse).await
}
