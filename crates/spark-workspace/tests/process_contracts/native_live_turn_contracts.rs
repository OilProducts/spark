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

/// A model listing once no harness is still being checked.
fn settled(list: impl Fn() -> Value) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let response = list();
        let checking = response["providers"]
            .as_object()
            .unwrap()
            .values()
            .any(|provider| provider["status"] == "checking");
        if !checking {
            return response;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "harness checks finish: {response}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

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
fn acp_session_controls_edited_during_a_turn_apply_from_the_next_turn() {
    let temp = tempfile::tempdir().unwrap();
    let gate = temp.path().join("gate");
    let (service, project) = acp_chat(
        temp.path(),
        json!({
            "gate": gate,
            "config_options": [{"id": "approval", "name": "Approval", "type": "select", "currentValue": "ask",
                                "options": [{"value": "ask", "name": "Ask"}, {"value": "auto", "name": "Auto"}]}],
            "updates": acp_answer("Done."),
        }),
    );
    let running = {
        let (service, project) = (service.clone(), project.clone());
        std::thread::spawn(move || service.execute_turn("acp-edit", acp_turn(&project, "Work")))
    };
    let waited = std::time::Instant::now();
    while acp_received(temp.path(), "session/prompt").is_empty() {
        assert!(
            waited.elapsed() < std::time::Duration::from_secs(10),
            "prompt"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let revision = service.get_snapshot("acp-edit", Some(&project)).unwrap()["settings"]["models"]
        ["revision"]
        .clone();
    service
        .update_conversation_settings(
            "acp-edit",
            serde_json::from_value(json!({
                "project_path": project, "expected_revision": revision,
                "model_settings": {"provider": "acp:fake", "session_config": {"approval": "auto"}},
            }))
            .unwrap(),
        )
        .expect("an edit during work is accepted");
    std::fs::write(&gate, "").unwrap();
    let first = running.join().unwrap().unwrap();
    assert_eq!(assistant_turn(&first)["status"], "complete", "{first}");
    assert!(
        acp_received(temp.path(), "session/set_config_option").is_empty(),
        "the accepted turn keeps the settings it started with"
    );
    assert_eq!(
        first["model_settings"]["session_config"],
        json!({"approval": "auto"}),
        "finishing the turn keeps the edit: {first}"
    );

    let second = service
        .execute_turn("acp-edit", acp_turn(&project, "Next"))
        .unwrap();
    assert_eq!(assistant_turn(&second)["status"], "complete", "{second}");
    let set = acp_received(temp.path(), "session/set_config_option");
    assert_eq!(set.len(), 1);
    assert_eq!(
        (&set[0]["configId"], &set[0]["value"]),
        (&json!("approval"), &json!("auto"))
    );
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
fn acp_choosers_offer_what_a_session_last_acknowledged() {
    // Its listings check Claude Code with its own settings, replacing other tests' checks.
    let _env = super::utility_contracts::claude_env();
    let temp = tempfile::tempdir().unwrap();
    let model = |current: &str| {
        json!({"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": current,
               "options": [{"value": "fast", "name": "Fast"}, {"value": "deep", "name": "Deep"}]})
    };
    let effort = |values: &[&str]| {
        json!({"id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": "Low",
               "options": values.iter().map(|value| json!({"value": value, "name": value})).collect::<Vec<_>>()})
    };
    let approval = |values: &[&str]| {
        json!({"id": "approval", "name": "Approval", "type": "select", "currentValue": "ask",
               "options": values.iter().map(|value| json!({"value": value, "name": value})).collect::<Vec<_>>()})
    };
    // Choosing deep offers High; a later notification offers auto approval.
    let notified = json!([
        model("deep"),
        effort(&["Low", "High"]),
        approval(&["ask", "auto"])
    ]);
    let mut updates =
        vec![json!({"sessionUpdate": "config_option_update", "configOptions": notified})];
    updates.extend(acp_answer("Done.").as_array().unwrap().clone());
    let (service, project) = acp_chat(
        temp.path(),
        json!({
            "config_options": [model("fast"), effort(&["Low"]), approval(&["ask"])],
            "on_set": {"model=deep": [model("deep"), effort(&["Low", "High"]), approval(&["ask"])]},
            "updates": updates,
        }),
    );
    let settings = settings(temp.path());
    let chooser = || {
        let payload = settled(|| {
            spark_workspace::models::chat_models_with_codex_result(
                &settings,
                std::path::Path::new(&project),
                Ok(vec![]),
            )
            .unwrap()
        });
        let deep = payload["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["provider"] == "acp:fake" && model["id"] == "deep")
            .cloned()
            .unwrap();
        (
            deep,
            payload["providers"]["acp:fake"]["config_options"].clone(),
        )
    };
    let (deep, _) = chooser();
    assert_eq!(deep["supported_reasoning_efforts"], json!(["Low"]));

    let turn = |model: &str, effort: Option<&str>| {
        let mut request = acp_turn(&project, "Work");
        request.model = Some(model.to_string());
        request.reasoning_effort = effort.map(str::to_string);
        service.execute_turn("acp-acknowledged", request).unwrap()
    };
    let first = turn("deep", None);
    assert_eq!(assistant_turn(&first)["status"], "complete", "{first}");
    let (deep, options) = chooser();
    assert_eq!(
        deep["supported_reasoning_efforts"],
        json!(["Low", "High"]),
        "the dependent effort is offered without a settings change or restart"
    );
    assert_eq!(deep["is_default"], true);
    assert_eq!(options[2], approval(&["ask", "auto"]), "{options}");

    let second = turn("deep", Some("High"));
    assert_eq!(assistant_turn(&second)["status"], "complete", "{second}");
    assert_eq!(
        acp_received(temp.path(), "session/set_config_option")
            .last()
            .unwrap()["value"],
        "High"
    );

    // A value the harness no longer offers is still refused before the prompt.
    let prompts = acp_received(temp.path(), "session/prompt").len();
    let mut stale = acp_turn(&project, "Work");
    stale.model = Some("fast".to_string());
    stale.reasoning_effort = Some("High".to_string());
    let refused = service.execute_turn("acp-acknowledged", stale).unwrap();
    assert_eq!(assistant_turn(&refused)["status"], "failed", "{refused}");
    assert!(refused
        .to_string()
        .contains(r#"does not offer the value \"High\""#));
    assert_eq!(acp_received(temp.path(), "session/prompt").len(), prompts);
}

#[test]
fn acp_discovery_probes_harnesses_without_prompting_and_caches_by_generation() {
    // Native discovery in the same response reads the Claude binary override,
    // which other tests change meanwhile.
    let _env = super::utility_contracts::claude_env();
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
    // Twelve lines, more than the stderr tail Spark keeps, some of them short.
    let key = (1..=12)
        .map(|line| match line % 3 {
            0 => format!("Q{line:02}"),
            _ => format!("key-line-{line}-sentinel"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let harness = |name: &str, executable: &str, script: Value| {
        let mut script = script;
        script["log"] = json!(log);
        json!({"name": name, "executable": executable, "env": {
            "SPARK_FAKE_ACP": script.to_string(), "AGENT_TOKEN": "discovery-secret-sentinel",
            "AGENT_PIN": "cat", "AGENT_KEY": key,
        }})
    };
    let signed_in = temp.path().join("locked-signed-in");
    let browser =
        json!({"id": "browser", "name": "Browser sign-in", "description": "Opens your browser."});
    let terminal = json!({"id": "tui", "name": "Terminal", "type": "terminal", "args": ["login"],
        "env": {"LOGIN_MODE": "device"}, "description": "Signs in with discovery-secret-sentinel."});
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    let agents = json!({"agents": {
        "native": {"claude_binary": "/nonexistent/claude-for-tests"},
        "acp_harnesses": {
            "fake": harness("Fake ACP", FAKE_ACP, json!({"system_prompt": true, "resume": true, "config_options": options})),
            "plain": harness("Plain", FAKE_ACP, json!({"modes": modes})),
            "locked": harness("Locked", FAKE_ACP, json!({"new_error": {"code": -32000, "message": "Authentication required: run locked login"},
                "auth_methods": [browser, terminal], "signed_in": signed_in})),
            "leaky": harness("Leaky", FAKE_ACP, json!({"echo_env": ["AGENT_TOKEN", "AGENT_PIN"], "new_error": {"code": -32603, "message": "model catalog unavailable"}})),
            "spills": harness("Spills", FAKE_ACP, json!({"echo_env": ["AGENT_KEY"], "new_error": {"code": -32603, "message": "bad key"}})),
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
        settled(|| {
            spark_workspace::models::chat_models_with_codex_result(&settings, &project, Ok(vec![]))
                .unwrap()
        })
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
        ("forms", "supported"),
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
    assert_eq!(
        plain["flow"]["state"], "limited",
        "a step that sets a model is refused: {plain}"
    );
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
    assert!(
        locked["error"]
            .as_str()
            .unwrap()
            .contains("Authentication required: run locked login"),
        "the harness's own sign-in message is kept: {locked}"
    );
    assert_eq!(
        locked["auth_methods"],
        json!([browser, {"id": "tui", "name": "Terminal", "description": "Signs in with [redacted].",
            "terminal": ["LOGIN_MODE=device", FAKE_ACP, "login"]}]),
        "a terminal method is the command to run, and what the harness echoes stays hidden"
    );
    assert!(
        !locked["error"].as_str().unwrap().contains("Settings"),
        "shown where the user signs in, so it points nowhere else: {locked}"
    );
    assert_eq!(locked["chat"]["state"], "unverified");
    assert_eq!(
        locked["capabilities"]["model_selection"]["state"],
        "unverified"
    );
    assert!(models("acp:locked").is_empty());

    let missing = &providers["acp:missing"];
    assert_eq!(missing["setup_status"], "not_installed", "{missing}");
    assert_eq!(providers["claude-code"]["setup_status"], "not_installed");
    assert!(missing["error"]
        .as_str()
        .unwrap()
        .contains("executable not found"));

    let leaky = &providers["acp:leaky"];
    assert_eq!(leaky["setup_status"], "unreachable", "{leaky}");
    let error = leaky["error"].as_str().unwrap();
    assert!(
        error.contains("session/new failed")
            && error.contains("model catalog unavailable")
            && error.contains("AGENT_TOKEN=[redacted]")
            && error.contains("AGENT_PIN=[redacted]"),
        "the setup error stays actionable without the values, short ones included: {error}"
    );

    assert!(!response.to_string().contains("discovery-secret-sentinel"));
    assert_eq!(providers["acp:spills"]["setup_status"], "unreachable");
    assert!(
        providers["acp:spills"]["error"]
            .as_str()
            .unwrap()
            .ends_with("stderr tail:\n[redacted]"),
        "a secret's lines show as one: {}",
        providers["acp:spills"]
    );
    assert!(
        !response.to_string().contains("key-line-") && !response.to_string().contains("Q06"),
        "no line of a value of several lines shows: {}",
        providers["acp:spills"]
    );
    assert!(!response.to_string().contains("=cat"));
    assert!(acp_received(temp.path(), "session/prompt").is_empty());
    let sessions = acp_received(temp.path(), "session/new");
    assert_eq!(
        sessions.len(),
        5,
        "one temporary session per reachable harness"
    );
    assert!(sessions
        .iter()
        .all(|session| session["cwd"] == json!(project)));
    let probes = launches();
    assert_eq!(probes.len(), 5);
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
    assert_eq!(launches().len(), 5, "failures are kept too");
    spark_workspace::models::invalidate_model_discovery("agents");
    discover();
    assert_eq!(launches().len(), 10, "an agents change probes again");

    // One misconfigured registration stops no other, and a ready harness
    // whose program is removed shows so once checked again.
    let copied = temp.path().join("copied-acp");
    std::fs::copy(FAKE_ACP, &copied).unwrap();
    let mut changed = agents.clone();
    changed["agents"]["acp_harnesses"]["copied"] =
        harness("Copied", copied.to_str().unwrap(), json!({}));
    changed["agents"]["acp_harnesses"]["broken"] = json!({"name": "Broken", "executable": "agent",
        "env": {"SPARK_HOME": "broken-secret-sentinel"}});
    changed["agents"]["acp_harnesses"]["waits"] = harness(
        "Waits",
        FAKE_ACP,
        json!({"new_error": {"code": -32000, "message": "Authentication required"},
        "auth_methods": [browser], "authenticate_waits": true}),
    );
    changed["agents"]["acp_harnesses"]["quits"] = json!({"name": "Quits", "executable": "sh",
        "args": ["-c", "echo no agent config found >&2; exit 3"]});
    std::fs::write(
        settings.config_dir.join("spark.toml"),
        toml::to_string(&changed).unwrap(),
    )
    .unwrap();
    let response = discover();
    let providers = &response["providers"];
    assert_eq!(providers["acp:copied"]["setup_status"], "ready");
    assert_eq!(providers["acp:fake"]["setup_status"], "ready");
    assert_eq!(providers["acp:broken"]["setup_status"], "misconfigured");
    assert_eq!(providers["acp:broken"]["status"], "unavailable");
    assert!(providers["acp:broken"]["error"]
        .as_str()
        .unwrap()
        .contains("SPARK_HOME"));
    assert!(!response.to_string().contains("broken-secret-sentinel"));
    let quits = &providers["acp:quits"];
    assert_eq!(quits["setup_status"], "unreachable");
    let error = quits["error"].as_str().unwrap();
    assert!(
        error.contains("quit during setup with exit code 3")
            && error.contains("no agent config found"),
        "a harness that quits says so, with what it wrote: {error}"
    );
    std::fs::remove_file(&copied).unwrap();
    assert_eq!(
        discover()["providers"]["acp:copied"]["setup_status"],
        "ready",
        "kept until checked again"
    );
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(
        discover()["providers"]["acp:copied"]["setup_status"],
        "not_installed"
    );

    // Signing in with a method the harness offered keeps its sign-in.
    let signed =
        spark_workspace::models::acp_sign_in(&settings, &project, "locked", "browser").unwrap();
    assert_eq!(signed, json!({"setup_status": "ready", "error": null}));
    assert!(acp_received(temp.path(), "authenticate")
        .iter()
        .any(|request| request["methodId"] == "browser"));
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(
        discover()["providers"]["acp:locked"]["setup_status"],
        "ready"
    );

    // A sign-in the user leaves unfinished can be cancelled.
    let waiting = std::thread::spawn({
        let (settings, project) = (settings.clone(), project.clone());
        move || spark_workspace::models::acp_sign_in(&settings, &project, "waits", "browser")
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !spark_agent_adapter::acp::cancel_sign_in("waits") {
        assert!(std::time::Instant::now() < deadline, "the sign-in started");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let cancelled = waiting.join().unwrap().unwrap();
    assert_eq!(cancelled["setup_status"], "unauthenticated", "{cancelled}");
    assert!(cancelled["error"].as_str().unwrap().contains("cancelled"));
    assert!(!spark_agent_adapter::acp::cancel_sign_in("waits"));
}

/// A model listing never waits on a harness: one that has not answered yet
/// is `checking` while one background check runs, whose end is announced.
/// Its result, a failure included, is kept until Check again or an agents
/// change, and a check that ends after its entry was replaced or cleared is
/// discarded.
#[test]
fn model_listing_never_waits_on_a_harness_check() {
    use std::time::{Duration, Instant};
    // Discovery's caches are shared: no other discovery test checks again meanwhile.
    let _env = super::utility_contracts::claude_env();
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    let [launches, ended, release, fixed] =
        ["launches", "ended", "release", "fixed"].map(|name| temp.path().join(name));
    // Answers nothing until released, then quits, or once fixed is the fake agent.
    let script = format!(
        "echo >> '{}'; until [ -e '{}' ]; do sleep 0.02; done; echo >> '{}'; [ -e '{}' ] && exec '{FAKE_ACP}'; exit 3",
        launches.display(),
        release.display(),
        ended.display(),
        fixed.display(),
    );
    // A null `env` removes the registration.
    let register = |env: Value| {
        let harnesses = match env {
            Value::Null => json!({}),
            env => {
                json!({"slow": {"name": "Slow", "executable": "sh", "args": ["-c", script], "env": env}})
            }
        };
        let agents = json!({"agents": {
            "native": {"codex_binary": "/nonexistent/codex-for-tests", "claude_binary": "/nonexistent/claude-for-tests"},
            "acp_harnesses": harnesses,
        }});
        std::fs::write(
            settings.config_dir.join("spark.toml"),
            toml::to_string(&agents).unwrap(),
        )
        .unwrap();
    };
    register(json!({"SPARK_FAKE_ACP": "{}"}));
    let project = project.canonicalize().unwrap();
    let (sender, events) = std::sync::mpsc::channel::<String>();
    let finished: spark_workspace::models::CheckFinished =
        std::sync::Arc::new(move |provider: &str| {
            let _ = sender.send(provider.to_string());
        });
    let list = |refresh: bool| {
        spark_workspace::models::chat_models(&settings, &project, refresh, &finished).unwrap()
    };
    // The same work area by another name, as Home is when HOME is a symlink.
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&project, &link).unwrap();
    let list_link =
        || spark_workspace::models::chat_models(&settings, &link, false, &finished).unwrap();
    let slow = |response: &Value| {
        let slow = &response["providers"]["acp:slow"];
        (slow["status"].clone(), slow["setup_status"].clone())
    };
    let count = |path: &std::path::Path| {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .count()
    };
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let announced = || loop {
        let provider = events
            .recv_timeout(Duration::from_secs(30))
            .expect("the check's end is announced");
        if provider == "acp:slow" {
            break;
        }
    };
    // Waits out a check that ends now, then reports whether it was announced.
    let announced_since = || {
        std::thread::sleep(Duration::from_millis(300));
        events.try_iter().any(|provider| provider == "acp:slow")
    };
    let checking = (json!("checking"), Value::Null);
    let failed = (json!("unavailable"), json!("unreachable"));
    let ready = (json!("available"), json!("ready"));

    // Concurrent listings answer while the agent says nothing, sharing one
    // check whatever the work area is called.
    let listed = std::thread::scope(|scope| {
        let listings = (0..8)
            .map(|index| {
                scope.spawn(move || {
                    if index % 2 == 0 {
                        list(false)
                    } else {
                        list_link()
                    }
                })
            })
            .collect::<Vec<_>>();
        listings
            .into_iter()
            .map(|listing| listing.join().unwrap())
            .collect::<Vec<_>>()
    });
    for response in &listed {
        assert_eq!(slow(response), checking, "{response}");
        let providers = response["providers"].as_object().unwrap();
        assert!(providers.contains_key("codex") && providers.contains_key("claude-code"));
    }
    wait_for("the check started", &|| count(&launches) == 1);
    let started = Instant::now();
    assert_eq!(slow(&list(false)), checking);
    assert!(started.elapsed() < Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        count(&launches),
        1,
        "one check per harness and registration"
    );

    // Its failure shows once it ends, and is kept without checking again.
    std::fs::write(&release, "").unwrap();
    announced();
    let response = list(false);
    assert_eq!(slow(&response), failed, "{response}");
    assert!(response["providers"]["acp:slow"]["error"]
        .as_str()
        .unwrap()
        .contains("exit code 3"));
    assert_eq!(slow(&list(false)), failed);
    assert!(!announced_since());
    assert_eq!(count(&launches), 1, "a kept failure starts no check");

    // Once fixed, Check again shows the last result until the new one is in.
    // Checking again while it runs shares it, and the refetch on its end
    // starts nothing.
    std::fs::remove_file(&release).unwrap();
    std::fs::write(&fixed, "").unwrap();
    assert_eq!(slow(&list(true)), failed);
    wait_for("the new check started", &|| count(&launches) == 2);
    assert_eq!(slow(&list(true)), failed);
    std::fs::write(&release, "").unwrap();
    announced();
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(slow(&list_link()), ready);
    assert!(!announced_since());
    assert_eq!(
        count(&launches),
        2,
        "one check for overlapping Check agains"
    );

    // Check again in one work area leaves another's kept checks alone.
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let other = other.canonicalize().unwrap();
    let list_other =
        || spark_workspace::models::chat_models(&settings, &other, false, &finished).unwrap();
    assert_eq!(slow(&list_other()), checking);
    announced();
    assert_eq!(slow(&list_other()), ready);
    assert_eq!(count(&launches), 3);
    assert_eq!(slow(&list(true)), ready);
    announced();
    assert_eq!(slow(&list_other()), ready);
    assert!(!announced_since());
    assert_eq!(
        count(&launches),
        4,
        "only the listed work area is checked again"
    );

    // Saving agents settings checks again from nothing.
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    announced();
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(count(&launches), 5);

    // A check for a registration since changed is discarded.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the first check started", &|| count(&launches) == 6);
    register(json!({"SPARK_FAKE_ACP": "{}", "CHANGED": "1"}));
    assert_eq!(slow(&list(false)), checking);
    wait_for("the changed one's check started", &|| count(&launches) == 7);
    std::fs::write(&release, "").unwrap();
    wait_for("both checks ended", &|| count(&ended) == 7);
    announced();
    assert!(!announced_since(), "only the current registration's check");
    assert_eq!(slow(&list(false)), ready);

    // So is one whose entry was cleared while it ran.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the check started", &|| count(&launches) == 8);
    spark_workspace::models::invalidate_model_discovery("agents");
    std::fs::write(&release, "").unwrap();
    wait_for("the check ended", &|| count(&ended) == 8);
    assert!(!announced_since());
    assert_eq!(slow(&list(false)), checking, "its result was not kept");
    announced();
    assert_eq!(slow(&list(false)), ready);

    // And one whose registration was removed while it ran: restored, it is checked afresh.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the check started", &|| count(&launches) == 10);
    register(Value::Null);
    assert_eq!(list(false)["providers"].get("acp:slow"), None);
    std::fs::write(&release, "").unwrap();
    wait_for("the check ended", &|| count(&ended) == 10);
    assert!(!announced_since());
    register(json!({"SPARK_FAKE_ACP": "{}", "CHANGED": "1"}));
    assert_eq!(slow(&list(false)), checking, "its result was not kept");
    announced();
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(count(&launches), 11);

    // A registration changed while it ran, seen by a listing in another work
    // area: it is discarded there too, and only the listed one checks again.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the check started", &|| count(&launches) == 12);
    register(json!({"SPARK_FAKE_ACP": "{}"}));
    assert_eq!(slow(&list_other()), checking);
    wait_for("the other's check started", &|| count(&launches) == 13);
    std::fs::write(&release, "").unwrap();
    wait_for("both checks ended", &|| count(&ended) == 13);
    announced();
    assert!(!announced_since(), "only the current registration's check");
    assert_eq!(slow(&list_other()), ready);
    assert_eq!(count(&launches), 13, "no check where nothing was listed");
    register(json!({"SPARK_FAKE_ACP": "{}", "CHANGED": "1"}));
    assert_eq!(slow(&list(false)), checking, "its result was not kept");
    announced();
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(count(&launches), 14);

    // And one whose registration changed before it ended, with no listing
    // meanwhile: restored, it is checked afresh.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the check started", &|| count(&launches) == 15);
    register(json!({"SPARK_FAKE_ACP": "{}"}));
    std::fs::write(&release, "").unwrap();
    wait_for("the check ended", &|| count(&ended) == 15);
    assert!(!announced_since());
    register(json!({"SPARK_FAKE_ACP": "{}", "CHANGED": "1"}));
    assert_eq!(slow(&list(false)), checking, "its result was not kept");
    announced();
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(count(&launches), 16);

    // A save while a check runs, then a listing: the listing answers at once
    // and starts no second probe beside the running one, whose result is
    // discarded. Once it ends, a check from after the save runs and is the
    // only one announced.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the check started", &|| count(&launches) == 17);
    spark_workspace::models::invalidate_model_discovery("agents");
    let started = Instant::now();
    assert_eq!(slow(&list(false)), checking);
    assert_eq!(slow(&list(true)), checking);
    assert!(started.elapsed() < Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(count(&launches), 17, "no second probe beside the first");
    std::fs::write(&release, "").unwrap();
    wait_for("the check after the save started", &|| {
        count(&launches) == 18
    });
    announced();
    assert!(!announced_since(), "only the check after the save");
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(count(&launches), 18);

    // Saved as another registration and back while its first check runs,
    // listed each time: no second probe of it starts beside the first. Both
    // superseded checks are discarded, and a check from after the change back
    // follows, the only one announced.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the first check started", &|| count(&launches) == 19);
    register(json!({"SPARK_FAKE_ACP": "{}"}));
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the other registration's check started", &|| {
        count(&launches) == 20
    });
    register(json!({"SPARK_FAKE_ACP": "{}", "CHANGED": "1"}));
    spark_workspace::models::invalidate_model_discovery("agents");
    let started = Instant::now();
    assert_eq!(slow(&list(false)), checking);
    assert!(started.elapsed() < Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(count(&launches), 20, "no second probe beside the first");
    std::fs::write(&release, "").unwrap();
    wait_for("the check after the change back ended", &|| {
        count(&ended) == 21
    });
    announced();
    assert!(!announced_since(), "only the check after the change back");
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(count(&launches), 21);

    // A check asked for after one save is dropped by the next: with no
    // listing since, nothing starts once the running check ends.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    wait_for("the check started", &|| count(&launches) == 22);
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list(false)), checking);
    spark_workspace::models::invalidate_model_discovery("agents");
    std::fs::write(&release, "").unwrap();
    wait_for("the check ended", &|| count(&ended) == 22);
    assert!(!announced_since());
    assert_eq!(count(&launches), 22, "nothing starts without a listing");
    assert_eq!(slow(&list(false)), checking);
    announced();
    assert_eq!(slow(&list(false)), ready);
    assert_eq!(count(&launches), 23);

    // Listings that read the agents settings before a save, here from a copy
    // of them, leave the saved registration's running check alone in every
    // work area: it ends announced, and the next listing shows its result
    // without checking again. The stale listings' own checks are discarded.
    std::fs::remove_file(&release).unwrap();
    let saved = settings.config_dir.join("spark.toml");
    let old = std::fs::read_to_string(&saved).unwrap();
    let mut before = settings.clone();
    before.config_dir = temp.path().join("before-save");
    std::fs::create_dir_all(&before.config_dir).unwrap();
    std::fs::write(before.config_dir.join("spark.toml"), &old).unwrap();
    let stale = |work_area: &std::path::Path| {
        spark_workspace::models::chat_models(&before, work_area, false, &finished).unwrap()
    };
    register(json!({"SPARK_FAKE_ACP": "{}"}));
    spark_workspace::models::invalidate_model_discovery("agents");
    assert_eq!(slow(&list_other()), checking);
    wait_for("the saved registration's check started", &|| {
        count(&launches) == 24
    });
    assert_eq!(slow(&stale(&other)), checking);
    assert_eq!(slow(&stale(&project)), checking);
    wait_for("the stale listings' checks started", &|| {
        count(&launches) == 26
    });
    // By the time those checks end, the settings they read are saved over.
    std::fs::copy(&saved, before.config_dir.join("spark.toml")).unwrap();
    std::fs::write(&release, "").unwrap();
    wait_for("every check ended", &|| count(&ended) == 26);
    announced();
    assert!(!announced_since(), "only the saved registration's check");
    assert_eq!(slow(&list_other()), ready);
    assert_eq!(count(&launches), 26, "its result was kept");

    // Nor does one leave its kept result behind.
    std::fs::remove_file(&release).unwrap();
    std::fs::write(before.config_dir.join("spark.toml"), &old).unwrap();
    assert_eq!(slow(&stale(&other)), checking);
    wait_for("the stale listing's check started", &|| {
        count(&launches) == 27
    });
    std::fs::copy(&saved, before.config_dir.join("spark.toml")).unwrap();
    assert_eq!(slow(&list_other()), ready);
    std::fs::write(&release, "").unwrap();
    wait_for("the stale check ended", &|| count(&ended) == 27);
    assert!(!announced_since());
    assert_eq!(slow(&list_other()), ready);
    assert_eq!(count(&launches), 27, "its result is still kept");
}

#[test]
fn each_native_harness_keeps_its_check_when_the_other_changes() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};
    let _env = super::utility_contracts::claude_env();
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    let [claude, launches, ended, release] =
        ["claude", "launches", "ended", "release"].map(|name| temp.path().join(name));
    // Answers nothing until released, then quits.
    std::fs::write(
        &claude,
        format!(
            "#!/bin/sh\necho >> '{}'\nuntil [ -e '{}' ]; do sleep 0.02; done\necho >> '{}'\nexit 3\n",
            launches.display(),
            release.display(),
            ended.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A changed Codex program is as installing Codex on PATH when none is set.
    let configure = |codex: &str, claude: &std::path::Path| {
        let agents =
            json!({"agents": {"native": {"codex_binary": codex, "claude_binary": claude}}});
        std::fs::write(
            settings.config_dir.join("spark.toml"),
            toml::to_string(&agents).unwrap(),
        )
        .unwrap();
    };
    let project = temp.path().canonicalize().unwrap();
    let (sender, events) = std::sync::mpsc::channel::<String>();
    let finished: spark_workspace::models::CheckFinished = Arc::new(move |provider: &str| {
        let _ = sender.send(provider.to_string());
    });
    let claude_code = || {
        spark_workspace::models::chat_models(&settings, &project, false, &finished).unwrap()
            ["providers"]["claude-code"]["status"]
            .clone()
    };
    let count = |path: &std::path::Path| {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .count()
    };
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let announced = || {
        while events
            .recv_timeout(Duration::from_secs(30))
            .expect("the check ended")
            != "claude-code"
        {}
    };

    configure("/nonexistent/codex-one", &claude);
    assert_eq!(claude_code(), "checking");
    wait_for("the check started", &|| count(&launches) == 1);
    // Codex's program changing leaves Claude Code's running check alone,
    configure("/nonexistent/codex-two", &claude);
    assert_eq!(claude_code(), "checking");
    std::fs::write(&release, "").unwrap();
    announced();
    // and its kept result.
    configure("/nonexistent/codex-three", &claude);
    assert_eq!(claude_code(), "unavailable");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(count(&launches), 1);

    // A check whose own settings change before it ends is discarded, listed meanwhile or not.
    std::fs::remove_file(&release).unwrap();
    spark_workspace::models::invalidate_model_discovery("claude_code");
    assert_eq!(claude_code(), "checking");
    wait_for("the check started", &|| count(&launches) == 2);
    configure(
        "/nonexistent/codex-three",
        std::path::Path::new("/nonexistent/claude-moved"),
    );
    std::fs::write(&release, "").unwrap();
    wait_for("the check ended", &|| count(&ended) == 2);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!events.try_iter().any(|provider| provider == "claude-code"));
    configure("/nonexistent/codex-three", &claude);
    assert_eq!(claude_code(), "checking", "its result was not kept");
    announced();
    assert_eq!(claude_code(), "unavailable");
    assert_eq!(count(&launches), 3);
}

#[test]
fn a_kept_check_keeps_the_setup_it_found() {
    use std::os::unix::fs::PermissionsExt;
    let _env = super::utility_contracts::claude_env();
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    let bin = temp.path().join("bin");
    let programs = ["codex", "claude", "agent"].map(|name| bin.join(name));
    let [codex, claude, agent] = &programs;
    let agents = json!({"agents": {
        "native": {"codex_binary": codex, "claude_binary": claude},
        "acp_harnesses": {"later": {"name": "Later", "executable": agent}},
    }});
    std::fs::write(
        settings.config_dir.join("spark.toml"),
        toml::to_string(&agents).unwrap(),
    )
    .unwrap();
    let project = temp.path().canonicalize().unwrap();
    let finished: spark_workspace::models::CheckFinished = Arc::new(|_: &str| {});
    let list =
        || spark_workspace::models::chat_models(&settings, &project, false, &finished).unwrap();
    let setups = |response: Value| {
        ["codex", "claude-code", "acp:later"]
            .map(|provider| response["providers"][provider]["setup_status"].clone())
    };
    let missing = [
        json!("not_installed"),
        json!("not_installed"),
        json!("not_installed"),
    ];
    assert_eq!(setups(settled(list)), missing);

    // Installing a program where it was missing changes nothing until checked again.
    std::fs::create_dir_all(&bin).unwrap();
    for program in &programs {
        std::fs::write(program, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(setups(list()), missing);
}
