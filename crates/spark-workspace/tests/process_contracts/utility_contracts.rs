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
use spark_storage::conversation::{ConversationMetadataPatch, ConversationMutation};
use spark_storage::settings::{load_core_settings, read_settings_document};
use spark_storage::ConversationRepository;
use spark_workspace::live::{envelope_matches_query, validate_live_query};
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

/// A service whose live updates (generated titles) arrive on the receiver.
fn service_with_updates(
    settings: SparkSettings,
) -> (WorkspaceConversationService, mpsc::Receiver<LiveEnvelope>) {
    let (sender, receiver) = mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    let service =
        WorkspaceConversationService::new_with_agent_turn_backend(settings, Arc::new(ReplyBackend))
            .with_live_publisher(Arc::new(move |envelope| {
                let _ = sender.lock().unwrap().send(envelope);
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

/// The thread-list and conversation updates a generated title publishes.
fn title_updates(updates: &mpsc::Receiver<LiveEnvelope>, wait: Duration) -> Option<(Value, Value)> {
    let summary = updates.recv_timeout(wait).ok()?;
    let snapshot = updates.recv_timeout(Duration::from_secs(1)).ok()?;
    assert_eq!(summary.event_type, "conversation.summary_upsert");
    assert_eq!(summary.resource.kind, "conversation_summary");
    // Only clients that ask for the thread-list feed receive it.
    let query = |include: Option<&str>| {
        validate_live_query(RawLiveQuery {
            include_conversations: include.map(str::to_string),
            ..Default::default()
        })
        .unwrap()
    };
    assert!(envelope_matches_query(&summary, &query(Some("true"))));
    assert!(!envelope_matches_query(&summary, &query(None)));
    assert_eq!(snapshot.event_type, "conversation.snapshot");
    Some((
        summary.payload["conversation"].clone(),
        snapshot.payload["state"].clone(),
    ))
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
    let calls = fake_claude(&settings, "\"Deploy pipeline repair.\"", "1", 0);
    let (service, updates) = service_with_updates(settings);

    let started = Instant::now();
    let snapshot = turn(&service, "titled", "please fix the deploy pipeline");
    // The turn does not wait for the one-second utility call.
    assert!(started.elapsed() < Duration::from_millis(900));
    assert_eq!(snapshot["title"], "please fix the deploy pipeline");

    // Every client's thread list and the thread's own view get the title.
    let (summary, state) = title_updates(&updates, Duration::from_secs(10)).unwrap();
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
        .run_turn(prepared.agent_turn_request.clone())
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

    let (summary, _) = title_updates(&updates, Duration::from_secs(10)).unwrap();
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
    ConversationRepository::new(settings.data_dir.clone())
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
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(ReplyBackend),
    );
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

const TITLED_FLOW: &str = r#"schema_version: "1"
id: titled
title: Implement Change
goal: Implement the requested change.
inputs:
- {key: context.request.objective, label: Objective}
- {key: context.request.unused, label: Unused}
nodes:
  start: {kind: start}
  done: {kind: exit}
edges:
- {from: start, to: done}
"#;

/// A service whose run-record writes reach the receiver as live run upserts,
/// the way the HTTP run event publisher delivers them.
fn run_service_with_updates(
    settings: SparkSettings,
) -> (WorkspaceConversationService, mpsc::Receiver<LiveEnvelope>) {
    let flow = settings.flows_dir.join("ops/titled.yaml");
    fs::create_dir_all(flow.parent().unwrap()).unwrap();
    fs::write(flow, TITLED_FLOW).unwrap();
    let (sender, receiver) = mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    let observed = settings.clone();
    let service = WorkspaceConversationService::new(settings).with_run_event_observer(Arc::new(
        move |run_id: &str| {
            if let Ok(Some(envelope)) =
                spark_workspace::live::run_upsert_envelope(&observed, run_id)
            {
                let _ = sender.lock().unwrap().send(envelope);
            }
        },
    ));
    (service, receiver)
}

fn launch(service: &WorkspaceConversationService, project: &Path) -> String {
    let response = service
        .launch_workspace_run(RunLaunchRequest {
            flow_name: "ops/titled.yaml".into(),
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
    let calls = fake_claude(&settings, "\"Deploy pipeline fix.\"", "1", 0);
    let (service, updates) = run_service_with_updates(settings.clone());

    let started = Instant::now();
    let run_id = launch(&service, temp.path());
    // Launch does not wait for the one-second utility call.
    assert!(started.elapsed() < Duration::from_millis(900));

    // A live client gets the title as a run-record update.
    let deadline = Instant::now() + Duration::from_secs(10);
    let run = loop {
        let envelope = updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("titled run upsert");
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
            flow_source: Some(TITLED_FLOW.into()),
            flow_definition_json: None,
        })
        .unwrap();
    let (service, _updates) = run_service_with_updates(settings.clone());
    assert!(service.generate_run_title("child-run").unwrap().is_none());
    assert_eq!(call_count(&calls), 0);
    assert_eq!(stored_run_title(&settings, "child-run"), None);
}
