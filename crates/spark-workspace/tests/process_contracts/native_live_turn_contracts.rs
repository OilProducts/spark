//! Chat with a native-provider turn: messages sent while it runs join it,
//! and Stop completes it.

use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use spark_agent_adapter::RustLlmAgentTurnBackend;
use spark_workspace::{ConversationTurnRequest, WorkspaceConversationService};
use unified_llm_adapter::{
    stream_events, AdapterError, Client, FinishReason, Message, ProviderAdapter, Request, Response,
    StreamEvent, StreamEventType, StreamEvents, ToolCall, Usage,
};

use super::conversation_event_normalization_contracts::settings;

const PROJECT: &str = "/projects/native-live";

type Script = dyn Fn(usize, &Request) -> Result<Vec<StreamEvent>, AdapterError> + Send + Sync;

/// An `openai` model whose streamed answers come from the script, by call index.
struct ScriptedModel {
    calls: Arc<Mutex<Vec<Request>>>,
    script: Box<Script>,
}

impl ProviderAdapter for ScriptedModel {
    fn name(&self) -> &str {
        "openai"
    }

    fn complete(&self, _request: Request) -> Result<Response, AdapterError> {
        unreachable!("native chat streams")
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        let index = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(request.clone());
            calls.len() - 1
        };
        (self.script)(index, &request).map(|events| stream_events(events.into_iter().map(Ok)))
    }
}

fn answer(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::text_delta(text),
        StreamEvent::finish(FinishReason::Stop, None),
    ]
}

type Service = Arc<OnceLock<WorkspaceConversationService>>;

/// The service runs a native chat turn whose model calls `script`, which
/// can reach the service while the turn runs.
fn native_chat(
    script: impl Fn(&WorkspaceConversationService, usize, &Request) -> Result<Vec<StreamEvent>, AdapterError>
        + Send
        + Sync
        + 'static,
) -> (tempfile::TempDir, Service, Arc<Mutex<Vec<Request>>>) {
    let temp = tempfile::tempdir().unwrap();
    let service: Service = Arc::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedModel {
        calls: calls.clone(),
        script: Box::new({
            let service = service.clone();
            move |index, request| script(service.get().unwrap(), index, request)
        }),
    });
    let client = Client::from_adapters(vec![adapter], None).unwrap();
    let _ = service.set(crate::conversations_with_backend(
        settings(temp.path()),
        Arc::new(RustLlmAgentTurnBackend::new(client)),
    ));
    (temp, service, calls)
}

fn turn(message: &str) -> ConversationTurnRequest {
    ConversationTurnRequest {
        project_path: PROJECT.to_string(),
        message: message.to_string(),
        provider: Some("openai".to_string()),
        model: Some("gpt-live".to_string()),
        ..ConversationTurnRequest::default()
    }
}

/// Sends `message` while the turn runs; it must join the turn, not queue.
fn send_mid_turn(service: &WorkspaceConversationService, conversation_id: &str, message: &str) {
    let (started, acceptance) = service.send_turn(conversation_id, turn(message)).unwrap();
    assert!(started.is_none(), "the message started a new turn");
    assert_ne!(acceptance.status, "queued");
    let snapshot = service
        .get_snapshot(conversation_id, Some(PROJECT))
        .unwrap();
    let turns = snapshot["turns"].as_array().unwrap();
    assert!(
        !turns.iter().any(|turn| turn["status"] == "queued"),
        "the message was queued: {turns:?}"
    );
}

fn user_messages(snapshot: &Value) -> Vec<&Value> {
    snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|segment| segment["kind"] == "user_message")
        .collect()
}

fn assistant_turn(snapshot: &Value) -> &Value {
    snapshot["turns"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|turn| turn["role"] == "assistant")
        .unwrap()
}

