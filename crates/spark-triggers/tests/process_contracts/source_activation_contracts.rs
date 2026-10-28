use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration as StdDuration;

use serde_json::{json, Map, Value};
use spark_common::project::run_git;
use spark_common::settings::SparkSettings;
use spark_storage::{TriggerDefinition, TriggerRepositories, TriggerState};
use spark_triggers::{
    state, TriggerActivationRequest, TriggerActivationSink, TriggerActivationSinkOutcome,
    TriggerCreateRequest, TriggerError, TriggerService, TriggerSourceRuntime, WebhookHandleRequest,
};
use time::OffsetDateTime;

#[tokio::test]
async fn schedule_sources_update_runtime_state_and_due_decisions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let service = TriggerService::new(settings.clone());
    let runtime = TriggerSourceRuntime::new(TriggerRepositories::from_settings(&settings));
    let now = at("2026-06-24T10:00:00Z");

    let once = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Due once".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: action(),
            source: Map::from_iter([
                ("kind".to_string(), json!("once")),
                ("run_at".to_string(), json!("2026-06-24T09:00:00Z")),
            ]),
        })
        .expect("create once");
    let once_definition = definition(&settings, &project, &once.id);
    let once_outcomes = runtime
        .process_schedule_source(&once_definition, now)
        .expect("process once");
    assert_eq!(once_outcomes.len(), 1);
    assert_eq!(
        once_outcomes[0].source_payload,
        json!({"scheduled_at": "2026-06-24T09:00:00Z"})
    );
    assert_eq!(
        once_outcomes[0].trigger.state.last_result.as_deref(),
        Some("success")
    );
    assert_eq!(once_outcomes[0].trigger.state.next_run_at, None);
    assert!(runtime
        .process_schedule_source(&once_definition, now)
        .expect("process once again")
        .is_empty());

    let interval = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Due interval".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: action(),
            source: Map::from_iter([
                ("kind".to_string(), json!("interval")),
                ("interval_seconds".to_string(), json!(60)),
            ]),
        })
        .expect("create interval");
    let interval_definition = definition(&settings, &project, &interval.id);
    let interval_outcomes = runtime
        .process_schedule_source(&interval_definition, now)
        .expect("process interval");
    assert_eq!(interval_outcomes.len(), 1);
    assert_eq!(
        interval_outcomes[0].trigger.state.next_run_at.as_deref(),
        Some("2026-06-24T10:01:00Z")
    );

    let weekly = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Due weekly".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: action(),
            source: Map::from_iter([
                ("kind".to_string(), json!("weekly")),
                ("weekdays".to_string(), json!(["wed"])),
                ("hour".to_string(), json!(9)),
                ("minute".to_string(), json!(30)),
            ]),
        })
        .expect("create weekly");
    let weekly_definition = definition(&settings, &project, &weekly.id);
    let weekly_outcomes = runtime
        .process_schedule_source(&weekly_definition, now)
        .expect("process weekly");
    assert_eq!(weekly_outcomes.len(), 1);
    assert_eq!(
        weekly_outcomes[0].source_payload,
        json!({"scheduled_at": "2026-06-24T09:30:00Z"})
    );

    let disabled = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Disabled once".to_string(),
            enabled: false,
            source_type: "schedule".to_string(),
            action: action(),
            source: Map::from_iter([
                ("kind".to_string(), json!("once")),
                ("run_at".to_string(), json!("2026-06-24T09:00:00Z")),
            ]),
        })
        .expect("create disabled");
    let disabled_definition = definition(&settings, &project, &disabled.id);
    assert!(runtime
        .process_schedule_source(&disabled_definition, now)
        .expect("process disabled")
        .is_empty());
}

