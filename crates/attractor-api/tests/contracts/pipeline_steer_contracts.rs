use attractor_api::{AttractorApiService, ContinuePipelineRequest, PipelineStartRequest};
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
        // The fake agents' steered turns end with a decision envelope.
        .env("SPARK_FAKE_FINAL_ANSWER", r#"{"decision":"done"}"#)
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

/// The latest transcript segment for the message `text` sent into `node_id`'s turn.
fn sent_message(settings: &SparkSettings, run_id: &str, node_id: &str, text: &str) -> Value {
    let bundle = RunStore::for_settings(settings)
        .read_run_bundle(run_id)
        .unwrap()
        .unwrap();
    let mut latest = Value::Null;
    let mut dirs = vec![bundle.paths.root.join("logs").join(node_id)];
    while let Some(dir) = dirs.pop() {
        for path in fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
        {
            if path.is_dir() {
                dirs.push(path);
            } else if path.ends_with("transcript.jsonl") {
                for record in fs::read_to_string(&path).unwrap().lines() {
                    let segment = &serde_json::from_str::<Value>(record).unwrap()["segment"];
                    if segment["kind"] == "user_message" && segment["content"] == text {
                        latest = segment.clone();
                    }
                }
            }
        }
    }
    latest
}

fn content(settings: &SparkSettings, flow: Value) -> PipelineStartRequest {
    PipelineStartRequest {
        flow_name: Some(crate::install_flow(settings, flow)),
        ..Default::default()
    }
}

fn agent_flow(id: &str) -> Value {
    json!({
        "schema_version": "2",
        "id": id,
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "work", "description": "Enter the first workflow step."}
                }
            },
            "work": {
                "kind": "agent_task",
                "config": {"kind": "agent_task", "prompt": "Do the work"},
                "contracts": {"response": "decision_envelope"},
                "routes": {"done": {"to": "done", "description": "The step is complete."}}
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
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
        content(&settings, agent_flow("steer_codex")),
    );
    wait_for_log(&log, "turn/start", 1);

    let result = steer(
        &service,
        "steer-codex",
        json!({"message": "Also update the docs."}),
    );
    assert_eq!(result["status"], "delivered", "{result}");
    assert_eq!(result["delivery_mode"], "live_turn");
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
    let sent = sent_message(&settings, "steer-codex", "work", "Also update the docs.");
    assert_eq!(sent["status"], "complete", "{sent}");
}

#[test]
fn steer_delivers_to_active_claude_code_agent_node() {
    let log = std::env::temp_dir().join(format!("spark-steer-{}-claude.log", std::process::id()));
    let envs = [
        (
            "SPARK_CLAUDE_CODE_BIN",
            fake_bin("spark-agent-fake-claude-code"),
        ),
        (
            "SPARK_FAKE_CLAUDE_CODE_MODE",
            "mid-turn-message".to_string(),
        ),
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
        content(&settings, agent_flow("steer_claude")),
    );
    // The CLI logs the prompt before it sends system/init, and the session
    // takes messages only after init; its reply follows init on the stream.
    wait(|| node_log_contains(&settings, "steer-claude", "work", "Working."));

    let result = steer(
        &service,
        "steer-claude",
        json!({"message": "Also update the docs."}),
    );
    assert_eq!(result["status"], "delivered", "{result}");
    assert_eq!(result["delivery_mode"], "live_turn");
    assert_eq!(result["target_node_id"], "work");

    wait_for_status(&settings, "steer-claude", "completed");
    let log = fs::read_to_string(&log).unwrap();
    assert!(log.contains("MID_TURN=\"Also update the docs.\""), "{log}");
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
    let sent = sent_message(&settings, "steer-claude", "work", "Also update the docs.");
    assert_eq!(sent["status"], "complete", "{sent}");
}

