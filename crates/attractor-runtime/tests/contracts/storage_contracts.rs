use std::collections::BTreeMap;

use attractor_core::{
    CheckpointState, ExitOutcome, ExitResult, FlowDefinition, FlowNode, FlowRoute, NodeConfig,
    NodeContracts, NodeKind, RawRuntimeEvent, RunManifest, RunRecord,
};
use attractor_runtime::{
    checkpoint_saved_event, log_event, stage_completed_event, stage_started_event,
    stage_transition_event, CreateRunRequest, NodeArtifacts, RunRootPaths, RunStore, RunsRoot,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use spark_storage::{read_json, write_json_atomic, JsonWriteOptions};

fn store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_root(attractor_runtime::RunsRoot::Home(
        temp.path().join("spark-home"),
    ))
}

fn checkpoint(current_node: &str, completed_nodes: &[&str]) -> CheckpointState {
    CheckpointState {
        timestamp: "2026-06-22T17:40:17Z".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: completed_nodes
            .iter()
            .map(|value| value.to_string())
            .collect(),
        context: BTreeMap::from([(
            "_attractor.node_outcomes".to_string(),
            json!({
                "start": {"status": "completed", "decision": "next"},
                "work": {"status": "completed", "decision": "done"}
            }),
        )]),
        retry_counts: BTreeMap::new(),
        visit_counts: Default::default(),
        logs: Vec::new(),
    }
}

fn record(run_id: &str, project_path: &str) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path);
    record.flow_name = "compat-flow".to_string();
    record.model = "compat-model".to_string();
    record.started_at = "2026-06-22T17:40:17Z".to_string();
    record.execution_profile_id = Some("native".to_string());
    record.execution_profile_capabilities = Some(json!({}));
    record
}

#[test]
fn create_run_writes_current_run_root_layout_and_initial_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project One");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let record = record("run-create", &project_path.to_string_lossy());
    let checkpoint = checkpoint("start", &[]);
    let manifest = RunManifest {
        goal: "Create layout".to_string(),
        graph_id: "CompatGraph".to_string(),
        start_node: "start".to_string(),
        started_at: record.started_at.clone(),
        extra: BTreeMap::new(),
    };

    let flow = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "create",
        "nodes": {
            "start": { "kind": "start", "routes": { "next": { "to": "done", "description": "Finish." } } },
            "done": { "kind": "exit", "config": { "kind": "exit", "result": { "name": "completed", "outcome": "success" } } }
        }
    }))
    .expect("flow");
    let captured_definition = flow.to_canonical_json_string();
    let paths = store
        .create_run(CreateRunRequest {
            record: record.clone(),
            checkpoint: Some(checkpoint.clone()),
            manifest: Some(manifest),
            flow_definition_json: Some(captured_definition.clone()),
            flow_files: None,
        })
        .expect("create run");

    assert_eq!(
        paths.root,
        temp.path()
            .join("spark-home/projects")
            .join(&paths.project_id)
            .join("runs/run-create")
    );
    for path in [
        paths.run_json(),
        paths.events_jsonl(),
        paths.checkpoint_json(),
        paths.manifest_json(),
        paths.result_json(),
        paths.result_markdown(),
        paths.artifacts_dir().join("flow/flow-definition.json"),
    ] {
        assert!(path.exists(), "{} should exist", path.display());
    }
    let mut flow_artifacts = std::fs::read_dir(paths.artifacts_dir().join("flow"))
        .expect("flow artifacts")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("name")
        })
        .collect::<Vec<_>>();
    flow_artifacts.sort();
    assert_eq!(
        flow_artifacts,
        ["flow-definition.json"],
        "runs capture their flow as JSON only"
    );
    assert_eq!(
        store.read_graph_source(&paths).expect("read graph source"),
        Some(flow.normalize()),
        "the captured flow-definition.json is the run's graph source"
    );
    assert!(paths.logs_dir().is_dir());
    assert!(paths.logs_dir().join("artifacts").is_dir());
    assert!(paths.artifacts_dir().is_dir());
    assert!(paths.result_dir().is_dir());
    for duplicate in [
        "state.json",
        "logs/checkpoint.json",
        "logs/manifest.json",
        "run.log",
    ] {
        assert!(
            !paths.root.join(duplicate).exists(),
            "{duplicate} should not be written"
        );
    }

    let loaded = store
        .read_run_record(&paths)
        .expect("read run record")
        .expect("run record");
    assert_eq!(loaded.run_id, "run-create");
    assert_eq!(
        loaded.effective_flow_hash.as_deref(),
        Some(format!("{:x}", Sha256::digest(captured_definition.as_bytes())).as_str()),
        "the durable hash must describe the exact captured flow-definition.json"
    );
    assert_eq!(loaded.provider, "codex");
    assert_eq!(loaded.llm_provider, "codex");

    let events = store.read_raw_events(&paths).expect("events");
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].sequence, Some(1));
    assert_eq!(events[0].event_type, "lifecycle");
    assert_eq!(events[1].event_type, "runtime");
    assert_eq!(events[2].event_type, "run_meta");

    let initial_result = store
        .read_result(&paths)
        .expect("read result")
        .expect("initial result");
    assert_eq!(initial_result.state, "pending");
    assert_eq!(initial_result.run_id, "run-create");
}

