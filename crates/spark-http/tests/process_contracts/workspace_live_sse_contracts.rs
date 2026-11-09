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
use spark_storage::conversation::{Accepted, Admission, MessageInput, TranscriptSegment};
use spark_storage::ConversationRepository;
use spark_workspace::{
    project_run_milestones, ConversationTurnRequest, TriggerCreateRequest, WorkspaceTriggerService,
};
use tower::ServiceExt;

#[tokio::test]
async fn live_route_returns_sse_keepalive_and_json_cursor_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));

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
        app.clone(),
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
    let app = crate::store_app_default(settings.clone());

    // Without a cursor: the newest window and its watermark.
    let open = request(
        app.clone(),
        "GET",
        &conversation_live_uri("conversation-live", &project_path, None),
        None,
    )
    .await;
    assert_eq!(open.status(), StatusCode::OK);
    let mut open_stream = open.into_body().into_data_stream();
    let window = next_envelope(&mut open_stream).await;
    assert_eq!(window["type"], "conversation.window");
    assert_eq!(window["cursor"], Value::Null);
    assert_eq!(
        window["resource"],
        json!({"kind": "conversation", "id": "conversation-live"})
    );
    let page = &window["payload"];
    assert_eq!(page["conversation_id"], "conversation-live");
    assert_eq!(page["turn_ids"], json!(["turn-live"]));
    assert_eq!(page["older_cursor"], Value::Null);
    let (generation, high) = store_cursor(&settings);
    assert_eq!(page["generation"], generation.as_str());
    assert_eq!(watermark(&window), high);
    assert_eq!(
        carried(std::slice::from_ref(&window), "segment")[0]["id"],
        "segment-live"
    );

    // An older cursor gets the rows changed after it, never a snapshot.
    let replay = request(
        app.clone(),
        "GET",
        &conversation_live_uri("conversation-live", &project_path, Some((&generation, 0))),
        None,
    )
    .await;
    let mut replay_stream = replay.into_body().into_data_stream();
    let replayed = next_envelope(&mut replay_stream).await;
    assert_eq!(replayed["type"], "conversation.changes");
    assert_eq!(replayed["payload"]["from_revision"], "0");
    assert_eq!(watermark(&replayed), high);
    assert_eq!(
        carried(std::slice::from_ref(&replayed), "turn"),
        carried(std::slice::from_ref(&window), "turn")
    );

    // A cursor from another generation, or ahead of the store, resets.
    for (cursor_generation, revision, reason) in [
        ("another-generation", 0, "generation_changed"),
        (generation.as_str(), high + 5, "cursor_ahead"),
    ] {
        let reset = request(
            app.clone(),
            "GET",
            &conversation_live_uri(
                "conversation-live",
                &project_path,
                Some((cursor_generation, revision)),
            ),
            None,
        )
        .await;
        let mut reset_stream = reset.into_body().into_data_stream();
        let envelope = next_envelope(&mut reset_stream).await;
        assert_eq!(envelope["type"], "conversation.reset");
        assert_eq!(envelope["payload"]["code"], "resync_required");
        assert_eq!(envelope["payload"]["reason"], reason);
        assert_eq!(envelope["payload"]["generation"], generation.as_str());
    }
}

#[tokio::test]
async fn live_route_streams_conversation_mutations_on_open_connection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-live");
    let app = crate::store_app(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::new("Live route answer.")),
    );

    let (generation, high) = store_cursor(&settings);
    let live = request(
        app.clone(),
        "GET",
        &conversation_live_uri(
            "conversation-live",
            &project_path,
            Some((&generation, high)),
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    // Caught up already: an empty batch at the same watermark.
    let caught_up = next_envelope(&mut live_stream).await;
    assert_eq!(caught_up["type"], "conversation.changes");
    assert_eq!(caught_up["payload"]["rows"], json!([]));
    assert_eq!(watermark(&caught_up), high);

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
    let acceptance = json_body(posted).await;

    // The accepted turn arrives as the rows its commit changed.
    let accepted = next_of_type(&mut live_stream, "conversation.changes").await;
    assert_eq!(accepted["cursor"], Value::Null);
    assert!(watermark(&accepted) > high);
    assert_eq!(accepted["payload"]["from_revision"], high.to_string());
    let turns = carried(std::slice::from_ref(&accepted), "turn");
    assert!(turns
        .iter()
        .any(|turn| turn["role"] == "user" && turn["content"] == "Please run this live."));
    assert!(turns
        .iter()
        .any(|turn| turn["id"] == acceptance["assistant_turn_id"]));
    // History stays out of the batch: the seeded turn did not change.
    assert!(turns.iter().all(|turn| turn["id"] != "turn-live"));
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
    let app = crate::store_app(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::from_output(backend_output)),
    );

    let (generation, high) = store_cursor(&settings);
    let live = request(
        app.clone(),
        "GET",
        &conversation_live_uri(
            "conversation-live-ingested",
            &project_path,
            Some((&generation, high)),
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut live_stream).await["payload"]["rows"],
        json!([])
    );

    let posted = request(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-live-ingested/turns",
        Some(json!({
            "project_path": project_path,
            "message": "Please stream the ingested backend output."
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);
    let acceptance = json_body(posted).await;
    assert!(acceptance["changes"]["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .any(|row| row["kind"] == "turn"
            && row["payload"]["role"] == "assistant"
            && row["payload"]["status"] == "pending"
            && row["payload"]["content"] == ""));

    // Committed progress arrives as changed rows after each commit; the
    // completed assistant turn signals the turn finished durably.
    let mut envelopes = Vec::new();
    let mut turn_completed = false;
    for _ in 0..40 {
        let envelope = next_envelope(&mut live_stream).await;
        if carried(std::slice::from_ref(&envelope), "turn")
            .iter()
            .any(|turn| {
                turn["role"] == "assistant"
                    && turn["status"] == "complete"
                    && turn["content"] == "Live streamed answer."
            })
        {
            turn_completed = true;
        }
        envelopes.push(envelope);
        if turn_completed {
            break;
        }
    }
    assert!(turn_completed, "completed assistant turn row");
    let final_snapshot = &stored_snapshot(&settings, "conversation-live-ingested");
    let final_revision = final_snapshot["revision"].as_u64().expect("final revision");
    assert!(final_revision > high + 1);
    for _ in 0..40 {
        let last = envelopes
            .iter()
            .rev()
            .find(|envelope| envelope["type"] == "conversation.changes")
            .map(watermark)
            .unwrap_or(0);
        if last >= final_revision {
            break;
        }
        envelopes.push(next_envelope(&mut live_stream).await);
    }
    // Each batch starts where the previous one ended.
    let batches: Vec<&Value> = envelopes
        .iter()
        .filter(|envelope| envelope["type"] == "conversation.changes")
        .collect();
    let watermarks: Vec<u64> = batches.iter().map(|envelope| watermark(envelope)).collect();
    assert!(
        watermarks.windows(2).all(|pair| pair[0] < pair[1]),
        "{watermarks:?}"
    );
    for pair in batches.windows(2) {
        assert_eq!(
            pair[1]["payload"]["from_revision"],
            pair[0]["payload"]["watermark"]
        );
    }
    assert!(final_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| turn["role"] == "assistant"
            && turn["status"] == "complete"
            && turn["content"] == "Live streamed answer."
            && turn["token_usage"]["total"]["inputTokens"] == json!(11)
            && turn["token_usage_breakdown"]["last"]["outputTokens"] == json!(1)));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "reasoning"
            && segment["content"] == "Live route reasoning."));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "model_tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["id"] == "live-tool-1"
            && segment["tool_call"]["arguments"] == json!({"query": "live route"})));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["id"] == "live-tool-1"
            && segment["tool_call"]["output"] == "full live output"));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "agent_event"
            && segment["event_kind"] == "warning"
            && segment["message"] == "Live route compatibility warning."
            && segment["details"]["message"] == "Live route compatibility warning."));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "agent_event"
            && segment["event_kind"] == "processing_end"
            && segment["event_status"] == "idle"
            && segment["details"]["state"] == "idle"));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "agent_event"
            && segment["event_kind"] == "session_end"
            && segment["event_status"] == "closed"
            && segment["details"]["state"] == "closed"));

    assert!(carried(&envelopes, "turn").iter().any(|turn| {
        turn["role"] == "user" && turn["content"] == "Please stream the ingested backend output."
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "assistant_message" && segment["content"] == "Live streamed answer."
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "reasoning" && segment["content"] == "Live route reasoning."
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "model_tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["id"] == "live-tool-1"
            && segment["source"]["raw_kind"] == "model_tool_call_end"
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["output"] == "full live output"
    }));
    assert!(carried(&envelopes, "turn").iter().any(|turn| {
        turn["role"] == "assistant"
            && turn["token_usage"]["total"]["outputTokens"] == json!(4)
            && turn["token_usage_breakdown"]["last"]["inputTokens"] == json!(3)
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "request_user_input"
            && segment["request_user_input"]["status"] == "pending"
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["event_kind"] == "session_start"
            && segment["event_status"] == "processing"
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["event_kind"] == "warning"
            && segment["message"] == "Live route compatibility warning."
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["event_kind"] == "processing_end"
            && segment["event_status"] == "idle"
    }));
    assert!(watermarks.last().copied().unwrap_or(0) >= final_revision);

    let raw_log = ConversationRepository::new(crate::shared_store(&settings.data_dir))
        .read_codex_jsonrpc_trace(
            "conversation-live-ingested",
            &project_path.to_string_lossy(),
        )
        .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
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
    let app = crate::store_app(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::from_output(backend_output)),
    );

    let (generation, high) = store_cursor(&settings);
    let live = request(
        app.clone(),
        "GET",
        &conversation_live_uri(
            "conversation-live-failure",
            &project_path,
            Some((&generation, high)),
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut live_stream).await["payload"]["rows"],
        json!([])
    );

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
    assert!(rows_of(&posted_snapshot["changes"], "turn")
        .iter()
        .any(|turn| turn["role"] == "assistant" && turn["status"] == "pending"));

    let mut envelopes = Vec::new();
    let mut failed_envelope = None;
    for _ in 0..20 {
        let envelope = next_envelope(&mut live_stream).await;
        if carried(std::slice::from_ref(&envelope), "turn")
            .iter()
            .any(|turn| {
                turn["role"] == "assistant"
                    && turn["status"] == "failed"
                    && turn["error"] == "live thread resume failed"
            })
        {
            failed_envelope = Some(envelope.clone());
        }
        envelopes.push(envelope);
        if failed_envelope.is_some() {
            break;
        }
    }

    assert!(carried(&envelopes, "turn").iter().any(|turn| {
        turn["role"] == "assistant"
            && turn["status"] == "failed"
            && turn["error"] == "live thread resume failed"
            && turn["error_code"] == "thread_resume_failed"
            && turn["details"]["thread_id"] == "thread-live-resume"
            && turn["token_usage"]["total"]["inputTokens"] == json!(6)
            && turn["token_usage_breakdown"]["last"]["outputTokens"] == json!(0)
    }));
    // The failure's workflow event is a row of the same batch.
    let failed_envelope = failed_envelope.expect("failed turn batch");
    assert_eq!(failed_envelope["type"], "conversation.changes");
    assert_eq!(
        carried_artifacts(std::slice::from_ref(&failed_envelope), "event_log")[0]["details"]
            ["thread_id"],
        "thread-live-resume"
    );
    let raw_log = ConversationRepository::new(crate::shared_store(&settings.data_dir))
        .read_codex_jsonrpc_trace("conversation-live-failure", &project_path.to_string_lossy())
        .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );
}

