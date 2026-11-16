//! The ACP engine against a scripted fake agent over the shared stdio
//! transport: negotiation, streaming, permissions, cancellation,
//! instruction channels and session continuity.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use agent_client_protocol::v1::schema::StopReason;
use serde_json::{json, Value};
use spark_agent_adapter::acp::{self, AcpSessionRecord, AcpTurn};
use spark_agent_adapter::{
    AgentTurnEventSink, AgentTurnRequest, AssistantTurn, HistoryTurn, UserTurn,
    AGENT_INSTRUCTIONS_METADATA_KEY, PROJECT_WORK_DIR_METADATA_KEY,
};
use spark_common::events::{TurnStreamEvent, TurnStreamEventKind};

const FAKE: &str = env!("CARGO_BIN_EXE_spark-agent-fake-acp");

/// A request for `acp:fake`, registered with `registration` merged over a
/// default that runs the fake with `script`; the fake logs to `dir/acp.log`.
fn request(dir: &Path, script: Value, registration: Value) -> AgentTurnRequest {
    let mut script = script;
    script["log"] = json!(dir.join("acp.log"));
    let mut harness = json!({
        "name": "Fake ACP",
        "executable": FAKE,
        "env": {"SPARK_FAKE_ACP": script.to_string()},
    });
    for (key, value) in registration.as_object().unwrap() {
        match key.as_str() {
            "env" => harness["env"]
                .as_object_mut()
                .unwrap()
                .extend(value.as_object().unwrap().clone()),
            _ => harness[key] = value.clone(),
        }
    }
    AgentTurnRequest {
        conversation_id: format!("acp-{}", dir.display()),
        project_path: dir.to_string_lossy().into_owned(),
        prompt: "Do the thing.".to_string(),
        history: Vec::new(),
        provider: Some("acp:fake".to_string()),
        model: None,
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: None,
        metadata: BTreeMap::from([(
            "spark.execution.settings".to_string(),
            json!({"configuration": {"agents": {"acp_harnesses": {"fake": harness}}}}),
        )]),
    }
}

/// Runs the turn, collecting what the sink saw; `on_event` runs in the sink.
fn run(
    request: &AgentTurnRequest,
    on_event: impl Fn(&TurnStreamEvent) + Send + Sync + 'static,
) -> (Result<AcpTurn, String>, Vec<TurnStreamEvent>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink: AgentTurnEventSink = {
        let seen = Arc::clone(&seen);
        Arc::new(move |event: TurnStreamEvent| {
            on_event(&event);
            seen.lock().unwrap().push(event);
        })
    };
    let result = acp::run_turn(request, "fake", Some(&sink)).map_err(|failure| failure.message);
    let seen = seen.lock().unwrap().clone();
    (result, seen)
}

fn log(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("acp.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn received(dir: &Path, method: &str) -> Vec<Value> {
    log(dir)
        .into_iter()
        .filter(|entry| entry["received"]["method"] == method)
        .map(|entry| entry["received"]["params"].clone())
        .collect()
}

fn launches(dir: &Path) -> usize {
    log(dir)
        .iter()
        .filter(|entry| entry.get("argv").is_some())
        .count()
}

fn chunk(kind: &str, text: &str) -> Value {
    json!({"sessionUpdate": kind, "content": {"type": "text", "text": text}})
}

fn other(events: &[TurnStreamEvent], kind: &str) -> Vec<TurnStreamEvent> {
    events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::Other(kind.to_string()))
        .cloned()
        .collect()
}

