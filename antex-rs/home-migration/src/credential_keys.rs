use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) fn home_key(prefix: &str, home: &Path) -> String {
    let digest = format!("{:x}", Sha256::digest(home.to_string_lossy().as_bytes()));
    format!("{prefix}|{}", &digest[..16])
}

pub(crate) fn mcp_keys(server: &str, url: &str, home: &Path) -> anyhow::Result<Vec<String>> {
    let mut payload = Map::new();
    payload.insert("type".to_string(), Value::String("http".to_string()));
    payload.insert("url".to_string(), Value::String(url.to_string()));
    payload.insert("headers".to_string(), Value::Object(Map::new()));
    let bodies = if server.starts_with("ema-idp:") {
        payload.insert("codex_home".to_string(), serde_json::to_value(home)?);
        vec![serde_json::to_string(
            &payload.into_iter().collect::<BTreeMap<_, _>>(),
        )?]
    } else {
        vec![
            serde_json::to_string(&payload)?,
            serde_json::to_string(&payload.into_iter().collect::<BTreeMap<_, _>>())?,
            format!(
                r#"{{"type":"http","url":{},"headers":{{}}}}"#,
                serde_json::to_string(url)?
            ),
        ]
    };
    let name = server.strip_prefix("local:").unwrap_or(server);
    let separator = if server.starts_with("executor:") {
        ':'
    } else {
        '|'
    };
    let mut keys = Vec::new();
    for body in bodies {
        let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
        let key = format!("{name}{separator}{}", &digest[..16]);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

pub(crate) fn mcp_secret_name(key: &str) -> anyhow::Result<antex_secrets::SecretName> {
    let digest = format!("{:X}", Sha256::digest(key.as_bytes()));
    antex_secrets::SecretName::new(&format!("MCP_OAUTH_{}", &digest[..32]))
}
