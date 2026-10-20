use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use attractor_api::{
    AttractorApiService, ContinuePipelineRequest, PipelineStartRequest, RuntimeRouteResponse,
    SaveFlowRequest,
};
use attractor_core::Attribution;
use attractor_dsl::{accept, parse_flow_definition, promote, read_flow_dir, TrustStore};
use attractor_runtime::RunStore;
use serde_json::{json, Value};
use spark_common::approvals::{read_approvals, sha256_hex, ApprovalEntry};
use spark_common::settings::SparkSettings;

const REVIEW_FLOW: &str = r#"schema_version: "1"
id: review
title: Review
nodes:
  start: {kind: start}
  check: {kind: tool, config: {kind: tool, command: "test -f go"}}
  review: {kind: agent_task, role: reviewer, config: {kind: agent_task, prompt: Review.}}
  done: {kind: exit}
edges:
  - {from: start, to: check}
  - {from: check, to: review, condition: outcome=success}
  - {from: check, to: done, condition: outcome=fail}
  - {from: review, to: done}
"#;

const ROLE: &str = "---\ntype: Role\n---\n\nYou are a careful reviewer.\n";

const RETRY_FLOW: &str = r#"schema_version: "1"
id: retry
nodes:
  start: {kind: start}
  work: {kind: tool, config: {kind: tool, command: "test -f go"}}
  done: {kind: exit}
edges:
  - {from: start, to: work}
  - {from: work, to: done}
"#;