#[test]
fn acp_turn_streams_updates_while_the_prompt_is_open_and_returns_them_once() {
    let dir = tempfile::tempdir().unwrap();
    let gate = dir.path().join("gate");
    let script = json!({
        "updates": [
            chunk("agent_thought_chunk", "Thinking."),
            chunk("agent_message_chunk", "Looking "),
            chunk("agent_message_chunk", "around."),
            {"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "List files",
             "kind": "execute", "status": "pending", "rawInput": {"command": "ls"}},
            {"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed",
             "rawOutput": {"stdout": "a.txt"}},
            {"sessionUpdate": "usage_update", "used": 900, "size": 200000},
            {"sessionUpdate": "plan", "entries": [{"content": "Look", "priority": "high", "status": "completed"}]},
            chunk("agent_message_chunk", "Done."),
        ],
        "gate": gate,
        "usage": {"inputTokens": 12, "outputTokens": 5, "totalTokens": 17},
        "log_env": ["HARNESS_ONLY", "SPARK_PROJECT_WORK_DIR"],
    });
    let mut request = request(dir.path(), script, json!({"env": {"HARNESS_ONLY": "kept"}}));
    request.metadata.insert(
        PROJECT_WORK_DIR_METADATA_KEY.to_string(),
        json!(dir.path().join("work")),
    );

    // The fake holds its response until the sink has seen the last chunk,
    // so delivery happens while the prompt is still open.
    let (result, seen) = run(&request, move |event| {
        if event.content_delta.as_deref() == Some("Done.")
            && event.kind == TurnStreamEventKind::ContentDelta
        {
            std::fs::write(&gate, "").unwrap();
        }
    });
    let turn = result.expect("turn");

    assert_eq!(turn.stop_reason, StopReason::EndTurn);
    assert_eq!(
        turn.output.events, seen,
        "every event is delivered exactly once, in order"
    );
    assert_eq!(turn.output.final_assistant_text.as_deref(), Some("Done."));
    let completed = seen
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ContentCompleted)
        .map(|event| (event.message.clone().unwrap(), event.phase.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        completed,
        [
            ("Thinking.".to_string(), None),
            (
                "Looking around.".to_string(),
                Some("commentary".to_string())
            ),
            ("Done.".to_string(), Some("final_answer".to_string())),
        ]
    );
    let tools = seen
        .iter()
        .filter_map(|event| Some((event.kind.clone(), event.tool_call.clone()?)))
        .collect::<Vec<_>>();
    assert_eq!(tools[0].0, TurnStreamEventKind::ToolCallStarted);
    assert_eq!(tools[0].1["kind"], "command_execution");
    assert_eq!(tools[0].1["command"], "ls");
    assert_eq!(tools[1].0, TurnStreamEventKind::ToolCallCompleted);
    assert_eq!(tools[1].1["output"]["stdout"], "a.txt");
    // Context occupancy is not request consumption; plans are not the plan channel.
    assert_eq!(other(&seen, "context_usage_updated").len(), 1);
    assert_eq!(other(&seen, "plan_updated").len(), 1);
    let usage = json!({"inputTokens": 12, "outputTokens": 5, "totalTokens": 17});
    assert_eq!(turn.output.token_usage, Some(usage.clone()));
    let usage_events = seen
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::TokenUsageUpdated)
        .map(|event| event.token_usage.clone())
        .collect::<Vec<_>>();
    assert_eq!(usage_events, [Some(usage)]);
    let terminal = seen
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::TurnCompleted)
        .collect::<Vec<_>>();
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].status.as_deref(), Some("end_turn"));
    assert_eq!(
        seen.last().unwrap().kind,
        TurnStreamEventKind::TurnCompleted
    );

    let record = turn.record.expect("session record");
    assert_eq!(record.session_id, "fake-session");
    assert_eq!(turn.output.app_thread_id.as_deref(), Some("fake-session"));
    assert_eq!(record.instructions_sha256, None);
    let established = other(&seen, "session_established");
    assert_eq!(established[0].details, Some(json!(record)));

    let initialize = &received(dir.path(), "initialize")[0];
    assert_eq!(initialize["protocolVersion"], 1);
    assert!(
        !initialize.to_string().contains("elicitation"),
        "form elicitation is not advertised: {initialize}"
    );
    let new = &received(dir.path(), "session/new")[0];
    assert!(new.get("systemPrompt").is_none());
    assert_eq!(
        new["cwd"],
        json!(std::fs::canonicalize(dir.path()).unwrap())
    );
    let prompt = &received(dir.path(), "session/prompt")[0];
    assert_eq!(
        prompt["prompt"],
        json!([{"type": "text", "text": "Do the thing."}])
    );
    let startup = &log(dir.path())[0];
    assert_eq!(startup["env"]["HARNESS_ONLY"], "kept");
    assert_eq!(
        startup["env"]["SPARK_PROJECT_WORK_DIR"],
        json!(dir.path().join("work"))
    );
}

#[test]
fn acp_permission_policy_answers_from_the_offered_options() {
    let option = |id: &str, kind: &str| json!({"optionId": id, "name": id, "kind": kind});
    let cases = [
        (
            "allow",
            vec![
                option("always", "allow_always"),
                option("no", "reject_once"),
            ],
            Some("always"),
        ),
        (
            "allow",
            vec![
                option("always", "allow_always"),
                option("once", "allow_once"),
            ],
            Some("once"),
        ),
        (
            "deny",
            vec![
                option("once", "allow_once"),
                option("never", "reject_always"),
            ],
            Some("never"),
        ),
        ("allow", vec![option("no", "reject_once")], None),
    ];
    for (policy, options, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let script = json!({
            "permission": {
                "toolCall": {"toolCallId": "t1", "title": "Edit file"},
                "options": options,
            },
            "updates": [chunk("agent_message_chunk", "Asked.")],
        });
        let request = request(dir.path(), script, json!({"permission_policy": policy}));
        let (result, seen) = run(&request, |_| {});
        result.expect("turn");
        let reply = log(dir.path())
            .into_iter()
            .find(|entry| entry["received"]["id"] == "permission-1")
            .expect("permission reply")["received"]["result"]["outcome"]
            .clone();
        match expected {
            Some(option_id) => {
                assert_eq!(reply, json!({"outcome": "selected", "optionId": option_id}));
                assert!(other(&seen, "warning").is_empty());
            }
            None => {
                assert_eq!(reply, json!({"outcome": "cancelled"}));
                assert_eq!(
                    other(&seen, "warning").len(),
                    1,
                    "{policy}: refusal is visible"
                );
            }
        }
    }
}

