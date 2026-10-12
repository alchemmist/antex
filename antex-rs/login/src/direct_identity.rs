use std::io;
use std::path::Path;

use rand::RngCore;

/// Installation identity is independent of credentials and survives logout.
pub(crate) fn agent_host_id(home: &Path) -> io::Result<String> {
    let path = home.join("direct-chatgpt-host-id");
    match std::fs::read_to_string(&path) {
        Ok(id) => return parse(&id),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let id = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    );
    match crate::atomic_file::PendingFile::write(&path, id.as_bytes())?.create(&path) {
        Ok(()) => parse(&id),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            parse(&std::fs::read_to_string(path)?)
        }
        Err(error) => Err(error),
    }
}

fn parse(id: &str) -> io::Result<String> {
    if id.len() != 36
        || !id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(io::Error::other(
            "Invalid direct ChatGPT installation identity",
        ));
    }
    Ok(format!("urn:uuid:{}", id.to_ascii_lowercase()))
}

#[cfg(test)]
#[path = "direct_identity_tests.rs"]
mod tests;