#[test]
fn create_run_uses_normalized_record_for_initial_events_and_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Normalize");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let mut record = record("run-normalized", &project_path.to_string_lossy());
    record.status = "success".to_string();
    record.provider = "openai".to_string();
    record.llm_provider = String::new();

    let paths = store
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint("done", &["start"])),
            ..CreateRunRequest::default()
        })
        .expect("create run");

    let raw_run: serde_json::Value = read_json(paths.run_json()).expect("run json");
    assert_eq!(raw_run["status"], "completed");
    assert_eq!(raw_run["provider"], "openai");
    assert_eq!(raw_run["llm_provider"], "openai");

    let events = store.read_raw_events(&paths).expect("events");
    assert_eq!(events[1].payload["status"], "completed");
    assert_eq!(events[2].payload["status"], "completed");
    assert_eq!(events[2].payload["provider"], "openai");
    assert_eq!(events[2].payload["llm_provider"], "openai");

    let journal = store.read_journal(&paths).expect("journal");
    assert_eq!(journal[0].payload["status"], "completed");
    assert_eq!(journal[0].payload["provider"], "openai");
    assert_eq!(journal[0].payload["llm_provider"], "openai");
    assert_eq!(journal[1].payload["status"], "completed");

    let result = store
        .read_result(&paths)
        .expect("read result")
        .expect("result");
    assert_eq!(result.status, "completed");
}

#[test]
fn run_record_round_trips_nulls_aliases_and_unknown_python_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Two");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-record")
        .expect("run root");
    std::fs::create_dir_all(&paths.root).expect("run root dir");
    write_json_atomic(
        paths.run_json(),
        &json!({
            "run_id": "run-record",
            "flow_name": "legacy",
            "status": "success",
            "provider": "openai",
            "working_directory": project_path,
            "model": "gpt",
            "started_at": "2026-06-22T17:40:17Z",
            "python_extra": {"kept": true}
        }),
        JsonWriteOptions::default(),
    )
    .expect("seed legacy run json");

    let mut loaded = store
        .read_run_record(&paths)
        .expect("read record")
        .expect("record");
    assert_eq!(loaded.status, "completed");
    assert_eq!(loaded.llm_provider, "openai");
    assert_eq!(loaded.launch_context, None);
    loaded.outcome = Some("success".to_string());
    loaded.llm_profile = None;
    loaded.launch_context = Some(
        [("context.topic".to_string(), json!("legacy"))]
            .into_iter()
            .collect(),
    );
    store
        .write_run_record(&paths, &loaded)
        .expect("rewrite record");

    let raw: serde_json::Value = read_json(paths.run_json()).expect("run json");
    assert_eq!(raw["python_extra"], json!({"kept": true}));
    assert_eq!(raw["provider"], "openai");
    assert_eq!(raw["llm_provider"], "openai");
    assert!(raw["llm_profile"].is_null());
    assert!(raw["continued_from_run_id"].is_null());
    assert_eq!(raw["launch_context"], json!({"context.topic": "legacy"}));
}

