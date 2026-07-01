use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use attractor_api::{AttractorApiService, PipelineStartRequest};
use attractor_core::{RawRuntimeEvent, RunRecord};
use attractor_runtime::{CreateRunRequest, RunStore};
use axum::body::{to_bytes, Body};
use axum::http::{Request, Response, StatusCode};
use futures_util::StreamExt;
use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentError, AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::SparkSettings;
use spark_http::{build_app, build_app_with_agent_turn_backend};
use spark_storage::ConversationRepository;
use spark_workspace::{
    ConversationTurnRequest, TriggerCreateRequest, WorkspaceConversationService,
    WorkspaceTriggerService,
};
use tower::ServiceExt;

#[tokio::test]
async fn live_route_returns_sse_keepalive_and_json_cursor_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings(temp.path()));

    let response = request(app.clone(), "GET", "/workspace/api/live/events", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/event-stream; charset=utf-8"
    );
    assert_eq!(response.headers()["cache-control"], "no-cache");
    assert_eq!(response.headers()["connection"], "keep-alive");
    let mut stream = response.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut stream).await, ": keepalive\n\n");

    let invalid = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?conversation_revision=-1",
        None,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(invalid).await,
        json!({"detail": "conversation_revision must be a non-negative integer."})
    );

    let missing_scope = request(
        app,
        "GET",
        "/workspace/api/live/events?conversation_id=conversation-live",
        None,
    )
    .await;
    assert_eq!(missing_scope.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(missing_scope).await,
        json!({"detail": "conversation_project_path is required when conversation_id is provided."})
    );
}

