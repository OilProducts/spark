use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::paths::{ensure_directory, resolve_runtime_workspace_path_with_env};
use spark_common::project::{
    build_project_id, normalize_project_path, project_storage_dir, project_work_dir, RunsRoot,
};
use spark_common::settings::{
    parse_project_roots, resolve_settings_with_env, validate_settings, SettingsOverrides,
    SparkSettings,
};
use spark_common::source_checkout::{
    default_api_target_refusal_message, default_runtime_home_refusal_message,
    installed_package_root_from_executable, is_source_checkout,
    require_explicit_agent_base_url_with_env, require_explicit_dev_home_with_env,
};

#[cfg(unix)]
fn symlink_dir(original: &Path, link: &Path) {
    std::os::unix::fs::symlink(original, link).expect("symlink dir");
}

// TempDir may hand back a symlinked path (macOS /var -> /private/var); the code
// under test resolves symlinks, so expectations must start from the physical path.
fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    (temp, root)
}

fn env(entries: &[(&str, &Path)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.to_string_lossy().into_owned()))
        .collect()
}

#[test]
fn settings_resolution_preserves_precedence_and_derived_paths() {
    let (_temp, root) = canonical_tempdir();
    let env_home = root.join("env-home");
    let cli_home = root.join("cli-home");
    let ui = root.join("ui");
    let env = env(&[("SPARK_HOME", &env_home), ("SPARK_UI_DIR", &ui)]);

    let settings = resolve_settings_with_env(
        &SettingsOverrides {
            data_dir: Some(cli_home.clone()),
            ..SettingsOverrides::default()
        },
        &env,
    )
    .expect("settings");

    assert_eq!(settings.data_dir, cli_home);
    assert_eq!(settings.config_dir, settings.data_dir.join("config"));
    assert_eq!(settings.runtime_dir, settings.data_dir.join("runtime"));
    // Runs default to each project's private storage `G/runs`.
    assert_eq!(settings.runs_dir, None);
    let runs = settings.runs_root();
    assert_eq!(runs, RunsRoot::Home(cli_home.clone()));
    assert_eq!(runs.projects_dir(), cli_home.join("projects"));
    assert_eq!(
        runs.project_runs_dir("demo-0123456789ab"),
        project_storage_dir(&cli_home, "demo-0123456789ab").join("runs")
    );
    assert_eq!(
        project_storage_dir(&cli_home, "demo-0123456789ab"),
        cli_home.join("projects/demo-0123456789ab")
    );
    let project = "/work/demo";
    assert_eq!(
        project_work_dir(&cli_home, project).unwrap(),
        cli_home
            .join("projects")
            .join(build_project_id(project).unwrap())
            .join("work")
    );
    // A configured external root keeps `<root>/<project-id>/<run-id>`.
    let external = RunsRoot::External(cli_home.join("elsewhere"));
    assert_eq!(external.projects_dir(), cli_home.join("elsewhere"));
    assert_eq!(
        external.project_runs_dir("demo-0123456789ab"),
        cli_home.join("elsewhere/demo-0123456789ab")
    );
    assert_eq!(settings.bundled_dir(), settings.data_dir.join("bundled"));
    assert_eq!(settings.ui_dir, Some(ui));
}

/// This repository's ignore rules admit authored definitions and keep Spark's
/// staging names and other local `.spark` data out.
#[test]
fn repository_ignore_rules_admit_definitions_and_exclude_staging() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ignored = |path: &str| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["check-ignore", "-q", "--no-index", path])
            .status()
            .expect("git check-ignore")
            .success()
    };
    for path in [
        ".spark/definitions/flows/review/flow.md",
        ".spark/definitions/roles/reviewer.md",
        ".spark/definitions/triggers/nightly.toml",
    ] {
        assert!(!ignored(path), "{path}");
    }
    for path in [
        ".spark/definitions/flows/.spark-deleting-abc/flow.md",
        ".spark/definitions/triggers/.spark-storage-abc",
        ".spark/state/checkouts/run/file",
        ".spark/tmp/file",
    ] {
        assert!(ignored(path), "{path}");
    }
}

