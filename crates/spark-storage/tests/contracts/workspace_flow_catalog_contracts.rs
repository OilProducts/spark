use std::fs;

use spark_storage::{
    load_flow_catalog, read_flow_launch_policy, set_flow_catalog_entry, set_flow_launch_policy,
    FlowExecutionLockConfig, StorageError, LAUNCH_POLICY_AGENT_REQUESTABLE, LAUNCH_POLICY_DISABLED,
};

#[test]
fn uncataloged_flows_default_to_disabled_and_launch_policy_round_trips() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");

    let uncataloged = read_flow_launch_policy(&config_dir, "uncataloged.yaml").expect("policy");
    assert_eq!(uncataloged.launch_policy, None);
    assert_eq!(uncataloged.effective_launch_policy, LAUNCH_POLICY_DISABLED);

    let saved = set_flow_launch_policy(
        &config_dir,
        "agent-visible.yaml",
        LAUNCH_POLICY_AGENT_REQUESTABLE,
    )
    .expect("save");
    assert_eq!(
        saved.launch_policy.as_deref(),
        Some(LAUNCH_POLICY_AGENT_REQUESTABLE)
    );
    assert_eq!(
        saved.effective_launch_policy,
        LAUNCH_POLICY_AGENT_REQUESTABLE
    );

    let reloaded = read_flow_launch_policy(&config_dir, "agent-visible.yaml").expect("reload");
    assert_eq!(
        reloaded.launch_policy.as_deref(),
        Some(LAUNCH_POLICY_AGENT_REQUESTABLE)
    );
    assert_eq!(reloaded.execution_lock, None);
    assert_eq!(
        fs::read_to_string(config_dir.join("flow-catalog.toml")).expect("catalog"),
        "[flows.\"agent-visible.yaml\"]\nlaunch_policy = \"agent_requestable\"\n"
    );
}

#[test]
fn execution_lock_config_round_trips_through_catalog_toml() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");

    let saved = set_flow_catalog_entry(
        &config_dir,
        "locked.yaml",
        LAUNCH_POLICY_DISABLED,
        Some(FlowExecutionLockConfig {
            scope: "project".to_string(),
            key: "main-worktree-integration".to_string(),
            conflict_policy: "queue".to_string(),
        }),
    )
    .expect("save lock");
    assert_eq!(saved.launch_policy.as_deref(), Some(LAUNCH_POLICY_DISABLED));

    let reloaded = read_flow_launch_policy(&config_dir, "locked.yaml").expect("reload lock");
    assert_eq!(
        reloaded.execution_lock,
        Some(FlowExecutionLockConfig {
            scope: "project".to_string(),
            key: "main-worktree-integration".to_string(),
            conflict_policy: "queue".to_string(),
        })
    );
    let catalog = fs::read_to_string(config_dir.join("flow-catalog.toml")).expect("catalog");
    assert!(catalog.contains("[flows.\"locked.yaml\".execution_lock]"));
    assert!(catalog.contains("scope = \"project\""));
    assert!(catalog.contains("key = \"main-worktree-integration\""));
    assert!(catalog.contains("conflict_policy = \"queue\""));
}

#[test]
fn invalid_catalog_values_return_current_validation_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    fs::create_dir_all(&config_dir).expect("config");
    let catalog_path = config_dir.join("flow-catalog.toml");

    fs::write(
        &catalog_path,
        "[flows.\"bad.yaml\"]\nlaunch_policy = \"requestable\"\n",
    )
    .expect("catalog");
    assert_eq!(
        error_reason(load_flow_catalog(&config_dir).expect_err("invalid policy")),
        "Launch policy must be one of: agent_requestable, disabled, trigger_only"
    );

    fs::write(
        &catalog_path,
        "[flows.\"bad.yaml\"]\nlaunch_policy = \"disabled\"\n\n[flows.\"bad.yaml\".execution_lock]\nscope = \"workspace\"\nkey = \"repo\"\nconflict_policy = \"queue\"\n",
    )
    .expect("catalog");
    assert!(
        error_reason(load_flow_catalog(&config_dir).expect_err("invalid scope"))
            .contains("Execution lock scope must be one of: project")
    );

    fs::write(
        &catalog_path,
        "[flows.\"bad.yaml\"]\nlaunch_policy = \"disabled\"\n\n[flows.\"bad.yaml\".execution_lock]\nscope = \"project\"\nconflict_policy = \"queue\"\n",
    )
    .expect("catalog");
    assert!(
        error_reason(load_flow_catalog(&config_dir).expect_err("missing key"))
            .contains("Execution lock key is required.")
    );

    fs::write(
        &catalog_path,
        "[flows.\"bad.yaml\"]\nlaunch_policy = \"disabled\"\n\n[flows.\"bad.yaml\".execution_lock]\nscope = \"project\"\nkey = \"repo\"\nconflict_policy = \"reject\"\n",
    )
    .expect("catalog");
    assert!(
        error_reason(load_flow_catalog(&config_dir).expect_err("invalid conflict"))
            .contains("Execution lock conflict policy must be one of: queue")
    );
}

