use antex_cloud_config::cloud_config_bundle_loader_for_storage;
use antex_config::CloudConfigBundleLoader;
use antex_config::ConfigLoadOptions;
use antex_core::config::Config;
use antex_core::config::ConfigBuilder;
use antex_core::config::ConfigOverrides;
use antex_core::config::LoaderOverrides;
use antex_core::config::bootstrap_auth_config;
use antex_core::config::find_antex_home;
use antex_core::config::load_config_toml_with_layer_stack;
use antex_utils_absolute_path::AbsolutePathBuf;
use antex_utils_cli::CliConfigOverrides;
use anyhow::Context;
use anyhow::Result;

pub(crate) async fn load_config(
    config_overrides: &CliConfigOverrides,
    loader_overrides: LoaderOverrides,
) -> Result<Config> {
    config_builder(
        config_overrides,
        loader_overrides,
        ConfigOverrides::default(),
    )
    .await?
    .build()
    .await
    .context("failed to load configuration")
}

pub(crate) async fn config_builder(
    config_overrides: &CliConfigOverrides,
    loader_overrides: LoaderOverrides,
    harness_overrides: ConfigOverrides,
) -> Result<ConfigBuilder> {
    let cli_overrides = config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    let codex_home = find_antex_home().context("failed to resolve ANTEX_HOME")?;
    let cwd = match harness_overrides.cwd.as_deref() {
        Some(cwd) => AbsolutePathBuf::relative_to_current_dir(cwd),
        None => AbsolutePathBuf::current_dir(),
    }
    .context("failed to resolve current directory")?;
    let bootstrap_config = load_config_toml_with_layer_stack(
        codex_home.as_path(),
        Some(&cwd),
        cli_overrides.clone(),
        ConfigLoadOptions {
            loader_overrides: loader_overrides.clone(),
            strict_config: false,
            cloud_config_bundle: CloudConfigBundleLoader::default(),
        },
    )
    .await
    .context("failed to load bootstrap configuration")?;
    let cloud_config_bundle = cloud_config_bundle_loader_for_storage(
        bootstrap_auth_config(codex_home.as_path(), &bootstrap_config)
            .context("failed to resolve cloud configuration authentication")?,
        /*enable_antex_api_key_env*/ false,
    )
    .await
    .context("failed to initialize cloud configuration authentication")?;

    Ok(ConfigBuilder::default()
        .codex_home(codex_home.to_path_buf())
        .cli_overrides(cli_overrides)
        .loader_overrides(loader_overrides)
        .harness_overrides(harness_overrides)
        .cloud_config_bundle(cloud_config_bundle)
        .fallback_cwd(Some(cwd.to_path_buf())))
}