#[tokio::test]
async fn poll_sources_fetch_items_honor_headers_and_do_not_dedupe_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let (url, request_rx) =
        json_server(r#"{"items":[{"id":"a","value":1},{"value":2},{"id":"a","value":3}]}"#);
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Poll".to_string(),
            enabled: true,
            source_type: "poll".to_string(),
            action: action(),
            source: Map::from_iter([
                ("url".to_string(), json!(url)),
                ("interval_seconds".to_string(), json!(60)),
                ("items_path".to_string(), json!("items")),
                ("item_id_path".to_string(), json!("id")),
                (
                    "headers".to_string(),
                    json!({
                        "x-test": "compat",
                    }),
                ),
            ]),
        })
        .expect("create poll");
    let definition = definition(&settings, &project, &created.id);
    let runtime = TriggerSourceRuntime::new(TriggerRepositories::from_settings(&settings));

    let outcomes = runtime
        .process_poll_source(&definition, at("2030-01-01T00:00:00Z"))
        .await
        .expect("process poll");

    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].source_payload["poll_item"]["id"], "a");
    assert_eq!(outcomes[1].source_payload["poll_item"]["id"], "a");
    let request = request_rx.recv().expect("captured request");
    assert!(request.contains("x-test: compat"));
    let state = load_state(&settings, &project, &created.id);
    assert_eq!(state.last_result.as_deref(), Some("success"));
    assert_eq!(state.last_error, None);
    assert_eq!(state.next_run_at.as_deref(), Some("2030-01-01T00:01:00Z"));
    assert_eq!(state.recent_history.len(), 2);
}

#[tokio::test]
async fn poll_sources_record_failure_when_items_path_is_not_an_array() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let (url, _request_rx) = json_server(r#"{"items":{"id":"not-array"}}"#);
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Poll failure".to_string(),
            enabled: true,
            source_type: "poll".to_string(),
            action: action(),
            source: Map::from_iter([
                ("url".to_string(), json!(url)),
                ("interval_seconds".to_string(), json!(60)),
                ("items_path".to_string(), json!("items")),
                ("item_id_path".to_string(), json!("id")),
            ]),
        })
        .expect("create poll");
    let definition = definition(&settings, &project, &created.id);
    let runtime = TriggerSourceRuntime::new(TriggerRepositories::from_settings(&settings));

    let outcomes = runtime
        .process_poll_source(&definition, at("2030-01-01T00:00:00Z"))
        .await
        .expect("process poll");

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, "failed");
    assert_eq!(
        outcomes[0].message,
        "Polling failed: Poll source items_path did not resolve to a JSON array."
    );
    let state = load_state(&settings, &project, &created.id);
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(
        state.last_error.as_deref(),
        Some("Polling failed: Poll source items_path did not resolve to a JSON array.")
    );
    assert_eq!(state.recent_history.len(), 1);
}

#[test]
fn flow_event_sources_match_terminal_statuses_and_allow_repeated_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Flow event".to_string(),
            enabled: true,
            source_type: "flow_event".to_string(),
            action: action(),
            source: Map::from_iter([
                ("flow_name".to_string(), json!("ops/run.dot")),
                ("statuses".to_string(), json!(["completed"])),
            ]),
        })
        .expect("create flow event");
    let runtime = TriggerSourceRuntime::new(TriggerRepositories::from_settings(&settings));

    let event = |status: &str, project_path: &str| {
        Map::from_iter([
            ("run_id".to_string(), json!("run-1")),
            ("flow_name".to_string(), json!("ops/run.dot")),
            ("project_path".to_string(), json!(project_path)),
            ("status".to_string(), json!(status)),
        ])
    };
    let ignored = runtime
        .emit_flow_event(event("running", &project))
        .expect("running event");
    assert!(ignored.is_empty());
    // A matching run in another project never fires this project's trigger.
    let other = project_dir(&settings, "other");
    for project_path in [other.as_str(), ""] {
        assert!(runtime
            .emit_flow_event(event("completed", project_path))
            .expect("other project event")
            .is_empty());
    }

    for _ in 0..2 {
        let outcomes = runtime
            .emit_flow_event(event("completed", &project))
            .expect("completed event");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].trigger_id, created.id);
    }
    let state = load_state(&settings, &project, &created.id);
    assert_eq!(state.last_result.as_deref(), Some("success"));
    assert_eq!(state.recent_history.len(), 2);
}

