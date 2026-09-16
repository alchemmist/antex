use anyhow::Context;
use anyhow::ensure;
use std::fs;
use std::io::Write;
use std::path::Path;
use toml_edit::DocumentMut;
use toml_edit::Item;
use toml_edit::Key;
use toml_edit::Value;

pub(crate) fn rewrite(staging: &Path, source: &Path, destination: &Path) -> anyhow::Result<()> {
    for entry in fs::read_dir(staging)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name != "config.toml" && !name.ends_with(".config.toml") {
            continue;
        }
        let read_path = super::staged_file_path(&path, staging, destination)?;
        let original = fs::read_to_string(&read_path).context("read migrated configuration")?;
        let mut document = original
            .parse::<DocumentMut>()
            .map_err(|_| anyhow::anyhow!("invalid TOML configuration: {}", path.display()))?;
        let table = document.as_table_mut();
        if table.contains_key("allow_symlinked_codex_home") {
            ensure!(
                !table.contains_key("allow_symlinked_antex_home"),
                "both legacy and Antex home settings exist in {}",
                path.display()
            );
            let (key, item) = table
                .remove_entry("allow_symlinked_codex_home")
                .context("legacy home setting disappeared")?;
            let key = Key::new("allow_symlinked_antex_home")
                .with_leaf_decor(key.leaf_decor().clone())
                .with_dotted_decor(key.dotted_decor().clone());
            table.insert_formatted(&key, item);
        }
        for (key, item) in document.iter_mut() {
            rewrite_item(key.get(), item, source, destination);
        }
        let rewritten = document.to_string();
        if rewritten == original {
            continue;
        }
        let mut temporary = tempfile::NamedTempFile::new_in(staging)?;
        temporary.write_all(rewritten.as_bytes())?;
        temporary
            .as_file()
            .set_permissions(fs::metadata(read_path)?.permissions())?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path)?;
    }
    Ok(())
}

fn rewrite_item(key: &str, item: &mut Item, source: &Path, destination: &Path) {
    match item {
        Item::Value(value) => rewrite_value(key, value, source, destination),
        Item::Table(table) => {
            for (key, item) in table.iter_mut() {
                rewrite_item(key.get(), item, source, destination);
            }
        }
        Item::ArrayOfTables(tables) => {
            for table in tables.iter_mut() {
                for (key, item) in table.iter_mut() {
                    rewrite_item(key.get(), item, source, destination);
                }
            }
        }
        Item::None => {}
    }
}

fn rewrite_value(key: &str, value: &mut Value, source: &Path, destination: &Path) {
    match value {
        Value::String(text) => {
            let original = text.value();
            let rewritten = if key == "command" && original == "codex" {
                Some("antex".to_string())
            } else if matches!(key, "terminal_title" | "status_line") && original == "codex-version"
            {
                Some("antex-version".to_string())
            } else if key == "title" && original == "alchemmist codex" {
                Some("alchemmist antex".to_string())
            } else if (matches!(key, "args" | "command" | "cwd" | "path" | "sqlite_home")
                || key.ends_with("_path")
                || key.ends_with("_dir")
                || key.ends_with("_file"))
                && let Ok(relative) = Path::new(original).strip_prefix(source)
            {
                Some(destination.join(relative).to_string_lossy().into_owned())
            } else {
                None
            };
            if let Some(rewritten) = rewritten {
                let decor = value.decor().clone();
                *value = Value::from(rewritten);
                *value.decor_mut() = decor;
            }
        }
        Value::Array(array) => {
            for value in array.iter_mut() {
                rewrite_value(key, value, source, destination);
            }
        }
        Value::InlineTable(table) => {
            for (key, value) in table.iter_mut() {
                rewrite_value(key.get(), value, source, destination);
            }
        }
        Value::Integer(_) | Value::Float(_) | Value::Boolean(_) | Value::Datetime(_) => {}
    }
}
