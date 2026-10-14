//! Exercises folder consent with the real embedded engine and a loopback Responses provider.

use super::focus_palette::PtyAntex;
use super::focus_palette::write_test_config;
use anyhow::Context;
use anyhow::Result;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use wiremock::MockServer;
use wiremock::matchers::body_string_contains;

const COMPLETION: &str = "embedded-folder-consent-complete";
const HOME_INSTRUCTIONS: &str = "Home config sentinel: prefer concise answers.";
const PROJECT_INSTRUCTIONS: &str = "Project config sentinel: prefer numbered answers.";

#[derive(Clone, Copy)]
enum SavedTrust {
    Unknown,
    Untrusted,
}

struct EmbeddedTrustTest {
    terminal: PtyAntex,
    server: MockServer,
    completion_mock: responses::ResponseMock,
    config_path: PathBuf,
    initial_config: String,
    project_key: String,
    _workspace: TempDir,
}

impl EmbeddedTrustTest {
    async fn start(trust: SavedTrust, prompt: &str) -> Result<Self> {
        let workspace = tempfile::tempdir()?;
        let repo_root = workspace.path().canonicalize()?;
        // A real project marker is required: projectless directories need not prompt.
        std::fs::create_dir(repo_root.join(".git"))?;
        std::fs::write(repo_root.join(".git/HEAD"), "ref: refs/heads/main\n")?;
        std::fs::create_dir(repo_root.join(".antex"))?;
        // Harmless configuration, not a command or permission override. The accepted case
        // proves this sentinel is effective, so its absence in restricted mode is meaningful.
        std::fs::write(
            repo_root.join(".antex/config.toml"),
            format!("developer_instructions = \"{PROJECT_INSTRUCTIONS}\"\n"),
        )?;
        let codex_home = tempfile::tempdir()?;
        write_test_config(codex_home.path(), &repo_root)?;
        let config_path = codex_home.path().join("config.toml");
        let mut config: toml::Value = toml::from_str(&std::fs::read_to_string(&config_path)?)?;
        let project_key = repo_root.display().to_string();
        match trust {
            SavedTrust::Unknown => {
                config["projects"]
                    .as_table_mut()
                    .context("fixture projects table")?
                    .remove(&project_key);
            }
            SavedTrust::Untrusted => {
                config["projects"][project_key.as_str()]["trust_level"] = "untrusted".into();
            }
        }
        let root = config.as_table_mut().context("fixture root table")?;
        root.insert("developer_instructions".into(), HOME_INSTRUCTIONS.into());
        root.insert("model_provider".into(), "test".into());
        // Keep unrelated first-run accessibility persistence out of the no-write assertion.
        root.insert(
            "tui".into(),
            toml::Value::Table(toml::toml! {
                screen_reader_detection_done = true
            }),
        );

        let server = responses::start_mock_server().await;
        // Automatic title requests must not consume the user turn's completion.
        let _title_mock = responses::mount_sse_once_match(
            &server,
            body_string_contains(r#"\"thread_source\":\"system\""#),
            responses::sse(vec![
                responses::ev_assistant_message("title", r#"{"title":"Folder consent probe"}"#),
                responses::ev_completed("title"),
            ]),
        )
        .await;
        let completion_mock = responses::mount_sse_once_match(
            &server,
            body_string_contains(r#"\"thread_source\":\"user\""#),
            responses::sse(vec![
                responses::ev_assistant_message("answer", COMPLETION),
                responses::ev_completed("complete"),
            ]),
        )
        .await;
        let base_url = server.uri();
        let config = toml::to_string(&config)?;
        let initial_config = format!(
            "{config}\n[model_providers.test]\nname = \"Mock\"\n\
             base_url = \"{base_url}/v1\"\nwire_api = \"responses\"\n\
             requires_openai_auth = false\nsupports_websockets = false\n",
        );
        std::fs::write(&config_path, &initial_config)?;
        let terminal = PtyAntex::start(&repo_root, codex_home, &["--no-alt-screen", prompt])?;
        Ok(Self {
            terminal,
            server,
            completion_mock,
            config_path,
            initial_config,
            project_key,
            _workspace: workspace,
        })
    }

    async fn wait_for_consent(&mut self, action: &str) -> Result<()> {
        self.terminal.wait_for_screen("Folder access")?;
        self.terminal.wait_for_screen(action)?;
        self.terminal.wait_for_screen("Quit")?;
        assert_eq!(
            self.terminal
                .screen_contents()
                .lines()
                .map(str::trim)
                .skip_while(|line| *line != "Folder access")
                .skip(/*n*/ 1)
                .find(|line| !line.is_empty()),
            Some(self.project_key.as_str()),
        );
        // Keep the existing protected-screen input-boundary settling convention. No
        // confirmation is sent until this screen is visible and pending state is checked.
        tokio::time::sleep(Duration::from_millis(/*millis*/ 200)).await;
        self.terminal.ensure_running()?;
        assert_eq!(
            std::fs::read_to_string(&self.config_path)?,
            self.initial_config,
        );
        self.assert_no_model_requests().await;
        Ok(())
    }

    async fn assert_no_model_requests(&self) {
        // Inspect all received Responses requests, including titles and unmatched requests,
        // rather than treating an empty user-response recorder as proof of inactivity.
        let requests = self.server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .all(|request| !request.url.path().ends_with("/responses")),
            "model request crossed the pending/cancelled consent boundary: {requests:?}",
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_unknown_folder_acceptance_persists_trust_and_completes_turn() -> Result<()> {
    const PROMPT: &str = "unknown-folder-acceptance-probe";
    let mut test = EmbeddedTrustTest::start(SavedTrust::Unknown, PROMPT).await?;
    test.wait_for_consent("Trust and continue").await?;
    let initial: toml::Value = toml::from_str(&test.initial_config)?;
    assert_eq!(initial["projects"].get(test.project_key.as_str()), None);

    test.terminal.write_input(b"\r")?;
    test.terminal.wait_for_screen(COMPLETION)?;
    test.terminal.ensure_running()?;
    let request = test.completion_mock.single_request();
    assert!(
        request
            .message_input_texts("user")
            .contains(&PROMPT.to_string())
    );
    let instructions = request.message_input_texts("developer").join("\n");
    assert!(instructions.contains(PROJECT_INSTRUCTIONS));
    assert!(!instructions.contains(HOME_INSTRUCTIONS));

    let saved: toml::Value = toml::from_str(&std::fs::read_to_string(&test.config_path)?)?;
    let mut expected_projects = initial["projects"].clone();
    expected_projects
        .as_table_mut()
        .context("expected projects table")?
        .insert(
            test.project_key.clone(),
            toml::Value::Table(toml::toml! {
                trust_level = "trusted"
            }),
        );
    // Compare the entire trust table: acceptance must not grant trust to a broader root.
    assert_eq!(saved["projects"], expected_projects);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_saved_untrusted_folder_completes_with_project_config_disabled() -> Result<()> {
    const PROMPT: &str = "saved-untrusted-restricted-probe";
    let mut test = EmbeddedTrustTest::start(SavedTrust::Untrusted, PROMPT).await?;
    test.wait_for_consent("Open restricted").await?;
    let initial: toml::Value = toml::from_str(&test.initial_config)?;
    assert_eq!(
        initial["projects"][test.project_key.as_str()]["trust_level"].as_str(),
        Some("untrusted"),
    );

    test.terminal.write_input(b"\r")?;
    test.terminal.wait_for_screen(COMPLETION)?;
    test.terminal.ensure_running()?;
    let request = test.completion_mock.single_request();
    assert!(
        request
            .message_input_texts("user")
            .contains(&PROMPT.to_string())
    );
    let instructions = request.message_input_texts("developer").join("\n");
    assert!(instructions.contains(HOME_INSTRUCTIONS));
    assert!(
        !request.body_contains_text(PROJECT_INSTRUCTIONS),
        "restricted session loaded project-local developer instructions",
    );
    let saved: toml::Value = toml::from_str(&std::fs::read_to_string(&test.config_path)?)?;
    assert_eq!(saved["projects"], initial["projects"]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_folder_cancellation_exits_without_model_request_or_trust_write() -> Result<()> {
    for trust in [SavedTrust::Unknown, SavedTrust::Untrusted] {
        let mut test = EmbeddedTrustTest::start(trust, "cancelled-folder-launch-probe").await?;
        let action = match trust {
            SavedTrust::Unknown => "Trust and continue",
            SavedTrust::Untrusted => "Open restricted",
        };
        test.wait_for_consent(action).await?;
        test.terminal.write_input(b"\x1b")?;
        // Observe a real successful exit, not the helper's Drop-driven child kill.
        let status = test.terminal.wait_for_exit()?;
        assert!(status.success(), "folder cancellation exited with {status}");
        test.assert_no_model_requests().await;
        assert_eq!(
            std::fs::read_to_string(&test.config_path)?,
            test.initial_config,
        );
    }
    Ok(())
}