#[test]
fn project_roots_parse_absolute_entries_and_skip_empty_or_relative_entries() {
    let (_temp, root) = canonical_tempdir();
    let primary = root.join("primary");
    let secondary = root.join("secondary");
    let roots = std::env::join_paths([
        PathBuf::from("relative"),
        primary.clone(),
        PathBuf::new(),
        secondary.clone(),
    ])
    .expect("join paths");
    let roots = parse_project_roots(roots.to_str()).expect("project roots");

    assert_eq!(roots, vec![primary, secondary]);
}

#[cfg(unix)]
#[test]
fn project_roots_resolve_existing_symlink_prefixes_without_requiring_missing_tails() {
    let (_temp, root) = canonical_tempdir();
    let real_root = root.join("real-root");
    let linked_root = root.join("linked-root");
    std::fs::create_dir_all(&real_root).expect("real root");
    symlink_dir(&real_root, &linked_root);

    let requested_project_root = linked_root.join("future-project");
    let expected_project_root = real_root.join("future-project");
    let roots = std::env::join_paths([requested_project_root, PathBuf::from("relative")])
        .expect("join paths");

    let parsed = parse_project_roots(roots.to_str()).expect("project roots");

    assert_eq!(parsed, vec![expected_project_root]);
}

#[test]
fn validate_settings_creates_runtime_directories_and_checks_ui_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui = temp.path().join("ui");
    std::fs::create_dir_all(&ui).expect("ui dir");
    std::fs::write(ui.join("index.html"), "<main></main>").expect("ui index");
    let env = env(&[
        ("SPARK_HOME", &temp.path().join("spark-home")),
        ("SPARK_UI_DIR", &ui),
    ]);
    let settings =
        resolve_settings_with_env(&SettingsOverrides::default(), &env).expect("settings");

    validate_settings(&settings).expect("valid settings");

    for path in [&settings.config_dir, &settings.runtime_dir] {
        assert!(path.is_dir(), "{} should be created", path.display());
    }
    // No obsolete registration, global log or shared runs directories.
    for obsolete in ["logs", "workspace", "attractor", "projects"] {
        assert!(!settings.data_dir.join(obsolete).exists(), "{obsolete}");
    }
    let external = temp.path().join("external-runs");
    validate_settings(&SparkSettings {
        runs_dir: Some(external.clone()),
        ..settings
    })
    .expect("valid external runs root");
    assert!(external.is_dir());
}

#[test]
fn directory_creation_errors_use_compatible_message_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let blocked = temp.path().join("blocked");
    std::fs::write(&blocked, "not a dir").expect("blocked file");

    let error = ensure_directory(&blocked, "runtime").expect_err("directory creation");

    assert!(error
        .to_string()
        .starts_with("Unable to create runtime directory: "));
}

#[test]
fn project_identity_matches_slug_and_sha1_contract() {
    // Use a prefix that exists on no platform so resolve(strict=false) semantics
    // keep the path verbatim; /tmp is a symlink on macOS and would change the hash.
    let project_id = build_project_id("/spark-contract-fixture/Spark Demo!").expect("project id");

    assert_eq!(project_id, "spark-demo-23a3ac05822d");
    assert_eq!(normalize_project_path("  ").expect("empty normalize"), None);
    assert_eq!(
        build_project_id("  ")
            .expect_err("empty project")
            .to_string(),
        "Project path is required."
    );
}

#[cfg(unix)]
#[test]
fn project_identity_resolves_symlinked_prefixes_like_python_resolve_strict_false() {
    let (_temp, root) = canonical_tempdir();
    let real_root = root.join("real-root");
    let linked_root = root.join("linked-root");
    let real_project = real_root.join("Spark Demo!");
    std::fs::create_dir_all(&real_project).expect("real project");
    symlink_dir(&real_root, &linked_root);
    let requested_project = linked_root.join("Spark Demo!");

    assert_eq!(
        normalize_project_path(requested_project.to_string_lossy())
            .expect("normalize project")
            .as_deref(),
        Some(real_project.as_path())
    );
    assert_eq!(
        build_project_id(requested_project.to_string_lossy()).expect("linked project id"),
        build_project_id(real_project.to_string_lossy()).expect("real project id")
    );

    let requested_missing_tail = requested_project.join("future");
    let real_missing_tail = real_project.join("future");
    assert_eq!(
        normalize_project_path(requested_missing_tail.to_string_lossy())
            .expect("normalize missing tail")
            .as_deref(),
        Some(real_missing_tail.as_path())
    );
}

