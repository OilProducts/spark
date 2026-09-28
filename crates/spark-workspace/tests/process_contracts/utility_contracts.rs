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
use spark_workspace::settings::{update_workspace_settings, workspace_settings};
use spark_workspace::utility::utility_complete;
use spark_workspace::{ConversationTurnRequest, WorkspaceConversationService, WorkspaceError};

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
            "#!/bin/sh\ncat > /dev/null\necho call >> '{}'\nsleep {delay}\necho '{reply}'\nexit {status}\n",
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

fn turn(
    service: &WorkspaceConversationService,
    id: &str,
    message: &str,
) -> (Value, mpsc::Receiver<Value>) {
    let (sender, receiver) = mpsc::channel();
    let snapshot = service
        .execute_turn_with_progress_payloads(
            id,
            ConversationTurnRequest {
                project_path: PROJECT.into(),
                message: message.into(),
                provider: Some("codex".into()),
                ..Default::default()
            },
            move |payload| {
                if payload["type"] == "conversation_snapshot" {
                    let _ = sender.send(payload);
                }
            },
        )
        .unwrap();
    (snapshot, receiver)
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
    let service =
        WorkspaceConversationService::new_with_agent_turn_backend(settings, Arc::new(ReplyBackend));

    let started = Instant::now();
    let (snapshot, updates) = turn(&service, "titled", "please fix the deploy pipeline");
    // The turn does not wait for the one-second utility call.
    assert!(started.elapsed() < Duration::from_millis(900));
    assert_eq!(snapshot["title"], "please fix the deploy pipeline");

    let update = updates.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(update["state"]["title"], "Deploy pipeline repair");
    assert_eq!(update["revision"], update["state"]["revision"]);
    assert_eq!(stored_title(&service, "titled"), "Deploy pipeline repair");
    assert_eq!(call_count(&calls), 1);

    let (snapshot, updates) = turn(&service, "titled", "and the staging one");
    assert_eq!(snapshot["title"], "Deploy pipeline repair");
    assert!(updates.recv_timeout(Duration::from_millis(1500)).is_err());
    assert_eq!(call_count(&calls), 1);
    assert_eq!(stored_title(&service, "titled"), "Deploy pipeline repair");
}

#[cfg(unix)]
#[test]
fn stored_titles_stay_and_derived_titles_remain_when_off_or_failing() {
    let _env = claude_env();
    let (_temp, settings) = fixture();
    let calls = fake_claude(&settings, "Generated", "0", 0);
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(ReplyBackend),
    );
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
    let (_, updates) = turn(&service, "named", "first question");
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
    let service =
        WorkspaceConversationService::new_with_agent_turn_backend(settings, Arc::new(ReplyBackend));
    let (_, updates) = turn(&service, "failing", "first question");
    assert!(updates.recv_timeout(Duration::from_millis(1500)).is_err());
    assert_eq!(call_count(&calls), 1);
    turn(&service, "failing", "second question");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(call_count(&calls), 1);
    assert_eq!(stored_title(&service, "failing"), "first question");
}
