use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use attractor_core::{ContextMap, OutcomeStatus};
use attractor_dsl::parse_dot;
use attractor_execution::{
    CommandResult, CommandSpec, ContainerCommandRunner, ContainerizedNodeExecutor, ExecutionMode,
    ExecutionProfile, ExecutionProfileSelection,
};
use attractor_runtime::{
    outgoing_routing_edges, NodeExecutionRequest, NodeExecutor, RuntimeHandlerRunner,
};
use serde_json::json;

#[derive(Clone, Default)]
struct FakeRunner {
    commands: Arc<Mutex<Vec<CommandSpec>>>,
    docker_exists: bool,
    rm_fails: bool,
}

impl ContainerCommandRunner for FakeRunner {
    fn command_exists(&self, program: &str) -> bool {
        program == "docker" && self.docker_exists
    }

    fn run(&mut self, spec: CommandSpec) -> std::io::Result<CommandResult> {
        self.commands.lock().expect("commands").push(spec.clone());
        if spec.args.first().map(String::as_str) == Some("run") {
            return Ok(CommandResult {
                exit_code: 0,
                stdout: "container-123\n".to_string(),
                stderr: String::new(),
            });
        }
        if spec.args.first().map(String::as_str) == Some("exec") {
            assert!(spec.stdin.contains("\"node_id\":\"work\""));
            return Ok(CommandResult {
                exit_code: 0,
                stdout: serde_json::to_string(&json!({
                    "type": "result",
                    "outcome": {
                        "status": "success",
                        "preferred_label": "",
                        "suggested_next_ids": [],
                        "context_updates": {"context.container": "ok"},
                        "failure_reason": "",
                        "notes": "container done",
                        "retryable": null,
                        "raw_response_text": ""
                    },
                    "context": {"context.worker": "returned"}
                }))
                .expect("json")
                    + "\n",
                stderr: String::new(),
            });
        }
        if spec.args.first().map(String::as_str) == Some("rm") {
            return Ok(CommandResult {
                exit_code: if self.rm_fails { 1 } else { 0 },
                stdout: String::new(),
                stderr: if self.rm_fails {
                    "cleanup denied\n".to_string()
                } else {
                    String::new()
                },
            });
        }
        Ok(CommandResult::default())
    }
}

#[test]
fn local_container_executor_uses_docker_worker_protocol_and_cleanup() {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let fake = FakeRunner {
        commands: commands.clone(),
        docker_exists: true,
        rm_fails: false,
    };
    let mut executor =
        ContainerizedNodeExecutor::new(container_selection(), RuntimeHandlerRunner::new())
            .with_command_runner(fake);

    let outcome = executor.execute(request()).expect("container outcome");

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(outcome.notes, "container done");
    assert_eq!(outcome.context_updates["context.container"], json!("ok"));
    assert_eq!(outcome.context_updates["context.worker"], json!("returned"));

    let commands = commands.lock().expect("commands");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].args[0], "run");
    assert!(commands[0].args.contains(&"--label".to_string()));
    assert!(commands[0]
        .args
        .contains(&"spark.execution_mode=local_container".to_string()));
    assert_eq!(
        &commands[1].args[..5],
        ["exec", "-i", "container-123", "spark-server", "worker"]
    );
    assert_eq!(commands[1].args[5], "run-node");
    assert_eq!(&commands[2].args[..3], ["rm", "-f", "container-123"]);
}

#[test]
fn missing_docker_is_nonretryable_runtime_failure() {
    let mut executor =
        ContainerizedNodeExecutor::new(container_selection(), RuntimeHandlerRunner::new())
            .with_command_runner(FakeRunner {
                docker_exists: false,
                ..FakeRunner::default()
            });

    let error = executor.execute(request()).unwrap_err();

    assert_eq!(
        error.message,
        "Container execution requires Docker, but the docker CLI was not found."
    );
    assert_eq!(error.retryable, Some(false));
}

#[test]
fn cleanup_errors_are_available_to_pipeline_executor() {
    let mut executor =
        ContainerizedNodeExecutor::new(container_selection(), RuntimeHandlerRunner::new())
            .with_command_runner(FakeRunner {
                docker_exists: true,
                rm_fails: true,
                ..FakeRunner::default()
            });

    let outcome = executor
        .execute(request())
        .expect("outcome despite cleanup");

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(
        executor.take_cleanup_error().as_deref(),
        Some("cleanup denied")
    );
}

fn container_selection() -> ExecutionProfileSelection {
    ExecutionProfileSelection {
        selected_profile_id: "container".to_string(),
        selection_source: "explicit".to_string(),
        profile: ExecutionProfile {
            id: "container".to_string(),
            label: "Container".to_string(),
            mode: ExecutionMode::LocalContainer,
            enabled: true,
            image: Some("spark-worker:compat".to_string()),
            capabilities: vec!["filesystem".to_string()],
            metadata: BTreeMap::new(),
        },
    }
}

fn request() -> NodeExecutionRequest {
    let graph = parse_dot(
        r#"
        digraph G {
          work [shape=box]
        }
        "#,
    )
    .expect("dot parses");
    let node = graph.nodes["work"].clone();
    NodeExecutionRequest {
        node_id: "work".to_string(),
        stage_index: 0,
        context: ContextMap::from([("context.input".to_string(), json!("value"))]),
        prompt: String::new(),
        node,
        outgoing_edges: outgoing_routing_edges(&graph, "work").expect("edges"),
        graph,
        run_paths: None,
        run_workdir: std::env::current_dir().expect("cwd"),
        run_id: "run-container".to_string(),
    }
}
