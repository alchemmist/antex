use antex_protocol::config_types::ServiceTier;
use antex_protocol::openai_models::ModelInfo;
use antex_protocol::openai_models::ModelServiceTier;
use antex_protocol::openai_models::ModelVisibility;
use antex_protocol::openai_models::ReasoningEffort;
use antex_protocol::openai_models::ReasoningEffortPreset;
use serde::Deserialize;

#[derive(Deserialize)]
struct ApiModels {
    data: Vec<ApiModel>,
}

#[derive(Deserialize)]
struct ApiModel {
    id: String,
}

pub fn provider_catalog_models(
    body: &[u8],
    provider_name: &str,
) -> Result<Vec<ModelInfo>, serde_json::Error> {
    let response: ApiModels = serde_json::from_slice(body)?;
    let mut ids = std::collections::BTreeSet::new();
    for model in response.data.into_iter().take(/*n*/ 512) {
        if !model.id.is_empty() && model.id.len() <= 256 && !model.id.chars().any(char::is_control)
        {
            ids.insert(model.id);
        }
    }
    let (ids, fast_bases) = if provider_name == "Cursor subscription" {
        without_fast_duplicates(ids)
    } else {
        (ids, std::collections::BTreeSet::new())
    };
    Ok(catalog_presets(ids, &fast_bases))
}

const EFFORT_SUFFIXES: &[(&str, ReasoningEffort)] = &[
    ("extra-high", ReasoningEffort::XHigh),
    ("minimal", ReasoningEffort::Minimal),
    ("medium", ReasoningEffort::Medium),
    ("xhigh", ReasoningEffort::XHigh),
    ("high", ReasoningEffort::High),
    ("none", ReasoningEffort::None),
    ("low", ReasoningEffort::Low),
    ("max", ReasoningEffort::Max),
];

fn without_fast_duplicates(
    ids: std::collections::BTreeSet<String>,
) -> (
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
) {
    let mut fast_bases = std::collections::BTreeSet::new();
    let mut kept = std::collections::BTreeSet::new();
    for id in &ids {
        if let Some(base) = id.strip_suffix("-fast")
            && !base.is_empty()
            && ids.contains(base)
        {
            let grouped = split_effort_suffix(base)
                .map(|(model, _effort)| model)
                .unwrap_or_else(|| base.to_string());
            fast_bases.insert(grouped);
            continue;
        }
        kept.insert(id.clone());
    }
    (kept, fast_bases)
}

fn catalog_presets(
    ids: std::collections::BTreeSet<String>,
    fast_bases: &std::collections::BTreeSet<String>,
) -> Vec<ModelInfo> {
    let mut grouped: std::collections::BTreeMap<String, Vec<ReasoningEffort>> =
        std::collections::BTreeMap::new();
    let mut plain = std::collections::BTreeSet::new();
    for id in ids {
        match split_effort_suffix(&id) {
            Some((base, effort)) => grouped.entry(base).or_default().push(effort),
            None => {
                plain.insert(id);
            }
        }
    }
    let mut bare_variants = Vec::new();
    for id in &plain {
        if grouped.contains_key(id) {
            bare_variants.push(id.clone());
        }
    }
    for base in bare_variants {
        plain.remove(&base);
        if let Some(efforts) = grouped.get_mut(&base) {
            efforts.push(ReasoningEffort::None);
        }
    }
    let mut presets: Vec<_> = plain
        .into_iter()
        .map(|id| preset_for(&id, Vec::new(), fast_bases.contains(&id)))
        .chain(grouped.into_iter().map(|(base, efforts)| {
            preset_for(&base, dedup_efforts(efforts), fast_bases.contains(&base))
        }))
        .collect();
    presets.sort_by(|left, right| left.slug.cmp(&right.slug));
    presets
}

fn preset_for(slug: &str, efforts: Vec<ReasoningEffort>, fast: bool) -> ModelInfo {
    let mut info = crate::model_info::model_info_from_slug(slug);
    info.visibility = ModelVisibility::List;
    if fast {
        info.service_tiers.push(ModelServiceTier {
            id: ServiceTier::Fast.request_value().to_string(),
            name: "Fast".to_string(),
            description: "Faster variant of this model".to_string(),
        });
    }
    info.default_reasoning_level = preferred_effort(&efforts);
    info.supported_reasoning_levels = efforts
        .into_iter()
        .map(|effort| ReasoningEffortPreset {
            description: effort.as_str().to_string(),
            effort,
        })
        .collect();
    info
}

fn preferred_effort(efforts: &[ReasoningEffort]) -> Option<ReasoningEffort> {
    [
        ReasoningEffort::Medium,
        ReasoningEffort::Low,
        ReasoningEffort::High,
        ReasoningEffort::None,
    ]
    .into_iter()
    .find(|effort| efforts.contains(effort))
    .or_else(|| efforts.first().cloned())
}

fn dedup_efforts(efforts: Vec<ReasoningEffort>) -> Vec<ReasoningEffort> {
    let mut ordered = Vec::new();
    for effort in [
        ReasoningEffort::None,
        ReasoningEffort::Minimal,
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::XHigh,
        ReasoningEffort::Max,
    ] {
        if efforts.contains(&effort) {
            ordered.push(effort);
        }
    }
    ordered
}

fn split_effort_suffix(id: &str) -> Option<(String, ReasoningEffort)> {
    let (without_fast, fast) = id
        .strip_suffix("-fast")
        .map(|base| (base, true))
        .unwrap_or((id, false));
    let (base, effort) = EFFORT_SUFFIXES.iter().find_map(|(suffix, effort)| {
        without_fast
            .strip_suffix(&format!("-{suffix}"))
            .filter(|base| !base.is_empty())
            .map(|base| (base.to_string(), effort.clone()))
    })?;
    Some((if fast { format!("{base}-fast") } else { base }, effort))
}