#[test]
fn nested_flow_names_are_normalized_and_path_safety_is_enforced() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");

    let saved = set_flow_launch_policy(
        &config_dir,
        "ops\\review/nested",
        LAUNCH_POLICY_AGENT_REQUESTABLE,
    )
    .expect("save nested");
    assert_eq!(saved.name, "ops/review/nested.yaml");
    assert!(fs::read_to_string(config_dir.join("flow-catalog.toml"))
        .expect("catalog")
        .contains("[flows.\"ops/review/nested.yaml\"]"));

    assert_eq!(
        error_reason(
            read_flow_launch_policy(&config_dir, "/tmp/escape.yaml").expect_err("absolute")
        ),
        "Flow name must be a relative path inside flows_dir."
    );
    assert_eq!(
        error_reason(read_flow_launch_policy(&config_dir, "../escape.yaml").expect_err("parent")),
        "Flow name must be a relative path inside flows_dir."
    );
    assert_eq!(
        error_reason(read_flow_launch_policy(&config_dir, "nested/").expect_err("directory")),
        "Flow name must reference a file."
    );
}

fn error_reason(error: StorageError) -> String {
    match error {
        StorageError::InvalidRepositoryPath { reason, .. } => reason,
        other => other.to_string(),
    }
}

#[test]
fn legacy_dot_catalog_entries_fail_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("flow-catalog.toml"),
        "[flows.\"software-development/implement-change-request.dot\"]\nlaunch_policy = \"agent_requestable\"\n",
    )
    .expect("write legacy catalog");

    let error = load_flow_catalog(&config_dir).expect_err("legacy .dot entry is rejected");
    assert!(matches!(
        &error,
        StorageError::InvalidRepositoryPath { path, reason }
            if path.ends_with("implement-change-request.dot")
                && reason == "Flow name must end with .yaml or .yml."
    ));
}

#[test]
fn default_catalog_seed_registers_default_flows_and_locks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("flow-catalog.toml"),
        "[flows.\"software-development/spec-implementation/implement-spec.yaml\"]\nlaunch_policy = \"disabled\"\n",
    )
    .expect("write catalog");

    let missing = spark_storage::seed_default_flow_catalog(&config_dir).expect("seed");
    assert_eq!(
        missing,
        vec![
            "math-research/explore-conjecture.yaml".to_string(),
            "math-research/formalize-result.yaml".to_string(),
            "math-research/prove-refute.yaml".to_string(),
            "software-development/audit-codebase.yaml".to_string(),
            "software-development/design-change.yaml".to_string(),
            "software-development/implement-change.yaml".to_string(),
            "software-development/integrate-ready-branches.yaml".to_string(),
            "software-development/investigate-bug.yaml".to_string(),
            "software-development/merge-change.yaml".to_string(),
            "software-development/review-change.yaml".to_string(),
            "software-development/run-retrospective.yaml".to_string(),
        ]
    );
    let merge = read_flow_launch_policy(&config_dir, "software-development/merge-change.yaml")
        .expect("merge policy");
    assert_eq!(
        merge.execution_lock,
        Some(FlowExecutionLockConfig {
            scope: "project".to_string(),
            key: "software-development-integration".to_string(),
            conflict_policy: "queue".to_string(),
        })
    );
    for flow_name in [
        "math-research/explore-conjecture.yaml",
        "math-research/formalize-result.yaml",
        "math-research/prove-refute.yaml",
    ] {
        assert_eq!(
            read_flow_launch_policy(&config_dir, flow_name)
                .expect("math policy")
                .execution_lock,
            Some(FlowExecutionLockConfig {
                scope: "project".to_string(),
                key: "math-research".to_string(),
                conflict_policy: "queue".to_string(),
            })
        );
    }
}

#[test]
fn revision_checked_catalog_edits_preserve_extensions_and_reject_stale_writes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("flow-catalog.toml");
    fs::write(&path, "schema_version = 1\n[custom]\nretained = true\n[flows.\"one.yaml\"]\nlaunch_policy = \"disabled\"\nnote = \"keep\"\n[flows.\"two.yaml\"]\nlaunch_policy = \"trigger_only\"\n").unwrap();
    let before = read_flow_launch_policy(temp.path(), "one.yaml").unwrap();
    let saved = spark_storage::set_flow_catalog_entry_revision(
        temp.path(),
        "one.yaml",
        "agent_requestable",
        None,
        &before.revision,
    )
    .unwrap();
    assert_ne!(before.revision, saved.revision);
    let bytes = fs::read(&path).unwrap();
    let document = spark_storage::settings::read_settings_document(&path).unwrap();
    assert_eq!(document.values["custom"]["retained"].as_bool(), Some(true));
    assert_eq!(
        document.values["flows"]["one.yaml"]["note"].as_str(),
        Some("keep")
    );
    assert_eq!(
        document.values["flows"]["two.yaml"]["launch_policy"].as_str(),
        Some("trigger_only")
    );
    assert!(matches!(
        spark_storage::set_flow_catalog_entry_revision(
            temp.path(),
            "two.yaml",
            "disabled",
            None,
            &before.revision
        ),
        Err(StorageError::SettingsConflict { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    spark_storage::seed_default_flow_catalog(temp.path()).unwrap();
    let seeded = spark_storage::settings::read_settings_document(&path).unwrap();
    assert_eq!(seeded.values["custom"]["retained"].as_bool(), Some(true));
    assert_eq!(
        seeded.values["flows"]["one.yaml"]["note"].as_str(),
        Some("keep")
    );
}
