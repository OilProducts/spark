use std::fs;
use std::path::Path;

use attractor_api::{handle_attractor_request, AttractorApiService, PipelineStartRequest};
use attractor_runtime::RunStore;
use serde_json::json;
use spark_common::settings::SparkSettings;

#[test]
fn start_pipeline_from_flow_content_persists_run_and_returns_launch_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project_path = temp.path().join("Project Lifecycle");
    let service = AttractorApiService::new(settings.clone());

    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-api-start".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        model: Some("compat-model".to_string()),
        llm_provider: Some("codex".to_string()),
        launch_context: Some(
            [("context.topic".to_string(), json!("api"))]
                .into_iter()
                .collect(),
        ),
        spec_id: Some("spec-1".to_string()),
        plan_id: Some("plan-1".to_string()),
        ..PipelineStartRequest::default()
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("started"));
    assert_eq!(response.body["pipeline_id"], json!("run-api-start"));
    assert_eq!(response.body["model"], json!("compat-model"));
    assert_eq!(response.body["llm_provider"], json!("codex"));
    assert_eq!(response.body["execution_mode"], json!("native"));
    assert_eq!(response.body["execution_profile_id"], json!("native"));
    assert_eq!(response.body["diagnostics"], json!([]));
    assert_eq!(response.body["errors"], json!([]));

    let bundle = RunStore::for_settings(&settings)
        .read_run_bundle("run-api-start")
        .expect("read run")
        .expect("run exists");
    let metadata_event = bundle
        .raw_events
        .iter()
        .find(|event| event.event_type == "run_meta")
        .expect("run metadata event");
    assert_eq!(
        metadata_event.payload["flow_source_path"],
        json!(bundle
            .paths
            .artifacts_dir()
            .join("flow/flow-source.yaml")
            .to_string_lossy()
            .to_string())
    );
    assert_eq!(
        metadata_event.payload["flow_definition_path"],
        json!(bundle
            .paths
            .artifacts_dir()
            .join("flow/flow-definition.json")
            .to_string_lossy()
            .to_string())
    );
    let metadata_journal_entry = bundle
        .journal
        .iter()
        .find(|entry| entry.raw_type == "run_meta")
        .expect("run metadata journal entry");
    assert_eq!(
        metadata_journal_entry.payload["flow_source_path"],
        metadata_event.payload["flow_source_path"]
    );
    let record = bundle.record.expect("record");
    assert_eq!(record.status, "completed");
    assert_eq!(record.spec_id.as_deref(), Some("spec-1"));
    assert_eq!(record.plan_id.as_deref(), Some("plan-1"));
    assert_eq!(record.execution_mode, "native");
    assert_eq!(
        record.launch_context,
        Some(
            [("context.topic".to_string(), json!("api"))]
                .into_iter()
                .collect()
        )
    );

    let checkpoint = bundle.checkpoint.expect("checkpoint");
    assert_eq!(checkpoint.context["context.topic"], json!("api"));
    assert_eq!(
        checkpoint.context["_attractor.runtime.launch_model"],
        json!("compat-model")
    );
    assert_eq!(
        checkpoint.context["_attractor.runtime.launch_provider"],
        json!("codex")
    );
    assert_eq!(
        checkpoint.context["internal.run_id"],
        json!("run-api-start")
    );
    assert_eq!(
        checkpoint.context["internal.root_run_id"],
        json!("run-api-start")
    );
    assert!(bundle.paths.run_json().is_file());
    assert!(bundle.paths.checkpoint_json().is_file());
    assert!(bundle.paths.events_jsonl().is_file());
    assert!(bundle
        .paths
        .artifacts_dir()
        .join("flow/flow-source.yaml")
        .is_file());
    assert!(bundle
        .paths
        .artifacts_dir()
        .join("flow/flow-definition.json")
        .is_file());
    assert!(bundle.paths.result_json().is_file());
}

