use std::fs;
use std::path::Path;

use attractor_runtime::RunStore;
use serde_json::{json, Map, Value};
use spark_common::project::run_git;
use spark_common::settings::SparkSettings;
use spark_triggers::{state, TriggerCreateRequest, TriggerService, WebhookHandleRequest};
use spark_workspace::WorkspaceTriggerService;
use time::OffsetDateTime;

#[tokio::test]
async fn workspace_source_activation_records_missing_flow_failures_without_launching_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Missing target".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/missing")),
                ("static_context".to_string(), json!({"origin": "workspace"})),
            ]),
            source: Map::from_iter([
                ("kind".to_string(), json!("once")),
                ("run_at".to_string(), json!("2026-06-24T09:00:00Z")),
            ]),
        })
        .expect("create trigger");

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .process_due_trigger_sources_at(at("2026-06-24T10:00:00Z"))
        .await
        .expect("process source");

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].trigger_id, created.id);
    assert_eq!(outcomes[0].status, "failed");
    assert_eq!(outcomes[0].message, "Unknown flow: ops/missing");
    assert!(outcomes[0].run_id.is_none());
    let state = load_state(&settings, &project, &created.id);
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(
        state.last_error.as_deref(),
        Some("Unknown flow: ops/missing")
    );
    assert_eq!(state.recent_history.len(), 1);
}

#[tokio::test]
async fn workspace_source_activation_accepts_existing_flow_and_preserves_action_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: dispatched run records hold canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_flow(&settings, "ops/run");
    let project = project(&settings, "project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Existing target".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/run")),
                ("static_context".to_string(), json!({"origin": "workspace"})),
            ]),
            source: Map::from_iter([
                ("kind".to_string(), json!("once")),
                ("run_at".to_string(), json!("2026-06-24T09:00:00Z")),
            ]),
        })
        .expect("create trigger");

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .process_due_trigger_sources_at(at("2026-06-24T10:00:00Z"))
        .await
        .expect("process source");

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, "success");
    let run_id = outcomes[0].run_id.as_deref().expect("trigger run id");
    assert_eq!(outcomes[0].trigger.action.flow_name, "ops/run");
    assert_eq!(
        outcomes[0].trigger.action.static_context,
        Map::from_iter([("origin".to_string(), json!("workspace"))])
    );
    assert_eq!(
        load_state(&settings, &project, &created.id)
            .last_result
            .as_deref(),
        Some("success")
    );
    let run = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read trigger run")
        .expect("trigger run");
    let record = run.record.expect("run record");
    // A static trigger launches in its own project.
    assert_eq!(record.flow_name, "ops/run");
    assert_eq!(record.project_path, project);
    assert_eq!(record.working_directory, project);
    let context = run.checkpoint.expect("checkpoint").context;
    assert_eq!(
        context["context.trigger_static"],
        json!({"origin": "workspace"})
    );
    assert_eq!(
        context["context.trigger_payload"],
        json!({"scheduled_at": "2026-06-24T09:00:00Z"})
    );
    assert_eq!(
        context["context.spark_trigger"],
        json!({
            "trigger_id": created.id,
            "trigger_name": "Existing target",
            "source_type": "schedule"
        })
    );
}

#[test]
fn workspace_webhook_dispatch_records_success_and_duplicate_request_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/webhook");
    let project = project(&settings, "project");
    let service = WorkspaceTriggerService::new(settings.clone());
    let created = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Webhook launch".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/webhook")),
                ("static_context".to_string(), json!({"origin": "webhook"})),
            ]),
            source: Map::new(),
        })
        .expect("create webhook trigger");
    let webhook_key = created.source["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let webhook_secret = created.webhook_secret.expect("webhook secret");

    let first = service
        .dispatch_webhook(webhook_request(
            &webhook_key,
            &webhook_secret,
            Some("duplicate-request"),
            json!({"payload": "first"}),
        ))
        .expect("first webhook");
    let second = service
        .dispatch_webhook(webhook_request(
            &webhook_key,
            &webhook_secret,
            Some("duplicate-request"),
            json!({"payload": "second"}),
        ))
        .expect("second webhook");

    assert_eq!(first.response.trigger_id, created.id);
    assert_eq!(first.activation.status, "success");
    assert_eq!(second.activation.status, "success");
    let first_run_id = first.activation.run_id.as_deref().expect("first run id");
    let second_run_id = second.activation.run_id.as_deref().expect("second run id");
    assert_ne!(first_run_id, second_run_id);

    let state = load_state(&settings, &project, &created.id);
    assert_eq!(state.last_result.as_deref(), Some("success"));
    assert_eq!(state.recent_history.len(), 2);
    assert_eq!(
        state.recent_history[0].run_id.as_deref(),
        Some(second_run_id)
    );
    assert_eq!(
        state.recent_history[1].run_id.as_deref(),
        Some(first_run_id)
    );

    let first_run = RunStore::for_settings(&settings)
        .read_run_bundle(first_run_id)
        .expect("read first run")
        .expect("first run");
    let context = first_run.checkpoint.expect("first checkpoint").context;
    assert_eq!(
        context["context.trigger_static"],
        json!({"origin": "webhook"})
    );
    assert_eq!(
        context["context.trigger_payload"],
        json!({"payload": "first"})
    );
    assert_eq!(
        context["context.spark_trigger"],
        json!({
            "trigger_id": created.id,
            "trigger_name": "Webhook launch",
            "source_type": "webhook"
        })
    );
}

