use antex_protocol::ThreadId;
use clap::ValueEnum;
use serde::Serialize;
use std::path::Path;
use std::sync::OnceLock;

#[cfg(all(unix, not(test)))]
static LAST_BINDING: std::sync::Mutex<Option<(String, String, String)>> =
    std::sync::Mutex::new(None);

static RESUME_OPTIONS: OnceLock<Vec<String>> = OnceLock::new();

pub(crate) fn initialize(cli: &crate::cli::Cli) {
    let _ = RESUME_OPTIONS.set(resume_options(cli));
    clear_binding();
}

fn resume_options(cli: &crate::cli::Cli) -> Vec<String> {
    let mut options = Vec::new();
    for (flag, value) in [
        ("--model", cli.model.clone()),
        ("--local-provider", cli.oss_provider.clone()),
        (
            "--profile",
            cli.config_profile_v2.as_ref().map(ToString::to_string),
        ),
        (
            "--sandbox",
            cli.sandbox_mode
                .and_then(|mode| mode.to_possible_value())
                .map(|value| value.get_name().to_owned()),
        ),
        (
            "--ask-for-approval",
            cli.approval_policy
                .and_then(|mode| mode.to_possible_value())
                .map(|value| value.get_name().to_owned()),
        ),
    ] {
        if let Some(value) = value {
            options.extend([flag.to_owned(), value]);
        }
    }
    for (flag, enabled) in [
        ("--oss", cli.oss),
        ("--approve-for-me", cli.auto_review),
        (
            "--dangerously-bypass-approvals-and-sandbox",
            cli.dangerously_bypass_approvals_and_sandbox,
        ),
        ("--dangerously-bypass-hook-trust", cli.bypass_hook_trust),
        ("--search", cli.web_search),
        ("--no-alt-screen", cli.no_alt_screen),
        ("--strict-config", cli.strict_config),
    ] {
        if enabled {
            options.push(flag.to_owned());
        }
    }
    for directory in &cli.add_dir {
        options.extend([
            "--add-dir".to_owned(),
            directory.to_string_lossy().into_owned(),
        ]);
    }
    for value in &cli.config_overrides.raw_overrides {
        options.extend(["-c".to_owned(), value.clone()]);
    }
    options
}

#[derive(Serialize)]
struct Binding<'a> {
    version: u8,
    thread_id: ThreadId,
    pid: u32,
    process_started_at: &'a str,
    pane_id: &'a str,
    socket_path: &'a str,
    home: &'a Path,
    resume_argv: Vec<String>,
}

fn resume_argv(thread_id: ThreadId, options: &[String], cwd: &Path) -> Vec<String> {
    let mut argv = vec![
        "antex".to_owned(),
        "resume".to_owned(),
        thread_id.to_string(),
    ];
    argv.extend_from_slice(options);
    argv.extend(["--cd".to_owned(), cwd.to_string_lossy().into_owned()]);
    argv
}

#[cfg(all(unix, not(test)))]
pub(crate) fn publish_thread_id(thread_id: Option<ThreadId>, home: &Path, cwd: &Path) {
    use std::process::Command;
    if thread_id.is_none() {
        clear_binding();
        return;
    }
    if thread_id.is_some() && RESUME_OPTIONS.get().is_none() {
        return;
    }
    static STARTED_AT: OnceLock<Option<String>> = OnceLock::new();
    let (Ok(pane), Ok(tmux)) = (std::env::var("TMUX_PANE"), std::env::var("TMUX")) else {
        return;
    };
    let Some((socket, _)) = tmux.split_once(',') else {
        return;
    };
    if pane
        .strip_prefix('%')
        .and_then(|value| value.parse::<u64>().ok())
        .is_none()
    {
        tracing::warn!("invalid tmux pane identity");
        return;
    }
    let started_at = STARTED_AT.get_or_init(|| {
        let output = Command::new("ps")
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .args(["-p", &std::process::id().to_string(), "-o", "lstart="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let value = String::from_utf8(output.stdout)
            .ok()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        (!value.is_empty()).then_some(value)
    });
    let Some(started_at) = started_at else {
        tracing::warn!("cannot establish Antex process identity for tmux restoration");
        return;
    };
    let payload = match thread_id {
        Some(thread_id) => serde_json::to_string(&Binding {
            version: 1,
            thread_id,
            pid: std::process::id(),
            process_started_at: started_at,
            pane_id: &pane,
            socket_path: socket,
            home,
            resume_argv: resume_argv(
                thread_id,
                RESUME_OPTIONS.get().map(Vec::as_slice).unwrap_or_default(),
                cwd,
            ),
        }),
        None => Ok(String::new()),
    };
    let Ok(payload) = payload else {
        tracing::warn!("cannot serialize Antex tmux binding");
        return;
    };
    let token = std::env::var("LAZY_TMUX_RESTORE_TOKEN").ok();
    match Command::new("tmux")
        .args(binding_command_args(
            socket,
            &pane,
            &payload,
            token.as_deref(),
        ))
        .status()
    {
        Ok(status) if status.success() => {
            if let Ok(mut last) = LAST_BINDING.lock() {
                *last = Some((socket.to_owned(), pane, payload));
            }
        }
        Ok(status) => tracing::warn!(%status, "failed to publish Antex tmux binding"),
        Err(error) => tracing::warn!(%error, "failed to publish Antex tmux binding"),
    }
}

#[cfg(all(not(unix), not(test)))]
pub(crate) fn publish_thread_id(_thread_id: Option<ThreadId>, _home: &Path, _cwd: &Path) {}

#[cfg(test)]
pub(crate) fn publish_thread_id(thread_id: Option<ThreadId>, _home: &Path, _cwd: &Path) {
    tests::PUBLISHED_THREAD.set(thread_id);
}

fn ownership_condition(payload: &str) -> String {
    let escaped = payload
        .replace('#', "##")
        .replace(',', "#,")
        .replace('}', "#}");
    format!("#{{==:#{{@antex_binding}},{escaped}}}")
}

#[cfg(all(unix, not(test)))]
pub(crate) fn clear_binding() {
    let Ok(mut last) = LAST_BINDING.lock() else {
        return;
    };
    let Some((socket, pane, payload)) = last.take() else {
        return;
    };
    let condition = ownership_condition(&payload);
    let clear = format!("set-option -pu -t {pane} @antex_binding");
    match std::process::Command::new("tmux")
        .args([
            "-S", &socket, "if-shell", "-F", "-t", &pane, &condition, &clear,
        ])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(%status, "failed to clear Antex tmux binding"),
        Err(error) => tracing::warn!(%error, "failed to clear Antex tmux binding"),
    }
}

#[cfg(any(not(unix), test))]
pub(crate) fn clear_binding() {}

fn binding_command_args(
    socket: &str,
    pane: &str,
    payload: &str,
    restore_token: Option<&str>,
) -> Vec<String> {
    let mut args = [
        "-S",
        socket,
        "set-option",
        "-p",
        "-t",
        pane,
        "@antex_binding",
        payload,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    if let Some(token) = restore_token
        && !token.is_empty()
        && token.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        args.extend(
            [
                ";",
                "set-option",
                "-p",
                "-t",
                pane,
                "@antex_restore_ack",
                token,
            ]
            .into_iter()
            .map(str::to_owned),
        );
    }
    args
}

#[cfg(test)]
#[path = "tmux_session_tests.rs"]
pub(crate) mod tests;
