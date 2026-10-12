use super::*;
use antex_app_server_protocol::LoginAccountParams;
use antex_login::save_auth;
use antex_login::token_data::DirectTokenMetadata;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn direct_login_respects_forced_api_policy_without_replacing_auth() -> Result<()> {
    let home = TempDir::new()?;
    create_config_toml(
        home.path(),
        CreateConfigTomlParams {
            forced_method: Some("api".to_string()),
            ..Default::default()
        },
    )?;
    login_with_api_key(
        home.path(),
        "existing-key",
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?;
    let before = std::fs::read(home.path().join("auth.json"))?;
    let mut server = TestAppServer::builder()
        .with_antex_home(home.path())
        .without_auto_env()
        .build_initialized_with_timeout(DEFAULT_READ_TIMEOUT)
        .await?;
    let id = server
        .send_login_account_request(serde_json::to_value(LoginAccountParams::ChatgptDirect)?)
        .await?;
    let error = timeout(
        DEFAULT_READ_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(id)),
    )
    .await??;
    assert_eq!(
        error.error.message,
        "ChatGPT login is disabled. Use API key login instead."
    );
    assert_eq!(std::fs::read(home.path().join("auth.json"))?, before);
    Ok(())
}

#[tokio::test]
#[serial(login_port)]
async fn direct_browser_callback_validation_and_cancellation_preserve_legacy_auth() -> Result<()> {
    let home = TempDir::new()?;
    create_config_toml(home.path(), CreateConfigTomlParams::default())?;
    login_with_api_key(
        home.path(),
        "existing-key",
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?;
    let before = std::fs::read(home.path().join("auth.json"))?;
    let mut server = TestAppServer::builder()
        .with_mock_chatgpt_backend()
        .with_antex_home(home.path())
        .without_auto_env()
        .build_initialized_with_timeout(DEFAULT_READ_TIMEOUT)
        .await?;
    let id = server
        .send_login_account_request(serde_json::to_value(LoginAccountParams::ChatgptDirect)?)
        .await?;
    let response: LoginAccountResponse =
        timeout(DEFAULT_READ_TIMEOUT, server.read_response(id)).await??;
    let LoginAccountResponse::Chatgpt { login_id, auth_url } = response else {
        bail!("expected browser login response");
    };
    let url = Url::parse(&auth_url)?;
    assert_eq!(
        url.origin().ascii_serialization(),
        "https://auth.openai.com"
    );
    assert_eq!(url.path(), "/api/accounts/authorize");
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(
        query.get("client_id").map(String::as_str),
        Some("dynamic_agent_client")
    );
    assert_eq!(
        query.get("agent_name_hint").map(String::as_str),
        Some("Antex")
    );
    assert_eq!(
        query.get("resource").map(String::as_str),
        Some("https://api.openai.com/v1")
    );
    let redirect = query
        .get("redirect_uri")
        .ok_or_else(|| anyhow::anyhow!("missing redirect"))?;
    let state = query
        .get("state")
        .ok_or_else(|| anyhow::anyhow!("missing state"))?;
    let client = HttpClientBuilder::new()
        .without_redirects()
        .build_direct()?;
    for (callback_state, issued_id) in [
        (state.as_str(), None),
        ("wrong-state", Some("issued-client")),
    ] {
        let mut callback = Url::parse(redirect)?;
        callback
            .query_pairs_mut()
            .append_pair("code", "test-code")
            .append_pair("state", callback_state);
        if let Some(issued_id) = issued_id {
            callback
                .query_pairs_mut()
                .append_pair("client_id", issued_id);
        }
        assert_eq!(client.get(callback).send().await?.status(), 400);
        assert_eq!(std::fs::read(home.path().join("auth.json"))?, before);
    }
    let cancel_id = server
        .send_cancel_login_account_request(CancelLoginAccountParams {
            login_id: login_id.clone(),
        })
        .await?;
    let _: CancelLoginAccountResponse =
        timeout(DEFAULT_READ_TIMEOUT, server.read_response(cancel_id)).await??;
    let notification = timeout(
        DEFAULT_READ_TIMEOUT,
        server.read_stream_until_notification_message("account/login/completed"),
    )
    .await??;
    let ServerNotification::AccountLoginCompleted(completed) = notification.try_into()? else {
        bail!("expected login completion");
    };
    assert_eq!(
        (completed.login_id, completed.success),
        (Some(login_id), false)
    );
    assert!(completed.error.is_some());
    assert_eq!(std::fs::read(home.path().join("auth.json"))?, before);
    Ok(())
}

#[tokio::test]
async fn direct_account_is_a_subscription_without_backend_requests_or_token_export() -> Result<()> {
    let home = TempDir::new()?;
    let backend = MockServer::start().await;
    create_config_toml(
        home.path(),
        CreateConfigTomlParams {
            model_provider_id: Some("openai".to_string()),
            chatgpt_base_url: Some(backend.uri()),
            ..Default::default()
        },
    )?;
    write_chatgpt_auth(
        home.path(),
        ChatGptAuthFixture::new("direct-access-token")
            .email("direct@example.com")
            .plan_type("pro"),
        AuthCredentialsStoreMode::File,
    )?;
    let mut auth = load_auth_dot_json(
        home.path(),
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?
    .expect("fixture auth");
    auth.auth_mode = Some(DomainAuthMode::ChatgptDirect);
    auth.tokens.as_mut().expect("fixture tokens").direct = Some(DirectTokenMetadata {
        client_id: "test-issued-client".to_string(),
        expires_at: Utc::now() + ChronoDuration::hours(1),
        scopes: vec!["chatgpt.tokens.use.direct".to_string()],
    });
    save_auth(
        home.path(),
        &auth,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?;
    let mut server = TestAppServer::builder()
        .with_antex_home(home.path())
        .without_auto_env()
        .build_initialized_with_timeout(DEFAULT_READ_TIMEOUT)
        .await?;
    let id = server
        .send_get_account_request(GetAccountParams {
            refresh_token: false,
        })
        .await?;
    let response: GetAccountResponse =
        timeout(DEFAULT_READ_TIMEOUT, server.read_response(id)).await??;
    assert_eq!(
        response.account,
        Some(Account::ChatgptDirect {
            email: Some("direct@example.com".to_string()),
            plan_type: Some(AccountPlanType::Pro),
        })
    );
    assert_eq!(response.workspace_routing, None);
    let id = server
        .send_get_auth_status_request(GetAuthStatusParams {
            include_token: Some(true),
            refresh_token: Some(false),
        })
        .await?;
    let response: GetAuthStatusResponse =
        timeout(DEFAULT_READ_TIMEOUT, server.read_response(id)).await??;
    assert_eq!(
        response,
        GetAuthStatusResponse {
            auth_method: Some(AuthMode::ChatgptDirect),
            auth_token: None,
            requires_openai_auth: Some(true),
        }
    );
    let id = server.send_get_account_rate_limits_request().await?;
    let error = timeout(
        DEFAULT_READ_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(id)),
    )
    .await??;
    assert_eq!(
        error.error.message,
        "chatgpt authentication required to read rate limits"
    );
    assert!(
        backend
            .received_requests()
            .await
            .expect("backend requests")
            .is_empty()
    );
    Ok(())
}
