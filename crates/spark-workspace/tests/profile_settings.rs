use serde_json::{json, Value};
use spark_common::settings::{resolve_settings_with_env, SettingsOverrides, SparkSettings};
use spark_workspace::settings::{update_workspace_settings, workspace_settings};

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
    std::fs::create_dir_all(&settings.flows_dir).unwrap();
    (temp, settings)
}
fn llm(id: &str) -> Value {
    json!({"id": id, "provider": "openai_compatible", "base_url": "http://localhost:9999/v1", "models": ["model"], "default_model": "model", "api_key_env": "SPARK_PROFILE_TEST_MISSING"})
}
fn save(
    settings: &SparkSettings,
    section: &str,
    revision: &str,
    value: Value,
) -> spark_workspace::WorkspaceResult<Value> {
    update_workspace_settings(
        settings,
        serde_json::from_value(
            json!({"section": section, "expected_revision": revision, "value": value}),
        )
        .unwrap(),
    )
}
#[test]
fn llm_crud_is_revision_checked_preserves_extensions_and_blocks_referenced_deletion() {
    let (_temp, settings) = fixture();
    let first = save(&settings, "llm_profiles", "absent", json!([llm("team")])).unwrap();
    let rev = first["llm_profiles"]["revision"].as_str().unwrap();
    assert_eq!(
        first["llm_profiles"]["credential_status"]["team"],
        "missing"
    );
    assert!(matches!(
        save(&settings, "llm_profiles", "absent", json!([])),
        Err(spark_workspace::WorkspaceError::Conflict(_))
    ));
    let path = settings.config_dir.join("llm-profiles.toml");
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(!source.contains("id ="));
    save(
        &settings,
        "models",
        "absent",
        json!({"llm_profile": "team"}),
    )
    .unwrap();
    let error = save(&settings, "llm_profiles", rev, json!([])).unwrap_err();
    assert!(
        error.to_string().contains("spark.toml.models.llm_profile"),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    let models_rev = workspace_settings(&settings).unwrap()["models"]["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    save(
        &settings,
        "models",
        &models_rev,
        json!({"provider":"codex"}),
    )
    .unwrap();
    std::fs::write(&path, format!("{source}\n[extension]\nretained = true\n")).unwrap();
    let revision = spark_storage::settings::read_settings_document(&path)
        .unwrap()
        .revision;
    save(
        &settings,
        "llm_profiles",
        &revision,
        json!([llm("replacement")]),
    )
    .unwrap();
    let document = spark_storage::settings::read_settings_document(&path).unwrap();
    assert_eq!(
        document.values["extension"]["retained"].as_bool(),
        Some(true)
    );
    save(&settings, "llm_profiles", &document.revision, json!([])).unwrap();
}
#[test]
fn execution_profiles_validate_mounts_metadata_defaults_and_preserve_the_document_on_failure() {
    let (_temp, settings) = fixture();
    let profile = json!({"id":"container", "label":"Container", "mode":"local_container", "enabled":true, "image":"worker:latest", "capabilities":["shell"], "metadata":{"container.mounts":["/host:/worker:ro"], "custom":true}});
    let first = save(
        &settings,
        "execution_profiles",
        "absent",
        json!({"profiles":[profile.clone()], "default_execution_profile_id":"container"}),
    )
    .unwrap();
    let rev = first["execution_profiles"]["revision"].as_str().unwrap();
    let path = settings.config_dir.join("execution-profiles.toml");
    let source = std::fs::read(&path).unwrap();
    for mounts in [
        json!("bad"),
        json!(["bad"]),
        json!(["/host::ro"]),
        json!([42]),
    ] {
        let mut invalid = profile.clone();
        invalid["metadata"]["container.mounts"] = mounts;
        assert!(save(
            &settings,
            "execution_profiles",
            rev,
            json!({"profiles":[invalid], "default_execution_profile_id":"container"})
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), source);
    }
    assert!(save(
        &settings,
        "execution_profiles",
        rev,
        json!({"profiles":[profile], "default_execution_profile_id":"missing"})
    )
    .is_err());
    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    registry
        .register_project_with_execution_profile("/project", Some("container"))
        .unwrap();
    let error = save(
        &settings,
        "execution_profiles",
        rev,
        json!({"profiles":[], "default_execution_profile_id":null}),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("project.toml.execution_profile_id"),
        "{error}"
    );
}
#[test]
fn credential_values_in_endpoint_and_reference_fields_are_rejected_without_reflection() {
    let (_temp, settings) = fixture();
    for (field, value) in [
        ("base_url", "https://user:SECRET_VALUE@example.com/v1"),
        ("base_url", "https://example.com/v1?key=SECRET_VALUE"),
        ("api_key_env", "sk-SECRET_VALUE"),
    ] {
        let mut profile = llm("team");
        profile[field] = json!(value);
        let error = save(&settings, "llm_profiles", "absent", json!([profile])).unwrap_err();
        assert!(!error.to_string().contains("SECRET_VALUE"));
        assert!(!settings.config_dir.join("llm-profiles.toml").exists());
    }
}
#[test]
fn conversation_reference_blocks_deletion_but_historical_turn_capture_does_not() {
    let (_temp, settings) = fixture();
    let first = save(&settings, "llm_profiles", "absent", json!([llm("team")])).unwrap();
    let revision = first["llm_profiles"]["revision"].as_str().unwrap();
    let root = settings.projects_dir.join("orphan/conversations/chat");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("conversation.json");
    std::fs::write(&path, json!({"model_settings":{"llm_profile":" team "}, "turns":[{"execution_settings":{"llm_profile":"team"}}]}).to_string()).unwrap();
    assert!(save(&settings, "llm_profiles", revision, json!([]))
        .unwrap_err()
        .to_string()
        .contains("conversation.json.model_settings.llm_profile"));
    std::fs::write(
        &path,
        json!({"model_settings":null, "turns":[{"execution_settings":{"llm_profile":"team"}}]})
            .to_string(),
    )
    .unwrap();
    save(&settings, "llm_profiles", revision, json!([])).unwrap();
}

#[test]
fn simultaneous_profile_writers_cannot_overwrite_each_other() {
    let (_temp, settings) = fixture();
    let first = save(
        &settings,
        "llm_profiles",
        "absent",
        json!([llm("original")]),
    )
    .unwrap();
    let revision = first["llm_profiles"]["revision"].as_str().unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let writers: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|id| {
                let settings = &settings;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    save(settings, "llm_profiles", revision, json!([llm(id)]))
                })
            })
            .collect();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(spark_workspace::WorkspaceError::Conflict(_))))
            .count(),
        1
    );
}

