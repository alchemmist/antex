use super::models_cache_entry;
use crate::models_endpoint::OpenAiModelsEndpoint;
use antex_login::AntexAuth;
use antex_login::AuthManager;
use antex_model_provider_info::ModelProviderInfo;
use antex_models_manager::manager::ModelsEndpointClient;
use pretty_assertions::assert_eq;

#[test]
fn seeded_catalog_identity_matches_effective_provider_auth() {
    let ambient = AntexAuth::create_dummy_chatgpt_auth_for_testing();
    let manager = AuthManager::from_auth_for_testing(ambient.clone());
    let mut managed =
        ModelProviderInfo::create_openai_provider(Some("http://127.0.0.1:12345/v1".into()));
    managed.requires_openai_auth = true;
    let mut anonymous = managed.clone();
    anonymous.requires_openai_auth = false;
    let mut explicit = managed.clone();
    explicit.experimental_bearer_token = Some("synthetic-provider-token".into());

    for provider in [managed, anonymous, explicit] {
        let endpoint = OpenAiModelsEndpoint::new(
            provider.clone(),
            Some(manager.clone()),
            /*gateway_auth_manager*/ None,
        );
        let seeded = models_cache_entry(&provider, Some(&ambient), Vec::new());
        assert_eq!(seeded.identity.unwrap(), endpoint.identity().unwrap());
    }
}
