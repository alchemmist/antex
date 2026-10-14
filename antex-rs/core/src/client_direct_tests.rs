use super::*;
use pretty_assertions::assert_eq;

#[path = "client_direct_retry_tests.rs"]
mod retry_tests;
#[path = "client_direct_stream_tests.rs"]
mod stream_tests;

fn save_direct_grant(
    home: &std::path::Path,
    registration: &str,
    access_token: &str,
) -> anyhow::Result<()> {
    std::fs::write(
        home.join("auth.json"),
        serde_json::to_vec(&json!({
            "auth_mode": "chatgptDirect",
            "tokens": {
                "id_token": "header.e30.signature",
                "access_token": access_token,
                "refresh_token": "test-direct-refresh",
                "direct": {
                    "client_id": registration,
                    "expires_at": "2099-01-01T00:00:00Z",
                    "scopes": ["chatgpt.tokens.use.direct"]
                }
            },
            "last_refresh": "2099-01-01T00:00:00Z"
        }))?,
    )?;
    Ok(())
}

fn empty_response_rate_limits() -> antex_protocol::protocol::RateLimitSnapshot {
    antex_protocol::protocol::RateLimitSnapshot {
        limit_id: Some("codex".to_string()),
        limit_name: None,
        normal_model_slug: None,
        primary: None,
        secondary: None,
        credits: None,
        individual_limit: None,
        spend_control_reached: None,
        plan_type: None,
        rate_limit_reached_type: None,
    }
}

async fn direct_client() -> anyhow::Result<(TempDir, ModelClient, Arc<AuthManager>)> {
    let home = tempfile::tempdir()?;
    save_direct_grant(home.path(), "test-issued-client", "test-direct-access")?;
    let auth = AntexAuth::from_auth_storage(
        home.path(),
        antex_login::AuthCredentialsStoreMode::File,
        /*chatgpt_base_url*/ None,
        antex_login::AuthKeyringBackendKind::default(),
        &antex_login::test_support::transport_default_auth_route_config(),
    )
    .await?
    .expect("isolated direct auth should load");
    let manager = AuthManager::from_auth_for_testing_with_home(auth, home.path().to_path_buf());
    let mut client = test_model_client(SessionSource::Cli);
    Arc::get_mut(&mut client.state).unwrap().provider = create_model_provider(
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        Some(manager.clone()),
    );
    Ok((home, client, manager))
}

#[tokio::test]
async fn direct_agent_history_is_adapted_without_changing_legacy_requests() -> anyhow::Result<()> {
    let (_home, direct, _manager) = direct_client().await?;
    let mut legacy = test_model_client(SessionSource::Cli);
    Arc::get_mut(&mut legacy.state).unwrap().provider = create_model_provider(
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        Some(AuthManager::from_auth_for_testing(
            AntexAuth::create_dummy_chatgpt_auth_for_testing(),
        )),
    );
    let mut api_key = test_model_client(SessionSource::Cli);
    Arc::get_mut(&mut api_key.state).unwrap().provider = create_model_provider(
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        Some(AuthManager::from_auth_for_testing(AntexAuth::from_api_key(
            "test-api-key",
        ))),
    );
    let oss = test_model_client(SessionSource::Cli);
    let model = antex_models_manager::bundled_models_response()?
        .models
        .into_iter()
        .find(|model| model.slug == "gpt-6-astra")
        .expect("bundled Astra metadata");
    let native = json!({"type":"agent_message","author":"/root/review","recipient":"/root",
        "content":[{"type":"input_text","text":"Message Type: MESSAGE\nTask name: /root\nSender: /root/review\nPayload:\nDone"}]});
    let opaque = json!({"type":"agent_message","author":"/root","recipient":"/root/review",
        "content":[{"type":"encrypted_content","encrypted_content":"opaque-legacy-task"}]});
    let prompt = Prompt {
        input: vec![
            serde_json::from_value(native.clone())?,
            serde_json::from_value(opaque)?,
        ],
        ..Default::default()
    };
    let original = prompt.input.clone();
    for (client, expected_type) in [
        (&direct, "message"),
        (&legacy, "agent_message"),
        (&api_key, "agent_message"),
        (&oss, "agent_message"),
    ] {
        let metadata = test_responses_metadata_for_client(
            client,
            /*turn_id*/ None,
            format!("{}:0", client.state.thread_id),
            /*parent_thread_id*/ None,
            TestAntexResponsesRequestKind::Turn,
        );
        let request = client.build_responses_request(
            &prompt,
            &model,
            /*effort*/ None,
            antex_protocol::config_types::ReasoningSummary::None,
            /*service_tier*/ None,
            &metadata,
            /*include_internal*/ false,
        )?;
        let body = serde_json::to_value(request)?;
        let input = body["input"].as_array().unwrap();
        if expected_type == "agent_message" {
            assert_eq!(
                serde_json::to_value(&input[input.len() - original.len()..])?,
                serde_json::to_value(&original)?
            );
        } else {
            assert!(input.iter().all(|item| item["type"] != "agent_message"));
            let plaintext = input
                .iter()
                .find(|item| item["role"] == "assistant")
                .unwrap();
            assert_eq!(
                plaintext["content"][0]["text"],
                native["content"][0]["text"]
            );
            let call = input
                .iter()
                .find(|item| item["type"] == "function_call")
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(call["arguments"].as_str().unwrap())?,
                json!({"author":"/root","recipient":"/root/review","message":"opaque-legacy-task"})
            );
        }
        assert_eq!(prompt.input, original);
    }
    Ok(())
}