#[test]
fn raw_events_append_and_normalize_to_newest_first_journal_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Three");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-events", &project_path.to_string_lossy()),
            checkpoint: Some(checkpoint("start", &[])),
            ..CreateRunRequest::default()
        })
        .expect("create run");

    store
        .append_event(&paths, stage_started_event("run-events", 0, "work"))
        .expect("stage started");
    store
        .append_event(&paths, log_event("run-events", "[work] running"))
        .expect("log event");
    store
        .append_event(
            &paths,
            stage_completed_event("run-events", 0, "work", "done"),
        )
        .expect("stage completed");
    store
        .append_event(
            &paths,
            stage_transition_event("run-events", 0, "work", "done", "review", 2),
        )
        .expect("stage transition");
    store
        .append_event(
            &paths,
            checkpoint_saved_event("run-events", "done", vec!["work".to_string()]),
        )
        .expect("checkpoint event");

    let events = store.read_raw_events(&paths).expect("events");
    assert_eq!(events.len(), 8);
    assert_eq!(events.last().and_then(|event| event.sequence), Some(8));

    let journal = store.read_journal(&paths).expect("journal");
    assert_eq!(journal[0].id, "journal-8");
    assert_eq!(journal[0].kind, "checkpoint");
    assert_eq!(journal[0].summary, "Checkpoint saved at done");
    assert_eq!(journal[1].summary, "work → review on done (visit 2)");
    assert_eq!(journal[2].summary, "Stage work completed");
    assert_eq!(journal[3].summary, "[work] running");
    assert_eq!(journal[4].summary, "Stage work started");
    assert_eq!(journal[7].summary, "Run created");
}

#[test]
fn event_append_sequences_survive_blank_and_partial_trailing_lines() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Tail");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-tail", &project_path.to_string_lossy()),
            checkpoint: Some(checkpoint("start", &[])),
            ..CreateRunRequest::default()
        })
        .expect("create run");

    let appended = store
        .append_event(&paths, log_event("run-tail", "[work] running"))
        .expect("append");
    let last_sequence = appended.sequence.expect("sequence");

    // An interrupted append can leave blank lines or a partial event line at
    // the end of the log; the next append must skip them and continue the
    // sequence instead of restarting or failing.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(paths.events_jsonl())
        .expect("open events");
    write!(file, "\n{{\"sequence\": \n").expect("write partial line");
    drop(file);

    let appended = store
        .append_event(&paths, log_event("run-tail", "[work] still running"))
        .expect("append after partial trailing line");
    assert_eq!(appended.sequence, Some(last_sequence + 1));
}

#[test]
fn raw_event_append_does_not_update_render_transcript() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Three");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-raw-only", &project_path.to_string_lossy()),
            checkpoint: Some(checkpoint("start", &[])),
            ..CreateRunRequest::default()
        })
        .expect("create run");

    store
        .append_event(&paths, stage_started_event("run-raw-only", 0, "work"))
        .expect("stage started");

    let events = store.read_raw_events(&paths).expect("events");
    assert_eq!(events.last().and_then(|event| event.sequence), Some(4));

    assert!(
        !paths.root.join("transcript.jsonl").exists(),
        "runs do not own transcripts"
    );
    assert!(store
        .list_node_executions(&paths)
        .expect("executions")
        .is_empty());
}

#[test]
fn checkpoint_is_written_and_read_only_at_the_run_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Four");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-checkpoint", &project_path.to_string_lossy()),
            checkpoint: Some(checkpoint("start", &[])),
            ..CreateRunRequest::default()
        })
        .expect("create run");

    store
        .save_checkpoint(&paths, &checkpoint("work", &["start"]))
        .expect("save checkpoint");
    let written: CheckpointState = read_json(paths.checkpoint_json()).expect("root checkpoint");
    assert_eq!(written.current_node, "work");
    assert_eq!(
        store
            .read_checkpoint(&paths)
            .expect("read checkpoint")
            .expect("checkpoint"),
        written
    );
    assert!(!paths.root.join("state.json").exists());
    assert!(!paths.logs_dir().join("checkpoint.json").exists());

    // The old copies are never consulted, even when they are all that remains.
    std::fs::remove_file(paths.checkpoint_json()).expect("remove root checkpoint");
    for old in [
        paths.root.join("state.json"),
        paths.logs_dir().join("checkpoint.json"),
    ] {
        write_json_atomic(old, &checkpoint("old", &[]), JsonWriteOptions::default())
            .expect("old checkpoint");
    }
    assert_eq!(
        store.read_checkpoint(&paths).expect("read checkpoint"),
        None
    );
}

