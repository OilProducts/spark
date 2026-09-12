use std::fs;
use std::path::Path;

use attractor_api::{handle_attractor_request, AttractorApiService, PipelineStartRequest};
use attractor_runtime::{human_gate_pending_event, CreateRunRequest, RunStore};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

#[test]
fn execution_inventory_and_scoped_activity_routes_keep_resources_independent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("Project Executions");
    fs::create_dir_all(&project).expect("project");
    let store = RunStore::for_settings(&settings);
    let mut record = attractor_core::RunRecord::new("run-executions", project.to_string_lossy());
    record.status = "running".to_string();
    let paths = store
        .create_run(CreateRunRequest {
            record,
            ..Default::default()
        })
        .expect("run");
    let root = paths.logs_dir().join("work/executions/2-1");
    fs::create_dir_all(&root).expect("execution root");
    fs::write(root.join("status.json"), r#"{"outcome":"success"}"#).expect("status");
    let activity = spark_storage::ActivityRepository::new(&root);
    let event = activity
        .append_event(json!({"type":"completed"}), "2026-08-09T00:00:00Z")
        .expect("event");
    activity.append_transcript(&spark_storage::TranscriptRecord::TurnUpsert {
        revision: 1,
        committed_at: event.committed_at,
        source_event_sequence: event.sequence,
        turn: serde_json::from_value(json!({"id":"prompt","role":"user","kind":"message","content":"Ship it","status":"complete"})).expect("turn"),
    }).expect("transcript");

    let detail = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-executions",
        "",
        settings.clone(),
    );
    assert_eq!(detail.status_code, 200);
    assert_eq!(detail.body["executions"][0]["node_id"], "work");
    assert_eq!(detail.body["executions"][0]["stage_index"], 2);
    assert_eq!(detail.body["executions"][0]["attempt"], 1);

    let transcript = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-executions/executions/work/2-1/transcript",
        "",
        settings.clone(),
    );
    assert_eq!(transcript.status_code, 200);
    assert_eq!(transcript.body["records"][0]["turn"]["content"], "Ship it");
    let events = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-executions/executions/work/2-1/events",
        "",
        settings,
    );
    assert_eq!(events.status_code, 200);
    assert_eq!(events.body["events"][0]["sequence"], 1);
}

