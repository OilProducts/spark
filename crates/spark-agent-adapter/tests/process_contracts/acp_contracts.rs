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
    run_flow(request, None, on_event)
}

/// [`run`] for a flow node, whose forms go to `clarification`.
fn run_flow(
    request: &AgentTurnRequest,
    clarification: Option<spark_agent_adapter::codergen::ClarificationHandler>,
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
    let result = acp::run_turn(request, "fake", Some(&sink), clarification)
        .map_err(|failure| failure.message);
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

/// Elsewhere than Windows a launch searches PATH as the system does: an entry
/// relative to the work area, such as `node_modules/.bin`, finds the project's
/// program, and a file it cannot run earlier on PATH is passed over. An empty
/// entry adds no search of the work area. A program on no PATH entry is not
/// installed; one found whose interpreter is missing failed to launch.
#[cfg(unix)]
#[test]
fn acp_unix_launch_searches_path_from_the_work_area_past_files_it_cannot_run() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let bin = project.join("node_modules").join(".bin");
    let blocker = temp.path().join("blocker");
    for dir in [&bin, &blocker] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::os::unix::fs::symlink(FAKE, bin.join("agent")).unwrap();
    std::os::unix::fs::symlink(FAKE, project.join("stray")).unwrap();
    std::fs::write(blocker.join("agent"), "not a program").unwrap();
    let broken = bin.join("broken");
    std::fs::write(&broken, "#!/missing/interpreter\n").unwrap();
    std::fs::set_permissions(&broken, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let discover = |executable: &str| {
        let harness = serde_json::from_value(json!({
            "name": "Project agent",
            "executable": executable,
            "env": {"PATH": format!("{}::node_modules/.bin", blocker.display())},
        }))
        .unwrap();
        acp::discover(
            "project",
            &harness,
            spark_agent_adapter::EnvironmentInheritancePolicy::InheritNone,
            &project,
        )
    };

    let found = discover("agent");
    assert_eq!(found.setup_status, "ready", "{:?}", found.error);
    for executable in ["missing-agent", "stray"] {
        let missing = discover(executable);
        assert_eq!(
            missing.setup_status, "not_installed",
            "{executable}: {:?}",
            missing.error
        );
    }
    for executable in ["broken", broken.to_str().unwrap()] {
        let failed = discover(executable);
        assert_eq!(failed.setup_status, "unreachable", "{executable}");
        assert!(
            failed
                .error
                .as_deref()
                .is_some_and(|error| error.contains("launch failed")),
            "{executable}: {:?}",
            failed.error
        );
    }
}

/// Windows runs a batch shim through cmd.exe, which takes no line break in an
/// argument: a raw `{{instructions}}` argument launches for setup and a chat
/// without pinned instructions, a launch with multiline ones fails first, and
/// instructions through a file reach the agent whole.
#[cfg(windows)]
#[test]
fn acp_windows_batch_shim_takes_instructions_through_a_file_not_an_argument() {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("agent.cmd");
    std::fs::write(&shim, format!("@\"{FAKE}\" %*\r\n")).unwrap();
    let instructions = "Be exact.\nSay so twice.";
    let chat = |args: Value, instructions: Option<&str>| {
        let mut chat = request(
            dir.path(),
            json!({}),
            json!({"executable": shim, "system_instructions": {"args": args}}),
        );
        if let Some(instructions) = instructions {
            chat.metadata.insert(
                AGENT_INSTRUCTIONS_METADATA_KEY.to_string(),
                json!(instructions),
            );
        }
        (setup(&chat), run(&chat, |_| {}).0)
    };

    let raw = json!(["--system", "{{instructions}}"]);
    let (found, turn) = chat(raw.clone(), None);
    assert_eq!(found.setup_status, "ready", "{:?}", found.error);
    turn.expect("turn without instructions");
    let before = launches(dir.path());
    let (_, turn) = chat(raw, Some(instructions));
    let error = turn.err().unwrap();
    assert!(error.contains("{{instructions_file}}"), "{error}");
    assert_eq!(launches(dir.path()), before + 1, "only setup launched");

    let (found, turn) = chat(json!(["{{instructions_file}}"]), Some(instructions));
    assert_eq!(found.setup_status, "ready", "{:?}", found.error);
    turn.expect("turn");
    let startup = log(dir.path())
        .into_iter()
        .rev()
        .find(|entry| entry.get("argv").is_some())
        .unwrap();
    let file = startup["argv"][0].as_str().unwrap();
    assert_eq!(startup["files"][file], instructions);
}

/// Discovery of the harness `request` registers, in its project.
#[cfg(windows)]
fn setup(request: &AgentTurnRequest) -> acp::AcpDiscovery {
    let harness = serde_json::from_value(
        request.metadata["spark.execution.settings"]["configuration"]["agents"]["acp_harnesses"]
            ["fake"]
            .clone(),
    )
    .unwrap();
    acp::discover(
        "fake",
        &harness,
        spark_agent_adapter::EnvironmentInheritancePolicy::InheritCoreOnly,
        Path::new(&request.project_path),
    )
}

/// A Windows probe and turn run the program the harness's own child would:
/// by the `PATHEXT` it registers, in any spelling, and an explicit relative
/// path from the project.
#[cfg(windows)]
#[test]
fn acp_windows_launch_finds_the_program_by_the_harness_pathext_and_project() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // Spark's own PATHEXT would pick the failing batch file.
    std::fs::write(bin.join("agent.bat"), "@exit /b 1\r\n").unwrap();
    std::fs::write(bin.join("agent.cmd"), format!("@\"{FAKE}\" %*\r\n")).unwrap();
    for registration in [
        json!({"executable": "agent", "env": {"PATH": bin, "Pathext": ".CMD;.BAT;.EXE"}}),
        json!({"executable": r"bin\agent.cmd"}),
    ] {
        let chat = request(dir.path(), json!({}), registration.clone());
        let found = setup(&chat);
        assert_eq!(
            found.setup_status, "ready",
            "{registration}: {:?}",
            found.error
        );
        if let Err(error) = run(&chat, |_| {}).0 {
            panic!("{registration}: {error}");
        }
    }
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
        "usage": {"inputTokens": 12, "outputTokens": 5, "thoughtTokens": 2, "cachedReadTokens": 30, "cachedWriteTokens": 4, "totalTokens": 53},
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
    // ACP counts each token type once; Spark's input includes the cache and
    // its output includes thinking.
    let usage = json!({
        "input_tokens": 46, "cached_input_tokens": 30, "cache_creation_input_tokens": 4,
        "output_tokens": 7, "reasoning_tokens": 2, "total_tokens": 53,
    });
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
    assert_eq!(
        initialize["clientCapabilities"]["elicitation"],
        json!({"form": {}}),
        "only form elicitation is advertised: {initialize}"
    );
    let new = &received(dir.path(), "session/new")[0];
    assert!(new.get("systemPrompt").is_none());
    assert_eq!(
        new["cwd"],
        json!(spark_common::paths::canonicalize_clean(dir.path()).unwrap())
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
fn acp_diff_content_becomes_file_changes_beside_output_and_locations() {
    let dir = tempfile::tempdir().unwrap();
    let old = (1..=8).map(|n| format!("line {n}\n")).collect::<String>();
    let script = json!({
        "updates": [
            {"sessionUpdate": "tool_call", "toolCallId": "e1", "title": "Edit files", "kind": "edit",
             "status": "pending", "locations": [{"path": "/p/a.rs"}]},
            {"sessionUpdate": "tool_call_update", "toolCallId": "e1", "status": "completed", "content": [
                {"type": "content", "content": {"type": "text", "text": "Edited."}},
                {"type": "diff", "path": "/p/new.rs", "newText": "fn main() {}\n"},
                {"type": "diff", "path": "/p/a.rs", "oldText": old, "newText": "line 1\nline 2\nline 3\nline 4\nline 5\n"},
                {"type": "diff", "path": "/p/a.rs", "oldText": "x\ny", "newText": "x\nY\n"},
            ]},
            chunk("agent_message_chunk", "Done."),
        ],
    });
    let (result, seen) = run(&request(dir.path(), script, json!({})), |_| {});
    result.expect("turn");

    let completed = seen
        .iter()
        .find(|event| event.kind == TurnStreamEventKind::ToolCallCompleted)
        .and_then(|event| event.tool_call.clone())
        .expect("completed tool call");
    assert_eq!(completed["output"], "Edited.");
    assert_eq!(completed["file_paths"], json!(["/p/a.rs"]));
    // A new file as added lines; edits as line diffs with three lines of
    // context, in the order sent and one entry per diff.
    assert_eq!(
        completed["file_changes"],
        json!([
            {"path": "/p/new.rs", "diff": "+fn main() {}\n"},
            {"path": "/p/a.rs", "diff": "@@\n line 3\n line 4\n line 5\n-line 6\n-line 7\n-line 8\n"},
            {"path": "/p/a.rs", "diff": "@@\n x\n-y\n+Y\n"},
        ])
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
fn acp_pinned_instructions_reach_every_harness() {
    let _env = super::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Exact bytes, including quoting, newlines and trailing whitespace.
    let instructions = "Be \"exact\".\nLine two\\n  \n";

    // Neither channel: they lead the first prompt of a fresh session, and a
    // resumed session already has them.
    let dir = tempfile::tempdir().unwrap();
    let mut no_channel = request(dir.path(), json!({"resume": true}), json!({}));
    no_channel.metadata.insert(
        AGENT_INSTRUCTIONS_METADATA_KEY.to_string(),
        json!(instructions),
    );
    let turn = run(&no_channel, |_| {}).0.expect("turn");
    assert!(received(dir.path(), "session/new")[0]["systemPrompt"].is_null());
    let prompt = received(dir.path(), "session/prompt")[0].to_string();
    assert!(
        prompt.contains("<instructions>") && prompt.contains("exact"),
        "{prompt}"
    );
    let record = turn.record.unwrap();
    run(&with_history(no_channel.clone(), &record), |_| {})
        .0
        .expect("warm reused");
    assert_eq!(launches(dir.path()), 1, "warm chat session is reused");
    assert!(!received(dir.path(), "session/prompt")[1]
        .to_string()
        .contains("<instructions>"));
    assert!(acp::close_session(
        &no_channel.project_path,
        &no_channel.conversation_id
    ));
    run(&with_history(no_channel, &record), |_| {})
        .0
        .expect("cold resumed");
    assert_eq!(received(dir.path(), "session/resume").len(), 1);
    assert!(!received(dir.path(), "session/prompt")[2]
        .to_string()
        .contains("<instructions>"));

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
            "does not offer the value \"auto/edit\"; it offers [\"ask\", \"Auto/Edit\"]",
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
        cwd: spark_common::paths::canonicalize_clean(dir)
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
    assert!(error.contains("needs you to sign in"), "{error}");
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
    assert_eq!(
        launches(dir.path()),
        1,
        "same settings reuse the live session"
    );
    assert_eq!(received(dir.path(), "session/resume").len(), 0);
    assert!(output.thread_resume_failure.is_none());

    agents(&mut second)["environment_inheritance"] = json!("inherit_none");
    let output = chat(second, |_| {}).0.expect("fresh session");
    assert_eq!(launches(dir.path()), 2, "changed environment relaunches");
    assert_eq!(
        received(dir.path(), "session/resume").len(),
        0,
        "never resumed"
    );
    assert_eq!(received(dir.path(), "session/new").len(), 2);
    assert!(output.thread_resume_failure.is_some());
}

/// A sink that stops taking events neither holds back cancellation nor lets
/// the harness's output queue up without bound.
#[cfg(unix)]
#[test]
fn acp_blocked_sink_keeps_cancellation_independent_and_gets_every_update() {
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

    // A sink far behind still gets every update, and the turn succeeds.
    let dir = tempfile::tempdir().unwrap();
    let updates = (0..400)
        .map(|n| chunk("agent_message_chunk", &format!("{n} ")))
        .collect::<Vec<_>>();
    let flooding = request(dir.path(), json!({"updates": updates}), json!({}));
    let turn = std::thread::spawn({
        let sink = blocking(dir.path());
        move || chat(flooding, sink)
    });
    wait("the sink blocks", &|| exists(&dir.path().join("entered")));
    // Lets the burst pile up behind the blocked sink; passes either way.
    std::thread::sleep(std::time::Duration::from_millis(200));
    std::fs::write(dir.path().join("release"), "").unwrap();
    let (result, seen) = turn.join().unwrap();
    result.expect("a slow sink does not fail the turn");
    let chunks = seen
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ContentDelta)
        .count();
    assert_eq!(chunks, 400);
}

// ---- forms ----------------------------------------------------------------

fn form_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "title": "Name", "minLength": 1},
            "count": {"type": "integer", "minimum": 0},
            "agree": {"type": "boolean", "default": true},
            "color": {"type": "string", "enum": ["red", "green"]},
            "tags": {"type": "array", "items": {"type": "string", "enum": ["a", "b"]}},
        },
        "required": ["name", "count", "agree"],
    })
}

