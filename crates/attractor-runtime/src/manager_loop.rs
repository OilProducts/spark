use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use attractor_core::{
    evaluate_condition, AttractorContext, CheckpointState, ContextMap, FailureKind, FlowDefinition,
    LaunchContext, ManagerLoopConfig, NodeConfig, Outcome, OutcomeStatus, RunRecord,
};
use serde_json::{json, Value};

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

    let manager = manager_config(&runtime);
    let poll_interval = manager_duration(
        manager,
        |config| config.poll_interval.as_deref(),
        Duration::from_secs(45),
    );
    let max_cycles = manager.and_then(|config| config.max_cycles).unwrap_or(1000);
    let stop_condition = manager
        .and_then(|config| config.stop_condition.clone())
        .unwrap_or_default();
    let actions = manager_actions(manager);
    let steer_cooldown = manager_duration(
        manager,
        |config| config.steer_cooldown.as_deref(),
        Duration::ZERO,
    );
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
    let child_flow_ref = child_flow_ref(runtime);
    if child_flow_ref.is_empty() {
        return Ok(None);
    }
    if !child_autostart_enabled(manager_config(runtime)) {
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
    let authored_child_workdir = manager_config(runtime)
        .and_then(|manager| manager.child_workdir.clone())
        .unwrap_or_else(|| flow_metadata_text(&runtime.flow, "stack.child_workdir"));
    let child_workdir_from = manager_config(runtime)
        .and_then(|manager| manager.child_workdir_from.clone())
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    let child_workdir_path = if let Some(workdir_key) = child_workdir_from {
        if !authored_child_workdir.trim().is_empty() {
            return Ok(Some(fail_outcome(
                "Subflow declares both manager.child_workdir and manager.child_workdir_from"
                    .to_string(),
            )));
        }
        match resolve_context_child_workdir(context, &runtime.run_workdir, &workdir_key) {
            Ok(path) => path,
            Err(reason) => return Ok(Some(fail_outcome(reason))),
        }
    } else {
        resolve_child_workdir_path(context, &runtime.run_workdir, &authored_child_workdir)
    };
    let child_flow_path = resolve_child_flow_path(
        &child_flow_ref,
        &child_workdir_path,
        context,
        !authored_child_workdir.is_empty(),
    );
    if !child_flow_path.exists() {
        return Ok(Some(fail_outcome(format!(
            "Child flow file not found: {}",
            child_flow_path.display()
        ))));
    }

    let child_source = match fs::read_to_string(&child_flow_path) {
        Ok(source) => source,
        Err(error) => {
            return Ok(Some(fail_outcome(format!(
                "Unable to read child flow file: {error}"
            ))));
        }
    };
    let child_flow = match FlowDefinition::from_yaml_str(&child_source) {
        Ok(flow) => flow.normalize(),
        Err(error) => {
            return Ok(Some(fail_outcome(format!(
                "Failed to parse child flow YAML: {error}"
            ))));
        }
    };
    if let Err(error) = child_flow.validate() {
        return Ok(Some(fail_outcome(format!(
            "Child flow failed validation: {}",
            error.detail
        ))));
    }

    let child_flow_name = (!child_flow.title.trim().is_empty())
        .then(|| child_flow.title.trim().to_string())
        .or_else(|| {
            child_flow_path
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        })
        .or_else(|| (!child_flow.id.trim().is_empty()).then(|| child_flow.id.clone()))
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
    let input_map = child_input_map(runtime);
    let child_launch_context = mapped_child_context(&parent_context, &input_map);

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
        child_flow: child_flow.clone(),
        child_flow_name: child_flow_name.clone(),
        child_flow_path: child_flow_path.clone(),
        child_workdir: child_workdir_path.clone(),
        input_map,
        parent_context: child_launch_context,
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
    // Child runs inherit the parent's run-event observer so their journal
    // appends reach the live layer too.
    let store = match runner.run_event_observer() {
        Some(observer) => {
            RunStore::for_runs_dir(parent_paths.runs_dir.clone()).with_run_event_observer(observer)
        }
        None => RunStore::for_runs_dir(parent_paths.runs_dir.clone()),
    };
    let start_node = resolve_start_node(&request.child_flow).map_err(|error| error.to_string())?;
    let mut child_context = request.parent_context.clone();
    apply_child_input_map(
        &mut child_context,
        &request.parent_context,
        &request.input_map,
    );
    clear_child_snapshot(&mut child_context);
    for (key, value) in crate::flow_runtime::flow_context_seed(&request.child_flow) {
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
    set_context(
        &mut child_context,
        "internal.runs_dir",
        json!(parent_paths.runs_dir.to_string_lossy().to_string()),
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
    record.project_path = store
        .read_run_record(parent_paths)
        .ok()
        .flatten()
        .map(|parent| parent.project_path)
        .filter(|path| !path.trim().is_empty())
        .unwrap_or_else(|| record.working_directory.clone());
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
    let child_definition_json = request.child_flow.to_canonical_json_string();
    let mut executor = PipelineExecutor::new(runner);
    let result = executor
        .execute(ExecuteRunRequest {
            store,
            record,
            flow: request.child_flow,
            flow_source: Some(child_source.clone()),
            flow_definition_json: Some(child_definition_json),
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

fn manager_config(runtime: &HandlerRuntime) -> Option<&ManagerLoopConfig> {
    runtime
        .flow
        .nodes
        .get(&runtime.node_id)
        .and_then(|node| node.manager.as_ref())
}

fn manager_duration(
    manager: Option<&ManagerLoopConfig>,
    value: impl FnOnce(&ManagerLoopConfig) -> Option<&str>,
    default: Duration,
) -> Duration {
    manager
        .and_then(value)
        .and_then(|value| parse_duration_string(value, default))
        .unwrap_or(default)
}

fn child_autostart_enabled(manager: Option<&ManagerLoopConfig>) -> bool {
    manager
        .and_then(|manager| manager.child_autostart)
        .unwrap_or(true)
}

fn manager_actions(manager: Option<&ManagerLoopConfig>) -> BTreeSet<String> {
    let Some(manager) = manager else {
        return ["observe", "wait"]
            .into_iter()
            .map(str::to_string)
            .collect();
    };
    if manager.actions.is_empty() {
        return BTreeSet::new();
    }
    manager
        .actions
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .map(str::to_lowercase)
        .filter(|action| matches!(action.as_str(), "observe" | "steer" | "wait"))
        .collect()
}

fn stop_condition_met(condition: &str, context: &ContextMap) -> bool {
    let Ok(context) = AttractorContext::from_map(context.clone()) else {
        return false;
    };
    evaluate_condition(condition, &Outcome::new(OutcomeStatus::Success), &context)
}

fn child_flow_ref(runtime: &HandlerRuntime) -> String {
    runtime
        .flow
        .nodes
        .get(&runtime.node_id)
        .and_then(|node| match node.config.as_ref() {
            Some(NodeConfig::Subflow { flow_ref, .. }) => Some(flow_ref.trim().to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn child_input_map(runtime: &HandlerRuntime) -> BTreeMap<String, String> {
    runtime
        .flow
        .nodes
        .get(&runtime.node_id)
        .and_then(|node| match node.config.as_ref() {
            Some(NodeConfig::Subflow { input_map, .. }) => Some(input_map.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn mapped_child_context(
    parent_context: &ContextMap,
    input_map: &BTreeMap<String, String>,
) -> ContextMap {
    let mut child_context = parent_context.clone();
    apply_child_input_map(&mut child_context, parent_context, input_map);
    child_context
}

fn apply_child_input_map(
    child_context: &mut ContextMap,
    parent_context: &ContextMap,
    input_map: &BTreeMap<String, String>,
) {
    for (child_key, parent_path) in input_map {
        let child_key = child_key.trim();
        let parent_path = parent_path.trim();
        if child_key.is_empty() || parent_path.is_empty() {
            continue;
        }
        if let Some(value) = context_path_value(parent_context, parent_path) {
            set_context(child_context, child_key, value);
        }
    }
}

fn flow_metadata_text(flow: &FlowDefinition, key: &str) -> String {
    flow.metadata
        .get(key)
        .or_else(|| flow.extensions.get(key))
        .and_then(value_text)
        .unwrap_or_default()
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
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

fn resolve_context_child_workdir(
    context: &ContextMap,
    run_workdir: &Path,
    workdir_key: &str,
) -> std::result::Result<PathBuf, String> {
    let raw_value = context_path_value(context, workdir_key)
        .as_ref()
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!("manager.child_workdir_from key {workdir_key} did not resolve to a non-empty string")
        })?;
    let run_workdir =
        context_path(context, "internal.run_workdir").unwrap_or_else(|| run_workdir.to_path_buf());
    let candidate = resolve_path_from_base(&raw_value, &run_workdir);
    if !candidate.is_dir() {
        return Err(format!(
            "manager.child_workdir_from directory does not exist: {}",
            candidate.display()
        ));
    }
    let candidate = fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "Unable to canonicalize child working directory {}: {error}",
            candidate.display()
        )
    })?;
    let launch_root = fs::canonicalize(&run_workdir).map_err(|error| {
        format!(
            "Unable to canonicalize run working directory {}: {error}",
            run_workdir.display()
        )
    })?;
    if candidate.starts_with(&launch_root) {
        return Ok(candidate);
    }
    let candidate_repo = git_common_dir(&candidate);
    let launch_repo = git_common_dir(&launch_root);
    match (candidate_repo, launch_repo) {
        (Some(candidate_repo), Some(launch_repo)) if candidate_repo == launch_repo => {
            Ok(candidate)
        }
        _ => Err(format!(
            "manager.child_workdir_from directory {} is outside the run working directory {} and is not a linked worktree of the same repository",
            candidate.display(),
            launch_root.display()
        )),
    }
}

fn git_common_dir(dir: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }
    fs::canonicalize(&text).ok()
}

fn resolve_child_flow_path(
    child_flow_ref: &str,
    child_workdir_path: &Path,
    context: &ContextMap,
    child_workdir_is_authored: bool,
) -> PathBuf {
    let child_flow_path = PathBuf::from(child_flow_ref);
    if child_flow_path.is_absolute() {
        return child_flow_path;
    }
    let base_dir = if child_workdir_is_authored {
        child_workdir_path.to_path_buf()
    } else {
        context_path(context, "internal.flow_source_dir")
            .unwrap_or_else(|| child_workdir_path.to_path_buf())
    };
    resolve_path_from_base(child_flow_path, &base_dir)
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

pub(crate) fn context_path_value(context: &ContextMap, path: &str) -> Option<Value> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    for candidate in flat_path_candidates(path) {
        if let Some(value) = context.get(&candidate) {
            return Some(value.clone());
        }
    }
    for candidate in nested_path_candidates(path) {
        if let Some(value) = lookup_nested_value(context, &candidate) {
            return Some(value.clone());
        }
        if let Some(value) = lookup_flat_prefix_nested_value(context, &candidate) {
            return Some(value.clone());
        }
    }
    None
}

fn flat_path_candidates(path: &str) -> Vec<String> {
    if let Some(stripped) = path.strip_prefix("context.") {
        vec![path.to_string(), stripped.to_string()]
    } else {
        vec![path.to_string(), format!("context.{path}")]
    }
}

fn nested_path_candidates(path: &str) -> Vec<String> {
    if let Some(stripped) = path.strip_prefix("context.") {
        vec![path.to_string(), stripped.to_string()]
    } else {
        vec![path.to_string(), format!("context.{path}")]
    }
}

fn lookup_nested_value<'a>(context: &'a ContextMap, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = context.get(first)?;
    for part in parts {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn lookup_flat_prefix_nested_value<'a>(context: &'a ContextMap, path: &str) -> Option<&'a Value> {
    let parts = path.split('.').collect::<Vec<_>>();
    for split_at in (1..parts.len()).rev() {
        let flat_key = parts[..split_at].join(".");
        let Some(mut current) = context.get(&flat_key) else {
            continue;
        };
        for part in &parts[split_at..] {
            current = current.as_object()?.get(*part)?;
        }
        return Some(current);
    }
    None
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