#[test]
fn inspection_routes_read_durable_pipeline_state_and_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Inspect");
    let service = AttractorApiService::new(settings.clone());
    let start = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-inspect".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        model: Some("compat-model".to_string()),
        launch_context: Some(
            [("context.topic".to_string(), json!("inspect"))]
                .into_iter()
                .collect(),
        ),
        ..PipelineStartRequest::default()
    });
    assert_eq!(start.body["status"], json!("started"));
    let detail = service.get_pipeline("run-inspect");
    assert_eq!(detail.status_code, 200);
    assert_eq!(detail.body["pipeline_id"], json!("run-inspect"));
    assert_eq!(detail.body["run_id"], json!("run-inspect"));
    assert_eq!(detail.body["status"], json!("completed"));
    assert!(detail.body["progress"]["completed_count"].as_u64().unwrap() >= 1);
    assert_eq!(
        detail.body["launch_context"],
        json!({"context.topic": "inspect"})
    );

    // The runs list stays lean: launch inputs are detail-only.
    let listed = service.list_runs();
    let listed_run = listed.body["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .find(|run| run["run_id"] == json!("run-inspect"))
        .expect("listed run");
    assert!(listed_run.get("launch_context").is_none());

    let checkpoint = service.get_pipeline_checkpoint("run-inspect");
    assert_eq!(checkpoint.status_code, 200);
    assert_eq!(
        checkpoint.body["checkpoint"]["context"]["_attractor.runtime.launch_model"],
        json!("compat-model")
    );

    let context = service.get_pipeline_context("run-inspect");
    assert_eq!(context.status_code, 200);
    assert_eq!(
        context.body["context"]["internal.run_id"],
        json!("run-inspect")
    );

    let result = service.get_pipeline_result("run-inspect");
    assert_eq!(result.status_code, 200);
    assert_eq!(result.body["run_id"], json!("run-inspect"));
    assert!(matches!(
        result.body["state"].as_str(),
        Some("ready" | "unavailable")
    ));

    let artifacts = service.list_pipeline_artifacts("run-inspect");
    assert_eq!(artifacts.status_code, 200);
    assert!(artifacts.body["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .any(|artifact| artifact["path"] == json!("artifacts/flow/flow-source.yaml")));

    let source =
        service.get_pipeline_artifact_file("run-inspect", "artifacts/flow/flow-source.yaml");
    assert_eq!(source.status_code, 200);
    assert!(source
        .body
        .as_str()
        .expect("source")
        .contains("id: api_inspect"));

    let traversal = service.get_pipeline_artifact_file("run-inspect", "../run.json");
    assert_eq!(traversal.status_code, 400);
    assert_eq!(traversal.body, json!({"detail": "Invalid artifact path"}));

    let encoded_traversal = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-inspect/artifacts/%2E%2E/run.json",
        "",
        settings.clone(),
    );
    assert_eq!(encoded_traversal.status_code, 400);
    assert_eq!(
        encoded_traversal.body,
        json!({"detail": "Invalid artifact path"})
    );

    let missing_artifact =
        service.get_pipeline_artifact_file("run-inspect", "artifacts/missing.txt");
    assert_eq!(missing_artifact.status_code, 404);
    assert_eq!(
        missing_artifact.body,
        json!({"detail": "Artifact not found"})
    );

    let graph = service.get_pipeline_graph("run-inspect");
    assert_eq!(graph.status_code, 404);
    assert_eq!(
        graph.body,
        json!({"detail": "Graph visualization unavailable"})
    );

    let preview = service.get_pipeline_graph_preview("run-inspect", false);
    assert_eq!(preview.status_code, 200);
    assert_eq!(preview.body["status"], json!("ok"));
}

#[cfg(unix)]
#[test]
fn artifact_endpoint_rejects_symlink_file_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Symlink Artifact");
    let service = AttractorApiService::new(settings.clone());
    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-symlink-artifact".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });

    let bundle = RunStore::for_settings(&settings)
        .read_run_bundle("run-symlink-artifact")
        .expect("read")
        .expect("run");
    let outside_file = temp.path().join("outside-secret.txt");
    fs::write(&outside_file, "secret").expect("outside file");
    std::os::unix::fs::symlink(
        &outside_file,
        bundle.paths.artifacts_dir().join("secret-link.txt"),
    )
    .expect("artifact symlink");

    let response = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-symlink-artifact/artifacts/artifacts/secret-link.txt",
        "",
        settings.clone(),
    );
    assert_eq!(response.status_code, 400);
    assert_eq!(response.body, json!({"detail": "Invalid artifact path"}));

    let artifacts = service.list_pipeline_artifacts("run-symlink-artifact");
    assert_eq!(artifacts.status_code, 200);
    assert!(!artifacts.body["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .any(|artifact| artifact["path"] == json!("artifacts/secret-link.txt")));
}

#[test]
fn journal_events_questions_and_mounted_dispatch_preserve_route_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Journal");
    let service = AttractorApiService::new(settings.clone());
    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-journal".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });

    let journal = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/journal?limit=2",
        "",
        settings.clone(),
    );
    assert_eq!(journal.status_code, 200);
    assert_eq!(journal.body["pipeline_id"], json!("run-journal"));
    assert_eq!(
        journal.body["entries"].as_array().expect("entries").len(),
        2
    );
    assert!(journal.body["newest_sequence"].as_u64().is_some());
    assert!(journal.body["oldest_sequence"].as_u64().is_some());

    let invalid_journal = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/journal?limit=0",
        "",
        settings.clone(),
    );
    assert_eq!(invalid_journal.status_code, 400);
    assert_eq!(
        invalid_journal.body,
        json!({"detail": "limit must be greater than zero"})
    );

    let events = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/events?after_sequence=0",
        "",
        settings.clone(),
    );
    assert_eq!(events.status_code, 200);
    assert_eq!(events.content_type, "text/event-stream");
    let entries = sse_data_entries(events.body.as_str().expect("sse body"));
    assert!(!entries.is_empty());
    let sequences = entries
        .iter()
        .map(|entry| entry["sequence"].as_u64().expect("sequence"))
        .collect::<Vec<_>>();
    let mut sorted_sequences = sequences.clone();
    sorted_sequences.sort_unstable();
    assert_eq!(sequences, sorted_sequences);

    let no_cursor_events = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/events",
        "",
        settings.clone(),
    );
    assert_eq!(no_cursor_events.status_code, 200);
    assert_eq!(no_cursor_events.content_type, "text/event-stream");
    assert_eq!(no_cursor_events.body, json!(""));

    let invalid_cursor_events = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/events?after_sequence=not-a-number",
        "",
        settings.clone(),
    );
    assert_eq!(invalid_cursor_events.status_code, 400);
    assert_eq!(
        invalid_cursor_events.body,
        json!({"detail": "after_sequence must be zero or greater"})
    );

    let negative_cursor_events = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/events?after_sequence=-1",
        "",
        settings.clone(),
    );
    assert_eq!(negative_cursor_events.status_code, 400);
    assert_eq!(
        negative_cursor_events.body,
        json!({"detail": "after_sequence must be zero or greater"})
    );

    let unknown_events = handle_attractor_request(
        "GET",
        "/attractor/pipelines/missing/events?after_sequence=0",
        "",
        settings.clone(),
    );
    assert_eq!(unknown_events.status_code, 404);
    assert_eq!(unknown_events.body, json!({"detail": "Unknown pipeline"}));

    let store = RunStore::for_settings(&settings);
    let bundle = store
        .read_run_bundle("run-journal")
        .expect("read")
        .expect("run");
    store
        .append_event(
            &bundle.paths,
            human_gate_pending_event(
                "run-journal",
                "question-1",
                "gate",
                "Flow",
                "Approve plan?",
                None,
                vec![json!({"label": "Approve", "value": "approve"})],
            ),
        )
        .expect("pending question");
    let transcript = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/transcript",
        "",
        settings.clone(),
    );
    assert_eq!(
        transcript.status_code, 404,
        "runs no longer expose a transcript"
    );

    let journal_after_question = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/journal?limit=20",
        "",
        settings.clone(),
    );
    assert_eq!(journal_after_question.status_code, 200);
    let journal_json = serde_json::to_string(&journal_after_question.body).expect("journal json");
    assert!(journal_json.contains("Human gate pending"));
    assert!(!journal_json.contains("\"kind\":\"request_user_input\""));
    assert!(!journal_json.contains("\"request_user_input\""));

    let questions = service.list_pipeline_questions("run-journal");
    assert_eq!(questions.status_code, 200);
    assert_eq!(
        questions.body,
        json!({
            "questions": [
                {
                    "question_id": "question-1",
                    "run_id": "run-journal",
                    "node_id": "gate",
                    "flow_name": "Flow",
                    "prompt": "Approve plan?",
                    "options": [{"label": "Approve", "value": "approve"}],
                }
            ]
        })
    );

    let answer = handle_attractor_request(
        "POST",
        "/attractor/pipelines/run-journal/questions/question-1/answer",
        &json!({"selected_value": "yes", "note": " Ship it after the freeze lifts. "}).to_string(),
        settings.clone(),
    );
    assert_eq!(answer.status_code, 200);
    assert_eq!(
        answer.body,
        json!({"status": "accepted", "pipeline_id": "run-journal", "question_id": "question-1"})
    );
    assert_eq!(
        service.list_pipeline_questions("run-journal").body,
        json!({"questions": []})
    );
    let answered_bundle = store
        .read_run_bundle("run-journal")
        .expect("read answered")
        .expect("run");
    assert!(answered_bundle.raw_events.iter().any(|event| {
        event.event_type == "InterviewCompleted"
            && event.payload.get("question_id") == Some(&json!("question-1"))
            && event.payload.get("answer") == Some(&json!("yes"))
            && event.payload.get("note") == Some(&json!("Ship it after the freeze lifts."))
            && event.payload.get("outcome_provenance") == Some(&json!("accepted"))
    }));
    let answered_transcript = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-journal/transcript",
        "",
        settings.clone(),
    );
    assert_eq!(answered_transcript.status_code, 404);

    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-other".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    let wrong_run_answer = handle_attractor_request(
        "POST",
        "/attractor/pipelines/run-other/questions/question-1/answer",
        &json!({"selected_value": "yes"}).to_string(),
        settings.clone(),
    );
    assert_eq!(wrong_run_answer.status_code, 404);
    assert_eq!(
        wrong_run_answer.body,
        json!({"detail": "Unknown question for pipeline"})
    );

    let missing_graph = service.get_pipeline_graph("run-journal");
    assert_eq!(missing_graph.status_code, 404);
    assert_eq!(
        missing_graph.body,
        json!({"detail": "Graph visualization unavailable"})
    );

    let missing_pipeline = service.get_pipeline("missing");
    assert_eq!(missing_pipeline.status_code, 404);
    assert_eq!(missing_pipeline.body, json!({"detail": "Unknown pipeline"}));
}

