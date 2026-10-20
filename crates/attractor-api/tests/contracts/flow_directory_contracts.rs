use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use attractor_api::{preview_with_flows_dir, AttractorApiService, PreviewRequest, SaveFlowRequest};
use attractor_core::{ExecutionConfig, NodeConfig};
use attractor_dsl::{
    flow_yaml, parse_flow_definition, read_flow_dir, resolve_flow_ref, resolve_flow_roles,
    write_flow_dir, FlowDefinition,
};
use serde_json::json;
use spark_common::settings::SparkSettings;

const FLOW: &str = r#"schema_version: "1"
id: review
title: Review
goal: Review the work.
nodes:
  start:
    kind: start
    label: Start
  check:
    kind: tool
    label: Check
    config:
      kind: tool
      command: "echo ``` && true"
  review:
    kind: agent_task
    label: Review
    role: reviewer
    config:
      kind: agent_task
      prompt: Review the diff.
    execution:
      llm_model: node-model
  done:
    kind: exit
    label: Done
    config:
      kind: exit
      result_summary: true
      result_summary_prompt: Summarize.
edges:
  - from: start
    to: check
  - from: check
    to: review
    condition: outcome=success
  - from: review
    to: done
"#;

const ROLE: &str = "---\ntype: Role\nexecution:\n  llm_model: role-model\n  reasoning_effort: high\n---\n\nYou are a careful reviewer.\n";

fn flow() -> FlowDefinition {
    parse_flow_definition(FLOW).expect("valid flow")
}

fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .expect("flow dir")
        .map(|entry| {
            let entry = entry.expect("entry");
            (
                entry.file_name().into_string().expect("name"),
                fs::read(entry.path()).expect("file"),
            )
        })
        .collect()
}

fn texts(dir: &Path) -> BTreeMap<String, String> {
    files(dir)
        .into_iter()
        .map(|(name, bytes)| (name, String::from_utf8(bytes).expect("utf-8")))
        .collect()
}

#[test]
fn node_bodies_follow_their_kind_and_routes_live_on_the_source_node() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path().join("review");
    write_flow_dir(&dir, &flow()).expect("write");
    let texts = texts(&dir);

    assert_eq!(
        texts.keys().collect::<Vec<_>>(),
        ["check.md", "done.md", "flow.md", "review.md", "start.md"]
    );
    assert!(texts["flow.md"].starts_with("---\ntype: Flow\nid: review\n"));
    assert!(texts["flow.md"].ends_with("---\n\nReview the work.\n"));
    // The command holds a ``` run, so its fence is longer.
    assert!(texts["check.md"].ends_with("---\n\n````sh\necho ``` && true\n````\n"));
    assert!(texts["check.md"].contains("routes:\n- to: review\n  condition: outcome=success\n"));
    assert!(texts["review.md"].contains("role: reviewer\n"));
    assert!(texts["review.md"].ends_with("---\n\nReview the diff.\n"));
    assert!(texts["done.md"].ends_with("---\n\nSummarize.\n"));
    assert!(texts["start.md"].ends_with("routes:\n- to: check\n---\n"));

    assert_eq!(read_flow_dir(&dir).expect("load"), flow());
}