#[tokio::test]
async fn live_route_replays_conversation_snapshots_and_events_from_storage() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-live");
    let app = build_app(settings);

    let snapshot = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?conversation_id=conversation-live&conversation_project_path={}",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(snapshot.status(), StatusCode::OK);
    let mut snapshot_stream = snapshot.into_body().into_data_stream();
    let snapshot_envelope = sse_data_json(&next_sse_chunk(&mut snapshot_stream).await);
    assert_eq!(snapshot_envelope["type"], "conversation.snapshot");
    assert_eq!(
        snapshot_envelope["cursor"],
        json!({"kind": "conversation_revision", "value": 2})
    );
    assert_eq!(
        snapshot_envelope["payload"]["state"]["conversation_id"],
        "conversation-live"
    );

    let replay = request(
        app,
        "GET",
        &format!(
            "/workspace/api/live/events?conversation_id=conversation-live&conversation_project_path={}&conversation_revision=0",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    let mut replay_stream = replay.into_body().into_data_stream();
    let first = sse_data_json(&next_sse_chunk(&mut replay_stream).await);
    let second = sse_data_json(&next_sse_chunk(&mut replay_stream).await);
    assert_eq!(first["type"], "conversation.turn_upsert");
    assert_eq!(first["cursor"]["value"], 1);
    assert_eq!(second["type"], "conversation.segment_upsert");
    assert_eq!(second["cursor"]["value"], 2);
}

#[tokio::test]
async fn live_route_streams_conversation_mutations_on_open_connection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-live");
    let app = build_app_with_agent_turn_backend(
        settings,
        Arc::new(StaticAgentTurnBackend::new("Live route answer.")),
    );

    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?conversation_id=conversation-live&conversation_project_path={}&conversation_revision=2",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut live_stream).await, ": keepalive\n\n");

    let posted = request(
        app,
        "POST",
        "/workspace/api/conversations/conversation-live/turns",
        Some(json!({
            "project_path": project_path,
            "message": "Please run this live."
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);

    let first = sse_data_json(&next_sse_chunk(&mut live_stream).await);
    let second = sse_data_json(&next_sse_chunk(&mut live_stream).await);
    assert_eq!(first["type"], "conversation.turn_upsert");
    assert_eq!(
        first["cursor"],
        json!({"kind": "conversation_revision", "value": 3})
    );
    assert_eq!(first["payload"]["turn"]["role"], "user");
    assert_eq!(second["type"], "conversation.turn_upsert");
    assert_eq!(
        second["cursor"],
        json!({"kind": "conversation_revision", "value": 4})
    );
    assert_eq!(second["payload"]["turn"]["role"], "assistant");
}

#[tokio::test]
async fn live_route_streams_full_backend_ingested_revision_range_for_turn_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-live-ingested");
    let backend_output = AgentTurnOutput {
        raw_log_lines: vec![AgentRawLogLine {
            direction: "incoming".to_string(),
            line: "{\"event\":\"http-live-ingested\"}".to_string(),
        }],
        events: vec![
            agent_event(
                "session_start",
                "processing",
                json!({"state": "processing"}),
            ),
            content_delta("Live ", "app-turn-live", "final-answer"),
            content_delta("streamed answer.", "app-turn-live", "final-answer"),
            content_completed(
                TurnStreamChannel::Reasoning,
                "Live route reasoning.",
                "app-turn-live",
                "reasoning",
            ),
            model_tool_event("model_tool_call_start", "live-tool-1", "proposed", None),
            model_tool_event(
                "model_tool_call_delta",
                "live-tool-1",
                "streaming",
                Some("{\"query\":\"live route\"}"),
            ),
            model_tool_event("model_tool_call_end", "live-tool-1", "completed", None),
            tool_event(
                TurnStreamEventKind::ToolCallStarted,
                "live-tool-1",
                "running",
                "partial live output",
            ),
            tool_event(
                TurnStreamEventKind::ToolCallCompleted,
                "live-tool-1",
                "completed",
                "full live output",
            ),
            token_usage(json!({"total": {"inputTokens": 11, "outputTokens": 4}})),
            agent_warning_event("Live route compatibility warning."),
            request_user_input_event(),
            processing_completed_event(),
            agent_event("session_end", "closed", json!({"state": "closed"})),
        ],
        final_assistant_text: Some("Live streamed answer.".to_string()),
        token_usage: Some(json!({"total": {"inputTokens": 11, "outputTokens": 4}})),
        token_usage_breakdown: Some(json!({
            "total": {"inputTokens": 11, "outputTokens": 4},
            "last": {"inputTokens": 3, "outputTokens": 1}
        })),
        ..AgentTurnOutput::default()
    };
    let app = build_app_with_agent_turn_backend(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::from_output(backend_output)),
    );

    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?conversation_id=conversation-live-ingested&conversation_project_path={}&conversation_revision=2",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut live_stream).await, ": keepalive\n\n");

    let posted = request(
        app,
        "POST",
        "/workspace/api/conversations/conversation-live-ingested/turns",
        Some(json!({
            "project_path": project_path,
            "message": "Please stream the ingested backend output."
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);
    let posted_snapshot = json_body(posted).await;
    let final_revision = posted_snapshot["revision"]
        .as_i64()
        .expect("final snapshot revision");
    assert!(final_revision > 4);
    assert!(posted_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| turn["role"] == "assistant"
            && turn["status"] == "complete"
            && turn["content"] == "Live streamed answer."
            && turn["token_usage"]["total"]["inputTokens"] == json!(11)
            && turn["token_usage_breakdown"]["last"]["outputTokens"] == json!(1)));
    assert!(posted_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "reasoning"
            && segment["content"] == "Live route reasoning."));
    assert!(posted_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "model_tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["id"] == "live-tool-1"
            && segment["tool_call"]["arguments"] == json!({"query": "live route"})));
    assert!(posted_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["id"] == "live-tool-1"
            && segment["tool_call"]["output"] == "full live output"));
    assert!(posted_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "agent_event"
            && segment["event_kind"] == "warning"
            && segment["message"] == "Live route compatibility warning."
            && segment["details"]["message"] == "Live route compatibility warning."));
    assert!(posted_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "agent_event"
            && segment["event_kind"] == "processing_end"
            && segment["event_status"] == "idle"
            && segment["details"]["state"] == "idle"));
    assert!(posted_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "agent_event"
            && segment["event_kind"] == "session_end"
            && segment["event_status"] == "closed"
            && segment["details"]["state"] == "closed"));

    let mut envelopes = Vec::new();
    for _ in 0..40 {
        let envelope = sse_data_json(&next_sse_chunk(&mut live_stream).await);
        let cursor = envelope["cursor"]["value"].as_i64().expect("cursor");
        envelopes.push(envelope);
        if cursor >= final_revision {
            break;
        }
    }
    assert_eq!(
        envelopes.last().expect("snapshot envelope")["type"],
        "conversation.snapshot"
    );
    let cursors = envelopes
        .iter()
        .map(|envelope| envelope["cursor"]["value"].as_i64().expect("cursor"))
        .collect::<Vec<_>>();
    assert_eq!(cursors, (3..=final_revision).collect::<Vec<_>>());

    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.turn_upsert"
            && envelope["payload"]["turn"]["role"] == "user"
            && envelope["payload"]["turn"]["content"]
                == "Please stream the ingested backend output."
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "assistant_message"
            && envelope["payload"]["segment"]["content"] == "Live streamed answer."
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "reasoning"
            && envelope["payload"]["segment"]["content"] == "Live route reasoning."
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "model_tool_call"
            && envelope["payload"]["segment"]["status"] == "complete"
            && envelope["payload"]["segment"]["tool_call"]["id"] == "live-tool-1"
            && envelope["payload"]["segment"]["source"]["raw_kind"] == "model_tool_call_end"
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "tool_call"
            && envelope["payload"]["segment"]["status"] == "complete"
            && envelope["payload"]["segment"]["tool_call"]["output"] == "full live output"
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.turn_upsert"
            && envelope["payload"]["turn"]["role"] == "assistant"
            && envelope["payload"]["turn"]["token_usage"]["total"]["outputTokens"] == json!(4)
            && envelope["payload"]["turn"]["token_usage_breakdown"]["last"]["inputTokens"]
                == json!(3)
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "request_user_input"
            && envelope["payload"]["segment"]["request_user_input"]["status"] == "pending"
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "agent_event"
            && envelope["payload"]["segment"]["event_kind"] == "session_start"
            && envelope["payload"]["segment"]["event_status"] == "processing"
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "agent_event"
            && envelope["payload"]["segment"]["event_kind"] == "warning"
            && envelope["payload"]["segment"]["message"] == "Live route compatibility warning."
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "agent_event"
            && envelope["payload"]["segment"]["event_kind"] == "processing_end"
            && envelope["payload"]["segment"]["event_status"] == "idle"
    }));
    let snapshot_envelope = envelopes.last().expect("snapshot envelope");
    assert_eq!(snapshot_envelope["cursor"]["value"], json!(final_revision));
    assert_eq!(
        snapshot_envelope["payload"]["state"]["revision"],
        json!(final_revision)
    );
    assert_eq!(
        snapshot_envelope["payload"]["state"]["turns"],
        posted_snapshot["turns"]
    );

    let raw_log = ConversationRepository::new(&settings.data_dir)
        .read_raw_rpc_log(
            "conversation-live-ingested",
            &project_path.to_string_lossy(),
        )
        .expect("raw log");
    assert_eq!(raw_log.last().expect("raw log line").direction, "incoming");
    assert_eq!(
        raw_log.last().expect("raw log line").line,
        "{\"event\":\"http-live-ingested\"}"
    );
}