#[test]
fn webhook_sources_authenticate_dispatch_and_record_duplicate_launches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Webhook".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: action(),
            source: Map::new(),
        })
        .expect("create webhook");
    let webhook_key = created.source["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let webhook_secret = created.webhook_secret.expect("webhook secret");
    let launches = Arc::new(AtomicUsize::new(0));
    let runtime = TriggerSourceRuntime::with_sink(
        TriggerRepositories::from_settings(&settings),
        CountingSink {
            launches: launches.clone(),
        },
    );

    assert!(matches!(
        runtime.process_webhook(webhook_request(
            "missing-key",
            &webhook_secret,
            Some("request-1"),
            json!({"payload": "compat"})
        )),
        Err(TriggerError::UnknownWebhookKey)
    ));
    assert!(matches!(
        runtime.process_webhook(webhook_request(
            &webhook_key,
            "not-the-secret",
            Some("request-1"),
            json!({"payload": "compat"})
        )),
        Err(TriggerError::InvalidWebhookSecret)
    ));

    let first = runtime
        .process_webhook(webhook_request(
            &webhook_key,
            &webhook_secret,
            Some("request-1"),
            json!({"payload": "first"}),
        ))
        .expect("first webhook");
    let second = runtime
        .process_webhook(webhook_request(
            &webhook_key,
            &webhook_secret,
            Some("request-1"),
            json!({"payload": "second"}),
        ))
        .expect("second webhook");

    assert!(first.response.ok);
    assert_eq!(first.response.trigger_id, created.id);
    assert_eq!(first.activation.status, "success");
    assert_eq!(first.activation.run_id.as_deref(), Some("run-1"));
    assert_eq!(second.activation.run_id.as_deref(), Some("run-2"));
    assert_eq!(launches.load(Ordering::SeqCst), 2);
    let state = load_state(&settings, &project, &created.id);
    assert_eq!(state.last_result.as_deref(), Some("success"));
    assert_eq!(state.recent_history.len(), 2);
    assert_eq!(state.recent_history[0].run_id.as_deref(), Some("run-2"));
    assert_eq!(state.recent_history[1].run_id.as_deref(), Some("run-1"));
}

#[test]
fn webhook_sources_record_launch_failures_after_authentication() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Webhook".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: action(),
            source: Map::new(),
        })
        .expect("create webhook");
    let webhook_key = created.source["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let webhook_secret = created.webhook_secret.expect("webhook secret");
    let runtime = TriggerSourceRuntime::with_sink(
        TriggerRepositories::from_settings(&settings),
        FailingSink {
            message: "launch failed".to_string(),
        },
    );

    let outcome = runtime
        .process_webhook(webhook_request(
            &webhook_key,
            &webhook_secret,
            Some("request-2"),
            json!({"payload": "failure"}),
        ))
        .expect("accepted failed webhook");

    assert!(outcome.response.ok);
    assert_eq!(outcome.activation.status, "failed");
    assert_eq!(outcome.activation.message, "launch failed");
    assert!(outcome.activation.run_id.is_none());
    let state = load_state(&settings, &project, &created.id);
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(state.last_error.as_deref(), Some("launch failed"));
    assert_eq!(state.recent_history.len(), 1);
}

