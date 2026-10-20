use attractor_api::{AttractorApiService, PipelineStartRequest};
use attractor_runtime::{
    parallel_branch_node_started_event, parallel_branch_started_event, stage_started_event,
    CreateRunRequest, RunStore,
};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

const CHILD_ENV: &str = "SPARK_PIPELINE_STEER_CHILD_TEST";

/// The fake agent binaries are selected by process environment, so each
/// test runs alone in a child process of this test binary.
fn in_child_process(test: &str, envs: &[(&str, String)]) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("pipeline_steer_contracts::{test}"),
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .envs(envs.iter().map(|(key, value)| (key, value)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn fake_bin(name: &str) -> String {
    let dir = std::env::current_exe().unwrap();
    let dir = dir.parent().unwrap().parent().unwrap();
    dir.join(name).to_string_lossy().into_owned()
}

/// Codex app-server fake whose turns stay open until steered.
fn codex_env(test: &str) -> Vec<(&'static str, String)> {
    let log = std::env::temp_dir().join(format!("spark-steer-{}-{test}.log", std::process::id()));
    vec![
        (
            "SPARK_CODEX_APP_SERVER_BIN",
            fake_bin("spark-agent-fake-codex-app-server"),
        ),
        ("SPARK_FAKE_CODEX_APP_SERVER_MODE", "steerable".into()),
        (
            "SPARK_FAKE_CODEX_APP_SERVER_LOG",
            log.to_string_lossy().into(),
        ),
    ]
}

fn log_path(key: &str) -> std::path::PathBuf {
    std::env::var_os(key).unwrap().into()
}

#[track_caller]
fn wait(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !check() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_log(path: &Path, needle: &str, count: usize) {
    wait(|| fs::read_to_string(path).is_ok_and(|text| text.matches(needle).count() >= count));
}

fn settings(root: &Path) -> SparkSettings {
    let mut settings = SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("project"),
        data_dir: root.join("home"),
        config_dir: root.join("home/config"),
        runtime_dir: root.join("home/runtime"),
        logs_dir: root.join("home/logs"),
        workspace_dir: root.join("home/workspace"),
        projects_dir: root.join("home/projects"),
        attractor_dir: root.join("home/attractor"),
        runs_dir: root.join("home/runs"),
        flows_dir: root.join("home/flows"),
        ui_dir: None,
        project_roots: vec![],
    };
    settings.agents.native.codex_runtime_root =
        Some(root.join("codex-runtime").to_string_lossy().into());
    fs::create_dir_all(&settings.project_root).unwrap();
    settings
}

fn service(settings: &SparkSettings) -> AttractorApiService {
    AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        attractor_api::rust_llm_runtime_handler_runner_factory(unified_llm_adapter::Client::new()),
    )
}

fn start(
    service: &AttractorApiService,
    settings: &SparkSettings,
    run_id: &str,
    provider: &str,
    flow: PipelineStartRequest,
) {
    let started = service.start_pipeline(PipelineStartRequest {
        run_id: Some(run_id.into()),
        working_directory: settings.project_root.to_string_lossy().into(),
        llm_provider: Some(provider.into()),
        model: Some("test-model".into()),
        ..flow
    });
    assert_eq!(started.body["status"], "started", "{}", started.body);
}

fn steer(service: &AttractorApiService, run_id: &str, body: Value) -> Value {
    let response = service.dispatch(
        "POST",
        &format!("/attractor/pipelines/{run_id}/steer"),
        &body.to_string(),
    );
    assert_eq!(response.status_code, 200, "{}", response.body);
    response.body
}

fn wait_for_status(settings: &SparkSettings, run_id: &str, status: &str) {
    wait(|| {
        RunStore::for_settings(settings)
            .read_run_bundle(run_id)
            .unwrap()
            .and_then(|bundle| bundle.record)
            .is_some_and(|record| record.status == status)
    });
}

fn intervention_events(settings: &SparkSettings, run_id: &str) -> Vec<Value> {
    RunStore::for_settings(settings)
        .read_run_bundle(run_id)
        .unwrap()
        .unwrap()
        .raw_events
        .into_iter()
        .filter(|event| event.event_type == "HumanInterventionRequested")
        .map(|event| json!(event.payload))
        .collect()
}

/// Whether the node's durable turn log under the run's `logs/` records `text`.
fn node_log_contains(settings: &SparkSettings, run_id: &str, node_id: &str, text: &str) -> bool {
    let bundle = RunStore::for_settings(settings)
        .read_run_bundle(run_id)
        .unwrap()
        .unwrap();
    let mut dirs = vec![bundle.paths.root.join("logs").join(node_id)];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if fs::read_to_string(&path).is_ok_and(|body| body.contains(text)) {
                return true;
            }
        }
    }
    false
}