#[tokio::test]
async fn live_route_streams_structured_backend_failure_from_persisted_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-live-failure");
    let backend_output = AgentTurnOutput {
        raw_log_lines: vec![AgentRawLogLine {
            direction: "incoming".to_string(),
            line: "{\"event\":\"http-live-thread-resume-failed\"}".to_string(),
        }],
        token_usage: Some(json!({"total": {"inputTokens": 6, "outputTokens": 0}})),
        token_usage_breakdown: Some(json!({
            "total": {"inputTokens": 6, "outputTokens": 0},
            "last": {"inputTokens": 6, "outputTokens": 0}
        })),
        thread_resume_failure: Some(AgentThreadResumeFailure {
            message: "live thread resume failed".to_string(),
            error_code: Some("thread_resume_failed".to_string()),
            details: Some(json!({
                "thread_id": "thread-live-resume",
                "retryable": false
            })),
        }),
        ..AgentTurnOutput::default()
    };
    let app = build_app_with_agent_turn_backend(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::from_output(backend_output)),
    );

    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?conversation_id=conversation-live-failure&conversation_project_path={}&conversation_revision=2",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut live_stream).await, ": keepalive\n\n");

    let posted = request(
        app,
        "POST",
        "/workspace/api/conversations/conversation-live-failure/turns",
        Some(json!({
            "project_path": project_path,
            "message": "Resume and fail with structured details."
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);
    let posted_snapshot = json_body(posted).await;
    let final_revision = posted_snapshot["revision"]
        .as_i64()
        .expect("final snapshot revision");

    let mut envelopes = Vec::new();
    for _ in 0..20 {
        let envelope = sse_data_json(&next_sse_chunk(&mut live_stream).await);
        let cursor = envelope["cursor"]["value"].as_i64().expect("cursor");
        envelopes.push(envelope);
        if cursor >= final_revision {
            break;
        }
    }

    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.turn_upsert"
            && envelope["payload"]["turn"]["role"] == "assistant"
            && envelope["payload"]["turn"]["status"] == "failed"
            && envelope["payload"]["turn"]["error"] == "live thread resume failed"
            && envelope["payload"]["turn"]["error_code"] == "thread_resume_failed"
            && envelope["payload"]["turn"]["details"]["thread_id"] == "thread-live-resume"
            && envelope["payload"]["turn"]["token_usage"]["total"]["inputTokens"] == json!(6)
            && envelope["payload"]["turn"]["token_usage_breakdown"]["last"]["outputTokens"]
                == json!(0)
    }));
    let snapshot_envelope = envelopes.last().expect("snapshot envelope");
    assert_eq!(snapshot_envelope["type"], "conversation.snapshot");
    assert_eq!(
        snapshot_envelope["payload"]["state"]["event_log"][0]["details"]["thread_id"],
        "thread-live-resume"
    );
    assert_eq!(
        snapshot_envelope["payload"]["state"]["turns"],
        posted_snapshot["turns"]
    );

    let raw_log = ConversationRepository::new(&settings.data_dir)
        .read_raw_rpc_log("conversation-live-failure", &project_path.to_string_lossy())
        .expect("raw log");
    assert_eq!(
        raw_log.last().expect("raw log line").line,
        "{\"event\":\"http-live-thread-resume-failed\"}"
    );
}

