const PERSONAL_ACCESS_TOKEN_PREFIX: &str = "at-";

pub(super) enum AntexAccessToken<'a> {
    PersonalAccessToken(&'a str),
    AgentIdentityJwt(&'a str),
}

pub(super) fn classify_antex_access_token(access_token: &str) -> AntexAccessToken<'_> {
    if access_token.starts_with(PERSONAL_ACCESS_TOKEN_PREFIX) {
        AntexAccessToken::PersonalAccessToken(access_token)
    } else {
        AntexAccessToken::AgentIdentityJwt(access_token)
    }
}

#[cfg(test)]
#[path = "access_token_tests.rs"]
mod tests;
