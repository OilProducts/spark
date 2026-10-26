use std::fs;
use std::path::Path;
use std::process::Command;

use attractor_api::{preview_with_roots, AttractorApiService, PreviewRequest, SaveFlowRequest};
use attractor_core::FlowDefinition;
use attractor_dsl::{read_flow_dir, validated_flow, write_flow_dir, FlowFileText, FlowRoots};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

fn valid_flow_json() -> Value {
    json!({
    "schema_version": "1",
    "id": "workflow",
    "title": "Workflow",
    "goal": "Do work",
    "nodes": {
        "start": {"kind": "start", "label": "Start"},
        "task": {
            "kind": "agent_task",
            "label": "Task",
            "config": {"kind": "agent_task", "prompt": "Do work"}
        },
        "done": {"kind": "exit", "label": "Done"}
    },
    "edges": [{"from": "start", "to": "task"}, {"from": "task", "to": "done"}]
    })
}

fn flow(value: Value) -> FlowDefinition {
    validated_flow(serde_json::from_value(value).expect("flow definition JSON"))
        .expect("valid flow")
}

fn valid_flow() -> FlowDefinition {
    flow(valid_flow_json())
}

/// The valid flow with the task's prompt replaced.
fn with_prompt(prompt: &str) -> FlowDefinition {
    let mut value = valid_flow_json();
    value["nodes"]["task"]["config"]["prompt"] = json!(prompt);
    flow(value)
}

#[test]
fn list_and_get_layer_the_project_org_over_bundled_flows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let bundled = settings.bundled_dir().join("flows");
    write_flow_dir(&bundled.join("examples/simple-linear"), &valid_flow()).expect("write flow");
    write_flow_dir(&bundled.join("root"), &valid_flow()).expect("write flow");
    // Retired YAML files are not flows.
    fs::write(
        bundled.join("legacy.yaml"),
        "schema_version: \"1\"\nid: legacy\n",
    )
    .expect("write yaml");
    let org = spark_common::project::org_dir(&project).join("flows");
    let mut project_root = valid_flow();
    project_root.title = "Project Root".to_string();
    write_flow_dir(&org.join("root"), &project_root).expect("write flow");
    write_flow_dir(&org.join("project-only"), &valid_flow()).expect("write flow");
    let service = AttractorApiService::new(settings);
    let project_path = project.to_str();

    // No project: bundled only.
    let list = service.list_flows(None);
    assert_eq!(list.status_code, 200);
    assert_eq!(list.body, json!(["examples/simple-linear", "root"]));
    assert_eq!(service.get_flow("root", None).body["layer"], "bundled");
    assert_eq!(service.get_flow("project-only", None).status_code, 404);

    // A project file with the same name replaces the bundled one.
    let list = service.list_flows(project_path);
    assert_eq!(
        list.body,
        json!(["examples/simple-linear", "project-only", "root"])
    );
    let root = service.get_flow("root", project_path);
    assert_eq!(root.status_code, 200);
    assert_eq!(root.body["layer"], "org");
    assert_eq!(root.body["flow"]["title"], "Project Root");
    assert!(root.body["files"][0]["content"]
        .as_str()
        .expect("flow.md")
        .contains("Project Root"));
    let get = service.get_flow("examples/simple-linear", project_path);
    assert_eq!(get.body["name"], json!("examples/simple-linear"));
    assert_eq!(get.body["layer"], "bundled");
    assert_eq!(flow(get.body["flow"].clone()), valid_flow());

    // A missing org repo falls back to bundled; reads create nothing.
    let elsewhere = temp.path().join("no-org");
    assert_eq!(
        service.list_flows(elsewhere.to_str()).body,
        json!(["examples/simple-linear", "root"])
    );
    assert!(!elsewhere.exists());
}

#[test]
fn get_and_delete_missing_flow_match_error_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let get = service.get_flow("missing", None);
    assert_eq!(get.status_code, 404);
    assert_eq!(get.body, json!({"detail": "Flow not found: missing"}));

    let delete = service.delete_flow("missing", None);
    assert_eq!(delete.status_code, 404);
    assert_eq!(delete.body, json!({"detail": "Flow not found: missing"}));
}

