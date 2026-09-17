//! Fixtures for integration tests that seed the provider's model cache.

use antex_login::AntexAuth;
use antex_model_provider_info::ModelProviderInfo;
use antex_models_manager::cache::ModelsCacheEntry;
use antex_protocol::openai_models::ModelInfo;

/// Constructs a cache fixture matching the supplied test provider and authentication.
pub fn models_cache_entry(
    provider_info: &ModelProviderInfo,
    auth: Option<&AntexAuth>,
    models: Vec<ModelInfo>,
) -> ModelsCacheEntry {
    ModelsCacheEntry {
        fetched_at: std::time::SystemTime::now().into(),
        etag: None,
        client_version: Some(antex_models_manager::client_version_to_whole()),
        identity: crate::models_identity::identity(provider_info, auth).ok(),
        models,
    }
}