#[test]
fn acp_interrupt_cancels_the_open_prompt_and_keeps_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let script = json!({
        "updates": [chunk("agent_message_chunk", "Working")],
        "await_cancel": "respond",
    });
    let request = request(dir.path(), script, json!({}));
    let (project, conversation) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let (result, seen) = run(&request, move |event| {
        if event.kind == TurnStreamEventKind::ContentDelta {
            assert!(acp::interrupt(&project, &conversation));
        }
    });
    let turn = result.expect("cancelled turn");
    assert_eq!(turn.stop_reason, StopReason::Cancelled);
    assert!(turn.record.is_some(), "a settled cancel keeps continuity");
    assert_eq!(turn.output.final_assistant_text, None);
    assert_eq!(received(dir.path(), "session/cancel").len(), 1);
    assert!(
        seen.iter()
            .all(|event| event.kind != TurnStreamEventKind::TurnCompleted),
        "the caller settles a cancelled turn"
    );
    assert!(!acp::interrupt(
        &request.project_path,
        &request.conversation_id
    ));
}

#[test]
fn acp_cancel_that_never_settles_stops_the_process_and_drops_continuity() {
    let dir = tempfile::tempdir().unwrap();
    let script = json!({
        "updates": [chunk("agent_message_chunk", "Working")],
        "await_cancel": "ignore",
    });
    let request = request(dir.path(), script, json!({}));
    let (project, conversation) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let (result, _) = run(&request, move |event| {
        if event.kind == TurnStreamEventKind::ContentDelta {
            acp::interrupt(&project, &conversation);
        }
    });
    let turn = result.expect("stopped turn");
    assert_eq!(turn.stop_reason, StopReason::Cancelled);
    assert!(turn.record.is_none());
    assert_eq!(turn.output.app_thread_id, None);
    assert!(turn.output.thread_resume_failure.is_some());
}

#[test]
fn acp_pinned_instructions_need_system_prompt_or_a_declared_channel() {
    // Exact bytes, including quoting, newlines and trailing whitespace.
    let instructions = "Be \"exact\".\nLine two\\n  \n";

    // Neither channel: refused before any session or prompt.
    let dir = tempfile::tempdir().unwrap();
    let mut no_channel = request(dir.path(), json!({}), json!({}));
    no_channel.metadata.insert(
        AGENT_INSTRUCTIONS_METADATA_KEY.to_string(),
        json!(instructions),
    );
    let error = run(&no_channel, |_| {}).0.err().expect("refused");
    assert!(
        error.contains("systemPrompt") && error.contains("system_instructions"),
        "{error}"
    );
    assert!(received(dir.path(), "session/new").is_empty());
    assert!(received(dir.path(), "session/prompt").is_empty());

    // Instruction-free work runs on the same harness.
    let dir = tempfile::tempdir().unwrap();
    run(&request(dir.path(), json!({}), json!({})), |_| {})
        .0
        .expect("flow node");

    // Advertised systemPrompt carries them on session/new, never in the prompt.
    let dir = tempfile::tempdir().unwrap();
    let mut advertised = request(dir.path(), json!({"system_prompt": true}), json!({}));
    advertised.metadata.insert(
        AGENT_INSTRUCTIONS_METADATA_KEY.to_string(),
        json!(instructions),
    );
    let turn = run(&advertised, |_| {}).0.expect("turn");
    let new = &received(dir.path(), "session/new")[0];
    assert_eq!(new["systemPrompt"]["mode"], "append");
    assert_eq!(new["systemPrompt"]["content"][0]["text"], instructions);
    assert!(!received(dir.path(), "session/prompt")[0]
        .to_string()
        .contains("exact"));
    assert_eq!(
        turn.record.unwrap().instructions_sha256,
        Some(spark_common::project::sha256_hex(instructions.as_bytes()))
    );

    // A declared channel renders at the first launch; systemPrompt is never sent.
    let dir = tempfile::tempdir().unwrap();
    let script = json!({
        "system_prompt": true,
        "log_env": ["FAKE_TEXT", "FAKE_JSON"],
        "config_options": [{
            "id": "persona", "name": "Persona", "type": "select", "currentValue": "default",
            "options": [{"value": "default", "name": "Default"}, {"value": "Custom-Mode", "name": "Custom"}],
        }],
    });
    let mut channel = request(
        dir.path(),
        script,
        json!({"args": ["--acp"], "system_instructions": {
            "env": {
                "FAKE_TEXT": {"text": "{{instructions}}"},
                "FAKE_JSON": {"json": {"system": "{{instructions}}"}},
            },
            "args": ["{{instructions_file}}"],
            "config": {"persona": "Custom-Mode"},
        }}),
    );
    channel.metadata.insert(
        AGENT_INSTRUCTIONS_METADATA_KEY.to_string(),
        json!(instructions),
    );
    run(&channel, |_| {}).0.expect("turn");
    assert_eq!(launches(dir.path()), 1, "no relaunch during setup");
    let startup = &log(dir.path())[0];
    assert_eq!(startup["env"]["FAKE_TEXT"], instructions);
    let rendered: Value =
        serde_json::from_str(startup["env"]["FAKE_JSON"].as_str().unwrap()).unwrap();
    assert_eq!(rendered, json!({"system": instructions}));
    let argv = startup["argv"].as_array().unwrap();
    assert_eq!(argv[0], "--acp");
    assert_eq!(startup["files"][argv[1].as_str().unwrap()], instructions);
    assert!(received(dir.path(), "session/new")[0]
        .get("systemPrompt")
        .is_none());
    let set = received(dir.path(), "session/set_config_option");
    assert_eq!(set.len(), 1);
    assert_eq!(
        (set[0]["configId"].clone(), set[0]["value"].clone()),
        (json!("persona"), json!("Custom-Mode"))
    );
    assert!(!received(dir.path(), "session/prompt")[0]
        .to_string()
        .contains("exact"));
}

