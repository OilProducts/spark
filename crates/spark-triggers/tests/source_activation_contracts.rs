use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration as StdDuration;

use serde_json::{json, Map, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{read_trigger_definition, TriggerRepositories};
use spark_triggers::{
    state, TriggerActivationRequest, TriggerActivationSink, TriggerActivationSinkOutcome,
    TriggerCreateRequest, TriggerError, TriggerService, TriggerSourceRuntime, WebhookHandleRequest,
};
use time::OffsetDateTime;

#[tokio::test]
async fn schedule_sources_update_runtime_state_and_due_decisions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = TriggerService::new(settings.clone());
    let runtime = TriggerSourceRuntime::new(TriggerRepositories::from_settings(&settings));
    let now = at("2026-06-24T10:00:00Z");

    let once = service
        .create_trigger(TriggerCreateRequest {
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
    let once_definition = read_trigger_definition(&settings.config_dir, &once.id)
        .expect("read once")
        .expect("once definition");
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
    let interval_definition = read_trigger_definition(&settings.config_dir, &interval.id)
        .expect("read interval")
        .expect("interval definition");
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
    let weekly_definition = read_trigger_definition(&settings.config_dir, &weekly.id)
        .expect("read weekly")
        .expect("weekly definition");
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
    let disabled_definition = read_trigger_definition(&settings.config_dir, &disabled.id)
        .expect("read disabled")
        .expect("disabled definition");
    assert!(runtime
        .process_schedule_source(&disabled_definition, now)
        .expect("process disabled")
        .is_empty());
}

#[tokio::test]
async fn poll_sources_fetch_items_honor_headers_and_do_not_dedupe_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let (url, request_rx) =
        json_server(r#"{"items":[{"id":"a","value":1},{"value":2},{"id":"a","value":3}]}"#);
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
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
    let definition = read_trigger_definition(&settings.config_dir, &created.id)
        .expect("read poll")
        .expect("poll definition");
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
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load poll state");
    assert_eq!(state.last_result.as_deref(), Some("success"));
    assert_eq!(state.last_error, None);
    assert_eq!(state.next_run_at.as_deref(), Some("2030-01-01T00:01:00Z"));
    assert_eq!(state.recent_history.len(), 2);
}

#[tokio::test]
async fn poll_sources_record_failure_when_items_path_is_not_an_array() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let (url, _request_rx) = json_server(r#"{"items":{"id":"not-array"}}"#);
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
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
    let definition = read_trigger_definition(&settings.config_dir, &created.id)
        .expect("read poll")
        .expect("poll definition");
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
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load poll state");
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
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
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

    let ignored = runtime
        .emit_flow_event(Map::from_iter([
            ("run_id".to_string(), json!("run-1")),
            ("flow_name".to_string(), json!("ops/run.dot")),
            ("status".to_string(), json!("running")),
        ]))
        .expect("running event");
    assert!(ignored.is_empty());

    for _ in 0..2 {
        let outcomes = runtime
            .emit_flow_event(Map::from_iter([
                ("run_id".to_string(), json!("run-1")),
                ("flow_name".to_string(), json!("ops/run.dot")),
                ("status".to_string(), json!("completed")),
            ]))
            .expect("completed event");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].trigger_id, created.id);
    }
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load flow state");
    assert_eq!(state.last_result.as_deref(), Some("success"));
    assert_eq!(state.recent_history.len(), 2);
}

#[test]
fn webhook_sources_authenticate_dispatch_and_record_duplicate_launches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
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
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load webhook state");
    assert_eq!(state.last_result.as_deref(), Some("success"));
    assert_eq!(state.recent_history.len(), 2);
    assert_eq!(state.recent_history[0].run_id.as_deref(), Some("run-2"));
    assert_eq!(state.recent_history[1].run_id.as_deref(), Some("run-1"));
}

#[test]
fn webhook_sources_record_launch_failures_after_authentication() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let created = TriggerService::new(settings.clone())
        .create_trigger(TriggerCreateRequest {
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
    let state = spark_storage::load_trigger_state(&settings.data_dir, &created.id)
        .expect("load failed webhook state");
    assert_eq!(state.last_result.as_deref(), Some("failed"));
    assert_eq!(state.last_error.as_deref(), Some("launch failed"));
    assert_eq!(state.recent_history.len(), 1);
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
        ("project_path".to_string(), json!("/tmp/project")),
        ("static_context".to_string(), json!({"origin": "test"})),
    ])
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
