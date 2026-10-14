use antex_arg0::Arg0DispatchPaths;
use antex_tui::Cli as TuiCli;
use antex_utils_cli::CliConfigOverrides;
use clap::Parser;
use clap::builder::NonEmptyStringValueParser;

use crate::SessionArchiveConfigOverrides;
use crate::finalize_session_archive_interactive;

#[derive(Debug, Parser)]
pub(crate) struct QueueCommand {
    /// Session UUID or exact session name.
    #[arg(long, value_name = "THREAD")]
    thread: String,

    /// Message text to queue.
    #[arg(long, value_name = "TEXT", value_parser = NonEmptyStringValueParser::new())]
    message: String,

    #[clap(flatten)]
    pub(crate) config_overrides: SessionArchiveConfigOverrides,
}

pub(crate) async fn run_queue_command(
    command: QueueCommand,
    interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<String> {
    let QueueCommand {
        thread,
        message,
        config_overrides,
    } = command;
    let cli =
        finalize_session_archive_interactive(interactive, root_config_overrides, config_overrides);
    if !cli.images.is_empty() {
        anyhow::bail!("`antex queue` does not support image attachments");
    }
    antex_tui::run_session_queue_command(
        thread,
        message,
        antex_tui::SessionArchiveCommandOptions {
            cli,
            arg0_paths,
            explicit_remote_endpoint: None,
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!("{error:#}"))
}