#[test]
fn acp_unsupported_settings_fail_before_the_prompt() {
    let options = json!([{
        "id": "model", "name": "Model", "category": "model", "type": "select",
        "currentValue": "small",
        "options": [{"value": "small", "name": "Small"}, {"value": "Large-1", "name": "Large"}],
    }]);

    let dir = tempfile::tempdir().unwrap();
    let mut plan = request(dir.path(), json!({}), json!({}));
    plan.chat_mode = Some("plan".to_string());
    assert!(run(&plan, |_| {}).0.err().unwrap().contains("Plan mode"));
    assert_eq!(launches(dir.path()), 0);

    let dir = tempfile::tempdir().unwrap();
    let mut effort = request(dir.path(), json!({"config_options": options}), json!({}));
    effort.reasoning_effort = Some("high".to_string());
    let error = run(&effort, |_| {}).0.err().expect("no effort selection");
    assert!(error.contains("reasoning effort"), "{error}");
    assert!(received(dir.path(), "session/prompt").is_empty());

    let dir = tempfile::tempdir().unwrap();
    let mut unknown = request(dir.path(), json!({"config_options": options}), json!({}));
    unknown.model = Some("huge".to_string());
    assert!(run(&unknown, |_| {}).0.err().unwrap().contains("\"huge\""));
    assert!(received(dir.path(), "session/prompt").is_empty());

    let dir = tempfile::tempdir().unwrap();
    let mut model = request(dir.path(), json!({"config_options": options}), json!({}));
    model.model = Some("Large-1".to_string());
    run(&model, |_| {}).0.expect("turn");
    let set = received(dir.path(), "session/set_config_option");
    assert_eq!(
        (set[0]["configId"].clone(), set[0]["value"].clone()),
        (json!("model"), json!("Large-1"))
    );
    assert_eq!(received(dir.path(), "session/prompt").len(), 1);
}

