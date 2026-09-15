use antex_protocol::protocol::Op;
use anyhow::Result;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_antex::test_antex;
use pretty_assertions::assert_eq;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspection_includes_the_complete_latest_model_request() -> Result<()> {
    let server = start_mock_server().await;
    let response = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("response-1"),
            ev_assistant_message("message-1", "done"),
            ev_completed("response-1"),
        ]),
    )
    .await;
    let test = test_antex().build_with_auto_env(&server).await?;

    test.submit_turn("show the complete request").await?;
    let expected = response.single_request().body_json();
    let (reply, inspection) = tokio::sync::oneshot::channel();
    test.antex.submit(Op::InspectContext { reply }).await?;

    let captured = inspection
        .await?
        .latest_model_request
        .map(|request| serde_json::from_str(&request))
        .transpose()?;
    assert_eq!(captured, Some(expected));
    Ok(())
}