#[tokio::test]
async fn live_route_streams_backend_ingested_revision_range_for_request_user_input_answer_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();
    let service = WorkspaceConversationService::new(settings.clone());
    let (prepared, _) = service
        .start_turn(
            "conversation-live-answer",
            ConversationTurnRequest {
                project_path: project_path_text.clone(),
                message: "Need a live answer.".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let pending_snapshot = service
        .ingest_agent_turn_output(
            "conversation-live-answer",
            &project_path_text,
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("pending input");
    let before_revision = pending_snapshot["revision"]
        .as_i64()
        .expect("pending revision");
    let answer_output = AgentTurnOutput {
        raw_log_lines: vec![AgentRawLogLine {
            direction: "incoming".to_string(),
            line: "{\"event\":\"http-live-answer\"}".to_string(),
        }],
        events: vec![
            content_delta("Answered ", "app-turn-live-answer", "final-answer"),
            content_delta("over live SSE.", "app-turn-live-answer", "final-answer"),
            token_usage(json!({"total": {"inputTokens": 5, "outputTokens": 3}})),
        ],
        final_assistant_text: Some("Answered over live SSE.".to_string()),
        token_usage: Some(json!({"total": {"inputTokens": 5, "outputTokens": 3}})),
        ..AgentTurnOutput::default()
    };
    let app = build_app_with_agent_turn_backend(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::from_output(answer_output)),
    );

    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?conversation_id=conversation-live-answer&conversation_project_path={}&conversation_revision={before_revision}",
            url_encode(&project_path_text)
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut live_stream).await, ": keepalive\n\n");

    let posted = request(
        app,
        "POST",
        "/workspace/api/conversations/conversation-live-answer/request-user-input/decision/answer",
        Some(json!({
            "project_path": project_path_text,
            "answers": {"decision": "Approve"}
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);
    let posted_snapshot = json_body(posted).await;
    let final_revision = posted_snapshot["revision"]
        .as_i64()
        .expect("final snapshot revision");
    assert!(final_revision > before_revision);

    let mut envelopes = Vec::new();
    for _ in 0..20 {
        let envelope = sse_data_json(&next_sse_chunk(&mut live_stream).await);
        let cursor = envelope["cursor"]["value"].as_i64().expect("cursor");
        envelopes.push(envelope);
        if cursor >= final_revision {
            break;
        }
    }
    assert_eq!(
        envelopes.last().expect("snapshot envelope")["type"],
        "conversation.snapshot"
    );
    let cursors = envelopes
        .iter()
        .map(|envelope| envelope["cursor"]["value"].as_i64().expect("cursor"))
        .collect::<Vec<_>>();
    assert_eq!(
        cursors,
        ((before_revision + 1)..=final_revision).collect::<Vec<_>>()
    );

    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "request_user_input"
            && envelope["payload"]["segment"]["request_user_input"]["status"] == "answered"
            && envelope["payload"]["segment"]["request_user_input"]["answers"]["decision"]
                == "Approve"
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.segment_upsert"
            && envelope["payload"]["segment"]["kind"] == "assistant_message"
            && envelope["payload"]["segment"]["content"] == "Answered over live SSE."
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.turn_upsert"
            && envelope["payload"]["turn"]["id"] == prepared.assistant_turn_id
            && envelope["payload"]["turn"]["status"] == "complete"
            && envelope["payload"]["turn"]["content"] == "Answered over live SSE."
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["type"] == "conversation.turn_upsert"
            && envelope["payload"]["turn"]["id"] == prepared.assistant_turn_id
            && envelope["payload"]["turn"]["token_usage"]["total"]["outputTokens"] == json!(3)
    }));
    let snapshot_envelope = envelopes.last().expect("snapshot envelope");
    assert_eq!(snapshot_envelope["cursor"]["value"], json!(final_revision));
    assert_eq!(
        snapshot_envelope["payload"]["state"]["turns"],
        posted_snapshot["turns"]
    );

    let raw_log = ConversationRepository::new(&settings.data_dir)
        .read_raw_rpc_log("conversation-live-answer", &project_path.to_string_lossy())
        .expect("raw log");
    assert_eq!(raw_log.last().expect("raw log line").direction, "incoming");
    assert_eq!(
        raw_log.last().expect("raw log line").line,
        "{\"event\":\"http-live-answer\"}"
    );
}

struct StaticAgentTurnBackend {
    output: AgentTurnOutput,
}

impl StaticAgentTurnBackend {
    fn new(final_assistant_text: &str) -> Self {
        Self::from_output(AgentTurnOutput {
            final_assistant_text: Some(final_assistant_text.to_string()),
            ..AgentTurnOutput::default()
        })
    }

    fn from_output(output: AgentTurnOutput) -> Self {
        Self { output }
    }
}

impl AgentTurnBackend for StaticAgentTurnBackend {
    fn run_turn(&self, _request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        Ok(self.output.clone())
    }

