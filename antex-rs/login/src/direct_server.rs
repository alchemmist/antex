//! Short-lived loopback server for the public direct ChatGPT OAuth flow.
use std::io;
use std::sync::Arc;
use std::time::Duration;

use antex_protocol::auth::AuthMode;
use chrono::Utc;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;

use crate::AuthDotJson;
use crate::LoginCallbackResult;
use crate::LoginServer;
use crate::ServerOptions;
use crate::ShutdownHandle;
use crate::direct_oauth;
use crate::oauth::generate_pkce;
use crate::oauth::generate_state;

/// Start an explicit public ChatGPT subscription login. Legacy credentials are untouched
/// until a complete valid grant is saved. Unlike legacy login this never cancels another
/// listener, falls back to another port, or uses issuer/client overrides.
/// Callers must enforce their configured allowed login methods before calling this API.
pub fn run_direct_login_server(opts: ServerOptions) -> io::Result<LoginServer> {
    if opts.forced_chatgpt_workspace_id.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Direct ChatGPT login is unavailable under workspace restrictions; use legacy Codex login",
        ));
    }
    if opts.port != 1455 || opts.issuer != "https://auth.openai.com" || opts.force_state.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Direct ChatGPT login requires the official issuer, port 1455, and a random state",
        ));
    }
    // Bind before generating/publishing a URL. Never send a cancellation request to this port.
    let server = Server::http(("127.0.0.1", 1455)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AddrInUse,
            "Cannot bind port 1455; cancel the other pending login and try again",
        )
    })?;
    let client = direct_oauth::client(&opts.auth_route_config)?;
    start_bound(opts, server, client)
}

fn start_bound(
    opts: ServerOptions,
    server: Server,
    client: antex_http_client::HttpClient,
) -> io::Result<LoginServer> {
    start_with_exchange(opts, server, move |code, client_id, pkce| async move {
        direct_oauth::exchange(&client, &code, &client_id, &pkce).await
    })
}

