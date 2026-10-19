use super::McpServerActivation;
use super::McpServerConfig;
use antex_utils_absolute_path::AbsolutePathBuf;
use antex_utils_absolute_path::AbsolutePathBufGuard;
use pretty_assertions::assert_eq;

#[test]
fn directory_activation_expands_home_and_roundtrips() {
    let home = tempfile::tempdir().unwrap();
    let _guard = AbsolutePathBufGuard::new(home.path());
    let config: McpServerConfig = AbsolutePathBufGuard::with_home_directory(home.path(), || {
        toml::from_str(
            r#"
command = "mcp-server"
activation = { type = "directories", roots = ["~/code", "~/projects"] }
"#,
        )
        .unwrap()
    });
    assert_eq!(
        config.activation,
        Some(McpServerActivation::Directories {
            roots: vec![
                AbsolutePathBuf::try_from(home.path().join("code")).unwrap(),
                AbsolutePathBuf::try_from(home.path().join("projects")).unwrap(),
            ],
        })
    );
    let serialized = toml::to_string(&config).unwrap();
    assert_eq!(
        toml::from_str::<McpServerConfig>(&serialized).unwrap(),
        config
    );
}

#[test]
fn arcadia_activation_roundtrips_for_http_transport() {
    let config: McpServerConfig = toml::from_str(
        r#"
url = "https://example.com/mcp"
activation = { type = "arcadia" }
"#,
    )
    .unwrap();
    assert_eq!(config.activation, Some(McpServerActivation::Arcadia {}));
    let serialized = toml::to_string(&config).unwrap();
    assert_eq!(
        toml::from_str::<McpServerConfig>(&serialized).unwrap(),
        config
    );
}

#[test]
fn absent_activation_preserves_existing_config() {
    let config: McpServerConfig = toml::from_str("command = 'mcp-server'").unwrap();
    assert_eq!(config.activation, None);
    let serialized = toml::to_string(&config).unwrap();
    assert!(!serialized.contains("activation"));
    assert_eq!(
        toml::from_str::<McpServerConfig>(&serialized).unwrap(),
        config
    );
}

#[test]
fn invalid_activation_rejects_empty_roots_unknown_types_and_unknown_fields() {
    let base = tempfile::tempdir().unwrap();
    let _guard = AbsolutePathBufGuard::new(base.path());
    for activation in [
        "{ type = 'directories', roots = [] }",
        "{ type = 'directories' }",
        "{ type = 'directory', roots = ['/tmp'] }",
        "{ type = 'directories', roots = ['/tmp'], typo = true }",
        "{ type = 'arcadia', roots = ['/tmp'] }",
    ] {
        let input = format!("command = 'mcp-server'\nactivation = {activation}");
        assert!(
            toml::from_str::<McpServerConfig>(&input).is_err(),
            "{input}"
        );
    }
}
