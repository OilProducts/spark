use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use spark_agent_adapter::{
    create_anthropic_profile, create_gemini_profile, create_openai_profile, CommandOptions,
    DirEntry, EnvironmentError, EnvironmentResult, EventKind, ExecResult, ExecutionEnvironment,
    ExecutionEnvironmentBackend, GrepOptions, HistoryTurn, RegisteredTool, Session, SessionConfig,
    SessionState, SubAgentStatus, ToolDefinition, ToolDispatchContext, ToolDispatchEvent,
    ToolExecutionOutput, ToolRegistry,
};
use tempfile::tempdir;
use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Client, ContentPart, FinishReason, Message, ProviderAdapter,
    Request, Response, StreamEvents, Tool, ToolCall, ToolResult,
};

const PNG_BYTES: &[u8] = &[
    0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
    b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15,
    0xc4, 0x89, 0x00, 0x00, 0x00, 0x0c, b'I', b'D', b'A', b'T', 0x08, 0xd7, b'c', 0xf8, 0x0f, 0x00,
    0x01, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d, 0x18, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D',
    0xae, b'B', b'`', 0x82,
];

fn definition(name: &str, description: &str, parameters: Value) -> ToolDefinition {
    ToolDefinition::new(name, description, Some(parameters)).expect("valid tool definition")
}

fn dispatch_with_recorded_events(
    registry: &ToolRegistry,
    tool_call: ToolCall,
) -> (ToolResult, Vec<ToolDispatchEvent>) {
    dispatch_with_context_recorded_events(registry, tool_call, ToolDispatchContext::default())
}

fn dispatch_with_context_recorded_events(
    registry: &ToolRegistry,
    tool_call: ToolCall,
    context: ToolDispatchContext,
) -> (ToolResult, Vec<ToolDispatchEvent>) {
    let events = Arc::new(Mutex::new(Vec::<ToolDispatchEvent>::new()));
    let captured_events = events.clone();
    let result = registry.dispatch(
        tool_call,
        ToolDispatchContext {
            event_hook: Some(Arc::new(move |event| {
                captured_events.lock().expect("events").push(event);
            })),
            ..context
        },
    );
    let events = events.lock().expect("events").clone();
    (result, events)
}

fn assert_single_start_end(
    events: &[ToolDispatchEvent],
    tool_call_id: &str,
    tool_name: &str,
    arguments: Value,
    raw_arguments: Option<&str>,
) {
    assert_eq!(events.len(), 2, "events for {tool_call_id}: {events:?}");
    assert_eq!(events[0].kind, EventKind::ToolCallStart);
    assert_eq!(events[1].kind, EventKind::ToolCallEnd);
    for event in events {
        assert_eq!(event.data["tool_call_id"], json!(tool_call_id));
        assert_eq!(event.data["tool_name"], json!(tool_name));
    }
    assert_eq!(events[0].data["arguments"], arguments);
    match raw_arguments {
        Some(raw_arguments) => assert_eq!(events[0].data["raw_arguments"], json!(raw_arguments)),
        None => assert!(!events[0].data.contains_key("raw_arguments")),
    }
}

fn assistant_text_response(id: &str, text: &str) -> Response {
    Response {
        id: id.to_string(),
        model: "fake-model".to_string(),
        provider: "fake-provider".to_string(),
        message: Message::assistant(text),
        finish_reason: FinishReason::Stop,
        ..Response::default()
    }
}

fn tool_call_response(id: &str, tool_call: ToolCall) -> Response {
    let mut message = Message::assistant("");
    message.content.push(ContentPart::ToolCall { tool_call });
    Response {
        id: id.to_string(),
        model: "fake-model".to_string(),
        provider: "fake-provider".to_string(),
        message,
        finish_reason: FinishReason::ToolCalls,
        ..Response::default()
    }
}