#[tokio::test]
async fn live_route_streams_backend_ingested_revision_range_for_request_user_input_answer_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: the SSE subscription and route publishes must agree on the
    // canonical project path (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let project_path = root.join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();
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
    // Built first: startup settles turns left in flight, pending questions included.
    let app = crate::store_app(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::from_output(answer_output)),
    );
    let service = crate::conversations(settings.clone());
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
        .as_u64()
        .expect("pending revision");
    let (generation, _) = store_cursor(&settings);

    let live = request(
        app.clone(),
        "GET",
        &conversation_live_uri(
            "conversation-live-answer",
            &project_path,
            Some((&generation, before_revision)),
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();

    let posted = request(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-live-answer/request-user-input/decision/answer",
        Some(json!({
            "project_path": project_path_text,
            "answers": {"decision": "Approve"}
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);
    // A MutationAck whose rows cover every commit the answer made.
    let ack = json_body(posted).await;
    let final_revision: u64 = ack["changes"]["watermark"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(final_revision > before_revision);
    assert!(rows_of(&ack["changes"], "turn")
        .iter()
        .any(|turn| { turn["id"] == prepared.assistant_turn_id && turn["status"] == "complete" }));

    let mut envelopes = Vec::new();
    for _ in 0..20 {
        let envelope = next_of_type(&mut live_stream, "conversation.changes").await;
        let reached = watermark(&envelope);
        envelopes.push(envelope);
        if reached >= final_revision {
            break;
        }
    }
    // Committed state arrives in increasing watermarks up to the final one.
    let watermarks = envelopes.iter().map(watermark).collect::<Vec<_>>();
    assert!(
        watermarks.windows(2).all(|pair| pair[0] < pair[1]),
        "{watermarks:?}"
    );
    assert!(watermarks.last().copied().unwrap() >= final_revision);

    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "request_user_input"
            && segment["request_user_input"]["status"] == "answered"
            && segment["request_user_input"]["answers"]["decision"] == "Approve"
    }));
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "assistant_message" && segment["content"] == "Answered over live SSE."
    }));
    assert!(carried(&envelopes, "turn").iter().any(|turn| {
        turn["id"] == prepared.assistant_turn_id
            && turn["status"] == "complete"
            && turn["content"] == "Answered over live SSE."
    }));
    assert!(carried(&envelopes, "turn").iter().any(|turn| {
        turn["id"] == prepared.assistant_turn_id
            && turn["token_usage"]["total"]["outputTokens"] == json!(3)
    }));
    // A fresh window after the replay holds the same durable rows the answer
    // route reported.
    let hydrated = request(
        app,
        "GET",
        &format!(
            "/workspace/api/conversations/conversation-live-answer?project_path={}",
            url_encode(&project_path_text)
        ),
        None,
    )
    .await;
    assert_eq!(hydrated.status(), StatusCode::OK);
    let window = json_body(hydrated).await;
    for turn in rows_of(&ack["changes"], "turn") {
        assert!(rows_of(&window, "turn").contains(&turn), "{turn}");
    }

    let raw_log = ConversationRepository::new(crate::shared_store(&settings.data_dir))
        .read_codex_jsonrpc_trace("conversation-live-answer", &project_path.to_string_lossy())
        .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
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

    fn from_output(mut output: AgentTurnOutput) -> Self {
        add_final_answer_event(&mut output);
        Self { output }
    }
}

fn add_final_answer_event(output: &mut AgentTurnOutput) {
    if output
        .events
        .iter()
        .any(|event| event.kind == TurnStreamEventKind::Error)
    {
        return;
    }
    let Some(text) = output.final_assistant_text.clone() else {
        return;
    };
    if output.events.iter().any(|event| {
        event.kind == TurnStreamEventKind::ContentCompleted
            && event.channel == Some(TurnStreamChannel::Assistant)
            && event.phase.as_deref() == Some("final_answer")
    }) {
        return;
    }
    output.events.push(content_completed(
        TurnStreamChannel::Assistant,
        &text,
        "app-turn-static",
        "final-answer",
    ));
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

struct StreamingAgentTurnBackend {
    stream_events: Vec<TurnStreamEvent>,
    output: AgentTurnOutput,
}

impl AgentTurnBackend for StreamingAgentTurnBackend {
    fn run_turn(&self, _request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        Ok(self.output.clone())
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<spark_agent_adapter::AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let output = self.run_turn(request)?;
        if let Some(sink) = event_sink {
            // Every event a backend returns has gone through its sink.
            for event in self.stream_events.iter().chain(&output.events) {
                sink(event.clone());
            }
        }
        Ok(output)
    }
}

#[tokio::test]
async fn live_plan_route_promotes_authoritative_claude_result_in_place() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-live-plan-claude");

    let mut narration = content_completed(
        TurnStreamChannel::Assistant,
        "Inspecting.",
        "app-turn-claude",
        "block-1",
    );
    narration.phase = Some("commentary".to_string());
    let mut text_completion = content_completed(
        TurnStreamChannel::Assistant,
        "Draft answer.",
        "app-turn-claude",
        "block-2",
    );
    text_completion.phase = Some("commentary".to_string());
    let final_answer = content_completed(
        TurnStreamChannel::Assistant,
        "Final answer.",
        "app-turn-claude",
        "block-2",
    );
    let events = vec![narration, text_completion, final_answer];
    let app = crate::store_app(
        settings.clone(),
        Arc::new(StreamingAgentTurnBackend {
            stream_events: events.clone(),
            output: AgentTurnOutput {
                events,
                final_assistant_text: Some("Final answer.".to_string()),
                ..AgentTurnOutput::default()
            },
        }),
    );

    let (generation, high) = store_cursor(&settings);
    let live = request(
        app.clone(),
        "GET",
        &conversation_live_uri(
            "conversation-live-plan-claude",
            &project_path,
            Some((&generation, high)),
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut live_stream).await["payload"]["rows"],
        json!([])
    );

    let posted = request(
        app,
        "POST",
        "/workspace/api/conversations/conversation-live-plan-claude/turns",
        Some(json!({
            "project_path": project_path,
            "message": "Make a plan.",
            "chat_mode": "plan"
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);

    let mut promoted = None;
    for _ in 0..20 {
        let envelope = next_envelope(&mut live_stream).await;
        // A finished segment arrives as a committed row.
        promoted = carried(std::slice::from_ref(&envelope), "segment")
            .into_iter()
            .find(|segment| {
                segment["source"]["item_id"] == "block-2" && segment["phase"] == "final_answer"
            });
        if promoted.is_some() {
            break;
        }
    }
    let segment = &promoted.expect("live final-answer promotion");
    assert_eq!(segment["content"], "Final answer.");
    assert_eq!(segment["status"], "complete");
    assert_eq!(segment["phase"], "final_answer");
}

#[tokio::test]
async fn live_route_streams_transient_deltas_with_stream_sequence_cursors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-transient");
    let app = crate::store_app(
        settings.clone(),
        Arc::new(StreamingAgentTurnBackend {
            stream_events: vec![
                content_delta("Str", "app-turn-transient", "final-answer"),
                content_delta("eaming.", "app-turn-transient", "final-answer"),
                request_user_input_event(),
            ],
            output: AgentTurnOutput {
                events: vec![content_completed(
                    TurnStreamChannel::Assistant,
                    "Streaming.",
                    "app-turn-transient",
                    "final-answer",
                )],
                final_assistant_text: Some("Streaming.".to_string()),
                ..AgentTurnOutput::default()
            },
        }),
    );

    let (generation, high) = store_cursor(&settings);
    let live = request(
        app.clone(),
        "GET",
        &conversation_live_uri(
            "conversation-transient",
            &project_path,
            Some((&generation, high)),
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut live_stream).await["payload"]["rows"],
        json!([])
    );

    let posted = request(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-transient/turns",
        Some(json!({
            "project_path": project_path,
            "message": "Stream transient deltas."
        })),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);

    // A catch-up that reads the finished turn may be sent before the
    // deltas queued meanwhile, so read until both have arrived.
    let mut envelopes = Vec::new();
    let mut turn_completed = false;
    let mut streamed = false;
    for _ in 0..60 {
        let envelope = next_envelope(&mut live_stream).await;
        if carried(std::slice::from_ref(&envelope), "turn")
            .iter()
            .any(|turn| turn["status"] == "complete" && turn["content"] == "Streaming.")
        {
            turn_completed = true;
        }
        streamed |= envelope["type"] == "conversation.stream_delta"
            && envelope["payload"]["text"] == "Streaming.";
        envelopes.push(envelope);
        if turn_completed && streamed {
            break;
        }
    }
    assert!(turn_completed, "committed completion turn upsert arrives");

    // Transient deltas ride conversation.stream_delta envelopes with a
    // stream-sequence cursor, no revision, and coalesced render bodies.
    let deltas = envelopes
        .iter()
        .filter(|envelope| envelope["type"] == "conversation.stream_delta")
        .collect::<Vec<_>>();
    assert!(!deltas.is_empty(), "stream deltas were published live");
    for delta in &deltas {
        assert_eq!(delta["cursor"]["kind"], "conversation_stream_sequence");
        assert_eq!(delta["payload"]["type"], "stream_delta");
        assert!(delta["payload"].get("revision").is_none());
        assert_eq!(
            delta["payload"]["conversation_id"],
            "conversation-transient"
        );
    }
    let sequences = deltas
        .iter()
        .map(|delta| delta["cursor"]["value"].as_i64().expect("sequence"))
        .collect::<Vec<_>>();
    let mut sorted = sequences.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sequences, sorted, "stream sequences strictly increase");
    // Each frame carries its open segment's whole text so far.
    let texts = deltas
        .iter()
        .filter(|delta| delta["payload"]["delta_kind"] == "segment_text")
        .map(|delta| delta["payload"]["text"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["Str", "Streaming."]);

    // The pending request-user-input mid-turn commit publishes durable
    // segment upserts with real revision cursors.
    assert!(carried(&envelopes, "segment").iter().any(|segment| {
        segment["kind"] == "request_user_input"
            && segment["request_user_input"]["status"] == "pending"
    }));

    // Reconnect hydration restores committed state without any stream deltas.
    let hydrated_snapshot = stored_snapshot(&settings, "conversation-transient");
    let final_revision = hydrated_snapshot["revision"]
        .as_u64()
        .expect("final revision");

    // A reconnect catches up on committed rows only, up to the final
    // revision, never stream deltas.
    let replay = request(
        app,
        "GET",
        &conversation_live_uri(
            "conversation-transient",
            &project_path,
            Some((&generation, high)),
        ),
        None,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let mut replay_stream = replay.into_body().into_data_stream();
    let replayed = vec![next_envelope(&mut replay_stream).await];
    assert_eq!(replayed[0]["type"], "conversation.changes");
    assert!(watermark(&replayed[0]) >= final_revision);
    assert!(carried(&replayed, "turn")
        .iter()
        .any(|turn| { turn["content"] == "Streaming." && turn["status"] == "complete" }));
    assert!(hydrated_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| turn["role"] == "assistant"
            && turn["status"] == "complete"
            && turn["content"] == "Streaming."));
    assert!(hydrated_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "request_user_input"
            && segment["request_user_input"]["status"] == "pending"));
}

/// Streams `before`, waits for the test to open the gate, then streams `after`.
struct GatedStreamingBackend {
    before: Vec<TurnStreamEvent>,
    after: Vec<TurnStreamEvent>,
    output: AgentTurnOutput,
    gate: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl AgentTurnBackend for GatedStreamingBackend {
    fn run_turn(&self, _request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        Ok(self.output.clone())
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<spark_agent_adapter::AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let sink = event_sink.expect("event sink");
        for event in &self.before {
            sink(event.clone());
        }
        self.gate
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(60))
            .expect("gate opened");
        for event in &self.after {
            sink(event.clone());
        }
        self.run_turn(request)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_turn_updates_survive_mid_turn_reads_flow_launches_and_title_commits() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project = project_path.to_string_lossy().to_string();
    let conversation_id = "conversation-mid-turn";
    seed_conversation(&settings, &project_path, conversation_id);
    write_flow(&settings, "ops/live");

    let mut commentary = content_completed(
        TurnStreamChannel::Assistant,
        "Looking around.",
        "app-turn-mid",
        "commentary-1",
    );
    commentary.phase = Some("commentary".to_string());
    let final_answer = content_completed(
        TurnStreamChannel::Assistant,
        "Half done. All done.",
        "app-turn-mid",
        "final-answer",
    );
    let (open_gate, gate) = std::sync::mpsc::channel();
    let app = crate::store_app(
        settings.clone(),
        Arc::new(GatedStreamingBackend {
            before: vec![
                commentary.clone(),
                content_delta("Half done.", "app-turn-mid", "final-answer"),
            ],
            after: vec![
                content_delta(" All done.", "app-turn-mid", "final-answer"),
                final_answer.clone(),
            ],
            output: AgentTurnOutput {
                events: vec![commentary, final_answer],
                final_assistant_text: Some("Half done. All done.".to_string()),
                ..AgentTurnOutput::default()
            },
            gate: std::sync::Mutex::new(gate),
        }),
    );

    let (generation, high) = store_cursor(&settings);
    let live = request(
        app.clone(),
        "GET",
        &conversation_live_uri(conversation_id, &project_path, Some((&generation, high))),
        None,
    )
    .await;
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut live_stream).await["payload"]["rows"],
        json!([])
    );

    let posted = request(
        app.clone(),
        "POST",
        &format!("/workspace/api/conversations/{conversation_id}/turns"),
        Some(json!({"project_path": project, "message": "Take your time."})),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);
    let started = json_body(posted).await;
    let turn_id = started["assistant_turn_id"]
        .as_str()
        .expect("assistant turn")
        .to_string();
    let started_revision: i64 = started["revision"].as_str().unwrap().parse().unwrap();

    // Wait until the gated backend has streamed its first half.
    let mut last_sequence;
    loop {
        let envelope = next_envelope(&mut live_stream).await;
        if envelope["type"] != "conversation.stream_delta" {
            continue;
        }
        // The turn's own commits of finished segments move its base forward.
        assert!(envelope["payload"]["base_revision"].as_i64().expect("base") >= started_revision);
        last_sequence = envelope["payload"]["stream_sequence"]
            .as_i64()
            .expect("sequence");
        if envelope["payload"]["text"] == "Half done." {
            break;
        }
    }

    // Mid-turn reads of the snapshot and the thread list never commit.
    let stored = || {
        crate::shared_store(&settings.data_dir)
            .database()
            .read(|read| read.conversation(conversation_id))
            .expect("rows")
    };
    let stored_before = stored();
    let snapshot_uri = format!(
        "/workspace/api/conversations/{conversation_id}?project_path={}",
        url_encode(&project)
    );
    let mut read_revision = None;
    for _ in 0..2 {
        let snapshot = json_body(request(app.clone(), "GET", &snapshot_uri, None).await).await;
        // Reads leave the revision where the turn's own commits put it...
        let revision = page_revision(&snapshot);
        assert_eq!(*read_revision.get_or_insert(revision), revision);
        // ...and show what the turn has finished so far: a running turn is
        // in the window even before its page would reach it.
        assert!(rows_of(&snapshot, "segment")
            .iter()
            .any(|segment| segment["turn_id"] == turn_id.as_str()
                && segment["content"] == "Looking around."
                && segment["status"] == "complete"));
        let threads = request(
            app.clone(),
            "GET",
            &format!(
                "/workspace/api/projects/conversations?project_path={}",
                url_encode(&project)
            ),
            None,
        )
        .await;
        assert_eq!(threads.status(), StatusCode::OK);
    }
    assert_eq!(stored(), stored_before);

    // The agent launches a flow and the thread title is stored mid-turn.
    let handle = rows_of(
        &json_body(request(app.clone(), "GET", &snapshot_uri, None).await).await,
        "conversation",
    )[0]["handle"]
        .as_str()
        .expect("handle")
        .to_string();
    let launched = request(
        app.clone(),
        "POST",
        &format!("/workspace/api/conversations/by-handle/{handle}/flow-run-requests"),
        Some(json!({"flow_name": "ops/live", "summary": "Run it."})),
    )
    .await;
    assert_eq!(launched.status(), StatusCode::OK);
    // The seeded reply is the only completed one, so a title still fits.
    assert!(
        ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .set_generated_title(conversation_id, &project, "Stored title")
            .expect("store title")
    );
    let mid_turn = json_body(request(app.clone(), "GET", &snapshot_uri, None).await).await;
    let mid_turn_revision = page_revision(&mid_turn);
    assert!(mid_turn_revision > started_revision);
    let mid_turn_status = rows_of(&mid_turn, "turn")
        .iter()
        .find(|turn| turn["id"] == turn_id.as_str())
        .expect("assistant turn")["status"]
        .clone();
    assert!(mid_turn_status == "pending" || mid_turn_status == "streaming");

    // Live updates keep flowing after those commits, each newer for its turn,
    // and the turn's final state is committed. A catch-up reading the
    // finished turn may be sent before deltas queued meanwhile.
    open_gate.send(()).expect("open gate");
    let mut late_deltas = Vec::new();
    let (mut completed, mut streamed) = (false, false);
    while !(completed && streamed) {
        let envelope = next_envelope(&mut live_stream).await;
        if envelope["type"] == "conversation.stream_delta"
            && envelope["payload"]["turn_id"] == turn_id.as_str()
        {
            streamed |= envelope["payload"]["delta_kind"] == "segment_text"
                && envelope["payload"]["text"] == "Half done. All done.";
            late_deltas.push(envelope["payload"].clone());
        }
        completed |= carried(std::slice::from_ref(&envelope), "turn")
            .iter()
            .any(|turn| turn["id"] == turn_id.as_str() && turn["status"] == "complete");
    }
    for delta in &late_deltas {
        let sequence = delta["stream_sequence"].as_i64().expect("sequence");
        assert!(
            sequence > last_sequence,
            "stream sequence advances per turn"
        );
        last_sequence = sequence;
        // Stamped with the turn's base, now behind the conversation revision.
        assert!(delta["base_revision"].as_i64().expect("base") < mid_turn_revision);
    }
}

