use std::fs;
use std::path::{Path, PathBuf};

use attractor_core::{CheckpointState, RunRecord};
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{ConversationHandleRepository, ConversationRepository, ProjectRegistry};
use spark_workspace::{
    RunContinueRequest, RunLaunchRequest, RunRetryRequest, WorkspaceConversationService,
    WorkspaceError,
};

#[test]
fn direct_launch_by_conversation_handle_creates_flow_launch_and_starts_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_flow(&settings, "ops/run.dot", simple_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-launch",
    );
    let service = WorkspaceConversationService::new(settings.clone());

    let response = service
        .launch_workspace_run(RunLaunchRequest {
            flow_name: "ops/run.dot".to_string(),
            summary: "Launch the implementation flow.".to_string(),
            conversation_handle: Some("amber-anchor".to_string()),
            project_path: Some(project_path.to_string_lossy().into_owned()),
            goal: Some("Implement the approved scope.".to_string()),
            launch_context: Some(json!({"context.request.summary": "Implement"})),
            model: Some("compat-model".to_string()),
            llm_provider: Some("Anthropic".to_string()),
            reasoning_effort: Some("LOW".to_string()),
            ..RunLaunchRequest::default()
        })
        .expect("launch");

    assert_eq!(response["ok"], true);
    assert_eq!(response["status"], "started");
    assert_eq!(response["conversation_id"], "conversation-launch");
    let run_id = response["run_id"].as_str().expect("run id");
    let flow_launch_id = response["flow_launch_id"].as_str().expect("launch id");

    let snapshot = service
        .get_snapshot(
            "conversation-launch",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("snapshot");
    let launch = snapshot["flow_launches"]
        .as_array()
        .expect("launches")
        .iter()
        .find(|entry| entry["id"] == flow_launch_id)
        .expect("flow launch");
    assert_eq!(launch["status"], "launched");
    assert_eq!(launch["run_id"], run_id);
    assert_eq!(launch["llm_provider"], "anthropic");
    assert_eq!(launch["reasoning_effort"], "low");
    assert_eq!(launch["goal"], "Implement the approved scope.");
    let segment = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|entry| entry["artifact_id"] == flow_launch_id)
        .expect("launch segment");
    assert_eq!(segment["kind"], "flow_launch");
    assert_eq!(segment["turn_id"], "turn-assistant");

    let run = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read run")
        .expect("run")
        .record
        .expect("record");
    assert_eq!(run.flow_name, "ops/run.dot");
    assert_eq!(run.project_path, project_path.to_string_lossy());
    assert_eq!(run.llm_provider, "anthropic");
    assert_eq!(run.reasoning_effort.as_deref(), Some("low"));
}

#[test]
fn direct_launch_project_only_and_selection_errors_are_explicit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    let other_path = temp.path().join("other");
    fs::create_dir_all(&project_path).expect("project dir");
    fs::create_dir_all(&other_path).expect("other dir");
    write_flow(&settings, "ops/run.dot", simple_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-launch",
    );
    let service = WorkspaceConversationService::new(settings.clone());

    let response = service
        .launch_workspace_run(RunLaunchRequest {
            flow_name: "ops/run.dot".to_string(),
            summary: "Launch without a conversation.".to_string(),
            project_path: Some(project_path.to_string_lossy().into_owned()),
            ..RunLaunchRequest::default()
        })
        .expect("project-only launch");
    assert_eq!(response["ok"], true);
    assert!(response.get("flow_launch_id").is_none());

    let missing_project = service
        .launch_workspace_run(RunLaunchRequest {
            flow_name: "ops/run.dot".to_string(),
            summary: "Missing selection.".to_string(),
            ..RunLaunchRequest::default()
        })
        .expect_err("missing project");
    assert!(matches!(missing_project, WorkspaceError::Validation(_)));

    let mismatch = service
        .launch_workspace_run(RunLaunchRequest {
            flow_name: "ops/run.dot".to_string(),
            summary: "Wrong project.".to_string(),
            conversation_handle: Some("amber-anchor".to_string()),
            project_path: Some(other_path.to_string_lossy().into_owned()),
            ..RunLaunchRequest::default()
        })
        .expect_err("project mismatch");
    assert!(
        matches!(mismatch, WorkspaceError::Validation(message) if message.contains("does not match"))
    );

    write_flow(&settings, "ops/invalid.dot", "digraph Broken { start -> }");
    let invalid_launch = service
        .launch_workspace_run(RunLaunchRequest {
            flow_name: "ops/invalid.dot".to_string(),
            summary: "Invalid DOT should fail the launch.".to_string(),
            conversation_handle: Some("amber-anchor".to_string()),
            ..RunLaunchRequest::default()
        })
        .expect_err("invalid launch");
    assert!(matches!(invalid_launch, WorkspaceError::Internal(message)
            if message.contains("Expected") || message.contains("parse") || message.contains("line")));
    let snapshot = service
        .get_snapshot(
            "conversation-launch",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("snapshot after invalid launch");
    let failed_launch = snapshot["flow_launches"]
        .as_array()
        .expect("launches")
        .iter()
        .find(|entry| entry["flow_name"] == "ops/invalid.dot")
        .expect("failed launch");
    assert_eq!(failed_launch["status"], "launch_failed");
    let launch_error = failed_launch["launch_error"]
        .as_str()
        .expect("launch error");
    assert!(
        launch_error.contains("Expected")
            || launch_error.contains("parse")
            || launch_error.contains("line")
    );
}

