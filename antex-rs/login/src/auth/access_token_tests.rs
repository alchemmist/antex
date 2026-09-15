use super::*;

#[test]
fn classifies_personal_access_tokens_by_prefix() {
    assert!(matches!(
        classify_antex_access_token("at-example"),
        AntexAccessToken::PersonalAccessToken("at-example")
    ));
    assert!(matches!(
        classify_antex_access_token("header.payload.signature"),
        AntexAccessToken::AgentIdentityJwt("header.payload.signature")
    ));
}
