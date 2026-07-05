use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use attractor_core::{
    attr_text, dot_value_text, evaluate_condition, AttractorContext, CheckpointState, ContextMap,
    DotAttribute, DotValue, FailureKind, LaunchContext, Outcome, OutcomeStatus, RunRecord,
};
use attractor_dsl::{apply_graph_transforms, parse_dot, validate_graph, DiagnosticSeverity};
use serde_json::{json, Value};

use crate::context::graph_attr_context_seed;
use crate::events::{
    child_intervention_requested_event, child_run_completed_event, child_run_started_event,
};
use crate::executor::{
    ExecuteRunRequest, ExecutionStart, PipelineExecutionResult, PipelineExecutor, RuntimeNodeError,
};
use crate::handlers::{
    ChildInterventionRequest, ChildInterventionResult, ChildRunRequest, ChildRunResult,
    HandlerRuntime, RuntimeHandlerRunner,
};
use crate::routing::resolve_start_node;
use crate::store::RunStore;

const AUTO_STEER_ATTEMPT_LIMIT: u64 = 1;
const CHILD_KEYS: &[&str] = &[
    "context.stack.child.run_id",
    "context.stack.child.status",
    "context.stack.child.outcome",
    "context.stack.child.outcome_reason_code",
    "context.stack.child.outcome_reason_message",
    "context.stack.child.active_stage",
    "context.stack.child.completed_nodes",
    "context.stack.child.route_trace",
    "context.stack.child.failure_reason",
    "context.stack.child.retry_count",
    "context.stack.child.retry_counts",
    "context.stack.child.artifact_count",
    "context.stack.child.event_count",
    "context.stack.child.checkpoint_timestamp",
    "context.stack.child.latest_event_at",
    "context.stack.child.started_at",
    "context.stack.child.ended_at",
    "context.stack.child.intervention",
    "context.stack.child.intervention_status",
    "context.stack.child.intervention_delivery_mode",
    "context.stack.child.intervention_reason",
];

pub(crate) fn execute_manager_loop(
    runner: &RuntimeHandlerRunner,
    runtime: HandlerRuntime,
) -> std::result::Result<Outcome, RuntimeNodeError> {
    let original_context = runtime.context.clone();
    let mut context = runtime.context.clone();

    if let Some(outcome) = autostart_child_pipeline(runner, &runtime, &mut context)? {
        return Ok(outcome_with_context_updates(
            outcome,
            &original_context,
            &context,
        ));
    }

    let poll_interval = poll_interval_duration(runtime.node_attrs.get("manager.poll_interval"))
        .unwrap_or_else(|| Duration::from_secs(45));
    let max_cycles = max_cycles(runtime.node_attrs.get("manager.max_cycles"));
    let stop_condition = stop_condition(runtime.node_attrs.get("manager.stop_condition"));
    let actions = manager_actions(runtime.node_attrs.get("manager.actions"));
    let steer_cooldown = poll_interval_duration(runtime.node_attrs.get("manager.steer_cooldown"))
        .unwrap_or_else(|| Duration::from_secs(0));
    let mut last_steer_at: Option<Instant> = None;
    let mut automatic_steer_attempts = BTreeMap::<(String, Option<String>, String), u64>::new();

    for cycle in 1..=max_cycles {
        if actions.contains("observe") {
            ingest_child_telemetry(runner, &runtime, &mut context);
            append_manager_artifact(
                runtime.logs_root.as_deref(),
                &runtime.node_id,
                "manager_telemetry.jsonl",
                telemetry_payload(&context, &runtime.node_id, cycle),
            );
        }

        let now = Instant::now();
        if actions.contains("steer") && steer_cooldown_elapsed(now, last_steer_at, steer_cooldown) {
            if let Some(intervention) = request_child_intervention(
                runner,
                &runtime,
                &mut context,
                cycle,
                &mut automatic_steer_attempts,
            )? {
                let failure_reason = child_failure_context(&context);
                append_manager_artifact(
                    runtime.logs_root.as_deref(),
                    &runtime.node_id,
                    "manager_interventions.jsonl",
                    intervention_payload(
                        &context,
                        &runtime.node_id,
                        cycle,
                        &intervention,
                        &failure_reason,
                    ),
                );
                if intervention.status != "skipped" {
                    last_steer_at = Some(now);
                }
            }
        }

        if let Some(outcome) = resolve_child_status(&context) {
            return Ok(outcome_with_context_updates(
                outcome,
                &original_context,
                &context,
            ));
        }

        if !stop_condition.is_empty() && stop_condition_met(&stop_condition, &context) {
            return Ok(outcome_with_context_updates(
                Outcome {
                    status: OutcomeStatus::Success,
                    notes: "Stop condition satisfied".to_string(),
                    ..Outcome::new(OutcomeStatus::Success)
                },
                &original_context,
                &context,
            ));
        }

        if actions.contains("wait") {
            if !poll_interval.is_zero() {
                std::thread::sleep(poll_interval);
            }
        }
    }

    Ok(outcome_with_context_updates(
        Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: "Max cycles exceeded".to_string(),
            ..Outcome::new(OutcomeStatus::Fail)
        },
        &original_context,
        &context,
    ))
}