#[test]
fn flow_name_safety_errors_are_exposed_through_api_responses() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));
    let get = |name: &str| service.get_flow(name, None).body;

    assert_eq!(get(""), json!({"detail": "Flow name is required."}));
    let outside =
        json!({"detail": "Flow name must be a relative path inside the flows directory."});
    assert_eq!(get("/tmp/escape.yaml"), outside);
    assert_eq!(get("../escape.yaml"), outside);
    assert_eq!(
        get("nested/"),
        json!({"detail": "Flow name must not end with '/'."})
    );
    assert_eq!(
        get("nested/flow.yaml"),
        json!({"detail": "Flow names name a flow directory; drop the file extension."})
    );
}

#[test]
fn named_flow_sources_preview_through_the_roots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow_dir(
        &settings.bundled_dir().join("flows/nested/helper"),
        &valid_flow(),
    )
    .expect("write flow");
    let roots = FlowRoots::new(settings.bundled_dir(), None);

    assert_eq!(
        roots.list_flow_names().expect("names"),
        vec!["nested/helper"]
    );
    let source = roots
        .read_named_flow_source("nested/helper")
        .expect("source");
    assert_eq!(source.name, "nested/helper");
    assert_eq!(
        source.path,
        settings.bundled_dir().join("flows/nested/helper")
    );
    assert_eq!(source.flow, valid_flow());

    let preview = preview_with_roots(
        PreviewRequest {
            flow_name: Some(source.name.clone()),
            ..Default::default()
        },
        &roots,
    );
    assert_eq!(preview.status_code, 200);
    assert_eq!(preview.body["status"], "ok");
    assert!(preview.body["graph"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .any(|node| node["id"] == json!("start")));
}

#[test]
fn save_of_a_flow_that_is_not_a_flow_definition_is_a_bad_request() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.dispatch(
        "POST",
        "/api/flows",
        &json!({"name": "bad", "project_path": project, "flow": "nodes: [\n"}).to_string(),
    );

    assert_eq!(response.status_code, 400, "{}", response.body);
    assert!(response.body["detail"]
        .as_str()
        .expect("detail")
        .starts_with("Invalid flow save request JSON"));
    assert!(!project.exists());
}

#[test]
fn save_takes_exactly_one_of_flow_or_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let service = AttractorApiService::new(settings(temp.path()));
    let post = |body: Value| service.dispatch("POST", "/api/flows", &body.to_string());

    let neither = post(json!({"name": "work", "project_path": project}));
    let both = post(json!({
        "name": "work",
        "project_path": project,
        "flow": valid_flow(),
        "files": [{"name": "flow.md", "content": "---\ntype: Flow\n---\n"}],
    }));

    for response in [neither, both] {
        assert_eq!(response.status_code, 400, "{}", response.body);
        assert_eq!(
            response.body["detail"],
            json!("A flow save takes exactly one of flow or files.")
        );
    }
    assert!(!project.exists());
}

