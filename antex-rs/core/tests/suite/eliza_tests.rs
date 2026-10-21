use antex_features::Feature;
use antex_model_provider_info::WireApi;
use core_test_support::test_antex::test_antex;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn eliza_executes_a_tool_and_receives_its_result() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"internal-test","title":"Internal test","vendor":"internal","namespace":"communal","endpoint":"/internal/internal-test/v1/chat/completions","architecture":{"context_length":32768,"max_output_tokens":8192}}]})))
        .mount(&server).await;
    let attempt = std::sync::atomic::AtomicUsize::new(0);
    Mock::given(method("POST")).and(path("/raw/internal/internal-test/v1/chat/completions"))
        .and(header("authorization","Bearer internal-token"))
        .respond_with(move |request: &wiremock::Request| {
            if attempt.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                return ResponseTemplate::new(429).insert_header("retry-after", "0");
            }
            let body: serde_json::Value = serde_json::from_slice(&request.body).expect("chat request");
            let has_result = body["messages"].as_array().expect("messages").iter().any(|m| m["role"] == "tool");
            let (message, reason) = if has_result {
                (json!({"role":"assistant","content":"323"}),"stop")
            } else {
                let tool = body["tools"].as_array().expect("tools").iter().find(|t| t["function"]["name"].as_str().is_some_and(|name| name.starts_with("exec_command_"))).expect("exec command tool");
                (json!({"role":"assistant","content":null,"tool_calls":[{"id":"eliza-call","type":"function","function":{"name":tool["function"]["name"],"arguments":"{\"cmd\":\"echo 323\"}"}}]}),"tool_calls")
            };
            ResponseTemplate::new(200).set_body_json(json!({"id":"chat-test","choices":[{"message":message,"finish_reason":reason}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}))
        }).expect(3).mount(&server).await;
    let test = test_antex()
        .with_model("internal-test")
        .with_config(|config| {
            config.model_provider.name = "Yandex Internal".to_string();
            config.model_provider.wire_api = WireApi::Eliza;
            config.model_provider.requires_openai_auth = false;
            config.model_provider.experimental_bearer_token = Some("internal-token".into());
            config.model_provider_id = "eliza".to_string();
            config.model_catalog = None;
            config
                .features
                .enable(Feature::ApiKeyModelDiscovery)
                .expect("discovery");
            config
                .features
                .disable(Feature::CodeModeOnly)
                .expect("direct tools");
            config
                .features
                .disable(Feature::CodeMode)
                .expect("direct tools");
        })
        .build_with_auto_env(&server)
        .await?;
    test.submit_turn("Calculate 17*19 using a shell command")
        .await?;
    let requests = server.received_requests().await.expect("recording");
    let posts: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
    let body: serde_json::Value =
        serde_json::from_slice(&posts.last().expect("completion request").body)?;
    let result = body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("tool result");
    assert_eq!(result["tool_call_id"], "eliza-call");
    assert!(result["content"].as_str().expect("content").contains("323"));
    assert!(body.get("service_tier").is_none());
    assert!(body.get("reasoning").is_none());
    Ok(())
}
