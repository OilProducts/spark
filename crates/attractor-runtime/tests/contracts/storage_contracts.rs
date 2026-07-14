use std::collections::BTreeMap;

use attractor_core::{
    CheckpointState, FlowDefinition, FlowEdge, FlowNode, NodeKind, RawRuntimeEvent, RunManifest,
    RunRecord,
};
use attractor_runtime::{
    checkpoint_saved_event, log_event, stage_completed_event, stage_started_event,
    CheckpointWriteOptions, CreateRunRequest, NodeArtifacts, RunRootPaths, RunStore,
};
use serde_json::json;
use spark_storage::{read_json, write_json_atomic, JsonWriteOptions};

fn store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"))
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
            json!({"start": "success", "work": "success"}),
        )]),
        retry_counts: BTreeMap::new(),
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

    let paths = store
        .create_run(CreateRunRequest {
            record: record.clone(),
            checkpoint: Some(checkpoint.clone()),
            manifest: Some(manifest),
            flow_source: Some("schema_version: '1'\nid: create\n".to_string()),
            flow_definition_json: Some(
                "{\"schema_version\":\"1\",\"id\":\"create\"}\n".to_string(),
            ),
        })
        .expect("create run");

    assert_eq!(
        paths.root,
        temp.path()
            .join("spark-home/attractor/runs")
            .join(&paths.project_id)
            .join("run-create")
    );
    for path in [
        paths.run_json(),
        paths.events_jsonl(),
        paths.state_json(),
        paths.logs_manifest_json(),
        paths.run_log(),
        paths.result_json(),
        paths.result_markdown(),
        paths.artifacts_dir().join("flow/flow-source.yaml"),
        paths.artifacts_dir().join("flow/flow-definition.json"),
    ] {
        assert!(path.exists(), "{} should exist", path.display());
    }
    assert!(paths.logs_dir().is_dir());
    assert!(paths.logs_dir().join("artifacts").is_dir());
    assert!(paths.artifacts_dir().is_dir());
    assert!(paths.result_dir().is_dir());
    assert!(!paths.manifest_json().exists());

    let loaded = store
        .read_run_record(&paths)
        .expect("read run record")
        .expect("run record");
    assert_eq!(loaded.run_id, "run-create");
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
            stage_completed_event("run-events", 0, "work", "success"),
        )
        .expect("stage completed");
    store
        .append_event(
            &paths,
            checkpoint_saved_event("run-events", "done", vec!["work".to_string()]),
        )
        .expect("checkpoint event");

    let events = store.read_raw_events(&paths).expect("events");
    assert_eq!(events.len(), 7);
    assert_eq!(events.last().and_then(|event| event.sequence), Some(7));

    let journal = store.read_journal(&paths).expect("journal");
    assert_eq!(journal[0].id, "journal-7");
    assert_eq!(journal[0].kind, "checkpoint");
    assert_eq!(journal[0].summary, "Checkpoint saved at done");
    assert_eq!(journal[1].summary, "Stage work completed (success)");
    assert_eq!(journal[2].summary, "[work] running");
    assert_eq!(journal[3].summary, "Stage work started");
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

    // Transcripts are a read-time projection of the journal now: the
    // appended stage event surfaces as a boundary segment on read.
    let transcript =
        attractor_runtime::project_run_transcript(&store.read_journal(&paths).expect("journal"));
    assert!(transcript
        .segments
        .iter()
        .any(|segment| segment.kind == "boundary"
            && segment
                .boundary
                .as_ref()
                .and_then(|meta| meta.node_id.as_deref())
                == Some("work")));
}

