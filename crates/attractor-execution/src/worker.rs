use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use attractor_core::{ContextMap, FailureKind, Outcome, OutcomeStatus};
use attractor_runtime::{
    outgoing_routing_edges, NodeExecutionRequest, NodeExecutor, RuntimeHandlerRunner,
};
use serde_json::json;

use crate::protocol::{
    outcome_to_payload, EventFrame, ResultFrame, WorkerFrame, WorkerNodeRequest,
};

pub fn run_worker_node_stdio() -> i32 {
    run_worker_node_from_reader_writer(
        std::io::stdin().lock(),
        std::io::stdout(),
        RuntimeHandlerRunner::new(),
    )
}

pub fn run_worker_node_from_reader_writer<R, W>(
    reader: R,
    writer: W,
    mut runner: RuntimeHandlerRunner,
) -> i32
where
    R: Read,
    W: Write + Send + 'static,
{
    let writer = Arc::new(Mutex::new(writer));
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => return write_process_error(&writer, "worker request missing"),
        Ok(_) => {}
        Err(error) => return write_process_error(&writer, format!("worker read failed: {error}")),
    }

    let request = match serde_json::from_str::<WorkerNodeRequest>(&line) {
        Ok(request) => request,
        Err(error) => {
            return write_process_error(&writer, format!("invalid worker request JSON: {error}"))
        }
    };
    let run_paths = request.run_root.as_ref().and_then(|metadata| {
        attractor_runtime::RunRootPaths::from_existing_root(
            metadata.runs_dir.clone(),
            metadata.project_id.clone(),
            &request.run_id,
            metadata.root.clone(),
        )
        .ok()
    });
    let event_writer = writer.clone();
    runner.set_external_event_sink(move |event| write_event(&event_writer, event));
    let Some(node) = request.flow.nodes.get(&request.node_id).cloned() else {
        let outcome = runtime_failure(format!("Unknown runtime node: {}", request.node_id));
        let _ = write_result(&writer, &outcome, request.context);
        return 0;
    };
    let outgoing_edges = match outgoing_routing_edges(&request.flow, &request.node_id) {
        Ok(edges) => edges,
        Err(error) => {
            let outcome = runtime_failure(error.to_string());
            let _ = write_result(&writer, &outcome, request.context);
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
        run_paths,
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
    let _ = write_result(&writer, &outcome, context);
    0
}

fn write_event<W: Write>(
    writer: &Arc<Mutex<W>>,
    event: attractor_core::RawRuntimeEvent,
) -> attractor_runtime::Result<()> {
    let io_error = |action, source| attractor_runtime::RuntimeStorageError::Io {
        action,
        path: "stdout".into(),
        source,
    };
    let mut writer = writer.lock().map_err(|_| {
        io_error(
            "lock worker stdout",
            std::io::Error::other("worker stdout lock poisoned"),
        )
    })?;
    serde_json::to_writer(&mut *writer, &WorkerFrame::Event(EventFrame { event }))
        .map_err(|error| io_error("write worker event", std::io::Error::other(error)))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| io_error("flush worker event", error))
}

fn write_process_error<W: Write>(writer: &Arc<Mutex<W>>, message: impl AsRef<str>) -> i32 {
    let outcome = runtime_failure(message.as_ref().to_string());
    let _ = write_result(writer, &outcome, ContextMap::new());
    1
}

fn write_result<W: Write>(
    writer: &Arc<Mutex<W>>,
    outcome: &Outcome,
    context: ContextMap,
) -> std::io::Result<()> {
    let mut writer = writer
        .lock()
        .map_err(|_| std::io::Error::other("worker stdout lock poisoned"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn concurrent_worker_events_are_complete_json_before_one_terminal_result() {
        let output = SharedWriter::default();
        let bytes = output.0.clone();
        let writer = Arc::new(Mutex::new(output));
        let threads = (0..32)
            .map(|sequence| {
                let writer = writer.clone();
                std::thread::spawn(move || {
                    write_event(
                        &writer,
                        attractor_core::RawRuntimeEvent {
                            sequence: Some(sequence),
                            event_type: "concurrent".into(),
                            run_id: "run".into(),
                            emitted_at: "2026-07-22T12:00:00Z".into(),
                            payload: BTreeMap::new(),
                        },
                    )
                    .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        write_result(
            &writer,
            &Outcome::new(OutcomeStatus::Success),
            ContextMap::new(),
        )
        .unwrap();

        let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        let frames = text
            .lines()
            .map(|line| serde_json::from_str::<WorkerFrame>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(frames.len(), 33);
        assert!(frames[..32]
            .iter()
            .all(|frame| matches!(frame, WorkerFrame::Event(_))));
        assert!(matches!(frames.last(), Some(WorkerFrame::Result(_))));
        assert_eq!(
            frames
                .iter()
                .filter(|frame| matches!(frame, WorkerFrame::Result(_)))
                .count(),
            1
        );
    }
}
