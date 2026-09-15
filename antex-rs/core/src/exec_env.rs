pub use antex_apply_patch::ANTEX_APPLY_PATCH_PRESERVE_LINE_ENDINGS_ENV_VAR;
use antex_features::Feature;
use antex_features::Features;
use antex_protocol::SessionId;
use antex_protocol::ThreadId;
#[cfg(test)]
use antex_protocol::config_types::EnvironmentVariablePattern;
use antex_protocol::config_types::ShellEnvironmentPolicy;
use antex_protocol::models::ActivePermissionProfile;
use antex_protocol::shell_environment;
use std::collections::HashMap;

pub use antex_protocol::shell_environment::ANTEX_SESSION_ID_ENV_VAR;
pub use antex_protocol::shell_environment::ANTEX_THREAD_ID_ENV_VAR;

pub(crate) const ANTEX_VERSION_ENV_VAR: &str = "ANTEX_VERSION";

/// Informational name of the active permission profile. Child processes can
/// overwrite this value, so it must not be treated as proof of enforcement.
pub const ANTEX_PERMISSION_PROFILE_ENV_VAR: &str = "ANTEX_PERMISSION_PROFILE";

/// Construct an environment map based on the rules in the specified policy. The
/// resulting map can be passed directly to `Command::envs()` after calling
/// `env_clear()` to ensure no unintended variables are leaked to the spawned
/// process.
///
/// The derivation follows the algorithm documented in the struct-level comment
/// for [`ShellEnvironmentPolicy`].
///
/// `ANTEX_THREAD_ID` is injected when a thread id is provided, even when
/// `include_only` is set.
pub fn create_env(
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> HashMap<String, String> {
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    shell_environment::create_env(policy, thread_id.as_deref())
}

/// Exposes the shared root-session identity and harness version to shell commands.
pub(crate) fn inject_session_env(env: &mut HashMap<String, String>, session_id: SessionId) {
    env.insert(ANTEX_SESSION_ID_ENV_VAR.to_string(), session_id.to_string());
    env.insert("CODEX_SESSION_ID".to_string(), session_id.to_string());
    if let Some(thread_id) = env.get(ANTEX_THREAD_ID_ENV_VAR).cloned() {
        env.insert("CODEX_THREAD_ID".to_string(), thread_id);
    }
    if cfg!(windows) {
        env.retain(|key, _| {
            !key.eq_ignore_ascii_case(ANTEX_VERSION_ENV_VAR)
                && !key.eq_ignore_ascii_case("CODEX_VERSION")
        });
    }
    env.insert(
        ANTEX_VERSION_ENV_VAR.to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    env.insert(
        "CODEX_VERSION".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
}

/// Injects the selected named permission profile into a shell tool's environment.
///
/// This is applied after the shell environment policy so the runtime-selected
/// profile wins over inherited or configured values.
pub(crate) fn inject_permission_profile_env(
    env: &mut HashMap<String, String>,
    active_permission_profile: Option<&ActivePermissionProfile>,
) {
    if cfg!(windows) {
        env.retain(|key, _| !key.eq_ignore_ascii_case(ANTEX_PERMISSION_PROFILE_ENV_VAR));
    } else {
        env.remove(ANTEX_PERMISSION_PROFILE_ENV_VAR);
    }
    if let Some(active_permission_profile) = active_permission_profile {
        env.insert(
            ANTEX_PERMISSION_PROFILE_ENV_VAR.to_string(),
            active_permission_profile.id.clone(),
        );
    }
}

/// Carries the configured apply-patch line-ending rollout state into child
/// processes.
///
/// Apply this after inherited or client-provided environment overrides so the
/// active feature configuration remains authoritative. The in-process
/// apply-patch path reads the feature directly.
pub fn inject_apply_patch_env(env: &mut HashMap<String, String>, features: &Features) {
    env.retain(|key, _| !key.eq_ignore_ascii_case(ANTEX_APPLY_PATCH_PRESERVE_LINE_ENDINGS_ENV_VAR));
    if features.enabled(Feature::ApplyPatchPreserveLineEndings) {
        env.insert(
            ANTEX_APPLY_PATCH_PRESERVE_LINE_ENDINGS_ENV_VAR.to_string(),
            "1".to_string(),
        );
    }
}

#[cfg(all(test, target_os = "windows"))]
fn create_env_from_vars<I>(
    vars: I,
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    shell_environment::create_env_from_vars(vars, policy, thread_id.as_deref())
}

#[cfg(test)]
fn populate_env<I>(
    vars: I,
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    shell_environment::populate_env(vars, policy, thread_id.as_deref())
}

#[cfg(test)]
#[path = "exec_env_tests.rs"]
mod tests;