fn content(flow: String) -> PipelineStartRequest {
    PipelineStartRequest {
        flow_content: Some(flow),
        ..Default::default()
    }
}

fn agent_flow(id: &str) -> String {
    format!(
        r#"schema_version: "1"
id: {id}
nodes:
  start: {{ kind: start }}
  work:
    kind: agent_task
    config: {{ kind: agent_task, prompt: Do the work }}
  done: {{ kind: exit }}
edges:
- {{ from: start, to: work }}
- {{ from: work, to: done }}
"#
    )
}

#[test]
fn steer_delivers_to_active_codex_agent_node() {
    let envs = codex_env("codex");
    if !in_child_process("steer_delivers_to_active_codex_agent_node", &envs) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CODEX_APP_SERVER_LOG");
    start(
        &service,
        &settings,
        "steer-codex",
        "codex",
        content(agent_flow("steer_codex")),
    );
    wait_for_log(&log, "turn/start", 1);

    let result = steer(
        &service,
        "steer-codex",
        json!({"message": "Also update the docs."}),
    );
    assert_eq!(result["status"], "delivered", "{result}");
    assert_eq!(result["delivery_mode"], "codex_app_server_turn");
    assert_eq!(result["target_run_id"], "steer-codex");
    assert_eq!(result["target_node_id"], "work");

    wait_for_status(&settings, "steer-codex", "completed");
    let rpc = fs::read_to_string(&log).unwrap();
    assert!(
        rpc.lines()
            .any(|line| line.contains("turn/steer") && line.contains("Also update the docs.")),
        "{rpc}"
    );
    let events = intervention_events(&settings, "steer-codex");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["intervention_status"], "delivered");
    assert_eq!(events[0]["target_node_id"], "work");
    assert!(node_log_contains(
        &settings,
        "steer-codex",
        "work",
        "Also update the docs."
    ));
}

#[test]
fn steer_delivers_to_active_claude_code_agent_node() {
    let log = std::env::temp_dir().join(format!("spark-steer-{}-claude.log", std::process::id()));
    let envs = [
        (
            "SPARK_CLAUDE_CODE_BIN",
            fake_bin("spark-agent-fake-claude-code"),
        ),
        ("SPARK_FAKE_CLAUDE_CODE_MODE", "steer".to_string()),
        ("SPARK_FAKE_CLAUDE_CODE_LOG", log.to_string_lossy().into()),
    ];
    if !in_child_process("steer_delivers_to_active_claude_code_agent_node", &envs) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CLAUDE_CODE_LOG");
    start(
        &service,
        &settings,
        "steer-claude",
        "claude-code",
        content(agent_flow("steer_claude")),
    );
    wait_for_log(&log, "USER_CONTENT=", 1);

    let result = steer(
        &service,
        "steer-claude",
        json!({"message": "Also update the docs."}),
    );
    assert_eq!(result["status"], "delivered", "{result}");
    assert_eq!(result["delivery_mode"], "claude_code_turn");
    assert_eq!(result["target_node_id"], "work");

    wait_for_status(&settings, "steer-claude", "completed");
    let log = fs::read_to_string(&log).unwrap();
    assert!(
        log.contains("STEER_CONTENT=\"Also update the docs.\""),
        "{log}"
    );
    assert_eq!(
        intervention_events(&settings, "steer-claude")[0]["intervention_status"],
        "delivered"
    );
    assert!(node_log_contains(
        &settings,
        "steer-claude",
        "work",
        "Also update the docs."
    ));
}