#[test]
fn checkpoint_reads_state_root_checkpoint_and_logs_checkpoint_fallbacks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runs_dir = temp.path().join("runs");
    let project_path = temp.path().join("Project Four");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = RunRootPaths::new(runs_dir, project_path.to_string_lossy(), "run-checkpoint")
        .expect("paths");
    std::fs::create_dir_all(paths.logs_dir()).expect("logs dir");

    let logs_checkpoint = checkpoint("from-logs", &["a"]);
    write_json_atomic(
        paths.logs_checkpoint_json(),
        &logs_checkpoint,
        JsonWriteOptions::default(),
    )
    .expect("logs checkpoint");
    assert_eq!(
        attractor_runtime::read_checkpoint(&paths)
            .expect("read logs fallback")
            .expect("checkpoint")
            .current_node,
        "from-logs"
    );

    let root_checkpoint = checkpoint("from-root", &["a", "b"]);
    write_json_atomic(
        paths.checkpoint_json(),
        &root_checkpoint,
        JsonWriteOptions::default(),
    )
    .expect("root checkpoint");
    assert_eq!(
        attractor_runtime::read_checkpoint(&paths)
            .expect("read root fallback")
            .expect("checkpoint")
            .current_node,
        "from-root"
    );

    let state_checkpoint = checkpoint("from-state", &["a", "b", "c"]);
    attractor_runtime::save_checkpoint(
        &paths,
        &state_checkpoint,
        CheckpointWriteOptions {
            write_root_checkpoint: false,
            mirror_logs_checkpoint: false,
        },
    )
    .expect("state checkpoint");
    assert_eq!(
        attractor_runtime::read_checkpoint(&paths)
            .expect("read state preferred")
            .expect("checkpoint")
            .current_node,
        "from-state"
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
            &NodeArtifacts {
                prompt: Some("Prompt".to_string()),
                response: Some("Raw response body\n".to_string()),
                status: Some(json!({"outcome": "success"})),
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
        Some("logs/work/response.md")
    );
    assert_eq!(result.body_markdown, "Raw response body\n");
    assert_eq!(
        std::fs::read_to_string(paths.result_markdown()).expect("result md"),
        "Raw response body\n"
    );

    std::fs::write(paths.result_markdown(), "Edited body\n").expect("edit markdown");
    let reread = store
        .read_result(&paths)
        .expect("read result")
        .expect("result");
    assert_eq!(reread.body_markdown, "Edited body\n");
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
            flow_source: Some("schema_version: '1'\nid: artifacts\n".to_string()),
            ..CreateRunRequest::default()
        })
        .expect("create run");
    store
        .write_node_artifacts(
            &paths,
            "work",
            &NodeArtifacts {
                prompt: Some("Prompt".to_string()),
                response: Some("Response".to_string()),
                status: Some(json!({"outcome": "success"})),
                under_logs: true,
            },
        )
        .expect("node artifacts");
    std::fs::write(
        paths.logs_dir().join("work/initial-context.txt"),
        "captured request",
    )
    .expect("initial context");
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
    assert!(paths_only.contains(&"artifacts/flow/flow-source.yaml"));
    assert!(paths_only.contains(&"logs/work/response.md"));
    assert!(artifacts.iter().any(|artifact| {
        artifact.path == "logs/work/initial-context.txt"
            && artifact.context_capture_kind.as_deref() == Some("codex_turn_input")
    }));
    assert!(paths_only.contains(&"logs/manifest.json"));
    assert!(paths_only.contains(&"result/result.json"));
    assert!(paths_only.contains(&"run.log"));
    assert!(!paths_only.contains(&"manifest.json"));
    assert!(!paths_only.contains(&"run.json"));
    assert!(!paths_only.contains(&"state.json"));
    assert!(!paths_only.contains(&"events.jsonl"));
    assert!(artifacts
        .iter()
        .find(|artifact| artifact.path == "artifacts/flow/flow-source.yaml")
        .is_some_and(|artifact| artifact.media_type == "text/yaml" && artifact.viewable));
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

#[test]
fn fixture_derived_python_api_run_roots_read_without_additive_caches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runs_dir = temp.path().join("runs");
    let project_path = temp.path().join("Project Fixture");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths =
        RunRootPaths::new(runs_dir, project_path.to_string_lossy(), "run-fixture").expect("paths");
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
            "outcome_reason_code": null,
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
        paths.logs_checkpoint_json(),
        &json!({
            "completed_nodes": ["start"],
            "context": {
                "_attractor.node_outcomes": {"start": "success"},
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
                "internal.run_workdir": project_path.to_string_lossy(),
                "outcome": "success",
                "preferred_label": ""
            },
            "current_node": "done",
            "logs": [],
            "retry_counts": {},
            "timestamp": "2026-06-22T17:40:17.391759+00:00"
        }),
        JsonWriteOptions::default(),
    )
    .expect("write python-created logs checkpoint");
    write_json_atomic(
        paths.logs_manifest_json(),
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
        .any(|artifact| artifact.path == "logs/checkpoint.json"));
}

fn result_flow(explicit_result_node: Option<&str>) -> FlowDefinition {
    let mut extensions = BTreeMap::new();
    if let Some(node_id) = explicit_result_node {
        extensions.insert("spark.result_node".to_string(), json!(node_id));
    }
    FlowDefinition {
        schema_version: "1.0".to_string(),
        id: "ResultGraph".to_string(),
        title: "ResultGraph".to_string(),
        nodes: BTreeMap::from([
            ("start".to_string(), flow_node(NodeKind::Start)),
            ("work".to_string(), flow_node(NodeKind::AgentTask)),
            ("done".to_string(), flow_node(NodeKind::Exit)),
        ]),
        edges: vec![FlowEdge {
            from: "work".to_string(),
            to: "done".to_string(),
            ..FlowEdge::default()
        }],
        extensions,
        ..FlowDefinition::default()
    }
}

fn flow_node(kind: NodeKind) -> FlowNode {
    FlowNode {
        kind,
        ..FlowNode::default()
    }
}
