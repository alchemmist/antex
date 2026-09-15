use pretty_assertions::assert_eq;

use super::executable_identity_from_bytes;
use super::parse_antex_version;

#[test]
fn parses_antex_cli_version_output() {
    assert_eq!(
        parse_antex_version("codex 1.2.3\n").expect("version"),
        "1.2.3"
    );
}

#[test]
fn rejects_malformed_antex_cli_version_output() {
    assert!(parse_antex_version("codex\n").is_err());
}

#[test]
fn executable_identity_uses_binary_contents() {
    let old = executable_identity_from_bytes(b"old");
    let same = executable_identity_from_bytes(b"old");
    let new = executable_identity_from_bytes(b"new");

    assert_eq!(old, same);
    assert_ne!(old, new);
}