#[tokio::test]
async fn due_sources_scan_every_registered_org_and_pick_up_hand_edits() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let launches = Arc::new(AtomicUsize::new(0));
    let runtime = TriggerSourceRuntime::with_sink(
        TriggerRepositories::from_settings(&settings),
        CountingSink {
            launches: launches.clone(),
        },
    );
    let now = at("2026-06-24T10:00:00Z");
    assert!(runtime.process_due_sources(now).await.unwrap().is_empty());

    // A definition the Chief of Staff writes and commits by hand fires on the
    // next tick, in its own project.
    let org = spark_common::project::org_dir(Path::new(&project));
    std::fs::create_dir_all(org.join("triggers")).unwrap();
    let path = org.join("triggers/by-hand.toml");
    std::fs::write(
        &path,
        r#"id = "by-hand"
name = "By hand"
protected = false
source_type = "schedule"
created_at = "2026-06-24T09:00:00Z"
updated_at = "2026-06-24T09:00:00Z"

[action]
flow_name = "ops/run.dot"

[source]
kind = "once"
run_at = "2026-06-24T09:30:00Z"
"#,
    )
    .unwrap();
    run_git(&org, &["add", "triggers/by-hand.toml"]).unwrap();
    run_git(&org, &["commit", "-qm", "Add a trigger by hand"]).unwrap();
    let outcomes = runtime.process_due_sources(now).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].trigger_id, "by-hand");
    assert_eq!(outcomes[0].trigger.project_path, project);
    assert_eq!(launches.load(Ordering::SeqCst), 1);

    // Disabled in state, or deleted from the org repo: it no longer fires.
    let interval = path.with_file_name("by-hand-interval.toml");
    std::fs::write(
        &interval,
        std::fs::read_to_string(&path)
            .unwrap()
            .replace("by-hand", "by-hand-interval")
            .replace(
                "kind = \"once\"\nrun_at = \"2026-06-24T09:30:00Z\"",
                "kind = \"interval\"\ninterval_seconds = 60",
            ),
    )
    .unwrap();
    TriggerRepositories::from_settings(&settings)
        .runtime_state(&project)
        .unwrap()
        .save(
            "by-hand-interval",
            &TriggerState {
                enabled: false,
                ..TriggerState::default()
            },
        )
        .unwrap();
    assert!(runtime.process_due_sources(now).await.unwrap().is_empty());
    std::fs::remove_file(&interval).unwrap();
    assert!(runtime
        .process_due_sources(at("2026-06-25T10:00:00Z"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(launches.load(Ordering::SeqCst), 1);
}

#[test]
fn activation_recording_keeps_a_switch_off_and_secret_rotation_made_during_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let service = TriggerService::new(settings.clone());
    let created = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: None,
            name: "Webhook".to_string(),
            enabled: true,
            source_type: "webhook".to_string(),
            action: action(),
            source: Map::new(),
        })
        .expect("create webhook");
    let runtime = TriggerSourceRuntime::with_sink(
        TriggerRepositories::from_settings(&settings),
        SwitchOffSink {
            service: service.clone(),
            project: project.clone(),
            revision: created.revision.clone(),
        },
    );
    let outcome = runtime
        .process_webhook(webhook_request(
            created.source["webhook_key"].as_str().unwrap(),
            created.webhook_secret.as_deref().unwrap(),
            None,
            json!({}),
        ))
        .expect("webhook");
    assert_eq!(outcome.activation.status, "success");
    let state = load_state(&settings, &project, &created.id);
    assert!(
        !state.enabled,
        "the switch-off survives recording the activation"
    );
    let rotated = state.secret_hash.clone();
    assert_ne!(rotated, None);
    assert_eq!(state.recent_history.len(), 1);
    // The secret issued at creation no longer works: the rotation survived too.
    assert!(matches!(
        runtime.process_webhook(webhook_request(
            created.source["webhook_key"].as_str().unwrap(),
            created.webhook_secret.as_deref().unwrap(),
            None,
            json!({}),
        )),
        Err(TriggerError::Validation(_))
    ));
}

/// Switches its trigger off and rotates its secret while activating, as a
/// user would during a slow launch.
struct SwitchOffSink {
    service: TriggerService,
    project: String,
    revision: String,
}