    fn answer_request_user_input(
        &self,
        _request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, AgentError> {
        Ok(self.output.clone())
    }
}

#[tokio::test]
async fn live_route_replays_run_journals_and_runs_overview() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    let service = AttractorApiService::new(settings.clone());
    let started = service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-live-http".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        model: Some("compat-model".to_string()),
        ..PipelineStartRequest::default()
    });
    assert_eq!(started.status_code, 200);
    let app = build_app(settings);

    let replay = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?run_id=run-live-http&run_sequence=0",
        None,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let mut replay_stream = replay.into_body().into_data_stream();
    let first = sse_data_json(&next_sse_chunk(&mut replay_stream).await);
    assert_eq!(first["type"], "run.journal_entry");
    assert_eq!(
        first["resource"],
        json!({"kind": "run", "id": "run-live-http"})
    );
    assert_eq!(first["cursor"], json!({"kind": "run_sequence", "value": 1}));

    let overview = request(
        app,
        "GET",
        &format!(
            "/workspace/api/live/events?include_runs_overview=true&runs_project_path={}",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    let mut overview_stream = overview.into_body().into_data_stream();
    let upsert = sse_data_json(&next_sse_chunk(&mut overview_stream).await);
    assert_eq!(upsert["type"], "run.upsert");
    assert_eq!(upsert["payload"]["run"]["run_id"], "run-live-http");
    assert_eq!(upsert["payload"]["run"]["model"], "compat-model");
}

#[tokio::test]
async fn live_route_replays_manager_loop_child_journals_with_combined_cursor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_parent_child_run_journals(&settings, &project_path);
    let app = build_app(settings);

    let replay = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?run_id=run-live-parent&run_sequence=0",
        None,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let mut replay_stream = replay.into_body().into_data_stream();
    let mut envelopes = Vec::new();
    for _ in 0..7 {
        envelopes.push(sse_data_json(&next_sse_chunk(&mut replay_stream).await));
    }

    let cursor_values = envelopes
        .iter()
        .map(|envelope| envelope["cursor"]["value"].as_i64().expect("run cursor"))
        .collect::<Vec<_>>();
    assert_eq!(cursor_values, vec![1, 2, 3, 4, 5, 6, 7]);

    let child = envelopes
        .iter()
        .find(|envelope| envelope["payload"]["source_scope"] == "child")
        .expect("child-scoped journal envelope");
    assert_eq!(child["type"], "run.journal_entry");
    assert_eq!(
        child["resource"],
        json!({"kind": "run", "id": "run-live-parent"})
    );
    assert_eq!(child["payload"]["source_parent_node_id"], "manager");
    assert_eq!(child["payload"]["source_flow_name"], "child-live.dot");
    assert_eq!(child["payload"]["payload"]["source_scope"], "child");
    assert_eq!(
        child["payload"]["payload"]["source_parent_node_id"],
        "manager"
    );
    assert_eq!(
        child["payload"]["payload"]["source_flow_name"],
        "child-live.dot"
    );
    assert_eq!(
        child["payload"]["payload"]["source_run_id"],
        "run-live-child"
    );

    let child_cursor = child["cursor"]["value"].as_i64().expect("child cursor");
    let reconnect = request(
        app,
        "GET",
        &format!("/workspace/api/live/events?run_id=run-live-parent&run_sequence={child_cursor}"),
        None,
    )
    .await;
    assert_eq!(reconnect.status(), StatusCode::OK);
    let mut reconnect_stream = reconnect.into_body().into_data_stream();
    if child_cursor < 7 {
        let next = sse_data_json(&next_sse_chunk(&mut reconnect_stream).await);
        assert_eq!(
            next["cursor"]["value"].as_i64().expect("next cursor"),
            child_cursor + 1
        );
    } else {
        assert_eq!(
            next_sse_chunk(&mut reconnect_stream).await,
            ": keepalive\n\n"
        );
    }
}

#[tokio::test]
async fn live_route_streams_workspace_run_launches_and_selected_run_updates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/live.dot");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let app = build_app(settings.clone());

    let overview = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_runs_overview=true&runs_project_path={}",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(overview.status(), StatusCode::OK);
    let mut overview_stream = overview.into_body().into_data_stream();
    assert_eq!(
        next_sse_chunk(&mut overview_stream).await,
        ": keepalive\n\n"
    );

    let launched = request(
        app.clone(),
        "POST",
        "/workspace/api/runs/launch",
        Some(json!({
            "flow_name": "ops/live.dot",
            "summary": "Launch from open SSE overview",
            "project_path": project_path,
            "model": "compat-model"
        })),
    )
    .await;
    assert_eq!(launched.status(), StatusCode::OK);
    let launch_body = json_body(launched).await;
    let launched_run_id = launch_body["run_id"].as_str().expect("run id");

    let upsert = sse_data_json(&next_sse_chunk(&mut overview_stream).await);
    assert_eq!(upsert["type"], "run.upsert");
    assert_eq!(upsert["payload"]["run"]["run_id"], launched_run_id);
    assert_eq!(upsert["payload"]["run"]["model"], "compat-model");

    let service = AttractorApiService::new(settings.clone());
    let started = service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-live-selected".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(started.status_code, 200);
    let before_sequence = latest_journal_sequence(&settings, "run-live-selected");

    let selected = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?run_id=run-live-selected&run_sequence={before_sequence}"
        ),
        None,
    )
    .await;
    assert_eq!(selected.status(), StatusCode::OK);
    let mut selected_stream = selected.into_body().into_data_stream();
    assert_eq!(
        next_sse_chunk(&mut selected_stream).await,
        ": keepalive\n\n"
    );

    let steered = request(
        app,
        "POST",
        "/attractor/pipelines/run-live-selected/steer",
        Some(json!({"message": "inspect live stream", "target_run_id": "missing-child"})),
    )
    .await;
    assert_eq!(steered.status(), StatusCode::OK);

    let journal = sse_data_json(&next_sse_chunk(&mut selected_stream).await);
    assert_eq!(journal["type"], "run.question_pending");
    assert_eq!(
        journal["resource"],
        json!({"kind": "run", "id": "run-live-selected"})
    );
    assert!(journal["cursor"]["value"].as_u64().unwrap_or(0) > before_sequence);
}

