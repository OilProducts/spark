use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Map, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use spark_storage::{
    read_trigger_definition, write_trigger_definition, TriggerAction, TriggerDefinition,
};
use tokio::task::JoinHandle;

fn spark_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spark")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trigger_cli_exercises_real_m4_routes_and_storage_effects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.yaml");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let server = spawn_server(settings.clone()).await;

    let create_payload = temp.path().join("trigger-create.json");
    fs::write(
        &create_payload,
        json!({
            "name": "Compat webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/run.yaml",
                "project_path": project_dir,
                "static_context": {"origin": "compat"}
            },
            "source": {}
        })
        .to_string(),
    )
    .expect("write create payload");
    let created = run_spark(
        temp.path(),
        [
            "trigger",
            "create",
            "--json",
            create_payload.to_str().expect("create path"),
            "--base-url",
            server.base_url.as_str(),
        ],
    );
    assert_eq!(created.status.code(), Some(0), "{}", stderr(&created));
    let created_payload: Value = serde_json::from_slice(&created.stdout).expect("created json");
    assert_eq!(created_payload["name"], "Compat webhook");
    assert_eq!(created_payload["source_type"], "webhook");
    assert!(created_payload["source"]["secret_hash"].is_null());
    assert!(
        created_payload["webhook_secret"]
            .as_str()
            .expect("webhook secret")
            .len()
            == 32
    );
    let trigger_id = created_payload["id"]
        .as_str()
        .expect("trigger id")
        .to_string();
    assert!(settings
        .config_dir
        .join("triggers")
        .join(format!("{trigger_id}.toml"))
        .exists());
    assert!(settings
        .workspace_dir
        .join("trigger-state")
        .join(format!("{trigger_id}.json"))
        .exists());

    let described = run_spark(
        temp.path(),
        [
            "trigger",
            "describe",
            "--id",
            trigger_id.as_str(),
            "--text",
            "--base-url",
            server.base_url.as_str(),
        ],
    );
    assert_eq!(described.status.code(), Some(0), "{}", stderr(&described));
    let described_stdout = stdout(&described);
    assert!(described_stdout.contains(&format!("ID: {trigger_id}\n")));
    assert!(described_stdout.contains("Source Type: webhook\n"));
    assert!(described_stdout.contains("Project Target: "));
    assert!(described_stdout.contains("Last Fired: (never)\n"));
    assert!(!described_stdout.contains("Webhook Secret:"));

    let update_payload = temp.path().join("trigger-update.json");
    fs::write(
        &update_payload,
        json!({"name": "Compat webhook updated", "regenerate_webhook_secret": true}).to_string(),
    )
    .expect("write update payload");
    let updated = run_spark(
        temp.path(),
        [
            "trigger",
            "update",
            "--id",
            trigger_id.as_str(),
            "--json",
            update_payload.to_str().expect("update path"),
            "--base-url",
            server.base_url.as_str(),
        ],
    );
    assert_eq!(updated.status.code(), Some(0), "{}", stderr(&updated));
    let updated_payload: Value = serde_json::from_slice(&updated.stdout).expect("updated json");
    assert_eq!(updated_payload["name"], "Compat webhook updated");
    assert!(
        updated_payload["webhook_secret"]
            .as_str()
            .expect("updated webhook secret")
            .len()
            == 32
    );

    let deleted = run_spark(
        temp.path(),
        [
            "trigger",
            "delete",
            "--id",
            trigger_id.as_str(),
            "--base-url",
            server.base_url.as_str(),
        ],
    );
    assert_eq!(deleted.status.code(), Some(0), "{}", stderr(&deleted));
    let deleted_payload: Value = serde_json::from_slice(&deleted.stdout).expect("deleted json");
    assert_eq!(
        deleted_payload,
        json!({"id": trigger_id, "status": "deleted"})
    );
    assert!(read_trigger_definition(
        &settings.config_dir,
        deleted_payload["id"].as_str().unwrap()
    )
    .expect("read deleted trigger")
    .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trigger_cli_real_routes_preserve_protected_and_validation_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.yaml");
    write_trigger_definition(
        &settings.config_dir,
        &protected_definition("trigger-protected"),
    )
    .expect("protected definition");
    let server = spawn_server(settings.clone()).await;

    let protected_delete = run_spark(
        temp.path(),
        [
            "trigger",
            "delete",
            "--id",
            "trigger-protected",
            "--base-url",
            server.base_url.as_str(),
        ],
    );
    assert_eq!(protected_delete.status.code(), Some(1));
    assert_eq!(stdout(&protected_delete), "");
    assert_eq!(
        stderr(&protected_delete),
        "{\"ok\": false, \"status_code\": 400, \"error\": \"Protected triggers cannot be deleted.\"}\n"
    );

    let protected_update_payload = temp.path().join("protected-update.json");
    fs::write(
        &protected_update_payload,
        json!({"action": {"static_context": {"changed": true}}}).to_string(),
    )
    .expect("write protected update payload");
    let protected_update = run_spark(
        temp.path(),
        [
            "trigger",
            "update",
            "--id",
            "trigger-protected",
            "--json",
            protected_update_payload
                .to_str()
                .expect("protected update path"),
            "--base-url",
            server.base_url.as_str(),
        ],
    );
    assert_eq!(protected_update.status.code(), Some(1));
    assert_eq!(
        stderr(&protected_update),
        "{\"ok\": false, \"status_code\": 400, \"error\": \"Protected triggers do not allow static context changes.\"}\n"
    );

    let unknown_flow_payload = temp.path().join("unknown-flow.json");
    fs::write(
        &unknown_flow_payload,
        json!({
            "name": "Unknown flow",
            "source_type": "webhook",
            "action": {"flow_name": "missing.yaml"},
            "source": {}
        })
        .to_string(),
    )
    .expect("write unknown flow payload");
    let unknown_flow = run_spark(
        temp.path(),
        [
            "trigger",
            "create",
            "--json",
            unknown_flow_payload.to_str().expect("unknown flow path"),
            "--base-url",
            server.base_url.as_str(),
        ],
    );
    assert_eq!(unknown_flow.status.code(), Some(3));
    assert_eq!(
        stderr(&unknown_flow),
        "{\"ok\": false, \"status_code\": 404, \"error\": \"Unknown flow: missing.yaml\"}\n"
    );
    assert!(
        read_trigger_definition(&settings.config_dir, "trigger-protected")
            .expect("read protected trigger")
            .is_some()
    );
}