/// Session controls apply by exact config and value ID after model and effort,
/// only for the harness they were chosen for, and fail before the prompt
/// when the harness no longer offers them.
#[test]
fn acp_session_controls_apply_exactly_for_their_harness_and_fail_when_stale() {
    let options = json!([
        {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "small",
         "options": [{"value": "small", "name": "Small"}, {"value": "Large-1", "name": "Large"}]},
        {"id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": "Low",
         "options": [{"value": "Low", "name": "Low"}, {"value": "High", "name": "High"}]},
        {"id": "approval", "name": "Approval", "category": "mode", "type": "select", "currentValue": "ask",
         "options": [
            {"group": "manual", "name": "Manual", "options": [{"value": "ask", "name": "Ask"}]},
            {"group": "auto", "name": "Automatic", "options": [{"value": "Auto/Edit", "name": "Auto edit"}]},
         ]},
        {"id": "verbose", "name": "Verbose", "type": "boolean", "currentValue": false},
    ]);
    let turn = |dir: &Path, provider: &str, controls: Value| {
        let mut request = request(dir, json!({"config_options": options}), json!({}));
        request.model = Some("Large-1".to_string());
        request.reasoning_effort = Some("High".to_string());
        request
            .metadata
            .get_mut("spark.execution.settings")
            .unwrap()["model_settings"] = json!({"provider": provider, "session_config": controls});
        run(&request, |_| {}).0
    };
    let applied = |dir: &Path| {
        received(dir, "session/set_config_option")
            .iter()
            .map(|set| json!([set["configId"], set["value"]]))
            .collect::<Vec<_>>()
    };

    let dir = tempfile::tempdir().unwrap();
    turn(dir.path(), "acp:fake", json!({"approval": "Auto/Edit"})).expect("turn");
    assert_eq!(
        applied(dir.path()),
        [
            json!(["model", "Large-1"]),
            json!(["effort", "High"]),
            json!(["approval", "Auto/Edit"])
        ]
    );
    assert_eq!(received(dir.path(), "session/prompt").len(), 1);

    for (controls, expected) in [
        (
            json!({"approval": "auto/edit"}),
            "does not offer the value \"auto/edit\"",
        ),
        (
            json!({"retired": "on"}),
            "offers no config option \"retired\"",
        ),
        (json!({"verbose": "true"}), "\"verbose\""),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let error = turn(dir.path(), "acp:fake", controls)
            .err()
            .expect("stale control fails");
        assert!(error.contains(expected), "{error}");
        assert!(received(dir.path(), "session/prompt").is_empty());
    }

    // Choosing the mode resets the model, or drops the effort: an earlier
    // explicit selection no longer holds, so nothing is prompted.
    let mut reset = options.clone();
    reset[0]["currentValue"] = json!("small");
    reset[1]["currentValue"] = json!("High");
    let mut dropped = options.clone();
    dropped[0]["currentValue"] = json!("Large-1");
    dropped.as_array_mut().unwrap().remove(1);
    for (acknowledged, expected) in [
        (
            reset,
            "config option \"model\" is \"small\" after the later selections, not the selected \"Large-1\"",
        ),
        (
            dropped,
            "no longer offers config option \"effort\" after the later selections",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut request = request(
            dir.path(),
            json!({"config_options": options, "on_set": {"approval=Auto/Edit": acknowledged}}),
            json!({}),
        );
        request.model = Some("Large-1".to_string());
        request.reasoning_effort = Some("High".to_string());
        request
            .metadata
            .get_mut("spark.execution.settings")
            .unwrap()["model_settings"] =
            json!({"provider": "acp:fake", "session_config": {"approval": "Auto/Edit"}});
        let error = run(&request, |_| {}).0.err().expect("a replaced selection fails");
        assert!(error.contains(expected), "{error}");
        assert_eq!(applied(dir.path()).len(), 3);
        assert!(received(dir.path(), "session/prompt").is_empty());
    }

    let dir = tempfile::tempdir().unwrap();
    turn(dir.path(), "acp:other", json!({"approval": "gone"})).expect("another harness's controls");
    assert_eq!(
        applied(dir.path()),
        [json!(["model", "Large-1"]), json!(["effort", "High"])]
    );
}

#[test]
fn acp_failures_propagate_without_a_second_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let crash = request(
        dir.path(),
        json!({"exit_on_prompt": {"code": 7, "stderr": "fatal: model unavailable"}}),
        json!({}),
    );
    let error = run(&crash, |_| {}).0.err().expect("crash fails");
    assert!(error.contains("fatal: model unavailable"), "{error}");

    let dir = tempfile::tempdir().unwrap();
    // A line that is not JSON-RPC ends the turn, even with an answer to follow.
    let malformed = request(
        dir.path(),
        json!({"malformed_on_prompt": true, "updates": [chunk("agent_message_chunk", "Fine.")]}),
        json!({}),
    );
    let error = run(&malformed, |_| {})
        .0
        .err()
        .expect("malformed output fails");
    assert!(error.contains("malformed protocol output"), "{error}");
    assert_eq!(received(dir.path(), "session/prompt").len(), 1);

    let dir = tempfile::tempdir().unwrap();
    let refused = request(
        dir.path(),
        json!({"prompt_error": {"code": -32603, "message": "upstream broke"}}),
        json!({}),
    );
    let error = run(&refused, |_| {}).0.err().expect("prompt error fails");
    assert!(error.contains("upstream broke"), "{error}");
    assert_eq!(
        received(dir.path(), "session/prompt").len(),
        1,
        "never retried"
    );

    let dir = tempfile::tempdir().unwrap();
    let old = request(dir.path(), json!({"protocol_version": 2}), json!({}));
    assert!(run(&old, |_| {})
        .0
        .err()
        .unwrap()
        .contains("protocol version"));

    let dir = tempfile::tempdir().unwrap();
    let missing = request(
        dir.path(),
        json!({}),
        json!({"executable": "/nonexistent/acp-agent"}),
    );
    assert!(run(&missing, |_| {}).0.err().unwrap().contains("not found"));
}

/// A record matching what `request` would establish for `dir`.
fn saved(dir: &Path, request: &AgentTurnRequest, session_id: &str) -> AcpSessionRecord {
    let harness: spark_common::agent_settings::AcpHarness = serde_json::from_value(
        request.metadata["spark.execution.settings"]["configuration"]["agents"]["acp_harnesses"]
            ["fake"]
            .clone(),
    )
    .unwrap();
    AcpSessionRecord {
        harness: "fake".to_string(),
        session_id: session_id.to_string(),
        cwd: std::fs::canonicalize(dir)
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        configuration_generation: acp::launch_generation(&harness, Default::default()),
        instructions_sha256: None,
    }
}

