//! Native providers' questions, plan mode, status envelopes and subagents.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use spark_agent_adapter::codergen::{CodergenBackendResponse, STATUS_ENVELOPE_SCHEMA_METADATA_KEY};
use spark_agent_adapter::{
    AgentRequestUserInputAnswerRequest, AgentTurnBackend, AgentTurnRequest, CodergenBackend,
    CodergenBackendRequest, CodergenRuntimeMode, RustLlmAgentTurnBackend, RustLlmCodergenBackend,
};
use spark_common::events::{TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind};
use unified_llm_adapter::{
    stream_events, AdapterError, Client, ContentPart, FinishReason, Message, MessageRole,
    ProviderAdapter, Request, Response, ResponseFormat, StreamEvent, StreamEventType, StreamEvents,
    ToolCall, Usage,
};

/// A reply the provider rejects the request with.
const REJECT: &str = "<reject>";
/// A reply whose stream fails after some text.
const STREAM_ERROR: &str = "<stream-error>";

type Reply = Box<dyn Fn(&Request) -> (String, Vec<ToolCall>) + Send + Sync>;

/// Answers each request with the text and tool calls `reply` picks for it.
struct Scripted {
    requests: Mutex<Vec<Request>>,
    reply: Reply,
}

impl Scripted {
    fn client(
        reply: impl Fn(&Request) -> (String, Vec<ToolCall>) + Send + Sync + 'static,
    ) -> (Client, Arc<Scripted>) {
        let adapter = Arc::new(Scripted {
            requests: Mutex::new(Vec::new()),
            reply: Box::new(reply),
        });
        let provider: Arc<dyn ProviderAdapter> = adapter.clone();
        (
            Client::from_adapters(vec![provider], Some("openai")).expect("client"),
            adapter,
        )
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl ProviderAdapter for Scripted {
    fn name(&self) -> &str {
        "openai"
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.requests.lock().unwrap().push(request.clone());
        // Only text-only nodes complete; they have no tools to call.
        let (text, _) = (self.reply)(&request);
        Ok(Response {
            model: request.model.clone(),
            provider: "openai".to_string(),
            message: Message::assistant(text),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
                total_tokens: 2,
                ..Usage::default()
            },
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.requests.lock().unwrap().push(request.clone());
        let (text, calls) = (self.reply)(&request);
        if text == REJECT {
            return Err(AdapterError::new(
                unified_llm_adapter::AdapterErrorKind::InvalidRequest,
                "response format unsupported with tools",
            ));
        }
        if text == STREAM_ERROR {
            return Ok(stream_events(
                vec![
                    Ok(StreamEvent::text_delta("{\"outcome\"")),
                    Err(AdapterError::new(
                        unified_llm_adapter::AdapterErrorKind::Stream,
                        "connection reset",
                    )),
                ]
                .into_iter(),
            ));
        }
        let mut events = Vec::new();
        if !text.is_empty() {
            events.push(Ok(StreamEvent::text_delta(text)));
        }
        for call in calls {
            for kind in [StreamEventType::ToolCallStart, StreamEventType::ToolCallEnd] {
                events.push(Ok(StreamEvent {
                    tool_call: Some(call.clone()),
                    ..StreamEvent::new(kind)
                }));
            }
        }
        events.push(Ok(StreamEvent::finish(FinishReason::Stop, None)));
        Ok(stream_events(events.into_iter()))
    }
}

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
        raw_arguments: None,
        r#type: "function".to_string(),
    }
}

fn text(text: &str) -> (String, Vec<ToolCall>) {
    (text.to_string(), Vec::new())
}

fn tool_names(request: &Request) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

/// The tool results a request sends back, by call id.
fn tool_results(request: &Request) -> BTreeMap<String, String> {
    request
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| match part {
            ContentPart::ToolResult { tool_result } => Some((
                tool_result.tool_call_id.clone(),
                match &tool_result.content {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                },
            )),
            _ => None,
        })
        .collect()
}

fn first_user_text(request: &Request) -> String {
    request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .map(Message::text)
        .unwrap_or_default()
}

fn question_call() -> ToolCall {
    call(
        "ask-1",
        "request_user_input",
        json!({"questions": [{"header": "Scope", "question": "Which file?", "options": [{"label": "a.rs"}, {"label": "b.rs"}]}]}),
    )
}

