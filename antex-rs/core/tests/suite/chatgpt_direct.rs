//! The real turn path must fail before sending a subscription grant to a custom endpoint.
use antex_core::TurnInputRequest;
use antex_login::AntexAuth;
use antex_login::AuthCredentialsStoreMode;
use antex_login::AuthKeyringBackendKind;
use antex_model_provider_info::ModelProviderInfo;
use antex_protocol::protocol::EventMsg;
use antex_protocol::user_input::UserInput;
use core_test_support::test_antex::test_antex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;
use wiremock::MockServer;

#[tokio::test]
async fn direct_subscription_turn_rejects_custom_endpoint_without_sending_credentials()
-> anyhow::Result<()> {
    let home = Arc::new(tempfile::tempdir()?);
    std::fs::write(
        home.path().join("auth.json"),
        serde_json::to_vec(&json!({
            "auth_mode": "chatgptDirect",
            "tokens": {
                "id_token": "header.e30.signature",
                "access_token": "test-private-direct-token",
                "refresh_token": "test-private-direct-refresh",
                "direct": {
                    "client_id": "issued-client",
                    "expires_at": "2099-01-01T00:00:00Z",
                    "scopes": ["chatgpt.tokens.use.direct"]
                }
            },
            "last_refresh": "2099-01-01T00:00:00Z"
        }))?,
    )?;
    let auth = AntexAuth::from_auth_storage(
        home.path(),
        AuthCredentialsStoreMode::File,
        /*chatgpt_base_url*/ None,
        AuthKeyringBackendKind::default(),
        &antex_login::test_support::transport_default_auth_route_config(),
    )
    .await?
    .expect("isolated direct grant should load");
    let server = MockServer::start().await;
    let provider = ModelProviderInfo::create_openai_provider(Some(format!("{}/v1", server.uri())));
    let test = test_antex()
        .with_home(home)
        .with_auth(auth)
        .with_config(move |config| config.model_provider = provider)
        .build_with_auto_env(&server)
        .await?;
    test.antex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "hello".to_string(),
            text_elements: vec![],
        }]))
        .await?;
    let mut errors = Vec::new();
    wait_for_event(&test.antex, |event| {
        if let EventMsg::Error(error) = event {
            errors.push(error.message.clone());
        }
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(errors, vec![
        "Direct ChatGPT authentication requires https://api.openai.com/v1 without gateway OAuth; use separate provider credentials for custom endpoints".to_string(),
    ]);
    assert!(server.received_requests().await.unwrap().is_empty());
    Ok(())
}
