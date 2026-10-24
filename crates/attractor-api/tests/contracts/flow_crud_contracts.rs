use std::fs;
use std::path::Path;
use std::process::Command;

use attractor_api::{preview_named_flow_source, AttractorApiService, SaveFlowRequest};
use attractor_dsl::{parse_flow_definition, read_flow_dir, write_flow_dir, FlowRoots};
use serde_json::json;
use spark_common::settings::SparkSettings;

const VALID_FLOW: &str = r#"schema_version: "1"
id: workflow
title: Workflow
goal: Do work
nodes:
  start:
    kind: start
    label: Start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Do work
  done:
    kind: exit
    label: Done
edges:
  - from: start
    to: task
  - from: task
    to: done
"#;

#[test]
fn list_and_get_layer_the_project_org_over_bundled_flows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let bundled = settings.bundled_dir().join("flows");
    write_flow(&bundled.join("examples/simple-linear"), VALID_FLOW);
    write_flow(&bundled.join("root"), VALID_FLOW);
    // Retired YAML files are not flows.
    fs::write(bundled.join("legacy.yaml"), VALID_FLOW).expect("write yaml");
    let org = spark_common::project::org_dir(&project).join("flows");
    write_flow(
        &org.join("root"),
        &VALID_FLOW.replace("title: Workflow", "title: Project Root"),
    );
    write_flow(&org.join("project-only"), VALID_FLOW);
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
    assert!(root.body["content"]
        .as_str()
        .expect("content")
        .contains("Project Root"));
    let get = service.get_flow("examples/simple-linear", project_path);
    assert_eq!(get.body["name"], json!("examples/simple-linear"));
    assert_eq!(get.body["layer"], "bundled");
    assert_eq!(
        parse_flow_definition(get.body["content"].as_str().expect("content")).expect("flow"),
        parse_flow_definition(VALID_FLOW).expect("flow")
    );

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
    write_flow(
        &settings.bundled_dir().join("flows/nested/helper"),
        VALID_FLOW,
    );
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
    assert_eq!(
        parse_flow_definition(&source.content).expect("flow"),
        source.flow
    );

    let preview = preview_named_flow_source(&roots, &source.name, &source.content);
    assert_eq!(preview.status_code, 200);
    assert_eq!(preview.body["status"], "ok");
    assert!(preview.body["graph"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .any(|node| node["id"] == json!("start")));
}

#[test]
fn save_parse_error_reports_flow_definition_diagnostics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.save_flow(save("bad", "nodes: [\n", &temp.path().join("project")));

    assert_eq!(response.status_code, 422);
    assert_eq!(response.body["detail"]["status"], json!("validation_error"));
    assert_eq!(
        response.body["detail"]["errors"][0]["rule_id"],
        json!("parse_error")
    );
}