#[tokio::test]
async fn thread_list_feed_receives_a_background_chats_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project = project_path.to_string_lossy().to_string();
    let conversation_id = "conversation-background";
    seed_conversation(&settings, &project_path, conversation_id);
    write_flow(&settings, "ops/live");
    let app = crate::store_app_default(settings.clone());
    let handle = rows_of(
        &json_body(
            request(
                app.clone(),
                "GET",
                &format!(
                    "/workspace/api/conversations/{conversation_id}?project_path={}",
                    url_encode(&project)
                ),
                None,
            )
            .await,
        )
        .await,
        "conversation",
    )[0]["handle"]
        .as_str()
        .expect("handle")
        .to_string();

    // A client following thread lists, not this conversation: it starts
    // from a summary snapshot of every project's threads.
    let live = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?include_conversations=true",
        None,
    )
    .await;
    let mut live_stream = live.into_body().into_data_stream();
    let snapshot = next_envelope(&mut live_stream).await;
    assert_eq!(snapshot["type"], "conversations.snapshot");
    assert_eq!(
        snapshot["resource"],
        json!({"kind": "conversations", "id": null})
    );
    assert_eq!(snapshot["payload"]["project_path"], Value::Null);
    assert!(carried(std::slice::from_ref(&snapshot), "conversation")
        .iter()
        .any(|row| row["handle"] == handle.as_str()));

    let launched = request(
        app.clone(),
        "POST",
        "/workspace/api/runs/launch",
        Some(json!({
            "flow_name": "ops/live",
            "summary": "Launch from a background chat",
            "conversation_handle": handle,
        })),
    )
    .await;
    assert_eq!(launched.status(), StatusCode::OK);
    let run_id = json_body(launched).await["run_id"]
        .as_str()
        .expect("run id")
        .to_string();

    // The launch's commit arrives as the conversation's summary row.
    loop {
        let changes = next_of_type(&mut live_stream, "conversations.changes").await;
        assert_eq!(
            changes["payload"]["from_revision"],
            snapshot["payload"]["watermark"]
        );
        let rows = changes["payload"]["rows"].as_array().expect("rows");
        assert!(rows.iter().all(|row| row["kind"] == "conversation"));
        if let Some(row) = rows.iter().find(|row| row["id"] == conversation_id) {
            assert_eq!(row["payload"]["launched_run_ids"], json!([run_id]));
            break;
        }
    }
}

#[tokio::test]
async fn thread_list_feed_flips_running_across_an_http_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let conversation_id = "conversation-running";
    seed_conversation(&settings, &project_path, conversation_id);
    // Held open until the feed shows it running: a turn that finishes first
    // reaches the feed as one row with `running` already cleared.
    let (open_gate, gate) = std::sync::mpsc::channel();
    let app = crate::store_app(
        settings,
        Arc::new(GatedStreamingBackend {
            before: Vec::new(),
            after: Vec::new(),
            output: AgentTurnOutput {
                final_assistant_text: Some("Finished.".to_string()),
                ..AgentTurnOutput::default()
            },
            gate: std::sync::Mutex::new(gate),
        }),
    );
    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_conversations=true&conversations_project_path={}",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut live_stream).await["type"],
        "conversations.snapshot"
    );

    let posted = request(
        app,
        "POST",
        &format!("/workspace/api/conversations/{conversation_id}/turns"),
        Some(json!({"project_path": project_path, "message": "Work on this."})),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::OK);

    // The summary reports running when the turn starts and clears when it ends.
    let mut started = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "started: {started}");
        let envelope = next_envelope(&mut live_stream).await;
        if envelope["type"] != "conversations.changes" {
            continue;
        }
        let Some(row) = envelope["payload"]["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .find(|row| row["id"] == conversation_id)
            .cloned()
        else {
            continue;
        };
        match row["payload"]["running"].as_bool().expect("running") {
            true => {
                started = true;
                let _ = open_gate.send(());
            }
            false if started => break,
            false => {}
        }
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
        flow_name: Some(crate::install_flow(&settings, simple_flow())),
        working_directory: project_path.to_string_lossy().to_string(),
        model: Some("compat-model".to_string()),
        launch_context: Some(
            [(
                "context.request.path".to_string(),
                json!("changes/CR-1/request.md\nsecond line"),
            )]
            .into(),
        ),
        ..PipelineStartRequest::default()
    });
    assert_eq!(started.status_code, 200);
    let app = crate::store_app_default(settings);

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
    assert_eq!(
        upsert["payload"]["run"]["first_launch_input"],
        "changes/CR-1/request.md"
    );
    assert!(upsert["payload"]["run"].get("launch_context").is_none());
}

#[tokio::test]
async fn live_route_replays_manager_loop_child_journals_with_source_local_cursors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_parent_child_run_journals(&settings, &project_path);
    let app = crate::store_app_default(settings);

    let replay = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?run_id=run-live-parent&run_sequence=0",
        None,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let mut replay_stream = replay.into_body().into_data_stream();
    let mut parent_envelopes = Vec::new();
    for _ in 0..4 {
        parent_envelopes.push(sse_data_json(&next_sse_chunk(&mut replay_stream).await));
    }
    assert_eq!(
        parent_envelopes
            .iter()
            .map(|envelope| envelope["cursor"]["value"].as_i64().expect("parent cursor"))
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(parent_envelopes
        .iter()
        .all(|envelope| envelope["resource"] == json!({"kind": "run", "id": "run-live-parent"})));

    let child_replay = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?run_id=run-live-child&run_sequence=0",
        None,
    )
    .await;
    let mut child_stream = child_replay.into_body().into_data_stream();
    let mut envelopes = Vec::new();
    for _ in 0..3 {
        envelopes.push(sse_data_json(&next_sse_chunk(&mut child_stream).await));
    }
    let cursor_values = envelopes
        .iter()
        .map(|envelope| envelope["cursor"]["value"].as_i64().expect("run cursor"))
        .collect::<Vec<_>>();
    assert_eq!(cursor_values, vec![1, 2, 3]);

    let child = &envelopes[0];
    assert_eq!(child["type"], "run.journal_entry");
    assert_eq!(
        child["resource"],
        json!({"kind": "run", "id": "run-live-child"})
    );
    assert!(envelopes
        .iter()
        .all(|envelope| envelope["payload"]["source_scope"] == "root"));

    let child_cursor = 2;
    let reconnect = request(
        app,
        "GET",
        &format!("/workspace/api/live/events?run_id=run-live-child&run_sequence={child_cursor}"),
        None,
    )
    .await;
    assert_eq!(reconnect.status(), StatusCode::OK);
    let mut reconnect_stream = reconnect.into_body().into_data_stream();
    let next = sse_data_json(&next_sse_chunk(&mut reconnect_stream).await);
    assert_eq!(next["cursor"]["value"].as_i64().expect("next cursor"), 3);
}

