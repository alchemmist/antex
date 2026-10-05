use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use antex_core::ContextKind;
use antex_core::Message;
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use cap_std::fs::OpenOptions;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

use crate::session_codec;

mod checkpoint;
mod pending;
mod previews;
mod ui_state;
pub use previews::SessionPreview;
mod transcript;
pub use transcript::TranscriptPage;

const MAX_RECORD_BYTES: u64 = 2 * antex_core::MAX_TRANSCRIPT_BYTES as u64;
const MAX_RECORDS: usize = 100_000;

pub struct SessionStore {
    home: PathBuf,
    workspace: PathBuf,
    relative: PathBuf,
}

pub struct Session {
    id: Uuid,
    head: Uuid,
    file: File,
    records: HashMap<Uuid, Index>,
    poisoned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionMessage {
    pub record_id: Uuid,
    pub message: Message,
}

pub struct ImportedSession {
    pub id: Uuid,
    pub messages: Vec<Message>,
    pub ui_state: Vec<(String, Value)>,
}

struct Index {
    parent: Option<Uuid>,
    offset: u64,
    kind: Kind,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Session,
    User,
    Assistant,
    ToolCall,
    ToolResult,
    Summary,
    Branch,
    Extension,
    Pending,
    UiState,
}

#[derive(Serialize, Deserialize)]
struct Record {
    schema_version: u32,
    id: Uuid,
    parent_id: Option<Uuid>,
    session_id: Uuid,
    timestamp: u64,
    kind: Kind,
    payload: Value,
}

impl SessionStore {
    pub fn new(home: &Path, workspace: &Path) -> io::Result<Self> {
        let workspace = workspace.canonicalize()?;
        let key = format!(
            "{:x}",
            Sha256::digest(workspace.as_os_str().as_encoded_bytes())
        );
        Ok(Self {
            home: home.to_owned(),
            workspace,
            relative: PathBuf::from("sessions").join(key),
        })
    }

    pub fn create(&self) -> io::Result<Session> {
        self.create_with_id(Uuid::new_v4())
    }

    pub fn import(&self, imported: ImportedSession) -> io::Result<()> {
        if imported.messages.len() > MAX_RECORDS || imported.ui_state.len() > 64 {
            return Err(io::Error::other("imported session exceeds its record budget"));
        }
        let mut session = self.create_with_id(imported.id)?;
        for message in imported.messages {
            session.append(&message)?;
        }
        for (name, value) in imported.ui_state {
            session.save_ui_state(&name, &value)?;
        }
        session.finish_turn()
    }