#[test]
fn result_materialization_selects_successful_response_artifact_and_overlays_markdown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Five");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-result", &project_path.to_string_lossy()),
            checkpoint: Some(checkpoint("done", &["start", "work"])),
            ..CreateRunRequest::default()
        })
        .expect("create run");
    store
        .write_node_artifacts(
            &paths,
            "work",
            1,
            0,
            &NodeArtifacts {
                prompt: Some("Prompt".to_string()),
                response: Some("Raw response body\n".to_string()),
                status: Some(json!({"status": "completed", "decision": "done"})),
                under_logs: true,
            },
        )
        .expect("write node artifacts");
    let checkpoint = checkpoint("done", &["start", "work"]);
    let flow = result_flow(None);

    let result = store
        .materialize_result(&paths, "run-result", "completed", &flow, &checkpoint, None)
        .expect("materialize result");

    assert_eq!(result.state, "ready");
    assert_eq!(result.source_node_id.as_deref(), Some("work"));
    assert_eq!(
        result.source_artifact_path.as_deref(),
        Some("logs/work/executions/1-0/response.md")
    );
    assert_eq!(result.body_markdown, "Raw response body\n");
    assert_eq!(
        std::fs::read_to_string(paths.result_markdown()).expect("result md"),
        "Raw response body\n"
    );

    let metadata: serde_json::Value = read_json(paths.result_json()).expect("result json");
    assert_eq!(metadata["source_node_id"], "work");
    assert!(
        metadata.get("body_markdown").is_none(),
        "result.json stores metadata only: {metadata}"
    );

    std::fs::write(paths.result_markdown(), "Edited body\n").expect("edit markdown");
    let reread = store
        .read_result(&paths)
        .expect("read result")
        .expect("result");
    assert_eq!(reread.body_markdown, "Edited body\n");

    std::fs::remove_file(paths.result_markdown()).expect("remove markdown");
    let error = store
        .read_result(&paths)
        .expect_err("missing result.md is an error");
    assert!(error.to_string().contains("result.md"), "{error}");
}

#[test]
fn repeated_visits_and_retries_preserve_separate_execution_attempts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Repeated Executions");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-repeated", &project_path.to_string_lossy()),
            ..Default::default()
        })
        .expect("run");
    for (stage, attempt, response) in [(1, 0, "visit one"), (2, 0, "failed"), (2, 1, "retry")] {
        store
            .write_node_artifacts(
                &paths,
                "work",
                stage,
                attempt,
                &NodeArtifacts {
                    response: Some(response.to_string()),
                    status: Some(if attempt == 0 && stage == 2 {
                        json!({"status": "error", "decision": "", "failure_kind": "runtime"})
                    } else {
                        json!({"status": "completed", "decision": "done"})
                    }),
                    under_logs: true,
                    ..Default::default()
                },
            )
            .expect("execution artifacts");
    }
    let identities = store
        .list_node_executions(&paths)
        .expect("executions")
        .into_iter()
        .map(|execution| (execution.stage_index, execution.attempt))
        .collect::<Vec<_>>();
    assert_eq!(identities, vec![(1, 0), (2, 0), (2, 1)]);
    assert_eq!(
        std::fs::read_to_string(paths.logs_dir().join("work/executions/2-0/response.md"))
            .expect("failed response"),
        "failed"
    );
}