fn autostart_child_pipeline(
    runner: &RuntimeHandlerRunner,
    runtime: &HandlerRuntime,
    context: &mut ContextMap,
) -> std::result::Result<Option<Outcome>, RuntimeNodeError> {
    let child_dotfile =
        attr_text(&runtime.graph.graph_attrs, "stack.child_dotfile").unwrap_or_default();
    if child_dotfile.is_empty() {
        return Ok(None);
    }
    if !child_autostart_enabled(runtime.node_attrs.get("stack.child_autostart")) {
        return Ok(None);
    }

    let linked_child_run_id = context_string(context, "context.stack.child.run_id");
    if !linked_child_run_id.is_empty() {
        if let Some(child_result) = resolve_child_result(runner, runtime, &linked_child_run_id) {
            apply_child_run_result(context, &child_result);
            if child_result.status.trim().eq_ignore_ascii_case("running") {
                return Ok(None);
            }
        }
    }
    if context_string(context, "context.stack.child.status").eq_ignore_ascii_case("running") {
        return Ok(None);
    }

    clear_child_snapshot(context);
    let authored_child_workdir =
        authored_attr_text(runtime.graph.graph_attrs.get("stack.child_workdir"));
    let child_workdir_path =
        resolve_child_workdir_path(context, &runtime.run_workdir, &authored_child_workdir);
    let child_dot_path = resolve_child_dot_path(
        &child_dotfile,
        &child_workdir_path,
        context,
        !authored_child_workdir.is_empty(),
    );
    if !child_dot_path.exists() {
        return Ok(Some(fail_outcome(format!(
            "Child DOT file not found: {}",
            child_dot_path.display()
        ))));
    }

    let child_source = match fs::read_to_string(&child_dot_path) {
        Ok(source) => source,
        Err(error) => {
            return Ok(Some(fail_outcome(format!(
                "Unable to read child DOT file: {error}"
            ))));
        }
    };
    let parsed = match parse_dot(&child_source) {
        Ok(graph) => graph,
        Err(error) => {
            return Ok(Some(fail_outcome(format!(
                "Failed to parse child DOT graph: {error}"
            ))));
        }
    };
    let child_graph = apply_graph_transforms(&parsed);
    if let Some(error) = validate_graph(&child_graph)
        .into_iter()
        .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Ok(Some(fail_outcome(format!(
            "Child DOT graph failed validation: {}",
            error.message
        ))));
    }

    let child_flow_name = child_dot_path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| (!child_graph.graph_id.trim().is_empty()).then(|| child_graph.graph_id.clone()))
        .unwrap_or_else(|| "child".to_string());
    let parent_run_id = context_string(context, "internal.run_id")
        .trim()
        .to_string();
    let parent_run_id = if parent_run_id.is_empty() {
        runtime.run_id.clone()
    } else {
        parent_run_id
    };
    let root_run_id = non_empty(context_string(context, "internal.root_run_id"))
        .unwrap_or_else(|| parent_run_id.clone());
    let child_run_id = generated_child_run_id();
    let parent_context = context.clone();

    set_context(
        context,
        "context.stack.child.run_id",
        json!(child_run_id.clone()),
    );
    set_context(context, "context.stack.child.status", json!("running"));
    runner
        .emit(
            runtime,
            child_run_started_event(
                &parent_run_id,
                &child_run_id,
                &runtime.node_id,
                &root_run_id,
                &child_flow_name,
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;

    let request = ChildRunRequest {
        child_run_id: child_run_id.clone(),
        child_graph: child_graph.clone(),
        child_flow_name: child_flow_name.clone(),
        child_flow_path: child_dot_path.clone(),
        child_workdir: child_workdir_path.clone(),
        parent_context,
        parent_run_id: parent_run_id.clone(),
        parent_node_id: runtime.node_id.clone(),
        root_run_id: root_run_id.clone(),
    };
    let child_result = if let Some(launcher) = runner.child_run_launcher() {
        launcher(request)
    } else {
        match launch_default_child_run(runner.clone(), runtime, request, child_source) {
            Ok(result) => result,
            Err(reason) => ChildRunResult {
                run_id: child_run_id.clone(),
                status: "failed".to_string(),
                failure_reason: format!(
                    "Unable to run child pipeline from {}: {reason}",
                    child_workdir_path.display()
                ),
                ..ChildRunResult::default()
            },
        }
    };
    apply_child_run_result(context, &child_result);
    runner
        .emit(
            runtime,
            child_run_completed_event(
                &parent_run_id,
                if child_result.run_id.is_empty() {
                    child_run_id
                } else {
                    child_result.run_id.clone()
                },
                &runtime.node_id,
                &root_run_id,
                &child_flow_name,
                &child_result.status,
                child_result.outcome.clone(),
                child_result.outcome_reason_code.clone(),
                child_result.outcome_reason_message.clone(),
                non_empty(child_result.failure_reason.clone()),
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))?;
    Ok(None)
}

fn launch_default_child_run(
    runner: RuntimeHandlerRunner,
    runtime: &HandlerRuntime,
    request: ChildRunRequest,
    child_source: String,
) -> std::result::Result<ChildRunResult, String> {
    let Some(parent_paths) = runtime.run_paths.as_ref() else {
        return Err("parent run paths are unavailable".to_string());
    };
    let store = RunStore::for_runs_dir(parent_paths.runs_dir.clone());
    let start_node = resolve_start_node(&request.child_graph).map_err(|error| error.to_string())?;
    let mut child_context = request.parent_context.clone();
    clear_child_snapshot(&mut child_context);
    for (key, value) in graph_attr_context_seed(&request.child_graph) {
        child_context.insert(key, value);
    }
    set_context(
        &mut child_context,
        "internal.run_id",
        json!(request.child_run_id.clone()),
    );
    set_context(
        &mut child_context,
        "internal.parent_run_id",
        json!(request.parent_run_id.clone()),
    );
    set_context(
        &mut child_context,
        "internal.parent_node_id",
        json!(request.parent_node_id.clone()),
    );
    set_context(
        &mut child_context,
        "internal.root_run_id",
        json!(request.root_run_id.clone()),
    );
    set_context(
        &mut child_context,
        "internal.run_workdir",
        json!(request.child_workdir.to_string_lossy().to_string()),
    );
    if let Some(source_dir) = request.child_flow_path.parent() {
        set_context(
            &mut child_context,
            "internal.flow_source_dir",
            json!(source_dir.to_string_lossy().to_string()),
        );
    }

    let child_invocation_index = store
        .next_child_invocation_index(&request.parent_run_id, &request.parent_node_id)
        .map_err(|error| error.to_string())?;
    let mut record = RunRecord::new(
        request.child_run_id.clone(),
        request.child_workdir.to_string_lossy().to_string(),
    );
    record.flow_name = request.child_flow_name.clone();
    record.working_directory = request.child_workdir.to_string_lossy().to_string();
    record.project_path = record.working_directory.clone();
    record.parent_run_id = Some(request.parent_run_id.clone());
    record.parent_node_id = Some(request.parent_node_id.clone());
    record.root_run_id = Some(request.root_run_id.clone());
    record.child_invocation_index = Some(child_invocation_index);
    record.started_at = crate::events::utc_timestamp();
    record.model = context_string(&child_context, "_attractor.runtime.launch_model");
    let provider = non_empty(context_string(
        &child_context,
        "_attractor.runtime.launch_provider",
    ))
    .unwrap_or_else(|| "codex".to_string());
    record.provider = provider.clone();
    record.llm_provider = provider;
    record.llm_profile = non_empty(context_string(
        &child_context,
        "_attractor.runtime.launch_profile",
    ));
    record.reasoning_effort = non_empty(context_string(
        &child_context,
        "_attractor.runtime.launch_reasoning_effort",
    ));

    let checkpoint = CheckpointState {
        timestamp: crate::events::utc_timestamp(),
        current_node: start_node.clone(),
        completed_nodes: Vec::new(),
        context: child_context,
        retry_counts: BTreeMap::new(),
        logs: Vec::new(),
    };
    let store_for_result = store.clone();
    let child_run_id = request.child_run_id.clone();
    let mut executor = PipelineExecutor::new(runner);
    let result = executor
        .execute(ExecuteRunRequest {
            store,
            record,
            graph: request.child_graph,
            graph_source: Some(child_source.clone()),
            graph_dot: Some(child_source),
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::FromCheckpoint {
                start_node,
                checkpoint,
            },
        })
        .map(pipeline_result_to_child_result)
        .map_err(|error| error.to_string())?;
    Ok(store_for_result
        .read_run_bundle(&child_run_id)
        .ok()
        .flatten()
        .and_then(child_result_from_bundle)
        .unwrap_or(result))
}

fn pipeline_result_to_child_result(result: PipelineExecutionResult) -> ChildRunResult {
    ChildRunResult {
        run_id: context_string(&result.context, "internal.run_id"),
        status: result.status,
        outcome: result.outcome,
        outcome_reason_code: result.outcome_reason_code,
        outcome_reason_message: result.outcome_reason_message,
        current_node: result.current_node,
        completed_nodes: result.completed_nodes,
        route_trace: result.route_trace,
        failure_reason: result.failure_reason,
        ..ChildRunResult::default()
    }
}

fn ingest_child_telemetry(
    runner: &RuntimeHandlerRunner,
    runtime: &HandlerRuntime,
    context: &mut ContextMap,
) {
    let child_run_id = context_string(context, "context.stack.child.run_id");
    if child_run_id.is_empty() {
        return;
    }
    if let Some(result) = resolve_child_result(runner, runtime, &child_run_id) {
        apply_child_run_result(context, &result);
    }
}

fn resolve_child_result(
    runner: &RuntimeHandlerRunner,
    runtime: &HandlerRuntime,
    child_run_id: &str,
) -> Option<ChildRunResult> {
    if let Some(resolver) = runner.child_status_resolver() {
        return resolver(child_run_id);
    }
    let run_paths = runtime.run_paths.as_ref()?;
    let store = RunStore::for_runs_dir(run_paths.runs_dir.clone());
    store
        .read_run_bundle(child_run_id)
        .ok()
        .flatten()
        .and_then(child_result_from_bundle)
}

fn child_result_from_bundle(bundle: crate::store::RunBundle) -> Option<ChildRunResult> {
    let record = bundle.record?;
    let (
        current_node,
        completed_nodes,
        route_trace,
        retry_counts,
        retry_count,
        checkpoint_timestamp,
    ) = if let Some(checkpoint) = bundle.checkpoint {
        let mut route_trace = checkpoint.completed_nodes.clone();
        if !checkpoint.current_node.is_empty()
            && route_trace.last() != Some(&checkpoint.current_node)
        {
            route_trace.push(checkpoint.current_node.clone());
        }
        let retry_count = if checkpoint.current_node.is_empty() {
            None
        } else {
            checkpoint
                .retry_counts
                .get(&checkpoint.current_node)
                .copied()
        };
        (
            checkpoint.current_node,
            checkpoint.completed_nodes,
            route_trace,
            checkpoint.retry_counts,
            retry_count,
            checkpoint.timestamp,
        )
    } else {
        (
            String::new(),
            Vec::new(),
            Vec::new(),
            BTreeMap::new(),
            None,
            String::new(),
        )
    };
    let artifact_count = crate::artifacts::list_artifacts(&bundle.paths)
        .ok()
        .map(|artifacts| artifacts.len());
    let event_count = Some(bundle.raw_events.len());
    let latest_event_at = bundle
        .raw_events
        .iter()
        .rev()
        .find_map(|event| non_empty(event.emitted_at.clone()))
        .unwrap_or_default();
    Some(ChildRunResult {
        run_id: record.run_id,
        status: record.status,
        outcome: record.outcome,
        outcome_reason_code: record.outcome_reason_code,
        outcome_reason_message: record.outcome_reason_message,
        current_node,
        completed_nodes,
        route_trace,
        failure_reason: record.last_error,
        retry_count,
        retry_counts,
        artifact_count,
        event_count,
        checkpoint_timestamp,
        latest_event_at,
        started_at: record.started_at,
        ended_at: record.ended_at,
    })
}

fn apply_child_run_result(context: &mut ContextMap, child_result: &ChildRunResult) {
    if !child_result.run_id.trim().is_empty() {
        set_context(
            context,
            "context.stack.child.run_id",
            json!(child_result.run_id.clone()),
        );
    }
    set_context(
        context,
        "context.stack.child.status",
        json!(child_result.status.clone()),
    );
    set_context(
        context,
        "context.stack.child.outcome",
        json!(child_result.outcome.clone().unwrap_or_default()),
    );
    set_context(
        context,
        "context.stack.child.outcome_reason_code",
        json!(child_result.outcome_reason_code.clone().unwrap_or_default()),
    );
    set_context(
        context,
        "context.stack.child.outcome_reason_message",
        json!(child_result
            .outcome_reason_message
            .clone()
            .unwrap_or_default()),
    );
    set_context(
        context,
        "context.stack.child.active_stage",
        json!(child_result.current_node.clone()),
    );
    set_context(
        context,
        "context.stack.child.completed_nodes",
        json!(child_result.completed_nodes.clone()),
    );
    set_context(
        context,
        "context.stack.child.route_trace",
        json!(child_result.route_trace.clone()),
    );
    set_context(
        context,
        "context.stack.child.failure_reason",
        json!(child_result.failure_reason.clone()),
    );
    set_context(
        context,
        "context.stack.child.retry_count",
        child_result
            .retry_count
            .map(Value::from)
            .unwrap_or_else(|| json!("")),
    );
    set_context(
        context,
        "context.stack.child.retry_counts",
        json!(child_result.retry_counts.clone()),
    );
    set_context(
        context,
        "context.stack.child.artifact_count",
        child_result
            .artifact_count
            .map(|value| json!(value))
            .unwrap_or_else(|| json!("")),
    );
    set_context(
        context,
        "context.stack.child.event_count",
        child_result
            .event_count
            .map(|value| json!(value))
            .unwrap_or_else(|| json!("")),
    );
    set_context(
        context,
        "context.stack.child.checkpoint_timestamp",
        json!(child_result.checkpoint_timestamp.clone()),
    );
    set_context(
        context,
        "context.stack.child.latest_event_at",
        json!(child_result.latest_event_at.clone()),
    );
    set_context(
        context,
        "context.stack.child.started_at",
        json!(child_result.started_at.clone()),
    );
    set_context(
        context,
        "context.stack.child.ended_at",
        json!(child_result.ended_at.clone().unwrap_or_default()),
    );
}

fn clear_child_snapshot(context: &mut ContextMap) {
    for key in CHILD_KEYS {
        let value = match *key {
            "context.stack.child.completed_nodes" | "context.stack.child.route_trace" => json!([]),
            "context.stack.child.retry_counts" => json!({}),
            _ => json!(""),
        };
        set_context(context, *key, value);
    }
}

fn request_child_intervention(
    runner: &RuntimeHandlerRunner,
    runtime: &HandlerRuntime,
    context: &mut ContextMap,
    cycle: u64,
    automatic_steer_attempts: &mut BTreeMap<(String, Option<String>, String), u64>,
) -> std::result::Result<Option<ChildInterventionResult>, RuntimeNodeError> {
    let failure_reason = child_failure_context(context);
    if failure_reason.is_empty() {
        return Ok(None);
    }

    let child_run_id = context_string(context, "context.stack.child.run_id");
    let target_node_id = non_empty(context_string(context, "context.stack.child.active_stage"));
    if child_run_id.is_empty() {
        let result = ChildInterventionResult {
            run_id: String::new(),
            status: "rejected".to_string(),
            delivery_mode: "none".to_string(),
            reason: "no_active_child_run".to_string(),
            message: "No active child run is available for intervention.".to_string(),
            target_node_id,
        };
        apply_intervention_result(runner, runtime, context, &result, &failure_reason)?;
        return Ok(Some(result));
    }

    let auto_steer_key = (
        child_run_id.clone(),
        target_node_id.clone(),
        failure_reason.clone(),
    );
    let prior_attempts = automatic_steer_attempts
        .get(&auto_steer_key)
        .copied()
        .unwrap_or(0);
    if prior_attempts >= AUTO_STEER_ATTEMPT_LIMIT {
        let result = ChildInterventionResult {
            run_id: child_run_id.clone(),
            status: "skipped".to_string(),
            delivery_mode: "none".to_string(),
            reason: "auto_steer_limit_reached".to_string(),
            message: auto_steer_limit_message(
                &child_run_id,
                target_node_id.as_deref(),
                &failure_reason,
            ),
            target_node_id,
        };
        apply_intervention_result(runner, runtime, context, &result, &failure_reason)?;
        return Ok(Some(result));
    }
    automatic_steer_attempts.insert(auto_steer_key, prior_attempts + 1);

    let mut message = context_string(context, "context.stack.child.intervention");
    if message.is_empty() {
        message = default_intervention_message(context, &failure_reason);
        set_context(
            context,
            "context.stack.child.intervention",
            json!(message.clone()),
        );
    }
    let parent_run_id = non_empty(context_string(context, "internal.run_id"))
        .unwrap_or_else(|| runtime.run_id.clone());
    let request = ChildInterventionRequest {
        child_run_id: child_run_id.clone(),
        message,
        parent_run_id: parent_run_id.clone(),
        parent_node_id: runtime.node_id.clone(),
        root_run_id: non_empty(context_string(context, "internal.root_run_id"))
            .unwrap_or(parent_run_id),
        reason: failure_reason.clone(),
        source: "manager_loop".to_string(),
        cycle: Some(cycle),
        target_node_id: target_node_id.clone(),
    };
    let result = if let Some(requester) = runner.child_intervention_requester() {
        requester(request)
    } else {
        ChildInterventionResult {
            run_id: child_run_id,
            status: "rejected".to_string(),
            delivery_mode: "unsupported".to_string(),
            reason: "backend_steering_unsupported".to_string(),
            message: "Child intervention requester is unavailable.".to_string(),
            target_node_id,
        }
    };
    apply_intervention_result(runner, runtime, context, &result, &failure_reason)?;
    Ok(Some(result))
}

fn apply_intervention_result(
    runner: &RuntimeHandlerRunner,
    runtime: &HandlerRuntime,
    context: &mut ContextMap,
    result: &ChildInterventionResult,
    failure_reason: &str,
) -> std::result::Result<(), RuntimeNodeError> {
    let status = if result.status.trim().is_empty() {
        "rejected".to_string()
    } else {
        result.status.clone()
    };
    let intervention_reason = if result.reason.trim().is_empty() {
        failure_reason.to_string()
    } else {
        result.reason.clone()
    };
    set_context(
        context,
        "context.stack.child.intervention_status",
        json!(status.clone()),
    );
    set_context(
        context,
        "context.stack.child.intervention_delivery_mode",
        json!(result.delivery_mode.clone()),
    );
    set_context(
        context,
        "context.stack.child.intervention_reason",
        json!(intervention_reason.clone()),
    );
    let parent_run_id = non_empty(context_string(context, "internal.run_id"))
        .unwrap_or_else(|| runtime.run_id.clone());
    let root_run_id = non_empty(context_string(context, "internal.root_run_id"))
        .unwrap_or_else(|| parent_run_id.clone());
    runner
        .emit(
            runtime,
            child_intervention_requested_event(
                &parent_run_id,
                &result.run_id,
                &runtime.node_id,
                root_run_id,
                result.target_node_id.clone(),
                status,
                &result.delivery_mode,
                intervention_reason,
                failure_reason,
                non_empty(result.message.clone()),
            ),
        )
        .map_err(|error| RuntimeNodeError::runtime(error.to_string()))
}

fn resolve_child_status(context: &ContextMap) -> Option<Outcome> {
    let child_status = context_string(context, "context.stack.child.status").to_lowercase();
    if !matches!(
        child_status.as_str(),
        "completed" | "failed" | "aborted" | "canceled" | "cancelled"
    ) {
        return None;
    }
    let child_outcome = context_string(context, "context.stack.child.outcome").to_lowercase();
    if child_status == "completed" && child_outcome == "success" {
        return Some(Outcome {
            status: OutcomeStatus::Success,
            notes: "Child completed".to_string(),
            ..Outcome::new(OutcomeStatus::Success)
        });
    }
    if child_status == "completed" && child_outcome == "failure" {
        let failure_reason = non_empty(context_string(
            context,
            "context.stack.child.outcome_reason_message",
        ))
        .unwrap_or_else(|| "Child completed with failure outcome".to_string());
        return Some(Outcome {
            status: OutcomeStatus::Fail,
            failure_reason,
            ..Outcome::new(OutcomeStatus::Fail)
        });
    }
    if child_status == "failed" {
        let failure_reason = non_empty(context_string(
            context,
            "context.stack.child.failure_reason",
        ))
        .unwrap_or_else(|| "Child failed".to_string());
        return Some(Outcome {
            status: OutcomeStatus::Fail,
            failure_reason,
            ..Outcome::new(OutcomeStatus::Fail)
        });
    }
    if matches!(child_status.as_str(), "aborted" | "canceled" | "cancelled") {
        return Some(Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: "aborted_by_user".to_string(),
            ..Outcome::new(OutcomeStatus::Fail)
        });
    }
    None
}

