use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_workspace::projects::{ProjectRegistrationRequest, ProjectStateUpdate};
use spark_workspace::WorkspaceProjectService;

#[test]
fn project_service_validates_execution_profile_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config");
    fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[profiles.native-dev]
label = "Native Dev"
mode = "native"

[profiles.disabled]
label = "Disabled"
mode = "native"
enabled = false
"#
        .trim(),
    )
    .expect("profiles");
    let service = WorkspaceProjectService::new(settings);
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("project");

    let record = service
        .register_project(ProjectRegistrationRequest {
            project_path: project_dir.to_string_lossy().into_owned(),
            execution_profile_id: Some("native-dev".to_string()),
        })
        .expect("register");
    assert_eq!(record.execution_profile_id.as_deref(), Some("native-dev"));

    let missing = service
        .update_project_state(ProjectStateUpdate {
            project_path: project_dir.to_string_lossy().into_owned(),
            execution_profile_id: Some(Some("missing".to_string())),
            ..ProjectStateUpdate::default()
        })
        .expect_err("missing profile");
    assert_eq!(missing.to_string(), "Unknown execution profile: missing");

    let disabled = service
        .update_project_state(ProjectStateUpdate {
            project_path: project_dir.to_string_lossy().into_owned(),
            execution_profile_id: Some(Some("disabled".to_string())),
            ..ProjectStateUpdate::default()
        })
        .expect_err("disabled profile");
    assert_eq!(
        disabled.to_string(),
        "Execution profile is disabled: disabled"
    );
}

#[test]
fn project_service_browse_defaults_to_project_root_and_sorts_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("projects");
    fs::create_dir_all(root.join("zeta")).expect("zeta");
    fs::create_dir_all(root.join("Alpha")).expect("alpha");
    fs::create_dir_all(root.join(".hidden-dir")).expect("hidden");
    fs::write(root.join("notes.txt"), "ignored").expect("file");
    let mut settings = settings(temp.path());
    settings.project_roots = vec![root.clone()];

    let response = WorkspaceProjectService::new(settings)
        .browse_project_directories(None)
        .expect("browse");

    assert_eq!(response.current_path, root.to_string_lossy());
    assert_eq!(
        response
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec![".hidden-dir", "Alpha", "zeta"]
    );
    assert!(response.entries.iter().all(|entry| entry.is_dir));
}

#[test]
fn project_service_metadata_returns_null_git_fields_for_non_repo() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: metadata directories come back canonical (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let project_dir = root.join("non-git-project");
    fs::create_dir_all(&project_dir).expect("project");

    let metadata = WorkspaceProjectService::new(settings(&root))
        .project_metadata(&project_dir.to_string_lossy())
        .expect("metadata");

    assert_eq!(metadata.name, "non-git-project");
    assert_eq!(metadata.directory, project_dir.to_string_lossy());
    assert_eq!(metadata.branch, None);
    assert_eq!(metadata.commit, None);
}

#[test]
fn project_service_chat_models_include_safe_configured_profile_models() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config");
    fs::write(
        settings.config_dir.join("llm-profiles.toml"),
        r#"
[profiles.local]
label = "Local"
provider = "openai_compatible"
base_url = "http://127.0.0.1:1234/v1"
api_key_env = "LOCAL_KEY"
models = ["local-model"]
default_model = "local-model"
"#
        .trim(),
    )
    .expect("profiles");

    let response = WorkspaceProjectService::new(settings)
        .chat_models("/projects/my-app")
        .expect("models");

    let models = response["models"].as_array().expect("models array");
    assert!(models
        .iter()
        .any(|model| model["provider"] == "openai" && model["id"] == "gpt-5.2"));
    assert!(models
        .iter()
        .any(|model| model["provider"] == "anthropic" && model["id"] == "claude-sonnet-4-5"));
    assert!(models
        .iter()
        .any(|model| model["provider"] == "gemini" && model["id"] == "gemini-3.1-pro-preview"));
    let configured = models
        .iter()
        .find(|model| model["id"] == "local-model")
        .expect("configured model");
    assert_eq!(configured["provider"], "openai_compatible");
    assert_eq!(configured["display"], "Local / local-model");
    assert_eq!(configured["is_default"], true);
    assert!(!response.to_string().contains("127.0.0.1"));
    assert!(!response.to_string().contains("LOCAL_KEY"));
}

