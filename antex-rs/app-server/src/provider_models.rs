use std::sync::Arc;
use std::time::Duration;

use antex_api::ModelsClient;
use antex_api::ReqwestTransport;
use antex_app_server_protocol::JSONRPCErrorError;
use antex_app_server_protocol::Model;
use antex_core::config::Config;
use antex_http_client::ClientRouteClass;
use antex_login::AuthManager;
use antex_login::default_client::ClientRedirectPolicy;
use antex_login::default_client::create_client_for_route_async;
use antex_models_manager::manager::RefreshStrategy;
use antex_protocol::openai_models::ModelPreset;
use antex_protocol::openai_models::ModelVisibility;
use serde::Deserialize;

#[derive(Deserialize)]
struct ApiModels {
    data: Vec<ApiModel>,
}

#[derive(Deserialize)]
struct ApiModel {
    id: String,
}

pub(crate) async fn list_models(
    config: &Config,
    auth: Arc<AuthManager>,
) -> Result<Vec<Model>, JSONRPCErrorError> {
    let failed = || JSONRPCErrorError {
        code: -32603,
        message:
            "Unable to load this provider's model catalog. Check its endpoint and credentials."
                .into(),
        data: None,
    };
    let discovery_enabled = config
        .features
        .enabled(antex_features::Feature::ApiKeyModelDiscovery);
    if !discovery_enabled
        && (config.model_provider.env_key.is_some()
            || config.model_provider.experimental_bearer_token.is_some())
    {
        return Err(JSONRPCErrorError {
            code: -32600,
            message: "API-key model discovery is disabled by configuration.".into(),
            data: None,
        });
    }
    let provider =
        antex_model_provider::create_model_provider(config.model_provider.clone(), Some(auth));
    if config.model_provider_id == "openai"
        && provider
            .auth_for_request()
            .await
            .map_err(|_| failed())?
            .is_none()
    {
        return Err(JSONRPCErrorError {
            code: -32600,
            message: "Sign in to OpenAI before loading its models.".into(),
            data: None,
        });
    }
    if config.model_provider_id == "openai"
        || config.model_provider.model_catalog_url.is_some()
        || config.model_catalog.is_some()
        || config.model_provider.is_amazon_bedrock()
    {
        let manager = provider.models_manager_without_cache(config.model_catalog.clone());
        manager.set_api_key_model_discovery_enabled(discovery_enabled);
        let models = manager
            .list_models(RefreshStrategy::Online, config.http_client_factory())
            .await;
        return Ok(crate::models::supported_models(
            models, /*include_hidden*/ true,
        ));
    }

    tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
        let route = provider.api_provider().await.map_err(|_| failed())?;
        let auth = provider.api_auth().await.map_err(|_| failed())?;
        let url = ModelsClient::<ReqwestTransport>::request_url(&route, env!("CARGO_PKG_VERSION"));
        let client = create_client_for_route_async(
            config.http_client_factory(),
            url.clone(),
            ClientRouteClass::Api,
            ClientRedirectPolicy::Reject,
        )
        .await
        .map_err(|_| failed())?
        .without_request_logging();
        let (body, _) = ModelsClient::new(ReqwestTransport::from_http_client(client), route, auth)
            .list_models_raw(url, Default::default(), Some(1024 * 1024))
            .await
            .map_err(|_| failed())?;
        let response: ApiModels = serde_json::from_slice(&body).map_err(|_| failed())?;
        let mut ids = std::collections::BTreeSet::new();
        for model in response.data.into_iter().take(/*n*/ 256) {
            if !model.id.is_empty()
                && model.id.len() <= 256
                && !model.id.chars().any(char::is_control)
            {
                ids.insert(model.id);
            }
        }
        let models = ids
            .into_iter()
            .map(|id| {
                let mut info = antex_models_manager::model_info::model_info_from_slug(&id);
                info.visibility = ModelVisibility::List;
                ModelPreset::from(info)
            })
            .collect();
        Ok(crate::models::supported_models(
            models, /*include_hidden*/ true,
        ))
    })
    .await
    .map_err(|_| failed())?
}
