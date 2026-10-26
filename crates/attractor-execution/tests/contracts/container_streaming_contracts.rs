use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use attractor_core::{OutcomeStatus, RawRuntimeEvent, RunRecord};
use attractor_execution::{
    run_worker_node_from_reader_writer, CommandResult, CommandSpec, ContainerCommandRunner,
    ContainerizedNodeExecutor, EventFrame, ExecutionMode, ExecutionProfile,
    ExecutionProfileSelection, ResultFrame, RunRootMetadata, WorkerFrame, WorkerNodeRequest,
};
use attractor_runtime::{
    CreateRunRequest, NodeExecutionRequest, NodeExecutor, RunStore, RuntimeHandlerRunner,
    HANDLER_CODERGEN,
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
    expected_identity: Option<(u64, u64)>,
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
        spec: CommandSpec,
        callback: &mut dyn FnMut(&str),
    ) -> io::Result<CommandResult> {
        if self.io_failure {
            return Err(io::Error::other("stream broke"));
        }
        if let Some(expected) = self.expected_identity {
            let request: attractor_execution::WorkerNodeRequest =
                serde_json::from_str(spec.stdin.trim()).unwrap();
            assert_eq!((request.stage_index, request.attempt), expected);
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
            flow_definition_json: None,
            flow_files: None,
        })
        .unwrap();
    let flow = attractor_dsl::validated_flow(
        serde_json::from_value(json!({
            "schema_version": "1",
            "id": "stream",
            "nodes": { "task": { "kind": "start" }, "done": { "kind": "exit" } },
            "edges": [{ "from": "task", "to": "done" }]
        }))
        .unwrap(),
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
        expected_identity: None,
    });
    let node = flow.nodes["task"].clone();
    let request = NodeExecutionRequest {
        node_id: "task".into(),
        stage_index: 0,
        attempt: 0,
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
    let initial_event_count = attractor_runtime::read_raw_events(&paths).unwrap().len();
    executor = executor.with_command_runner(StreamingFake {
        frames,
        exit_code: 0,
        stderr: String::new(),
        io_failure: false,
        delivered: delivered.clone(),
        after_first_line: Some(Arc::new(move || {
            let persisted = attractor_runtime::read_raw_events(&live_paths).unwrap();
            assert_eq!(persisted.len(), initial_event_count);
            assert_eq!(&*live_notifications.lock().unwrap(), &["stream-run"]);
            *live_observed_from_runner.lock().unwrap() = true;
        })),
        expected_identity: Some((7, 2)),
    });
    let mut request = request;
    request.stage_index = 7;
    request.attempt = 2;
    let outcome = executor.execute(request).unwrap();
    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(*delivered.lock().unwrap(), 3);
    assert!(*live_observed.lock().unwrap());
    assert_eq!(
        attractor_runtime::read_raw_events(&paths).unwrap().len(),
        initial_event_count
    );
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
fn reports_streaming_and_nonzero_exit_failures() {
    let (_temp, mut executor, request, _, _) = fixture(vec![]);
    executor = executor.with_command_runner(StreamingFake {
        frames: vec![],
        exit_code: 0,
        stderr: String::new(),
        io_failure: true,
        delivered: Arc::new(Mutex::new(0)),
        after_first_line: None,
        expected_identity: None,
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
        expected_identity: None,
    });
    let error = executor.execute(request).unwrap_err();
    assert!(error.message.contains("exit code 17") && error.message.contains("worker exploded"));
}

#[test]
fn worker_protocol_preserves_repeated_visit_and_retry_execution_identity() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "worker-identity";
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let paths = store
        .create_run(CreateRunRequest {
            record: RunRecord::new(run_id, temp.path().to_string_lossy()),
            checkpoint: None,
            manifest: None,
            flow_definition_json: None,
            flow_files: None,
        })
        .unwrap();
    let flow = attractor_dsl::validated_flow(
        serde_json::from_value(json!({
            "schema_version": "1",
            "id": "worker-identity",
            "nodes": {
                "start": { "kind": "start" },
                "task": { "kind": "agent_task" },
                "done": { "kind": "exit" }
            },
            "edges": [{ "from": "start", "to": "task" }, { "from": "task", "to": "done" }]
        }))
        .unwrap(),
    )
    .unwrap();

    for (stage_index, attempt) in [(3, 0), (3, 1), (4, 0)] {
        let request = WorkerNodeRequest {
            run_id: run_id.into(),
            flow: flow.clone(),
            node_id: "task".into(),
            stage_index,
            attempt,
            prompt: String::new(),
            context: Default::default(),
            context_logs: Vec::new(),
            logs_root: Some(paths.logs_dir()),
            working_dir: temp.path().into(),
            backend_name: None,
            model: None,
            config_dir: None,
            run_root: Some(RunRootMetadata {
                runs_dir: paths.runs_dir.clone(),
                project_id: paths.project_id.clone(),
                root: paths.root.clone(),
            }),
        };
        let mut runner = RuntimeHandlerRunner::new();
        runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime| {
            let root = runtime
                .run_paths
                .as_ref()
                .unwrap()
                .logs_dir()
                .join(&runtime.node_id)
                .join("executions")
                .join(format!("{}-{}", runtime.stage_index, runtime.attempt));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join("events.jsonl"),
                "{\"type\":\"provider_detail\"}\n",
            )
            .unwrap();
            Ok(attractor_core::Outcome::new(OutcomeStatus::Success))
        });
        let input = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            run_worker_node_from_reader_writer(input.as_slice(), Vec::new(), runner),
            0
        );
    }

    for identity in ["3-0", "3-1", "4-0"] {
        let events = paths
            .logs_dir()
            .join("task/executions")
            .join(identity)
            .join("events.jsonl");
        assert!(events.is_file(), "missing {}", events.display());
    }
    assert!(attractor_runtime::read_raw_events(&paths)
        .unwrap()
        .iter()
        .all(|event| event.event_type != "CodergenAdapter"));
}

