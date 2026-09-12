use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier, Mutex};

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
                ("flow_name".to_string(), json!("ops/missing.yaml")),
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
    assert_eq!(outcomes[0].message, "Unknown flow: ops/missing.yaml");
    assert!(outcomes[0].run_id.is_none());
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load trigger state");
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(
        state.last_error.as_deref(),
        Some("Unknown flow: ops/missing.yaml")
    );
    assert_eq!(state.recent_history.len(), 1);
}

#[tokio::test]
async fn workspace_source_activation_accepts_existing_flow_and_preserves_action_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: dispatched run records hold canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_flow(&settings, "ops/run.yaml");
    let project_path = root.join("project");
    fs::create_dir_all(&project_path).expect("project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            name: "Existing target".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/run.yaml")),
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
    assert_eq!(outcomes[0].trigger.action.flow_name, "ops/run.yaml");
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
    assert_eq!(record.flow_name, "ops/run.yaml");
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

#[tokio::test]
async fn workspace_draft_flow_event_launches_draft_once_with_profile() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    fs::create_dir_all(&settings.config_dir).expect("config");
    fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        "[profiles.math-lab]\nlabel = \"Math Lab\"\nmode = \"native\"\n",
    )
    .expect("profiles");
    write_typed_math_flow(&settings, "math-research/explore-conjecture.yaml");
    let project_path = root.join("project");
    fs::create_dir_all(project_path.join(".mathlab")).expect("mathlab");
    fs::write(
        project_path.join(".mathlab/next-session.json"),
        json!({
            "status": "continue",
            "flow": "math-research/explore-conjecture.yaml",
            "inputs": {"context.request.problem": "P"},
            "rationale": "next"
        })
        .to_string(),
    )
    .expect("draft");
    let created = WorkspaceTriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            name: "Chain".to_string(),
            enabled: true,
            source_type: "flow_event".to_string(),
            action: Map::from_iter([
                ("mode".to_string(), json!("workspace_draft")),
                ("project_path".to_string(), json!(project_path)),
                ("flow_allowlist".to_string(), json!(["math-research/*"])),
                ("execution_profile_id".to_string(), json!("math-lab")),
            ]),
            source: Map::from_iter([
                (
                    "flow_name".to_string(),
                    json!("math-research/explore-conjecture.yaml"),
                ),
                ("statuses".to_string(), json!(["completed"])),
            ]),
        })
        .expect("create draft trigger");

    let service = WorkspaceTriggerService::new(settings.clone());
    let first = service
        .emit_flow_event(Map::from_iter([
            (
                "flow_name".to_string(),
                json!("math-research/explore-conjecture.yaml"),
            ),
            ("project_path".to_string(), json!(project_path)),
            ("status".to_string(), json!("completed")),
        ]))
        .expect("first event");
    let second = service
        .emit_flow_event(Map::from_iter([
            (
                "flow_name".to_string(),
                json!("math-research/explore-conjecture.yaml"),
            ),
            ("project_path".to_string(), json!(project_path)),
            ("status".to_string(), json!("completed")),
        ]))
        .expect("second event");

    assert_eq!(first.len(), 1);
    assert_eq!(first[0].status, "success");
    let run_id = first[0]
        .run_id
        .as_deref()
        .unwrap_or_else(|| panic!("run id: {}", first[0].message));
    assert!(project_path
        .join(".mathlab/next-session.launched.json")
        .exists());
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].status, "success");
    assert!(second[0].run_id.is_none());
    let run = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read run")
        .expect("run");
    let record = run.record.expect("record");
    assert_eq!(record.flow_name, "math-research/explore-conjecture.yaml");
    assert_eq!(record.execution_profile_id.as_deref(), Some("math-lab"));
    let context = run.checkpoint.expect("checkpoint").context;
    assert_eq!(context["context.request.problem"], json!("P"));
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load trigger state");
    assert_eq!(
        state.last_error.as_deref(),
        Some("Next-session launch skipped: no next-session.json was available.")
    );
}

