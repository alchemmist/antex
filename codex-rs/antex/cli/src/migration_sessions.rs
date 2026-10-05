use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;

use antex_core::Content;
use antex_core::Message;
use antex_core::RawToolCall;
use antex_core::ToolOutcome;
use antex_core::ToolOutput;
use antex_core::ToolScope;
use antex_core::UserInput;
use antex_runtime::ImportedSession;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SESSION_RECORDS: usize = 100_000;

pub(super) struct PlannedSession {
    pub home: PathBuf,
    pub workspace: PathBuf,
    pub session: ImportedSession,
}

pub(super) fn plan(
    source: &Path,
    destination: &Path,
    descriptions: &mut Vec<String>,
) -> io::Result<Vec<PlannedSession>> {
    let stashes = stashes(source, descriptions)?;
    let root = source.join("sessions");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    collect(&root, &mut paths)?;
    paths.sort();
    let mut sessions = Vec::new();
    for path in paths {
        match parse(&path, destination, &stashes) {
            Ok(Some(session)) => sessions.push(session),
            Ok(None) => descriptions.push(format!(
                "skip session {}: no portable conversation records",
                path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown")
            )),
            Err(error) => descriptions.push(format!(
                "skip session {}: {error}",
                path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown")
            )),
        }
    }
    sessions.sort_by(|left, right| {
        left.workspace
            .cmp(&right.workspace)
            .then_with(|| left.session.id.cmp(&right.session.id))
    });
    descriptions.push(format!("import sessions: {} portable sessions", sessions.len()));
    Ok(sessions)
}

fn collect(root: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("legacy sessions directory is a symbolic link"));
    }
    let canonical_root = root.canonicalize()?;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
                if paths.len() >= 10_000 || !path.canonicalize()?.starts_with(&canonical_root) {
                    return Err(io::Error::other("legacy sessions exceed their path budget"));
                }
                paths.push(path);
            }
        }
    }
    Ok(())
}

fn parse(
    path: &Path,
    destination: &Path,
    stashes: &BTreeMap<Uuid, Value>,
) -> io::Result<Option<PlannedSession>> {
    if path.metadata()?.len() > MAX_SESSION_BYTES {
        return Err(io::Error::other("file exceeds 64 MiB"));
    }
    let mut id = None;
    let mut workspace = None;
    let mut messages = Vec::new();
    let reader = BufReader::new(fs::File::open(path)?);
    for (index, line) in reader.lines().enumerate() {
        if index >= MAX_SESSION_RECORDS {
            return Err(io::Error::other("record count exceeds 100000"));
        }
        let line = line?;
        if line.len() > antex_core::MAX_TRANSCRIPT_BYTES {
            return Err(io::Error::other("record exceeds its byte budget"));
        }
        let record: Value = serde_json::from_str(&line)
            .map_err(|_| io::Error::other("invalid JSONL record"))?;
        match record["type"].as_str() {
            Some("session_meta") => {
                let payload = &record["payload"];
                id = payload["id"]
                    .as_str()
                    .or_else(|| payload["session_id"].as_str())
                    .and_then(|value| Uuid::parse_str(value).ok());
                workspace = payload["cwd"].as_str().map(PathBuf::from);
            }
            Some("response_item") => {
                if let Some(message) = message(&record["payload"])? {
                    messages.push(message);
                }
            }
            _ => {}
        }
    }
    if messages.is_empty() {
        return Ok(None);
    }
    let id = id.ok_or_else(|| io::Error::other("missing session identifier"))?;
    let workspace = workspace.ok_or_else(|| io::Error::other("missing session workspace"))?;
    let workspace = workspace
        .canonicalize()
        .map_err(|_| io::Error::other("session workspace no longer exists"))?;
    let size = antex_core::context_size(&messages).map_err(io::Error::other)?;
    if size > antex_core::MAX_HISTORY_BYTES {
        return Err(io::Error::other("portable history exceeds the Antex replay budget"));
    }
    let ui_state = stashes
        .get(&id)
        .cloned()
        .map(|value| vec![("promptStash".into(), value)])
        .unwrap_or_default();
    Ok(Some(PlannedSession {
        home: destination.to_path_buf(),
        workspace,
        session: ImportedSession {
            id,
            messages,
            ui_state,
        },
    }))
}

