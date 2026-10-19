use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn cache_survives_restart_without_storing_keys_and_invalidates_changed_identity() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        r#"
[model_providers.local]
name = "Local"
base_url = "http://localhost:1234/v1"
experimental_bearer_token = "test-private-key"
"#,
    )
    .unwrap();
    let mut config = crate::legacy_core::config::ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await
        .unwrap();
    let model = crate::test_support::TEST_MODEL_PRESETS[0].clone();
    let catalog = ProviderCatalog {
        id: "local".into(),
        name: "Local".into(),
        models: vec![model.clone()],
        unavailable: false,
    };
    let cache = ProviderModelCache::new(&config);
    cache.store(std::slice::from_ref(&catalog)).unwrap();
    let bytes = std::fs::read_to_string(&cache.path).unwrap();
    assert!(!bytes.contains("test-private-key"));
    let cached = ProviderModelCache::new(&config).load(&config);
    assert_eq!(
        cached
            .into_iter()
            .map(|c| (c.id, c.models))
            .collect::<Vec<_>>(),
        vec![("local".into(), vec![model])]
    );
    let mut entries = cache.read();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for updated_at in [now - MAX_AGE_SECONDS - 1, now + MAX_AGE_SECONDS] {
        entries.get_mut("local").unwrap().updated_at = updated_at;
        std::fs::write(&cache.path, serde_json::to_vec(&entries).unwrap()).unwrap();
        assert!(cache.load(&config).is_empty());
    }
    cache.store(std::slice::from_ref(&catalog)).unwrap();
    config
        .model_providers
        .get_mut("local")
        .unwrap()
        .experimental_bearer_token = Some("another-key".into());
    assert!(ProviderModelCache::new(&config).load(&config).is_empty());
    config
        .model_providers
        .get_mut("local")
        .unwrap()
        .experimental_bearer_token = Some("test-private-key".into());
    config.model_providers.get_mut("local").unwrap().base_url =
        Some("http://localhost:9876/v1".into());
    assert!(ProviderModelCache::new(&config).load(&config).is_empty());
}

#[tokio::test]
async fn corrupt_or_oversized_cache_is_ignored() {
    let home = tempfile::tempdir().unwrap();
    let config = crate::legacy_core::config::ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await
        .unwrap();
    let cache = ProviderModelCache::new(&config);
    for bytes in [b"invalid json".to_vec(), vec![b' '; MAX_BYTES + 1]] {
        std::fs::write(&cache.path, bytes).unwrap();
        assert!(cache.load(&config).is_empty());
    }
}

#[test]
fn fingerprint_json_is_independent_of_header_insertion_order() {
    let mut a = serde_json::json!({"headers": {"b":"2","a":"1"}});
    let mut b = serde_json::json!({"headers": {"a":"1","b":"2"}});
    sort_objects(&mut a);
    sort_objects(&mut b);
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
}
