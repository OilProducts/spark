#![forbid(unsafe_code)]

//! Product-owned Workspace services for Rust HTTP compatibility milestones.

pub mod conversations;
pub mod errors;
pub mod flows;
pub mod live;
pub mod models;
pub mod projects;
pub mod settings;
pub mod triggers;
pub mod workflow_log;

pub use conversations::{
    ConversationDeleteResponse, ConversationRequestUserInputAnswerRequest,
    ConversationSettingsUpdate, ConversationSummary, ConversationTurnRequest,
    FlowRunRequestCreateByHandleRequest, FlowRunRequestCreateResponse, FlowRunRequestReviewRequest,
    PreparedConversationTurn, ProposedPlanReviewRequest, RunContinueRequest, RunLaunchRequest,
    RunRetryRequest, WorkspaceConversationService,
};
pub use errors::{WorkspaceError, WorkspaceResult};
pub use flows::{
    WorkspaceFlowDescription, WorkspaceFlowFeatures, WorkspaceFlowLaunchPolicyResponse,
    WorkspaceFlowLaunchPolicyUpdate, WorkspaceFlowRaw, WorkspaceFlowService, WorkspaceFlowSummary,
};
pub use live::{LiveCursor, LiveEnvelope, LiveQuery, LiveResource, RawLiveQuery};
pub use models::{chat_models, public_unified_chat_models};
pub use projects::{
    BrowseEntry, BrowseResponse, ProjectMetadata, ProjectRegistrationRequest, ProjectStateUpdate,
    WorkspaceProjectService,
};
pub use settings::workspace_settings;
pub use spark_triggers::{
    SerializedTrigger, TriggerActivationOutcome, TriggerCreateRequest, TriggerDeleteResponse,
    TriggerUpdateRequest, WebhookDispatchOutcome, WebhookHandleRequest, WebhookHandleResponse,
};
pub use triggers::WorkspaceTriggerService;
pub use workflow_log::{
    project_run_milestones, read_workflow_log_tail, workflow_log_envelope,
    workflow_log_tail_envelopes, WorkflowLogEntry, WORKFLOW_LOG_TAIL_LIMIT,
};