#[test]
fn get_flow_returns_the_exact_files_flow_md_first_and_the_flow_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let dir = settings.bundled_dir().join("flows/work");
    write_flow_dir(&dir, &valid_flow()).expect("write flow");
    // A hand-written file is returned as written, not re-rendered.
    let task = annotated(&fs::read_to_string(dir.join("task.md")).expect("task.md"));
    fs::write(dir.join("task.md"), &task).expect("annotate");
    let service = AttractorApiService::new(settings);

    let get = service.get_flow("work", None);

    assert_eq!(get.status_code, 200, "{}", get.body);
    assert_eq!(get.body["name"], "work");
    assert_eq!(get.body["layer"], "bundled");
    assert!(get.body.get("content").is_none());
    let files = get.body["files"].as_array().expect("files");
    let names = files
        .iter()
        .map(|file| file["name"].as_str().expect("name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["flow.md", "done.md", "start.md", "task.md"]);
    for file in files {
        let name = file["name"].as_str().expect("name");
        assert_eq!(
            file["content"].as_str().expect("content"),
            fs::read_to_string(dir.join(name)).expect("file"),
            "{name}"
        );
    }
    assert_eq!(flow(get.body["flow"].clone()), valid_flow());
}

#[test]
fn flow_save_rewrites_only_the_changed_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let org = init_org(&project);
    let service = AttractorApiService::new(settings(temp.path()));
    assert_eq!(
        service
            .save_flow(save("work", &valid_flow(), &project))
            .status_code,
        200
    );
    let dir = org.join("flows/work");
    // Hand formatting in an untouched file survives a structured save.
    let start = annotated(&fs::read_to_string(dir.join("start.md")).expect("start.md"));
    fs::write(dir.join("start.md"), &start).expect("annotate");
    let before = flow_dir_bytes(&dir);

    let saved = service.save_flow(save("work", &with_prompt("Do better work"), &project));

    assert_eq!(saved.status_code, 200, "{}", saved.body);
    let after = flow_dir_bytes(&dir);
    for (name, bytes) in &before {
        if name == "task.md" {
            assert_ne!(&after[name], bytes);
        } else {
            assert_eq!(&after[name], bytes, "{name} was rewritten");
        }
    }
    assert_eq!(
        read_flow_dir(&dir).expect("flow"),
        with_prompt("Do better work")
    );
}

#[test]
fn files_save_writes_the_submitted_bytes_exactly_and_commits_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let org = init_org(&project);
    let service = AttractorApiService::new(settings(temp.path()));
    assert_eq!(
        service
            .save_flow(save("work", &valid_flow(), &project))
            .status_code,
        200
    );
    let dir = org.join("flows/work");
    let before = flow_dir_bytes(&dir);
    let task = annotated(&before["task.md"]).replace("Do work", "Do files work");
    assert_ne!(task, before["task.md"]);

    let saved = service.save_flow(save_files("work", &[("task.md", &task)], &project));

    assert_eq!(saved.status_code, 200, "{}", saved.body);
    assert_eq!(saved.body, json!({"status": "saved", "name": "work"}));
    assert_eq!(
        fs::read_to_string(dir.join("task.md")).expect("task.md"),
        task
    );
    let after = flow_dir_bytes(&dir);
    for (name, bytes) in &before {
        if name != "task.md" {
            assert_eq!(&after[name], bytes, "{name} was rewritten");
        }
    }
    assert_eq!(
        read_flow_dir(&dir).expect("flow"),
        with_prompt("Do files work")
    );
    assert_eq!(
        last_commit(&org),
        "Edit work in the flow editor\nflows/work/task.md"
    );
    assert_eq!(git(&org, &["status", "--porcelain"]), "");
    assert_eq!(
        service.get_flow("work", project.to_str()).body["files"][3]["content"],
        json!(task)
    );
}

#[test]
fn files_save_that_fails_validation_or_names_an_unknown_file_writes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let org = init_org(&project);
    let service = AttractorApiService::new(settings(temp.path()));
    assert_eq!(
        service
            .save_flow(save("work", &valid_flow(), &project))
            .status_code,
        200
    );
    let dir = org.join("flows/work");
    let before = flow_dir_bytes(&dir);
    let head = git(&org, &["rev-parse", "HEAD"]);
    let start = before["start.md"].replace("to: task", "to: missing");
    assert_ne!(start, before["start.md"]);
    let task = before["task.md"].replace("Do work", "Do other work");

    // The task edit is valid on its own; the broken start edge fails the whole.
    let invalid = service.save_flow(save_files(
        "work",
        &[("task.md", &task), ("start.md", &start)],
        &project,
    ));
    assert_eq!(invalid.status_code, 422, "{}", invalid.body);
    assert_eq!(invalid.body["detail"]["status"], json!("validation_error"));

    let unknown = service.save_flow(save_files(
        "work",
        &[("task.md", &task), ("extra.md", "---\nkind: exit\n---\n")],
        &project,
    ));
    assert_eq!(unknown.status_code, 422, "{}", unknown.body);
    assert_eq!(unknown.body["detail"]["status"], json!("validation_error"));
    assert!(unknown.body["detail"]["error"]
        .as_str()
        .expect("error")
        .contains("extra.md is not a file of flow work"));

    assert_eq!(flow_dir_bytes(&dir), before);
    assert_eq!(git(&org, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&org, &["status", "--porcelain"]), "");
}

