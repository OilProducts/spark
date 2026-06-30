use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spark_agent_adapter::AgentTurnOutput;
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::SparkSettings;
use spark_storage::{ConversationHandleRepository, ConversationRepository, ProjectRegistry};
use spark_workspace::{
    ConversationTurnRequest, FlowRunRequestCreateByHandleRequest, FlowRunRequestReviewRequest,
    ProposedPlanReviewRequest, WorkspaceConversationService, WorkspaceError,
};

#[test]
fn by_handle_flow_run_request_creation_writes_pending_sidecar_without_launching() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_native_execution_profile(&settings);
    write_flow(&settings, "ops/review.dot", simple_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-review",
    );
    let service = WorkspaceConversationService::new(settings.clone());

    let created = service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.dot".to_string(),
                summary: "Run the approved review flow.".to_string(),
                goal: Some("Ship the reviewed change.".to_string()),
                launch_context: Some(json!({"context.request.id": "REQ-1"})),
                model: Some("gpt-5".to_string()),
                llm_provider: Some("OpenAI".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("HIGH".to_string()),
                execution_profile_id: Some("native".to_string()),
            },
        )
        .expect("created request");

    assert!(created.ok);
    assert_eq!(created.conversation_id, "conversation-review");
    let snapshot = service
        .get_snapshot(
            "conversation-review",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("snapshot");
    let request = &snapshot["flow_run_requests"][0];
    assert_eq!(request["status"], "pending");
    assert_eq!(request["source_turn_id"], "turn-assistant");
    assert_eq!(request["source_segment_id"], created.segment_id);
    assert_eq!(request["llm_provider"], "openai");
    assert_eq!(request["reasoning_effort"], "high");
    assert_eq!(snapshot["segments"][0]["kind"], "flow_run_request");
    assert_eq!(
        snapshot["segments"][0]["artifact_id"],
        created.flow_run_request_id
    );
    assert!(runs_dir_is_empty(&settings));

    let duplicate = service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.dot".to_string(),
                summary: "Run the approved review flow.".to_string(),
                goal: Some("Ship the reviewed change.".to_string()),
                launch_context: Some(json!({"context.request.id": "REQ-1"})),
                model: Some("gpt-5".to_string()),
                llm_provider: Some("openai".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("high".to_string()),
                execution_profile_id: Some("native".to_string()),
            },
        )
        .expect_err("duplicate");
    assert!(matches!(duplicate, WorkspaceError::Conflict(_)));
}

#[test]
fn flow_run_request_review_rejects_or_launches_and_records_provenance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_native_execution_profile(&settings);
    write_flow(&settings, "ops/review.dot", simple_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-launch",
    );
    let service = WorkspaceConversationService::new(settings.clone());

    let rejected = service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.dot".to_string(),
                summary: "Reject me.".to_string(),
                ..FlowRunRequestCreateByHandleRequest::default()
            },
        )
        .expect("created rejected");
    let rejected_snapshot = service
        .review_flow_run_request(
            "conversation-launch",
            &rejected.flow_run_request_id,
            FlowRunRequestReviewRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                disposition: "rejected".to_string(),
                message: "Not this one.".to_string(),
                ..FlowRunRequestReviewRequest::default()
            },
        )
        .expect("rejected");
    let rejected_request = request_by_id(&rejected_snapshot, &rejected.flow_run_request_id);
    assert_eq!(rejected_request["status"], "rejected");
    assert_eq!(rejected_request["review_message"], "Not this one.");
    assert!(runs_dir_is_empty(&settings));

    let approved = service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.dot".to_string(),
                summary: "Launch me.".to_string(),
                goal: Some("Run the tiny flow.".to_string()),
                launch_context: Some(json!({"context.review": "approved"})),
                model: Some("compat-model".to_string()),
                llm_provider: Some("codex".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("medium".to_string()),
                execution_profile_id: Some("native".to_string()),
            },
        )
        .expect("created approved");
    let approved_snapshot = service
        .review_flow_run_request(
            "conversation-launch",
            &approved.flow_run_request_id,
            FlowRunRequestReviewRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                disposition: "approved".to_string(),
                message: "Approved for launch.".to_string(),
                ..FlowRunRequestReviewRequest::default()
            },
        )
        .expect("approved");
    let approved_request = request_by_id(&approved_snapshot, &approved.flow_run_request_id);
    assert_eq!(approved_request["status"], "launched");
    assert_eq!(approved_request["review_message"], "Approved for launch.");
    assert_eq!(approved_request["source_turn_id"], "turn-assistant");
    assert_eq!(approved_request["flow_name"], "ops/review.dot");
    assert!(approved_request["run_id"]
        .as_str()
        .expect("run id")
        .starts_with("run-"));
}

