#![forbid(unsafe_code)]

//! Core Attractor contracts for the Rust rewrite.
//!
//! This crate reserves graph, context, routing, outcome, and runtime identifier
//! boundaries. DOT parsing, validation, handlers, runtime scheduling, and HTTP
//! APIs remain owned by later milestones.

/// Context namespace and launch context boundary.
pub mod context;

/// Runtime condition expression evaluator.
pub mod conditions;

/// Authored context read/write contract boundary.
pub mod context_contracts;

/// Core error types.
pub mod error;

/// FlowDefinition workflow model (JSON interchange; flows are stored as
/// per-node markdown directories by `attractor-dsl`).
pub mod flow_definition;

/// Graph model and identifier boundary.
pub mod graph;

/// Terminal outcome boundary.
pub mod outcomes;

/// Runtime identifier boundary.
pub mod runtime;

pub use conditions::{evaluate_condition, normalize_condition_literal, split_condition_clauses};
pub use context::{
    apply_launch_context, validate_context_key, validate_launch_context, AttractorContext,
    ContextMap, ContextValue, LaunchContext, ALLOWED_CONTEXT_PREFIXES, RUNTIME_SESSIONS_KEY,
    RUNTIME_UNAVAILABLE_ROUTES_KEY,
};
pub use context_contracts::{
    normalize_context_read_key, normalize_context_update_key, normalize_context_updates,
    parse_context_read_contract, parse_context_write_contract, resolve_context_read_contract,
    resolve_context_write_contract, validate_context_updates_against_contract,
    validate_context_updates_against_contract_with_exemptions, ContextReadContract,
    ContextWriteContract, ContextWriteContractViolation,
};
pub use error::{AttractorCoreError, Result};
pub use flow_definition::{
    decode_case_key, dot_edge, flow_definition_schema_value, is_legacy_routing_field,
    normalize_flow_ref, schema_version_diagnostic, switch_decision, valid_decision_id,
    ExecutionConfig, ExitOutcome, ExitResult, FlowDefaults, FlowDefinition, FlowDefinitionError,
    FlowDiagnostic, FlowEdge, FlowEdgeKind, FlowExecutionLock, FlowInput, FlowMetadata, FlowNode,
    FlowRoute, ManagerLoopConfig, NodeConfig, NodeContracts, NodeKind, NodeRuntimeConfig,
    RecoveryPolicy, RetryConfig, SwitchNumber, SwitchScalar, UiConfig, DECISION_ENVELOPE,
    FLOW_SCHEMA_VERSION, NEXT_DECISION,
};
pub use graph::{
    attr_bool, attr_i64, attr_string, attr_text, dot_value_text, dot_value_to_json,
    node_has_explicit_attr, node_shape, routing_edge_from_dot_edge, AttributeMap, AttributeValue,
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral, EdgeAttribute, EdgeId, EdgeRef, FlowId, FlowName,
    GraphAttribute, GraphId, GraphRef, NodeAttribute, NodeId, NodeRef, RoutingEdge,
};
pub use outcomes::{BranchTerminal, FailureKind, Outcome, OutcomeStatus};
pub use runtime::{
    ArtifactInfo, CheckpointId, CheckpointState, JournalEntry, JournalSequence, RawRuntimeEvent,
    RunExecutionLock, RunId, RunManifest, RunRecord, RunResult, RunStatus, RuntimeNodeState,
    StageId, RUN_STATUS_UNREADABLE,
};