#[test]
fn editor_save_stamps_changed_files_keeps_disk_verified_and_logs_human_save() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_role(&settings, ROLE);
    let service = AttractorApiService::new(settings.clone());
    let save = |flow: &attractor_core::FlowDefinition| {
        let response = service.save_flow(SaveFlowRequest {
            name: "team/review".to_string(),
            content: attractor_dsl::flow_yaml(flow).expect("yaml"),
        });
        assert_eq!(response.status_code, 200, "{}", response.body);
    };
    let dir = settings.flows_dir.join("team/review");
    save(&parse_flow_definition(REVIEW_FLOW).expect("flow"));

    // Every written file is stamped by the local human and logged.
    let saved = read_flow_dir(&dir).expect("load");
    assert_eq!(
        saved.generated.as_ref().expect("generated").by,
        "human:tester"
    );
    for node in saved.nodes.values() {
        assert_eq!(
            node.generated.as_ref().expect("generated").by,
            "human:tester"
        );
    }
    let names = ["check.md", "done.md", "flow.md", "review.md", "start.md"];
    for name in names {
        assert_eq!(
            entries_for(&settings, &format!("flows/team/review/{name}")),
            vec![("human_save".to_string(), file_hash(&dir.join(name)))]
        );
    }

    // A reviewer verified review.md by hand, as a bare mapping.
    let review = fs::read_to_string(dir.join("review.md")).expect("review");
    fs::write(
        dir.join("review.md"),
        review.replace(
            "type: agent_task\n",
            "type: agent_task\nverified: {by: 'human:lead', at: '2026-01-01T00:00:00+00:00'}\n",
        ),
    )
    .expect("verified");
    let before = files(&dir);

    // Incoming generated/verified are ignored; only the edited node changes.
    let mut edited = read_flow_dir(&dir).expect("load");
    let forged = Attribution {
        by: "human:mallory".to_string(),
        at: "2026-02-02T00:00:00+00:00".to_string(),
    };
    edited.generated = Some(forged.clone());
    for node in edited.nodes.values_mut() {
        node.generated = Some(forged.clone());
        node.verified = vec![forged.clone()];
    }
    if let Some(attractor_core::NodeConfig::AgentTask { prompt }) = edited
        .nodes
        .get_mut("review")
        .expect("review")
        .config
        .as_mut()
    {
        *prompt = "Review twice.".to_string();
    }
    save(&edited);
    let after = files(&dir);
    for (name, bytes) in &before {
        if name != "review.md" {
            assert_eq!(&after[name], bytes, "{name} must stay byte-identical");
        }
    }
    let review = read_flow_dir(&dir).expect("load").nodes["review"].clone();
    assert_eq!(
        review.generated.as_ref().expect("generated").by,
        "human:tester"
    );
    assert_eq!(
        review.verified,
        vec![Attribution {
            by: "human:lead".to_string(),
            at: "2026-01-01T00:00:00+00:00".to_string(),
        }]
    );
    let review_entries = entries_for(&settings, "flows/team/review/review.md");
    assert_eq!(review_entries.len(), 2);
    assert_eq!(
        review_entries[1],
        ("human_save".to_string(), file_hash(&dir.join("review.md")))
    );
    assert_eq!(
        entries_for(&settings, "flows/team/review/check.md").len(),
        1
    );

    // Malformed incoming attribution is dropped before validation, not rejected.
    let before = files(&dir);
    let mut raw = json!(read_flow_dir(&dir).expect("load"));
    raw["generated"] = json!("not an attribution");
    raw["verified"] = json!([{"by": "nobody", "at": "2026-99-99T99:99:99+99:99"}]);
    raw["nodes"]["review"]["generated"] = json!(5);
    raw["nodes"]["review"]["verified"] = json!({"by": "human:x"});
    let response = service.save_flow(SaveFlowRequest {
        name: "team/review".to_string(),
        content: raw.to_string(),
    });
    assert_eq!(response.status_code, 200, "{}", response.body);
    assert_eq!(files(&dir), before);
    assert_eq!(
        entries_for(&settings, "flows/team/review/review.md").len(),
        2
    );

    // A save whose approval cannot be logged writes nothing, so retrying the
    // same content still saves and approves it.
    let mut edited = read_flow_dir(&dir).expect("load");
    if let Some(attractor_core::NodeConfig::AgentTask { prompt }) = edited
        .nodes
        .get_mut("review")
        .expect("review")
        .config
        .as_mut()
    {
        *prompt = "Review three times.".to_string();
    }
    let content = attractor_dsl::flow_yaml(&edited).expect("yaml");
    let kept_log = settings.config_dir.join("approvals.kept");
    fs::rename(log(&settings), &kept_log).expect("move log");
    fs::create_dir(log(&settings)).expect("unwritable log");
    let failed = service.save_flow(SaveFlowRequest {
        name: "team/review".to_string(),
        content: content.clone(),
    });
    assert_ne!(failed.status_code, 200, "{}", failed.body);
    assert_eq!(files(&dir), before);
    fs::remove_dir(log(&settings)).expect("drop dir");
    fs::rename(&kept_log, log(&settings)).expect("restore log");
    let retried = service.save_flow(SaveFlowRequest {
        name: "team/review".to_string(),
        content,
    });
    assert_eq!(retried.status_code, 200, "{}", retried.body);
    let review_entries = entries_for(&settings, "flows/team/review/review.md");
    assert_eq!(review_entries.len(), 3);
    assert_eq!(
        review_entries[2],
        ("human_save".to_string(), file_hash(&dir.join("review.md")))
    );
}

#[test]
fn gate_refuses_unapproved_and_deprecated_files_naming_each() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());
    let dir = install(&settings, "team/review", REVIEW_FLOW);
    write_role(&settings, ROLE);
    approve(&settings, &settings.roles_dir().join("reviewer.md"));
    assert_eq!(
        launch(&service, &settings, "team/review", "approved").body["status"],
        "started"
    );

    // Hand edits to flow.md, a node file and the role all need approval.
    edit(&dir.join("flow.md"), "title: Review", "title: Reviewed");
    edit(&dir.join("check.md"), "test -f go", "test -f gone");
    edit(
        &settings.roles_dir().join("reviewer.md"),
        "careful",
        "lenient",
    );
    let refused = launch(&service, &settings, "team/review", "edited");
    assert_eq!(refused.body["status"], "failed", "{}", refused.body);
    let error = refused.body["error"].as_str().expect("error");
    for named in [
        "flows/team/review/flow.md: not approved",
        "flows/team/review/check.md: not approved",
        "roles/reviewer.md: not approved",
        "spark flow accept",
        "spark flow promote",
    ] {
        assert!(error.contains(named), "{named}: {error}");
    }
    assert!(!error.contains("done.md"), "{error}");
    assert!(RunStore::for_settings(&settings)
        .read_run_bundle("edited")
        .expect("read")
        .is_none());

    approve(&settings, &dir);
    approve(&settings, &settings.roles_dir().join("reviewer.md"));
    assert_eq!(
        launch(&service, &settings, "team/review", "accepted").body["status"],
        "started"
    );

    // An approved but deprecated file does not run.
    edit(
        &dir.join("done.md"),
        "type: exit\n",
        "type: exit\nstatus: deprecated\n",
    );
    approve(&settings, &dir.join("done.md"));
    let deprecated = launch(&service, &settings, "team/review", "deprecated");
    assert!(deprecated.body["error"].as_str().is_some_and(|error| error
        .contains("flows/team/review/done.md: status is deprecated")
        && !error.contains("not approved")));
}

