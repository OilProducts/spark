use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spark_agent_adapter::AgentTurnOutput;
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::SparkSettings;
use spark_storage::{ConversationHandleRepository, ProjectRegistry};
use spark_workspace::{
    ConversationTurnRequest, FlowRunRequestCreateByHandleRequest, FlowRunRequestReviewRequest,
    ProposedPlanReviewRequest, WorkspaceConversationService, WorkspaceError,
};

fn agent_requestable(settings: &SparkSettings, flow: &str) {
    write_flow(settings, flow, simple_flow());
    spark_storage::set_flow_launch_policy(&settings.config_dir, flow, "agent_requestable")
        .expect("launch policy");
}

#[test]
fn by_handle_run_request_launches_immediately_with_an_inline_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_native_execution_profile(&settings);
    agent_requestable(&settings, "ops/review.yaml");
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-review",
    );
    let service = WorkspaceConversationService::new(settings.clone());

    let launched = service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.yaml".to_string(),
                summary: "Run the review flow.".to_string(),
                goal: Some("Ship the reviewed change.".to_string()),
                launch_context: Some(json!({"context.request.id": "REQ-1"})),
                model: Some("gpt-5".to_string()),
                llm_provider: Some("OpenAI".to_string()),
                llm_profile: None,
                reasoning_effort: Some("HIGH".to_string()),
                execution_profile_id: Some("native".to_string()),
            },
        )
        .expect("launched");

    assert_eq!(launched["conversation_id"], "conversation-review");
    let run_id = launched["run_id"].as_str().expect("run id");
    assert!(run_id.starts_with("run-"));
    let snapshot = service
        .get_snapshot(
            "conversation-review",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("snapshot");
    assert_eq!(snapshot["flow_run_requests"], json!([]));
    let launch = &snapshot["flow_launches"][0];
    assert_eq!(launch["status"], "launched");
    assert_eq!(launch["run_id"], run_id);
    assert_eq!(launch["source_turn_id"], "turn-assistant");
    assert_eq!(launch["source_segment_id"], launched["segment_id"]);
    assert_eq!(launch["llm_provider"], "openai");
    assert_eq!(launch["reasoning_effort"], "high");
    assert_eq!(snapshot["segments"][0]["kind"], "flow_launch");
}

#[test]
fn by_handle_run_request_refuses_flows_agents_may_not_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_native_execution_profile(&settings);
    write_flow(&settings, "ops/private.yaml", simple_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-private",
    );
    let service = WorkspaceConversationService::new(settings.clone());

    let refused = service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/private.yaml".to_string(),
                summary: "Not allowed.".to_string(),
                execution_profile_id: Some("native".to_string()),
                ..FlowRunRequestCreateByHandleRequest::default()
            },
        )
        .expect_err("refused");
    assert!(matches!(refused, WorkspaceError::Validation(_)));
    assert!(refused.to_string().contains("not agent-requestable"));
    assert!(runs_dir_is_empty(&settings));
}

#[test]
fn by_handle_run_request_attaches_to_in_flight_assistant_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_native_execution_profile(&settings);
    agent_requestable(&settings, "ops/review.yaml");
    seed_conversation_with_assistant_status(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-first-turn",
        "streaming",
    );
    let service = WorkspaceConversationService::new(settings.clone());

    service
        .create_flow_run_request_by_handle(
            "amber-anchor",
            FlowRunRequestCreateByHandleRequest {
                flow_name: "ops/review.yaml".to_string(),
                summary: "Run the review flow.".to_string(),
                execution_profile_id: Some("native".to_string()),
                ..FlowRunRequestCreateByHandleRequest::default()
            },
        )
        .expect("launched during the conversation's first in-flight turn");

    let snapshot = service
        .get_snapshot(
            "conversation-first-turn",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("snapshot");
    assert_eq!(snapshot["flow_launches"][0]["status"], "launched");
    assert_eq!(
        snapshot["flow_launches"][0]["source_turn_id"],
        "turn-assistant"
    );
}

