use std::path::Path;

use attractor_api::{ContinuePipelineRequest, ResolvedContinueFlow, RuntimeControlService};
use attractor_core::{CheckpointState, LaunchContext, RunRecord};
use attractor_dsl::parse_dot;
use attractor_runtime::{
    CreateRunRequest, ExecuteRunRequest, PipelineExecutor, RunStore, RuntimeHandlerRunner,
};
use serde_json::json;

fn temp_store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"))
}

fn record(run_id: &str, project_path: &Path, status: &str) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "api-control.dot".to_string();
    record.model = "compat-model".to_string();
    record.status = status.to_string();
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    if status == "failed" {
        record.last_error = "previous failure".to_string();
    }
    record
}

fn checkpoint(current_node: &str) -> CheckpointState {
    CheckpointState {
        timestamp: "2026-06-23T10:00:00Z".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: vec!["start".to_string()],
        context: [("context.seed".to_string(), json!("api"))]
            .into_iter()
            .collect(),
        retry_counts: Default::default(),
        logs: Vec::new(),
    }
}

fn run_manager_child_pipeline(temp: &tempfile::TempDir) -> (RunStore, String) {
    let child_dot_path = temp.path().join("api-child.dot");
    std::fs::write(
        &child_dot_path,
        r#"
        digraph Child {
          start [shape=Mdiamond]
          task [shape=box, prompt="Child task"]
          done [shape=Msquare]

          start -> task -> done
        }
        "#,
    )
    .expect("child dot");
    let parent_source = format!(
        r#"
        digraph Parent {{
          graph [stack.child_dotfile="{}"]
          start [shape=Mdiamond]
          manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
          done [shape=Msquare]

          start -> manager -> done
        }}
        "#,
        child_dot_path.display()
    );
    let graph = parse_dot(&parent_source).expect("parent graph");
    let store = temp_store(temp);
    let project_path = temp.path().join("Project API Manager");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let parent_run_id = "run-api-manager-parent".to_string();
    let mut parent = RunRecord::new(&parent_run_id, project_path.to_string_lossy());
    parent.flow_name = "parent.dot".to_string();
    parent.started_at = "2026-06-23T10:00:00Z".to_string();
    let mut executor = PipelineExecutor::new(RuntimeHandlerRunner::new());
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: parent,
            graph,
            graph_source: Some(parent_source.clone()),
            graph_dot: Some(parent_source),
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("manager pipeline");
    assert_eq!(result.status, "completed");
    (store, parent_run_id)
}

#[test]
fn runtime_control_service_returns_checkpoint_context_and_unknown_pipeline_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let project_path = temp.path().join("Project API");
    std::fs::create_dir_all(&project_path).expect("project dir");
    store
        .create_run(CreateRunRequest {
            record: record("run-api", &project_path, "running"),
            checkpoint: Some(checkpoint("work")),
            ..CreateRunRequest::default()
        })
        .expect("create run");
    let service = RuntimeControlService::new(store);

    let checkpoint_response = service.get_checkpoint("run-api");
    assert_eq!(checkpoint_response.status_code, 200);
    assert_eq!(checkpoint_response.body["pipeline_id"], json!("run-api"));
    assert_eq!(
        checkpoint_response.body["checkpoint"]["current_node"],
        json!("work")
    );

    let context_response = service.get_context("run-api");
    assert_eq!(context_response.status_code, 200);
    assert_eq!(
        context_response.body["context"]["context.seed"],
        json!("api")
    );

    let missing_response = service.cancel_pipeline("missing");
    assert_eq!(missing_response.status_code, 404);
    assert_eq!(missing_response.body, json!({"detail": "Unknown pipeline"}));
}