#[tokio::test]
async fn live_route_streams_workspace_run_launches_and_selected_run_updates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/live");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let app = crate::store_app_default(settings.clone());

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
            "flow_name": "ops/live",
            "summary": "Launch from open SSE overview",
            "project_path": project_path,
            "model": "compat-model",
            "launch_context": {"context.request.objective": "fix the deploy pipeline"}
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
    assert_eq!(
        upsert["payload"]["run"]["first_launch_input"],
        "fix the deploy pipeline"
    );

    let service = AttractorApiService::new(settings.clone());
    // Wait for terminal state: this section exercises steer-event streaming
    // from a settled journal cursor, not detached-launch behavior.
    let started = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-live-selected".to_string()),
        flow_name: Some(crate::install_flow(&settings, simple_flow())),
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
    write_flow(&settings, "ops/run");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    // Triggers record canonical project paths; a temp dir may sit behind a
    // symlink (macOS /var -> /private/var).
    let project_path = project_path.canonicalize().expect("canonical project");
    trigger_project(&settings, &project_path);
    let app = crate::store_app_default(settings);

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
    let snapshot = next_envelope(&mut live_stream).await;
    assert_eq!(snapshot["type"], "trigger.snapshot");
    assert_eq!(snapshot["payload"], json!({"triggers": []}));

    let settings_live = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?include_settings=true",
        None,
    )
    .await;
    let mut settings_stream = settings_live.into_body().into_data_stream();
    assert_eq!(
        next_sse_chunk(&mut settings_stream).await,
        ": keepalive\n\n"
    );

    let created = request(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project_path,
            "name": "Compat webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/run",
                "static_context": {"origin": "compat"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = json_body(created).await;
    let trigger_id = created_body["id"].as_str().expect("trigger id").to_string();

    let settings_event = sse_data_json(&next_sse_chunk(&mut settings_stream).await);
    assert_eq!(settings_event["type"], "settings.changed");
    assert_eq!(settings_event["payload"]["section"], "triggers");
    let upsert = next_envelope(&mut live_stream).await;
    assert_eq!(upsert["type"], "trigger.upsert");
    assert_eq!(
        upsert["resource"],
        json!({"kind": "trigger", "id": trigger_id})
    );
    assert_eq!(upsert["payload"]["type"], "trigger_upsert");

    let deleted = request(
        app,
        "DELETE",
        &format!(
            "/workspace/api/triggers/{trigger_id}?project_path={}&expected_revision={}",
            url_encode(&project_path.to_string_lossy()),
            created_body["revision"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);

    let settings_event = sse_data_json(&next_sse_chunk(&mut settings_stream).await);
    assert_eq!(settings_event["type"], "settings.changed");
    assert_eq!(settings_event["payload"]["section"], "triggers");
    let delete = next_envelope(&mut live_stream).await;
    assert_eq!(delete["type"], "trigger.delete");
    assert_eq!(
        delete["resource"],
        json!({"kind": "trigger", "id": trigger_id})
    );
    assert_eq!(
        delete["payload"]["trigger"],
        json!({"status": "deleted", "id": trigger_id, "project_path": project_path})
    );
}

/// The handler subscribes before it reads initial state, so a change made
/// while that read runs still reaches the stream. The conversation database
/// is held, keeping the read open, while a trigger is created.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_route_delivers_a_change_made_while_it_reads_initial_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path = project_path.canonicalize().expect("canonical project");
    trigger_project(&settings, &project_path);
    // Seeded through a store that closes again, so the app loads the
    // conversation from the database when the live route first reads it.
    seed_conversation(&settings, &project_path, "conversation-gap");
    let app = crate::store_app_default(settings.clone());
    let store = crate::shared_store(&settings.data_dir);
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    // Holding the database keeps the handler inside its initial read.
    let writer = std::thread::spawn(move || {
        store
            .database()
            .read(|_| {
                held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
    });
    held_rx.recv().unwrap();
    let uri = format!(
        "/workspace/api/live/events?conversation_id=conversation-gap&conversation_project_path={path}&include_triggers=true&triggers_project_path={path}",
        path = url_encode(&project_path.to_string_lossy())
    );
    let live = tokio::spawn({
        let app = app.clone();
        async move { request(app, "GET", &uri, None).await }
    });
    // ponytail: a pause instead of a signal from inside the blocked read.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let created = request(
        app,
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project_path,
            "name": "Gap webhook",
            "source_type": "webhook",
            "action": {"flow_name": "ops/run"},
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let trigger_id = json_body(created).await["id"].as_str().unwrap().to_string();
    release_tx.send(()).unwrap();
    writer.join().unwrap();

    let mut stream = live.await.unwrap().into_body().into_data_stream();
    let mut types = Vec::new();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(10), stream.next())
            .await
            .unwrap_or_else(|_| panic!("no trigger.upsert after {types:?}"))
            .expect("SSE stream item")
            .expect("SSE bytes");
        let frame = String::from_utf8(frame.to_vec()).unwrap();
        if frame.starts_with(':') {
            continue;
        }
        let envelope = sse_data_json(&frame);
        if envelope["type"] == "trigger.upsert" {
            assert_eq!(envelope["resource"]["id"], trigger_id);
            break;
        }
        types.push(envelope["type"].clone());
    }
    assert_eq!(types, ["conversation.window", "trigger.snapshot"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_route_streams_mission_upserts_through_turn_delivery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project = project_path.to_string_lossy().to_string();
    let app = crate::store_app(
        settings,
        Arc::new(StaticAgentTurnBackend::new("Starting now.")),
    );
    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_missions=true&missions_project_path={}",
            url_encode(&project)
        ),
        None,
    )
    .await;
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut live_stream).await, ": keepalive\n\n");
    let base = format!(
        "/workspace/api/missions?project_path={}",
        url_encode(&project)
    );
    let created = json_body(
        request(
            app.clone(),
            "POST",
            &base,
            Some(json!({"fields": {"title": "Live mission"}})),
        )
        .await,
    )
    .await;
    let id = created["id"].as_str().expect("mission id").to_string();
    let upsert = next_envelope(&mut live_stream).await;
    assert_eq!(upsert["type"], "mission.upsert");
    assert_eq!(upsert["resource"], json!({"kind": "mission", "id": id}));
    assert_eq!(
        upsert["payload"]["mission"]["fields"]["title"],
        "Live mission"
    );
    let started = request(
        app.clone(),
        "POST",
        &format!(
            "/workspace/api/missions/{id}/start?project_path={}",
            url_encode(&project)
        ),
        None,
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "mission never settled"
        );
        let frame = next_sse_chunk(&mut live_stream).await;
        if !frame.contains("data: ") {
            continue;
        }
        let envelope = sse_data_json(&frame);
        if envelope["type"] != "mission.upsert" {
            continue;
        }
        // The finished first turn leaves the mission waiting on the user.
        let mission = &envelope["payload"]["mission"];
        if mission["status"] == "needs_you" {
            assert_eq!(mission["conversation_id"], json!(id));
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thread_list_feed_follows_a_standing_roles_lifecycle_and_turns() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let roles = settings.bundled_dir().join("roles");
    fs::create_dir_all(&roles).expect("roles");
    fs::write(
        roles.join("chief-of-staff.md"),
        include_str!("../../../spark-assets/assets/roles/chief-of-staff.md"),
    )
    .expect("role file");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project = project_path.to_string_lossy().to_string();
    // Hold each turn until the test has seen the role working.
    let (open_gate, gate) = std::sync::mpsc::channel();
    let app = crate::store_app(
        settings,
        Arc::new(GatedStreamingBackend {
            before: Vec::new(),
            after: Vec::new(),
            output: StaticAgentTurnBackend::new("On it.").output,
            gate: std::sync::Mutex::new(gate),
        }),
    );
    let live = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?include_conversations=true",
        None,
    )
    .await;
    let mut live_stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut live_stream).await["type"],
        "conversations.snapshot"
    );
    let control = |control: &str| {
        format!(
            "/workspace/api/roles/chief-of-staff/{control}?project_path={}",
            url_encode(&project)
        )
    };
    // The role's summary row reaching `state`, skipping other frames.
    async fn summary_state(stream: &mut axum::body::BodyDataStream, state: &str) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            assert!(tokio::time::Instant::now() < deadline, "role never {state}");
            let envelope = next_envelope(stream).await;
            if envelope["type"] != "conversations.changes" {
                continue;
            }
            if let Some(conversation) = carried(std::slice::from_ref(&envelope), "conversation")
                .into_iter()
                .find(|conversation| {
                    conversation["standing_role"]["name"] == "chief-of-staff"
                        && conversation["standing_role"]["state"] == state
                })
            {
                return conversation;
            }
        }
    }

    let started = request(app.clone(), "POST", &control("start"), None).await;
    assert_eq!(started.status(), StatusCode::OK);
    // A newly started role appears, works its first turn, and settles.
    let working = summary_state(&mut live_stream, "working").await;
    assert_eq!(working["title"], "Chief of Staff");
    assert_eq!(working["running"], true);
    // Release the first turn and any turn the resume below starts.
    open_gate.send(()).expect("gate");
    open_gate.send(()).expect("gate");
    let idle = summary_state(&mut live_stream, "idle").await;
    assert_eq!(idle["running"], false);
    let stopped = json_body(request(app.clone(), "POST", &control("stop"), None).await).await;
    assert_eq!(stopped["state"], "stopped");
    summary_state(&mut live_stream, "stopped").await;
    // The open conversation's own rows carry the same role state: the
    // browser keeps whichever scope delivers a revision first.
    let page = json_body(
        request(
            app.clone(),
            "GET",
            &format!(
                "/workspace/api/conversations/{}?project_path={}",
                stopped["conversation_id"].as_str().unwrap(),
                url_encode(&project)
            ),
            None,
        )
        .await,
    )
    .await;
    let row = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "conversation")
        .unwrap();
    assert_eq!(row["payload"]["standing_role"]["state"], "stopped", "{row}");
    assert_eq!(row["payload"]["title"], "Chief of Staff");
    let resumed = request(app.clone(), "POST", &control("start"), None).await;
    assert_eq!(resumed.status(), StatusCode::OK);
    summary_state(&mut live_stream, "idle").await;
}

/// An app whose bundled library holds the standing role `watcher`.
fn role_app(settings: &SparkSettings) -> axum::Router {
    let roles = settings.bundled_dir().join("roles");
    fs::create_dir_all(&roles).expect("roles");
    fs::write(
        roles.join("watcher.md"),
        "---\ntype: Role\nstanding: true\n---\n\nYou watch.\n",
    )
    .expect("role file");
    crate::store_app(
        settings.clone(),
        Arc::new(StaticAgentTurnBackend::new("On it.")),
    )
}

/// Starts `watcher` and waits for its first turn to settle.
async fn start_watcher(app: &axum::Router, project: &str) {
    let uri = |path: &str| {
        format!(
            "/workspace/api/roles{path}?project_path={}",
            url_encode(project)
        )
    };
    let started = request(app.clone(), "POST", &uri("/watcher/start"), None).await;
    assert_eq!(started.status(), StatusCode::OK);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let roles = json_body(request(app.clone(), "GET", &uri(""), None).await).await;
        if roles["roles"][0]["state"] == "idle" {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{roles}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn watcher_state(envelope: &Value) -> Option<String> {
    carried(std::slice::from_ref(envelope), "conversation")
        .into_iter()
        .find(|conversation| conversation["standing_role"]["name"] == "watcher")
        .and_then(|conversation| {
            conversation["standing_role"]["state"]
                .as_str()
                .map(str::to_string)
        })
}

/// A role's summary change made while nobody is connected reaches a client
/// reconnecting with the cursor it held, and so does a later change to
/// another thread.
#[tokio::test]
async fn a_role_summary_change_during_a_disconnect_arrives_on_reconnect() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project = project_path.to_string_lossy().to_string();
    seed_conversation(&settings, &project_path, "conversation-first");
    let app = role_app(&settings);
    start_watcher(&app, &project).await;
    let (generation, held) = store_cursor(&settings);
    let control = format!(
        "/workspace/api/roles/watcher/stop?project_path={}",
        url_encode(&project)
    );
    let stopped = request(app.clone(), "POST", &control, None).await;
    assert_eq!(stopped.status(), StatusCode::OK);
    assert_eq!(json_body(stopped).await["state"], "stopped");

    let live = request(
        app.clone(),
        "GET",
        &summaries_live_uri(Some(&project_path), Some((&generation, held))),
        None,
    )
    .await;
    let mut stream = live.into_body().into_data_stream();
    let envelope = next_of_type(&mut stream, "conversations.changes").await;
    assert_eq!(
        watcher_state(&envelope).as_deref(),
        Some("stopped"),
        "{envelope}"
    );
    // The change is behind the cursor it reports, so a later reconnect has no gap.
    let next_cursor = watermark(&envelope);
    assert!(next_cursor > held);
    drop(stream);

    let (generation, held) = store_cursor(&settings);
    let touched =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir));
    touched
        .touch_conversation(
            "conversation-first",
            &registered_path(&settings, &project_path),
        )
        .expect("touch");
    let live = request(
        app,
        "GET",
        &summaries_live_uri(Some(&project_path), Some((&generation, held))),
        None,
    )
    .await;
    let mut stream = live.into_body().into_data_stream();
    let envelope = next_of_type(&mut stream, "conversations.changes").await;
    assert_eq!(
        rows_with_id(std::slice::from_ref(&envelope), "conversation-first").len(),
        1,
        "{envelope}"
    );
}