#[tokio::test]
async fn live_route_fans_out_route_owned_trigger_upsert_and_delete() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.dot");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let app = build_app(settings);

    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_triggers=true&triggers_project_path={}",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    let mut live_stream = live.into_body().into_data_stream();
    let snapshot = sse_data_json(&next_sse_chunk(&mut live_stream).await);
    assert_eq!(snapshot["type"], "trigger.snapshot");
    assert_eq!(snapshot["payload"], json!({"triggers": []}));

    let created = request(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "name": "Compat webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/run.dot",
                "project_path": project_path,
                "static_context": {"origin": "compat"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = json_body(created).await;
    let trigger_id = created_body["id"].as_str().expect("trigger id").to_string();

    let upsert = sse_data_json(&next_sse_chunk(&mut live_stream).await);
    assert_eq!(upsert["type"], "trigger.upsert");
    assert_eq!(
        upsert["resource"],
        json!({"kind": "trigger", "id": trigger_id})
    );
    assert_eq!(upsert["payload"]["type"], "trigger_upsert");

    let deleted = request(
        app,
        "DELETE",
        &format!("/workspace/api/triggers/{trigger_id}"),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);

    let delete = sse_data_json(&next_sse_chunk(&mut live_stream).await);
    assert_eq!(delete["type"], "trigger.delete");
    assert_eq!(
        delete["resource"],
        json!({"kind": "trigger", "id": trigger_id})
    );
    assert_eq!(
        delete["payload"]["trigger"],
        json!({"status": "deleted", "id": trigger_id})
    );
}

#[tokio::test]
async fn live_route_streams_source_activation_trigger_upserts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.dot");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();
    let app = build_app(settings);

    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_triggers=true&triggers_project_path={}",
            url_encode(&project_path_text)
        ),
        None,
    )
    .await;
    let mut live_stream = live.into_body().into_data_stream();
    let snapshot = sse_data_json(&next_sse_chunk(&mut live_stream).await);
    assert_eq!(snapshot["type"], "trigger.snapshot");
    assert_eq!(snapshot["payload"], json!({"triggers": []}));

    let created = request(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "name": "Due schedule",
            "source_type": "schedule",
            "action": {
                "flow_name": "ops/run.dot",
                "project_path": project_path_text,
                "static_context": {"origin": "sse"}
            },
            "source": {
                "kind": "once",
                "run_at": "2026-06-24T09:00:00Z"
            }
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = json_body(created).await;
    let trigger_id = created_body["id"].as_str().expect("trigger id").to_string();

    let mut activation = None;
    for _ in 0..6 {
        let envelope = sse_data_json(&next_sse_chunk(&mut live_stream).await);
        if envelope["type"] == "trigger.upsert"
            && envelope["resource"] == json!({"kind": "trigger", "id": trigger_id})
            && envelope["payload"]["trigger"]["state"]["last_result"] == "success"
        {
            activation = Some(envelope);
            break;
        }
    }
    let activation = activation.expect("source activation upsert");
    assert_eq!(activation["payload"]["type"], "trigger_upsert");
    assert_eq!(
        activation["payload"]["trigger"]["state"]["recent_history"][0]["message"],
        "Trigger fired successfully."
    );
    drop(app);
}

#[tokio::test]
async fn live_route_streams_webhook_trigger_and_run_upserts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/webhook-live.dot");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();
    let app = build_app(settings);

    let trigger_live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_triggers=true&triggers_project_path={}",
            url_encode(&project_path_text)
        ),
        None,
    )
    .await;
    let mut trigger_stream = trigger_live.into_body().into_data_stream();
    let trigger_snapshot = sse_data_json(&next_sse_chunk(&mut trigger_stream).await);
    assert_eq!(trigger_snapshot["type"], "trigger.snapshot");

    let runs_live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_runs_overview=true&runs_project_path={}",
            url_encode(&project_path_text)
        ),
        None,
    )
    .await;
    let mut runs_stream = runs_live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut runs_stream).await, ": keepalive\n\n");

    let created = request(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "name": "Webhook live",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/webhook-live.dot",
                "project_path": project_path_text,
                "static_context": {"origin": "live-webhook"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = json_body(created).await;
    let trigger_id = created_body["id"].as_str().expect("trigger id").to_string();
    let webhook_key = created_body["source"]["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let webhook_secret = created_body["webhook_secret"]
        .as_str()
        .expect("webhook secret")
        .to_string();

    let create_upsert = sse_data_json(&next_sse_chunk(&mut trigger_stream).await);
    assert_eq!(create_upsert["type"], "trigger.upsert");

    let accepted = request_with_headers(
        app,
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", webhook_key.as_str()),
            ("X-Spark-Webhook-Secret", webhook_secret.as_str()),
            ("X-Spark-Webhook-Request-Id", "live-request"),
        ],
        Some(json!({"payload": "live"})),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        json_body(accepted).await,
        json!({"ok": true, "trigger_id": trigger_id})
    );

    let trigger_upsert = sse_data_json(&next_sse_chunk(&mut trigger_stream).await);
    assert_eq!(trigger_upsert["type"], "trigger.upsert");
    assert_eq!(
        trigger_upsert["resource"],
        json!({"kind": "trigger", "id": trigger_id})
    );
    assert_eq!(
        trigger_upsert["payload"]["trigger"]["state"]["last_result"],
        "success"
    );
    let run_id = trigger_upsert["payload"]["trigger"]["state"]["recent_history"][0]["run_id"]
        .as_str()
        .expect("trigger run id")
        .to_string();

    let run_upsert = sse_data_json(&next_sse_chunk(&mut runs_stream).await);
    assert_eq!(run_upsert["type"], "run.upsert");
    assert_eq!(run_upsert["payload"]["run"]["run_id"], run_id);
    assert_eq!(
        run_upsert["payload"]["run"]["flow_name"],
        "ops/webhook-live.dot"
    );
}

#[tokio::test]
async fn dropping_app_cancels_trigger_source_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.dot");
    let app = build_app(settings.clone());
    drop(app);

    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let service = WorkspaceTriggerService::new(settings);
    let created = service
        .create_trigger(TriggerCreateRequest {
            name: "Dropped app schedule".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: json!({
                "flow_name": "ops/run.dot",
                "project_path": project_path,
                "static_context": {"origin": "drop-test"}
            })
            .as_object()
            .expect("action object")
            .clone(),
            source: json!({
                "kind": "once",
                "run_at": "2020-01-01T00:00:00Z"
            })
            .as_object()
            .expect("source object")
            .clone(),
        })
        .expect("create trigger");

    tokio::time::sleep(Duration::from_millis(1200)).await;
    let stored = service.get_trigger(&created.id).expect("stored trigger");
    assert_eq!(stored.state.last_result, None);
    assert!(stored.state.recent_history.is_empty());
}

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> Response<Body> {
    request_with_headers(app, method, uri, &[], body).await
}

