#![forbid(unsafe_code)]

//! Rust-owned adapter contracts for Spark agent and codergen migration.

pub mod agent;
pub mod apply_patch;
pub mod codergen;
pub mod config;
pub mod environment;
pub mod events;
pub mod history;
pub mod llm_backend;
pub mod local_environment;
pub mod profiles;
pub mod session;
pub mod status_envelope;
pub mod tools;
pub mod truncation;

pub use agent::{
    AgentRawLogLine, AgentThreadResumeFailure, AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
};
pub use codergen::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenError,
    CodergenExecution, CodergenHandler, CodergenRequest,
};
pub use config::SessionConfig;
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
pub use session::{LlmClientHandle, Session, SessionState};
pub use status_envelope::{
    build_contract_repair_prompt, build_status_envelope_context_updates_contract_text,
    build_status_envelope_prompt_appendix, coerce_structured_text_outcome,
    contract_failure_outcome, extract_structured_outcome_payload, status_envelope_allowed_keys,
    validate_write_contract_violation, ModeledOutcomeParseResult, PlainTextParseResult,
    StructuredContractViolation, StructuredTextOutcome,
};
pub use tools::{
    RegisteredTool, ToolDefinition, ToolDispatchContext, ToolDispatchEvent, ToolExecution,
    ToolExecutionOutput, ToolRegistry, ToolTruncation,
};
pub use truncation::{truncate_lines, truncate_output, truncate_tool_output};
