use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use attractor_core::{OutcomeStatus, RawRuntimeEvent, RunRecord};
use attractor_execution::{
    CommandResult, CommandSpec, ContainerCommandRunner, ContainerizedNodeExecutor, EventFrame,
    ExecutionMode, ExecutionProfile, ExecutionProfileSelection, ResultFrame, WorkerFrame,
};
use attractor_runtime::{
    CreateRunRequest, NodeExecutionRequest, NodeExecutor, RunStore, RuntimeHandlerRunner,
};
use serde_json::json;

#[derive(Clone)]
struct StreamingFake {
    frames: Vec<String>,
    exit_code: i32,
    stderr: String,
    io_failure: bool,
    delivered: Arc<Mutex<usize>>,
    after_first_line: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ContainerCommandRunner for StreamingFake {
    fn command_exists(&self, _: &str) -> bool {
        true
    }

    fn run(&mut self, spec: CommandSpec) -> io::Result<CommandResult> {
        Ok(CommandResult {
            exit_code: 0,
            stdout: if spec.args.first().map(String::as_str) == Some("run") {
                "container-id\n".into()
            } else {
                String::new()
            },
            stderr: String::new(),
        })
    }

    fn run_streaming(
        &mut self,
        _: CommandSpec,
        callback: &mut dyn FnMut(&str),
    ) -> io::Result<CommandResult> {
        if self.io_failure {
            return Err(io::Error::other("stream broke"));
        }
        let mut stdout = String::new();
        for frame in &self.frames {
            callback(frame);
            *self.delivered.lock().unwrap() += 1;
            if *self.delivered.lock().unwrap() == 1 {
                if let Some(after_first_line) = &self.after_first_line {
                    after_first_line();
                }
            }
            stdout.push_str(frame);
            stdout.push('\n');
        }
        Ok(CommandResult {
            exit_code: self.exit_code,
            stdout,
            stderr: self.stderr.clone(),
        })
    }
}

fn event(kind: &str, run_id: &str, sequence: u64) -> String {
    serde_json::to_string(&WorkerFrame::Event(EventFrame {
        event: RawRuntimeEvent {
            sequence: Some(sequence),
            event_type: kind.into(),
            run_id: run_id.into(),
            emitted_at: "2026-07-22T12:00:00Z".into(),
            payload: BTreeMap::from([("node_id".into(), json!("task"))]),
        },
    }))
    .unwrap()
}

fn result() -> String {
    serde_json::to_string(&WorkerFrame::Result(ResultFrame {
        outcome: json!({
            "status": "success", "preferred_label": "", "suggested_next_ids": [],
            "context_updates": {}, "failure_reason": "", "notes": "",
            "retryable": false, "raw_response_text": "ok"
        }),
        context: Default::default(),
    }))
    .unwrap()
}

fn fixture(
    frames: Vec<String>,
) -> (
    tempfile::TempDir,
    ContainerizedNodeExecutor,
    NodeExecutionRequest,
    Arc<Mutex<usize>>,
    Arc<Mutex<Vec<String>>>,
) {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "stream-run";
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let paths = store
        .create_run(CreateRunRequest {
            record: RunRecord::new(run_id, temp.path().to_string_lossy()),
            checkpoint: None,
            manifest: None,
            flow_source: None,
            flow_definition_json: None,
        })
        .unwrap();
    let flow = attractor_dsl::parse_flow_definition(
        r#"
schema_version: "1"
id: stream
nodes:
  task: { kind: start }
  done: { kind: exit }
edges:
  - { from: task, to: done }
"#,
    )
    .unwrap();
    let delivered = Arc::new(Mutex::new(0));
    let notifications = Arc::new(Mutex::new(Vec::new()));
    let observer_notifications = notifications.clone();
    let runner = RuntimeHandlerRunner::new().with_run_event_observer(Arc::new(move |run_id| {
        observer_notifications
            .lock()
            .unwrap()
            .push(run_id.to_string());
    }));
    let profile = ExecutionProfile {
        id: "container".into(),
        label: "Container".into(),
        mode: ExecutionMode::LocalContainer,
        enabled: true,
        image: Some("worker:test".into()),
        capabilities: vec![],
        metadata: BTreeMap::new(),
    };
    let executor = ContainerizedNodeExecutor::new(
        ExecutionProfileSelection {
            profile,
            selected_profile_id: "container".into(),
            selection_source: "test".into(),
        },
        runner,
    )
    .with_command_runner(StreamingFake {
        frames,
        exit_code: 0,
        stderr: String::new(),
        io_failure: false,
        delivered: delivered.clone(),
        after_first_line: None,
    });
    let node = flow.nodes["task"].clone();
    let request = NodeExecutionRequest {
        node_id: "task".into(),
        stage_index: 0,
        context: Default::default(),
        prompt: String::new(),
        node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler("task", &node),
        node,
        flow,
        outgoing_edges: vec![],
        run_paths: Some(paths),
        run_workdir: temp.path().into(),
        run_id: run_id.into(),
        fallback_model: None,
        fallback_provider: None,
        fallback_profile: None,
        fallback_reasoning_effort: None,
    };
    (temp, executor, request, delivered, notifications)
}

#[test]
fn streams_canonical_events_to_host_once_in_order_before_exit_and_notifies() {
    let frames = vec![
        event("first", "worker-run", 41),
        event("second", "worker-run", 42),
        result(),
    ];
    let (_temp, mut executor, request, delivered, notifications) = fixture(frames.clone());
    let paths = request.run_paths.clone().unwrap();
    let live_observed = Arc::new(Mutex::new(false));
    let live_observed_from_runner = live_observed.clone();
    let live_notifications = notifications.clone();
    let live_paths = paths.clone();
    executor = executor.with_command_runner(StreamingFake {
        frames,
        exit_code: 0,
        stderr: String::new(),
        io_failure: false,
        delivered: delivered.clone(),
        after_first_line: Some(Arc::new(move || {
            let persisted = attractor_runtime::read_raw_events(&live_paths).unwrap();
            assert_eq!(persisted.last().unwrap().event_type, "first");
            assert_eq!(&*live_notifications.lock().unwrap(), &["stream-run"]);
            *live_observed_from_runner.lock().unwrap() = true;
        })),
    });
    let outcome = executor.execute(request).unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(*delivered.lock().unwrap(), 3);
    assert!(*live_observed.lock().unwrap());
    let persisted = attractor_runtime::read_raw_events(&paths).unwrap();
    let streamed = &persisted[persisted.len() - 2..];
    assert_eq!(
        streamed
            .iter()
            .map(|e| e.event_type.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(
        streamed.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        [Some(41), Some(42)]
    );
    assert!(streamed
        .iter()
        .all(|e| e.run_id == "worker-run" && e.emitted_at == "2026-07-22T12:00:00Z"));
    assert_eq!(
        &*notifications.lock().unwrap(),
        &["stream-run", "stream-run"]
    );
}

#[test]
fn rejects_every_invalid_stream_shape_with_diagnostics() {
    let cases = [
        (vec!["not-json".into()], "invalid worker protocol frame"),
        (vec![result(), result()], "after its result"),
        (
            vec![result(), event("late", "stream-run", 1)],
            "after its result",
        ),
        (vec![], "without a result payload"),
        (
            vec![serde_json::to_string(&WorkerFrame::HumanGateRequest(
                attractor_execution::HumanGateRequestFrame {
                    question: attractor_runtime::HumanQuestion {
                        text: "?".into(),
                        stage: "task".into(),
                        options: vec![],
                    },
                },
            ))
            .unwrap()],
            "unexpected worker protocol frame",
        ),
    ];
    for (frames, expected) in cases {
        let (_temp, mut executor, request, _, _) = fixture(frames);
        let error = executor.execute(request).unwrap_err();
        assert!(error.message.contains(expected), "{}", error.message);
    }
}

#[test]
fn reports_persistence_streaming_and_nonzero_exit_failures() {
    let (_temp, mut executor, request, _, notifications) =
        fixture(vec![event("x", "stream-run", 1), result()]);
    let events_path = request.run_paths.as_ref().unwrap().events_jsonl();
    std::fs::remove_file(&events_path).unwrap();
    std::fs::create_dir(&events_path).unwrap();
    let error = executor.execute(request).unwrap_err();
    assert!(error.message.contains("worker event ingestion failed"));
    assert!(notifications.lock().unwrap().is_empty());

    let (_temp, mut executor, request, _, _) = fixture(vec![]);
    executor = executor.with_command_runner(StreamingFake {
        frames: vec![],
        exit_code: 0,
        stderr: String::new(),
        io_failure: true,
        delivered: Arc::new(Mutex::new(0)),
        after_first_line: None,
    });
    assert!(executor
        .execute(request)
        .unwrap_err()
        .message
        .contains("stream broke"));

    let (_temp, mut executor, request, _, _) = fixture(vec![result()]);
    executor = executor.with_command_runner(StreamingFake {
        frames: vec![result()],
        exit_code: 17,
        stderr: "worker exploded".into(),
        io_failure: false,
        delivered: Arc::new(Mutex::new(0)),
        after_first_line: None,
    });
    let error = executor.execute(request).unwrap_err();
    assert!(error.message.contains("exit code 17") && error.message.contains("worker exploded"));
}