fn chat_request(conversation_id: &str, chat_mode: &str) -> AgentTurnRequest {
    AgentTurnRequest {
        conversation_id: conversation_id.to_string(),
        project_path: std::env::temp_dir().to_string_lossy().to_string(),
        prompt: "Do the task".to_string(),
        history: Vec::new(),
        provider: Some("openai".to_string()),
        model: Some("gpt-native".to_string()),
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: Some(chat_mode.to_string()),
        metadata: BTreeMap::new(),
    }
}

fn node_request(prompt: &str, runtime_mode: CodergenRuntimeMode) -> CodergenBackendRequest {
    CodergenBackendRequest {
        node_id: "node".to_string(),
        prompt: prompt.to_string(),
        runtime_mode,
        provider: "openai".to_string(),
        model: Some("gpt-native".to_string()),
        project_path: Some(std::env::temp_dir()),
        ..CodergenBackendRequest::default()
    }
}

fn response_text(response: &CodergenBackendResponse) -> String {
    match response {
        CodergenBackendResponse::Text(text) => text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

fn wait_for(events: &Mutex<Vec<TurnStreamEvent>>, kind: TurnStreamEventKind) -> TurnStreamEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = events
            .lock()
            .unwrap()
            .iter()
            .find(|event| event.kind == kind)
        {
            return event.clone();
        }
        assert!(Instant::now() < deadline, "no {kind:?} event");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn native_chat_question_is_asked_and_answered_through_the_live_turn() {
    let (client, adapter) = Scripted::client(|request| match tool_results(request).get("ask-1") {
        None => (String::new(), vec![question_call()]),
        Some(answer) => text(&format!("Using: {answer}")),
    });
    let backend = RustLlmAgentTurnBackend::new(client);
    let streamed = Arc::new(Mutex::new(Vec::new()));
    let sink = streamed.clone();
    let turn = {
        let backend = backend.clone();
        std::thread::spawn(move || {
            backend.run_turn_with_event_sink(
                chat_request("native-chat-question", "agent"),
                Some(Arc::new(move |event| sink.lock().unwrap().push(event))),
            )
        })
    };

    let asked = wait_for(&streamed, TurnStreamEventKind::RequestUserInputRequested);
    let request = asked.request_user_input.expect("question payload");
    assert_eq!(request["request_id"], json!("ask-1"));
    assert_eq!(request["questions"][0]["id"], json!("question-1"));
    assert_eq!(request["questions"][0]["question"], json!("Which file?"));
    assert!(tool_names(&adapter.requests()[0]).contains(&"request_user_input".to_string()));

    let delivered = backend
        .answer_request_user_input(AgentRequestUserInputAnswerRequest {
            conversation_id: "native-chat-question".to_string(),
            project_path: std::env::temp_dir().to_string_lossy().to_string(),
            request_id: "ask-1".to_string(),
            assistant_turn_id: "turn-1".to_string(),
            answers: BTreeMap::from([("question-1".to_string(), "a.rs".to_string())]),
            request_user_input: Some(request),
            history: Vec::new(),
            provider: Some("openai".to_string()),
            model: Some("gpt-native".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("answer");
    assert_eq!(
        delivered.events[0].source.raw_kind.as_deref(),
        Some("request_user_input_answer_delivered")
    );

    let output = turn.join().unwrap().expect("turn output");
    assert_eq!(
        output.final_assistant_text.as_deref(),
        Some("Using: Which file?\nAnswer: a.rs")
    );
    assert_eq!(adapter.requests().len(), 2);
}

#[test]
fn native_node_clarification_is_answered_through_the_flow_handler() {
    let (client, adapter) = Scripted::client(|request| match tool_results(request).get("ask-1") {
        None => (String::new(), vec![question_call()]),
        Some(answer) => text(&format!("Done with {answer}")),
    });
    let mut backend = RustLlmCodergenBackend::new(client);
    let raised = Arc::new(Mutex::new(Vec::new()));
    let seen = raised.clone();
    backend.set_clarification_handler(Arc::new(move |params: Value| {
        seen.lock().unwrap().push(params);
        let mut polls = 0;
        Ok(Box::new(move || {
            // The node waits until the run delivers the answer.
            polls += 1;
            Ok((polls > 2).then(|| BTreeMap::from([("question-1".into(), "b.rs".into())])))
        })
            as spark_agent_adapter::codergen::ClarificationPoll)
    }));

    let output = backend
        .run(node_request("Fix it", CodergenRuntimeMode::agent()))
        .expect("node output");

    let raised = raised.lock().unwrap().clone();
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0]["agent_request_id"], json!("ask-1"));
    assert_eq!(raised[0]["questions"][0]["id"], json!("question-1"));
    assert_eq!(
        response_text(&output.response),
        "Done with Which file?\nAnswer: b.rs"
    );
    let requests = adapter.requests();
    assert!(requests[0]
        .messages
        .iter()
        .any(|message| message.text().contains("Use request_user_input")));
    assert!(output.events.iter().any(|event| {
        event
            .payload
            .get("turn_stream_event")
            .map(|event| &event["kind"])
            == Some(&json!("request_user_input_requested"))
    }));
}

#[test]
fn native_node_reply_ending_with_a_question_does_not_wait() {
    let (client, adapter) = Scripted::client(|_| text("Should I also update the docs?"));
    let mut backend = RustLlmCodergenBackend::new(client);
    backend.set_clarification_handler(Arc::new(|_| panic!("no explicit question was asked")));

    let output = backend
        .run(node_request("Fix it", CodergenRuntimeMode::agent()))
        .expect("node output");

    assert_eq!(
        response_text(&output.response),
        "Should I also update the docs?"
    );
    assert_eq!(adapter.requests().len(), 1);
    // The turn completed instead of parking on the question.
    assert!(output
        .events
        .iter()
        .any(|event| event.payload.get("kind") == Some(&json!("processing_end"))));
}

#[test]
fn native_plan_mode_proposes_a_plan_with_write_tools_unavailable() {
    let (client, adapter) = Scripted::client(|_| {
        (
            String::new(),
            vec![call(
                "plan-1",
                "submit_plan",
                json!({"plan": "1. Edit a.rs\n2. Test"}),
            )],
        )
    });
    let backend = RustLlmAgentTurnBackend::new(client);
    let streamed = Arc::new(Mutex::new(Vec::new()));
    let sink = streamed.clone();

    let output = backend
        .run_turn_with_event_sink(
            chat_request("native-plan", "plan"),
            Some(Arc::new(move |event| sink.lock().unwrap().push(event))),
        )
        .expect("plan turn");

    let requests = adapter.requests();
    assert_eq!(requests.len(), 1, "submit_plan ends the turn");
    assert_eq!(
        tool_names(&requests[0]),
        [
            "read_file",
            "grep",
            "glob",
            "request_user_input",
            "submit_plan"
        ]
    );
    assert!(requests[0].messages[0]
        .text()
        .contains("You are in plan mode"));
    let plan = output
        .events
        .iter()
        .find(|event| event.channel == Some(TurnStreamChannel::Plan))
        .expect("plan event");
    assert_eq!(plan.kind, TurnStreamEventKind::ContentCompleted);
    assert_eq!(plan.message.as_deref(), Some("1. Edit a.rs\n2. Test"));
    assert_eq!(*streamed.lock().unwrap(), output.events);

    // The event becomes the turn's plan segment, the proposed plan's source.
    let mut snapshot = json!({"segments": []});
    let segment = spark_common::segments::materialize_segment_for_event(
        &mut snapshot,
        "turn-1",
        plan,
        "2026-10-22T00:00:00Z",
        None,
    )
    .expect("plan segment");
    assert_eq!(segment["kind"], json!("plan"));
    assert_eq!(segment["content"], json!("1. Edit a.rs\n2. Test"));
}

fn envelope_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"outcome": {"type": "string", "enum": ["success", "fail"]}},
        "required": ["outcome"],
        "additionalProperties": false
    })
}

