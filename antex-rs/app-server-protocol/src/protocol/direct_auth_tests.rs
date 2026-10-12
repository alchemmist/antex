use super::AuthMode;
use super::ClientRequest;
use crate::LoginAccountParams;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn direct_browser_login_round_trips_through_the_public_rpc_without_secrets() {
    let wire = json!({"method":"account/login/start", "id":7, "params":{"type":"chatgptDirect"}});
    let request: ClientRequest = serde_json::from_value(wire.clone()).unwrap();
    assert!(matches!(
        &request,
        ClientRequest::LoginAccount {
            params: LoginAccountParams::ChatgptDirect,
            ..
        }
    ));
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
}

#[test]
fn direct_auth_mode_does_not_alias_legacy_wire_modes() {
    for wire in [
        "chatgptDirect",
        "chatgpt",
        "chatgptAuthTokens",
        "apikey",
        "bedrockApiKey",
    ] {
        let mode: AuthMode = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(wire));
        if wire == "chatgptDirect" {
            assert!(mode.has_chatgpt_account());
            assert!(!mode.uses_antex_backend());
        }
    }
}