#[test]
fn retry_and_continue_record_recovery_artifacts_and_delegate_to_attractor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-recovery",
    );
    seed_failed_run(&settings, "run-failed", &project_path);
    let service = WorkspaceConversationService::new(settings.clone());

    let retry = service
        .retry_workspace_run(
            "run-failed",
            RunRetryRequest {
                conversation_handle: Some("amber-anchor".to_string()),
            },
        )
        .expect("retry");
    assert_eq!(retry["ok"], true);
    assert_eq!(retry["operation"], "retry");
    assert_eq!(retry["run_id"], "run-failed");
    let retry_recovery_id = retry["run_recovery_id"].as_str().expect("retry recovery");
    let retry_snapshot = service
        .get_snapshot(
            "conversation-recovery",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("retry snapshot");
    let retry_recovery = recovery_by_id(&retry_snapshot, retry_recovery_id);
    assert_eq!(retry_recovery["operation"], "retry");
    assert_eq!(retry_recovery["source_run_id"], "run-failed");
    assert_eq!(retry_recovery["result_run_id"], "run-failed");
    assert_eq!(retry_recovery["status"], "started");

    seed_completed_run(&settings, "run-source", &project_path);
    let store = RunStore::for_settings(&settings);
    let source_paths = store
        .read_run_bundle("run-source")
        .expect("read source")
        .expect("source")
        .paths;
    let source_checkpoint_before =
        fs::read_to_string(source_paths.state_json()).expect("source checkpoint before");
    let continued = service
        .continue_workspace_run(
            "run-source",
            RunContinueRequest {
                start_node: "task".to_string(),
                flow_source_mode: "snapshot".to_string(),
                project_path: Some(project_path.to_string_lossy().into_owned()),
                conversation_handle: Some("amber-anchor".to_string()),
                model: Some("gpt-5".to_string()),
                llm_provider: Some("openai".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("high".to_string()),
                flow_name: Some("ignored.dot".to_string()),
            },
        )
        .expect("continue");
    assert_eq!(continued["ok"], true);
    assert_eq!(continued["operation"], "continue");
    assert_eq!(continued["continued_from_run_id"], "run-source");
    assert!(continued.get("flow_name").is_none());
    let continued_run_id = continued["run_id"].as_str().expect("continued run");
    assert_ne!(continued_run_id, "run-source");
    assert_eq!(
        fs::read_to_string(source_paths.state_json()).expect("source checkpoint after"),
        source_checkpoint_before
    );
    let continued_record = store
        .read_run_bundle(continued_run_id)
        .expect("read continued")
        .expect("continued")
        .record
        .expect("continued record");
    assert_eq!(
        continued_record.continued_from_run_id.as_deref(),
        Some("run-source")
    );
    assert_eq!(
        continued_record.continued_from_node.as_deref(),
        Some("task")
    );
    assert_eq!(
        continued_record.continued_from_flow_mode.as_deref(),
        Some("snapshot")
    );
    assert_eq!(continued_record.llm_provider, "openai");
    assert_eq!(
        continued_record.llm_profile.as_deref(),
        Some("implementation")
    );
    assert_eq!(continued_record.reasoning_effort.as_deref(), Some("high"));

    let continue_snapshot = service
        .get_snapshot(
            "conversation-recovery",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("continue snapshot");
    let recovery = recovery_by_id(
        &continue_snapshot,
        continued["run_recovery_id"]
            .as_str()
            .expect("continue recovery"),
    );
    assert_eq!(recovery["operation"], "continue");
    assert_eq!(recovery["source_run_id"], "run-source");
    assert_eq!(recovery["result_run_id"], continued_run_id);
    assert_eq!(recovery["status"], "started");
    assert_eq!(recovery["start_node"], "task");
    assert_eq!(recovery["flow_source_mode"], "snapshot");
}

fn recovery_by_id<'a>(snapshot: &'a Value, recovery_id: &str) -> &'a Value {
    snapshot["run_recoveries"]
        .as_array()
        .expect("recoveries")
        .iter()
        .find(|entry| entry["id"] == recovery_id)
        .expect("recovery")
}