fn form(extra: Value) -> Value {
    let mut form = json!({"params": {
        "mode": "form",
        "message": "Pick the details.",
        "requestedSchema": form_schema(),
    }});
    form.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    form
}

fn typed_reply() -> spark_common::forms::FormReply {
    serde_json::from_value(json!({
        "action": "accept",
        "content": {"name": "x", "count": 0, "agree": false, "tags": []},
    }))
    .unwrap()
}

/// Form requests the sink saw, as `(status, request)`.
fn forms_seen(events: &[TurnStreamEvent]) -> Vec<(String, Value)> {
    events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::RequestUserInputRequested)
        .map(|event| {
            let request = event.request_user_input.clone().unwrap();
            let status = request["status"].as_str().unwrap_or("pending").to_string();
            (status, request)
        })
        .collect()
}

/// What Spark replied to the fake's request `id`.
fn reply_to(dir: &Path, id: &str) -> Vec<Value> {
    log(dir)
        .into_iter()
        .filter(|entry| entry["received"]["id"] == id && entry["received"].get("method").is_none())
        .map(|entry| entry["received"].clone())
        .collect()
}

#[test]
fn acp_chat_form_reaches_its_live_turn_once_with_typed_values() {
    let dir = tempfile::tempdir().unwrap();
    let request = request(dir.path(), json!({"forms": [form(json!({}))]}), json!({}));
    let (project, conversation) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let (result, seen) = run(&request, {
        let delivered = Arc::clone(&delivered);
        move |event| {
            let Some(asked) = &event.request_user_input else {
                return;
            };
            let id = asked["request_id"].as_str().unwrap();
            let mut delivered = delivered.lock().unwrap();
            delivered.push(acp::answer(
                &project,
                &conversation,
                "acp-form-stale",
                typed_reply(),
            ));
            delivered.push(acp::answer(&project, &conversation, id, typed_reply()));
            // A duplicate reply finds nothing pending.
            delivered.push(acp::answer(
                &project,
                &conversation,
                id,
                spark_common::forms::FormReply::Decline,
            ));
        }
    });
    assert_eq!(
        result.expect("answered turn").stop_reason,
        StopReason::EndTurn
    );
    assert_eq!(*delivered.lock().unwrap(), [false, true, false]);
    let forms = forms_seen(&seen);
    assert_eq!(forms.len(), 1, "an answered form never expires: {forms:?}");
    let (status, asked) = &forms[0];
    assert_eq!(status, "pending");
    assert!(asked["request_id"]
        .as_str()
        .unwrap()
        .starts_with("acp-form-"));
    assert_eq!(asked["questions"], json!([]));
    // The form shows people the Spark request ID their reply answers.
    assert_eq!(
        asked["form"],
        json!({
            "request_id": asked["request_id"],
            "harness": "acp:fake",
            "harness_label": "Fake ACP",
            "message": "Pick the details.",
            "schema": form_schema(),
        })
    );
    let event = seen
        .iter()
        .find(|event| event.request_user_input.is_some())
        .unwrap();
    let correlation = event.details.as_ref().unwrap();
    assert_eq!(correlation["session_id"], "fake-session");
    assert_eq!(correlation["rpc_request_id"], "form-1");
    assert!(correlation["connection"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    assert!(event.source.app_turn_id.is_some(), "tied to its prompt");
    assert_eq!(
        reply_to(dir.path(), "form-1"),
        [json!({"jsonrpc": "2.0", "id": "form-1", "result": {
            "action": "accept",
            "content": {"name": "x", "count": 0, "agree": false, "tags": []},
        }})],
        "false, zero and an empty selection reach the harness as typed; omitted fields stay omitted"
    );
    let id = asked["request_id"].as_str().unwrap();
    assert!(
        !acp::answer(
            &request.project_path,
            &request.conversation_id,
            id,
            typed_reply()
        ),
        "an ended turn's form is never revived"
    );
}

#[test]
fn acp_pending_forms_expire_on_cancel_withdrawal_and_connection_loss() {
    // An interrupt cancels the form with the harness, then the prompt.
    let dir = tempfile::tempdir().unwrap();
    let script = json!({"forms": [form(json!({}))], "await_cancel": "respond"});
    let request = request(dir.path(), script, json!({}));
    let (project, conversation) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let (result, seen) = run(&request, move |event| {
        if event.request_user_input.is_some() && event.status.is_none() {
            assert!(acp::interrupt(&project, &conversation));
        }
    });
    assert_eq!(
        result.expect("cancelled turn").stop_reason,
        StopReason::Cancelled
    );
    assert_eq!(
        reply_to(dir.path(), "form-1")[0]["result"],
        json!({"action": "cancel"})
    );
    assert_eq!(received(dir.path(), "session/cancel").len(), 1);
    let forms = forms_seen(&seen);
    let statuses = forms
        .iter()
        .map(|(status, _)| status.as_str())
        .collect::<Vec<_>>();
    assert_eq!(statuses, ["pending", "expired"]);
    assert_eq!(forms[0].1["request_id"], forms[1].1["request_id"]);

    // The harness withdraws its own request.
    let dir = tempfile::tempdir().unwrap();
    let script = json!({"forms": [form(json!({"withdraw": true}))]});
    let (result, seen) = run(&request_in(dir.path(), script), |_| {});
    assert_eq!(
        result.expect("finished turn").stop_reason,
        StopReason::EndTurn
    );
    let statuses = forms_seen(&seen)
        .into_iter()
        .map(|(status, _)| status)
        .collect::<Vec<_>>();
    assert_eq!(statuses, ["pending", "expired"]);
    assert!(reply_to(dir.path(), "form-1")[0].get("error").is_some());

    // The connection is lost while the form waits.
    let dir = tempfile::tempdir().unwrap();
    let crash = dir.path().join("crash");
    let script = json!({"forms": [form(json!({"exit": crash}))]});
    let (result, seen) = run(&request_in(dir.path(), script), move |event| {
        if event.request_user_input.is_some() {
            std::fs::write(&crash, "").unwrap();
        }
    });
    result.err().expect("a lost connection fails the turn");
    let statuses = forms_seen(&seen)
        .into_iter()
        .map(|(status, _)| status)
        .collect::<Vec<_>>();
    assert_eq!(statuses, ["pending", "expired"]);
}

fn request_in(dir: &Path, script: Value) -> AgentTurnRequest {
    request(dir, script, json!({}))
}

#[test]
fn acp_sdk_shaped_form_is_asked_as_sent_and_answered_exactly() {
    use agent_client_protocol::v1::schema::{
        CreateElicitationResponse, ElicitationSchema, EnumOption, IntegerPropertySchema,
        MultiSelectItems, MultiSelectPropertySchema, StringPropertySchema, TitledMultiSelectItems,
    };
    use spark_common::forms::check_reply;
    let meta = serde_json::Map::from_iter([("origin".to_string(), json!("sdk"))]);
    let option = |value: &str, title: &str| {
        EnumOption::new(value, title)
            .description(format!("About {title}"))
            .meta(meta.clone())
    };
    let mut tags = MultiSelectPropertySchema::titled(Vec::new()).max_items(2);
    tags.items = MultiSelectItems::Titled(
        TitledMultiSelectItems::new(vec![option("a", "A"), option("b", "B")]).meta(meta.clone()),
    );
    let sdk = ElicitationSchema::new()
        .property(
            "plan",
            StringPropertySchema::new()
                .title("Plan")
                .one_of(vec![option("free", "Free"), option("pro", "Pro")])
                .meta(meta.clone()),
            true,
        )
        .property(
            "count",
            IntegerPropertySchema::new()
                .minimum(0)
                .maximum(9_007_199_254_740_993),
            true,
        )
        .property("tags", tags, false)
        .property("note", StringPropertySchema::new(), false);
    // What the SDK emits, plus the nulls it reads as omitted keywords.
    let mut schema = serde_json::to_value(&sdk).unwrap();
    for (field, key) in [
        ("note", "minLength"),
        ("note", "pattern"),
        ("note", "format"),
        ("note", "default"),
        ("note", "enum"),
        ("plan", "enum"),
        ("plan", "description"),
        ("count", "default"),
        ("tags", "default"),
        ("tags", "minItems"),
    ] {
        schema["properties"][field][key] = Value::Null;
    }
    schema["properties"]["plan"]["oneOf"][0]["description"] = Value::Null;
    schema["properties"]["tags"]["items"]["anyOf"][1]["_meta"] = Value::Null;
    serde_json::from_value::<ElicitationSchema>(schema.clone()).expect("the SDK reads it");

    let dir = tempfile::tempdir().unwrap();
    let params = json!({"mode": "form", "message": "Plan?", "requestedSchema": schema});
    let request = request_in(dir.path(), json!({"forms": [{"params": params}]}));
    let (project, conversation) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let answer = |count: Value| {
        serde_json::from_value::<spark_common::forms::FormReply>(json!({
            "action": "accept",
            "content": {"plan": "pro", "count": count, "tags": ["b"]},
        }))
        .unwrap()
    };
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let (result, seen) = run(&request, {
        let delivered = Arc::clone(&delivered);
        move |event| {
            let Some(asked) = &event.request_user_input else {
                return;
            };
            let (id, asked) = (
                asked["request_id"].as_str().unwrap(),
                &asked["form"]["schema"],
            );
            // One past the declared maximum is refused, though both round to the same f64.
            let over = answer(json!(9_007_199_254_740_994_u64));
            assert!(check_reply(asked, &over)
                .unwrap_err()
                .contains("above its maximum"));
            let reply = answer(json!(9_007_199_254_740_993_u64));
            assert_eq!(check_reply(asked, &reply), Ok(()));
            let sent = acp::answer(&project, &conversation, id, reply);
            delivered.lock().unwrap().push(sent);
        }
    });
    assert_eq!(
        result.expect("answered turn").stop_reason,
        StopReason::EndTurn
    );
    assert_eq!(*delivered.lock().unwrap(), [true]);
    let forms = forms_seen(&seen);
    assert_eq!(forms.len(), 1, "{forms:?}");
    assert_eq!(forms[0].1["form"]["schema"], schema, "the original schema");
    assert!(other(&seen, "warning").is_empty());
    let replied = &reply_to(dir.path(), "form-1")[0]["result"];
    assert_eq!(
        replied.to_string(),
        r#"{"action":"accept","content":{"plan":"pro","count":9007199254740993,"tags":["b"]}}"#
    );
    serde_json::from_value::<CreateElicitationResponse>(replied.clone())
        .expect("the SDK reads the reply");
}

#[test]
fn acp_unsupported_form_schemas_and_modes_are_refused_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let nested = json!({"params": {
        "mode": "form",
        "message": "Address?",
        "requestedSchema": {"type": "object", "properties": {"address": {"type": "object"}}},
    }});
    let url = json!({"params": {
        "mode": "url",
        "message": "Sign in.",
        "elicitationId": "e-1",
        "url": "https://example.com/sign-in",
    }});
    let (result, seen) = run(
        &request_in(dir.path(), json!({"forms": [nested, url]})),
        |_| {},
    );
    assert_eq!(
        result.expect("finished turn").stop_reason,
        StopReason::EndTurn
    );
    for id in ["form-1", "form-2"] {
        assert_eq!(reply_to(dir.path(), id)[0]["error"]["code"], -32602, "{id}");
    }
    assert!(forms_seen(&seen).is_empty(), "nothing is asked in part");
    let warnings = other(&seen, "warning");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.as_deref().unwrap().contains("address"));
}