fn envelope_request(prompt: &str, runtime_mode: CodergenRuntimeMode) -> CodergenBackendRequest {
    let mut request = node_request(prompt, runtime_mode);
    request.response_contract = "status_envelope".to_string();
    request.contract_repair_attempts = 1;
    request.metadata.insert(
        STATUS_ENVELOPE_SCHEMA_METADATA_KEY.to_string(),
        envelope_schema(),
    );
    request
}

fn schema_of(request: &Request) -> Option<Value> {
    match request.response_format.as_ref()? {
        ResponseFormat::JsonSchema { json_schema, .. } => Some(json_schema.clone()),
        other => panic!("unexpected response format {other:?}"),
    }
}

#[test]
fn native_agent_node_asks_for_the_envelope_on_its_final_answer_and_repairs_in_session() {
    let (client, adapter) = Scripted::client(|request| {
        let results = tool_results(request);
        if request.messages.last().map(Message::text).as_deref() == Some("Repair your envelope") {
            text(r#"{"outcome":"success"}"#)
        } else if request.response_format.is_some() {
            text(r#"{"outcome":"bogus"}"#)
        } else if results.contains_key("glob-1") {
            text("All done.")
        } else {
            (
                String::new(),
                vec![call("glob-1", "glob", json!({"pattern": "*.none"}))],
            )
        }
    });
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(envelope_request("Do it", CodergenRuntimeMode::agent()))
        .expect("first attempt");

    let requests = adapter.requests();
    assert_eq!(requests.len(), 3);
    // Tool rounds run unconstrained; only the final answer is asked for
    // under the schema, with the tools still offered.
    assert_eq!(schema_of(&requests[0]), None);
    assert_eq!(schema_of(&requests[1]), None);
    assert_eq!(schema_of(&requests[2]), Some(envelope_schema()));
    assert!(tool_names(&requests[2]).contains(&"glob".to_string()));
    assert_eq!(response_text(&output.response), r#"{"outcome":"bogus"}"#);
    assert!(!output.resume_metadata.is_empty());

    let mut repair = envelope_request("Repair your envelope", CodergenRuntimeMode::agent());
    repair.metadata.extend(output.resume_metadata);
    repair.repair_attempt = Some(1);
    let repaired = backend.run(repair).expect("repair attempt");

    assert_eq!(
        response_text(&repaired.response),
        r#"{"outcome":"success"}"#
    );
    let repair_request = adapter.requests().pop().unwrap();
    // The same conversation continues: the first attempt's prompt, its tool
    // round and its answer precede the repair prompt.
    let texts = repair_request
        .messages
        .iter()
        .map(Message::text)
        .collect::<Vec<_>>();
    assert_eq!(texts[1], "Do it");
    assert!(texts.contains(&r#"{"outcome":"bogus"}"#.to_string()));
    assert_eq!(texts.last().unwrap(), "Repair your envelope");
    assert!(tool_results(&repair_request).contains_key("glob-1"));
    // The repair's conforming answer is still asked for under the schema.
    assert_eq!(schema_of(&repair_request), Some(envelope_schema()));
}

#[test]
fn native_agent_node_asks_for_the_envelope_even_when_its_answer_already_conforms() {
    let (client, adapter) = Scripted::client(|request| {
        if tool_results(request).contains_key("glob-1") {
            text(r#"{"outcome":"success"}"#)
        } else {
            (
                String::new(),
                vec![call("glob-1", "glob", json!({"pattern": "*.none"}))],
            )
        }
    });
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(envelope_request("Do it", CodergenRuntimeMode::agent()))
        .expect("node output");

    let requests = adapter.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(schema_of(&requests[1]), None);
    assert_eq!(schema_of(&requests[2]), Some(envelope_schema()));
    assert!(tool_names(&requests[2]).contains(&"glob".to_string()));
    assert_eq!(response_text(&output.response), r#"{"outcome":"success"}"#);
}

#[test]
fn native_agent_node_keeps_its_answer_when_the_provider_cannot_constrain_it() {
    let (client, adapter) = Scripted::client(|request| {
        if request.response_format.is_some() {
            text(REJECT)
        } else {
            text("All done.")
        }
    });
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(envelope_request("Do it", CodergenRuntimeMode::agent()))
        .expect("node output");

    assert_eq!(adapter.requests().len(), 2);
    // Contract repair takes the unconstrained answer from here.
    assert_eq!(response_text(&output.response), "All done.");
}

#[test]
fn native_agent_node_keeps_its_answer_when_the_schema_request_yields_none() {
    // The schema request (a) answers with empty text, or (b) its stream
    // fails after some text.
    for failure in ["", STREAM_ERROR] {
        let (client, adapter) = Scripted::client(move |request| {
            if request.response_format.is_some() {
                text(failure)
            } else {
                text("All done.")
            }
        });
        let mut backend = RustLlmCodergenBackend::new(client);

        let output = backend
            .run(envelope_request("Do it", CodergenRuntimeMode::agent()))
            .expect("node output");

        let requests = adapter.requests();
        assert_eq!(requests.len(), 2, "{failure:?}");
        // The answer is asked for again, not continued.
        assert_eq!(requests[1].messages, requests[0].messages);
        assert_eq!(schema_of(&requests[1]), Some(envelope_schema()));
        assert_eq!(response_text(&output.response), "All done.", "{failure:?}");
    }
}

#[test]
fn native_text_only_node_requests_the_envelope_schema_and_repairs_in_session() {
    let (client, adapter) = Scripted::client(|request| {
        if request.messages.len() == 1 {
            text("not json")
        } else {
            text(r#"{"outcome":"success"}"#)
        }
    });
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(envelope_request(
            "Classify",
            CodergenRuntimeMode::text_only(),
        ))
        .expect("first attempt");
    assert_eq!(schema_of(&adapter.requests()[0]), Some(envelope_schema()));

    let mut repair = envelope_request("Repair your envelope", CodergenRuntimeMode::text_only());
    repair.metadata.extend(output.resume_metadata);
    let repaired = backend.run(repair).expect("repair attempt");

    assert_eq!(
        response_text(&repaired.response),
        r#"{"outcome":"success"}"#
    );
    let repair_request = adapter.requests().pop().unwrap();
    assert_eq!(
        repair_request
            .messages
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>(),
        ["Classify", "not json", "Repair your envelope"]
    );
    assert_eq!(schema_of(&repair_request), Some(envelope_schema()));
}

/// A parent that spawns a child, waits for it and answers; the child globs
/// once and answers.
fn subagent_script(request: &Request) -> (String, Vec<ToolCall>) {
    let results = tool_results(request);
    if first_user_text(request) == "Look around" {
        return if results.contains_key("child-glob") {
            text("child found nothing")
        } else {
            (
                String::new(),
                vec![call("child-glob", "glob", json!({"pattern": "*.none"}))],
            )
        };
    }
    if results.contains_key("wait-1") {
        return text("Delegated");
    }
    if let Some(spawned) = results.get("spawn-1") {
        let agent_id = spawned
            .lines()
            .find_map(|line| line.strip_prefix("agent_id: "))
            .map(str::to_string)
            .or_else(|| {
                serde_json::from_str::<Value>(spawned).ok()?["agent_id"]
                    .as_str()
                    .map(str::to_string)
            })
            .expect("spawn result names the agent");
        return (
            String::new(),
            vec![call("wait-1", "wait", json!({"agent_id": agent_id}))],
        );
    }
    (
        String::new(),
        vec![call(
            "spawn-1",
            "spawn_agent",
            json!({"task": "Look around"}),
        )],
    )
}

fn assert_nested_under_spawn(tool_calls: &[Value]) {
    let spawn = tool_calls
        .iter()
        .find(|tool_call| tool_call["id"] == json!("spawn-1"))
        .expect("spawn call");
    assert_eq!(spawn["kind"], json!("subagent"));
    let child = tool_calls
        .iter()
        .filter(|tool_call| tool_call["id"] == json!("child-glob"))
        .collect::<Vec<_>>();
    assert!(child.len() >= 2, "the child's call starts and completes");
    assert!(child
        .iter()
        .all(|tool_call| tool_call["parent_call_id"] == json!("spawn-1")));
}

#[test]
fn native_subagent_work_nests_under_the_spawn_call() {
    let (client, _) = Scripted::client(subagent_script);
    let backend = RustLlmAgentTurnBackend::new(client.clone());
    let streamed = Arc::new(Mutex::new(Vec::new()));
    let sink = streamed.clone();
    let mut request = chat_request("native-subagent", "agent");
    request.prompt = "Delegate".to_string();

    let output = backend
        .run_turn_with_event_sink(
            request,
            Some(Arc::new(move |event| sink.lock().unwrap().push(event))),
        )
        .expect("chat turn");

    assert_eq!(output.final_assistant_text.as_deref(), Some("Delegated"));
    assert_eq!(*streamed.lock().unwrap(), output.events);
    assert_nested_under_spawn(
        &output
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    TurnStreamEventKind::ToolCallStarted | TurnStreamEventKind::ToolCallCompleted
                )
            })
            .filter_map(|event| event.tool_call.clone())
            .collect::<Vec<_>>(),
    );

    // A workflow node's transcript nests the same way.
    let mut backend = RustLlmCodergenBackend::new(client);
    let node = backend
        .run(node_request("Delegate", CodergenRuntimeMode::agent()))
        .expect("node output");
    assert_nested_under_spawn(
        &node
            .events
            .iter()
            .filter(|event| event.payload.get("category") == Some(&json!("tool_execution")))
            .filter_map(|event| event.payload.get("tool_event").cloned())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn native_agent_node_keeps_its_answer_at_the_tool_round_limit() {
    let (client, adapter) = Scripted::client(|request| {
        if request.response_format.is_some() {
            text(r#"{"outcome":"success"}"#)
        } else if first_user_text(request) == "Answer now" || !tool_results(request).is_empty() {
            text("All done.")
        } else {
            (
                String::new(),
                vec![call("glob-1", "glob", json!({"pattern": "*.none"}))],
            )
        }
    });
    let mut backend = RustLlmCodergenBackend::new(client);

    // With a limit of one tool round the node still answers (a) immediately,
    // and (b) right after that round, when the answer is asked for under the
    // schema directly.
    for (prompt, max_rounds, requests) in [("Answer now", 1, 2), ("Use a tool", 1, 2)] {
        let mut request = envelope_request(prompt, CodergenRuntimeMode::agent());
        request.metadata.insert(
            "spark.execution.settings".to_string(),
            json!({"configuration": {"agents": {"max_tool_rounds_per_input": max_rounds}}}),
        );
        let before = adapter.requests().len();
        let output = backend.run(request).expect("node output");
        let sent = adapter.requests().split_off(before);
        assert_eq!(sent.len(), requests, "{prompt}");
        assert_eq!(schema_of(sent.last().unwrap()), Some(envelope_schema()));
        assert_eq!(response_text(&output.response), r#"{"outcome":"success"}"#);
    }
}

#[test]
fn native_agent_node_keeps_its_answer_when_the_schema_request_only_calls_a_tool() {
    let (client, adapter) = Scripted::client(|request| {
        if request.response_format.is_some() {
            (
                String::new(),
                vec![call("schema-glob", "glob", json!({"pattern": "*.none"}))],
            )
        } else if first_user_text(request) == "Answer now" || !tool_results(request).is_empty() {
            text("All done.")
        } else {
            (
                String::new(),
                vec![call("glob-1", "glob", json!({"pattern": "*.none"}))],
            )
        }
    });
    let mut backend = RustLlmCodergenBackend::new(client);

    // The schema request spends the last permitted tool round on a tool call,
    // so the round limit ends the turn before any envelope is recorded.
    for (prompt, max_rounds, requests) in [("Answer now", 1, 2), ("Use a tool", 2, 3)] {
        let mut request = envelope_request(prompt, CodergenRuntimeMode::agent());
        request.metadata.insert(
            "spark.execution.settings".to_string(),
            json!({"configuration": {"agents": {"max_tool_rounds_per_input": max_rounds}}}),
        );
        let before = adapter.requests().len();
        let output = backend.run(request).expect("node output");
        let sent = adapter.requests().split_off(before);
        assert_eq!(sent.len(), requests, "{prompt}");
        assert_eq!(schema_of(sent.last().unwrap()), Some(envelope_schema()));
        assert_eq!(response_text(&output.response), "All done.", "{prompt}");
    }

    // Once the last permitted tool round is over, the schema request's tool
    // call is not run: the turn ends with that single request.
    let mut request = envelope_request("Use a tool", CodergenRuntimeMode::agent());
    request.metadata.insert(
        "spark.execution.settings".to_string(),
        json!({"configuration": {"agents": {"max_tool_rounds_per_input": 1}}}),
    );
    let before = adapter.requests().len();
    let output = backend.run(request).expect("node output");
    let sent = adapter.requests().split_off(before);
    assert_eq!(sent.len(), 2);
    assert_eq!(schema_of(&sent[1]), Some(envelope_schema()));
    assert!(!output.events.iter().any(|event| {
        event.payload.get("category") == Some(&json!("tool_execution"))
            && event.payload["tool_event"]["id"] == json!("schema-glob")
    }));
}
