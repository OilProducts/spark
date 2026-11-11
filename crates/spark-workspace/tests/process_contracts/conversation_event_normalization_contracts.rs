use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentError, AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnBackend, AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
};
use spark_common::debug::{
    CODEX_JSONRPC_TRACE_FILE_NAME, CODEX_JSONRPC_TRACE_PATH_METADATA_KEY,
    ENV_SPARK_DEBUG_CODEX_JSONRPC,
};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::SparkSettings;
use spark_storage::{ConversationRepository, ProjectRegistry, CONVERSATION_STATE_SCHEMA_VERSION};
use spark_workspace::{
    ConversationRequestUserInputAnswerRequest, ConversationSettingsUpdate, ConversationTurnRequest,
    WorkspaceConversationService, WorkspaceError,
};

use super::test_support::ENV_LOCK;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn start_turn_persists_one_user_one_assistant_turn_settings_and_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());

    let (prepared, acceptance) = service
        .start_turn(
            "conversation-turn",
            ConversationTurnRequest {
                project_path: "/projects/turns".to_string(),
                message: "  Build this feature  ".to_string(),
                provider: Some("openai".to_string()),
                model: Some("gpt-5".to_string()),
                llm_profile: None,
                reasoning_effort: Some("high".to_string()),
                chat_mode: Some("plan".to_string()),
                attachment_ids: Vec::new(),
                client_turn_id: None,
            },
        )
        .expect("start turn");

    assert_eq!(prepared.chat_mode, "plan");
    assert_eq!(prepared.provider, "openai");
    assert_eq!(prepared.model.as_deref(), Some("gpt-5"));
    let snapshot = service
        .get_snapshot("conversation-turn", Some("/projects/turns"))
        .expect("snapshot");
    assert_eq!(snapshot["chat_mode"], "plan");
    assert_eq!(snapshot["provider"], "openai");
    assert_eq!(snapshot["llm_profile"], Value::Null);
    assert_eq!(snapshot["reasoning_effort"], "high");
    assert_eq!(snapshot["title"], "Build this feature");
    let turns = snapshot["turns"].as_array().expect("turns");
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0]["kind"], "mode_change");
    assert_eq!(turns[1]["role"], "user");
    assert_eq!(turns[1]["content"], "Build this feature");
    assert_eq!(turns[2]["role"], "assistant");
    assert_eq!(turns[2]["status"], "pending");
    assert_eq!(turns[2]["parent_turn_id"], turns[1]["id"]);
    // Mode-change turn, user turn, assistant turn and the settings change
    // commit as one revision; the acceptance carries exactly those rows.
    assert_eq!(snapshot["revision"], acceptance.revision);
    assert_eq!(acceptance.status, "accepted");
    assert_eq!(acceptance.user_turn_id, turns[1]["id"].as_str().unwrap());
    assert_eq!(
        acceptance.assistant_turn_id.as_deref(),
        turns[2]["id"].as_str()
    );
    let mut changed: Vec<_> = acceptance
        .changes
        .rows
        .iter()
        .map(|row| (row.kind, row.id.clone()))
        .collect();
    changed.sort_by_key(|(_, id)| id.clone());
    let mut expected = vec![
        (
            spark_storage::conversation::EntityKind::Conversation,
            "conversation-turn".to_string(),
        ),
        (
            spark_storage::conversation::EntityKind::Turn,
            turns[0]["id"].as_str().unwrap().to_string(),
        ),
        (
            spark_storage::conversation::EntityKind::Turn,
            turns[1]["id"].as_str().unwrap().to_string(),
        ),
        (
            spark_storage::conversation::EntityKind::Turn,
            turns[2]["id"].as_str().unwrap().to_string(),
        ),
    ];
    expected.sort_by_key(|(_, id)| id.clone());
    assert_eq!(changed, expected);

    // One transaction stored the three turns and the settings metadata.
    let events = crate::stored_upserts(&settings, "conversation-turn");
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| event["type"] == "turn_upsert"));
    assert!(events
        .windows(2)
        .all(|pair| pair[0]["revision"] == pair[1]["revision"]));

    let conflict = service
        .start_turn(
            "conversation-turn",
            ConversationTurnRequest {
                project_path: "/projects/turns".to_string(),
                message: "again".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect_err("active assistant conflict");
    assert!(matches!(conflict, WorkspaceError::Conflict(_)));
}

#[test]
fn project_chat_writes_a_large_stream_once_at_its_segment_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());
    let (prepared, _) = service
        .start_turn(
            "conversation-large-provider-stream",
            ConversationTurnRequest {
                project_path: "/projects/large-provider-stream".to_string(),
                message: "Stream it".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let watermark = || {
        crate::shared_store(&settings.data_dir)
            .database()
            .read(|read| Ok(read.watermark()))
            .expect("watermark")
    };
    let before = watermark();
    let mut events = (0..50_000)
        .map(|_| content_delta("assistant", "x", "app-turn", "answer"))
        .collect::<Vec<_>>();
    events.push(content_completed(
        "assistant",
        "complete",
        "app-turn",
        "answer",
    ));

    service
        .ingest_agent_turn_output(
            "conversation-large-provider-stream",
            "/projects/large-provider-stream",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events,
                final_assistant_text: Some("complete".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest provider stream");

    // Deltas write nothing: the segment's opening, its end and the
    // settlement are the only commits, and no provider trace exists.
    assert!(
        watermark() - before <= 4,
        "{} commits",
        watermark() - before
    );
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths("/projects/large-provider-stream")
        .expect("project paths");
    let root = project
        .conversations_dir
        .join("conversation-large-provider-stream");
    assert!(!root.join("provider-trace.jsonl").exists());
    let stored = crate::shared_store(&settings.data_dir)
        .database()
        .read(|read| read.conversation("conversation-large-provider-stream"))
        .expect("rows")
        .expect("conversation");
    let logical_units = stored
        .segments
        .iter()
        .filter(|segment| segment["turn_id"] == prepared.assistant_turn_id)
        .collect::<Vec<_>>();
    assert_eq!(logical_units.len(), 1);
    assert_eq!(logical_units[0]["content"], "complete");
}

#[test]
fn project_chat_event_and_transcript_share_externalized_tool_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());
    let (prepared, _) = service
        .start_turn(
            "conversation-large-tool-output",
            ConversationTurnRequest {
                project_path: "/projects/large-tool-output".to_string(),
                message: "Run it".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let full_output = "large output".repeat(2_000);
    service
        .ingest_agent_turn_output(
            "conversation-large-tool-output",
            "/projects/large-tool-output",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![tool_event(
                    "tool_call_completed",
                    "tool-large",
                    "completed",
                    &full_output,
                )],
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest tool output");

    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths("/projects/large-tool-output")
        .expect("project paths");
    let root = project
        .conversations_dir
        .join("conversation-large-tool-output");
    // The stored segment references one immutable blob.
    let snapshot = service
        .get_snapshot(
            "conversation-large-tool-output",
            Some("/projects/large-tool-output"),
        )
        .expect("snapshot");
    let stored = crate::shared_store(&settings.data_dir)
        .database()
        .read(|read| read.conversation("conversation-large-tool-output"))
        .expect("rows")
        .expect("conversation");
    let segment = stored
        .segments
        .iter()
        .find(|segment| segment["kind"] == "tool_call")
        .expect("tool segment");
    let artifact = segment["tool_call"]["output_artifact"]
        .as_str()
        .expect("externalized");
    assert!(artifact.starts_with("tool-output/segment-tool-app-turn-tool-large."));
    assert_eq!(
        fs::read_to_string(root.join(artifact)).expect("artifact"),
        full_output
    );
    assert!(snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["tool_call"]["output_artifact"].as_str() == Some(artifact)));
}

#[test]
fn settling_an_orphaned_turn_keeps_its_committed_segments_across_reopens() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());
    let (prepared, _) = service
        .start_turn(
            "conversation-recovered-provider-unit",
            ConversationTurnRequest {
                project_path: "/projects/recovered-provider-unit".to_string(),
                message: "Recover it".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let repository = ConversationRepository::new(crate::shared_store(&settings.data_dir));
    let completed = content_completed("assistant", "recovered", "app-turn", "answer");
    repository
        .record_provider_event(
            "conversation-recovered-provider-unit",
            "/projects/recovered-provider-unit",
            &prepared.assistant_turn_id,
            &completed,
            &Default::default(),
        )
        .expect("record a completed segment");
    let read = || {
        ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .read_snapshot(
                "conversation-recovered-provider-unit",
                Some("/projects/recovered-provider-unit"),
            )
            .expect("reopen")
            .expect("snapshot")
    };
    let recovered_count = |snapshot: &Value| {
        snapshot["segments"]
            .as_array()
            .expect("segments")
            .iter()
            .filter(|segment| {
                segment["turn_id"] == prepared.assistant_turn_id
                    && segment["content"] == "recovered"
            })
            .count()
    };

    // The completed segment committed when it ended; reads change nothing.
    let before = read();
    assert_eq!(recovered_count(&before), 1);
    assert_eq!(read()["revision"], before["revision"]);

    service.settle_orphaned_turns().expect("settle");
    service.settle_orphaned_turns().expect("settle again");
    for _ in 0..2 {
        let snapshot = read();
        assert_eq!(recovered_count(&snapshot), 1);
        let turn = snapshot["turns"]
            .as_array()
            .expect("turns")
            .iter()
            .find(|turn| turn["id"] == prepared.assistant_turn_id)
            .expect("assistant turn");
        assert_eq!(turn["status"], "failed");
        assert_eq!(turn["error"], "Spark restarted before this turn finished.");
    }
}

#[test]
fn live_thinking_progress_reaches_clients_as_a_transient_delta_and_is_never_stored() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_stream_events(
        vec![AgentTurnOutput::default()],
        vec![vec![
            TurnStreamEvent::thinking_progress(true, Some(50)),
            TurnStreamEvent::thinking_progress(false, Some(186)),
        ]],
    );
    let service = crate::conversations_with_backend(settings(temp.path()), Arc::new(backend));
    let (prepared, _) = service
        .start_turn(
            "conversation-live-thinking",
            ConversationTurnRequest {
                project_path: "/projects/live-thinking".to_string(),
                message: "Think".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let assistant_turn_id = prepared.assistant_turn_id.clone();
    let deltas = Arc::new(Mutex::new(Vec::new()));
    let seen = deltas.clone();
    service
        .complete_started_turn_with_progress_payloads(prepared, move |payload| {
            if payload["delta_kind"] == "thinking" {
                seen.lock().unwrap().push(payload);
            }
        })
        .expect("turn completes");
    // Nothing stores it, and there is no provider trace to hold it.
    let snapshot = service
        .get_snapshot(
            "conversation-live-thinking",
            Some("/projects/live-thinking"),
        )
        .expect("snapshot");
    assert!(!snapshot.to_string().contains("thinking_progress"));
    assert!(!walkdir_files(temp.path())
        .iter()
        .any(|entry| entry.ends_with("provider-trace.jsonl")));

    let deltas = deltas.lock().unwrap();
    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0]["type"], "stream_delta");
    assert_eq!(deltas[0]["turn_id"], json!(assistant_turn_id));
    assert_eq!(deltas[0]["active"], true);
    assert_eq!(deltas[0]["estimated_tokens"], 50);
    assert_eq!(deltas[1]["active"], false);
    assert_eq!(deltas[1]["estimated_tokens"], 186);
}

/// A harness that stops mid-thought never reports the end, so a client told it
/// is thinking, or a later one told so on subscribing, would show it forever.
#[test]
fn a_turn_ending_mid_thought_still_reports_that_thinking_ended() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_stream_events(
        vec![AgentTurnOutput::default()],
        vec![vec![TurnStreamEvent::thinking_progress(true, Some(50))]],
    );
    let service = crate::conversations_with_backend(settings(temp.path()), Arc::new(backend));
    let (prepared, _) = service
        .start_turn(
            "conversation-open-thinking",
            ConversationTurnRequest {
                project_path: "/projects/open-thinking".to_string(),
                message: "Think".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let assistant_turn_id = prepared.assistant_turn_id.clone();
    let deltas = Arc::new(Mutex::new(Vec::new()));
    let seen = deltas.clone();
    service
        .complete_started_turn_with_progress_payloads(prepared, move |payload| {
            if payload["delta_kind"] == "thinking" {
                seen.lock().unwrap().push(payload);
            }
        })
        .expect("turn completes");

    let deltas = deltas.lock().unwrap();
    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0]["active"], true);
    assert_eq!(deltas[1]["active"], false);
    assert_eq!(deltas[1]["turn_id"], json!(assistant_turn_id));
}

fn walkdir_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn live_provider_event_write_failure_prevents_transcript_commit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let completion = tool_event(
        "tool_call_completed",
        "tool-unbacked",
        "completed",
        &"must not commit".repeat(2_000),
    );
    let backend = ScriptedAgentTurnBackend::with_stream_events(
        vec![AgentTurnOutput {
            events: vec![completion.clone()],
            final_assistant_text: Some("must not commit".to_string()),
            ..AgentTurnOutput::default()
        }],
        vec![vec![completion]],
    );
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let (prepared, _) = service
        .start_turn(
            "conversation-provider-write-failure",
            ConversationTurnRequest {
                project_path: "/projects/provider-write-failure".to_string(),
                message: "Do not publish an unbacked answer".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths("/projects/provider-write-failure")
        .expect("project paths");
    let root = project
        .conversations_dir
        .join("conversation-provider-write-failure");
    let rows = || {
        crate::shared_store(&settings.data_dir)
            .database()
            .read(|read| read.conversation("conversation-provider-write-failure"))
            .expect("rows")
    };
    let before = rows();
    // A file where tool output belongs makes the tool segment's end write fail.
    fs::create_dir_all(&root).expect("conversation root");
    fs::write(root.join("tool-output"), "").expect("block tool output");

    let assistant_turn_id = prepared.assistant_turn_id.clone();
    let error = service
        .complete_started_turn_with_progress_payloads(prepared, |_| {})
        .expect_err("provider event write must fail the turn");
    assert!(matches!(error, WorkspaceError::Internal(_)));

    // The turn settles as a visible failure; none of the unbacked provider
    // output is committed.
    let after = rows().expect("conversation");
    let (_, _, assistant) = after
        .turns
        .iter()
        .find(|(id, ..)| *id == assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant["status"], "failed", "{assistant}");
    assert!(assistant["error"]
        .as_str()
        .is_some_and(|error| error.contains("could not record")));
    assert!(!serde_json::to_string(&after.segments)
        .unwrap()
        .contains("must not commit"));
    assert_eq!(after.segments.len(), before.expect("before").segments.len());

    // The conversation is not left busy: the next message starts a turn.
    let (next, _) = service
        .send_turn(
            "conversation-provider-write-failure",
            ConversationTurnRequest {
                project_path: "/projects/provider-write-failure".to_string(),
                message: "Try again".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("next send");
    assert!(next.is_some(), "the next message started a turn");
}

/// A turn prepared before its project was deleted never reaches the
/// provider when it is resumed afterwards.
#[test]
fn a_prepared_turn_resumed_after_project_deletion_starts_no_provider_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![AgentTurnOutput {
        final_assistant_text: Some("must not run".to_string()),
        ..AgentTurnOutput::default()
    }]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let (prepared, _) = service
        .start_turn(
            "conversation-deleted-before-start",
            ConversationTurnRequest {
                project_path: "/projects/deleted-before-start".to_string(),
                message: "Run later".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    spark_workspace::WorkspaceProjectService::new(settings.clone())
        .delete_project(&service, "/projects/deleted-before-start")
        .expect("delete project");

    let error = service
        .complete_started_turn_with_progress_payloads(prepared, |_| {})
        .expect_err("a deleted conversation's turn cannot run");
    assert!(matches!(error, WorkspaceError::NotFound(_)), "{error}");
    assert!(requests.lock().unwrap().is_empty());
    assert!(!ProjectRegistry::new(&settings.data_dir)
        .project_paths("/projects/deleted-before-start")
        .unwrap()
        .root
        .exists());
}

#[test]
fn start_turn_passes_codex_jsonrpc_trace_path_only_in_debug_mode() {
    let _env_lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());

    let _debug_guard = EnvVarGuard::remove(ENV_SPARK_DEBUG_CODEX_JSONRPC);
    let (default_prepared, _) = service
        .start_turn(
            "conversation-trace-default",
            ConversationTurnRequest {
                project_path: "/projects/trace-default".to_string(),
                message: "Default trace".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start default turn");
    assert!(service
        .prepare_agent_turn_request(&default_prepared)
        .expect("default request")
        .metadata
        .get(CODEX_JSONRPC_TRACE_PATH_METADATA_KEY)
        .is_none());

    drop(_debug_guard);
    let _debug_guard = EnvVarGuard::set(ENV_SPARK_DEBUG_CODEX_JSONRPC, "1");
    let (debug_prepared, _) = service
        .start_turn(
            "conversation-trace-debug",
            ConversationTurnRequest {
                project_path: "/projects/trace-debug".to_string(),
                message: "Debug trace".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start debug turn");
    let debug_request = service
        .prepare_agent_turn_request(&debug_prepared)
        .expect("debug request");
    let trace_path = debug_request
        .metadata
        .get(CODEX_JSONRPC_TRACE_PATH_METADATA_KEY)
        .and_then(Value::as_str)
        .expect("trace path");
    assert!(trace_path.ends_with(&format!(
        "/conversation-trace-debug/{CODEX_JSONRPC_TRACE_FILE_NAME}"
    )));
    assert!(!Path::new(trace_path).exists());
}

#[test]
fn start_turn_prepares_agent_request_with_persisted_history_and_selectors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = crate::conversations(settings(temp.path()));

    let (first_prepared, _snapshot) = service
        .start_turn(
            "conversation-agent-request",
            ConversationTurnRequest {
                project_path: "/projects/agent-request".to_string(),
                message: "First question".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("first turn");
    service
        .ingest_agent_turn_output(
            "conversation-agent-request",
            "/projects/agent-request",
            &first_prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![content_completed(
                    "assistant",
                    "First answer",
                    "app-turn-first",
                    "final",
                )],
                final_assistant_text: Some("First answer".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("first output");

    let (prepared, _) = service
        .start_turn(
            "conversation-agent-request",
            ConversationTurnRequest {
                project_path: "/projects/agent-request".to_string(),
                message: "Second question".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("openrouter/model".to_string()),
                llm_profile: None,
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("plan".to_string()),
                attachment_ids: Vec::new(),
                client_turn_id: None,
            },
        )
        .expect("second turn");

    let request = &service
        .prepare_agent_turn_request(&prepared)
        .expect("agent turn request");
    assert_eq!(request.conversation_id, prepared.conversation_id);
    assert_eq!(request.project_path, prepared.project_path);
    assert_eq!(request.prompt, "Second question");
    assert_eq!(request.provider.as_deref(), Some("openrouter"));
    assert_eq!(request.model.as_deref(), Some("openrouter/model"));
    assert_eq!(request.llm_profile.as_deref(), None);
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.chat_mode.as_deref(), Some("plan"));
    assert_eq!(
        request.metadata["spark.workspace.user_turn_id"],
        json!(prepared.user_turn_id.clone())
    );
    assert_eq!(
        request.metadata["spark.workspace.assistant_turn_id"],
        json!(prepared.assistant_turn_id.clone())
    );

    let history = serde_json::to_value(&request.history).expect("history json");
    assert_eq!(history.as_array().expect("history").len(), 2);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[0]["content"], "First question");
    assert_eq!(history[1]["role"], "assistant");
    assert_eq!(history[1]["content"], "First answer");
    assert!(history[0].get("id").is_none());
    assert!(history[0].get("status").is_none());
    assert!(history[0].get("kind").is_none());
    assert!(history[1].get("segments").is_none());

    let snapshot = service
        .get_snapshot(
            "conversation-agent-request",
            Some("/projects/agent-request"),
        )
        .expect("snapshot");
    let turns = snapshot["turns"].as_array().expect("turns");
    assert!(turns
        .iter()
        .any(|turn| turn["id"] == prepared.user_turn_id && turn["content"] == "Second question"));
    assert!(request
        .history
        .iter()
        .all(
            |turn| serde_json::to_value(turn).expect("turn json")["content"] != "Second question"
        ));

    let decoded: AgentTurnRequest =
        serde_json::from_value(serde_json::to_value(request).expect("request json"))
            .expect("deserialize request");
    assert_eq!(&decoded, request);
}

#[test]
fn start_turn_agent_history_does_not_hydrate_removed_legacy_state_turns() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/legacy-agent-request";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    write_state(
        &project.conversations_dir,
        "conversation-legacy-agent-request",
        json!({
            "schema_version": CONVERSATION_STATE_SCHEMA_VERSION,
            "revision": 2,
            "conversation_id": "conversation-legacy-agent-request",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "Legacy conversation",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:02Z",
            "turns": [
                {
                    "id": "turn-legacy-user",
                    "role": "user",
                    "content": "Legacy question",
                    "timestamp": "2026-01-01T00:00:00Z"
                },
                {
                    "id": "turn-legacy-assistant",
                    "role": "assistant",
                    "content": "Legacy answer",
                    "timestamp": "2026-01-01T00:00:01Z"
                }
            ],
            "segments": []
        }),
    );

    let service = crate::conversations(settings);
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-legacy-agent-request",
            ConversationTurnRequest {
                project_path: project_path.to_string(),
                message: "Current question".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let request = service
        .prepare_agent_turn_request(&prepared)
        .expect("agent turn request");
    let history = serde_json::to_value(&request.history).expect("history");
    assert!(history.as_array().expect("history").is_empty());
}

#[test]
fn execute_turn_runs_injected_backend_and_returns_ingested_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            final_assistant_text: Some("First answer".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            raw_log_lines: vec![AgentRawLogLine {
                direction: "outgoing".to_string(),
                line: "{\"event\":\"backend-second\"}".to_string(),
            }],
            final_assistant_text: Some("Second answer".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 12, "outputTokens": 5}})),
            token_usage_breakdown: Some(json!({
                "total": {"inputTokens": 12, "outputTokens": 5},
                "last": {"inputTokens": 4, "outputTokens": 2}
            })),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));

    service
        .execute_turn(
            "conversation-execute",
            ConversationTurnRequest {
                project_path: "/projects/execute".to_string(),
                message: "First question".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("first execution");
    let snapshot = service
        .execute_turn(
            "conversation-execute",
            ConversationTurnRequest {
                project_path: "/projects/execute".to_string(),
                message: "Second question".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("openrouter/model".to_string()),
                llm_profile: None,
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("plan".to_string()),
                attachment_ids: Vec::new(),
                client_turn_id: None,
            },
        )
        .expect("second execution");

    let requests = requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    let request = &requests[1];
    assert_eq!(request.conversation_id, "conversation-execute");
    assert_eq!(request.project_path, "/projects/execute");
    assert_eq!(request.prompt, "Second question");
    assert_eq!(request.provider.as_deref(), Some("openrouter"));
    assert_eq!(request.model.as_deref(), Some("openrouter/model"));
    assert_eq!(request.llm_profile.as_deref(), None);
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.chat_mode.as_deref(), Some("plan"));
    let history = serde_json::to_value(&request.history).expect("history");
    assert_eq!(history.as_array().expect("history").len(), 2);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[0]["content"], "First question");
    assert_eq!(history[1]["role"], "assistant");
    assert_eq!(history[1]["content"], "First answer");
    assert!(history[0].get("id").is_none());
    assert!(history[1].get("status").is_none());
    let assistant_turn_id = request.metadata["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();
    let user_turn_id = request.metadata["spark.workspace.user_turn_id"]
        .as_str()
        .expect("user turn id")
        .to_string();

    let turns = snapshot["turns"].as_array().expect("turns");
    assert!(turns
        .iter()
        .any(|turn| turn["id"] == user_turn_id && turn["content"] == "Second question"));
    let assistant_turn = turns
        .iter()
        .find(|turn| turn["id"] == assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Second answer");
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 12, "outputTokens": 5}})
    );
    assert_eq!(
        assistant_turn["token_usage_breakdown"]["last"]["inputTokens"],
        json!(4)
    );

    let persisted = service
        .get_snapshot("conversation-execute", Some("/projects/execute"))
        .expect("persisted snapshot");
    assert_eq!(snapshot["revision"], persisted["revision"]);
    assert_eq!(snapshot["turns"], persisted["turns"]);

    let raw_log =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .read_codex_jsonrpc_trace("conversation-execute", "/projects/execute")
            .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    let events = crate::stored_upserts(&settings, "conversation-execute");
    assert!(events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == assistant_turn_id
            && event["turn"]["status"] == "complete"
    }));

    // A client behind the current revision gets one fresh snapshot carrying
    // the committed completion.
    let state = crate::live_rows_state(&settings, "conversation-execute", "/projects/execute", 0);
    assert!(state["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| {
            turn["id"] == assistant_turn_id
                && turn["status"] == "complete"
                && turn["content"] == "Second answer"
                && turn["token_usage"]["total"]["inputTokens"] == json!(12)
                && turn["token_usage"]["total"]["outputTokens"] == json!(5)
                && turn["token_usage_breakdown"]["last"]["outputTokens"] == json!(2)
        }));
}