    fn create_with_id(&self, id: Uuid) -> io::Result<Session> {
        let home = Dir::open_ambient_dir(&self.home, ambient_authority())?;
        home.create_dir_all(&self.relative)?;
        let directory = home.open_dir(&self.relative)?;
        let mut options = OpenOptions::new();
        options.read(true).append(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            use cap_std::fs::PermissionsExt;
            options.mode(0o600);
            directory.set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))?;
        }
        let file = directory
            .open_with(format!("{id}.jsonl"), &options)?
            .into_std();
        file.try_lock()
            .map_err(|_| io::Error::other("session is already open"))?;
        let mut session = Session {
            id,
            head: Uuid::nil(),
            file,
            records: HashMap::new(),
            poisoned: false,
        };
        session.append_record(Kind::Session,json!({"workspace":self.workspace.to_string_lossy(),"workspace_bytes":self.workspace.as_os_str().as_encoded_bytes()}),/*parent*/ None)?;
        session.finish_turn()?;
        Ok(session)
    }

    pub fn open(&self, id: Uuid) -> io::Result<Session> {
        let directory =
            Dir::open_ambient_dir(&self.home, ambient_authority())?.open_dir(&self.relative)?;
        let mut options = OpenOptions::new();
        options.read(true).append(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut file = directory
            .open_with(format!("{id}.jsonl"), &options)?
            .into_std();
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("session must be a regular file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata()?;
            if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::geteuid() } {
                return Err(io::Error::other(
                    "session must be private and owned by the current user",
                ));
            }
        }
        file.try_lock()
            .map_err(|_| io::Error::other("session is already open"))?;
        let mut reader = BufReader::new(&mut file);
        let mut records = HashMap::new();
        let mut head = Uuid::nil();
        let mut torn_tail = None;
        loop {
            let offset = reader.stream_position()?;
            let bytes = read_line(&mut reader)?;
            if bytes.is_empty() {
                break;
            }
            if bytes.last() != Some(&b'\n') {
                torn_tail = Some(offset);
                break;
            }
            let record: Record = serde_json::from_slice(&bytes)
                .map_err(|_| io::Error::other("invalid session record"))?;
            if record.schema_version != 1
                || record.session_id != id
                || records.contains_key(&record.id)
                || records.len() >= MAX_RECORDS
            {
                return Err(io::Error::other(
                    "invalid session schema, identifier, or record count",
                ));
            }
            match record.parent_id {
                None if records.is_empty() && record.kind == Kind::Session => {}
                Some(parent) if records.contains_key(&parent) && record.kind != Kind::Session => {}
                _ => return Err(io::Error::other("invalid session parent chain")),
            }
            head = record.id;
            records.insert(
                record.id,
                Index {
                    parent: record.parent_id,
                    offset,
                    kind: record.kind,
                },
            );
        }
        drop(reader);
        if records.is_empty() {
            return Err(io::Error::other("session has no valid root record"));
        }
        if let Some(offset) = torn_tail {
            file.set_len(offset)?;
            file.sync_all()?;
        }
        Ok(Session {
            id,
            head,
            file,
            records,
            poisoned: false,
        })
    }

    pub fn list(&self) -> io::Result<Vec<Uuid>> {
        let directory = match Dir::open_ambient_dir(&self.home, ambient_authority())
            .and_then(|home| home.open_dir(&self.relative))
        {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut sessions = Vec::new();
        for entry in directory.entries()?.take(10_000) {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".jsonl"))
                .and_then(|id| Uuid::parse_str(id).ok())
            {
                sessions.push((entry.metadata()?.modified()?, id));
            }
        }
        sessions.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        Ok(sessions.into_iter().map(|(_, id)| id).collect())
    }
}

impl Session {
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn head(&self) -> Uuid {
        self.head
    }

    pub fn append(&mut self, message: &Message) -> io::Result<Uuid> {
        let payload = session_codec::encode(message);
        session_codec::decode(&payload)?;
        let kind = match message {
            Message::User(_) => Kind::User,
            Message::Assistant { .. } => Kind::Assistant,
            Message::Tool(_) => Kind::ToolResult,
            Message::Context(fragment) if fragment.kind() == ContextKind::Summary => Kind::Summary,
            Message::Context(_) => Kind::Extension,
        };
        let id = self.append_record(kind, payload, Some(self.head))?;
        if let Message::Assistant { tool_calls, .. } = message {
            for call in tool_calls {
                self.append_record(
                    Kind::ToolCall,
                    json!({"call_id":call.id,"name":call.name,"arguments":call.arguments}),
                    Some(self.head),
                )?;
            }
        }
        Ok(id)
    }

    pub fn append_extension(&mut self, name: &str, payload: Value) -> io::Result<Uuid> {
        if name.len() > 128 || payload.to_string().len() > antex_core::MAX_TEXT_BYTES {
            return Err(io::Error::other("extension record exceeds its budget"));
        }
        self.append_record(
            Kind::Extension,
            json!({"extension":name,"data":payload}),
            Some(self.head),
        )
    }

    pub fn branch(&mut self, parent: Uuid) -> io::Result<Uuid> {
        if !self.records.contains_key(&parent) {
            return Err(io::Error::other("branch parent does not exist"));
        }
        self.append_record(Kind::Branch, json!({}), Some(parent))
    }

