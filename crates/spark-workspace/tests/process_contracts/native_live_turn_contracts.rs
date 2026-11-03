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
    let (started, snapshot) = service.send_turn(conversation_id, turn(message)).unwrap();
    assert!(started.is_none(), "the message started a new turn");
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
        let (started, snapshot) = service
            .send_turn(
                "native-resend",
                ConversationTurnRequest {
                    client_turn_id: Some("turn-steer1".to_string()),
                    ..turn("Also update the docs.")
                },
            )
            .unwrap();
        assert!(started.is_none(), "the resend started a turn");
        snapshot
    };
    let reported = |snapshot: &Value| {
        user_messages(snapshot)
            .iter()
            .any(|segment| segment["id"] == "segment-user-message-turn-steer1")
    };
    let (_temp, service, calls) = native_chat(move |service, index, _| {
        if index == 0 {
            let first = resend(service);
            assert!(reported(&first), "accepted before delivery: {first}");
            // Before the turn reports the delivery.
            let again = resend(service);
            assert!(reported(&again));
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
                matches!(conflict, spark_workspace::WorkspaceError::Conflict(_)),
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
    let snapshot = resend(service);

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
            let (started, snapshot) = service
                .send_turn(
                    "native-attach",
                    ConversationTurnRequest {
                        attachment_ids: vec![upload.id.clone()],
                        ..turn("See the notes.")
                    },
                )
                .unwrap();
            assert!(started.is_none(), "the message started a new turn");
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