#[test]
fn runtime_control_service_preserves_continue_retry_cancel_response_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let project_path = temp.path().join("Project API Controls");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let graph_source = r#"
    digraph G {
      start [shape=Mdiamond]
      work [shape=box]
      done [shape=Msquare]
      start -> work -> done
    }
    "#;
    store
        .create_run(CreateRunRequest {
            record: record("run-api-controls", &project_path, "failed"),
            checkpoint: Some(checkpoint("work")),
            graph_source: Some(graph_source.to_string()),
            graph_dot: Some(graph_source.to_string()),
            ..CreateRunRequest::default()
        })
        .expect("create run");
    let service = RuntimeControlService::new(store);

    let continue_guard = service.continue_pipeline(
        "run-api-controls",
        ContinuePipelineRequest {
            start_node: String::new(),
            flow_source_mode: "snapshot".to_string(),
            ..ContinuePipelineRequest::default()
        },
        ResolvedContinueFlow {
            graph: parse_dot(graph_source).expect("graph"),
            graph_source: Some(graph_source.to_string()),
            graph_dot: Some(graph_source.to_string()),
            flow_name: None,
        },
    );
    assert_eq!(continue_guard.status_code, 200);
    assert_eq!(
        continue_guard.body,
        json!({"status": "validation_error", "error": "start_node is required."})
    );

    let retry_response = service.retry_pipeline("run-api-controls");
    assert_eq!(retry_response.status_code, 200);
    assert_eq!(retry_response.body["status"], json!("started"));
    assert_eq!(
        retry_response.body["pipeline_id"],
        json!("run-api-controls")
    );

    let cancel_response = service.cancel_pipeline("run-api-controls");
    assert_eq!(cancel_response.status_code, 200);
    assert_eq!(
        cancel_response.body,
        json!({"status": "cancel_requested", "pipeline_id": "run-api-controls"})
    );
}

#[test]
fn runtime_control_service_exposes_child_run_detail_journal_and_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, parent_run_id) = run_manager_child_pipeline(&temp);
    let service = RuntimeControlService::new(store);

    let detail = service.get_run_detail(&parent_run_id);
    assert_eq!(detail.status_code, 200);
    assert_eq!(detail.body["pipeline_id"], json!(parent_run_id));
    let child_run_id = detail.body["checkpoint"]["context"]["context.stack.child.run_id"]
        .as_str()
        .expect("child run id")
        .to_string();
    assert!(!child_run_id.is_empty());
    assert_eq!(detail.body["child_runs"][0]["run_id"], json!(child_run_id));
    assert_eq!(
        detail.body["child_runs"][0]["record"]["parent_run_id"],
        json!(parent_run_id)
    );
    assert_eq!(
        detail.body["child_runs"][0]["record"]["parent_node_id"],
        json!("manager")
    );
    assert_eq!(
        detail.body["child_runs"][0]["record"]["child_invocation_index"],
        json!(1)
    );

    let parent_event_types = detail.body["raw_events"]
        .as_array()
        .expect("events")
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect::<Vec<_>>();
    assert!(parent_event_types.contains(&"ChildRunStarted"));
    assert!(parent_event_types.contains(&"ChildRunCompleted"));

    let journal = service.get_run_journal(&parent_run_id, true);
    assert_eq!(journal.status_code, 200);
    assert_eq!(
        journal.body["child_journals"][0]["run_id"],
        json!(child_run_id)
    );
    assert!(journal.body["child_journals"][0]["journal"]
        .as_array()
        .expect("child journal")
        .iter()
        .any(|entry| entry["raw_type"] == json!("PipelineStarted")));

    let events = service.get_run_events(&parent_run_id, true);
    assert_eq!(events.status_code, 200);
    assert_eq!(
        events.body["child_events"][0]["run_id"],
        json!(child_run_id)
    );
    assert!(events.body["child_events"][0]["events"]
        .as_array()
        .expect("child events")
        .iter()
        .any(|event| event["type"] == json!("PipelineStarted")));

    let missing = service.get_run_detail("missing");
    assert_eq!(missing.status_code, 404);
    assert_eq!(missing.body, json!({"detail": "Unknown pipeline"}));
}
