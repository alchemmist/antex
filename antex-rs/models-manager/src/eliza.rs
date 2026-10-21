use antex_protocol::eliza::ElizaCatalog;
use antex_protocol::eliza::ElizaModel;
use antex_protocol::openai_models::InputModality;
use antex_protocol::openai_models::ModelInfo;
use antex_protocol::openai_models::ModelVisibility;

pub fn eliza_catalog_models(body: &[u8]) -> Result<Vec<ModelInfo>, serde_json::Error> {
    let catalog: ElizaCatalog = serde_json::from_slice(body)?;
    Ok(catalog
        .data
        .into_iter()
        .filter(ElizaModel::is_interactive)
        .take(/*n*/ 512)
        .map(|entry| {
            let mut model = crate::model_info::model_info_from_slug(&entry.id);
            model.display_name = if entry.title.is_empty() {
                entry.id
            } else {
                entry
                    .title
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(/*n*/ 256)
                    .collect()
            };
            model.visibility = ModelVisibility::List;
            let context = entry
                .architecture
                .and_then(|architecture| architecture.context_length)
                .filter(|size| (1..=10_000_000).contains(size))
                .unwrap_or(32_768);
            model.context_window = Some(context);
            model.max_context_window = Some(context);
            model.input_modalities = vec![InputModality::Text];
            model.supports_reasoning_summary_parameter = false;
            model.used_fallback_model_metadata = false;
            model
        })
        .collect())
}