fn poll_interval_duration(attr: Option<&DotAttribute>) -> Option<Duration> {
    let attr = attr?;
    match &attr.value {
        DotValue::Duration(duration) => duration_from_parts(duration.value, &duration.unit),
        DotValue::Boolean(value) => Some(Duration::from_secs(u64::from(*value))),
        DotValue::Integer(value) => Some(Duration::from_secs_f64((*value).max(0) as f64)),
        DotValue::Float(value) => Some(Duration::from_secs_f64(value.max(0.0))),
        DotValue::String(value) => parse_duration_string(value, Duration::from_secs(45)),
        DotValue::Null => Some(Duration::from_secs(45)),
    }
}

fn max_cycles(attr: Option<&DotAttribute>) -> u64 {
    let Some(attr) = attr else {
        return 1000;
    };
    match &attr.value {
        DotValue::Boolean(value) => u64::from(*value),
        DotValue::Integer(value) => (*value).max(0) as u64,
        DotValue::Float(value) => value.max(0.0) as u64,
        DotValue::String(value) => {
            let normalized = value.trim().to_lowercase();
            if false_like(&normalized) {
                0
            } else {
                normalized
                    .parse::<i64>()
                    .map(|value| value.max(0) as u64)
                    .unwrap_or(1000)
            }
        }
        DotValue::Duration(value) => value
            .raw
            .trim()
            .parse::<i64>()
            .map(|value| value.max(0) as u64)
            .unwrap_or(1000),
        DotValue::Null => 0,
    }
}