#[test]
fn acp_flow_form_is_one_clarification_answered_through_its_handler() {
    use spark_agent_adapter::codergen::{
        ClarificationHandler, ClarificationPoll, ClarificationReply,
    };
    let asked = Arc::new(Mutex::new(Vec::new()));
    let handler = |asked: Arc<Mutex<Vec<Value>>>, answer: Result<ClarificationReply, String>| {
        Arc::new(move |params: Value| {
            asked.lock().unwrap().push(params);
            let (mut polls, answer) = (0, answer.clone());
            Ok(Box::new(move || {
                polls += 1;
                match polls {
                    1 => Ok(None),
                    _ => answer.clone().map(Some),
                }
            }) as ClarificationPoll)
        }) as ClarificationHandler
    };

    let dir = tempfile::tempdir().unwrap();
    let request = request_in(dir.path(), json!({"forms": [form(json!({}))]}));
    let answer = Ok(ClarificationReply::Form(typed_reply()));
    let (result, seen) = run_flow(&request, Some(handler(Arc::clone(&asked), answer)), |_| {});
    assert_eq!(
        result.expect("answered turn").stop_reason,
        StopReason::EndTurn
    );
    let asked = asked.lock().unwrap().clone();
    let (_, request_asked) = forms_seen(&seen).remove(0);
    assert_eq!(
        asked,
        [json!({"agent_request_id": request_asked["request_id"], "form": request_asked["form"]})],
        "the whole form is one clarification"
    );
    assert_eq!(
        reply_to(dir.path(), "form-1")[0]["result"],
        json!({"action": "accept", "content": {"name": "x", "count": 0, "agree": false, "tags": []}})
    );

    // A flow whose wait fails, such as a cancelled run, fails the turn.
    let dir = tempfile::tempdir().unwrap();
    let request = request_in(dir.path(), json!({"forms": [form(json!({}))]}));
    let failed = Err("Run canceled while waiting for clarification".to_string());
    let (result, _) = run_flow(&request, Some(handler(Arc::default(), failed)), |_| {});
    let error = result.err().expect("failed wait");
    assert!(error.contains("Run canceled while waiting"), "{error}");
}

