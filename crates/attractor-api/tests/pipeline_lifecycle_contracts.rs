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
    store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            checkpoint: Some(checkpoint),
            manifest: None,
            flow_source: Some(simple_flow()),
            flow_definition_json: Some(flow.to_canonical_json_string()),
        })
        .expect("seed failed run");

    let response = service.retry_pipeline_route("run-retry-exec");
    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("started"));

    assert_eq!(
        wait_for_terminal_status(&settings, "run-retry-exec"),
        "completed",
        "retry must actually execute the prepared run",
    );
}
