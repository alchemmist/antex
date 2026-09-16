use antex_app_server_protocol::ConfigLayer as ApiConfigLayer;
use antex_app_server_protocol::ConfigLayerMetadata as ApiConfigLayerMetadata;
use antex_app_server_protocol::ConfigLayerSource as ApiConfigLayerSource;
use antex_config::ConfigLayer;
use antex_config::ConfigLayerMetadata;
use antex_config::ConfigLayerSource;

/// Converts a config-layer source owned by `antex-config` into the app-server wire type owned by
/// `antex-app-server-protocol`.
///
/// The types stay separate so app-server protocol ownership does not leak into the config domain
/// crate. Because this crate owns neither type, Rust's orphan rules require an explicit conversion
/// function instead of a `From` implementation.
pub(crate) fn config_layer_source_to_api(source: ConfigLayerSource) -> ApiConfigLayerSource {
    match source {
        ConfigLayerSource::PackagedDefaults { file } => {
            ApiConfigLayerSource::PackagedDefaults { file }
        }
        ConfigLayerSource::Mdm { domain, key } => ApiConfigLayerSource::Mdm { domain, key },
        ConfigLayerSource::System { file } => ApiConfigLayerSource::System { file },
        ConfigLayerSource::EnterpriseManaged { id, name } => {
            ApiConfigLayerSource::EnterpriseManaged { id, name }
        }
        ConfigLayerSource::User { file, profile } => ApiConfigLayerSource::User { file, profile },
        ConfigLayerSource::Project { dot_antex_folder } => {
            ApiConfigLayerSource::Project { dot_antex_folder }
        }
        ConfigLayerSource::SessionFlags => ApiConfigLayerSource::SessionFlags,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile { file } => {
            ApiConfigLayerSource::LegacyManagedConfigTomlFromFile { file }
        }
        ConfigLayerSource::LegacyManagedConfigTomlFromMdm => {
            ApiConfigLayerSource::LegacyManagedConfigTomlFromMdm
        }
    }
}

/// Converts config-layer metadata owned by `antex-config` into the app-server wire type owned by
/// `antex-app-server-protocol`.
///
/// The types stay separate so app-server protocol ownership does not leak into the config domain
/// crate. Because this crate owns neither type, Rust's orphan rules require an explicit conversion
/// function instead of a `From` implementation.
pub(crate) fn config_layer_metadata_to_api(
    metadata: ConfigLayerMetadata,
) -> ApiConfigLayerMetadata {
    ApiConfigLayerMetadata {
        name: config_layer_source_to_api(metadata.name),
        version: metadata.version,
    }
}

/// Converts a config layer owned by `antex-config` into the app-server wire type owned by
/// `antex-app-server-protocol`.
///
/// The types stay separate so app-server protocol ownership does not leak into the config domain
/// crate. Because this crate owns neither type, Rust's orphan rules require an explicit conversion
/// function instead of a `From` implementation.
pub(crate) fn config_layer_to_api(layer: ConfigLayer) -> ApiConfigLayer {
    ApiConfigLayer {
        name: config_layer_source_to_api(layer.name),
        version: layer.version,
        config: layer.config,
        disabled_reason: layer.disabled_reason,
    }
}
