//! Check the actual thread/resume parameters, not just preference detection.
use super::*;
use crate::legacy_core::config::ConfigBuilder;
use crate::legacy_core::config::ConfigOverrides;
use crate::resume_permissions::ResumePermissions;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn local_resume_sends_explicit_never_but_remote_resume_remains_server_owned()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("config.toml"),
        "approval_policy = \"never\"\nsandbox_mode = \"danger-full-access\"\n",
    )?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .loader_overrides(antex_config::LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await?;
    let selected = ResumePermissions::from_overrides(&config, &ConfigOverrides::default());
    for (mode, approval, sandbox) in [
        (
            ThreadParamsMode::Embedded,
            Some(antex_app_server_protocol::AskForApproval::Never),
            Some(antex_app_server_protocol::SandboxMode::DangerFullAccess),
        ),
        (ThreadParamsMode::Remote, None, None),
    ] {
        let params = thread_resume_params_from_config(
            config.clone(),
            ThreadId::new(),
            mode,
            /*remote_cwd_override*/ None,
            ResumeModelSettings::RestoreFromThread,
            selected,
        );
        assert_eq!(
            (
                params.approval_policy,
                params.sandbox,
                params.runtime_workspace_roots
            ),
            (approval, sandbox, None)
        );
    }
    Ok(())
}

#[tokio::test]
async fn local_resume_without_explicit_policy_keeps_the_saved_policy() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .loader_overrides(antex_config::LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await?;
    let selected = ResumePermissions::from_overrides(&config, &ConfigOverrides::default());
    let params = thread_resume_params_from_config(
        config,
        ThreadId::new(),
        ThreadParamsMode::Embedded,
        /*remote_cwd_override*/ None,
        ResumeModelSettings::RestoreFromThread,
        selected,
    );
    assert_eq!(
        (params.approval_policy, params.sandbox, params.permissions),
        (None, None, None)
    );
    Ok(())
}
