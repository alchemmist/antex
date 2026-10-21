use serde::Deserialize;

pub const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
pub struct ElizaCatalog {
    pub data: Vec<ElizaModel>,
}

#[derive(Deserialize)]
pub struct ElizaModel {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub disabled: bool,
    pub tags: Option<Vec<String>>,
    pub architecture: Option<ElizaArchitecture>,
}

#[derive(Deserialize)]
pub struct ElizaArchitecture {
    pub context_length: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

impl ElizaModel {
    pub fn is_interactive(&self) -> bool {
        self.vendor == "internal"
            && self.namespace == "communal"
            && !self.disabled
            && !self.id.is_empty()
            && self.id.len() <= 256
            && !self.id.chars().any(char::is_control)
            && !self.id.contains("batch")
            && !self
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag == "communal_batch"))
            && self.endpoint.starts_with("/internal/")
            && self.endpoint.ends_with("/v1/chat/completions")
            && !self.endpoint.contains("..")
            && self.endpoint.len() <= 512
            && self
                .endpoint
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/_-.".contains(&c))
    }
}
