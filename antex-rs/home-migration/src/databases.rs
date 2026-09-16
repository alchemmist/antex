use antex_state::SqliteConfig;
use antex_utils_absolute_path::AbsolutePathBuf;
use anyhow::ensure;
use sqlx::Row;
use std::path::Path;

pub(crate) async fn rewrite_paths(
    staging: &Path,
    source: &Path,
    destination: &Path,
) -> anyhow::Result<()> {
    let config = SqliteConfig::from_sqlite_home(AbsolutePathBuf::from_absolute_path(staging)?);
    let source = source
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("source path must be UTF-8 for SQLite migration"))?;
    let destination = destination
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("destination path must be UTF-8 for SQLite migration"))?;
    let prefix = format!("{source}{}", std::path::MAIN_SEPARATOR);
    for database in config.runtime_db_paths() {
        ensure!(
            !database.path.is_symlink(),
            "SQLite symlink requires a separate migration: {}",
            database.path.display()
        );
        if !database.path.is_file() {
            continue;
        }
        let pool = config.open_read_write_pool(&database.path).await?;
        let result = async {
            let checks = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
                .fetch_all(&pool)
                .await?;
            ensure!(
                checks == ["ok"],
                "SQLite integrity check failed: {}",
                database.path.display()
            );
            let mut transaction = pool.begin().await?;
            for (table, column) in [
                ("threads", "rollout_path"),
                ("threads", "cwd"),
                ("threads", "agent_path"),
                ("rollout_migration_state", "rollout_path"),
                ("agent_jobs", "input_csv_path"),
                ("agent_jobs", "output_csv_path"),
                ("projects", "path"),
            ] {
                let columns = sqlx::query("SELECT name FROM pragma_table_info(?)")
                    .bind(table)
                    .fetch_all(&mut *transaction)
                    .await?;
                if !columns
                    .iter()
                    .any(|row| row.get::<String, _>("name") == column)
                {
                    continue;
                }
                let mut query = sqlx::QueryBuilder::new("UPDATE \"");
                query
                    .push(table)
                    .push("\" SET \"")
                    .push(column)
                    .push("\" = ")
                    .push_bind(destination)
                    .push(" || substr(\"")
                    .push(column)
                    .push("\", length(")
                    .push_bind(source)
                    .push(") + 1) WHERE \"")
                    .push(column)
                    .push("\" = ")
                    .push_bind(source)
                    .push(" OR substr(\"")
                    .push(column)
                    .push("\", 1, length(")
                    .push_bind(&prefix)
                    .push(")) = ")
                    .push_bind(&prefix);
                query.build().execute(&mut *transaction).await?;
            }
            transaction.commit().await?;
            let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                .fetch_one(&pool)
                .await?;
            ensure!(
                checkpoint.get::<i64, _>(0) == 0,
                "SQLite migration checkpoint is busy"
            );
            Ok::<_, anyhow::Error>(())
        }
        .await;
        pool.close().await;
        result?;
    }
    Ok(())
}