#[test]
fn project_service_lists_conversation_summaries_for_python_created_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_dir = temp.path().join("project-a");
    fs::create_dir_all(&project_dir).expect("project dir");
    let service = WorkspaceProjectService::new(settings.clone());
    let project = service
        .register_project(ProjectRegistrationRequest {
            project_path: project_dir.to_string_lossy().into_owned(),
            execution_profile_id: None,
        })
        .expect("register");
    let conversations_dir = settings
        .projects_dir
        .join(&project.project_id)
        .join("conversations");
    write_state(
        &conversations_dir,
        "conversation-a",
        json!({
            "schema_version": 5,
            "revision": 2,
            "conversation_id": "conversation-a",
            "conversation_handle": "amber-anchor",
            "project_path": project.project_path,
            "title": "",
            "created_at": "2026-03-07T13:00:00Z",
            "updated_at": "2026-03-07T13:01:00Z",
            "turns": [
                {
                    "id": "turn-a-1",
                    "role": "user",
                    "content": "Design thread title",
                    "timestamp": "2026-03-07T13:00:00Z",
                    "kind": "message"
                },
                {
                    "id": "turn-a-2",
                    "role": "assistant",
                    "content": "Design thread preview",
                    "timestamp": "2026-03-07T13:01:00Z",
                    "kind": "message"
                },
                {
                    "id": "turn-a-3",
                    "role": "system",
                    "content": "plan",
                    "timestamp": "2026-03-07T13:02:00Z",
                    "kind": "mode_change"
                }
            ],
            "segments": []
        }),
    );
    write_state(
        &conversations_dir,
        "conversation-b",
        json!({
            "schema_version": 5,
            "revision": 3,
            "conversation_id": "conversation-b",
            "conversation_handle": "brisk-bank",
            "project_path": project.project_path,
            "title": "Second thread",
            "created_at": "2026-03-07T13:02:00Z",
            "updated_at": "2026-03-07T13:05:00Z",
            "turns": [
                {
                    "id": "turn-b-1",
                    "role": "assistant",
                    "content": "Second thread context",
                    "timestamp": "2026-03-07T13:05:00Z",
                    "kind": "message"
                }
            ],
            "segments": []
        }),
    );
    write_state(
        &conversations_dir,
        "conversation-invalid",
        json!({
            "schema_version": 5,
            "revision": 1,
            "conversation_id": "conversation-invalid",
            "conversation_handle": "clear-cloud",
            "project_path": project.project_path,
            "title": "Invalid thread",
            "turns": []
        }),
    );

    let summaries = service
        .list_project_conversations(&project.project_path)
        .expect("conversations");

    assert_eq!(
        summaries
            .iter()
            .map(|summary| summary.conversation_id.as_str())
            .collect::<Vec<_>>(),
        vec!["conversation-b", "conversation-a"]
    );
    assert_eq!(summaries[0].conversation_handle, "brisk-bank");
    assert_eq!(
        summaries[0].last_message_preview.as_deref(),
        Some("Second thread context")
    );
    assert_eq!(summaries[1].title, "Design thread title");
    assert_eq!(
        summaries[1].last_message_preview.as_deref(),
        Some("Design thread preview")
    );
    assert!(summaries
        .iter()
        .all(|summary| summary.project_path == project.project_path));
}

fn write_state(conversations_dir: &Path, conversation_id: &str, payload: serde_json::Value) {
    let state_path = conversations_dir.join(conversation_id).join("state.json");
    fs::create_dir_all(state_path.parent().expect("state parent")).expect("state parent");
    fs::write(
        state_path,
        serde_json::to_string_pretty(&payload).expect("json"),
    )
    .expect("state");
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::<PathBuf>::new(),
    }
}

#[test]
fn codex_chat_models_map_live_metadata_and_synthesize_a_default() {
    let mapped = spark_workspace::models::codex_chat_models_from_metadata(vec![
        spark_agent_adapter::CodexModelMetadata {
            id: "gpt-5.5".to_string(),
            display: "GPT-5.5".to_string(),
            is_default: false,
            supported_reasoning_efforts: vec!["low".to_string(), "medium".to_string()],
            default_reasoning_effort: Some("medium".to_string()),
        },
        spark_agent_adapter::CodexModelMetadata {
            id: "gpt-5.5-mini".to_string(),
            display: "GPT-5.5 Mini".to_string(),
            is_default: false,
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
        },
    ]);
    assert_eq!(mapped.len(), 2);
    assert!(mapped.iter().all(|model| model.provider == "codex"));
    // No entry claimed default, so the first one leads the chooser.
    assert!(mapped[0].is_default);
    assert!(!mapped[1].is_default);
    assert_eq!(mapped[0].supported_reasoning_efforts, vec!["low", "medium"]);
    // Missing effort metadata falls back to the full ladder + medium.
    assert_eq!(
        mapped[1].supported_reasoning_efforts,
        vec!["low", "medium", "high", "xhigh", "max", "ultra"]
    );
    assert_eq!(
        mapped[1].default_reasoning_effort.as_deref(),
        Some("medium")
    );
}

#[test]
fn chat_models_report_codex_discovery_failure_without_synthesizing_models() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response = spark_workspace::models::chat_models_with_codex_result(
        &settings(temp.path()),
        Err("Codex model discovery failed: app-server exited".to_string()),
    )
    .expect("model response");

    assert_eq!(response["providers"]["codex"]["status"], "unavailable");
    assert_eq!(
        response["providers"]["codex"]["error"],
        "Codex model discovery failed: app-server exited"
    );
    assert!(response["models"]
        .as_array()
        .expect("models")
        .iter()
        .all(|model| model["provider"] != "codex"));
}

#[test]
fn chat_models_preserve_a_successful_empty_codex_model_list() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response = spark_workspace::models::chat_models_with_codex_result(
        &settings(temp.path()),
        Ok(Vec::new()),
    )
    .expect("model response");

    assert_eq!(response["providers"]["codex"]["status"], "available");
    assert_eq!(response["providers"]["codex"]["error"], Value::Null);
    assert!(response["models"]
        .as_array()
        .expect("models")
        .iter()
        .all(|model| model["provider"] != "codex"));
}