#[test]
fn gate_applies_to_flow_name_continue_and_subflow_children_but_not_retry_or_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());
    let store = RunStore::for_settings(&settings);
    let dir = install(&settings, "team/retry", RETRY_FLOW);
    let failed = launch(&service, &settings, "team/retry", "failing");
    assert_eq!(failed.body["status"], "started", "{}", failed.body);
    assert_eq!(status(&store, "failing"), "failed");

    // An unapproved edit refuses new launches and flow_name continues...
    fs::write(project(&settings).join("go"), "").expect("go");
    edit(
        &dir.join("flow.md"),
        "id: retry",
        "id: retry\ndescription: edited",
    );
    assert!(
        launch(&service, &settings, "team/retry", "refused").body["error"]
            .as_str()
            .is_some_and(|error| error.contains("flows/team/retry/flow.md: not approved"))
    );
    let continued = service.continue_pipeline_route(
        "failing",
        ContinuePipelineRequest {
            start_node: "work".to_string(),
            flow_source_mode: "flow_name".to_string(),
            flow_name: Some("team/retry".to_string()),
            ..ContinuePipelineRequest::default()
        },
    );
    assert_eq!(continued.body["status"], "failed", "{}", continued.body);
    assert!(continued.body["error"]
        .as_str()
        .is_some_and(|error| error.contains("Launch refused")));

    // ...but retry and snapshot continue reuse the stored resolved source.
    let retried = service.retry_pipeline_route("failing");
    assert_eq!(retried.body["status"], "started", "{}", retried.body);
    wait_for_status(&store, "failing", "completed");
    let snapshot = service.continue_pipeline_route(
        "failing",
        ContinuePipelineRequest {
            start_node: "work".to_string(),
            flow_source_mode: "snapshot".to_string(),
            ..ContinuePipelineRequest::default()
        },
    );
    assert_eq!(snapshot.body["status"], "started", "{}", snapshot.body);
    let snapshot_id = snapshot.body["run_id"]
        .as_str()
        .expect("run id")
        .to_string();
    wait_for_status(&store, &snapshot_id, "completed");
    // A snapshot continue keeps the source run's manifest.
    assert_eq!(manifest(&store, &snapshot_id), manifest(&store, "failing"));

    // Subflow children pass the gate when they start.
    let child = settings.flows_dir.join("team/child");
    attractor_dsl::write_flow_dir(&child, &parse_flow_definition(RETRY_FLOW).expect("child"))
        .expect("child");
    install(
        &settings,
        "team/parent",
        r#"schema_version: "1"
id: parent
nodes:
  start: {kind: start}
  run: {kind: subflow, config: {kind: subflow, flow_ref: child}}
  done: {kind: exit}
edges:
  - {from: start, to: run}
  - {from: run, to: done}
"#,
    );
    launch(&service, &settings, "team/parent", "parent-refused");
    assert_eq!(status(&store, "parent-refused"), "failed");
    let events = store
        .read_run_bundle("parent-refused")
        .expect("read")
        .expect("run")
        .raw_events;
    assert!(
        events.iter().any(|event| {
            let payload = serde_json::to_string(&event.payload).unwrap_or_default();
            payload.contains("Child flow refused") && payload.contains("flows/team/child/flow.md")
        }),
        "the parent records why its child was refused"
    );
    assert!(child_runs(&store, "parent-refused").is_empty());

    approve(&settings, &child);
    launch(&service, &settings, "team/parent", "parent-approved");
    assert_eq!(status(&store, "parent-approved"), "completed");
    // The child run records its own manifest.
    let children = child_runs(&store, "parent-approved");
    assert_eq!(children.len(), 1);
    let child_manifest = manifest(&store, &children[0]);
    assert_eq!(
        child_manifest.keys().cloned().collect::<Vec<_>>(),
        ["done.md", "flow.md", "start.md", "work.md"]
            .map(|name| format!("flows/team/child/{name}"))
    );
    assert_eq!(
        child_manifest["flows/team/child/work.md"],
        json!(file_hash(&child.join("work.md")))
    );
}