#[test]
fn a_message_sent_while_a_native_turn_runs_joins_it_in_order() {
    let (_temp, service, calls) = native_chat(|service, index, _| {
        if index == 0 {
            send_mid_turn(service, "native-steer", "Also update the docs.");
            return Ok(answer("Working."));
        }
        Ok(answer("Done."))
    });
    let service = service.get().unwrap();

    let snapshot = service
        .execute_turn("native-steer", turn("Do the work"))
        .unwrap();

    assert_eq!(
        calls.lock().unwrap()[1].messages.last(),
        Some(&Message::user("Also update the docs."))
    );
    let messages = user_messages(&snapshot);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0]["content"], "Also update the docs.");
    assert_eq!(messages[0]["status"], "complete");
    let order = |segment: &Value| segment["order"].as_i64().unwrap();
    let done = snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|segment| segment["content"] == "Done.")
        .expect("answer segment");
    assert!(order(messages[0]) < order(done), "{snapshot}");
    let assistant = assistant_turn(&snapshot);
    assert_eq!(assistant["status"], "complete");
    assert_eq!(assistant["content"], "Done.");
}

/// A mid-turn message is durably accepted, under its client turn id, before
/// it is delivered. A resend is neither delivered again nor started as a
/// turn: before the turn reports it, after it does, and after the turn ends.
/// Reusing the id for a different message is a conflict.
#[test]
fn a_resent_mid_turn_message_is_delivered_once() {
    let resend = |service: &WorkspaceConversationService| {
        let (started, acceptance) = service
            .send_turn(
                "native-resend",
                ConversationTurnRequest {
                    client_turn_id: Some("turn-steer1".to_string()),
                    ..turn("Also update the docs.")
                },
            )
            .unwrap();
        assert!(started.is_none(), "the resend started a turn");
        // Every resend answers with the original acceptance's ids.
        assert_eq!(acceptance.user_turn_id, "turn-steer1");
        assert_ne!(acceptance.status, "queued");
        let snapshot = service
            .get_snapshot("native-resend", Some(PROJECT))
            .unwrap();
        (acceptance, snapshot)
    };
    let reported = |snapshot: &Value| {
        user_messages(snapshot)
            .iter()
            .any(|segment| segment["id"] == "segment-user-message-turn-steer1")
    };
    let (_temp, service, calls) = native_chat(move |service, index, _| {
        if index == 0 {
            let (first_acceptance, first) = resend(service);
            assert!(reported(&first), "accepted before delivery: {first}");
            // Before the turn reports the delivery.
            let (again_acceptance, again) = resend(service);
            assert!(reported(&again));
            assert_eq!(
                again_acceptance.assistant_turn_id,
                first_acceptance.assistant_turn_id
            );
            let conflict = service
                .send_turn(
                    "native-resend",
                    ConversationTurnRequest {
                        client_turn_id: Some("turn-steer1".to_string()),
                        ..turn("A different message.")
                    },
                )
                .unwrap_err();
            assert!(
                matches!(conflict, spark_workspace::WorkspaceError::TurnIdConflict(_)),
                "{conflict}"
            );
            return Ok(answer("Working."));
        }
        // The turn reports the message before the model reads it.
        let snapshot = service
            .get_snapshot("native-resend", Some(PROJECT))
            .unwrap();
        assert!(reported(&snapshot), "{snapshot}");
        resend(service);
        Ok(answer("Done."))
    });
    let service = service.get().unwrap();

    service
        .execute_turn("native-resend", turn("Do the work"))
        .unwrap();
    let (_, snapshot) = resend(service);

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "a resend reached the model as a new turn");
    let delivered = calls[1]
        .messages
        .iter()
        .filter(|message| **message == Message::user("Also update the docs."))
        .count();
    assert_eq!(delivered, 1);
    assert_eq!(user_messages(&snapshot).len(), 1, "{snapshot}");
    let user_turns = snapshot["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|turn| turn["role"] == "user")
        .count();
    assert_eq!(user_turns, 1, "{snapshot}");
}

#[test]
fn stop_completes_a_native_turn_and_shows_its_unread_message_not_sent() {
    let (_temp, service, calls) = native_chat(|service, _, request| {
        send_mid_turn(service, "native-stop", "Never read.");
        assert!(service.interrupt_turn("native-stop", PROJECT).unwrap());
        let signal = request.abort_signal.clone().unwrap();
        assert!(signal.aborted());
        Err(unified_llm_adapter::abort_error("request", signal.reason()))
    });
    let service = service.get().unwrap();

    let snapshot = service
        .execute_turn("native-stop", turn("Do the work"))
        .unwrap();

    assert_eq!(calls.lock().unwrap().len(), 1);
    let assistant = assistant_turn(&snapshot);
    assert_eq!(assistant["status"], "complete", "{assistant}");
    assert_eq!(assistant["content"], "Stopped.");
    let messages = user_messages(&snapshot);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0]["content"], "Never read.");
    assert_eq!(messages[0]["status"], "failed");
    assert_eq!(
        messages[0]["error"],
        "Not sent: the turn ended before the agent read it."
    );
}