#[test]
fn launch_failure_is_persisted_on_approved_flow_run_request() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_flow(&settings, "ops/broken.dot", "digraph Broken { start -> }");
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-fail",
    );
    let service = WorkspaceConversationService::new(settings);

    let created = service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/broken.dot".to_string(),
                summary: "This launch should fail validation.".to_string(),
                ..FlowRunRequestCreateByHandleRequest::default()
            },
        )
        .expect("created");
    let snapshot = service
        .review_flow_run_request(
            "conversation-fail",
            &created.flow_run_request_id,
            FlowRunRequestReviewRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                disposition: "approved".to_string(),
                message: "Try it.".to_string(),
                ..FlowRunRequestReviewRequest::default()
            },
        )
        .expect("reviewed");
    let request = request_by_id(&snapshot, &created.flow_run_request_id);
    assert_eq!(request["status"], "launch_failed");
    assert!(request["launch_error"].as_str().expect("error").len() > 0);
}

#[test]
fn completed_plan_segments_create_proposed_plan_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = WorkspaceConversationService::new(settings(temp.path()));
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    let (prepared, _) = service
        .start_turn(
            "conversation-plan",
            ConversationTurnRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                message: "Draft a plan.".to_string(),
                chat_mode: Some("plan".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-plan",
            project_path.to_str().expect("utf-8"),
            &prepared.assistant_turn_id,
            "plan",
            AgentTurnOutput {
                events: vec![plan_completed(
                    "# Reviewable Proposed Plan\n\n1. Add the artifact.\n2. Wire the route.",
                )],
                final_assistant_text: None,
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest plan");

    assert_eq!(snapshot["proposed_plans"][0]["status"], "pending_review");
    assert_eq!(
        snapshot["proposed_plans"][0]["title"],
        "Reviewable Proposed Plan"
    );
    assert_eq!(
        snapshot["segments"][0]["artifact_id"],
        snapshot["proposed_plans"][0]["id"]
    );
}

#[test]
fn proposed_plan_review_writes_change_request_and_launch_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_flow(
        &settings,
        "software-development/implement-change-request.dot",
        simple_flow(),
    );
    seed_proposed_plan(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-plan",
    );
    let service = WorkspaceConversationService::new(settings);

    let rejected = service
        .review_proposed_plan(
            "conversation-plan",
            "proposed-plan-inline",
            ProposedPlanReviewRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                disposition: "rejected".to_string(),
                review_note: Some("Needs acceptance criteria.".to_string()),
            },
        )
        .expect("rejected");
    assert_eq!(rejected["proposed_plans"][0]["status"], "rejected");
    assert_eq!(
        rejected["proposed_plans"][0]["review_note"],
        "Needs acceptance criteria."
    );

    seed_proposed_plan(
        &settings_for_project(&project_path, temp.path()),
        project_path.to_str().expect("utf-8"),
        "conversation-plan-approved",
    );
    let service =
        WorkspaceConversationService::new(settings_for_project(&project_path, temp.path()));
    let approved = service
        .review_proposed_plan(
            "conversation-plan-approved",
            "proposed-plan-inline",
            ProposedPlanReviewRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                disposition: "approved".to_string(),
                review_note: Some("Ready.".to_string()),
            },
        )
        .expect("approved");
    let plan = &approved["proposed_plans"][0];
    assert_eq!(plan["status"], "approved");
    assert_eq!(plan["review_note"], "Ready.");
    assert!(plan["written_change_request_path"]
        .as_str()
        .expect("path")
        .ends_with("/request.md"));
    let request_path = PathBuf::from(plan["written_change_request_path"].as_str().expect("path"));
    assert_eq!(
        fs::read_to_string(request_path).expect("request"),
        "# Reviewable Proposed Plan\n\n1. Add the artifact.\n"
    );
    let launch = &approved["flow_launches"][0];
    assert_eq!(
        launch["flow_name"],
        "software-development/implement-change-request.dot"
    );
    assert_eq!(launch["status"], "launched");
    assert_eq!(launch["run_id"], plan["run_id"]);
}