#[test]
fn workspace_webhook_launch_failures_are_state_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/invalid");
    let project = project(&settings, "project");
    let service = WorkspaceTriggerService::new(settings.clone());
    let failing = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Invalid flow".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([("flow_name".to_string(), json!("ops/invalid"))]),
            source: Map::new(),
        })
        .expect("create invalid webhook");
    write_invalid_flow(&settings, "ops/invalid");
    let failing_key = failing.source["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let failing_secret = failing.webhook_secret.expect("webhook secret");
    let failed = service
        .dispatch_webhook(webhook_request(
            &failing_key,
            &failing_secret,
            None,
            json!({"payload": "bad-flow"}),
        ))
        .expect("accepted failed dispatch");
    assert!(failed.response.ok);
    assert_eq!(failed.activation.status, "failed");
    assert!(failed.activation.run_id.is_none());
    let state = load_state(&settings, &project, &failing.id);
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(
        state.last_error.as_deref(),
        Some(failed.activation.message.as_str())
    );
    assert!(!state.last_error.as_deref().unwrap_or("").is_empty());
}

#[test]
fn triggers_are_created_only_in_registered_projects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run");
    let unregistered = temp.path().join("unregistered");
    fs::create_dir_all(&unregistered).expect("project");
    let error = WorkspaceTriggerService::new(settings)
        .create_trigger(TriggerCreateRequest {
            project_path: unregistered.to_string_lossy().into_owned(),
            message: None,
            name: "Nowhere".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([("flow_name".to_string(), json!("ops/run"))]),
            source: Map::new(),
        })
        .expect_err("unregistered project");
    assert_eq!(
        error.detail(),
        "Triggers are created in a registered project."
    );
    assert!(!unregistered.join(".spark").exists());
}

#[test]
fn flow_event_trigger_keeps_static_context_nested() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_flow(&settings, "ops/run");
    let project_path = project(&settings, "project");
    let trigger = flow_event_definition(
        "flow-event",
        Map::from_iter([("context.request.problem".to_string(), json!("static P"))]),
    );
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions(&project_path)
        .unwrap()
        .put(&trigger, None)
        .expect("put trigger");

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .emit_flow_event(Map::from_iter([
            ("flow_name".to_string(), json!("ops/run")),
            ("project_path".to_string(), json!(project_path)),
            ("status".to_string(), json!("completed")),
        ]))
        .expect("emit event");

    let run = RunStore::for_settings(&settings)
        .read_run_bundle(outcomes[0].run_id.as_deref().expect("run"))
        .expect("read run")
        .expect("run");
    let context = run.checkpoint.expect("checkpoint").context;
    assert_eq!(
        context["context.trigger_static"],
        json!({"context.request.problem": "static P"})
    );
    assert!(!context.contains_key("context.request.problem"));
}

#[test]
fn flow_event_triggers_ignore_other_projects_runs_and_role_triggers_skip_the_roles_own_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let other = project(&settings, "other");
    let project = project(&settings, "project");
    let mut trigger = flow_event_definition("role-event", Map::new());
    trigger.action.mode = "role".into();
    trigger.action.role = Some("coordinator".into());
    trigger.action.flow_name = String::new();
    trigger.source = Map::from_iter([("statuses".to_string(), json!(["failed"]))]);
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions(&project)
        .unwrap()
        .put(&trigger, None)
        .expect("put trigger");
    let mut own = attractor_core::RunRecord::new("run-own", &project);
    own.launch_context = Some(
        [(
            "context.spark_owner".to_string(),
            json!({"kind": "role", "id": "coordinator"}),
        )]
        .into(),
    );
    RunStore::for_settings(&settings)
        .create_run(attractor_runtime::CreateRunRequest {
            record: own,
            ..Default::default()
        })
        .expect("own run");
    let service = WorkspaceTriggerService::new(settings.clone());
    let emit = |run_id: &str, project: &str| {
        service
            .emit_flow_event(Map::from_iter([
                ("run_id".to_string(), json!(run_id)),
                ("flow_name".to_string(), json!("ops/run")),
                ("project_path".to_string(), json!(project)),
                ("status".to_string(), json!("failed")),
            ]))
            .expect("emit event")
            .into_iter()
            .map(|outcome| outcome.message)
            .collect::<Vec<_>>()
    };
    // A matching run in another registered project does not fire it at all.
    assert!(emit("run-elsewhere", &other).is_empty());
    assert!(emit("run-own", &project)[0].contains("launched by this role"));
    assert_eq!(
        emit("run-other", &project),
        vec!["Trigger delivered to role."]
    );
    let role = spark_workspace::roles::WorkspaceRoleService::new(settings.clone())
        .get(&project, "coordinator")
        .expect("role");
    assert_eq!(role.inbox.event_seq, 1);
}