fn with_history(mut request: AgentTurnRequest, record: &AcpSessionRecord) -> AgentTurnRequest {
    request.history = vec![
        HistoryTurn::User(UserTurn::new("Earlier question")),
        HistoryTurn::Assistant(AssistantTurn::new("Earlier answer")),
    ];
    request.metadata.extend(record.to_metadata());
    request
}

#[test]
fn acp_continuity_resumes_matching_sessions_and_resets_once_otherwise() {
    // The inherited environment is part of a launch's identity.
    let _env = super::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Resume: same session, no history re-sent.
    let dir = tempfile::tempdir().unwrap();
    let base = request(dir.path(), json!({"resume": true, "load": true}), json!({}));
    let record = saved(dir.path(), &base, "Mixed-Case-Session");
    let turn = run(&with_history(base, &record), |_| {})
        .0
        .expect("resumed");
    assert_eq!(
        received(dir.path(), "session/resume")[0]["sessionId"],
        "Mixed-Case-Session"
    );
    assert!(received(dir.path(), "session/load").is_empty());
    assert!(received(dir.path(), "session/new").is_empty());
    assert_eq!(turn.record.as_ref(), Some(&record));
    assert_eq!(turn.output.thread_resume_failure, None);
    assert!(!received(dir.path(), "session/prompt")[0]
        .to_string()
        .contains("Earlier"));

    // Load: replayed history is not reported again.
    let dir = tempfile::tempdir().unwrap();
    let base = request(
        dir.path(),
        json!({"load": true, "replay": [chunk("agent_message_chunk", "Replayed")],
               "updates": [chunk("agent_message_chunk", "Fresh")]}),
        json!({}),
    );
    let record = saved(dir.path(), &base, "s-1");
    let (result, seen) = run(&with_history(base, &record), |_| {});
    let turn = result.expect("loaded");
    assert_eq!(received(dir.path(), "session/load").len(), 1);
    assert!(seen
        .iter()
        .all(|event| event.content_delta.as_deref() != Some("Replayed")));
    assert_eq!(turn.output.final_assistant_text.as_deref(), Some("Fresh"));

    // Rejected resume: one explicit reset, then one fresh session with history.
    let dir = tempfile::tempdir().unwrap();
    let base = request(
        dir.path(),
        json!({"resume": true, "resume_error": {"code": -32002, "message": "gone"}}),
        json!({}),
    );
    let record = saved(dir.path(), &base, "s-1");
    let (result, seen) = run(&with_history(base, &record), |_| {});
    let turn = result.expect("fresh fallback");
    let reset = other(&seen, "continuity_reset");
    assert_eq!(reset.len(), 1);
    let reset_at = seen.iter().position(|event| event == &reset[0]).unwrap();
    let established_at = seen
        .iter()
        .position(|event| event.kind == TurnStreamEventKind::Other("session_established".into()))
        .unwrap();
    assert!(reset_at < established_at, "reset precedes fresh setup");
    assert_eq!(received(dir.path(), "session/new").len(), 1);
    let failure = turn.output.thread_resume_failure.expect("continuity reset");
    assert_eq!(failure.error_code.as_deref(), Some("thread_resume_failure"));
    let prompt = received(dir.path(), "session/prompt")[0].to_string();
    assert!(
        prompt.contains("Earlier question") && prompt.contains("Earlier answer"),
        "{prompt}"
    );

    // Changed configuration: no resume attempt at all.
    let dir = tempfile::tempdir().unwrap();
    let base = request(dir.path(), json!({"resume": true}), json!({}));
    let mut record = saved(dir.path(), &base, "s-1");
    record.configuration_generation = "older".to_string();
    let turn = run(&with_history(base, &record), |_| {}).0.expect("fresh");
    assert!(received(dir.path(), "session/resume").is_empty());
    assert_eq!(received(dir.path(), "session/new").len(), 1);
    assert!(turn.output.thread_resume_failure.is_some());

    // No restore capability: reset rather than a silent new session.
    let dir = tempfile::tempdir().unwrap();
    let base = request(dir.path(), json!({}), json!({}));
    let record = saved(dir.path(), &base, "s-1");
    let turn = run(&with_history(base, &record), |_| {}).0.expect("fresh");
    assert!(turn
        .output
        .thread_resume_failure
        .unwrap()
        .message
        .contains("cannot resume"));

    // A login requirement is a setup failure, never a fresh retry.
    let dir = tempfile::tempdir().unwrap();
    let base = request(
        dir.path(),
        json!({"resume": true, "resume_error": {"code": -32000, "message": "auth"}}),
        json!({}),
    );
    let record = saved(dir.path(), &base, "s-1");
    let error = run(&with_history(base, &record), |_| {})
        .0
        .err()
        .expect("auth");
    assert!(error.contains("authentication"), "{error}");
    assert!(received(dir.path(), "session/new").is_empty());
}