#[test]
fn files_save_of_a_bundled_flow_commits_the_copy_then_the_edit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let org = init_org(&project);
    let bundled = settings.bundled_dir().join("flows/shared");
    write_flow_dir(&bundled, &valid_flow()).expect("write flow");
    let before = flow_dir_bytes(&bundled);
    let service = AttractorApiService::new(settings);
    let task = annotated(&before["task.md"]).replace("Do work", "Project work");

    let saved = service.save_flow(save_files("shared", &[("task.md", &task)], &project));

    assert_eq!(saved.status_code, 200, "{}", saved.body);
    // The bundled library is untouched.
    assert_eq!(flow_dir_bytes(&bundled), before);
    assert_eq!(
        git(&org, &["log", "--format=%s"]),
        "Edit shared in the flow editor\nCopy shared from bundled"
    );
    assert_eq!(
        last_commit(&org),
        "Edit shared in the flow editor\nflows/shared/task.md"
    );
    assert_eq!(
        git(&org, &["show", "HEAD~1:flows/shared/task.md"]),
        before["task.md"].trim()
    );
    let copy = flow_dir_bytes(&org.join("flows/shared"));
    assert_eq!(copy["task.md"], task);
    for name in ["done.md", "flow.md", "start.md"] {
        assert_eq!(copy[name], before[name], "{name}");
    }
    assert_eq!(
        service.get_flow("shared", project.to_str()).body["layer"],
        "org"
    );
}

#[test]
fn save_validation_error_reports_diagnostics_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.save_flow(save(
        "broken",
        &serde_json::from_value(json!({
            "schema_version": "1",
            "id": "broken",
            "nodes": {"start": {"kind": "start"}, "done": {"kind": "exit"}},
            "edges": [{"from": "start", "to": "missing"}]
        }))
        .expect("flow definition JSON"),
        &project,
    ));

    assert_eq!(response.status_code, 422);
    assert_eq!(response.body["detail"]["status"], json!("validation_error"));
    assert_eq!(
        response.body["detail"]["errors"][0]["rule_id"],
        json!("edge_target")
    );
    assert_eq!(
        response.body["detail"]["errors"][0]["edge"],
        json!(["start", "missing"])
    );
    assert!(!project.exists());
}

#[test]
fn save_requires_a_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());

    let response = service.save_flow(SaveFlowRequest {
        name: "flow".to_string(),
        flow: Some(valid_flow()),
        files: None,
        project_path: None,
    });

    assert_eq!(response.status_code, 400);
    assert!(!settings.bundled_dir().join("flows/flow").exists());
}

#[test]
fn org_repo_is_created_on_first_write_with_an_ignored_spark_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let org = spark_common::project::ensure_org_repo(&project).expect("org repo");
    assert_eq!(org, project.join(".spark/agent-org"));
    assert!(org.join(".git").is_dir());
    assert_eq!(
        fs::read_to_string(project.join(".spark/.gitignore")).expect("gitignore"),
        "*\n"
    );
    // An existing .gitignore is kept.
    fs::write(project.join(".spark/.gitignore"), "custom\n").expect("custom");
    spark_common::project::ensure_org_repo(&project).expect("org repo again");
    assert_eq!(
        fs::read_to_string(project.join(".spark/.gitignore")).expect("gitignore"),
        "custom\n"
    );
}

