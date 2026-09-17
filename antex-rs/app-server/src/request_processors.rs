use crate::bespoke_event_handling::apply_bespoke_event_handling;
use crate::command_exec::CommandExecManager;
use crate::command_exec::StartCommandExecParams;
use crate::config_manager::ConfigManager;
use crate::error_code::INPUT_TOO_LARGE_ERROR_CODE;
use crate::error_code::invalid_params;
use crate::models::supported_models;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::ConnectionRequestId;
use crate::outgoing_message::OutgoingMessageSender;
use crate::outgoing_message::RequestContext;
use crate::outgoing_message::ThreadScopedOutgoingMessageSender;
use crate::skills_watcher::SkillsWatcher;
use crate::thread_status::ThreadWatchManager;
use crate::thread_status::resolve_thread_status;
use antex_analytics::AnalyticsEventsClient;
use antex_analytics::AnalyticsJsonRpcError;
use antex_analytics::InputError;
use antex_analytics::TurnSteerRequestError;
use antex_app_server_protocol::Account;
use antex_app_server_protocol::AccountLoginCompletedNotification;
use antex_app_server_protocol::AccountTokenUsageDailyBucket;
use antex_app_server_protocol::AccountTokenUsageSummary;
use antex_app_server_protocol::AccountUpdatedNotification;
use antex_app_server_protocol::AddCreditsNudgeCreditType;
use antex_app_server_protocol::AddCreditsNudgeEmailStatus;
use antex_app_server_protocol::AdditionalContextEntry;
use antex_app_server_protocol::AdditionalContextKind;
use antex_app_server_protocol::AppListUpdatedNotification;
use antex_app_server_protocol::AppSummary;
use antex_app_server_protocol::AppTemplateSummary;
use antex_app_server_protocol::AppTemplateUnavailableReason;
use antex_app_server_protocol::AppsInstalledParams;
use antex_app_server_protocol::AppsInstalledResponse;
use antex_app_server_protocol::AppsListParams;
use antex_app_server_protocol::AppsListResponse;
use antex_app_server_protocol::AppsReadParams;
use antex_app_server_protocol::AppsReadResponse;
use antex_app_server_protocol::AskForApproval;
use antex_app_server_protocol::AuthMode;
use antex_app_server_protocol::CancelLoginAccountParams;
use antex_app_server_protocol::CancelLoginAccountResponse;
use antex_app_server_protocol::CancelLoginAccountStatus;
use antex_app_server_protocol::ClientInfo;
use antex_app_server_protocol::ClientRequest;
use antex_app_server_protocol::ClientResponsePayload;
use antex_app_server_protocol::CodexErrorInfo;
use antex_app_server_protocol::CollaborationModeListParams;
use antex_app_server_protocol::CollaborationModeListResponse;
use antex_app_server_protocol::CommandExecParams;
use antex_app_server_protocol::CommandExecResizeParams;
use antex_app_server_protocol::CommandExecTerminateParams;
use antex_app_server_protocol::CommandExecWriteParams;
use antex_app_server_protocol::ConfigWarningNotification;
use antex_app_server_protocol::ConsumeAccountRateLimitResetCreditOutcome;
use antex_app_server_protocol::ConsumeAccountRateLimitResetCreditParams;
use antex_app_server_protocol::ConsumeAccountRateLimitResetCreditResponse;
use antex_app_server_protocol::ConversationGitInfo;
use antex_app_server_protocol::ConversationSummary;
use antex_app_server_protocol::DeprecationNoticeNotification;
use antex_app_server_protocol::DynamicToolFunctionSpec;
use antex_app_server_protocol::DynamicToolNamespaceTool;
use antex_app_server_protocol::DynamicToolSpec;
use antex_app_server_protocol::EnvironmentAddParams;
use antex_app_server_protocol::EnvironmentAddResponse;
use antex_app_server_protocol::EnvironmentInfoParams;
use antex_app_server_protocol::EnvironmentInfoResponse;
use antex_app_server_protocol::EnvironmentShellInfo;
use antex_app_server_protocol::EnvironmentStatusKind;
use antex_app_server_protocol::EnvironmentStatusParams;
use antex_app_server_protocol::EnvironmentStatusResponse;
use antex_app_server_protocol::ExperimentalFeature as ApiExperimentalFeature;
use antex_app_server_protocol::ExperimentalFeatureListParams;
use antex_app_server_protocol::ExperimentalFeatureListResponse;
use antex_app_server_protocol::ExperimentalFeatureStage as ApiExperimentalFeatureStage;
use antex_app_server_protocol::FeedbackUploadParams;
use antex_app_server_protocol::FeedbackUploadResponse;
use antex_app_server_protocol::GetAccountParams;
use antex_app_server_protocol::GetAccountRateLimitsResponse;
use antex_app_server_protocol::GetAccountResponse;
use antex_app_server_protocol::GetAccountTokenUsageParams;
use antex_app_server_protocol::GetAccountTokenUsageResponse;
use antex_app_server_protocol::GetAuthStatusParams;
use antex_app_server_protocol::GetAuthStatusResponse;
use antex_app_server_protocol::GetConversationSummaryParams;
use antex_app_server_protocol::GetConversationSummaryResponse;
use antex_app_server_protocol::GetWorkspaceMessagesResponse;
use antex_app_server_protocol::GitDiffToRemoteParams;
use antex_app_server_protocol::GitDiffToRemoteResponse;
use antex_app_server_protocol::GitInfo as ApiGitInfo;
use antex_app_server_protocol::HookHandlerMetadata;
use antex_app_server_protocol::HookMetadata;
use antex_app_server_protocol::HooksListParams;
use antex_app_server_protocol::HooksListResponse;
use antex_app_server_protocol::InitializeParams;
use antex_app_server_protocol::InitializeResponse;
use antex_app_server_protocol::InstalledApp;
use antex_app_server_protocol::JSONRPCErrorError;
use antex_app_server_protocol::ListMcpServerStatusParams;
use antex_app_server_protocol::ListMcpServerStatusResponse;
use antex_app_server_protocol::LoginAccountParams;
use antex_app_server_protocol::LoginAccountResponse;
use antex_app_server_protocol::LoginApiKeyParams;
use antex_app_server_protocol::LoginAppBrand;
use antex_app_server_protocol::LogoutAccountResponse;
use antex_app_server_protocol::MarketplaceAddParams;
use antex_app_server_protocol::MarketplaceAddResponse;
use antex_app_server_protocol::MarketplaceInterface;
use antex_app_server_protocol::MarketplaceRemoveParams;
use antex_app_server_protocol::MarketplaceRemoveResponse;
use antex_app_server_protocol::MarketplaceUpgradeErrorInfo;
use antex_app_server_protocol::MarketplaceUpgradeParams;
use antex_app_server_protocol::MarketplaceUpgradeResponse;
use antex_app_server_protocol::McpResourceReadParams;
use antex_app_server_protocol::McpResourceReadResponse;
use antex_app_server_protocol::McpServerOauthClientRegistration;
use antex_app_server_protocol::McpServerOauthLoginCompletedNotification;
use antex_app_server_protocol::McpServerOauthLoginParams;
use antex_app_server_protocol::McpServerOauthLoginResponse;
use antex_app_server_protocol::McpServerRefreshResponse;
use antex_app_server_protocol::McpServerStatus;
use antex_app_server_protocol::McpServerStatusDetail;
use antex_app_server_protocol::McpServerToolCallParams;
use antex_app_server_protocol::McpServerToolCallResponse;
use antex_app_server_protocol::MemoryResetResponse;
use antex_app_server_protocol::MockExperimentalMethodParams;
use antex_app_server_protocol::MockExperimentalMethodResponse;
use antex_app_server_protocol::ModelListParams;
use antex_app_server_protocol::ModelListResponse;
use antex_app_server_protocol::PermissionProfileListParams;
use antex_app_server_protocol::PermissionProfileListResponse;
use antex_app_server_protocol::PermissionProfileSummary;
use antex_app_server_protocol::PluginDetail;
use antex_app_server_protocol::PluginInstallParams;
use antex_app_server_protocol::PluginInstallResponse;
use antex_app_server_protocol::PluginInstalledParams;
use antex_app_server_protocol::PluginInstalledResponse;
use antex_app_server_protocol::PluginInterface;
use antex_app_server_protocol::PluginListMarketplaceKind;
use antex_app_server_protocol::PluginListParams;
use antex_app_server_protocol::PluginListResponse;
use antex_app_server_protocol::PluginMarketplaceEntry;
use antex_app_server_protocol::PluginReadParams;
use antex_app_server_protocol::PluginReadResponse;
use antex_app_server_protocol::PluginShareCheckoutParams;
use antex_app_server_protocol::PluginShareCheckoutResponse;
use antex_app_server_protocol::PluginShareContext;
use antex_app_server_protocol::PluginShareDeleteParams;
use antex_app_server_protocol::PluginShareDeleteResponse;
use antex_app_server_protocol::PluginShareDiscoverability;
use antex_app_server_protocol::PluginShareListItem;
use antex_app_server_protocol::PluginShareListParams;
use antex_app_server_protocol::PluginShareListResponse;
use antex_app_server_protocol::PluginSharePrincipal;
use antex_app_server_protocol::PluginSharePrincipalType;
use antex_app_server_protocol::PluginShareSaveParams;
use antex_app_server_protocol::PluginShareSaveResponse;
use antex_app_server_protocol::PluginShareTarget;
use antex_app_server_protocol::PluginShareUpdateDiscoverability;
use antex_app_server_protocol::PluginShareUpdateTargetsParams;
use antex_app_server_protocol::PluginShareUpdateTargetsResponse;
use antex_app_server_protocol::PluginSkillReadParams;
use antex_app_server_protocol::PluginSkillReadResponse;
use antex_app_server_protocol::PluginSource;
use antex_app_server_protocol::PluginSummary;
use antex_app_server_protocol::PluginUninstallParams;
use antex_app_server_protocol::PluginUninstallResponse;
use antex_app_server_protocol::RateLimitResetCredit;
use antex_app_server_protocol::RateLimitResetCreditStatus;
use antex_app_server_protocol::RateLimitResetCreditsSummary;
use antex_app_server_protocol::RateLimitResetType;
use antex_app_server_protocol::RequestId;
use antex_app_server_protocol::ReviewDelivery as ApiReviewDelivery;
use antex_app_server_protocol::ReviewStartParams;
use antex_app_server_protocol::ReviewStartResponse;
use antex_app_server_protocol::ReviewTarget as ApiReviewTarget;
use antex_app_server_protocol::SandboxMode;
use antex_app_server_protocol::SendAddCreditsNudgeEmailParams;
use antex_app_server_protocol::SendAddCreditsNudgeEmailResponse;
use antex_app_server_protocol::ServerNotification;
use antex_app_server_protocol::ServerRequestResolvedNotification;
use antex_app_server_protocol::SkillSummary;
use antex_app_server_protocol::SkillsConfigWriteParams;
use antex_app_server_protocol::SkillsConfigWriteResponse;
use antex_app_server_protocol::SkillsExtraRootsSetParams;
use antex_app_server_protocol::SkillsExtraRootsSetResponse;
use antex_app_server_protocol::SkillsListParams;
use antex_app_server_protocol::SkillsListResponse;
use antex_app_server_protocol::SortDirection;
use antex_app_server_protocol::Thread;
use antex_app_server_protocol::ThreadApproveGuardianDeniedActionParams;
use antex_app_server_protocol::ThreadApproveGuardianDeniedActionResponse;
use antex_app_server_protocol::ThreadArchiveParams;
use antex_app_server_protocol::ThreadArchiveResponse;
use antex_app_server_protocol::ThreadArchivedNotification;
use antex_app_server_protocol::ThreadBackgroundTerminal;
use antex_app_server_protocol::ThreadBackgroundTerminalsCleanParams;
use antex_app_server_protocol::ThreadBackgroundTerminalsCleanResponse;
use antex_app_server_protocol::ThreadBackgroundTerminalsListParams;
use antex_app_server_protocol::ThreadBackgroundTerminalsListResponse;
use antex_app_server_protocol::ThreadBackgroundTerminalsTerminateParams;
use antex_app_server_protocol::ThreadBackgroundTerminalsTerminateResponse;
use antex_app_server_protocol::ThreadClosedNotification;
use antex_app_server_protocol::ThreadCompactStartParams;
use antex_app_server_protocol::ThreadCompactStartResponse;
use antex_app_server_protocol::ThreadContextReadParams;
use antex_app_server_protocol::ThreadContextReadResponse;
use antex_app_server_protocol::ThreadDecrementElicitationParams;
use antex_app_server_protocol::ThreadDecrementElicitationResponse;
use antex_app_server_protocol::ThreadDeleteParams;
use antex_app_server_protocol::ThreadDeleteResponse;
use antex_app_server_protocol::ThreadDeletedNotification;
use antex_app_server_protocol::ThreadForkParams;
use antex_app_server_protocol::ThreadForkResponse;
use antex_app_server_protocol::ThreadGoal;
use antex_app_server_protocol::ThreadGoalClearParams;
use antex_app_server_protocol::ThreadGoalClearResponse;
use antex_app_server_protocol::ThreadGoalClearedNotification;
use antex_app_server_protocol::ThreadGoalGetParams;
use antex_app_server_protocol::ThreadGoalGetResponse;
use antex_app_server_protocol::ThreadGoalSetParams;
use antex_app_server_protocol::ThreadGoalSetResponse;
use antex_app_server_protocol::ThreadGoalStatus;
use antex_app_server_protocol::ThreadGoalUpdatedNotification;
use antex_app_server_protocol::ThreadHistoryBuilder;
#[cfg(test)]
use antex_app_server_protocol::ThreadHistoryMode;
use antex_app_server_protocol::ThreadIncrementElicitationParams;
use antex_app_server_protocol::ThreadIncrementElicitationResponse;
use antex_app_server_protocol::ThreadInjectItemsParams;
use antex_app_server_protocol::ThreadInjectItemsResponse;
use antex_app_server_protocol::ThreadItem;
use antex_app_server_protocol::ThreadItemEntry;
use antex_app_server_protocol::ThreadItemsListParams;
use antex_app_server_protocol::ThreadItemsListResponse;
use antex_app_server_protocol::ThreadListCwdFilter;
use antex_app_server_protocol::ThreadListParams;
use antex_app_server_protocol::ThreadListResponse;
use antex_app_server_protocol::ThreadLoadedListParams;
use antex_app_server_protocol::ThreadLoadedListResponse;
use antex_app_server_protocol::ThreadMemoryModeSetParams;
use antex_app_server_protocol::ThreadMemoryModeSetResponse;
use antex_app_server_protocol::ThreadMetadataGitInfoUpdateParams;
use antex_app_server_protocol::ThreadMetadataUpdateParams;
use antex_app_server_protocol::ThreadMetadataUpdateResponse;
use antex_app_server_protocol::ThreadNameUpdatedNotification;
use antex_app_server_protocol::ThreadProjectUpdatedNotification;
use antex_app_server_protocol::ThreadReadParams;
use antex_app_server_protocol::ThreadReadResponse;
use antex_app_server_protocol::ThreadRealtimeAppendAudioParams;
use antex_app_server_protocol::ThreadRealtimeAppendAudioResponse;
use antex_app_server_protocol::ThreadRealtimeAppendSpeechParams;
use antex_app_server_protocol::ThreadRealtimeAppendSpeechResponse;
use antex_app_server_protocol::ThreadRealtimeAppendTextParams;
use antex_app_server_protocol::ThreadRealtimeAppendTextResponse;
use antex_app_server_protocol::ThreadRealtimeListVoicesResponse;
use antex_app_server_protocol::ThreadRealtimeStartParams;
use antex_app_server_protocol::ThreadRealtimeStartResponse;
use antex_app_server_protocol::ThreadRealtimeStartTransport;
use antex_app_server_protocol::ThreadRealtimeStopParams;
use antex_app_server_protocol::ThreadRealtimeStopResponse;
use antex_app_server_protocol::ThreadResumeInitialTurnsPageParams;
use antex_app_server_protocol::ThreadResumeParams;
use antex_app_server_protocol::ThreadResumeResponse;
use antex_app_server_protocol::ThreadSearchOccurrence;
use antex_app_server_protocol::ThreadSearchOccurrencesParams;
use antex_app_server_protocol::ThreadSearchOccurrencesResponse;
use antex_app_server_protocol::ThreadSearchParams;
use antex_app_server_protocol::ThreadSearchResponse;
use antex_app_server_protocol::ThreadSearchResult;
use antex_app_server_protocol::ThreadSearchSortKey;
use antex_app_server_protocol::ThreadSearchTextRange;
use antex_app_server_protocol::ThreadSetNameParams;
use antex_app_server_protocol::ThreadSetNameResponse;
use antex_app_server_protocol::ThreadSettings;
use antex_app_server_protocol::ThreadSettingsUpdateParams;
use antex_app_server_protocol::ThreadSettingsUpdateResponse;
use antex_app_server_protocol::ThreadShellCommandParams;
use antex_app_server_protocol::ThreadShellCommandResponse;
use antex_app_server_protocol::ThreadSortKey;
use antex_app_server_protocol::ThreadSourceKind;
use antex_app_server_protocol::ThreadStartParams;
use antex_app_server_protocol::ThreadStartResponse;
use antex_app_server_protocol::ThreadStartedNotification;
use antex_app_server_protocol::ThreadStatus;
use antex_app_server_protocol::ThreadTimelineListParams;
use antex_app_server_protocol::ThreadTimelineListResponse;
use antex_app_server_protocol::ThreadTurnsListParams;
use antex_app_server_protocol::ThreadTurnsListResponse;
use antex_app_server_protocol::ThreadUnarchiveParams;
use antex_app_server_protocol::ThreadUnarchiveResponse;
use antex_app_server_protocol::ThreadUnarchivedNotification;
use antex_app_server_protocol::ThreadUnsubscribeParams;
use antex_app_server_protocol::ThreadUnsubscribeResponse;
use antex_app_server_protocol::ThreadUnsubscribeStatus;
use antex_app_server_protocol::Turn;
use antex_app_server_protocol::TurnEnvironmentParams;
use antex_app_server_protocol::TurnError;
use antex_app_server_protocol::TurnInterruptParams;
use antex_app_server_protocol::TurnInterruptResponse;
use antex_app_server_protocol::TurnItemsView;
use antex_app_server_protocol::TurnSettingsUpdateParams;
use antex_app_server_protocol::TurnSettingsUpdateResponse;
use antex_app_server_protocol::TurnSettingsUpdateStatus;
use antex_app_server_protocol::TurnStartParams;
use antex_app_server_protocol::TurnStartResponse;
use antex_app_server_protocol::TurnStatus;
use antex_app_server_protocol::TurnSteerParams;
use antex_app_server_protocol::TurnSteerResponse;
use antex_app_server_protocol::UserInput as V2UserInput;
use antex_app_server_protocol::WindowsSandboxReadiness;
use antex_app_server_protocol::WindowsSandboxReadinessResponse;
use antex_app_server_protocol::WindowsSandboxSetupCompletedNotification;
use antex_app_server_protocol::WindowsSandboxSetupMode;
use antex_app_server_protocol::WindowsSandboxSetupStartParams;
use antex_app_server_protocol::WindowsSandboxSetupStartResponse;
use antex_app_server_protocol::WorkspaceMessage;
use antex_app_server_protocol::WorkspaceMessageType;
use antex_arg0::Arg0DispatchPaths;
use antex_backend_client::AddCreditsNudgeCreditType as BackendAddCreditsNudgeCreditType;
use antex_backend_client::AntexWorkspaceMessage as BackendWorkspaceMessage;
use antex_backend_client::AntexWorkspaceMessageType as BackendWorkspaceMessageType;
use antex_backend_client::AntexWorkspaceMessagesResponse as BackendWorkspaceMessagesResponse;
use antex_backend_client::Client as BackendClient;
use antex_backend_client::ConsumeRateLimitResetCreditCode as BackendConsumeRateLimitResetCreditCode;
use antex_backend_client::RateLimitResetCreditDetails as BackendRateLimitResetCreditDetails;
use antex_backend_client::RateLimitResetCreditsDetails as BackendRateLimitResetCreditsDetails;
use antex_backend_client::RequestError as BackendRequestError;
use antex_backend_client::TokenUsageProfile;
use antex_chatgpt::connectors;
use antex_config::CloudConfigBundleLoadError;
use antex_config::CloudConfigBundleLoadErrorCode;
use antex_config::ConfigLayerStack;
use antex_config::loader::project_trust_key;
use antex_config::types::McpServerTransportConfig;
use antex_connectors::AppInfo;
use antex_core::AntexThread;
use antex_core::AntexThreadSettingsOverrides;
use antex_core::ForkSnapshot;
use antex_core::McpManager;
use antex_core::NewThread;
use antex_core::NotSubmittedReason;
#[cfg(test)]
use antex_core::SessionMeta;
use antex_core::StartThreadOptions;
use antex_core::SteerSubmission;
use antex_core::ThreadConfigSnapshot;
use antex_core::ThreadManager;
use antex_core::TurnInput;
use antex_core::TurnInputRequest;
use antex_core::TurnInputSubmission;
use antex_core::TurnStartOptions;
use antex_core::config::Config;
use antex_core::config::ConfigOverrides;
use antex_core::config::NetworkProxyAuditMetadata;
use antex_core::config::edit::ConfigEdit;
use antex_core::config::edit::ConfigEditsBuilder;
use antex_core::connectors::AccessibleConnectorsStatus;
use antex_core::exec::ExecCapturePolicy;
use antex_core::exec::ExecExpiration;
use antex_core::exec::ExecParams;
use antex_core::exec_env::create_env;
use antex_core::path_utils;
#[cfg(test)]
use antex_core::read_head_for_summary;
use antex_core::sandboxing::SandboxPermissions;
use antex_core::truncate_rollout_after_turn_id;
use antex_core::truncate_rollout_before_turn_id;
use antex_core::windows_sandbox::WindowsSandboxLevelExt;
use antex_core::windows_sandbox::WindowsSandboxSetupMode as CoreWindowsSandboxSetupMode;
use antex_core::windows_sandbox::WindowsSandboxSetupRequest;
use antex_core::windows_sandbox::sandbox_setup_is_complete;
use antex_core_plugins::PluginInstallError as CorePluginInstallError;
use antex_core_plugins::PluginInstallRequest;
use antex_core_plugins::PluginReadRequest;
use antex_core_plugins::PluginUninstallError as CorePluginUninstallError;
use antex_core_plugins::PluginsManager;
use antex_core_plugins::loader::load_plugin_apps;
use antex_core_plugins::manifest::PluginManifestInterface;
use antex_core_plugins::marketplace::MarketplaceError;
use antex_core_plugins::marketplace::MarketplacePluginSource;
use antex_core_plugins::marketplace_add::MarketplaceAddError;
use antex_core_plugins::marketplace_add::MarketplaceAddRequest;
use antex_core_plugins::marketplace_add::add_marketplace as add_marketplace_to_antex_home;
use antex_core_plugins::marketplace_remove::MarketplaceRemoveError;
use antex_core_plugins::marketplace_remove::MarketplaceRemoveRequest as CoreMarketplaceRemoveRequest;
use antex_core_plugins::marketplace_remove::remove_marketplace;
use antex_core_plugins::remote::RemoteMarketplace;
use antex_core_plugins::remote::RemoteMarketplaceSource;
use antex_core_plugins::remote::RemotePluginCatalogError;
use antex_core_plugins::remote::RemotePluginDetail as RemoteCatalogPluginDetail;
use antex_core_plugins::remote::RemotePluginServiceConfig;
use antex_core_plugins::remote::RemotePluginShareContext as RemoteCatalogPluginShareContext;
use antex_core_plugins::remote::RemotePluginShareSummary as RemoteCatalogPluginShareSummary;
use antex_core_plugins::remote::RemotePluginSummary as RemoteCatalogPluginSummary;
use antex_exec_server::EnvironmentManager;
use antex_exec_server::EnvironmentObservedStatus;
use antex_exec_server::LOCAL_ENVIRONMENT_ID;
use antex_exec_server::LOCAL_FS;
use antex_features::FEATURES;
use antex_features::Feature;
use antex_features::Stage;
use antex_feedback::AntexFeedback;
use antex_feedback::FeedbackAttachmentPath;
use antex_feedback::FeedbackUploadOptions;
use antex_git_utils::git_diff_to_remote;
use antex_git_utils::resolve_root_git_project_for_trust;
use antex_login::ANTEX_OPEN_APP_URL;
use antex_login::AntexAuth;
use antex_login::AuthManager;
use antex_login::LoginSuccessPage;
use antex_login::LoginSuccessPageBrand;
use antex_login::ServerOptions as LoginServerOptions;
use antex_login::ShutdownHandle;
use antex_login::complete_device_code_login;
use antex_login::login_with_api_key;
use antex_login::login_with_bedrock_api_key;
use antex_login::oauth_client_id;
use antex_login::request_device_code;
use antex_login::run_login_server;
use antex_mcp::McpRuntimeContext;
use antex_mcp::McpServerStatusSnapshot;
use antex_mcp::McpSnapshotDetail;
use antex_mcp::collect_mcp_server_status_snapshot_with_detail;
use antex_mcp::discover_supported_scopes;
use antex_mcp::read_mcp_resource as read_mcp_resource_without_thread;
use antex_mcp::resolve_oauth_scopes;
use antex_memories_write::clear_memory_roots_contents;
use antex_model_provider::create_model_provider;
use antex_models_manager::collaboration_mode_presets::builtin_collaboration_mode_presets;
use antex_protocol::ThreadId;
use antex_protocol::config_types::CollaborationMode;
use antex_protocol::config_types::ForcedLoginMethod;
use antex_protocol::config_types::Personality;
use antex_protocol::config_types::ReasoningSummary;
use antex_protocol::config_types::TrustLevel;
use antex_protocol::config_types::WindowsSandboxLevel;
use antex_protocol::error::CodexErr;
use antex_protocol::error::Result as AntexResult;
#[cfg(test)]
use antex_protocol::items::TurnItem;
use antex_protocol::models::ResponseItem;
use antex_protocol::openai_models::ReasoningEffort;
use antex_protocol::protocol::AgentStatus;
use antex_protocol::protocol::ConversationAudioParams;
use antex_protocol::protocol::ConversationSpeechParams;
use antex_protocol::protocol::ConversationStartParams;
use antex_protocol::protocol::ConversationStartTransport;
use antex_protocol::protocol::ConversationTextParams;
use antex_protocol::protocol::EnvironmentConfigState;
use antex_protocol::protocol::EventMsg;
#[cfg(test)]
use antex_protocol::protocol::GitInfo as CoreGitInfo;
use antex_protocol::protocol::McpAuthStatus as CoreMcpAuthStatus;
use antex_protocol::protocol::Op;
use antex_protocol::protocol::RealtimeVoicesList;
use antex_protocol::protocol::ReviewDelivery as CoreReviewDelivery;
use antex_protocol::protocol::ReviewRequest;
use antex_protocol::protocol::ReviewTarget as CoreReviewTarget;
use antex_protocol::protocol::SessionConfiguredEvent;
#[cfg(test)]
use antex_protocol::protocol::SessionMetaLine;
use antex_protocol::protocol::TurnEnvironmentSelection;
use antex_protocol::protocol::TurnEnvironmentSelections;
use antex_protocol::protocol::W3cTraceContext;
use antex_protocol::protocol::strip_user_message_prefix;
use antex_protocol::user_input::MAX_USER_INPUT_TEXT_CHARS;
use antex_protocol::user_input::UserInput as CoreInputItem;
use antex_rmcp_client::McpOAuthClientRegistration;
use antex_rmcp_client::StreamableHttpRedirectMode;
use antex_rmcp_client::perform_oauth_login_return_url;
use antex_rollout::InitialHistory;
use antex_rollout::ResumedHistory;
use antex_rollout::RolloutItem;
use antex_rollout::is_persisted_rollout_item;
use antex_rollout::state_db::StateDbHandle;
use antex_rollout::state_db::reconcile_rollout;
use antex_state::ThreadMetadata;
use antex_state::log_db::LogDbLayer;
use antex_thread_store::ArchiveThreadParams as StoreArchiveThreadParams;
use antex_thread_store::ArchiveThreadsParams as StoreArchiveThreadsParams;
use antex_thread_store::ClearableField as StoreClearableField;
use antex_thread_store::DeleteThreadsParams as StoreDeleteThreadsParams;
use antex_thread_store::GitInfoPatch as StoreGitInfoPatch;
use antex_thread_store::ItemSortKey as StoreItemSortKey;
use antex_thread_store::ListItemsParams as StoreListItemsParams;
use antex_thread_store::ListThreadsParams as StoreListThreadsParams;
use antex_thread_store::ListTimelineParams as StoreListTimelineParams;
use antex_thread_store::ListTurnsParams as StoreListTurnsParams;
use antex_thread_store::LoadThreadHistoryParams as StoreLoadThreadHistoryParams;
use antex_thread_store::LocalThreadStore;
use antex_thread_store::ReadThreadByRolloutPathParams as StoreReadThreadByRolloutPathParams;
use antex_thread_store::ReadThreadParams as StoreReadThreadParams;
use antex_thread_store::SearchThreadOccurrencesParams as StoreSearchThreadOccurrencesParams;
use antex_thread_store::SearchThreadsParams as StoreSearchThreadsParams;
use antex_thread_store::SortDirection as StoreSortDirection;
use antex_thread_store::StoredThread;
use antex_thread_store::StoredTurn;
use antex_thread_store::StoredTurnItemsView;
use antex_thread_store::StoredTurnStatus;
use antex_thread_store::ThreadMetadataPatch as StoreThreadMetadataPatch;
use antex_thread_store::ThreadRelationFilter as StoreThreadRelationFilter;
use antex_thread_store::ThreadSortKey as StoreThreadSortKey;
use antex_thread_store::ThreadStore;
use antex_thread_store::ThreadStoreError;
use antex_utils_absolute_path::AbsolutePathBuf;
use antex_utils_pty::DEFAULT_OUTPUT_BYTES_CAP;
use chrono::Duration as ChronoDuration;
use chrono::SecondsFormat;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Error as IoError;
use std::path::Path;
use std::path::PathBuf;
use std::result::Result;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::sync::SemaphorePermit;
use tokio::sync::broadcast;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tokio_util::sync::DropGuard;
use tokio_util::task::TaskTracker;
use toml::Value as TomlValue;
use tracing::Instrument;
use tracing::error;
use tracing::info;
use tracing::warn;
use uuid::Uuid;

