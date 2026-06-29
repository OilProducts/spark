#![forbid(unsafe_code)]

//! Rust-owned adapter contracts for Spark agent and codergen migration.

pub mod agent;
pub mod codergen;
pub mod config;
pub mod events;
pub mod history;
pub mod llm_backend;
pub mod profiles;
pub mod session;
pub mod status_envelope;

pub use agent::{
    AgentRawLogLine, AgentThreadResumeFailure, AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
};
pub use codergen::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenError,
    CodergenExecution, CodergenHandler, CodergenRequest,
};
pub use config::SessionConfig;
pub use events::{EventKind, SessionEvent};
pub use history::{
    history_to_messages, AssistantTurn, HistoryTurn, SteeringTurn, SystemTurn, ToolResultsTurn,
    TurnContent, UserTurn,
};
pub use llm_backend::{RustLlmAgentTurnBackend, RustLlmCodergenBackend};
pub use profiles::ProviderProfile;
pub use session::{ExecutionEnvironment, LlmClientHandle, Session, SessionState};
pub use status_envelope::{
    build_contract_repair_prompt, build_status_envelope_context_updates_contract_text,
    build_status_envelope_prompt_appendix, coerce_structured_text_outcome,
    contract_failure_outcome, extract_structured_outcome_payload, status_envelope_allowed_keys,
    validate_write_contract_violation, ModeledOutcomeParseResult, PlainTextParseResult,
    StructuredContractViolation, StructuredTextOutcome,
};