/// A role's change is a committed row, so a change made in the gap and
/// followed by a restart is in the catch-up as well as a fresh snapshot.
#[tokio::test]
async fn a_role_change_before_a_restart_reaches_catch_up_and_snapshots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project = project_path.to_string_lossy().to_string();
    let app = role_app(&settings);
    start_watcher(&app, &project).await;
    let (generation, held) = store_cursor(&settings);
    let control = format!(
        "/workspace/api/roles/watcher/stop?project_path={}",
        url_encode(&project)
    );
    let stopped = request(app.clone(), "POST", &control, None).await;
    assert_eq!(stopped.status(), StatusCode::OK);
    drop(app);

    let restarted = role_app(&settings);
    let live = request(
        restarted.clone(),
        "GET",
        &summaries_live_uri(Some(&project_path), Some((&generation, held))),
        None,
    )
    .await;
    let mut stream = live.into_body().into_data_stream();
    let caught_up = next_of_type(&mut stream, "conversations.changes").await;
    assert_eq!(
        watcher_state(&caught_up).as_deref(),
        Some("stopped"),
        "{caught_up}"
    );

    let live = request(
        restarted,
        "GET",
        &summaries_live_uri(Some(&project_path), None),
        None,
    )
    .await;
    let mut stream = live.into_body().into_data_stream();
    let snapshot = next_of_type(&mut stream, "conversations.snapshot").await;
    assert_eq!(
        watcher_state(&snapshot).as_deref(),
        Some("stopped"),
        "{snapshot}"
    );
}

/// Opening a chat on a connection that already holds a summary cursor serves
/// both scopes: the chat's window, the summary rows after the cursor and,
/// from then on, later changes in each.
#[tokio::test]
async fn opening_a_chat_while_holding_a_summary_cursor_serves_both_scopes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-open");
    let app = crate::store_app_default(settings.clone());
    let (generation, held) = store_cursor(&settings);
    seed_conversation(&settings, &project_path, "conversation-new");

    let uri = format!(
        "{}&include_conversations=true&conversations_project_path={}&conversations_generation={generation}&conversations_revision={held}",
        conversation_live_uri("conversation-open", &project_path, None),
        url_encode(&project_path.to_string_lossy()),
    );
    let live = request(app, "GET", &uri, None).await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut stream = live.into_body().into_data_stream();
    let window = next_envelope(&mut stream).await;
    assert_eq!(window["type"], "conversation.window", "{window}");
    assert_eq!(window["resource"]["id"], "conversation-open");
    let changes = next_envelope(&mut stream).await;
    assert_eq!(changes["type"], "conversations.changes", "{changes}");
    assert_eq!(
        rows_with_id(std::slice::from_ref(&changes), "conversation-new").len(),
        1,
        "{changes}"
    );
    assert!(
        rows_with_id(std::slice::from_ref(&changes), "conversation-open").is_empty(),
        "behind the cursor: {changes}"
    );

    // A later turn in the open chat reaches it and the thread list.
    append_turn(&settings, &project_path, "conversation-open", "turn-later");
    let (mut chat, mut summary) = (false, false);
    while !(chat && summary) {
        let envelope = next_envelope(&mut stream).await;
        chat |= envelope["type"] == "conversation.changes"
            && !rows_with_id(std::slice::from_ref(&envelope), "turn-later").is_empty();
        summary |= envelope["type"] == "conversations.changes"
            && !rows_with_id(std::slice::from_ref(&envelope), "conversation-open").is_empty();
    }
}

#[tokio::test]
async fn live_route_streams_source_activation_trigger_upserts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    trigger_project(&settings, &project_path);
    let project_path_text = project_path.to_string_lossy().to_string();
    let app = crate::store_app_default(settings);

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
    let snapshot = next_envelope(&mut live_stream).await;
    assert_eq!(snapshot["type"], "trigger.snapshot");
    assert_eq!(snapshot["payload"], json!({"triggers": []}));

    let created = request(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project_path_text,
            "name": "Due schedule",
            "source_type": "schedule",
            "action": {
                "flow_name": "ops/run",
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
        let envelope = next_envelope(&mut live_stream).await;
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
    write_flow(&settings, "ops/webhook-live");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    trigger_project(&settings, &project_path);
    let project_path_text = project_path.to_string_lossy().to_string();
    let app = crate::store_app_default(settings);

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
            "project_path": project_path_text,
            "name": "Webhook live",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/webhook-live",
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
        "ops/webhook-live"
    );
}

