#![forbid(unsafe_code)]

//! Rust-owned adapter contracts for Spark agent and codergen migration.

pub mod agent;
pub mod codergen;
pub mod llm_backend;
pub mod status_envelope;

pub use agent::{
    AgentRawLogLine, AgentThreadResumeFailure, AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
};
pub use codergen::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenError,
    CodergenExecution, CodergenHandler, CodergenRequest,
};
pub use llm_backend::{RustLlmAgentTurnBackend, RustLlmCodergenBackend};
pub use status_envelope::{
    build_contract_repair_prompt, build_status_envelope_context_updates_contract_text,
    build_status_envelope_prompt_appendix, coerce_structured_text_outcome,
    contract_failure_outcome, extract_structured_outcome_payload, status_envelope_allowed_keys,
    validate_write_contract_violation, ModeledOutcomeParseResult, PlainTextParseResult,
    StructuredContractViolation, StructuredTextOutcome,
};
