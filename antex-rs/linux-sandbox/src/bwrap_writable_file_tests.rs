use super::*;
use antex_protocol::permissions::FileSystemAccessMode;
use antex_protocol::permissions::FileSystemSandboxEntry;
use pretty_assertions::assert_eq;

#[test]
fn writable_files_and_file_aliases_do_not_gain_metadata_children() {
    let home = tempfile::tempdir().expect("home");
    let file = home.path().join("captures");
    let alias = home.path().join("capture-alias");
    fs::write(&file, "").expect("precreate writable file");
    std::os::unix::fs::symlink(&file, &alias).expect("file alias");
    let canonical_file = fs::canonicalize(&file).expect("canonical file");
    let file_arg = path_to_string(&canonical_file);

    for grant in [&file, &alias] {
        let mut policy = FileSystemSandboxPolicy::read_only();
        policy.entries.push(FileSystemSandboxEntry::new(
            AbsolutePathBuf::from_absolute_path(grant)
                .expect("absolute grant")
                .into(),
            FileSystemAccessMode::Write,
        ));
        let args = create_filesystem_args(&policy, home.path(), BwrapOptions::default())
            .expect("filesystem args");

        assert!(
            args.args
                .windows(/*size*/ 3)
                .any(|window| { window == ["--bind", file_arg.as_str(), file_arg.as_str()] })
        );
        // Only the exact file is rebound. No mount arguments may refer to
        // nonexistent children such as captures/.git or captures/.aws.
        let file_paths: Vec<&str> = args
            .args
            .iter()
            .filter(|arg| Path::new(arg).starts_with(&canonical_file))
            .map(String::as_str)
            .collect();
        assert_eq!(file_paths, vec![file_arg.as_str(), file_arg.as_str()]);
        assert!(
            args.synthetic_mount_targets
                .iter()
                .all(|target| !target.path().starts_with(&canonical_file))
        );
        assert!(
            args.protected_create_targets
                .iter()
                .all(|target| !target.path().starts_with(&canonical_file))
        );
        assert_eq!(fs::read_to_string(&file).expect("unchanged file"), "");
    }
}

#[test]
fn writable_directories_still_mask_missing_metadata_children() {
    let home = tempfile::tempdir().expect("home");
    let directory = home.path().join("workspace");
    fs::create_dir(&directory).expect("writable directory");
    let directory = fs::canonicalize(directory).expect("canonical directory");
    let mut policy = FileSystemSandboxPolicy::read_only();
    policy.entries.push(FileSystemSandboxEntry::new(
        AbsolutePathBuf::from_absolute_path(&directory)
            .expect("absolute grant")
            .into(),
        FileSystemAccessMode::Write,
    ));
    let args = create_filesystem_args(&policy, home.path(), BwrapOptions::default())
        .expect("filesystem args");

    for name in [".git", ".agents", ".antex", ".codex", ".aws"] {
        let child = directory.join(name);
        let child_arg = path_to_string(&child);
        let mount = args
            .args
            .windows(/*size*/ 4)
            .position(|window| window == ["--perms", "555", "--tmpfs", child_arg.as_str()])
            .expect("metadata directory mask");
        assert_eq!(
            &args.args[mount + 4..mount + 6],
            ["--remount-ro", child_arg.as_str()]
        );
        assert!(args.synthetic_mount_targets.iter().any(|target| {
            target.path() == child && target.kind() == SyntheticMountTargetKind::EmptyDirectory
        }));
    }
}
