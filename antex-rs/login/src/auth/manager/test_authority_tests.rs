//! Hermetic HTTP authority with an explicit first-response barrier. No sleeps
//! determine when a login/logout races the request; tests release the response.
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use tokio::sync::mpsc;

pub(super) struct Authority {
    pub url: String,
    pub requests: mpsc::UnboundedReceiver<Value>,
    release: Arc<Notify>,
    count: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Authority {
    pub async fn start(
        response: impl Fn(usize, &Value) -> (u16, Value) + Send + Sync + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let (tx, requests) = mpsc::unbounded_channel();
        let release = Arc::new(Notify::new());
        let count = Arc::new(AtomicUsize::new(0));
        let task_release = release.clone();
        let task_count = count.clone();
        let response = Arc::new(response);
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let tx = tx.clone();
                        let release = task_release.clone();
                        let count = task_count.clone();
                        let response = response.clone();
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            let (header_end, length) = loop {
                                let mut buffer = [0u8; 2048];
                                let read = socket.read(&mut buffer).await.unwrap();
                                assert_ne!(read, 0);
                                bytes.extend_from_slice(&buffer[..read]);
                                assert!(bytes.len() <= 65536);
                                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                                    let headers = String::from_utf8_lossy(&bytes[..end]);
                                    let length = headers.lines().find_map(|line| {
                                        let (name, value) = line.split_once(':')?;
                                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                                    }).unwrap();
                                    break (end + 4, length);
                                }
                            };
                            while bytes.len() < header_end + length {
                                let mut buffer = [0u8; 2048];
                                let read = socket.read(&mut buffer).await.unwrap();
                                assert_ne!(read, 0);
                                bytes.extend_from_slice(&buffer[..read]);
                            }
                            let body = &bytes[header_end..header_end + length];
                            let request = if body.starts_with(b"{") {
                                serde_json::from_slice(body).unwrap()
                            } else {
                                Value::Object(url::form_urlencoded::parse(body).into_owned()
                                    .map(|(key, value)| (key, Value::String(value))).collect())
                            };
                            let index = count.fetch_add(1, Ordering::SeqCst);
                            // The authority consumes/rotates before exposing the
                            // request barrier; only delivery of its response waits.
                            let (status, body) = response(index, &request);
                            tx.send(request.clone()).unwrap();
                            if index == 0 { release.notified().await; }
                            let body = serde_json::to_string(&body).unwrap();
                            let wire = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = socket.write_all(wire.as_bytes()).await;
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); }
                }
            }
        });
        Self {
            url,
            requests,
            release,
            count,
            task,
        }
    }

    pub fn release(&self) {
        self.release.notify_one();
    }
    pub fn calls(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

impl Drop for Authority {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) fn document(mode: super::AuthMode, registration: &str) -> super::AuthDotJson {
    use base64::Engine;
    let claims = serde_json::json!({"https://api.openai.com/auth": {
        "chatgpt_account_id":"workspace-a", "chatgpt_user_id":"user-a"
    }});
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    let mut value = serde_json::json!({
        "auth_mode":mode,
        "tokens": {
            "id_token":format!("e30.{payload}.sig"), "access_token":"access-original",
            "refresh_token":"refresh-original", "account_id":"workspace-a"
        },
        "last_refresh":chrono::Utc::now()
    });
    if mode == super::AuthMode::ChatgptDirect {
        value["tokens"]["direct"] = serde_json::json!({
            "client_id":registration, "expires_at":chrono::Utc::now() + chrono::Duration::hours(1),
            "scopes":["chatgpt.tokens.use.direct"]
        });
    }
    serde_json::from_value(value).unwrap()
}

pub(super) async fn manager(
    home: &std::path::Path,
    doc: &super::AuthDotJson,
    mode: super::AuthCredentialsStoreMode,
) -> Arc<super::AuthManager> {
    super::save_auth(home, doc, mode, super::AuthKeyringBackendKind::default()).unwrap();
    // Non-injected production destinations remain policy-denied even if a
    // regression unexpectedly reaches a second refresh/recovery step.
    let policy = antex_http_client::NetworkPolicyController::default();
    let route = crate::AuthRouteConfig::from_http_client_factory(
        antex_http_client::HttpClientFactory::new(
            antex_http_client::OutboundProxyPolicy::ReqwestDefault,
        )
        .with_network_policy(policy.policy()),
    );
    let auth = super::AntexAuth::from_auth_dot_json(
        home,
        doc.clone(),
        mode,
        /*chatgpt_base_url*/ None,
        super::AuthKeyringBackendKind::default(),
        /*agent_identity_authapi_base_url*/ None,
        &route,
    )
    .await
    .unwrap();
    let mut manager = super::AuthManager::from_auth_for_testing_with_home(auth, home.into());
    Arc::get_mut(&mut manager).unwrap().auth_route_config = route;
    manager
}

/// Inject only the local authority exchange; ownership, rotation publication,
/// manager reload and permanent-error caching are the production operations.
pub(super) async fn refresh(
    manager: &super::AuthManager,
    url: &str,
) -> Result<(), super::RefreshTokenError> {
    assert_eq!(url::Url::parse(url).unwrap().host_str(), Some("127.0.0.1"));
    let _guard = manager.refresh_lock.acquire().await.unwrap();
    let http = antex_http_client::HttpClientBuilder::new()
        .without_redirects()
        .build_direct()
        .unwrap();
    let url = url.to_string();
    manager
        .refresh_with(|attempted| async move {
            let auth = attempted.unwrap();
            manager
                .refresh_managed_with(&auth, move |original| async move {
                    let tokens = original.tokens.as_ref().unwrap();
                    if original.resolved_mode() == super::AuthMode::ChatgptDirect {
                        let rotated =
                            crate::direct_oauth::test_exchange::refresh(&http, &url, tokens)
                                .await?;
                        super::direct::refreshed_document(original, rotated).map_err(Into::into)
                    } else {
                        let oauth = crate::oauth::OAuthClient::new(
                            &http,
                            crate::oauth::TokenEndpoint {
                                url: &url,
                                client_id: super::CLIENT_ID,
                                encoding: crate::oauth::TokenEncoding::Json,
                                timeout: Some(std::time::Duration::from_secs(2)),
                                error_body_limit: crate::oauth::ErrorBodyLimit::Bytes(4096),
                            },
                        );
                        let response = oauth
                            .refresh(crate::oauth::RefreshTokenGrant {
                                refresh_token: &tokens.refresh_token,
                                resource: None,
                            })
                            .await
                            .map_err(crate::direct_refresh_error::classify)?;
                        super::refresh::refreshed_legacy(original, response)
                    }
                })
                .await
        })
        .await
}