#[test]
fn deletion_cannot_pass_a_reference_waiting_at_the_real_document_boundary() {
    use fs2::FileExt;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let (_temp, settings) = fixture();
    let first = save(&settings, "llm_profiles", "absent", json!([llm("team")])).unwrap();
    let profile_revision = first["llm_profiles"]["revision"].as_str().unwrap();
    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    registry.register_project("/project").unwrap();
    let path = registry.project_paths("/project").unwrap().project_file;
    let revision = spark_storage::settings::read_settings_document(&path)
        .unwrap()
        .revision;
    let revision =
        spark_storage::settings::update_settings_section(&path, &revision, "unused", None, |_| {
            Ok(())
        })
        .unwrap()
        .revision;
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        // An unrelated writer holds the real project lock. Its no-op patch leaves
        // the revision unchanged, so the waiting model update remains valid.
        let blocker_path = &path;
        let blocker_revision = &revision;
        let blocker = scope.spawn(move || {
            spark_storage::settings::update_settings_section(
                blocker_path,
                blocker_revision,
                "unused",
                None,
                |_| {
                    locked_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    Ok(())
                },
            )
            .unwrap();
        });
        locked_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let reference = scope.spawn(|| {
            save(
                &settings,
                "project_models",
                &revision,
                json!({"project_path":"/project", "model_settings":{"llm_profile":"team"}}),
            )
        });
        let lock_path = settings.config_dir.join("profile-references.lock");
        let probe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match FileExt::try_lock_exclusive(&probe) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("unexpected lock failure: {error}"),
                Ok(()) => FileExt::unlock(&probe).unwrap(),
            }
            assert!(
                Instant::now() < deadline,
                "reference writer did not acquire the graph lock"
            );
            std::thread::yield_now();
        }
        let deletion = scope.spawn(|| save(&settings, "llm_profiles", profile_revision, json!([])));
        release_tx.send(()).unwrap();
        blocker.join().unwrap();
        reference.join().unwrap().unwrap();
        assert!(deletion
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("project.toml.model_settings.llm_profile"));
    });
    assert_eq!(
        spark_storage::settings::read_settings_document(&path)
            .unwrap()
            .values["model_settings"]["llm_profile"]
            .as_str(),
        Some("team")
    );
}

#[test]
fn reference_creation_after_deletion_is_rejected() {
    let (_temp, settings) = fixture();
    let first = save(&settings, "llm_profiles", "absent", json!([llm("team")])).unwrap();
    save(
        &settings,
        "llm_profiles",
        first["llm_profiles"]["revision"].as_str().unwrap(),
        json!([]),
    )
    .unwrap();
    assert!(save(&settings, "models", "absent", json!({"llm_profile":"team"})).is_err());
    assert!(!settings.config_dir.join("spark.toml").exists());
}

