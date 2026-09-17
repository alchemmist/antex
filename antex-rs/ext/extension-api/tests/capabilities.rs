use antex_extension_api::NoopResponseItemInjector;
use antex_extension_api::ResponseItemInjector;
use antex_protocol::models::ContentItem;
use antex_protocol::models::ResponseInputItem;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn noop_response_item_injector_returns_original_items() {
    let items = vec![ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "keep this input".to_string(),
        }],
        phase: None,
    }];

    let returned_items = NoopResponseItemInjector
        .inject_response_items(items.clone())
        .await
        .expect_err("noop injector should reject same-turn injection");

    assert_eq!(returned_items, items);
}
