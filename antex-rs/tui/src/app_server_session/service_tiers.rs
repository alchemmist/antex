use super::AppServerSession;
use antex_app_server_protocol::ClientRequest;
use antex_app_server_protocol::TurnSettingsUpdateParams;
use antex_app_server_protocol::TurnSettingsUpdateResponse;
use antex_protocol::ThreadId;
use color_eyre::eyre::Result;
use color_eyre::eyre::WrapErr;

impl AppServerSession {
    pub(crate) async fn update_active_service_tier(
        &mut self,
        thread_id: ThreadId,
        turn_id: String,
        service_tier: Option<String>,
    ) -> Result<()> {
        let request_id = self.next_request_id();
        self.client
            .request_typed::<TurnSettingsUpdateResponse>(ClientRequest::TurnSettingsUpdate {
                request_id,
                params: TurnSettingsUpdateParams {
                    thread_id: thread_id.to_string(),
                    turn_id,
                    subagent_spawn_policy: None,
                    approvals_reviewer: None,
                    model: None,
                    effort: None,
                    summary: None,
                    service_tier: Some(service_tier),
                },
            })
            .await
            .wrap_err("turn/settings/update failed while changing the active service tier")?;
        Ok(())
    }
}

impl AppServerSession {
    pub(crate) async fn update_active_subagent_mode(
        &mut self,
        thread_id: ThreadId,
        turn_id: String,
        policy: antex_protocol::config_types::SubagentSpawnPolicy,
    ) -> Result<()> {
        let request_id = self.next_request_id();
        self.client
            .request_typed::<TurnSettingsUpdateResponse>(ClientRequest::TurnSettingsUpdate {
                request_id,
                params: TurnSettingsUpdateParams {
                    thread_id: thread_id.to_string(),
                    turn_id,
                    subagent_spawn_policy: Some(policy),
                    ..Default::default()
                },
            })
            .await
            .wrap_err("turn/settings/update failed while changing subagent mode")?;
        Ok(())
    }
}
