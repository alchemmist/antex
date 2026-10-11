//! Migration preflight must not parse or block internal runtime-helper invocations.

use super::is_runtime_helper;
use std::ffi::OsString;

#[test]
fn runtime_helpers_bypass_migration_preflight_even_with_cli_like_payloads() {
    for helper in [
        "--antex-run-as-fs-helper",
        "--antex-run-as-arg0-exec-helper",
        "--antex-run-as-apply-patch",
        "--run-as-windows-sandbox",
        "--codex-run-as-process-setup",
        antex_sandboxing::CODEX_WINDOWS_MXC_ARG1,
    ] {
        let args = ["antex", helper, "--help", "migrate"].map(OsString::from);
        assert!(is_runtime_helper(&args), "{helper}");
    }
}

#[test]
fn normal_commands_still_run_migration_preflight() {
    for command in ["migrate", "exec", "app-server", "sandbox"] {
        let args = ["antex", command, "--codex-run-as-process-setup"].map(OsString::from);
        assert!(!is_runtime_helper(&args), "{command}");
    }
}