fn seed_failed_run(settings: &SparkSettings, run_id: &str, project_path: &Path) {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "ops/retry.dot".to_string();
    record.status = "failed".to_string();
    record.last_error = "previous failure".to_string();
    RunStore::for_settings(settings)
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint("task")),
            graph_source: Some(simple_flow().to_string()),
            graph_dot: Some(simple_flow().to_string()),
            ..CreateRunRequest::default()
        })
        .expect("failed run");
}

fn seed_completed_run(settings: &SparkSettings, run_id: &str, project_path: &Path) {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "ops/source.dot".to_string();
    record.status = "completed".to_string();
    RunStore::for_settings(settings)
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint("task")),
            graph_source: Some(simple_flow().to_string()),
            graph_dot: Some(simple_flow().to_string()),
            ..CreateRunRequest::default()
        })
        .expect("completed run");
}

fn checkpoint(current_node: &str) -> CheckpointState {
    CheckpointState {
        timestamp: "2026-06-23T10:00:00Z".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: vec!["start".to_string(), current_node.to_string()],
        context: [
            ("context.seed".to_string(), json!("workspace")),
            (
                "_attractor.node_outcomes".to_string(),
                json!({current_node: "fail"}),
            ),
        ]
        .into_iter()
        .collect(),
        retry_counts: Default::default(),
        logs: Vec::new(),
    }
}

fn seed_conversation(settings: &SparkSettings, project_path: &str, conversation_id: &str) {
    let registry = ProjectRegistry::new(&settings.data_dir);
    let project = registry
        .ensure_project_paths(project_path)
        .expect("project");
    ConversationRepository::new(&settings.data_dir)
        .write_snapshot(&json!({
            "schema_version": 5,
            "revision": 0,
            "conversation_id": conversation_id,
            "conversation_handle": "amber-anchor",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "Run thread",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [
                {"id": "turn-user", "role": "user", "content": "Run it.", "timestamp": "2026-01-01T00:00:00Z", "status": "complete", "kind": "message"},
                {"id": "turn-assistant", "role": "assistant", "content": "Ready.", "timestamp": "2026-01-01T00:00:01Z", "status": "complete", "kind": "message"}
            ],
            "segments": [],
            "event_log": [],
            "flow_run_requests": [],
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": []
        }))
        .expect("write state");
    ConversationHandleRepository::new(&settings.data_dir)
        .ensure_conversation_handle(
            conversation_id,
            &project.project_id,
            project_path,
            "2026-01-01T00:00:00Z",
            Some("amber-anchor"),
        )
        .expect("handle");
}

fn write_flow(settings: &SparkSettings, name: &str, content: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow dir");
    fs::write(path, content).expect("flow");
}

fn simple_flow() -> &'static str {
    r#"
    digraph WorkspaceRun {
      start [shape=Mdiamond]
      task [shape=box, prompt="Write a workspace note"]
      done [shape=Msquare]
      start -> task -> done
    }
    "#
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
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::<PathBuf>::new(),
    }
}