#[test]
fn a_native_turns_persisted_usage_sums_every_model_request() {
    // A tool round then an answer: 100 then 200 input tokens, 10 output each.
    let (_temp, service, calls) = native_chat(|_, index, _| {
        let (mut events, input_tokens) = if index == 0 {
            let call = ToolCall::new("call-1", "shell", json!({"command": "true"}));
            (
                vec![
                    StreamEvent {
                        tool_call: Some(call),
                        ..StreamEvent::new(StreamEventType::ToolCallEnd)
                    },
                    StreamEvent::finish(FinishReason::ToolCalls, None),
                ],
                100,
            )
        } else {
            (answer("Done."), 200)
        };
        events.last_mut().unwrap().usage = Some(Usage {
            input_tokens,
            output_tokens: 10,
            ..Usage::default()
        });
        Ok(events)
    });
    let service = service.get().unwrap();
    let mut request = turn("Do the work");
    request.model = Some("gpt-5.5".to_string());

    service.execute_turn("native-usage", request).unwrap();

    assert_eq!(calls.lock().unwrap().len(), 2);
    let snapshot = service.get_snapshot("native-usage", Some(PROJECT)).unwrap();
    let usage = &assistant_turn(&snapshot)["token_usage"];
    assert_eq!(usage["total"]["inputTokens"], json!(300), "{usage}");
    assert_eq!(usage["total"]["outputTokens"], json!(20), "{usage}");
    assert_eq!(usage["last"]["inputTokens"], json!(200), "{usage}");
    assert_eq!(usage["context"]["used_tokens"], json!(200), "{usage}");
}

/// Uploads sent with a message that joins a running turn belong to that
/// message from its acceptance on: neither deletion nor retention pruning
/// removes them, through the turn's own delivery report and after it ends.
#[test]
fn uploads_delivered_into_a_running_turn_are_owned_by_the_message() {
    let (_temp, service, _calls) = native_chat(|service, index, _| {
        if index == 0 {
            let upload = service
                .upload_attachment("native-attach", PROJECT, "notes.txt", b"delivered notes")
                .unwrap();
            let (started, _) = service
                .send_turn(
                    "native-attach",
                    ConversationTurnRequest {
                        attachment_ids: vec![upload.id.clone()],
                        ..turn("See the notes.")
                    },
                )
                .unwrap();
            assert!(started.is_none(), "the message started a new turn");
            let snapshot = service
                .get_snapshot("native-attach", Some(PROJECT))
                .unwrap();
            assert_eq!(
                user_messages(&snapshot)[0]["attachments"][0]["id"],
                json!(upload.id)
            );
            return Ok(answer("Working."));
        }
        Ok(answer("Done."))
    });
    let service = service.get().unwrap();
    let snapshot = service
        .execute_turn("native-attach", turn("Start"))
        .unwrap();
    let messages = user_messages(&snapshot);
    assert_eq!(messages.len(), 1, "{snapshot}");
    let id = messages[0]["attachments"][0]["id"]
        .as_str()
        .expect("the delivery report kept the message's uploads")
        .to_string();

    let deletion = service
        .delete_draft_attachment("native-attach", PROJECT, &id)
        .unwrap_err();
    assert!(
        matches!(deletion, spark_workspace::WorkspaceError::Conflict(_)),
        "{deletion}"
    );
    let pruned = spark_storage::ConversationRepository::new(service.store().clone())
        .prune_attachments(
            "native-attach",
            PROJECT,
            time::OffsetDateTime::now_utc() + time::Duration::hours(25),
        )
        .unwrap();
    assert!(pruned.is_empty(), "{pruned:?}");
    let (_, bytes) = service
        .read_attachment("native-attach", PROJECT, &id)
        .unwrap();
    assert_eq!(bytes, b"delivered notes");
}

const FAKE_ACP: &str = env!("CARGO_BIN_EXE_spark-workspace-fake-acp");