#[test]
fn start_pipeline_uses_launch_context_llm_selection_before_graph_defaults() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    fs::write(
        settings.config_dir.join("llm-profiles.toml"),
        r#"[profiles.launch-profile]
provider = "openai_compatible"
base_url = "http://localhost:4000/v1"
models = ["launch-model"]
"#,
    )
    .unwrap();
    let project_path = temp.path().join("Project Launch Context");
    let service = AttractorApiService::new(settings.clone());

    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-api-launch-context".to_string()),
        flow_content: Some(
            r#"schema_version: "1"
id: api_launch_context
title: API Launch Context
defaults:
  llm_model: graph-model
  llm_provider: Anthropic
  llm_profile: graph-profile
nodes:
  start:
    kind: start
  done:
    kind: exit
edges:
  - from: start
    to: done
"#
            .to_string(),
        ),
        working_directory: project_path.to_string_lossy().to_string(),
        launch_context: Some(
            [
                (
                    unified_llm_adapter::RUNTIME_LAUNCH_MODEL_KEY.to_string(),
                    json!("launch-model"),
                ),
                (
                    unified_llm_adapter::RUNTIME_LAUNCH_PROVIDER_KEY.to_string(),
                    json!("Gemini"),
                ),
                (
                    unified_llm_adapter::RUNTIME_LAUNCH_PROFILE_KEY.to_string(),
                    json!("launch-profile"),
                ),
                (
                    unified_llm_adapter::RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
                    json!("HIGH"),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        ..PipelineStartRequest::default()
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["model"], json!("launch-model"));
    assert_eq!(response.body["llm_provider"], json!("gemini"));
    assert_eq!(response.body["llm_profile"], json!("launch-profile"));
    assert_eq!(response.body["reasoning_effort"], json!("high"));

    let bundle = RunStore::for_settings(&settings)
        .read_run_bundle("run-api-launch-context")
        .expect("read run")
        .expect("run exists");
    let record = bundle.record.expect("record");
    assert_eq!(record.model, "launch-model");
    assert_eq!(record.llm_provider, "gemini");
    assert_eq!(record.llm_profile.as_deref(), Some("launch-profile"));
    assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
    let checkpoint = bundle.checkpoint.expect("checkpoint");
    assert_eq!(
        checkpoint.context[unified_llm_adapter::RUNTIME_LAUNCH_MODEL_KEY],
        json!("launch-model")
    );
    assert_eq!(
        checkpoint.context[unified_llm_adapter::RUNTIME_LAUNCH_PROVIDER_KEY],
        json!("gemini")
    );
    assert_eq!(
        checkpoint.context[unified_llm_adapter::RUNTIME_LAUNCH_REASONING_EFFORT_KEY],
        json!("high")
    );
}

#[test]
fn start_pipeline_reports_validation_errors_without_creating_duplicate_or_invalid_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Validation");
    let service = AttractorApiService::new(settings.clone());

    let first = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-duplicate".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(first.body["status"], json!("started"));

    let duplicate = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-duplicate".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(
        duplicate.body,
        json!({"status": "validation_error", "error": "Run id already exists: run-duplicate"})
    );

    let missing_content = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(
        missing_content.body,
        json!({"status": "validation_error", "error": "Either flow_content or flow_name is required."})
    );

    let parse_error = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-parse-error".to_string()),
        flow_content: Some("nodes: [".to_string()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(parse_error.body["status"], json!("validation_error"));
    assert!(parse_error.body["errors"].as_array().expect("errors").len() == 1);
    assert_eq!(
        parse_error.body["errors"][0]["rule_id"],
        json!("parse_error")
    );

    let validation_error = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-validation-error".to_string()),
        flow_content: Some(
            r#"schema_version: "1"
id: broken
nodes:
  start:
    kind: start
  done:
    kind: exit
edges:
  - from: start
    to: missing
"#
            .to_string(),
        ),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(validation_error.body["status"], json!("validation_error"));
    assert_eq!(
        validation_error.body["errors"][0]["rule_id"],
        json!("edge_target")
    );
    assert_eq!(
        validation_error.body["errors"][0]["edge"],
        json!(["start", "missing"])
    );

    let launch_context_error = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-bad-context".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        launch_context: Some(
            [("internal.bad".to_string(), json!(true))]
                .into_iter()
                .collect(),
        ),
        ..PipelineStartRequest::default()
    });
    assert_eq!(
        launch_context_error.body["status"],
        json!("validation_error")
    );
    assert!(launch_context_error.body["error"]
        .as_str()
        .expect("error")
        .contains("launch_context key must use the context.* namespace"));
}

