use super::credential_keys::home_key;
use super::credential_keys::mcp_keys;
use super::inspect;
use super::prepare_with_keyring;
use antex_keyring_store::CredentialStoreError;
use antex_keyring_store::KeyringStore;
use antex_secrets::LocalSecretsNamespace;
use antex_secrets::SecretName;
use antex_secrets::SecretScope;
use antex_secrets::SecretsBackendKind;
use antex_secrets::SecretsManager;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::tempdir;

#[derive(Default)]
struct TestKeyring(Mutex<BTreeMap<(String, String), String>>);

impl fmt::Debug for TestKeyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestKeyring")
            .finish_non_exhaustive()
    }
}

impl KeyringStore for TestKeyring {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, CredentialStoreError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }

    fn save(&self, service: &str, account: &str, value: &str) -> Result<(), CredentialStoreError> {
        self.0.lock().unwrap().insert(
            (service.to_string(), account.to_string()),
            value.to_string(),
        );
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool, CredentialStoreError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .remove(&(service.to_string(), account.to_string()))
            .is_some())
    }
}

#[tokio::test]
async fn copies_cli_and_mcp_credentials_without_removing_legacy_entries() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    fs::write(
        source.join("config.toml"),
        "cli_auth_credentials_store = 'keyring'\n[mcp_servers.sheets]\nurl = 'https://example.test/mcp'\n",
    )?;
    let plan = inspect(&source, &destination)?;
    let source_key = home_key("cli", &plan.source);
    let destination_key = home_key("cli", &plan.destination);
    let mcp_key = mcp_keys("sheets", "https://example.test/mcp", &plan.source)?[0].clone();
    let store = Arc::new(TestKeyring::default());
    store.save("Codex Auth", &source_key, "fake-cli-credential")?;
    store.save("Codex MCP Credentials", &mcp_key, "fake-mcp-credential")?;
    prepare_with_keyring(plan, store.clone()).await?.publish()?;
    assert_eq!(
        store.load("Antex Auth", &destination_key)?,
        Some("fake-cli-credential".to_string())
    );
    assert_eq!(
        store.load("Antex MCP Credentials", &mcp_key)?,
        Some("fake-mcp-credential".to_string())
    );
    assert_eq!(
        store.load("Codex Auth", &source_key)?,
        Some("fake-cli-credential".to_string())
    );
    assert_eq!(
        store.load("Codex MCP Credentials", &mcp_key)?,
        Some("fake-mcp-credential".to_string())
    );
    Ok(())
}

#[tokio::test]
async fn changed_credentials_abort_publication_and_remove_only_new_entries() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    fs::write(
        source.join("config.toml"),
        "cli_auth_credentials_store = 'keyring'\n",
    )?;
    let plan = inspect(&source, &destination)?;
    let source_key = home_key("cli", &plan.source);
    let destination_key = home_key("cli", &plan.destination);
    let store = Arc::new(TestKeyring::default());
    store.save("Codex Auth", &source_key, "original")?;
    let prepared = prepare_with_keyring(plan, store.clone()).await?;
    store.save("Codex Auth", &source_key, "refreshed")?;
    assert!(prepared.publish().is_err());
    assert!(!destination.exists());
    assert_eq!(store.load("Antex Auth", &destination_key)?, None);
    assert_eq!(
        store.load("Codex Auth", &source_key)?,
        Some("refreshed".to_string())
    );
    Ok(())
}

#[tokio::test]
async fn encrypted_auth_keeps_its_payload_under_the_new_namespace() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    let plan = inspect(&source, &destination)?;
    let store = Arc::new(TestKeyring::default());
    let old_manager = SecretsManager::new_with_keyring_store_and_namespace(
        plan.source.clone(),
        SecretsBackendKind::Local,
        store.clone(),
        LocalSecretsNamespace::AntexAuth,
    );
    old_manager.set(
        &SecretScope::Global,
        &SecretName::new("CODEX_AUTH")?,
        "fake-encrypted-auth",
    )?;
    let source_key = home_key("secrets", &plan.source);
    let passphrase = store.load("antex", &source_key)?.unwrap();
    store.save("codex", &source_key, &passphrase)?;
    store.delete("antex", &source_key)?;
    fs::rename(
        source.join("secrets/antex_auth.age"),
        source.join("secrets/codex_auth.age"),
    )?;
    let old_bytes = fs::read(source.join("secrets/codex_auth.age"))?;
    let plan = inspect(&source, &destination)?;
    let destination = plan.destination.clone();
    prepare_with_keyring(plan, store.clone()).await?.publish()?;
    let manager = SecretsManager::new_with_keyring_store_and_namespace(
        destination.clone(),
        SecretsBackendKind::Local,
        store,
        LocalSecretsNamespace::AntexAuth,
    );
    assert_eq!(
        manager.get(&SecretScope::Global, &SecretName::new("ANTEX_AUTH")?)?,
        Some("fake-encrypted-auth".to_string())
    );
    assert_eq!(
        manager.get(&SecretScope::Global, &SecretName::new("CODEX_AUTH")?)?,
        None
    );
    assert!(destination.join("secrets/antex_auth.age").is_file());
    assert_eq!(fs::read(source.join("secrets/codex_auth.age"))?, old_bytes);
    Ok(())
}