#[test]
fn finalization_records_both_removal_failure_types_and_never_retries() {
    struct RemovalFailure {
        launch_error: bool,
        removals: Arc<Mutex<usize>>,
    }
    impl ContainerCommandRunner for RemovalFailure {
        fn command_exists(&self, _: &str) -> bool {
            true
        }
        fn run(&mut self, spec: CommandSpec) -> io::Result<CommandResult> {
            if spec.args[0] == "rm" {
                *self.removals.lock().unwrap() += 1;
                if self.launch_error {
                    return Err(io::Error::other("removal launch failed"));
                }
                return Ok(CommandResult {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "removal failed".into(),
                });
            }
            Ok(CommandResult {
                exit_code: 0,
                stdout: if spec.args[0] == "run" {
                    "container-id".into()
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
            callback(&result());
            Ok(CommandResult {
                exit_code: 0,
                stdout: result(),
                stderr: String::new(),
            })
        }
    }
    for launch_error in [false, true] {
        let (_temp, executor, request, _, _) = fixture(vec![]);
        let removals = Arc::new(Mutex::new(0));
        let mut executor = executor
            .keep_container_open()
            .with_command_runner(RemovalFailure {
                launch_error,
                removals: removals.clone(),
            });
        assert_eq!(
            executor.execute(request).unwrap().status,
            OutcomeStatus::Success
        );
        assert_eq!(*removals.lock().unwrap(), 0);
        executor.finalize();
        assert!(executor.take_cleanup_error().unwrap().contains("removal"));
        executor.finalize();
        drop(executor);
        assert_eq!(*removals.lock().unwrap(), 1);
    }
}

/// Asks one question through the flow clarification handler and answers
/// with what it receives, as an agent backend inside a worker would.
struct AskingBackend(Option<spark_agent_adapter::codergen::ClarificationHandler>);

impl spark_agent_adapter::CodergenBackend for AskingBackend {
    fn set_clarification_handler(
        &mut self,
        handler: spark_agent_adapter::codergen::ClarificationHandler,
    ) {
        self.0 = Some(handler);
    }

    fn run(
        &mut self,
        _: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let handler = self
            .0
            .as_ref()
            .expect("worker sets a clarification handler");
        let mut poll = handler(json!({"agent_request_id": "q1", "questions": [
            {"id": "question-1", "question": "Which audience?", "options": []}
        ]}))
        .map_err(spark_agent_adapter::CodergenError::Backend)?;
        let answers = loop {
            if let Some(answers) = poll().map_err(spark_agent_adapter::CodergenError::Backend)? {
                break answers;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text(format!(
                "Audience: {}",
                answers["question-1"]
            )),
            events: Vec::new(),
            usage: None,
            resume_metadata: Default::default(),
        })
    }
}

#[test]
fn worker_clarification_reaches_the_host_journal_and_reads_its_answer() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "worker-clarify";
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut record = RunRecord::new(run_id, temp.path().to_string_lossy());
    record.status = "running".into();
    let paths = store
        .create_run(CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    let flow = attractor_dsl::validated_flow(
        serde_json::from_value(json!({
            "schema_version": "1",
            "id": "worker-clarify",
            "nodes": {
                "start": { "kind": "start" },
                "task": {
                    "kind": "agent_task",
                    "config": {
                        "kind": "agent_task",
                        "prompt": "Pick an audience"
                    }
                },
                "done": { "kind": "exit" }
            },
            "edges": [{ "from": "start", "to": "task" }, { "from": "task", "to": "done" }]
        }))
        .unwrap(),
    )
    .unwrap();
    let request = WorkerNodeRequest {
        run_id: run_id.into(),
        flow,
        node_id: "task".into(),
        stage_index: 1,
        attempt: 0,
        prompt: "Pick an audience".into(),
        context: Default::default(),
        context_logs: Vec::new(),
        logs_root: Some(paths.logs_dir()),
        working_dir: temp.path().into(),
        backend_name: None,
        model: None,
        config_dir: None,
        run_root: Some(RunRootMetadata {
            runs_dir: paths.runs_dir.clone(),
            project_id: paths.project_id.clone(),
            root: paths.root.clone(),
        }),
    };
    let input = serde_json::to_vec(&request).unwrap();
    let frames = SharedFrames::default();
    let worker_frames = frames.clone();
    let worker = std::thread::spawn(move || {
        let runner = RuntimeHandlerRunner::new()
            .with_codergen_backend_factory(|| Box::new(AskingBackend(None)));
        run_worker_node_from_reader_writer(input.as_slice(), worker_frames, runner)
    });

    // The host reads questions from its own journal, not the worker's frames.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let question = loop {
        if let Some(event) = attractor_runtime::read_raw_events(&paths)
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == "human_gate")
        {
            break event;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no human_gate in host journal"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    assert_eq!(question.payload["origin"], "agent_clarification");
    assert_eq!(
        store.read_run_record(&paths).unwrap().unwrap().status,
        "waiting"
    );
    let question = json!(question.payload);
    let mut answer = attractor_runtime::human_gate_answered_event(
        run_id,
        question["question_id"].as_str().unwrap(),
        Some("task".into()),
        None,
        None,
        "Developers",
        None,
    );
    answer.payload.insert(
        "clarification_id".into(),
        question["clarification_id"].clone(),
    );
    attractor_runtime::clarification::accept_answer(&store, &paths, &question, answer).unwrap();

    assert_eq!(worker.join().unwrap(), 0);
    let output = String::from_utf8(frames.0.lock().unwrap().clone()).unwrap();
    let frames = output
        .lines()
        .map(|line| serde_json::from_str::<WorkerFrame>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(frames.iter().any(|frame| matches!(
        frame,
        WorkerFrame::Event(EventFrame { event }) if event.event_type == "human_gate"
    )));
    let Some(WorkerFrame::Result(result)) = frames.last() else {
        panic!("worker ended without a result: {output}");
    };
    assert_eq!(result.outcome["status"], "success", "{output}");
    assert_eq!(
        store.read_run_record(&paths).unwrap().unwrap().status,
        "running"
    );
}

#[derive(Clone, Default)]
struct SharedFrames(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedFrames {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
