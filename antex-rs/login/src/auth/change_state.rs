//! Tracks credential changes separately from ownership changes, before notifications coalesce.

use super::AntexAuth;

/// Opaque revisions local to one auth manager. Consumers must reset on reconnect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthChangeState {
    /// Advances whenever cached credentials change.
    pub generation: u64,
    /// Advances on login, logout, or changes of user, workspace, or auth mode.
    /// Credential changes with incomplete owner identity also advance this revision.
    pub owner_generation: u64,
}

pub(super) fn same_owner(previous: Option<&AntexAuth>, current: Option<&AntexAuth>) -> bool {
    let (Some(previous), Some(current)) = (previous, current) else {
        return false;
    };
    if previous.api_auth_mode() != current.api_auth_mode() {
        return false;
    }
    if matches!(previous, AntexAuth::ChatgptDirect(_)) {
        let registration = |auth: &AntexAuth| {
            auth.get_token_data()
                .ok()
                .and_then(|tokens| tokens.direct)
                .map(|direct| direct.client_id)
        };
        let previous = registration(previous);
        return previous.is_some() && previous == registration(current);
    }
    let (Some(user), Some(workspace)) = (previous.get_chatgpt_user_id(), previous.get_account_id())
    else {
        return false;
    };
    !user.trim().is_empty()
        && !workspace.trim().is_empty()
        && current.get_chatgpt_user_id().as_ref() == Some(&user)
        && current.get_account_id().as_ref() == Some(&workspace)
}
