use super::*;
use antex_api::AuthError;
use antex_api::AuthHeadersFuture;
use antex_api::AuthProvider;
use antex_exec_server::HttpRequestResponse;
use futures::future::BoxFuture;

struct NoDispatch;

impl HttpClient for NoDispatch {
    fn http_request(
        &self,
        _params: HttpRequestParams,
    ) -> BoxFuture<'_, Result<HttpRequestResponse, ExecServerError>> {
        Box::pin(async { panic!("failed auth must not dispatch buffered MCP HTTP") })
    }

    fn http_request_stream(
        &self,
        _params: HttpRequestParams,
    ) -> BoxFuture<'_, Result<(HttpRequestResponse, HttpResponseBodyStream), ExecServerError>> {
        Box::pin(async { panic!("failed auth must not dispatch streaming MCP HTTP") })
    }
}

struct CachedButUnavailableAuth;

impl AuthProvider for CachedButUnavailableAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer stale-synthetic"),
        );
    }

    fn resolve_auth_headers(&self) -> AuthHeadersFuture<'_> {
        Box::pin(async { Err(AuthError::Transient("synthetic source unavailable".into())) })
    }
}

#[tokio::test]
async fn all_streamable_http_operations_reject_failed_request_auth_before_dispatch() {
    let adapter = StreamableHttpClientAdapter::new(
        Arc::new(NoDispatch),
        HeaderMap::new(),
        Some(Arc::new(CachedButUnavailableAuth)),
        /*has_configured_headers*/ false,
        StreamableHttpRedirectMode::Legacy,
        Arc::new(Mutex::new(None)),
    );
    for _ in 0..2 {
        let message = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "ping"
        }))
        .unwrap();
        assert!(
            adapter
                .post_message(
                    Arc::from("https://mcp.example.com/mcp"),
                    message,
                    /*session_id*/ None,
                    /*auth_token*/ None,
                    HashMap::new(),
                )
                .await
                .is_err()
        );
        assert!(
            adapter
                .delete_session(
                    Arc::from("https://mcp.example.com/mcp"),
                    Arc::from("session"),
                    /*auth_token*/ None,
                    HashMap::new(),
                )
                .await
                .is_err()
        );
        assert!(
            adapter
                .get_stream(
                    Arc::from("https://mcp.example.com/mcp"),
                    /*session_id*/ None,
                    /*last_event_id*/ None,
                    /*auth_token*/ None,
                    HashMap::new(),
                )
                .await
                .is_err()
        );
    }
}
