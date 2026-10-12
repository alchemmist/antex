//! Local-authority access for cross-module hermetic lifecycle tests only.
//! Production callers can only use the fixed-endpoint exchange/refresh functions.
use crate::TokenData;
use antex_http_client::HttpClient;

pub(crate) async fn refresh(
    http: &HttpClient,
    url: &str,
    tokens: &TokenData,
) -> Result<TokenData, crate::RefreshTokenError> {
    assert_eq!(url::Url::parse(url).unwrap().host_str(), Some("127.0.0.1"));
    super::refresh_at(http, url, tokens).await
}

pub(crate) async fn exchange(
    http: &HttpClient,
    url: &str,
    code: &str,
    client_id: &str,
    pkce: &crate::oauth::PkceCodes,
) -> std::io::Result<TokenData> {
    assert_eq!(url::Url::parse(url).unwrap().host_str(), Some("127.0.0.1"));
    super::exchange_at(http, url, code, client_id, pkce).await
}