#[tokio::test]
async fn live_route_streams_workflow_log_tail_and_new_milestones() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/workflow-log");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    trigger_project(&settings, &project_path);
    let project_path_text = project_path.to_string_lossy().to_string();

    // Seed one milestone before any stream connects: it must arrive as the
    // initial tail for include_workflow_log subscribers.
    let store = RunStore::for_settings(&settings);
    let mut seeded = RunRecord::new("run-log-seeded", "/spark-contract-fixture/project");
    seeded.flow_name = "ops/seeded".to_string();
    seeded.started_at = "2026-01-01T00:00:00Z".to_string();
    store
        .create_run(CreateRunRequest {
            record: seeded,
            ..CreateRunRequest::default()
        })
        .expect("seed run");
    let seeded_entries =
        project_run_milestones(&settings, "run-log-seeded", &[]).expect("seed milestones");
    assert_eq!(seeded_entries.len(), 1);

    let app = crate::store_app_default(settings.clone());

    let workflow_live = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?include_workflow_log=true",
        None,
    )
    .await;
    assert_eq!(workflow_live.status(), StatusCode::OK);
    let mut workflow_stream = workflow_live.into_body().into_data_stream();
    let tail_entry = sse_data_json(&next_sse_chunk(&mut workflow_stream).await);
    assert_eq!(tail_entry["type"], "workflow_log.entry");
    assert_eq!(tail_entry["resource"]["kind"], "workflow_log");
    assert_eq!(tail_entry["payload"]["kind"], "run_started");
    assert_eq!(tail_entry["payload"]["run_id"], "run-log-seeded");
    assert_eq!(tail_entry["payload"]["flow_name"], "ops/seeded");
    assert_eq!(tail_entry["cursor"]["kind"], "workflow_log_seq");

    // A stream without the flag never sees workflow log envelopes: its first
    // frame stays the keepalive even while milestones are published below.
    let plain_live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_runs_overview=true&runs_project_path={}",
            url_encode(&project_path_text)
        ),
        None,
    )
    .await;
    let mut plain_stream = plain_live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut plain_stream).await, ": keepalive\n\n");

    // Dispatch a webhook-triggered run: publish_live_run_after projects and
    // publishes the run_started milestone live.
    let created = request(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project_path_text,
            "name": "Workflow log live",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/workflow-log",
                "static_context": {"origin": "workflow-log"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = json_body(created).await;
    let webhook_key = created_body["source"]["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let webhook_secret = created_body["webhook_secret"]
        .as_str()
        .expect("webhook secret")
        .to_string();

    let accepted = request_with_headers(
        app,
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", webhook_key.as_str()),
            ("X-Spark-Webhook-Secret", webhook_secret.as_str()),
            ("X-Spark-Webhook-Request-Id", "workflow-log-request"),
        ],
        Some(json!({"payload": "workflow-log"})),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);

    let live_entry = sse_data_json(&next_sse_chunk(&mut workflow_stream).await);
    assert_eq!(live_entry["type"], "workflow_log.entry");
    assert_eq!(live_entry["payload"]["kind"], "run_started");
    assert_eq!(live_entry["payload"]["flow_name"], "ops/workflow-log");
    assert_ne!(live_entry["payload"]["run_id"], "run-log-seeded");

    // The plain stream's next frame is run detail traffic, not workflow log.
    let plain_next = sse_data_json(&next_sse_chunk(&mut plain_stream).await);
    assert_ne!(plain_next["resource"]["kind"], "workflow_log");
}

#[tokio::test]
async fn dropping_app_cancels_trigger_source_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run");
    let app = crate::store_app_default(settings.clone());
    drop(app);

    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    trigger_project(&settings, &project_path);
    let service =
        WorkspaceTriggerService::new(settings.clone(), crate::shared_store(&settings.data_dir));
    let created = service
        .create_trigger(TriggerCreateRequest {
            project_path: project_path.to_string_lossy().into_owned(),
            message: None,
            name: "Dropped app schedule".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: json!({
                "flow_name": "ops/run",
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
    let stored = service
        .get_trigger(&created.project_path, &created.id)
        .expect("stored trigger");
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
    // Must exceed the 15s keepalive interval: between keepalives a quiet
    // stream legitimately produces no frames, and a loaded test host can
    // stretch any gap. Callers bound overall progress with their own
    // deadlines.
    let chunk = tokio::time::timeout(Duration::from_secs(60), stream.next())
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
    crate::seed_conversation(
        &settings.data_dir,
        &json!({
            "schema_version": 5, "revision": 1, "conversation_id": conversation_id,
            "project_path": project_path.to_string_lossy(),
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:02Z",
            "turns": [{
                "id": "turn-live", "role": "assistant", "content": "Ready.",
                "timestamp": "2026-01-01T00:00:01Z", "status": "complete", "kind": "message"
            }],
            "segments": [{
                "id": "segment-live", "turn_id": "turn-live", "role": "assistant",
                "kind": "message", "content": "Ready.", "timestamp": "2026-01-01T00:00:02Z",
                "status": "complete", "order": 1
            }],
        }),
    );
}

fn write_flow(settings: &SparkSettings, name: &str) {
    attractor_dsl::write_flow_dir(
        &settings.bundled_dir().join("flows").join(name),
        &attractor_dsl::validated_flow(serde_json::from_value(simple_flow()).expect("flow JSON"))
            .expect("valid flow"),
    )
    .expect("write flow");
}

fn simple_flow() -> Value {
    json!({
        "schema_version": "2",
        "id": "live-route",
        "title": "Live Route",
        "goal": "Run live route",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "done", "description": "Enter the first workflow step."}
                }
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

fn seed_parent_child_run_journals(settings: &SparkSettings, project_path: &Path) {
    let project_path = project_path.to_string_lossy().to_string();
    let store = RunStore::for_settings(settings);
    let mut parent = RunRecord::new("run-live-parent", project_path.clone());
    parent.flow_name = "parent-live".to_string();
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
        .insert("child_flow_name".to_string(), json!("child-live"));
    store
        .append_event(&parent_paths, child_started)
        .expect("parent child-started event");

    let mut child = RunRecord::new("run-live-child", project_path);
    child.flow_name = "child-live".to_string();
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
    crate::isolate_home();
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

/// Registers the project so triggers can be written in it.
fn trigger_project(settings: &SparkSettings, project: &Path) {
    spark_storage::ProjectRegistry::new(settings.data_dir.clone())
        .ensure_project_paths(&project.to_string_lossy())
        .expect("register project");
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

#[tokio::test]
async fn live_route_streams_detached_run_progress_mid_flight() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();

    // A custom handler slow enough that the run is verifiably mid-flight when
    // the launch response and first live frames arrive. Slow tool nodes hold
    // the run open without needing a custom handler type.
    let app = crate::store_app_default(settings.clone());

    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?include_runs_overview=true&runs_project_path={}",
            url_encode(&project_path_text)
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut stream = live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut stream).await, ": keepalive\n\n");

    let launched = request(
        app.clone(),
        "POST",
        "/attractor/pipelines",
        Some(json!({
            "flow_name": crate::install_flow(&settings, json!({
                "schema_version": "2",
                "id": "slow_live",
                "title": "Slow Live",
                "nodes": {
                    "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "a", "description": "Enter the first workflow step."}}},
                    "a": {
                        "kind": "tool",
                        "config": {"kind": "tool", "command": "sleep 1", "exit_codes": {"0": "done"}},
                        "routes": {"done": {"to": "b", "description": "The first sleep finished."}}
                    },
                    "b": {
                        "kind": "tool",
                        "config": {"kind": "tool", "command": "sleep 1", "exit_codes": {"0": "done"}},
                        "routes": {"done": {"to": "done", "description": "The second sleep finished."}}
                    },
                    "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
                }
            })),
            "working_directory": project_path_text,
            "run_id": "run-mid-flight",
            "model": "compat-model"
        })),
    )
    .await;
    assert_eq!(launched.status(), StatusCode::OK);
    let launch_body = json_body(launched).await;
    assert_eq!(launch_body["status"], "started");
    assert_eq!(launch_body["terminal_status"], "running");

    // While the run executes, the bridge must deliver a workflow-log
    // run_started milestone and a run.upsert whose status is still running.
    let store = RunStore::for_settings(&settings);
    let mut saw_running_upsert = false;
    let mut saw_terminal_upsert = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !(saw_running_upsert && saw_terminal_upsert) {
        assert!(
            std::time::Instant::now() < deadline,
            "missing frames: running_upsert={saw_running_upsert} terminal_upsert={saw_terminal_upsert}",
        );
        let frame = next_sse_chunk(&mut stream).await;
        if frame.starts_with(": keepalive") {
            continue;
        }
        let envelope = sse_data_json(&frame);
        if envelope["type"] == "run.upsert"
            && envelope["payload"]["run"]["run_id"] == "run-mid-flight"
        {
            match envelope["payload"]["run"]["status"].as_str() {
                Some("running") => {
                    // Confirm the run really is still executing on disk.
                    let record_status = store
                        .read_run_bundle("run-mid-flight")
                        .expect("bundle")
                        .and_then(|bundle| bundle.record)
                        .map(|record| record.status);
                    if record_status.as_deref() == Some("running") {
                        saw_running_upsert = true;
                    }
                }
                Some("completed") => saw_terminal_upsert = true,
                _ => {}
            }
        }
    }
    drop(app);
}

#[tokio::test]
async fn live_route_streams_gate_lifecycle_from_waiting_to_answered() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("gate-project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();

    // Production-shaped runner: blocking human gates enabled.
    let factory: attractor_api::RuntimeHandlerRunnerFactory =
        Arc::new(|| attractor_runtime::RuntimeHandlerRunner::new().with_blocking_human_gates());
    let app = spark_http::build_app_with_runtime_handler_runner_factory(settings.clone(), factory)
        .expect("app");

    let launched = request(
        app.clone(),
        "POST",
        "/attractor/pipelines",
        Some(json!({
            "flow_name": crate::install_flow(&settings, json!({
                "schema_version": "2",
                "id": "gate_live",
                "title": "Gate Live",
                "nodes": {
                    "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "review", "description": "Enter the first workflow step."}}},
                    "review": {
                        "kind": "human_gate",
                        "config": {"kind": "human_gate", "prompt": "Ship it?"},
                        "routes": {"approve": {"to": "approved", "label": "Approve", "description": "The change is approved for shipping."}}
                    },
                    "approved": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "ship"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {"done": {"to": "done", "description": "The change shipped."}}
                    },
                    "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
                }
            })),
            "working_directory": project_path_text,
            "run_id": "run-gate-live",
            "model": "compat-model"
        })),
    )
    .await;
    assert_eq!(launched.status(), StatusCode::OK);

    let live = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?run_id=run-gate-live&run_sequence=0",
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut stream = live.into_body().into_data_stream();

    // Phase 1: the gate opens — the pending question arrives via journal
    // replay (published before this subscription); the record itself parks
    // in 'waiting' on disk.
    let store = RunStore::for_settings(&settings);
    let wait_deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let status = store
            .read_run_bundle("run-gate-live")
            .expect("bundle")
            .and_then(|bundle| bundle.record)
            .map(|record| record.status);
        if status.as_deref() == Some("waiting") {
            break;
        }
        assert!(
            std::time::Instant::now() < wait_deadline,
            "run never parked in waiting (last: {status:?})",
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut question_id: Option<String> = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while question_id.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "gate lifecycle frames missing: question={question_id:?}",
        );
        let frame = next_sse_chunk(&mut stream).await;
        if frame.starts_with(": keepalive") {
            continue;
        }
        let envelope = sse_data_json(&frame);
        if envelope["type"] == "run.question_pending" {
            let payload_question_id = envelope["payload"]["payload"]["question_id"]
                .as_str()
                .or_else(|| envelope["payload"]["question_id"].as_str())
                .map(str::to_string);
            if payload_question_id.is_some() {
                question_id = payload_question_id;
            }
        }
    }
    let question_id = question_id.expect("question id");

    // The pending question is also visible on the questions route.
    let questions = request(
        app.clone(),
        "GET",
        "/attractor/pipelines/run-gate-live/questions",
        None,
    )
    .await;
    let questions_body = json_body(questions).await;
    assert_eq!(
        questions_body["questions"][0]["question_id"],
        json!(question_id.clone()),
    );

    // Phase 2: answer through the API route; the waiting gate resumes, the
    // run routes down the approved edge and completes.
    let answered = request(
        app.clone(),
        "POST",
        &format!("/attractor/pipelines/run-gate-live/questions/{question_id}/answer"),
        Some(json!({"question_id": question_id, "selected_value": "approve"})),
    )
    .await;
    assert_eq!(answered.status(), StatusCode::OK);

    let mut saw_question_answered = false;
    let mut saw_pipeline_completed = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !saw_question_answered || !saw_pipeline_completed {
        assert!(
            std::time::Instant::now() < deadline,
            "post-answer frames missing: answered={saw_question_answered} completed={saw_pipeline_completed}",
        );
        let frame = next_sse_chunk(&mut stream).await;
        if frame.starts_with(": keepalive") {
            continue;
        }
        let envelope = sse_data_json(&frame);
        match envelope["type"].as_str() {
            Some("run.question_answered") => saw_question_answered = true,
            Some("run.journal_entry") => {
                if envelope["payload"]["raw_type"] == "PipelineCompleted" {
                    saw_pipeline_completed = true;
                }
            }
            _ => {}
        }
    }

    // The run resumed, took the approved route, and completed.
    let final_status = store
        .read_run_record(
            &store
                .find_run_root("run-gate-live")
                .expect("find root")
                .expect("root"),
        )
        .expect("record")
        .expect("record")
        .status;
    assert_eq!(final_status, "completed");
    let bundle = store
        .read_run_bundle("run-gate-live")
        .expect("bundle")
        .expect("run exists");
    assert!(bundle
        .raw_events
        .iter()
        .any(|event| event.event_type == "StageCompleted"
            && event
                .payload
                .get("node_id")
                .and_then(|value| value.as_str())
                == Some("approved")));
    drop(app);
}

#[tokio::test]
async fn live_route_streams_execution_transcript_upserts_through_the_publisher() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let project_path = root.join("segment-project");
    fs::create_dir_all(&project_path).expect("project");
    let store = RunStore::for_settings(&settings);

    // A failed run with a stored flow snapshot, checkpoint, and journaled
    // agent stream events: retrying it flows through the observed store, so
    // the coalescing publisher must emit run.segment_upsert envelopes.
    let mut record = attractor_core::RunRecord::new(
        "run-segment-live",
        project_path.to_string_lossy().to_string(),
    );
    record.flow_name = "segment-live".to_string();
    record.status = "failed".to_string();
    let flow = json!({
        "schema_version": "2",
        "id": "seg_live",
        "title": "Seg Live",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "done", "description": "Enter the first workflow step."}
                }
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    });
    let checkpoint = attractor_core::CheckpointState {
        timestamp: "2026-07-08T12:00:00Z".to_string(),
        current_node: "start".to_string(),
        completed_nodes: Vec::new(),
        context: Default::default(),
        retry_counts: Default::default(),
        visit_counts: Default::default(),
        logs: Vec::new(),
    };
    let paths = store
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint),
            flow_definition_json: Some(flow.to_string()),
            ..CreateRunRequest::default()
        })
        .expect("seed run");
    let execution_root = store
        .node_execution_root(&paths, "implement", 1, 0)
        .expect("execution root");
    fs::create_dir_all(&execution_root).expect("execution directory");
    let activity = spark_storage::ActivityRepository::new(execution_root);
    let event = activity
        .append_event(json!({"type": "content_completed"}), "2026-07-08T12:00:01Z")
        .expect("execution event");
    let segment: TranscriptSegment = serde_json::from_value(json!({
        "id": "assistant-1", "turn_id": "response", "order": 1,
        "kind": "assistant_message", "role": "assistant", "status": "complete",
        "timestamp": "2026-07-08T12:00:01Z", "content": "Streamed answer."
    }))
    .expect("segment");
    activity
        .append_transcript(&spark_storage::TranscriptRecord::SegmentUpsert {
            revision: 1,
            committed_at: "2026-07-08T12:00:01Z".to_string(),
            source_event_sequence: event.sequence,
            segment,
        })
        .expect("execution transcript");

    let app = crate::store_app_default(settings.clone());
    let before_sequence = latest_journal_sequence(&settings, "run-segment-live");
    let live = request(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/live/events?run_id=run-segment-live&run_sequence={before_sequence}"
        ),
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut stream = live.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut stream).await, ": keepalive\n\n");

    let retried = request(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-segment-live/retry",
        Some(json!({})),
    )
    .await;
    assert_eq!(retried.status(), StatusCode::OK);

    // The publisher coalesces observer notifications and must deliver the
    // projected segment while the retried run progresses.
    let mut saw_segment_upsert = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !saw_segment_upsert {
        assert!(
            std::time::Instant::now() < deadline,
            "never received run.segment_upsert",
        );
        let frame = next_sse_chunk(&mut stream).await;
        if frame.starts_with(": keepalive") {
            continue;
        }
        let envelope = sse_data_json(&frame);
        if envelope["type"] == "conversation.segment_upsert" {
            assert_eq!(envelope["resource"]["kind"], "node_execution");
            assert_eq!(envelope["resource"]["id"], "run-segment-live:implement:1:0");
            assert_eq!(
                envelope["cursor"],
                json!({"kind": "transcript_revision", "value": 1})
            );
            let segment = &envelope["payload"]["record"]["segment"];
            assert_eq!(segment["kind"], "assistant_message");
            assert_eq!(segment["content"], "Streamed answer.");
            assert_eq!(envelope["payload"]["node_id"], "implement");
            saw_segment_upsert = true;
        }
    }
    drop(app);
}

struct SlowStreamingCodergenBackend;

impl spark_agent_adapter::CodergenBackend for SlowStreamingCodergenBackend {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        spark_agent_adapter::CodergenBackend::run_with_event_sink(self, request, None)
    }

    fn run_with_event_sink(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
        event_sink: Option<spark_agent_adapter::CodergenEventSink>,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let streamed = spark_agent_adapter::CodergenEvent::new(
            "rust_agent_session_event",
            std::collections::BTreeMap::from([
                ("node_id".to_string(), json!(request.node_id.clone())),
                (
                    "turn_stream_event".to_string(),
                    json!({
                        "kind": "content_completed",
                        "channel": "assistant",
                        "content_delta": "Mid-node text",
                        "message": "Mid-node text",
                        "source": {"backend": "rust_unified_llm_adapter"},
                    }),
                ),
            ]),
        );
        if let Some(sink) = &event_sink {
            sink(streamed.clone());
        }
        // Keep the node running long enough for the live layer to publish
        // the streamed event while execution is still inside this node.
        std::thread::sleep(Duration::from_millis(900));
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text(
                "{\"decision\":\"done\"}".to_string(),
            ),
            events: vec![streamed],
            usage: None,
            resume_metadata: Default::default(),
        })
    }
}

#[tokio::test]
async fn live_route_streams_codergen_segments_while_the_node_executes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();

    let factory: attractor_api::RuntimeHandlerRunnerFactory = Arc::new(|| {
        attractor_runtime::RuntimeHandlerRunner::new()
            .with_codergen_backend_factory(|| Box::new(SlowStreamingCodergenBackend))
    });
    let app = spark_http::build_app_with_runtime_handler_runner_factory(settings.clone(), factory)
        .expect("app");

    // Detached launch returns at prepare time; the codergen node then holds
    // for ~900ms, leaving a wide window to subscribe and observe mid-node.
    let launched = request(
        app.clone(),
        "POST",
        "/attractor/pipelines",
        Some(json!({
            "flow_name": crate::install_flow(&settings, json!({
                "schema_version": "2",
                "id": "mid_node",
                "title": "Mid Node",
                "nodes": {
                    "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "work", "description": "Enter the first workflow step."}}},
                    "work": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "stream"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {"done": {"to": "done", "description": "The streamed work is complete."}}
                    },
                    "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
                }
            })),
            "working_directory": project_path_text,
            "run_id": "run-mid-node",
            "model": "compat-model"
        })),
    )
    .await;
    assert_eq!(launched.status(), StatusCode::OK);

    let live = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?run_id=run-mid-node&run_sequence=0",
        None,
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut stream = live.into_body().into_data_stream();

    // The streamed segment must arrive while the codergen node is still
    // executing (the backend holds the node open for ~900ms after sinking).
    let store = RunStore::for_settings(&settings);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "never received a mid-node run.segment_upsert",
        );
        let frame = next_sse_chunk(&mut stream).await;
        if frame.starts_with(": keepalive") {
            continue;
        }
        let envelope = sse_data_json(&frame);
        if envelope["type"] != "conversation.segment_upsert" {
            continue;
        }
        let segment = &envelope["payload"]["record"]["segment"];
        if segment["content"] != "Mid-node text" {
            continue;
        }
        assert_eq!(envelope["resource"]["kind"], "node_execution");
        assert_eq!(envelope["payload"]["node_id"], "work");
        assert_eq!(segment["status"], "complete");
        // Proof of mid-node delivery: the run record on disk is still running
        // and the stage has not completed.
        let bundle = store
            .read_run_bundle("run-mid-node")
            .expect("bundle")
            .expect("run exists");
        assert_eq!(bundle.record.expect("record").status, "running");
        assert!(
            !bundle.raw_events.iter().any(|event| {
                event.event_type == "StageCompleted"
                    && event
                        .payload
                        .get("node_id")
                        .and_then(|value| value.as_str())
                        == Some("work")
            }),
            "segment must arrive before the codergen stage completes",
        );
        break;
    }
    drop(app);
}