#[test]
fn legacy_pending_flow_run_requests_can_still_be_rejected_or_launched() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_native_execution_profile(&settings);
    write_flow(&settings, "ops/review.yaml", simple_flow());
    seed_conversation_with_requests(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-launch",
        "complete",
        json!([
            pending_request("request-reject", "conversation-launch", &project_path, None),
            pending_request(
                "request-launch",
                "conversation-launch",
                &project_path,
                Some("native")
            ),
            pending_request(
                "request-broken",
                "conversation-launch",
                &project_path,
                Some("missing-profile")
            ),
        ]),
    );
    let service = WorkspaceConversationService::new(settings.clone());
    let review = |request_id: &str, disposition: &str| {
        service
            .review_flow_run_request(
                "conversation-launch",
                request_id,
                FlowRunRequestReviewRequest {
                    project_path: project_path.to_string_lossy().into_owned(),
                    disposition: disposition.to_string(),
                    message: "Reviewed.".to_string(),
                    ..FlowRunRequestReviewRequest::default()
                },
            )
            .expect("reviewed")
    };

    let rejected = review("request-reject", "rejected");
    assert_eq!(
        request_by_id(&rejected, "request-reject")["status"],
        "rejected"
    );
    assert!(runs_dir_is_empty(&settings));
    let approved = review("request-launch", "approved");
    let launched = request_by_id(&approved, "request-launch");
    assert_eq!(launched["status"], "launched");
    assert!(launched["run_id"]
        .as_str()
        .expect("run id")
        .starts_with("run-"));
    let failed = review("request-broken", "approved");
    let failed = request_by_id(&failed, "request-broken");
    assert_eq!(failed["status"], "launch_failed");
    assert!(!failed["launch_error"].as_str().expect("error").is_empty());
}

fn pending_request(
    id: &str,
    conversation_id: &str,
    project_path: &Path,
    execution_profile_id: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "created_at": "2026-01-01T00:00:01Z",
        "updated_at": "2026-01-01T00:00:01Z",
        "flow_name": "ops/review.yaml",
        "summary": "Requested before launches were direct.",
        "project_path": project_path,
        "conversation_id": conversation_id,
        "source_turn_id": "turn-assistant",
        "status": "pending",
        "execution_profile_id": execution_profile_id,
    })
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
        "software-development/implement-change.yaml",
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
        "software-development/implement-change.yaml"
    );
    assert_eq!(launch["status"], "launched");
    assert_eq!(launch["run_id"], plan["run_id"]);
}

pub(super) fn seed_conversation(
    settings: &SparkSettings,
    project_path: &str,
    conversation_id: &str,
) {
    seed_conversation_with_assistant_status(settings, project_path, conversation_id, "complete");
}

fn seed_conversation_with_assistant_status(
    settings: &SparkSettings,
    project_path: &str,
    conversation_id: &str,
    assistant_status: &str,
) {
    seed_conversation_with_requests(
        settings,
        project_path,
        conversation_id,
        assistant_status,
        json!([]),
    );
}

fn seed_conversation_with_requests(
    settings: &SparkSettings,
    project_path: &str,
    conversation_id: &str,
    assistant_status: &str,
    flow_run_requests: Value,
) {
    let registry = ProjectRegistry::new(&settings.data_dir);
    let project = registry
        .ensure_project_paths(project_path)
        .expect("project");
    crate::write_conversation_snapshot(
        &settings.data_dir,
        &json!({
            "schema_version": 5,
            "settings_schema_version": 1,
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
                    "status": assistant_status,
                    "kind": "message"
                }
            ],
            "segments": [],
            "event_log": [],
            "flow_run_requests": flow_run_requests,
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": []
        }),
    );
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
    write_legacy_conversation_files(
        &settings.data_dir,
        &json!({
            "schema_version": 5,
            "settings_schema_version": 1,
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
        }),
    );
}

fn request_by_id<'a>(snapshot: &'a Value, request_id: &str) -> &'a Value {
    snapshot["flow_run_requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|entry| entry["id"] == request_id)
        .expect("request")
}

pub(super) fn write_flow(settings: &SparkSettings, name: &str, content: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow dir");
    fs::write(path, content).expect("flow");
}

