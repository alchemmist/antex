use super::*;
use antex_protocol::permissions::FileSystemAccessMode;
use antex_protocol::permissions::FileSystemSandboxEntry;
use pretty_assertions::assert_eq;

#[test]
fn denied_home_preserves_only_explicit_readable_descendants_and_deeper_denials() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let active = home.join("tmp/arg0/antex-arg0-active");
    let denied = active.join("private");
    let reopened = denied.join("readable");
    let leaf = reopened.join("secret");
    fs::create_dir_all(&leaf).expect("create read and deny roots");
    let outside = home.join("outside-workspace.txt");
    fs::write(&outside, "private").expect("home secret");
    let sibling = home.join("tmp/arg0/antex-arg0-other");
    fs::create_dir(&sibling).expect("other session");
    let policy = FileSystemSandboxPolicy::restricted(
        [
            (&home, FileSystemAccessMode::Deny),
            (&active, FileSystemAccessMode::Read),
            (&denied, FileSystemAccessMode::Deny),
            (&reopened, FileSystemAccessMode::Read),
            (&leaf, FileSystemAccessMode::Deny),
        ]
        .into_iter()
        .map(|(path, access)| {
            FileSystemSandboxEntry::new(
                AbsolutePathBuf::from_absolute_path(path)
                    .expect("absolute root")
                    .into(),
                access,
            )
        })
        .collect(),
    );
    let args = create_filesystem_args(&policy, temp.path(), BwrapOptions::default())
        .expect("filesystem args");

    let mut previous_bind = None;
    for (parent, permissions, child) in [
        (&home, "111", Some(&active)),
        (&denied, "111", Some(&reopened)),
        (&leaf, "000", None),
    ] {
        let parent_arg = path_to_string(parent);
        let mask = args
            .args
            .windows(/*size*/ 4)
            .rposition(|window| window == ["--perms", permissions, "--tmpfs", parent_arg.as_str()])
            .expect("denied parent retains its mask");
        if let Some(previous_bind) = previous_bind {
            assert!(
                previous_bind < mask,
                "deeper denial must survive the parent read bind"
            );
        }
        if let Some(child) = child {
            let child_arg = path_to_string(child);
            let mkdir = args
                .args
                .windows(/*size*/ 2)
                .rposition(|window| window == ["--dir", child_arg.as_str()])
                .expect("readable mount target recreated inside denied parent");
            let freeze = args
                .args
                .windows(/*size*/ 2)
                .rposition(|window| window == ["--remount-ro", parent_arg.as_str()])
                .expect("denied parent remains read-only");
            let bind = args
                .args
                .windows(/*size*/ 3)
                .rposition(|window| window == ["--ro-bind", child_arg.as_str(), child_arg.as_str()])
                .expect("explicit readable child rebound after masking its parent");
            assert!(mask < mkdir && mkdir < freeze && freeze < bind);
            previous_bind = Some(bind);
        }
    }

    assert!(!args.args.iter().any(|arg| arg == "--bind"));
    assert!(
        !args
            .args
            .windows(/*size*/ 3)
            .any(|window| window == ["--ro-bind", "/", "/"])
    );
    for private in [&outside, &sibling] {
        assert!(!args.args.iter().any(|arg| Path::new(arg) == private));
    }
    assert_eq!(
        fs::read_to_string(outside).expect("unchanged secret"),
        "private"
    );
}