#[test]
fn run_manifest_lists_every_file_used_with_its_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());
    let store = RunStore::for_settings(&settings);
    let dir = install(&settings, "team/review", REVIEW_FLOW);
    let role = settings.roles_dir().join("reviewer.md");
    write_role(&settings, ROLE);
    approve(&settings, &role);
    let started = launch_with_goal(&service, &settings, "team/review", "manifest", "Ship it.");
    assert_eq!(started.body["status"], "started", "{}", started.body);

    let mut expected = ["check.md", "done.md", "flow.md", "review.md", "start.md"]
        .map(|name| {
            (
                format!("flows/team/review/{name}"),
                json!(file_hash(&dir.join(name))),
            )
        })
        .into_iter()
        .collect::<serde_json::Map<_, _>>();
    expected.insert("roles/reviewer.md".to_string(), json!(file_hash(&role)));
    // Hashes are of the files on disk, before the launch goal is injected.
    assert_eq!(
        Value::Object(manifest(&store, "manifest")),
        Value::Object(expected)
    );

    let bundle = store
        .read_run_bundle("manifest")
        .expect("read")
        .expect("run");
    let files_path = bundle.paths.artifacts_dir().join("flow/files.json");
    let run_meta = bundle
        .raw_events
        .iter()
        .find(|event| event.event_type == "run_meta")
        .expect("run metadata event");
    assert_eq!(
        run_meta.payload["flow_files_path"],
        json!(files_path.to_string_lossy())
    );
}

#[test]
fn accept_logs_the_current_hash_without_modifying_the_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let dir = install(&settings, "team/retry", RETRY_FLOW);
    edit(&dir.join("work.md"), "test -f go", "true");
    let before = fs::read(dir.join("work.md")).expect("work");

    // A $SPARK_HOME-relative path names the live file.
    let entries = accept(
        &trust(&settings),
        Path::new("flows/team/retry/work.md"),
        "human:tester",
    )
    .expect("accept");
    assert_eq!(fs::read(dir.join("work.md")).expect("work"), before);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].event, "human_save");
    assert_eq!(entries[0].path, "flows/team/retry/work.md");
    assert_eq!(entries[0].sha256, sha256_hex(&before));
    assert_eq!(
        read_approvals(&log(&settings)).expect("log").last(),
        Some(&entries[0])
    );

    // Only live flow and role files can be accepted.
    let outside = temp.path().join("elsewhere.md");
    fs::write(&outside, "---\ntype: Role\n---\n").expect("outside");
    assert!(accept(&trust(&settings), &outside, "human:tester").is_err());
}