#[test]
fn mounted_start_route_accepts_current_json_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Route");
    let body = json!({
        "run_id": "run-mounted-start",
        "flow_content": simple_flow(),
        "working_directory": project_path,
        "model": "compat-model"
    })
    .to_string();

    let response = handle_attractor_request("POST", "/attractor/pipelines", &body, settings);

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("started"));
    assert_eq!(response.body["pipeline_id"], json!("run-mounted-start"));
}

fn simple_flow() -> String {
    r#"schema_version: "1"
id: api_lifecycle
title: API Lifecycle
nodes:
  start:
    kind: start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Write a lifecycle note
  done:
    kind: exit
edges:
  - from: start
    to: task
  - from: task
    to: done
"#
    .to_string()
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("project"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("spark-home/flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

fn wait_for_terminal_status(settings: &SparkSettings, run_id: &str) -> String {
    let store = RunStore::for_settings(settings);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let status = store
            .read_run_bundle(run_id)
            .ok()
            .flatten()
            .and_then(|bundle| bundle.record)
            .map(|record| record.status);
        if let Some(status) = status.as_deref() {
            if matches!(status, "completed" | "failed" | "canceled" | "paused") {
                return status.to_string();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "run {run_id} never reached a terminal status (last: {status:?})",
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn detached_start_returns_immediately_with_a_running_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project_path = temp.path().join("Project Detached");
    let service = AttractorApiService::new(settings.clone());

    let response = service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-detached".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        model: Some("compat-model".to_string()),
        ..PipelineStartRequest::default()
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("started"));
    assert_eq!(response.body["run_id"], json!("run-detached"));
    assert_eq!(response.body["terminal_status"], json!("running"));

    // The run record and initial journal exist the moment the response is
    // built, even if the background executor has not progressed yet.
    let bundle = RunStore::for_settings(&settings)
        .read_run_bundle("run-detached")
        .expect("read run")
        .expect("run exists");
    let record = bundle.record.expect("record");
    // Content launches take the flow title as their display identity.
    assert_eq!(record.flow_name, "API Lifecycle");
    assert!(bundle
        .raw_events
        .iter()
        .any(|event| event.event_type == "lifecycle"));

    assert_eq!(
        wait_for_terminal_status(&settings, "run-detached"),
        "completed"
    );
}

#[test]
fn retry_route_executes_the_prepared_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project_path = temp.path().join("Project Retry Exec");
    let service = AttractorApiService::new(settings.clone());

    // Seed a failed run with a stored graph source and checkpoint.
    let store = RunStore::for_settings(&settings);
    let mut record = attractor_core::RunRecord::new(
        "run-retry-exec",
        project_path.to_string_lossy().to_string(),
    );
    record.execution_profile_id = Some("native".to_string());
    record.flow_name = "retry-exec.yaml".to_string();
    record.status = "failed".to_string();
    let flow = attractor_dsl::parse_flow_definition(&simple_flow()).expect("flow");
    let checkpoint = attractor_core::CheckpointState {
        timestamp: "2026-07-08T10:00:00Z".to_string(),
        current_node: "start".to_string(),
        completed_nodes: Vec::new(),
        context: Default::default(),
        retry_counts: Default::default(),
        logs: Vec::new(),
    };
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            checkpoint: Some(checkpoint),
            manifest: None,
            flow_source: Some(simple_flow()),
            flow_definition_json: Some(flow.to_canonical_json_string()),
        })
        .expect("seed failed run");

    store.write_node_artifacts(&paths, "start", 0, 0, &attractor_runtime::NodeArtifacts {
        response: Some("persisted failure".to_string()),
        status: Some(json!({"outcome": "fail", "preferred_label": "", "suggested_next_ids": [], "context_updates": {}, "notes": "", "failure_reason": "old failure", "retryable": false})),
        under_logs: true,
        ..Default::default()
    }).expect("old failure artifacts");

    let response = service.retry_pipeline_route("run-retry-exec");
    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("started"));

    assert_eq!(
        wait_for_terminal_status(&settings, "run-retry-exec"),
        "completed",
        "retry must actually execute the prepared run",
    );
}

