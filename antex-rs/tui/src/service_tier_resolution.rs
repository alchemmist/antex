use crate::legacy_core::config::Config;
use antex_features::Feature;
use antex_protocol::config_types::SERVICE_TIER_DEFAULT_REQUEST_VALUE;
use antex_protocol::config_types::ServiceTier;
use antex_protocol::openai_models::ModelPreset;

pub(crate) fn reset_fast_mode_for_new_process(config: &mut Config) {
    config.service_tier = service_tier_for_new_process(config.service_tier.take());
}

fn service_tier_for_new_process(service_tier: Option<String>) -> Option<String> {
    if service_tier.is_none()
        || service_tier
            .as_deref()
            .and_then(ServiceTier::from_request_value)
            == Some(ServiceTier::Fast)
    {
        Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE.to_string())
    } else {
        service_tier
    }
}

pub(crate) fn constrain_server_service_tiers(
    models: &mut [ModelPreset],
    requirements: &antex_app_server_protocol::ConfigRequirementsReadResponse,
) {
    let Some(features) = requirements
        .requirements
        .as_ref()
        .and_then(|requirements| requirements.feature_requirements.as_ref())
    else {
        return;
    };
    let fast_enabled = features.get("fast_mode") != Some(&false);
    // Older servers enforce Ultra Fast through their shared Fast mode gate.
    let ultrafast_enabled = features.get("ultrafast_mode") != Some(&false)
        && (requirements.supports_independent_speed_modes == Some(true) || fast_enabled);
    let tier_enabled = |tier: &str| match tier {
        "flex" => true,
        "ultrafast" => ultrafast_enabled,
        _ => fast_enabled,
    };
    for model in models {
        model.service_tiers.retain(|tier| tier_enabled(&tier.id));
        if model
            .default_service_tier
            .as_deref()
            .is_some_and(|tier| !tier_enabled(tier))
        {
            model.default_service_tier = None;
        }
    }
}

pub(crate) fn configured_service_tier(
    config: &Config,
    notices: &antex_config::types::Notice,
) -> Option<String> {
    config.service_tier.clone().or_else(|| {
        (notices.fast_default_opt_out == Some(true))
            .then(|| SERVICE_TIER_DEFAULT_REQUEST_VALUE.to_string())
    })
}

pub(crate) fn effective_service_tier(
    config: &Config,
    notices: &antex_config::types::Notice,
    model: &str,
    models: &[ModelPreset],
) -> Option<String> {
    let configured = configured_service_tier(config, notices);
    if configured.as_deref() == Some(ServiceTier::Flex.request_value()) {
        return configured;
    }
    let Some(preset) = models.iter().find(|preset| preset.model == model) else {
        return configured.filter(|tier| config.features.service_tier_enabled(tier));
    };

    match configured.as_deref() {
        Some(service_tier) if service_tier == SERVICE_TIER_DEFAULT_REQUEST_VALUE => configured,
        Some(service_tier) if model_supports_service_tier(preset, service_tier) => configured,
        Some(_) => None,
        None => preset
            .default_service_tier
            .clone()
            .filter(|service_tier| model_supports_service_tier(preset, service_tier)),
    }
    .filter(|tier| config.features.service_tier_enabled(tier))
}

pub(crate) fn service_tier_update_for_core(
    config: &Config,
    notices: &antex_config::types::Notice,
    model: &str,
    models: &[ModelPreset],
) -> Option<Option<String>> {
    let effective = effective_service_tier(config, notices, model, models);
    if let Some(service_tier) = effective {
        return Some(Some(service_tier));
    }

    if !config.features.enabled(Feature::FastMode)
        && !config.features.enabled(Feature::UltrafastMode)
    {
        return None;
    }

    if !models.iter().any(|preset| preset.model == model) {
        return None;
    }

    Some(Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE.to_string()))
}

pub(crate) fn model_supports_service_tier(model: &ModelPreset, service_tier: &str) -> bool {
    model
        .service_tiers
        .iter()
        .any(|tier| tier.id == service_tier)
}

#[cfg(test)]
#[path = "service_tier_resolution_tests.rs"]
mod tests;