#[test]
fn artifact_listing_is_relative_viewable_and_excludes_internal_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Six");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-artifacts", &project_path.to_string_lossy()),
            checkpoint: Some(checkpoint("work", &["start"])),
            manifest: Some(RunManifest {
                goal: "Artifacts".to_string(),
                graph_id: "Artifacts".to_string(),
                start_node: "start".to_string(),
                started_at: "2026-06-22T17:40:17Z".to_string(),
                extra: BTreeMap::new(),
            }),
            flow_definition_json: Some(
                "{\"schema_version\":\"2\",\"id\":\"artifacts\"}\n".to_string(),
            ),
            ..CreateRunRequest::default()
        })
        .expect("create run");
    store
        .write_node_artifacts(
            &paths,
            "work",
            1,
            0,
            &NodeArtifacts {
                prompt: Some("Prompt".to_string()),
                response: Some("Response".to_string()),
                status: Some(json!({"status": "completed", "decision": "done"})),
                under_logs: true,
            },
        )
        .expect("node artifacts");
    std::fs::write(
        paths.logs_dir().join("work/initial-context.txt"),
        "captured request",
    )
    .expect("initial context");
    std::fs::write(paths.logs_dir().join("work/notes.yaml"), "a: 1\n").expect("yaml artifact");
    let mut capture_event = RawRuntimeEvent::new("CodergenAdapter", "run-artifacts");
    capture_event.payload.extend(BTreeMap::from([
        ("node_id".to_string(), json!("work")),
        (
            "adapter_event_type".to_string(),
            json!("codex_app_server_request_completed"),
        ),
        (
            "payload".to_string(),
            json!({"context_capture_kind": "codex_turn_input"}),
        ),
    ]));
    store
        .append_event(&paths, capture_event)
        .expect("capture event");

    let artifacts = store.list_artifacts(&paths).expect("artifacts");
    let paths_only = artifacts
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect::<Vec<_>>();
    assert!(paths_only.contains(&"artifacts/flow/flow-definition.json"));
    assert!(paths_only.contains(&"logs/work/executions/1-0/response.md"));
    assert!(artifacts.iter().any(|artifact| {
        artifact.path == "logs/work/initial-context.txt"
            && artifact.context_capture_kind.as_deref() == Some("codex_turn_input")
    }));
    assert!(paths_only.contains(&"manifest.json"));
    assert!(paths_only.contains(&"checkpoint.json"));
    assert!(paths_only.contains(&"result/result.json"));
    assert!(paths_only.contains(&"result/result.md"));
    for absent in [
        "logs/manifest.json",
        "logs/checkpoint.json",
        "state.json",
        "run.log",
        "run.json",
        "events.jsonl",
    ] {
        assert!(
            !paths_only.contains(&absent),
            "{absent} should not be listed"
        );
    }
    assert!(artifacts
        .iter()
        .find(|artifact| artifact.path == "artifacts/flow/flow-definition.json")
        .is_some_and(|artifact| artifact.media_type == "application/json" && artifact.viewable));
    assert!(artifacts
        .iter()
        .find(|artifact| artifact.path == "logs/work/notes.yaml")
        .is_some_and(|artifact| artifact.media_type == "text/plain" && artifact.viewable));
    assert!(paths.safe_join("../run.json").is_err());
    assert!(paths.safe_join("/tmp/file").is_err());
}

#[cfg(unix)]
#[test]
fn artifact_listing_refuses_symlink_directory_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project_path = temp.path().join("Project Symlink");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-symlink-list", &project_path.to_string_lossy()),
            checkpoint: Some(checkpoint("work", &["start"])),
            ..CreateRunRequest::default()
        })
        .expect("create run");

    std::fs::write(paths.artifacts_dir().join("inside.txt"), "inside").expect("inside artifact");
    let outside = temp.path().join("outside-artifacts");
    std::fs::create_dir_all(&outside).expect("outside dir");
    std::fs::write(outside.join("secret.txt"), "secret").expect("outside file");
    std::os::unix::fs::symlink(&outside, paths.artifacts_dir().join("outside"))
        .expect("artifact symlink");

    let artifacts = store.list_artifacts(&paths).expect("artifacts");
    let paths_only = artifacts
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect::<Vec<_>>();
    assert!(paths_only.contains(&"artifacts/inside.txt"));
    assert!(!paths_only.contains(&"artifacts/outside/secret.txt"));
}