#[tokio::test]
async fn direct_subscription_disables_codex_framing_and_websocket_prewarm() -> anyhow::Result<()> {
    let (_home, client, _manager) = direct_client().await?;
    let setup = client
        .current_client_setup(crate::client::ClientRouting::Workspace)
        .await?;
    assert_eq!(setup.api_provider.base_url, "https://api.openai.com/v1");
    assert!(!client.responses_websocket_enabled());
    assert!(client.generate_attestation_header_for().await.is_none());
    assert!(
        client
            .responses_headers(setup.auth.as_ref(), "gpt-6-astra")
            .is_empty()
    );

    let model = antex_models_manager::bundled_models_response()?
        .models
        .into_iter()
        .find(|model| model.slug == "gpt-6-astra")
        .expect("bundled Astra metadata");
    let prompt = Prompt {
        tools: vec![antex_tools::ToolSpec::Freeform(antex_tools::FreeformTool {
            name: "exec".to_string(),
            description: "Execute code".to_string(),
            defer_loading: None,
            format: antex_tools::FreeformToolFormat {
                r#type: "grammar".to_string(),
                syntax: "lark".to_string(),
                definition: "start: /.+/".to_string(),
            },
        })]
        .into(),
        parallel_tool_calls: true,
        ..Default::default()
    };
    let metadata = test_responses_metadata_for_client(
        &client,
        /*turn_id*/ None,
        format!("{}:0", client.state.thread_id),
        /*parent_thread_id*/ None,
        TestAntexResponsesRequestKind::Turn,
    );
    let request = client.build_responses_request(
        &prompt,
        &model,
        /*effort*/ None,
        antex_protocol::config_types::ReasoningSummary::None,
        /*service_tier*/ None,
        &metadata,
        /*include_internal*/ false,
    )?;
    let body = serde_json::to_value(request)?;
    assert_eq!(body["tools"][0]["name"], "exec");
    assert_eq!(body["parallel_tool_calls"], true);
    assert!(body.get("client_metadata").is_none());
    assert!(body["reasoning"].get("context").is_none());
    assert!(
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "additional_tools")
    );
    let session = client.new_session();
    let options = session
        .build_responses_options(
            &metadata,
            antex_api::Compression::None,
            /*use_responses_lite*/ true,
        )
        .await;
    assert!(options.extra_headers.is_empty());
    assert!(options.session_id.is_none());
    assert!(options.thread_id.is_none());
    assert!(options.turn_state.is_none());
    Ok(())
}
