//! Metrics derived from loaded configuration at session start.

use super::Config;
use antex_features::FEATURES;
use antex_features::Stage;
use antex_otel::SessionTelemetry;

pub(crate) fn emit_session_start_metrics(config: &Config, telemetry: &SessionTelemetry) {
    for feature in FEATURES {
        if matches!(feature.stage, Stage::Removed) {
            continue;
        }
        if config.features.enabled(feature.id) != feature.default_enabled {
            telemetry.counter(
                "codex.feature.state",
                /*inc*/ 1,
                &[
                    ("feature", feature.key),
                    ("value", &config.features.enabled(feature.id).to_string()),
                ],
            );
        }
    }
    #[cfg(windows)]
    crate::windows_system_config::emit_namespace_squatting_probe();
}
