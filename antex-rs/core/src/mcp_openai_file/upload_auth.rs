//! Adapts the prepared call's strict upload authority to the files API.
use antex_api::AuthHeadersFuture;
use antex_api::AuthProvider;
use antex_mcp::PreparedMcpCall;
use http::HeaderMap;

pub(crate) struct BoundUploadAuth<'a> {
    call: &'a PreparedMcpCall,
    provider: &'a dyn AuthProvider,
}

impl<'a> BoundUploadAuth<'a> {
    pub(crate) fn new(call: &'a PreparedMcpCall) -> Option<Self> {
        Some(Self {
            call,
            provider: call.openai_file_upload_auth()?,
        })
    }
}

impl AuthProvider for BoundUploadAuth<'_> {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        if self.call.ensure_openai_file_upload_owner().is_err() {
            return;
        }
        let resolved = self.provider.to_auth_headers();
        if self.call.ensure_openai_file_upload_owner().is_ok() {
            headers.extend(resolved);
        }
    }

    fn resolve_auth_headers(&self) -> AuthHeadersFuture<'_> {
        Box::pin(async move {
            self.call.ensure_openai_file_upload_owner()?;
            let headers = self.provider.resolve_auth_headers().await?;
            self.call.ensure_openai_file_upload_owner()?;
            Ok(headers)
        })
    }
}