#[test]
fn runtime_workspace_path_remaps_host_root_to_existing_runtime_root() {
    let (_temp, root) = canonical_tempdir();
    let host_root = root.join("host/repo");
    let runtime_root = root.join("runtime/repo");
    let runtime_file = runtime_root.join("project/src/main.rs");
    std::fs::create_dir_all(runtime_file.parent().expect("parent")).expect("runtime dirs");
    std::fs::write(&runtime_file, "fn main() {}\n").expect("runtime file");
    let requested = host_root.join("project/src/main.rs");
    let env = env(&[
        ("ATTRACTOR_HOST_REPO_ROOT", &host_root),
        ("ATTRACTOR_RUNTIME_REPO_ROOT", &runtime_root),
    ]);

    let resolved = resolve_runtime_workspace_path_with_env(requested.to_string_lossy(), &env)
        .expect("runtime path");

    assert_eq!(resolved, runtime_file.to_string_lossy());
}

#[test]
fn runtime_workspace_path_keeps_existing_requested_path() {
    let (_temp, root) = canonical_tempdir();
    let host_root = root.join("host/repo");
    let runtime_root = root.join("runtime/repo");
    let requested = host_root.join("project/src/main.rs");
    let runtime_file = runtime_root.join("project/src/main.rs");
    std::fs::create_dir_all(requested.parent().expect("parent")).expect("host dirs");
    std::fs::create_dir_all(runtime_file.parent().expect("parent")).expect("runtime dirs");
    std::fs::write(&requested, "host").expect("host file");
    std::fs::write(&runtime_file, "runtime").expect("runtime file");
    let env = env(&[
        ("ATTRACTOR_HOST_REPO_ROOT", &host_root),
        ("ATTRACTOR_RUNTIME_REPO_ROOT", &runtime_root),
    ]);

    let resolved = resolve_runtime_workspace_path_with_env(requested.to_string_lossy(), &env)
        .expect("runtime path");

    assert_eq!(resolved, requested.to_string_lossy());
}

#[test]
fn source_checkout_guard_messages_match_cargo_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("cargo manifest");
    std::fs::create_dir_all(root.join("crates/spark-cli")).expect("cli crate");
    std::fs::create_dir_all(root.join("crates/spark-server")).expect("server crate");
    std::fs::create_dir_all(root.join("crates/spark-assets/assets/flows")).expect("flows");
    std::fs::create_dir_all(root.join("frontend")).expect("frontend");
    let empty_env = BTreeMap::<String, String>::new();

    assert!(is_source_checkout(root));
    let home_error =
        require_explicit_dev_home_with_env("spark-server init", None, root, &empty_env)
            .expect_err("home guard");
    assert_eq!(
        home_error.to_string(),
        default_runtime_home_refusal_message("spark-server init", root)
    );
    let api_error =
        require_explicit_agent_base_url_with_env("spark flow list", None, root, &empty_env)
            .expect_err("api guard");
    assert_eq!(
        api_error.to_string(),
        default_api_target_refusal_message("spark flow list", root)
    );

    let mut configured_env = BTreeMap::new();
    configured_env.insert(
        "SPARK_HOME".to_string(),
        root.join(".spark-dev").display().to_string(),
    );
    require_explicit_dev_home_with_env("spark-server init", None, root, &configured_env)
        .expect("configured home");
    configured_env.insert("SPARK_HOME".to_string(), String::new());
    require_explicit_dev_home_with_env("spark-server init", None, root, &configured_env)
        .expect_err("empty home does not satisfy guard");
}

#[test]
fn installed_package_root_detection_accepts_cargo_install_roots_without_source_guarding() {
    let temp = tempfile::tempdir().expect("tempdir");
    let install_root = temp.path().join("cargo-root");
    let executable_path = install_root.join("bin/spark-server");
    std::fs::create_dir_all(executable_path.parent().expect("bin parent")).expect("bin dir");

    assert_eq!(
        installed_package_root_from_executable(&executable_path, "spark-server").as_deref(),
        Some(install_root.as_path())
    );
    assert!(!is_source_checkout(&install_root));
    require_explicit_dev_home_with_env(
        "spark-server init",
        None,
        &install_root,
        &BTreeMap::<String, String>::new(),
    )
    .expect("installed cargo root may use default home");
}

