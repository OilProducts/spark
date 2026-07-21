#![forbid(unsafe_code)]

//! Durable Attractor runtime storage for run records, events, journals,
//! checkpoints, manifests, artifacts, and result files.

pub mod artifacts;
pub mod checkpoints;
pub mod codergen;
pub mod context;
pub mod controls;
pub mod error;
pub mod events;
pub mod executor;
pub mod flow_runtime;
pub mod handlers;
pub mod journals;
mod manager_loop;
pub mod paths;
pub mod records;
pub mod results;
pub mod retry;
pub mod routing;
pub mod segments;
pub mod store;
pub mod terminal;
pub mod transcript;
pub mod usage;

pub use artifacts::{
    append_tool_hook_failure, copy_tool_artifact_matches, ensure_run_layout, list_artifacts,
    write_node_artifacts, write_tool_output_log, write_tool_text_artifact, NodeArtifacts,
    ToolHookFailureRecord,
};
pub use checkpoints::{
    normalize_checkpoint_for_write, read_checkpoint, save_checkpoint, CheckpointWriteOptions,
};
pub use codergen::{codergen_events_for_journal, codergen_outcome, RuntimeCodergen};
pub use context::{
    apply_outcome_context_updates, checkpoint_from_context,
    checkpoint_requests_full_fidelity_degrade, clear_runtime_retry_context,
    graph_attr_context_seed, initialize_runtime_context, reset_workflow_outcome_context,
    resolve_runtime_fidelity, resolve_runtime_thread_id, seed_builtin_context,
    set_runtime_fidelity_context, set_runtime_retry_context, CURRENT_NODE_KEY,
    DEFAULT_MAX_RETRIES_KEY, INTERNAL_PIPELINE_RETRY_RUN_ID_KEY, NODE_OUTCOMES_KEY, OUTCOME_KEY,
    PREFERRED_LABEL_KEY, RUNTIME_FIDELITY_KEY, RUNTIME_RETRY_ATTEMPT_KEY,
    RUNTIME_RETRY_FAILURE_REASON_KEY, RUNTIME_RETRY_MAX_ATTEMPTS_KEY, RUNTIME_RETRY_NODE_ID_KEY,
    RUNTIME_THREAD_ID_KEY, WORKFLOW_OUTCOME_KEY, WORKFLOW_OUTCOME_REASON_CODE_KEY,
    WORKFLOW_OUTCOME_REASON_MESSAGE_KEY,
};
pub use controls::disk_execution_control;
pub use controls::{
    ContinueRunRequest, ContinueRunStarted, ControlResult, RetryRunPrepared, RuntimeControlError,
    RuntimeControlStatus, RuntimeControls,
};
pub use error::{Result, RuntimeStorageError};
pub use events::{
    append_event, cancel_requested_event, checkpoint_saved_event,
    child_intervention_requested_event, child_run_completed_event, child_run_started_event,
    cleanup_error_event, human_gate_answered_event, human_gate_pending_event,
    human_intervention_requested_event, interview_completed_event, interview_started_event,
    lifecycle_event, llm_request_completed_event, llm_request_started_event, llm_token_usage_event,
    log_event, parallel_branch_completed_event, parallel_branch_started_event,
    parallel_completed_event, parallel_started_event, pipeline_completed_event,
    pipeline_completed_event_with_reasons, pipeline_failed_event, pipeline_paused_event,
    pipeline_retry_completed_event, pipeline_retry_started_event, pipeline_started_event,
    read_raw_events, run_metadata_event, run_metadata_event_with_graph_paths, runtime_status_event,
    stage_completed_event, stage_failed_event, stage_retrying_event, stage_started_event,
    state_event,
};
pub use executor::{
    prepare_fresh_run, ExecuteRunRequest, ExecutionControlAction, ExecutionStart,
    NodeExecutionRequest, NodeExecutor, PipelineExecutionResult, PipelineExecutor,
    RuntimeNodeError,
};
pub use handlers::{
    AutoApproveInterviewer, ChildInterventionRequest, ChildInterventionResult, ChildRunRequest,
    ChildRunResult, FanInRankingRequest, HandlerRuntime, HumanAnswer, HumanOption, HumanQuestion,
    Interviewer, QueueInterviewer, RuntimeHandlerFn, RuntimeHandlerRunner, HANDLER_CODERGEN,
    HANDLER_CONDITIONAL, HANDLER_EXIT, HANDLER_FAN_IN, HANDLER_MANAGER_LOOP, HANDLER_PARALLEL,
    HANDLER_START, HANDLER_TOOL, HANDLER_WAIT_HUMAN,
};
pub use journals::{
    combined_run_journal_entries, journal_entries_from_events, journal_entry_from_event,
    journal_replay_order, resequence_combined_journal,
};
pub use paths::{RunRootPaths, RuntimePaths};
pub use records::{
    mark_record_cancel_requested, mark_record_canceled, mark_record_paused,
    mark_record_retry_started, normalize_run_status, read_run_record, write_run_record,
};
pub use results::{
    failed_run_result, materialize_run_result, read_materialized_run_result,
    result_summary_request, write_run_result, ResultSummaryAttempt, DEFAULT_RESULT_SUMMARY_PROMPT,
};
pub use retry::{
    coerce_retry_exhausted_outcome, max_retries_for_node, retry_policy_for_node,
    should_retry_attempt, should_retry_outcome, BackoffConfig, RetryPolicy, RetryPreset,
};
pub use routing::{
    is_conditional_node, is_exit_node, outgoing_routing_edges, resolve_start_node,
    routing_outcome_for_node, routing_outcome_for_node_with_prior, select_next_node,
    select_next_node_with_prior, NextNodeSelection,
};
pub use segments::{project_run_segments, RunSegmentProjection};
pub use store::{CreateRunRequest, RunArtifactFile, RunBundle, RunEventObserver, RunMeta, RunStore};
pub use terminal::{
    check_goal_gates, invalid_workflow_outcome_reason, is_goal_gate_node,
    resolve_failure_retry_target, resolve_goal_gate_retry_target,
    resolve_terminal_workflow_outcome, GoalGateCheck, TerminalWorkflowOutcome,
    GOAL_GATE_NO_RETRY_TARGET_REASON, GOAL_GATE_UNSATISFIED_OUTCOME_CODE,
};
pub use transcript::project_run_transcript;