#[test]
fn execute_turn_persists_backend_error_outputs_with_failed_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            events: vec![structured_stream_error_event(
                "backend stream failed",
                "rate_limit_exceeded",
                json!({
                    "provider": "openai",
                    "status_code": 429,
                    "raw": {"error": {"code": "rate_limit_exceeded"}}
                }),
            )],
            final_assistant_text: Some("Do not persist this final text".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 7, "outputTokens": 1}})),
            token_usage_breakdown: Some(json!({
                "total": {"inputTokens": 7, "outputTokens": 1},
                "last": {"inputTokens": 7, "outputTokens": 1}
            })),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            raw_log_lines: vec![AgentRawLogLine {
                direction: "incoming".to_string(),
                line: "{\"event\":\"thread-resume-failed\"}".to_string(),
            }],
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "thread resume failed".to_string(),
                error_code: Some("thread_resume_failed".to_string()),
                details: Some(json!({"thread_id": "thread-public-execute"})),
            }),
            token_usage: Some(json!({"total": {"inputTokens": 2, "outputTokens": 0}})),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));

    let stream_failed = service
        .execute_turn(
            "conversation-execute-stream-failure",
            ConversationTurnRequest {
                project_path: "/projects/execute-errors".to_string(),
                message: "Run the failing stream".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("stream failure output");

    let stream_request = requests.lock().expect("requests")[0].clone();
    assert_eq!(
        stream_request.conversation_id,
        "conversation-execute-stream-failure"
    );
    assert_eq!(stream_request.project_path, "/projects/execute-errors");
    assert_eq!(stream_request.prompt, "Run the failing stream");
    let stream_assistant_turn_id = stream_request.metadata["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();

    let stream_assistant_turn = stream_failed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == stream_assistant_turn_id)
        .expect("stream failed assistant turn");
    assert_eq!(stream_assistant_turn["status"], "failed");
    assert_eq!(stream_assistant_turn["error"], "backend stream failed");
    assert_eq!(stream_assistant_turn["error_code"], "rate_limit_exceeded");
    assert_eq!(stream_assistant_turn["details"]["provider"], "openai");
    assert_ne!(
        stream_assistant_turn["content"],
        "Do not persist this final text"
    );
    assert_eq!(
        stream_assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 7, "outputTokens": 1}})
    );
    assert_eq!(
        stream_assistant_turn["token_usage_breakdown"]["last"]["outputTokens"],
        json!(1)
    );
    let stream_segment = segment_by_kind(
        stream_failed["segments"].as_array().expect("segments"),
        "assistant_message",
    );
    assert_eq!(stream_segment["status"], "failed");
    assert_eq!(stream_segment["error"], "backend stream failed");
    assert_eq!(stream_segment["error_code"], "rate_limit_exceeded");
    assert_eq!(stream_segment["details"]["status_code"], json!(429));

    let stream_events = crate::stored_upserts(&settings, "conversation-execute-stream-failure");
    assert!(stream_events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == stream_assistant_turn_id
            && event["turn"]["status"] == "failed"
            && event["turn"]["token_usage"]["total"]["inputTokens"] == json!(7)
            && event["turn"]["error_code"] == "rate_limit_exceeded"
    }));
    assert!(stream_events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["status"] == "failed"
            && event["segment"]["error"] == "backend stream failed"
            && event["segment"]["details"]["raw"]["error"]["code"] == "rate_limit_exceeded"
    }));

    let stream_live = crate::live_rows_state(
        &settings,
        "conversation-execute-stream-failure",
        "/projects/execute-errors",
        0,
    );
    assert!(stream_live["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| {
            turn["id"] == stream_assistant_turn_id
                && turn["status"] == "failed"
                && turn["error"] == "backend stream failed"
                && turn["error_code"] == "rate_limit_exceeded"
                && turn["details"]["provider"] == "openai"
                && turn["token_usage_breakdown"]["last"]["outputTokens"] == json!(1)
        }));

    let resume_failed = service
        .execute_turn(
            "conversation-execute-resume-failure",
            ConversationTurnRequest {
                project_path: "/projects/execute-errors".to_string(),
                message: "Resume the previous thread".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("thread resume failure output");

    let resume_request = requests.lock().expect("requests")[1].clone();
    assert_eq!(
        resume_request.conversation_id,
        "conversation-execute-resume-failure"
    );
    assert_eq!(resume_request.prompt, "Resume the previous thread");
    let resume_assistant_turn_id = resume_request.metadata["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();

    let resume_assistant_turn = resume_failed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == resume_assistant_turn_id)
        .expect("resume failed assistant turn");
    assert_eq!(resume_assistant_turn["status"], "failed");
    assert_eq!(resume_assistant_turn["error"], "thread resume failed");
    assert_eq!(resume_assistant_turn["error_code"], "thread_resume_failed");
    assert_eq!(
        resume_assistant_turn["details"]["thread_id"],
        "thread-public-execute"
    );
    assert_eq!(
        resume_assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 2, "outputTokens": 0}})
    );
    assert_eq!(resume_failed["event_log"][0]["kind"], "continuity_reset");
    assert_eq!(
        resume_failed["event_log"][0]["details"]["thread_id"],
        "thread-public-execute"
    );
    let resume_raw_log =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .read_codex_jsonrpc_trace(
                "conversation-execute-resume-failure",
                "/projects/execute-errors",
            )
            .expect("resume raw log");
    assert!(
        resume_raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    let resume_events = crate::stored_upserts(&settings, "conversation-execute-resume-failure");
    assert!(resume_events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == resume_assistant_turn_id
            && event["turn"]["status"] == "failed"
            && event["turn"]["error_code"] == "thread_resume_failed"
            && event["turn"]["details"]["thread_id"] == "thread-public-execute"
    }));
    let resume_live = crate::live_rows_state(
        &settings,
        "conversation-execute-resume-failure",
        "/projects/execute-errors",
        0,
    );
    assert!(resume_live["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| {
            turn["id"] == resume_assistant_turn_id
                && turn["status"] == "failed"
                && turn["error_code"] == "thread_resume_failed"
                && turn["details"]["thread_id"] == "thread-public-execute"
        }));
    assert_eq!(
        resume_live["event_log"][0]["details"]["thread_id"],
        "thread-public-execute"
    );
}

