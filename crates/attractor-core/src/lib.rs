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

/// Graph model and identifier boundary.
pub mod graph;

/// Terminal outcome boundary.
pub mod outcomes;

/// Routing input and next-id boundary.
pub mod routing;

/// Runtime identifier boundary.
pub mod runtime;

pub use conditions::{evaluate_condition, normalize_condition_literal, split_condition_clauses};
pub use context::{
    apply_launch_context, validate_context_key, validate_launch_context, AttractorContext,
    ContextMap, ContextValue, LaunchContext, ALLOWED_CONTEXT_PREFIXES,
};
pub use context_contracts::{
    normalize_context_read_key, normalize_context_update_key, normalize_context_updates,
    parse_context_read_contract, parse_context_write_contract, resolve_context_read_contract,
    resolve_context_write_contract, validate_context_updates_against_contract,
    validate_context_updates_against_contract_with_exemptions, ContextReadContract,
    ContextWriteContract, ContextWriteContractViolation,
};
pub use error::{AttractorCoreError, Result};
pub use graph::{
    attr_bool, attr_i64, attr_string, attr_text, dot_value_text, dot_value_to_json,
    node_has_explicit_attr, node_shape, routing_edge_from_dot_edge, AttributeMap, AttributeValue,
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral, EdgeAttribute, EdgeId, EdgeRef, FlowId, FlowName,
    GraphAttribute, GraphId, GraphRef, NodeAttribute, NodeId, NodeRef, RoutingEdge,
};
pub use outcomes::{FailureKind, Outcome, OutcomePayload, OutcomeStatus};
pub use routing::{
    best_edge_by_weight_then_lexical, is_exact_outcome_fail_condition, normalize_label,
    select_failure_route_edge_with_context, select_next_edge,
    select_next_edge_with_condition_results, select_next_edge_with_context, NextNodeSuggestion,
    RoutingDecision, RoutingInput, SuggestedNextIds,
};
pub use runtime::{
    ArtifactInfo, CheckpointId, CheckpointState, JournalEntry, JournalSequence, RawRuntimeEvent,
    RunExecutionLock, RunId, RunManifest, RunRecord, RunResult, RunStatus, RuntimeNodeState,
    StageId,
};