#[test]
fn acp_chat_form_answers_reach_only_the_live_turn_that_asked() {
    use spark_agent_adapter::{
        AgentRequestUserInputAnswerRequest, AgentTurnBackend, AgentTurnOutput,
        RustLlmAgentTurnBackend,
    };
    let dir = tempfile::tempdir().unwrap();
    let request = request(dir.path(), json!({"forms": [form(json!({}))]}), json!({}));
    let answer = {
        let (project, conversation) = (
            request.project_path.clone(),
            request.conversation_id.clone(),
        );
        move |request_id: &str| -> AgentTurnOutput {
            let request: AgentRequestUserInputAnswerRequest = serde_json::from_value(json!({
                "conversation_id": conversation,
                "project_path": project,
                "request_id": request_id,
                "assistant_turn_id": "turn-assistant",
                "form": typed_reply(),
                "provider": "acp:fake",
            }))
            .unwrap();
            RustLlmAgentTurnBackend::new(unified_llm_adapter::Client::new())
                .answer_request_user_input(request)
                .unwrap()
        }
    };
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let (result, seen) = chat(request, {
        let (delivered, answer) = (Arc::clone(&delivered), answer.clone());
        move |event| {
            if let Some(asked) = event
                .request_user_input
                .as_ref()
                .filter(|_| event.status.is_none())
            {
                delivered
                    .lock()
                    .unwrap()
                    .push(answer(asked["request_id"].as_str().unwrap()));
            }
        }
    });
    result.expect("answered chat turn");
    let delivered = delivered.lock().unwrap().clone();
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].events[0].source.raw_kind.as_deref(),
        Some("request_user_input_answer_delivered")
    );
    assert_eq!(
        reply_to(dir.path(), "form-1")[0]["result"]["action"],
        "accept"
    );
    let (_, asked) = forms_seen(&seen).remove(0);
    let stale = answer(asked["request_id"].as_str().unwrap());
    assert_eq!(
        stale
            .thread_resume_failure
            .and_then(|failure| failure.error_code)
            .as_deref(),
        Some("request_user_input_not_pending"),
        "an ended turn's form is never answered from history"
    );
}

