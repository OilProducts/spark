use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use attractor_core::{
    AttractorContext, CheckpointState, ContextMap, DotGraph, DotNode, FailureKind, LaunchContext,
    Outcome, OutcomeStatus, RoutingEdge, RunManifest, RunRecord, RunResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::artifacts::NodeArtifacts;
use crate::checkpoints::CheckpointWriteOptions;
use crate::context::{
    apply_outcome_context_updates, checkpoint_from_context,
    checkpoint_requests_full_fidelity_degrade, clear_runtime_retry_context,
    graph_attr_context_seed, initialize_runtime_context, reset_workflow_outcome_context,
    seed_builtin_context, set_runtime_fidelity_context, set_runtime_retry_context,
    CURRENT_NODE_KEY, OUTCOME_KEY, PREFERRED_LABEL_KEY,
};
use crate::error::{Result, RuntimeStorageError};
use crate::events::{
    checkpoint_saved_event, cleanup_error_event, log_event, pipeline_completed_event_with_reasons,
    pipeline_failed_event, pipeline_paused_event, pipeline_started_event, runtime_status_event,
    stage_completed_event, stage_failed_event, stage_retrying_event, stage_started_event,
};
use crate::records::{mark_record_canceled, mark_record_paused, write_run_record};
use crate::results::write_run_result;
use crate::retry::{coerce_retry_exhausted_outcome, retry_policy_for_node, should_retry_attempt};
use crate::routing::{
    is_conditional_node, is_exit_node, outgoing_routing_edges, resolve_start_node,
    select_next_node_with_prior, NextNodeSelection,
};
use crate::store::{CreateRunRequest, RunStore};
use crate::terminal::{
    check_goal_gates, invalid_workflow_outcome_reason, is_goal_gate_node,
    resolve_failure_retry_target, resolve_goal_gate_retry_target,
    resolve_terminal_workflow_outcome, GOAL_GATE_NO_RETRY_TARGET_REASON,
    GOAL_GATE_UNSATISFIED_OUTCOME_CODE,
};

#[derive(Debug, Clone)]
pub struct ExecuteRunRequest {
    pub store: RunStore,
    pub record: RunRecord,
    pub graph: DotGraph,
    pub graph_source: Option<String>,
    pub graph_dot: Option<String>,
    pub launch_context: LaunchContext,
    pub runtime_context: ContextMap,
    pub max_steps: Option<usize>,
    pub start: ExecutionStart,
}

#[derive(Debug, Clone, Default)]
pub enum ExecutionStart {
    #[default]
    Fresh,
    Resume {
        paths: crate::paths::RunRootPaths,
        checkpoint: CheckpointState,
    },
    FromCheckpoint {
        start_node: String,
        checkpoint: CheckpointState,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeExecutionRequest {
    pub node_id: String,
    pub stage_index: u64,
    pub context: ContextMap,
    pub prompt: String,
    pub node: DotNode,
    pub graph: DotGraph,
    pub outgoing_edges: Vec<RoutingEdge>,
    pub run_paths: Option<crate::paths::RunRootPaths>,
    pub run_workdir: PathBuf,
    pub run_id: String,
}

pub trait NodeExecutor {
    fn execute(
        &mut self,
        request: NodeExecutionRequest,
    ) -> std::result::Result<Outcome, RuntimeNodeError>;

    fn take_cleanup_error(&mut self) -> Option<String> {
        None
    }
}

impl<F> NodeExecutor for F
where
    F: FnMut(NodeExecutionRequest) -> std::result::Result<Outcome, RuntimeNodeError>,
{
    fn execute(
        &mut self,
        request: NodeExecutionRequest,
    ) -> std::result::Result<Outcome, RuntimeNodeError> {
        self(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeNodeError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,
}

impl RuntimeNodeError {
    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: Some(true),
            failure_kind: Some(FailureKind::Runtime),
        }
    }

    pub fn terminal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: Some(false),
            failure_kind: Some(FailureKind::Runtime),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineExecutionResult {
    pub status: String,
    pub current_node: String,
    #[serde(default)]
    pub completed_nodes: Vec<String>,
    #[serde(default)]
    pub context: ContextMap,
    #[serde(default)]
    pub node_outcomes: BTreeMap<String, Outcome>,
    #[serde(default)]
    pub route_trace: Vec<String>,
    #[serde(default)]
    pub failure_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_reason_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionControlAction {
    Pause,
    Cancel,
}

pub struct PipelineExecutor<E> {
    node_executor: E,
    control: Option<Box<dyn FnMut() -> Option<ExecutionControlAction>>>,
}

impl<E> PipelineExecutor<E>
where
    E: NodeExecutor,
{
    pub fn new(node_executor: E) -> Self {
        Self {
            node_executor,
            control: None,
        }
    }

    pub fn with_control(
        node_executor: E,
        control: impl FnMut() -> Option<ExecutionControlAction> + 'static,
    ) -> Self {
        Self {
            node_executor,
            control: Some(Box::new(control)),
        }
    }

    pub fn execute(&mut self, request: ExecuteRunRequest) -> Result<PipelineExecutionResult> {
        let ExecuteRunRequest {
            store,
            mut record,
            graph,
            graph_source,
            graph_dot,
            launch_context,
            runtime_context,
            max_steps,
            start,
        } = request;
        let start_node = resolve_start_node(&graph)?;
        let run_id = ensure_run_record_defaults(&mut record);
        let mut context;
        let mut completed_nodes;
        let mut retry_counts;
        let mut node_outcomes = BTreeMap::<String, Outcome>::new();
        let mut status_transitions = BTreeMap::<String, Vec<String>>::new();
        let mut artifact_node_ids = BTreeSet::<String>::new();
        let mut route_trace;
        let mut current_node;
        let mut incoming_edge: Option<RoutingEdge> = None;
        let mut degrade_resume_fidelity_once = false;

        let (paths, resumed) = match start {
            ExecutionStart::Fresh => {
                context = initialize_runtime_context(&graph, &start_node, &launch_context)?;
                context.apply_updates(&runtime_context)?;
                seed_execution_record_context(&mut context, &record)?;
                reset_workflow_outcome_context(&mut context)?;
                clear_runtime_retry_context(&mut context)?;
                completed_nodes = Vec::new();
                retry_counts = BTreeMap::new();
                current_node = start_node.clone();
                route_trace = vec![current_node.clone()];
                let initial_checkpoint = checkpoint_from_context(
                    &current_node,
                    completed_nodes.clone(),
                    &context,
                    retry_counts.clone(),
                );
                let paths = store.create_run(CreateRunRequest {
                    record: record.clone(),
                    checkpoint: Some(initial_checkpoint),
                    manifest: Some(RunManifest {
                        goal: graph.goal(),
                        graph_id: graph.graph_id.clone(),
                        start_node: start_node.clone(),
                        started_at: record.started_at.clone(),
                        extra: BTreeMap::new(),
                    }),
                    graph_source,
                    graph_dot,
                })?;
                (paths, false)
            }
            ExecutionStart::Resume { paths, checkpoint } => {
                paths.ensure_exists()?;
                context = context_from_checkpoint(&graph, &checkpoint, &checkpoint.current_node)?;
                completed_nodes = checkpoint
                    .completed_nodes
                    .iter()
                    .filter(|node_id| graph.nodes.contains_key(node_id.as_str()))
                    .cloned()
                    .collect();
                retry_counts = checkpoint
                    .retry_counts
                    .iter()
                    .filter(|(node_id, _)| graph.nodes.contains_key(node_id.as_str()))
                    .map(|(node_id, count)| (node_id.clone(), *count))
                    .collect();
                current_node = if graph.nodes.contains_key(&checkpoint.current_node) {
                    checkpoint.current_node.clone()
                } else {
                    start_node.clone()
                };
                route_trace = vec![current_node.clone()];
                if completed_nodes
                    .iter()
                    .any(|node_id| node_id == &current_node)
                {
                    let resume_outcome = resume_outcome_for_node(&current_node, &context);
                    let selection = select_next_node_with_prior(
                        &graph,
                        &current_node,
                        &resume_outcome,
                        &context,
                        None,
                        "",
                    )?;
                    let Some(next_node) = selection.selected_node.clone() else {
                        return terminal_checkpoint_result(&checkpoint, &graph);
                    };
                    current_node = next_node;
                    incoming_edge = selection.selected_edge.clone();
                    route_trace.push(current_node.clone());
                    context.set(CURRENT_NODE_KEY, json!(current_node.clone()))?;
                }
                degrade_resume_fidelity_once =
                    checkpoint_requests_full_fidelity_degrade(&checkpoint);
                (paths, true)
            }
            ExecutionStart::FromCheckpoint {
                start_node,
                checkpoint,
            } => {
                if !graph.nodes.contains_key(&start_node) {
                    return Err(RuntimeStorageError::InvalidRuntimeGraph {
                        reason: format!("Unknown runtime node: {start_node}"),
                    });
                }
                context = context_from_checkpoint(&graph, &checkpoint, &start_node)?;
                context.apply_updates(&runtime_context)?;
                seed_execution_record_context(&mut context, &record)?;
                completed_nodes = Vec::new();
                retry_counts = checkpoint.retry_counts.clone();
                current_node = start_node.clone();
                route_trace = vec![current_node.clone()];
                let initial_checkpoint = checkpoint_from_context(
                    &current_node,
                    completed_nodes.clone(),
                    &context,
                    retry_counts.clone(),
                );
                let paths = store.create_run(CreateRunRequest {
                    record: record.clone(),
                    checkpoint: Some(initial_checkpoint),
                    manifest: Some(RunManifest {
                        goal: graph.goal(),
                        graph_id: graph.graph_id.clone(),
                        start_node: current_node.clone(),
                        started_at: record.started_at.clone(),
                        extra: BTreeMap::new(),
                    }),
                    graph_source,
                    graph_dot,
                })?;
                (paths, false)
            }
        };
        store.append_event(
            &paths,
            pipeline_started_event(&run_id, &graph.graph_id, &current_node, resumed),
        )?;
        if !resumed {
            save_checkpoint_event(
                &store,
                &paths,
                &run_id,
                &current_node,
                &completed_nodes,
                &context,
                &retry_counts,
            )?;
        }

        let mut steps = 0usize;
        loop {
            context.set(CURRENT_NODE_KEY, json!(current_node.clone()))?;
            if let Some(action) = self.poll_control() {
                return match action {
                    ExecutionControlAction::Pause => finalize_paused(
                        &store,
                        &paths,
                        &run_id,
                        &mut record,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                        &node_outcomes,
                        &route_trace,
                    ),
                    ExecutionControlAction::Cancel => finalize_canceled(
                        &store,
                        &paths,
                        &run_id,
                        &mut record,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                        &node_outcomes,
                        &route_trace,
                        "aborted_by_user",
                    ),
                };
            }
            if is_exit_node(&graph, &current_node) {
                let gate_check = check_goal_gates(&graph, &context, &completed_nodes);
                if !gate_check.satisfied {
                    if let Some(failed_gate_node) = gate_check.failed_node_id.as_deref() {
                        if let Some(retry_target) =
                            resolve_goal_gate_retry_target(&graph, failed_gate_node)
                        {
                            current_node = retry_target;
                            incoming_edge = None;
                            context.set(CURRENT_NODE_KEY, json!(current_node.clone()))?;
                            route_trace.push(current_node.clone());
                            save_checkpoint_event(
                                &store,
                                &paths,
                                &run_id,
                                &current_node,
                                &completed_nodes,
                                &context,
                                &retry_counts,
                            )?;
                            continue;
                        }
                    }
                    let Some(terminal) = resolve_terminal_workflow_outcome(
                        &context,
                        "failure",
                        Some(GOAL_GATE_UNSATISFIED_OUTCOME_CODE),
                        Some(GOAL_GATE_NO_RETRY_TARGET_REASON),
                    ) else {
                        let reason = invalid_workflow_outcome_reason(&context);
                        return finalize_failed(
                            &store,
                            &paths,
                            &run_id,
                            &mut record,
                            &graph,
                            &current_node,
                            &completed_nodes,
                            &context,
                            &retry_counts,
                            &node_outcomes,
                            &route_trace,
                            reason,
                            artifact_node_ids.len(),
                        );
                    };
                    return finalize_completed(
                        &store,
                        &paths,
                        &run_id,
                        &mut record,
                        &graph,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                        &node_outcomes,
                        &route_trace,
                        &terminal.outcome,
                        terminal.outcome_reason_code,
                        terminal.outcome_reason_message,
                        artifact_node_ids.len(),
                    );
                }

                let exit_node = graph.nodes.get(&current_node).ok_or_else(|| {
                    RuntimeStorageError::InvalidRuntimeGraph {
                        reason: format!("Unknown runtime node: {current_node}"),
                    }
                })?;
                write_stage_artifacts(
                    &store,
                    &paths,
                    &current_node,
                    &prompt_for_node(exit_node),
                    &Outcome::new(OutcomeStatus::Success),
                    &mut status_transitions,
                    &mut artifact_node_ids,
                )?;

                let Some(terminal) =
                    resolve_terminal_workflow_outcome(&context, "success", None, None)
                else {
                    let reason = invalid_workflow_outcome_reason(&context);
                    return finalize_failed(
                        &store,
                        &paths,
                        &run_id,
                        &mut record,
                        &graph,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                        &node_outcomes,
                        &route_trace,
                        reason,
                        artifact_node_ids.len(),
                    );
                };
                return finalize_completed(
                    &store,
                    &paths,
                    &run_id,
                    &mut record,
                    &graph,
                    &current_node,
                    &completed_nodes,
                    &context,
                    &retry_counts,
                    &node_outcomes,
                    &route_trace,
                    &terminal.outcome,
                    terminal.outcome_reason_code,
                    terminal.outcome_reason_message,
                    artifact_node_ids.len(),
                );
            }

            let node = graph.nodes.get(&current_node).ok_or_else(|| {
                RuntimeStorageError::InvalidRuntimeGraph {
                    reason: format!("Unknown runtime node: {current_node}"),
                }
            })?;
            if context.get("internal.run_id").is_none() {
                context.set("internal.run_id", json!(run_id.clone()))?;
            }
            if context.get("internal.run_workdir").is_none() {
                context.set(
                    "internal.run_workdir",
                    json!(record.working_directory.clone()),
                )?;
            }
            let forced_fidelity = if degrade_resume_fidelity_once {
                degrade_resume_fidelity_once = false;
                Some("summary:high")
            } else {
                None
            };
            set_runtime_fidelity_context(
                &graph,
                &current_node,
                incoming_edge.as_ref(),
                &mut context,
                forced_fidelity,
            )?;
            let stage_index = completed_nodes.len() as u64;
            let prompt = prompt_for_node(node);
            let prior_status = context
                .get(OUTCOME_KEY)
                .and_then(|value| value.as_str())
                .and_then(|value| value.trim().parse::<OutcomeStatus>().ok());
            let prior_preferred_label = context
                .get(PREFERRED_LABEL_KEY)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            store.append_event(
                &paths,
                stage_started_event(&run_id, stage_index, &current_node),
            )?;
            let outgoing_edges = outgoing_routing_edges(&graph, &current_node)?;
            let run_workdir = context
                .get("internal.run_workdir")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&record.working_directory));
            let execution_request = NodeExecutionRequest {
                node_id: current_node.clone(),
                stage_index,
                context: context.snapshot(),
                prompt: prompt.clone(),
                node: node.clone(),
                graph: graph.clone(),
                outgoing_edges,
                run_paths: Some(paths.clone()),
                run_workdir,
                run_id: run_id.clone(),
            };

            let raw_outcome = match catch_unwind(AssertUnwindSafe(|| {
                self.node_executor.execute(execution_request)
            })) {
                Ok(result) => result.unwrap_or_else(outcome_from_node_error),
                Err(payload) => outcome_from_node_error(RuntimeNodeError::runtime(
                    panic_payload_message(payload),
                )),
            };
            if let Some(cleanup_error) = self.node_executor.take_cleanup_error() {
                record.cleanup_error = Some(cleanup_error.clone());
                write_run_record(&paths, &record)?;
                store.append_event(&paths, cleanup_error_event(&run_id, cleanup_error))?;
            }
            let mut outcome = apply_outcome_context_updates(
                &current_node,
                &node.attrs,
                &mut context,
                &raw_outcome,
            )?;
            node_outcomes.insert(current_node.clone(), outcome.clone());
            write_stage_artifacts(
                &store,
                &paths,
                &current_node,
                &prompt,
                &outcome,
                &mut status_transitions,
                &mut artifact_node_ids,
            )?;

            let policy = retry_policy_for_node(&graph, &current_node);
            let max_retries = policy.max_attempts.saturating_sub(1);
            let retries_so_far = retry_counts.get(&current_node).copied().unwrap_or(0);
            if should_retry_attempt(&outcome, retries_so_far, &policy) {
                let next_attempt = retries_so_far.saturating_add(1);
                retry_counts.insert(current_node.clone(), next_attempt);
                set_runtime_retry_context(
                    &mut context,
                    &current_node,
                    next_attempt,
                    policy.max_attempts,
                    &stage_failure_reason(&outcome),
                )?;
                if outcome.status == OutcomeStatus::Fail {
                    store.append_event(
                        &paths,
                        stage_failed_event(
                            &run_id,
                            stage_index,
                            &current_node,
                            stage_failure_reason(&outcome),
                            true,
                            Some(next_attempt),
                        ),
                    )?;
                }
                store.append_event(
                    &paths,
                    stage_retrying_event(
                        &run_id,
                        stage_index,
                        &current_node,
                        next_attempt,
                        policy.backoff.delay_for_attempt(next_attempt),
                    ),
                )?;
                save_checkpoint_event(
                    &store,
                    &paths,
                    &run_id,
                    &current_node,
                    &completed_nodes,
                    &context,
                    &retry_counts,
                )?;
                continue;
            }

            let coerced = coerce_retry_exhausted_outcome(
                &graph,
                &current_node,
                &outcome,
                retries_so_far,
                max_retries,
            );
            if coerced != outcome {
                outcome = apply_outcome_context_updates(
                    &current_node,
                    &node.attrs,
                    &mut context,
                    &coerced,
                )?;
                node_outcomes.insert(current_node.clone(), outcome.clone());
                write_stage_artifacts(
                    &store,
                    &paths,
                    &current_node,
                    &prompt,
                    &outcome,
                    &mut status_transitions,
                    &mut artifact_node_ids,
                )?;
            }

            clear_runtime_retry_context(&mut context)?;
            if matches!(
                outcome.status,
                OutcomeStatus::Success | OutcomeStatus::PartialSuccess
            ) {
                retry_counts.remove(&current_node);
            }

            if outcome.status == OutcomeStatus::Fail {
                store.append_event(
                    &paths,
                    stage_failed_event(
                        &run_id,
                        stage_index,
                        &current_node,
                        stage_failure_reason(&outcome),
                        false,
                        None,
                    ),
                )?;
            } else {
                store.append_event(
                    &paths,
                    stage_completed_event(
                        &run_id,
                        stage_index,
                        &current_node,
                        outcome.status.as_str(),
                    ),
                )?;
            }
            completed_nodes.push(current_node.clone());

            if let Some(action) = self.poll_control() {
                return match action {
                    ExecutionControlAction::Pause => finalize_paused(
                        &store,
                        &paths,
                        &run_id,
                        &mut record,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                        &node_outcomes,
                        &route_trace,
                    ),
                    ExecutionControlAction::Cancel => finalize_canceled(
                        &store,
                        &paths,
                        &run_id,
                        &mut record,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                        &node_outcomes,
                        &route_trace,
                        "aborted_by_user",
                    ),
                };
            }

            let next_selection = select_route_after_outcome(
                &graph,
                &current_node,
                &outcome,
                &context,
                prior_status,
                &prior_preferred_label,
            )?;
            let Some(selection) = next_selection else {
                if outcome.status == OutcomeStatus::Fail {
                    let reason = stage_failure_reason(&outcome);
                    save_checkpoint_event(
                        &store,
                        &paths,
                        &run_id,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                    )?;
                    return finalize_failed(
                        &store,
                        &paths,
                        &run_id,
                        &mut record,
                        &graph,
                        &current_node,
                        &completed_nodes,
                        &context,
                        &retry_counts,
                        &node_outcomes,
                        &route_trace,
                        reason,
                        artifact_node_ids.len(),
                    );
                }
                return finalize_completed(
                    &store,
                    &paths,
                    &run_id,
                    &mut record,
                    &graph,
                    &current_node,
                    &completed_nodes,
                    &context,
                    &retry_counts,
                    &node_outcomes,
                    &route_trace,
                    "success",
                    None,
                    None,
                    artifact_node_ids.len(),
                );
            };

            current_node = selection.selected_node.unwrap_or_default();
            incoming_edge = selection.selected_edge.clone();
            context.set(CURRENT_NODE_KEY, json!(current_node.clone()))?;
            route_trace.push(current_node.clone());
            save_checkpoint_event(
                &store,
                &paths,
                &run_id,
                &current_node,
                &completed_nodes,
                &context,
                &retry_counts,
            )?;

            steps += 1;
            if max_steps.is_some_and(|max_steps| steps >= max_steps) {
                return finalize_paused(
                    &store,
                    &paths,
                    &run_id,
                    &mut record,
                    &current_node,
                    &completed_nodes,
                    &context,
                    &retry_counts,
                    &node_outcomes,
                    &route_trace,
                );
            }
        }
    }

    fn poll_control(&mut self) -> Option<ExecutionControlAction> {
        self.control.as_mut().and_then(|control| control())
    }
}

fn context_from_checkpoint(
    graph: &DotGraph,
    checkpoint: &CheckpointState,
    current_node: &str,
) -> Result<AttractorContext> {
    let mut values = checkpoint.context.clone();
    for (key, value) in graph_attr_context_seed(graph) {
        values.insert(key, value);
    }
    let mut context = AttractorContext::from_map(values)?;
    seed_builtin_context(&mut context, current_node)?;
    Ok(context)
}

fn seed_execution_record_context(context: &mut AttractorContext, record: &RunRecord) -> Result<()> {
    let execution_mode = if record.execution_mode.trim().is_empty() {
        "native"
    } else {
        record.execution_mode.trim()
    };
    let profile_id = record
        .execution_profile_id
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();
    let container_image = record
        .execution_container_image
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();
    let capabilities = record
        .execution_profile_capabilities
        .clone()
        .unwrap_or_else(|| json!([]));
    let provider = if record.llm_provider.trim().is_empty() {
        record.provider.trim()
    } else {
        record.llm_provider.trim()
    };
    for (key, value) in [
        (
            "_attractor.runtime.launch_model",
            json!(record.model.clone()),
        ),
        (
            "_attractor.runtime.launch_provider",
            json!(if provider.is_empty() {
                "codex"
            } else {
                provider
            }),
        ),
        (
            "_attractor.runtime.launch_profile",
            json!(record.llm_profile.clone().unwrap_or_default()),
        ),
        (
            "_attractor.runtime.launch_reasoning_effort",
            json!(record.reasoning_effort.clone().unwrap_or_default()),
        ),
        ("execution_mode", json!(execution_mode)),
        ("execution_profile_id", json!(profile_id.clone())),
        ("execution_container_image", json!(container_image.clone())),
        ("execution_profile_capabilities", capabilities.clone()),
        ("_attractor.runtime.execution_mode", json!(execution_mode)),
        ("_attractor.runtime.execution_profile_id", json!(profile_id)),
        (
            "_attractor.runtime.execution_container_image",
            json!(container_image),
        ),
        (
            "_attractor.runtime.execution_profile_capabilities",
            capabilities,
        ),
        ("internal.run_id", json!(record.run_id.clone())),
        (
            "internal.root_run_id",
            json!(record
                .root_run_id
                .clone()
                .unwrap_or_else(|| record.run_id.clone())),
        ),
        (
            "internal.run_workdir",
            json!(record.working_directory.clone()),
        ),
    ] {
        context.set(key, value)?;
    }
    Ok(())
}

fn resume_outcome_for_node(node_id: &str, context: &AttractorContext) -> Outcome {
    let status = context
        .get(crate::context::NODE_OUTCOMES_KEY)
        .and_then(|value| value.as_object())
        .and_then(|outcomes| outcomes.get(node_id))
        .and_then(|value| value.as_str())
        .and_then(|value| value.trim().parse::<OutcomeStatus>().ok())
        .or_else(|| {
            context
                .get(OUTCOME_KEY)
                .and_then(|value| value.as_str())
                .and_then(|value| value.trim().parse::<OutcomeStatus>().ok())
        })
        .unwrap_or(OutcomeStatus::Success);
    Outcome {
        status,
        preferred_label: context
            .get(PREFERRED_LABEL_KEY)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        ..Outcome::new(status)
    }
}

fn terminal_checkpoint_result(
    checkpoint: &CheckpointState,
    graph: &DotGraph,
) -> Result<PipelineExecutionResult> {
    let completed_nodes = checkpoint
        .completed_nodes
        .iter()
        .filter(|node_id| graph.nodes.contains_key(node_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let outcome = checkpoint
        .context
        .get(OUTCOME_KEY)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let status = if outcome.as_deref() == Some("fail") {
        "failed"
    } else {
        "completed"
    };
    Ok(PipelineExecutionResult {
        status: status.to_string(),
        current_node: checkpoint.current_node.clone(),
        completed_nodes,
        context: checkpoint.context.clone(),
        node_outcomes: BTreeMap::new(),
        route_trace: vec![checkpoint.current_node.clone()],
        failure_reason: if status == "failed" {
            checkpoint
                .context
                .get("failure_reason")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        } else {
            String::new()
        },
        outcome,
        outcome_reason_code: None,
        outcome_reason_message: None,
    })
}

fn ensure_run_record_defaults(record: &mut RunRecord) -> String {
    if record.run_id.trim().is_empty() {
        record.run_id = format!(
            "run-{}",
            crate::events::utc_timestamp().replace([':', '.'], "-")
        );
    }
    if record.status.trim().is_empty() {
        record.status = "running".to_string();
    }
    if record.started_at.trim().is_empty() {
        record.started_at = crate::events::utc_timestamp();
    }
    if record.root_run_id.is_none() {
        record.root_run_id = Some(record.run_id.clone());
    }
    record.run_id.clone()
}

fn prompt_for_node(node: &attractor_core::DotNode) -> String {
    spark_agent_adapter::codergen::authored_prompt_for_node(node)
}

fn outcome_from_node_error(error: RuntimeNodeError) -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        failure_reason: error.message,
        retryable: error.retryable,
        failure_kind: Some(error.failure_kind.unwrap_or(FailureKind::Runtime)),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}

pub(crate) fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return format!("handler panic: {message}");
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return format!("handler panic: {message}");
    }
    "handler panic".to_string()
}

fn write_stage_artifacts(
    store: &RunStore,
    paths: &crate::paths::RunRootPaths,
    node_id: &str,
    prompt: &str,
    outcome: &Outcome,
    status_transitions: &mut BTreeMap<String, Vec<String>>,
    artifact_node_ids: &mut BTreeSet<String>,
) -> Result<()> {
    artifact_node_ids.insert(node_id.to_string());
    let transitions = status_transitions.entry(node_id.to_string()).or_default();
    transitions.push(outcome.status.as_str().to_string());
    let mut status = json!({
        "outcome": outcome.status.as_str(),
        "preferred_label": outcome.preferred_label,
        "suggested_next_ids": outcome.suggested_next_ids,
        "context_updates": outcome.context_updates,
        "notes": outcome.notes,
        "status_transitions": transitions,
    });
    if let Some(failure_kind) = outcome.failure_kind {
        status["failure_kind"] = json!(failure_kind.as_str());
    }
    store.write_node_artifacts(
        paths,
        node_id,
        &NodeArtifacts {
            prompt: Some(format!("{prompt}\n")),
            response: Some(format!("{}\n", response_text_for_outcome(outcome))),
            status: Some(status),
            under_logs: true,
        },
    )?;
    Ok(())
}

fn response_text_for_outcome(outcome: &Outcome) -> String {
    if !outcome.raw_response_text.is_empty() {
        return outcome.raw_response_text.clone();
    }
    if !outcome.notes.is_empty() {
        return outcome.notes.clone();
    }
    outcome.failure_reason.clone()
}

fn select_route_after_outcome(
    graph: &DotGraph,
    node_id: &str,
    outcome: &Outcome,
    context: &AttractorContext,
    prior_status: Option<OutcomeStatus>,
    prior_preferred_label: &str,
) -> Result<Option<NextNodeSelection>> {
    let selection = select_next_node_with_prior(
        graph,
        node_id,
        outcome,
        context,
        prior_status,
        prior_preferred_label,
    )?;
    if outcome.status != OutcomeStatus::Fail {
        return Ok(selection.selected_node.is_some().then_some(selection));
    }
    if selection
        .selected_edge
        .as_ref()
        .is_some_and(|edge| !edge.condition_text().is_empty())
    {
        return Ok(selection.selected_node.is_some().then_some(selection));
    }
    if let Some(target) = resolve_failure_retry_target(graph, node_id) {
        return Ok(Some(NextNodeSelection {
            current_node: node_id.to_string(),
            selected_node: Some(target),
            selected_edge: None,
            reason: "retry_target".to_string(),
        }));
    }
    if let Some(target) = selection.selected_node.as_deref() {
        if is_conditional_node(graph, target) {
            return Ok(Some(selection));
        }
        if is_goal_gate_node(graph, node_id) && is_exit_node(graph, target) {
            return Ok(Some(selection));
        }
    }
    Ok(None)
}

fn save_checkpoint_event(
    store: &RunStore,
    paths: &crate::paths::RunRootPaths,
    run_id: &str,
    current_node: &str,
    completed_nodes: &[String],
    context: &AttractorContext,
    retry_counts: &BTreeMap<String, u64>,
) -> Result<CheckpointState> {
    let checkpoint = checkpoint_from_context(
        current_node,
        completed_nodes.iter().cloned(),
        context,
        retry_counts
            .iter()
            .map(|(key, value)| (key.clone(), *value)),
    );
    store.save_checkpoint(paths, &checkpoint, CheckpointWriteOptions::default())?;
    store.append_event(
        paths,
        checkpoint_saved_event(run_id, current_node, completed_nodes.to_vec()),
    )?;
    Ok(checkpoint)
}

#[allow(clippy::too_many_arguments)]
fn finalize_completed(
    store: &RunStore,
    paths: &crate::paths::RunRootPaths,
    run_id: &str,
    record: &mut RunRecord,
    graph: &DotGraph,
    current_node: &str,
    completed_nodes: &[String],
    context: &AttractorContext,
    retry_counts: &BTreeMap<String, u64>,
    node_outcomes: &BTreeMap<String, Outcome>,
    route_trace: &[String],
    outcome: &str,
    outcome_reason_code: Option<String>,
    outcome_reason_message: Option<String>,
    artifact_count: usize,
) -> Result<PipelineExecutionResult> {
    let checkpoint = save_checkpoint_event(
        store,
        paths,
        run_id,
        current_node,
        completed_nodes,
        context,
        retry_counts,
    )?;
    record.status = "completed".to_string();
    record.outcome = Some(outcome.to_string());
    record.outcome_reason_code = outcome_reason_code.clone();
    record.outcome_reason_message = outcome_reason_message.clone();
    record.ended_at = Some(crate::events::utc_timestamp());
    record.last_error.clear();
    write_run_record(paths, record)?;
    store.append_event(
        paths,
        runtime_status_event(
            run_id,
            &record.status,
            record.outcome.clone(),
            record.outcome_reason_code.clone(),
            record.outcome_reason_message.clone(),
            None,
        ),
    )?;
    store.append_event(
        paths,
        pipeline_completed_event_with_reasons(
            run_id,
            current_node,
            &record.status,
            record.outcome.clone(),
            record.outcome_reason_code.clone(),
            record.outcome_reason_message.clone(),
            artifact_count,
        ),
    )?;
    store.materialize_result(paths, run_id, &record.status, graph, &checkpoint, None)?;
    Ok(PipelineExecutionResult {
        status: "completed".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: completed_nodes.to_vec(),
        context: context.snapshot(),
        node_outcomes: node_outcomes.clone(),
        route_trace: route_trace.to_vec(),
        failure_reason: String::new(),
        outcome: record.outcome.clone(),
        outcome_reason_code,
        outcome_reason_message,
    })
}

#[allow(clippy::too_many_arguments)]
fn finalize_failed(
    store: &RunStore,
    paths: &crate::paths::RunRootPaths,
    run_id: &str,
    record: &mut RunRecord,
    graph: &DotGraph,
    current_node: &str,
    completed_nodes: &[String],
    context: &AttractorContext,
    retry_counts: &BTreeMap<String, u64>,
    node_outcomes: &BTreeMap<String, Outcome>,
    route_trace: &[String],
    failure_reason: String,
    artifact_count: usize,
) -> Result<PipelineExecutionResult> {
    let checkpoint = save_checkpoint_event(
        store,
        paths,
        run_id,
        current_node,
        completed_nodes,
        context,
        retry_counts,
    )?;
    record.status = "failed".to_string();
    record.outcome = None;
    record.outcome_reason_code = None;
    record.outcome_reason_message = None;
    record.ended_at = Some(crate::events::utc_timestamp());
    record.last_error = failure_reason.clone();
    write_run_record(paths, record)?;
    store.append_event(
        paths,
        runtime_status_event(
            run_id,
            &record.status,
            None,
            None,
            None,
            Some(failure_reason.clone()),
        ),
    )?;
    store.append_event(
        paths,
        pipeline_failed_event(run_id, current_node, &failure_reason, artifact_count),
    )?;
    write_run_result(
        paths,
        &RunResult {
            run_id: run_id.to_string(),
            status: "failed".to_string(),
            state: "error".to_string(),
            error: Some(failure_reason.clone()),
            ..RunResult::default()
        },
    )?;
    let _ = graph;
    let _ = checkpoint;
    Ok(PipelineExecutionResult {
        status: "failed".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: completed_nodes.to_vec(),
        context: context.snapshot(),
        node_outcomes: node_outcomes.clone(),
        route_trace: route_trace.to_vec(),
        failure_reason,
        outcome: None,
        outcome_reason_code: None,
        outcome_reason_message: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn finalize_paused(
    store: &RunStore,
    paths: &crate::paths::RunRootPaths,
    run_id: &str,
    record: &mut RunRecord,
    current_node: &str,
    completed_nodes: &[String],
    context: &AttractorContext,
    retry_counts: &BTreeMap<String, u64>,
    node_outcomes: &BTreeMap<String, Outcome>,
    route_trace: &[String],
) -> Result<PipelineExecutionResult> {
    save_checkpoint_event(
        store,
        paths,
        run_id,
        current_node,
        completed_nodes,
        context,
        retry_counts,
    )?;
    mark_record_paused(record);
    write_run_record(paths, record)?;
    store.append_event(paths, pipeline_paused_event(run_id, current_node))?;
    store.append_event(
        paths,
        runtime_status_event(run_id, "paused", None, None, None, None),
    )?;
    Ok(PipelineExecutionResult {
        status: "paused".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: completed_nodes.to_vec(),
        context: context.snapshot(),
        node_outcomes: node_outcomes.clone(),
        route_trace: route_trace.to_vec(),
        failure_reason: String::new(),
        outcome: None,
        outcome_reason_code: None,
        outcome_reason_message: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn finalize_canceled(
    store: &RunStore,
    paths: &crate::paths::RunRootPaths,
    run_id: &str,
    record: &mut RunRecord,
    current_node: &str,
    completed_nodes: &[String],
    context: &AttractorContext,
    retry_counts: &BTreeMap<String, u64>,
    node_outcomes: &BTreeMap<String, Outcome>,
    route_trace: &[String],
    last_error: &str,
) -> Result<PipelineExecutionResult> {
    save_checkpoint_event(
        store,
        paths,
        run_id,
        current_node,
        completed_nodes,
        context,
        retry_counts,
    )?;
    mark_record_canceled(record, last_error);
    write_run_record(paths, record)?;
    store.append_event(
        paths,
        runtime_status_event(
            run_id,
            "canceled",
            None,
            None,
            None,
            Some(last_error.to_string()),
        ),
    )?;
    store.append_event(
        paths,
        pipeline_failed_event(run_id, current_node, last_error, node_outcomes.len()),
    )?;
    store.append_event(
        paths,
        log_event(run_id, "Pipeline Canceled: aborted_by_user"),
    )?;
    write_run_result(
        paths,
        &RunResult {
            run_id: run_id.to_string(),
            status: "canceled".to_string(),
            state: "error".to_string(),
            error: Some(last_error.to_string()),
            ..RunResult::default()
        },
    )?;
    Ok(PipelineExecutionResult {
        status: "canceled".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: completed_nodes.to_vec(),
        context: context.snapshot(),
        node_outcomes: node_outcomes.clone(),
        route_trace: route_trace.to_vec(),
        failure_reason: last_error.to_string(),
        outcome: None,
        outcome_reason_code: None,
        outcome_reason_message: None,
    })
}

fn stage_failure_reason(outcome: &Outcome) -> String {
    let reason = outcome.failure_reason.trim();
    if reason.is_empty() {
        "stage_failed".to_string()
    } else {
        reason.to_string()
    }
}