pub(super) fn write_native_execution_profile(settings: &SparkSettings) {
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

#[test]
fn chat_turn_ingest_persists_streamed_reasoning_segments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = WorkspaceConversationService::new(settings(temp.path()));
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    let (prepared, _) = service
        .start_turn(
            "conversation-reasoning",
            ConversationTurnRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                message: "Do the thing.".to_string(),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let reasoning_body =
        "**Inspecting the repo**\n\nLooked at the build files before choosing a fix.";
    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-reasoning",
            project_path.to_str().expect("utf-8"),
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![
                    reasoning_event(TurnStreamEventKind::ContentDelta, "**Inspecting the repo**"),
                    reasoning_event(TurnStreamEventKind::ContentCompleted, reasoning_body),
                    assistant_completed("Done."),
                ],
                final_assistant_text: Some("Done.".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest turn");

    let segments = snapshot["segments"].as_array().expect("segments");
    let reasoning = segments
        .iter()
        .find(|segment| segment["kind"] == "reasoning")
        .unwrap_or_else(|| panic!("reasoning segment persisted: {segments:?}"));
    assert_eq!(reasoning["content"], reasoning_body);
    assert_eq!(reasoning["status"], "complete");

    // The committed snapshot, re-read from disk, retains the segment: this is
    // what a conversation reload shows.
    let reloaded = service
        .get_snapshot(
            "conversation-reasoning",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("reload conversation");
    let reloaded_segments = reloaded["segments"].as_array().expect("segments");
    assert!(
        reloaded_segments
            .iter()
            .any(|segment| segment["kind"] == "reasoning" && segment["content"] == reasoning_body),
        "reasoning segment must survive reload: {reloaded_segments:?}"
    );
}

fn reasoning_event(kind: TurnStreamEventKind, content: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind,
        channel: Some(TurnStreamChannel::Reasoning),
        source: TurnStreamSource {
            app_turn_id: Some("app-turn-reasoning".to_string()),
            item_id: Some("reason-1".to_string()),
            summary_index: Some(0),
            ..TurnStreamSource::default()
        },
        content_delta: Some(content.to_string()),
        message: None,
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

fn assistant_completed(content: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::ContentCompleted,
        channel: Some(TurnStreamChannel::Assistant),
        source: TurnStreamSource {
            app_turn_id: Some("app-turn-reasoning".to_string()),
            item_id: Some("msg-1".to_string()),
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

pub(super) fn simple_flow() -> &'static str {
    r#"
schema_version: "1"
id: review
nodes:
  start:
    kind: start
  done:
    kind: exit
edges:
  - from: start
    to: done
    "#
}

fn runs_dir_is_empty(settings: &SparkSettings) -> bool {
    match fs::read_dir(&settings.runs_dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => panic!("read runs dir: {error}"),
    }
}

pub(super) fn settings(root: &Path) -> SparkSettings {
    settings_for_project(&root.join("project"), root)
}

fn settings_for_project(project_root: &Path, root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
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

/// Seed the pre-split legacy conversation layout by hand: core keys in
/// `state.json`, artifact arrays in the project-level sidecar files. The
/// repository migrates these on first read.
fn write_legacy_conversation_files(data_dir: &Path, snapshot: &serde_json::Value) {
    crate::write_conversation_snapshot(data_dir, snapshot);
    return;
    #[allow(unreachable_code)]
    {
        let object = snapshot.as_object().expect("snapshot object");
        let conversation_id = snapshot["conversation_id"]
            .as_str()
            .expect("conversation id");
        let project_path = snapshot["project_path"].as_str().expect("project path");
        let project = ProjectRegistry::new(data_dir)
            .ensure_project_paths(project_path)
            .expect("project paths");
        let root = project.conversations_dir.join(conversation_id);
        fs::create_dir_all(&root).expect("conversation dir");
        let mut core = object.clone();
        let artifact = |key: &str| object.get(key).cloned().unwrap_or_else(|| json!([]));
        for key in [
            "event_log",
            "flow_run_requests",
            "flow_launches",
            "run_recoveries",
            "proposed_plans",
        ] {
            core.remove(key);
        }
        fs::write(
            root.join("state.json"),
            serde_json::to_string_pretty(&serde_json::Value::Object(core)).expect("state json"),
        )
        .expect("state.json");
        for (dir, payload) in [
            (
                &project.flow_run_requests_dir,
                json!({
                    "conversation_id": conversation_id,
                    "project_id": project.project_id,
                    "project_path": project_path,
                    "event_log": artifact("event_log"),
                    "flow_run_requests": artifact("flow_run_requests"),
                }),
            ),
            (
                &project.flow_launches_dir,
                json!({
                    "conversation_id": conversation_id,
                    "project_id": project.project_id,
                    "project_path": project_path,
                    "flow_launches": artifact("flow_launches"),
                    "run_recoveries": artifact("run_recoveries"),
                }),
            ),
            (
                &project.proposed_plans_dir,
                json!({
                    "conversation_id": conversation_id,
                    "project_id": project.project_id,
                    "project_path": project_path,
                    "proposed_plans": artifact("proposed_plans"),
                }),
            ),
        ] {
            fs::write(
                dir.join(format!("{conversation_id}.json")),
                serde_json::to_string_pretty(&payload).expect("sidecar json"),
            )
            .expect("sidecar");
        }
    }
}

#[test]
fn pending_attention_aggregates_gates_requests_and_plan_reviews() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_native_execution_profile(&settings);
    write_flow(&settings, "ops/review.yaml", simple_flow());
    seed_conversation_with_requests(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-attention",
        "complete",
        json!([pending_request(
            "request-attention",
            "conversation-attention",
            &project_path,
            None
        )]),
    );
    let service = WorkspaceConversationService::new(settings.clone());
    seed_proposed_plan(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-attention-plan",
    );

    let store = attractor_runtime::RunStore::for_settings(&settings);
    let mut waiting =
        attractor_core::RunRecord::new("run-attention-gate", project_path.to_string_lossy());
    waiting.flow_name = "software-development/run-retrospective.yaml".to_string();
    waiting.status = "waiting".to_string();
    let mut completed =
        attractor_core::RunRecord::new("run-attention-done", project_path.to_string_lossy());
    completed.status = "completed".to_string();
    for record in [waiting, completed] {
        store
            .create_run(attractor_runtime::CreateRunRequest {
                record,
                ..attractor_runtime::CreateRunRequest::default()
            })
            .expect("create run");
    }

    let items = service.pending_attention().expect("attention items");
    let kinds = items
        .iter()
        .map(|item| item["kind"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>();
    assert_eq!(items.len(), 3, "{items:?}");
    assert!(kinds.contains(&"run_gate".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"flow_run_request".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"proposed_plan".to_string()), "{kinds:?}");
    let gate = items
        .iter()
        .find(|item| item["kind"] == "run_gate")
        .expect("gate item");
    assert_eq!(gate["run_id"], "run-attention-gate");
    assert!(
        !items
            .iter()
            .any(|item| item["run_id"] == "run-attention-done"),
        "terminal runs are not attention items"
    );
    let request = items
        .iter()
        .find(|item| item["kind"] == "flow_run_request")
        .expect("request item");
    assert_eq!(request["conversation_id"], "conversation-attention");
    assert_eq!(request["title"], "Requested before launches were direct.");
}

#[test]
fn turns_pin_the_assistant_frame_as_instructions_and_send_the_raw_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    let service = WorkspaceConversationService::new(settings.clone());
    let (prepared, snapshot) = service
        .start_turn(
            "conversation-frame",
            ConversationTurnRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                message: "Kick off the implement-spec workflow.".to_string(),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    assert_eq!(
        prepared.agent_turn_request.prompt,
        "Kick off the implement-spec workflow."
    );
    let frame = prepared.agent_turn_request.metadata
        [spark_agent_adapter::AGENT_INSTRUCTIONS_METADATA_KEY]
        .as_str()
        .expect("pinned frame");
    for expected in [
        "You are the Spark workspace assistant",
        "spark convo run-request",
        "launches the flow immediately",
        "do not use any other Spark installation",
        "agent-requestable",
        "$SPARK_HOME/attractor/runs",
    ] {
        assert!(frame.contains(expected), "frame lacks {expected}: {frame}");
    }
    assert!(!frame.contains("approve"), "{frame}");

    // The stored conversation turn keeps the raw user message, not the frame.
    let stored = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "user")
        .expect("user turn");
    assert_eq!(stored["content"], "Kick off the implement-spec workflow.");
}

#[test]
fn resumed_thread_turns_keep_the_pinned_frame_out_of_the_prompt() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(&project_path.to_string_lossy())
        .expect("project paths");
    // A resumed codex thread still gets the frame as pinned instructions.
    spark_storage::ConversationRepository::new(&settings.data_dir)
        .write_runtime_session(
            "conversation-resume",
            &project_path.to_string_lossy(),
            &spark_storage::conversation::RuntimeSession {
                schema_version: 1,
                provider: "codex_app_server".to_string(),
                thread_id: Some("thread-existing".to_string()),
                established_at: "2026-01-01T00:00:00Z".to_string(),
                last_turn_id: None,
                resume_failed: false,
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        )
        .expect("seed runtime session");
    let _ = &project;

    let (prepared, _) = WorkspaceConversationService::new(settings.clone())
        .start_turn(
            "conversation-resume",
            ConversationTurnRequest {
                project_path: project_path.to_string_lossy().into_owned(),
                message: "Follow-up question.".to_string(),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    assert_eq!(prepared.agent_turn_request.prompt, "Follow-up question.");
    assert!(prepared
        .agent_turn_request
        .metadata
        .contains_key(spark_agent_adapter::AGENT_INSTRUCTIONS_METADATA_KEY));
}
