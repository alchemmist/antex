use super::*;
use antex_protocol::permissions::FileSystemAccessMode;
use antex_protocol::permissions::FileSystemSandboxEntry;
use pretty_assertions::assert_eq;

#[test]
fn descriptor_denial_masks_a_real_device_entry_not_a_procfs_symlink() {
    let cwd = tempfile::tempdir().expect("cwd");
    for deny_descriptors in [false, true] {
        let mut policy = FileSystemSandboxPolicy::read_only();
        if deny_descriptors {
            policy.entries.push(FileSystemSandboxEntry::new(
                AbsolutePathBuf::from_absolute_path("/dev/fd")
                    .expect("fd path")
                    .into(),
                FileSystemAccessMode::Deny,
            ));
        }
        let args = create_bwrap_command_args(
            vec!["/bin/true".to_string()],
            &policy,
            cwd.path(),
            cwd.path(),
            BwrapOptions::default(),
        )
        .expect("filesystem setup");
        let dev_option = if deny_descriptors {
            "--dev-no-fd"
        } else {
            "--dev"
        };
        let dev = args
            .args
            .windows(/*size*/ 2)
            .position(|pair| pair == [dev_option, "/dev"])
            .expect("synthetic device tree");
        let mask = args
            .args
            .windows(/*size*/ 4)
            .position(|args| args == ["--perms", "000", "--tmpfs", "/dev/fd"]);
        assert_eq!(mask.is_some(), deny_descriptors);
        if let Some(mask) = mask {
            assert!(dev < mask);
            assert_eq!(&args.args[mask + 4..mask + 6], ["--remount-ro", "/dev/fd"]);
            let proc_mount = args
                .args
                .windows(/*size*/ 2)
                .position(|pair| pair == ["--proc", "/proc"])
                .expect("fresh procfs");
            assert!(
                mask < proc_mount,
                "fresh procfs must not replace the logical fd mask"
            );
        }
        assert!(
            !args
                .args
                .iter()
                .any(|arg| arg.starts_with(&format!("/proc/{}/", std::process::id())))
        );
    }
}
