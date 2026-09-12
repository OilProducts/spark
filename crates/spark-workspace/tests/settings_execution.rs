use std::fs;

use serde_json::{json, Value};
use spark_common::settings::{resolve_settings_with_env, SettingsOverrides, SparkSettings};
use spark_storage::settings::read_settings_document;
use spark_storage::ProjectRegistry;
use spark_workspace::conversations::{
    ConversationSettingsUpdate, ConversationTurnRequest, WorkspaceConversationService,
};
use spark_workspace::settings::{
    project_model_settings_view, update_workspace_settings, workspace_settings,
};

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
    (temp, settings)
}

fn group(provider: &str, model: Option<&str>) -> Value {
    json!({"provider": provider, "model": model})
}

fn save_workspace(settings: &SparkSettings, value: Value) {
    let revision = read_settings_document(&settings.config_dir.join("spark.toml"))
        .unwrap()
        .revision;
    update_workspace_settings(
        settings,
        serde_json::from_value(
            json!({"section": "models", "expected_revision": revision, "value": value}),
        )
        .unwrap(),
    )
    .unwrap();
}

fn save_project(settings: &SparkSettings, value: Value) {
    let view = project_model_settings_view(settings, "/projects/inheritance").unwrap();
    update_workspace_settings(settings, serde_json::from_value(json!({"section": "project_models", "expected_revision": view["models"]["revision"], "value": {"project_path": "/projects/inheritance", "model_settings": value}})).unwrap()).unwrap();
}