#[test]
fn launch_defaults_follow_precedence_and_capture_profiles_without_mutating_history() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    let core = settings.config_dir.join("spark.toml");
    fs::write(&core, "[models]\nprovider = \"codex\"\nmodel = \"workspace-model\"\nreasoning_effort = \"high\"\n").unwrap();
    let project = temp.path().join("project");
    let project = project.to_str().unwrap();
    let metadata = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .project_paths(project)
        .unwrap()
        .project_file;
    fs::create_dir_all(metadata.parent().unwrap()).unwrap();
    fs::write(
        &metadata,
        "[model_settings]\nprovider = \"codex\"\nmodel = \"project-model\"\n",
    )
    .unwrap();
    let service = AttractorApiService::new(settings.clone());
    let flow = "schema_version: '1'\nid: defaults\nnodes:\n  start: {kind: start}\n  end: {kind: exit}\nedges:\n  - {from: start, to: end}\n";
    for (id, explicit, context, authored, expected) in [
        (
            "explicit",
            Some("explicit-model"),
            Some("context-model"),
            Some("flow-model"),
            "explicit-model",
        ),
        (
            "context",
            None,
            Some("context-model"),
            Some("flow-model"),
            "context-model",
        ),
        ("flow", None, None, Some("flow-model"), "flow-model"),
        ("project", None, None, None, "project-model"),
    ] {
        let response = service.start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some(id.into()),
            working_directory: project.into(),
            flow_content: Some(format!(
                "{flow}{}",
                authored
                    .map(|model| format!("defaults:\n  llm_model: {model}\n"))
                    .unwrap_or_default()
            )),
            model: explicit.map(Into::into),
            launch_context: context.map(|model| {
                [(
                    unified_llm_adapter::RUNTIME_LAUNCH_MODEL_KEY.into(),
                    json!(model),
                )]
                .into()
            }),
            ..Default::default()
        });
        assert_eq!(response.body["status"], "started", "{response:?}");
        assert_eq!(response.body["model"], expected);
        let bundle = RunStore::for_settings(&settings)
            .read_run_bundle(id)
            .unwrap()
            .unwrap();
        let context = bundle.checkpoint.unwrap().context;
        assert_eq!(
            context["internal.model_defaults_snapshot"]["source"],
            "project"
        );
        assert_eq!(
            context["internal.execution_profile_snapshot"]["profile"]["id"],
            "native"
        );
        assert_eq!(
            unified_llm_adapter::resolve_effective_llm_model(
                &unified_llm_adapter::LlmResolutionInputs {
                    node_model: Some("node-model".into()),
                    ..Default::default()
                },
                &context
            )
            .as_deref(),
            Some("node-model")
        );
    }
    fs::write(&metadata, "").unwrap();
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("workspace".into()),
        flow_content: Some(flow.into()),
        working_directory: project.into(),
        ..Default::default()
    });
    assert_eq!(response.body["model"], "workspace-model");
    assert_eq!(response.body["reasoning_effort"], "high");
    fs::write(&core, "[models]\nllm_profile = \"local\"\n").unwrap();
    let profile_file = settings.config_dir.join("llm-profiles.toml");
    fs::write(&profile_file, "[profiles.local]\nprovider = \"openai_compatible\"\nbase_url = \"http://localhost:4000/v1\"\nmodels = [\"local-model\"]\ndefault_model = \"local-model\"\n").unwrap();
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("profile".into()),
        flow_content: Some(flow.into()),
        working_directory: project.into(),
        ..Default::default()
    });
    assert_eq!(response.body["status"], "started");
    assert_eq!(response.body["llm_profile"], "local");
    assert_eq!(response.body["llm_provider"], "openai_compatible");
    fs::write(profile_file, "invalid = [").unwrap();
    let bundle = RunStore::for_settings(&settings)
        .read_run_bundle("profile")
        .unwrap()
        .unwrap();
    assert_eq!(
        bundle.checkpoint.unwrap().context["internal.llm_profiles_snapshot"]["local"]["base_url"],
        "http://localhost:4000/v1"
    );
    let previous = RunStore::for_settings(&settings)
        .read_run_bundle("project")
        .unwrap()
        .unwrap();
    assert_eq!(previous.record.unwrap().model, "project-model");
    let invalid = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        flow_content: Some(flow.into()),
        working_directory: project.into(),
        ..Default::default()
    });
    assert_eq!(invalid.body["status"], "validation_error");
}

