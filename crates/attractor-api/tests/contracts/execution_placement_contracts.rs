use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use attractor_api::{
    execution_placement_settings, handle_attractor_request, AttractorApiService,
    ContinuePipelineRequest, PipelineStartRequest,
};
use attractor_execution::{CommandResult, CommandSpec, ContainerCommandRunner};
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::json;
use spark_common::settings::SparkSettings;

#[test]
fn execution_placement_settings_exposes_profile_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[defaults]
execution_profile_id = "native-fast"

[profiles.native-fast]
label = "Native Fast"
mode = "native"
capabilities = ["filesystem"]
"#,
    )
    .expect("write profiles");

    let response = execution_placement_settings(&settings);

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["loaded"], json!(true));
    assert_eq!(
        response.body["default_execution_profile_id"],
        json!("native-fast")
    );
    assert_eq!(response.body["profiles"][0]["id"], json!("native-fast"));
}

#[test]
fn execution_placement_settings_route_uses_mounted_attractor_metadata_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[profiles.native-fast]
label = "Native Fast"
mode = "native"
"#,
    )
    .expect("write profiles");

    let response = handle_attractor_request(
        "GET",
        "/attractor/api/execution-placement-settings",
        "",
        settings,
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["loaded"], json!(true));
    assert_eq!(response.body["profiles"][0]["id"], json!("native-fast"));
}

#[test]
fn missing_config_synthesizes_native_default_for_public_settings_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());

    let response = handle_attractor_request(
        "GET",
        "/attractor/api/execution-placement-settings",
        "",
        settings,
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["exists"], json!(false));
    assert_eq!(response.body["config"]["loaded"], json!(true));
    assert_eq!(
        response.body["config"]["synthesized_native_default"],
        json!(true)
    );
    assert_eq!(response.body["default_execution_profile_id"], json!(null));
    assert_eq!(
        response.body["profiles"],
        json!([{
            "id": "native",
            "label": "Native",
            "mode": "native",
            "enabled": true,
            "image": null,
            "capabilities": {},
            "metadata": {},
        }])
    );
}

#[test]
fn execution_placement_settings_reports_invalid_config_as_validation_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[profiles.container]
label = "Container"
mode = "local_container"
"#,
    )
    .expect("write profiles");

    let response = execution_placement_settings(&settings);

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["loaded"], json!(false));
    assert_eq!(
        response.body["validation_errors"][0],
        json!({
            "field": "profiles.container.image",
            "message": "image is required for local_container profiles",
            "profile_id": "container"
        })
    );
}

#[derive(Clone)]
struct FakeDocker {
    commands: Arc<Mutex<Vec<CommandSpec>>>,
}

impl ContainerCommandRunner for FakeDocker {
    fn command_exists(&self, _program: &str) -> bool {
        true
    }

    fn run(&mut self, spec: CommandSpec) -> std::io::Result<CommandResult> {
        let is_run = spec.args.first().map(String::as_str) == Some("run");
        let is_exec = spec.args.first().map(String::as_str) == Some("exec");
        self.commands.lock().expect("commands").push(spec);
        Ok(CommandResult {
            exit_code: 0,
            stdout: if is_run {
                "fake-container\n".to_string()
            } else if is_exec {
                serde_json::to_string(&json!({
                    "type": "result",
                    "outcome": {
                        "status": "success",
                        "preferred_label": "",
                        "suggested_next_ids": [],
                        "context_updates": {},
                        "failure_reason": "",
                        "notes": "",
                        "retryable": false,
                        "raw_response_text": "fake worker completed"
                    },
                    // The real worker echoes its full post-node context
                    // snapshot here (request context plus runtime builtins),
                    // not just the node's declared writes. The executor must
                    // not fold this into context_updates or contract
                    // enforcement rejects every containerized node.
                    "context": {
                        "graph.id": "worker_flow",
                        "graph.goal": "worker flow goal",
                        "internal.run_id": "run-fake",
                        "internal.run_workdir": "/work",
                        "current_node": "work",
                        "outcome": "success",
                        "preferred_label": "",
                        "_attractor.node_outcomes": {"work": "success"},
                        "_attractor.runtime.execution_mode": "local_container"
                    }
                }))
                .expect("worker frame")
                    + "\n"
            } else {
                String::new()
            },
            stderr: String::new(),
        })
    }