fn start(
    service: &WorkspaceConversationService,
    id: &str,
) -> spark_workspace::conversations::PreparedConversationTurn {
    service
        .start_turn(
            id,
            ConversationTurnRequest {
                project_path: "/projects/inheritance".into(),
                message: "hello".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .0
}

#[test]
fn workspace_and_project_groups_resolve_at_the_next_message_and_clearing_restores_inheritance() {
    let (_temp, settings) = fixture();
    ProjectRegistry::new(&settings.data_dir)
        .register_project("/projects/inheritance")
        .unwrap();
    save_workspace(&settings, group("codex", Some("workspace-one")));
    let service = WorkspaceConversationService::new(settings.clone());
    let first = start(&service, "chat");
    assert_eq!(first.model.as_deref(), Some("workspace-one"));
    save_workspace(&settings, group("codex", Some("workspace-two")));
    assert_eq!(first.model.as_deref(), Some("workspace-one"));
    service
        .ingest_agent_turn_output(
            "chat",
            "/projects/inheritance",
            &first.assistant_turn_id,
            "chat",
            spark_agent_adapter::AgentTurnOutput {
                final_assistant_text: Some("done".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let second = start(&service, "chat");
    assert_eq!(second.model.as_deref(), Some("workspace-two"));
    let stored = service
        .get_snapshot("chat", Some("/projects/inheritance"))
        .unwrap();
    assert!(stored["model_settings"].is_null());
    assert_eq!(
        stored["turns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|turn| turn["id"] == first.assistant_turn_id)
            .unwrap()["execution_settings"]["model_settings"]["model"],
        "workspace-one"
    );

    save_project(&settings, group("claude-code", None));
    let project = start(&service, "project-chat");
    assert_eq!(project.provider, "claude-code");
    assert_eq!(
        project.model, None,
        "omitted fields do not leak from workspace"
    );
    let initial = service
        .get_snapshot("override-chat", Some("/projects/inheritance"))
        .unwrap();
    let saved = service.update_conversation_settings("override-chat", serde_json::from_value(json!({"project_path": "/projects/inheritance", "expected_revision": initial["settings"]["models"]["revision"], "model_settings": group("codex", Some("conversation"))})).unwrap()).unwrap();
    assert_eq!(saved["settings"]["models"]["source"], "conversation");
    let unchanged = service
        .update_conversation_settings(
            "override-chat",
            ConversationSettingsUpdate {
                project_path: "/projects/inheritance".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(unchanged["model_settings"], saved["model_settings"]);
    let cleared = service.update_conversation_settings("override-chat", serde_json::from_value(json!({"project_path": "/projects/inheritance", "expected_revision": saved["settings"]["models"]["revision"], "model_settings": null})).unwrap()).unwrap();
    assert_eq!(cleared["settings"]["models"]["source"], "project");
    save_project(&settings, Value::Null);
    assert_eq!(
        service
            .get_snapshot("override-chat", Some("/projects/inheritance"))
            .unwrap()["settings"]["models"]["source"],
        "workspace"
    );
}

#[test]
fn conversation_settings_reject_stale_or_missing_revisions_and_invalid_profiles() {
    let (_temp, settings) = fixture();
    let service = WorkspaceConversationService::new(settings.clone());
    let update = |revision: Value, value: Value| {
        serde_json::from_value(json!({"project_path": "/projects/inheritance", "expected_revision": revision, "model_settings": value})).unwrap()
    };
    assert!(service
        .update_conversation_settings("chat", update(Value::Null, group("codex", None)))
        .is_err());
    service
        .update_conversation_settings("chat", update(json!("0"), group("codex", Some("one"))))
        .unwrap();
    assert!(matches!(
        service
            .update_conversation_settings("chat", update(json!("0"), group("codex", Some("two")))),
        Err(spark_workspace::WorkspaceError::Conflict(_))
    ));
    let revision = service
        .get_snapshot("chat", Some("/projects/inheritance"))
        .unwrap()["settings"]["models"]["revision"]
        .clone();
    assert!(service
        .update_conversation_settings("chat", update(revision, json!({"llm_profile": "missing"})))
        .is_err());
    assert_eq!(
        service
            .get_snapshot("chat", Some("/projects/inheritance"))
            .unwrap()["model"],
        "one"
    );
}

#[test]
fn selected_profile_contents_and_default_model_are_captured_and_secret_values_are_absent() {
    let (_temp, settings) = fixture();
    fs::create_dir_all(&settings.config_dir).unwrap();
    let path = settings.config_dir.join("llm-profiles.toml");
    fs::write(&path, "[profiles.team]\nprovider='openai_compatible'\nbase_url='http://localhost:9999/v1'\nmodels=['first','second']\ndefault_model='first'\napi_key_env='SPARK_TEST_PROFILE_SECRET'\n").unwrap();
    save_workspace(&settings, json!({"llm_profile": "team"}));
    let service = WorkspaceConversationService::new(settings.clone());
    let first = start(&service, "first");
    fs::write(&path, "[profiles.team]\nprovider='openai_compatible'\nbase_url='http://localhost:9998/v1'\nmodels=['second']\ndefault_model='second'\n").unwrap();
    let second = start(&service, "second");
    assert_eq!(first.model.as_deref(), Some("first"));
    assert_eq!(second.model.as_deref(), Some("second"));
    assert_eq!(
        first.agent_turn_request.metadata["spark.execution.settings"]["llm_profile"]["base_url"],
        "http://localhost:9999/v1"
    );
    assert_eq!(
        first.agent_turn_request.metadata["spark.execution.settings"]["llm_profile"]["api_key_env"],
        "SPARK_TEST_PROFILE_SECRET"
    );
    assert!(
        first.agent_turn_request.metadata["spark.execution.settings"]["llm_profile"]
            .get("api_key")
            .is_none()
    );
    fs::write(&path, "secret = SECRET_DO_NOT_EXPOSE\n").unwrap();
    assert!(!workspace_settings(&settings)
        .unwrap_err()
        .to_string()
        .contains("SECRET_DO_NOT_EXPOSE"));
}

#[test]
fn project_operational_updates_preserve_model_group_and_extensions() {
    let (_temp, settings) = fixture();
    let registry = ProjectRegistry::new(&settings.data_dir);
    let paths = registry
        .ensure_project_paths("/projects/inheritance")
        .unwrap();
    save_project(&settings, group("codex", Some("project")));
    let text = fs::read_to_string(&paths.project_file).unwrap();
    fs::write(
        &paths.project_file,
        format!("{text}\n[extension]\ncustom = true\n"),
    )
    .unwrap();
    registry
        .update_project_record(
            "/projects/inheritance",
            spark_storage::ProjectRecordUpdate {
                is_favorite: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    registry.register_project("/projects/inheritance").unwrap();
    let doc = read_settings_document(&paths.project_file).unwrap();
    assert_eq!(
        doc.values["model_settings"]["model"].as_str(),
        Some("project")
    );
    assert_eq!(doc.values["extension"]["custom"].as_bool(), Some(true));
}

#[test]
fn active_turn_uses_captured_profile_after_its_document_becomes_invalid() {
    active_turn_uses_captured_connection(true);
}

#[test]
fn active_turn_uses_captured_provider_after_core_configuration_becomes_invalid() {
    active_turn_uses_captured_connection(false);
}

fn active_turn_uses_captured_connection(profile: bool) {
    use std::io::{BufRead, Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let (_temp, settings) = fixture();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    fs::create_dir_all(&settings.config_dir).unwrap();
    let path = settings.config_dir.join(if profile {
        "llm-profiles.toml"
    } else {
        "spark.toml"
    });
    if profile {
        fs::write(&path, format!("[profiles.captured]\nprovider='openai_compatible'\nbase_url='http://{address}/v1'\nmodels=['captured-model']\ndefault_model='captured-model'\n")).unwrap();
        save_workspace(&settings, json!({"llm_profile": "captured"}));
    } else {
        fs::write(&path, format!("[models]\nprovider='openai_compatible'\nmodel='captured-model'\n[providers.openai_compatible]\nbase_url='http://{address}/v1'\n")).unwrap();
    }
    let service = WorkspaceConversationService::new(settings.clone());
    let (prepared, started) = service
        .start_turn(
            "captured-chat",
            ConversationTurnRequest {
                project_path: "/projects/inheritance".into(),
                message: "Answer briefly".into(),
                ..Default::default()
            },
        )
        .unwrap();
    // If execution rereads the mutable document, it fails before reaching the endpoint.
    fs::write(&path, "invalid = SECRET_DO_NOT_EXPOSE\n").unwrap();
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
                Err(error) => panic!("No captured-profile request: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut request = vec![0; length];
        reader.read_exact(&mut request).unwrap();
        let response = json!({"id": "captured", "object": "chat.completion", "model": "captured-model", "choices": [{"index": 0, "message": {"role": "assistant", "content": "captured endpoint"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        serde_json::from_slice::<Value>(&request).unwrap()
    });
    let completed = service
        .complete_started_turn_with_progress_payloads(prepared.clone(), started, |_| {})
        .unwrap();
    let request = server.join().unwrap();
    assert_eq!(request["model"], "captured-model");
    let turn = completed["turns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .unwrap();
    assert_eq!(turn["status"], "complete", "{turn}");
    assert_eq!(turn["content"], "captured endpoint");
}

#[test]
fn saved_and_file_edited_provider_and_session_settings_apply_only_to_new_messages() {
    let (_temp, settings) = fixture();
    let path = settings.config_dir.join("spark.toml");
    let save = |section: &str, value: Value| {
        let revision = read_settings_document(&path).unwrap().revision;
        update_workspace_settings(
            &settings,
            serde_json::from_value(
                json!({"section": section, "expected_revision": revision, "value": value}),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let view = save(
        "providers",
        json!({"openai_compatible": {"base_url": "http://localhost:10001", "api_key_env": "CR_PROVIDER_REFERENCE"}}),
    );
    assert!(view["providers"]["credential_status"]["openai_compatible"].is_boolean());
    save(
        "agents",
        json!({"max_turns": 7, "default_command_timeout_ms": 321, "max_command_timeout_ms": 999}),
    );
    let service = WorkspaceConversationService::new(settings.clone());
    let first = start(&service, "capture-first");
    let snapshot = first.agent_turn_request.metadata["spark.execution.settings"].clone();
    save("agents", json!({"max_turns": 11}));
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("localhost:10001", "localhost:10002");
    fs::write(&path, text).unwrap();
    let second = start(&service, "capture-second");
    let next = &second.agent_turn_request.metadata["spark.execution.settings"];
    assert_eq!(snapshot["configuration"]["agents"]["max_turns"], 7);
    assert_eq!(next["configuration"]["agents"]["max_turns"], 11);
    assert_eq!(
        snapshot["configuration"]["providers"]["openai_compatible"]["base_url"],
        "http://localhost:10001"
    );
    assert_eq!(
        next["configuration"]["providers"]["openai_compatible"]["base_url"],
        "http://localhost:10002"
    );
    let historical = service
        .get_snapshot("capture-first", Some("/projects/inheritance"))
        .unwrap();
    assert!(historical.to_string().contains("localhost:10001"));
    assert!(!historical.to_string().contains("localhost:10002"));
    assert_eq!(
        first.agent_turn_request.metadata["spark.execution.settings"],
        snapshot
    );
    let before = fs::read(&path).unwrap();
    let revision = read_settings_document(&path).unwrap().revision;
    assert!(update_workspace_settings(&settings, serde_json::from_value(json!({"section": "agents", "expected_revision": revision, "value": {"default_command_timeout_ms": 0}})).unwrap()).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}
