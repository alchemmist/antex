use super::*;
use base64::Engine;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;

fn response() -> serde_json::Value {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(br#"{"email":"direct@example.test"}"#);
    json!({
        "id_token": format!("e30.{payload}.sig"),
        "access_token": "secret-access",
        "refresh_token": "secret-refresh",
        "scope": "openid resource.invoke chatgpt.tokens.use.direct",
        "expires_in": 3600
    })
}

fn tokens() -> TokenData {
    serde_json::from_value::<TokenResponse>(response())
        .unwrap()
        .into_tokens("issued-client", /*previous*/ None)
        .unwrap()
}

#[test]
fn callback_binds_state_code_and_issued_client_and_rejects_ambiguity() {
    assert_eq!(
        callback(
            "/auth/callback?state=expected&code=a%2Bb&client_id=issued",
            "expected"
        )
        .unwrap(),
        ("a+b".to_string(), "issued".to_string())
    );
    for callback_url in [
        "/auth/callback?state=wrong&code=secret&client_id=issued",
        "/auth/callback?state=expected&code=secret",
        "/auth/callback?state=expected&client_id=issued",
        "/auth/callback?state=expected&code=secret&client_id=dynamic_agent_client",
        "/auth/callback?state=expected&code=secret&client_id=%20",
        "/auth/callback?state=expected&code=secret&client_id=x%0Ay",
        "/auth/callback?state=expected&state=other&code=secret&client_id=issued",
        "/auth/callback?state=expected&code=secret&client_id=issued&client_id=other",
        "/auth/callback?state=expected&code=secret&client_id=issued&error=secret",
        "/auth/callback?state=expected&code=secret&client_id=issued#fragment",
        "http://evil.test/auth/callback?state=expected&code=secret&client_id=issued",
        "//evil.test/auth/callback?state=expected&code=secret&client_id=issued",
        "/auth/callback/../callback?state=expected&code=secret&client_id=issued",
    ] {
        let error = callback(callback_url, "expected").unwrap_err();
        assert!(!format!("{error:?}").contains("secret"));
    }
}

#[test]
fn response_requires_scope_tokens_id_and_finite_positive_expiry() {
    for (field, value) in [
        ("scope", json!("openid resource.invoke")),
        ("scope", json!("chatgpt.tokens.use.direct.fake")),
        ("access_token", json!(" ")),
        ("refresh_token", json!("")),
        ("id_token", json!(null)),
        ("id_token", json!("")),
        ("expires_in", json!(0)),
        ("expires_in", json!(-1)),
        ("expires_in", json!(1e100)),
    ] {
        let mut value_response = response();
        value_response[field] = value;
        assert!(
            serde_json::from_value::<TokenResponse>(value_response)
                .unwrap()
                .into_tokens("issued", /*previous*/ None)
                .is_err(),
            "accepted invalid {field}"
        );
    }
    for expires_in in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut response: TokenResponse = serde_json::from_value(response()).unwrap();
        response.expires_in = expires_in;
        assert!(response.into_tokens("issued", /*previous*/ None).is_err());
    }
    let tokens = tokens();
    let direct = tokens.direct.as_ref().unwrap();
    assert!(direct.expires_at > Utc::now() + chrono::Duration::minutes(59));
    assert!(direct.expires_at <= Utc::now() + chrono::Duration::hours(1));
    assert_eq!(
        serde_json::from_value::<TokenData>(serde_json::to_value(&tokens).unwrap()).unwrap(),
        tokens
    );
    let mut legacy = serde_json::to_value(tokens).unwrap();
    legacy.as_object_mut().unwrap().remove("direct");
    assert!(
        serde_json::from_value::<TokenData>(legacy)
            .unwrap()
            .direct
            .is_none()
    );
}

#[tokio::test]
async fn code_exchange_binds_issued_id_pkce_resource_and_rotation_preserves_registration() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response()))
        .expect(1)
        .mount(&server)
        .await;
    let http = HttpClientBuilder::new()
        .without_redirects()
        .build_direct()
        .unwrap();
    let pkce = crate::oauth::generate_pkce();
    let original = exchange_at(&http, &server.uri(), "code+&value", "issued-client", &pkce)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let form: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(&requests[0].body)
        .into_owned()
        .collect();
    assert_eq!(
        form,
        std::collections::BTreeMap::from([
            ("grant_type".into(), "authorization_code".into()),
            ("client_id".into(), "issued-client".into()),
            ("code".into(), "code+&value".into()),
            ("code_verifier".into(), pkce.code_verifier),
            ("redirect_uri".into(), REDIRECT_URI.into()),
            ("resource".into(), RESOURCE.into()),
        ])
    );
    assert!(!requests[0].headers.contains_key("authorization"));
    assert!(!requests[0].headers.contains_key("chatgpt-account-id"));
    server.reset().await;
    let mut rotated = response();
    rotated.as_object_mut().unwrap().remove("id_token");
    rotated["access_token"] = json!("rotated-access");
    rotated["refresh_token"] = json!("rotated-refresh");
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rotated))
        .expect(1)
        .mount(&server)
        .await;
    let refreshed = refresh_at(&http, &server.uri(), &original).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let form: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(&requests[0].body)
        .into_owned()
        .collect();
    assert_eq!(
        form,
        std::collections::BTreeMap::from([
            ("grant_type".into(), "refresh_token".into()),
            ("client_id".into(), "issued-client".into()),
            ("refresh_token".into(), "secret-refresh".into()),
            ("resource".into(), RESOURCE.into()),
        ])
    );
    assert_eq!(
        (
            &refreshed.access_token,
            &refreshed.refresh_token,
            &refreshed.id_token
        ),
        (
            &"rotated-access".to_string(),
            &"rotated-refresh".to_string(),
            &original.id_token
        )
    );
    assert_eq!(
        refreshed.direct.as_ref().unwrap().client_id,
        "issued-client"
    );
}

#[tokio::test]
async fn refresh_rejects_redirects_and_missing_scope_without_mutating_old_tokens() {
    let server = MockServer::start().await;
    let unrelated = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(307).insert_header("Location", unrelated.uri()))
        .expect(1)
        .mount(&server)
        .await;
    let http = HttpClientBuilder::new()
        .without_redirects()
        .build_direct()
        .unwrap();
    let original = tokens();
    assert!(refresh_at(&http, &server.uri(), &original).await.is_err());
    assert!(unrelated.received_requests().await.unwrap().is_empty());
    server.reset().await;
    let mut denied = response();
    denied["scope"] = json!("openid");
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(denied))
        .expect(1)
        .mount(&server)
        .await;
    let before = original.clone();
    assert!(refresh_at(&http, &server.uri(), &original).await.is_err());
    assert_eq!(original, before);
    let debug = format!("{original:?} {:?}", original.id_token);
    for secret in [
        "secret-access",
        "secret-refresh",
        "direct@example.test",
        &original.id_token.raw_jwt,
    ] {
        assert!(!debug.contains(secret));
    }
}