fn message(payload: &Value) -> io::Result<Option<Message>> {
    match payload["type"].as_str() {
        Some("message") => match payload["role"].as_str() {
            Some("user") => {
                let content = content(payload, &["input_text"], true)?;
                Ok((!content.is_empty()).then_some(Message::User(UserInput {
                    content,
                    tool_scope: ToolScope::Default,
                })))
            }
            Some("assistant") => {
                let content = content(payload, &["output_text"], false)?;
                Ok((!content.is_empty()).then_some(Message::Assistant {
                    content,
                    tool_calls: Vec::new(),
                }))
            }
            _ => Ok(None),
        },
        Some("reasoning") => {
            let content = payload["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item["text"].as_str())
                .take(64)
                .map(bounded)
                .collect::<io::Result<Vec<_>>>()?
                .into_iter()
                .map(Content::Reasoning)
                .collect::<Vec<_>>();
            Ok((!content.is_empty()).then_some(Message::Assistant {
                content,
                tool_calls: Vec::new(),
            }))
        }
        Some("function_call" | "custom_tool_call") => {
            let id = required(payload, "call_id", 128)?;
            let name = required(payload, "name", 64)?;
            let arguments = payload["arguments"]
                .as_str()
                .or_else(|| payload["input"].as_str())
                .ok_or_else(|| io::Error::other("tool call has no arguments"))?;
            let arguments = bounded(arguments)?;
            Ok(Some(Message::Assistant {
                content: Vec::new(),
                tool_calls: vec![RawToolCall { id, name, arguments }],
            }))
        }
        Some("function_call_output" | "custom_tool_call_output") => {
            let id = required(payload, "call_id", 128)?;
            let text = output_text(&payload["output"])?;
            Ok(Some(Message::Tool(ToolOutput::new(
                id,
                ToolOutcome::Success,
                text,
            ))))
        }
        _ => Ok(None),
    }
}

fn content(payload: &Value, text_types: &[&str], images: bool) -> io::Result<Vec<Content>> {
    let Some(items) = payload["content"].as_array() else {
        return Err(io::Error::other("message content is not an array"));
    };
    if items.len() > 64 {
        return Err(io::Error::other("message has too many content blocks"));
    }
    let mut content = Vec::new();
    for item in items {
        match item["type"].as_str() {
            Some(kind) if text_types.contains(&kind) => {
                content.push(Content::Text(bounded(required_ref(item, "text")?)?));
            }
            Some("input_image") if images => {
                if let Some(image) = image(item["image_url"].as_str().unwrap_or_default())? {
                    content.push(image);
                }
            }
            _ => {}
        }
    }
    Ok(content)
}

fn image(url: &str) -> io::Result<Option<Content>> {
    let Some((header, encoded)) = url.split_once(',') else {
        return Ok(None);
    };
    let Some(media_type) = header
        .strip_prefix("data:")
        .and_then(|header| header.strip_suffix(";base64"))
    else {
        return Ok(None);
    };
    if !matches!(media_type, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
        || encoded.len() > antex_core::MAX_IMAGE_BYTES.div_ceil(3) * 4
    {
        return Err(io::Error::other("legacy image exceeds its import budget"));
    }
    let data = STANDARD
        .decode(encoded)
        .map_err(|_| io::Error::other("legacy image is not valid base64"))?;
    if data.len() > antex_core::MAX_IMAGE_BYTES {
        return Err(io::Error::other("legacy image exceeds its import budget"));
    }
    Ok(Some(Content::Image {
        media_type: media_type.into(),
        data: data.into(),
    }))
}

fn output_text(value: &Value) -> io::Result<String> {
    if let Some(text) = value.as_str() {
        return bounded(text);
    }
    let Some(items) = value.as_array() else {
        return Err(io::Error::other("tool output has an unsupported shape"));
    };
    let text = items
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    bounded(&text)
}

fn stashes(source: &Path, descriptions: &mut Vec<String>) -> io::Result<BTreeMap<Uuid, Value>> {
    let root = source.join("prompt-stashes");
    let mut stashes = BTreeMap::new();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(stashes),
        Err(error) => return Err(error),
    };
    for entry in entries.take(10_000) {
        let entry = entry?;
        if !entry.file_type()?.is_file() || entry.metadata()?.len() > 16 * 1024 * 1024 {
            continue;
        }
        let Some(id) = entry
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(|value| Uuid::parse_str(value).ok())
        else {
            continue;
        };
        let value: Value = serde_json::from_slice(&fs::read(entry.path())?)
            .map_err(|_| io::Error::other("invalid legacy prompt stash"))?;
        let composer = &value["composer"];
        let portable = value["version"] == 1
            && composer["text"].as_str().is_some_and(|text| text.len() <= 1024 * 1024)
            && ["local_images", "remote_image_urls", "text_elements", "mention_bindings", "pending_pastes"]
                .iter()
                .all(|key| composer[*key].as_array().is_some_and(Vec::is_empty));
        if portable {
            stashes.insert(
                id,
                json!({"version":1,"text":composer["text"],"elements":[],"images":[]}),
            );
        } else {
            descriptions.push(format!("skip prompt stash {id}: unsupported rich legacy state"));
        }
    }
    Ok(stashes)
}

fn required(value: &Value, key: &str, limit: usize) -> io::Result<String> {
    let value = required_ref(value, key)?;
    if value.is_empty() || value.len() > limit {
        return Err(io::Error::other("legacy identifier exceeds its budget"));
    }
    Ok(value.into())
}

fn required_ref<'a>(value: &'a Value, key: &str) -> io::Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| io::Error::other("legacy record is missing text"))
}

fn bounded(text: &str) -> io::Result<String> {
    if text.len() > antex_core::MAX_TEXT_BYTES {
        return Err(io::Error::other("legacy text exceeds its import budget"));
    }
    Ok(text.into())
}
