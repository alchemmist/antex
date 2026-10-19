use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use antex_protocol::openai_models::ModelPreset;
use antex_protocol::protocol::MultiAgentVersion;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use crate::legacy_core::config::Config;
use crate::model_catalog::ProviderCatalog;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_AGE_SECONDS: u64 = 24 * 60 * 60;
const CACHE_FILE: &str = "provider-models-cache.json";

#[derive(Serialize, Deserialize)]
struct Entry {
    fingerprint: String,
    updated_at: u64,
    models: Vec<CachedModel>,
}

#[derive(Serialize, Deserialize)]
struct CachedModel {
    preset: ModelPreset,
    multi_agent_version: Option<MultiAgentVersion>,
}

pub(crate) struct ProviderModelCache {
    path: std::path::PathBuf,
    fingerprints: BTreeMap<String, String>,
}

impl ProviderModelCache {
    pub(crate) fn new(config: &Config) -> Self {
        let fingerprints = config
            .model_providers
            .iter()
            .filter_map(|(id, provider)| {
                if !config
                    .features
                    .enabled(antex_features::Feature::ApiKeyModelDiscovery)
                    || id == "openai"
                    || provider.requires_openai_auth
                    || provider.auth.is_some()
                    || provider.gateway_oauth.is_some()
                    || provider.aws.is_some()
                {
                    return None;
                }
                let mut value = serde_json::to_value(provider).ok()?;
                sort_objects(&mut value);
                let mut environment = BTreeMap::new();
                for name in provider.env_key.iter().chain(
                    provider
                        .env_http_headers
                        .iter()
                        .flat_map(|headers| headers.values()),
                ) {
                    environment.insert(name, std::env::var(name).ok());
                }
                let bytes = serde_json::to_vec(&(value, environment)).ok()?;
                Some((id.clone(), format!("{:x}", Sha256::digest(bytes))))
            })
            .collect();
        Self {
            path: config.codex_home.join(CACHE_FILE).to_path_buf(),
            fingerprints,
        }
    }

    fn read(&self) -> BTreeMap<String, Entry> {
        let Ok(file) = std::fs::File::open(&self.path) else {
            return BTreeMap::new();
        };
        let mut bytes = Vec::new();
        if file
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > MAX_BYTES
        {
            return BTreeMap::new();
        }
        let mut entries: BTreeMap<String, Entry> =
            serde_json::from_slice(&bytes).unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        entries.retain(|id, entry| {
            self.fingerprints.get(id) == Some(&entry.fingerprint)
                && entry.updated_at <= now
                && now - entry.updated_at <= MAX_AGE_SECONDS
                && entry.models.len() <= 256
        });
        entries
    }

    pub(crate) fn load(&self, config: &Config) -> Vec<ProviderCatalog> {
        self.read()
            .into_iter()
            .take(/*n*/ 16)
            .filter_map(|(id, entry)| {
                let provider = config.model_providers.get(&id)?;
                Some(ProviderCatalog {
                    id,
                    name: provider.name.clone(),
                    models: entry
                        .models
                        .into_iter()
                        .map(|cached| {
                            let mut preset = cached.preset;
                            preset.multi_agent_version = cached.multi_agent_version;
                            preset
                        })
                        .collect(),
                    unavailable: false,
                })
            })
            .collect()
    }

    pub(crate) fn store(&self, catalogs: &[ProviderCatalog]) -> std::io::Result<()> {
        if self.fingerprints.is_empty() {
            return Ok(());
        }
        let mut entries = self.read();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for catalog in catalogs
            .iter()
            .filter(|catalog| !catalog.unavailable && catalog.models.len() <= 256)
            .take(/*n*/ 16)
        {
            if let Some(fingerprint) = self.fingerprints.get(&catalog.id) {
                entries.insert(
                    catalog.id.clone(),
                    Entry {
                        fingerprint: fingerprint.clone(),
                        updated_at: now,
                        models: catalog
                            .models
                            .iter()
                            .map(|preset| CachedModel {
                                preset: preset.clone(),
                                multi_agent_version: preset.multi_agent_version,
                            })
                            .collect(),
                    },
                );
            }
        }
        let entries: BTreeMap<_, _> = entries.into_iter().take(/*n*/ 16).collect();
        let bytes = serde_json::to_vec(&entries)?;
        if bytes.len() > MAX_BYTES {
            return Ok(());
        }
        let Some(parent) = self.path.parent() else {
            return Ok(());
        };
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn sort_objects(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for nested in object.values_mut() {
                sort_objects(nested);
            }
            object.sort_keys();
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(sort_objects),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

#[cfg(test)]
#[path = "provider_model_cache_tests.rs"]
mod tests;