fn child_autostart_enabled(attr: Option<&DotAttribute>) -> bool {
    let Some(attr) = attr else {
        return true;
    };
    match &attr.value {
        DotValue::Boolean(value) => *value,
        DotValue::Null => false,
        _ => {
            let normalized = dot_value_text(&attr.value).trim().to_lowercase();
            if false_like(&normalized) {
                false
            } else if matches!(normalized.as_str(), "true" | "1" | "yes" | "on") {
                true
            } else {
                true
            }
        }
    }
}

fn manager_actions(attr: Option<&DotAttribute>) -> BTreeSet<String> {
    let Some(attr) = attr else {
        return ["observe", "wait"]
            .into_iter()
            .map(str::to_string)
            .collect();
    };
    let raw = dot_value_text(&attr.value).trim().to_lowercase();
    if raw.is_empty() {
        return BTreeSet::new();
    }
    raw.split(',')
        .map(str::trim)
        .filter(|action| matches!(*action, "observe" | "steer" | "wait"))
        .map(str::to_string)
        .collect()
}

fn stop_condition(attr: Option<&DotAttribute>) -> String {
    attr.map(|attr| dot_value_text(&attr.value).trim().to_string())
        .unwrap_or_default()
}

fn stop_condition_met(condition: &str, context: &ContextMap) -> bool {
    let Ok(context) = AttractorContext::from_map(context.clone()) else {
        return false;
    };
    evaluate_condition(condition, &Outcome::new(OutcomeStatus::Success), &context)
}