#[test]
fn loader_rejects_stray_body_text_unknown_files_and_bad_tool_bodies() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path().join("review");
    write_flow_dir(&dir, &flow()).expect("write");
    let start = fs::read_to_string(dir.join("start.md")).expect("start");
    let check = fs::read_to_string(dir.join("check.md")).expect("check");

    // OKF-reserved files and hidden files are ignored.
    fs::write(dir.join("index.md"), "# Index\n").expect("index");
    fs::write(dir.join("log.md"), "# Log\n").expect("log");
    fs::write(dir.join(".start.md.swp"), "swap").expect("swap");
    assert_eq!(read_flow_dir(&dir).expect("load"), flow());

    fs::write(dir.join("start.md"), format!("{start}\nignored prose\n")).expect("start");
    let error = read_flow_dir(&dir).expect_err("stray body");
    assert!(error.detail().contains("start.md"), "{}", error.detail());
    assert!(
        error.detail().contains("take no body"),
        "{}",
        error.detail()
    );
    fs::write(dir.join("start.md"), &start).expect("start");

    let body_start = check.find("````sh").expect("fence");
    for bad_body in [
        "echo hi\n",
        "```sh\necho a\n```\n\n```sh\necho b\n```\n",
        "```sh\necho a\n```\ntrailing\n",
    ] {
        fs::write(
            dir.join("check.md"),
            format!("{}{bad_body}", &check[..body_start]),
        )
        .expect("check");
        let error = read_flow_dir(&dir).expect_err("bad tool body");
        assert!(
            error.detail().contains("exactly one fenced code block"),
            "{bad_body:?}: {}",
            error.detail()
        );
    }
    fs::write(dir.join("check.md"), &check).expect("check");

    fs::write(dir.join("notes.txt"), "notes").expect("notes");
    fs::write(dir.join("old.yaml"), FLOW).expect("yaml");
    let error = read_flow_dir(&dir).expect_err("unknown files");
    assert!(
        error.detail().contains("notes.txt, old.yaml"),
        "{}",
        error.detail()
    );
}

#[test]
fn roles_prepend_their_persona_and_merge_execution_under_the_node() {
    let temp = tempfile::tempdir().expect("tempdir");
    let roles_dir = temp.path().join("roles");
    fs::create_dir_all(&roles_dir).expect("roles");
    fs::write(roles_dir.join("reviewer.md"), ROLE).expect("role");

    let resolved = resolve_flow_roles(&flow(), &roles_dir).expect("resolve");
    let review = &resolved.nodes["review"];
    assert_eq!(review.role, None);
    assert_eq!(
        review.config,
        Some(NodeConfig::AgentTask {
            prompt: "You are a careful reviewer.\n\nReview the diff.".to_string()
        })
    );
    assert_eq!(
        review.execution,
        Some(ExecutionConfig {
            llm_model: Some("node-model".to_string()),
            reasoning_effort: Some("high".to_string()),
            ..Default::default()
        })
    );

    let error = resolve_flow_roles(&flow(), &temp.path().join("missing")).expect_err("unknown");
    assert!(
        error.detail().contains("unknown role 'reviewer'"),
        "{}",
        error.detail()
    );

    let mut misplaced = flow();
    misplaced.nodes.get_mut("check").expect("check").role = Some("reviewer".to_string());
    let error = attractor_dsl::validate_flow_definition(&misplaced).expect_err("role on tool");
    assert!(
        error.detail.contains("only allowed on agent_task"),
        "{}",
        error.detail
    );
}

#[test]
fn editor_sees_the_authored_form_and_launches_execute_the_resolved_form() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());

    let unknown_role = service.save_flow(SaveFlowRequest {
        name: "review".to_string(),
        content: FLOW.to_string(),
    });
    assert_eq!(unknown_role.status_code, 422, "{}", unknown_role.body);
    assert!(unknown_role
        .body
        .to_string()
        .contains("unknown role 'reviewer'"));

    fs::create_dir_all(settings.roles_dir()).expect("roles");
    fs::write(settings.roles_dir().join("reviewer.md"), ROLE).expect("role");
    let saved = service.save_flow(SaveFlowRequest {
        name: "review".to_string(),
        content: FLOW.to_string(),
    });
    assert_eq!(saved.status_code, 200, "{}", saved.body);

    let loaded = service.get_flow("review");
    let content = loaded.body["content"].as_str().expect("content");
    assert!(content.contains("role: reviewer"));
    assert!(!content.contains("careful reviewer"));
    assert_eq!(parse_flow_definition(content).expect("authored"), flow());

    let resolved = attractor_dsl::load_resolved_flow_content(
        &settings.flows_dir,
        &settings.roles_dir(),
        "review",
    )
    .expect("resolved");
    assert!(resolved.contains("You are a careful reviewer."));
    assert!(!resolved.contains("role: reviewer"));
}

