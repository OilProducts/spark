use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use spark_agent_adapter::{AgentError, AgentTurnBackend, AgentTurnOutput, AgentTurnRequest};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::{resolve_settings_with_env, SettingsOverrides, SparkSettings};
use spark_storage::conversation::CommitNotice;
use spark_storage::conversation::{ConversationMetadataPatch, ConversationMutation};
use spark_storage::settings::{load_core_settings, read_settings_document};
use spark_storage::ConversationRepository;
use spark_workspace::live::{
    conversation_changes_envelope, conversations_changes_envelope, lagged_resync_envelopes,
    validate_live_query,
};
use spark_workspace::settings::{update_workspace_settings, workspace_settings};
use spark_workspace::utility::utility_complete;
use spark_workspace::{
    ConversationTurnRequest, LiveEnvelope, RawLiveQuery, RunLaunchRequest,
    WorkspaceConversationService, WorkspaceError,
};

const PROJECT: &str = "/projects/utility";

/// Holds the shared env lock with the Claude binary override cleared, so the
/// fake CLI stored in spark.toml is the one that runs.
struct ClaudeEnv {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<String>,
}

fn claude_env() -> ClaudeEnv {
    let lock = super::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let previous = std::env::var("SPARK_CLAUDE_CODE_BIN").ok();
    std::env::remove_var("SPARK_CLAUDE_CODE_BIN");
    ClaudeEnv {
        _lock: lock,
        previous,
    }
}

impl Drop for ClaudeEnv {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("SPARK_CLAUDE_CODE_BIN", previous);
        }
    }
}