impl TriggerActivationSink for SwitchOffSink {
    fn activate(
        &self,
        request: TriggerActivationRequest,
    ) -> spark_triggers::TriggerResult<TriggerActivationSinkOutcome> {
        self.service.update_trigger(
            &self.project,
            &request.trigger_id,
            spark_triggers::TriggerUpdateRequest {
                expected_revision: self.revision.clone(),
                enabled: Some(false),
                regenerate_webhook_secret: true,
                ..Default::default()
            },
        )?;
        Ok(TriggerActivationSinkOutcome {
            run_id: Some("run-1".into()),
            message: None,
            no_op: false,
        })
    }
}

#[derive(Debug, Clone)]
struct CountingSink {
    launches: Arc<AtomicUsize>,
}

impl TriggerActivationSink for CountingSink {
    fn activate(
        &self,
        request: TriggerActivationRequest,
    ) -> spark_triggers::TriggerResult<TriggerActivationSinkOutcome> {
        let index = self.launches.fetch_add(1, Ordering::SeqCst) + 1;
        assert_eq!(request.action.flow_name, "ops/run.dot");
        Ok(TriggerActivationSinkOutcome {
            run_id: Some(format!("run-{index}")),
            message: Some("Trigger fired successfully.".to_string()),
            no_op: false,
        })
    }
}

#[derive(Debug, Clone)]
struct FailingSink {
    message: String,
}

impl TriggerActivationSink for FailingSink {
    fn activate(
        &self,
        _request: TriggerActivationRequest,
    ) -> spark_triggers::TriggerResult<TriggerActivationSinkOutcome> {
        Err(TriggerError::Validation(self.message.clone()))
    }
}

fn json_server(body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind server");
    let address = listener.local_addr().expect("server addr");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        stream
            .set_read_timeout(Some(StdDuration::from_secs(2)))
            .expect("read timeout");
        let mut buffer = [0_u8; 4096];
        let len = stream.read(&mut buffer).expect("read request");
        tx.send(String::from_utf8_lossy(&buffer[..len]).to_string())
            .expect("send request");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });
    (format!("http://{address}/items"), rx)
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

fn action() -> Map<String, Value> {
    Map::from_iter([
        ("flow_name".to_string(), json!("ops/run.dot")),
        ("static_context".to_string(), json!({"origin": "test"})),
    ])
}

fn definition(settings: &SparkSettings, project: &str, id: &str) -> TriggerDefinition {
    TriggerRepositories::from_settings(settings)
        .definitions(project)
        .unwrap()
        .get(id)
        .expect("read definition")
        .expect("definition")
}

fn load_state(settings: &SparkSettings, project: &str, id: &str) -> TriggerState {
    TriggerRepositories::from_settings(settings)
        .runtime_state(project)
        .unwrap()
        .load(id)
        .expect("load state")
}

/// The normalized path of a project directory under the test root.
fn project_dir(settings: &SparkSettings, name: &str) -> String {
    let project = settings.data_dir.parent().unwrap().join(name);
    std::fs::create_dir_all(&project).expect("project dir");
    spark_storage::ProjectRegistry::new(settings.data_dir.clone())
        .project_paths(&project.to_string_lossy())
        .expect("project paths")
        .project_path
}

/// A registered project whose org repo has a git identity, since every
/// definition write commits.
fn project(settings: &SparkSettings, name: &str) -> String {
    let project = project_dir(settings, name);
    let org = spark_common::project::ensure_org_repo(Path::new(&project)).expect("org repo");
    for (key, value) in [
        ("user.name", "Trigger Test"),
        ("user.email", "trigger@example.com"),
    ] {
        run_git(&org, &["config", key, value]).expect("git config");
    }
    spark_storage::ProjectRegistry::new(settings.data_dir.clone())
        .ensure_project_paths(&project)
        .expect("register project");
    project
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
