use std::path::{Path, PathBuf};

use attractor_core::{CheckpointState, FlowDefinition, NodeConfig, NodeKind, RunResult};
use spark_storage::{read_json_optional, write_json_atomic, write_text_atomic, JsonWriteOptions};

use crate::error::{Result, RuntimeStorageError};
use crate::paths::RunRootPaths;

pub const DEFAULT_RESULT_SUMMARY_PROMPT: &str = "You are writing the final result of a Spark flow run from its recorded artifacts in the current working directory. Read run.json, events.jsonl, checkpoint.json, and the per-node records under logs/ (prompt.md, response.md, status.json, tool_output.txt) as needed. Report concisely, in markdown: what the flow accomplished; what was actually built or changed (branches, commits, validation outcomes); deviations from the requested task; blockers or issues encountered, especially when the run failed; and anything else the user should know. Be faithful to the artifacts and do not speculate beyond them. Do not modify any files. Respond with only the markdown report.";
const SUCCESSFUL_SOURCE_OUTCOMES: &[&str] = &["success", "partial_success"];

/// The prompt and outcome of one summarizer-agent attempt.
pub type ResultSummaryAttempt = (String, std::result::Result<String, String>);

/// Resolves whether this terminal run wants a summarized result and with
/// which prompt. Completed runs honor the exit node they reached; failed
/// runs, which never reach an exit, honor any opted-in exit.
pub fn result_summary_request(
    flow: &FlowDefinition,
    checkpoint: &CheckpointState,
    status: &str,
) -> Option<String> {
    let reached = flow
        .nodes
        .get(&checkpoint.current_node)
        .and_then(exit_summary_prompt);
    if let Some(prompt) = reached {
        return Some(prompt);
    }
    if status == "failed" {
        return flow.nodes.values().find_map(exit_summary_prompt);
    }
    None
}

fn exit_summary_prompt(node: &attractor_core::FlowNode) -> Option<String> {
    match node.config.as_ref() {
        Some(NodeConfig::Exit {
            result_summary: true,
            result_summary_prompt,
        }) => Some(
            result_summary_prompt
                .as_deref()
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .unwrap_or(DEFAULT_RESULT_SUMMARY_PROMPT)
                .to_string(),
        ),
        _ => None,
    }
}

/// Builds the result for a failed run. Without a summary attempt this is the
/// bare failure record; a successful summary upgrades it to a readable
/// report while preserving the failure reason.
pub fn failed_run_result(
    run_id: &str,
    failure_reason: &str,
    summary: Option<ResultSummaryAttempt>,
) -> RunResult {
    let mut result = RunResult {
        run_id: run_id.to_string(),
        status: "failed".to_string(),
        state: "error".to_string(),
        error: Some(failure_reason.to_string()),
        ..RunResult::default()
    };
    match summary {
        Some((prompt, Ok(body))) => {
            result.state = "ready".to_string();
            result.display_mode = Some("summary".to_string());
            result.body_markdown = body;
            result.summary_enabled = true;
            result.summary_prompt = Some(prompt);
        }
        Some((prompt, Err(summary_error))) => {
            result.summary_enabled = true;
            result.summary_prompt = Some(prompt);
            result.summary_error = Some(summary_error);
        }
        None => {}
    }
    result
}

pub fn read_materialized_run_result(paths: &RunRootPaths) -> Result<Option<RunResult>> {
    let Some(mut result) = read_json_optional::<RunResult>(paths.result_json())? else {
        return Ok(None);
    };
    if paths.result_markdown().exists() {
        result.body_markdown =
            std::fs::read_to_string(paths.result_markdown()).map_err(|source| {
                RuntimeStorageError::io("read result markdown", paths.result_markdown(), source)
            })?;
    }
    Ok(Some(result))
}

pub fn write_run_result(paths: &RunRootPaths, result: &RunResult) -> Result<()> {
    write_text_atomic(paths.result_markdown(), &result.body_markdown)?;
    write_json_atomic(paths.result_json(), result, JsonWriteOptions::default())?;
    Ok(())
}