/// Two runs of one flow node at once each get their own process and
/// session; a harness with no instruction channel still runs flow nodes,
/// which carry no pinned instructions, but never utility calls.
#[test]
fn acp_parallel_runs_of_one_flow_node_keep_separate_sessions() {
    use spark_agent_adapter::{
        run_utility_call, CodergenBackend, CodergenBackendRequest, CodergenBackendResponse,
        RustLlmCodergenBackend, UtilityCall,
    };

    let dir = tempfile::tempdir().unwrap();
    let gate = dir.path().join("gate");
    let script = json!({"gate": gate, "updates": [chunk("agent_message_chunk", "Reviewed.")]});
    let base = request(dir.path(), script, json!({}));
    let node = |run_id: &str| {
        let mut metadata = base.metadata.clone();
        metadata.insert("spark.runtime.run_id".to_string(), json!(run_id));
        CodergenBackendRequest {
            node_id: "review".to_string(),
            prompt: "Review it.".to_string(),
            provider: "acp:fake".to_string(),
            project_path: Some(dir.path().to_path_buf()),
            metadata,
            ..CodergenBackendRequest::default()
        }
    };
    let runs = ["run-a", "run-b"].map(|run_id| {
        let request = node(run_id);
        std::thread::spawn(move || {
            RustLlmCodergenBackend::new(unified_llm_adapter::Client::new()).run(request)
        })
    });
    // Both prompts are open at once before either may finish.
    let waited = std::time::Instant::now();
    while received(dir.path(), "session/prompt").len() < 2 {
        assert!(
            waited.elapsed() < std::time::Duration::from_secs(10),
            "both runs prompt"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::write(&gate, "").unwrap();
    for run in runs {
        let output = run.join().unwrap().expect("node turn");
        assert_eq!(
            output.response,
            CodergenBackendResponse::Text("Reviewed.".to_string())
        );
        assert!(AcpSessionRecord::from_metadata(&output.resume_metadata).is_some());
        assert!(output
            .events
            .iter()
            .any(|event| event.event_type == "agent_harness_request_completed"));
    }
    assert_eq!(launches(dir.path()), 2);
    assert!(received(dir.path(), "session/new")
        .iter()
        .all(|params| params.get("systemPrompt").is_none()));

    let error = run_utility_call(
        &unified_llm_adapter::Client::new(),
        &UtilityCall {
            provider: Some("acp:fake".to_string()),
            model: None,
            llm_profile: None,
            reasoning_effort: None,
            instructions: "Summarize.".to_string(),
            input: "Text.".to_string(),
            metadata: base.metadata.clone(),
            timeout: std::time::Duration::from_secs(5),
        },
    )
    .expect_err("utility calls need a utility-capable provider");
    assert!(error.contains("does not run utility calls"), "{error}");
}

/// The public chat backend over `request`, collecting what its sink saw.
fn chat(
    request: AgentTurnRequest,
    on_event: impl Fn(&TurnStreamEvent) + Send + Sync + 'static,
) -> (
    Result<spark_agent_adapter::AgentTurnOutput, spark_agent_adapter::AgentError>,
    Vec<TurnStreamEvent>,
) {
    use spark_agent_adapter::{AgentTurnBackend, RustLlmAgentTurnBackend};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink: AgentTurnEventSink = {
        let seen = Arc::clone(&seen);
        Arc::new(move |event: TurnStreamEvent| {
            on_event(&event);
            seen.lock().unwrap().push(event);
        })
    };
    let backend = RustLlmAgentTurnBackend::new(unified_llm_adapter::Client::new());
    let result = backend.run_turn_with_event_sink(request, Some(sink));
    let seen = seen.lock().unwrap().clone();
    (result, seen)
}

fn interrupt_chat(project: &str, conversation: &str) -> bool {
    use spark_agent_adapter::{AgentTurnBackend, RustLlmAgentTurnBackend};
    RustLlmAgentTurnBackend::new(unified_llm_adapter::Client::new())
        .interrupt_turn(project, conversation)
}

fn terminals(events: &[TurnStreamEvent]) -> Vec<Option<String>> {
    events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::TurnCompleted)
        .map(|event| event.status.clone())
        .collect()
}

fn agents(request: &mut AgentTurnRequest) -> &mut Value {
    &mut request
        .metadata
        .get_mut("spark.execution.settings")
        .unwrap()["configuration"]["agents"]
}

#[test]
fn acp_chat_cancel_settles_once_and_explicit_reasoning_controls_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let script = json!({
        "updates": [chunk("agent_message_chunk", "Working")],
        "await_cancel": "respond",
    });
    let cancelled = request(dir.path(), script, json!({}));
    let (project, conversation) = (
        cancelled.project_path.clone(),
        cancelled.conversation_id.clone(),
    );
    let (result, seen) = chat(cancelled, move |event| {
        if event.kind == TurnStreamEventKind::ContentDelta {
            assert!(interrupt_chat(&project, &conversation));
        }
    });
    let output = result.expect("stopped turn");
    assert_eq!(terminals(&seen), [Some("interrupted".to_string())]);
    assert_eq!(terminals(&output.events), [Some("interrupted".to_string())]);

    for (key, value) in [
        ("thinking", json!("off")),
        ("thinking_budget_tokens", json!(2048)),
        ("reasoning_mode", json!("adaptive")),
        ("reasoning_summary", json!("auto")),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut request = request(dir.path(), json!({}), json!({}));
        request
            .metadata
            .get_mut("spark.execution.settings")
            .unwrap()["model_settings"] = json!({"provider": "acp:fake", key: value});
        let error = chat(request, |_| {}).0.err().expect("refused").message;
        assert!(error.contains(key), "{error}");
        assert_eq!(launches(dir.path()), 0, "{key} is refused before launch");
    }
}

