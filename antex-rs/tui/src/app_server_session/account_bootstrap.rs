use antex_app_server_client::TypedRequestError;
use antex_app_server_protocol::GetAccountResponse;
use std::future::Future;
use std::time::Duration;

pub(super) async fn retry_account_read<F, R>(
    mut request: F,
) -> Result<GetAccountResponse, TypedRequestError>
where
    F: FnMut() -> R,
    R: Future<Output = Result<GetAccountResponse, TypedRequestError>>,
{
    let mut retries = 0;
    loop {
        match request().await {
            Err(error) if retries < 2 && transient_routing_error(&error) => {
                retries += 1;
                tracing::warn!(retries, %error, "retrying account discovery during startup");
                tokio::time::sleep(Duration::from_millis(500 * retries)).await;
            }
            result => return result,
        }
    }
}

fn transient_routing_error(error: &TypedRequestError) -> bool {
    matches!(error, TypedRequestError::Server { source, .. }
        if source.code == -32603 && matches!(source.message.as_str(),
            "workspace routing discovery timed out" |
            "workspace routing discovery failed" |
            "configuration changed during workspace routing discovery; retry account/read"))
}

#[cfg(test)]
#[path = "account_bootstrap_tests.rs"]
mod tests;