#[test]
fn workflow_executes_with_captured_profile_after_profile_file_changes() {
    workflow_executes_with_captured_connection(true);
}

#[test]
fn workflow_executes_with_captured_provider_after_core_file_changes() {
    workflow_executes_with_captured_connection(false);
}

fn workflow_executes_with_captured_connection(profile: bool) {
    use std::io::{BufRead, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let profiles = settings.config_dir.join(if profile {
        "llm-profiles.toml"
    } else {
        "spark.toml"
    });
    if profile {
        fs::write(&profiles, format!("[profiles.captured]\nprovider='openai_compatible'\nbase_url='http://{address}/v1'\nmodels=['captured-model']\ndefault_model='captured-model'\n")).unwrap();
        fs::write(
            settings.config_dir.join("spark.toml"),
            "[models]\nllm_profile = 'captured'\n",
        )
        .unwrap();
    } else {
        fs::write(&profiles, format!("[models]\nprovider='openai_compatible'\nmodel='captured-model'\n[providers.openai_compatible]\nbase_url='http://{address}/v1'\n[agents]\nmax_turns=5\n")).unwrap();
    }
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        Arc::new(move || {
            // The factory runs after preparation, before executing the first node.
            fs::write(&profiles, "invalid = SECRET_DO_NOT_EXPOSE\n").unwrap();
            attractor_runtime::RuntimeHandlerRunner::new()
                .with_rust_llm_client(unified_llm_adapter::Client::new())
        }),
    );
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(stream) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("No captured-profile request: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut request = vec![0; length];
        reader.read_exact(&mut request).unwrap();
        let response = json!({"id":"captured", "object":"chat.completion", "model":"captured-model", "choices":[{"index":0,"message":{"role":"assistant","content":"captured workflow endpoint"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        serde_json::from_slice::<serde_json::Value>(&request).unwrap()
    });
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("captured-workflow".into()),
        flow_content: Some(simple_flow()),
        working_directory: temp.path().join("project").to_string_lossy().into_owned(),
        ..Default::default()
    });
    assert_eq!(server.join().unwrap()["model"], "captured-model");
    assert_eq!(response.body["status"], "started", "{response:?}");
    let bundle = RunStore::for_settings(&settings)
        .read_run_bundle("captured-workflow")
        .unwrap()
        .unwrap();
    assert_eq!(bundle.record.unwrap().status, "completed");
}

#[test]
fn retry_retains_captured_execution_profile_after_configuration_changes() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    let path = settings.config_dir.join("execution-profiles.toml");
    fs::write(&path, "[defaults]\nexecution_profile_id='stable'\n[profiles.stable]\nmode='native'\nlabel='Captured'\ncapabilities=['filesystem']\n[profiles.stable.metadata]\nmarker='original'\n").unwrap();
    let service = AttractorApiService::new(settings.clone());
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("captured-execution".into()),
        flow_content: Some(simple_flow()),
        working_directory: temp.path().join("project").to_string_lossy().into_owned(),
        ..Default::default()
    });
    assert_eq!(response.body["status"], "started");
    let store = RunStore::for_settings(&settings);
    store
        .update_run_record("captured-execution", |record| {
            record.status = "failed".into()
        })
        .unwrap();
    fs::write(&path, "invalid = [").unwrap();
    let response = service.retry_pipeline_route("captured-execution");
    assert_eq!(response.status_code, 200, "{response:?}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let bundle = store
            .read_run_bundle("captured-execution")
            .unwrap()
            .unwrap();
        let record = bundle.record.unwrap();
        if record.status == "completed" {
            assert_eq!(
                bundle.checkpoint.unwrap().context["internal.execution_profile_snapshot"]
                    ["profile"]["metadata"]["marker"],
                "original"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Retry did not complete: {record:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
