use super::*;
use antex_app_server_protocol::JSONRPCErrorError;
use pretty_assertions::assert_eq;
use std::collections::VecDeque;

fn routing_error(message: &str) -> TypedRequestError {
    TypedRequestError::Server {
        method: "account/read".to_owned(),
        source: JSONRPCErrorError {
            code: -32603,
            message: message.to_owned(),
            data: None,
        },
    }
}

#[tokio::test(start_paused = true)]
async fn startup_retries_transient_discovery_without_inventing_an_account() {
    let expected = GetAccountResponse {
        account: None,
        requires_openai_auth: true,
        workspace_routing: None,
    };
    let mut replies = VecDeque::from([
        Err(routing_error("workspace routing discovery timed out")),
        Err(routing_error("workspace routing discovery failed")),
        Ok(expected.clone()),
    ]);
    let result = retry_account_read(|| std::future::ready(replies.pop_front().unwrap()))
        .await
        .unwrap();
    assert_eq!(result, expected);
    assert!(replies.is_empty());
}

#[tokio::test(start_paused = true)]
async fn permanent_routing_errors_are_not_retried_or_bypassed() {
    let mut calls = 0;
    let result = retry_account_read(|| {
        calls += 1;
        std::future::ready(Err(routing_error(
            "selected workspace missing from routing discovery",
        )))
    })
    .await;
    assert!(result.is_err());
    assert_eq!(calls, 1);
}

#[tokio::test(start_paused = true)]
async fn repeated_timeouts_remain_bounded_and_visible() {
    let mut calls = 0;
    let result = retry_account_read(|| {
        calls += 1;
        std::future::ready(Err(routing_error("workspace routing discovery timed out")))
    })
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("workspace routing discovery timed out")
    );
    assert_eq!(calls, 3);
}