async fn request_with_headers(
    app: axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.oneshot(
        builder
            .body(match body {
                Some(value) => Body::from(value.to_string()),
                None => Body::empty(),
            })
            .expect("request"),
    )
    .await
    .expect("response")
}

async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("json")
}

async fn next_sse_chunk(stream: &mut axum::body::BodyDataStream) -> String {
    let chunk = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("timely SSE frame")
        .expect("SSE stream item")
        .expect("SSE bytes");
    String::from_utf8(chunk.to_vec()).expect("utf-8 SSE frame")
}

fn sse_data_json(frame: &str) -> Value {
    let data = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&data).expect("SSE data JSON")
}

fn content_delta(text: &str, app_turn_id: &str, item_id: &str) -> TurnStreamEvent {
    let mut event = TurnStreamEvent::content_delta(TurnStreamChannel::Assistant, text);
    event.source = TurnStreamSource {
        app_turn_id: Some(app_turn_id.to_string()),
        item_id: Some(item_id.to_string()),
        ..TurnStreamSource::default()
    };
    event.phase = Some("final_answer".to_string());
    event
}

fn content_completed(
    channel: TurnStreamChannel,
    text: &str,
    app_turn_id: &str,
    item_id: &str,
) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::ContentCompleted,
        channel: Some(channel),
        source: source(app_turn_id, item_id),
        content_delta: Some(text.to_string()),
        message: Some(text.to_string()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: Some("final_answer".to_string()),
        status: None,
    }
}

