#![allow(clippy::unwrap_used)]

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn installation_identity_is_stable_concurrent_and_not_shared_between_homes() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let results = std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(8);
        for _ in 0..8 {
            workers.push(scope.spawn(|| agent_host_id(first.path()).unwrap()));
        }
        let mut results = Vec::with_capacity(workers.len());
        for worker in workers {
            results.push(worker.join().unwrap());
        }
        results
    });
    assert!(results.iter().all(|id| id == &results[0]));
    assert_eq!(agent_host_id(first.path()).unwrap(), results[0]);
    assert_ne!(agent_host_id(second.path()).unwrap(), results[0]);
    let uuid = results[0].strip_prefix("urn:uuid:").unwrap();
    assert_eq!(uuid.as_bytes()[14], b'4');
    assert!(b"89ab".contains(&uuid.as_bytes()[19]));
    assert_eq!(std::fs::read_dir(first.path()).unwrap().count(), 1);
}

#[test]
fn invalid_saved_identity_is_rejected_not_replaced_or_echoed() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("direct-chatgpt-host-id");
    std::fs::write(&path, "secret-invalid-identity").unwrap();
    let error = agent_host_id(home.path()).unwrap_err();
    assert!(!format!("{error:?}").contains("secret-invalid"));
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "secret-invalid-identity"
    );
}
