use pretty_assertions::assert_eq;
use tokio::process::Command;

use super::*;
use crate::workflow::python_host::python_program;

#[tokio::test]
async fn project_workflow_overrides_user_and_builtin_workflows() {
    if Command::new(python_program())
        .arg("--version")
        .output()
        .await
        .is_err()
    {
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let codex_home = temp.path().join("home");
    let cwd = temp.path().join("project");
    tokio::fs::create_dir_all(codex_home.join("workflows"))
        .await
        .expect("create user workflow dir");
    tokio::fs::create_dir_all(cwd.join(".antex/workflows"))
        .await
        .expect("create project workflow dir");
    tokio::fs::write(
        codex_home.join("workflows/user.py"),
        "WORKFLOW = {'id':'same','title':'User'}\ndef run(ctx): pass\n",
    )
    .await
    .expect("write user workflow");
    tokio::fs::write(
        cwd.join(".antex/workflows/project.py"),
        "WORKFLOW = {'id':'same','title':'Project'}\ndef run(ctx): pass\n",
    )
    .await
    .expect("write project workflow");

    let (definitions, diagnostics) = discover_workflows(&codex_home, &cwd).await;

    assert_eq!(diagnostics, Vec::<String>::new());
    assert!(definitions.iter().any(|definition| {
        definition.manifest.id == "same"
            && definition.manifest.title == "Project"
            && definition.source == "project"
    }));
    assert!(definitions.iter().any(|definition| {
        definition.manifest.id == BUILTIN_PR_BABYSITTER_WORKFLOW_ID
            && definition.manifest.guardrails.max_agent_calls == 50_000
    }));
    assert!(
        definitions
            .iter()
            .any(|definition| definition.manifest.id == BUILTIN_RUFF_WORKFLOW_ID)
    );
    assert!(definitions.iter().any(|definition| {
        definition.manifest.id == BUILTIN_GITHUB_BOT_PR_WORKFLOW_ID
            && definition.manifest.fields.iter().any(|field| {
                field.id == "action" && field.default == Some(serde_json::json!("merge"))
            })
    }));
    assert!(
        codex_home
            .join("workflow-cache/github-bot-pr-maintenance-v1.py")
            .is_file()
    );
    assert!(
        codex_home
            .join("workflow-cache/pr-babysitter-v1.py")
            .is_file()
    );
}

#[tokio::test]
async fn builtin_pr_babysitter_completes_only_after_a_clean_snapshot() {
    if Command::new(python_program())
        .arg("--version")
        .output()
        .await
        .is_err()
    {
        return;
    }
    let test_script =
        antex_utils_cargo_bin::find_resource!("src/workflow/_builtin_pr_babysitter_tests.py")
            .expect("PR babysitter test script");
    let workflow = antex_utils_cargo_bin::find_resource!("src/workflow/builtin_pr_babysitter.py")
        .expect("PR babysitter workflow");

    let output = Command::new(python_program())
        .arg(test_script)
        .arg(workflow)
        .output()
        .await
        .expect("run PR babysitter tests");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn builtin_bot_maintenance_rejects_failed_and_incomplete_reports() {
    let test_script = antex_utils_cargo_bin::find_resource!(
        "src/workflow/_builtin_github_bot_pr_maintenance_tests.py"
    )
    .expect("bot maintenance test script");
    let output = Command::new(python_program())
        .arg(test_script)
        .output()
        .await
        .expect("run bot maintenance tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