#[cfg(test)]
use antex_app_server_protocol::ServerRequest;

mod account_processor;
mod apps_processor;
mod bedrock_auth;
mod catalog_processor;
mod command_exec_processor;
mod config_processor;
mod diagnostics;
mod environment_processor;
mod feedback_doctor_report;
mod feedback_processor;
mod feedback_thread_index;
mod fs_processor;
mod git_processor;
mod initialize_processor;
mod marketplace_processor;
mod mcp_event_stream;
mod mcp_processor;
mod memory_status;
mod persisted_resume_settings;
mod plugins;
mod process_exec_processor;
mod projects;
mod remote_control_processor;
mod search;
mod thread_attachments;
mod thread_enrichment;
mod thread_fork_goal;
mod thread_input;
mod thread_processor;
mod thread_queue_processor;
mod thread_sections;
mod token_usage_replay;
mod turn_processor;
mod windows_sandbox_processor;

pub(crate) use account_processor::AccountRequestProcessor;
pub(crate) use apps_processor::AppsRequestProcessor;
pub(crate) use catalog_processor::CatalogRequestProcessor;
pub(crate) use command_exec_processor::CommandExecRequestProcessor;
pub(crate) use config_processor::ConfigRequestProcessor;
pub(crate) use diagnostics::read_server_diagnostics;
pub(crate) use environment_processor::EnvironmentRequestProcessor;
pub(crate) use feedback_processor::FeedbackRequestProcessor;
pub(crate) use fs_processor::FsRequestProcessor;
pub(crate) use git_processor::GitRequestProcessor;
pub(crate) use initialize_processor::InitializeRequestProcessor;
pub(crate) use marketplace_processor::MarketplaceRequestProcessor;
pub(crate) use mcp_event_stream::McpEventStreamReady;
pub(crate) use mcp_event_stream::McpEventStreams;
pub(crate) use mcp_processor::McpRequestProcessor;
pub(crate) use plugins::PluginRequestProcessor;
pub(crate) use process_exec_processor::ProcessExecRequestProcessor;
pub(crate) use projects::ProjectRequestProcessor;
pub(crate) use remote_control_processor::RemoteControlRequestProcessor;
pub(crate) use search::SearchRequestProcessor;
pub(crate) use thread_goal_processor::ThreadGoalRequestProcessor;
pub(crate) use thread_processor::ThreadRequestProcessor;
pub(crate) use thread_processor::ThreadResumeTarget;
pub(crate) use thread_queue_processor::ThreadQueueRequestProcessor;
pub(crate) use turn_processor::TurnRequestProcessor;
pub(crate) use windows_sandbox_processor::WindowsSandboxRequestProcessor;

