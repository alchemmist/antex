use super::*;
use antex_http_client::HttpClientBuilder;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;

#[tokio::test]
async fn completed_exchange_waiting_for_storage_can_be_canceled_without_late_publication() {
    let home = tempfile::tempdir().unwrap();
    crate::login_with_api_key(
        home.path(),
        "old-key",
        crate::AuthCredentialsStoreMode::File,
        crate::AuthKeyringBackendKind::default(),
    )
    .unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(home.path().join("auth-mutation.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let mut opts = ServerOptions::new(
        home.path().into(),
        "unused".into(),
        /*forced_chatgpt_workspace_id*/ None,
        crate::AuthCredentialsStoreMode::File,
        crate::AuthKeyringBackendKind::default(),
        crate::test_support::transport_default_auth_route_config(),
    );
    opts.open_browser = false;
    let authority = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id_token":"e30.e30.sig", "access_token":"new-access", "refresh_token":"new-refresh",
            "scope":"chatgpt.tokens.use.direct", "expires_in":3600
        })))
        .expect(1)
        .mount(&authority)
        .await;
    let http = HttpClientBuilder::new()
        .without_redirects()
        .build_direct()
        .unwrap();
    let endpoint = authority.uri();
    let (exchanged_tx, exchanged_rx) = tokio::sync::oneshot::channel();
    let server = start_with_exchange(
        opts,
        Server::http(("127.0.0.1", 0)).unwrap(),
        move |code, id, pkce| async move {
            let result =
                direct_oauth::test_exchange::exchange(&http, &endpoint, &code, &id, &pkce).await;
            exchanged_tx.send(result.is_ok()).unwrap();
            result
        },
    )
    .unwrap();
    let port = server.actual_port;
    let url = url::Url::parse(&server.auth_url).unwrap();
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    let callback = tokio::spawn(async move {
        HttpClientBuilder::new()
            .build_direct()
            .unwrap()
            .get(format!(
                "http://127.0.0.1:{port}/auth/callback?state={state}&code=code&client_id=issued"
            ))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    assert!(exchanged_rx.await.unwrap());
    server.cancel();
    // The lock is STILL held. Cancellation must release the listener, not wait
    // until an unbounded File::lock or a queued background write finishes.
    assert!(
        tokio::time::timeout(Duration::from_secs(2), server.block_until_done())
            .await
            .unwrap()
            .is_err()
    );
    let _port_released = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    assert_eq!(callback.await.unwrap(), 400);
    let logout = {
        let home = home.path().to_path_buf();
        tokio::task::spawn_blocking(move || {
            crate::logout(
                &home,
                crate::AuthCredentialsStoreMode::File,
                crate::AuthKeyringBackendKind::default(),
            )
        })
    };
    drop(lock);
    logout.await.unwrap().unwrap();
    assert!(!home.path().join("auth.json").exists());
    // A later unrelated auth mutation cannot wake a forgotten login write.
    crate::login_with_api_key(
        home.path(),
        "winner",
        crate::AuthCredentialsStoreMode::File,
        crate::AuthKeyringBackendKind::default(),
    )
    .unwrap();
    let doc: AuthDotJson =
        serde_json::from_slice(&std::fs::read(home.path().join("auth.json")).unwrap()).unwrap();
    assert_eq!(doc.openai_api_key.as_deref(), Some("winner"));
}
