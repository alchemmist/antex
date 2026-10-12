//! Private per-client transport substitution for tests of the real Responses retry loop.
//! Routing/client construction and request authentication still run before this boundary.
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use antex_http_client::HttpTransport;
use antex_http_client::Request;
use antex_http_client::ReqwestTransport;
use antex_http_client::Response;
use antex_http_client::StreamResponse;
use antex_http_client::TransportError;

type StreamFuture = Pin<Box<dyn Future<Output = Result<StreamResponse, TransportError>> + Send>>;

#[derive(Clone)]
pub(super) struct StreamHook(Arc<dyn Fn(Request) -> StreamFuture + Send + Sync>);

impl StreamHook {
    pub(super) fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<StreamResponse, TransportError>> + Send + 'static,
    {
        Self(Arc::new(move |request| Box::pin(callback(request))))
    }
}

impl fmt::Debug for StreamHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StreamHook(<test callback>)")
    }
}

impl super::ModelClient {
    pub(crate) fn set_test_stream_transport<F, Fut>(&mut self, callback: F)
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<StreamResponse, TransportError>> + Send + 'static,
    {
        Arc::get_mut(&mut self.state)
            .expect("install before sharing the client")
            .test_transport = Some(StreamHook::new(callback));
    }
}

pub(super) struct ApiTransport {
    real: ReqwestTransport,
    hook: Option<StreamHook>,
}

impl ApiTransport {
    pub(super) fn new(real: ReqwestTransport, hook: Option<StreamHook>) -> Self {
        Self { real, hook }
    }
}

impl HttpTransport for ApiTransport {
    async fn execute(&self, request: Request) -> Result<Response, TransportError> {
        if self.hook.is_some() {
            // Never accidentally reach a real unary endpoint from a scripted streaming test.
            return Err(TransportError::Build(
                "unexpected unary request in streaming test".to_string(),
            ));
        }
        self.real.execute(request).await
    }

    async fn stream(&self, request: Request) -> Result<StreamResponse, TransportError> {
        match &self.hook {
            Some(hook) => (hook.0)(request).await,
            None => self.real.stream(request).await,
        }
    }
}