#[test]
fn workspace_webhook_dispatch_records_success_and_duplicate_request_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/webhook.yaml");
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project");
    let service = WorkspaceTriggerService::new(settings.clone());
    let created = service
        .create_trigger(TriggerCreateRequest {
            name: "Webhook launch".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/webhook.yaml")),
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
    write_flow(&settings, "ops/no-project.yaml");
    write_flow(&settings, "ops/invalid.yaml");
    let service = WorkspaceTriggerService::new(settings.clone());
    let no_project = service
        .create_trigger(TriggerCreateRequest {
            name: "No project".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/no-project.yaml")),
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
            action: Map::from_iter([("flow_name".to_string(), json!("ops/invalid.yaml"))]),
            source: Map::new(),
        })
        .expect("create invalid webhook");
    write_invalid_flow(&settings, "ops/invalid.yaml");
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

#[test]
fn math_flow_event_claims_valid_draft_once_and_invalid_drafts_remain_unconsumed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_typed_math_flow(&settings, "math-research/prove-refute.yaml");
    let project_path = root.join("project");
    let mathlab = project_path.join(".mathlab");
    fs::create_dir_all(&mathlab).expect("mathlab");
    let service = WorkspaceTriggerService::new(settings.clone());
    let trigger = flow_event_definition("math-continuation-chain", &project_path, Map::new());
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions
        .put(&trigger)
        .expect("put chain trigger");
    let active = mathlab.join("next-session.json");
    fs::write(
        &active,
        serde_json::to_vec(&json!({
            "flow_name": "math-research/prove-refute.yaml",
            "inputs": {"context.request.problem": "P", "context.request.tags": ["one"]}
        }))
        .expect("draft json"),
    )
    .expect("draft");
    let payload = Map::from_iter([
        (
            "flow_name".to_string(),
            json!("math-research/prove-refute.yaml"),
        ),
        ("project_path".to_string(), json!(project_path)),
        ("status".to_string(), json!("completed")),
    ]);

    let first = service
        .emit_flow_event(payload.clone())
        .expect("first event");
    let duplicate = service
        .emit_flow_event(payload.clone())
        .expect("duplicate event");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].status, "success", "{}", first[0].message);
    assert!(first[0].run_id.is_some());
    assert_eq!(duplicate.len(), 1);
    assert!(duplicate[0].run_id.is_none());
    assert!(mathlab.join("next-session.launched.json").is_file());
    assert!(!active.exists());

    fs::write(
        &active,
        serde_json::to_vec(&json!({
            "flow_name": "math-research/prove-refute.yaml",
            "inputs": {"context.request.problem": 3}
        }))
        .expect("invalid draft json"),
    )
    .expect("invalid draft");
    let invalid = service.emit_flow_event(payload).expect("invalid event");
    assert_eq!(invalid[0].trigger_id, trigger.id);
    assert!(invalid[0].run_id.is_none());
    assert!(invalid[0].message.contains("must be string"));
    assert!(active.is_file());
    assert!(!mathlab.join("next-session.launching.json").exists());
}

#[test]
fn math_chain_terminal_events_activate_only_the_trigger_for_their_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_typed_math_flow(&settings, "math-research/prove-refute.yaml");
    let projects = [root.join("tuza"), root.join("bsd")];
    let repository = spark_storage::TriggerRepositories::from_settings(&settings);
    for (id, project) in ["tuza-chain", "bsd-chain"].into_iter().zip(&projects) {
        let mathlab = project.join(".mathlab");
        fs::create_dir_all(&mathlab).expect("mathlab");
        write_valid_draft(&mathlab.join("next-session.json"));
        repository
            .definitions
            .put(&flow_event_definition(id, project, Map::new()))
            .expect("put chain trigger");
    }
    let service = WorkspaceTriggerService::new(settings.clone());

    for (index, project) in projects.iter().enumerate() {
        let outcomes = service
            .emit_flow_event(terminal_payload(project))
            .expect("terminal event");
        assert_eq!(outcomes.len(), 2);
        let own_id = if index == 0 {
            "tuza-chain"
        } else {
            "bsd-chain"
        };
        let own = outcomes
            .iter()
            .find(|outcome| outcome.trigger_id == own_id)
            .expect("own trigger outcome");
        assert!(own.run_id.is_some(), "{own_id} must consume its own draft");
        let other = outcomes
            .iter()
            .find(|outcome| outcome.trigger_id != own_id)
            .expect("other trigger outcome");
        assert!(other.run_id.is_none());
        assert!(other.message.contains("belongs to another project"));
        assert!(project
            .join(".mathlab/next-session.launched.json")
            .is_file());
        if let Some(next_project) = projects.get(index + 1) {
            assert!(
                next_project.join(".mathlab/next-session.json").is_file(),
                "a mismatched trigger must not consume another project's draft"
            );
        }
    }
    assert_eq!(
        RunStore::for_settings(&settings)
            .list_run_records()
            .expect("runs")
            .len(),
        2
    );
}