#[tokio::test]
async fn settings_only_stream_receives_conversation_resource_changes() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    seed_conversation(&settings, &project_path, "settings-live");
    let app = crate::store_app_default(settings);
    let conversation = json_body(
        request(
            app.clone(),
            "GET",
            &format!(
                "/workspace/api/conversations/settings-live?project_path={}",
                url_encode(&project_path.to_string_lossy())
            ),
            None,
        )
        .await,
    )
    .await;
    let response = request(
        app.clone(),
        "GET",
        "/workspace/api/live/events?include_settings=true",
        None,
    )
    .await;
    let mut stream = response.into_body().into_data_stream();
    assert_eq!(next_sse_chunk(&mut stream).await, ": keepalive\n\n");
    for (path, payload, section) in [(
        "/workspace/api/conversations/settings-live/settings",
        json!({"project_path":project_path, "expected_revision":rows_of(&conversation, "conversation")[0]["settings"]["models"]["revision"], "model_settings":{"provider":"claude-code"}}),
        "models",
    )] {
        let saved = request(app.clone(), "PUT", path, Some(payload)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", json_body(saved).await);
        let event = sse_data_json(&next_sse_chunk(&mut stream).await);
        assert_eq!(event["type"], "settings.changed");
        assert_eq!(event["resource"]["kind"], "settings");
        assert_eq!(event["payload"]["section"], section);
        assert!(!event["payload"]["revision"].is_null());
    }
}

/// One conversation's full committed state, for assertions only.
fn stored_snapshot(settings: &SparkSettings, conversation_id: &str) -> Value {
    ConversationRepository::new(crate::shared_store(&settings.data_dir))
        .read_snapshot(conversation_id, None)
        .expect("read")
        .expect("stored conversation")
}