/// A service whose `acp:fake` harness is the fake ACP agent running
/// `script`, logging to `acp.log`, and the project it works in.
fn acp_chat(temp: &std::path::Path, mut script: Value) -> (WorkspaceConversationService, String) {
    script["log"] = json!(temp.join("acp.log"));
    script["system_prompt"] = json!(true);
    script["resume"] = json!(true);
    let settings = settings(temp);
    let project = temp.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    let agents = json!({"agents": {"acp_harnesses": {"fake": {
        "name": "Fake ACP",
        "executable": FAKE_ACP,
        "env": {"SPARK_FAKE_ACP": script.to_string()},
    }}}});
    std::fs::write(
        settings.config_dir.join("spark.toml"),
        toml::to_string(&agents).unwrap(),
    )
    .unwrap();
    let project = project.canonicalize().unwrap();
    (
        crate::conversations(settings),
        project.to_string_lossy().into_owned(),
    )
}

fn acp_turn(project: &str, message: &str) -> ConversationTurnRequest {
    ConversationTurnRequest {
        project_path: project.to_string(),
        message: message.to_string(),
        provider: Some("acp:fake".to_string()),
        ..ConversationTurnRequest::default()
    }
}

/// What the fake received for `method`, in order.
fn acp_received(temp: &std::path::Path, method: &str) -> Vec<Value> {
    std::fs::read_to_string(temp.join("acp.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["received"]["method"] == method)
        .map(|entry| entry["received"]["params"].clone())
        .collect()
}

fn acp_answer(text: &str) -> Value {
    json!([{"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}])
}

#[test]
fn acp_chat_turns_answer_from_the_harness_and_continue_its_session() {
    let temp = tempfile::tempdir().unwrap();
    let (service, project) = acp_chat(
        temp.path(),
        json!({"updates": acp_answer("Hello from ACP.")}),
    );

    let first = service
        .execute_turn("acp-chat", acp_turn(&project, "Hi"))
        .unwrap();
    assert_eq!(assistant_turn(&first)["status"], "complete", "{first}");
    assert_eq!(assistant_turn(&first)["content"], "Hello from ACP.");
    let session = spark_storage::ConversationRepository::new(crate::shared_store(
        settings(temp.path()).data_dir,
    ))
    .read_runtime_session("acp-chat", Some(&project))
    .unwrap()
    .expect("session");
    assert_eq!(session.provider, "acp:fake");
    assert_eq!(session.thread_id.as_deref(), Some("fake-session"));
    assert_eq!(session.cwd.as_deref(), Some(project.as_str()));

    let second = service
        .execute_turn("acp-chat", acp_turn(&project, "Again"))
        .unwrap();
    assert_eq!(assistant_turn(&second)["content"], "Hello from ACP.");
    // The next turn continues the session, so nothing is replayed into it,
    // and the pinned instructions travel as the system prompt both times.
    assert_eq!(acp_received(temp.path(), "session/new").len(), 1);
    let resumed = acp_received(temp.path(), "session/resume");
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0]["sessionId"], "fake-session");
    assert!(resumed[0].get("systemPrompt").is_some());
    let prompts = acp_received(temp.path(), "session/prompt");
    assert_eq!(prompts.len(), 2);
    assert_eq!(
        prompts[1]["prompt"],
        json!([{"type": "text", "text": "Again"}])
    );
}

#[test]
fn a_message_sent_during_an_acp_turn_waits_for_the_next_turn() {
    let temp = tempfile::tempdir().unwrap();
    let gate = temp.path().join("gate");
    let (service, project) = acp_chat(
        temp.path(),
        json!({"gate": gate, "updates": acp_answer("Answered.")}),
    );

    let running = {
        let (service, project) = (service.clone(), project.clone());
        std::thread::spawn(move || service.execute_turn("acp-queue", acp_turn(&project, "Work")))
    };
    let waited = std::time::Instant::now();
    while acp_received(temp.path(), "session/prompt").is_empty() {
        assert!(
            waited.elapsed() < std::time::Duration::from_secs(10),
            "prompt"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let (started, acceptance) = service
        .send_turn("acp-queue", acp_turn(&project, "Later"))
        .unwrap();
    assert!(started.is_none());
    assert_eq!(acceptance.status, "queued");
    std::fs::write(&gate, "").unwrap();
    let snapshot = running.join().unwrap().unwrap();

    // The message ran as its own prompt after the turn, never inside it.
    let prompts = acp_received(temp.path(), "session/prompt");
    let texts = prompts
        .iter()
        .map(|prompt| prompt["prompt"][0]["text"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["Work", "Later"]);
    assert!(user_messages(&snapshot).is_empty(), "{snapshot}");
    let assistants = snapshot["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|turn| turn["role"] == "assistant" && turn["status"] == "complete")
        .count();
    assert_eq!(assistants, 2, "{snapshot}");
}

#[test]
fn acp_effort_ids_reach_the_harness_exactly() {
    let temp = tempfile::tempdir().unwrap();
    let (service, project) = acp_chat(
        temp.path(),
        json!({
            "config_options": [{
                "id": "effort", "name": "Effort", "category": "thought_level", "type": "select",
                "currentValue": "Low",
                "options": [{"value": "Low", "name": "Low"}, {"value": "High", "name": "High"}],
            }],
            "updates": acp_answer("Thought it through."),
        }),
    );
    let high = Some("High".to_string());

    service
        .update_conversation_settings(
            "acp-effort",
            spark_workspace::ConversationSettingsUpdate {
                project_path: project.clone(),
                provider: Some("acp:fake".to_string()),
                reasoning_effort: high.clone(),
                expected_revision: Some("0".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let saved = service.get_snapshot("acp-effort", Some(&project)).unwrap();
    assert_eq!(saved["reasoning_effort"], "High", "{saved}");
    assert_eq!(
        saved["model_settings"]["reasoning_effort"], "High",
        "{saved}"
    );

    let snapshot = service
        .execute_turn(
            "acp-effort",
            ConversationTurnRequest {
                reasoning_effort: high,
                ..acp_turn(&project, "Think")
            },
        )
        .unwrap();
    assert_eq!(
        assistant_turn(&snapshot)["status"],
        "complete",
        "{snapshot}"
    );
    let set = acp_received(temp.path(), "session/set_config_option");
    assert_eq!(
        (set[0]["configId"].clone(), set[0]["value"].clone()),
        (json!("effort"), json!("High"))
    );
}

/// Files under `root`, other than `except`, whose bytes contain `needle`.
fn files_containing(
    root: &std::path::Path,
    needle: &str,
    except: &[&str],
) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_containing(&path, needle, except));
        } else if !except.iter().any(|name| path.ends_with(name))
            && std::fs::read(&path)
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
        {
            found.push(path);
        }
    }
    found
}

#[test]
fn acp_registration_secrets_reach_the_harness_but_not_chat_history() {
    let temp = tempfile::tempdir().unwrap();
    let (service, project) = acp_chat(
        temp.path(),
        json!({"log_env": ["AGENT_TOKEN", "FAKE_CHANNEL"], "updates": acp_answer("Done.")}),
    );
    let config = settings(temp.path()).config_dir.join("spark.toml");
    let mut document: toml::Value =
        toml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let fake = document["agents"]["acp_harnesses"]["fake"]
        .as_table_mut()
        .unwrap();
    fake["env"]
        .as_table_mut()
        .unwrap()
        .insert("AGENT_TOKEN".into(), "fake-secret-sentinel".into());
    // Declared channels may hold secrets too, beside the instructions.
    fake.insert(
        "system_instructions".into(),
        toml::Value::try_from(json!({"env": {"FAKE_CHANNEL": {"json": {
            "api_key": "fake-channel-secret-sentinel", "system": "{{instructions}}",
        }}}}))
        .unwrap(),
    );
    document["agents"]["acp_harnesses"]
        .as_table_mut()
        .unwrap()
        .insert(
            "other".into(),
            toml::Value::try_from(json!({
                "name": "Other", "executable": "other-agent",
                "env": {"OTHER_TOKEN": "other-secret-sentinel"},
                "system_instructions": {"env": {"OTHER_PROMPT": {
                    "text": "other-channel-secret-sentinel {{instructions}}",
                }}},
            }))
            .unwrap(),
        );
    std::fs::write(&config, toml::to_string(&document).unwrap()).unwrap();

    let snapshot = service
        .execute_turn("acp-secrets", acp_turn(&project, "Hi"))
        .unwrap();
    assert_eq!(
        assistant_turn(&snapshot)["status"],
        "complete",
        "{snapshot}"
    );
    let launch: Value = serde_json::from_str(
        std::fs::read_to_string(temp.path().join("acp.log"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(launch["env"]["AGENT_TOKEN"], "fake-secret-sentinel");
    let channel: Value =
        serde_json::from_str(launch["env"]["FAKE_CHANNEL"].as_str().unwrap()).unwrap();
    assert_eq!(channel["api_key"], "fake-channel-secret-sentinel");
    assert!(!channel["system"].as_str().unwrap().is_empty(), "{channel}");
    let reread = service.get_snapshot("acp-secrets", Some(&project)).unwrap();
    for response in [&snapshot, &reread] {
        assert!(
            !response.to_string().contains("secret-sentinel"),
            "{response}"
        );
    }
    assert_eq!(
        files_containing(temp.path(), "secret-sentinel", &["spark.toml", "acp.log"]),
        Vec::<std::path::PathBuf>::new()
    );
}

/// Spark restarted under other inherited values resets an ACP chat's
/// session, explicitly; under the same values it continues it.
#[test]
fn a_restart_with_a_changed_inherited_environment_resets_acp_continuity() {
    // No other test changes the environment meanwhile.
    let _env = super::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    const PROBE: &str = "SPARK_ACP_RESTART_PROBE";
    let temp = tempfile::tempdir().unwrap();
    let (_, project) = acp_chat(temp.path(), json!({"updates": acp_answer("Hi.")}));
    let config = settings(temp.path()).config_dir.join("spark.toml");
    let mut document: toml::Value =
        toml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    document["agents"]
        .as_table_mut()
        .unwrap()
        .insert("environment_inheritance".into(), "inherit_all".into());
    std::fs::write(&config, toml::to_string(&document).unwrap()).unwrap();
    let restart_and_chat = |value: &str| {
        std::env::set_var(PROBE, value);
        // A new service over the same stores, as after a restart.
        let snapshot = crate::conversations(settings(temp.path()))
            .execute_turn("acp-restart", acp_turn(&project, "Hi"))
            .unwrap();
        assert_eq!(
            assistant_turn(&snapshot)["status"],
            "complete",
            "{snapshot}"
        );
        snapshot
    };

    restart_and_chat("inherited-one");
    restart_and_chat("inherited-one");
    let changed = restart_and_chat("inherited-two");
    std::env::remove_var(PROBE);
    assert_eq!(acp_received(temp.path(), "session/resume").len(), 1);
    assert_eq!(acp_received(temp.path(), "session/new").len(), 2);
    assert!(
        changed.to_string().contains("continuity_reset"),
        "{changed}"
    );
    // Only a hash of the inherited values is kept.
    assert_eq!(
        files_containing(temp.path(), "inherited-", &[]),
        Vec::<std::path::PathBuf>::new()
    );
}

fn acp_chunk(message_id: &str, text: &str) -> Value {
    json!({"sessionUpdate": "agent_message_chunk", "messageId": message_id,
        "content": {"type": "text", "text": text}})
}

fn segments_of_kind<'a>(snapshot: &'a Value, kind: &str) -> Vec<&'a Value> {
    snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|segment| segment["kind"] == kind)
        .collect()
}

#[test]
fn an_acp_message_keeps_its_text_around_the_tool_calls_inside_it() {
    let temp = tempfile::tempdir().unwrap();
    let (service, project) = acp_chat(
        temp.path(),
        json!({"updates": [
            acp_chunk("m0", "Let me check."),
            {"sessionUpdate": "tool_call", "toolCallId": "look", "title": "Look", "status": "pending"},
            {"sessionUpdate": "tool_call_update", "toolCallId": "look", "status": "completed"},
            acp_chunk("m1", "The complete answer is: "),
            {"sessionUpdate": "tool_call", "toolCallId": "bg", "title": "Background", "status": "in_progress"},
            {"sessionUpdate": "tool_call_update", "toolCallId": "bg", "status": "completed"},
            acp_chunk("m1", "42."),
        ]}),
    );

    service
        .execute_turn("acp-message", acp_turn(&project, "Answer"))
        .unwrap();

    // A new message id starts a message; the same id continues it around tools.
    let snapshot = service.get_snapshot("acp-message", Some(&project)).unwrap();
    assert_eq!(
        assistant_turn(&snapshot)["content"],
        "The complete answer is: 42.",
        "{snapshot}"
    );
    let messages = segments_of_kind(&snapshot, "assistant_message")
        .iter()
        .map(|segment| segment["content"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(messages, ["Let me check.", "The complete answer is: 42."]);
}

#[test]
fn acp_tool_results_given_only_as_content_reach_chat_history() {
    let temp = tempfile::tempdir().unwrap();
    let text = |text: &str| json!([{"type": "content", "content": {"type": "text", "text": text}}]);
    let (service, project) = acp_chat(
        temp.path(),
        json!({"updates": [
            {"sessionUpdate": "tool_call", "toolCallId": "read", "title": "Read", "status": "pending"},
            {"sessionUpdate": "tool_call_update", "toolCallId": "read", "status": "in_progress",
             "content": text("READ-PROGRESS")},
            {"sessionUpdate": "tool_call_update", "toolCallId": "read", "status": "completed",
             "content": text("READ-RESULT")},
            {"sessionUpdate": "tool_call", "toolCallId": "write", "title": "Write", "status": "pending"},
            {"sessionUpdate": "tool_call_update", "toolCallId": "write", "status": "failed",
             "content": text("WRITE-DENIED")},
            acp_chunk("m0", "Done."),
        ]}),
    );

    service
        .execute_turn("acp-tools", acp_turn(&project, "Work"))
        .unwrap();

    let snapshot = service.get_snapshot("acp-tools", Some(&project)).unwrap();
    let tools = segments_of_kind(&snapshot, "tool_call")
        .iter()
        .map(|segment| {
            let tool = &segment["tool_call"];
            (
                tool["id"].clone(),
                tool["status"].clone(),
                tool["output"].clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tools,
        [
            (json!("read"), json!("completed"), json!("READ-RESULT")),
            (json!("write"), json!("failed"), json!("WRITE-DENIED")),
        ],
        "{snapshot}"
    );
}

/// Discovery probes each registered harness in the work area without a
/// prompt, reports raw config options and separate setup and capability
/// states, and reuses its result until the agents settings change.
#[test]
fn acp_discovery_probes_harnesses_without_prompting_and_caches_by_generation() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("acp.log");
    let options = json!([
        {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "fast",
         "options": [
            {"group": "quick", "name": "Quick", "options": [{"value": "fast", "name": "Fast"}]},
            {"group": "deep", "name": "Deep", "options": [{"value": "Deep/1", "name": "Deep One"}]},
         ]},
        {"id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": "Med",
         "options": [{"value": "Low", "name": "Low"}, {"value": "Med", "name": "Medium"}]},
        {"id": "approval", "name": "Approval", "category": "mode", "type": "select", "currentValue": "ask",
         "options": [{"value": "ask", "name": "Ask"}, {"value": "plan", "name": "Plan"}]},
        {"id": "verbose", "name": "Verbose", "type": "boolean", "currentValue": false},
        {"id": "dial", "name": "Dial", "type": "slider", "currentValue": 3},
    ]);
    let modes =
        json!({"currentModeId": "code", "availableModes": [{"id": "code", "name": "Code"}]});
    let harness = |name: &str, executable: &str, script: Value| {
        let mut script = script;
        script["log"] = json!(log);
        json!({"name": name, "executable": executable, "env": {
            "SPARK_FAKE_ACP": script.to_string(), "AGENT_TOKEN": "discovery-secret-sentinel",
        }})
    };
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    let agents = json!({"agents": {
        "native": {"claude_binary": "/nonexistent/claude-for-tests"},
        "acp_harnesses": {
            "fake": harness("Fake ACP", FAKE_ACP, json!({"system_prompt": true, "resume": true, "config_options": options})),
            "plain": harness("Plain", FAKE_ACP, json!({"modes": modes})),
            "locked": harness("Locked", FAKE_ACP, json!({"new_error": {"code": -32000, "message": "Authentication required"}})),
            "missing": harness("Missing", "/nonexistent/acp-agent", json!({})),
        },
    }});
    std::fs::write(
        settings.config_dir.join("spark.toml"),
        toml::to_string(&agents).unwrap(),
    )
    .unwrap();
    let project = project.canonicalize().unwrap();
    let discover = || {
        spark_workspace::models::chat_models_with_codex_result(&settings, &project, Ok(vec![]))
            .unwrap()
    };
    let launches = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|entry| entry.get("pid").is_some())
            .collect::<Vec<_>>()
    };

    let response = discover();
    let providers = &response["providers"];
    let fake = &providers["acp:fake"];
    assert_eq!(fake["status"], "available", "{fake}");
    assert_eq!(fake["label"], "Fake ACP");
    assert_eq!(fake["setup_status"], "ready");
    assert_eq!(fake["config_options"], options, "options are kept exactly");
    assert!(fake.get("modes").is_none(), "config options win over modes");
    assert_eq!(fake["chat"]["state"], "supported");
    assert_eq!(fake["flow"]["state"], "supported");
    for (capability, state) in [
        ("model_selection", "supported"),
        ("reasoning_effort", "supported"),
        ("session_resume", "supported"),
        ("pinned_instructions", "supported"),
        ("plan_mode", "unsupported"),
        ("forms", "unsupported"),
        ("per_prompt_usage", "unverified"),
    ] {
        assert_eq!(
            fake["capabilities"][capability]["state"], state,
            "{capability}"
        );
        assert!(fake["capabilities"][capability]["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()));
    }
    let models = |provider: &str| {
        response["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|model| model["provider"] == provider)
            .map(|model| {
                json!([
                    model["id"],
                    model["display"],
                    model["is_default"],
                    model["supported_reasoning_efforts"],
                    model["default_reasoning_effort"]
                ])
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        models("acp:fake"),
        vec![
            json!(["fast", "Fast", true, ["Low", "Med"], "Med"]),
            json!(["Deep/1", "Deep One", false, ["Low", "Med"], "Med"]),
        ]
    );

    let plain = &providers["acp:plain"];
    assert_eq!(plain["setup_status"], "ready", "{plain}");
    assert_eq!(
        plain["chat"]["state"], "unsupported",
        "no systemPrompt or channel"
    );
    assert_eq!(plain["flow"]["state"], "supported");
    assert_eq!(
        plain["capabilities"]["model_selection"]["state"],
        "unsupported"
    );
    assert_eq!(
        plain["modes"], modes,
        "legacy modes show without config options"
    );
    assert_eq!(
        models("acp:plain"),
        vec![json!(["", "Harness default", true, [], null])]
    );

    let locked = &providers["acp:locked"];
    assert_eq!(locked["status"], "unavailable", "{locked}");
    assert_eq!(locked["setup_status"], "unauthenticated");
    assert!(locked["error"]
        .as_str()
        .is_some_and(|error| !error.is_empty()));
    assert_eq!(locked["chat"]["state"], "unverified");
    assert_eq!(
        locked["capabilities"]["model_selection"]["state"],
        "unverified"
    );
    assert!(models("acp:locked").is_empty());

    let missing = &providers["acp:missing"];
    assert_eq!(missing["setup_status"], "unreachable", "{missing}");
    assert!(missing["error"]
        .as_str()
        .unwrap()
        .contains("executable not found"));

    assert!(!response.to_string().contains("discovery-secret-sentinel"));
    assert!(acp_received(temp.path(), "session/prompt").is_empty());
    let sessions = acp_received(temp.path(), "session/new");
    assert_eq!(
        sessions.len(),
        3,
        "one temporary session per reachable harness"
    );
    assert!(sessions
        .iter()
        .all(|session| session["cwd"] == json!(project)));
    let probes = launches();
    assert_eq!(probes.len(), 3);
    for probe in &probes {
        assert_eq!(probe["cwd"], json!(project));
        let alive = std::process::Command::new("kill")
            .args(["-0", &probe["pid"].to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the probe's process was released: {probe}");
    }

    assert_eq!(discover(), response, "cached");
    assert_eq!(launches().len(), 3, "a cached discovery launches nothing");
    spark_workspace::models::invalidate_model_discovery("agents");
    discover();
    assert_eq!(launches().len(), 6, "an agents change probes again");
}