    fn run_streaming(
        &mut self,
        spec: CommandSpec,
        on_stdout_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<CommandResult> {
        on_stdout_line(
            &serde_json::to_string(&json!({
                "type": "event",
                "event": {
                    "type": "container_live",
                    "run_id": "run-fake",
                    "emitted_at": "2026-07-22T12:00:00Z",
                    "node_id": "work"
                }
            }))
            .expect("event frame"),
        );
        let result = self.run(spec)?;
        for line in result.stdout.lines() {
            on_stdout_line(line);
        }
        Ok(result)
    }
}

fn service_with_fake_docker(
    settings: SparkSettings,
) -> (AttractorApiService, Arc<Mutex<Vec<CommandSpec>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let factory_commands = commands.clone();
    let service = AttractorApiService::new(settings).with_container_command_runner_factory(
        Arc::new(move || {
            Box::new(FakeDocker {
                commands: factory_commands.clone(),
            })
        }),
    );
    (service, commands)
}

fn write_container_profile(settings: &SparkSettings, image: &str) {
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        format!(
            r#"[profiles.container]
label = "Container"
mode = "local_container"
image = "{image}"
"#
        ),
    )
    .expect("write profiles");
}

fn write_container_profile_with_missing_default(settings: &SparkSettings) {
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"[defaults]
execution_profile_id = "missing-default"

[profiles.container]
label = "Container"
mode = "local_container"
image = "spark-worker:test"
"#,
    )
    .expect("write profiles");
}

fn worker_flow() -> String {
    r#"schema_version: "1"
id: docker_dispatch
nodes:
  start:
    kind: start
  task:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Exercise the worker
  done:
    kind: exit
edges:
  - from: start
    to: task
  - from: task
    to: done
"#
    .to_string()
}

fn assert_docker_dispatch(commands: &[CommandSpec]) {
    let run = commands
        .iter()
        .position(|command| command.args.first().map(String::as_str) == Some("run"))
        .expect("docker run");
    let exec = commands
        .iter()
        .position(|command| command.args.first().map(String::as_str) == Some("exec"))
        .expect("docker exec");
    assert!(run < exec, "docker run must precede docker exec");
    assert_eq!(
        &commands[exec].args[..6],
        [
            "exec",
            "-i",
            "fake-container",
            "spark-server",
            "worker",
            "run-node",
        ]
    );
}

fn wait_for_status(settings: &SparkSettings, run_id: &str, expected: &str) {
    let store = RunStore::for_settings(settings);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let record = store
            .read_run_bundle(run_id)
            .expect("bundle")
            .and_then(|bundle| bundle.record)
            .expect("record");
        let status = record.status.clone();
        if status == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "last status: {status}; error: {}",
            record.last_error
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn waited_and_detached_container_launches_dispatch_through_docker_and_record_placement() {
    for (run_id, wait) in [("container-waited", true), ("container-detached", false)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = settings(temp.path());
        write_container_profile(&settings, "spark-worker:test");
        let (service, commands) = service_with_fake_docker(settings.clone());
        let response = service.start_pipeline(PipelineStartRequest {
            run_id: Some(run_id.to_string()),
            flow_content: Some(worker_flow()),
            working_directory: temp.path().join("project").to_string_lossy().to_string(),
            execution_profile_id: Some("container".to_string()),
            wait: Some(wait),
            ..PipelineStartRequest::default()
        });
        assert_eq!(response.status_code, 200, "{:?}", response.body);
        wait_for_status(&settings, run_id, "completed");
        assert_docker_dispatch(&commands.lock().expect("commands"));
        let record = RunStore::for_settings(&settings)
            .read_run_bundle(run_id)
            .expect("bundle")
            .expect("run")
            .record
            .expect("record");
        assert_eq!(record.execution_mode, "local_container");
        assert_eq!(record.execution_profile_id.as_deref(), Some("container"));
        assert_eq!(
            record.execution_container_image.as_deref(),
            Some("spark-worker:test")
        );
        let store = RunStore::for_settings(&settings);
        let paths = store.find_run_root(run_id).expect("run root").expect("run");
        let events = store.read_raw_events(&paths).expect("events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "container_live")
                .count(),
            2
        );
    }
}

#[test]
fn native_launch_never_invokes_docker() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let (service, commands) = service_with_fake_docker(settings.clone());
    let response = service.start_pipeline(PipelineStartRequest {
        run_id: Some("native-no-docker".to_string()),
        flow_content: Some(worker_flow()),
        working_directory: temp.path().join("project").to_string_lossy().to_string(),
        wait: Some(true),
        ..PipelineStartRequest::default()
    });
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    assert!(commands.lock().expect("commands").is_empty());
}

