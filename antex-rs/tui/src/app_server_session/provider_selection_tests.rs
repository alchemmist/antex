//! Exercise provider defaults and explicit choices through real app-server requests.

use super::*;
use crate::legacy_core::config::ConfigBuilder;
use crate::legacy_core::config::ConfigOverrides;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn starts_and_forks_use_server_provider_unless_explicitly_selected() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("config.toml"),
        r#"
model_provider = "server-provider"
model = "gpt-5.2"
[model_providers.server-provider]
name = "Server provider"
base_url = "http://127.0.0.1:9/v1"
wire_api = "responses"
requires_openai_auth = false
[model_providers.client-provider]
name = "Client provider"
base_url = "http://127.0.0.1:9/v1"
wire_api = "responses"
requires_openai_auth = false
"#,
    )?;
    let server_config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await?;
    let client_config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            model_provider: Some("client-provider".into()),
            ..Default::default()
        })
        .build()
        .await?;
    let local_settings = LocalSettings::from(&client_config);
    let parent = crate::tests::write_session_rollout(
        home.path(),
        "2025-01-02T10-00-00",
        "2025-01-02T10:00:00Z",
        "Server history",
        "server-provider",
        server_config.cwd.as_path(),
    )?;
    let mut server = crate::start_embedded_app_server_for_picker(&server_config).await?;
    assert_eq!(
        crate::lookup_session_target_with_app_server(&mut server, &client_config, "Server history")
            .await?
            .unwrap()
            .thread_id,
        parent
    );
    let resumed = server
        .resume_thread(
            &local_settings,
            client_config.clone(),
            parent,
            ResumeModelSettings::OverrideFromCurrentConfig,
        )
        .await?;
    assert_eq!(resumed.session.model_provider_id, "server-provider");
    for explicit in [false, true] {
        server.model_provider_override = explicit.then(|| "client-provider".to_string());
        let expected = if explicit {
            "client-provider"
        } else {
            "server-provider"
        };
        let started = server.start_thread(&client_config).await?;
        let forked = server
            .fork_thread(&local_settings, client_config.clone(), parent)
            .await?;
        assert_eq!(
            (
                started.session.model_provider_id.as_str(),
                forked.session.model_provider_id.as_str()
            ),
            (expected, expected)
        );
    }
    server.model_provider_override = None;
    let path = home.path().join("config.toml");
    let updated = std::fs::read_to_string(&path)?.replacen(
        "model_provider = \"server-provider\"",
        "model_provider = \"client-provider\"",
        1,
    );
    std::fs::write(path, updated)?;
    assert_eq!(
        server.history_model_provider(&client_config).await?,
        Some("client-provider".into())
    );
    assert_eq!(
        server
            .start_thread(&client_config)
            .await?
            .session
            .model_provider_id,
        "client-provider"
    );
    // A fork of the active session preserves its provider even if launch defaults differ.
    server.model_provider_override = Some("client-provider".into());
    let forked = server
        .fork_thread_at(
            &local_settings,
            server_config,
            parent,
            /*last_turn_id*/ None,
            /*before_turn_id*/ None,
            ForkGoalContinuation::StartIfIdle,
            /*selected_profile*/ None,
        )
        .await?;
    assert_eq!(forked.session.model_provider_id, "server-provider");
    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn selected_profile_provider_is_explicit() -> Result<()> {
    let home = tempfile::tempdir()?;
    let profile_path = AbsolutePathBuf::try_from(home.path().join("selected.config.toml"))?;
    std::fs::write(&profile_path, "model_provider = \"openai\"\n")?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .loader_overrides(antex_config::LoaderOverrides {
            user_config_path: Some(profile_path),
            user_config_profile: Some("selected".parse()?),
            ..antex_config::LoaderOverrides::without_managed_config_for_tests()
        })
        .build()
        .await?;
    assert_eq!(
        provider_selection::explicit_provider(&config),
        Some("openai".into())
    );
    Ok(())
}

