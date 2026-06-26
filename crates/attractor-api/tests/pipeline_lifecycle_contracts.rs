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
        metadata_event.payload["graph_source_path"],
        json!(bundle
            .paths
            .artifacts_dir()
            .join("graphviz/pipeline-source.dot")
            .to_string_lossy()
            .to_string())
    );
    assert_eq!(
        metadata_event.payload["graph_dot_path"],
        json!(bundle
            .paths
            .artifacts_dir()
            .join("graphviz/pipeline.dot")
            .to_string_lossy()
            .to_string())
    );
    assert_eq!(metadata_event.payload["graph_render_path"], json!(null));
    let metadata_journal_entry = bundle
        .journal
        .iter()
        .find(|entry| entry.raw_type == "run_meta")
        .expect("run metadata journal entry");
    assert_eq!(
        metadata_journal_entry.payload["graph_source_path"],
        metadata_event.payload["graph_source_path"]
    );
    let record = bundle.record.expect("record");
    assert_eq!(record.status, "completed");
    assert_eq!(record.spec_id.as_deref(), Some("spec-1"));
    assert_eq!(record.plan_id.as_deref(), Some("plan-1"));
    assert_eq!(record.execution_mode, "native");

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
        .join("graphviz/pipeline-source.dot")
        .is_file());
    assert!(bundle
        .paths
        .artifacts_dir()
        .join("graphviz/pipeline.dot")
        .is_file());
    assert!(bundle.paths.result_json().is_file());
}

#[test]
fn start_pipeline_reports_validation_errors_without_creating_duplicate_or_invalid_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Validation");
    let service = AttractorApiService::new(settings.clone());

    let first = service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-duplicate".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(first.body["status"], json!("started"));

    let duplicate = service.start_pipeline(PipelineStartRequest {
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
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(
        missing_content.body,
        json!({"status": "validation_error", "error": "Either flow_content or flow_name is required."})
    );

    let parse_error = service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-parse-error".to_string()),
        flow_content: Some("digraph {".to_string()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(parse_error.body["status"], json!("validation_error"));
    assert!(parse_error.body["errors"].as_array().expect("errors").len() == 1);

    let launch_context_error = service.start_pipeline(PipelineStartRequest {
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
    r#"
    digraph ApiLifecycle {
      start [shape=Mdiamond]
      task [shape=box, prompt="Write a lifecycle note"]
      done [shape=Msquare]
      start -> task -> done
    }
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