#[test]
fn math_flow_event_rejects_installed_non_catalog_successor_without_claiming_or_running() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_typed_math_flow(&settings, "math-research/prove-refute.yaml");
    write_typed_math_flow(&settings, "approval-gated/arbitrary.yaml");
    let project_path = root.join("project");
    let mathlab = project_path.join(".mathlab");
    fs::create_dir_all(&mathlab).expect("mathlab");
    let trigger = flow_event_definition("math-authorization-chain", &project_path, Map::new());
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions
        .put(&trigger)
        .expect("put chain trigger");
    let active = mathlab.join("next-session.json");
    fs::write(
        &active,
        serde_json::to_vec(&json!({
            "flow_name": "approval-gated/arbitrary.yaml",
            "inputs": {"context.request.problem": "P"}
        }))
        .expect("draft json"),
    )
    .expect("draft");

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .emit_flow_event(terminal_payload(&project_path))
        .expect("event");

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].run_id.is_none());
    assert!(outcomes[0].message.contains("not allowlisted"));
    assert!(
        active.is_file(),
        "authorization failure must not consume draft"
    );
    assert!(!mathlab.join("next-session.launching.json").exists());
    assert!(
        attractor_runtime::RunStore::for_settings(&settings)
            .list_run_records()
            .expect("runs")
            .is_empty(),
        "authorization failure must not create a run"
    );
}

#[test]
fn successful_math_successor_records_claim_before_real_preparation_cleans_handoffs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_preparing_math_flow(&settings, "math-research/prove-refute.yaml");
    let project_path = root.join("project");
    let mathlab = project_path.join(".mathlab");
    fs::create_dir_all(&mathlab).expect("mathlab");
    let trigger = flow_event_definition("math-preparation-chain", &project_path, Map::new());
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions
        .put(&trigger)
        .expect("trigger");
    write_valid_draft(&mathlab.join("next-session.json"));

    let observed_claim_state = Arc::new(Mutex::new(None));
    let observed_claim_state_for_event = observed_claim_state.clone();
    let launching_for_event = mathlab.join("next-session.launching.json");
    let launched_for_event = mathlab.join("next-session.launched.json");
    let observer: attractor_runtime::RunEventObserver = Arc::new(move |_| {
        let mut state = observed_claim_state_for_event.lock().expect("claim state");
        if state.is_none() {
            *state = Some((launching_for_event.is_file(), launched_for_event.exists()));
        }
    });
    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .with_run_event_observer(observer)
        .emit_flow_event(terminal_payload(&project_path))
        .expect("event");

    let run_id = outcomes[0].run_id.as_deref().expect("successor run");
    assert_eq!(
        *observed_claim_state.lock().expect("claim state"),
        Some((true, false)),
        "the durable run-preparation boundary must retain the launching claim and must not expose a launched marker"
    );
    let store = RunStore::for_settings(&settings);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let terminal_status = loop {
        let status = store
            .read_run_bundle(run_id)
            .expect("read successor run")
            .and_then(|bundle| bundle.record)
            .map(|record| record.status);
        if matches!(
            status.as_deref(),
            Some("completed" | "failed" | "cancelled")
        ) {
            break status.expect("terminal status");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "successor run {run_id} did not reach a terminal state; last status: {status:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    assert_eq!(terminal_status, "completed");
    assert!(
        mathlab.join("prepared").is_file(),
        "the successor preparation step must execute"
    );
    assert!(
        mathlab.join("next-session.launched.json").is_file(),
        "preparation must not race successful claim recording"
    );
    assert!(!mathlab.join("next-session.draft.json").exists());
    assert!(!mathlab.join("next-session.json").exists());
    assert!(!mathlab.join("next-session.launching.json").exists());
}

#[test]
fn math_chain_launch_failure_restores_claim_without_creating_a_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_typed_math_flow(&settings, "math-research/prove-refute.yaml");
    let project_path = root.join("project");
    let mathlab = project_path.join(".mathlab");
    fs::create_dir_all(&mathlab).expect("mathlab");
    fs::create_dir_all(settings.runs_dir.parent().expect("runs parent")).expect("runs parent");
    fs::write(&settings.runs_dir, "not a directory").expect("block run storage");
    let trigger = flow_event_definition("math-failure-chain", &project_path, Map::new());
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions
        .put(&trigger)
        .expect("trigger");
    let active = mathlab.join("next-session.json");
    write_valid_draft(&active);

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .emit_flow_event(terminal_payload(&project_path))
        .expect("event");

    assert_eq!(outcomes[0].status, "failed");
    assert!(outcomes[0].run_id.is_none());
    assert!(
        active.is_file(),
        "failed launch must restore the active claim"
    );
    assert!(!mathlab.join("next-session.launching.json").exists());
    assert!(fs::read_dir(settings.runs_dir.parent().unwrap())
        .expect("run parent")
        .all(|entry| entry.expect("entry").path() == settings.runs_dir));
}

