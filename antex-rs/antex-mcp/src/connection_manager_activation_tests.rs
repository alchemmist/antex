use super::*;
use antex_config::McpServerActivation;
use antex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn activation_uses_session_directory_and_disables_required_stdio_server() {
    let root = tempdir().expect("activation root");
    let outside = tempdir().expect("outside directory");
    let config: McpServerConfig = serde_json::from_value(serde_json::json!({
        "command": "antex-mcp-must-not-start",
        "cwd": root.path(),
        "required": true,
        "activation": { "type": "directories", "roots": [root.path()] }
    }))
    .expect("scoped server configuration");
    let runtime = McpRuntimeContext::new(
        Arc::new(environment_manager_without_environments()),
        outside.path().to_path_buf(),
    );
    let previous = McpConnectionSet::new_uninitialized(
        &Constrained::allow_any(AskForApproval::OnRequest),
        &Constrained::allow_any(PermissionProfile::default()),
        /*prefix_mcp_tool_names*/ true,
    );
    let manager = Arc::new(reconcile_reusable_server(&previous, config, runtime).await);
    assert!(manager.servers.is_empty());
    assert!(manager.required_servers.is_empty());
    let binding = capture_binding(&manager).await;
    assert!(binding.tools().is_empty());
    assert!(binding.list_all_resources(|_| true).await.is_empty());
    assert!(
        binding
            .list_all_resource_templates(|_| true)
            .await
            .is_empty()
    );
    assert!(binding.prepare_call("docs", "search").is_none());
}

#[tokio::test]
async fn activation_reuses_allowed_catalog_but_hides_it_after_leaving_directory() {
    let root = tempdir().expect("activation root");
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).expect("nested directory");
    let outside = tempdir().expect("outside directory");
    let runtime =
        McpRuntimeContext::new(Arc::new(environment_manager_without_environments()), nested);
    let mut config = reusable_server_config("http://127.0.0.1:1/mcp");
    config.activation = Some(McpServerActivation::Directories {
        roots: vec![AbsolutePathBuf::try_from(root.path()).expect("absolute root")],
    });
    let previous = manager_with_reusable_ready_server(
        &config,
        &runtime,
        vec![create_test_tool("docs", "search")],
    )
    .await;
    let inside =
        Arc::new(reconcile_reusable_server(&previous, config.clone(), runtime.clone()).await);
    assert!(previous.shares_test_connection_with(&inside, "docs"));
    let inside_binding = capture_binding(&inside).await;
    assert_eq!(
        model_tool_names(inside_binding.tools()),
        HashSet::from([ToolName::namespaced("mcp__docs", "search")]),
    );
    let outside_runtime = McpRuntimeContext::new(
        Arc::new(environment_manager_without_environments()),
        outside.path().to_path_buf(),
    );
    let outside_manager =
        Arc::new(reconcile_reusable_server(&inside, config, outside_runtime).await);
    assert!(outside_manager.servers.is_empty());
    let outside_binding = capture_binding(&outside_manager).await;
    assert!(outside_binding.tools().is_empty());
    assert!(outside_binding.prepare_call("docs", "search").is_none());
    assert!(
        outside_binding
            .list_all_resources(|_| true)
            .await
            .is_empty()
    );
    assert!(inside_binding.prepare_call("docs", "search").is_some());
}

#[tokio::test]
async fn activation_blocks_http_connection_and_shared_catalog_outside_directory() {
    let root = tempdir().expect("activation root");
    let outside = tempdir().expect("outside directory");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP listener");
    let address = listener.local_addr().expect("listener address");
    let mut config = reusable_server_config(&format!("http://{address}/mcp"));
    config.required = true;
    config.activation = Some(McpServerActivation::Directories {
        roots: vec![AbsolutePathBuf::try_from(root.path()).expect("absolute root")],
    });
    let runtime = McpRuntimeContext::new(
        Arc::new(environment_manager_without_environments()),
        outside.path().to_path_buf(),
    );
    let identity = reusable_server_identity("docs", &config, &runtime);
    let cache = McpToolCatalogCache::default();
    let cache_context = cache
        .context(
            "docs",
            &config,
            &runtime,
            /*resolved_environment*/ None,
            (
                &ElicitationCapability::default(),
                &ClientMcpExtensions::default(),
            ),
            Some((
                &identity,
                crate::McpProtocolMode::Legacy,
                /*agent_plugin*/ false,
            )),
        )
        .expect("shared catalog context");
    cache_context.publish_if_newest(
        cache_context.begin_fetch(),
        &[create_test_tool("docs", "cached_search")],
    );
    let manager = Arc::new(
        McpConnectionSet::new(
            /*previous*/ None,
            McpPublicationGate::already_published(),
            McpRuntimeInput {
                startup_policy: McpStartupPolicy::Eager,
                config: Arc::new(crate::mcp::tests::test_mcp_config(
                    outside.path().to_path_buf(),
                )),
                plugins_available: false,
                ready_selected_capability_roots: Vec::new(),
                mcp_servers: HashMap::from([(
                    "docs".to_string(),
                    EffectiveMcpServer::from_host_config(config),
                )]),
                submit_id: "activation".to_string(),
                tx_event: None,
                startup_cancellation_token: CancellationToken::new(),
                runtime_context: runtime,
                codex_apps_tools_cache: ConnectorRuntimeManager::default(),
                tool_catalog_cache: cache,
                codex_apps_tools_cache_key: ConnectorRuntimeContextKey::personal(
                    /*account_id*/ None, /*chatgpt_user_id*/ None,
                ),
                client_mcp_extensions: ClientMcpExtensions::default(),
                auth: None,
                auth_manager: None,
                elicitation_reviewer: None,
                elicitation_lifecycle: None,
            },
            ElicitationRequestRouter::default(),
        )
        .await,
    );
    assert!(manager.servers.is_empty());
    assert!(manager.required_servers.is_empty());
    let binding = capture_binding(&manager).await;
    assert!(binding.tools().is_empty());
    assert!(binding.prepare_call("docs", "cached_search").is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(/*millis*/ 10), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn activation_skips_http_auth_discovery_outside_directory() {
    let root = tempdir().expect("activation root");
    let outside = tempdir().expect("outside directory");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP listener");
    let address = listener.local_addr().expect("listener address");
    let config: McpServerConfig = serde_json::from_value(serde_json::json!({
        "url": format!("http://{address}/mcp"),
        "activation": { "type": "directories", "roots": [root.path()] }
    }))
    .expect("scoped HTTP server");
    let servers = HashMap::from([(
        "docs".to_string(),
        EffectiveMcpServer::from_host_config(config.clone()),
    )]);
    let runtime = McpRuntimeContext::new(
        Arc::new(environment_manager_without_environments()),
        outside.path().to_path_buf(),
    );
    let statuses = crate::compute_auth_statuses(
        &servers,
        OAuthCredentialsStoreMode::default(),
        AuthKeyringBackendKind::default(),
        /*auth*/ None,
        &runtime,
    )
    .await;
    let mut expected_config = config;
    expected_config.enabled = false;
    assert_eq!(statuses["docs"].config, Some(expected_config));
    assert_eq!(statuses["docs"].auth_state, McpAuthState::Unsupported);
    assert!(
        tokio::time::timeout(Duration::from_millis(/*millis*/ 10), listener.accept())
            .await
            .is_err()
    );
}