#[test]
fn execute_turn_starts_fresh_codex_thread_after_resume_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            final_assistant_text: Some("Started".to_string()),
            app_thread_id: Some("thread-stale".to_string()),
            app_turn_id: Some("turn-started".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "thread resume failed".to_string(),
                error_code: Some("codex_app_server_resume_failed".to_string()),
                details: Some(json!({"thread_id": "thread-stale"})),
            }),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Recovered".to_string()),
            app_thread_id: Some("thread-fresh".to_string()),
            app_turn_id: Some("turn-recovered".to_string()),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings, Arc::new(backend));

    service
        .execute_turn(
            "conversation-resume-reset",
            ConversationTurnRequest {
                project_path: "/projects/resume-reset".to_string(),
                message: "Start".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start output");
    service
        .execute_turn(
            "conversation-resume-reset",
            ConversationTurnRequest {
                project_path: "/projects/resume-reset".to_string(),
                message: "Resume stale".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("resume failure output");
    let recovered = service
        .execute_turn(
            "conversation-resume-reset",
            ConversationTurnRequest {
                project_path: "/projects/resume-reset".to_string(),
                message: "Start fresh".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("fresh output");

    let requests = requests.lock().expect("requests");
    assert_eq!(
        requests[1].metadata["spark.runtime.codex_app_server.thread_id"],
        json!("thread-stale")
    );
    assert!(
        !requests[2]
            .metadata
            .contains_key("spark.runtime.codex_app_server.thread_id"),
        "next explicit turn should not retry the stale app-server thread id"
    );
    assert_eq!(
        recovered["turns"].as_array().unwrap().last().unwrap()["content"],
        "Recovered"
    );
}

#[test]
fn normalized_agent_events_update_segments_raw_logs_usage_and_resume_failures() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-events",
            ConversationTurnRequest {
                project_path: "/projects/events".to_string(),
                message: "Explain the change".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let output = AgentTurnOutput {
        raw_log_lines: vec![AgentRawLogLine {
            direction: "incoming".to_string(),
            line: "{\"event\":\"delta\"}".to_string(),
        }],
        events: vec![
            agent_event(
                "session_start",
                "processing",
                json!({"state": "processing"}),
            ),
            content_delta("assistant", "Hel", "app-turn", "final"),
            content_completed("assistant", "Hello", "app-turn", "final"),
            content_delta("reasoning", "Because", "app-turn", "reasoning"),
            content_completed("reasoning", "Because.", "app-turn", "reasoning"),
            content_delta("plan", "1. Do it", "app-turn", "plan"),
            content_completed("plan", "1. Do it", "app-turn", "plan"),
            model_tool_event("model_tool_call_start", "tool-1", "proposed", None),
            model_tool_event(
                "model_tool_call_delta",
                "tool-1",
                "streaming",
                Some("{\"query\":\"rust\"}"),
            ),
            model_tool_event("model_tool_call_end", "tool-1", "completed", None),
            tool_event("tool_call_started", "tool-1", "running", "partial"),
            tool_event("tool_call_completed", "tool-1", "completed", "full output"),
            token_usage(json!({"total": {"inputTokens": 10, "outputTokens": 4}})),
            agent_warning_event("Context usage at 95%."),
            processing_completed_event(),
            agent_event("session_end", "closed", json!({"state": "closed"})),
        ],
        app_thread_id: None,
        app_turn_id: None,
        final_assistant_text: Some("Hello".to_string()),
        token_usage: Some(json!({"total": {"inputTokens": 10, "outputTokens": 4}})),
        token_usage_breakdown: Some(json!({
            "total": {"inputTokens": 10, "outputTokens": 4},
            "last": {"inputTokens": 2, "outputTokens": 1}
        })),
        thread_resume_failure: None,
    };
    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-events",
            "/projects/events",
            &prepared.assistant_turn_id,
            "chat",
            output,
        )
        .expect("ingest output");

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Hello");
    assert_eq!(
        assistant_turn["token_usage"]["total"]["inputTokens"],
        json!(10)
    );
    assert_eq!(
        assistant_turn["token_usage_breakdown"]["last"]["outputTokens"],
        json!(1)
    );
    let segments = snapshot["segments"].as_array().expect("segments");
    assert_eq!(
        segments
            .iter()
            .filter(|segment| segment["kind"] == "assistant_message")
            .count(),
        1
    );
    let reasoning = segment_by_kind(segments, "reasoning");
    assert_eq!(reasoning["content"], "Because.");
    assert_eq!(reasoning["source"]["app_turn_id"], "app-turn");
    assert_eq!(reasoning["source"]["item_id"], "reasoning");
    let plan = segment_by_kind(segments, "plan");
    assert_eq!(plan["content"], "1. Do it");
    let model_tool = segment_by_kind(segments, "model_tool_call");
    assert_eq!(model_tool["status"], "complete");
    assert_eq!(model_tool["tool_call"]["id"], "tool-1");
    assert_eq!(model_tool["tool_call"]["kind"], "model_tool_call");
    assert_eq!(model_tool["tool_call"]["name"], "lookup");
    assert_eq!(
        model_tool["tool_call"]["arguments"],
        json!({"query": "rust"})
    );
    assert_eq!(model_tool["source"]["call_id"], "tool-1");
    assert_eq!(model_tool["source"]["raw_kind"], "model_tool_call_end");
    assert_eq!(model_tool["source"]["response_id"], "resp-1");
    let tool = segment_by_kind(segments, "tool_call");
    assert_eq!(tool["status"], "complete");
    assert_eq!(tool["tool_call"]["kind"], "command_execution");
    assert_eq!(tool["tool_call"]["title"], "Run command");
    assert_eq!(tool["tool_call"]["command"], "cargo test");
    assert_eq!(tool["tool_call"]["output"], "full output");
    assert_eq!(tool["source"]["call_id"], "tool-1");
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "lifecycle"
            && segment["event_kind"] == "session_start"
            && segment["event_status"] == "processing"
            && segment["details"]["state"] == "processing"
    }));
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "warning"
            && segment["event_kind"] == "warning"
            && segment["message"] == "Context usage at 95%."
            && segment["details"]["message"] == "Context usage at 95%."
    }));
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "processing"
            && segment["event_kind"] == "processing_end"
            && segment["event_status"] == "idle"
            && segment["details"]["state"] == "idle"
    }));
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "lifecycle"
            && segment["event_kind"] == "session_end"
            && segment["event_status"] == "closed"
            && segment["details"]["state"] == "closed"
    }));

    let raw_log =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .read_codex_jsonrpc_trace("conversation-events", "/projects/events")
            .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    // A live client catching up receives the committed rows.
    let live_rows =
        crate::live_change_rows(&settings, "conversation-events", "/projects/events", 0);
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "turn"
            && row["payload"]["id"] == prepared.assistant_turn_id
            && row["payload"]["token_usage_breakdown"]["last"]["outputTokens"] == json!(1)
    }));
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "assistant_message"
            && row["payload"]["content"] == "Hello"
    }));
    assert!(!live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "assistant_message"
            && row["payload"]["content"] == "Hel"
    }));
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "reasoning"
            && row["payload"]["content"] == "Because."
            && row["payload"]["source"]["item_id"] == "reasoning"
    }));
    assert!(!live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "reasoning"
            && row["payload"]["content"] == "Because"
    }));
    assert!(!live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "model_tool_call"
            && row["payload"]["source"]["raw_kind"] == "model_tool_call_delta"
    }));
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "model_tool_call"
            && row["payload"]["status"] == "complete"
            && row["payload"]["tool_call"]["id"] == "tool-1"
            && row["payload"]["source"]["raw_kind"] == "model_tool_call_end"
            && row["payload"]["source"]["response_id"] == "resp-1"
    }));
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "tool_call"
            && row["payload"]["status"] == "complete"
            && row["payload"]["tool_call"]["output"] == "full output"
            && row["payload"]["source"]["call_id"] == "tool-1"
    }));
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "agent_event"
            && row["payload"]["event_kind"] == "warning"
            && row["payload"]["message"] == "Context usage at 95%."
    }));
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "agent_event"
            && row["payload"]["event_kind"] == "processing_end"
            && row["payload"]["event_status"] == "idle"
    }));
    assert!(live_rows.iter().any(|row| {
        row["kind"] == "segment"
            && row["payload"]["kind"] == "agent_event"
            && row["payload"]["event_kind"] == "session_end"
            && row["payload"]["event_status"] == "closed"
    }));

    let (failure_prepared, _snapshot) = service
        .start_turn(
            "conversation-failure",
            ConversationTurnRequest {
                project_path: "/projects/events".to_string(),
                message: "Resume".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start failure turn");
    let failed = service
        .ingest_agent_turn_output(
            "conversation-failure",
            "/projects/events",
            &failure_prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                token_usage: Some(json!({"total": {"inputTokens": 3, "outputTokens": 0}})),
                thread_resume_failure: Some(AgentThreadResumeFailure {
                    message: "thread resume failed".to_string(),
                    error_code: Some("thread_resume_failed".to_string()),
                    details: Some(json!({"thread_id": "thread-1"})),
                }),
                ..AgentTurnOutput::default()
            },
        )
        .expect("failure output");
    let assistant_turn = failed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == failure_prepared.assistant_turn_id)
        .expect("failed assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(assistant_turn["error_code"], "thread_resume_failed");
    assert_eq!(assistant_turn["details"]["thread_id"], "thread-1");
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 3, "outputTokens": 0}})
    );
    assert_eq!(failed["event_log"][0]["kind"], "continuity_reset");
}

#[test]
fn missing_final_answer_event_fails_even_when_final_assistant_text_is_present() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = crate::conversations(settings(temp.path()));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-fallback-final-text",
            ConversationTurnRequest {
                project_path: "/projects/fallback".to_string(),
                message: "Explain the result".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-fallback-final-text",
            "/projects/fallback",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![content_delta("assistant", "Partial", "app-turn", "final")],
                final_assistant_text: Some("Complete answer".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest fallback text");

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");

    let assistant_segments = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .filter(|segment| segment["kind"] == "assistant_message")
        .collect::<Vec<_>>();
    assert_eq!(assistant_segments.len(), 1);
    let segment = assistant_segments[0];
    assert_eq!(segment["status"], "failed");
    assert_eq!(segment["content"], "Partial");
    assert_eq!(segment["source"]["app_turn_id"], "app-turn");
    assert_eq!(segment["source"]["item_id"], "final");
}

#[test]
fn claude_commentary_materializes_before_tools_and_final_answer_in_chat_and_plan() {
    for chat_mode in ["chat", "plan"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let service = crate::conversations(settings(temp.path()));
        let conversation_id = format!("conversation-claude-order-{chat_mode}");
        let (prepared, _) = service
            .start_turn(
                &conversation_id,
                ConversationTurnRequest {
                    project_path: "/projects/claude-order".to_string(),
                    message: "Do the work".to_string(),
                    chat_mode: Some(chat_mode.to_string()),
                    ..ConversationTurnRequest::default()
                },
            )
            .expect("start turn");
        let mut narration = content_completed("assistant", "Inspecting.", "app-turn", "block-1");
        narration.phase = Some("commentary".to_string());
        let mut final_block = content_completed("assistant", "Draft.", "app-turn", "block-2");
        final_block.phase = Some("commentary".to_string());
        let final_answer = content_completed("assistant", "Done.", "app-turn", "block-2");

        let snapshot = service
            .ingest_agent_turn_output(
                &conversation_id,
                "/projects/claude-order",
                &prepared.assistant_turn_id,
                chat_mode,
                AgentTurnOutput {
                    events: vec![
                        narration,
                        tool_event("tool_call_started", "tool-1", "running", ""),
                        tool_event("tool_call_completed", "tool-1", "completed", "ok"),
                        final_block,
                        final_answer,
                    ],
                    final_assistant_text: Some("Done.".to_string()),
                    ..AgentTurnOutput::default()
                },
            )
            .expect("ingest output");
        let segments = snapshot["segments"].as_array().expect("segments");
        assert_eq!(
            segments
                .iter()
                .map(|segment| (
                    segment["kind"].as_str().unwrap(),
                    segment["content"].as_str().unwrap_or("")
                ))
                .collect::<Vec<_>>(),
            vec![
                ("assistant_message", "Inspecting."),
                ("tool_call", ""),
                ("assistant_message", "Done."),
            ]
        );
        assert_eq!(segments[0]["phase"], "commentary");
        assert_eq!(segments[2]["phase"], "final_answer");
    }
}

#[test]
fn stream_error_without_final_answer_remains_failed_and_does_not_write_fallback_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = crate::conversations(settings(temp.path()));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-stream-error",
            ConversationTurnRequest {
                project_path: "/projects/stream-error".to_string(),
                message: "Run the agent".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-stream-error",
            "/projects/stream-error",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![stream_error_event("agent crashed")],
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest stream error");

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(assistant_turn["error"], "agent crashed");
    assert_ne!(assistant_turn["content"], "I reviewed that request.");

    let segment = segment_by_kind(
        snapshot["segments"].as_array().expect("segments"),
        "assistant_message",
    );
    assert_eq!(segment["status"], "failed");
    assert_eq!(segment["error"], "agent crashed");
    assert_eq!(segment["source"]["app_turn_id"], "app-turn");
    assert_eq!(segment["source"]["item_id"], "error-item");

    let events = crate::stored_upserts(&settings(temp.path()), "conversation-stream-error");
    assert!(events
        .iter()
        .any(|event| { event["type"] == "turn_upsert" && event["turn"]["status"] == "failed" }));
    assert!(events.iter().any(|event| {
        event["type"] == "segment_upsert" && event["segment"]["status"] == "failed"
    }));
}

#[test]
fn stream_error_with_final_text_and_usage_stays_failed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = crate::conversations(settings(temp.path()));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-stream-error-final-text",
            ConversationTurnRequest {
                project_path: "/projects/stream-error-final-text".to_string(),
                message: "Run the agent".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-stream-error-final-text",
            "/projects/stream-error-final-text",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![stream_error_event("backend failed")],
                final_assistant_text: Some("Do not persist as success".to_string()),
                token_usage: Some(json!({"total": {"inputTokens": 9, "outputTokens": 2}})),
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest stream error");

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(assistant_turn["error"], "backend failed");
    assert_ne!(assistant_turn["content"], "Do not persist as success");
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 9, "outputTokens": 2}})
    );

    let segment = segment_by_kind(
        snapshot["segments"].as_array().expect("segments"),
        "assistant_message",
    );
    assert_eq!(segment["status"], "failed");
    assert_eq!(segment["error"], "backend failed");
    assert_ne!(segment["content"], "Do not persist as success");

    let events = crate::stored_upserts(
        &settings(temp.path()),
        "conversation-stream-error-final-text",
    );
    assert!(events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == prepared.assistant_turn_id
            && event["turn"]["status"] == "failed"
            && event["turn"]["token_usage"]["total"]["inputTokens"] == json!(9)
    }));
}

