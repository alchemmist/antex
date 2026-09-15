use super::inspect;
use super::prepare;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn sqlite_paths_move_without_rewriting_ids_or_user_text() -> anyhow::Result<()> {
    use antex_state::SqliteConfig;
    use antex_utils_absolute_path::AbsolutePathBuf;

    let root = tempdir()?;
    let source = root.path().join(".codex");
    let destination = root.path().join(".antex");
    fs::create_dir(&source)?;
    let source = source.canonicalize()?;
    let config = SqliteConfig::from_sqlite_home(AbsolutePathBuf::from_absolute_path(&source)?);
    let pool = config.open_read_write_pool(&config.state_db_path()).await?;
    sqlx::query("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT)")
        .execute(&pool)
        .await?;
    let history_path = source.join("sessions/one.jsonl");
    let unchanged = format!("{}-other/session.jsonl", source.display());
    let title = format!("I mentioned {} in my prompt", source.display());
    for (id, path) in [
        ("one", history_path.to_string_lossy().into_owned()),
        ("two", unchanged.clone()),
    ] {
        sqlx::query("INSERT INTO threads VALUES (?, ?, ?)")
            .bind(id)
            .bind(path)
            .bind(&title)
            .execute(&pool)
            .await?;
    }
    pool.close().await;
    let plan = inspect(&source, &destination)?;
    let destination = plan.destination.clone();
    prepare(plan).await?.publish()?;
    let copied = config
        .open_read_only_pool(&destination.join("state_5.sqlite"), None)
        .await?;
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, rollout_path, title FROM threads ORDER BY id")
            .fetch_all(&copied)
            .await?;
    copied.close().await;
    assert_eq!(
        rows,
        vec![
            (
                "one".to_string(),
                destination
                    .join("sessions/one.jsonl")
                    .to_string_lossy()
                    .into_owned(),
                title.clone()
            ),
            ("two".to_string(), unchanged, title),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn publishes_a_copy_without_changing_session_history() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join(".codex");
    let destination = root.path().join(".antex");
    fs::create_dir_all(source.join("sessions"))?;
    let history = b"{\"id\":\"thread-1\",\"message\":\"codex should stay in my history\"}\n";
    fs::write(source.join("sessions/thread-1.jsonl"), history)?;
    fs::write(source.join("config.toml"), "model = 'custom-model'\n")?;
    let plan = inspect(&source, &destination)?;
    assert_eq!(plan.files, 2);
    let prepared = prepare(plan.clone()).await?;
    assert!(!destination.exists());
    assert_eq!(prepared.publish()?, plan);
    assert_eq!(
        fs::read(destination.join("sessions/thread-1.jsonl"))?,
        history
    );
    assert_eq!(fs::read(source.join("sessions/thread-1.jsonl"))?, history);
    assert!(destination.join("antex-migration.json").is_file());
    Ok(())
}

#[tokio::test]
async fn a_changed_source_cannot_publish_a_stale_copy() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    fs::write(source.join("history.jsonl"), "first turn\n")?;
    let prepared = prepare(inspect(&source, &destination)?).await?;
    let staging = prepared.staging_path().to_path_buf();
    fs::write(source.join("history.jsonl"), "second turn\n")?;
    assert!(prepared.publish().is_err());
    assert!(!destination.exists());
    assert!(!staging.exists());
    assert_eq!(
        fs::read_to_string(source.join("history.jsonl"))?,
        "second turn\n"
    );
    Ok(())
}

#[tokio::test]
async fn existing_destination_is_preserved() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    fs::create_dir(&destination)?;
    fs::write(destination.join("history.jsonl"), "existing session")?;
    assert!(inspect(&source, &destination).is_err());
    assert_eq!(
        fs::read_to_string(destination.join("history.jsonl"))?,
        "existing session"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn external_symlinks_preserve_the_external_target() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let external = root.path().join("external.toml");
    fs::create_dir(&source)?;
    fs::write(&external, "model = 'custom-model'\n")?;
    std::os::unix::fs::symlink(&external, source.join("config.toml"))?;
    let plan = inspect(&source, &destination)?;
    assert_eq!(
        plan.external_symlinks,
        vec![std::path::PathBuf::from("config.toml")]
    );
    prepare(plan).await?.publish()?;
    assert_eq!(fs::read_link(destination.join("config.toml"))?, external);
    assert_eq!(fs::read_to_string(external)?, "model = 'custom-model'\n");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rewriting_a_linked_config_does_not_change_dotfiles() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let external = root.path().join("dotfiles.toml");
    fs::create_dir(&source)?;
    let source = source.canonicalize()?;
    let original = format!(
        "[mcp_servers.agent]\ncommand = 'codex' # retain this note\nargs = ['{}/sessions']\n",
        source.display()
    );
    fs::write(&external, &original)?;
    std::os::unix::fs::symlink(&external, source.join("config.toml"))?;
    let plan = inspect(&source, &destination)?;
    let destination = plan.destination.clone();
    prepare(plan).await?.publish()?;
    let migrated = fs::read_to_string(destination.join("config.toml"))?;
    assert!(migrated.contains("command = \"antex\" # retain this note"));
    assert!(migrated.contains(&destination.join("sessions").to_string_lossy().to_string()));
    assert!(!destination.join("config.toml").is_symlink());
    assert_eq!(fs::read_to_string(&external)?, original);
    assert_eq!(fs::read_link(source.join("config.toml"))?, external);
    Ok(())
}

#[tokio::test]
async fn metadata_paths_move_but_messages_and_historical_arguments_do_not() -> anyhow::Result<()> {
    let root = tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir_all(source.join("sessions"))?;
    fs::create_dir_all(source.join("workflow-runs/run"))?;
    let source = source.canonicalize()?;
    let old_path = source
        .join("sessions/image.png")
        .to_string_lossy()
        .into_owned();
    let record = serde_json::json!({
        "id": "keep-id",
        "text": old_path,
        "arguments": {"path": old_path},
        "content": [{"image_url": url::Url::from_file_path(&old_path).unwrap().to_string()}]
    });
    fs::write(
        source.join("sessions/one.jsonl"),
        serde_json::to_vec(&record)?,
    )?;
    fs::write(
        source.join("workflow-runs/run/state.json"),
        serde_json::to_vec(&serde_json::json!({
            "report_path": source.join("workflow-runs/run/report.md"),
            "summary": old_path
        }))?,
    )?;
    let plan = inspect(&source, &destination)?;
    let destination = plan.destination.clone();
    prepare(plan).await?.publish()?;
    let migrated: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("sessions/one.jsonl"))?)?;
    let mut expected = record.clone();
    expected["content"][0]["image_url"] =
        url::Url::from_file_path(destination.join("sessions/image.png"))
            .unwrap()
            .to_string()
            .into();
    assert_eq!(migrated, expected);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(source.join("sessions/one.jsonl"))?)?,
        record
    );
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("workflow-runs/run/state.json"))?)?;
    assert_eq!(
        state,
        serde_json::json!({"report_path": destination.join("workflow-runs/run/report.md"), "summary": old_path})
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_source_home_and_internal_relative_links_are_relocated() -> anyhow::Result<()> {
    let root = tempdir()?;
    let physical = root.path().join("physical");
    let source = root.path().join("legacy");
    let destination = root.path().join("destination");
    fs::create_dir(&physical)?;
    fs::write(physical.join("history.jsonl"), "history")?;
    std::os::unix::fs::symlink(&physical, &source)?;
    std::os::unix::fs::symlink("history.jsonl", physical.join("current-history"))?;
    let plan = inspect(&source, &destination)?;
    let destination = plan.destination.clone();
    prepare(plan).await?.publish()?;
    assert_eq!(
        fs::read_link(destination.join("current-history"))?,
        destination.join("history.jsonl")
    );
    assert_eq!(
        fs::read_to_string(destination.join("current-history"))?,
        "history"
    );
    assert!(super::completed(&source, &destination)?.is_some());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn read_only_files_and_private_directory_permissions_survive_migration() -> anyhow::Result<()>
{
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir()?;
    let source = root.path().join(".codex");
    let destination = root.path().join(".antex");
    fs::create_dir_all(source.join("sessions"))?;
    fs::write(
        source.join("sessions/one.jsonl"),
        "{\"message\":\"unchanged\"}\n",
    )?;
    fs::set_permissions(source.join("sessions"), fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(
        source.join("sessions/one.jsonl"),
        fs::Permissions::from_mode(0o400),
    )?;
    prepare(inspect(&source, &destination)?).await?.publish()?;
    assert_eq!(
        ["sessions", "sessions/one.jsonl"].map(|path| fs::metadata(destination.join(path))
            .unwrap()
            .permissions()
            .mode()
            & 0o777),
        [0o700, 0o400],
    );
    assert_eq!(
        fs::read(source.join("sessions/one.jsonl"))?,
        fs::read(destination.join("sessions/one.jsonl"))?
    );
    Ok(())
}