#[test]
fn hand_deleted_definitions_stop_firing_without_a_restart() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_flow(&settings, "ops/run");
    let project = project(&settings, "project");
    let definitions = spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions(&project)
        .unwrap();
    definitions
        .put(&flow_event_definition("flow-event", Map::new()), None)
        .expect("put trigger");
    let service = WorkspaceTriggerService::new(settings.clone());
    let emit = || {
        service
            .emit_flow_event(Map::from_iter([
                ("flow_name".to_string(), json!("ops/run")),
                ("project_path".to_string(), json!(project)),
                ("status".to_string(), json!("completed")),
            ]))
            .expect("emit event")
            .len()
    };
    assert_eq!(emit(), 1);
    let org = spark_common::project::org_dir(Path::new(&project));
    run_git(&org, &["rm", "-q", "triggers/flow-event.toml"]).expect("git rm");
    run_git(&org, &["commit", "-qm", "Remove the trigger"]).expect("commit");
    assert_eq!(emit(), 0);
}

fn write_flow(settings: &SparkSettings, name: &str) {
    crate::install_named_flow(
        settings,
        name,
        json!({
            "schema_version": "1",
            "id": "workspace-trigger",
            "nodes": {"start": {"kind": "start"}, "done": {"kind": "exit"}},
            "edges": [{"from": "start", "to": "done"}]
        }),
    );
}

fn flow_event_definition(
    id: &str,
    static_context: Map<String, Value>,
) -> spark_storage::TriggerDefinition {
    spark_storage::TriggerDefinition {
        revision: String::new(),
        project_path: String::new(),
        id: id.to_string(),
        name: id.to_string(),
        protected: false,
        source_type: "flow_event".to_string(),
        action: spark_storage::TriggerAction {
            mission_id: None,
            role: None,
            mode: "static".to_string(),
            flow_name: "ops/run".to_string(),
            static_context,
            execution_profile_id: None,
        },
        source: Map::from_iter([
            ("flow_name".to_string(), json!("ops/run")),
            ("statuses".to_string(), json!(["completed"])),
        ]),
        created_at: "2026-06-24T08:00:00Z".to_string(),
        updated_at: "2026-06-24T08:00:00Z".to_string(),
    }
}

fn write_invalid_flow(settings: &SparkSettings, name: &str) {
    let path = settings.bundled_dir().join("flows").join(name);
    fs::create_dir_all(&path).expect("flow dir");
    fs::write(
        path.join("flow.md"),
        "---\ntype: Flow\nid: broken\nnodes: [\n---\n",
    )
    .expect("invalid flow");
}

fn webhook_request(
    key: &str,
    secret: &str,
    request_id: Option<&str>,
    payload: Value,
) -> WebhookHandleRequest {
    WebhookHandleRequest {
        webhook_key: key.to_string(),
        webhook_secret: secret.to_string(),
        request_id: request_id.map(str::to_string),
        payload: payload.as_object().cloned().unwrap_or_default(),
    }
}

fn at(value: &str) -> OffsetDateTime {
    state::parse_iso_datetime(Some(value)).expect("timestamp")
}

fn load_state(settings: &SparkSettings, project: &str, id: &str) -> spark_storage::TriggerState {
    spark_storage::TriggerRepositories::from_settings(settings)
        .runtime_state(project)
        .unwrap()
        .load(id)
        .expect("load trigger state")
}

/// A registered project whose org repo has a git identity, since every
/// definition write commits.
fn project(settings: &SparkSettings, name: &str) -> String {
    let project = settings.data_dir.parent().unwrap().join(name);
    fs::create_dir_all(&project).expect("project dir");
    let org = spark_common::project::ensure_org_repo(&project).expect("org repo");
    for (key, value) in [
        ("user.name", "Trigger Test"),
        ("user.email", "trigger@example.com"),
    ] {
        run_git(&org, &["config", key, value]).expect("git config");
    }
    spark_storage::ProjectRegistry::new(settings.data_dir.clone())
        .ensure_project_paths(&project.to_string_lossy())
        .expect("register project")
        .project_path
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
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