#[test]
fn save_splits_nested_flow_into_an_org_directory_and_commits_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let org = init_org(&project);
    let service = AttractorApiService::new(settings.clone());

    let created = service.save_flow(save("nested/new-flow", &valid_flow(), &project));
    assert_eq!(created.status_code, 200, "{}", created.body);
    assert_eq!(
        created.body,
        json!({"status": "saved", "name": "nested/new-flow"})
    );
    let flow_dir = org.join("flows/nested/new-flow");
    let mut files = fs::read_dir(&flow_dir)
        .expect("flow dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("name")
        })
        .collect::<Vec<_>>();
    files.sort();
    assert_eq!(files, ["done.md", "flow.md", "start.md", "task.md"]);
    assert!(fs::read_to_string(flow_dir.join("flow.md"))
        .expect("flow.md")
        .starts_with("---\ntype: Flow\n"));
    assert_eq!(
        last_commit(&org),
        "Edit nested/new-flow in the flow editor\nflows/nested/new-flow/done.md\nflows/nested/new-flow/flow.md\nflows/nested/new-flow/start.md\nflows/nested/new-flow/task.md"
    );
    assert!(!settings.bundled_dir().join("flows/nested").exists());

    // A folder of flows, or a path inside a flow, is not a flow directory.
    for name in ["nested", "nested/new-flow/inner"] {
        let refused = service.save_flow(save(name, &valid_flow(), &project));
        assert_eq!(refused.status_code, 409, "{name}: {}", refused.body);
    }
    assert!(!org.join("flows/nested/flow.md").exists());
    assert!(!flow_dir.join("inner").exists());
}

#[test]
fn editor_save_commits_only_the_files_that_changed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let org = init_org(&project);
    let service = AttractorApiService::new(settings(temp.path()));
    assert_eq!(
        service
            .save_flow(save("work", &valid_flow(), &project))
            .status_code,
        200
    );
    let before = git(&org, &["rev-parse", "HEAD"]);
    // An unrelated uncommitted change in the org stays out of the commit.
    fs::write(org.join("notes.md"), "draft\n").expect("notes");

    let edited = with_prompt("Do better work");
    assert_eq!(
        service
            .save_flow(save("work", &edited, &project))
            .status_code,
        200
    );
    assert_eq!(
        last_commit(&org),
        "Edit work in the flow editor\nflows/work/task.md"
    );
    assert_eq!(git(&org, &["status", "--porcelain"]), "?? notes.md");

    // Saving unchanged content commits nothing.
    let head = git(&org, &["rev-parse", "HEAD"]);
    assert_ne!(head, before);
    assert_eq!(
        service
            .save_flow(save("work", &edited, &project))
            .status_code,
        200
    );
    assert_eq!(git(&org, &["rev-parse", "HEAD"]), head);

    // Removing a node deletes and commits its file.
    let without_task = flow(json!({
        "schema_version": "1",
        "id": "workflow",
        "title": "Workflow",
        "goal": "Do work",
        "nodes": {
            "start": {"kind": "start", "label": "Start"},
            "done": {"kind": "exit", "label": "Done"}
        },
        "edges": [{"from": "start", "to": "done"}]
    }));
    assert_eq!(
        service
            .save_flow(save("work", &without_task, &project))
            .status_code,
        200
    );
    assert_eq!(
        last_commit(&org),
        "Edit work in the flow editor\nflows/work/start.md\nflows/work/task.md"
    );
    assert!(!org.join("flows/work/task.md").exists());
}

#[test]
fn editor_save_leaves_other_files_in_the_flow_directory_alone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let org = init_org(&project);
    let service = AttractorApiService::new(settings(temp.path()));
    assert_eq!(
        service
            .save_flow(save("work", &valid_flow(), &project))
            .status_code,
        200
    );
    let head = git(&org, &["rev-parse", "HEAD"]);
    fs::write(org.join("flows/work/unrelated.txt"), "draft\n").expect("unstaged");
    fs::write(org.join("flows/work/staged.txt"), "draft\n").expect("staged");
    git(&org, &["add", "flows/work/staged.txt"]);

    // An unchanged save commits nothing.
    assert_eq!(
        service
            .save_flow(save("work", &valid_flow(), &project))
            .status_code,
        200
    );
    assert_eq!(git(&org, &["rev-parse", "HEAD"]), head);

    // A changed save commits only the flow's own files.
    let edited = with_prompt("Do better work");
    assert_eq!(
        service
            .save_flow(save("work", &edited, &project))
            .status_code,
        200
    );
    assert_eq!(
        last_commit(&org),
        "Edit work in the flow editor\nflows/work/task.md"
    );
    assert_eq!(
        git(&org, &["status", "--porcelain"]),
        "A  flows/work/staged.txt\n?? flows/work/unrelated.txt"
    );
}