#[tokio::test]
async fn required_provider_overrides_oss_history_selection() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("requirements.toml"),
        "model_provider = 'openai'",
    )?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .loader_overrides(
            antex_config::LoaderOverrides::with_managed_config_path_for_tests(
                home.path().join("managed_config.toml"),
            ),
        )
        .harness_overrides(ConfigOverrides {
            model_provider: Some("ollama".into()),
            ..Default::default()
        })
        .build()
        .await?;
    let mut server = crate::start_embedded_app_server_for_picker(&config).await?;
    server.model_provider_override = Some("ollama".into());
    assert_eq!(config.model_provider_id, "openai");
    assert_eq!(
        server.history_model_provider(&config).await?,
        Some("openai".into())
    );
    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn provider_switch_preserves_history_and_routes_credentials() -> Result<()> {
    use core_test_support::responses;
    use serde_json::json;
    let first = responses::start_mock_server().await;
    let second = responses::start_mock_server().await;
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            r#"
model_provider = "first"
model = "gpt-5.5"
[model_providers.first]
name = "First"
base_url = "{}/v1"
experimental_bearer_token = "first-secret"
[model_providers.second]
name = "Second"
base_url = "{}/v1"
experimental_bearer_token = "second-secret"
"#,
            first.uri(),
            second.uri()
        ),
    )?;
    let mut config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await?;
    let local_settings = LocalSettings::from(&config);
    let mut server = crate::start_embedded_app_server_for_picker(&config).await?;
    let started = server.start_thread(&config).await?;
    let thread_id = started.session.thread_id;
    for (index, (endpoint, provider, token)) in [
        (&first, "first", "Bearer first-secret"),
        (&second, "second", "Bearer second-secret"),
        (&first, "first", "Bearer first-secret"),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            server.thread_unsubscribe(thread_id).await?;
            config.model_provider_id = provider.into();
            config.model_provider = config.model_providers[provider].clone();
            let defaults = (
                server.model_provider_override.clone(),
                server.default_model.clone(),
            );
            let models = server.available_models.clone();
            let resumed = server
                .select_provider_thread(
                    &local_settings,
                    config.clone(),
                    models,
                    ProviderThreadTarget::Resume(thread_id),
                )
                .await?;
            assert_eq!(
                (
                    server.model_provider_override.clone(),
                    server.default_model.clone()
                ),
                defaults
            );
            assert_eq!(
                (resumed.session.thread_id, resumed.session.model_provider_id),
                (thread_id, provider.to_string())
            );
        }
        let response = responses::mount_sse_once(
            endpoint,
            responses::sse(vec![
                responses::ev_response_created(&format!("response-{index}")),
                responses::ev_assistant_message(&format!("answer-{index}"), "first provider reply"),
                responses::ev_completed(&format!("response-{index}")),
            ]),
        )
        .await;
        server
            .turn_start(
                thread_id,
                format!("provider-turn-{index}"),
                vec![UserInput::Text {
                    text: "hello".into(),
                    text_elements: Vec::new(),
                }],
                config.cwd.to_path_buf(),
                /*approval_policy*/ None,
                /*approvals_reviewer*/ None,
                TurnPermissionsOverride::Preserve,
                &config.workspace_roots,
                "gpt-5.5".into(),
                /*effort*/ None,
                /*summary*/ None,
                /*service_tier*/ None,
                /*collaboration_mode*/ None,
                /*output_schema*/ None,
                antex_protocol::config_types::SubagentSpawnPolicy::default(),
                /*cyber_access_program*/ None,
            )
            .await?;
        tokio::time::timeout(std::time::Duration::from_secs(/*secs*/ 30), async {
            while let Some(event) = server.next_event().await {
                if let AppServerEvent::ServerNotification(notification) = event
                    && let antex_app_server_protocol::ServerNotification::TurnCompleted(completed) =
                        *notification
                {
                    assert_eq!(
                        completed.turn.status,
                        antex_app_server_protocol::TurnStatus::Completed
                    );
                    break;
                }
            }
        })
        .await?;
        let request = response.single_request();
        assert_eq!(
            json!({"model":request.body_json()["model"], "authorization":request.header("authorization"), "history":request.body_json()["input"].to_string().contains("first provider reply")}),
            json!({"model":"gpt-5.5", "authorization":token, "history":index>0})
        );
    }
    let defaults = (
        server.model_provider_override.clone(),
        server.default_model.clone(),
        server.available_models.clone(),
    );
    let mut rejected = config.clone();
    rejected.model_provider_id = "missing-provider".into();
    assert!(
        server
            .select_provider_thread(
                &local_settings,
                rejected,
                Vec::new(),
                ProviderThreadTarget::New
            )
            .await
            .is_err()
    );
    assert_eq!(
        (
            server.model_provider_override.clone(),
            server.default_model.clone(),
            server.available_models.clone()
        ),
        defaults
    );
    let next = server.start_thread(&config).await?;
    assert_eq!(next.session.model_provider_id, "first");
    server.shutdown().await?;
    Ok(())
}