#[test]
fn acp_chat_starts_fresh_when_the_inherited_environment_changes() {
    let _env = super::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let script = json!({"resume": true, "updates": [chunk("agent_message_chunk", "Hi.")]});
    let mut first = request(dir.path(), script, json!({}));
    agents(&mut first)["environment_inheritance"] = json!("inherit_all");
    let mut second = first.clone();
    let (result, seen) = chat(first, |_| {});
    result.expect("first turn");
    let record: AcpSessionRecord = serde_json::from_value(
        other(&seen, "session_established")[0]
            .details
            .clone()
            .unwrap(),
    )
    .unwrap();

    second.metadata.extend(record.to_metadata());
    let output = chat(second.clone(), |_| {}).0.expect("same settings");
    assert_eq!(received(dir.path(), "session/resume").len(), 1);
    assert!(output.thread_resume_failure.is_none());

    agents(&mut second)["environment_inheritance"] = json!("inherit_none");
    let output = chat(second, |_| {}).0.expect("fresh session");
    assert_eq!(
        received(dir.path(), "session/resume").len(),
        1,
        "never resumed"
    );
    assert_eq!(received(dir.path(), "session/new").len(), 2);
    assert!(output.thread_resume_failure.is_some());
}

/// A sink that stops taking events neither holds back cancellation nor lets
/// the harness's output queue up without bound.
#[cfg(unix)]
#[test]
fn acp_blocked_sink_keeps_cancellation_independent_and_fails_on_overload() {
    let exists = |path: &Path| path.exists();
    let wait = |what: &str, done: &dyn Fn() -> bool| {
        let started = std::time::Instant::now();
        while !done() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "{what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    let blocking = |dir: &Path| {
        let (entered, release) = (dir.join("entered"), dir.join("release"));
        move |event: &TurnStreamEvent| {
            if event.kind == TurnStreamEventKind::ContentDelta && !entered.exists() {
                std::fs::write(&entered, "").unwrap();
                while !release.exists() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    };

    // Cancellation reaches the harness while the sink is blocked.
    let dir = tempfile::tempdir().unwrap();
    let script = json!({
        "updates": [chunk("agent_message_chunk", "Working")],
        "await_cancel": "respond",
    });
    let cancelled = request(dir.path(), script, json!({}));
    let (project, conversation) = (
        cancelled.project_path.clone(),
        cancelled.conversation_id.clone(),
    );
    let turn = std::thread::spawn({
        let sink = blocking(dir.path());
        move || chat(cancelled, sink)
    });
    wait("the sink blocks", &|| exists(&dir.path().join("entered")));
    assert!(interrupt_chat(&project, &conversation));
    wait("cancel is sent", &|| {
        received(dir.path(), "session/cancel").len() == 1
    });
    std::fs::write(dir.path().join("release"), "").unwrap();
    let (result, _) = turn.join().unwrap();
    let output = result.expect("cancelled turn");
    assert_eq!(terminals(&output.events), [Some("interrupted".to_string())]);

    // A sink a full queue behind fails the turn and stops the harness.
    let dir = tempfile::tempdir().unwrap();
    let updates = (0..400)
        .map(|n| chunk("agent_message_chunk", &format!("{n} ")))
        .collect::<Vec<_>>();
    let flooding = request(
        dir.path(),
        json!({"updates": updates, "await_cancel": "ignore"}),
        json!({}),
    );
    let turn = std::thread::spawn({
        let sink = blocking(dir.path());
        move || chat(flooding, sink)
    });
    wait("the sink blocks", &|| exists(&dir.path().join("entered")));
    let pid = log(dir.path())[0]["pid"].to_string();
    wait("the harness is stopped while the sink is blocked", &|| {
        !std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    });
    std::fs::write(dir.path().join("release"), "").unwrap();
    let (result, seen) = turn.join().unwrap();
    let error = result.err().expect("overloaded").message;
    assert!(error.contains("overloaded"), "{error}");
    assert!(seen.len() < 200, "the queue is bounded: {}", seen.len());
}
