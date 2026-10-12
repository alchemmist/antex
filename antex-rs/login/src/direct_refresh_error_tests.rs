use crate::RefreshTokenError;
use crate::auth::RefreshTokenFailedReason;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;

#[tokio::test]
async fn definite_rejections_are_typed_without_exposing_issuer_diagnostics() {
    let tokens: crate::TokenData = serde_json::from_value(json!({
        "id_token":"e30.e30.sig", "access_token":"access-secret", "refresh_token":"refresh-secret",
        "direct":{"client_id":"issued", "expires_at":chrono::Utc::now(), "scopes":["chatgpt.tokens.use.direct"]}
    })).unwrap();
    let http = antex_http_client::HttpClientBuilder::new()
        .without_redirects()
        .build_direct()
        .unwrap();
    for (status, code, reason) in [
        (400, "invalid_grant", RefreshTokenFailedReason::Other),
        (
            401,
            "refresh_token_expired",
            RefreshTokenFailedReason::Expired,
        ),
        (
            403,
            "refresh_token_revoked",
            RefreshTokenFailedReason::Revoked,
        ),
        (
            400,
            "refresh_token_invalidated",
            RefreshTokenFailedReason::Revoked,
        ),
        (
            400,
            "refresh_token_reused",
            RefreshTokenFailedReason::Exhausted,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "error":{"code":code, "message":"issuer-private-diagnostic refresh-secret"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let error = crate::direct_oauth::test_exchange::refresh(&http, &server.uri(), &tokens)
            .await
            .unwrap_err();
        assert_eq!(error.failed_reason(), Some(reason));
        assert!(matches!(error, RefreshTokenError::Permanent(_)));
        for secret in [
            "issuer-private-diagnostic",
            "refresh-secret",
            "access-secret",
        ] {
            assert!(!format!("{error:?} {error}").contains(secret));
        }
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let unavailable = format!("http://{}/token", listener.local_addr().unwrap());
    drop(listener);
    assert!(matches!(
        crate::direct_oauth::test_exchange::refresh(&http, &unavailable, &tokens).await,
        Err(RefreshTokenError::Transient(_))
    ));
}
