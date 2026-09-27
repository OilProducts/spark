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
    let service = WorkspaceProjectService::new(settings.clone());
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("project");

    let record = service
        .register_project(ProjectRegistrationRequest {
            project_path: project_dir.to_string_lossy().into_owned(),
            execution_profile_id: Some("native-dev".to_string()),
        })
        .expect("register");
    assert_eq!(record.execution_profile_id.as_deref(), Some("native-dev"));

    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    let paths = registry.ensure_project_paths(&record.project_path).unwrap();
    let text = fs::read_to_string(&paths.project_file).unwrap().replace(
        &format!("last_opened_at = \"{}\"", record.last_opened_at),
        "last_opened_at = \"2001-01-01T00:00:00Z\"",
    );
    fs::write(&paths.project_file, text).unwrap();
    let updated = service
        .update_project_state(ProjectStateUpdate {
            project_path: record.project_path.clone(),
            last_accessed_at: Some(Some("2002-01-01T00:00:00Z".into())),
            is_favorite: Some(true),
            ..ProjectStateUpdate::default()
        })
        .unwrap();
    assert_eq!(updated.last_opened_at, "2001-01-01T00:00:00Z");
    let reopened = service
        .register_project(ProjectRegistrationRequest {
            project_path: record.project_path.clone(),
            execution_profile_id: None,
        })
        .unwrap();
    assert!(reopened.last_opened_at > updated.last_opened_at);
    assert_eq!(reopened.created_at, record.created_at);
    assert_eq!(reopened.execution_profile_id, record.execution_profile_id);
    assert_eq!(reopened.last_accessed_at, updated.last_accessed_at);
    assert!(reopened.is_favorite);

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

/// Keeps chat-model tests hermetic: claude-code discovery fails fast and
/// falls back to the static aliases instead of probing an installed CLI.
fn pin_claude_code_bin_to_missing() {
    std::env::set_var(
        "SPARK_CLAUDE_CODE_BIN",
        "/nonexistent/claude-code-for-tests",
    );
}