fn latest_agent_id(request: &Request) -> String {
    request
        .messages
        .iter()
        .rev()
        .flat_map(|message| message.content.iter())
        .find_map(|part| match part {
            ContentPart::ToolResult { tool_result } => tool_result
                .content
                .get("agent_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        })
        .expect("agent_id from previous subagent tool result")
}

fn has_message_text(request: &Request, expected: &str) -> bool {
    request
        .messages
        .iter()
        .any(|message| message.text() == expected)
}

fn last_tool_result_id(request: &Request) -> Option<String> {
    request
        .messages
        .iter()
        .rev()
        .flat_map(|message| message.content.iter())
        .find_map(|part| match part {
            ContentPart::ToolResult { tool_result } => Some(tool_result.tool_call_id.clone()),
            _ => None,
        })
}

fn session_tool_results(session: &Session) -> Vec<unified_llm_adapter::ToolResultData> {
    session
        .history
        .iter()
        .flat_map(|turn| match turn {
            HistoryTurn::ToolResults(turn) => turn.results().to_vec(),
            _ => Vec::new(),
        })
        .collect()
}

fn subagent_enabled_profile() -> spark_agent_adapter::ProviderProfile {
    let mut profile = create_openai_profile("fake-model");
    profile.id = "fake-provider".to_string();
    profile.request_provider = Some("fake-provider".to_string());
    profile.display_name = Some("Fake".to_string());
    profile.supports_streaming = false;
    profile
}

#[derive(Default)]
struct HappySubagentAdapter {
    requests: Mutex<Vec<Request>>,
}

impl HappySubagentAdapter {
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("requests").clone()
    }
}

impl ProviderAdapter for HappySubagentAdapter {
    fn name(&self) -> &str {
        "fake-provider"
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let mut requests = self.requests.lock().expect("requests");
        let response = if has_message_text(&request, "Investigate the repository") {
            if has_message_text(&request, "Please continue") {
                assistant_text_response("child-2", "child response 2")
            } else {
                assistant_text_response("child-1", "child response 1")
            }
        } else {
            match last_tool_result_id(&request).as_deref() {
                None => tool_call_response(
                    "parent-spawn",
                    ToolCall::new(
                        "spawn-1",
                        "spawn_agent",
                        json!({"task": "Investigate the repository"}),
                    ),
                ),
                Some("spawn-1") => {
                    let agent_id = latest_agent_id(&request);
                    tool_call_response(
                        "parent-send",
                        ToolCall::new(
                            "send-1",
                            "send_input",
                            json!({"agent_id": agent_id, "message": "Please continue"}),
                        ),
                    )
                }
                Some("send-1") => {
                    let agent_id = latest_agent_id(&request);
                    tool_call_response(
                        "parent-wait",
                        ToolCall::new("wait-1", "wait", json!({"agent_id": agent_id})),
                    )
                }
                Some("wait-1") => {
                    let agent_id = latest_agent_id(&request);
                    tool_call_response(
                        "parent-close",
                        ToolCall::new("close-1", "close_agent", json!({"agent_id": agent_id})),
                    )
                }
                Some("close-1") => assistant_text_response("parent-done", "parent done"),
                _ => {
                    return Err(AdapterError::new(
                        AdapterErrorKind::Configuration,
                        "unexpected complete call",
                    ));
                }
            }
        };
        requests.push(request);
        Ok(response)
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, AdapterError> {
        Err(AdapterError::new(
            AdapterErrorKind::Configuration,
            "streaming is disabled",
        ))
    }
}

#[derive(Default)]
struct ModelOverrideSubagentAdapter {
    requests: Mutex<Vec<Request>>,
}

impl ModelOverrideSubagentAdapter {
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("requests").clone()
    }
}

impl ProviderAdapter for ModelOverrideSubagentAdapter {
    fn name(&self) -> &str {
        "fake-provider"
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let mut requests = self.requests.lock().expect("requests");
        let response = if has_message_text(&request, "Investigate with child model") {
            assistant_text_response("child-model-response", "child response")
        } else {
            match last_tool_result_id(&request).as_deref() {
                None => tool_call_response(
                    "parent-spawn",
                    ToolCall::new(
                        "spawn-model-override",
                        "spawn_agent",
                        json!({
                            "task": "Investigate with child model",
                            "model": "child-model"
                        }),
                    ),
                ),
                Some("spawn-model-override") => {
                    let agent_id = latest_agent_id(&request);
                    tool_call_response(
                        "parent-wait",
                        ToolCall::new("wait-model-override", "wait", json!({"agent_id": agent_id})),
                    )
                }
                Some("wait-model-override") => {
                    assistant_text_response("parent-done", "parent done")
                }
                _ => {
                    return Err(AdapterError::new(
                        AdapterErrorKind::Configuration,
                        "unexpected complete call",
                    ));
                }
            }
        };
        requests.push(request);
        Ok(response)
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, AdapterError> {
        Err(AdapterError::new(
            AdapterErrorKind::Configuration,
            "streaming is disabled",
        ))
    }
}

#[derive(Default)]
struct ChildProcessingFailureAdapter {
    requests: Mutex<Vec<Request>>,
}

impl ProviderAdapter for ChildProcessingFailureAdapter {
    fn name(&self) -> &str {
        "fake-provider"
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let mut requests = self.requests.lock().expect("requests");
        let response = if has_message_text(&request, "Investigate failing child") {
            requests.push(request);
            return Err(AdapterError::new(
                AdapterErrorKind::Provider,
                "child provider failed",
            ));
        } else {
            match last_tool_result_id(&request).as_deref() {
                None => tool_call_response(
                    "parent-spawn",
                    ToolCall::new(
                        "spawn-failing-child",
                        "spawn_agent",
                        json!({"task": "Investigate failing child"}),
                    ),
                ),
                Some("spawn-failing-child") => {
                    let agent_id = latest_agent_id(&request);
                    tool_call_response(
                        "parent-wait",
                        ToolCall::new("wait-failing-child", "wait", json!({"agent_id": agent_id})),
                    )
                }
                Some("wait-failing-child") => assistant_text_response("parent-done", "parent done"),
                _ => {
                    return Err(AdapterError::new(
                        AdapterErrorKind::Configuration,
                        "unexpected complete call",
                    ));
                }
            }
        };
        requests.push(request);
        Ok(response)
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, AdapterError> {
        Err(AdapterError::new(
            AdapterErrorKind::Configuration,
            "streaming is disabled",
        ))
    }
}

#[derive(Default)]
struct RecoverableSubagentFailureAdapter {
    requests: Mutex<Vec<Request>>,
}

impl ProviderAdapter for RecoverableSubagentFailureAdapter {
    fn name(&self) -> &str {
        "fake-provider"
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let mut requests = self.requests.lock().expect("requests");
        let index = requests.len();
        let response = match index {
            0 => tool_call_response(
                "unknown-agent",
                ToolCall::new(
                    "send-unknown",
                    "send_input",
                    json!({"agent_id": "missing-agent", "message": "hello"}),
                ),
            ),
            1 => tool_call_response(
                "bad-working-dir",
                ToolCall::new(
                    "spawn-bad-dir",
                    "spawn_agent",
                    json!({"task": "Investigate", "working_dir": "../escape"}),
                ),
            ),
            2 => tool_call_response(
                "bad-model",
                ToolCall::new(
                    "spawn-bad-model",
                    "spawn_agent",
                    json!({"task": "Investigate", "model": "other-model"}),
                ),
            ),
            3 => assistant_text_response("done", "done"),
            _ => {
                return Err(AdapterError::new(
                    AdapterErrorKind::Configuration,
                    "unexpected complete call",
                ));
            }
        };
        requests.push(request);
        Ok(response)
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, AdapterError> {
        Err(AdapterError::new(
            AdapterErrorKind::Configuration,
            "streaming is disabled",
        ))
    }
}

#[test]
fn tool_definition_requires_object_root_json_schema_and_defaults_to_empty_object() {
    let definition = ToolDefinition::new("lookup", "Lookup values", None).expect("definition");
    assert_eq!(definition.parameters, json!({"type": "object"}));

    let error = ToolDefinition::new("lookup", "Lookup values", Some(json!({"type": "string"})))
        .expect_err("non-object root rejected");
    assert!(error.contains("root type must be object"));
}

#[test]
fn registry_register_unregister_get_definitions_and_names_are_latest_wins() {
    let first_definition = definition("lookup", "first", json!({"type": "object"}));
    let second_definition = definition("lookup", "second", json!({"type": "object"}));

    let mut registry = ToolRegistry::new();
    let previous = registry.register(RegisteredTool::new::<Value, _>(
        first_definition.clone(),
        |_| Ok(json!("first")),
    ));
    assert!(previous.is_none());

    let previous = registry.register(RegisteredTool::new::<Value, _>(
        second_definition.clone(),
        |_| Ok(json!("second")),
    ));
    assert_eq!(
        previous.expect("replaced").definition.description,
        first_definition.description
    );
    assert_eq!(registry.names(), ["lookup"]);
    assert_eq!(registry.definitions(), [second_definition.clone()]);
    assert_eq!(
        registry
            .dispatch(
                ToolCall::new("call-1", "lookup", json!({})),
                ToolDispatchContext::default(),
            )
            .content,
        json!("second")
    );

    let removed = registry.unregister("lookup").expect("removed");
    assert_eq!(removed.definition, second_definition);
    assert!(registry.get("lookup").is_none());
    assert!(registry.definitions().is_empty());
    assert!(registry.names().is_empty());
}

#[test]
fn dispatch_executes_valid_calls_then_invokes_truncation_and_event_hooks_before_result() {
    let observations = Arc::new(Mutex::new(Vec::<String>::new()));
    let executor_observations = observations.clone();
    let definition = definition(
        "lookup",
        "Lookup values",
        json!({
            "type": "object",
            "properties": {"value": {"type": "integer"}},
            "required": ["value"],
            "additionalProperties": false
        }),
    );
    let mut registry = ToolRegistry::new();
    registry.register(RegisteredTool::new::<Value, _>(
        definition,
        move |invocation| {
            assert_eq!(invocation.tool_call_id, "call-ok");
            assert_eq!(invocation.arguments, json!({"value": 7}));
            executor_observations
                .lock()
                .expect("observations")
                .push("execute".to_string());
            Ok(json!("0123456789"))
        },
    ));

    let truncation_observations = observations.clone();
    let event_observations = observations.clone();
    let mut config = SessionConfig::default();
    config.tool_output_limits.insert("lookup".to_string(), 4);
    let context = ToolDispatchContext {
        config,
        truncation_hook: Some(Arc::new(move |truncation| {
            assert_eq!(truncation.full_content, json!("0123456789"));
            assert!(truncation
                .default_model_content
                .as_str()
                .expect("string content")
                .contains("Tool output was truncated"));
            truncation_observations
                .lock()
                .expect("observations")
                .push("truncate".to_string());
            json!("hook-truncated")
        })),
        event_hook: Some(Arc::new(move |event| match event.kind {
            EventKind::ToolCallStart => {
                assert_eq!(event.data["tool_call_id"], json!("call-ok"));
                assert_eq!(event.data["tool_name"], json!("lookup"));
                assert_eq!(event.data["arguments"], json!({"value": 7}));
                assert_eq!(event.data["raw_arguments"], json!(r#"{"value": 7}"#));
                event_observations
                    .lock()
                    .expect("observations")
                    .push("event-start".to_string());
            }
            EventKind::ToolCallEnd => {
                assert_eq!(event.data["tool_call_id"], json!("call-ok"));
                assert_eq!(event.data["tool_name"], json!("lookup"));
                assert_eq!(event.data["output"], json!("0123456789"));
                assert_eq!(event.data["model_content"], json!("hook-truncated"));
                event_observations
                    .lock()
                    .expect("observations")
                    .push("event-end".to_string());
            }
            other => panic!("unexpected tool dispatch event: {other:?}"),
        })),
        ..ToolDispatchContext::default()
    };

    let result = registry.dispatch(
        ToolCall::from_raw_arguments("call-ok", "lookup", r#"{"value": 7}"#),
        context,
    );

    assert_eq!(result.tool_call_id, "call-ok");
    assert!(!result.is_error);
    assert_eq!(result.content, json!("hook-truncated"));
    assert_eq!(
        observations.lock().expect("observations").as_slice(),
        ["event-start", "execute", "truncate", "event-end"]
    );
}

#[test]
fn dispatch_emits_one_start_then_one_end_for_each_outcome() {
    let lookup_definition = definition(
        "lookup",
        "Lookup values",
        json!({
            "type": "object",
            "properties": {"value": {"type": "integer"}},
            "required": ["value"],
            "additionalProperties": false
        }),
    );
    let mut registry = ToolRegistry::new();
    registry.register(RegisteredTool::new::<Value, _>(lookup_definition, |_| {
        Ok(json!("ok"))
    }));
    registry.register(RegisteredTool::new::<Value, _>(
        definition("fails", "Always fails", json!({"type": "object"})),
        |_| {
            Err(AdapterError::new(
                AdapterErrorKind::InvalidToolCall,
                "permission denied",
            ))
        },
    ));

    let (success, events) = dispatch_with_recorded_events(
        &registry,
        ToolCall::from_raw_arguments("call-ok", "lookup", r#"{"value": 7}"#),
    );
    assert!(!success.is_error);
    assert_eq!(success.content, json!("ok"));
    assert_single_start_end(
        &events,
        "call-ok",
        "lookup",
        json!({"value": 7}),
        Some(r#"{"value": 7}"#),
    );
    assert_eq!(events[1].data["output"], json!("ok"));

    let (unknown, events) = dispatch_with_recorded_events(
        &registry,
        ToolCall::new("call-unknown", "missing", json!({"value": 7})),
    );
    assert!(unknown.is_error);
    assert_eq!(unknown.content, json!("Unknown tool: missing"));
    assert_single_start_end(
        &events,
        "call-unknown",
        "missing",
        json!({"value": 7}),
        None,
    );
    assert_eq!(events[1].data["error"], json!("Unknown tool: missing"));

    let (invalid_json, events) = dispatch_with_recorded_events(
        &registry,
        ToolCall::from_raw_arguments("call-json", "lookup", "{not json"),
    );
    assert!(invalid_json.is_error);
    assert_single_start_end(
        &events,
        "call-json",
        "lookup",
        json!("{not json"),
        Some("{not json"),
    );
    assert!(events[1].data["error"]
        .as_str()
        .expect("error content")
        .starts_with("Invalid arguments for tool: lookup"));

    let (schema_failure, events) =
        dispatch_with_recorded_events(&registry, ToolCall::new("call-schema", "lookup", json!({})));
    assert!(schema_failure.is_error);
    assert_single_start_end(&events, "call-schema", "lookup", json!({}), None);
    assert!(events[1].data["error"]
        .as_str()
        .expect("error content")
        .starts_with("Invalid arguments for tool: lookup"));

    let (executor_failure, events) =
        dispatch_with_recorded_events(&registry, ToolCall::new("call-fails", "fails", json!({})));
    assert!(executor_failure.is_error);
    assert_eq!(
        executor_failure.content,
        json!("Tool error (fails): permission denied")
    );
    assert_single_start_end(&events, "call-fails", "fails", json!({}), None);
    assert_eq!(
        events[1].data["error"],
        json!("Tool error (fails): permission denied")
    );
}

#[test]
fn dispatch_returns_recoverable_errors_without_executing_invalid_calls() {
    let called = Arc::new(AtomicBool::new(false));
    let called_by_executor = called.clone();
    let definition = definition(
        "lookup",
        "Lookup values",
        json!({
            "type": "object",
            "properties": {"value": {"type": "integer"}},
            "required": ["value"],
            "additionalProperties": false
        }),
    );
    let mut registry = ToolRegistry::new();
    registry.register(RegisteredTool::new::<Value, _>(definition, move |_| {
        called_by_executor.store(true, Ordering::SeqCst);
        Ok(json!("unexpected"))
    }));

    let unknown = registry.dispatch(
        ToolCall::new("call-unknown", "missing", json!({})),
        ToolDispatchContext::default(),
    );
    assert_eq!(unknown.tool_call_id, "call-unknown");
    assert!(unknown.is_error);
    assert_eq!(unknown.content, json!("Unknown tool: missing"));

    let invalid_json = registry.dispatch(
        ToolCall::from_raw_arguments("call-json", "lookup", "{not json"),
        ToolDispatchContext::default(),
    );
    assert!(invalid_json.is_error);
    assert!(invalid_json
        .content
        .as_str()
        .expect("error content")
        .starts_with("Invalid arguments for tool: lookup"));

    let schema_failure = registry.dispatch(
        ToolCall::new("call-schema", "lookup", json!({})),
        ToolDispatchContext::default(),
    );
    assert!(schema_failure.is_error);
    assert!(schema_failure
        .content
        .as_str()
        .expect("error content")
        .starts_with("Invalid arguments for tool: lookup"));
    assert!(!called.load(Ordering::SeqCst));
}

#[test]
fn dispatch_converts_executor_failures_to_recoverable_tool_results() {
    let definition = definition("shell", "Run a command", json!({"type": "object"}));
    let mut registry = ToolRegistry::new();
    registry.register(RegisteredTool::new::<Value, _>(definition, |_| {
        Err(AdapterError::new(
            AdapterErrorKind::InvalidToolCall,
            "permission denied",
        ))
    }));

    let result = registry.dispatch(
        ToolCall::new("call-error", "shell", json!({})),
        ToolDispatchContext::default(),
    );

    assert_eq!(result.tool_call_id, "call-error");
    assert!(result.is_error);
    assert_eq!(
        result.content,
        json!("Tool error (shell): permission denied")
    );
}

#[test]
fn parallel_dispatch_preserves_result_order_and_tool_call_id_association() {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let events = Arc::new(Mutex::new(Vec::<ToolDispatchEvent>::new()));
    let mut registry = ToolRegistry::new();

    let first_release_rx = release_rx.clone();
    registry.register(RegisteredTool::new::<Value, _>(
        definition("first", "First tool", json!({"type": "object"})),
        move |_| {
            first_release_rx
                .lock()
                .expect("release receiver")
                .recv_timeout(Duration::from_secs(2))
                .map_err(|error| {
                    AdapterError::new(
                        AdapterErrorKind::InvalidToolCall,
                        format!("parallel dispatch did not start the second tool: {error}"),
                    )
                })?;
            Ok(json!("first-result"))
        },
    ));
    registry.register(RegisteredTool::new::<Value, _>(
        definition("second", "Second tool", json!({"type": "object"})),
        move |_| {
            release_tx.send(()).expect("release first tool");
            Ok(json!("second-result"))
        },
    ));

    let results = registry.dispatch_many(
        [
            ToolCall::new("call-first", "first", json!({})),
            ToolCall::new("call-second", "second", json!({})),
        ],
        ToolDispatchContext {
            supports_parallel_tool_calls: true,
            event_hook: Some(Arc::new({
                let events = events.clone();
                move |event| {
                    events.lock().expect("events").push(event);
                }
            })),
            ..ToolDispatchContext::default()
        },
    );

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].tool_call_id, "call-first");
    assert_eq!(results[0].content, json!("first-result"));
    assert!(!results[0].is_error);
    assert_eq!(results[1].tool_call_id, "call-second");
    assert_eq!(results[1].content, json!("second-result"));
    assert!(!results[1].is_error);

    let events = events.lock().expect("events");
    assert_eq!(events.len(), 4, "parallel dispatch events: {events:?}");
    for (tool_call_id, tool_name) in [("call-first", "first"), ("call-second", "second")] {
        let call_events = events
            .iter()
            .filter(|event| event.data.get("tool_call_id") == Some(&json!(tool_call_id)))
            .collect::<Vec<_>>();
        assert_eq!(call_events.len(), 2, "events for {tool_call_id}");
        assert_eq!(call_events[0].kind, EventKind::ToolCallStart);
        assert_eq!(call_events[1].kind, EventKind::ToolCallEnd);
        assert_eq!(call_events[0].data["tool_name"], json!(tool_name));
        assert_eq!(call_events[0].data["arguments"], json!({}));
        assert_eq!(call_events[1].data["tool_name"], json!(tool_name));
    }
}

#[test]
fn registered_tools_can_return_recoverable_error_outputs_directly() {
    let definition = definition("lookup", "Lookup values", json!({"type": "object"}));
    let mut registry = ToolRegistry::new();
    registry.register(RegisteredTool::new_with_executor(
        definition,
        Arc::new(|_| Ok(ToolExecutionOutput::error(json!("not found")))),
    ));

    let result = registry.dispatch(
        ToolCall::new("call-direct-error", "lookup", json!({})),
        ToolDispatchContext::default(),
    );

    assert_eq!(result.tool_call_id, "call-direct-error");
    assert!(result.is_error);
    assert_eq!(result.content, json!("not found"));
}

#[test]
fn openai_read_file_dispatch_formats_text_and_gates_image_results_by_capability() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    environment
        .write_file("notes.txt", "alpha\nbeta\ngamma\n")
        .expect("write notes");
    std::fs::write(tempdir.path().join("image.png"), PNG_BYTES).expect("write image");

    let registry = create_openai_profile("gpt-5.2").registry();
    let read_text = registry.dispatch(
        ToolCall::new(
            "call-read-text",
            "read_file",
            json!({"path": "notes.txt", "offset": 2, "limit": 1}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            capabilities: BTreeMap::from([
                ("vision".to_string(), false),
                ("multimodal".to_string(), false),
                ("image".to_string(), false),
            ]),
            ..ToolDispatchContext::default()
        },
    );

    assert!(!read_text.is_error);
    assert_eq!(read_text.content, json!("002 | beta"));
    assert_eq!(read_text.image_data, None);

    let missing = registry.dispatch(
        ToolCall::new("call-missing", "read_file", json!({"path": "missing.txt"})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(missing.is_error);
    assert_eq!(missing.content, json!("File not found: missing.txt"));

    let image_without_vision = registry.dispatch(
        ToolCall::new(
            "call-image-denied",
            "read_file",
            json!({"path": "image.png"}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            capabilities: BTreeMap::from([
                ("vision".to_string(), false),
                ("multimodal".to_string(), false),
                ("image".to_string(), false),
            ]),
            ..ToolDispatchContext::default()
        },
    );
    assert!(image_without_vision.is_error);
    assert_eq!(
        image_without_vision.content,
        json!("Binary file not supported: image.png")
    );
    assert_eq!(image_without_vision.image_data, None);

    let image_with_vision = registry.dispatch(
        ToolCall::new("call-image", "read_file", json!({"path": "image.png"})),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(!image_with_vision.is_error);
    assert!(image_with_vision
        .content
        .as_str()
        .expect("image summary")
        .contains("image.png"));
    assert_eq!(image_with_vision.image_data.as_deref(), Some(PNG_BYTES));
    assert_eq!(
        image_with_vision.image_media_type.as_deref(),
        Some("image/png")
    );
}

#[test]
fn gemini_write_edit_and_read_many_dispatch_use_provider_native_argument_shapes() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    let registry = create_gemini_profile("gemini-3.1-pro-preview").registry();

    let write_result = registry.dispatch(
        ToolCall::new(
            "call-write",
            "write_file",
            json!({"file_path": "notes.txt", "content": "alpha\nbeta\nalpha\n"}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(!write_result.is_error);
    assert_eq!(
        write_result.content,
        json!({"path": "notes.txt", "bytes_written": 17})
    );

    let duplicate_without_allow_multiple = registry.dispatch(
        ToolCall::new(
            "call-duplicate",
            "edit_file",
            json!({
                "file_path": "notes.txt",
                "instruction": "Replace one alpha",
                "old_string": "alpha",
                "new_string": "gamma"
            }),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(duplicate_without_allow_multiple.is_error);
    assert_eq!(
        duplicate_without_allow_multiple.content,
        json!("old_string is not unique in notes.txt: 2 matches")
    );

    let edit_result = registry.dispatch(
        ToolCall::new(
            "call-edit",
            "edit_file",
            json!({
                "file_path": "notes.txt",
                "instruction": "Replace all alpha with gamma",
                "old_string": "alpha",
                "new_string": "gamma",
                "allow_multiple": true
            }),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(!edit_result.is_error);
    assert_eq!(edit_result.content["path"], json!("notes.txt"));
    assert_eq!(edit_result.content["replacements"], json!(2));
    assert_eq!(
        environment
            .read_file("notes.txt", None, None)
            .expect("read edited file"),
        "gamma\nbeta\ngamma\n"
    );

    let not_found = registry.dispatch(
        ToolCall::new(
            "call-not-found",
            "edit_file",
            json!({
                "file_path": "notes.txt",
                "instruction": "Replace missing text",
                "old_string": "delta",
                "new_string": "epsilon"
            }),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(not_found.is_error);
    assert_eq!(
        not_found.content,
        json!("old_string not found in notes.txt")
    );

    environment
        .write_file("other.txt", "delta\n")
        .expect("write other file");
    let read_many = registry.dispatch(
        ToolCall::new(
            "call-read-many",
            "read_many_files",
            json!({"paths": ["notes.txt", "other.txt"]}),
        ),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(!read_many.is_error);
    assert_eq!(read_many.content["count"], json!(2));
    assert_eq!(read_many.content["files"][0]["path"], json!("notes.txt"));
    assert_eq!(
        read_many.content["files"][0]["content"],
        json!("001 | gamma\n002 | beta\n003 | gamma")
    );
    assert_eq!(read_many.content["files"][1]["path"], json!("other.txt"));
}

#[test]
fn grep_glob_and_list_dir_dispatch_return_structured_results_and_recoverable_errors() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    environment
        .write_file("notes.txt", "Alpha\nignored\nALPHA\n")
        .expect("write notes");
    environment
        .write_file("nested/ignore.md", "alpha\n")
        .expect("write nested ignore");
    environment
        .write_file("nested/readme.txt", "beta\n")
        .expect("write nested readme");

    let old_file = tempdir.path().join("old.txt");
    let new_file = tempdir.path().join("new.txt");
    environment.write_file(&old_file, "old").expect("write old");
    environment.write_file(&new_file, "new").expect("write new");
    filetime::set_file_mtime(&old_file, filetime::FileTime::from_unix_time(1, 0))
        .expect("old mtime");
    filetime::set_file_mtime(&new_file, filetime::FileTime::from_unix_time(2, 0))
        .expect("new mtime");
    filetime::set_file_mtime(
        tempdir.path().join("notes.txt"),
        filetime::FileTime::from_unix_time(0, 0),
    )
    .expect("notes mtime");

    let registry = create_gemini_profile("gemini-3.1-pro-preview").registry();
    let grep = registry.dispatch(
        ToolCall::new(
            "call-grep",
            "grep",
            json!({
                "pattern": "alpha",
                "path": ".",
                "glob_filter": "*.txt",
                "case_insensitive": true,
                "max_results": 2
            }),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(!grep.is_error);
    assert_eq!(
        grep.content,
        json!({
            "matches": [
                {"path": "notes.txt", "line_number": 1, "line": "Alpha"},
                {"path": "notes.txt", "line_number": 3, "line": "ALPHA"}
            ]
        })
    );

    let invalid_regex = registry.dispatch(
        ToolCall::new(
            "call-invalid-regex",
            "grep",
            json!({"pattern": "(", "path": "."}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(invalid_regex.is_error);
    assert!(invalid_regex
        .content
        .as_str()
        .expect("regex error")
        .starts_with("Invalid regex pattern:"));

    let glob = registry.dispatch(
        ToolCall::new(
            "call-glob",
            "glob",
            json!({"pattern": "*.txt", "path": "."}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(!glob.is_error);
    assert_eq!(glob.content, json!(["new.txt", "old.txt", "notes.txt"]));

    let missing_glob_base = registry.dispatch(
        ToolCall::new(
            "call-missing-glob",
            "glob",
            json!({"pattern": "*.txt", "path": "missing"}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(missing_glob_base.is_error);
    assert_eq!(missing_glob_base.content, json!("File not found: missing"));

    let list_dir = registry.dispatch(
        ToolCall::new("call-list", "list_dir", json!({"path": ".", "depth": 1})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(!list_dir.is_error);
    assert_eq!(list_dir.content["path"], json!("."));
    assert_eq!(list_dir.content["depth"], json!(1));
    assert!(list_dir.content["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .any(|entry| entry["name"] == json!("nested/readme.txt")));

    let list_missing = registry.dispatch(
        ToolCall::new("call-list-missing", "list_dir", json!({"path": "missing"})),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(list_missing.is_error);
    assert_eq!(list_missing.content, json!("File not found: missing"));
}

#[test]
fn shell_dispatch_uses_provider_defaults_explicit_timeouts_and_maximum_cap() {
    let backend = RecordingShellEnvironment::default();
    let calls = backend.calls.clone();
    let environment = ExecutionEnvironment::from_backend(backend);
    let mut config = SessionConfig::default();
    config.max_command_timeout_ms = 60_000;

    let openai = create_openai_profile("gpt-5.2").registry().dispatch(
        ToolCall::new("call-openai-shell", "shell", json!({"command": "openai"})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            config: config.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(!openai.is_error);
    assert_eq!(openai.content["stdout"], json!("openai"));

    let anthropic = create_anthropic_profile("claude-sonnet-4-5")
        .registry()
        .dispatch(
            ToolCall::new(
                "call-anthropic-shell",
                "shell",
                json!({"command": "anthropic"}),
            ),
            ToolDispatchContext {
                execution_environment: environment.clone(),
                config: config.clone(),
                ..ToolDispatchContext::default()
            },
        );
    assert!(!anthropic.is_error);
    assert_eq!(anthropic.content["stdout"], json!("anthropic"));

    let gemini = create_gemini_profile("gemini-3.1-pro-preview")
        .registry()
        .dispatch(
            ToolCall::new(
                "call-gemini-shell",
                "shell",
                json!({"command": "gemini", "timeout_ms": 40}),
            ),
            ToolDispatchContext {
                execution_environment: environment.clone(),
                config: config.clone(),
                ..ToolDispatchContext::default()
            },
        );
    assert!(!gemini.is_error);
    assert_eq!(gemini.content["stdout"], json!("gemini"));

    config.max_command_timeout_ms = 50;
    let capped = create_anthropic_profile("claude-sonnet-4-5")
        .registry()
        .dispatch(
            ToolCall::new("call-capped-shell", "shell", json!({"command": "capped"})),
            ToolDispatchContext {
                execution_environment: environment,
                config,
                ..ToolDispatchContext::default()
            },
        );
    assert!(!capped.is_error);

    assert_eq!(
        calls.lock().expect("shell calls").as_slice(),
        [
            ("openai".to_string(), 10_000),
            ("anthropic".to_string(), 60_000),
            ("gemini".to_string(), 40),
            ("capped".to_string(), 50),
        ]
    );
}

#[test]
fn shell_dispatch_truncates_structured_model_content_with_default_limit_and_keeps_raw_event_output()
{
    let full_stdout = format!("shell-start\n{}\nshell-end", "A".repeat(35_000));
    let backend = RecordingShellEnvironment::default();
    *backend.exec_result.lock().expect("exec result") = Some(ExecResult {
        stdout: full_stdout.clone(),
        stderr: String::new(),
        exit_code: 0,
        timed_out: false,
        duration_ms: 1,
    });
    let environment = ExecutionEnvironment::from_backend(backend);
    let registry = create_openai_profile("gpt-5.2").registry();

    let (result, events) = dispatch_with_context_recorded_events(
        &registry,
        ToolCall::new("call-shell-large", "shell", json!({"command": "large"})),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );

    assert!(!result.is_error);
    let model_content = result.content.as_str().expect("truncated shell model text");
    assert!(model_content.contains("[WARNING: Tool output was truncated."));
    assert!(model_content.contains("characters were removed from the middle"));
    assert!(model_content.contains("The full output is available in the event stream."));
    assert!(model_content.contains("shell-start"));
    assert!(model_content.contains("shell-end"));
    assert!(model_content.len() < full_stdout.len());
    assert_single_start_end(
        &events,
        "call-shell-large",
        "shell",
        json!({"command": "large"}),
        None,
    );
    assert_eq!(events[1].data["output"]["stdout"], json!(full_stdout));
    assert_eq!(events[1].data.get("model_content"), Some(&result.content));
}

#[test]
fn shell_dispatch_truncates_structured_error_payloads_with_overrides_and_keeps_raw_event_error() {
    let full_stderr = format!("error-start\n{}\nerror-end", "E".repeat(2_000));
    let backend = RecordingShellEnvironment::default();
    *backend.exec_result.lock().expect("exec result") = Some(ExecResult {
        stdout: String::new(),
        stderr: full_stderr.clone(),
        exit_code: 2,
        timed_out: false,
        duration_ms: 1,
    });
    let environment = ExecutionEnvironment::from_backend(backend);
    let registry = create_openai_profile("gpt-5.2").registry();
    let mut config = SessionConfig::default();
    config.tool_output_limits.insert("shell".to_string(), 120);

    let (result, events) = dispatch_with_context_recorded_events(
        &registry,
        ToolCall::new("call-shell-error", "shell", json!({"command": "fail"})),
        ToolDispatchContext {
            execution_environment: environment,
            config,
            ..ToolDispatchContext::default()
        },
    );

    assert!(result.is_error);
    let model_content = result.content.as_str().expect("truncated shell error text");
    assert!(model_content.contains("[WARNING: Tool output was truncated."));
    assert!(model_content.contains("characters were removed from the middle"));
    assert!(model_content.contains("The full output is available in the event stream."));
    assert!(model_content.contains("error-start"));
    assert!(model_content.contains("error-end"));
    assert!(model_content.len() < full_stderr.len());
    assert_single_start_end(
        &events,
        "call-shell-error",
        "shell",
        json!({"command": "fail"}),
        None,
    );
    assert_eq!(events[1].data["error"]["stderr"], json!(full_stderr));
    assert_eq!(events[1].data.get("model_content"), Some(&result.content));
}

#[test]
fn glob_dispatch_truncates_structured_model_content_with_default_and_override_limits() {
    let glob_matches = (0..900)
        .map(|index| format!("file-{index:04}.txt"))
        .collect::<Vec<_>>();
    let backend = RecordingShellEnvironment::default();
    *backend.glob_matches.lock().expect("glob matches") = Some(glob_matches.clone());
    let environment = ExecutionEnvironment::from_backend(backend);
    let registry = create_openai_profile("gpt-5.2").registry();

    let (default_result, default_events) = dispatch_with_context_recorded_events(
        &registry,
        ToolCall::new("call-glob-default", "glob", json!({"pattern": "**/*.txt"})),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );

    assert!(!default_result.is_error);
    let default_content = default_result
        .content
        .as_str()
        .expect("default glob truncation text");
    assert!(default_content.contains("[WARNING: Tool output was truncated. 400 lines omitted."));
    assert!(default_content.contains("The full output is available in the event stream."));
    assert!(default_content.starts_with("file-0000.txt"));
    assert!(default_content.contains("file-0899.txt"));
    assert_single_start_end(
        &default_events,
        "call-glob-default",
        "glob",
        json!({"pattern": "**/*.txt"}),
        None,
    );
    assert_eq!(default_events[1].data["output"], json!(glob_matches));
    assert_eq!(
        default_events[1].data.get("model_content"),
        Some(&default_result.content)
    );

    let backend = RecordingShellEnvironment::default();
    *backend.glob_matches.lock().expect("glob matches") = Some(glob_matches);
    let environment = ExecutionEnvironment::from_backend(backend);
    let mut config = SessionConfig::default();
    config.tool_output_limits.insert("glob".to_string(), 120);
    let override_result = registry.dispatch(
        ToolCall::new("call-glob-override", "glob", json!({"pattern": "**/*.txt"})),
        ToolDispatchContext {
            execution_environment: environment,
            config,
            ..ToolDispatchContext::default()
        },
    );

    assert!(!override_result.is_error);
    let override_content = override_result
        .content
        .as_str()
        .expect("override glob truncation text");
    assert!(override_content.starts_with("[WARNING: Tool output was truncated. First "));
    assert!(override_content.contains("characters were removed."));
    assert!(override_content.contains("file-0899.txt"));
    assert!(override_content.len() < default_content.len());
}

#[test]
fn registry_preserves_unified_active_tool_executors_for_compatibility() {
    let mut registry = ToolRegistry::new();
    registry.register(
        Tool::active_with_schema(
            "lookup",
            Some("Lookup values".to_string()),
            Some(json!({"type": "object"})),
            |invocation| {
                Ok(json!({
                    "tool_call_id": invocation.tool_call_id,
                    "arguments": invocation.arguments
                }))
            },
        )
        .expect("active tool"),
    );

    let result = registry.dispatch(
        ToolCall::new("call-active", "lookup", json!({"value": 7})),
        ToolDispatchContext::default(),
    );

    assert_eq!(result.tool_call_id, "call-active");
    assert!(!result.is_error);
    assert_eq!(
        result.content,
        json!({
            "tool_call_id": "call-active",
            "arguments": {"value": 7}
        })
    );
}

#[test]
fn provider_subagent_tools_are_executable_registry_tools() {
    let result = create_openai_profile("gpt-5.2").registry().dispatch(
        ToolCall::new(
            "spawn-without-session",
            "spawn_agent",
            json!({"task": "Investigate"}),
        ),
        ToolDispatchContext::default(),
    );

    assert!(result.is_error);
    assert_eq!(
        result.content,
        json!("subagent runtime is unavailable for spawn_agent")
    );
}

#[test]
fn session_dispatches_subagent_tools_through_provider_registry() {
    let adapter = Arc::new(HappySubagentAdapter::default());
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let workspace = tempdir().expect("workspace");
    let mut session = Session::new(
        subagent_enabled_profile(),
        ExecutionEnvironment::local(workspace.path()),
        SessionConfig {
            max_subagent_depth: 1,
            ..SessionConfig::default()
        },
    );

    session
        .process_input(&client, "parent task")
        .expect("subagent tool flow");

    assert_eq!(session.state, SessionState::Idle);
    assert_eq!(session.active_subagents.len(), 1);
    assert_eq!(
        session
            .active_subagents
            .values()
            .next()
            .expect("subagent handle")
            .status,
        SubAgentStatus::Completed
    );
    let tool_results = session_tool_results(&session);
    assert_eq!(
        tool_results
            .iter()
            .map(|result| result.tool_call_id.as_str())
            .collect::<Vec<_>>(),
        ["spawn-1", "send-1", "wait-1", "close-1"]
    );
    let wait_result = tool_results
        .iter()
        .find(|result| result.tool_call_id == "wait-1")
        .expect("wait result");
    assert!(!wait_result.is_error);
    assert_eq!(wait_result.content["status"], json!("completed"));
    assert_eq!(wait_result.content["success"], json!(true));
    assert_eq!(wait_result.content["output"], json!("child response 2"));
    assert_eq!(wait_result.content["turns_used"], json!(4));

    let requests = adapter.requests();
    assert_eq!(requests.len(), 7);
    let child_initial = requests
        .iter()
        .find(|request| {
            has_message_text(request, "Investigate the repository")
                && !has_message_text(request, "Please continue")
        })
        .expect("child initial request");
    assert_eq!(
        child_initial.messages[1].text(),
        "Investigate the repository"
    );
    let child_follow_up = requests
        .iter()
        .find(|request| has_message_text(request, "Please continue"))
        .expect("child follow-up request");
    assert_eq!(child_follow_up.messages[3].text(), "Please continue");
}

#[test]
fn session_spawn_agent_allowed_model_override_changes_only_child_model() {
    let adapter = Arc::new(ModelOverrideSubagentAdapter::default());
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let workspace = tempdir().expect("workspace");
    let mut profile = subagent_enabled_profile();
    profile
        .subagent_model_overrides
        .push("child-model".to_string());
    let parent_model = profile.model.clone();
    let parent_id = profile.id.clone();
    let parent_request_provider = profile.request_provider.clone();
    let parent_tools = profile.tools();
    let parent_provider_options = profile.provider_options();
    let parent_capabilities = profile.capability_flags().clone();
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::local(workspace.path()),
        SessionConfig {
            max_subagent_depth: 1,
            ..SessionConfig::default()
        },
    );

    session
        .process_input(&client, "parent task")
        .expect("subagent model override flow");

    assert_eq!(session.state, SessionState::Idle);
    assert_eq!(session.provider_profile.model, parent_model);
    assert_eq!(
        session.provider_profile.capability_flags(),
        &parent_capabilities
    );
    assert_eq!(
        session.provider_profile.provider_options(),
        parent_provider_options
    );

    let handle = session
        .active_subagents
        .values()
        .next()
        .expect("subagent handle");
    assert_eq!(handle.status, SubAgentStatus::Completed);
    let child_profile = handle
        .provider_profile
        .as_ref()
        .expect("child provider profile");
    assert_eq!(child_profile.model, "child-model");
    assert_eq!(child_profile.id, parent_id);
    assert_eq!(child_profile.request_provider, parent_request_provider);
    assert_eq!(child_profile.tools(), parent_tools);
    assert_eq!(child_profile.provider_options(), parent_provider_options);
    assert_eq!(child_profile.capability_flags(), &parent_capabilities);

    let requests = adapter.requests();
    let child_request = requests
        .iter()
        .find(|request| has_message_text(request, "Investigate with child model"))
        .expect("child request");
    assert_eq!(child_request.model, "child-model");
    assert_eq!(child_request.provider.as_deref(), Some("fake-provider"));
    assert_eq!(
        child_request.provider_options,
        BTreeMap::from([("fake-provider".to_string(), json!({}))])
    );
    assert_eq!(
        child_request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        parent_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
    );

    let wait_result = session_tool_results(&session)
        .into_iter()
        .find(|result| result.tool_call_id == "wait-model-override")
        .expect("wait result");
    assert!(!wait_result.is_error);
    assert_eq!(wait_result.content["status"], json!("completed"));
    assert_eq!(wait_result.content["output"], json!("child response"));
}

#[test]
fn subagent_wait_returns_recoverable_error_when_child_processing_fails() {
    let adapter = Arc::new(ChildProcessingFailureAdapter::default());
    let client_adapter: Arc<dyn ProviderAdapter> = adapter;
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let workspace = tempdir().expect("workspace");
    let mut session = Session::new(
        subagent_enabled_profile(),
        ExecutionEnvironment::local(workspace.path()),
        SessionConfig {
            max_subagent_depth: 1,
            ..SessionConfig::default()
        },
    );

    session
        .process_input(&client, "parent task")
        .expect("parent continues after child processing failure");

    assert_eq!(session.state, SessionState::Idle);
    assert_eq!(session.active_subagents.len(), 1);
    let handle = session
        .active_subagents
        .values()
        .next()
        .expect("subagent handle");
    assert_eq!(handle.status, SubAgentStatus::Failed);

    let tool_results = session_tool_results(&session);
    assert_eq!(
        tool_results
            .iter()
            .map(|result| result.tool_call_id.as_str())
            .collect::<Vec<_>>(),
        ["spawn-failing-child", "wait-failing-child"]
    );
    let wait_result = tool_results
        .iter()
        .find(|result| result.tool_call_id == "wait-failing-child")
        .expect("wait result");
    assert!(wait_result.is_error);
    assert_eq!(
        wait_result.content["agent_id"],
        json!(handle.id.to_string())
    );
    assert_eq!(wait_result.content["status"], json!("failed"));
    assert_eq!(wait_result.content["success"], json!(false));
    assert_eq!(wait_result.content["output"], Value::Null);
    assert_eq!(wait_result.content["turns_used"], json!(1));
    assert_eq!(wait_result.content["error"], json!("child provider failed"));
}

#[test]
fn subagent_tool_failures_are_recoverable_tool_results() {
    let adapter = Arc::new(RecoverableSubagentFailureAdapter::default());
    let client_adapter: Arc<dyn ProviderAdapter> = adapter;
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let workspace = tempdir().expect("workspace");
    let mut session = Session::new(
        subagent_enabled_profile(),
        ExecutionEnvironment::local(workspace.path()),
        SessionConfig {
            max_subagent_depth: 1,
            ..SessionConfig::default()
        },
    );

    session
        .process_input(&client, "parent task")
        .expect("recoverable subagent failures");

    assert_eq!(session.state, SessionState::Idle);
    assert!(session.active_subagents.is_empty());
    let tool_results = session_tool_results(&session);
    assert_eq!(tool_results.len(), 3);
    assert!(tool_results.iter().all(|result| result.is_error));
    assert!(tool_results[0]
        .content
        .as_str()
        .expect("unknown agent error")
        .contains("Unknown child agent"));
    assert!(tool_results[1]
        .content
        .as_str()
        .expect("working dir error")
        .contains("working_dir"));
    assert!(tool_results[2]
        .content
        .as_str()
        .expect("model override error")
        .contains("model override is not allowed"));
    assert!(tool_results[2]
        .content
        .as_str()
        .expect("model override error")
        .contains("requested_model=\"other-model\""));
    assert!(tool_results[2]
        .content
        .as_str()
        .expect("model override error")
        .contains("parent_model=\"fake-model\""));
    assert!(tool_results[2]
        .content
        .as_str()
        .expect("model override error")
        .contains("provider=\"fake-provider\""));
}

#[test]
fn openai_apply_patch_dispatch_applies_add_delete_update_rename_and_eof_marker() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    environment
        .write_file(
            "src/app.py",
            "def main():\n    print(\"Hello\")\n    return 0\n# trailing\n",
        )
        .expect("write app");
    environment
        .write_file(
            "src/old_name.py",
            "import os\nimport old_dep\n\ndef main():\n    return 0\n",
        )
        .expect("write old name");
    environment
        .write_file("src/remove_me.txt", "delete me\n")
        .expect("write remove me");

    let patch = [
        "*** Begin Patch",
        "*** Add File: src/new_file.py",
        "+first line",
        "+second line",
        "*** Delete File: src/remove_me.txt",
        "*** Update File: src/app.py",
        "@@ def main():",
        " def main():",
        "     print(\"Hello\")",
        "-    return 0",
        "+    return 1",
        "@@ # trailing",
        "-# trailing",
        "+# updated trailing",
        "*** End of File",
        "*** Update File: src/old_name.py",
        "*** Move to: src/new_name.py",
        "@@ import old_dep",
        " import os",
        "-import old_dep",
        "+import new_dep",
        "*** End Patch",
    ]
    .join("\n");

    let result = create_openai_profile("gpt-5.2").registry().dispatch(
        ToolCall::new("call-apply", "apply_patch", json!({"patch": patch})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );

    assert!(!result.is_error);
    assert_eq!(
        result.content,
        json!([
            {"operation": "add", "path": "src/new_file.py"},
            {"operation": "delete", "path": "src/remove_me.txt"},
            {"operation": "update", "path": "src/app.py", "hunks": 2},
            {
                "operation": "update+rename",
                "path": "src/old_name.py",
                "new_path": "src/new_name.py",
                "hunks": 1
            }
        ])
    );
    assert_eq!(
        environment
            .read_file("src/new_file.py", None, None)
            .expect("read new file"),
        "first line\nsecond line\n"
    );
    assert!(!environment.file_exists("src/remove_me.txt"));
    assert_eq!(
        environment
            .read_file("src/app.py", None, None)
            .expect("read app"),
        "def main():\n    print(\"Hello\")\n    return 1\n# updated trailing\n"
    );
    assert_eq!(
        environment
            .read_file("src/new_name.py", None, None)
            .expect("read renamed"),
        "import os\nimport new_dep\n\ndef main():\n    return 0\n"
    );
    assert!(!environment.file_exists("src/old_name.py"));
}

#[test]
fn openai_apply_patch_dispatch_creates_empty_file() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    let patch = [
        "*** Begin Patch",
        "*** Add File: src/empty.py",
        "*** End Patch",
    ]
    .join("\n");

    let result = create_openai_profile("gpt-5.2").registry().dispatch(
        ToolCall::new("call-empty-add", "apply_patch", json!({"patch": patch})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );

    assert!(!result.is_error);
    assert_eq!(
        result.content,
        json!([{"operation": "add", "path": "src/empty.py"}])
    );
    assert_eq!(
        environment
            .read_file("src/empty.py", None, None)
            .expect("read empty file"),
        ""
    );
}

#[test]
fn openai_apply_patch_uses_context_hints_and_fuzzy_matching() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    environment
        .write_file(
            "duplicate.py",
            "def one():\n    value = \"alpha\"\n    return value\n\n\
             def two():\n    value = \"alpha\"\n    return value\n",
        )
        .expect("write duplicate");
    environment
        .write_file("notes.txt", "answer = \"Cost \u{2013} benefit\"\n")
        .expect("write notes");

    let patch = [
        "*** Begin Patch",
        "*** Update File: duplicate.py",
        "@@ def two():",
        "-    value = \"alpha\"",
        "+    value = \"beta\"",
        "*** Update File: notes.txt",
        "@@ answer = \"Cost - benefit\"",
        "-answer   =   \"Cost - benefit\"",
        "+answer = \"Cost - benefit!\"",
        "*** End Patch",
    ]
    .join("\n");

    let result = create_openai_profile("gpt-5.2").registry().dispatch(
        ToolCall::new("call-fuzzy", "apply_patch", json!({"patch": patch})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );

    assert!(!result.is_error);
    assert_eq!(
        result.content,
        json!([
            {"operation": "update", "path": "duplicate.py", "hunks": 1},
            {"operation": "update", "path": "notes.txt", "hunks": 1}
        ])
    );
    assert_eq!(
        environment
            .read_file("duplicate.py", None, None)
            .expect("read duplicate"),
        "def one():\n    value = \"alpha\"\n    return value\n\n\
         def two():\n    value = \"beta\"\n    return value\n"
    );
    assert_eq!(
        environment
            .read_file("notes.txt", None, None)
            .expect("read notes"),
        "answer = \"Cost - benefit!\"\n"
    );
}

#[test]
fn openai_apply_patch_returns_recoverable_parse_missing_and_verification_errors() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    environment
        .write_file("verify.txt", "old\n")
        .expect("write verify");
    let registry = create_openai_profile("gpt-5.2").registry();

    let parse_error = registry.dispatch(
        ToolCall::new(
            "call-parse",
            "apply_patch",
            json!({"patch": "*** Update File: verify.txt\n@@ verify.txt\n-old\n+new\n*** End Patch"}),
        ),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(parse_error.is_error);
    assert_eq!(
        parse_error.content,
        json!("Patch parse error: missing *** Begin Patch marker")
    );
    assert_eq!(
        environment
            .read_file("verify.txt", None, None)
            .expect("read after parse error"),
        "old\n"
    );

    let missing_patch = [
        "*** Begin Patch",
        "*** Update File: missing.txt",
        "@@ missing.txt",
        "-missing",
        "+present",
        "*** End Patch",
    ]
    .join("\n");
    let missing_error = registry.dispatch(
        ToolCall::new(
            "call-missing",
            "apply_patch",
            json!({"patch": missing_patch}),
        ),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(missing_error.is_error);
    assert_eq!(missing_error.content, json!("File not found: missing.txt"));

    let backend = RecordingPatchEnvironment::default();
    let files = backend.files.clone();
    let noop_writes = backend.noop_writes.clone();
    files
        .lock()
        .expect("files")
        .insert("verify.txt".to_string(), "old\n".to_string());
    noop_writes.store(true, Ordering::SeqCst);
    let verification_environment = ExecutionEnvironment::from_backend(backend);

    let verification_patch = [
        "*** Begin Patch",
        "*** Update File: verify.txt",
        "@@ verify.txt",
        "-old",
        "+new",
        "*** End Patch",
    ]
    .join("\n");
    let verification_error = registry.dispatch(
        ToolCall::new(
            "call-verification",
            "apply_patch",
            json!({"patch": verification_patch}),
        ),
        ToolDispatchContext {
            execution_environment: verification_environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(verification_error.is_error);
    assert_eq!(
        verification_error.content,
        json!("Patch verification failed: verify.txt")
    );
    assert_eq!(
        files.lock().expect("files").get("verify.txt"),
        Some(&"old\n".to_string())
    );
}

#[test]
fn openai_apply_patch_routes_filesystem_operations_through_execution_environment() {
    let backend = RecordingPatchEnvironment::default();
    let files = backend.files.clone();
    let calls = backend.calls.clone();
    files.lock().expect("files").extend([
        (
            "src/app.py".to_string(),
            "def main():\n    print(\"Hello\")\n    return 0\n# trailing\n".to_string(),
        ),
        (
            "src/old_name.py".to_string(),
            "import os\nimport old_dep\n\ndef main():\n    return 0\n".to_string(),
        ),
        ("src/remove_me.txt".to_string(), "delete me\n".to_string()),
    ]);
    let environment = ExecutionEnvironment::from_backend(backend);

    let patch = [
        "*** Begin Patch",
        "*** Add File: src/new_file.py",
        "+first line",
        "+second line",
        "*** Delete File: src/remove_me.txt",
        "*** Update File: src/app.py",
        "@@ def main():",
        " def main():",
        "     print(\"Hello\")",
        "-    return 0",
        "+    return 1",
        "*** Update File: src/old_name.py",
        "*** Move to: src/new_name.py",
        "@@ import old_dep",
        " import os",
        "-import old_dep",
        "+import new_dep",
        "*** End Patch",
    ]
    .join("\n");

    let result = create_openai_profile("gpt-5.2").registry().dispatch(
        ToolCall::new("call-recording", "apply_patch", json!({"patch": patch})),
        ToolDispatchContext {
            execution_environment: environment,
            ..ToolDispatchContext::default()
        },
    );

    assert!(!result.is_error);
    assert_eq!(
        files.lock().expect("files").clone(),
        BTreeMap::from([
            (
                "src/app.py".to_string(),
                "def main():\n    print(\"Hello\")\n    return 1\n# trailing\n".to_string()
            ),
            (
                "src/new_file.py".to_string(),
                "first line\nsecond line\n".to_string()
            ),
            (
                "src/new_name.py".to_string(),
                "import os\nimport new_dep\n\ndef main():\n    return 0\n".to_string()
            ),
        ])
    );
    let calls = calls.lock().expect("calls");
    assert!(calls.contains(&vec![
        "delete_file".to_string(),
        "src/remove_me.txt".to_string()
    ]));
    assert!(calls.contains(&vec![
        "rename_file".to_string(),
        "src/old_name.py".to_string(),
        "src/new_name.py".to_string()
    ]));
    assert!(calls.contains(&vec!["read_file".to_string(), "src/app.py".to_string()]));
    assert!(calls.contains(&vec![
        "read_file".to_string(),
        "src/new_name.py".to_string()
    ]));
}

#[test]
fn openai_apply_patch_dispatch_applies_pure_rename() {
    let tempdir = tempdir().expect("tempdir");
    let environment = ExecutionEnvironment::local(tempdir.path());
    environment
        .write_file("src/old_name.py", "print('old')\n")
        .expect("write rename source");

    let patch = [
        "*** Begin Patch",
        "*** Update File: src/old_name.py",
        "*** Move to: src/new_name.py",
        "*** End Patch",
    ]
    .join("\n");

    let result = create_openai_profile("gpt-5.2").registry().dispatch(
        ToolCall::new("call-rename", "apply_patch", json!({"patch": patch})),
        ToolDispatchContext {
            execution_environment: environment.clone(),
            ..ToolDispatchContext::default()
        },
    );

    assert!(!result.is_error);
    assert_eq!(
        result.content,
        json!([{
            "operation": "rename",
            "path": "src/old_name.py",
            "new_path": "src/new_name.py"
        }])
    );
    assert!(!environment.file_exists("src/old_name.py"));
    assert_eq!(
        environment
            .read_file("src/new_name.py", None, None)
            .expect("read renamed"),
        "print('old')\n"
    );
}

#[test]
fn openai_apply_patch_pure_rename_returns_recoverable_errors() {
    let registry = create_openai_profile("gpt-5.2").registry();

    let missing_tempdir = tempdir().expect("tempdir");
    let missing_environment = ExecutionEnvironment::local(missing_tempdir.path());
    let missing_patch = [
        "*** Begin Patch",
        "*** Update File: src/missing.py",
        "*** Move to: src/new_name.py",
        "*** End Patch",
    ]
    .join("\n");
    let missing_result = registry.dispatch(
        ToolCall::new(
            "call-rename-missing",
            "apply_patch",
            json!({"patch": missing_patch}),
        ),
        ToolDispatchContext {
            execution_environment: missing_environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(missing_result.is_error);
    assert_eq!(
        missing_result.content,
        json!("File not found: src/missing.py")
    );

    let existing_tempdir = tempdir().expect("tempdir");
    let existing_environment = ExecutionEnvironment::local(existing_tempdir.path());
    existing_environment
        .write_file("src/old_name.py", "old\n")
        .expect("write old");
    existing_environment
        .write_file("src/new_name.py", "new\n")
        .expect("write new");
    let existing_patch = [
        "*** Begin Patch",
        "*** Update File: src/old_name.py",
        "*** Move to: src/new_name.py",
        "*** End Patch",
    ]
    .join("\n");
    let existing_result = registry.dispatch(
        ToolCall::new(
            "call-rename-existing",
            "apply_patch",
            json!({"patch": existing_patch}),
        ),
        ToolDispatchContext {
            execution_environment: existing_environment.clone(),
            ..ToolDispatchContext::default()
        },
    );
    assert!(existing_result.is_error);
    assert_eq!(
        existing_result.content,
        json!("File already exists: src/new_name.py")
    );
    assert_eq!(
        existing_environment
            .read_file("src/old_name.py", None, None)
            .expect("read old"),
        "old\n"
    );
    assert_eq!(
        existing_environment
            .read_file("src/new_name.py", None, None)
            .expect("read new"),
        "new\n"
    );

    let backend = RecordingPatchEnvironment::default();
    let files = backend.files.clone();
    let fail_renames = backend.fail_renames.clone();
    files
        .lock()
        .expect("files")
        .insert("src/old_name.py".to_string(), "old\n".to_string());
    fail_renames.store(true, Ordering::SeqCst);
    let failing_environment = ExecutionEnvironment::from_backend(backend);
    let failing_result = registry.dispatch(
        ToolCall::new(
            "call-rename-failure",
            "apply_patch",
            json!({"patch": existing_patch}),
        ),
        ToolDispatchContext {
            execution_environment: failing_environment,
            ..ToolDispatchContext::default()
        },
    );
    assert!(failing_result.is_error);
    assert_eq!(
        failing_result.content,
        json!(
            "Patch apply error: failed to rename src/old_name.py to src/new_name.py: operation failed: rename failed"
        )
    );
    assert_eq!(
        files.lock().expect("files").clone(),
        BTreeMap::from([("src/old_name.py".to_string(), "old\n".to_string())])
    );
}

#[derive(Debug, Default)]
struct RecordingPatchEnvironment {
    files: Arc<Mutex<BTreeMap<String, String>>>,
    calls: Arc<Mutex<Vec<Vec<String>>>>,
    noop_writes: Arc<AtomicBool>,
    fail_renames: Arc<AtomicBool>,
}

impl RecordingPatchEnvironment {
    fn key(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn record(&self, call: Vec<String>) {
        self.calls.lock().expect("calls").push(call);
    }
}

impl ExecutionEnvironmentBackend for RecordingPatchEnvironment {
    fn read_file(
        &self,
        path: &Path,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> EnvironmentResult<String> {
        let key = Self::key(path);
        self.record(vec!["read_file".to_string(), key.clone()]);
        let content = self
            .files
            .lock()
            .expect("files")
            .get(&key)
            .cloned()
            .ok_or_else(|| EnvironmentError::FileNotFound(PathBuf::from(&key)))?;
        if matches!(offset, Some(0)) {
            return Err(EnvironmentError::InvalidInput(
                "offset must be at least 1".to_string(),
            ));
        }
        let Some(start) = offset.map(|value| value.saturating_sub(1)) else {
            return Ok(content);
        };
        let end = limit.map(|value| start.saturating_add(value));
        Ok(content
            .split_inclusive('\n')
            .enumerate()
            .filter_map(|(index, line)| {
                if index < start || end.is_some_and(|end| index >= end) {
                    None
                } else {
                    Some(line)
                }
            })
            .collect())
    }

    fn read_file_bytes(&self, path: &Path) -> EnvironmentResult<Vec<u8>> {
        Ok(self.read_file(path, None, None)?.into_bytes())
    }

    fn write_file(&self, path: &Path, content: &str) -> EnvironmentResult<()> {
        let key = Self::key(path);
        self.record(vec!["write_file".to_string(), key.clone()]);
        if !self.noop_writes.load(Ordering::SeqCst) {
            self.files
                .lock()
                .expect("files")
                .insert(key, content.to_string());
        }
        Ok(())
    }

    fn file_exists(&self, path: &Path) -> bool {
        let key = Self::key(path);
        self.record(vec!["file_exists".to_string(), key.clone()]);
        self.files.lock().expect("files").contains_key(&key)
    }

    fn is_directory(&self, path: &Path) -> bool {
        let key = Self::key(path);
        self.record(vec!["is_directory".to_string(), key]);
        false
    }

    fn delete_file(&self, path: &Path) -> EnvironmentResult<()> {
        let key = Self::key(path);
        self.record(vec!["delete_file".to_string(), key.clone()]);
        self.files
            .lock()
            .expect("files")
            .remove(&key)
            .map(|_| ())
            .ok_or_else(|| EnvironmentError::FileNotFound(PathBuf::from(key)))
    }

    fn rename_file(&self, source_path: &Path, destination_path: &Path) -> EnvironmentResult<()> {
        let source = Self::key(source_path);
        let destination = Self::key(destination_path);
        self.record(vec![
            "rename_file".to_string(),
            source.clone(),
            destination.clone(),
        ]);
        if self.fail_renames.load(Ordering::SeqCst) {
            return Err(EnvironmentError::Other("rename failed".to_string()));
        }
        let mut files = self.files.lock().expect("files");
        if !files.contains_key(&source) {
            return Err(EnvironmentError::FileNotFound(PathBuf::from(source)));
        }
        if files.contains_key(&destination) {
            return Err(EnvironmentError::AlreadyExists(PathBuf::from(destination)));
        }
        let content = files.remove(&source).expect("source checked");
        files.insert(destination, content);
        Ok(())
    }

    fn list_directory(&self, _path: &Path, _depth: usize) -> EnvironmentResult<Vec<DirEntry>> {
        Ok(Vec::new())
    }

    fn exec_command(
        &self,
        _command: &str,
        _options: CommandOptions,
    ) -> EnvironmentResult<ExecResult> {
        Ok(ExecResult::default())
    }

    fn grep(
        &self,
        _pattern: &str,
        _path: &Path,
        _options: &GrepOptions,
    ) -> EnvironmentResult<String> {
        Ok(String::new())
    }

    fn glob(&self, _pattern: &str, _path: &Path) -> EnvironmentResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn initialize(&self) -> EnvironmentResult<()> {
        Ok(())
    }

    fn cleanup(&self) -> EnvironmentResult<()> {
        Ok(())
    }

    fn working_directory(&self) -> String {
        "workspace".to_string()
    }

    fn platform(&self) -> String {
        "test".to_string()
    }

    fn os_version(&self) -> String {
        "test".to_string()
    }
}

#[derive(Debug, Default)]
struct RecordingShellEnvironment {
    calls: Arc<Mutex<Vec<(String, u64)>>>,
    exec_result: Arc<Mutex<Option<ExecResult>>>,
    grep_output: Arc<Mutex<Option<String>>>,
    glob_matches: Arc<Mutex<Option<Vec<String>>>>,
}

impl ExecutionEnvironmentBackend for RecordingShellEnvironment {
    fn read_file(
        &self,
        _path: &Path,
        _offset: Option<usize>,
        _limit: Option<usize>,
    ) -> EnvironmentResult<String> {
        Ok(String::new())
    }

    fn read_file_bytes(&self, _path: &Path) -> EnvironmentResult<Vec<u8>> {
        Ok(Vec::new())
    }

    fn write_file(&self, _path: &Path, _content: &str) -> EnvironmentResult<()> {
        Ok(())
    }

    fn file_exists(&self, _path: &Path) -> bool {
        false
    }

    fn is_directory(&self, _path: &Path) -> bool {
        false
    }

    fn delete_file(&self, _path: &Path) -> EnvironmentResult<()> {
        Ok(())
    }

    fn rename_file(&self, _source_path: &Path, _destination_path: &Path) -> EnvironmentResult<()> {
        Ok(())
    }

    fn list_directory(&self, _path: &Path, _depth: usize) -> EnvironmentResult<Vec<DirEntry>> {
        Ok(Vec::new())
    }

    fn exec_command(
        &self,
        command: &str,
        options: CommandOptions,
    ) -> EnvironmentResult<ExecResult> {
        self.calls.lock().expect("calls").push((
            command.to_string(),
            options.timeout_ms.expect("tool sets timeout"),
        ));
        Ok(self
            .exec_result
            .lock()
            .expect("exec result")
            .clone()
            .unwrap_or_else(|| ExecResult {
                stdout: command.to_string(),
                stderr: String::new(),
                exit_code: 0,
                timed_out: false,
                duration_ms: 1,
            }))
    }

    fn grep(
        &self,
        _pattern: &str,
        _path: &Path,
        _options: &GrepOptions,
    ) -> EnvironmentResult<String> {
        Ok(self
            .grep_output
            .lock()
            .expect("grep output")
            .clone()
            .unwrap_or_default())
    }

    fn glob(&self, _pattern: &str, _path: &Path) -> EnvironmentResult<Vec<String>> {
        Ok(self
            .glob_matches
            .lock()
            .expect("glob matches")
            .clone()
            .unwrap_or_default())
    }

    fn initialize(&self) -> EnvironmentResult<()> {
        Ok(())
    }

    fn cleanup(&self) -> EnvironmentResult<()> {
        Ok(())
    }

    fn working_directory(&self) -> String {
        ".".to_string()
    }

    fn platform(&self) -> String {
        "test".to_string()
    }

    fn os_version(&self) -> String {
        "test".to_string()
    }
}