#[test]
fn steer_targets_agent_node_of_child_run() {
    let envs = codex_env("child");
    if !in_child_process("steer_targets_agent_node_of_child_run", &envs) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.flows_dir).unwrap();
    fs::write(
        settings.flows_dir.join("child.yaml"),
        agent_flow("steer_child"),
    )
    .unwrap();
    fs::write(
        settings.flows_dir.join("parent.yaml"),
        r#"schema_version: "1"
id: steer_parent
nodes:
  start: { kind: start }
  branch:
    kind: subflow
    config: { kind: subflow, flow_ref: child.yaml }
  done: { kind: exit }
edges:
- { from: start, to: branch }
- { from: branch, to: done }
"#,
    )
    .unwrap();
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CODEX_APP_SERVER_LOG");
    start(
        &service,
        &settings,
        "steer-parent",
        "codex",
        PipelineStartRequest {
            flow_name: Some("parent.yaml".into()),
            ..Default::default()
        },
    );
    wait_for_log(&log, "turn/start", 1);

    let result = steer(
        &service,
        "steer-parent",
        json!({"message": "Child, keep it small."}),
    );
    assert_eq!(result["status"], "delivered", "{result}");
    assert_eq!(result["target_node_id"], "work");
    let child_run_id = result["target_run_id"].as_str().unwrap().to_string();
    assert_ne!(child_run_id, "steer-parent");

    wait_for_status(&settings, "steer-parent", "completed");
    let events = intervention_events(&settings, "steer-parent");
    assert_eq!(events[0]["target_run_id"], json!(child_run_id));
    assert_eq!(events[0]["intervention_status"], "delivered");
    assert!(node_log_contains(
        &settings,
        &child_run_id,
        "work",
        "Child, keep it small."
    ));
}