fn authored_attr_text(attr: Option<&DotAttribute>) -> String {
    attr.filter(|attr| attr.line > 0)
        .map(|attr| dot_value_text(&attr.value).trim().to_string())
        .unwrap_or_default()
}

fn resolve_child_workdir_path(
    context: &ContextMap,
    run_workdir: &Path,
    authored_child_workdir: &str,
) -> PathBuf {
    let run_workdir =
        context_path(context, "internal.run_workdir").unwrap_or_else(|| run_workdir.to_path_buf());
    if authored_child_workdir.trim().is_empty() {
        return run_workdir;
    }
    resolve_path_from_base(authored_child_workdir, &run_workdir)
}

fn resolve_child_dot_path(
    child_dotfile: &str,
    child_workdir_path: &Path,
    context: &ContextMap,
    child_workdir_is_authored: bool,
) -> PathBuf {
    let child_dot_path = PathBuf::from(child_dotfile);
    if child_dot_path.is_absolute() {
        return child_dot_path;
    }
    let base_dir = if child_workdir_is_authored {
        child_workdir_path.to_path_buf()
    } else {
        context_path(context, "internal.flow_source_dir")
            .unwrap_or_else(|| child_workdir_path.to_path_buf())
    };
    resolve_path_from_base(child_dot_path, &base_dir)
}

