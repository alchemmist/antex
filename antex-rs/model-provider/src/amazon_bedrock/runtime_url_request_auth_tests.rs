use super::*;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use antex_login::auth::BedrockAccessKeysAuth;
use antex_login::auth::BedrockApiKeyAuth;
use antex_protocol::config_types::ModelProviderAuthInfo;
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

struct Source {
    auth: AntexAuth,
    unavailable: AtomicBool,
    resolutions: AtomicUsize,
}

impl ExternalAuth for Source {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            self.resolutions.fetch_add(/*val*/ 1, Ordering::SeqCst);
            if self.unavailable.load(Ordering::SeqCst) {
                Err(std::io::Error::other(
                    "synthetic selected source unavailable",
                ))
            } else {
                Ok(self.auth.clone())
            }
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }
}

#[tokio::test]
async fn command_credentials_are_not_executed_for_configured_routing() {
    let home = tempfile::tempdir().unwrap();
    for endpoint in [BedrockEndpoint::Mantle, BedrockEndpoint::Runtime] {
        for base_url in [None, Some("https://proxy.example.com/v1")] {
            let aws = ModelProviderAwsAuthInfo {
                profile: Some("profile-that-must-not-be-loaded".to_string()),
                region: Some(" us-west-2 ".to_string()),
                credential_export: None,
                auth_refresh: None,
            };
            let mut info = match endpoint {
                BedrockEndpoint::Mantle => {
                    ModelProviderInfo::create_amazon_bedrock_provider(Some(aws))
                }
                BedrockEndpoint::Runtime => {
                    ModelProviderInfo::create_amazon_bedrock_runtime_provider(Some(aws))
                }
            };
            info.base_url = base_url.map(str::to_string);
            info.auth = Some(ModelProviderAuthInfo {
                command: home
                    .path()
                    .join("missing-token-fetcher")
                    .to_string_lossy()
                    .into_owned(),
                args: Vec::new(),
                timeout_ms: NonZeroU64::new(1_000).unwrap(),
                refresh_interval_ms: 300_000,
                cwd: home.path().to_path_buf().try_into().unwrap(),
            });
            let provider = AmazonBedrockModelProvider::new(info, /*auth_manager*/ None);
            let expected = base_url
                .map(str::to_string)
                .unwrap_or_else(|| match endpoint {
                    BedrockEndpoint::Mantle => {
                        "https://bedrock-mantle.us-west-2.api.aws/openai/v1".to_string()
                    }
                    BedrockEndpoint::Runtime => {
                        "https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1".to_string()
                    }
                });
            assert_eq!(
                provider.runtime_base_url().await.unwrap(),
                Some(expected.clone())
            );
            assert_eq!(provider.api_provider().await.unwrap().base_url, expected);
            assert_eq!(provider.auth_manager().unwrap().auth_cached(), None);
            assert!(ModelProvider::auth_for_request(&provider).await.is_err());
            assert!(provider.api_auth().await.is_err());

            // Count actual acquisitions independently of the missing executable.
            let source = Arc::new(Source {
                auth: AntexAuth::from_api_key("synthetic-command-bearer"),
                unavailable: AtomicBool::new(false),
                resolutions: AtomicUsize::new(/*v*/ 0),
            });
            provider
                .auth_manager()
                .unwrap()
                .set_external_auth(source.clone())
                .await
                .unwrap();
            let installed = source.resolutions.load(Ordering::SeqCst);
            source.unavailable.store(true, Ordering::SeqCst);
            assert_eq!(provider.runtime_base_url().await.unwrap(), Some(expected));
            assert_eq!(source.resolutions.load(Ordering::SeqCst), installed);
            assert!(provider.api_auth().await.is_err());
            assert!(source.resolutions.load(Ordering::SeqCst) > installed);
        }
    }
}

#[tokio::test]
async fn managed_bearer_region_still_wins_and_failed_source_cannot_fall_back() {
    for endpoint in [BedrockEndpoint::Mantle, BedrockEndpoint::Runtime] {
        let auth = AntexAuth::BedrockApiKey(BedrockApiKeyAuth {
            api_key: "synthetic-managed-bearer".to_string(),
            region: "eu-west-1".to_string(),
        });
        let manager = AuthManager::from_auth_for_testing(auth.clone());
        let source = Arc::new(Source {
            auth: auth.clone(),
            unavailable: AtomicBool::new(false),
            resolutions: AtomicUsize::new(/*v*/ 0),
        });
        manager.set_external_auth(source.clone()).await.unwrap();
        let aws = ModelProviderAwsAuthInfo {
            profile: None,
            region: Some("us-west-2".to_string()),
            credential_export: None,
            auth_refresh: None,
        };
        let info = match endpoint {
            BedrockEndpoint::Mantle => ModelProviderInfo::create_amazon_bedrock_provider(Some(aws)),
            BedrockEndpoint::Runtime => {
                ModelProviderInfo::create_amazon_bedrock_runtime_provider(Some(aws))
            }
        };
        let mut provider = AmazonBedrockModelProvider::new(info, Some(manager.clone()));
        let expected = match endpoint {
            BedrockEndpoint::Mantle => "https://bedrock-mantle.eu-west-1.api.aws/openai/v1",
            BedrockEndpoint::Runtime => "https://bedrock-runtime.eu-west-1.amazonaws.com/openai/v1",
        };
        assert_eq!(
            provider.runtime_base_url().await.unwrap(),
            Some(expected.to_string())
        );
        source.unavailable.store(true, Ordering::SeqCst);
        assert!(provider.runtime_base_url().await.is_err());
        assert!(provider.api_provider().await.is_err());
        assert!(provider.api_auth().await.is_err());
        assert_eq!(manager.auth_cached(), Some(auth));

        // An explicit URL is configuration, but does not authorize a request.
        provider.info.base_url = Some("https://proxy.example.com/v1".to_string());
        assert_eq!(
            provider.runtime_base_url().await.unwrap(),
            provider.info.base_url
        );
        assert!(provider.api_auth().await.is_err());
    }
}

#[tokio::test]
async fn access_key_region_from_config_does_not_acquire_failed_credentials() {
    let auth = AntexAuth::BedrockAccessKeys(BedrockAccessKeysAuth {
        access_key_id: "synthetic-access-key".to_string(),
        secret_access_key: "synthetic-secret-key".to_string(),
        session_token: None,
    });
    let manager = AuthManager::from_auth_for_testing(auth.clone());
    let source = Arc::new(Source {
        auth,
        unavailable: AtomicBool::new(false),
        resolutions: AtomicUsize::new(/*v*/ 0),
    });
    manager.set_external_auth(source.clone()).await.unwrap();
    let provider = AmazonBedrockModelProvider::new(
        ModelProviderInfo::create_amazon_bedrock_runtime_provider(Some(ModelProviderAwsAuthInfo {
            profile: None,
            region: Some("us-west-2".to_string()),
            credential_export: None,
            auth_refresh: None,
        })),
        Some(manager),
    );
    let installed = source.resolutions.load(Ordering::SeqCst);
    source.unavailable.store(true, Ordering::SeqCst);
    assert_eq!(
        provider.runtime_base_url().await.unwrap(),
        Some("https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1".to_string())
    );
    assert_eq!(source.resolutions.load(Ordering::SeqCst), installed);
    assert!(provider.api_auth().await.is_err());
}