#[test]
fn promote_requires_draft_candidates_that_validate_and_installs_them_verified() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());
    let candidates = settings.data_dir.join("candidates");
    let live = install(&settings, "team/retry", RETRY_FLOW);
    let candidate = candidates.join("flows/team/retry/work.md");
    let promote_candidate = || promote(&trust(&settings), &candidates, &candidate, "human:tester");
    let write_candidate = |frontmatter: &str| {
        fs::create_dir_all(candidate.parent().expect("parent")).expect("candidate dir");
        fs::write(
            &candidate,
            format!("---\ntype: tool\ntitle: Work\n{frontmatter}routes:\n- to: done\n---\n\n```sh\ntrue\n```\n"),
        )
        .expect("candidate");
    };
    let live_before = fs::read(live.join("work.md")).expect("live");
    let log_before = read_approvals(&log(&settings)).expect("log").len();
    let generated = "generated: {by: codex/gpt-5, at: '2026-10-20T12:00:00+00:00'}\n";

    for (frontmatter, reason) in [
        ("status: draft\n".to_string(), "has no generated field"),
        (generated.to_string(), "status is not draft"),
    ] {
        write_candidate(&frontmatter);
        let error = promote_candidate().expect_err(reason);
        assert!(error.detail().contains(reason), "{}", error.detail());
    }
    // The result must still assemble and validate.
    write_candidate(&format!("{generated}status: draft\n"));
    fs::write(
        &candidate,
        fs::read_to_string(&candidate)
            .expect("candidate")
            .replace("- to: done", "- to: missing"),
    )
    .expect("broken route");
    let error = promote_candidate().expect_err("invalid result");
    assert!(
        error.detail().contains("flow team/retry"),
        "{}",
        error.detail()
    );
    assert_eq!(fs::read(live.join("work.md")).expect("live"), live_before);
    assert_eq!(
        read_approvals(&log(&settings)).expect("log").len(),
        log_before
    );

    write_candidate(&format!("{generated}status: draft\n"));
    let entries = promote_candidate().expect("promote");
    let promoted = fs::read_to_string(live.join("work.md")).expect("promoted");
    assert!(promoted.contains("status: stable"), "{promoted}");
    let node = read_flow_dir(&live).expect("load").nodes["work"].clone();
    assert_eq!(node.status, attractor_core::TrustStatus::Stable);
    assert_eq!(node.generated.expect("generated").by, "codex/gpt-5");
    assert_eq!(node.verified.len(), 1);
    assert_eq!(node.verified[0].by, "human:tester");
    assert_eq!(
        entries,
        vec![ApprovalEntry {
            at: entries[0].at.clone(),
            event: "promoted".to_string(),
            path: "flows/team/retry/work.md".to_string(),
            sha256: sha256_hex(promoted.as_bytes()),
            by: "human:tester".to_string(),
        }]
    );
    assert!(!candidate.exists());
    assert!(
        !candidates.join("flows").exists(),
        "emptied candidate dirs are removed"
    );
    assert_eq!(
        launch(&service, &settings, "team/retry", "promoted").body["status"],
        "started"
    );

    // A whole candidate directory promotes a new flow.
    let fresh = candidates.join("flows/team/fresh");
    fs::create_dir_all(&fresh).expect("fresh");
    let draft = |body: &str| format!("---\n{generated}status: draft\n{body}");
    fs::write(fresh.join("flow.md"), draft("type: Flow\nid: fresh\n---\n")).expect("flow.md");
    fs::write(
        fresh.join("start.md"),
        draft("type: start\nroutes:\n- to: done\n---\n"),
    )
    .expect("start");
    fs::write(fresh.join("done.md"), draft("type: exit\n---\n")).expect("done");
    let entries =
        promote(&trust(&settings), &candidates, &fresh, "human:tester").expect("promote dir");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        [
            "flows/team/fresh/done.md",
            "flows/team/fresh/flow.md",
            "flows/team/fresh/start.md"
        ]
    );
    assert!(!fresh.exists());
    assert_eq!(
        launch(&service, &settings, "team/fresh", "fresh").body["status"],
        "started"
    );
}

#[test]
fn promote_refuses_a_role_whose_dependent_flow_does_not_assemble() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_role(&settings, ROLE);
    let live_role = settings.roles_dir().join("reviewer.md");
    approve(&settings, &live_role);
    let live = install(&settings, "demo", REVIEW_FLOW);
    // The flow references the role but no longer assembles.
    edit(&live.join("review.md"), "to: done", "to: missing");
    assert!(read_flow_dir(&live).is_err());

    let candidates = settings.data_dir.join("candidates");
    let candidate = candidates.join("roles/reviewer.md");
    fs::create_dir_all(candidate.parent().expect("parent")).expect("candidates");
    let draft = "---\ntype: Role\ngenerated: {by: codex/gpt-5, at: '2026-10-20T12:00:00+00:00'}\nstatus: draft\n---\n\nYou are a stricter reviewer.\n";
    fs::write(&candidate, draft).expect("candidate");
    let role_before = fs::read(&live_role).expect("role");
    let log_before = read_approvals(&log(&settings)).expect("log");

    let error = promote(&trust(&settings), &candidates, &candidate, "human:tester")
        .expect_err("dependent flow is invalid");
    assert!(error.detail().contains("flow demo"), "{}", error.detail());
    assert_eq!(fs::read(&live_role).expect("role"), role_before);
    assert_eq!(read_approvals(&log(&settings)).expect("log"), log_before);
    assert_eq!(fs::read_to_string(&candidate).expect("candidate"), draft);
}