#[test]
fn a_failed_editor_commit_is_reported_and_recorded_by_the_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let org = init_org(&project);
    let service = AttractorApiService::new(settings(temp.path()));
    fs::write(org.join("notes.md"), "draft\n").expect("notes");
    let hook = reject_commits(&org);

    let failed = service.save_flow(save("work", &valid_flow(), &project));
    assert_eq!(failed.status_code, 500, "{}", failed.body);
    // Nothing is left staged.
    assert_eq!(git(&org, &["diff", "--cached", "--name-only"]), "");

    // Retrying the identical save records the files the failure left behind.
    fs::remove_file(&hook).expect("unhook");
    let retried = service.save_flow(save("work", &valid_flow(), &project));
    assert_eq!(retried.status_code, 200, "{}", retried.body);
    assert_eq!(
        last_commit(&org),
        "Edit work in the flow editor\nflows/work/done.md\nflows/work/flow.md\nflows/work/start.md\nflows/work/task.md"
    );
    assert_eq!(git(&org, &["status", "--porcelain"]), "?? notes.md");
}

#[test]
fn a_failed_editor_delete_keeps_the_copy_for_a_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let org = init_org(&project);
    write_flow_dir(&settings.bundled_dir().join("flows/shared"), &valid_flow())
        .expect("write flow");
    write_flow_dir(
        &settings.bundled_dir().join("flows/team/shared"),
        &valid_flow(),
    )
    .expect("write flow");
    let service = AttractorApiService::new(settings);
    let project_path = project.to_str();
    let edited = with_prompt("Project work");
    let local = with_prompt("Local work");
    for name in ["work", "shared", "team/shared"] {
        let saved = service.save_flow(save(name, &edited, &project));
        assert_eq!(saved.status_code, 200, "{}", saved.body);
        // Local, uncommitted edits to the copy survive a failed delete.
        write_flow_dir(&org.join(format!("flows/{name}")), &local).expect("write flow");
    }
    fs::write(org.join("notes.md"), "draft\n").expect("notes");
    let hook = reject_commits(&org);

    for name in ["work", "shared", "team/shared"] {
        let failed = service.delete_flow(name, project_path);
        assert_eq!(failed.status_code, 500, "{}", failed.body);
        let copy = org.join(format!("flows/{name}"));
        assert_eq!(read_flow_dir(&copy).expect("copy"), local);
        assert_eq!(service.get_flow(name, project_path).body["layer"], "org");
    }
    assert_eq!(
        git(&org, &["status", "--porcelain"]),
        "M flows/shared/task.md\n M flows/team/shared/task.md\n M flows/work/task.md\n?? notes.md"
    );

    fs::remove_file(&hook).expect("unhook");
    for name in ["work", "shared", "team/shared"] {
        let retried = service.delete_flow(name, project_path);
        assert_eq!(retried.status_code, 200, "{}", retried.body);
        assert!(!org.join(format!("flows/{name}")).exists());
        assert_eq!(
            last_commit(&org),
            format!("Delete {name} in the flow editor\nflows/{name}/done.md\nflows/{name}/flow.md\nflows/{name}/start.md\nflows/{name}/task.md")
        );
    }
    for name in ["shared", "team/shared"] {
        assert_eq!(
            service.get_flow(name, project_path).body["layer"],
            "bundled"
        );
    }
    assert_eq!(service.get_flow("work", project_path).status_code, 404);
    assert_eq!(git(&org, &["status", "--porcelain"]), "?? notes.md");
}