#[test]
fn project_service_chat_models_include_safe_configured_profile_models() {
    pin_claude_code_bin_to_missing();
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

    let service = WorkspaceProjectService::new(settings);
    assert!(service.chat_models(Some("")).is_err());
    let workspace = service.chat_models(None).expect("workspace models");
    let response = service
        .chat_models(Some("/projects/my-app"))
        .expect("models");

    assert_eq!(
        workspace["provider_reasoning_efforts"],
        response["provider_reasoning_efforts"]
    );
    assert_eq!(workspace["models"], response["models"]);
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
    // Discovery is pinned to a missing CLI, so claude-code falls back to the
    // static aliases rather than offering a blank provider.
    for alias in ["opus", "sonnet", "haiku"] {
        assert!(models
            .iter()
            .any(|model| model["provider"] == "claude-code" && model["id"] == alias));
    }
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
#[ignore = "legacy state.json fixtures are unsupported after the conversation hard cutover"]
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
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
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
fn claude_code_chat_models_map_catalog_metadata_with_effort_support() {
    let mapped = spark_workspace::models::claude_code_chat_models_from_metadata(vec![
        spark_agent_adapter::ClaudeCodeModelMetadata {
            id: String::new(),
            display: "Default (recommended)".to_string(),
            supported_efforts: vec![],
        },
        spark_agent_adapter::ClaudeCodeModelMetadata {
            id: "claude-fable-5[1m]".to_string(),
            display: "Fable".to_string(),
            supported_efforts: vec!["low".to_string(), "max".to_string()],
        },
    ]);
    assert_eq!(mapped.len(), 2);
    assert!(mapped.iter().all(|model| model.provider == "claude-code"));
    // The blank id is the CLI-default pseudo-entry and leads the chooser.
    assert!(mapped[0].is_default);
    assert_eq!(mapped[0].display, "Default (recommended)");
    assert!(!mapped[1].is_default);
    assert_eq!(mapped[1].id, "claude-fable-5[1m]");
    // Catalog effort levels pass through; the adapter applies them as --effort.
    assert!(mapped[0].supported_reasoning_efforts.is_empty());
    assert_eq!(mapped[1].supported_reasoning_efforts, ["low", "max"]);
    assert!(mapped
        .iter()
        .all(|model| model.default_reasoning_effort.is_none()));
}

#[test]
fn chat_models_report_codex_discovery_failure_without_synthesizing_models() {
    pin_claude_code_bin_to_missing();
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
    pin_claude_code_bin_to_missing();
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

#[test]
fn missing_project_profile_lookup_keeps_native_fallback_without_registration() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    let default_profile = registry
        .read_project_record("/projects/unregistered-flow")
        .unwrap()
        .and_then(|record| record.execution_profile_id);
    assert_eq!(default_profile, None);
    fs::create_dir_all(&settings.project_root).unwrap();
    let response = attractor_api::AttractorApiService::new(settings.clone()).start_pipeline(
        attractor_api::PipelineStartRequest {
            flow_content: Some(super::review_artifact_contracts::simple_flow().into()),
            working_directory: settings.project_root.to_string_lossy().into_owned(),
            project_default_execution_profile_id: default_profile,
            wait: Some(true),
            ..attractor_api::PipelineStartRequest::default()
        },
    );
    assert_eq!(response.body["status"], "started", "{:?}", response.body);
    assert_eq!(response.body["execution_profile_id"], "native");
    assert!(!registry.projects_root().exists());
}

#[test]
fn chat_models_expose_catalog_defaults_fallbacks_and_profile_specific_levels() {
    pin_claude_code_bin_to_missing();
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    fs::write(
        settings.config_dir.join("llm-profiles.toml"),
        r#"
[profiles.local]
provider = "openai_compatible"
base_url = "http://localhost:4000/v1"
models = ["shared"]
reasoning_efforts = ["minimal", "high"]
[profiles.other]
provider = "openai_compatible"
base_url = "http://localhost:4001/v1"
models = ["shared"]
[profiles.router]
provider = "openrouter"
base_url = "https://openrouter.ai/api/v1"
models = ["remote"]
reasoning_efforts = ["high"]
[profiles.proxy]
provider = "litellm"
base_url = "http://localhost:4002/v1"
models = ["remote"]
reasoning_efforts = ["low"]
"#,
    )
    .unwrap();
    let response =
        spark_workspace::models::chat_models_with_codex_result(&settings, Ok(vec![])).unwrap();
    let models = response["models"].as_array().unwrap();
    for catalog in unified_llm_adapter::list_models(None)
        .into_iter()
        .filter(|model| model.provider != "anthropic")
    {
        let model = models
            .iter()
            .find(|model| model["id"] == catalog.id && model["provider"] == catalog.provider)
            .unwrap();
        assert_eq!(
            model["supported_reasoning_efforts"],
            json!(catalog.reasoning_efforts)
        );
        assert_eq!(
            model["default_reasoning_effort"],
            json!(catalog.default_reasoning_effort)
        );
    }
    for (profile, levels) in [
        ("local", vec!["minimal", "high"]),
        ("other", vec![]),
        ("router", vec!["high"]),
        ("proxy", vec!["low"]),
    ] {
        let model = models
            .iter()
            .find(|model| model["llm_profile"] == profile)
            .unwrap();
        assert_eq!(model["supported_reasoning_efforts"], json!(levels));
    }
    assert_eq!(
        response["provider_reasoning_efforts"]["gemini"],
        json!(["minimal", "low", "medium", "high"])
    );
    assert!(response["provider_reasoning_efforts"]
        .get("openai_compatible")
        .is_none());
}

#[test]
fn chat_models_discovery_uses_resolved_configuration_and_invalidates_cache() {
    use std::io::{BufRead, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    const CHILD: &str = "SPARK_DISCOVERY_CONFIGURATION_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "project_registry_contracts::chat_models_discovery_uses_resolved_configuration_and_invalidates_cache",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, "1")
            .env("SPARK_TEST_ANTHROPIC_KEY", "configured-secret")
            .env("SPARK_TEST_OTHER_KEY", "changed-secret")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    assert!(std::env::var_os("ANTHROPIC_API_KEY").is_none());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    for (id, key_env, secret, efforts) in [
        (
            "live-first",
            "SPARK_TEST_ANTHROPIC_KEY",
            "configured-secret",
            vec!["low", "high"],
        ),
        (
            "live-changed",
            "SPARK_TEST_OTHER_KEY",
            "changed-secret",
            vec!["medium"],
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        fs::write(
            settings.config_dir.join("spark.toml"),
            format!(
                "[providers.anthropic]\nbase_url='http://{address}/v1'\napi_key_env='{key_env}'\n"
            ),
        )
        .unwrap();
        let capabilities: serde_json::Map<String, Value> = efforts
            .iter()
            .map(|level| (level.to_string(), json!({"supported": true})))
            .collect();
        let response = json!({"data": [{"id": id, "display_name": "Live model", "capabilities": {"effort": capabilities}}], "has_more": false}).to_string();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(stream) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("No discovery request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            assert!(headers.starts_with("GET /v1/models?limit=1000 HTTP/1.1\r\n"));
            assert!(headers
                .to_ascii_lowercase()
                .contains(&format!("x-api-key: {secret}\r\n")));
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        });
        let response =
            spark_workspace::models::chat_models_with_codex_result(&settings, Ok(vec![])).unwrap();
        server.join().unwrap();
        let models: Vec<_> = response["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|model| model["provider"] == "anthropic")
            .collect();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["id"], id);
        assert_eq!(models[0]["display"], "Live model");
        assert_eq!(models[0]["supported_reasoning_efforts"], json!(efforts));
        assert!(!response.to_string().contains(secret));
        // The endpoint is now closed: repeated discovery must use the cached live metadata.
        assert_eq!(
            spark_workspace::models::chat_models_with_codex_result(&settings, Ok(vec![])).unwrap(),
            response
        );
    }
}