fn seed_retry(settings: &SparkSettings, run_id: &str) {
    let flow = attractor_dsl::parse_flow_definition(&worker_flow()).expect("flow");
    let mut record =
        attractor_core::RunRecord::new(run_id, settings.project_root.to_string_lossy().to_string());
    record.status = "failed".to_string();
    record.execution_profile_id = Some("container".to_string());
    record.execution_mode = "local_container".to_string();
    record.execution_container_image = Some("spark-worker:test".to_string());
    RunStore::for_settings(settings)
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(attractor_core::CheckpointState {
                timestamp: "2026-07-22T00:00:00Z".to_string(),
                current_node: "start".to_string(),
                completed_nodes: Vec::new(),
                context: Default::default(),
                retry_counts: Default::default(),
                logs: Vec::new(),
            }),
            manifest: None,
            flow_source: Some(worker_flow()),
            flow_definition_json: Some(flow.to_canonical_json_string()),
        })
        .expect("seed run");
}

#[test]
fn retry_reconstructs_matching_container_placement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_container_profile_with_missing_default(&settings);
    seed_retry(&settings, "retry-container");
    let (service, commands) = service_with_fake_docker(settings.clone());
    let response = service.retry_pipeline_route("retry-container");
    assert_eq!(response.body["status"], json!("started"));
    wait_for_status(&settings, "retry-container", "completed");
    assert_docker_dispatch(&commands.lock().expect("commands"));
}

#[test]
fn continue_reconstructs_matching_container_placement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_container_profile_with_missing_default(&settings);
    seed_retry(&settings, "continue-container-source");
    let (service, commands) = service_with_fake_docker(settings.clone());
    let response = service.continue_pipeline_route(
        "continue-container-source",
        ContinuePipelineRequest {
            start_node: "task".to_string(),
            flow_source_mode: "snapshot".to_string(),
            ..ContinuePipelineRequest::default()
        },
    );
    assert_eq!(
        response.body["status"],
        json!("started"),
        "{:?}",
        response.body
    );
    let run_id = response.body["run_id"].as_str().expect("continued run id");
    wait_for_status(&settings, run_id, "completed");
    assert_docker_dispatch(&commands.lock().expect("commands"));
}

#[test]
fn startup_recovery_reconstructs_matching_container_placement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_container_profile_with_missing_default(&settings);
    seed_retry(&settings, "startup-container");
    RunStore::for_settings(&settings)
        .update_run_record("startup-container", |record| {
            record.status = "waiting".to_string();
        })
        .expect("mark waiting");
    let (service, commands) = service_with_fake_docker(settings.clone());
    let recovery = service.recover_interrupted_runs();
    assert_eq!(
        recovery["resumed"],
        json!(["startup-container"]),
        "{recovery:?}"
    );
    wait_for_status(&settings, "startup-container", "completed");
    assert_docker_dispatch(&commands.lock().expect("commands"));
}

#[test]
fn retry_reconstruction_failures_are_persisted_without_native_or_docker_execution() {
    for (case, config, expected) in [
        ("missing", "[profiles.other]\nlabel = \"Other\"\nmode = \"native\"\n", "does not exist"),
        ("disabled", "[profiles.container]\nlabel = \"Container\"\nmode = \"local_container\"\nimage = \"spark-worker:test\"\nenabled = false\n", "disabled"),
        ("invalid", "[profiles.container]\nlabel = \"Container\"\nmode = \"local_container\"\n", "image is required"),
        ("mode-changed", "[profiles.container]\nlabel = \"Container\"\nmode = \"native\"\n", "no longer matches"),
        ("image-changed", "[profiles.container]\nlabel = \"Container\"\nmode = \"local_container\"\nimage = \"spark-worker:new\"\n", "no longer matches"),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = settings(temp.path());
        std::fs::create_dir_all(&settings.config_dir).expect("config dir");
        std::fs::write(settings.config_dir.join("execution-profiles.toml"), config)
            .expect("profiles");
        let run_id = format!("retry-{case}");
        seed_retry(&settings, &run_id);
        let (service, commands) = service_with_fake_docker(settings.clone());
        let response = service.retry_pipeline_route(&run_id);
        assert_eq!(response.body["status"], json!("started"));
        wait_for_status(&settings, &run_id, "failed");
        let record = RunStore::for_settings(&settings)
            .read_run_bundle(&run_id)
            .expect("bundle")
            .expect("run")
            .record
            .expect("record");
        assert!(record.last_error.contains(expected), "{}", record.last_error);
        assert!(commands.lock().expect("commands").is_empty());
    }
}

fn settings(root: &std::path::Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("project"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("spark-home/flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
