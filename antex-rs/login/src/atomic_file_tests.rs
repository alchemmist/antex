use super::*;
use pretty_assertions::assert_eq;

#[test]
fn failed_publication_preserves_previous_file_and_cleans_temporary_files() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("auth.json");
    fs::write(&path, "existing credential").unwrap();
    let pending = PendingFile::write(&path, b"replacement credential").unwrap();
    fs::remove_file(&pending.0).unwrap();
    assert!(pending.replace(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "existing credential");
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
    PendingFile::write(&path, b"replacement credential")
        .unwrap()
        .replace(&path)
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "replacement credential");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