#[test]
fn editor_save_rewrites_only_changed_files_and_deletes_removed_nodes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(settings.roles_dir()).expect("roles");
    fs::write(settings.roles_dir().join("reviewer.md"), ROLE).expect("role");
    let service = AttractorApiService::new(settings.clone());
    let save = |flow: &FlowDefinition| {
        let response = service.save_flow(SaveFlowRequest {
            name: "review".to_string(),
            content: flow_yaml(flow).expect("yaml"),
        });
        assert_eq!(response.status_code, 200, "{}", response.body);
    };
    save(&flow());
    let dir = settings.flows_dir.join("review");

    // Hand formatting that keeps the meaning survives a save untouched.
    let start = fs::read_to_string(dir.join("start.md")).expect("start");
    let hand_formatted = start.replace("type: start\n", "type: start   # entry\n");
    fs::write(dir.join("start.md"), &hand_formatted).expect("start");
    let before = files(&dir);

    let mut edited = flow();
    edited.nodes.get_mut("review").expect("review").config = Some(NodeConfig::AgentTask {
        prompt: "Review the diff twice.".to_string(),
    });
    save(&edited);
    let after = files(&dir);
    for (name, bytes) in &before {
        if name == "review.md" {
            assert_ne!(&after[name], bytes);
        } else {
            assert_eq!(&after[name], bytes, "{name} must stay byte-identical");
        }
    }
    assert_eq!(read_flow_dir(&dir).expect("load"), edited);

    let mut removed = edited.clone();
    removed.nodes.remove("check");
    removed
        .edges
        .retain(|edge| edge.from != "check" && edge.to != "check");
    removed.edges.push(attractor_core::FlowEdge {
        from: "start".to_string(),
        to: "review".to_string(),
        ..Default::default()
    });
    save(&removed);
    assert!(!dir.join("check.md").exists());
    assert_eq!(read_flow_dir(&dir).expect("load"), removed.normalize());
    assert!(!files(&dir).keys().any(|name| name.starts_with('.')));
}

#[test]
fn flow_refs_resolve_against_the_directory_containing_the_parent_flow() {
    let temp = tempfile::tempdir().expect("tempdir");
    let flows_dir = temp.path().join("flows");
    let child = parse_flow_definition(
        "id: child\nnodes:\n  start: {kind: start}\n  done: {kind: exit}\nedges:\n  - {from: start, to: done}\n",
    )
    .expect("child");
    write_flow_dir(&flows_dir.join("team/workers/child"), &child).expect("child");
    let parent = parse_flow_definition(
        "id: parent\nnodes:\n  start: {kind: start}\n  run:\n    kind: subflow\n    config: {kind: subflow, flow_ref: workers/child}\n  done: {kind: exit}\nedges:\n  - {from: start, to: run}\n  - {from: run, to: done}\n",
    )
    .expect("parent");
    write_flow_dir(&flows_dir.join("team/parent"), &parent).expect("parent");

    // The runtime resolves against internal.flow_source_dir, the directory
    // containing the parent's directory; preview uses the same resolver.
    assert_eq!(
        resolve_flow_ref(&flows_dir.join("team"), "workers/child"),
        flows_dir.join("team/workers/child")
    );
    let preview = preview_with_flows_dir(
        PreviewRequest {
            flow_content: flow_yaml(&parent).expect("yaml"),
            flow_name: Some("team/parent".to_string()),
            expand_children: true,
        },
        &flows_dir,
    );
    assert_eq!(
        preview.body["graph"]["child_previews"]["run"]["flow_name"],
        json!("team/workers/child"),
        "{}",
        preview.body
    );

    assert!(attractor_dsl::validate_flow_definition(&{
        let mut stale = parent.clone();
        stale.nodes.get_mut("run").expect("run").config = Some(NodeConfig::Subflow {
            flow_ref: "workers/child.yaml".to_string(),
            input_map: BTreeMap::new(),
        });
        stale
    })
    .is_err());
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
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
    }
}
