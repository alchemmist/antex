use antex_features::Feature;
use antex_protocol::openai_models::ReasoningEffort;
use antex_protocol::protocol::ThreadSettingsOverrides;
use core_test_support::responses::ev_completed;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::submit_thread_settings;
use core_test_support::test_antex::test_antex;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn cursor_fast_switches_model_without_changing_effort() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {"id": "grok-4.7-low"},
                {"id": "grok-4.7-low-fast"}
            ]
        })))
        .with_priority(/*p*/ 1)
        .expect(1..)
        .mount(&server)
        .await;
    let test = test_antex()
        .with_model("grok-4.7")
        .with_config(|config| {
            config.model_provider.name = "Cursor subscription".to_string();
            config.model_provider_id = "cursor".to_string();
            config.model_provider.experimental_bearer_token = Some("cursor-test-token".into());
            config.model_provider.requires_openai_auth = false;
            config.model_catalog = None;
            config.model_reasoning_effort = Some(ReasoningEffort::Low);
            config
                .features
                .enable(Feature::ApiKeyModelDiscovery)
                .expect("enable provider model discovery");
            config
                .features
                .enable(Feature::FastMode)
                .expect("enable Fast mode");
        })
        .build_with_auto_env(&server)
        .await?;
    for (tier, model) in [
        (None, "grok-4.7"),
        (Some("priority"), "grok-4.7-fast"),
        (None, "grok-4.7"),
    ] {
        submit_thread_settings(
            &test.antex,
            ThreadSettingsOverrides {
                service_tier: Some(tier.map(str::to_string)),
                ..Default::default()
            },
        )
        .await?;
        let response = mount_sse_once(&server, sse(vec![ev_completed("done")])).await;
        test.submit_text_turn("hello").await?;
        let body = response.single_request().body_json();
        assert_eq!(
            json!({
                "model": body["model"],
                "effort": body["reasoning"]["effort"],
                "service_tier": body.get("service_tier"),
            }),
            json!({"model": model, "effort": "low", "service_tier": null})
        );
    }
    Ok(())
}