#[test]
fn turn_stream_events_round_trip_and_validate_content_channel() {
    let mut event = TurnStreamEvent::content_delta(TurnStreamChannel::Assistant, "Ack");
    event.source = TurnStreamSource {
        backend: Some("codex_app_server".to_string()),
        item_id: Some("msg-1".to_string()),
        raw_kind: Some("response/output_text/delta".to_string()),
        ..TurnStreamSource::default()
    };
    let payload = serde_json::to_value(&event).expect("serialize event");

    assert_eq!(
        payload,
        json!({
            "kind": "content_delta",
            "channel": "assistant",
            "source": {
                "backend": "codex_app_server",
                "item_id": "msg-1",
                "raw_kind": "response/output_text/delta"
            },
            "content_delta": "Ack",
            "message": "Ack"
        })
    );
    assert_eq!(
        serde_json::from_value::<TurnStreamEvent>(payload).expect("deserialize event"),
        event
    );
    assert!(
        serde_json::from_value::<TurnStreamEvent>(json!({"kind": "content_delta"}))
            .expect_err("content channel validation")
            .to_string()
            .contains("content TurnStreamEvent values must set channel")
    );

    let unknown = TurnStreamEvent::new(TurnStreamEventKind::Other("custom".to_string()))
        .expect("unknown kind event");
    assert_eq!(
        serde_json::to_value(unknown).expect("unknown json")["kind"],
        "custom"
    );
}

#[test]
fn reasoning_settings_are_nullable_backward_compatible_and_field_validated() {
    use spark_common::settings::ModelSettings;
    let old: ModelSettings =
        serde_json::from_value(serde_json::json!({"provider":"openai"})).unwrap();
    old.validate().unwrap();
    let value = serde_json::to_value(&old).unwrap();
    for field in [
        "thinking",
        "thinking_budget_tokens",
        "reasoning_mode",
        "reasoning_summary",
    ] {
        assert!(value[field].is_null());
    }
    let valid = serde_json::json!({"provider":"anthropic", "thinking":"budget", "thinking_budget_tokens":1024,"reasoning_mode":"pro","reasoning_summary":"detailed"});
    let settings: ModelSettings = serde_json::from_value(valid.clone()).unwrap();
    settings.validate().unwrap();
    let restored: ModelSettings =
        serde_json::from_value(serde_json::to_value(settings.clone()).unwrap()).unwrap();
    assert_eq!(settings, restored);
    for (field, bad) in [
        ("thinking", serde_json::json!("enabled")),
        ("reasoning_mode", serde_json::json!("fast")),
        ("reasoning_summary", serde_json::json!("none")),
        ("thinking_budget_tokens", serde_json::json!(100)),
        ("thinking_budget_tokens", serde_json::json!(1024.5)),
        ("thinking_budget_tokens", serde_json::json!(-1)),
        ("thinking_budget_tokens", serde_json::Value::Null),
    ] {
        let mut value = valid.clone();
        value[field] = bad;
        let result = serde_json::from_value::<ModelSettings>(value)
            .map_err(|e| e.to_string())
            .and_then(|s| s.validate().map_err(|e| e.to_string()));
        assert!(result.unwrap_err().contains(field));
    }
    let mut invalid = restored;
    invalid.thinking = None;
    assert!(invalid
        .validate()
        .unwrap_err()
        .to_string()
        .contains("thinking_budget_tokens"));
}

#[test]
fn compaction_segment_says_what_it_freed_when_the_backend_reports_it() {
    let content = |message: Option<&str>| {
        let event: TurnStreamEvent = serde_json::from_value(json!({
            "kind": "context_compaction_completed",
            "message": message,
            "source": {"app_turn_id": "turn-app"},
        }))
        .unwrap();
        let mut snapshot = json!({"segments": []});
        spark_common::segments::materialize_segment_for_event(
            &mut snapshot,
            "turn-1",
            &event,
            "2026-10-15T00:00:00Z",
            None,
        )
        .unwrap()["content"]
            .clone()
    };
    assert_eq!(
        content(Some("Context compacted (21.6k \u{2192} 1.9k tokens).")),
        "Context compacted (21.6k \u{2192} 1.9k tokens)."
    );
    assert_eq!(content(None), "Context compacted to continue the turn.");
}