#[test]
fn run_listing_filters_by_project_path_query() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_a = temp.path().join("ProjectA");
    let project_b = temp.path().join("ProjectB");
    let service = AttractorApiService::new(settings.clone());
    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-project-a".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_a.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-project-b".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_b.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });

    let unfiltered = service.list_runs();
    assert_eq!(unfiltered.status_code, 200);
    assert_eq!(unfiltered.body["runs"].as_array().expect("runs").len(), 2);

    let filtered = handle_attractor_request(
        "GET",
        &format!(
            "/attractor/runs?project_path={}",
            project_a.to_string_lossy()
        ),
        "",
        settings,
    );
    assert_eq!(filtered.status_code, 200);
    let runs = filtered.body["runs"].as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], "run-project-a");
}

#[test]
fn run_listing_includes_worktree_child_under_parent_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let parent_project = temp.path().join("parent-project");
    let worktree = temp.path().join("worktrees/child-run");
    let mut child =
        attractor_core::RunRecord::new("child-run", worktree.to_string_lossy().to_string());
    child.project_path = parent_project.to_string_lossy().to_string();
    child.parent_run_id = Some("parent-run".to_string());
    RunStore::for_settings(&settings)
        .create_run(CreateRunRequest {
            record: child,
            checkpoint: None,
            manifest: None,
            flow_source: None,
            flow_definition_json: None,
        })
        .expect("create child run");

    let response = handle_attractor_request(
        "GET",
        &format!(
            "/attractor/runs?project_path={}",
            parent_project.to_string_lossy()
        ),
        "",
        settings,
    );

    assert_eq!(response.status_code, 200);
    let runs = response.body["runs"].as_array().expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], "child-run");
    assert_eq!(
        runs[0]["project_path"],
        parent_project.to_string_lossy().as_ref()
    );
    assert_eq!(
        runs[0]["working_directory"],
        worktree.to_string_lossy().as_ref()
    );
}

