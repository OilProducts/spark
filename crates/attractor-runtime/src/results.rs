use std::path::{Path, PathBuf};

use attractor_core::{CheckpointState, FlowDefinition, NodeKind, RunResult};
use spark_storage::{read_json_optional, write_json_atomic, write_text_atomic, JsonWriteOptions};

use crate::error::{Result, RuntimeStorageError};
use crate::paths::RunRootPaths;

pub const DEFAULT_RESULT_SUMMARY_PROMPT: &str = "Summarize the following flow result for a user who needs the final outcome, important details, and any follow-up actions. Keep the answer concise and faithful to the source text.";
const SUCCESSFUL_SOURCE_OUTCOMES: &[&str] = &["success", "partial_success"];

pub type ResultSummaryFn = dyn Fn(&str, &str) -> std::result::Result<String, String>;

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
    summarize: Option<&ResultSummaryFn>,
) -> Result<RunResult> {
    let result = match resolve_run_result(paths, run_id, status, flow, checkpoint, summarize) {
        Ok(result) => result,
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
    summarize: Option<&ResultSummaryFn>,
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
    let summary_enabled = flow_attr_bool(flow, "spark.result_summary_enabled", false);
    let mut summary_prompt = flow_attr_text(flow, "spark.result_summary_prompt");
    if summary_enabled && summary_prompt.is_none() {
        summary_prompt = Some(DEFAULT_RESULT_SUMMARY_PROMPT.to_string());
    }

    let mut body_markdown = source_text.clone();
    let mut display_mode = Some("raw".to_string());
    let mut summary_error = None;
    if summary_enabled {
        if let Some(summarize) = summarize {
            match summarize(
                summary_prompt
                    .as_deref()
                    .unwrap_or(DEFAULT_RESULT_SUMMARY_PROMPT),
                &source_text,
            ) {
                Ok(summary) if !summary.trim().is_empty() => {
                    body_markdown = summary;
                    display_mode = Some("summary".to_string());
                }
                Ok(_) => {
                    summary_error =
                        Some("Result summarizer returned an empty response.".to_string());
                }
                Err(error) => {
                    summary_error = Some(error);
                }
            }
        } else {
            summary_error = Some("Result summarizer is unavailable.".to_string());
        }
    }

    Ok(RunResult {
        run_id: run_id.to_string(),
        status: status.to_string(),
        state: "ready".to_string(),
        source_node_id: Some(source_node_id),
        source_artifact_path: Some(source_artifact_path.to_string_lossy().replace('\\', "/")),
        display_mode,
        body_markdown,
        summary_enabled,
        summary_prompt,
        summary_error,
        error: None,
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

fn flow_attr_bool(flow: &FlowDefinition, key: &str, default: bool) -> bool {
    flow_attr_text(flow, key)
        .map(|value| matches!(value.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}