    pub fn active_path(&mut self) -> io::Result<Vec<SessionMessage>> {
        let ids = self.selected_records()?;
        let mut messages = Vec::new();
        let mut bytes = 0;
        for (position, id) in ids.into_iter().enumerate() {
            if matches!(
                self.records[&id].kind,
                Kind::Session | Kind::ToolCall | Kind::Branch | Kind::Pending | Kind::UiState
            ) {
                continue;
            }
            let (mut record, length) = self.read_record(id)?;
            if record.kind == Kind::Summary && record.payload["type"] == "checkpoint" {
                if position != 0 {
                    continue;
                }
                record.payload = record.payload["summary"].take();
            }
            match record.kind {
                Kind::User | Kind::Assistant | Kind::ToolResult | Kind::Summary => {}
                Kind::Extension if record.payload["type"] == "context" => {}
                Kind::Session
                | Kind::ToolCall
                | Kind::Branch
                | Kind::Extension
                | Kind::Pending
                | Kind::UiState => {
                    continue;
                }
            }
            bytes += length;
            if bytes > antex_core::MAX_HISTORY_BYTES {
                return Err(io::Error::other("active session exceeds its replay budget"));
            }
            messages.push(SessionMessage {
                record_id: id,
                message: session_codec::decode(&record.payload)?,
            });
        }
        Ok(messages)
    }

    fn read_record(&mut self, id: Uuid) -> io::Result<(Record, usize)> {
        let offset = self
            .records
            .get(&id)
            .ok_or_else(|| io::Error::other("missing session record"))?
            .offset;
        self.file.seek(SeekFrom::Start(offset))?;
        let line = read_line(&mut BufReader::new(&mut self.file))?;
        let record: Record = serde_json::from_slice(&line)
            .map_err(|_| io::Error::other("invalid session record"))?;
        if record.id != id || record.session_id != self.id || record.schema_version != 1 {
            return Err(io::Error::other("session record changed unexpectedly"));
        }
        Ok((record, line.len()))
    }

    pub fn finish_turn(&self) -> io::Result<()> {
        self.file.sync_all()
    }

    pub fn recover_pending_tools(&mut self) -> io::Result<usize> {
        let mut pending = Vec::new();
        for entry in self.active_path()? {
            match entry.message {
                Message::Assistant { tool_calls, .. } => {
                    for call in tool_calls {
                        if pending.contains(&call.id) || pending.len() >= antex_core::MAX_TOOLS {
                            return Err(io::Error::other("invalid pending tool call chain"));
                        }
                        pending.push(call.id);
                    }
                }
                Message::Tool(output) => pending.retain(|id| id != &output.call_id),
                Message::Context(_) | Message::User(_) => {}
            }
        }
        let count = pending.len();
        for id in pending {
            self.append(&Message::Tool(antex_core::ToolOutput::new(id,antex_core::ToolOutcome::Cancelled,"tool outcome was not recorded before interruption; inspect the workspace before retrying".into())))?;
        }
        if count > 0 {
            self.finish_turn()?;
        }
        Ok(count)
    }

    fn append_record(
        &mut self,
        kind: Kind,
        payload: Value,
        parent: Option<Uuid>,
    ) -> io::Result<Uuid> {
        if self.poisoned || self.records.len() >= MAX_RECORDS {
            return Err(io::Error::other("session is not writable"));
        }
        let id = Uuid::new_v4();
        let record = Record {
            schema_version: 1,
            id,
            parent_id: parent,
            session_id: self.id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            kind,
            payload,
        };
        let mut bytes = serde_json::to_vec(&record)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(io::Error::other("session record exceeds its byte budget"));
        }
        let offset = self.file.seek(SeekFrom::End(0))?;
        if let Err(error) = self.file.write_all(&bytes) {
            self.poisoned = true;
            return Err(error);
        }
        self.records.insert(
            id,
            Index {
                parent,
                offset,
                kind,
            },
        );
        self.head = id;
        Ok(id)
    }
}

fn read_line(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_RECORD_BYTES + 1)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(io::Error::other("session record exceeds its byte budget"));
    }
    Ok(bytes)
}
