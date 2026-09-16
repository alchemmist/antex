use super::credential_keys::home_key;
use super::credential_keys::mcp_keys;
use super::credential_keys::mcp_secret_name;
use antex_keyring_store::CredentialStoreError;
use antex_keyring_store::KeyringStore;
use antex_secrets::LocalSecretsNamespace;
use antex_secrets::SecretName;
use antex_secrets::SecretScope;
use antex_secrets::SecretsBackendKind;
use antex_secrets::SecretsManager;
use anyhow::Context;
use anyhow::ensure;
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use toml_edit::DocumentMut;

struct Credential {
    service: &'static str,
    account: String,
    value: Option<String>,
}

pub(crate) struct KeyringMigration {
    store: Arc<dyn KeyringStore>,
    sources: Vec<Credential>,
    created: Vec<Credential>,
    committed: bool,
}

impl KeyringMigration {
    pub(crate) fn prepare(
        source: &Path,
        staging: &Path,
        destination: &Path,
        store: Arc<dyn KeyringStore>,
    ) -> anyhow::Result<Self> {
        let mut migration = Self {
            store,
            sources: Vec::new(),
            created: Vec::new(),
            committed: false,
        };
        for entry in fs::read_dir(source)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if name != "config.toml" && !name.ends_with(".config.toml") {
                continue;
            }
            let config = fs::read_to_string(path)?
                .parse::<DocumentMut>()
                .map_err(|_| {
                    anyhow::anyhow!("cannot inspect credential settings in source configuration")
                })?;
            let mode = config
                .get("cli_auth_credentials_store")
                .and_then(|value| value.as_str())
                .unwrap_or("file");
            if matches!(mode, "keyring" | "auto") {
                migration.copy(
                    "Codex Auth",
                    &[home_key("cli", source)],
                    "Antex Auth",
                    home_key("cli", destination),
                )?;
            }
            if config
                .get("mcp_oauth_credentials_store")
                .and_then(|value| value.as_str())
                == Some("file")
            {
                continue;
            }
            if let Some(servers) = config.get("mcp_servers").and_then(|value| value.as_table()) {
                for (name, server) in servers {
                    if server.get("bearer_token_env_var").is_some() {
                        continue;
                    }
                    if let Some(url) = server.get("url").and_then(|value| value.as_str()) {
                        migration.copy(
                            "Codex MCP Credentials",
                            &mcp_keys(name, url, source)?,
                            "Antex MCP Credentials",
                            mcp_keys(name, url, destination)?[0].clone(),
                        )?;
                    }
                }
            }
        }
        let secrets = staging.join("secrets");
        if ["local.age", "codex_auth.age", "mcp_oauth.age"]
            .iter()
            .any(|name| secrets.join(name).exists())
        {
            ensure!(
                !secrets.is_symlink(),
                "external encrypted secrets directory must be copied into the source home before migration"
            );
            let passphrase = migration
                .copy(
                    "codex",
                    &[home_key("secrets", source)],
                    "antex",
                    home_key("secrets", destination),
                )?
                .context("source encrypted secrets have no readable keyring key")?;
            let memory: Arc<dyn KeyringStore> =
                Arc::new(MigrationKeyring(Mutex::new(Some(passphrase))));
            for name in ["local.age", "codex_auth.age", "mcp_oauth.age"] {
                ensure!(
                    !secrets.join(name).is_symlink(),
                    "external encrypted secret file requires a separate migration"
                );
            }
            if secrets.join("codex_auth.age").exists() {
                ensure!(
                    !secrets.join("antex_auth.age").exists(),
                    "both legacy and Antex encrypted auth files exist"
                );
                fs::rename(
                    secrets.join("codex_auth.age"),
                    secrets.join("antex_auth.age"),
                )?;
                let manager = SecretsManager::new_with_keyring_store_and_namespace(
                    staging.to_path_buf(),
                    SecretsBackendKind::Local,
                    Arc::clone(&memory),
                    LocalSecretsNamespace::AntexAuth,
                );
                let old = SecretName::new("CODEX_AUTH")?;
                if let Some(value) = manager.get(&SecretScope::Global, &old)? {
                    manager.set(
                        &SecretScope::Global,
                        &SecretName::new("ANTEX_AUTH")?,
                        &value,
                    )?;
                    manager.delete(&SecretScope::Global, &old)?;
                }
            }
            if secrets.join("mcp_oauth.age").exists() {
                let manager = SecretsManager::new_with_keyring_store_and_namespace(
                    staging.to_path_buf(),
                    SecretsBackendKind::Local,
                    memory,
                    LocalSecretsNamespace::McpOAuth,
                );
                for entry in manager.list(/*scope_filter*/ None)? {
                    let value = manager
                        .get(&entry.scope, &entry.name)?
                        .context("encrypted MCP credential disappeared")?;
                    let payload: Value = serde_json::from_str(&value)
                        .map_err(|_| anyhow::anyhow!("invalid encrypted MCP credential"))?;
                    let server = payload
                        .get("server_name")
                        .and_then(Value::as_str)
                        .context("MCP credential has no server name")?;
                    let url = payload
                        .get("url")
                        .and_then(Value::as_str)
                        .context("MCP credential has no URL")?;
                    let key = mcp_secret_name(&mcp_keys(server, url, destination)?[0])?;
                    if key != entry.name {
                        ensure!(
                            manager.get(&entry.scope, &key)?.is_none(),
                            "conflicting encrypted MCP credentials"
                        );
                        manager.set(&entry.scope, &key, &value)?;
                        manager.delete(&entry.scope, &entry.name)?;
                    }
                }
            }
        }
        rewrite_file_credentials(staging, destination)?;
        Ok(migration)
    }

    fn copy(
        &mut self,
        source_service: &'static str,
        accounts: &[String],
        target_service: &'static str,
        target_account: String,
    ) -> anyhow::Result<Option<String>> {
        let mut selected = None;
        for account in accounts {
            let value = self
                .store
                .load(source_service, account)
                .context("read source keyring credential")?;
            if let Some(value) = &value {
                ensure!(
                    selected.as_ref().is_none_or(|previous| previous == value),
                    "conflicting legacy credentials in {source_service}"
                );
                selected = Some(value.clone());
            }
            self.sources.push(Credential {
                service: source_service,
                account: account.clone(),
                value,
            });
        }
        if let Some(value) = &selected {
            let existing = self.store.load(target_service, &target_account)?;
            ensure!(
                existing.as_ref().is_none_or(|existing| existing == value),
                "destination keyring credential already exists in {target_service}"
            );
            if existing.is_none() {
                self.store.save(target_service, &target_account, value)?;
                self.created.push(Credential {
                    service: target_service,
                    account: target_account,
                    value: selected.clone(),
                });
            }
        }
        Ok(selected)
    }

    pub(crate) fn verify(&self) -> anyhow::Result<()> {
        for source in &self.sources {
            ensure!(
                self.store.load(source.service, &source.account)? == source.value,
                "source credentials changed during migration; stop source clients and retry"
            );
        }
        Ok(())
    }

    pub(crate) fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for KeyringMigration {
    fn drop(&mut self) {
        if !self.committed {
            for credential in &self.created {
                if self
                    .store
                    .load(credential.service, &credential.account)
                    .ok()
                    == Some(credential.value.clone())
                {
                    let _ = self.store.delete(credential.service, &credential.account);
                }
            }
        }
    }
}