fn start_with_exchange<F, Fut>(
    opts: ServerOptions,
    server: Server,
    exchange: F,
) -> io::Result<LoginServer>
where
    F: FnOnce(String, String, crate::oauth::PkceCodes) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = io::Result<crate::TokenData>> + Send + 'static,
{
    let publication = crate::auth::login_publication::Publication::capture(&opts)?;
    let attempt = Arc::new(crate::auth::login_publication::LoginAttempt::default());
    let actual_port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| io::Error::other("Missing callback address"))?
        .port();
    let host_id = crate::direct_identity::agent_host_id(&opts.codex_home)?;
    let pkce = generate_pkce();
    let state = generate_state();
    let nonce = generate_state();
    let mut url = url::Url::parse(direct_oauth::AUTHORIZE_URL).map_err(io::Error::other)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", "dynamic_agent_client"),
        ("agent_name_hint", "Antex"),
        ("ext_agent_host_id", host_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", direct_oauth::REDIRECT_URI),
        ("resource", direct_oauth::RESOURCE),
        (
            "scope",
            "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
        ),
        ("state", state.as_str()),
        ("nonce", nonce.as_str()),
        ("code_challenge", pkce.code_challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    let auth_url = url.to_string();
    let server = Arc::new(server);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Request>(16);
    let receiver_server = server.clone();
    let receiver = std::thread::spawn(move || {
        while let Ok(request) = receiver_server.recv() {
            if tx.blocking_send(request).is_err() {
                break;
            }
        }
    });
    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    let shutdown_handle = ShutdownHandle {
        shutdown_notify: shutdown_notify.clone(),
        direct_attempt: Some(attempt.clone()),
    };
    let open_browser = opts.open_browser;
    let task = async move {
        let expires_at = tokio::time::Instant::now() + Duration::from_secs(600);
        let deadline = tokio::time::sleep_until(expires_at);
        tokio::pin!(deadline);
        let result = loop {
            let req = tokio::select! {
                biased;
                _ = shutdown_notify.notified() => break Err(io::Error::other("Login canceled")),
                _ = &mut deadline => break Err(io::Error::other("Login timed out")),
                request = rx.recv() => match request {
                    Some(request) => request,
                    None => break Err(io::Error::other("Login callback server stopped")),
                },
            };
            let expected_host = format!("127.0.0.1:{actual_port}");
            let mut hosts = req
                .headers()
                .iter()
                .filter(|header| header.field.equiv("Host"));
            let valid_host = hosts
                .next()
                .is_some_and(|header| header.value.as_str() == expected_host.as_str())
                && hosts.next().is_none();
            let authorization = if req.method() == &tiny_http::Method::Get && valid_host {
                direct_oauth::callback(req.url(), &state)
            } else {
                Err(io::Error::other("Invalid callback request"))
            };
            let (code, client_id) = match authorization {
                Ok(value) => value,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    respond(
                        req,
                        /*status*/ 400,
                        "ChatGPT authorization was not granted.",
                    )
                    .await;
                    break Err(error);
                }
                Err(_) => {
                    respond(
                        req,
                        /*status*/ 400,
                        "Invalid OAuth callback. Return to the sign-in page.",
                    )
                    .await;
                    continue;
                }
            };
            // Cancellation and expiry cover the network exchange, not just waiting for a callback.
            let tokens = tokio::select! {
                biased;
                _ = shutdown_notify.notified() => Err(io::Error::other("Login canceled")),
                _ = &mut deadline => Err(io::Error::other("Login timed out")),
                result = exchange(code, client_id, pkce) => result,
            };
            let result = match tokens {
                Ok(tokens) => {
                    publication
                        .publish(
                            AuthDotJson {
                                auth_mode: Some(AuthMode::ChatgptDirect),
                                openai_api_key: None,
                                tokens: Some(tokens),
                                last_refresh: Some(Utc::now()),
                                agent_identity: None,
                                personal_access_token: None,
                                bedrock_api_key: None,
                                bedrock_access_keys: None,
                            },
                            attempt,
                            &shutdown_notify,
                            expires_at,
                        )
                        .await
                }
                Err(error) => Err(error),
            };
            if result.is_ok() {
                respond(
                    req,
                    /*status*/ 200,
                    "ChatGPT authentication completed. You can close this window.",
                )
                .await;
            } else {
                respond(
                    req,
                    /*status*/ 400,
                    "ChatGPT authentication failed. Return to Antex to try again.",
                )
                .await;
            }
            break result.map(|()| LoginCallbackResult::default());
        };
        drop(rx);
        server.unblock();
        let _ = tokio::task::spawn_blocking(move || receiver.join()).await;
        // tiny_http 0.12 drops its listener on a detached accept thread. Joining
        // our request receiver alone does not release the callback port. Drop
        // the final server owner, then observe release before reporting done.
        drop(server);
        let released = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match std::net::TcpListener::bind(("127.0.0.1", actual_port)) {
                    Ok(listener) => {
                        drop(listener);
                        break Ok(());
                    }
                    Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => break Err(error),
                }
            }
        })
        .await
        .map_err(|_| io::Error::other("OAuth callback listener did not finish shutting down"))?;
        released?;
        result
    };
    let server_handle =
        tokio::spawn(antex_otel::auth_storage::AuthStorageOriginator::current().scope(task));
    if open_browser {
        let _ = webbrowser::open(&auth_url);
    }
    Ok(LoginServer {
        auth_url,
        actual_port,
        server_handle,
        shutdown_handle,
    })
}

async fn respond(request: Request, status: u16, text: &'static str) {
    let _ = tokio::task::spawn_blocking(move || {
        let mut response = Response::from_string(text).with_status_code(status);
        for (name, value) in [
            ("Cache-Control", "no-store"),
            ("Connection", "close"),
            ("Referrer-Policy", "no-referrer"),
        ] {
            if let Ok(header) = tiny_http::Header::from_bytes(name, value) {
                response.add_header(header);
            }
        }
        request.respond(response)
    })
    .await;
}

#[cfg(test)]
#[path = "direct_server_publication_tests.rs"]
mod publication_tests;

#[cfg(test)]
#[path = "direct_server_tests.rs"]
mod tests;