fn resolve_path_from_base(raw_path: impl AsRef<Path>, base_dir: &Path) -> PathBuf {
    let raw_path = raw_path.as_ref();
    if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base_dir.join(raw_path)
    }
}

fn context_path(context: &ContextMap, key: &str) -> Option<PathBuf> {
    non_empty(context_string(context, key)).map(PathBuf::from)
}

fn steer_cooldown_elapsed(
    now: Instant,
    last_steer_at: Option<Instant>,
    cooldown: Duration,
) -> bool {
    cooldown.is_zero() || last_steer_at.is_none_or(|last| now.duration_since(last) >= cooldown)
}

fn child_failure_context(context: &ContextMap) -> String {
    for key in [
        "context.stack.child.failure_reason",
        "context.stack.child.outcome_reason_message",
        "context.stack.child.outcome_reason_code",
    ] {
        let value = context_string(context, key);
        if !value.is_empty() {
            return value;
        }
    }
    if context_string(context, "context.stack.child.outcome").eq_ignore_ascii_case("failure") {
        "child reported failure outcome".to_string()
    } else {
        String::new()
    }
}

fn default_intervention_message(context: &ContextMap, failure_reason: &str) -> String {
    let stage = context_string(context, "context.stack.child.active_stage");
    let child_run_id = context_string(context, "context.stack.child.run_id");
    let mut target = if child_run_id.is_empty() {
        "the child run".to_string()
    } else {
        format!("child run {child_run_id}")
    };
    if !stage.is_empty() {
        target = format!("{target} at {stage}");
    }
    format!("{target} reported a failure: {failure_reason}. Address the failure and continue.")
}

