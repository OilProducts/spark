//! Chat with a native-provider turn: messages sent while it runs join it,
//! and Stop completes it.

use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use spark_agent_adapter::RustLlmAgentTurnBackend;
use spark_workspace::{ConversationTurnRequest, WorkspaceConversationService};
use unified_llm_adapter::{
    stream_events, AdapterError, Client, FinishReason, Message, ProviderAdapter, Request, Response,
    StreamEvent, StreamEvents,
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
    let _ = service.set(WorkspaceConversationService::new_with_agent_turn_backend(
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
