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
fn legacy_dot_catalog_entries_are_skipped_instead_of_failing_load() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    fs::create_dir_all(&config_dir).expect("config dir");
    // A catalog written before the YAML cutover: startup must not abort on it.
    fs::write(
        config_dir.join("flow-catalog.toml"),
        concat!(
            "[flows.\"software-development/implement-change-request.dot\"]\n",
            "launch_policy = \"agent_requestable\"\n",
            "[flows.\"software-development/spec-implementation/implement-spec.yaml\"]\n",
            "launch_policy = \"disabled\"\n",
        ),
    )
    .expect("write legacy catalog");

    let catalog = load_flow_catalog(&config_dir).expect("legacy catalog loads");
    assert_eq!(catalog.len(), 1, "legacy .dot entry is skipped");
    assert_eq!(
        catalog
            .get("software-development/spec-implementation/implement-spec.yaml")
            .expect("yaml entry survives")
            .launch_policy
            .as_deref(),
        Some(LAUNCH_POLICY_DISABLED)
    );

    // Seeding after the skip re-registers the default flows under their
    // .yaml names and the rewritten catalog drops the legacy keys.
    let missing = spark_storage::seed_default_flow_catalog(&config_dir).expect("seed");
    assert_eq!(
        missing,
        vec![
            "software-development/implement-change.yaml".to_string(),
            "software-development/investigate-bug.yaml".to_string(),
            "software-development/merge-change.yaml".to_string(),
            "software-development/review-change.yaml".to_string(),
        ]
    );
    let rewritten = fs::read_to_string(config_dir.join("flow-catalog.toml")).expect("catalog");
    assert!(!rewritten.contains(".dot"));
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
}