struct TestServer {
    base_url: String,
    handle: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn spawn_server(settings: SparkSettings) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let app = build_app(settings);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    TestServer { base_url, handle }
}

fn run_spark<'a>(home: &Path, args: impl IntoIterator<Item = &'a str>) -> Output {
    Command::new(spark_bin())
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("SPARK_HOME", home.join("spark-home"))
        .output()
        .expect("run spark")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout utf8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr utf8")
}

fn protected_definition(id: &str) -> TriggerDefinition {
    TriggerDefinition {
        id: id.to_string(),
        name: "Protected".to_string(),
        enabled: true,
        protected: true,
        source_type: "webhook".to_string(),
        action: TriggerAction {
            flow_name: "ops/run.yaml".to_string(),
            project_path: Some("/tmp/project".to_string()),
            static_context: Map::from_iter([("origin".to_string(), json!("compat"))]),
        },
        source: Map::from_iter([
            ("webhook_key".to_string(), json!("protected-key")),
            ("secret_hash".to_string(), json!("protected-secret-hash")),
        ]),
        created_at: "2026-06-22T16:16:08Z".to_string(),
        updated_at: "2026-06-22T16:16:08Z".to_string(),
    }
}

fn write_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(
        path,
        "schema_version: '1'\nid: trigger-flow\ntitle: Trigger Flow\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    )
    .expect("flow");
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