fn rewrite_file_credentials(staging: &Path, destination: &Path) -> anyhow::Result<()> {
    let path = staging.join(".credentials.json");
    let read_path = super::staged_file_path(&path, staging, destination)?;
    if !read_path.exists() {
        return Ok(());
    }
    let credentials: serde_json::Map<String, Value> =
        serde_json::from_slice(&fs::read(&read_path)?)
            .map_err(|_| anyhow::anyhow!("invalid MCP credentials file"))?;
    let mut rewritten = serde_json::Map::new();
    for value in credentials.values() {
        let server = value
            .get("server_name")
            .and_then(Value::as_str)
            .context("MCP credential has no server name")?;
        let url = value
            .get("server_url")
            .and_then(Value::as_str)
            .context("MCP credential has no server URL")?;
        let key = mcp_keys(server, url, destination)?[0].clone();
        ensure!(
            rewritten.get(&key).is_none_or(|previous| previous == value),
            "conflicting MCP file credentials"
        );
        rewritten.insert(key, value.clone());
    }
    if rewritten != credentials {
        let mut temporary = tempfile::NamedTempFile::new_in(staging)?;
        temporary.write_all(&serde_json::to_vec_pretty(&rewritten)?)?;
        temporary
            .as_file()
            .set_permissions(fs::metadata(read_path)?.permissions())?;
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
    }
    Ok(())
}

struct MigrationKeyring(Mutex<Option<String>>);

impl fmt::Debug for MigrationKeyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MigrationKeyring")
            .finish_non_exhaustive()
    }
}

impl KeyringStore for MigrationKeyring {
    fn load(&self, _service: &str, _account: &str) -> Result<Option<String>, CredentialStoreError> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone())
    }

    fn save(
        &self,
        _service: &str,
        _account: &str,
        value: &str,
    ) -> Result<(), CredentialStoreError> {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value.to_string());
        Ok(())
    }

    fn delete(&self, _service: &str, _account: &str) -> Result<bool, CredentialStoreError> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .is_some())
    }
}
