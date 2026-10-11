//! Derives the enterprise credential owner from the active Codex account.

use antex_config::McpEmaAuthScope;
use antex_login::AntexAuth;

pub fn ema_auth_scope(auth: Option<&AntexAuth>) -> Option<McpEmaAuthScope> {
    let auth = auth?;
    McpEmaAuthScope::new(auth.get_chatgpt_user_id()?, auth.get_account_id()?)
}