#[test]
fn connection_updates_preserve_other_sections_and_report_running_values() {
    let (_temp, mut settings) = fixture();
    settings.connections.server_port = Some(8000);
    save(&settings, "models", "absent", json!({"provider":"codex"})).unwrap();
    let view = workspace_settings(&settings).unwrap();
    let revision = view["connections"]["revision"].as_str().unwrap();
    let saved = save(
        &settings,
        "connections",
        revision,
        json!({"server_port":9123,"client_api_base_url":"https://spark.example"}),
    )
    .unwrap();
    assert_eq!(saved["connections"]["stored"]["server_port"], 9123);
    assert_eq!(saved["connections"]["effective"]["server_port"], 8000);
    assert_eq!(saved["models"]["stored"]["provider"], "codex");
    assert!(save(
        &settings,
        "connections",
        revision,
        json!({"server_port":8123})
    )
    .is_err());
}

#[test]
fn candidate_profile_contents_are_checked_by_validate_and_save_without_writes() {
    use spark_workspace::settings::validate_workspace_settings_update;
    let (_temp, settings) = fixture();
    let first = save(&settings, "llm_profiles", "absent", json!([llm("team")])).unwrap();
    let revision = first["llm_profiles"]["revision"].as_str().unwrap();
    save(
        &settings,
        "models",
        "absent",
        json!({"llm_profile":"team", "model":"model"}),
    )
    .unwrap();
    let path = settings.config_dir.join("llm-profiles.toml");
    let before = std::fs::read(&path).unwrap();
    let mut changed = llm("team");
    changed["models"] = json!(["other"]);
    changed["default_model"] = json!("other");
    for value in [json!([]), json!([changed])] {
        let request = serde_json::from_value(
            json!({"section":"llm_profiles", "expected_revision":revision, "value":value}),
        )
        .unwrap();
        assert!(validate_workspace_settings_update(&settings, &request)
            .unwrap_err()
            .to_string()
            .contains("spark.toml.models.llm_profile"));
        assert!(update_workspace_settings(&settings, request).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    let mut valid = llm("team");
    valid["label"] = json!("Renamed");
    valid["base_url"] = json!("http://localhost:9998/v1");
    valid["models"] = json!(["model", "other"]);
    save(&settings, "llm_profiles", revision, json!([valid])).unwrap();
}

#[test]
fn invalid_model_reference_is_scoped_and_repairable_without_blocking_connections() {
    let (_temp, settings) = fixture();
    let path = settings.config_dir.join("spark.toml");
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    std::fs::write(&path, "[models]\nllm_profile='missing'\n").unwrap();
    let view = workspace_settings(&settings).unwrap();
    assert_eq!(view["models"]["stored"]["llm_profile"], "missing");
    assert!(view["models"]["effective"].is_null());
    assert!(!view["models"]["validation_errors"]
        .as_array()
        .unwrap()
        .is_empty());
    let saved = save(
        &settings,
        "connections",
        view["connections"]["revision"].as_str().unwrap(),
        json!({"client_api_base_url":"http://localhost:4987"}),
    )
    .unwrap();
    assert_eq!(
        saved["connections"]["effective"]["client_api_base_url"],
        spark_storage::settings::resolve_client_api_base_url(
            &spark_common::settings::resolve_settings_with_env(
                &Default::default(),
                &spark_common::paths::ProcessEnvironment
            )
            .unwrap()
            .config_dir,
            &spark_common::paths::ProcessEnvironment,
        )
        .unwrap()
        .0
    );
    assert_eq!(saved["models"]["stored"]["llm_profile"], "missing");
    let repaired = save(
        &settings,
        "models",
        saved["models"]["revision"].as_str().unwrap(),
        json!({"provider":"codex"}),
    )
    .unwrap();
    assert_eq!(repaired["models"]["validation_errors"], json!([]));
}

#[test]
fn disabling_referenced_execution_profile_fails_validate_and_save() {
    let (_temp, settings) = fixture();
    let profile = json!({"id":"native", "label":"Native", "mode":"native", "enabled":true, "capabilities":[], "metadata":{}});
    let first = save(
        &settings,
        "execution_profiles",
        "absent",
        json!({"profiles":[profile.clone()], "default_execution_profile_id":null}),
    )
    .unwrap();
    spark_storage::ProjectRegistry::new(&settings.data_dir)
        .register_project_with_execution_profile("/project", Some("native"))
        .unwrap();
    let mut disabled = profile;
    disabled["enabled"] = json!(false);
    let request = serde_json::from_value(json!({"section":"execution_profiles", "expected_revision":first["execution_profiles"]["revision"], "value":{"profiles":[disabled], "default_execution_profile_id":null}})).unwrap();
    let path = settings.config_dir.join("execution-profiles.toml");
    let before = std::fs::read(&path).unwrap();
    assert!(
        spark_workspace::settings::validate_workspace_settings_update(&settings, &request).is_err()
    );
    assert!(update_workspace_settings(&settings, request).is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn invalid_section_shape_preserves_stored_value_and_can_be_replaced_independently() {
    let (_temp, settings) = fixture();
    let path = settings.config_dir.join("spark.toml");
    std::fs::create_dir_all(&settings.config_dir).unwrap();
    std::fs::write(
        &path,
        "[runtime]\nproject_roots=42\n[models]\nprovider='codex'\n",
    )
    .unwrap();
    let view = workspace_settings(&settings).unwrap();
    assert_eq!(view["runtime"]["stored"]["project_roots"], 42);
    assert_eq!(
        view["runtime"]["active_startup"]["flows_dir"],
        settings.flows_dir.to_string_lossy().as_ref()
    );
    assert_eq!(
        view["runtime"]["repair_defaults"]["project_roots"],
        json!([])
    );
    assert_eq!(view["models"]["effective"]["provider"], "codex");
    let repaired = save(
        &settings,
        "runtime",
        view["runtime"]["revision"].as_str().unwrap(),
        view["runtime"]["repair_defaults"].clone(),
    )
    .unwrap();
    assert_eq!(repaired["runtime"]["validation_errors"], json!([]));
    assert_eq!(repaired["models"]["stored"], view["models"]["stored"]);
}

#[test]
fn inherited_flow_profile_checks_node_models_and_explicit_selection_precedence() {
    use spark_workspace::settings::validate_workspace_settings_update;
    let (_temp, settings) = fixture();
    let mut team = llm("team");
    team["models"] = json!(["m1", "m2"]);
    team["default_model"] = json!("m1");
    let mut other = llm("other");
    other["models"] = json!(["m2"]);
    other["default_model"] = json!("m2");
    let first = save(&settings, "llm_profiles", "absent", json!([team, other])).unwrap();
    let revision = first["llm_profiles"]["revision"].as_str().unwrap();
    let flow_path = settings.flows_dir.join("inherited.yaml");
    let source = r#"schema_version: "1"
id: inherited
defaults:
  llm_profile: team
  llm_model: m1
nodes:
  start:
    kind: start
  work:
    kind: agent_task
    config:
      kind: agent_task
      prompt: No model call in this test
    execution:
      llm_model: m2
  exit:
    kind: exit
edges:
  - from: start
    to: work
  - from: work
    to: exit
"#;
    std::fs::write(&flow_path, source).unwrap();
    let path = settings.config_dir.join("llm-profiles.toml");
    let before = std::fs::read(&path).unwrap();
    let mut candidate = json!([team, other]);
    candidate[0]["models"] = json!(["m1"]);
    let request = || {
        serde_json::from_value(json!({
            "section":"llm_profiles", "expected_revision":revision, "value":candidate
        }))
        .unwrap()
    };
    let error = validate_workspace_settings_update(&settings, &request()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("flow inherited.yaml.nodes.work.execution.llm_profile"),
        "{error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let error = update_workspace_settings(&settings, request()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("flow inherited.yaml.nodes.work.execution.llm_profile"),
        "{error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // Explicit node profile wins over the flow profile; its model wins over m1.
    std::fs::write(
        &flow_path,
        source.replace("llm_model: m2", "llm_model: m2\n      llm_profile: other"),
    )
    .unwrap();
    validate_workspace_settings_update(&settings, &request()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let saved = update_workspace_settings(&settings, request()).unwrap();
    assert_eq!(saved["llm_profiles"]["stored"][1]["models"], json!(["m1"]));

    // Endpoint, label and supported model-list edits remain valid with inheritance.
    std::fs::write(&flow_path, source).unwrap();
    candidate[0]["models"] = json!(["m1", "m2", "m3"]);
    candidate[0]["label"] = json!("Renamed team");
    candidate[0]["base_url"] = json!("http://localhost:9998/v1");
    let request = serde_json::from_value(json!({
        "section":"llm_profiles", "expected_revision":saved["llm_profiles"]["revision"], "value":candidate
    })).unwrap();
    let before = std::fs::read(&path).unwrap();
    validate_workspace_settings_update(&settings, &request).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    update_workspace_settings(&settings, request).unwrap();
}