/// Default runs live in each project's private storage `G/runs`; a
/// configured external root keeps `<root>/<project-id>/<run-id>`. Both are
/// found by directory alone, with no project registration, and resetting runs
/// removes only run directories.
#[test]
fn default_and_external_run_roots_resolve_enumerate_and_reset_by_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let project = "/projects/layout";
    let project_id = spark_common::project::build_project_id(project).unwrap();
    let storage = home.join("projects").join(&project_id);
    let create = |store: &RunStore, run_id: &str| {
        store
            .create_run(CreateRunRequest {
                record: RunRecord::new(run_id, project),
                ..CreateRunRequest::default()
            })
            .unwrap()
    };

    let default = RunStore::for_runs_root(RunsRoot::Home(home.clone()));
    let paths = create(&default, "run-default");
    assert_eq!(paths.root, storage.join("runs/run-default"));
    let external_root = temp.path().join("external");
    let external = RunStore::for_runs_dir(&external_root);
    let external_paths = create(&external, "run-external");
    assert_eq!(
        external_paths.root,
        external_root.join(&project_id).join("run-external")
    );

    // Private siblings of `runs` are not runs, and are never reset.
    std::fs::create_dir_all(storage.join("work/checkout")).unwrap();
    std::fs::create_dir_all(home.join("projects/no-runs/memory")).unwrap();
    let found = default.find_run_root("run-default").unwrap().unwrap();
    assert_eq!(found, paths);
    assert!(default.read_run_meta("run-default").unwrap().is_some());
    assert!(default.find_run_root("run-external").unwrap().is_none());
    let ids: Vec<_> = default
        .list_run_records()
        .unwrap()
        .into_iter()
        .map(|record| record.run_id)
        .collect();
    assert_eq!(ids, vec!["run-default"]);
    assert!(external.read_run_meta("run-external").unwrap().is_some());

    default.delete_all_runs().unwrap();
    assert!(!storage.join("runs").exists());
    assert!(storage.join("work/checkout").is_dir());
    assert!(home.join("projects/no-runs/memory").is_dir());
    assert!(external_paths.root.is_dir());
    assert!(default.list_run_records().unwrap().is_empty());
}