fn auto_steer_limit_message(
    child_run_id: &str,
    target_node_id: Option<&str>,
    failure_reason: &str,
) -> String {
    let mut target = format!("child run {child_run_id}");
    if let Some(target_node_id) = target_node_id {
        target = format!("{target} at {target_node_id}");
    }
    format!("Automatic manager steering already attempted for {target}: {failure_reason}")
}

fn telemetry_payload(context: &ContextMap, node_id: &str, cycle: u64) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("cycle".to_string(), json!(cycle)),
        ("node_id".to_string(), json!(node_id)),
        (
            "timestamp_unix".to_string(),
            json!(unix_timestamp_seconds()),
        ),
        (
            "child_status".to_string(),
            json!(context_value(context, "context.stack.child.status")),
        ),
        (
            "child_outcome".to_string(),
            json!(context_value(context, "context.stack.child.outcome")),
        ),
        (
            "child_active_stage".to_string(),
            json!(context_value(context, "context.stack.child.active_stage")),
        ),
        (
            "child_retry_count".to_string(),
            json!(context_value(context, "context.stack.child.retry_count")),
        ),
        (
            "child_artifact_count".to_string(),
            json!(context_value(context, "context.stack.child.artifact_count")),
        ),
        (
            "child_event_count".to_string(),
            json!(context_value(context, "context.stack.child.event_count")),
        ),
        (
            "child_checkpoint_timestamp".to_string(),
            json!(context_value(
                context,
                "context.stack.child.checkpoint_timestamp"
            )),
        ),
        (
            "child_latest_event_at".to_string(),
            json!(context_value(
                context,
                "context.stack.child.latest_event_at"
            )),
        ),
    ])
}