#[test]
fn provider_output_after_settlement_changes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-late-segment",
            ConversationTurnRequest {
                project_path: "/projects/late-segment".to_string(),
                message: "Run a tool then explain".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let settled = service
        .ingest_agent_turn_output(
            "conversation-late-segment",
            "/projects/late-segment",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![
                    tool_event("tool_call_started", "tool-1", "running", "partial output"),
                    content_completed("assistant", "Tool is running.", "app-turn", "final"),
                ],
                final_assistant_text: Some("Tool is running.".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("initial ingest");

    let late = service
        .ingest_agent_turn_output(
            "conversation-late-segment",
            "/projects/late-segment",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![tool_event(
                    "tool_call_completed",
                    "tool-1",
                    "completed",
                    "full output",
                )],
                final_assistant_text: Some("Tool is running.".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("late tool completion");

    // Settlement is final: neither the late event nor a second return writes.
    assert_eq!(late, settled);
    let tool = segment_by_kind(late["segments"].as_array().unwrap(), "tool_call");
    assert_eq!(tool["tool_call"]["output"], "partial output");
}

#[test]
fn execute_turn_streams_transient_events_without_persisting_delta_payloads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::with_stream_events(
        vec![AgentTurnOutput {
            events: vec![content_completed("assistant", "Hello", "app-turn", "final")],
            app_thread_id: Some("thread-live".to_string()),
            app_turn_id: Some("app-turn".to_string()),
            final_assistant_text: Some("Hello".to_string()),
            ..AgentTurnOutput::default()
        }],
        vec![vec![
            content_delta("assistant", "Hel", "app-turn", "final"),
            content_delta("reasoning", "Thinking", "app-turn", "reasoning"),
        ]],
    );
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let progress_payloads = Arc::new(Mutex::new(Vec::new()));
    let progress_payloads_for_callback = Arc::clone(&progress_payloads);

    let snapshot = service
        .execute_turn_with_progress_payloads(
            "conversation-live-stream",
            ConversationTurnRequest {
                project_path: "/projects/live-stream".to_string(),
                message: "Stream this".to_string(),
                provider: Some("codex".to_string()),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
            move |payload| {
                progress_payloads_for_callback
                    .lock()
                    .expect("progress payloads")
                    .push(payload);
            },
        )
        .expect("execute turn");

    let live_payloads = progress_payloads.lock().expect("progress payloads");
    // The committed start is durable rows, not a progress payload: progress
    // receives only transient stream deltas.
    assert!(live_payloads
        .iter()
        .all(|payload| payload["type"] == "stream_delta"));
    assert!(crate::stored_upserts(&settings, "conversation-live-stream")
        .iter()
        .any(|event| event["turn"]["role"] == "user" && event["turn"]["content"] == "Stream this"));
    // Streamed text is transient: each open segment's whole text so far, a
    // per-execution stream sequence, the committed base revision, and no
    // durable revision.
    for (prefix, text) in [
        ("segment-assistant-", "Hel"),
        ("segment-reasoning-", "Thinking"),
    ] {
        assert!(
            live_payloads.iter().any(|payload| {
                payload["delta_kind"] == "segment_text"
                    && payload["segment_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with(prefix))
                    && payload["text"] == text
                    && payload["base_revision"].as_i64().is_some()
                    && payload.get("revision").is_none()
            }),
            "{live_payloads:#?}"
        );
    }
    let stream_sequences = live_payloads
        .iter()
        .filter(|payload| payload["type"] == "stream_delta")
        .map(|payload| payload["stream_sequence"].as_i64().expect("sequence"))
        .collect::<Vec<_>>();
    let mut sorted_sequences = stream_sequences.clone();
    sorted_sequences.sort_unstable();
    sorted_sequences.dedup();
    assert_eq!(
        stream_sequences, sorted_sequences,
        "stream sequences strictly increase"
    );
    drop(live_payloads);

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Hello");

    let events = crate::stored_upserts(&settings, "conversation-live-stream");
    assert!(!events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["kind"] == "assistant_message"
            && event["segment"]["content"] == "Hel"
    }));
    assert!(events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["kind"] == "assistant_message"
            && event["segment"]["content"] == "Hello"
    }));
}

#[test]
fn claude_code_live_text_completions_update_delta_segments_in_place() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let mut first_delta = content_delta("assistant", "First", "app-turn", "block-1");
    first_delta.phase = Some("commentary".to_string());
    let mut second_delta = content_delta("assistant", "Second", "app-turn", "block-2");
    second_delta.phase = Some("commentary".to_string());
    let mut first_completed = content_completed("assistant", "First", "app-turn", "block-1");
    first_completed.phase = Some("commentary".to_string());
    let mut second_completed = content_completed("assistant", "Second", "app-turn", "block-2");
    second_completed.phase = Some("commentary".to_string());
    let backend = ScriptedAgentTurnBackend::with_stream_events(
        vec![AgentTurnOutput {
            events: vec![
                first_completed,
                second_completed,
                content_completed("assistant", "Second", "app-turn", "block-2"),
            ],
            final_assistant_text: Some("Second".to_string()),
            ..AgentTurnOutput::default()
        }],
        vec![vec![first_delta, second_delta]],
    );
    let service = crate::conversations_with_backend(settings, Arc::new(backend));
    let live_payloads = Arc::new(Mutex::new(Vec::new()));
    let callback_payloads = Arc::clone(&live_payloads);

    let snapshot = service
        .execute_turn_with_progress_payloads(
            "conversation-claude-live",
            ConversationTurnRequest {
                project_path: "/projects/claude-live".to_string(),
                message: "Stream blocks".to_string(),
                provider: Some("claude-code".to_string()),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
            move |payload| callback_payloads.lock().expect("payloads").push(payload),
        )
        .expect("execute turn");

    let assistant_segments = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .filter(|segment| segment["kind"] == "assistant_message")
        .collect::<Vec<_>>();
    assert_eq!(assistant_segments.len(), 2);
    assert_eq!(assistant_segments[0]["content"], "First");
    assert_eq!(assistant_segments[1]["content"], "Second");
    assert_eq!(assistant_segments[1]["phase"], "final_answer");

    let mut live_segment_ids = live_payloads
        .lock()
        .expect("payloads")
        .iter()
        .filter(|payload| {
            payload["type"] == "stream_delta" && payload["delta_kind"] == "segment_text"
        })
        .filter_map(|payload| payload["segment_id"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    live_segment_ids.sort_unstable();
    live_segment_ids.dedup();
    // Live text streams under the ids its segments are stored with.
    let mut stored_ids = assistant_segments
        .iter()
        .map(|segment| segment["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    stored_ids.sort_unstable();
    assert_eq!(live_segment_ids, stored_ids);
}

#[cfg(unix)]
#[test]
fn claude_code_result_only_follow_up_keeps_the_earlier_result_text() {
    use std::os::unix::fs::PermissionsExt;
    let _env_lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (subtype, is_error) in [("success", false), ("error_during_execution", true)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = settings(temp.path());
        // A background task's completion brings a second result with no
        // assistant message of its own.
        let script = temp.path().join("fake-claude.sh");
        fs::write(
            &script,
            format!(
                r#"#!/bin/sh
read -r prompt
echo '{{"type":"system","subtype":"init","session_id":"s","capabilities":["interrupt_receipt_v1"]}}'
echo '{{"type":"system","subtype":"background_tasks_changed","session_id":"s","tasks":[{{"task_id":"b","task_type":"local_bash"}}]}}'
echo '{{"type":"assistant","session_id":"s","message":{{"role":"assistant","content":[{{"type":"text","text":"Started the build."}}]}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"Started the build.","session_id":"s","queued_turn_count":0}}'
echo '{{"type":"system","subtype":"background_tasks_changed","session_id":"s","tasks":[]}}'
echo '{{"type":"result","subtype":"{subtype}","is_error":{is_error},"result":"Build finished.","session_id":"s","queued_turn_count":0}}'
cat > /dev/null
"#
            ),
        )
        .expect("script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", &script);
        let service = crate::conversations(settings);

        let snapshot = service
            .execute_turn(
                "conversation-claude-results",
                ConversationTurnRequest {
                    project_path: temp.path().to_string_lossy().to_string(),
                    message: "Build it".to_string(),
                    provider: Some("claude-code".to_string()),
                    chat_mode: Some("chat".to_string()),
                    ..ConversationTurnRequest::default()
                },
            )
            .expect("execute turn");

        let contents = snapshot["segments"]
            .as_array()
            .expect("segments")
            .iter()
            .filter(|segment| segment["kind"] == "assistant_message")
            .map(|segment| segment["content"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            contents,
            ["Started the build.", "Build finished."],
            "{subtype}"
        );
    }
}

#[test]
fn claude_code_completed_tool_segment_preserves_canonical_command_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![AgentTurnOutput {
        events: vec![
            tool_event("tool_call_started", "tool-1", "started", ""),
            tool_event("tool_call_completed", "tool-1", "completed", "tests passed"),
            content_completed("assistant", "Done.", "app-turn", "final"),
        ],
        final_assistant_text: Some("Done.".to_string()),
        ..AgentTurnOutput::default()
    }]);
    let service = crate::conversations_with_backend(settings, Arc::new(backend));

    let snapshot = service
        .execute_turn(
            "conversation-claude-tool",
            ConversationTurnRequest {
                project_path: "/projects/claude-tool".to_string(),
                message: "Run tests".to_string(),
                provider: Some("claude-code".to_string()),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("execute turn");
    let tool = segment_by_kind(
        snapshot["segments"].as_array().expect("segments"),
        "tool_call",
    );
    assert_eq!(tool["tool_call"]["command"], "cargo test");
    assert_eq!(tool["tool_call"]["title"], "Run command");
    assert_eq!(tool["tool_call"]["output"], "tests passed");
}

#[test]
fn request_user_input_answers_call_backend_lifecycle_and_ingest_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(
        Vec::new(),
        vec![AgentTurnOutput {
            raw_log_lines: vec![AgentRawLogLine {
                direction: "incoming".to_string(),
                line: "{\"event\":\"request-user-input-answer\"}".to_string(),
            }],
            events: vec![
                content_delta("assistant", "Approved ", "app-answer", "final-answer"),
                content_completed(
                    "assistant",
                    "Approved after input.",
                    "app-answer",
                    "final-answer",
                ),
                token_usage(json!({"total": {"inputTokens": 7, "outputTokens": 3}})),
            ],
            final_assistant_text: Some("Approved after input.".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 7, "outputTokens": 3}})),
            ..AgentTurnOutput::default()
        }],
    );
    let answer_requests = backend.answer_requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input",
            ConversationTurnRequest {
                project_path: "/projects/input".to_string(),
                message: "Need approval".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("openrouter/model".to_string()),
                llm_profile: None,
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let pending = service
        .ingest_agent_turn_output(
            "conversation-input",
            "/projects/input",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");
    let request_segment = segment_by_kind(
        pending["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["request_user_input"]["status"], "pending");

    let answered_ack = service
        .submit_request_user_input_answer(
            "conversation-input",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("answer");
    let answered = service
        .get_snapshot("conversation-input", Some("/projects/input"))
        .expect("snapshot");
    assert_acks_request_segment(&answered_ack);
    let request_segment = segment_by_kind(
        answered["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["status"], "complete");
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Approved after input.");
    assert_eq!(
        assistant_turn["token_usage"]["total"]["inputTokens"],
        json!(7)
    );
    assert!(answered["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "assistant_message"
            && segment["content"] == "Approved after input."));
    let raw_log =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .read_codex_jsonrpc_trace("conversation-input", "/projects/input")
            .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    let answer_requests = answer_requests.lock().expect("answer requests");
    assert_eq!(answer_requests.len(), 1);
    let answer_request = &answer_requests[0];
    assert_eq!(answer_request.conversation_id, "conversation-input");
    assert_eq!(answer_request.project_path, "/projects/input");
    assert_eq!(answer_request.request_id, "input-1");
    assert_eq!(answer_request.assistant_turn_id, prepared.assistant_turn_id);
    assert_eq!(answer_request.answers["decision"], "Approve");
    assert_eq!(
        answer_request.request_user_input.as_ref().unwrap()["status"],
        "answered"
    );
    assert_eq!(
        answer_request.request_user_input.as_ref().unwrap()["answers"]["decision"],
        "Approve"
    );
    assert_eq!(answer_request.history.len(), 1);
    match &answer_request.history[0] {
        spark_agent_adapter::HistoryTurn::User(turn) => {
            assert_eq!(turn.text(), "Need approval");
        }
        other => panic!("unexpected history turn: {other:?}"),
    }
    assert_eq!(answer_request.provider.as_deref(), Some("openrouter"));
    assert_eq!(answer_request.model.as_deref(), Some("openrouter/model"));
    assert_eq!(answer_request.llm_profile.as_deref(), None);
    assert_eq!(answer_request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(answer_request.chat_mode.as_deref(), Some("chat"));
    assert_eq!(
        answer_request.metadata["spark.workspace.assistant_turn_id"],
        json!(prepared.assistant_turn_id)
    );
    assert_eq!(
        answer_request.metadata["spark.workspace.request_user_input.lookup_id"],
        json!("decision")
    );
    assert_eq!(
        answer_request.metadata["spark.runtime.codex_app_server.thread_id"],
        json!("app-thread")
    );
    assert_eq!(
        answer_request.metadata["spark.runtime.codex_app_server.turn_id"],
        json!("app-turn")
    );
    drop(answer_requests);

    let idempotent = service
        .submit_request_user_input_answer(
            "conversation-input",
            "input-1",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("idempotent answer");
    let after_idempotent = service
        .get_snapshot("conversation-input", Some("/projects/input"))
        .expect("snapshot");
    // A repeated answer commits nothing.
    assert!(idempotent.changes.rows.is_empty());
    assert_eq!(after_idempotent["revision"], answered["revision"]);

    let changed = service
        .submit_request_user_input_answer(
            "conversation-input",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Reject".to_string())]),
            },
        )
        .expect_err("changed answer conflict");
    assert!(matches!(changed, WorkspaceError::Conflict(_)));

    let missing = service
        .submit_request_user_input_answer(
            "conversation-input",
            "missing",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect_err("missing request");
    assert!(matches!(missing, WorkspaceError::NotFound(_)));
}

/// A question-only call that ends its provider turn returns while the user
/// answers: the answer commits after the call's turn boundary and before the
/// call's own return is recorded. That return still waits for the answer, so
/// the continuation completes the same turn.
#[test]
fn an_answer_recorded_while_its_question_only_call_returns_is_continued() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = AnswerDuringReturnBackend::default();
    let (asked, returned) = (backend.asked.clone(), backend.returned.clone());
    let service = crate::conversations_with_backend(settings, Arc::new(backend));
    let (prepared, _) = service
        .start_turn(
            "conversation-answer-race",
            ConversationTurnRequest {
                project_path: "/projects/answer-race".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let assistant_turn_id = prepared.assistant_turn_id.clone();
    let original = {
        let service = service.clone();
        std::thread::spawn(move || {
            let result = service.complete_started_turn_with_progress_payloads(prepared, |_| {});
            returned.open();
            result
        })
    };
    // The answer's backend call releases the original return, after the
    // answer commits, and continues only once that return is recorded.
    asked.wait();
    service
        .submit_request_user_input_answer(
            "conversation-answer-race",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/answer-race".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("answer");
    original.join().unwrap().expect("original call");

    let snapshot = service
        .get_snapshot("conversation-answer-race", Some("/projects/answer-race"))
        .expect("snapshot");
    let assistant = snapshot["turns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|turn| turn["id"] == assistant_turn_id.as_str())
        .expect("assistant turn");
    assert_eq!(assistant["status"], "complete", "{assistant}");
    assert_eq!(assistant["content"], "Approved.");
    let question = segment_by_kind(
        snapshot["segments"].as_array().unwrap(),
        "request_user_input",
    );
    assert_eq!(question["request_user_input"]["status"], "answered");
    assert_eq!(
        question["request_user_input"]["answers"]["decision"],
        "Approve"
    );
}

/// A gate one side opens and the other waits on.
#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn open(&self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }

    fn wait(&self) {
        let open = self.0 .0.lock().unwrap();
        let (open, _) = self
            .0
             .1
            .wait_timeout_while(open, Duration::from_secs(60), |open| !*open)
            .unwrap();
        assert!(*open, "timed out at a gate");
    }
}

/// Asks a question and ends its provider turn, then returns question-only
/// once the answer has committed; the answer continues in a new call after
/// that return is recorded.
#[derive(Default)]
struct AnswerDuringReturnBackend {
    asked: Gate,
    released: Gate,
    returned: Gate,
}

impl AgentTurnBackend for AnswerDuringReturnBackend {
    fn run_turn(&self, _request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        unreachable!("the turn runs with an event sink")
    }

    fn run_turn_with_event_sink(
        &self,
        _request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let sink = event_sink.expect("event sink");
        sink(request_user_input_event());
        sink(processing_completed_event());
        self.asked.open();
        self.released.wait();
        Ok(AgentTurnOutput::default())
    }

    fn answer_request_user_input(
        &self,
        _request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, AgentError> {
        self.released.open();
        self.returned.wait();
        Ok(AgentTurnOutput {
            events: vec![
                content_completed("assistant", "Approved.", "app-answer", "final-answer"),
                processing_completed_event(),
            ],
            final_assistant_text: Some("Approved.".to_string()),
            ..AgentTurnOutput::default()
        })
    }
}

#[test]
fn request_user_input_live_delivery_keeps_assistant_turn_streaming_until_original_turn_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(
        Vec::new(),
        vec![AgentTurnOutput {
            events: vec![TurnStreamEvent {
                kind: TurnStreamEventKind::Other("request_user_input_answer_delivered".to_string()),
                channel: None,
                source: TurnStreamSource {
                    backend: Some("codex_app_server".to_string()),
                    app_thread_id: Some("app-thread".to_string()),
                    app_turn_id: Some("app-turn".to_string()),
                    item_id: Some("input-1".to_string()),
                    raw_kind: Some("request_user_input_answer_delivered".to_string()),
                    ..TurnStreamSource::default()
                },
                content_delta: None,
                message: Some("request-user-input answer delivered.".to_string()),
                tool_call: None,
                request_user_input: None,
                token_usage: None,
                error: None,
                error_code: None,
                details: None,
                phase: Some("request_user_input_answer".to_string()),
                status: Some("delivered".to_string()),
            }],
            ..AgentTurnOutput::default()
        }],
    );
    let service = crate::conversations_with_backend(settings(temp.path()), Arc::new(backend));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input-live",
            ConversationTurnRequest {
                project_path: "/projects/input-live".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    service
        .ingest_agent_turn_output(
            "conversation-input-live",
            "/projects/input-live",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");

    let answered_ack = service
        .submit_request_user_input_answer(
            "conversation-input-live",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-live".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("live answer");
    let answered = service
        .get_snapshot("conversation-input-live", Some("/projects/input-live"))
        .expect("snapshot");
    assert_acks_request_segment(&answered_ack);
    let request_segment = segment_by_kind(
        answered["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["status"], "complete");
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "streaming");
    assert_eq!(assistant_turn["content"], "");
    assert!(!answered["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "assistant_message"));
}

#[test]
fn request_user_input_live_request_is_answerable_before_original_turn_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    // The answer alone must not finish the turn: "before it finishes" is held.
    let backend = BlockingRequestInputBackend::holding_turn_after_answer();
    let service =
        crate::conversations_with_backend(settings(temp.path()), Arc::new(backend.clone()));
    let (prepared, _) = service
        .start_turn(
            "conversation-input-live-pending",
            ConversationTurnRequest {
                project_path: "/projects/input-live-pending".to_string(),
                message: "Need approval".to_string(),
                provider: Some("codex".to_string()),
                chat_mode: Some("plan".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let assistant_turn_id = prepared.assistant_turn_id.clone();
    let progress_payloads = Arc::new(Mutex::new(Vec::new()));
    let progress_payloads_for_callback = Arc::clone(&progress_payloads);
    let service_for_turn = service.clone();

    let turn_handle = std::thread::spawn(move || {
        service_for_turn.complete_started_turn_with_progress_payloads(prepared, move |payload| {
            progress_payloads_for_callback
                .lock()
                .expect("progress payloads")
                .push(payload);
        })
    });

    backend.wait_for_request();
    let pending = service
        .get_snapshot(
            "conversation-input-live-pending",
            Some("/projects/input-live-pending"),
        )
        .expect("pending snapshot");
    assert!(
        pending["segments"]
            .as_array()
            .expect("segments")
            .iter()
            .any(|segment| {
                segment["kind"] == "request_user_input"
                    && segment["request_user_input"]["status"] == "pending"
            }),
        "request-user-input card should be committed before the turn completes"
    );
    assert!(progress_payloads
        .lock()
        .expect("progress payloads")
        .iter()
        .all(|payload| payload["type"] == "stream_delta"));

    let answered_ack = service
        .submit_request_user_input_answer(
            "conversation-input-live-pending",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-live-pending".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("live pending answer");
    let answered = service
        .get_snapshot(
            "conversation-input-live-pending",
            Some("/projects/input-live-pending"),
        )
        .expect("snapshot");
    assert_acks_request_segment(&answered_ack);
    let request_segment = segment_by_kind(
        answered["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "streaming");

    backend.release_turn();
    turn_handle
        .join()
        .expect("turn thread")
        .expect("turn completes after answer");
    let completed = service
        .get_snapshot(
            "conversation-input-live-pending",
            Some("/projects/input-live-pending"),
        )
        .expect("completed snapshot");
    let completed_assistant_turn = completed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(completed_assistant_turn["status"], "complete");
    assert_eq!(completed_assistant_turn["content"], "Approved.");
    assert_eq!(backend.answer_requests().lock().expect("answers").len(), 1);

    let completed_segments = completed["segments"].as_array().expect("segments");
    assert!(completed_segments.iter().any(|segment| {
        segment["kind"] == "request_user_input"
            && segment["request_user_input"]["request_id"] == "input-1"
            && segment["request_user_input"]["status"] == "answered"
    }));
    assert!(completed_segments.iter().any(|segment| {
        segment["kind"] == "request_user_input"
            && segment["request_user_input"]["request_id"] == "input-2"
            && segment["request_user_input"]["status"] == "pending"
    }));
}

#[test]
fn request_user_input_answers_expire_when_backend_lifecycle_cannot_resume() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(
        Vec::new(),
        vec![AgentTurnOutput {
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "request-user-input answer could not resume.".to_string(),
                error_code: Some("request_user_input_not_pending".to_string()),
                details: Some(json!({"request_id": "input-1"})),
            }),
            ..AgentTurnOutput::default()
        }],
    );
    let service = crate::conversations_with_backend(settings(temp.path()), Arc::new(backend));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input-expired",
            ConversationTurnRequest {
                project_path: "/projects/input-expired".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    service
        .ingest_agent_turn_output(
            "conversation-input-expired",
            "/projects/input-expired",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");

    let answered_ack = service
        .submit_request_user_input_answer(
            "conversation-input-expired",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-expired".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("answer");
    let answered = service
        .get_snapshot(
            "conversation-input-expired",
            Some("/projects/input-expired"),
        )
        .expect("snapshot");
    assert_acks_request_segment(&answered_ack);
    let request_segment = segment_by_kind(
        answered["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["status"], "failed");
    assert_eq!(request_segment["request_user_input"]["status"], "expired");
    assert_eq!(
        request_segment["error"],
        "The requested input expired before the answer could be used."
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(
        assistant_turn["error"],
        "The requested input expired before the answer could be used."
    );

    let idempotent = service
        .submit_request_user_input_answer(
            "conversation-input-expired",
            "input-1",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-expired".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("idempotent answer");
    let after_idempotent = service
        .get_snapshot(
            "conversation-input-expired",
            Some("/projects/input-expired"),
        )
        .expect("snapshot");
    // A repeated answer commits nothing.
    assert!(idempotent.changes.rows.is_empty());
    assert_eq!(after_idempotent["revision"], answered["revision"]);

    let changed = service
        .submit_request_user_input_answer(
            "conversation-input-expired",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-expired".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Reject".to_string())]),
            },
        )
        .expect_err("changed answer conflict");
    assert!(matches!(changed, WorkspaceError::Conflict(_)));
}

#[test]
fn request_user_input_answer_backend_errors_return_error_without_expiring_request() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(Vec::new(), Vec::new());
    let answer_requests = backend.answer_requests();
    let service = crate::conversations_with_backend(settings(temp.path()), Arc::new(backend));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input-backend-error",
            ConversationTurnRequest {
                project_path: "/projects/input-backend-error".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    service
        .ingest_agent_turn_output(
            "conversation-input-backend-error",
            "/projects/input-backend-error",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");

    let error = service
        .submit_request_user_input_answer(
            "conversation-input-backend-error",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-backend-error".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect_err("backend answer error");
    assert!(matches!(
        error,
        WorkspaceError::Internal(message)
            if message == "No scripted agent answer output available."
    ));
    let answer_requests = answer_requests.lock().expect("answer requests");
    assert_eq!(answer_requests.len(), 1);
    assert_eq!(answer_requests[0].request_id, "input-1");
    drop(answer_requests);

    let snapshot = service
        .get_snapshot(
            "conversation-input-backend-error",
            Some("/projects/input-backend-error"),
        )
        .expect("stored snapshot");
    let request_segment = segment_by_kind(
        snapshot["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["status"], "complete");
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    assert!(request_segment.get("error").is_none());
    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "streaming");
    assert!(assistant_turn.get("error").is_none());
}

#[derive(Clone)]
struct BlockingRequestInputBackend {
    requests: Arc<Mutex<Vec<AgentTurnRequest>>>,
    answer_requests: Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>>,
    request_emitted: Arc<(Mutex<bool>, Condvar)>,
    answer_delivered: Arc<(Mutex<bool>, Condvar)>,
    /// Open unless a test holds the turn running past its answer.
    turn_released: Arc<(Mutex<bool>, Condvar)>,
}

impl BlockingRequestInputBackend {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            answer_requests: Arc::new(Mutex::new(Vec::new())),
            request_emitted: Arc::new((Mutex::new(false), Condvar::new())),
            answer_delivered: Arc::new((Mutex::new(false), Condvar::new())),
            turn_released: Arc::new((Mutex::new(true), Condvar::new())),
        }
    }

    fn holding_turn_after_answer() -> Self {
        let backend = Self::new();
        *backend.turn_released.0.lock().expect("turn released") = false;
        backend
    }

    fn release_turn(&self) {
        let (lock, cvar) = &*self.turn_released;
        *lock.lock().expect("turn released") = true;
        cvar.notify_all();
    }

    fn wait_for_request(&self) {
        let (lock, cvar) = &*self.request_emitted;
        let emitted = lock.lock().expect("request emitted");
        let (emitted, timeout) = cvar
            .wait_timeout_while(emitted, Duration::from_secs(120), |emitted| !*emitted)
            .expect("request emitted wait");
        assert!(*emitted, "request-user-input event was not emitted");
        assert!(
            !timeout.timed_out(),
            "timed out waiting for request-user-input"
        );
    }

    fn answer_requests(&self) -> Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>> {
        Arc::clone(&self.answer_requests)
    }
}

impl AgentTurnBackend for BlockingRequestInputBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.run_turn_with_event_sink(request, None)
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        self.requests.lock().expect("requests").push(request);
        if let Some(sink) = event_sink.as_ref() {
            sink(request_user_input_event());
        }
        {
            let (lock, cvar) = &*self.request_emitted;
            *lock.lock().expect("request emitted") = true;
            cvar.notify_all();
        }
        let (lock, cvar) = &*self.answer_delivered;
        let delivered = lock.lock().expect("answer delivered");
        let (delivered, timeout) = cvar
            .wait_timeout_while(delivered, Duration::from_secs(120), |delivered| !*delivered)
            .expect("answer delivered wait");
        if !*delivered || timeout.timed_out() {
            return Err(AgentError {
                message: "Timed out waiting for request-user-input answer.".to_string(),
                retryable: false,
                raw: None,
            });
        }
        drop(delivered);
        {
            let (lock, cvar) = &*self.turn_released;
            let released = lock.lock().expect("turn released");
            drop(
                cvar.wait_timeout_while(released, Duration::from_secs(120), |released| !*released)
                    .expect("turn released wait"),
            );
        }
        let completed = content_completed("assistant", "Approved.", "app-turn", "final");
        if let Some(sink) = event_sink.as_ref() {
            sink(request_user_input_event_with("input-2", "decision-2"));
            sink(completed.clone());
        }
        Ok(AgentTurnOutput {
            events: vec![completed],
            app_thread_id: Some("thread-live".to_string()),
            app_turn_id: Some("app-turn".to_string()),
            final_assistant_text: Some("Approved.".to_string()),
            ..AgentTurnOutput::default()
        })
    }

    fn answer_request_user_input(
        &self,
        request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, AgentError> {
        self.answer_requests
            .lock()
            .expect("answer requests")
            .push(request);
        let (lock, cvar) = &*self.answer_delivered;
        *lock.lock().expect("answer delivered") = true;
        cvar.notify_all();
        Ok(AgentTurnOutput {
            events: vec![TurnStreamEvent {
                kind: TurnStreamEventKind::Other("request_user_input_answer_delivered".to_string()),
                channel: None,
                source: TurnStreamSource {
                    backend: Some("codex_app_server".to_string()),
                    app_thread_id: Some("app-thread".to_string()),
                    app_turn_id: Some("app-turn".to_string()),
                    item_id: Some("input-1".to_string()),
                    raw_kind: Some("request_user_input_answer_delivered".to_string()),
                    ..TurnStreamSource::default()
                },
                content_delta: None,
                message: Some("request-user-input answer delivered.".to_string()),
                tool_call: None,
                request_user_input: None,
                token_usage: None,
                error: None,
                error_code: None,
                details: None,
                phase: Some("request_user_input_answer".to_string()),
                status: Some("delivered".to_string()),
            }],
            ..AgentTurnOutput::default()
        })
    }
}

#[derive(Clone)]
struct ScriptedAgentTurnBackend {
    requests: Arc<Mutex<Vec<AgentTurnRequest>>>,
    outputs: Arc<Mutex<VecDeque<AgentTurnOutput>>>,
    stream_events: Arc<Mutex<VecDeque<Vec<TurnStreamEvent>>>>,
    answer_requests: Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>>,
    answer_outputs: Arc<Mutex<VecDeque<AgentTurnOutput>>>,
}

impl ScriptedAgentTurnBackend {
    fn new(outputs: Vec<AgentTurnOutput>) -> Self {
        Self::with_answer_outputs(outputs, Vec::new())
    }

    fn with_stream_events(
        outputs: Vec<AgentTurnOutput>,
        stream_events: Vec<Vec<TurnStreamEvent>>,
    ) -> Self {
        let backend = Self::with_answer_outputs(outputs, Vec::new());
        *backend.stream_events.lock().expect("stream events") = VecDeque::from(stream_events);
        backend
    }

    fn with_answer_outputs(
        mut outputs: Vec<AgentTurnOutput>,
        mut answer_outputs: Vec<AgentTurnOutput>,
    ) -> Self {
        for (index, output) in outputs.iter_mut().enumerate() {
            add_final_answer_event(output, &format!("scripted-turn-{index}"));
        }
        for (index, output) in answer_outputs.iter_mut().enumerate() {
            add_final_answer_event(output, &format!("scripted-answer-{index}"));
        }
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(VecDeque::from(outputs))),
            stream_events: Arc::new(Mutex::new(VecDeque::new())),
            answer_requests: Arc::new(Mutex::new(Vec::new())),
            answer_outputs: Arc::new(Mutex::new(VecDeque::from(answer_outputs))),
        }
    }

    fn requests(&self) -> Arc<Mutex<Vec<AgentTurnRequest>>> {
        Arc::clone(&self.requests)
    }

    fn answer_requests(&self) -> Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>> {
        Arc::clone(&self.answer_requests)
    }
}

fn add_final_answer_event(output: &mut AgentTurnOutput, app_turn_id: &str) {
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
    output
        .events
        .push(content_completed("assistant", &text, app_turn_id, "final"));
}

impl AgentTurnBackend for ScriptedAgentTurnBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.requests.lock().expect("requests").push(request);
        self.outputs
            .lock()
            .expect("outputs")
            .pop_front()
            .ok_or_else(|| AgentError {
                message: "No scripted agent output available.".to_string(),
                retryable: false,
                raw: None,
            })
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let Some(sink) = event_sink else {
            return self.run_turn(request);
        };
        if let Some(events) = self
            .stream_events
            .lock()
            .expect("stream events")
            .pop_front()
        {
            for event in events {
                sink(event);
            }
        }
        // Every event a backend returns has gone through its sink, on its thread.
        let output = self.run_turn(request)?;
        for event in &output.events {
            let mut event = event.clone();
            event.source.app_thread_id = output.app_thread_id.clone();
            sink(event);
        }
        Ok(output)
    }

    fn answer_request_user_input(
        &self,
        request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, AgentError> {
        self.answer_requests
            .lock()
            .expect("answer requests")
            .push(request);
        self.answer_outputs
            .lock()
            .expect("answer outputs")
            .pop_front()
            .ok_or_else(|| AgentError {
                message: "No scripted agent answer output available.".to_string(),
                retryable: false,
                raw: None,
            })
    }
}

fn content_delta(channel: &str, text: &str, app_turn_id: &str, item_id: &str) -> TurnStreamEvent {
    let mut event = TurnStreamEvent::content_delta(parse_channel(channel), text);
    event.source = source(app_turn_id, item_id);
    event.phase = Some(
        if channel == "assistant" {
            "final_answer"
        } else {
            "commentary"
        }
        .to_string(),
    );
    event
}

fn content_completed(
    channel: &str,
    text: &str,
    app_turn_id: &str,
    item_id: &str,
) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::ContentCompleted,
        channel: Some(parse_channel(channel)),
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

fn tool_event(kind: &str, id: &str, status: &str, output: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: kind.parse().expect("kind"),
        channel: None,
        source: source("app-turn", "tool-1"),
        content_delta: None,
        message: None,
        tool_call: Some(json!({
            "id": id,
            "kind": "command_execution",
            "status": status,
            "title": "Run command",
            "command": "cargo test",
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

fn model_tool_event(kind: &str, id: &str, status: &str, delta: Option<&str>) -> TurnStreamEvent {
    let mut tool_call = json!({
        "id": id,
        "kind": "model_tool_call",
        "status": status,
        "name": "lookup",
        "title": "lookup",
        "arguments": {"query": "rust"},
    });
    if let Some(delta) = delta {
        tool_call["delta"] = json!(delta);
    }

    TurnStreamEvent {
        kind: TurnStreamEventKind::Other(kind.to_string()),
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-turn".to_string()),
            item_id: Some(id.to_string()),
            response_id: Some("resp-1".to_string()),
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

fn token_usage(usage: Value) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::TokenUsageUpdated,
        channel: None,
        source: source("app-turn", "usage"),
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

fn request_user_input_event() -> TurnStreamEvent {
    request_user_input_event_with("input-1", "decision")
}

fn request_user_input_event_with(item_id: &str, question_id: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::RequestUserInputRequested,
        channel: None,
        source: source("app-turn", item_id),
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: Some(json!({
            "itemId": item_id,
            "questions": [
                {
                    "id": question_id,
                    "header": "Approve",
                    "question": "Approve this change?",
                    "options": [
                        {"label": "Approve", "description": "Continue"},
                        {"label": "Reject", "description": "Stop"}
                    ],
                    "isOther": false,
                    "isSecret": false
                }
            ]
        })),
        token_usage: None,
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
            app_turn_id: Some("app-turn".to_string()),
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
            app_turn_id: Some("app-turn".to_string()),
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

fn stream_error_event(message: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::Error,
        channel: Some(TurnStreamChannel::Assistant),
        source: source("app-turn", "error-item"),
        content_delta: None,
        message: Some(message.to_string()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: Some(message.to_string()),
        error_code: None,
        details: None,
        phase: Some("final_answer".to_string()),
        status: Some("failed".to_string()),
    }
}

#[test]
fn live_catch_up_sends_current_rows_to_any_older_cursor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());

    // start_turn commits the mode-change, user and assistant turns and the
    // settings change as one revision.
    let (_, acceptance) = service
        .start_turn(
            "conversation-ref-replay",
            ConversationTurnRequest {
                project_path: "/projects/ref-replay".to_string(),
                message: "Replay across refs".to_string(),
                provider: Some("openai".to_string()),
                model: Some("gpt-5".to_string()),
                llm_profile: None,
                reasoning_effort: None,
                chat_mode: Some("plan".to_string()),
                attachment_ids: Vec::new(),
                client_turn_id: None,
            },
        )
        .expect("start turn");

    // Any older cursor gets the current rows, carrying full current state;
    // the current revision gets nothing.
    let state = crate::live_rows_state(
        &settings,
        "conversation-ref-replay",
        "/projects/ref-replay",
        0,
    );
    assert_eq!(
        state["chat_mode"], "plan",
        "the conversation row carries full current metadata"
    );
    assert_eq!(state["turns"].as_array().expect("turns").len(), 3);
    let revision = acceptance.revision;
    assert_eq!(state["revision"], revision);
    let state = crate::live_rows_state(
        &settings,
        "conversation-ref-replay",
        "/projects/ref-replay",
        revision - 1,
    );
    assert_eq!(state["revision"], revision);
    assert_eq!(state["turns"].as_array().expect("turns").len(), 3);
    assert!(crate::live_change_rows(
        &settings,
        "conversation-ref-replay",
        "/projects/ref-replay",
        revision,
    )
    .is_empty());
}

/// The answer's result carries its request-user-input segment row.
fn assert_acks_request_segment(ack: &spark_workspace::conversations::MutationAck) {
    assert!(
        ack.changes.rows.iter().any(|row| {
            row.kind == spark_storage::conversation::EntityKind::Segment
                && row
                    .payload
                    .as_ref()
                    .is_some_and(|segment| segment["kind"] == "request_user_input")
        }),
        "{ack:?}"
    );
}

fn structured_stream_error_event(
    message: &str,
    error_code: &str,
    details: Value,
) -> TurnStreamEvent {
    let mut event = stream_error_event(message);
    event.error_code = Some(error_code.to_string());
    event.details = Some(details);
    event
}

fn parse_channel(channel: &str) -> TurnStreamChannel {
    channel.parse().expect("channel")
}

fn source(app_turn_id: &str, item_id: &str) -> TurnStreamSource {
    TurnStreamSource {
        app_thread_id: Some("app-thread".to_string()),
        app_turn_id: Some(app_turn_id.to_string()),
        item_id: Some(item_id.to_string()),
        ..TurnStreamSource::default()
    }
}

fn segment_by_kind<'a>(segments: &'a [Value], kind: &str) -> &'a Value {
    segments
        .iter()
        .find(|segment| segment["kind"] == kind)
        .expect("segment kind")
}

fn write_state(conversations_dir: &Path, conversation_id: &str, state: Value) {
    let directory = conversations_dir.join(conversation_id);
    fs::create_dir_all(&directory).expect("conversation dir");
    fs::write(
        directory.join("state.json"),
        serde_json::to_string_pretty(&state).expect("state json"),
    )
    .expect("write state");
}

pub(crate) fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        runs_dir: None,
        ui_dir: None,
        project_roots: Vec::<PathBuf>::new(),
    }
}

#[test]
fn runtime_session_records_thread_continuity_and_tombstones_on_resume_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            app_thread_id: Some("thread-alpha".to_string()),
            final_assistant_text: Some("First answer".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "thread resume failed".to_string(),
                error_code: Some("thread_resume_failed".to_string()),
                details: Some(json!({"thread_id": "thread-alpha"})),
            }),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Fresh thread answer".to_string()),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let repository =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir));

    service
        .execute_turn(
            "conversation-runtime-session",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Establish the thread".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("first turn");
    let first_assistant_turn_id = requests.lock().expect("requests")[0].metadata
        ["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();
    let session = repository
        .read_runtime_session(
            "conversation-runtime-session",
            Some("/projects/runtime-session"),
        )
        .expect("read session")
        .expect("session written after thread id observed");
    assert_eq!(session.provider, "codex_app_server");
    assert_eq!(session.thread_id.as_deref(), Some("thread-alpha"));
    assert_eq!(
        session.last_turn_id.as_deref(),
        Some(first_assistant_turn_id.as_str())
    );
    assert!(!session.resume_failed);
    assert!(!session.established_at.is_empty());

    service
        .execute_turn(
            "conversation-runtime-session",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Resume the previous thread".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("resume failure turn");
    assert_eq!(
        requests.lock().expect("requests")[1].metadata["spark.runtime.codex_app_server.thread_id"],
        json!("thread-alpha")
    );
    let tombstoned = repository
        .read_runtime_session(
            "conversation-runtime-session",
            Some("/projects/runtime-session"),
        )
        .expect("read tombstone")
        .expect("session still present");
    assert!(tombstoned.resume_failed);
    assert_eq!(
        tombstoned.thread_id.as_deref(),
        Some("thread-alpha"),
        "failed thread id is kept for debugging"
    );

    service
        .execute_turn(
            "conversation-runtime-session",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Start over".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("fresh thread turn");
    assert!(
        !requests.lock().expect("requests")[2]
            .metadata
            .contains_key("spark.runtime.codex_app_server.thread_id"),
        "tombstoned continuity must not resume the failed thread"
    );
}

#[test]
fn claude_code_failed_resume_is_tombstoned() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            app_thread_id: Some("claude-session-old".to_string()),
            final_assistant_text: Some("First answer".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Answer without a session".to_string()),
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "discarded stale Claude Code session".to_string(),
                error_code: Some("thread_resume_failure".to_string()),
                details: Some(json!({"discarded_session_id": "claude-session-old"})),
            }),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Continued answer".to_string()),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let (conversation, project_path) = (
        "conversation-claude-tombstone",
        "/projects/claude-tombstone",
    );
    service
        .update_conversation_settings(
            conversation,
            ConversationSettingsUpdate {
                project_path: project_path.to_string(),
                provider: Some("claude-code".to_string()),
                model: Some(String::new()),
                expected_revision: Some("0".into()),
                llm_profile: None,
                ..ConversationSettingsUpdate::default()
            },
        )
        .expect("claude-code settings");
    for message in ["first", "recover", "continue"] {
        service
            .execute_turn(
                conversation,
                ConversationTurnRequest {
                    project_path: project_path.to_string(),
                    message: message.to_string(),
                    ..ConversationTurnRequest::default()
                },
            )
            .expect("claude-code turn");
    }

    let requests = requests.lock().expect("requests");
    assert!(!requests[2]
        .metadata
        .contains_key("spark.runtime.claude_code.session_id"));
    let session =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .read_runtime_session(conversation, Some(project_path))
            .expect("session read")
            .expect("session");
    assert_eq!(session.thread_id.as_deref(), Some("claude-session-old"));
    assert!(session.resume_failed);
}

#[test]
fn claude_code_recovery_keeps_fresh_turn_and_replaces_runtime_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            app_thread_id: Some("claude-session-old".to_string()),
            final_assistant_text: Some("First answer".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            app_thread_id: Some("claude-session-fresh".to_string()),
            final_assistant_text: Some("Recovered answer".to_string()),
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "discarded stale Claude Code session".to_string(),
                error_code: Some("thread_resume_failure".to_string()),
                details: Some(json!({"discarded_session_id": "claude-session-old"})),
            }),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Continued answer".to_string()),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let project_path = "/projects/claude-code-continuity";

    service
        .update_conversation_settings(
            "conversation-claude-code-continuity",
            ConversationSettingsUpdate {
                project_path: project_path.to_string(),
                provider: Some("claude-code".to_string()),
                model: Some(String::new()),
                expected_revision: Some("0".into()),
                llm_profile: None,
                ..ConversationSettingsUpdate::default()
            },
        )
        .expect("claude-code accepts its default model");

    for message in ["first", "recover", "continue"] {
        service
            .execute_turn(
                "conversation-claude-code-continuity",
                ConversationTurnRequest {
                    project_path: project_path.to_string(),
                    message: message.to_string(),
                    ..ConversationTurnRequest::default()
                },
            )
            .expect("claude-code turn");
    }

    let requests = requests.lock().expect("requests");
    assert_eq!(
        requests[1].metadata["spark.runtime.claude_code.session_id"],
        json!("claude-session-old")
    );
    assert_eq!(
        requests[2].metadata["spark.runtime.claude_code.session_id"],
        json!("claude-session-fresh")
    );
    let snapshot = service
        .get_snapshot("conversation-claude-code-continuity", Some(project_path))
        .expect("snapshot");
    assert!(snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| turn["content"] == "Recovered answer" && turn["status"] == "complete"));
    let session =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir))
            .read_runtime_session("conversation-claude-code-continuity", Some(project_path))
            .expect("session read")
            .expect("session");
    assert_eq!(session.provider, "claude_code_cli");
    assert_eq!(session.thread_id.as_deref(), Some("claude-session-fresh"));
    assert!(!session.resume_failed);
}

/// A settled turn is stored in its final form, including an unmatched tool
/// start finalized as `yielded` at the turn boundary, so reading it again
/// writes nothing: no new revision, identical rows.
#[test]
fn a_settled_turn_reads_back_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = crate::conversations(settings.clone());
    let conversation_id = "conversation-live-equivalence";
    let project_path = "/projects/live-equivalence";
    let (prepared, _) = service
        .start_turn(
            conversation_id,
            ConversationTurnRequest {
                project_path: project_path.to_string(),
                message: "Run things".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let turn_completed = TurnStreamEvent {
        kind: TurnStreamEventKind::TurnCompleted,
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-turn".to_string()),
            raw_kind: Some("turn_completed".to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: Some("completed".to_string()),
    };
    service
        .ingest_agent_turn_output(
            conversation_id,
            project_path,
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![
                    tool_event("tool_call_started", "exec-a", "running", ""),
                    content_completed("assistant", "Done.", "app-turn", "answer"),
                    turn_completed,
                ],
                final_assistant_text: Some("Done.".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest");

    let repo = ConversationRepository::new(crate::shared_store(&settings.data_dir));
    let snapshot = repo
        .read_snapshot(conversation_id, Some(project_path))
        .expect("read")
        .expect("snapshot");
    let revision = snapshot["revision"].as_i64().expect("revision");
    let segments: Vec<&serde_json::Value> = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .filter(|segment| segment["turn_id"] == prepared.assistant_turn_id.as_str())
        .collect();

    let tool = segments
        .iter()
        .find(|segment| segment["id"] == "segment-tool-app-turn-exec-a")
        .expect("yielded tool");
    assert_eq!(tool["status"], "complete");
    assert_eq!(tool["tool_call"]["status"], "yielded");
    assert_eq!(
        tool["tool_call"]["completion_reason"],
        "turn_boundary_yield"
    );
    let answer = segments
        .iter()
        .find(|segment| segment["kind"] == "assistant_message")
        .expect("answer");
    assert!(tool["order"].as_i64().expect("order") < answer["order"].as_i64().expect("order"));
    let lifecycle: Vec<&&serde_json::Value> = segments
        .iter()
        .filter(|segment| segment["kind"] == "agent_event")
        .collect();
    assert_eq!(lifecycle.len(), 1);
    assert_eq!(
        lifecycle[0]["id"],
        "segment-agent-event-app-turn-turn_completed"
    );

    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project paths");
    let _ = project;
    let rows = || {
        crate::shared_store(&settings.data_dir)
            .database()
            .read(|read| read.conversation(conversation_id))
            .expect("rows")
    };
    let rows_before = rows();
    let reopened = repo
        .read_snapshot(conversation_id, Some(project_path))
        .expect("reopen")
        .expect("snapshot");
    assert_eq!(reopened["revision"].as_i64().expect("revision"), revision);
    assert_eq!(rows(), rows_before);
}

#[test]
fn claude_questions_checkpoint_resume_and_expire_using_existing_segments() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _bin = EnvVarGuard::set(
        "SPARK_CLAUDE_CODE_BIN",
        env!("CARGO_BIN_EXE_spark-workspace-fake-claude-code"),
    );
    for mode in ["question", "dead-question", "interrupt-question"] {
        let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", mode);
        let config = settings(temp.path());
        let service = crate::conversations(config.clone());
        let id = format!("claude-{mode}");
        let (prepared, _) = service
            .start_turn(
                &id,
                ConversationTurnRequest {
                    project_path: project.clone(),
                    message: "Ask me which color".into(),
                    provider: Some("claude-code".into()),
                    ..ConversationTurnRequest::default()
                },
            )
            .unwrap();
        let running = service.clone();
        let handle = std::thread::spawn(move || {
            running.complete_started_turn_with_progress_payloads(prepared, |_| {})
        });
        // The pending question is committed before the turn completes.
        let repository = ConversationRepository::new(crate::shared_store(&config.data_dir));
        let waited = std::time::Instant::now();
        let persisted = loop {
            let snapshot = repository
                .read_snapshot(&id, Some(&project))
                .unwrap()
                .unwrap();
            if snapshot["segments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|segment| segment["kind"] == "request_user_input")
            {
                break snapshot;
            }
            assert!(
                waited.elapsed() < Duration::from_secs(10),
                "pending question before turn completion"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let segment = persisted["segments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|segment| segment["kind"] == "request_user_input")
            .unwrap();
        assert_eq!(segment["request_user_input"]["status"], "pending");
        assert_eq!(
            segment["request_user_input"]["questions"][0]["options"][1]["label"],
            "Blue"
        );
        // A fresh service reads the checkpoint and routes to the same live adapter.
        let reopened = crate::conversations(config.clone());
        if mode == "question" {
            let answered_ack = reopened
                .submit_request_user_input_answer(
                    &id,
                    "question-1",
                    ConversationRequestUserInputAnswerRequest {
                        project_path: project.clone(),
                        answers: BTreeMap::from([("question-1".into(), "Blue".into())]),
                    },
                )
                .unwrap();
            assert_acks_request_segment(&answered_ack);
            let answered = reopened.get_snapshot(&id, Some(&project)).unwrap();
            assert!(answered["segments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|segment| segment["request_user_input"]["status"] == "answered"));
        } else if mode == "interrupt-question" {
            assert!(reopened.interrupt_turn(&id, &project).unwrap());
        }
        handle.join().unwrap().unwrap();
        let completed = reopened.get_snapshot(&id, Some(&project)).unwrap();
        let assistant = completed["turns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|turn| turn["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant["status"], "complete");
        if mode == "interrupt-question" {
            assert_eq!(assistant["content"], "Stopped.");
        }
        if mode != "question" {
            reopened
                .submit_request_user_input_answer(
                    &id,
                    "question-1",
                    ConversationRequestUserInputAnswerRequest {
                        project_path: project.clone(),
                        answers: BTreeMap::from([("question-1".into(), "Blue".into())]),
                    },
                )
                .unwrap();
            let expired = reopened.get_snapshot(&id, Some(&project)).unwrap();
            assert!(expired["segments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|segment| segment["request_user_input"]["status"] == "expired"));
        } else {
            assert!(completed["segments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|segment| segment["request_user_input"]["status"] == "answered"));
            assert_eq!(assistant["status"], "complete");
        }
        assert!(!reopened.interrupt_turn(&id, &project).unwrap());
    }
}

#[test]
fn claude_chat_turns_on_one_session_each_report_their_own_usage_delta() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _bin = EnvVarGuard::set(
        "SPARK_CLAUDE_CODE_BIN",
        env!("CARGO_BIN_EXE_spark-workspace-fake-claude-code"),
    );
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "default");
    let service = crate::conversations(settings(temp.path()));
    for message in ["first", "second"] {
        service
            .execute_turn(
                "claude-usage",
                ConversationTurnRequest {
                    project_path: project.clone(),
                    message: message.into(),
                    provider: Some("claude-code".into()),
                    ..ConversationTurnRequest::default()
                },
            )
            .unwrap();
    }
    let snapshot = service
        .get_snapshot("claude-usage", Some(&project))
        .unwrap();
    let usage = snapshot["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|turn| turn["role"] == "assistant")
        .map(|turn| {
            assert_eq!(turn["app_thread_id"], "sess-fake-claude-1");
            assert_eq!(turn["token_usage_breakdown"], turn["token_usage"]);
            (
                turn["token_usage"]["input_tokens"].clone(),
                turn["token_usage"]["by_model"]["claude-opus-4-8"]["cost_usd"].clone(),
            )
        })
        .collect::<Vec<_>>();
    // The second result's modelUsage is cumulative; the turn reports only its delta.
    assert_eq!(usage, [(json!(12), json!(0.5)), (json!(101), json!(0.25))]);
}

#[test]
fn claude_plan_mode_turn_creates_a_pending_review_proposed_plan() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _bin = EnvVarGuard::set(
        "SPARK_CLAUDE_CODE_BIN",
        env!("CARGO_BIN_EXE_spark-workspace-fake-claude-code"),
    );
    // The fake submits a plan through ExitPlanMode and asserts the deny.
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "exit-plan-mode");
    let service = crate::conversations(settings(temp.path()));

    let snapshot = service
        .execute_turn(
            "claude-plan",
            ConversationTurnRequest {
                project_path: project,
                message: "Plan it.".into(),
                provider: Some("claude-code".into()),
                chat_mode: Some("plan".into()),
                ..ConversationTurnRequest::default()
            },
        )
        .unwrap();

    let plan = snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|segment| segment["kind"] == "plan")
        .expect("plan segment");
    assert_eq!(plan["content"], "# Plan\n\n- Do the thing.");
    let artifact = &snapshot["proposed_plans"][0];
    assert_eq!(artifact["status"], "pending_review");
    assert_eq!(artifact["content"], "# Plan\n\n- Do the thing.");
    assert_eq!(plan["artifact_id"], artifact["id"]);
}

#[test]
fn claude_stream_interrupt_error_result_completes_shortened_turn() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _bin = EnvVarGuard::set(
        "SPARK_CLAUDE_CODE_BIN",
        env!("CARGO_BIN_EXE_spark-workspace-fake-claude-code"),
    );
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "interrupt-aborted");
    let service = crate::conversations(settings(temp.path()));
    let (prepared, _) = service
        .start_turn(
            "interrupt-stream",
            ConversationTurnRequest {
                project_path: project.clone(),
                message: "Start writing".into(),
                provider: Some("claude-code".into()),
                ..ConversationTurnRequest::default()
            },
        )
        .unwrap();
    let controller = service.clone();
    let interrupted = std::sync::atomic::AtomicBool::new(false);
    let controller_project = project.clone();
    service
        .complete_started_turn_with_progress_payloads(prepared, move |payload| {
            if payload["type"] == "stream_delta"
                && payload["text"] == "Shortened turn."
                && !interrupted.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                assert!(controller
                    .interrupt_turn("interrupt-stream", &controller_project)
                    .unwrap());
            }
        })
        .unwrap();
    let completed = service
        .get_snapshot("interrupt-stream", Some(&project))
        .unwrap();
    let assistant = completed["turns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["status"], "complete");
    let text_segments = completed["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|segment| segment["content"] == "Shortened turn.")
        .collect::<Vec<_>>();
    assert_eq!(
        text_segments.len(),
        1,
        "interrupt must finish the existing streamed item"
    );
    assert_eq!(text_segments[0]["status"], "complete");
}

fn turn_with<'a>(snapshot: &'a Value, role: &str, nth: usize) -> &'a Value {
    snapshot["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|turn| turn["role"] == role && turn["kind"] == "message")
        .nth(nth)
        .unwrap_or_else(|| panic!("{role} turn {nth} in {snapshot}"))
}

/// Retries the failed assistant turn with the fake CLI signed in again and
/// checks the new turn resent the same user text.
fn assert_retry_succeeds(
    service: &WorkspaceConversationService,
    id: &str,
    project: &str,
    log: &Path,
    text: &str,
) {
    let failed = service.get_snapshot(id, Some(project)).unwrap();
    let failed_turn = turn_with(&failed, "assistant", 0);
    assert_eq!(failed_turn["status"], "failed", "{failed}");
    assert!(failed_turn["error"]
        .as_str()
        .unwrap()
        .contains("Claude Code needs sign-in"));
    let failed_id = failed_turn["id"].as_str().unwrap();
    let user_turn = turn_with(&failed, "user", 0)["id"].clone();
    assert!(matches!(
        service.retry_turn(id, project, user_turn.as_str().unwrap()),
        Err(WorkspaceError::NotFound(_))
    ));

    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "default");
    let (prepared, _) = service.retry_turn(id, project, failed_id).unwrap();
    assert_eq!(prepared.prompt, text);
    service
        .complete_started_turn_with_progress_payloads(prepared, |_| {})
        .unwrap();
    let completed = service.get_snapshot(id, Some(project)).unwrap();
    assert_eq!(turn_with(&completed, "user", 1)["content"], text);
    assert_eq!(turn_with(&completed, "assistant", 1)["status"], "complete");
    let sent = std::fs::read_to_string(log).unwrap().matches(text).count();
    assert!(sent >= 2, "both attempts carry the user text");
}

#[test]
fn a_chat_turn_failed_on_sign_in_retries_with_the_same_message() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let log = temp.path().join("claude.log");
    let _bin = EnvVarGuard::set(
        "SPARK_CLAUDE_CODE_BIN",
        env!("CARGO_BIN_EXE_spark-workspace-fake-claude-code"),
    );
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", &log);
    let service = crate::conversations(settings(temp.path()));
    {
        let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "auth-error");
        let (prepared, _) = service
            .start_turn(
                "retry-chat",
                ConversationTurnRequest {
                    project_path: project.clone(),
                    message: "Summarize the plan".into(),
                    provider: Some("claude-code".into()),
                    ..ConversationTurnRequest::default()
                },
            )
            .unwrap();
        let _ = service.complete_started_turn_with_progress_payloads(prepared, |_| {});
    }
    assert_retry_succeeds(&service, "retry-chat", &project, &log, "Summarize the plan");
}

#[test]
fn a_mission_turn_failed_on_sign_in_retries_with_the_same_delivered_prompt() {
    use spark_workspace::inbox::{install_runtime, InboxRuntime};

    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let log = temp.path().join("claude.log");
    let _bin = EnvVarGuard::set(
        "SPARK_CLAUDE_CODE_BIN",
        env!("CARGO_BIN_EXE_spark-workspace-fake-claude-code"),
    );
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", &log);
    let config = settings(temp.path());
    install_runtime(
        &config,
        InboxRuntime {
            agent_turn_backend: WorkspaceConversationService::default_agent_turn_backend(&config),
            publish: Arc::new(|_| {}),
        },
    );
    let missions = crate::missions(config.clone());
    let mission = missions
        .create(
            &project,
            serde_json::from_value(json!({"fields": {"title": "Retry"}})).unwrap(),
        )
        .unwrap();
    let service = crate::conversations(config.clone());
    service
        .update_conversation_settings(
            &mission.id,
            ConversationSettingsUpdate {
                project_path: project.clone(),
                provider: Some("claude-code".into()),
                expected_revision: Some("0".into()),
                ..ConversationSettingsUpdate::default()
            },
        )
        .unwrap();
    {
        let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "auth-error");
        missions.start(&project, &mission.id).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while turn_status(&service, &mission.id, &project) != Some("failed".into()) {
            assert!(
                std::time::Instant::now() < deadline,
                "mission turn never failed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert_retry_succeeds(
        &service,
        &mission.id,
        &project,
        &log,
        "Begin work on this mission.",
    );
}

fn turn_status(service: &WorkspaceConversationService, id: &str, project: &str) -> Option<String> {
    let snapshot = service.get_snapshot(id, Some(project)).ok()?;
    snapshot["turns"]
        .as_array()?
        .iter()
        .find(|turn| turn["role"] == "assistant")?["status"]
        .as_str()
        .map(str::to_string)
}

/// Holds every turn open until released or stopped. It cannot take a
/// message mid-turn, so sends during a turn queue.
#[derive(Default)]
struct GatedAgentTurnBackend {
    prompts: Mutex<Vec<String>>,
    released: (Mutex<bool>, Condvar),
}

impl GatedAgentTurnBackend {
    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}

impl AgentTurnBackend for GatedAgentTurnBackend {
    fn interrupt_turn(&self, _project_path: &str, _conversation_id: &str) -> bool {
        self.release();
        true
    }

    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.prompts.lock().unwrap().push(request.prompt.clone());
        let mut released = self.released.0.lock().unwrap();
        while !*released {
            released = self.released.1.wait(released).unwrap();
        }
        let text = format!("Re: {}", request.prompt);
        Ok(AgentTurnOutput {
            events: vec![content_completed("assistant", &text, &text, &text)],
            final_assistant_text: Some(text),
            ..AgentTurnOutput::default()
        })
    }
}

/// Holds the first turn open like [`GatedAgentTurnBackend`] and keeps every
/// request. With `error_first`, that turn reports a provider error at once
/// and, once released, returns with nothing more.
#[derive(Default)]
struct HeldFirstTurnBackend {
    gate: GatedAgentTurnBackend,
    requests: Mutex<Vec<AgentTurnRequest>>,
    error_first: bool,
}

impl AgentTurnBackend for HeldFirstTurnBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.run_turn_with_event_sink(request, None)
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let sink = event_sink.expect("conversation turns stream");
        let failing = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            self.error_first && requests.len() == 1
        };
        if failing {
            sink(stream_error_event("The provider failed."));
        }
        let output = self.gate.run_turn(request)?;
        if failing {
            return Ok(AgentTurnOutput::default());
        }
        for event in &output.events {
            sink(event.clone());
        }
        Ok(output)
    }
}

/// Waits until the conversation's first assistant turn shows `status`.
fn wait_for_turn_status(
    service: &WorkspaceConversationService,
    id: &str,
    project: &str,
    status: &str,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while turn_status(service, id, project).as_deref() != Some(status) {
        assert!(
            std::time::Instant::now() < deadline,
            "the turn never showed {status}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Sends a message while a turn runs. Returns its acceptance and the
/// conversation read right after it.
fn send(
    service: &WorkspaceConversationService,
    id: &str,
    project: &str,
    message: &str,
) -> (spark_workspace::conversations::TurnAcceptance, Value) {
    let (prepared, acceptance) = service
        .send_turn(
            id,
            ConversationTurnRequest {
                project_path: project.to_string(),
                message: message.to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("a send during a turn is not a conflict");
    assert!(prepared.is_none(), "{message} must not start a second turn");
    let snapshot = service.get_snapshot(id, Some(project)).unwrap();
    (acceptance, snapshot)
}

/// Starts `first` and completes it, and every message queued behind it, on a thread.
fn start_gated_turn(
    service: &WorkspaceConversationService,
    id: &str,
    project: &str,
) -> std::thread::JoinHandle<Result<(), WorkspaceError>> {
    let (prepared, _) = service
        .send_turn(
            id,
            ConversationTurnRequest {
                project_path: project.to_string(),
                message: "first".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .unwrap();
    let running = service.clone();
    std::thread::spawn(move || {
        running.complete_started_turn_with_progress_payloads(prepared.unwrap(), |_| {})
    })
}

fn message_turns(snapshot: &Value) -> Vec<(String, String, String)> {
    snapshot["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|turn| turn["kind"] == "message")
        .map(|turn| {
            (
                turn["role"].as_str().unwrap().to_string(),
                turn["status"].as_str().unwrap().to_string(),
                turn["content"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn messages_sent_during_a_turn_that_cannot_take_them_queue_and_start_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let backend = Arc::new(GatedAgentTurnBackend::default());
    let service = crate::conversations_with_backend(settings(temp.path()), backend.clone());
    let project = "/projects/queue";
    let running = start_gated_turn(&service, "queue", project);

    let (second, _) = send(&service, "queue", project, "second");
    let (third, queued) = send(&service, "queue", project, "third");
    // Each is accepted as queued, with no turn of its own yet.
    for acceptance in [&second, &third] {
        assert_eq!(acceptance.status, "queued");
        assert_eq!(acceptance.assistant_turn_id, None);
    }
    assert!(second.revision < third.revision);
    let user_turn = |id: &str| {
        queued["turns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|turn| turn["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("turn {id} in {queued}"))
    };
    assert_eq!(user_turn(&second.user_turn_id)["content"], "second");
    assert_eq!(user_turn(&third.user_turn_id)["content"], "third");
    let user =
        |content: &str, status: &str| ("user".to_string(), status.to_string(), content.to_string());
    assert_eq!(
        message_turns(&queued),
        vec![
            user("first", "complete"),
            (
                "assistant".to_string(),
                "pending".to_string(),
                String::new()
            ),
            user("second", "queued"),
            user("third", "queued"),
        ]
    );

    backend.release();
    running.join().unwrap().unwrap();
    let completed = service.get_snapshot("queue", Some(project)).unwrap();
    assert_eq!(
        *backend.prompts.lock().unwrap(),
        ["first", "second", "third"]
    );
    let assistant = |content: &str| {
        (
            "assistant".to_string(),
            "complete".to_string(),
            content.to_string(),
        )
    };
    assert_eq!(
        message_turns(&completed),
        vec![
            user("first", "complete"),
            assistant("Re: first"),
            user("second", "complete"),
            assistant("Re: second"),
            user("third", "complete"),
            assistant("Re: third"),
        ]
    );
}

#[test]
fn stop_starts_the_queued_message_next_instead_of_dropping_it() {
    let temp = tempfile::tempdir().unwrap();
    let backend = Arc::new(GatedAgentTurnBackend::default());
    let service = crate::conversations_with_backend(settings(temp.path()), backend.clone());
    let project = "/projects/stop-queue";
    let running = start_gated_turn(&service, "stop-queue", project);
    send(&service, "stop-queue", project, "after stop");

    assert!(service.interrupt_turn("stop-queue", project).unwrap());
    running.join().unwrap().unwrap();
    let completed = service.get_snapshot("stop-queue", Some(project)).unwrap();

    assert_eq!(*backend.prompts.lock().unwrap(), ["first", "after stop"]);
    assert!(message_turns(&completed)
        .iter()
        .all(|(_, status, _)| status == "complete"));
}

/// The picker switches provider before the running turn reports its thread:
/// the thread stays with the provider that ran it, and the new provider
/// never resumes it.
#[test]
fn a_thread_reported_after_a_provider_switch_stays_with_the_provider_that_ran_it() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let backend = Arc::new(HeldFirstTurnBackend::default());
    let service = crate::conversations_with_backend(settings.clone(), backend.clone());
    let project = "/projects/provider-switch";
    let running = start_gated_turn(&service, "provider-switch", project);
    let revision = service
        .get_snapshot("provider-switch", Some(project))
        .unwrap()["settings_revision"]
        .to_string();
    service
        .update_conversation_settings(
            "provider-switch",
            ConversationSettingsUpdate {
                project_path: project.to_string(),
                provider: Some("claude-code".to_string()),
                model: Some(String::new()),
                expected_revision: Some(revision),
                ..ConversationSettingsUpdate::default()
            },
        )
        .expect("the picker switches provider while the turn runs");
    backend.gate.release();
    running.join().unwrap().unwrap();

    let session = ConversationRepository::new(crate::shared_store(&settings.data_dir))
        .read_runtime_session("provider-switch", Some(project))
        .unwrap()
        .expect("session");
    assert_eq!(session.provider, "codex_app_server");
    assert_eq!(session.thread_id.as_deref(), Some("app-thread"));

    service
        .execute_turn(
            "provider-switch",
            ConversationTurnRequest {
                project_path: project.to_string(),
                message: "next".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("claude-code turn");
    let requests = backend.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        !requests[1]
            .metadata
            .contains_key("spark.runtime.claude_code.session_id"),
        "{:?}",
        requests[1].metadata
    );
}

/// A message sent after a provider error failed the turn, before its backend
/// call returns, queues behind that execution and runs once it settles.
#[test]
fn a_message_sent_between_a_provider_error_and_the_return_runs_after_it() {
    let temp = tempfile::tempdir().unwrap();
    let backend = Arc::new(HeldFirstTurnBackend {
        error_first: true,
        ..HeldFirstTurnBackend::default()
    });
    let service = crate::conversations_with_backend(settings(temp.path()), backend.clone());
    let project = "/projects/error-gap";
    let running = start_gated_turn(&service, "error-gap", project);
    // The error shows while the backend call still runs.
    wait_for_turn_status(&service, "error-gap", project, "failed");

    let (sent, snapshot) = send(&service, "error-gap", project, "second");
    assert_eq!((sent.status, sent.assistant_turn_id), ("queued", None));
    let turn = |role: &str, status: &str, content: &str| {
        (role.to_string(), status.to_string(), content.to_string())
    };
    assert_eq!(
        message_turns(&snapshot),
        vec![
            turn("user", "complete", "first"),
            turn("assistant", "failed", ""),
            turn("user", "queued", "second"),
        ]
    );

    backend.gate.release();
    running.join().unwrap().unwrap();
    assert_eq!(*backend.gate.prompts.lock().unwrap(), ["first", "second"]);
    assert_eq!(
        message_turns(&service.get_snapshot("error-gap", Some(project)).unwrap()),
        vec![
            turn("user", "complete", "first"),
            turn("assistant", "failed", ""),
            turn("user", "complete", "second"),
            turn("assistant", "complete", "Re: second"),
        ]
    );
}

/// A mission message that arrives between a provider error and the return
/// waits in the inbox, and runs once the execution settles.
#[test]
fn a_mission_message_between_a_provider_error_and_the_return_runs_after_it() {
    use spark_workspace::inbox::{install_runtime, InboxPost, InboxRuntime};

    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let config = settings(temp.path());
    let backend = Arc::new(HeldFirstTurnBackend {
        error_first: true,
        ..HeldFirstTurnBackend::default()
    });
    install_runtime(
        &config,
        InboxRuntime {
            agent_turn_backend: backend.clone(),
            publish: Arc::new(|_| {}),
        },
    );
    let missions = crate::missions(config.clone());
    let mission = missions
        .create(
            &project,
            serde_json::from_value(json!({"fields": {"title": "Error gap"}})).unwrap(),
        )
        .unwrap();
    let service = crate::conversations(config.clone());
    missions.start(&project, &mission.id).unwrap();
    wait_for_turn_status(&service, &mission.id, &project, "failed");

    missions
        .post_event(
            &project,
            &mission.id,
            InboxPost {
                kind: "human.message".into(),
                payload: json!({"message": "Also check the docs."}),
                ..InboxPost::default()
            },
        )
        .unwrap();
    let statuses = || {
        message_turns(&service.get_snapshot(&mission.id, Some(&project)).unwrap())
            .into_iter()
            .map(|(role, status, _)| format!("{role} {status}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(statuses(), ["user complete", "assistant failed"]);
    assert_eq!(
        spark_storage::ConversationRepository::new(crate::shared_store(&config.data_dir))
            .pending_inbox_events(&mission.id)
            .unwrap()
            .len(),
        1
    );
    backend.gate.release();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !statuses().contains(&"assistant complete".to_string()) {
        assert!(
            std::time::Instant::now() < deadline,
            "the message never ran"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let prompts = backend.gate.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(prompts[1].contains("Also check the docs."), "{prompts:?}");
}

/// Runs a Claude turn with the fake CLI in `mode`, sends "Also check the
/// docs." once it streams, and optionally stops it right after. Returns the
/// send's acceptance, the completed snapshot, and the live frames in order,
/// with a `{"type": "sent"}` marker where the message was sent.
fn claude_turn_with_mid_turn_send(
    temp: &Path,
    mode: &str,
    stop: bool,
) -> (
    spark_workspace::conversations::TurnAcceptance,
    Value,
    Vec<Value>,
) {
    let project = temp.canonicalize().unwrap().to_string_lossy().to_string();
    let _bin = EnvVarGuard::set(
        "SPARK_CLAUDE_CODE_BIN",
        env!("CARGO_BIN_EXE_spark-workspace-fake-claude-code"),
    );
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", mode);
    let service = crate::conversations(settings(temp));
    // The composer sends settings only when they change.
    let request = |message: &str| ConversationTurnRequest {
        project_path: project.clone(),
        message: message.into(),
        ..ConversationTurnRequest::default()
    };
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let (prepared, _) = service
        .send_turn(
            "mid-turn",
            ConversationTurnRequest {
                provider: Some("claude-code".into()),
                ..request("Start")
            },
        )
        .unwrap();
    let (streaming, streamed) = std::sync::mpsc::channel();
    let live = payloads.clone();
    let running = service.clone();
    let handle = std::thread::spawn(move || {
        running.complete_started_turn_with_progress_payloads(prepared.unwrap(), move |payload| {
            let streaming_text = payload["delta_kind"] == "segment_text";
            live.lock().unwrap().push(payload);
            if streaming_text {
                let _ = streaming.send(());
            }
        })
    });
    streamed.recv_timeout(Duration::from_secs(120)).unwrap();
    payloads.lock().unwrap().push(json!({"type": "sent"}));
    let (none, sent) = service
        .send_turn("mid-turn", request("Also check the docs."))
        .expect("no 409 while the turn streams");
    assert!(none.is_none());
    if stop {
        assert!(service.interrupt_turn("mid-turn", &project).unwrap());
    }
    handle.join().unwrap().unwrap();
    let completed = service.get_snapshot("mid-turn", Some(&project)).unwrap();
    let payloads = payloads.lock().unwrap().clone();
    (sent, completed, payloads)
}

fn user_message_segment(snapshot: &Value) -> Value {
    snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|segment| segment["kind"] == "user_message")
        .unwrap_or_else(|| panic!("user_message segment in {snapshot}"))
        .clone()
}

#[test]
fn a_send_while_a_claude_turn_streams_is_delivered_into_that_turn() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let (sent, completed, _) =
        claude_turn_with_mid_turn_send(temp.path(), "mid-turn-message", false);

    // The message joined the turn instead of queuing behind it.
    assert_ne!(sent.status, "queued");
    assert!(sent.assistant_turn_id.is_some());
    assert!(message_turns(&completed)
        .iter()
        .all(|(_, status, _)| status != "queued"));
    let delivered = user_message_segment(&completed);
    assert_eq!(delivered["role"], "user");
    assert_eq!(delivered["status"], "complete");
    assert_eq!(delivered["content"], "Also check the docs.");
    // One turn: the message is part of it, not a queued turn of its own.
    assert_eq!(
        message_turns(&completed)
            .iter()
            .map(|(role, status, _)| (role.as_str(), status.as_str()))
            .collect::<Vec<_>>(),
        vec![("user", "complete"), ("assistant", "complete")]
    );
    let segments = completed["segments"].as_array().unwrap();
    let order = |predicate: &dyn Fn(&Value) -> bool| {
        segments.iter().find(|segment| predicate(segment)).unwrap()["order"]
            .as_i64()
            .unwrap()
    };
    let message = order(&|segment| segment["kind"] == "user_message");
    let reply = order(&|segment| segment["content"] == "Read your note.");
    assert!(
        message < reply,
        "the reply follows the message: {completed}"
    );
    assert_eq!(
        turn_with(&completed, "assistant", 0)["content"],
        "Read your note."
    );
}

#[test]
fn a_message_the_stopped_claude_turn_never_read_is_shown_not_sent() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let (_, completed, _) = claude_turn_with_mid_turn_send(temp.path(), "mid-turn-stop", true);

    let segment = user_message_segment(&completed);
    assert_eq!(segment["status"], "failed");
    assert!(segment["error"].as_str().unwrap().starts_with("Not sent"));
}

#[test]
fn output_streamed_the_moment_a_message_is_accepted_follows_it_live_and_persisted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let (_, completed, payloads) =
        claude_turn_with_mid_turn_send(temp.path(), "mid-turn-continue", false);

    // The block streaming when the message went in continues after it.
    let mut segments = completed["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|segment| {
            matches!(
                segment["kind"].as_str(),
                Some("assistant_message" | "user_message")
            )
        })
        .collect::<Vec<_>>();
    segments.sort_by_key(|segment| segment["order"].as_i64());
    let shown = segments
        .iter()
        .map(|segment| {
            (
                segment["kind"].as_str().unwrap(),
                segment["status"].as_str().unwrap(),
                segment["content"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        vec![
            ("assistant_message", "complete", "Working"),
            ("user_message", "complete", "Also check the docs."),
            ("assistant_message", "complete", "still."),
            ("assistant_message", "complete", "Read your note."),
        ],
        "{completed}"
    );

    // Live, text streamed after the message is sent goes to new segments,
    // and the earlier part never takes the later text.
    let ids = segments
        .iter()
        .map(|segment| segment["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    let first_seen = |id: &str| {
        payloads
            .iter()
            .position(|payload| payload["segment_id"] == id || payload["type"] == id)
            .unwrap_or_else(|| panic!("{id} live in {payloads:#?}"))
    };
    assert!(first_seen(ids[0]) < first_seen("sent"));
    assert!(first_seen("sent") < first_seen(ids[2]));
    assert!(payloads
        .iter()
        .filter(|payload| payload["segment_id"] == ids[0])
        .all(|payload| !payload["text"].as_str().unwrap().contains("still")));
}

/// A turn that takes messages while it runs: it says "Looking.", waits to
/// be released, then reports `not_sent` messages as never used and answers.
/// Every turn's history is recorded.
#[derive(Default)]
struct LiveMessageBackend {
    sink: Mutex<Option<AgentTurnEventSink>>,
    delivered: Mutex<Vec<(String, String)>>,
    histories: Mutex<Vec<Vec<(String, String)>>>,
    released: (Mutex<bool>, Condvar),
    /// Ends the turn with a plan instead of a final answer, as plan mode does.
    plan: bool,
}

impl AgentTurnBackend for LiveMessageBackend {
    fn send_message(
        &self,
        _project_path: &str,
        _conversation_id: &str,
        message_id: &str,
        message: &str,
    ) -> bool {
        let Some(sink) = self.sink.lock().unwrap().clone() else {
            return false;
        };
        sink(TurnStreamEvent {
            kind: TurnStreamEventKind::Other("user_message_delivered".into()),
            source: TurnStreamSource {
                item_id: Some(message_id.to_string()),
                ..TurnStreamSource::default()
            },
            message: Some(message.to_string()),
            ..content_delta("assistant", "", "t1", "unused")
        });
        self.delivered
            .lock()
            .unwrap()
            .push((message_id.to_string(), message.to_string()));
        true
    }

    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.run_turn_with_event_sink(request, None)
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        self.histories.lock().unwrap().push(
            request
                .history
                .iter()
                .map(|turn| match turn {
                    spark_agent_adapter::HistoryTurn::User(turn) => {
                        ("user".to_string(), turn.content.text())
                    }
                    spark_agent_adapter::HistoryTurn::Assistant(turn) => {
                        ("assistant".to_string(), turn.content.text())
                    }
                    other => panic!("unexpected history turn {other:?}"),
                })
                .collect(),
        );
        if request.prompt != "Start" {
            return Ok(AgentTurnOutput {
                final_assistant_text: Some("Again.".to_string()),
                ..AgentTurnOutput::default()
            });
        }
        // Like a real backend: every event goes through the sink as it happens,
        // and the returned output repeats them all.
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let inner = event_sink.unwrap();
        let sink: AgentTurnEventSink = {
            let recorded = recorded.clone();
            Arc::new(move |event: TurnStreamEvent| {
                recorded.lock().unwrap().push(event.clone());
                inner(event);
            })
        };
        sink(content_delta("reasoning", "Checking.", "t1", "r1"));
        sink(content_completed("reasoning", "Checking.", "t1", "r1"));
        sink(content_delta("assistant", "Looking.", "t1", "a1"));
        sink(content_completed("assistant", "Looking.", "t1", "a1"));
        *self.sink.lock().unwrap() = Some(sink.clone());
        let mut released = self.released.0.lock().unwrap();
        while !*released {
            released = self.released.1.wait(released).unwrap();
        }
        drop(released);
        self.sink.lock().unwrap().take();
        for (message_id, message) in self.delivered.lock().unwrap().iter() {
            if message == "Never mind." {
                sink(TurnStreamEvent {
                    kind: TurnStreamEventKind::Other("user_message_not_delivered".into()),
                    source: TurnStreamSource {
                        item_id: Some(message_id.clone()),
                        ..TurnStreamSource::default()
                    },
                    message: Some("Not sent.".to_string()),
                    ..content_delta("assistant", "", "t1", "unused")
                });
            }
        }
        let channel = if self.plan { "plan" } else { "assistant" };
        sink(content_completed(
            channel,
            "Done with the docs.",
            "t1",
            "a2",
        ));
        sink(TurnStreamEvent {
            kind: TurnStreamEventKind::TurnCompleted,
            ..content_delta("assistant", "", "t1", "unused")
        });
        let events = recorded.lock().unwrap().clone();
        Ok(AgentTurnOutput {
            final_assistant_text: (!self.plan).then(|| "Done with the docs.".to_string()),
            events,
            ..AgentTurnOutput::default()
        })
    }
}

/// Streams part of an answer and never completes it.
struct PartialAnswerBackend {
    fails: bool,
}

impl AgentTurnBackend for PartialAnswerBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.run_turn_with_event_sink(request, None)
    }

    fn run_turn_with_event_sink(
        &self,
        _request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let sink = event_sink.unwrap();
        sink(content_delta("assistant", "Half an answer", "t1", "a1"));
        if self.fails {
            return Err(AgentError {
                message: "Provider went away.".to_string(),
                retryable: false,
                raw: None,
            });
        }
        sink(TurnStreamEvent {
            kind: TurnStreamEventKind::TurnCompleted,
            ..content_delta("assistant", "", "t1", "unused")
        });
        Ok(AgentTurnOutput {
            final_assistant_text: Some("Half an answer".to_string()),
            ..AgentTurnOutput::default()
        })
    }
}

#[test]
fn a_turn_keeps_the_segment_it_was_streaming_when_it_ended() {
    for fails in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let backend = Arc::new(PartialAnswerBackend { fails });
        let service = crate::conversations_with_backend(settings(temp.path()), backend);
        let project = "/projects/partial";
        let (prepared, _) = service
            .send_turn(
                "partial",
                ConversationTurnRequest {
                    project_path: project.to_string(),
                    message: "Start".to_string(),
                    ..ConversationTurnRequest::default()
                },
            )
            .unwrap();
        service
            .complete_started_turn_with_progress_payloads(prepared.unwrap(), |_| {})
            .unwrap();

        let snapshot = service.get_snapshot("partial", Some(project)).unwrap();
        let contents = snapshot["segments"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|segment| segment["kind"] == "assistant_message")
            .map(|segment| segment["content"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(contents, vec!["Half an answer"], "fails: {fails}");
        assert_eq!(
            snapshot["turns"][1]["status"],
            if fails { "failed" } else { "complete" },
            "fails: {fails}"
        );
    }
}

#[test]
fn a_backend_reading_history_sees_each_message_delivered_into_a_turn_once_in_order() {
    // A plan-mode turn ends with a plan rather than a final answer; the plan
    // must stay in history alongside the delivered message.
    for plan in [false, true] {
        assert_delivered_history(plan);
    }
}

fn assert_delivered_history(plan: bool) {
    let temp = tempfile::tempdir().unwrap();
    let backend = Arc::new(LiveMessageBackend {
        plan,
        ..LiveMessageBackend::default()
    });
    let service = crate::conversations_with_backend(settings(temp.path()), backend.clone());
    let project = "/projects/history";
    let request = |message: &str| ConversationTurnRequest {
        project_path: project.to_string(),
        message: message.to_string(),
        ..ConversationTurnRequest::default()
    };
    let (prepared, _) = service.send_turn("history", request("Start")).unwrap();
    let running = service.clone();
    let turn = std::thread::spawn(move || {
        running.complete_started_turn_with_progress_payloads(prepared.unwrap(), |_| {})
    });
    let waited = std::time::Instant::now();
    while backend.sink.lock().unwrap().is_none() {
        assert!(waited.elapsed() < Duration::from_secs(10), "turn running");
        std::thread::sleep(Duration::from_millis(5));
    }
    send(&service, "history", project, "Use the docs.");
    send(&service, "history", project, "Never mind.");
    *backend.released.0.lock().unwrap() = true;
    backend.released.1.notify_all();
    turn.join().unwrap().unwrap();

    // Each block is stored once, where it happened.
    let snapshot = service.get_snapshot("history", Some(project)).unwrap();
    let running_turn = &snapshot["turns"][1]["id"];
    let mut segments = snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|segment| &segment["turn_id"] == running_turn && segment["kind"] != "agent_event")
        .collect::<Vec<_>>();
    segments.sort_by_key(|segment| segment["order"].as_i64());
    let segments = segments
        .iter()
        .map(|segment| {
            (
                segment["kind"].as_str().unwrap(),
                segment["content"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        segments,
        vec![
            ("reasoning", "Checking."),
            ("assistant_message", "Looking."),
            ("user_message", "Use the docs."),
            ("user_message", "Never mind."),
            (
                if plan { "plan" } else { "assistant_message" },
                "Done with the docs."
            ),
        ],
        "plan: {plan}"
    );

    let (next, _) = service.send_turn("history", request("Next")).unwrap();
    service
        .complete_started_turn_with_progress_payloads(next.unwrap(), |_| {})
        .unwrap();

    let pair = |role: &str, text: &str| (role.to_string(), text.to_string());
    assert_eq!(
        backend.histories.lock().unwrap().last().unwrap(),
        &vec![
            pair("user", "Start"),
            pair("assistant", "Looking."),
            pair("user", "Use the docs."),
            pair("assistant", "Done with the docs."),
        ],
        "plan: {plan}"
    );
}

#[test]
fn a_message_with_attachments_sent_mid_turn_is_delivered_into_the_running_turn_for_every_provider()
{
    for provider in ["codex", "claude-code", "openai"] {
        let temp = tempfile::tempdir().unwrap();
        let backend = Arc::new(LiveMessageBackend::default());
        let service = crate::conversations_with_backend(settings(temp.path()), backend.clone());
        let project = "/projects/attach";
        let request = |message: &str, attachment_ids: Vec<String>| ConversationTurnRequest {
            project_path: project.to_string(),
            message: message.to_string(),
            provider: Some(provider.to_string()),
            attachment_ids,
            ..ConversationTurnRequest::default()
        };
        let (prepared, _) = service
            .send_turn("attach", request("Start", Vec::new()))
            .unwrap();
        let running = service.clone();
        let turn = std::thread::spawn(move || {
            running.complete_started_turn_with_progress_payloads(prepared.unwrap(), |_| {})
        });
        let waited = std::time::Instant::now();
        while backend.sink.lock().unwrap().is_none() {
            assert!(waited.elapsed() < Duration::from_secs(10), "turn running");
            std::thread::sleep(Duration::from_millis(5));
        }

        let notes = service
            .upload_attachment("attach", project, "notes.txt", b"notes")
            .unwrap();
        // As the composer sends it: the conversation keeps its provider.
        let (prepared, acceptance) = service
            .send_turn(
                "attach",
                ConversationTurnRequest {
                    provider: None,
                    ..request("Read this.", vec![notes.id.clone()])
                },
            )
            .unwrap();

        assert!(prepared.is_none(), "{provider}");
        assert_ne!(acceptance.status, "queued", "{provider}");
        let snapshot = service.get_snapshot("attach", Some(project)).unwrap();
        assert!(
            !snapshot["turns"]
                .as_array()
                .unwrap()
                .iter()
                .any(|turn| turn["status"] == "queued"),
            "{provider}: {snapshot}"
        );
        let delivered = backend.delivered.lock().unwrap().clone();
        assert_eq!(delivered.len(), 1, "{provider}");
        let message = &delivered[0].1;
        assert!(
            message.starts_with("Read this.\n\nAttached files"),
            "{provider}: {message}"
        );
        assert!(
            message.contains("- notes.txt (text/plain, 5 bytes): /"),
            "{provider}: {message}"
        );
        *backend.released.0.lock().unwrap() = true;
        backend.released.1.notify_all();
        turn.join().unwrap().unwrap();
    }
}

fn scripted_answers(answers: &[&str]) -> ScriptedAgentTurnBackend {
    ScriptedAgentTurnBackend::new(
        answers
            .iter()
            .map(|answer| AgentTurnOutput {
                final_assistant_text: Some(answer.to_string()),
                ..AgentTurnOutput::default()
            })
            .collect(),
    )
}

fn message(project_path: &str, message: &str) -> ConversationTurnRequest {
    ConversationTurnRequest {
        project_path: project_path.to_string(),
        message: message.to_string(),
        ..ConversationTurnRequest::default()
    }
}

#[test]
fn a_history_failure_after_acceptance_leaves_the_accepted_turn_visibly_failed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = scripted_answers(&["Read them.", "must not run"]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(backend));
    let (id, project) = ("conversation-history-failure", "/projects/history-failure");
    let upload = service
        .upload_attachment(id, project, "notes.txt", b"notes")
        .expect("upload");
    service
        .execute_turn(
            id,
            ConversationTurnRequest {
                attachment_ids: vec![upload.id.clone()],
                ..message(project, "Read the notes.")
            },
        )
        .expect("first turn");
    // The earlier message's attachment bytes disappear.
    let (_, blob) = ConversationRepository::new(crate::shared_store(&settings.data_dir))
        .read_attachment(id, project, &upload.id)
        .expect("read attachment")
        .expect("attachment");
    fs::remove_file(blob).expect("remove blob");

    let (prepared, acceptance) = service
        .send_turn(id, message(project, "Follow up."))
        .expect("the send is accepted without building history");
    let prepared = prepared.expect("a started turn");
    assert_eq!(acceptance.status, "accepted");
    assert_eq!(
        acceptance.assistant_turn_id.as_deref(),
        Some(prepared.assistant_turn_id.as_str())
    );
    assert_eq!(requests.lock().unwrap().len(), 1);

    let error = service
        .complete_started_turn_with_progress_payloads(prepared.clone(), |_| {})
        .expect_err("the history cannot be built");
    assert!(matches!(error, WorkspaceError::Conflict(_)), "{error}");
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "no provider turn starts without its history"
    );
    let snapshot = service.get_snapshot(id, Some(project)).expect("snapshot");
    let assistant = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant["status"], "failed", "{assistant}");
    assert!(
        assistant["error"]
            .as_str()
            .is_some_and(|error| error.contains("cannot be sent")),
        "{assistant}"
    );
}

#[test]
fn a_send_is_accepted_before_any_history_is_built_and_carries_only_its_rows() {
    use spark_storage::conversation::EntityKind;
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = scripted_answers(&["First answer.", "Second answer."]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings(temp.path()), Arc::new(backend));
    let (id, project) = ("conversation-accept-rows", "/projects/accept-rows");
    let before = service
        .execute_turn(id, message(project, "First question."))
        .expect("first turn");
    let earlier: Vec<String> = before["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .map(|turn| turn["id"].as_str().expect("id").to_string())
        .collect();
    assert!(!earlier.is_empty());

    let (prepared, acceptance) = service
        .send_turn(id, message(project, "Second question."))
        .expect("send");
    let prepared = prepared.expect("a started turn");
    // Accepted before the backend is asked to run anything.
    assert_eq!(requests.lock().unwrap().len(), 1);
    let mut changed: Vec<_> = acceptance
        .changes
        .rows
        .iter()
        .map(|row| (row.kind, row.id.clone()))
        .collect();
    changed.sort_by(|a, b| a.1.cmp(&b.1));
    let mut expected = vec![
        (EntityKind::Conversation, id.to_string()),
        (EntityKind::Turn, prepared.user_turn_id.clone()),
        (EntityKind::Turn, prepared.assistant_turn_id.clone()),
    ];
    expected.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(changed, expected);
    assert!(acceptance
        .changes
        .rows
        .iter()
        .all(|row| !earlier.contains(&row.id)
            && row
                .turn_id
                .as_ref()
                .is_none_or(|turn| !earlier.contains(turn))));

    service
        .complete_started_turn_with_progress_payloads(prepared, |_| {})
        .expect("second turn");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].prompt, "Second question.");
    assert_eq!(
        requests[1].history.len(),
        2,
        "history is built when the turn runs"
    );
}

#[test]
fn a_duplicate_send_returns_the_original_acceptance_and_starts_no_second_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = scripted_answers(&["Answer.", "must not run"]);
    let requests = backend.requests();
    let service = crate::conversations_with_backend(settings(temp.path()), Arc::new(backend));
    let (id, project) = ("conversation-duplicate", "/projects/duplicate");
    let send = |text: &str| {
        service.send_turn(
            id,
            ConversationTurnRequest {
                client_turn_id: Some("turn-dup1".to_string()),
                ..message(project, text)
            },
        )
    };
    let (prepared, original) = send("Do it once.").expect("send");
    let prepared = prepared.expect("a started turn");
    assert_eq!(original.user_turn_id, "turn-dup1");
    assert_eq!(
        original.assistant_turn_id.as_deref(),
        Some(prepared.assistant_turn_id.as_str())
    );

    let assert_original = |stage: &str| {
        let (started, resent) = send("Do it once.").expect("resend");
        assert!(started.is_none(), "{stage}: a resend started a turn");
        assert_eq!(resent.user_turn_id, original.user_turn_id, "{stage}");
        assert_eq!(
            resent.assistant_turn_id, original.assistant_turn_id,
            "{stage}"
        );
        assert_eq!(resent.status, "accepted", "{stage}");
    };
    // Before and after the turn runs.
    assert_original("accepted");
    service
        .complete_started_turn_with_progress_payloads(prepared, |_| {})
        .expect("turn");
    assert_original("complete");
    assert_eq!(requests.lock().unwrap().len(), 1, "one provider turn");

    let conflict = send("A different message.").expect_err("reused id");
    assert!(
        matches!(conflict, WorkspaceError::TurnIdConflict(_)),
        "{conflict}"
    );
    let snapshot = service.get_snapshot(id, Some(project)).expect("snapshot");
    let user_turns = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .filter(|turn| turn["role"] == "user")
        .count();
    assert_eq!(user_turns, 1, "{snapshot}");
    assert_eq!(requests.lock().unwrap().len(), 1);
}