#[test]
fn concurrent_math_chain_consumers_create_exactly_one_run_and_one_claim_loss_noop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_typed_math_flow(&settings, "math-research/prove-refute.yaml");
    let project_path = root.join("project");
    let mathlab = project_path.join(".mathlab");
    fs::create_dir_all(&mathlab).expect("mathlab");
    let trigger = flow_event_definition("math-race-chain", &project_path, Map::new());
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions
        .put(&trigger)
        .expect("trigger");
    write_valid_draft(&mathlab.join("next-session.json"));
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let settings = settings.clone();
        let project_path = project_path.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            WorkspaceTriggerService::new(settings)
                .emit_flow_event(terminal_payload(&project_path))
                .expect("event")
                .remove(0)
        }));
    }
    barrier.wait();
    let outcomes = threads
        .into_iter()
        .map(|thread| thread.join().expect("consumer"))
        .collect::<Vec<_>>();

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome.run_id.is_some())
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome.run_id.is_none())
            .count(),
        1
    );
    assert!(outcomes
        .iter()
        .find(|outcome| outcome.run_id.is_none())
        .expect("claim loss")
        .message
        .contains("another consumer claimed"));
    assert_eq!(
        fs::read_dir(&settings.runs_dir)
            .expect("runs")
            .filter_map(Result::ok)
            .count(),
        1
    );
}

#[test]
fn ordinary_math_flow_event_uses_static_context_without_next_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_typed_math_flow(&settings, "math-research/prove-refute.yaml");
    let project_path = root.join("project");
    fs::create_dir_all(&project_path).expect("project");
    let trigger = flow_event_definition(
        "ordinary-math-event",
        &project_path,
        Map::from_iter([("context.request.problem".to_string(), json!("static P"))]),
    );
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions
        .put(&trigger)
        .expect("put trigger");

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .emit_flow_event(Map::from_iter([
            (
                "flow_name".to_string(),
                json!("math-research/prove-refute.yaml"),
            ),
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
    assert!(!project_path
        .join(".mathlab/next-session.launched.json")
        .exists());
}

#[tokio::test]
async fn non_math_chain_id_keeps_static_schedule_context_nested() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    write_flow(&settings, "ops/run.yaml");
    let project_path = root.join("project");
    fs::create_dir_all(&project_path).expect("project");
    let definition = spark_storage::TriggerDefinition {
        revision: String::new(),
        id: "ordinary-chain".to_string(),
        name: "Ordinary chain".to_string(),
        enabled: true,
        protected: false,
        source_type: "schedule".to_string(),
        action: spark_storage::TriggerAction {
            mode: "static".to_string(),
            flow_name: "ops/run.yaml".to_string(),
            project_path: Some(project_path.to_string_lossy().into_owned()),
            static_context: Map::from_iter([("origin".to_string(), json!("nested"))]),
            flow_allowlist: Vec::new(),
            execution_profile_id: None,
        },
        source: Map::from_iter([
            ("kind".to_string(), json!("once")),
            ("run_at".to_string(), json!("2026-06-24T09:00:00Z")),
        ]),
        created_at: "2026-06-24T08:00:00Z".to_string(),
        updated_at: "2026-06-24T08:00:00Z".to_string(),
    };
    spark_storage::TriggerRepositories::from_settings(&settings)
        .definitions
        .put(&definition)
        .expect("put trigger");

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .process_due_trigger_sources_at(at("2026-06-24T10:00:00Z"))
        .await
        .expect("process source");
    let run = RunStore::for_settings(&settings)
        .read_run_bundle(outcomes[0].run_id.as_deref().expect("run"))
        .expect("read run")
        .expect("run");
    let context = run.checkpoint.expect("checkpoint").context;
    assert_eq!(
        context["context.trigger_static"],
        json!({"origin": "nested"})
    );
    assert!(!context.contains_key("origin"));
}

fn write_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(
        path,
        "schema_version: '1'\nid: workspace-trigger\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    )
    .expect("flow");
}

fn write_typed_math_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(
        path,
        "schema_version: '1'\nid: math-chain\ninputs:\n  - key: context.request.problem\n    type: string\n    required: true\n  - key: context.request.tags\n    type: string[]\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    )
    .expect("flow");
}

fn write_preparing_math_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(
        path,
        "schema_version: '1'\nid: math-chain\ninputs:\n  - key: context.request.problem\n    type: string\n    required: true\nnodes:\n  start:\n    kind: start\n  prepare:\n    kind: tool\n    config:\n      kind: tool\n      command: |-\n        mkdir -p .mathlab\n        rm -f .mathlab/next-session.draft.json .mathlab/next-session.json .mathlab/next-session.launching.json\n        touch .mathlab/prepared\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: prepare\n  - from: prepare\n    to: done\n",
    )
    .expect("flow");
}