#[test]
fn save_validation_error_reports_diagnostics_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.save_flow(save(
        "broken",
        r#"schema_version: "1"
id: broken
nodes:
  start:
    kind: start
  done:
    kind: exit
edges:
  - from: start
    to: missing
"#,
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
        content: VALID_FLOW.to_string(),
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

    let created = service.save_flow(save("nested/new-flow", VALID_FLOW, &project));
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
        let refused = service.save_flow(save(name, VALID_FLOW, &project));
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
            .save_flow(save("work", VALID_FLOW, &project))
            .status_code,
        200
    );
    let before = git(&org, &["rev-parse", "HEAD"]);
    // An unrelated uncommitted change in the org stays out of the commit.
    fs::write(org.join("notes.md"), "draft\n").expect("notes");

    let edited = VALID_FLOW.replace("prompt: Do work", "prompt: Do better work");
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
    let without_task = r#"schema_version: "1"
id: workflow
title: Workflow
goal: Do work
nodes:
  start:
    kind: start
    label: Start
  done:
    kind: exit
    label: Done
edges:
  - from: start
    to: done
"#;
    assert_eq!(
        service
            .save_flow(save("work", without_task, &project))
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
            .save_flow(save("work", VALID_FLOW, &project))
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
            .save_flow(save("work", VALID_FLOW, &project))
            .status_code,
        200
    );
    assert_eq!(git(&org, &["rev-parse", "HEAD"]), head);

    // A changed save commits only the flow's own files.
    let edited = VALID_FLOW.replace("prompt: Do work", "prompt: Do better work");
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

    let failed = service.save_flow(save("work", VALID_FLOW, &project));
    assert_eq!(failed.status_code, 500, "{}", failed.body);
    // Nothing is left staged.
    assert_eq!(git(&org, &["diff", "--cached", "--name-only"]), "");

    // Retrying the identical save records the files the failure left behind.
    fs::remove_file(&hook).expect("unhook");
    let retried = service.save_flow(save("work", VALID_FLOW, &project));
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
    write_flow(&settings.bundled_dir().join("flows/shared"), VALID_FLOW);
    write_flow(
        &settings.bundled_dir().join("flows/team/shared"),
        VALID_FLOW,
    );
    let service = AttractorApiService::new(settings);
    let project_path = project.to_str();
    let edited = VALID_FLOW.replace("prompt: Do work", "prompt: Project work");
    let local = VALID_FLOW.replace("prompt: Do work", "prompt: Local work");
    for name in ["work", "shared", "team/shared"] {
        let saved = service.save_flow(save(name, &edited, &project));
        assert_eq!(saved.status_code, 200, "{}", saved.body);
        // Local, uncommitted edits to the copy survive a failed delete.
        write_flow(&org.join(format!("flows/{name}")), &local);
    }
    fs::write(org.join("notes.md"), "draft\n").expect("notes");
    let hook = reject_commits(&org);

    for name in ["work", "shared", "team/shared"] {
        let failed = service.delete_flow(name, project_path);
        assert_eq!(failed.status_code, 500, "{}", failed.body);
        let copy = org.join(format!("flows/{name}"));
        assert_eq!(
            read_flow_dir(&copy).expect("copy"),
            parse_flow_definition(&local).expect("flow")
        );
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
    write_flow(&bundled, VALID_FLOW);
    let service = AttractorApiService::new(settings);
    let project_path = project.to_str();

    let edited = VALID_FLOW.replace("prompt: Do work", "prompt: Project work");
    let saved = service.save_flow(save("shared", &edited, &project));
    assert_eq!(saved.status_code, 200, "{}", saved.body);
    // The bundled library is untouched; the org holds the whole copy.
    assert_eq!(
        read_flow_dir(&bundled).expect("bundled"),
        parse_flow_definition(VALID_FLOW).expect("flow")
    );
    assert_eq!(
        read_flow_dir(&org.join("flows/shared")).expect("copy"),
        parse_flow_definition(&edited).expect("flow")
    );
    assert_eq!(
        last_commit(&org),
        "Edit shared in the flow editor\nflows/shared/done.md\nflows/shared/flow.md\nflows/shared/start.md\nflows/shared/task.md"
    );
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
    write_flow(&org.join("flows/shared"), &edited);
    assert_eq!(service.delete_flow("shared", project_path).status_code, 200);
    assert_eq!(git(&org, &["rev-parse", "HEAD"]), head);

    // Deleting a bundled flow is refused.
    let refused = service.delete_flow("shared", project_path);
    assert_eq!(refused.status_code, 409);
    assert!(bundled.join("flow.md").is_file());
    assert_eq!(service.delete_flow("shared", None).status_code, 409);
}

fn save(name: &str, content: &str, project: &Path) -> SaveFlowRequest {
    SaveFlowRequest {
        name: name.to_string(),
        content: content.to_string(),
        project_path: Some(project.to_string_lossy().to_string()),
    }
}

fn write_flow(dir: &Path, content: &str) {
    write_flow_dir(dir, &parse_flow_definition(content).expect("valid flow")).expect("write");
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