use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::filters::compute_source_filters;
use crate::filters::source_kind_matches;
use crate::thread_state::ConnectionCapabilities;
use crate::thread_state::ThreadListenerCommand;
use crate::thread_state::ThreadState;
use crate::thread_state::ThreadStateManager;
use token_usage_replay::restored_token_usage_turn_id;
use token_usage_replay::send_thread_token_usage_update_to_connection;

pub(crate) fn apply_live_thread_settings(
    thread: &mut Thread,
    config_snapshot: &ThreadConfigSnapshot,
) {
    thread.model = Some(config_snapshot.model.clone());
    thread.reasoning_effort = config_snapshot.reasoning_effort.clone();
    thread.environments = Some(
        config_snapshot
            .environment_selections()
            .iter()
            .map(Into::into)
            .collect(),
    );
}

fn resolve_request_cwd(cwd: Option<PathBuf>) -> Result<Option<AbsolutePathBuf>, JSONRPCErrorError> {
    cwd.map(|cwd| {
        AbsolutePathBuf::relative_to_current_dir(path_utils::normalize_for_native_workdir(cwd))
            .map_err(|err| invalid_request(format!("invalid cwd: {err}")))
    })
    .transpose()
}

fn resolve_turn_environment_selections(
    thread_manager: &ThreadManager,
    environments: Option<Vec<TurnEnvironmentParams>>,
) -> Result<Option<Vec<TurnEnvironmentSelection>>, JSONRPCErrorError> {
    let Some(environments) = environments else {
        return Ok(None);
    };
    let mut selections = Vec::with_capacity(environments.len());
    for environment in environments {
        let environment_id = environment.environment_id;
        let cwd = environment
            .cwd
            .to_inferred_path_uri()
            .ok_or_else(|| {
                invalid_request(format!(
                    "invalid cwd for environment `{environment_id}`: path `{}` does not use absolute POSIX or Windows path syntax",
                    environment.cwd
                ))
            })?;
        let workspace_roots = environment
            .runtime_workspace_roots
            .map(|roots| {
                let mut resolved_roots = Vec::new();
                for root in roots {
                    let root = root.to_inferred_path_uri().ok_or_else(|| {
                        invalid_request(format!(
                            "invalid runtime workspace root for environment `{environment_id}`: path `{root}` does not use absolute POSIX or Windows path syntax"
                        ))
                    })?;
                    if !resolved_roots.contains(&root) {
                        resolved_roots.push(root);
                    }
                }
                Ok::<_, JSONRPCErrorError>(resolved_roots)
            })
            .transpose()?
            .unwrap_or_else(|| vec![cwd.clone()]);
        selections.push(TurnEnvironmentSelection {
            environment_id,
            cwd,
            workspace_roots,
            config: EnvironmentConfigState::FromThread,
        });
    }
    thread_manager
        .validate_environment_selections(&selections)
        .map_err(environment_selection_error)?;
    Ok(Some(selections))
}