fn install(settings: &SparkSettings, name: &str, yaml: &str) -> PathBuf {
    let dir = settings.flows_dir.join(name);
    attractor_dsl::write_flow_dir(&dir, &parse_flow_definition(yaml).expect("flow"))
        .expect("write flow");
    approve(settings, &dir);
    dir
}

fn approve(settings: &SparkSettings, path: &Path) {
    accept(&trust(settings), path, "human:tester").expect("approve");
}

fn trust(settings: &SparkSettings) -> TrustStore {
    TrustStore::for_settings(settings)
}

fn log(settings: &SparkSettings) -> PathBuf {
    settings.config_dir.join("approvals.jsonl")
}

fn write_role(settings: &SparkSettings, text: &str) {
    fs::create_dir_all(settings.roles_dir()).expect("roles");
    fs::write(settings.roles_dir().join("reviewer.md"), text).expect("role");
}

fn edit(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).expect("read");
    assert!(text.contains(from), "{}: {text}", path.display());
    fs::write(path, text.replacen(from, to, 1)).expect("write");
}

fn file_hash(path: &Path) -> String {
    sha256_hex(&fs::read(path).expect("read"))
}

fn entries_for(settings: &SparkSettings, path: &str) -> Vec<(String, String)> {
    read_approvals(&log(settings))
        .expect("log")
        .into_iter()
        .filter(|entry| entry.path == path)
        .map(|entry| (entry.event, entry.sha256))
        .collect()
}

fn files(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .expect("dir")
        .map(|entry| {
            let entry = entry.expect("entry");
            (
                entry.file_name().to_string_lossy().to_string(),
                fs::read(entry.path()).expect("file"),
            )
        })
        .collect()
}

fn project(settings: &SparkSettings) -> PathBuf {
    settings.project_root.clone()
}

fn launch(
    service: &AttractorApiService,
    settings: &SparkSettings,
    name: &str,
    run_id: &str,
) -> RuntimeRouteResponse {
    launch_with_goal(service, settings, name, run_id, "")
}

fn launch_with_goal(
    service: &AttractorApiService,
    settings: &SparkSettings,
    name: &str,
    run_id: &str,
    goal: &str,
) -> RuntimeRouteResponse {
    fs::create_dir_all(project(settings)).expect("project");
    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some(run_id.to_string()),
        flow_name: Some(name.to_string()),
        goal: (!goal.is_empty()).then(|| goal.to_string()),
        working_directory: project(settings).to_string_lossy().into_owned(),
        ..PipelineStartRequest::default()
    })
}

fn status(store: &RunStore, run_id: &str) -> String {
    store
        .read_run_bundle(run_id)
        .expect("read")
        .and_then(|bundle| bundle.record)
        .map(|record| record.status)
        .unwrap_or_default()
}

fn wait_for_status(store: &RunStore, run_id: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while status(store, run_id) != expected {
        assert!(
            Instant::now() < deadline,
            "{run_id} never reached {expected} (last: {})",
            status(store, run_id)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn manifest(store: &RunStore, run_id: &str) -> serde_json::Map<String, Value> {
    let bundle = store.read_run_bundle(run_id).expect("read").expect("run");
    let text = fs::read(bundle.paths.artifacts_dir().join("flow/files.json")).expect("files.json");
    serde_json::from_slice(&text).expect("manifest json")
}

fn child_runs(store: &RunStore, parent: &str) -> Vec<String> {
    store
        .list_run_records()
        .expect("runs")
        .into_iter()
        .filter(|record| record.parent_run_id.as_deref() == Some(parent))
        .map(|record| record.run_id)
        .collect()
}

fn settings(root: &Path) -> SparkSettings {
    let settings = SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
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
    };
    fs::create_dir_all(&settings.config_dir).expect("config");
    fs::write(
        settings.config_dir.join("spark.toml"),
        "schema_version = 1\nhuman_id = \"tester\"\n",
    )
    .expect("spark.toml");
    settings
}
