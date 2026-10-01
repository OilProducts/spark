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
        json!({"expected_revision": created_payload["revision"], "name": "Compat webhook updated", "regenerate_webhook_secret": true}).to_string(),
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
            "--expected-revision",
            updated_payload["revision"].as_str().unwrap(),
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
            "--expected-revision",
            "protected",
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
        json!({"expected_revision": read_trigger_definition(&settings.config_dir, "trigger-protected").unwrap().unwrap().revision, "action": {"static_context": {"changed": true}}}).to_string(),
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
    serve(build_app(settings)).await
}

async fn serve(app: axum::Router) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
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
        revision: String::new(),
        id: id.to_string(),
        name: "Protected".to_string(),
        enabled: true,
        protected: true,
        source_type: "webhook".to_string(),
        action: TriggerAction {
            mission_id: None,
            mode: "static".to_string(),
            flow_name: "ops/run.yaml".to_string(),
            project_path: Some("/tmp/project".to_string()),
            static_context: Map::from_iter([("origin".to_string(), json!("compat"))]),
            flow_allowlist: Vec::new(),
            execution_profile_id: None,
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
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mission_cli_and_ui_http_share_revisions_and_durable_records() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let project = project.to_str().unwrap();
    // Mission turns run on an agent that answers without spawning a real CLI.
    let server = serve(spark_http::build_app_with_agent_turn_backend(
        settings.clone(),
        std::sync::Arc::new(QuietAgent),
    ))
    .await;
    let payload_file = temp.path().join("mission.json");
    fs::write(
        &payload_file,
        json!({"fields":{"title":"Mission from CLI","description":"First line\nSecond line"}})
            .to_string(),
    )
    .unwrap();
    let output = run_spark(
        temp.path(),
        [
            "mission",
            "create",
            "--project",
            project,
            "--json",
            payload_file.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = created["id"].as_str().unwrap();
    let client = reqwest::Client::new();
    let url = format!("{}/workspace/api/missions/{id}", server.base_url);
    let response = client
        .patch(&url)
        .query(&[("project_path", project)])
        .json(&json!({"revision":created["revision"],"fields":{"archived":true},"actor":"human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let output = run_spark(
        temp.path(),
        [
            "mission",
            "get",
            "--project",
            project,
            "--id",
            id,
            "--base-url",
            &server.base_url,
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let mission: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(mission["fields"]["archived"], true);
    assert_eq!(mission["status"], "draft");
    assert_eq!(mission["activity"][0]["actor"], "assistant");
    assert_eq!(mission["activity"][1]["actor"], "human");
    fs::write(
        &payload_file,
        json!({"revision":1,"fields":{"title":"Stale overwrite"}}).to_string(),
    )
    .unwrap();
    let output = run_spark(
        temp.path(),
        [
            "mission",
            "update",
            "--project",
            project,
            "--id",
            id,
            "--json",
            payload_file.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ],
    );
    assert_ne!(output.status.code(), Some(0));
    let output = run_spark(
        temp.path(),
        [
            "mission",
            "list",
            "--project",
            project,
            "--base-url",
            &server.base_url,
        ],
    );
    let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed["missions"][0]["revision"], 2);
    assert_eq!(listed["missions"][0]["fields"]["title"], "Mission from CLI");
    assert_eq!(listed.as_object().unwrap().len(), 1);
    for key in [
        "stage",
        "hooks",
        "priority",
        "acceptance_criteria",
        "next_action",
        "blocked",
        "needs_input",
        "conversations",
        "artifacts",
        "runs",
        "state",
    ] {
        fs::write(
            &payload_file,
            json!({"revision":2,"fields":{key:null}}).to_string(),
        )
        .unwrap();
        let output = run_spark(
            temp.path(),
            [
                "mission",
                "update",
                "--project",
                project,
                "--id",
                id,
                "--json",
                payload_file.to_str().unwrap(),
                "--base-url",
                &server.base_url,
            ],
        );
        assert_ne!(output.status.code(), Some(0), "{key}");
    }
    let root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project)
        .unwrap()
        .root;
    assert_eq!(
        spark_storage::workspace_missions::MissionRepository::new(&root)
            .read(id)
            .unwrap()
            .unwrap()["activity"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mission_cli = |args: &[&str]| {
        let mut argv = vec!["mission"];
        argv.extend_from_slice(args);
        argv.extend_from_slice(&[
            "--project",
            project,
            "--id",
            id,
            "--base-url",
            &server.base_url,
        ]);
        let output = run_spark(temp.path(), argv);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let started = mission_cli(&["start"]);
    assert_eq!(started["conversation_id"], id);
    assert!(started["started_at"].is_string());
    let waiting = mission_cli(&["wait", "--reason", "Review pending"]);
    assert_eq!(waiting["wait_reason"], "Review pending");
    let mission_action = serde_json::json!({"name":"Review watcher", "source_type":"schedule", "action":{"mode":"mission","mission_id":id,"project_path":project},"source":{"kind":"interval","interval_seconds":300}}).to_string();
    let action_file = temp.path().join("mission-trigger.json");
    fs::write(&action_file, mission_action).unwrap();
    let output = run_spark(
        temp.path(),
        vec![
            "trigger",
            "create",
            "--json",
            action_file.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let trigger: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(trigger["action"]["mission_id"], id);
    fs::write(&action_file, serde_json::json!({"expected_revision":trigger["revision"], "name":"Updated watcher", "action":{"mode":"mission", "mission_id":id, "project_path":project}}).to_string()).unwrap();
    let updated = run_spark(
        temp.path(),
        vec![
            "trigger",
            "update",
            "--id",
            trigger["id"].as_str().unwrap(),
            "--json",
            action_file.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ],
    );
    assert_eq!(updated.status.code(), Some(0), "{}", stderr(&updated));
    let updated: Value = serde_json::from_slice(&updated.stdout).unwrap();
    assert_eq!(updated["action"]["mission_id"], id);
    assert_eq!(updated["name"], "Updated watcher");
    let sent = mission_cli(&["send", "--message", "Focus on the parser"]);
    assert_eq!(sent["id"], id);

    // The mission's agent launches directly from its conversation.
    write_flow(&settings, "work/parse.yaml");
    spark_storage::set_flow_launch_policy(
        &settings.config_dir,
        "work/parse.yaml",
        "agent_requestable",
    )
    .unwrap();
    let snapshot: Value = client
        .get(format!(
            "{}/workspace/api/conversations/{id}",
            server.base_url
        ))
        .query(&[("project_path", project)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let handle = snapshot["conversation_handle"].as_str().unwrap();
    let output = run_spark(
        temp.path(),
        [
            "convo",
            "run-request",
            "--conversation",
            handle,
            "--flow",
            "work/parse.yaml",
            "--summary",
            "Parse it",
            "--base-url",
            &server.base_url,
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let launched: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = launched["run_id"].as_str().unwrap();
    let mission = mission_cli(&["get"]);
    assert_eq!(mission["runs"][0]["run_id"], run_id);
    assert_eq!(mission["runs"][0]["summary"], "Parse it");

    let closed = mission_cli(&["close", "--status", "done", "--reason", "Parser shipped"]);
    assert_eq!(closed["status"], "closed");
    assert_eq!(closed["closed"]["actor"], "assistant");
    assert_eq!(closed["closed"]["reason"], "Parser shipped");
    for removed in ["events", "pause", "resume"] {
        let output = run_spark(
            temp.path(),
            [
                "mission",
                removed,
                "--project",
                project,
                "--id",
                id,
                "--base-url",
                &server.base_url,
            ],
        );
        assert_ne!(output.status.code(), Some(0), "{removed}");
    }
}

struct QuietAgent;
impl spark_agent_adapter::AgentTurnBackend for QuietAgent {
    fn run_turn(
        &self,
        _request: spark_agent_adapter::AgentTurnRequest,
    ) -> Result<spark_agent_adapter::AgentTurnOutput, spark_agent_adapter::AgentError> {
        Ok(final_answer("Understood."))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_cli_validates_and_saves_through_the_real_revision_checked_route() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let server = spawn_server(settings.clone()).await;
    let read = run_spark(
        temp.path(),
        ["settings", "get", "--base-url", &server.base_url],
    );
    assert!(read.status.success(), "{}", stderr(&read));
    let first: Value = serde_json::from_slice(&read.stdout).unwrap();
    let payload = temp.path().join("settings.json");
    fs::write(
        &payload,
        json!({
            "expected_revision": first["runtime"]["revision"],
            "section": "runtime", "value": {"flows_dir": "/from-cli", "project_roots": []}
        })
        .to_string(),
    )
    .unwrap();
    let validated = run_spark(
        temp.path(),
        [
            "settings",
            "validate",
            "--json",
            payload.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ],
    );
    assert!(validated.status.success(), "{}", stderr(&validated));
    assert!(!settings.config_dir.join("spark.toml").exists());
    let saved = run_spark(
        temp.path(),
        [
            "settings",
            "set",
            "--json",
            payload.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ],
    );
    assert!(saved.status.success(), "{}", stderr(&saved));
    let document: Value = serde_json::from_slice(&saved.stdout).unwrap();
    assert_eq!(document["runtime"]["stored"]["flows_dir"], "/from-cli");
    let stale = run_spark(
        temp.path(),
        [
            "settings",
            "set",
            "--json",
            payload.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ],
    );
    assert!(!stale.status.success());
    assert!(stderr(&stale).contains("Settings changed"));
    let read = run_spark(
        temp.path(),
        ["settings", "get", "--base-url", &server.base_url],
    );
    let latest: Value = serde_json::from_slice(&read.stdout).unwrap();
    assert_eq!(
        latest["runtime"]["revision"],
        document["runtime"]["revision"]
    );
}

/// A final answer as a codex turn reports it.
fn final_answer(text: &str) -> spark_agent_adapter::AgentTurnOutput {
    static ITEMS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let item = ITEMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut event = spark_common::events::TurnStreamEvent::content_delta(
        spark_common::events::TurnStreamChannel::Assistant,
        text,
    );
    event.kind = spark_common::events::TurnStreamEventKind::ContentCompleted;
    event.phase = Some("final_answer".into());
    event.source.app_turn_id = Some(format!("app-turn-{item}"));
    event.source.item_id = Some(format!("item-{item}"));
    spark_agent_adapter::AgentTurnOutput {
        events: vec![event],
        final_assistant_text: Some(text.into()),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playbook_cli_lists_gets_and_validates_mission_playbooks() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let project = project.to_str().unwrap();
    fs::create_dir_all(settings.data_dir.join("playbooks")).unwrap();
    fs::write(
        settings.data_dir.join("playbooks/bug-report.md"),
        "---\ntitle: Bug report\ndescription: Fix a bug.\n---\n\nReproduce, then fix.\n",
    )
    .unwrap();
    let server = serve(spark_http::build_app_with_agent_turn_backend(
        settings.clone(),
        std::sync::Arc::new(QuietAgent),
    ))
    .await;
    let spark = |args: &[&str]| {
        let mut argv = args.to_vec();
        argv.extend_from_slice(&["--base-url", &server.base_url]);
        run_spark(temp.path(), argv)
    };

    let output = spark(&["playbook", "list"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!([{"name":"bug-report","title":"Bug report","description":"Fix a bug."}])
    );
    let output = spark(&["playbook", "get", "--name", "bug-report"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let playbook: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(playbook["text"], "Reproduce, then fix.");

    let payload_file = temp.path().join("mission.json");
    fs::write(
        &payload_file,
        json!({"fields":{"title":"Fix it","playbook":"bug-report"}}).to_string(),
    )
    .unwrap();
    let create = [
        "mission",
        "create",
        "--project",
        project,
        "--json",
        payload_file.to_str().unwrap(),
    ];
    let output = spark(&create);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(created["fields"]["playbook"], "bug-report");

    fs::write(
        &payload_file,
        json!({"fields":{"title":"Fix it","playbook":"missing"}}).to_string(),
    )
    .unwrap();
    let output = spark(&create);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("Unknown playbook `missing`"),
        "{}",
        stderr(&output)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mission_create_from_a_conversation_takes_its_project_and_records_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("chat-project");
    let other = temp.path().join("other-project");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&other).unwrap();
    let paths = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project.to_str().unwrap())
        .unwrap();
    spark_storage::ConversationHandleRepository::new(&settings.data_dir)
        .ensure_conversation_handle(
            "conversation-chat",
            &paths.project_id,
            &paths.project_path,
            "2026-10-01T00:00:00Z",
            Some("amber-anchor"),
        )
        .unwrap();
    let server = serve(build_app(settings.clone())).await;
    let payload_file = temp.path().join("mission.json");
    fs::write(
        &payload_file,
        json!({"fields":{"title":"From chat"}}).to_string(),
    )
    .unwrap();
    let create = |extra: &[&str]| {
        let mut argv = vec![
            "mission",
            "create",
            "--conversation",
            "amber-anchor",
            "--json",
            payload_file.to_str().unwrap(),
            "--base-url",
            &server.base_url,
        ];
        argv.extend_from_slice(extra);
        run_spark(temp.path(), argv)
    };

    let output = create(&[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(created["project_path"], paths.project_path.as_str());
    assert_eq!(created["source_conversation_id"], "conversation-chat");

    let output = create(&["--project", &paths.project_path]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    let output = create(&["--project", other.to_str().unwrap()]);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("does not match"),
        "{}",
        stderr(&output)
    );
    let listed: Value = reqwest::Client::new()
        .get(format!("{}/workspace/api/missions", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let projects: Vec<&str> = listed["missions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mission| mission["project_path"].as_str().unwrap())
        .collect();
    assert_eq!(projects, [paths.project_path.as_str(); 2]);
}