fn intervention_payload(
    context: &ContextMap,
    node_id: &str,
    cycle: u64,
    intervention_result: &ChildInterventionResult,
    failure_reason: &str,
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("cycle".to_string(), json!(cycle)),
        ("node_id".to_string(), json!(node_id)),
        (
            "timestamp_unix".to_string(),
            json!(unix_timestamp_seconds()),
        ),
        (
            "child_run_id".to_string(),
            json!(intervention_result.run_id.clone()),
        ),
        (
            "child_status".to_string(),
            json!(context_value(context, "context.stack.child.status")),
        ),
        (
            "child_active_stage".to_string(),
            json!(context_value(context, "context.stack.child.active_stage")),
        ),
        (
            "target_node_id".to_string(),
            json!(intervention_result.target_node_id.clone()),
        ),
        (
            "instruction".to_string(),
            json!(context_value(context, "context.stack.child.intervention")),
        ),
        (
            "intervention_status".to_string(),
            json!(context_value(
                context,
                "context.stack.child.intervention_status"
            )),
        ),
        (
            "intervention_delivery_mode".to_string(),
            json!(context_value(
                context,
                "context.stack.child.intervention_delivery_mode"
            )),
        ),
        (
            "intervention_reason".to_string(),
            json!(context_value(
                context,
                "context.stack.child.intervention_reason"
            )),
        ),
        (
            "result_message".to_string(),
            json!(intervention_result.message.clone()),
        ),
        ("failure_reason".to_string(), json!(failure_reason)),
        (
            "child_failure_reason".to_string(),
            json!(context_value(context, "context.stack.child.failure_reason")),
        ),
        (
            "child_outcome_reason_code".to_string(),
            json!(context_value(
                context,
                "context.stack.child.outcome_reason_code"
            )),
        ),
        (
            "child_outcome_reason_message".to_string(),
            json!(context_value(
                context,
                "context.stack.child.outcome_reason_message"
            )),
        ),
    ])
}

fn append_manager_artifact(
    logs_root: Option<&Path>,
    node_id: &str,
    filename: &str,
    payload: BTreeMap<String, Value>,
) {
    let Some(logs_root) = logs_root else {
        return;
    };
    let stage_dir = logs_root.join(node_id);
    if fs::create_dir_all(&stage_dir).is_err() {
        return;
    }
    let path = stage_dir.join(filename);
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    if let Ok(line) = serde_json::to_string(&payload) {
        let _ = writeln!(file, "{line}");
    }
}

fn outcome_with_context_updates(
    outcome: Outcome,
    original: &ContextMap,
    current: &ContextMap,
) -> Outcome {
    let updates = current
        .iter()
        .filter(|(key, value)| {
            key.starts_with("context.stack.child.") && original.get(key.as_str()) != Some(*value)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Outcome {
        context_updates: updates,
        ..outcome
    }
}

fn fail_outcome(reason: impl Into<String>) -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        failure_reason: reason.into(),
        retryable: Some(false),
        failure_kind: Some(FailureKind::Runtime),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}

fn parse_duration_string(raw: &str, default: Duration) -> Option<Duration> {
    let value = raw.trim().to_lowercase();
    if value.is_empty() {
        return Some(default);
    }
    if let Ok(seconds) = value.parse::<f64>() {
        return Some(Duration::from_secs_f64(seconds.max(0.0)));
    }
    for unit in ["ms", "s", "m", "h", "d"] {
        if let Some(number) = value.strip_suffix(unit) {
            if let Ok(amount) = number.parse::<i64>() {
                return duration_from_parts(amount, unit).or(Some(default));
            }
        }
    }
    Some(default)
}

fn duration_from_parts(amount: i64, unit: &str) -> Option<Duration> {
    let amount = amount.max(0) as f64;
    let seconds = match unit {
        "ms" => amount / 1000.0,
        "s" => amount,
        "m" => amount * 60.0,
        "h" => amount * 3600.0,
        "d" => amount * 86_400.0,
        _ => return None,
    };
    Some(Duration::from_secs_f64(seconds))
}

fn false_like(value: &str) -> bool {
    matches!(value, "false" | "0" | "no" | "off")
}

fn context_string(context: &ContextMap, key: &str) -> String {
    context.get(key).map(value_to_string).unwrap_or_default()
}

fn context_value(context: &ContextMap, key: &str) -> Value {
    context
        .get(key)
        .cloned()
        .unwrap_or(Value::String(String::new()))
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn set_context(context: &mut ContextMap, key: impl Into<String>, value: Value) {
    context.insert(key.into(), value);
}

fn generated_child_run_id() -> String {
    let first: u128 = rand::random();
    format!("{first:032x}")
}

fn unix_timestamp_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}