#[test]
fn saving_a_bundled_flow_creates_a_project_copy_and_deleting_it_reverts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let org = init_org(&project);
    let bundled = settings.bundled_dir().join("flows/shared");
    write_flow_dir(&bundled, &valid_flow()).expect("write flow");
    let service = AttractorApiService::new(settings);
    let project_path = project.to_str();

    let edited = with_prompt("Project work");
    let saved = service.save_flow(save("shared", &edited, &project));
    assert_eq!(saved.status_code, 200, "{}", saved.body);
    // The bundled library is untouched; the org holds the whole copy.
    assert_eq!(read_flow_dir(&bundled).expect("bundled"), valid_flow());
    assert_eq!(
        read_flow_dir(&org.join("flows/shared")).expect("copy"),
        edited
    );
    // The unmodified copy is committed first, then the edit.
    assert_eq!(
        git(&org, &["log", "--format=%s"]),
        "Edit shared in the flow editor\nCopy shared from bundled"
    );
    assert_eq!(
        last_commit(&org),
        "Edit shared in the flow editor\nflows/shared/task.md"
    );
    for file in ["done.md", "flow.md", "start.md", "task.md"] {
        assert_eq!(
            git(&org, &["show", &format!("HEAD~1:flows/shared/{file}")]),
            fs::read_to_string(bundled.join(file))
                .expect("bundled file")
                .trim()
        );
    }
    assert_eq!(
        service.get_flow("shared", project_path).body["layer"],
        "org"
    );

    // Deleting the project copy reverts to the bundled flow.
    let deleted = service.delete_flow("shared", project_path);
    assert_eq!(deleted.status_code, 200);
    assert_eq!(deleted.body, json!({"status": "deleted"}));
    assert!(!org.join("flows/shared").exists());
    assert!(last_commit(&org).starts_with("Delete shared in the flow editor\n"));
    assert_eq!(git(&org, &["status", "--porcelain"]), "");
    let reverted = service.get_flow("shared", project_path);
    assert_eq!(reverted.body["layer"], "bundled");

    // An uncommitted copy deletes too, with nothing to commit.
    let head = git(&org, &["rev-parse", "HEAD"]);
    write_flow_dir(&org.join("flows/shared"), &edited).expect("write flow");
    assert_eq!(service.delete_flow("shared", project_path).status_code, 200);
    assert_eq!(git(&org, &["rev-parse", "HEAD"]), head);

    // Deleting a bundled flow is refused.
    let refused = service.delete_flow("shared", project_path);
    assert_eq!(refused.status_code, 409);
    assert!(bundled.join("flow.md").is_file());
    assert_eq!(service.delete_flow("shared", None).status_code, 409);
}

#[test]
fn a_failed_copy_commit_is_recopied_by_the_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let org = init_org(&project);
    let bundled = settings.bundled_dir().join("flows/shared");
    write_flow_dir(&bundled, &valid_flow()).expect("write flow");
    let before = fs::read_to_string(bundled.join("task.md")).expect("bundled");
    let service = AttractorApiService::new(settings);
    let edited = with_prompt("Project work");
    let hook = reject_commits(&org);

    let failed = service.save_flow(save("shared", &edited, &project));
    assert_eq!(failed.status_code, 500, "{}", failed.body);
    // The failed copy left its files behind, and a stray one appears.
    fs::write(org.join("flows/shared/stray.md"), "stray\n").expect("stray");

    fs::remove_file(&hook).expect("unhook");
    let retried = service.save_flow(save("shared", &edited, &project));
    assert_eq!(retried.status_code, 200, "{}", retried.body);
    assert_eq!(
        git(&org, &["log", "--format=%s"]),
        "Edit shared in the flow editor\nCopy shared from bundled"
    );
    assert_eq!(
        git(&org, &["show", "--name-only", "--format=", "HEAD~1"]),
        "flows/shared/done.md\nflows/shared/flow.md\nflows/shared/start.md\nflows/shared/task.md"
    );
    assert_eq!(
        git(&org, &["show", "HEAD~1:flows/shared/task.md"]),
        before.trim()
    );
    assert_eq!(
        read_flow_dir(&org.join("flows/shared")).expect("copy"),
        edited
    );
    assert_eq!(
        fs::read_to_string(bundled.join("task.md")).expect("bundled"),
        before
    );
}