#[test]
fn fixture_derived_python_api_run_roots_read_without_additive_caches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runs_dir = temp.path().join("runs");
    let project_path = temp.path().join("Project Fixture");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = RunRootPaths::new(
        RunsRoot::External(runs_dir),
        project_path.to_string_lossy(),
        "run-fixture",
    )
    .expect("paths");
    std::fs::create_dir_all(paths.logs_dir()).expect("logs dir");

    write_json_atomic(
        paths.run_json(),
        &json!({
            "child_invocation_index": null,
            "continued_from_flow_mode": null,
            "continued_from_flow_name": null,
            "continued_from_node": null,
            "continued_from_run_id": null,
            "ended_at": "2026-06-22T17:40:17Z",
            "estimated_model_cost": null,
            "execution_mode": "native",
            "execution_profile_capabilities": {},
            "execution_profile_id": "native",
            "flow_name": "",
            "git_branch": null,
            "git_commit": null,
            "last_error": "",
            "llm_profile": null,
            "llm_provider": "codex",
            "model": "compat-model",
            "outcome": "success",
            "outcome_reason_code": "completed",
            "outcome_reason_message": null,
            "parent_node_id": null,
            "parent_run_id": null,
            "plan_id": null,
            "project_path": project_path.to_string_lossy(),
            "provider": "codex",
            "reasoning_effort": null,
            "root_run_id": "run-m0-i04-api",
            "run_id": "run-m0-i04-api",
            "spec_id": null,
            "started_at": "2026-06-22T17:40:17Z",
            "status": "completed",
            "token_usage": null,
            "token_usage_breakdown": null,
            "working_directory": project_path.to_string_lossy()
        }),
        JsonWriteOptions::default(),
    )
    .expect("write python-created run");
    write_json_atomic(
        paths.checkpoint_json(),
        &json!({
            "completed_nodes": ["start"],
            "context": {
                "_attractor.node_outcomes": {"start": {"status": "completed", "decision": "next"}},
                "_attractor.runtime.execution_mode": "native",
                "_attractor.runtime.execution_profile_capabilities": {},
                "_attractor.runtime.execution_profile_id": "native",
                "_attractor.runtime.launch_model": "compat-model",
                "_attractor.runtime.launch_provider": "codex",
                "context.topic": "compat",
                "current_node": "done",
                "execution_mode": "native",
                "execution_profile_capabilities": {},
                "execution_profile_id": "native",
                "graph.default_max_retries": 0,
                "graph.goal": "Capture API runtime durability",
                "graph.release": "compat",
                "internal.root_run_id": "run-m0-i04-api",
                "internal.run_id": "run-m0-i04-api",
                "internal.run_workdir": project_path.to_string_lossy()
            },
            "current_node": "done",
            "logs": [],
            "retry_counts": {},
            "timestamp": "2026-06-22T17:40:17.391759+00:00"
        }),
        JsonWriteOptions::default(),
    )
    .expect("write python-created checkpoint");
    write_json_atomic(
        paths.manifest_json(),
        &json!({"goal": "fixture", "graph_id": "Fixture", "start_node": "start"}),
        JsonWriteOptions::default(),
    )
    .expect("write fixture manifest");

    let record = attractor_runtime::read_run_record(&paths)
        .expect("read fixture run")
        .expect("record");
    assert_eq!(record.run_id, "run-m0-i04-api");
    assert_eq!(record.status, "completed");
    assert_eq!(record.execution_mode, "native");

    let checkpoint = attractor_runtime::read_checkpoint(&paths)
        .expect("read fixture checkpoint")
        .expect("checkpoint");
    assert_eq!(checkpoint.current_node, "done");
    assert_eq!(checkpoint.completed_nodes, vec!["start"]);

    let artifacts = attractor_runtime::list_artifacts(&paths).expect("fixture artifacts");
    assert!(artifacts
        .iter()
        .any(|artifact| artifact.path == "checkpoint.json"));
}

fn result_flow(explicit_result_node: Option<&str>) -> FlowDefinition {
    let mut extensions = BTreeMap::new();
    if let Some(node_id) = explicit_result_node {
        extensions.insert("spark.result_node".to_string(), json!(node_id));
    }
    let route = |decision: &str, to: &str| {
        BTreeMap::from([(
            decision.to_string(),
            FlowRoute {
                to: to.to_string(),
                description: format!("Continue to {to}."),
                ..FlowRoute::default()
            },
        )])
    };
    FlowDefinition {
        schema_version: "2".to_string(),
        id: "ResultGraph".to_string(),
        title: "ResultGraph".to_string(),
        nodes: BTreeMap::from([
            (
                "start".to_string(),
                FlowNode {
                    kind: NodeKind::Start,
                    routes: route("next", "work"),
                    ..FlowNode::default()
                },
            ),
            (
                "work".to_string(),
                FlowNode {
                    kind: NodeKind::AgentTask,
                    config: Some(NodeConfig::AgentTask {
                        prompt: "Work.".to_string(),
                    }),
                    contracts: Some(NodeContracts {
                        response: Some("decision_envelope".to_string()),
                        ..NodeContracts::default()
                    }),
                    routes: route("done", "done"),
                    ..FlowNode::default()
                },
            ),
            (
                "done".to_string(),
                FlowNode {
                    kind: NodeKind::Exit,
                    config: Some(NodeConfig::Exit {
                        result: ExitResult {
                            name: "completed".to_string(),
                            outcome: ExitOutcome::Success,
                        },
                        result_summary: false,
                        result_summary_prompt: None,
                    }),
                    ..FlowNode::default()
                },
            ),
        ]),
        extensions,
        ..FlowDefinition::default()
    }
}

