use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

fn server(activation: serde_json::Value, enabled: bool) -> EffectiveMcpServer {
    EffectiveMcpServer::from_host_config(
        serde_json::from_value(json!({
            "command": "must-not-start",
            "enabled": enabled,
            "activation": activation,
        }))
        .unwrap(),
    )
}

#[tokio::test]
async fn directory_scope_matches_descendants_not_sibling_prefixes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("code");
    let nested = root.join("homelab/ansible");
    let outside = temp.path().join("code-other");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let original = HashMap::from([
        (
            "scoped".to_string(),
            server(
                json!({"type": "directories", "roots": [root]}),
                /*enabled*/ true,
            ),
        ),
        (
            "manual".to_string(),
            server(
                json!({"type": "directories", "roots": [root]}),
                /*enabled*/ false,
            ),
        ),
        (
            "unscoped".to_string(),
            server(serde_json::Value::Null, /*enabled*/ true),
        ),
    ]);
    for (cwd, expected) in [(root, true), (nested, true), (outside, false)] {
        let mut servers = original.clone();
        apply_activation(&mut servers, &cwd).await;
        assert_eq!(
            (
                servers["scoped"].enabled(),
                servers["manual"].enabled(),
                servers["unscoped"].enabled()
            ),
            (expected, false, true),
        );
    }
}

#[tokio::test]
async fn missing_paths_fail_closed_and_any_existing_root_can_match() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    let scoped = server(
        json!({"type": "directories", "roots": [missing, temp.path()]}),
        /*enabled*/ true,
    );
    let mut servers = HashMap::from([("scoped".to_string(), scoped)]);
    apply_activation(&mut servers, temp.path()).await;
    assert!(servers["scoped"].enabled());
    apply_activation(&mut servers, &missing).await;
    assert!(!servers["scoped"].enabled());
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_paths_allow_links_into_scope_and_reject_links_out() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("code");
    let outside = temp.path().join("elsewhere");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let alias = temp.path().join("alias");
    let escape = root.join("escape");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    std::os::unix::fs::symlink(&outside, &escape).unwrap();
    for (cwd, expected) in [(alias, true), (escape, false)] {
        let mut servers = HashMap::from([(
            "scoped".to_string(),
            server(
                json!({"type": "directories", "roots": [root]}),
                /*enabled*/ true,
            ),
        )]);
        apply_activation(&mut servers, &cwd).await;
        assert_eq!(servers["scoped"].enabled(), expected);
    }
}

#[tokio::test]
async fn arc_root_must_be_absolute_existing_and_contain_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let worktree = root.join("arbitrary-task-directory");
    std::fs::create_dir(&worktree).unwrap();
    let output = format!("{}\n", root.display());
    assert_eq!(
        verified_arcadia_root(&worktree, output.as_bytes()).await,
        Some(root.clone())
    );
    for invalid in [
        "".to_string(),
        "relative/path".to_string(),
        worktree.display().to_string(),
        root.join("missing").display().to_string(),
    ] {
        assert_eq!(verified_arcadia_root(&root, invalid.as_bytes()).await, None);
    }
}
