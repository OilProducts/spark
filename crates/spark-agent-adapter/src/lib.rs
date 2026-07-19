#![forbid(unsafe_code)]

//! Rust-owned adapter contracts for Spark agent and codergen migration.

pub mod agent;
pub mod apply_patch;
pub mod boundary_cli;
pub mod claude_code;
pub mod codergen;
pub mod codex_app_server;
pub mod config;
pub mod context;
pub mod environment;
pub mod events;
pub mod history;
mod initial_context;
pub mod llm_backend;
pub mod local_environment;
pub mod profiles;
pub mod project_docs;
pub mod session;
pub mod status_envelope;
pub mod subagents;
pub mod tools;
pub mod truncation;

pub use agent::{
    AgentError, AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnBackend, AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
};
pub use claude_code::{
    claude_code_models_from_list_result, list_available_claude_code_models,
    usage_from_claude_code_usage_payload, ClaudeCodeBackend, ClaudeCodeError,
    ClaudeCodeModelMetadata, CLAUDE_CODE_BACKEND,
};
pub use codergen::{
    ActiveCodergenSession, ActiveCodergenSessionGuard, CodergenBackend, CodergenBackendOutput,
    CodergenBackendRequest, CodergenBackendResponse, CodergenChildInterventionRequest,
    CodergenChildInterventionResult, CodergenError, CodergenEvent, CodergenEventSink,
    CodergenExecution, CodergenExecutionMode, CodergenHandler, CodergenRequest,
    CodergenRuntimeMode, CodergenSessionInterventionBroker,
};
pub use codex_app_server::{
    build_codex_runtime_environment, codex_models_from_list_result, configure_codex_api_base_url,
    configure_codex_spark_home, list_available_codex_models, parse_jsonrpc_line,
    process_codex_app_server_message, CodexAppServerBackend, CodexAppServerClient,
    CodexAppServerError, CodexAppServerTurnState, CodexModelMetadata,
};
pub use config::SessionConfig;
pub use context::{
    build_environment_context_block, build_provider_base_instructions, build_system_prompt,
    build_system_prompt_from_snapshot, build_system_prompt_with_user_overrides,
    build_tool_descriptions, context_usage_warning_payload, estimate_context_usage,
    snapshot_environment_context, ContextUsageEstimate, EnvironmentContext,
    CONTEXT_WARNING_THRESHOLD_RATIO,
};
pub use environment::{
    CommandOptions, DirEntry, EnvironmentError, EnvironmentInheritancePolicy, EnvironmentResult,
    ExecResult, ExecutionEnvironment, ExecutionEnvironmentBackend, GrepOptions,
};
pub use events::{EventKind, SessionEvent};
pub use history::{
    history_to_messages, AssistantTurn, HistoryTurn, SteeringTurn, SystemTurn, ToolResultsTurn,
    TurnContent, UserTurn,
};
pub use llm_backend::{RustLlmAgentTurnBackend, RustLlmCodergenBackend};
pub use local_environment::LocalExecutionEnvironment;
pub use profiles::{
    create_anthropic_profile, create_gemini_profile, create_gemini_profile_with_options,
    create_openai_compatible_profile, create_openai_profile, create_provider_profile,
    normalize_provider_selector, GeminiProfileOptions, NormalizedProviderSelector, ProviderFamily,
    ProviderProfile,
};
pub use project_docs::{
    discover_project_documents, discover_project_documents_with_budget, load_project_documents,
    render_project_documents, ProjectDocument, ProjectDocuments, PROJECT_INSTRUCTION_BYTE_BUDGET,
    PROJECT_INSTRUCTION_TRUNCATION_MARKER,
};
pub use session::{
    detect_loop, tool_call_signature, LlmClientHandle, Session, SessionAbortHandle, SessionState,
    ToolCallSignature, LOOP_DETECTION_WARNING,
};
pub use status_envelope::{
    build_contract_repair_prompt, build_status_envelope_context_updates_contract_text,
    build_status_envelope_prompt_appendix, coerce_structured_text_outcome,
    contract_failure_outcome, extract_structured_outcome_payload, status_envelope_allowed_keys,
    validate_write_contract_violation, ModeledOutcomeParseResult, PlainTextParseResult,
    StructuredContractViolation, StructuredTextOutcome,
};
pub use subagents::{
    create_child_session, ChildSessionOptions, SubAgentError, SubAgentHandle, SubAgentLimitError,
    SubAgentLookupError, SubAgentResult, SubAgentRuntimeResult, SubAgentStateError, SubAgentStatus,
    SubAgentWorkingDirectoryError,
};
pub use tools::{
    RegisteredTool, ToolDefinition, ToolDispatchContext, ToolDispatchEvent, ToolExecution,
    ToolExecutionOutput, ToolHostControls, ToolRegistry, ToolTruncation,
};
pub use truncation::{truncate_lines, truncate_output, truncate_tool_output};
