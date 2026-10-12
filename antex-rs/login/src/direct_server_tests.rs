use super::*;
use antex_http_client::HttpClientBuilder;
use antex_http_client::HttpClientFactory;
use antex_http_client::NetworkPolicyController;
use antex_http_client::OutboundProxyPolicy;
use pretty_assertions::assert_eq;

fn options(home: &std::path::Path) -> ServerOptions {
    let mut options = ServerOptions::new(
        home.to_path_buf(),
        "legacy-client-must-not-be-used".into(),
        /*forced_chatgpt_workspace_id*/ None,
        crate::AuthCredentialsStoreMode::File,
        crate::AuthKeyringBackendKind::default(),
        crate::test_support::transport_default_auth_route_config(),
    );
    options.open_browser = false;
    options
}

fn start(home: &std::path::Path) -> LoginServer {
    let opts = options(home);
    let policy = NetworkPolicyController::default();
    let route = crate::AuthRouteConfig::from_http_client_factory(
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault)
            .with_network_policy(policy.policy()),
    );
    // A real policy-aware client, with no published permission, makes any token
    // exchange fail before network I/O. No test can send a grant to OpenAI.
    start_bound(
        opts,
        Server::http(("127.0.0.1", 0)).unwrap(),
        direct_oauth::client(&route).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn malformed_callbacks_do_not_consume_attempt_and_failed_exchange_preserves_credentials() {
    let home = tempfile::tempdir().unwrap();
    crate::login_with_api_key(
        home.path(),
        "existing-secret",
        crate::AuthCredentialsStoreMode::File,
        crate::AuthKeyringBackendKind::default(),
    )
    .unwrap();
    let original = std::fs::read(home.path().join("auth.json")).unwrap();
    let server = start(home.path());
    let authorize = url::Url::parse(&server.auth_url).unwrap();
    let params: std::collections::HashMap<_, _> = authorize.query_pairs().into_owned().collect();
    assert_eq!(params["client_id"], "dynamic_agent_client");
    assert_eq!(params["agent_name_hint"], "Antex");
    assert_eq!(
        params["ext_agent_host_id"],
        crate::direct_identity::agent_host_id(home.path()).unwrap()
    );
    assert_ne!(params["state"], params["nonce"]);
    assert!(params["state"].len() >= 32);
    assert!(params["nonce"].len() >= 32);
    let client = HttpClientBuilder::new().build_direct().unwrap();
    let base = format!("http://127.0.0.1:{}", server.actual_port);
    let response = client
        .get(format!(
            "{base}/auth/callback?state=wrong&code=secret&client_id=issued"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    assert!(!response.text().await.unwrap().contains("secret"));
    let response = client
        .post(format!(
            "{base}/auth/callback?state={}&code=secret&client_id=issued",
            params["state"]
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let response = client
        .get(format!(
            "{base}/auth/callback?state={}&code=secret&client_id=issued",
            params["state"]
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    assert!(server.block_until_done().await.is_err());
    assert_eq!(
        std::fs::read(home.path().join("auth.json")).unwrap(),
        original
    );
}

#[tokio::test]
async fn cancellation_releases_callback_port_and_does_not_write_credentials() {
    let home = tempfile::tempdir().unwrap();
    let server = start(home.path());
    let port = server.actual_port;
    server.cancel();
    assert!(server.block_until_done().await.is_err());
    let _listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    assert!(!home.path().join("auth.json").exists());
}

#[tokio::test]
async fn public_entrypoint_rejects_restrictions_and_untrusted_options_before_binding() {
    let home = tempfile::tempdir().unwrap();
    let mut opts = options(home.path());
    opts.forced_chatgpt_workspace_id = Some(vec!["restricted".into()]);
    assert_eq!(
        run_direct_login_server(opts).err().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    let mutations: [fn(&mut ServerOptions); 3] = [
        |opts: &mut ServerOptions| opts.issuer = "https://unrelated.test".into(),
        |opts: &mut ServerOptions| opts.port = 1457,
        |opts: &mut ServerOptions| opts.force_state = Some("fixed".into()),
    ];
    for mutate in mutations {
        let mut opts = options(home.path());
        mutate(&mut opts);
        assert_eq!(
            run_direct_login_server(opts).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}

#[tokio::test]
#[serial_test::serial(direct_callback_port)]
async fn occupied_port_is_not_canceled_or_replaced() {
    let home = tempfile::tempdir().unwrap();
    // If a real pending login owns this port, the entrypoint still must fail and
    // must not send it a cancellation request. Never connect to that listener.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 1455)).ok();
    assert_eq!(
        run_direct_login_server(options(home.path()))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::AddrInUse
    );
    if let Some(listener) = listener {
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    assert!(!home.path().join("auth.json").exists());
}
