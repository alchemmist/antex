use super::MultitoolCli;
use super::Subcommand;
use anyhow::Context;
use anyhow::ensure;
use clap::Args;
use clap::Parser;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Args)]
pub(crate) struct MigrationCommand {
    #[arg(
        long,
        value_name = "DIRECTORY",
        help = "Legacy Codex home; defaults to CODEX_HOME or ~/.codex"
    )]
    source: Option<PathBuf>,
    #[arg(
        long,
        value_name = "DIRECTORY",
        help = "New Antex home; defaults to ANTEX_HOME or ~/.antex"
    )]
    destination: Option<PathBuf>,
    #[arg(
        long,
        help = "Copy and migrate data after all Codex clients have stopped"
    )]
    apply: bool,
}

pub(crate) fn dispatch() -> anyhow::Result<Option<()>> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if is_runtime_helper(&arguments)
        || !arguments.iter().skip(1).any(|arg| {
            matches!(
                arg.to_str(),
                Some("migrate" | "--help" | "-h" | "--version" | "-V")
            )
        })
    {
        return Ok(None);
    }
    let cli = MultitoolCli::try_parse_from(arguments).unwrap_or_else(|error| error.exit());
    let Some(Subcommand::Migrate(command)) = cli.subcommand else {
        return Ok(None);
    };
    ensure!(
        cli.remote.remote.is_none() && cli.remote.remote_auth_token_env.is_none(),
        "migration operates on local data; omit remote options"
    );
    ensure!(
        cli.config_overrides.raw_overrides.is_empty()
            && cli.feature_toggles.to_overrides()?.is_empty(),
        "migration reads source configuration; omit configuration overrides"
    );
    ensure!(
        cli.interactive.config_profile_v2.is_none() && !cli.interactive.strict_config,
        "migration reads all saved profiles; omit profile and strict-config options"
    );
    run(command)?;
    Ok(Some(()))
}

pub(crate) fn guard_initial_start() -> anyhow::Result<()> {
    if is_runtime_helper(&std::env::args_os().collect::<Vec<_>>()) {
        return Ok(());
    }
    if std::env::var_os("ANTEX_HOME").is_some_and(|value| !value.is_empty()) {
        return Ok(());
    }
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    if home.join(".antex").exists() {
        return Ok(());
    }
    let legacy = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    if legacy.is_dir() && std::fs::read_dir(&legacy)?.next().is_some() {
        anyhow::bail!(
            "existing Codex data found at {}. Run `antex migrate` to inspect the migration before starting Antex",
            legacy.display()
        );
    }
    Ok(())
}

fn is_runtime_helper(arguments: &[OsString]) -> bool {
    let stem = arguments
        .first()
        .and_then(|arg| Path::new(arg).file_stem())
        .and_then(OsStr::to_str);
    matches!(
        stem,
        Some("apply_patch" | "applypatch" | "antex-linux-sandbox" | "antex-execve-wrapper")
    ) || matches!(
        arguments.get(1).and_then(|arg| arg.to_str()),
        Some(
            "--antex-run-as-fs-helper"
                | "--antex-run-as-arg0-exec-helper"
                | "--antex-run-as-apply-patch"
                | "--run-as-windows-sandbox"
        )
    )
}

fn run(command: MigrationCommand) -> anyhow::Result<()> {
    let home = dirs::home_dir().context("cannot locate the home directory")?;
    let explicit_source = command.source.is_some();
    let source = command
        .source
        .or_else(|| {
            std::env::var_os("CODEX_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| home.join(".codex"));
    let destination = command
        .destination
        .or_else(|| {
            std::env::var_os("ANTEX_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| home.join(".antex"));
    let completed = if explicit_source {
        antex_home_migration::completed(&source, &destination)?
    } else {
        antex_home_migration::completed_destination(&destination)?
    };
    if let Some(plan) = completed {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"status": "already_migrated", "migration": plan})
            )?
        );
        return Ok(());
    }
    if command.apply {
        ensure_source_clients_stopped()?;
    }
    eprintln!("Inspecting source data...");
    let plan = antex_home_migration::inspect(&source, &destination)?;
    if !command.apply {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"status": "preview", "migration": plan})
            )?
        );
        eprintln!("Preview only. Stop Codex clients, then repeat this command with --apply.");
        return Ok(());
    }
    eprintln!("Copying data and updating metadata...");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let prepared = runtime.block_on(antex_home_migration::prepare(plan))?;
    ensure_source_clients_stopped()?;
    let plan = prepared.publish()?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"status": "complete", "migration": plan})
        )?
    );
    Ok(())
}

#[cfg(unix)]
fn ensure_source_clients_stopped() -> anyhow::Result<()> {
    let uid = Command::new("id").arg("-u").output()?;
    ensure!(
        uid.status.success(),
        "cannot determine the current user for migration"
    );
    let uid = String::from_utf8(uid.stdout)?;
    let output = Command::new("ps")
        .args(["-u", uid.trim(), "-o", "pid=,comm="])
        .output()?;
    ensure!(output.status.success(), "cannot inspect source processes");
    let mut running = Vec::new();
    for line in String::from_utf8(output.stdout)?.lines() {
        let Some((pid, executable)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let name = Path::new(executable.trim())
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        if matches!(
            name,
            "codex" | "Codex" | "codex-code-mode-host" | "codex-app-server" | "codex-exec-server"
        ) {
            running.push(format!("{name} (PID {pid})"));
        }
    }
    ensure!(
        running.is_empty(),
        "stop source clients before migrating: {}",
        running.join(", ")
    );
    Ok(())
}

#[cfg(windows)]
fn ensure_source_clients_stopped() -> anyhow::Result<()> {
    let output = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()?;
    ensure!(output.status.success(), "cannot inspect source processes");
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let name = line
            .split(',')
            .next()
            .unwrap_or_default()
            .trim_matches('"')
            .to_ascii_lowercase();
        ensure!(
            !matches!(
                name.as_str(),
                "codex.exe"
                    | "codex-code-mode-host.exe"
                    | "codex-app-server.exe"
                    | "codex-exec-server.exe"
            ),
            "stop source Codex clients before migrating"
        );
    }
    Ok(())
}