#[test]
fn journal_page_keeps_parent_and_child_sequence_spaces_independent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Combined");
    let service = AttractorApiService::new(settings.clone());
    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-combined-parent".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });

    let store = RunStore::for_settings(&settings);
    let mut child = attractor_core::RunRecord::new(
        "run-combined-child",
        project_path.to_string_lossy().to_string(),
    );
    child.parent_run_id = Some("run-combined-parent".to_string());
    let child_paths = store
        .create_run(CreateRunRequest {
            record: child,
            checkpoint: None,
            manifest: None,
            flow_source: None,
            flow_definition_json: None,
        })
        .expect("create child run");
    store
        .append_event(
            &child_paths,
            attractor_runtime::log_event("run-combined-child", "[child] working"),
        )
        .expect("child event");

    let parent_paths = store
        .run_root(&project_path.to_string_lossy(), "run-combined-parent")
        .expect("parent paths");
    for (paths, node, content) in [
        (&parent_paths, "parent-task", "parent output"),
        (&child_paths, "child-task", "child output"),
    ] {
        let root = paths.logs_dir().join(node).join("executions/1-0");
        fs::create_dir_all(&root).expect("execution root");
        fs::write(root.join("status.json"), r#"{"outcome":"success"}"#).expect("status");
        let activity = spark_storage::ActivityRepository::new(root);
        let event = activity
            .append_event(json!({"type": "content_completed"}), "now")
            .expect("execution event");
        activity
            .append_transcript(&spark_storage::TranscriptRecord::SegmentUpsert {
                revision: 1,
                committed_at: "now".to_string(),
                source_event_sequence: event.sequence,
                segment: serde_json::from_value(json!({
                    "id": "answer", "turn_id": "turn", "kind": "assistant_message",
                    "status": "complete", "content": content
                }))
                .expect("segment"),
            })
            .expect("transcript");
    }

    let parent_detail = service.get_pipeline("run-combined-parent");
    let child_detail = service.get_pipeline("run-combined-child");
    assert!(parent_detail.body["executions"]
        .as_array()
        .expect("parent executions")
        .iter()
        .any(|execution| execution["node_id"] == "parent-task"));
    assert!(child_detail.body["executions"]
        .as_array()
        .expect("child executions")
        .iter()
        .any(|execution| execution["node_id"] == "child-task"));

    let journal = handle_attractor_request(
        "GET",
        "/attractor/pipelines/run-combined-parent/journal?limit=500",
        "",
        settings.clone(),
    );
    assert_eq!(journal.status_code, 200);
    let entries = journal.body["entries"].as_array().expect("entries");

    // Only the parent's source-local orchestration journal is returned.
    let sequences = entries
        .iter()
        .map(|entry| entry["sequence"].as_u64().expect("sequence"))
        .collect::<Vec<_>>();
    assert_eq!(journal.body["newest_sequence"], json!(sequences[0]));
    assert_eq!(
        journal.body["oldest_sequence"],
        json!(*sequences.last().unwrap())
    );
    assert!(sequences.windows(2).all(|pair| pair[0] >= pair[1]));
    assert!(entries
        .iter()
        .all(|entry| entry["source_scope"] != json!("child")));
}

fn simple_flow() -> String {
    r#"schema_version: "1"
id: api_inspect
title: API Inspect
nodes:
  start:
    kind: start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Write an inspection note
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

fn sse_data_entries(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .map(|payload| serde_json::from_str::<Value>(payload).expect("sse data json"))
        .collect()
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