/// The live payloads of one row kind that conversation envelopes carry:
/// rows of windows, change batches and summary snapshots.
fn carried(envelopes: &[Value], kind: &str) -> Vec<Value> {
    envelopes
        .iter()
        .filter(|envelope| {
            matches!(
                envelope["type"].as_str(),
                Some(
                    "conversation.window"
                        | "conversation.changes"
                        | "conversations.snapshot"
                        | "conversations.changes"
                )
            )
        })
        .flat_map(|envelope| {
            envelope["payload"]["rows"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter(|row| row["kind"] == kind && row["deleted"] == false)
        .map(|row| row["payload"].clone())
        .collect()
}

/// A Page's conversation row revision.
fn page_revision(page: &Value) -> i64 {
    page["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["kind"] == "conversation")
        .and_then(|row| row["revision"].as_str())
        .expect("conversation row")
        .parse()
        .expect("decimal revision")
}

/// The payloads of one row kind in a Batch (an acceptance's `changes`).
fn rows_of(batch: &Value, kind: &str) -> Vec<Value> {
    batch["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == kind && row["deleted"] == false)
        .map(|row| row["payload"].clone())
        .collect()
}

/// The artifact payloads of one collection that conversation envelopes carry.
fn carried_artifacts(envelopes: &[Value], collection: &str) -> Vec<Value> {
    envelopes
        .iter()
        .flat_map(|envelope| {
            envelope["payload"]["rows"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter(|row| row["kind"] == "artifact" && row["collection"] == collection)
        .map(|row| row["payload"].clone())
        .collect()
}

/// The next data envelope, skipping keepalives.
async fn next_envelope(stream: &mut axum::body::BodyDataStream) -> Value {
    // Keepalives alone must not let a quiet feed wait forever.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "no live envelope");
        let chunk = next_sse_chunk(stream).await;
        if !chunk.starts_with(':') {
            return sse_data_json(&chunk);
        }
    }
}

/// The next envelope of `event_type`, skipping others and keepalives.
async fn next_of_type(stream: &mut axum::body::BodyDataStream, event_type: &str) -> Value {
    loop {
        let envelope = next_envelope(stream).await;
        if envelope["type"] == event_type {
            return envelope;
        }
    }
}

/// A conversation envelope's decimal-string watermark.
fn watermark(envelope: &Value) -> u64 {
    envelope["payload"]["watermark"]
        .as_str()
        .expect("watermark")
        .parse()
        .expect("decimal watermark")
}

/// The live route for one conversation, with an optional catch-up cursor.
fn conversation_live_uri(
    conversation_id: &str,
    project_path: &Path,
    cursor: Option<(&str, u64)>,
) -> String {
    let mut uri = format!(
        "/workspace/api/live/events?conversation_id={conversation_id}&conversation_project_path={}",
        url_encode(&project_path.to_string_lossy())
    );
    if let Some((generation, revision)) = cursor {
        uri.push_str(&format!(
            "&conversation_generation={generation}&conversation_revision={revision}"
        ));
    }
    uri
}

/// The store's generation and current watermark.
fn store_cursor(settings: &SparkSettings) -> (String, u64) {
    let store = crate::shared_store(&settings.data_dir);
    let database = store.database();
    let watermark = database.read(|read| Ok(read.watermark())).unwrap();
    (database.generation().to_string(), watermark as u64)
}

/// Queues one user message in a conversation, returning its revision.
fn append_turn(
    settings: &SparkSettings,
    project_path: &Path,
    conversation_id: &str,
    id: &str,
) -> u64 {
    let repository = ConversationRepository::new(crate::shared_store(&settings.data_dir));
    let settings_revision = repository
        .read_head(conversation_id, None)
        .expect("read")
        .map_or(0, |head| head["settings_revision"].as_i64().unwrap());
    let (commit, accepted) = repository
        .accept_message(
            conversation_id,
            &project_path.to_string_lossy(),
            MessageInput {
                admission: Admission::Send,
                user_turn_id: id.to_string(),
                content: id.to_string(),
                attachments: None,
                client_request_sha256: None,
                expected_settings_revision: settings_revision,
                settings: Default::default(),
                held: true,
                assistant_turn_id: format!("{id}-reply"),
                execution_settings: json!({}),
            },
        )
        .expect("append turn");
    assert_eq!(accepted, Accepted::Queued);
    commit.expect("commit").revision as u64
}

/// The live route for summary rows, optionally filtered and with a cursor.
fn summaries_live_uri(project_path: Option<&Path>, cursor: Option<(&str, u64)>) -> String {
    let mut uri = "/workspace/api/live/events?include_conversations=true".to_string();
    if let Some(project_path) = project_path {
        uri.push_str(&format!(
            "&conversations_project_path={}",
            url_encode(&project_path.to_string_lossy())
        ));
    }
    if let Some((generation, revision)) = cursor {
        uri.push_str(&format!(
            "&conversations_generation={generation}&conversations_revision={revision}"
        ));
    }
    uri
}

/// Rows of one id across envelopes, tombstones included.
fn rows_with_id(envelopes: &[Value], id: &str) -> Vec<Value> {
    envelopes
        .iter()
        .flat_map(|envelope| {
            envelope["payload"]["rows"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter(|row| row["id"] == id)
        .collect()
}

/// A commit racing the subscription's initial read reaches the browser
/// exactly once: in the window when it committed first, otherwise as a
/// change after the window's watermark. Same for a summary scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initial_windows_and_snapshots_hand_off_to_catch_up_without_gaps_or_duplicates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-handoff");
    let app = crate::store_app_default(settings.clone());
    let store = crate::shared_store(&settings.data_dir);
    for (uri, raced_id) in [
        (
            conversation_live_uri("conversation-handoff", &project_path, None),
            "turn-raced",
        ),
        (
            summaries_live_uri(Some(&project_path), None),
            "conversation-raced",
        ),
    ] {
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn({
            let store = store.clone();
            move || {
                store
                    .database()
                    .read(|_| {
                        held_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok(())
                    })
                    .unwrap();
            }
        });
        held_rx.recv().unwrap();
        // The subscriber registers, then blocks reading its initial state;
        // the commit blocks on the same database.
        let live = tokio::spawn({
            let app = app.clone();
            async move { request(app, "GET", &uri, None).await }
        });
        let committer = std::thread::spawn({
            let (settings, project_path) = (settings.clone(), project_path.clone());
            let raced_id = raced_id.to_string();
            move || {
                std::thread::sleep(Duration::from_millis(100));
                if raced_id.starts_with("turn") {
                    append_turn(&settings, &project_path, "conversation-handoff", &raced_id);
                } else {
                    seed_conversation(&settings, &project_path, &raced_id);
                }
            }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        committer.join().unwrap();
        let mut stream = live.await.unwrap().into_body().into_data_stream();
        let mut envelopes = vec![next_envelope(&mut stream).await];
        // A marker commit after the race bounds what the stream may still send.
        let marker = append_turn(
            &settings,
            &project_path,
            "conversation-handoff",
            &format!("marker-{raced_id}"),
        );
        loop {
            let reached = envelopes.last().map(watermark).unwrap_or(0);
            if reached >= marker {
                break;
            }
            envelopes.push(next_envelope(&mut stream).await);
        }
        assert_eq!(
            rows_with_id(&envelopes, raced_id).len(),
            1,
            "{envelopes:#?}"
        );
    }
}

/// A client that was away catches up from its cursor with every row that
/// changed meanwhile, tombstones included, and a deleted conversation
/// answers an old cursor with its tombstone.
#[tokio::test]
async fn disconnected_clients_catch_up_with_changed_rows_and_tombstones() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-away");
    let app = crate::store_app_default(settings.clone());
    let (generation, away_at) = store_cursor(&settings);
    let window = {
        let live = request(
            app.clone(),
            "GET",
            &conversation_live_uri("conversation-away", &project_path, None),
            None,
        )
        .await;
        let mut stream = live.into_body().into_data_stream();
        next_envelope(&mut stream).await
    };
    assert_eq!(watermark(&window), away_at);

    // While away: a new turn, and the seeded segment removed.
    append_turn(
        &settings,
        &project_path,
        "conversation-away",
        "turn-while-away",
    );
    crate::conversation_seed::write_rows(
        &crate::shared_store(&settings.data_dir),
        |tx, revision| {
            tx.execute(
                "UPDATE segments SET deleted = 1, revision = ?1 \
                 WHERE conversation_id = 'conversation-away' AND segment_id = 'segment-live'",
                [revision],
            )?;
            tx.execute(
                "UPDATE conversations SET revision = ?1 WHERE id = 'conversation-away'",
                [revision],
            )
        },
    );
    let back = request(
        app.clone(),
        "GET",
        &conversation_live_uri(
            "conversation-away",
            &project_path,
            Some((&generation, away_at)),
        ),
        None,
    )
    .await;
    let mut stream = back.into_body().into_data_stream();
    let caught_up = next_envelope(&mut stream).await;
    assert_eq!(caught_up["type"], "conversation.changes");
    assert_eq!(caught_up["payload"]["from_revision"], away_at.to_string());
    let rows = caught_up["payload"]["rows"].as_array().unwrap();
    assert!(rows
        .iter()
        .any(|row| row["id"] == "turn-while-away" && row["deleted"] == false));
    let tombstone = rows
        .iter()
        .find(|row| row["id"] == "segment-live")
        .expect("tombstone");
    assert_eq!(tombstone["deleted"], true);
    assert_eq!(tombstone["payload"], Value::Null);
    assert_eq!(tombstone["turn_id"], "turn-live");
    // Unchanged history is not resent.
    assert!(rows.iter().all(|row| row["id"] != "turn-live"));
    let tombstone_revision: u64 = tombstone["revision"].as_str().unwrap().parse().unwrap();
    assert!(tombstone_revision > away_at && tombstone_revision <= watermark(&caught_up));

    // Deleted: an old cursor still gets the conversation's tombstone, in
    // both its own scope and the summary scope; a fresh open is not found.
    let deleted = request(
        app.clone(),
        "DELETE",
        &format!(
            "/workspace/api/conversations/conversation-away?project_path={}",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    for uri in [
        conversation_live_uri(
            "conversation-away",
            &project_path,
            Some((&generation, away_at)),
        ),
        summaries_live_uri(Some(&project_path), Some((&generation, away_at))),
    ] {
        let live = request(app.clone(), "GET", &uri, None).await;
        let mut stream = live.into_body().into_data_stream();
        let envelope = next_envelope(&mut stream).await;
        let conversation = rows_with_id(std::slice::from_ref(&envelope), "conversation-away");
        assert_eq!(conversation.len(), 1, "{envelope}");
        assert_eq!(conversation[0]["kind"], "conversation");
        assert_eq!(conversation[0]["deleted"], true);
    }
    let reopened = request(
        app,
        "GET",
        &format!(
            "/workspace/api/conversations/conversation-away?project_path={}",
            url_encode(&project_path.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(reopened.status(), StatusCode::NOT_FOUND);
}

/// Notifications are wakeups, not truth: a commit whose notice never
/// arrives still reaches the subscriber, at its next heartbeat.
#[tokio::test]
async fn a_commit_without_a_notice_arrives_at_the_next_heartbeat() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-quiet");
    let app = crate::store_app_default(settings.clone());
    let (generation, high) = store_cursor(&settings);
    let live = request(
        app,
        "GET",
        &conversation_live_uri(
            "conversation-quiet",
            &project_path,
            Some((&generation, high)),
        ),
        None,
    )
    .await;
    let mut stream = live.into_body().into_data_stream();
    assert_eq!(
        next_envelope(&mut stream).await["payload"]["rows"],
        json!([])
    );
    // Committed behind the store's back: no notice is published.
    let revision = crate::conversation_seed::write_rows(
        &crate::shared_store(&settings.data_dir),
        |tx, revision| {
            tx.execute(
                "UPDATE conversations SET revision = ?1 WHERE id = 'conversation-quiet'",
                [revision],
            )?;
            Ok(revision)
        },
    );
    let arrived = next_envelope(&mut stream).await;
    assert_eq!(arrived["type"], "conversation.changes");
    assert_eq!(watermark(&arrived), revision as u64);
    assert_eq!(
        rows_with_id(std::slice::from_ref(&arrived), "conversation-quiet")[0]["revision"],
        revision.to_string()
    );
}

/// A filtered stream advances over revisions of conversations it does not
/// follow: gaps are valid, and only its own rows arrive.
#[tokio::test]
async fn filtered_streams_advance_over_other_scopes_revisions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    fs::create_dir_all(&project_a).expect("project");
    fs::create_dir_all(&project_b).expect("project");
    seed_conversation(&settings, &project_a, "conversation-a");
    seed_conversation(&settings, &project_b, "conversation-b");
    let app = crate::store_app_default(settings.clone());
    let (generation, high) = store_cursor(&settings);
    let mut conversation_stream = request(
        app.clone(),
        "GET",
        &conversation_live_uri("conversation-a", &project_a, Some((&generation, high))),
        None,
    )
    .await
    .into_body()
    .into_data_stream();
    let mut summary_stream = request(
        app.clone(),
        "GET",
        &summaries_live_uri(Some(&project_a), Some((&generation, high))),
        None,
    )
    .await
    .into_body()
    .into_data_stream();
    assert_eq!(
        next_envelope(&mut conversation_stream).await["payload"]["rows"],
        json!([])
    );
    assert_eq!(
        next_envelope(&mut summary_stream).await["payload"]["rows"],
        json!([])
    );

    let other = append_turn(&settings, &project_b, "conversation-b", "turn-b");
    let own = append_turn(&settings, &project_a, "conversation-a", "turn-a");
    assert!(own > other && other > high);
    for (stream, kinds) in [
        (&mut conversation_stream, vec!["conversation", "turn"]),
        (&mut summary_stream, vec!["conversation"]),
    ] {
        let batch = next_envelope(stream).await;
        assert_eq!(batch["payload"]["from_revision"], high.to_string());
        assert_eq!(watermark(&batch), own);
        let rows = batch["payload"]["rows"].as_array().unwrap();
        assert!(rows
            .iter()
            .all(|row| row["conversation_id"] == "conversation-a"
                && row["revision"] == own.to_string()));
        let mut seen: Vec<&str> = rows
            .iter()
            .map(|row| row["kind"].as_str().unwrap())
            .collect();
        seen.sort();
        assert_eq!(seen, kinds);
    }
}

/// Summary snapshots: complete membership of one project or all, empty
/// lists included, at one watermark.
#[tokio::test]
async fn summary_snapshots_list_empty_one_and_every_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    let project_empty = temp.path().join("project-empty");
    for project in [&project_a, &project_b, &project_empty] {
        fs::create_dir_all(project).expect("project");
    }
    seed_conversation(&settings, &project_a, "conversation-a");
    seed_conversation(&settings, &project_b, "conversation-b");
    let app = crate::store_app_default(settings.clone());
    let (_, high) = store_cursor(&settings);
    let listed = |envelope: &Value| {
        let mut ids: Vec<String> = envelope["payload"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["payload"]["standing_role"].is_null())
            .map(|row| row["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };
    for (filter, expected) in [
        (Some(&project_empty), vec![]),
        (Some(&project_a), vec!["conversation-a".to_string()]),
        (
            None,
            vec!["conversation-a".to_string(), "conversation-b".to_string()],
        ),
    ] {
        let mut stream = request(
            app.clone(),
            "GET",
            &summaries_live_uri(filter.map(|path| path.as_path()), None),
            None,
        )
        .await
        .into_body()
        .into_data_stream();
        let snapshot = next_envelope(&mut stream).await;
        assert_eq!(snapshot["type"], "conversations.snapshot");
        assert_eq!(watermark(&snapshot), high);
        assert_eq!(listed(&snapshot), expected);
        // The HTTP list is the same SummarySnapshot.
        let uri = match filter {
            Some(path) => format!(
                "/workspace/api/projects/conversations?project_path={}",
                url_encode(&path.to_string_lossy())
            ),
            None => "/workspace/api/projects/conversations".to_string(),
        };
        let http = json_body(request(app.clone(), "GET", &uri, None).await).await;
        assert_eq!(http["watermark"], snapshot["payload"]["watermark"]);
        assert_eq!(listed(&json!({"payload": http})), expected);
    }
}

/// A summary catch-up over the limits resets, and the cursorless
/// reconnect gets a complete snapshot, which no limit caps.
#[tokio::test]
async fn summary_catch_up_over_the_limit_resets_to_a_cursorless_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-first");
    let app = crate::store_app_default(settings.clone());
    let (generation, high) = store_cursor(&settings);
    let project = registered_path(&settings, &project_path);
    let count = spark_storage::conversation::CATCH_UP_MAX_ROWS + 1;
    crate::conversation_seed::write_rows(
        &crate::shared_store(&settings.data_dir),
        |tx, revision| {
            for index in 0..count {
                let id = format!("bulk-{index:04}");
                tx.execute(
                    "INSERT INTO conversations (id, project_path, handle, created_at, \
                     updated_at, revision, title, preview, running, needs_attention, \
                     launched_run_ids, metadata) VALUES (?1, ?2, ?1, '2026-01-01T00:00:00Z', \
                     '2026-01-01T00:00:00Z', ?3, 'Bulk', '', 0, 0, '[]', ?4)",
                    rusqlite::params![
                        id,
                        project,
                        revision,
                        json!({"revision": 1, "conversation_id": id}).to_string(),
                    ],
                )?;
            }
            Ok(())
        },
    );
    let mut stream = request(
        app.clone(),
        "GET",
        &summaries_live_uri(Some(&project_path), Some((&generation, high))),
        None,
    )
    .await
    .into_body()
    .into_data_stream();
    let reset = next_envelope(&mut stream).await;
    assert_eq!(reset["type"], "conversations.reset");
    assert_eq!(reset["payload"]["code"], "resync_required");
    assert_eq!(reset["payload"]["reason"], "catchup_limit");
    let mut stream = request(
        app,
        "GET",
        &summaries_live_uri(Some(&project_path), None),
        None,
    )
    .await
    .into_body()
    .into_data_stream();
    let snapshot = next_envelope(&mut stream).await;
    assert_eq!(snapshot["type"], "conversations.snapshot");
    let rows = snapshot["payload"]["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .filter(|row| row["id"].as_str().unwrap().starts_with("bulk-"))
            .count()
            == count
    );
    assert!(rows.iter().any(|row| row["id"] == "conversation-first"));
}

/// Deleting a conversation removes it from its own project's summary scope
/// only; another project's filtered scope never hears of it.
#[tokio::test]
async fn deletions_reach_only_the_affected_summary_membership() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    fs::create_dir_all(&project_a).expect("project");
    fs::create_dir_all(&project_b).expect("project");
    seed_conversation(&settings, &project_a, "conversation-a");
    seed_conversation(&settings, &project_b, "conversation-b");
    let app = crate::store_app_default(settings.clone());
    let mut stream_a = request(
        app.clone(),
        "GET",
        &summaries_live_uri(Some(&project_a), None),
        None,
    )
    .await
    .into_body()
    .into_data_stream();
    let mut stream_b = request(
        app.clone(),
        "GET",
        &summaries_live_uri(Some(&project_b), None),
        None,
    )
    .await
    .into_body()
    .into_data_stream();
    assert_eq!(
        next_envelope(&mut stream_a).await["type"],
        "conversations.snapshot"
    );
    assert_eq!(
        next_envelope(&mut stream_b).await["type"],
        "conversations.snapshot"
    );
    let deleted = request(
        app.clone(),
        "DELETE",
        &format!(
            "/workspace/api/conversations/conversation-a?project_path={}",
            url_encode(&project_a.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let removal = next_envelope(&mut stream_a).await;
    let rows = removal["payload"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0]["id"].as_str(), rows[0]["deleted"].as_bool()),
        (Some("conversation-a"), Some(true))
    );
    // Project B's next batch is its own change, without A's tombstone.
    let marker = append_turn(&settings, &project_b, "conversation-b", "turn-marker");
    let next = next_envelope(&mut stream_b).await;
    assert_eq!(watermark(&next), marker);
    assert!(rows_with_id(std::slice::from_ref(&next), "conversation-a").is_empty());
    assert_eq!(
        rows_with_id(std::slice::from_ref(&next), "conversation-b").len(),
        1
    );
}

/// The compat SSE fixtures document the conversation envelopes the server
/// sends: same types, resources and payload and row fields.
#[tokio::test]
async fn conversation_sse_fixtures_match_the_live_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    seed_conversation(&settings, &project_path, "conversation-compat");
    let app = crate::store_app_default(settings.clone());
    let (generation, _) = store_cursor(&settings);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-fixtures/compat/sse");
    let keys = |value: &Value| {
        let mut keys: Vec<String> = value
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        keys
    };
    for (name, uri) in [
        (
            "live-conversation-snapshot.json",
            conversation_live_uri("conversation-compat", &project_path, None),
        ),
        (
            "live-conversation-replay.json",
            conversation_live_uri("conversation-compat", &project_path, Some((&generation, 0))),
        ),
        (
            "live-conversation-resync-unavailable.json",
            conversation_live_uri(
                "missing-conversation",
                &project_path,
                Some(("previous-generation", 0)),
            ),
        ),
    ] {
        let fixture: Value =
            serde_json::from_str(&fs::read_to_string(fixtures.join(name)).unwrap()).unwrap();
        let expected = &fixture["frames"][0]["data"];
        assert_eq!(
            serde_json::from_str::<Value>(fixture["frames"][0]["data_text"].as_str().unwrap())
                .unwrap(),
            *expected,
            "{name}"
        );
        let mut stream = request(app.clone(), "GET", &uri, None)
            .await
            .into_body()
            .into_data_stream();
        let actual = next_envelope(&mut stream).await;
        assert_eq!(actual["type"], expected["type"], "{name}");
        assert_eq!(
            actual["resource"]["kind"], expected["resource"]["kind"],
            "{name}"
        );
        assert_eq!(actual["cursor"], expected["cursor"], "{name}");
        assert_eq!(
            keys(&actual["payload"]),
            keys(&expected["payload"]),
            "{name}"
        );
        for (actual_row, expected_row) in actual["payload"]["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(expected["payload"]["rows"].as_array().into_iter().flatten())
        {
            assert_eq!(keys(actual_row), keys(expected_row), "{name}");
            assert_eq!(
                keys(&actual_row["payload"]),
                keys(&expected_row["payload"]),
                "{name}"
            );
        }
    }
}

/// A project's registered path, as the store records it.
fn registered_path(settings: &SparkSettings, project_path: &Path) -> String {
    spark_storage::ProjectRegistry::new(&settings.data_dir)
        .project_paths(&project_path.to_string_lossy())
        .unwrap()
        .project_path
}
