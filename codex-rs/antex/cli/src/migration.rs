use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use serde_json::json;
use toml::Value;

const MCP_BRIDGE: &[u8] = include_bytes!("../../extensions/mcp/antex_ext_mcp.py");

#[path = "migration_sessions.rs"]
mod legacy_sessions;

pub(crate) struct MigrationPlan {
    writes: Vec<MigrationWrite>,
    descriptions: Vec<String>,
    sessions: Vec<legacy_sessions::PlannedSession>,
}

struct MigrationWrite {
    path: PathBuf,
    bytes: Vec<u8>,
    executable: bool,
}

impl MigrationPlan {
    pub(crate) fn codex(source: &Path, destination: &Path) -> io::Result<Self> {
        let config_path = source.join("config.toml");
        let text = match fs::read_to_string(&config_path) {
            Ok(text) if text.len() <= 1024 * 1024 => text,
            Ok(_) => return Err(io::Error::other("legacy config exceeds its byte budget")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let root: toml::Table = if text.is_empty() {
            toml::Table::new()
        } else {
            toml::from_str(&text).map_err(|_| io::Error::other("invalid legacy config"))?
        };
        let mut writes = Vec::new();
        let mut descriptions = Vec::new();
        migrate_config(&root, destination, &mut writes, &mut descriptions)?;
        migrate_mcp(&root, destination, &mut writes, &mut descriptions)?;
        migrate_skills(source, destination, &mut writes, &mut descriptions)?;
        let sessions = legacy_sessions::plan(source, destination, &mut descriptions)?;
        descriptions.sort();
        writes.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            writes,
            descriptions,
            sessions,
        })
    }

    pub(crate) fn describe(&self) -> &[String] {
        &self.descriptions
    }

    pub(crate) fn apply(self) -> io::Result<()> {
        for write in self.writes {
            if write.path.exists() {
                continue;
            }
            let parent = write
                .path
                .parent()
                .ok_or_else(|| io::Error::other("migration target has no parent"))?;
            fs::create_dir_all(parent)?;
            let temporary = parent.join(format!(
                ".antex-migrate-{}-{}",
                std::process::id(),
                write
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
            ));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(if write.executable { 0o700 } else { 0o600 });
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&write.bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &write.path)?;
        }
        for planned in self.sessions {
            fs::create_dir_all(&planned.home)?;
            let store = antex_runtime::SessionStore::new(&planned.home, &planned.workspace)?;
            if let Err(error) = store.import(planned.session)
                && error.kind() != io::ErrorKind::AlreadyExists
            {
                return Err(error);
            }
        }
        Ok(())
    }
}

fn migrate_skills(
    source: &Path,
    destination: &Path,
    writes: &mut Vec<MigrationWrite>,
    descriptions: &mut Vec<String>,
) -> io::Result<()> {
    let source = source.join("skills");
    if !source.exists() {
        return Ok(());
    }
    let source_metadata = fs::symlink_metadata(&source)?;
    if source_metadata.file_type().is_symlink() {
        descriptions.push("skip skills: source is a symbolic link".into());
        return Ok(());
    }
    if !source_metadata.is_dir() {
        descriptions.push("skip skills: source is not a directory".into());
        return Ok(());
    }
    let canonical_source = source.canonicalize()?;
    let target = destination.join("skills");
    if target.exists() {
        descriptions.push("skip skills: destination already exists".into());
        return Ok(());
    }
    let mut pending = vec![source.clone()];
    let mut files = Vec::new();
    let mut total = 0usize;
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                descriptions.push(format!(
                    "skip skill path {}: symbolic links are not imported",
                    path.strip_prefix(&source).unwrap_or(&path).display()
                ));
                continue;
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            if !kind.is_file() {
                descriptions.push(format!(
                    "skip skill path {}: unsupported file type",
                    path.strip_prefix(&source).unwrap_or(&path).display()
                ));
                continue;
            }
            if files.len() >= 1024 {
                return Err(io::Error::other("legacy skills exceed the file limit"));
            }
            let canonical_path = path.canonicalize()?;
            if !canonical_path.starts_with(&canonical_source) {
                return Err(io::Error::other(
                    "legacy skill path escaped its source directory",
                ));
            }
            let bytes = fs::read(canonical_path)?;
            if bytes.len() > 1024 * 1024 {
                return Err(io::Error::other("a legacy skill file exceeds 1 MiB"));
            }
            total = total.saturating_add(bytes.len());
            if total > 16 * 1024 * 1024 {
                return Err(io::Error::other("legacy skills exceed 16 MiB"));
            }
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                entry.metadata()?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            files.push(MigrationWrite {
                path: target.join(
                    path.strip_prefix(&source).map_err(|_| {
                        io::Error::other("legacy skill escaped its source directory")
                    })?,
                ),
                bytes,
                executable,
            });
        }
    }
    descriptions.push(format!("copy skills: {} regular files", files.len()));
    writes.extend(files);
    Ok(())
}

