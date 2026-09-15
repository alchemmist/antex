use super::MigrationPlan;
use anyhow::Context;
use serde_json::Value;
use std::fs;
use std::fs::File;
use std::fs::FileTimes;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use url::Url;
use walkdir::WalkDir;

#[derive(Clone, Copy)]
enum DocumentKind {
    State,
    History,
}

pub(crate) fn rewrite(staging: &Path, plan: &MigrationPlan) -> anyhow::Result<()> {
    for entry in WalkDir::new(staging).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("auth.json" | ".credentials.json")
        ) {
            continue;
        }
        let extension = path.extension().and_then(|extension| extension.to_str());
        if !matches!(extension, Some("json" | "jsonl")) {
            continue;
        }
        let metadata = path.metadata()?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().context("metadata file has no parent")?)?;
        let mut changed = false;
        if extension == Some("jsonl") {
            let mut reader = BufReader::new(File::open(path)?);
            let mut line = Vec::new();
            loop {
                line.clear();
                if reader.read_until(b'\n', &mut line)? == 0 {
                    break;
                }
                if let Ok(mut value) = serde_json::from_slice::<Value>(&line)
                    && rewrite_value(&mut value, "", DocumentKind::History, plan)
                {
                    serde_json::to_writer(temporary.as_file_mut(), &value)?;
                    if line.ends_with(b"\r\n") {
                        temporary.write_all(b"\r\n")?;
                    } else if line.ends_with(b"\n") {
                        temporary.write_all(b"\n")?;
                    }
                    changed = true;
                } else {
                    temporary.write_all(&line)?;
                }
            }
        } else {
            let bytes = fs::read(path)?;
            if let Ok(mut value) = serde_json::from_slice::<Value>(&bytes)
                && rewrite_value(&mut value, "", DocumentKind::State, plan)
            {
                serde_json::to_writer_pretty(temporary.as_file_mut(), &value)?;
                changed = true;
            }
        }
        if changed {
            temporary
                .as_file()
                .set_times(FileTimes::new().set_modified(metadata.modified()?))?;
            temporary
                .as_file()
                .set_permissions(metadata.permissions())?;
            temporary.as_file().sync_all()?;
            temporary.persist(path)?;
        }
    }
    Ok(())
}

fn rewrite_value(value: &mut Value, key: &str, kind: DocumentKind, plan: &MigrationPlan) -> bool {
    if matches!(kind, DocumentKind::History) && matches!(key, "arguments" | "output" | "result") {
        return false;
    }
    match value {
        Value::Object(object) => {
            let mut changed = false;
            for (key, value) in object {
                changed |= rewrite_value(value, key, kind, plan);
            }
            changed
        }
        Value::Array(array) => {
            let mut changed = false;
            for value in array {
                changed |= rewrite_value(value, key, kind, plan);
            }
            changed
        }
        Value::String(text)
            if matches!(
                key,
                "path" | "cwd" | "home" | "codex_home" | "image_url" | "imageUrl" | "uri"
            ) || key.ends_with("_path")
                || key.ends_with("_dir")
                || key.ends_with("Path")
                || key.ends_with("Dir") =>
        {
            let file_url = text
                .starts_with("file://")
                .then(|| Url::parse(text).ok())
                .flatten();
            let path = match file_url.as_ref() {
                Some(url) => match url.to_file_path() {
                    Ok(path) => path,
                    Err(()) => return false,
                },
                None => Path::new(text).to_path_buf(),
            };
            if let Some(relative) = plan
                .source_aliases
                .iter()
                .find_map(|source| path.strip_prefix(source).ok())
            {
                let relocated = plan.destination.join(relative);
                *text = if file_url.is_some() {
                    match Url::from_file_path(relocated) {
                        Ok(url) => url.to_string(),
                        Err(()) => return false,
                    }
                } else {
                    relocated.to_string_lossy().into_owned()
                };
                true
            } else {
                false
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}
