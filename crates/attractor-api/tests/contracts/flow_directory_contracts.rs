use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use attractor_api::{preview_with_roots, AttractorApiService, PreviewRequest, SaveFlowRequest};
use attractor_core::{ExecutionConfig, NodeConfig};
use attractor_dsl::{
    flow_ref_name, flow_yaml, load_flow, parse_flow_definition, read_flow_dir, resolve_flow_roles,
    write_flow_dir, FlowDefinition, FlowRoots,
};
use serde_json::{json, Value};
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

/// A project with an org repo that has a local git identity, since editor
/// saves commit under the repo's normal git config.
fn project_with_org(root: &Path) -> std::path::PathBuf {
    let project = root.join("project");
    let org = spark_common::project::ensure_org_repo(&project).expect("org repo");
    for (key, value) in [
        ("user.name", "Editor Test"),
        ("user.email", "editor@example.com"),
    ] {
        spark_common::project::run_git(&org, &["config", key, value]).expect("git config");
    }
    project
}

fn save_request(name: &str, content: String, project: &Path) -> SaveFlowRequest {
    SaveFlowRequest {
        name: name.to_string(),
        content,
        project_path: Some(project.to_string_lossy().to_string()),
    }
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

    // Only the OKF-reserved files are ignored.
    fs::write(dir.join("index.md"), "# Index\n").expect("index");
    fs::write(dir.join("log.md"), "# Log\n").expect("log");
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
        "",
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

    fs::write(dir.join(".start.md.swp"), "swap").expect("swap");
    fs::create_dir(dir.join(".git")).expect("hidden dir");
    fs::write(dir.join("notes.txt"), "notes").expect("notes");
    fs::write(dir.join("old.yaml"), FLOW).expect("yaml");
    let error = read_flow_dir(&dir).expect_err("unknown files");
    assert!(
        error
            .detail()
            .contains(".git, .start.md.swp, notes.txt, old.yaml"),
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

    let resolved =
        resolve_flow_roles(&flow(), &FlowRoots::new(temp.path(), None)).expect("resolve");
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

    let error = resolve_flow_roles(&flow(), &FlowRoots::new(temp.path().join("missing"), None))
        .expect_err("unknown");
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
    let project = project_with_org(temp.path());
    let service = AttractorApiService::new(settings.clone());

    let unknown_role = service.save_flow(save_request("review", FLOW.to_string(), &project));
    assert_eq!(unknown_role.status_code, 422, "{}", unknown_role.body);
    assert!(unknown_role
        .body
        .to_string()
        .contains("unknown role 'reviewer'"));

    // Settings a role contributes are validated like the node's own.
    fs::create_dir_all(settings.bundled_dir().join("roles")).expect("roles");
    for (execution, expected) in [
        ("reasoning_mode: turbo", "reasoning_mode must be one of"),
        ("llm_profile: missing-profile", "Unknown profile reference"),
    ] {
        fs::write(
            settings.bundled_dir().join("roles").join("reviewer.md"),
            format!("---\ntype: Role\nexecution:\n  {execution}\n---\n\nPersona.\n"),
        )
        .expect("role");
        let rejected = service.save_flow(save_request("review", FLOW.to_string(), &project));
        assert_ne!(rejected.status_code, 200, "{}", rejected.body);
        assert!(
            rejected.body.to_string().contains(expected),
            "{}",
            rejected.body
        );
    }
    let org = spark_common::project::org_dir(&project);
    assert!(!org.join("flows/review").exists());

    fs::write(
        settings.bundled_dir().join("roles").join("reviewer.md"),
        ROLE,
    )
    .expect("role");
    let saved = service.save_flow(save_request("review", FLOW.to_string(), &project));
    assert_eq!(saved.status_code, 200, "{}", saved.body);

    let loaded = service.get_flow("review", project.to_str());
    let content = loaded.body["content"].as_str().expect("content");
    assert!(content.contains("role: reviewer"));
    assert!(!content.contains("careful reviewer"));
    assert_eq!((parse_flow_definition(content).expect("authored")), flow());

    let roots = FlowRoots::new(settings.bundled_dir(), Some(&project));
    let resolved = flow_yaml(&load_flow(&roots, "review").expect("resolved").flow).expect("yaml");
    assert!(resolved.contains("You are a careful reviewer."));
    assert!(!resolved.contains("role: reviewer"));

    // A project role replaces the bundled role of the same name.
    fs::create_dir_all(org.join("roles")).expect("org roles");
    fs::write(
        org.join("roles/reviewer.md"),
        "---\ntype: Role\n---\n\nYou are the project's reviewer.\n",
    )
    .expect("org role");
    let resolved = flow_yaml(&load_flow(&roots, "review").expect("resolved").flow).expect("yaml");
    assert!(
        resolved.contains("You are the project's reviewer."),
        "{resolved}"
    );
    assert!(!resolved.contains("careful reviewer"));
}

#[test]
fn editor_save_rewrites_only_changed_files_and_deletes_removed_nodes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(settings.bundled_dir().join("roles")).expect("roles");
    fs::write(
        settings.bundled_dir().join("roles").join("reviewer.md"),
        ROLE,
    )
    .expect("role");
    let project = project_with_org(temp.path());
    let service = AttractorApiService::new(settings.clone());
    let save = |flow: &FlowDefinition| {
        let response = service.save_flow(save_request(
            "review",
            flow_yaml(flow).expect("yaml"),
            &project,
        ));
        assert_eq!(response.status_code, 200, "{}", response.body);
    };
    save(&flow());
    let dir = spark_common::project::org_dir(&project).join("flows/review");

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
    assert_eq!((read_flow_dir(&dir).expect("load")), edited);

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
    assert_eq!((read_flow_dir(&dir).expect("load")), removed.normalize());
    assert!(!files(&dir).keys().any(|name| name.starts_with('.')));
    // Temp files live beside the flow directory and are renamed away.
    assert!(fs::read_dir(dir.parent().expect("parent"))
        .expect("parent entries")
        .all(|entry| !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
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

    // A ref names the flow beside the parent; the runtime and preview then
    // resolve that name through the roots.
    assert_eq!(
        flow_ref_name(Some("team/parent"), "workers/child"),
        Some("team/workers/child".to_string())
    );
    assert_eq!(
        flow_ref_name(None, "workers/child"),
        Some("workers/child".to_string())
    );
    let preview = preview_with_roots(
        PreviewRequest {
            flow_content: flow_yaml(&parent).expect("yaml"),
            flow_name: Some("team/parent".to_string()),
            expand_children: true,
            project_path: None,
        },
        &FlowRoots::new(temp.path(), None),
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

#[test]
fn backslash_and_traversal_flow_refs_are_rejected_by_validation_preview_and_resolver() {
    let temp = tempfile::tempdir().expect("tempdir");
    let flows_dir = temp.path().join("flows");
    let child = parse_flow_definition(
        "id: child\nnodes:\n  start: {kind: start}\n  done: {kind: exit}\nedges:\n  - {from: start, to: done}\n",
    )
    .expect("child");
    write_flow_dir(&flows_dir.join("team/workers/child"), &child).expect("child");
    write_flow_dir(&flows_dir.join("outside"), &child).expect("outside");
    let parent_with = |flow_ref: &str| {
        let mut parent = parse_flow_definition(
            "id: parent\nnodes:\n  start: {kind: start}\n  run:\n    kind: subflow\n    config: {kind: subflow, flow_ref: workers/child}\n  done: {kind: exit}\nedges:\n  - {from: start, to: run}\n  - {from: run, to: done}\n",
        )
        .expect("parent");
        parent.nodes.get_mut("run").expect("run").config = Some(NodeConfig::Subflow {
            flow_ref: flow_ref.to_string(),
            input_map: BTreeMap::new(),
        });
        parent
    };
    let child_preview = |parent: &FlowDefinition| {
        preview_with_roots(
            PreviewRequest {
                flow_content: flow_yaml(parent).expect("yaml"),
                flow_name: Some("team/parent".to_string()),
                expand_children: true,
                project_path: None,
            },
            &FlowRoots::new(temp.path(), None),
        )
        .body["graph"]["child_previews"]["run"]["flow_name"]
            .clone()
    };

    // A backslash-separated relative ref is the same ref everywhere.
    let parent = parent_with("workers\\child");
    attractor_dsl::validate_flow_definition(&parent).expect("backslash ref is valid");
    assert_eq!(
        flow_ref_name(Some("team/parent"), "workers\\child"),
        Some("team/workers/child".to_string())
    );
    assert_eq!(child_preview(&parent), json!("team/workers/child"));

    for flow_ref in [
        "..\\outside",
        "\\absolute\\path",
        "../outside",
        "workers/../../outside",
    ] {
        let parent = parent_with(flow_ref);
        let error = attractor_dsl::validate_flow_definition(&parent).expect_err(flow_ref);
        assert!(
            error.diagnostics.iter().any(|d| d.rule_id == "subflow_ref"),
            "{flow_ref}: {error}"
        );
        assert_eq!(flow_ref_name(Some("team/parent"), flow_ref), None);
        assert_eq!(child_preview(&parent), Value::Null, "{flow_ref}");
    }
}

#[test]
fn authored_child_workdir_does_not_change_the_flow_ref_base() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let child = parse_flow_definition(
        "id: child\ntitle: Child\nnodes:\n  start: {kind: start}\n  done: {kind: exit}\nedges:\n  - {from: start, to: done}\n",
    )
    .expect("child");
    write_flow_dir(
        &settings
            .bundled_dir()
            .join("flows")
            .join("team/workers/child"),
        &child,
    )
    .expect("child");
    let parent = parse_flow_definition(
        "id: parent\nnodes:\n  start: {kind: start}\n  run:\n    kind: subflow\n    config: {kind: subflow, flow_ref: workers/child}\n    manager: {max_cycles: 1, child_autostart: true, child_workdir: worktree}\n  done: {kind: exit}\nedges:\n  - {from: start, to: run}\n  - {from: run, to: done}\n",
    )
    .expect("parent");
    write_flow_dir(
        &settings.bundled_dir().join("flows").join("team/parent"),
        &parent,
    )
    .expect("parent");
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("worktree")).expect("worktree");

    let preview = preview_with_roots(
        PreviewRequest {
            flow_content: flow_yaml(&parent).expect("yaml"),
            flow_name: Some("team/parent".to_string()),
            expand_children: true,
            project_path: None,
        },
        &FlowRoots::new(settings.bundled_dir(), None),
    );
    assert_eq!(
        preview.body["graph"]["child_previews"]["run"]["flow_name"],
        json!("team/workers/child"),
        "{}",
        preview.body
    );

    let service = AttractorApiService::new(settings.clone());
    let started = service.start_pipeline(attractor_api::PipelineStartRequest {
        wait: Some(true),
        run_id: Some("parent-run".to_string()),
        flow_name: Some("team/parent".to_string()),
        working_directory: project.to_string_lossy().into_owned(),
        ..Default::default()
    });
    assert_eq!(started.body["status"], "started", "{}", started.body);
    let store = attractor_runtime::RunStore::for_settings(&settings);
    let parent_record = store
        .read_run_bundle("parent-run")
        .expect("read")
        .expect("run")
        .record
        .expect("record");
    assert_eq!(parent_record.status, "completed", "{parent_record:?}");
    let child_record = store
        .list_run_records()
        .expect("runs")
        .into_iter()
        .find(|record| record.parent_run_id.as_deref() == Some("parent-run"))
        .expect("child run");
    assert_eq!(child_record.flow_name, "Child");
    assert_eq!(
        child_record.working_directory,
        project.join("worktree").to_string_lossy()
    );
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
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

#[test]
fn named_launches_execute_the_resolved_form() {
    use std::io::{BufRead, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    fs::write(
        settings.config_dir.join("spark.toml"),
        format!("[models]\nprovider='openai_compatible'\nmodel='config-model'\n[providers.openai_compatible]\nbase_url='http://{address}/v1'\n[agents]\nmax_turns=5\n"),
    )
    .expect("spark.toml");
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        Arc::new(|| {
            attractor_runtime::RuntimeHandlerRunner::new()
                .with_rust_llm_client(unified_llm_adapter::Client::new())
        }),
    );
    let flow = r#"id: inline-review
nodes:
  start: {kind: start}
  first:
    kind: agent_task
    role: reviewer
    config: {kind: agent_task, prompt: Review the first part.}
  second:
    kind: agent_task
    role: reviewer
    config: {kind: agent_task, prompt: Review the second part.}
    execution: {llm_model: node-model}
  done: {kind: exit}
edges:
  - {from: start, to: first}
  - {from: first, to: second}
  - {from: second, to: done}
"#;
    // Runs launch the flow on disk by name.
    let name = crate::install_flow(&settings, flow);
    let launch = |run_id: &str| {
        service.start_pipeline(attractor_api::PipelineStartRequest {
            wait: Some(true),
            run_id: Some(run_id.to_string()),
            flow_name: Some(name.clone()),
            model: Some("launch-model".to_string()),
            llm_provider: Some("openai_compatible".to_string()),
            working_directory: temp.path().join("project").to_string_lossy().into_owned(),
            ..Default::default()
        })
    };

    let unknown = launch("unknown-role");
    assert_eq!(
        unknown.body["status"], "validation_error",
        "{}",
        unknown.body
    );
    assert!(unknown.body["error"]
        .as_str()
        .is_some_and(|error| error.contains("unknown role 'reviewer'")));

    fs::create_dir_all(settings.bundled_dir().join("roles")).expect("roles");
    fs::write(
        settings.bundled_dir().join("roles").join("reviewer.md"),
        "---\ntype: Role\nexecution:\n  llm_model: role-model\n---\n\nYou are a careful reviewer.\n",
    )
    .expect("role");
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut requests = Vec::new();
        while requests.len() < 2 {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("missing LLM request: {error}"),
            };
            stream.set_nonblocking(false).expect("blocking");
            let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header");
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().expect("length");
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("body");
            let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
            let response = json!({"id":"r","object":"chat.completion","model":body["model"],"choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).expect("respond");
            requests.push(body);
        }
        requests
    });

    let started = launch("inline-roles");
    assert_eq!(started.body["status"], "started", "{}", started.body);
    let requests = server.join().expect("server");
    // A role's model beats the launch override; the node's own beats the role's.
    assert_eq!(requests[0]["model"], "role-model");
    assert_eq!(requests[1]["model"], "node-model");
    for (request, part) in requests.iter().zip(["first", "second"]) {
        let body = request.to_string();
        assert!(
            body.contains(
                "You are a careful reviewer.\\n\\nReview the {part} part."
                    .replace("{part}", part)
                    .as_str()
            ),
            "{body}"
        );
    }

    let bundle = attractor_runtime::RunStore::for_settings(&settings)
        .read_run_bundle("inline-roles")
        .expect("read")
        .expect("run");
    assert_eq!(bundle.record.expect("record").status, "completed");
    let snapshot = fs::read_to_string(bundle.paths.artifacts_dir().join("flow/flow-source.yaml"))
        .expect("snapshot");
    assert!(
        snapshot.contains("You are a careful reviewer."),
        "{snapshot}"
    );
    assert!(!snapshot.contains("role:"), "{snapshot}");
}

#[test]
fn launches_resolve_child_flows_by_name_to_the_project_copy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let bundled = settings.bundled_dir().join("flows");
    let child = |title: &str| {
        parse_flow_definition(&format!(
            "id: child\ntitle: {title}\nnodes:\n  start: {{kind: start}}\n  done: {{kind: exit}}\nedges:\n  - {{from: start, to: done}}\n"
        ))
        .expect("child")
    };
    write_flow_dir(&bundled.join("team/workers/child"), &child("Bundled Child")).expect("child");
    let parent = parse_flow_definition(
        "id: parent\nnodes:\n  start: {kind: start}\n  run:\n    kind: subflow\n    config: {kind: subflow, flow_ref: workers/child}\n    manager: {max_cycles: 1, child_autostart: true}\n  done: {kind: exit}\nedges:\n  - {from: start, to: run}\n  - {from: run, to: done}\n",
    )
    .expect("parent");
    write_flow_dir(&bundled.join("team/parent"), &parent).expect("parent");
    let project = temp.path().join("project");
    write_flow_dir(
        &spark_common::project::org_dir(&project).join("flows/team/workers/child"),
        &child("Project Child"),
    )
    .expect("project child");
    let service = AttractorApiService::new(settings.clone());
    let launch = |run_id: &str, project_path: Option<&Path>| {
        let started = service.start_pipeline(attractor_api::PipelineStartRequest {
            wait: Some(true),
            run_id: Some(run_id.to_string()),
            flow_name: Some("team/parent".to_string()),
            working_directory: project.to_string_lossy().into_owned(),
            project_path: project_path.map(|path| path.to_string_lossy().into_owned()),
            ..Default::default()
        });
        assert_eq!(started.body["status"], "started", "{}", started.body);
        attractor_runtime::RunStore::for_settings(&settings)
            .list_run_records()
            .expect("runs")
            .into_iter()
            .find(|record| record.parent_run_id.as_deref() == Some(run_id))
            .expect("child run")
            .flow_name
    };

    // The parent is bundled; its child resolves by name, project copy first.
    assert_eq!(launch("with-project", Some(&project)), "Project Child");
    // A launch with no project uses bundled flows only.
    assert_eq!(launch("bundled-only", None), "Bundled Child");
}

#[test]
fn run_manifests_hash_each_file_by_layer_with_org_state_and_spark_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let bundled = settings.bundled_dir();
    write_flow_dir(&bundled.join("flows/review"), &flow()).expect("bundled flow");
    fs::create_dir_all(bundled.join("roles")).expect("roles");
    fs::write(bundled.join("roles/reviewer.md"), ROLE).expect("role");
    let hash = |bytes: &[u8]| spark_common::project::sha256_hex(bytes);

    // No project: bundled files only, and no org.
    let loaded = load_flow(&FlowRoots::new(&bundled, None), "review").expect("load");
    let manifest = serde_json::to_value(&loaded.manifest).expect("manifest");
    assert_eq!(manifest["org"], Value::Null);
    assert_eq!(manifest["spark_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        manifest["files"]["bundled/roles/reviewer.md"],
        json!(hash(ROLE.as_bytes()))
    );
    assert_eq!(
        manifest["files"]["bundled/flows/review/flow.md"],
        json!(hash(
            &fs::read(bundled.join("flows/review/flow.md")).expect("flow.md")
        ))
    );
    assert_eq!(manifest["files"].as_object().expect("files").len(), 6);

    // A project copy is hashed under org/, with the org repo's state.
    let project = project_with_org(temp.path());
    let org = spark_common::project::org_dir(&project);
    let roots = FlowRoots::new(&bundled, Some(&project));
    write_flow_dir(&org.join("flows/review"), &flow()).expect("org flow");
    spark_common::project::run_git(&org, &["add", "-A"]).expect("add");
    spark_common::project::run_git(&org, &["commit", "-q", "-m", "Add review"]).expect("commit");
    let head = spark_common::project::run_git(&org, &["rev-parse", "HEAD"]).expect("head");
    let manifest = serde_json::to_value(load_flow(&roots, "review").expect("load").manifest)
        .expect("manifest");
    assert_eq!(manifest["org"], json!({"commit": head, "dirty": false}));
    let keys = manifest["files"]
        .as_object()
        .expect("files")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        keys.contains(&"org/flows/review/review.md".to_string()),
        "{keys:?}"
    );
    assert!(
        keys.contains(&"bundled/roles/reviewer.md".to_string()),
        "{keys:?}"
    );
    assert!(
        keys.iter().all(|key| !key.starts_with("bundled/flows/")),
        "{keys:?}"
    );

    fs::write(org.join("notes.md"), "uncommitted\n").expect("dirty");
    let manifest = serde_json::to_value(load_flow(&roots, "review").expect("load").manifest)
        .expect("manifest");
    assert_eq!(manifest["org"], json!({"commit": head, "dirty": true}));

    // Launches record the manifest beside the flow snapshot.
    let started = AttractorApiService::new(settings.clone()).start_pipeline(
        attractor_api::PipelineStartRequest {
            run_id: Some("manifest-run".to_string()),
            flow_name: Some("review".to_string()),
            working_directory: project.to_string_lossy().into_owned(),
            project_path: Some(project.to_string_lossy().into_owned()),
            ..Default::default()
        },
    );
    assert_eq!(started.status_code, 200, "{}", started.body);
    let recorded: Value = serde_json::from_slice(
        &fs::read(
            started.body["flow_files_path"]
                .as_str()
                .or_else(|| started.body["graph_paths"]["flow_files_path"].as_str())
                .expect("files.json path"),
        )
        .expect("files.json"),
    )
    .expect("json");
    assert_eq!(recorded, manifest);
}
