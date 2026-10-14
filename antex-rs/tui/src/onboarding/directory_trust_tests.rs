use super::*;
use crate::config_update::trusted_project_edit;
use crate::config_update::write_config_batch;
use crate::legacy_core::config::ConfigBuilder;
use crate::legacy_core::config::ConfigOverrides;
use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::persist_selected_trust;
use crate::test_backend::VT100Backend;
use antex_config::LoaderOverrides;
use crossterm::event::KeyCode;
use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::widgets::WidgetRef;
use std::cell::Cell;
use std::path::PathBuf;

#[tokio::test]
async fn embedded_loaded_task_discloses_preserved_settings_after_trust_changes() -> Result<()> {
    let home = tempfile::tempdir()?;
    let project = tempfile::tempdir()?;
    let cwd = dunce::canonicalize(project.path())?;
    std::fs::create_dir(cwd.join(".git"))?;
    std::fs::write(cwd.join(".git/HEAD"), "ref: refs/heads/main\n")?;
    let project_table = trusted_project_edit(&cwd)
        .key_path
        .trim_end_matches(".trust_level")
        .to_string();
    std::fs::write(
        home.path().join("config.toml"),
        format!("[{project_table}]\ntrust_level = \"trusted\"\n"),
    )?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(cwd.clone()),
            ..Default::default()
        })
        .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await?;
    let mut app_server = crate::start_embedded_app_server_for_picker(&config).await?;
    let started = app_server.start_thread(&config).await?;
    let thread_id = started.session.thread_id;
    let loaded = app_server
        .thread_read(thread_id, /*include_turns*/ false)
        .await?;
    assert_eq!(loaded.cwd.as_path(), cwd.as_path());
    assert_eq!(loaded.status, antex_app_server_protocol::ThreadStatus::Idle);
    assert_eq!(
        app_server
            .thread_loaded_list(Default::default())
            .await?
            .data,
        vec![thread_id.to_string()]
    );

    // Change real saved trust after the embedded engine has loaded the task. Consent must
    // describe that live task, not promise that it was recreated with restricted settings.
    let mut untrusted = trusted_project_edit(&cwd);
    untrusted.value = serde_json::json!("untrusted");
    write_config_batch(app_server.request_handle(), vec![untrusted]).await?;
    let restricted_config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(cwd.clone()),
            ..Default::default()
        })
        .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await?;
    assert!(restricted_config.active_project.is_untrusted());
    let saved_config = std::fs::read(home.path().join("config.toml"))?;
    let mut tui = crate::tui::test_support::make_test_tui()?;

    // Both cases use the production lookup, screen construction and (for a loaded task)
    // thread/read cwd recheck. Only terminal interaction is supplied by the test.
    for resumed_thread in [Some(&loaded), None] {
        let screens = Cell::new(0);
        let result = check_directory_trust_with_screen(
            &mut tui,
            &app_server,
            &restricted_config,
            &AppServerTarget::Embedded,
            &cwd,
            DirectoryTrustOptions {
                resumed_thread,
                ..Default::default()
            },
            /*startup_draft*/ None,
            async |mut screen, app_server, _tui| {
                screens.set(screens.get() + 1);
                assert_eq!(screen.remote_trust_key, None);
                assert_eq!(screen.steps.len(), 1);
                let Step::TrustDirectory(widget) = &mut screen.steps[0] else {
                    panic!("expected folder consent");
                };
                assert_eq!(
                    (
                        widget.restricted,
                        widget.existing_task,
                        widget.cancel,
                        widget.cwd.as_path(),
                        widget.trust_target.as_path(),
                    ),
                    (
                        true,
                        resumed_thread.is_some(),
                        TrustCancelAction::Quit,
                        cwd.as_path(),
                        cwd.as_path(),
                    )
                );
                // Normalize only the already-asserted temporary paths for rendering; keep
                // all production disclosure, selection and cancellation state unchanged.
                let original_cwd =
                    std::mem::replace(&mut widget.cwd, PathBuf::from("/workspace/project"));
                let original_target = std::mem::replace(
                    &mut widget.trust_target,
                    PathBuf::from("/workspace/project"),
                );
                let mut terminal =
                    Terminal::new(VT100Backend::new(/*width*/ 70, /*height*/ 18))?;
                terminal.draw(|frame| (&*widget).render_ref(frame.area(), frame.buffer_mut()))?;
                let rendered = terminal
                    .backend()
                    .to_string()
                    .lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
                    .join("\n");
                if resumed_thread.is_some() {
                    insta::assert_snapshot!(rendered.trim(), @r"
                    Folder access
                      /workspace/project

                      This existing task may retain settings and history, including
                      project configuration or hooks loaded while it was trusted. To use
                      restricted settings, start a new task. The folder's trust setting
                      will not change.

                    › 1. Open existing task
                      2. Quit

                      enter continue · esc quit
                    ");
                } else {
                    insta::assert_snapshot!(rendered.trim(), @r"
                    Folder access
                      /workspace/project

                      Config, hooks, and exec policies from untrusted folders stay
                      disabled. Trusted project folders can still contribute settings.
                      Skills still load, and tools follow your permission settings.
                      Opening will not change saved trust.

                    › 1. Open restricted
                      2. Quit

                      enter continue · esc quit
                    ");
                }
                widget.cwd = original_cwd;
                widget.trust_target = original_target;
                screen.handle_key_event(KeyCode::Enter.into());
                let directory_trust_persisted =
                    persist_selected_trust(&mut screen, Some(app_server.request_handle())).await;
                assert!(screen.is_done());
                Ok(OnboardingResult {
                    directory_trust_persisted,
                    should_exit: screen.should_exit(),
                })
            },
        )
        .await?;
        assert_eq!(screens.get(), 1);
        assert_eq!(
            (result.directory_trust_persisted, result.should_exit),
            (false, false)
        );
        assert_eq!(
            std::fs::read(home.path().join("config.toml"))?,
            saved_config
        );
        let after_consent = app_server
            .thread_read(thread_id, /*include_turns*/ false)
            .await?;
        assert_eq!(
            Thread {
                created_at: loaded.created_at,
                updated_at: loaded.updated_at,
                recency_at: loaded.recency_at,
                ..after_consent
            },
            loaded
        );
    }
    app_server.shutdown().await?;
    Ok(())
}