#[test]
fn a_failed_save_keeps_an_uncommitted_project_override() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let org = init_org(&project);
    let bundled = settings.bundled_dir().join("flows/shared");
    write_flow_dir(&bundled, &valid_flow()).expect("write flow");
    let service = AttractorApiService::new(settings);
    let copy = org.join("flows/shared");
    let override_flow = with_prompt("Override work");
    write_flow_dir(&copy, &override_flow).expect("write flow");
    let hook = reject_commits(&org);

    // The override is the project's own, not a bundled copy to recopy.
    let edited = with_prompt("Project work");
    let failed = service.save_flow(save("shared", &edited, &project));
    assert_eq!(failed.status_code, 500, "{}", failed.body);
    assert_eq!(read_flow_dir(&copy).expect("override"), edited);

    fs::remove_file(&hook).expect("unhook");
    let untouched = service.save_flow(save("shared", &override_flow, &project));
    assert_eq!(untouched.status_code, 200, "{}", untouched.body);
    assert_eq!(
        git(&org, &["log", "--format=%s"]),
        "Edit shared in the flow editor"
    );
    assert_eq!(read_flow_dir(&copy).expect("override"), override_flow);
}

fn save(name: &str, flow: &FlowDefinition, project: &Path) -> SaveFlowRequest {
    SaveFlowRequest {
        name: name.to_string(),
        flow: Some(flow.clone()),
        files: None,
        project_path: Some(project.to_string_lossy().to_string()),
    }
}

/// A file-view save of `files` (name, exact text).
fn save_files(name: &str, files: &[(&str, &str)], project: &Path) -> SaveFlowRequest {
    SaveFlowRequest {
        name: name.to_string(),
        flow: None,
        files: Some(
            files
                .iter()
                .map(|(name, content)| FlowFileText {
                    name: name.to_string(),
                    content: content.to_string(),
                })
                .collect(),
        ),
        project_path: Some(project.to_string_lossy().to_string()),
    }
}

/// `text` (a flow file) with a frontmatter comment that rendering never
/// produces, so a test can tell a written file from a re-rendered one.
fn annotated(text: &str) -> String {
    let rest = text.strip_prefix("---\n").expect("frontmatter");
    format!("---\n# written by hand\n{rest}")
}

/// Every file of a flow directory, by name, with its exact text.
fn flow_dir_bytes(dir: &Path) -> std::collections::BTreeMap<String, String> {
    fs::read_dir(dir)
        .expect("flow dir")
        .map(|entry| {
            let path = entry.expect("entry").path();
            (
                path.file_name()
                    .expect("name")
                    .to_string_lossy()
                    .to_string(),
                fs::read_to_string(&path).expect("file"),
            )
        })
        .collect()
}

/// The project's org repo with a local git identity, as saves commit under
/// the repo's normal git config.
fn init_org(project: &Path) -> std::path::PathBuf {
    let org = spark_common::project::ensure_org_repo(project).expect("org repo");
    git(&org, &["config", "user.name", "Editor Test"]);
    git(&org, &["config", "user.email", "editor@example.com"]);
    org
}

/// Installs a pre-commit hook that rejects every commit in the org repo.
fn reject_commits(org: &Path) -> std::path::PathBuf {
    let hook = org.join(".git/hooks/pre-commit");
    fs::create_dir_all(hook.parent().expect("hooks")).expect("hooks dir");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    hook
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The last commit's subject and changed files.
fn last_commit(org: &Path) -> String {
    git(org, &["show", "--name-only", "--format=%s", "HEAD"])
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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