fn fixture() -> (tempfile::TempDir, SparkSettings) {
    let temp = tempfile::tempdir().unwrap();
    let settings = resolve_settings_with_env(
        &SettingsOverrides {
            data_dir: Some(temp.path().join("home")),
            ..Default::default()
        },
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    fs::create_dir_all(&settings.config_dir).unwrap();
    (temp, settings)
}

fn core(settings: &SparkSettings) -> PathBuf {
    settings.config_dir.join("spark.toml")
}

fn save_utility(settings: &SparkSettings, value: Value) -> Result<Value, WorkspaceError> {
    let revision = read_settings_document(&core(settings)).unwrap().revision;
    update_workspace_settings(
        settings,
        &crate::shared_store(&(settings).data_dir),
        serde_json::from_value(
            json!({"section": "utility_models", "expected_revision": revision, "value": value}),
        )
        .unwrap(),
    )
}

/// A fake Claude Code CLI: counts calls, echoes `reply`, and exits `status`.
#[cfg(unix)]
fn fake_claude(settings: &SparkSettings, reply: &str, delay: &str, status: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let calls = settings.config_dir.join("claude-calls");
    let script = settings.config_dir.join("fake-claude.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat >> '{}.input'\necho call >> '{}'\nsleep {delay}\necho '{reply}'\nexit {status}\n",
            calls.display(),
            calls.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let path = core(settings);
    let text = fs::read_to_string(&path).unwrap_or_default();
    fs::write(
        &path,
        format!(
            "{text}\n[agents.native]\nclaude_binary = '{}'\n\n[utility_models]\nprovider = 'claude-code'\n",
            script.display()
        ),
    )
    .unwrap();
    calls
}

/// A fake utility call that cannot answer until the returned gate file exists,
/// so whatever finishes before the test creates it did not wait for the call.
/// Without the gate it fails after 30s, and no title is generated.
fn fake_claude_behind_gate(settings: &SparkSettings, reply: &str) -> (PathBuf, PathBuf) {
    let gate = settings.config_dir.join("utility-gate");
    let wait = format!(
        "0; i=0; while [ ! -e '{gate}' ]; do i=$((i+1)); [ $i -lt 300 ] || exit 1; sleep 0.1; done",
        gate = gate.display()
    );
    (fake_claude(settings, reply, &wait, 0), gate)
}

fn call_count(calls: &Path) -> usize {
    fs::read_to_string(calls)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

struct ReplyBackend;

impl AgentTurnBackend for ReplyBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        let text = format!("Reply to {}", request.prompt);
        Ok(AgentTurnOutput {
            final_assistant_text: Some(text.clone()),
            events: vec![TurnStreamEvent {
                kind: TurnStreamEventKind::ContentCompleted,
                channel: Some(TurnStreamChannel::Assistant),
                source: TurnStreamSource {
                    app_turn_id: Some(request.prompt.clone()),
                    item_id: Some("final".into()),
                    ..TurnStreamSource::default()
                },
                content_delta: Some(text.clone()),
                message: Some(text),
                tool_call: None,
                request_user_input: None,
                token_usage: None,
                error: None,
                error_code: None,
                details: None,
                phase: Some("final_answer".into()),
                status: None,
            }],
            ..AgentTurnOutput::default()
        })
    }
}

/// A service whose title changes arrive on the receiver: the commit notice
/// of each write that changed a conversation's stored title after the
/// conversation first appeared.
fn service_with_updates(
    settings: SparkSettings,
) -> (WorkspaceConversationService, mpsc::Receiver<CommitNotice>) {
    let (sender, receiver) = mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    let service = crate::conversations_with_backend(settings, Arc::new(ReplyBackend));
    let store = Arc::downgrade(service.store());
    let titles = std::sync::Mutex::new(std::collections::HashMap::<String, Value>::new());
    service
        .store()
        .set_commit_listener(Arc::new(move |notice: &CommitNotice| {
            let Some(store) = store.upgrade() else {
                return;
            };
            for (conversation_id, _) in &notice.conversations {
                let Ok(Some(row)) = store
                    .database()
                    .read(|read| read.conversation_row(conversation_id))
                else {
                    continue;
                };
                let title = row.payload.map(|payload| payload["title"].clone());
                let title = title.unwrap_or(Value::Null);
                let previous = titles
                    .lock()
                    .unwrap()
                    .insert(conversation_id.clone(), title.clone());
                if previous.is_some_and(|previous| previous != title) {
                    let _ = sender.lock().unwrap().send(notice.clone());
                }
            }
        }));
    (service, receiver)
}

fn request(message: &str) -> ConversationTurnRequest {
    ConversationTurnRequest {
        project_path: PROJECT.into(),
        message: message.into(),
        provider: Some("codex".into()),
        ..Default::default()
    }
}

fn turn(service: &WorkspaceConversationService, id: &str, message: &str) -> Value {
    service
        .execute_turn_with_progress_payloads(id, request(message), |_| {})
        .unwrap()
}

/// The thread-list row and the conversation row a live client catching up
/// past a generated title's commit receives, each with its id and revision.
fn title_updates(
    service: &WorkspaceConversationService,
    updates: &mpsc::Receiver<CommitNotice>,
    wait: Duration,
) -> Option<(Value, Value)> {
    let notice = updates.recv_timeout(wait).ok()?;
    let (conversation_id, project_path) = notice.conversations[0].clone();
    let generation = service.store().database().generation().to_string();
    let row = |envelope: spark_workspace::LiveEnvelope| {
        let row = envelope.payload["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == conversation_id.as_str())
            .unwrap_or_else(|| panic!("{conversation_id} row in {}", envelope.payload))
            .clone();
        let mut view = row["payload"].clone();
        view["conversation_id"] = row["id"].clone();
        view["revision"] = row["revision"].clone();
        view
    };
    let (summary, _) = conversations_changes_envelope(
        service,
        Some(&project_path),
        &generation,
        notice.revision - 1,
    )
    .unwrap();
    assert_eq!(summary.event_type, "conversations.changes");
    assert_eq!(summary.resource.kind, "conversations");
    let (conversation, _) = conversation_changes_envelope(
        service,
        &conversation_id,
        &project_path,
        &generation,
        notice.revision - 1,
    )
    .unwrap();
    assert_eq!(conversation.event_type, "conversation.changes");
    Some((row(summary), row(conversation)))
}

#[test]
fn lagged_thread_list_subscriber_is_told_to_resync_conversation_summaries() {
    let query = |include: Option<&str>| {
        validate_live_query(RawLiveQuery {
            include_conversations: include.map(str::to_string),
            ..Default::default()
        })
        .unwrap()
    };
    // Summary scopes catch up from their row cursor, not a resync frame.
    for include in [Some("true"), None] {
        assert!(lagged_resync_envelopes(&query(include))
            .iter()
            .all(|envelope| !envelope.resource.kind.starts_with("conversation")));
    }

    let (_temp, settings) = fixture();
    let service = crate::conversations_with_backend(settings, Arc::new(ReplyBackend));
    turn(&service, "lagged", "first question");
    let generation = service.store().database().generation().to_string();
    // Behind its cursor, it receives the changed summary rows.
    let (caught_up, cursor) =
        conversations_changes_envelope(&service, Some(PROJECT), &generation, 0).unwrap();
    assert_eq!(caught_up.event_type, "conversations.changes");
    assert!(caught_up.payload["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == "lagged"));
    assert_eq!(cursor, Some(service.watermark().unwrap()));
    // A cursor from another generation is told to reload.
    let (reset, cursor) =
        conversations_changes_envelope(&service, Some(PROJECT), "another-generation", 0).unwrap();
    assert_eq!(reset.event_type, "conversations.reset");
    assert_eq!(cursor, None);
}

fn stored_title(service: &WorkspaceConversationService, id: &str) -> String {
    service.get_snapshot(id, Some(PROJECT)).unwrap()["title"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn utility_setting_round_trips_validates_and_leaves_older_files_unchanged() {
    let _env = claude_env();
    let (_temp, settings) = fixture();
    let older =
        "schema_version = 1\n\n[models]\nprovider = 'anthropic'\nmodel = 'claude-sonnet-4-5'\n";
    fs::write(core(&settings), older).unwrap();
    load_core_settings(&core(&settings)).unwrap();
    let view = workspace_settings(&settings).unwrap();
    assert_eq!(fs::read_to_string(core(&settings)).unwrap(), older);
    assert_eq!(view["models"]["effective"]["model"], "claude-sonnet-4-5");
    assert_eq!(view["utility_models"]["stored"], Value::Null);
    assert_eq!(view["utility_models"]["effective"], Value::Null);
    assert_eq!(view["utility_models"]["validation_errors"], json!([]));

    let group = json!({"provider": "anthropic", "llm_profile": null, "model": "claude-haiku-4-5",
        "reasoning_effort": "low", "thinking": "budget", "thinking_budget_tokens": 2048,
        "reasoning_mode": null, "reasoning_summary": null});
    let view = save_utility(&settings, group.clone()).unwrap();
    assert_eq!(view["utility_models"]["stored"], group);
    assert_eq!(view["utility_models"]["effective"], group);
    assert_eq!(
        workspace_settings(&settings).unwrap()["utility_models"]["stored"],
        group
    );
    assert_eq!(view["models"]["effective"]["model"], "claude-sonnet-4-5");
    let document = read_settings_document(&core(&settings)).unwrap();
    assert_eq!(
        document.values["utility_models"]["model"].as_str(),
        Some("claude-haiku-4-5")
    );
    load_core_settings(&core(&settings)).unwrap();

    for invalid in [
        json!({"provider": "not-a-provider"}),
        json!({"provider": "openai", "model": "claude-sonnet-4-5"}),
        json!({"provider": "anthropic", "llm_profile": "team"}),
        json!({"llm_profile": "missing"}),
        json!({"provider": "anthropic", "thinking": "budget"}),
        json!({"provider": "codex", "reasoning_summary": "verbose"}),
        json!({"provider": "openrouter"}),
    ] {
        let error = save_utility(&settings, invalid.clone()).expect_err(&invalid.to_string());
        assert!(
            matches!(error, WorkspaceError::Validation(_)),
            "{invalid}: {error}"
        );
    }
    assert_eq!(
        workspace_settings(&settings).unwrap()["utility_models"]["stored"],
        group
    );

    let view = save_utility(&settings, Value::Null).unwrap();
    assert_eq!(view["utility_models"]["stored"], Value::Null);
    assert!(!read_settings_document(&core(&settings))
        .unwrap()
        .values
        .contains_key("utility_models"));

    // A hand-edited invalid section is reported, not silently used.
    let text = fs::read_to_string(core(&settings)).unwrap();
    fs::write(
        core(&settings),
        format!("{text}\n[utility_models]\nprovider = 'not-a-provider'\n"),
    )
    .unwrap();
    let view = workspace_settings(&settings).unwrap();
    assert_eq!(view["utility_models"]["effective"], Value::Null);
    assert_eq!(
        view["utility_models"]["validation_errors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(utility_complete(&settings, "Name it.", "input").is_err());
}

#[test]
fn utility_call_is_not_configured_without_a_utility_model() {
    let _env = claude_env();
    let (_temp, settings) = fixture();
    fs::write(
        core(&settings),
        "schema_version = 1\n\n[models]\nprovider = 'codex'\n",
    )
    .unwrap();
    assert_eq!(
        utility_complete(&settings, "Name it.", "input").unwrap(),
        None
    );
}

#[cfg(unix)]
#[test]
fn utility_call_runs_the_configured_model_and_surfaces_failures() {
    let _env = claude_env();
    let (_temp, settings) = fixture();
    let calls = fake_claude(&settings, "Generated", "0", 0);
    assert_eq!(
        utility_complete(&settings, "Name it.", "input")
            .unwrap()
            .as_deref(),
        Some("Generated")
    );
    assert_eq!(call_count(&calls), 1);

    let (_temp, settings) = fixture();
    fake_claude(&settings, "ignored", "0", 2);
    assert!(matches!(
        utility_complete(&settings, "Name it.", "input"),
        Err(WorkspaceError::ServiceUnavailable(_))
    ));
}

#[cfg(unix)]
#[test]
fn first_completed_turn_gets_one_generated_title_in_the_background() {
    let _env = claude_env();
    let (_temp, settings) = fixture();
    let (calls, gate) = fake_claude_behind_gate(&settings, "\"Deploy pipeline repair.\"");
    let (service, updates) = service_with_updates(settings);

    // The turn returns while the utility call is still held open.
    let snapshot = turn(&service, "titled", "please fix the deploy pipeline");
    assert_eq!(snapshot["title"], "please fix the deploy pipeline");
    fs::write(&gate, "").unwrap();

    // Every client's thread list and the thread's own view get the title.
    let (summary, state) = title_updates(&service, &updates, Duration::from_secs(10)).unwrap();
    assert_eq!(summary["conversation_id"], "titled");
    assert_eq!(summary["project_path"], PROJECT);
    assert_eq!(summary["title"], "Deploy pipeline repair");
    assert_eq!(summary["revision"], state["revision"]);
    assert_eq!(state["title"], "Deploy pipeline repair");
    assert_eq!(stored_title(&service, "titled"), "Deploy pipeline repair");
    assert_eq!(call_count(&calls), 1);

    let snapshot = turn(&service, "titled", "and the staging one");
    assert_eq!(snapshot["title"], "Deploy pipeline repair");
    assert!(updates.recv_timeout(Duration::from_millis(1500)).is_err());
    assert_eq!(call_count(&calls), 1);
    assert_eq!(stored_title(&service, "titled"), "Deploy pipeline repair");
}

#[cfg(unix)]
#[test]
fn a_first_turn_resumed_after_a_question_gets_a_generated_title() {
    let _env = claude_env();
    let (_temp, settings) = fixture();
    fake_claude(&settings, "Deploy question", "0", 0);
    let (service, updates) = service_with_updates(settings);

    // A turn that stopped on a question finishes through the answer path,
    // which ingests the resumed output directly.
    let (prepared, _) = service
        .start_turn("asked", request("which deploy should I fix?"))
        .unwrap();
    let output = ReplyBackend
        .run_turn(service.prepare_agent_turn_request(&prepared).unwrap())
        .unwrap();
    service
        .ingest_agent_turn_output(
            "asked",
            PROJECT,
            &prepared.assistant_turn_id,
            &prepared.chat_mode,
            output,
        )
        .unwrap();

    let (summary, _) = title_updates(&service, &updates, Duration::from_secs(10)).unwrap();
    assert_eq!(summary["title"], "Deploy question");
    assert_eq!(stored_title(&service, "asked"), "Deploy question");
}

#[cfg(unix)]
#[test]
fn stored_titles_stay_and_derived_titles_remain_when_off_or_failing() {
    let _env = claude_env();
    let (_temp, settings) = fixture();
    let calls = fake_claude(&settings, "Generated", "0", 0);
    let (service, updates) = service_with_updates(settings.clone());
    ConversationRepository::new(crate::shared_store(settings.data_dir.clone()))
        .commit_conversation(
            "named",
            PROJECT,
            0,
            vec![ConversationMutation::MetadataUpdated {
                patch: ConversationMetadataPatch {
                    title: Some("Named by hand".into()),
                    ..Default::default()
                },
            }],
        )
        .unwrap();
    turn(&service, "named", "first question");
    assert!(updates.recv_timeout(Duration::from_millis(500)).is_err());
    assert_eq!(stored_title(&service, "named"), "Named by hand");
    assert_eq!(call_count(&calls), 0);

    // Off: no utility model, so the derived title stays.
    let (_temp, settings) = fixture();
    let service = crate::conversations_with_backend(settings.clone(), Arc::new(ReplyBackend));
    turn(&service, "off", "first question");
    assert_eq!(
        service.generate_conversation_title("off", PROJECT).unwrap(),
        None
    );
    assert_eq!(stored_title(&service, "off"), "first question");

    // Failing: the derived title stays and a later turn does not retry.
    let (_temp, settings) = fixture();
    let calls = fake_claude(&settings, "ignored", "0", 1);
    let (service, updates) = service_with_updates(settings);
    turn(&service, "failing", "first question");
    assert!(updates.recv_timeout(Duration::from_millis(1500)).is_err());
    assert_eq!(call_count(&calls), 1);
    turn(&service, "failing", "second question");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(call_count(&calls), 1);
    assert_eq!(stored_title(&service, "failing"), "first question");
}

fn titled_flow() -> Value {
    json!({
        "schema_version": "2",
        "id": "titled",
        "title": "Implement Change",
        "goal": "Implement the requested change.",
        "inputs": [
            {"key": "context.request.objective", "label": "Objective"},
            {"key": "context.request.unused", "label": "Unused"}
        ],
        "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "done", "description": "Enter the first workflow step."}}},
            "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
        }
    })
}

/// A service whose run-record writes reach the receiver as live run upserts,
/// the way the HTTP run event publisher delivers them.
fn run_service_with_updates(
    settings: SparkSettings,
) -> (WorkspaceConversationService, mpsc::Receiver<LiveEnvelope>) {
    crate::install_named_flow(&settings, "ops/titled", titled_flow());
    let (sender, receiver) = mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    let observed = settings.clone();
    let service =
        crate::conversations(settings).with_run_event_observer(Arc::new(move |run_id: &str| {
            if let Ok(Some(envelope)) =
                spark_workspace::live::run_upsert_envelope(&observed, run_id)
            {
                let _ = sender.lock().unwrap().send(envelope);
            }
        }));
    (service, receiver)
}

fn launch(service: &WorkspaceConversationService, project: &Path) -> String {
    let response = service
        .launch_workspace_run(RunLaunchRequest {
            flow_name: "ops/titled".into(),
            summary: "Launch.".into(),
            project_path: Some(project.to_string_lossy().into_owned()),
            launch_context: Some(json!({"context.request.objective": "fix the deploy pipeline"})),
            ..RunLaunchRequest::default()
        })
        .unwrap();
    response["run_id"].as_str().unwrap().to_string()
}

fn stored_run_title(settings: &SparkSettings, run_id: &str) -> Option<String> {
    attractor_runtime::RunStore::for_settings(settings)
        .read_run_bundle(run_id)
        .unwrap()
        .unwrap()
        .record
        .unwrap()
        .title
}

#[cfg(unix)]
#[test]
fn root_run_gets_a_generated_title_in_the_background() {
    let _env = claude_env();
    let (temp, settings) = fixture();
    let (calls, gate) = fake_claude_behind_gate(&settings, "\"Deploy pipeline fix.\"");
    let (service, updates) = run_service_with_updates(settings.clone());

    // Launch returns while the utility call is still held open.
    let run_id = launch(&service, temp.path());
    fs::write(&gate, "").unwrap();

    // A live client gets the title as a run-record update.
    let deadline = Instant::now() + Duration::from_secs(10);
    let run = loop {
        let envelope = updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("titled run upsert");
        // Every live record carries the launch input that names an untitled run.
        assert_eq!(
            envelope.payload["run"]["first_launch_input"],
            "fix the deploy pipeline"
        );
        if envelope.payload["run"]["title"] != Value::Null {
            break envelope.payload["run"].clone();
        }
    };
    assert_eq!(run["run_id"], run_id.as_str());
    assert_eq!(run["title"], "Deploy pipeline fix");
    assert_eq!(call_count(&calls), 1);
    let input = fs::read_to_string(calls.with_extension("input")).unwrap();
    assert!(input.contains("Implement Change"), "{input}");
    assert!(input.contains("Implement the requested change."), "{input}");
    assert!(
        input.contains("Objective: fix the deploy pipeline"),
        "{input}"
    );
    assert!(!input.contains("Unused"), "{input}");

    // A later write from a record read before the title keeps the title.
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let paths = store.find_run_root(&run_id).unwrap().unwrap();
    let mut stale = store.read_run_record(&paths).unwrap().unwrap();
    stale.title = None;
    store.write_run_record(&paths, &stale).unwrap();
    assert_eq!(
        stored_run_title(&settings, &run_id).as_deref(),
        Some("Deploy pipeline fix")
    );
}

#[cfg(unix)]
#[test]
fn runs_stay_untitled_when_utility_is_off_or_fails_and_child_runs_get_no_call() {
    let _env = claude_env();

    // Off: no utility model.
    let (temp, settings) = fixture();
    let (service, _updates) = run_service_with_updates(settings.clone());
    let run_id = launch(&service, temp.path());
    assert!(service.generate_run_title(&run_id).unwrap().is_none());
    assert_eq!(stored_run_title(&settings, &run_id), None);

    // Failing: launch succeeds, one call, nothing stored.
    let (temp, settings) = fixture();
    let calls = fake_claude(&settings, "ignored", "0", 1);
    let (service, _updates) = run_service_with_updates(settings.clone());
    let run_id = launch(&service, temp.path());
    let deadline = Instant::now() + Duration::from_secs(10);
    while call_count(&calls) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(call_count(&calls), 1);
    assert_eq!(stored_run_title(&settings, &run_id), None);

    // Child: no utility call.
    let (temp, settings) = fixture();
    let calls = fake_claude(&settings, "Child title", "0", 0);
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let mut record = attractor_core::RunRecord::new("child-run", temp.path().to_string_lossy());
    record.parent_run_id = Some("parent-run".into());
    store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            checkpoint: None,
            manifest: None,
            flow_definition_json: Some(titled_flow().to_string()),
            flow_files: None,
        })
        .unwrap();
    let (service, _updates) = run_service_with_updates(settings.clone());
    assert!(service.generate_run_title("child-run").unwrap().is_none());
    assert_eq!(call_count(&calls), 0);
    assert_eq!(stored_run_title(&settings, "child-run"), None);
}

#[test]
fn node_thinking_is_published_once_as_a_transient_envelope_for_that_run() {
    let (_temp, settings) = fixture();
    let runner = attractor_runtime::RuntimeHandlerRunner::new()
        .with_run_event_observer(std::sync::Arc::new(|_: &str| {}));
    let thinking = |active, estimated_tokens| attractor_runtime::events::NodeThinking {
        node_id: "task".to_string(),
        stage_index: 2,
        attempt: 1,
        active,
        estimated_tokens,
    };
    runner.publish_node_thinking("run-a", thinking(true, Some(10)));
    runner.publish_node_thinking("run-a", thinking(true, Some(90)));

    let envelopes = spark_workspace::live::node_thinking_envelopes(&settings, "run-a");

    // Between publisher passes only the node's latest state survives.
    assert_eq!(envelopes.len(), 1);
    let envelope = &envelopes[0];
    assert_eq!(envelope.event_type, "run.thinking");
    assert!(envelope.cursor.is_none(), "never replayed from a cursor");
    assert_eq!(envelope.payload["run_id"], "run-a");
    assert_eq!(envelope.payload["node_id"], "task");
    assert_eq!(envelope.payload["stage_index"], 2);
    assert_eq!(envelope.payload["attempt"], 1);
    assert_eq!(envelope.payload["active"], true);
    assert_eq!(envelope.payload["estimated_tokens"], 90);
    let query = |run_id: &str| {
        validate_live_query(RawLiveQuery {
            run_id: Some(run_id.to_string()),
            ..Default::default()
        })
        .unwrap()
    };
    assert!(spark_workspace::live::envelope_matches_query(
        envelope,
        &query("run-a")
    ));
    assert!(!spark_workspace::live::envelope_matches_query(
        envelope,
        &query("run-b")
    ));
    assert!(spark_workspace::live::node_thinking_envelopes(&settings, "run-a").is_empty());
}