#[test]
fn steer_rejects_ambiguous_target_until_a_node_is_named() {
    let envs = codex_env("ambiguous");
    if !in_child_process(
        "steer_rejects_ambiguous_target_until_a_node_is_named",
        &envs,
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CODEX_APP_SERVER_LOG");
    start(
        &service,
        &settings,
        "steer-fork",
        "codex",
        content(
            r#"schema_version: "1"
id: steer_fork
nodes:
  start: { kind: start }
  fork:
    kind: parallel
    config: { kind: parallel, join_policy: wait_all }
  left:
    kind: agent_task
    config: { kind: agent_task, prompt: Left work }
  right:
    kind: agent_task
    config: { kind: agent_task, prompt: Right work }
  join:
    kind: fan_in
    config: { kind: fan_in }
  done: { kind: exit }
edges:
- { from: start, to: fork }
- { from: fork, to: left }
- { from: fork, to: right }
- { from: left, to: join }
- { from: right, to: join }
- { from: join, to: done }
"#
            .into(),
        ),
    );
    wait_for_log(&log, "turn/start", 2);

    let result = steer(&service, "steer-fork", json!({"message": "Which one?"}));
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["reason"], "ambiguous_target");
    let message = result["message"].as_str().unwrap();
    assert!(
        message.contains("left") && message.contains("right"),
        "{message}"
    );

    for node in ["left", "right"] {
        let result = steer(
            &service,
            "steer-fork",
            json!({"message": format!("Finish {node}."), "target_node_id": node}),
        );
        assert_eq!(result["status"], "delivered", "{result}");
        assert_eq!(result["target_node_id"], node);
    }
    // The runtime then enters `left` again from the fork; with one node
    // active, an untargeted steer finds it.
    wait_for_log(&log, "turn/start", 3);
    let result = steer(&service, "steer-fork", json!({"message": "Finish."}));
    assert_eq!(result["status"], "delivered", "{result}");
    wait_for_status(&settings, "steer-fork", "completed");
    let statuses = intervention_events(&settings, "steer-fork")
        .iter()
        .map(|event| event["intervention_status"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        [
            json!("rejected"),
            json!("delivered"),
            json!("delivered"),
            json!("delivered")
        ]
    );
}

#[test]
fn steer_follows_a_parallel_branch_past_its_entry_node() {
    let envs = codex_env("branch-chain");
    if !in_child_process("steer_follows_a_parallel_branch_past_its_entry_node", &envs) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CODEX_APP_SERVER_LOG");
    start(
        &service,
        &settings,
        "steer-chain",
        "codex",
        content(
            r#"schema_version: "1"
id: steer_chain
nodes:
  start: { kind: start }
  fork:
    kind: parallel
    config: { kind: parallel, join_policy: wait_all }
  first:
    kind: agent_task
    config: { kind: agent_task, prompt: First work }
  second:
    kind: agent_task
    config: { kind: agent_task, prompt: Second work }
  join:
    kind: fan_in
    config: { kind: fan_in }
  done: { kind: exit }
edges:
- { from: start, to: fork }
- { from: fork, to: first }
- { from: first, to: second }
- { from: second, to: join }
- { from: join, to: done }
"#
            .into(),
        ),
    );
    // Each steer ends the fake's open turn, so `first` completes and the
    // branch moves on to `second`; the runtime then runs both again from
    // the fork.
    for (node, count) in [("first", 1), ("second", 2), ("first", 3), ("second", 4)] {
        wait_for_log(&log, "turn/start", count);
        let result = steer(
            &service,
            "steer-chain",
            json!({"message": format!("Finish {node}.")}),
        );
        assert_eq!(result["status"], "delivered", "{result}");
        assert_eq!(result["target_node_id"], node);
    }
    wait_for_status(&settings, "steer-chain", "completed");
}

#[test]
fn steer_rejects_when_no_agent_node_is_active() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let started = service.start_pipeline(PipelineStartRequest {
        run_id: Some("steer-idle".into()),
        working_directory: settings.project_root.to_string_lossy().into(),
        flow_content: Some(
            "schema_version: \"1\"\nid: idle\nnodes:\n  start: { kind: start }\n  done: { kind: exit }\nedges:\n- { from: start, to: done }\n"
                .into(),
        ),
        wait: Some(true),
        ..Default::default()
    });
    assert_eq!(started.status_code, 200, "{}", started.body);

    let result = steer(&service, "steer-idle", json!({"message": "Anyone there?"}));
    assert_eq!(result["status"], "rejected");
    assert_eq!(result["reason"], "no_active_agent_node");
    let events = intervention_events(&settings, "steer-idle");
    assert_eq!(events[0]["intervention_status"], "rejected");
    assert_eq!(events[0]["intervention_reason"], "no_active_agent_node");
}