#[test]
fn acp_defers_end_turn_while_background_tool_call_runs_and_delivers_wakeup() {
    let dir = tempfile::tempdir().unwrap();
    let gate = dir.path().join("bg-done");
    let script = json!({
        "updates": [
            {"sessionUpdate": "tool_call", "toolCallId": "bg-1", "title": "cargo test",
             "kind": "execute", "status": "in_progress",
             "rawInput": {"CommandLine": "cargo test"}},
            chunk("agent_message_chunk", "Running tests now."),
        ],
        "post_prompt_gate": gate,
        "post_prompt_updates": [
            {"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "status": "completed",
             "rawOutput": {"output": "1 passed", "exit_code": 0}},
            chunk("agent_message_chunk", "All tests passed."),
            {"sessionUpdate": "usage_update", "used": 1200, "size": 200000},
        ],
    });
    let req = request(dir.path(), script, json!({}));
    let saw_commentary = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let saw_completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = std::thread::spawn({
        let req = req.clone();
        let saw_commentary = Arc::clone(&saw_commentary);
        let saw_completed = Arc::clone(&saw_completed);
        move || {
            run(&req, move |event| {
                if event.kind == TurnStreamEventKind::ContentCompleted
                    && event.message.as_deref() == Some("Running tests now.")
                {
                    saw_commentary.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if event.kind == TurnStreamEventKind::TurnCompleted {
                    saw_completed.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            })
        }
    });

    // Wait until end_turn closed the commentary while holding the turn open.
    let started = std::time::Instant::now();
    while !saw_commentary.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "commentary closes when deferred end_turn arrives"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !saw_completed.load(std::sync::atomic::Ordering::SeqCst),
        "turn stays active while the background tool call is still in_progress"
    );

    // Release the background command completion and post-wakeup response.
    std::fs::write(&gate, "").unwrap();
    let (result, seen) = handle.join().unwrap();
    let turn = result.expect("deferred turn completes");
    assert_eq!(turn.stop_reason, StopReason::EndTurn);
    assert_eq!(
        turn.output.final_assistant_text.as_deref(),
        Some("All tests passed.")
    );
    let completed_messages = seen
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ContentCompleted)
        .map(|event| (event.message.clone().unwrap(), event.phase.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        completed_messages,
        [
            (
                "Running tests now.".to_string(),
                Some("commentary".to_string())
            ),
            (
                "All tests passed.".to_string(),
                Some("final_answer".to_string())
            ),
        ]
    );
    let completed_tool = seen
        .iter()
        .find(|event| event.kind == TurnStreamEventKind::ToolCallCompleted)
        .and_then(|event| event.tool_call.as_ref())
        .expect("background tool call completes");
    assert_eq!(completed_tool["command"], "cargo test");
    assert_eq!(completed_tool["output"]["output"], "1 passed");
    assert_eq!(terminals(&seen), [Some("end_turn".to_string())]);
    assert!(acp::close_session(&req.project_path, &req.conversation_id));
}

#[test]
fn acp_interrupt_during_deferred_background_wait_cancels_cleanly_or_forces_stop() {
    // Settled cancel: harness closes the background tool call after session/cancel.
    let dir = tempfile::tempdir().unwrap();
    let script = json!({
        "updates": [
            {"sessionUpdate": "tool_call", "toolCallId": "bg-1", "title": "cargo test",
             "kind": "execute", "status": "in_progress"},
            chunk("agent_message_chunk", "Running."),
        ],
        "post_prompt_await_cancel": "respond",
        "post_prompt_updates": [
            {"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "status": "failed"},
        ],
    });
    let req = request(dir.path(), script, json!({}));
    let (project, conversation) = (req.project_path.clone(), req.conversation_id.clone());
    let (result, _) = run(&req, move |event| {
        if event.kind == TurnStreamEventKind::ContentCompleted {
            assert!(acp::interrupt(&project, &conversation));
        }
    });
    let turn = result.expect("settled cancel");
    assert_eq!(turn.stop_reason, StopReason::Cancelled);
    assert!(turn.record.is_some());
    assert_eq!(received(dir.path(), "session/cancel").len(), 1);
    assert!(acp::close_session(&req.project_path, &req.conversation_id));

    // Ignored cancel: process is force-stopped after CANCEL_GRACE and continuity dropped.
    let dir = tempfile::tempdir().unwrap();
    let script = json!({
        "updates": [
            {"sessionUpdate": "tool_call", "toolCallId": "bg-2", "title": "cargo test",
             "kind": "execute", "status": "in_progress"},
            chunk("agent_message_chunk", "Running."),
        ],
        "post_prompt_await_cancel": "ignore",
    });
    let req = request(dir.path(), script, json!({}));
    let (project, conversation) = (req.project_path.clone(), req.conversation_id.clone());
    let (result, _) = run(&req, move |event| {
        if event.kind == TurnStreamEventKind::ContentCompleted {
            assert!(acp::interrupt(&project, &conversation));
        }
    });
    let turn = result.expect("forced stop");
    assert_eq!(turn.stop_reason, StopReason::Cancelled);
    assert!(turn.record.is_none());
    assert!(turn.output.thread_resume_failure.is_some());
}

#[test]
fn acp_daemon_tool_call_does_not_block_and_warm_session_reuses_process_across_turns() {
    let dir = tempfile::tempdir().unwrap();
    let options = json!([{
        "id": "model", "name": "Model", "category": "model", "type": "select",
        "currentValue": "small",
        "options": [{"value": "small", "name": "Small"}, {"value": "Large-1", "name": "Large"}],
    }]);
    let script = json!({
        "resume": true,
        "config_options": options,
        "updates": [
            {"sessionUpdate": "tool_call", "toolCallId": "srv-1", "title": "vite dev",
             "kind": "execute", "status": "in_progress",
             "rawInput": {"CommandLine": "npm run dev", "IsDaemon": true}},
        ],
        "answers": ["Server started.", "Still running.", "Resumed cold."],
    });
    let first = request(dir.path(), script, json!({}));
    let turn1 = run(&first, |_| {})
        .0
        .expect("first turn finishes despite daemon");
    assert_eq!(
        turn1.output.final_assistant_text.as_deref(),
        Some("Server started.")
    );
    let record = turn1.record.expect("session record");
    assert_eq!(launches(dir.path()), 1);

    // Second turn with changed model reuses the warm process without relaunching.
    let mut second = with_history(first.clone(), &record);
    second.model = Some("Large-1".to_string());
    let turn2 = run(&second, |_| {}).0.expect("second turn on warm process");
    assert_eq!(
        turn2.output.final_assistant_text.as_deref(),
        Some("Still running.")
    );
    assert_eq!(launches(dir.path()), 1);
    assert_eq!(received(dir.path(), "session/resume").len(), 0);
    let set = received(dir.path(), "session/set_config_option");
    assert_eq!(set.len(), 1);
    assert_eq!(
        (set[0]["configId"].clone(), set[0]["value"].clone()),
        (json!("model"), json!("Large-1"))
    );

    // Closing the pooled session drops the process; the next turn resumes cold.
    assert!(acp::close_session(
        &first.project_path,
        &first.conversation_id
    ));
    let turn3 = run(&with_history(first, &record), |_| {})
        .0
        .expect("cold resume after close");
    assert_eq!(
        turn3.output.final_assistant_text.as_deref(),
        Some("Resumed cold.")
    );
    assert_eq!(launches(dir.path()), 2);
    assert_eq!(received(dir.path(), "session/resume").len(), 1);
}

#[test]
fn acp_delivers_live_message_during_deferred_background_wait() {
    let dir = tempfile::tempdir().unwrap();
    let gate = dir.path().join("bg-done");
    let script = json!({
        "prompt_updates": [
            [
                {"sessionUpdate": "tool_call", "toolCallId": "bg-1", "title": "just test",
                 "kind": "execute", "status": "in_progress",
                 "rawInput": {"CommandLine": "just test"}},
                chunk("agent_message_chunk", "Running tests. Should I push when green?"),
            ],
            [
                chunk("agent_message_chunk", "Got it — will push once tests pass."),
            ],
        ],
        "post_prompt_gate": gate,
        "post_prompt_updates": [
            {"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "status": "completed",
             "rawOutput": {"output": "all passed", "exit_code": 0}},
            chunk("agent_message_chunk", "Tests passed and pushed."),
            {"sessionUpdate": "usage_update", "used": 1400, "size": 200000},
        ],
    });
    let req = request(dir.path(), script, json!({}));
    let saw_question = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let saw_reply = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = std::thread::spawn({
        let req = req.clone();
        let saw_question = Arc::clone(&saw_question);
        let saw_reply = Arc::clone(&saw_reply);
        move || {
            run(&req, move |event| {
                if event.kind == TurnStreamEventKind::ContentCompleted {
                    match event.message.as_deref() {
                        Some("Running tests. Should I push when green?") => {
                            saw_question.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        Some("Got it — will push once tests pass.") => {
                            saw_reply.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        _ => {}
                    }
                }
            })
        }
    });

    // Wait until prompt 1 ended into DeferredEnd::WaitingForTools.
    let started = std::time::Instant::now();
    while !saw_question.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "question commentary closes when deferred end_turn arrives"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Deliver the user's answer live while bg-1 is still running.
    assert!(
        acp::send_message(
            &req.project_path,
            &req.conversation_id,
            "msg-1",
            "Yes, go ahead and push."
        ),
        "live message must be accepted during post-end_turn background tool wait"
    );

    // Wait until the agent answers the live message while bg-1 is STILL open.
    let replied = std::time::Instant::now();
    while !saw_reply.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            replied.elapsed() < std::time::Duration::from_secs(10),
            "agent replies to live message before background tool completes"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Release bg-1 completion and post-wakeup final answer.
    std::fs::write(&gate, "").unwrap();
    let (result, seen) = handle.join().unwrap();
    let turn = result.expect("turn settles after background wakeup");
    assert_eq!(turn.stop_reason, StopReason::EndTurn);
    assert_eq!(
        turn.output.final_assistant_text.as_deref(),
        Some("Tests passed and pushed.")
    );
    assert!(
        seen.iter().any(|event| {
            event.kind == TurnStreamEventKind::Other("user_message_delivered".into())
                && event.source.item_id.as_deref() == Some("msg-1")
                && event.status.as_deref() == Some("delivered")
        }),
        "must emit user_message_delivered for the live reply"
    );
    let prompts = received(dir.path(), "session/prompt");
    let texts = prompts
        .iter()
        .map(|prompt| prompt["prompt"][0]["text"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["Do the thing.", "Yes, go ahead and push."]);
    assert_eq!(launches(dir.path()), 1);
    assert!(acp::close_session(&req.project_path, &req.conversation_id));
}

#[test]
fn acp_post_tool_wakeup_does_not_settle_after_thought_before_delayed_tools_and_answer() {
    let dir = tempfile::tempdir().unwrap();
    let gate = dir.path().join("bg-done");
    let script = json!({
        "updates": [
            {"sessionUpdate": "tool_call", "toolCallId": "bg-1", "title": "bench",
             "kind": "execute", "status": "in_progress",
             "rawInput": {"CommandLine": "bench"}},
            chunk("agent_message_chunk", "Running bench."),
        ],
        "post_prompt_gate": gate,
        "post_prompt_updates": [
            {"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "status": "completed",
             "rawOutput": {"output": "ok", "exit_code": 0}},
            chunk("agent_thought_chunk", "Planning four file edits."),
            // Usage arriving with only thoughts open must not shrink the deadline to 2s.
            {"sessionUpdate": "usage_update", "used": 1200, "size": 200000},
            {"sleep_ms": 2500},
            {"sessionUpdate": "tool_call", "toolCallId": "edit-1", "title": "edit_file",
             "kind": "edit", "status": "in_progress"},
            {"sessionUpdate": "tool_call_update", "toolCallId": "edit-1", "status": "completed",
             "rawOutput": {"output": "edited"}},
            chunk("agent_message_chunk", "Applied edits."),
            {"sessionUpdate": "usage_update", "used": 1500, "size": 200000},
        ],
    });
    let req = request(dir.path(), script, json!({}));
    let saw_commentary = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = std::thread::spawn({
        let req = req.clone();
        let saw_commentary = Arc::clone(&saw_commentary);
        move || {
            run(&req, move |event| {
                if event.kind == TurnStreamEventKind::ContentCompleted
                    && event.message.as_deref() == Some("Running bench.")
                {
                    saw_commentary.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            })
        }
    });

    let started = std::time::Instant::now();
    while !saw_commentary.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "commentary closes when deferred end_turn arrives"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::write(&gate, "").unwrap();

    let (result, seen) = handle.join().unwrap();
    let turn = result.expect("turn waits through post-thought gap for delayed tool and answer");
    assert_eq!(turn.stop_reason, StopReason::EndTurn);
    assert_eq!(
        turn.output.final_assistant_text.as_deref(),
        Some("Applied edits.")
    );
    assert!(
        seen.iter().any(|event| {
            event.kind == TurnStreamEventKind::ToolCallCompleted
                && event.source.item_id.as_deref() == Some("edit-1")
        }),
        "delayed tool call must be captured in the same turn"
    );
    assert!(acp::close_session(&req.project_path, &req.conversation_id));
}

#[cfg(unix)]
#[test]
fn antigravity_usage_reads_windows_from_headless_slash_usage_and_reports_errors() {
    use spark_common::agent_settings::{
        AcpHarness, AcpPermissionPolicy, EnvironmentInheritancePolicy,
    };
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("agy");
    let fake = |body: &str| {
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    };
    let harness = AcpHarness {
        name: "Antigravity".into(),
        executable: script.to_string_lossy().into_owned(),
        args: vec!["acp".into()],
        env: BTreeMap::new(),
        permission_policy: AcpPermissionPolicy::Allow,
        system_instructions: None,
    };
    assert!(acp::is_antigravity_harness(&harness));
    assert!(!acp::is_antigravity_harness(&AcpHarness {
        executable: "/usr/bin/other-agent".into(),
        ..harness.clone()
    }));

    fake(
        r#"if [ "$1" = "-p" ] && [ "$2" = "/usage" ] && [ "$3" = "--output-format" ] && [ "$4" = "json" ]; then
cat <<'EOF'
{"status":"SUCCESS","num_turns":0,"command":{"name":"usage","data":{"groups":[{"name":"Gemini Models","buckets":[{"window":"weekly","remaining_fraction":0.75,"reset_time":"2026-11-28T13:35:29Z"},{"window":"5h","remaining_fraction":0.1,"reset_time":"2026-11-21T18:35:29Z"}]},{"name":"Claude and GPT models","buckets":[{"window":"weekly","remaining_fraction":1.0,"reset_time":""}]}]}}}
EOF
else
  echo "unexpected args: $*" >&2
  exit 1
fi"#,
    );
    let windows = acp::read_antigravity_usage(&harness, EnvironmentInheritancePolicy::InheritAll)
        .expect("antigravity usage");
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[0].label, "Weekly · Gemini Models");
    assert!((windows[0].used_percent - 25.0).abs() < 1e-6);
    assert_eq!(
        windows[0].resets_at.as_deref(),
        Some("2026-11-28T13:35:29Z")
    );
    assert_eq!(windows[1].label, "5-hour · Gemini Models");
    assert!((windows[1].used_percent - 90.0).abs() < 1e-6);
    assert_eq!(
        windows[1].resets_at.as_deref(),
        Some("2026-11-21T18:35:29Z")
    );
    assert_eq!(windows[2].label, "Weekly · Claude and GPT models");
    assert!((windows[2].used_percent - 0.0).abs() < 1e-6);
    assert_eq!(windows[2].resets_at, None);

    fake(r#"printf '{"status":"ERROR","error":"Please sign in first."}'"#);
    let error = acp::read_antigravity_usage(&harness, EnvironmentInheritancePolicy::InheritAll)
        .expect_err("error status must fail");
    assert!(error.contains("Please sign in first."), "{error}");

    fake(r#"printf '{"status":"SUCCESS","command":{"data":{}}}'"#);
    let error = acp::read_antigravity_usage(&harness, EnvironmentInheritancePolicy::InheritAll)
        .expect_err("unknown shape must fail");
    assert!(error.contains("unknown format"), "{error}");
}