fn model_tool_event(kind: &str, id: &str, status: &str, delta: Option<&str>) -> TurnStreamEvent {
    let mut tool_call = json!({
        "id": id,
        "kind": "model_tool_call",
        "status": status,
        "name": "lookup",
        "title": "lookup",
        "arguments": {"query": "live route"},
    });
    if let Some(delta) = delta {
        tool_call["delta"] = json!(delta);
    }

    TurnStreamEvent {
        kind: TurnStreamEventKind::Other(kind.to_string()),
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-turn-live".to_string()),
            item_id: Some(id.to_string()),
            response_id: Some("resp-live".to_string()),
            raw_kind: Some(kind.to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: delta.map(str::to_string),
        message: None,
        tool_call: Some(tool_call),
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: Some(status.to_string()),
    }
}

fn tool_event(kind: TurnStreamEventKind, id: &str, status: &str, output: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind,
        channel: None,
        source: source("app-turn-live", id),
        content_delta: None,
        message: None,
        tool_call: Some(json!({
            "id": id,
            "kind": "command_execution",
            "status": status,
            "title": "Run command",
            "output": output,
            "file_paths": [],
        })),
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn token_usage(usage: Value) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::TokenUsageUpdated,
        channel: None,
        source: TurnStreamSource::default(),
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: Some(usage),
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn agent_event(kind: &str, status: &str, details: Value) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::Other(kind.to_string()),
        channel: None,
        source: TurnStreamSource {
            backend: Some("agent_session".to_string()),
            app_turn_id: Some("app-turn-live".to_string()),
            item_id: Some(kind.to_string()),
            raw_kind: Some(kind.to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: Some(details),
        phase: None,
        status: Some(status.to_string()),
    }
}

fn agent_warning_event(message: &str) -> TurnStreamEvent {
    let mut event = agent_event("warning", "warning", json!({"message": message}));
    event.message = Some(message.to_string());
    event
}

fn processing_completed_event() -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::TurnCompleted,
        channel: None,
        source: TurnStreamSource {
            backend: Some("agent_session".to_string()),
            app_turn_id: Some("app-turn-live".to_string()),
            item_id: Some("processing".to_string()),
            raw_kind: Some("processing_end".to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: Some(json!({"state": "idle"})),
        phase: Some("turn".to_string()),
        status: Some("idle".to_string()),
    }
}

fn source(app_turn_id: &str, item_id: &str) -> TurnStreamSource {
    TurnStreamSource {
        app_turn_id: Some(app_turn_id.to_string()),
        item_id: Some(item_id.to_string()),
        ..TurnStreamSource::default()
    }
}

fn request_user_input_event() -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::RequestUserInputRequested,
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-turn-live".to_string()),
            item_id: Some("approval".to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: Some(json!({
            "itemId": "approval",
            "questions": [{
                "id": "decision",
                "header": "Approve",
                "question": "Approve this change?",
                "options": [
                    {"label": "Approve", "description": "Continue"},
                    {"label": "Reject", "description": "Stop"}
                ]
            }]
        })),
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn seed_conversation(settings: &SparkSettings, project_path: &Path, conversation_id: &str) {
    let project_path = project_path.to_string_lossy();
    let repository = ConversationRepository::new(&settings.data_dir);
    repository
        .write_snapshot(&json!({
            "schema_version": 5,
            "revision": 2,
            "conversation_id": conversation_id,
            "conversation_handle": "amber-anchor",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "Live route",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:02Z",
            "turns": [{
                "id": "turn-live",
                "role": "assistant",
                "content": "Ready.",
                "timestamp": "2026-01-01T00:00:01Z",
                "status": "complete",
                "kind": "message"
            }],
            "segments": [{
                "id": "segment-live",
                "turn_id": "turn-live",
                "role": "assistant",
                "kind": "message",
                "content": "Ready.",
                "timestamp": "2026-01-01T00:00:02Z",
                "status": "complete",
                "order": 1
            }],
            "event_log": [],
            "flow_run_requests": [],
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": []
        }))
        .expect("write conversation");
    repository
        .append_conversation_event(
            conversation_id,
            &project_path,
            &json!({
                "type": "turn_upsert",
                "revision": 1,
                "conversation_id": conversation_id,
                "project_path": project_path,
                "title": "Live route",
                "updated_at": "2026-01-01T00:00:01Z",
                "turn": {
                    "id": "turn-live",
                    "role": "assistant",
                    "content": "Ready.",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "status": "complete",
                    "kind": "message"
                }
            }),
        )
        .expect("append turn event");
    repository
        .append_conversation_event(
            conversation_id,
            &project_path,
            &json!({
                "type": "segment_upsert",
                "revision": 2,
                "conversation_id": conversation_id,
                "project_path": project_path,
                "title": "Live route",
                "updated_at": "2026-01-01T00:00:02Z",
                "segment": {
                    "id": "segment-live",
                    "turn_id": "turn-live",
                    "role": "assistant",
                    "kind": "message",
                    "content": "Ready.",
                    "timestamp": "2026-01-01T00:00:02Z",
                    "status": "complete",
                    "order": 1
                }
            }),
        )
        .expect("append segment event");
}

fn write_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(path, simple_flow()).expect("flow");
}

fn simple_flow() -> String {
    "digraph LiveRoute { graph [goal=\"Run live route\"] start [shape=Mdiamond] done [shape=Msquare] start -> done }\n".to_string()
}

fn seed_parent_child_run_journals(settings: &SparkSettings, project_path: &Path) {
    let project_path = project_path.to_string_lossy().to_string();
    let store = RunStore::for_settings(settings);
    let mut parent = RunRecord::new("run-live-parent", project_path.clone());
    parent.flow_name = "parent-live.dot".to_string();
    parent.model = "compat-model".to_string();
    parent.started_at = "2026-01-01T00:00:00Z".to_string();
    let parent_paths = store
        .create_run(CreateRunRequest {
            record: parent,
            ..CreateRunRequest::default()
        })
        .expect("parent run");
    let mut child_started = RawRuntimeEvent::new("ChildRunStarted", "run-live-parent");
    child_started.sequence = Some(4);
    child_started.emitted_at = "2026-01-01T00:00:04Z".to_string();
    child_started
        .payload
        .insert("child_run_id".to_string(), json!("run-live-child"));
    child_started
        .payload
        .insert("parent_run_id".to_string(), json!("run-live-parent"));
    child_started
        .payload
        .insert("parent_node_id".to_string(), json!("manager"));
    child_started
        .payload
        .insert("root_run_id".to_string(), json!("run-live-parent"));
    child_started
        .payload
        .insert("child_flow_name".to_string(), json!("child-live.dot"));
    store
        .append_event(&parent_paths, child_started)
        .expect("parent child-started event");

    let mut child = RunRecord::new("run-live-child", project_path);
    child.flow_name = "child-live.dot".to_string();
    child.model = "compat-model".to_string();
    child.started_at = "2026-01-01T00:00:05Z".to_string();
    child.parent_run_id = Some("run-live-parent".to_string());
    child.parent_node_id = Some("manager".to_string());
    child.root_run_id = Some("run-live-parent".to_string());
    child.child_invocation_index = Some(1);
    store
        .create_run(CreateRunRequest {
            record: child,
            ..CreateRunRequest::default()
        })
        .expect("child run");
}

fn latest_journal_sequence(settings: &SparkSettings, run_id: &str) -> u64 {
    RunStore::for_settings(settings)
        .read_run_bundle(run_id)
        .expect("read run")
        .expect("run exists")
        .journal
        .iter()
        .map(|entry| entry.sequence)
        .max()
        .unwrap_or(0)
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("source"),
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
        project_roots: Vec::new(),
    }
}

fn url_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            other => format!("%{other:02X}").chars().collect(),
        })
        .collect()
}