fn acp_config(
    harnesses: serde_json::Value,
) -> Result<spark_common::agent_settings::SessionConfig, String> {
    let config: spark_common::agent_settings::SessionConfig =
        serde_json::from_value(json!({ "acp_harnesses": harnesses }))
            .map_err(|error| error.to_string())?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

#[test]
fn acp_selectors_own_their_namespace_and_normalize_native_aliases() {
    use spark_common::agent_settings::HarnessIdentity;
    use spark_common::settings::ModelSettings;

    assert_eq!(
        HarnessIdentity::parse(" Claude_Code "),
        Ok(Some(HarnessIdentity::ClaudeCode))
    );
    assert_eq!(
        HarnessIdentity::parse("CODEX"),
        Ok(Some(HarnessIdentity::Codex))
    );
    assert_eq!(HarnessIdentity::parse("openai"), Ok(None));
    let acp = HarnessIdentity::parse("ACP:my-agent_2").unwrap().unwrap();
    assert_eq!(acp, HarnessIdentity::Acp("my-agent_2".into()));
    assert_eq!(acp.selector(), "acp:my-agent_2");
    assert_eq!(acp.session_owner(), "acp:my-agent_2");
    assert_eq!(
        HarnessIdentity::ClaudeCode.session_owner(),
        "claude_code_cli"
    );
    // A malformed ACP selector never falls through to a direct provider.
    for selector in ["acp:", "acp:Upper", "acp:-x", "acp:a b", "acp:codex/../x"] {
        assert!(HarnessIdentity::parse(selector).is_err(), "{selector}");
    }

    let selection = |provider: Option<&str>, profile: Option<&str>| ModelSettings {
        provider: provider.map(str::to_string),
        llm_profile: profile.map(str::to_string),
        ..ModelSettings::default()
    };
    assert!(selection(Some("acp:agent"), None).validate().is_ok());
    assert!(selection(Some("acp:Bad"), None).validate().is_err());
    assert!(selection(None, Some("ACP:agent")).validate().is_err());
    assert!(selection(None, Some("team-profile")).validate().is_ok());
    // Session controls belong to the ACP harness they were chosen for.
    let controls = |provider: &str, id: &str, value: &str| ModelSettings {
        session_config: [(id.to_string(), value.to_string())].into(),
        ..selection(Some(provider), None)
    };
    assert!(controls("acp:agent", "Mode/ID", "Auto Edit")
        .validate()
        .is_ok());
    assert!(controls("codex", "mode", "auto").validate().is_err());
    assert!(controls("acp:agent", "mode", " ").validate().is_err());

    assert!(acp_config(json!({"Bad": {"name": "x", "executable": "x"}})).is_err());
    assert!(acp_config(json!({"ok": {"name": "x", "executable": "x", "shell": true}})).is_err());
}

#[test]
fn acp_registrations_keep_literal_args_and_reject_reserved_or_unknown_bindings() {
    let config = acp_config(json!({"agent": {
        "name": "Agent",
        "executable": "/opt/agent bin/run",
        "args": ["acp", "--flag=$HOME", "'quoted' && echo; *"],
        "env": {"AGENT_TOKEN": "secret"}
    }}))
    .expect("valid registration");
    let harness = &config.acp_harnesses["agent"];
    assert_eq!(harness.args, ["acp", "--flag=$HOME", "'quoted' && echo; *"]);
    assert_eq!(
        harness.permission_policy,
        spark_common::agent_settings::AcpPermissionPolicy::Allow
    );

    for reserved in spark_common::agent_settings::RESERVED_HARNESS_ENV {
        assert!(acp_config(
            json!({"agent": {"name": "A", "executable": "a", "env": {reserved: "x"}}})
        )
        .is_err());
        assert!(acp_config(json!({"agent": {"name": "A", "executable": "a",
            "system_instructions": {"env": {reserved: {"text": "{{instructions}}"}}}}}))
        .is_err());
    }
    // Unknown placeholders and channels that never carry the instructions are invalid.
    for channel in [
        json!({"args": ["--system={{instruction}}"]}),
        json!({"env": {"X": {"json": {"a": "{{home}}"}}}, "args": ["{{instructions}}"]}),
        json!({"args": ["--static"]}),
        json!({}),
        json!({"args": ["{{instructions}}"], "config": {"mode": ""}}),
    ] {
        assert!(
            acp_config(
                json!({"agent": {"name": "A", "executable": "a", "system_instructions": channel}})
            )
            .is_err(),
            "{channel}"
        );
    }
}

#[test]
fn acp_instruction_channels_render_exact_bytes_once() {
    use spark_common::agent_settings::{EnvTemplate, InstructionChannel};

    let channel = InstructionChannel {
        env: BTreeMap::from([
            (
                "TEXT".into(),
                EnvTemplate::Text("pre {{instructions}}".into()),
            ),
            (
                "JSON".into(),
                EnvTemplate::Json(
                    json!({"agent": {"prompt": "{{instructions}}", "n": 1}, "file": ["{{instructions_file}}"]}),
                ),
            ),
        ]),
        args: vec!["--prompt-file={{instructions_file}}".into()],
        config: BTreeMap::new(),
    };
    assert!(channel.uses_file());
    let instructions = "Line \"one\"\nTab\t{{instructions_file}} stays literal";
    let (env, args) = channel.render(instructions, "/tmp/i.md").unwrap();
    let env = env.into_iter().collect::<BTreeMap<_, _>>();
    assert_eq!(env["TEXT"], format!("pre {instructions}"));
    let json: serde_json::Value = serde_json::from_str(&env["JSON"]).unwrap();
    assert_eq!(json["agent"]["prompt"], instructions);
    assert_eq!(json["agent"]["n"], 1);
    assert_eq!(json["file"][0], "/tmp/i.md");
    assert_eq!(args, ["--prompt-file=/tmp/i.md"]);
}

#[test]
fn acp_configuration_generation_tracks_launch_settings_but_not_labels() {
    use spark_common::agent_settings::EnvironmentInheritancePolicy;
    let base = json!({"name": "Agent", "executable": "agent", "args": ["acp"], "env": {"K": "v"}});
    let generation = |patch: serde_json::Value| {
        let mut harness = base.clone();
        for (key, value) in patch.as_object().unwrap() {
            harness[key] = value.clone();
        }
        acp_config(json!({"agent": harness})).unwrap().acp_harnesses["agent"]
            .generation(EnvironmentInheritancePolicy::InheritAll)
    };
    let original = generation(json!({}));
    assert_eq!(original, generation(json!({"name": "Renamed"})));
    for change in [
        json!({"executable": "other"}),
        json!({"args": ["acp", "--x"]}),
        json!({"env": {"K": "changed"}}),
        json!({"permission_policy": "deny"}),
        json!({"system_instructions": {"args": ["{{instructions}}"]}}),
    ] {
        let changed = generation(change.clone());
        assert_ne!(original, changed, "{change}");
        assert!(!changed.contains("changed"), "the generation is opaque");
    }
    let harness = &acp_config(json!({"agent": base})).unwrap().acp_harnesses["agent"];
    assert_ne!(
        original,
        harness.generation(EnvironmentInheritancePolicy::InheritNone),
        "the inherited environment shapes the launch"
    );
}

#[test]
fn redacted_acp_environments_restore_only_from_the_same_registration() {
    let inheritance = spark_common::agent_settings::EnvironmentInheritancePolicy::default();
    // A secret in the environment, or only in a declared channel's.
    let registrations = [
        |token: &str| json!({"name": "Agent", "executable": "agent", "env": {"TOKEN": token}}),
        |token: &str| {
            json!({"name": "Agent", "executable": "agent", "system_instructions": {"env": {
                "PROMPT": {"json": {"key": token, "system": "{{instructions_file}}"}},
            }}})
        },
    ];
    for registration in registrations {
        let config = |token: &str| acp_config(json!({"agent": registration(token)})).unwrap();
        let live = config("secret-sentinel");
        let mut captured = live.clone();
        captured.redact_acp_env();
        assert!(captured.validate().is_ok());
        assert!(!json!(captured).to_string().contains("secret-sentinel"));
        let harness = &captured.acp_harnesses["agent"];

        let restored = harness
            .clone()
            .restore_env(Some(&live.acp_harnesses["agent"]), inheritance)
            .unwrap();
        assert_eq!(restored, live.acp_harnesses["agent"]);
        let rotated = config("rotated");
        for current in [None, Some(&rotated.acp_harnesses["agent"])] {
            let error = harness
                .clone()
                .restore_env(current, inheritance)
                .unwrap_err();
            assert!(error.contains("start the work again"), "{error}");
        }
    }
}

#[test]
fn windows_home_tilde_and_verbatim_prefix_helpers_normalize_cleanly() {
    use spark_common::paths::{
        expand_tilde_with_env, strip_verbatim_prefix, user_home_dir_with_env,
    };
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    let env_profile = BTreeMap::from([("USERPROFILE".to_string(), r"C:\Users\chris".to_string())]);
    assert_eq!(
        user_home_dir_with_env(&env_profile),
        Some(PathBuf::from(r"C:\Users\chris"))
    );
    assert_eq!(
        expand_tilde_with_env(Path::new(r"~\.spark"), &env_profile),
        PathBuf::from(r"C:\Users\chris").join(".spark")
    );
    assert_eq!(
        expand_tilde_with_env(Path::new("~/.spark"), &env_profile),
        PathBuf::from(r"C:\Users\chris").join(".spark")
    );

    let env_drive = BTreeMap::from([
        ("HOMEDRIVE".to_string(), "D:".to_string()),
        ("HOMEPATH".to_string(), r"\Users\builder".to_string()),
    ]);
    assert_eq!(
        user_home_dir_with_env(&env_drive),
        Some(PathBuf::from(r"D:\Users\builder"))
    );

    assert_eq!(
        strip_verbatim_prefix(PathBuf::from(r"\\?\C:\Users\chris\repo")),
        PathBuf::from(r"C:\Users\chris\repo")
    );
    assert_eq!(
        strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\fileserver\share\repo")),
        PathBuf::from(r"\\fileserver\share\repo")
    );
    assert_eq!(
        strip_verbatim_prefix(PathBuf::from("/Users/chris/repo")),
        PathBuf::from("/Users/chris/repo")
    );
}

#[test]
fn verbatim_prefixes_are_kept_wherever_win32_would_name_another_target() {
    use spark_common::paths::strip_verbatim_prefix;

    let strip = |text: &str| strip_verbatim_prefix(PathBuf::from(text));
    // 3 + 244 = 247 UTF-16 code units: one under the threshold.
    let longest = format!(r"C:\{}", "a".repeat(244));
    for (verbatim, conventional) in [
        (r"\\?\C:\".to_string(), r"C:\".to_string()),
        (
            r"\\?\d:\Users\chris\notes.v2".into(),
            r"d:\Users\chris\notes.v2".into(),
        ),
        (
            r"\\?\UNC\fileserver\share".into(),
            r"\\fileserver\share".into(),
        ),
        (
            r"\\?\UNC\fileserver\share\repo".into(),
            r"\\fileserver\share\repo".into(),
        ),
        // Only look like device names.
        (
            r"\\?\C:\console\com10\nullable\lpt.txt".into(),
            r"C:\console\com10\nullable\lpt.txt".into(),
        ),
        (format!(r"\\?\{longest}"), longest.clone()),
    ] {
        assert_eq!(strip(&verbatim), PathBuf::from(conventional), "{verbatim}");
    }

    for kept in [
        r"\\?\C:\repo\trailing.".to_string(),
        r"\\?\C:\repo.\file".into(),
        r"\\?\C:\repo\trailing ".into(),
        r"\\?\UNC\fileserver\share \repo".into(),
        r"\\?\C:\repo\nul".into(),
        r"\\?\C:\repo\NUL.txt\file".into(),
        r"\\?\C:\repo\nul .txt".into(),
        r"\\?\C:\repo\con.tar.gz".into(),
        r"\\?\C:\repo\Aux".into(),
        r"\\?\C:\repo\prn.md".into(),
        r"\\?\C:\repo\COM1".into(),
        r"\\?\C:\repo\com9.log".into(),
        r"\\?\C:\repo\LPT1".into(),
        r"\\?\C:\repo\lpt9.txt".into(),
        r"\\?\C:\repo\COM¹".into(),
        r"\\?\UNC\fileserver\share\CON".into(),
        r"\\?\C:\repo\.\file".into(),
        r"\\?\C:\repo\..\file".into(),
        r"\\?\C:\repo/file".into(),
        // 248 UTF-16 code units, then well over.
        format!(r"\\?\{longest}b"),
        format!(r"\\?\UNC\fileserver\share\{}", "a".repeat(240)),
        // Not the drive or UNC shape.
        r"\\?\C:".into(),
        r"\\?\C:relative".into(),
        r"\\?\UNC\fileserver".into(),
        r"\\?\Volume{0b1c}\repo".into(),
        r"\\?\GLOBALROOT\Device\HarddiskVolume1".into(),
    ] {
        assert_eq!(strip(&kept), PathBuf::from(&kept), "{kept}");
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let non_utf8 = PathBuf::from(std::ffi::OsStr::from_bytes(b"\\\\?\\C:\\repo-\xff"));
        assert_eq!(strip_verbatim_prefix(non_utf8.clone()), non_utf8);
    }
}

#[test]
fn home_resolution_skips_empty_values_before_each_fallback() {
    use spark_common::paths::{expand_tilde_with_env, user_home_dir_with_env};

    let vars = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>()
    };
    let profile = vars(&[("HOME", ""), ("USERPROFILE", r"C:\Users\chris")]);
    assert_eq!(
        user_home_dir_with_env(&profile),
        Some(PathBuf::from(r"C:\Users\chris"))
    );
    let drive = vars(&[
        ("HOME", ""),
        ("USERPROFILE", ""),
        ("HOMEDRIVE", "D:"),
        ("HOMEPATH", r"\Users\builder"),
    ]);
    assert_eq!(
        user_home_dir_with_env(&drive),
        Some(PathBuf::from(r"D:\Users\builder"))
    );
    assert_eq!(
        expand_tilde_with_env(Path::new("~/.spark"), &drive),
        PathBuf::from(r"D:\Users\builder").join(".spark")
    );
    // HOME still comes first when set.
    let both = vars(&[("HOME", "/home/chris"), ("USERPROFILE", r"C:\Users\chris")]);
    assert_eq!(
        user_home_dir_with_env(&both),
        Some(PathBuf::from("/home/chris"))
    );
    let none = vars(&[("HOME", ""), ("HOMEDRIVE", "D:"), ("HOMEPATH", "")]);
    assert_eq!(user_home_dir_with_env(&none), None);
    assert_eq!(
        expand_tilde_with_env(Path::new("~/.spark"), &none),
        PathBuf::from("~/.spark")
    );
}

/// Runs in a child process: HOME is process-wide.
#[cfg(unix)]
#[test]
fn a_non_utf8_home_survives_tilde_expansion_and_settings() {
    use spark_common::paths::{expand_tilde, normalize_path, user_home_dir, ProcessEnvironment};
    use spark_common::settings::resolve_data_dir;
    use std::os::unix::ffi::OsStrExt;

    const EXPECTED_HOME: &str = "SPARK_NON_UTF8_HOME_TEST";
    let Some(home) = std::env::var_os(EXPECTED_HOME) else {
        let (_temp, root) = canonical_tempdir();
        let home = root.join(std::ffi::OsStr::from_bytes(b"home-\xff"));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "common_contracts::a_non_utf8_home_survives_tilde_expansion_and_settings",
                "--nocapture",
            ])
            // No SPARK_HOME: the data dir falls back to `~/.spark`.
            .env_clear()
            .env("SPARK_TEST_ENV_ISOLATED", "1")
            .env(EXPECTED_HOME, &home)
            .env("HOME", &home)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let home = PathBuf::from(home);
    assert!(home.to_str().is_none());
    assert_eq!(user_home_dir(), Some(home.clone()));
    assert_eq!(expand_tilde(Path::new("~")), home);
    assert_eq!(expand_tilde(Path::new("~/.spark")), home.join(".spark"));
    let suffix = Path::new(std::ffi::OsStr::from_bytes(b"~/data-\xfe"));
    assert_eq!(
        expand_tilde(suffix),
        home.join(std::ffi::OsStr::from_bytes(b"data-\xfe"))
    );
    assert_eq!(normalize_path("~/.spark").unwrap(), home.join(".spark"));
    assert_eq!(
        resolve_data_dir(&ProcessEnvironment).unwrap(),
        home.join(".spark")
    );
}
