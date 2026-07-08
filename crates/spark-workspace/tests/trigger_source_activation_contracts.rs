use std::fs;
use std::path::Path;

use attractor_runtime::RunStore;
use serde_json::{json, Map, Value};
use spark_common::settings::SparkSettings;
use spark_triggers::{state, TriggerCreateRequest, TriggerService, WebhookHandleRequest};
use spark_workspace::WorkspaceTriggerService;
use time::OffsetDateTime;

#[tokio::test]
async fn workspace_source_activation_records_missing_flow_failures_without_launching_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            name: "Missing target".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/missing.dot")),
                (
                    "project_path".to_string(),
                    json!(temp.path().join("project")),
                ),
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
    assert_eq!(outcomes[0].message, "Unknown flow: ops/missing.dot");
    assert!(outcomes[0].run_id.is_none());
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load trigger state");
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(
        state.last_error.as_deref(),
        Some("Unknown flow: ops/missing.dot")
    );
    assert_eq!(state.recent_history.len(), 1);
}

#[tokio::test]
async fn workspace_source_activation_accepts_existing_flow_and_preserves_action_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: dispatched run records hold canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_flow(&settings, "ops/run.dot");
    let project_path = root.join("project");
    fs::create_dir_all(&project_path).expect("project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            name: "Existing target".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/run.dot")),
                ("project_path".to_string(), json!(project_path)),
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
    assert_eq!(outcomes[0].trigger.action.flow_name, "ops/run.dot");
    assert_eq!(
        outcomes[0].trigger.action.static_context,
        Map::from_iter([("origin".to_string(), json!("workspace"))])
    );
    assert_eq!(
        spark_storage::load_trigger_state(&settings.data_dir, &created.id)
            .expect("load trigger state")
            .last_result
            .as_deref(),
        Some("success")
    );
    let run = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read trigger run")
        .expect("trigger run");
    let record = run.record.expect("run record");
    assert_eq!(record.flow_name, "ops/run.dot");
    assert_eq!(record.project_path, project_path.to_string_lossy());
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
    write_flow(&settings, "ops/webhook.dot");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let service = WorkspaceTriggerService::new(settings.clone());
    let created = service
        .create_trigger(TriggerCreateRequest {
            name: "Webhook launch".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/webhook.dot")),
                ("project_path".to_string(), json!(project_path)),
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

    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load trigger state");
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
fn workspace_webhook_without_project_uses_spark_home_and_launch_failures_are_state_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/no-project.dot");
    write_invalid_flow(&settings, "ops/invalid.dot");
    let service = WorkspaceTriggerService::new(settings.clone());
    let no_project = service
        .create_trigger(TriggerCreateRequest {
            name: "No project".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/no-project.dot")),
                ("static_context".to_string(), json!({"origin": "fallback"})),
            ]),
            source: Map::new(),
        })
        .expect("create no-project webhook");
    let no_project_key = no_project.source["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let no_project_secret = no_project.webhook_secret.expect("webhook secret");
    let accepted = service
        .dispatch_webhook(webhook_request(
            &no_project_key,
            &no_project_secret,
            None,
            json!({"payload": "fallback"}),
        ))
        .expect("dispatch no-project webhook");
    let run_id = accepted.activation.run_id.as_deref().expect("run id");
    let record = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read fallback run")
        .expect("fallback run")
        .record
        .expect("record");
    assert_eq!(record.project_path, settings.data_dir.to_string_lossy());

    let failing = service
        .create_trigger(TriggerCreateRequest {
            name: "Invalid flow".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([("flow_name".to_string(), json!("ops/invalid.dot"))]),
            source: Map::new(),
        })
        .expect("create invalid webhook");
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
    let state = spark_storage::load_trigger_state(&settings.data_dir, &failing.id)
        .expect("load failed trigger state");
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(
        state.last_error.as_deref(),
        Some(failed.activation.message.as_str())
    );
    assert!(!state.last_error.as_deref().unwrap_or("").is_empty());
}

fn write_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(
        path,
        "digraph WorkspaceTrigger { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }\n",
    )
    .expect("flow");
}

fn write_invalid_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(path, "digraph Broken { start -> }").expect("invalid flow");
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
        project_roots: Vec::new(),
    }
}