fn migrate_config(
    root: &toml::Table,
    destination: &Path,
    writes: &mut Vec<MigrationWrite>,
    descriptions: &mut Vec<String>,
) -> io::Result<()> {
    let target = destination.join("config.toml");
    if target.exists() {
        descriptions.push("skip config.toml: destination already exists".into());
        return Ok(());
    }
    let mut migrated = toml::Table::new();
    for key in ["model", "model_reasoning_effort"] {
        if let Some(value) = root.get(key) {
            migrated.insert(key.into(), value.clone());
        }
    }
    if let Some(Value::String(mode)) = root.get("sandbox_mode") {
        let permissions = match mode.as_str() {
            "read-only" => Some("read-only"),
            "workspace-write" => Some("workspace"),
            "danger-full-access" => Some("full"),
            _ => None,
        };
        if let Some(permissions) = permissions {
            migrated.insert("permissions".into(), Value::String(permissions.into()));
        } else {
            descriptions.push("skip config key sandbox_mode: unsupported value".into());
        }
    }
    let known = [
        "model",
        "model_reasoning_effort",
        "sandbox_mode",
        "mcp_servers",
    ];
    for key in root.keys().filter(|key| !known.contains(&key.as_str())) {
        descriptions.push(format!("skip config key {key}: unsupported by Antex"));
    }
    if migrated.is_empty() {
        descriptions.push("skip config.toml: no supported settings found".into());
        return Ok(());
    }
    let bytes = toml::to_string_pretty(&migrated)
        .map_err(|_| io::Error::other("failed to encode migrated config"))?
        .into_bytes();
    descriptions.push("write config.toml with supported settings".into());
    writes.push(MigrationWrite {
        path: target,
        bytes,
        executable: false,
    });
    Ok(())
}

fn migrate_mcp(
    root: &toml::Table,
    destination: &Path,
    writes: &mut Vec<MigrationWrite>,
    descriptions: &mut Vec<String>,
) -> io::Result<()> {
    let Some(servers) = root.get("mcp_servers").and_then(Value::as_table) else {
        return Ok(());
    };
    for (name, value) in BTreeMap::from_iter(servers) {
        let Some(server) = value.as_table() else {
            descriptions.push(format!("skip MCP {name}: definition is not a table"));
            continue;
        };
        if server.get("enabled").and_then(Value::as_bool) == Some(false) {
            descriptions.push(format!("skip MCP {name}: disabled"));
            continue;
        }
        let Some(command) = server.get("command").and_then(Value::as_str) else {
            let reason = if server.contains_key("url") {
                "streamable HTTP/OAuth is not implemented"
            } else {
                "missing stdio command"
            };
            descriptions.push(format!("skip MCP {name}: {reason}"));
            continue;
        };
        let arguments = match server.get("args") {
            None => Vec::new(),
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| io::Error::other("legacy MCP args must be strings"))
                })
                .collect::<io::Result<Vec<_>>>()?,
            Some(_) => return Err(io::Error::other("legacy MCP args must be an array")),
        };
        let identifier = identifier(name);
        let directory = destination
            .join("extensions")
            .join(format!("mcp-{identifier}"));
        if directory.exists() {
            descriptions.push(format!("skip MCP {name}: destination already exists"));
            continue;
        }
        let mut extension_arguments = vec![
            "--name".to_string(),
            identifier.clone(),
            "--".into(),
            command.into(),
        ];
        extension_arguments.extend(arguments);
        let definition = serde_json::to_vec_pretty(&json!({
            "name": identifier,
            "program": "antex_ext_mcp.py",
            "arguments": extension_arguments,
            "capabilities": ["network", "shell"]
        }))
        .map_err(|_| io::Error::other("failed to encode MCP extension definition"))?;
        writes.extend([
            MigrationWrite {
                path: directory.join("extension.json"),
                bytes: definition,
                executable: false,
            },
            MigrationWrite {
                path: directory.join("antex_ext_mcp.py"),
                bytes: MCP_BRIDGE.to_vec(),
                executable: true,
            },
        ]);
        descriptions.push(format!("write stdio MCP extension {name} as {identifier}"));
        for key in server.keys().filter(|key| {
            ![
                "command",
                "args",
                "enabled",
                "startup_timeout_sec",
                "tool_timeout_sec",
            ]
            .contains(&key.as_str())
        }) {
            descriptions.push(format!("skip MCP {name} key {key}: unsupported by adapter"));
        }
    }
    Ok(())
}

fn identifier(name: &str) -> String {
    let mut output = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .take(64)
        .collect::<String>();
    if output.is_empty() {
        output = "mcp".into();
    }
    output
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
