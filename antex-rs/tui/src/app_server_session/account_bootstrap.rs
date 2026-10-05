use antex_app_server_client::TypedRequestError;
use antex_app_server_protocol::GetAccountResponse;
use std::future::Future;
use std::time::Duration;

const ACCOUNT_DISCOVERY_BUDGET: Duration = Duration::from_secs(120);

pub(super) async fn retry_account_read<F, R>(
    mut request: F,
) -> Result<GetAccountResponse, TypedRequestError>
where
    F: FnMut() -> R,
    R: Future<Output = Result<GetAccountResponse, TypedRequestError>>,
{
    let deadline = tokio::time::Instant::now() + ACCOUNT_DISCOVERY_BUDGET;
    let mut retries = 0_u32;
    loop {
        let result = tokio::time::timeout_at(deadline, request()).await.map_err(|_| {
            TypedRequestError::Transport {
                method: "account/read".to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "account discovery did not recover within two minutes; check connectivity and retry",
                ),
            }
        })?;
        match result {
            Err(error) if transient_routing_error(&error) => {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(error);
                }
                let delay = (Duration::from_secs(1_u64 << retries.min(3))
                    + Duration::from_millis(rand::random_range(0..=250)))
                .min(remaining);
                retries += 1;
                tracing::warn!(retries, delay_ms = delay.as_millis(), %error, "retrying account discovery during startup");
                tokio::time::sleep(delay).await;
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
            }
            result => return result,
        }
    }
}

fn transient_routing_error(error: &TypedRequestError) -> bool {
    matches!(error, TypedRequestError::Server { source, .. }
        if source.code == -32603
            && source.data.as_ref().and_then(|data| data.get("retryable")).and_then(serde_json::Value::as_bool) != Some(false)
            && matches!(source.message.as_str(),
            "workspace routing discovery timed out" |
            "workspace routing discovery failed" |
            "configuration changed during workspace routing discovery; retry account/read"))
}

#[cfg(test)]
#[path = "account_bootstrap_tests.rs"]
mod tests;
