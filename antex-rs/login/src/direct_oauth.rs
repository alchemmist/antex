//! Public ChatGPT subscription grants. Endpoints are intentionally not configurable:
//! legacy issuer/client overrides must never receive direct subscription credentials.
use std::io;
use std::time::Duration;

use antex_http_client::ClientRouteClass;
use antex_http_client::HttpClient;
use antex_http_client::HttpClientBuilder;
use chrono::Utc;
use serde::Deserialize;

use crate::AuthRouteConfig;
use crate::oauth::AuthorizationCodeGrant;
use crate::oauth::ErrorBodyLimit;
use crate::oauth::OAuthClient;
use crate::oauth::PkceCodes;
use crate::oauth::RefreshTokenGrant;
use crate::oauth::TokenEncoding;
use crate::oauth::TokenEndpoint;
use crate::token_data::DirectTokenMetadata;
use crate::token_data::TokenData;
use crate::token_data::parse_chatgpt_jwt_claims;

pub(crate) const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub(crate) const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub(crate) const RESOURCE: &str = "https://api.openai.com/v1";
pub(crate) const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
pub(crate) const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";

pub(crate) fn client(route: &AuthRouteConfig) -> io::Result<HttpClient> {
    HttpClientBuilder::new()
        .without_redirects()
        .without_request_logging()
        .build_respecting_outbound_proxy_policy(
            &route.authentication_factory(TOKEN_URL),
            TOKEN_URL,
            ClientRouteClass::Auth,
        )
        .map_err(|_| {
            io::Error::other("Could not construct the direct ChatGPT authentication client")
        })
}

fn endpoint<'a>(http: &'a HttpClient, url: &'a str, client_id: &'a str) -> OAuthClient<'a> {
    OAuthClient::new(
        http,
        TokenEndpoint {
            url,
            client_id,
            encoding: TokenEncoding::Form,
            timeout: Some(Duration::from_secs(60)),
            error_body_limit: ErrorBodyLimit::Bytes(4096),
        },
    )
}

// Deliberately no Debug: token endpoint responses contain credentials.
#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    access_token: String,
    refresh_token: String,
    scope: String,
    expires_in: f64,
}

fn invalid_grant() -> io::Error {
    io::Error::other("Invalid direct ChatGPT token grant; reconnect ChatGPT")
}

fn valid_client_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 2048
        && id.trim() == id
        && !id.chars().any(char::is_control)
        && id != "dynamic_agent_client"
}

pub(crate) fn validate_tokens(tokens: &TokenData) -> io::Result<()> {
    let direct = tokens.direct.as_ref().ok_or_else(invalid_grant)?;
    if !valid_client_id(&direct.client_id)
        || !direct.scopes.iter().any(|scope| scope == DIRECT_SCOPE)
        || tokens.access_token.trim().is_empty()
        || tokens.refresh_token.trim().is_empty()
        || tokens.id_token.raw_jwt.trim().is_empty()
    {
        return Err(invalid_grant());
    }
    Ok(())
}

impl TokenResponse {
    fn into_tokens(self, client_id: &str, previous: Option<&TokenData>) -> io::Result<TokenData> {
        if !self.expires_in.is_finite() || self.expires_in <= 0.0 {
            return Err(invalid_grant());
        }
        let millis = self.expires_in * 1000.0;
        if !millis.is_finite() || millis < 1.0 || millis >= i64::MAX as f64 {
            return Err(invalid_grant());
        }
        let expires_at = Utc::now()
            .checked_add_signed(chrono::Duration::milliseconds(millis as i64))
            .ok_or_else(invalid_grant)?;
        let id_token = match self.id_token {
            Some(jwt) => parse_chatgpt_jwt_claims(&jwt).map_err(|_| invalid_grant())?,
            None => previous
                .map(|tokens| tokens.id_token.clone())
                .ok_or_else(invalid_grant)?,
        };
        let tokens = TokenData {
            account_id: id_token.chatgpt_account_id.clone(),
            id_token,
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            direct: Some(DirectTokenMetadata {
                client_id: client_id.to_string(),
                expires_at,
                scopes: self.scope.split_whitespace().map(str::to_string).collect(),
            }),
        };
        validate_tokens(&tokens)?;
        Ok(tokens)
    }
}

pub(crate) async fn exchange(
    http: &HttpClient,
    code: &str,
    client_id: &str,
    pkce: &PkceCodes,
) -> io::Result<TokenData> {
    exchange_at(http, TOKEN_URL, code, client_id, pkce).await
}

async fn exchange_at(
    http: &HttpClient,
    url: &str,
    code: &str,
    client_id: &str,
    pkce: &PkceCodes,
) -> io::Result<TokenData> {
    if !valid_client_id(client_id) {
        return Err(invalid_grant());
    }
    let response: TokenResponse = endpoint(http, url, client_id)
        .exchange_code(AuthorizationCodeGrant {
            code,
            redirect_uri: REDIRECT_URI,
            pkce,
            resource: Some(RESOURCE),
        })
        .await
        .map_err(|_| io::Error::other("Direct ChatGPT token exchange failed"))?;
    response.into_tokens(client_id, /*previous*/ None)
}

pub(crate) async fn refresh(
    http: &HttpClient,
    tokens: &TokenData,
) -> Result<TokenData, crate::RefreshTokenError> {
    refresh_at(http, TOKEN_URL, tokens).await
}

async fn refresh_at(
    http: &HttpClient,
    url: &str,
    tokens: &TokenData,
) -> Result<TokenData, crate::RefreshTokenError> {
    validate_tokens(tokens)?;
    let direct = tokens.direct.as_ref().ok_or_else(invalid_grant)?;
    let response: TokenResponse = endpoint(http, url, &direct.client_id)
        .refresh(RefreshTokenGrant {
            refresh_token: &tokens.refresh_token,
            resource: Some(RESOURCE),
        })
        .await
        .map_err(crate::direct_refresh_error::classify)?;
    response
        .into_tokens(&direct.client_id, Some(tokens))
        .map_err(Into::into)
}

/// Parse only origin-form callback requests, rejecting ambiguous duplicate parameters.
/// Invalid requests do not consume the pending attempt. Never echo their values.
pub(crate) fn callback(raw: &str, expected_state: &str) -> io::Result<(String, String)> {
    if raw.len() > 16384 || !raw.starts_with("/auth/callback?") || raw.contains('#') {
        return Err(io::Error::other("Invalid direct ChatGPT callback"));
    }
    let url = url::Url::parse(&format!("http://127.0.0.1:1455{raw}"))
        .map_err(|_| io::Error::other("Invalid direct ChatGPT callback"))?;
    let mut fields = std::collections::HashMap::new();
    for (key, value) in url.query_pairs() {
        if fields
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(io::Error::other("Duplicate OAuth callback parameter"));
        }
    }
    if fields.get("state").map(String::as_str) != Some(expected_state) {
        return Err(io::Error::other("OAuth state mismatch"));
    }
    if fields.contains_key("error") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "ChatGPT authorization was not granted",
        ));
    }
    let code = fields
        .remove("code")
        .filter(|code| !code.trim().is_empty())
        .ok_or_else(|| io::Error::other("Missing authorization code"))?;
    let client_id = fields
        .remove("client_id")
        .filter(|id| valid_client_id(id))
        .ok_or_else(|| io::Error::other("Missing or invalid issued client ID"))?;
    Ok((code, client_id))
}

#[cfg(test)]
#[path = "direct_oauth_exchange_tests.rs"]
pub(crate) mod test_exchange;

#[cfg(test)]
#[path = "direct_oauth_tests.rs"]
mod tests;