pub fn materialize_run_result(
    paths: &RunRootPaths,
    run_id: &str,
    status: &str,
    flow: &FlowDefinition,
    checkpoint: &CheckpointState,
    summary: Option<ResultSummaryAttempt>,
) -> Result<RunResult> {
    if let Some((prompt, Ok(body))) = summary {
        let result = RunResult {
            run_id: run_id.to_string(),
            status: status.to_string(),
            state: "ready".to_string(),
            display_mode: Some("summary".to_string()),
            body_markdown: body,
            summary_enabled: true,
            summary_prompt: Some(prompt),
            ..RunResult::default()
        };
        write_run_result(paths, &result)?;
        return Ok(result);
    }
    let summary_failure = match summary {
        Some((prompt, Err(summary_error))) => Some((prompt, summary_error)),
        _ => None,
    };
    let result = match resolve_run_result(paths, run_id, status, flow, checkpoint) {
        Ok(mut result) => {
            if let Some((prompt, summary_error)) = summary_failure {
                result.summary_enabled = true;
                result.summary_prompt = Some(prompt);
                result.summary_error = Some(summary_error);
            }
            result
        }
        Err(error) => RunResult {
            run_id: run_id.to_string(),
            status: status.to_string(),
            state: "error".to_string(),
            error: Some(error.to_string()),
            ..RunResult::default()
        },
    };
    write_run_result(paths, &result)?;
    Ok(result)
}

fn resolve_run_result(
    paths: &RunRootPaths,
    run_id: &str,
    status: &str,
    flow: &FlowDefinition,
    checkpoint: &CheckpointState,
) -> Result<RunResult> {
    let Some(source_node_id) = select_source_node_id(flow, checkpoint) else {
        return Ok(RunResult::unavailable(run_id, status));
    };
    let Some(source_artifact_path) = source_artifact_path(paths, &source_node_id) else {
        return Ok(RunResult::unavailable(run_id, status));
    };

    let source_text =
        std::fs::read_to_string(paths.root.join(&source_artifact_path)).map_err(|source| {
            RuntimeStorageError::io(
                "read result source artifact",
                paths.root.join(&source_artifact_path),
                source,
            )
        })?;

    Ok(RunResult {
        run_id: run_id.to_string(),
        status: status.to_string(),
        state: "ready".to_string(),
        source_node_id: Some(source_node_id),
        source_artifact_path: Some(source_artifact_path.to_string_lossy().replace('\\', "/")),
        display_mode: Some("raw".to_string()),
        body_markdown: source_text,
        ..RunResult::default()
    })
}

fn select_source_node_id(flow: &FlowDefinition, checkpoint: &CheckpointState) -> Option<String> {
    if let Some(explicit) = flow_attr_text(flow, "spark.result_node") {
        return source_node_is_valid(flow, checkpoint, &explicit).then_some(explicit);
    }

    let exit_predecessors = flow
        .edges
        .iter()
        .filter(|edge| {
            flow.nodes
                .get(&edge.to)
                .is_some_and(|node| node.kind == NodeKind::Exit)
                && (checkpoint.current_node.is_empty()
                    || edge.to == checkpoint.current_node
                    || !flow.nodes.contains_key(&checkpoint.current_node))
        })
        .map(|edge| edge.from.clone())
        .collect::<Vec<_>>();

    for node_id in checkpoint.completed_nodes.iter().rev() {
        if !exit_predecessors.contains(node_id) {
            continue;
        }
        if source_node_is_valid(flow, checkpoint, node_id) {
            return Some(node_id.clone());
        }
    }
    checkpoint
        .completed_nodes
        .iter()
        .rev()
        .find(|node_id| source_node_is_valid(flow, checkpoint, node_id))
        .cloned()
}

fn source_node_is_valid(
    flow: &FlowDefinition,
    checkpoint: &CheckpointState,
    node_id: &str,
) -> bool {
    let Some(node) = flow.nodes.get(node_id) else {
        return false;
    };
    if matches!(node.kind, NodeKind::Start | NodeKind::Exit) {
        return false;
    }
    let outcomes = checkpoint.context.get("_attractor.node_outcomes");
    if let Some(outcomes) = outcomes.and_then(|value| value.as_object()) {
        return outcomes
            .get(node_id)
            .and_then(|value| value.as_str())
            .is_some_and(|outcome| SUCCESSFUL_SOURCE_OUTCOMES.contains(&outcome));
    }
    checkpoint
        .completed_nodes
        .iter()
        .any(|item| item == node_id)
}

fn source_artifact_path(paths: &RunRootPaths, node_id: &str) -> Option<PathBuf> {
    [
        Path::new("logs").join(node_id).join("response.md"),
        Path::new(node_id).join("response.md"),
    ]
    .into_iter()
    .find(|candidate| {
        let path = paths.root.join(candidate);
        path.exists() && path.is_file()
    })
}

fn flow_attr_text(flow: &FlowDefinition, key: &str) -> Option<String> {
    crate::flow_runtime::flow_attr_text(flow, key)
}