fn write_valid_draft(path: &Path) {
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "flow_name": "math-research/prove-refute.yaml",
            "inputs": {"context.request.problem": "P"}
        }))
        .expect("draft json"),
    )
    .expect("draft");
}

fn terminal_payload(project_path: &Path) -> Map<String, Value> {
    Map::from_iter([
        (
            "flow_name".to_string(),
            json!("math-research/prove-refute.yaml"),
        ),
        ("project_path".to_string(), json!(project_path)),
        ("status".to_string(), json!("completed")),
    ])
}

fn flow_event_definition(
    id: &str,
    project_path: &Path,
    static_context: Map<String, Value>,
) -> spark_storage::TriggerDefinition {
    spark_storage::TriggerDefinition {
        revision: String::new(),
        id: id.to_string(),
        name: id.to_string(),
        enabled: true,
        protected: false,
        source_type: "flow_event".to_string(),
        action: spark_storage::TriggerAction {
            mode: if id.ends_with("-chain") {
                "workspace_draft".to_string()
            } else {
                "static".to_string()
            },
            flow_name: "math-research/prove-refute.yaml".to_string(),
            project_path: Some(project_path.to_string_lossy().into_owned()),
            static_context,
            flow_allowlist: if id.ends_with("-chain") {
                vec!["math-research/*".to_string()]
            } else {
                Vec::new()
            },
            execution_profile_id: None,
        },
        source: Map::from_iter([
            (
                "flow_name".to_string(),
                json!("math-research/prove-refute.yaml"),
            ),
            ("statuses".to_string(), json!(["completed"])),
        ]),
        created_at: "2026-06-24T08:00:00Z".to_string(),
        updated_at: "2026-06-24T08:00:00Z".to_string(),
    }
}

fn write_invalid_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(path, "schema_version: '1'\nid: broken\nnodes: [").expect("invalid flow");
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
        project_roots: Vec::new(),
    }
}

#[tokio::test]
async fn workspace_draft_schedule_activation_launches_without_event_payload() {
    // Regression: schedule/poll activations carry no flow-event payload, so the
    // cross-project event guard must not reject them; they are scoped by the
    // action's own configured project path.
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    fs::create_dir_all(&settings.config_dir).expect("config");
    fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        "[profiles.math-lab]\nlabel = \"Math Lab\"\nmode = \"native\"\n",
    )
    .expect("profiles");
    write_typed_math_flow(&settings, "math-research/explore-conjecture.yaml");
    let project_path = root.join("project");
    fs::create_dir_all(project_path.join(".mathlab")).expect("mathlab");
    fs::write(
        project_path.join(".mathlab/next-session.json"),
        json!({
            "status": "continue",
            "flow": "math-research/explore-conjecture.yaml",
            "inputs": {"context.request.problem": "P"},
            "rationale": "ignite"
        })
        .to_string(),
    )
    .expect("draft");
    let created = WorkspaceTriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            name: "Ignite once".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: Map::from_iter([
                ("mode".to_string(), json!("workspace_draft")),
                ("project_path".to_string(), json!(project_path)),
                ("flow_allowlist".to_string(), json!(["math-research/*"])),
                ("execution_profile_id".to_string(), json!("math-lab")),
            ]),
            source: Map::from_iter([
                ("kind".to_string(), json!("once")),
                ("run_at".to_string(), json!("2026-06-24T09:00:00Z")),
            ]),
        })
        .expect("create schedule draft trigger");

    let outcomes = WorkspaceTriggerService::new(settings.clone())
        .process_due_trigger_sources_at(at("2026-06-24T10:00:00Z"))
        .await
        .expect("process source");

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].trigger_id, created.id);
    let run_id = outcomes[0]
        .run_id
        .as_deref()
        .unwrap_or_else(|| panic!("run id: {}", outcomes[0].message));
    assert!(project_path
        .join(".mathlab/next-session.launched.json")
        .exists());
    assert!(!project_path.join(".mathlab/next-session.json").exists());
    let run = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read run")
        .expect("run");
    let record = run.record.expect("record");
    assert_eq!(record.flow_name, "math-research/explore-conjecture.yaml");
    assert_eq!(record.execution_profile_id.as_deref(), Some("math-lab"));
}
