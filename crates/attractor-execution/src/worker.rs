use std::io::{BufRead, BufReader, Read, Write};

use attractor_core::{ContextMap, FailureKind, Outcome, OutcomeStatus};
use attractor_runtime::{
    outgoing_routing_edges, NodeExecutionRequest, NodeExecutor, RuntimeHandlerRunner,
};
use serde_json::json;

use crate::protocol::{outcome_to_payload, ResultFrame, WorkerFrame, WorkerNodeRequest};

pub fn run_worker_node_stdio() -> i32 {
    run_worker_node_from_reader_writer(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        RuntimeHandlerRunner::new(),
    )
}

pub fn run_worker_node_from_reader_writer<R, W>(
    reader: R,
    mut writer: W,
    mut runner: RuntimeHandlerRunner,
) -> i32
where
    R: Read,
    W: Write,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => return write_process_error(&mut writer, "worker request missing"),
        Ok(_) => {}
        Err(error) => {
            return write_process_error(&mut writer, format!("worker read failed: {error}"))
        }
    }

    let request = match serde_json::from_str::<WorkerNodeRequest>(&line) {
        Ok(request) => request,
        Err(error) => {
            return write_process_error(
                &mut writer,
                format!("invalid worker request JSON: {error}"),
            )
        }
    };
    let Some(node) = request.flow.nodes.get(&request.node_id).cloned() else {
        let outcome = runtime_failure(format!("Unknown runtime node: {}", request.node_id));
        let _ = write_result(&mut writer, &outcome, request.context);
        return 0;
    };
    let outgoing_edges = match outgoing_routing_edges(&request.flow, &request.node_id) {
        Ok(edges) => edges,
        Err(error) => {
            let outcome = runtime_failure(error.to_string());
            let _ = write_result(&mut writer, &outcome, request.context);
            return 0;
        }
    };
    let run_id = if request.run_id.trim().is_empty() {
        request
            .context
            .get("internal.run_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else {
        request.run_id.clone()
    };
    let node_attrs =
        attractor_runtime::flow_runtime::node_attrs_for_handler(&request.node_id, &node);
    let execution_request = NodeExecutionRequest {
        node_id: request.node_id.clone(),
        stage_index: 0,
        context: request.context.clone(),
        prompt: request.prompt.clone(),
        node,
        node_attrs,
        flow: request.flow.clone(),
        outgoing_edges,
        run_paths: None,
        run_workdir: if request.working_dir.as_os_str().is_empty() {
            ".".into()
        } else {
            request.working_dir.clone()
        },
        run_id,
        fallback_model: None,
        fallback_provider: None,
        fallback_profile: None,
        fallback_reasoning_effort: None,
    };
    let outcome = match runner.execute(execution_request) {
        Ok(outcome) => outcome,
        Err(error) => runtime_failure(error.message),
    };
    let mut context = request.context;
    for (key, value) in &outcome.context_updates {
        if value.is_null() {
            context.remove(key);
        } else {
            context.insert(key.clone(), value.clone());
        }
    }
    let _ = write_result(&mut writer, &outcome, context);
    0
}

fn write_process_error(writer: &mut impl Write, message: impl AsRef<str>) -> i32 {
    let outcome = runtime_failure(message.as_ref().to_string());
    let _ = write_result(writer, &outcome, ContextMap::new());
    1
}

fn write_result(
    writer: &mut impl Write,
    outcome: &Outcome,
    context: ContextMap,
) -> std::io::Result<()> {
    let frame = WorkerFrame::Result(ResultFrame {
        outcome: outcome_to_payload(outcome),
        context,
    });
    serde_json::to_writer(&mut *writer, &frame)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn runtime_failure(message: String) -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        failure_reason: message,
        retryable: Some(false),
        failure_kind: Some(FailureKind::Runtime),
        raw_response_text: json!({"error": "runtime"}).to_string(),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}