#[test]
fn terminal_child_lock_metadata_is_normalized_on_read_and_write() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let paths = store
        .create_run(CreateRunRequest {
            record: record("legacy-child", "/tmp/project"),
            ..Default::default()
        })
        .unwrap();
    for parent in [None, Some("parent")] {
        for status in [
            "running",
            "queued",
            "waiting",
            "completed",
            "failed",
            "canceled",
        ] {
            for state in ["inherited", "holding", "queued"] {
                let mut raw = record("legacy-child", "/tmp/project");
                raw.parent_run_id = parent.map(str::to_string);
                raw.status = status.to_string();
                raw.execution_lock = Some(attractor_core::RunExecutionLock {
                    identity: "same-lock".into(),
                    state: state.into(),
                    queue_position: Some(1),
                    ..Default::default()
                });
                write_json_atomic(paths.run_json(), &raw, JsonWriteOptions::default()).unwrap();
                let loaded = store.read_run_record(&paths).unwrap().unwrap();
                let lock = loaded.execution_lock.as_ref().unwrap();
                let terminal_child =
                    parent.is_some() && matches!(status, "completed" | "failed" | "canceled");
                assert_eq!(lock.state, if terminal_child { "released" } else { state });
                assert_eq!(
                    lock.queue_position,
                    if terminal_child { None } else { Some(1) }
                );
                assert_eq!(lock.identity, "same-lock");
                store.write_run_record(&paths, &raw).unwrap();
                let persisted: RunRecord = read_json(paths.run_json()).unwrap();
                assert_eq!(persisted.execution_lock, loaded.execution_lock);
            }
        }
    }
}

#[test]
fn every_terminal_result_write_notifies_run_event_observers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let observed = seen.clone();
    let store = store(&temp).with_run_event_observer(std::sync::Arc::new(move |run_id: &str| {
        observed.lock().unwrap().push(run_id.to_string())
    }));
    let project_path = temp.path().join("Project Notify");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-failed", &project_path.to_string_lossy()),
            ..CreateRunRequest::default()
        })
        .expect("create run");
    seen.lock().unwrap().clear();
    // A failed run's result lands after pipeline_failed and must still notify.
    store
        .write_result(
            &paths,
            &attractor_runtime::failed_run_result("run-failed", "boom", None),
        )
        .expect("write failed result");
    assert_eq!(*seen.lock().unwrap(), ["run-failed"]);

    let mut record = record("run-orphan", &project_path.to_string_lossy());
    record.status = "cancel_requested".to_string();
    store
        .create_run(CreateRunRequest {
            record,
            ..CreateRunRequest::default()
        })
        .expect("create run");
    seen.lock().unwrap().clear();
    attractor_runtime::RuntimeControls::new(store.clone())
        .mark_canceled("run-orphan", "no executor")
        .expect("mark canceled");
    let paths = store.find_run_root("run-orphan").unwrap().unwrap();
    let result = store.read_result(&paths).unwrap().expect("result");
    assert_eq!(
        (result.status.as_str(), result.state.as_str()),
        ("canceled", "error")
    );
    assert!(seen.lock().unwrap().iter().any(|id| id == "run-orphan"));
}

/// Power loss can leave a run's record empty: that run is reported, and scans
/// over every run keep working for the others.
#[test]
fn an_unreadable_run_is_listed_as_unreadable_and_blocks_no_other_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project = temp.path().join("project").to_string_lossy().into_owned();
    let create = |run_id: &str| {
        store
            .create_run(CreateRunRequest {
                record: record(run_id, &project),
                ..CreateRunRequest::default()
            })
            .expect("run")
    };
    let bad = create("run-bad");
    create("run-good");
    std::fs::write(bad.run_json(), b"").expect("empty record");

    let records = store.list_run_records().expect("listing");
    let status = |run_id: &str| {
        let record = records.iter().find(|r| r.run_id == run_id).expect(run_id);
        (record.status.clone(), record.last_error.clone())
    };
    let (bad_status, bad_error) = status("run-bad");
    assert_eq!(bad_status, attractor_core::RUN_STATUS_UNREADABLE);
    assert!(bad_error.contains("run.json"), "{bad_error}");
    assert_eq!(status("run-good").0, "running");

    assert_eq!(
        store
            .next_child_invocation_index("run-good", "node")
            .unwrap(),
        1
    );
    assert!(store.list_child_run_bundles("run-good").unwrap().is_empty());
}