/// A steer the Claude Code turn never read stays in the node's transcript,
/// marked not sent.
#[test]
fn steer_a_claude_code_turn_never_reads_shows_as_not_sent() {
    let log = std::env::temp_dir().join(format!(
        "spark-steer-{}-claude-unread.log",
        std::process::id()
    ));
    let envs = [
        (
            "SPARK_CLAUDE_CODE_BIN",
            fake_bin("spark-agent-fake-claude-code"),
        ),
        ("SPARK_FAKE_CLAUDE_CODE_MODE", "mid-turn-stop".to_string()),
        ("SPARK_FAKE_CLAUDE_CODE_LOG", log.to_string_lossy().into()),
    ];
    if !in_child_process(
        "steer_a_claude_code_turn_never_reads_shows_as_not_sent",
        &envs,
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CLAUDE_CODE_LOG");
    start(
        &service,
        &settings,
        "steer-claude-unread",
        "claude-code",
        content(&settings, agent_flow("steer_claude_unread")),
    );
    // The CLI logs the prompt before it sends system/init, and the session
    // takes messages only after init; its reply follows init on the stream.
    wait(|| node_log_contains(&settings, "steer-claude-unread", "work", "Working."));

    let result = steer(
        &service,
        "steer-claude-unread",
        json!({"message": "Too late for this turn."}),
    );
    assert_eq!(result["status"], "delivered", "{result}");
    assert_eq!(result["message"], "Sent to the running Claude Code turn.");
    wait_for_log(&log, "MID_TURN=", 1);
    // Canceling interrupts the turn before Claude reads the message.
    let cancel = service.dispatch(
        "POST",
        "/attractor/pipelines/steer-claude-unread/cancel",
        "{}",
    );
    assert_eq!(cancel.status_code, 200, "{}", cancel.body);
    wait_for_status(&settings, "steer-claude-unread", "canceled");

    let unread = sent_message(
        &settings,
        "steer-claude-unread",
        "work",
        "Too late for this turn.",
    );
    assert_eq!(unread["status"], "failed", "{unread}");
    assert_eq!(
        unread["error"],
        "Not sent: the turn ended before the agent read it."
    );
}

#[test]
fn steer_targets_agent_node_of_child_run() {
    let envs = codex_env("child");
    if !in_child_process("steer_targets_agent_node_of_child_run", &envs) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    assert_eq!(
        crate::install_flow(&settings, agent_flow("steer_child")),
        "tests/steer-child"
    );
    let parent = crate::install_flow(
        &settings,
        json!({
            "schema_version": "2",
            "id": "steer_parent",
            "nodes": {
                "start": {
                    "kind": "start",
                    "config": {"kind": "start"},
                    "routes": {
                        "next": {
                            "to": "branch",
                            "description": "Enter the first workflow step."
                        }
                    }
                },
                "branch": {
                    "kind": "subflow",
                    "config": {
                        "kind": "subflow",
                        "flow_ref": "steer-child",
                        "results": {"completed": "done"}
                    },
                    "routes": {"done": {"to": "done", "description": "The step is complete."}}
                },
                "done": {
                    "kind": "exit",
                    "config": {
                        "kind": "exit",
                        "result": {"name": "completed", "outcome": "success"}
                    }
                }
            }
        }),
    );
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CODEX_APP_SERVER_LOG");
    start(
        &service,
        &settings,
        "steer-parent",
        "codex",
        PipelineStartRequest {
            flow_name: Some(parent),
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
            &settings,
            json!({
                "schema_version": "2",
                "id": "steer_fork",
                "nodes": {
                    "start": {
                        "kind": "start",
                        "config": {"kind": "start"},
                        "routes": {
                            "next": {
                                "to": "fork",
                                "description": "Enter the first workflow step."
                            }
                        }
                    },
                    "fork": {
                        "kind": "parallel",
                        "config": {"kind": "parallel", "branches": ["left", "right"], "join": "join"}
                    },
                    "left": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "Left work"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {
                            "done": {"to": "join", "description": "The step is complete."}
                        }
                    },
                    "right": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "Right work"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {
                            "done": {"to": "join", "description": "The step is complete."}
                        }
                    },
                    "join": {
                        "kind": "fan_in",
                        "config": {"kind": "fan_in"},
                        "routes": {
                            "next": {
                                "to": "done",
                                "description": "All branches settled; continue."
                            }
                        }
                    },
                    "done": {
                        "kind": "exit",
                        "config": {
                            "kind": "exit",
                            "result": {"name": "completed", "outcome": "success"}
                        }
                    }
                }
            }),
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
    // Both branches completed, so the fork enters its join once.
    wait_for_status(&settings, "steer-fork", "completed");
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .matches("turn/start")
            .count(),
        2
    );
    let statuses = intervention_events(&settings, "steer-fork")
        .iter()
        .map(|event| event["intervention_status"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        [json!("rejected"), json!("delivered"), json!("delivered")]
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
            &settings,
            json!({
                "schema_version": "2",
                "id": "steer_chain",
                "nodes": {
                    "start": {
                        "kind": "start",
                        "config": {"kind": "start"},
                        "routes": {
                            "next": {
                                "to": "fork",
                                "description": "Enter the first workflow step."
                            }
                        }
                    },
                    "fork": {
                        "kind": "parallel",
                        "config": {"kind": "parallel", "branches": ["first"], "join": "join"}
                    },
                    "first": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "First work"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {
                            "done": {"to": "second", "description": "The step is complete."}
                        }
                    },
                    "second": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "Second work"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {
                            "done": {"to": "join", "description": "The step is complete."}
                        }
                    },
                    "join": {
                        "kind": "fan_in",
                        "config": {"kind": "fan_in"},
                        "routes": {
                            "next": {
                                "to": "done",
                                "description": "All branches settled; continue."
                            }
                        }
                    },
                    "done": {
                        "kind": "exit",
                        "config": {
                            "kind": "exit",
                            "result": {"name": "completed", "outcome": "success"}
                        }
                    }
                }
            }),
        ),
    );
    // Each steer ends the fake's open turn, so `first` completes and the
    // branch moves on to `second`, whose `done` reaches the join.
    for (node, count) in [("first", 1), ("second", 2)] {
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
        flow_name: Some(crate::install_flow(
            &settings,
            json!({
                "schema_version": "2",
                "id": "idle",
                "nodes": {
                    "start": {
                        "kind": "start",
                        "config": {"kind": "start"},
                        "routes": {
                            "next": {
                                "to": "done",
                                "description": "Enter the first workflow step."
                            }
                        }
                    },
                    "done": {
                        "kind": "exit",
                        "config": {
                            "kind": "exit",
                            "result": {"name": "completed", "outcome": "success"}
                        }
                    }
                }
            }),
        )),
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
            &settings,
            json!({
                "schema_version": "2",
                "id": "steer_mixed",
                "nodes": {
                    "start": {
                        "kind": "start",
                        "config": {"kind": "start"},
                        "routes": {
                            "next": {
                                "to": "work",
                                "description": "Enter the first workflow step."
                            }
                        }
                    },
                    "work": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "Do the work"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {
                            "done": {"to": "other", "description": "The step is complete."}
                        }
                    },
                    "other": {
                        "kind": "agent_task",
                        "config": {"kind": "agent_task", "prompt": "Other work"},
                        "contracts": {"response": "decision_envelope"},
                        "routes": {
                            "done": {"to": "done", "description": "The step is complete."}
                        }
                    },
                    "done": {
                        "kind": "exit",
                        "config": {
                            "kind": "exit",
                            "result": {"name": "completed", "outcome": "success"}
                        }
                    }
                }
            }),
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
            flow_definition_json: Some(
                json!({
                    "schema_version": "2",
                    "id": "steer_unsupported",
                    "nodes": {
                        "start": {
                            "kind": "start",
                            "config": {"kind": "start"},
                            "routes": {
                                "next": {
                                    "to": "fork",
                                    "description": "Enter the first workflow step."
                                }
                            }
                        },
                        "fork": {
                            "kind": "parallel",
                            "config": {"kind": "parallel", "branches": ["left", "right"], "join": "join"}
                        },
                        "left": {
                            "kind": "agent_task",
                            "config": {"kind": "agent_task", "prompt": "Left work"},
                            "contracts": {"response": "decision_envelope"},
                            "routes": {
                                "done": {"to": "join", "description": "The step is complete."}
                            }
                        },
                        "right": {
                            "kind": "agent_task",
                            "config": {"kind": "agent_task", "prompt": "Right work"},
                            "contracts": {"response": "decision_envelope"},
                            "routes": {
                                "done": {"to": "join", "description": "The step is complete."}
                            }
                        },
                        "join": {
                            "kind": "fan_in",
                            "config": {"kind": "fan_in"},
                            "routes": {
                                "next": {
                                    "to": "done",
                                    "description": "All branches settled; continue."
                                }
                            }
                        },
                        "done": {
                            "kind": "exit",
                            "config": {
                                "kind": "exit",
                                "result": {"name": "completed", "outcome": "success"}
                            }
                        }
                    }
                })
                .to_string(),
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

/// Runs `run` to `status` and returns each logged CLI launch's `--effort`.
fn launched_efforts(
    log: &Path,
    settings: &SparkSettings,
    run_id: &str,
    status: &str,
    run: impl FnOnce(),
) -> Vec<String> {
    let _ = fs::remove_file(log);
    run();
    wait_for_status(settings, run_id, status);
    // Parallel launches can interleave log entries, but each argument list is
    // written whole, so `--effort` stays next to its value.
    let log = fs::read_to_string(log).unwrap_or_default();
    let args: Vec<_> = log.lines().collect();
    args.windows(2)
        .filter(|pair| pair[0] == "--effort")
        .map(|pair| pair[1].to_string())
        .collect()
}

/// An agent flow declaring `defaults.reasoning_effort = xhigh`, routed by the
/// fake CLI's structured `approve` decision.
fn xhigh_flow(id: &str) -> Value {
    let mut flow = agent_flow(id);
    flow["defaults"] = json!({"reasoning_effort": "xhigh"});
    flow["nodes"]["work"]["routes"] =
        json!({"approve": {"to": "done", "description": "The step is complete."}});
    flow
}

#[test]
fn flow_default_effort_reaches_the_cli_on_every_launch_path() {
    let log = std::env::temp_dir().join(format!("spark-effort-{}.log", std::process::id()));
    let envs = [
        (
            "SPARK_CLAUDE_CODE_BIN",
            fake_bin("spark-agent-fake-claude-code"),
        ),
        ("SPARK_FAKE_CLAUDE_CODE_MODE", "structured".to_string()),
        ("SPARK_FAKE_CLAUDE_CODE_LOG", log.to_string_lossy().into()),
    ];
    if !in_child_process(
        "flow_default_effort_reaches_the_cli_on_every_launch_path",
        &envs,
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    fs::write(
        settings.config_dir.join("spark.toml"),
        "[models]\nprovider = \"claude-code\"\nreasoning_effort = \"minimal\"\n",
    )
    .unwrap();
    let service = service(&settings);
    let log = log_path("SPARK_FAKE_CLAUDE_CODE_LOG");
    let launch = |run_id: &str, request: PipelineStartRequest| {
        launched_efforts(&log, &settings, run_id, "completed", || {
            start(&service, &settings, run_id, "claude-code", request)
        })
    };
    let flow = crate::install_flow(&settings, xhigh_flow("effort_flow"));
    let from_flow = |request| PipelineStartRequest {
        flow_name: Some(flow.clone()),
        ..request
    };

    // Flow default beats the project default and reaches a node without effort.
    assert_eq!(
        launch("effort-default", from_flow(Default::default())),
        ["xhigh"]
    );
    let record = RunStore::for_settings(&settings)
        .read_run_bundle("effort-default")
        .unwrap()
        .unwrap()
        .record
        .unwrap();
    assert_eq!(record.reasoning_effort.as_deref(), Some("xhigh"));

    // Explicit request and launch context beat the flow default.
    assert_eq!(
        launch(
            "effort-request",
            from_flow(PipelineStartRequest {
                reasoning_effort: Some("low".into()),
                launch_context: Some(
                    [(
                        unified_llm_adapter::RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
                        json!("medium"),
                    )]
                    .into()
                ),
                ..Default::default()
            })
        ),
        ["low"]
    );
    assert_eq!(
        launch(
            "effort-context",
            from_flow(PipelineStartRequest {
                launch_context: Some(
                    [(
                        unified_llm_adapter::RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
                        json!("medium"),
                    )]
                    .into()
                ),
                ..Default::default()
            })
        ),
        ["medium"]
    );

    // A node's own effort beats all of them.
    let mut own_effort = xhigh_flow("effort_node");
    own_effort["nodes"]["work"]["execution"] = json!({"reasoning_effort": "high"});
    assert_eq!(
        launch(
            "effort-node",
            PipelineStartRequest {
                reasoning_effort: Some("low".into()),
                ..content(&settings, own_effort)
            }
        ),
        ["high"]
    );

    // Without a flow default, the project default still applies.
    let mut no_default = xhigh_flow("effort_project");
    no_default.as_object_mut().unwrap().remove("defaults");
    assert_eq!(
        launch("effort-project", content(&settings, no_default)),
        ["minimal"]
    );

    // Parallel branches and subflows inherit the parent's resolved effort.
    let mut child = xhigh_flow("effort_child");
    child.as_object_mut().unwrap().remove("defaults");
    crate::install_flow(&settings, child);
    let branch = json!({
        "kind": "agent_task",
        "config": {"kind": "agent_task", "prompt": "Branch work"},
        "contracts": {"response": "decision_envelope"},
        "routes": {"approve": {"to": "join", "description": "The step is complete."}}
    });
    let nested = json!({
        "schema_version": "2",
        "id": "effort_nested",
        "defaults": {"reasoning_effort": "xhigh"},
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {"next": {"to": "fork", "description": "Enter the first workflow step."}}
            },
            "fork": {
                "kind": "parallel",
                "config": {"kind": "parallel", "branches": ["left", "right"], "join": "join"}
            },
            "left": branch,
            "right": branch,
            "join": {
                "kind": "fan_in",
                "config": {"kind": "fan_in"},
                "routes": {"next": {"to": "child", "description": "All branches settled; continue."}}
            },
            "child": {
                "kind": "subflow",
                "config": {
                    "kind": "subflow",
                    "flow_ref": "effort-child",
                    "results": {"completed": "done"}
                },
                "routes": {"done": {"to": "done", "description": "The step is complete."}}
            },
            "done": {
                "kind": "exit",
                "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}
            }
        }
    });
    assert_eq!(
        launch("effort-nested", content(&settings, nested)),
        ["xhigh", "xhigh", "xhigh"]
    );

    // Continue keeps the resolved effort unless the request replaces it.
    for (run_id, requested, expected) in [
        ("effort-continue", None, "xhigh"),
        ("effort-continue-override", Some("low"), "low"),
    ] {
        let _ = fs::remove_file(&log);
        let response = service.continue_pipeline_route(
            "effort-default",
            ContinuePipelineRequest {
                start_node: "work".into(),
                flow_source_mode: "snapshot".into(),
                reasoning_effort: requested.map(Into::into),
                ..Default::default()
            },
        );
        assert_eq!(response.body["status"], "started", "{run_id}: {response:?}");
        let new_run = response.body["run_id"].as_str().unwrap();
        wait_for_status(&settings, new_run, "completed");
        assert!(
            fs::read_to_string(&log)
                .unwrap()
                .contains(&format!("--effort\n{expected}\n")),
            "{run_id}"
        );
    }

    // Retry resumes a failed run with the effort it captured at launch.
    let mut retried = xhigh_flow("effort_retry");
    retried["nodes"]["start"]["routes"]["next"]["to"] = json!("gate");
    retried["nodes"]["gate"] = json!({
        "kind": "tool",
        "config": {"kind": "tool", "command": "test -f retry-ok", "exit_codes": {"0": "done"}},
        "routes": {"done": {"to": "work", "description": "The gate passed."}}
    });
    let failed = launched_efforts(&log, &settings, "effort-retry", "failed", || {
        start(
            &service,
            &settings,
            "effort-retry",
            "claude-code",
            content(&settings, retried),
        )
    });
    assert!(failed.is_empty(), "{failed:?}");
    fs::write(settings.project_root.join("retry-ok"), "").unwrap();
    // The failed status lands just before the executor releases the run.
    wait(|| service.retry_pipeline_route("effort-retry").status_code != 409);
    wait_for_status(&settings, "effort-retry", "completed");
    assert!(fs::read_to_string(&log)
        .unwrap()
        .contains("--effort\nxhigh\n"));
}