fn resolve_runtime_workspace_roots(workspace_roots: Vec<AbsolutePathBuf>) -> Vec<AbsolutePathBuf> {
    let mut resolved_roots = Vec::new();
    for root in workspace_roots {
        if !resolved_roots.iter().any(|existing| existing == &root) {
            resolved_roots.push(root);
        }
    }
    resolved_roots
}

mod config_errors;
mod request_errors;
mod thread_delete;
mod thread_goal_processor;
mod thread_lifecycle;
mod thread_resume_redaction;
mod thread_summary;

use self::config_errors::*;
use self::request_errors::*;
use self::thread_goal_processor::api_thread_goal_from_state;
use self::thread_lifecycle::*;
use self::thread_resume_redaction::*;
use self::thread_summary::*;

pub(crate) use self::thread_lifecycle::populate_thread_turns_from_history;
pub(crate) use self::thread_processor::thread_from_stored_thread;
#[cfg(test)]
pub(crate) use self::thread_summary::read_summary_from_rollout;
#[cfg(test)]
pub(crate) use self::thread_summary::summary_to_thread;
pub(crate) use self::thread_summary::thread_settings_from_config_snapshot;

pub(crate) fn build_legacy_api_turns_from_rollout_items(items: &[RolloutItem]) -> Vec<Turn> {
    let mut builder = ThreadHistoryBuilder::new();
    for item in items {
        if is_persisted_rollout_item(item, antex_protocol::protocol::ThreadHistoryMode::Legacy) {
            builder.handle_rollout_item(item);
        }
    }
    builder.finish()
}
