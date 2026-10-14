use super::ResumePermissions;
use crate::legacy_core::config::ConfigBuilder;
use crate::legacy_core::config::ConfigOverrides;
use antex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn implicit_defaults_and_unselected_profiles_preserve_saved_permissions()
-> color_eyre::Result<()> {
    let home = tempfile::tempdir()?;
    for text in [
        "",
        "[profiles.review]\napproval_policy = \"never\"\nsandbox_mode = \"danger-full-access\"\n",
    ] {
        std::fs::write(home.path().join("config.toml"), text)?;
        let config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .loader_overrides(antex_config::LoaderOverrides::without_managed_config_for_tests())
            .build()
            .await?;
        assert_eq!(
            ResumePermissions::from_overrides(&config, &ConfigOverrides::default()),
            ResumePermissions::default()
        );
    }
    Ok(())
}

#[tokio::test]
async fn explicit_never_and_full_access_are_selected_without_overriding_workspace_roots()
-> color_eyre::Result<()> {
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
    assert_eq!(
        ResumePermissions::from_overrides(&config, &ConfigOverrides::default()),
        ResumePermissions {
            approval_policy: true,
            profile: true,
            ..Default::default()
        }
    );
    Ok(())
}

#[tokio::test]
async fn explicit_user_and_launch_choices_override_saved_permissions() -> color_eyre::Result<()> {
    let home = tempfile::tempdir()?;
    let profile = home.path().join("work.config.toml");
    std::fs::write(
        &profile,
        "default_permissions = \":read-only\"\napproval_policy = \"never\"\napprovals_reviewer = \"auto_review\"\n",
    )?;
    let builder = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .loader_overrides(antex_config::LoaderOverrides {
            user_config_path: Some(AbsolutePathBuf::try_from(profile)?),
            user_config_profile: Some("work".parse()?),
            ..antex_config::LoaderOverrides::without_managed_config_for_tests()
        });
    let config = builder.clone().build().await?;
    assert_eq!(
        ResumePermissions::from_overrides(&config, &ConfigOverrides::default()),
        ResumePermissions {
            approval_policy: true,
            approvals_reviewer: true,
            profile: true,
            workspace_roots: false,
        },
    );
    let direct_rules = builder
        .clone()
        .cli_overrides(vec![(
            "permissions.safe.extends".into(),
            ":read-only".into(),
        )])
        .build()
        .await?;
    assert_eq!(
        ResumePermissions::from_overrides(&direct_rules, &ConfigOverrides::default()),
        ResumePermissions {
            approval_policy: true,
            approvals_reviewer: true,
            profile: true,
            workspace_roots: false,
        },
    );
    let config = builder
        .cli_overrides(vec![("approval_policy".into(), "on-request".into())])
        .build()
        .await?;
    assert_eq!(
        ResumePermissions::from_overrides(
            &config,
            &ConfigOverrides {
                default_permissions: Some(":read-only".into()),
                cwd: Some(home.path().to_path_buf()),
                ..Default::default()
            },
        ),
        ResumePermissions {
            approval_policy: true,
            approvals_reviewer: true,
            profile: true,
            workspace_roots: true,
        },
    );
    Ok(())
}