#[test]
fn steer_counts_unsteerable_agent_nodes_when_resolving_the_target() {
    let envs = codex_env("mixed");
    if !in_child_process(
        "steer_counts_unsteerable_agent_nodes_when_resolving_the_target",
        &envs,
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CODEX_APP_SERVER_LOG");
    start(
        &service,
        &settings,
        "steer-mixed",
        "codex",
        content(
            r#"schema_version: "1"
id: steer_mixed
nodes:
  start: { kind: start }
  work:
    kind: agent_task
    config: { kind: agent_task, prompt: Do the work }
  other:
    kind: agent_task
    config: { kind: agent_task, prompt: Other work }
  done: { kind: exit }
edges:
- { from: start, to: work }
- { from: work, to: other }
- { from: other, to: done }
"#
            .into(),
        ),
    );
    wait_for_log(&log, "turn/start", 1);
    // `other` is recorded as running with no live session: an agent whose
    // provider cannot be steered, beside the steerable `work`.
    let store = RunStore::for_settings(&settings);
    let paths = store.read_run_bundle("steer-mixed").unwrap().unwrap().paths;
    store
        .append_event(&paths, stage_started_event("steer-mixed", 99, "other"))
        .unwrap();

    let result = steer(&service, "steer-mixed", json!({"message": "Which one?"}));
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["reason"], "ambiguous_target");
    let message = result["message"].as_str().unwrap();
    assert!(
        message.contains("steer-mixed/work") && message.contains("steer-mixed/other"),
        "{message}"
    );

    for (node, count) in [("work", 1), ("other", 2)] {
        wait_for_log(&log, "turn/start", count);
        let result = steer(
            &service,
            "steer-mixed",
            json!({"message": format!("Finish {node}."), "target_node_id": node}),
        );
        assert_eq!(result["status"], "delivered", "{result}");
    }
    wait_for_status(&settings, "steer-mixed", "completed");
    let events = intervention_events(&settings, "steer-mixed");
    assert_eq!(events[0]["intervention_status"], "rejected");
    assert_eq!(events[0]["intervention_reason"], "ambiguous_target");
    assert_eq!(events[1]["intervention_status"], "delivered");
}

#[test]
fn steer_rejects_unsteerable_parallel_agent_nodes() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let store = RunStore::for_settings(&settings);
    let mut record = attractor_core::RunRecord::new(
        "steer-unsupported",
        settings.project_root.to_string_lossy(),
    );
    record.status = "running".to_string();
    let paths = store
        .create_run(CreateRunRequest {
            record,
            flow_source: Some(
                r#"schema_version: "1"
id: steer_unsupported
nodes:
  start: { kind: start }
  fork:
    kind: parallel
    config: { kind: parallel, join_policy: wait_all }
  left:
    kind: agent_task
    config: { kind: agent_task, prompt: Left work }
  right:
    kind: agent_task
    config: { kind: agent_task, prompt: Right work }
  join:
    kind: fan_in
    config: { kind: fan_in }
  done: { kind: exit }
edges:
- { from: start, to: fork }
- { from: fork, to: left }
- { from: fork, to: right }
- { from: left, to: join }
- { from: right, to: join }
- { from: join, to: done }
"#
                .into(),
            ),
            ..Default::default()
        })
        .unwrap();
    store
        .append_event(&paths, stage_started_event("steer-unsupported", 1, "fork"))
        .unwrap();
    for (index, branch) in ["left", "right"].into_iter().enumerate() {
        store
            .append_event(
                &paths,
                parallel_branch_started_event("steer-unsupported", "fork", branch, index),
            )
            .unwrap();
        store
            .append_event(
                &paths,
                parallel_branch_node_started_event(
                    "steer-unsupported",
                    "fork",
                    branch,
                    index,
                    branch,
                ),
            )
            .unwrap();
    }

    let result = steer(&service, "steer-unsupported", json!({"message": "Anyone?"}));
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["reason"], "ambiguous_target");

    let result = steer(
        &service,
        "steer-unsupported",
        json!({"message": "Left, stop.", "target_node_id": "left"}),
    );
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["reason"], "provider_steering_unsupported");
    assert_eq!(result["target_run_id"], "steer-unsupported");
    assert_eq!(result["target_node_id"], "left");

    let events = intervention_events(&settings, "steer-unsupported");
    assert_eq!(events[0]["intervention_reason"], "ambiguous_target");
    assert_eq!(events[1]["intervention_status"], "rejected");
    assert_eq!(
        events[1]["intervention_reason"],
        "provider_steering_unsupported"
    );
    assert_eq!(events[1]["target_node_id"], "left");
}