fn seed_conversation(settings: &SparkSettings, project_path: &str, conversation_id: &str) {
    let registry = ProjectRegistry::new(&settings.data_dir);
    let project = registry
        .ensure_project_paths(project_path)
        .expect("project");
    let state_path = project
        .conversations_dir
        .join(conversation_id)
        .join("state.json");
    fs::create_dir_all(state_path.parent().expect("state parent")).expect("state dir");
    fs::write(
        state_path,
        serde_json::to_string_pretty(&json!({
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
            "title": "Review thread",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [
                {
                    "id": "turn-user",
                    "role": "user",
                    "content": "Please prepare the change.",
                    "timestamp": "2026-01-01T00:00:00Z",
                    "status": "complete",
                    "kind": "message"
                },
                {
                    "id": "turn-assistant",
                    "role": "assistant",
                    "content": "I can request that flow.",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "status": "complete",
                    "kind": "message"
                }
            ],
            "segments": [],
            "event_log": [],
            "flow_run_requests": [],
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": []
        }))
        .expect("json"),
    )
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

fn seed_proposed_plan(settings: &SparkSettings, project_path: &str, conversation_id: &str) {
    ConversationRepository::new(&settings.data_dir)
        .write_snapshot(&json!({
            "schema_version": 5,
            "revision": 0,
            "conversation_id": conversation_id,
            "conversation_handle": "",
            "project_path": project_path,
            "chat_mode": "plan",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "Plan review",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [
                {
                    "id": "turn-user-plan",
                    "role": "user",
                    "content": "Draft the implementation plan.",
                    "timestamp": "2026-01-01T00:00:00Z",
                    "status": "complete",
                    "kind": "message"
                },
                {
                    "id": "turn-assistant-plan",
                    "role": "assistant",
                    "content": "Here is the proposed plan.",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "status": "complete",
                    "kind": "message"
                }
            ],
            "segments": [
                {
                    "id": "segment-plan-inline",
                    "turn_id": "turn-assistant-plan",
                    "order": 1,
                    "kind": "plan",
                    "role": "assistant",
                    "status": "complete",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "completed_at": "2026-01-01T00:00:01Z",
                    "content": "# Reviewable Proposed Plan\n\n1. Add the artifact.",
                    "artifact_id": "proposed-plan-inline",
                    "source": {}
                }
            ],
            "event_log": [],
            "flow_run_requests": [],
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": [
                {
                    "id": "proposed-plan-inline",
                    "created_at": "2026-01-01T00:00:01Z",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "title": "Reviewable Proposed Plan",
                    "content": "# Reviewable Proposed Plan\n\n1. Add the artifact.",
                    "project_path": project_path,
                    "conversation_id": conversation_id,
                    "source_turn_id": "turn-assistant-plan",
                    "status": "pending_review",
                    "source_segment_id": "segment-plan-inline"
                }
            ]
        }))
        .expect("write proposed plan");
}

fn request_by_id<'a>(snapshot: &'a Value, request_id: &str) -> &'a Value {
    snapshot["flow_run_requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|entry| entry["id"] == request_id)
        .expect("request")
}

fn write_flow(settings: &SparkSettings, name: &str, content: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow dir");
    fs::write(path, content).expect("flow");
}

fn write_native_execution_profile(settings: &SparkSettings) {
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[profiles.native]
label = "Native"
mode = "native"
"#,
    )
    .expect("execution profile");
}

fn plan_completed(content: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::ContentCompleted,
        channel: Some(TurnStreamChannel::Plan),
        source: TurnStreamSource {
            app_turn_id: Some("app-turn-plan".to_string()),
            item_id: Some("plan-item".to_string()),
            summary_index: None,
            ..TurnStreamSource::default()
        },
        content_delta: Some(content.to_string()),
        message: Some(content.to_string()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn simple_flow() -> &'static str {
    r#"
    digraph Review {
      start [shape=Mdiamond]
      done [shape=Msquare]
      start -> done
    }
    "#
}

fn runs_dir_is_empty(settings: &SparkSettings) -> bool {
    match fs::read_dir(&settings.runs_dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => panic!("read runs dir: {error}"),
    }
}

fn settings(root: &Path) -> SparkSettings {
    settings_for_project(&root.join("project"), root)
}

fn settings_for_project(project_root: &Path, root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: project_root.to_path_buf(),
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
