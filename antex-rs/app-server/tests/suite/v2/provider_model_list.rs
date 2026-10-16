use antex_app_server_protocol::ClientRequest;
use antex_app_server_protocol::JSONRPCErrorError;
use antex_app_server_protocol::ModelListParams;
use antex_app_server_protocol::ModelListResponse;
use antex_app_server_protocol::RequestId;
use anyhow::Result;
use app_test_support::TestAppServer;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[test_case::test_case(true; "enabled")]
#[test_case::test_case(false; "disabled")]
#[tokio::test]
async fn configured_provider_catalogs_use_their_own_credentials(enabled: bool) -> Result<()> {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    let home = tempfile::tempdir()?;
    app_test_support::write_chatgpt_auth(
        home.path(),
        app_test_support::ChatGptAuthFixture::new("subscription-secret").plan_type("pro"),
        antex_config::types::AuthCredentialsStoreMode::File,
    )?;
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            r#"
model_provider = "first"
[features]
api_key_model_discovery = {enabled}
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
    for (server, key) in [
        (&first, "Bearer first-secret"),
        (&second, "Bearer second-secret"),
    ] {
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", key))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data":[{"id":"shared-model"},{"id":"shared-model"},{"id":""}]}),
            ))
            .expect(if enabled { 1 } else { 0 })
            .mount(server)
            .await;
    }
    let mut app = TestAppServer::builder()
        .with_antex_home(home.path())
        .build_initialized()
        .await?;
    for provider in ["first", "second"] {
        if !enabled {
            let id = app
                .send_request("model/list", Some(json!({"modelProvider":provider})))
                .await?;
            let error = tokio::time::timeout(
                std::time::Duration::from_secs(/*secs*/ 10),
                app.read_stream_until_error_message(RequestId::Integer(id)),
            )
            .await??;
            assert_eq!(
                error.error,
                JSONRPCErrorError {
                    code: -32600,
                    message: "API-key model discovery is disabled by configuration.".into(),
                    data: None
                }
            );
            continue;
        }
        let response = app
            .request::<ModelListResponse>(|request_id| ClientRequest::ModelList {
                request_id,
                params: ModelListParams {
                    model_provider: Some(provider.into()),
                    cursor: None,
                    limit: Some(1),
                    include_hidden: None,
                },
            })
            .await;
        let response = response?;
        assert_eq!(
            (
                response
                    .data
                    .into_iter()
                    .map(|model| model.model)
                    .collect::<Vec<_>>(),
                response.next_cursor
            ),
            (vec!["shared-model".to_string()], None)
        );
    }
    Ok(())
}

#[tokio::test]
async fn openai_catalog_requires_its_own_login() -> Result<()> {
    let home = tempfile::tempdir()?;
    let mut app = TestAppServer::builder()
        .with_antex_home(home.path())
        .with_env_overrides(&[
            ("OPENAI_API_KEY", None),
            ("ANTEX_API_KEY", None),
            ("CODEX_API_KEY", None),
        ])
        .build_initialized()
        .await?;
    let id = app
        .send_request("model/list", Some(json!({"modelProvider":"openai"})))
        .await?;
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(/*secs*/ 10),
        app.read_stream_until_error_message(RequestId::Integer(id)),
    )
    .await??;
    assert_eq!(
        error.error,
        JSONRPCErrorError {
            code: -32600,
            message: "Sign in to OpenAI before loading its models.".into(),
            data: None
        }
    );
    Ok(())
}
