use std::fs;
use std::path::Path;

use attractor_api::{AttractorApiService, PipelineStartRequest};
use serde_json::json;
use spark_common::settings::SparkSettings;

fn settings(root: &Path) -> SparkSettings {
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

fn git_in(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

const PARENT_FLOW: &str = r#"schema_version: "1"
id: lifecycle_parent
title: Lifecycle Parent
nodes:
  start:
    kind: start
  prepare_workspace:
    kind: tool
    config:
      kind: tool
      command: |
        set -eu
        repo_root=$(git rev-parse --show-toplevel)
        branch="spark/lifecycle/${SPARK_RUN_ID}"
        checkout_root="${repo_root}/.spark/checkouts"
        worktree="${checkout_root}/${SPARK_RUN_ID}"
        mkdir -p "$checkout_root"
        printf '*\n' > "${repo_root}/.spark/.gitignore"
        git worktree add -b "$branch" "$worktree" HEAD >/dev/null
        printf '{"path":"%s","branch":"%s"}' "$worktree" "$branch"
      env_map:
        SPARK_RUN_ID: internal.run_id
      output_map:
        context.workspace.path: path
        context.workspace.branch: branch
    contracts:
      writes_context:
      - context.workspace.path
      - context.workspace.branch
  work:
    kind: subflow
    config:
      kind: subflow
      flow_ref: workers/child.yaml
    manager:
      child_workdir_from: context.workspace.path
  finalize_commit:
    kind: tool
    config:
      kind: tool
      command: |
        set -eu
        cd "$SPARK_WORKSPACE_PATH"
        git add --all
        git commit -m "lifecycle change" >/dev/null
        printf '{"commit":"%s"}' "$(git rev-parse HEAD)"
      env_map:
        SPARK_WORKSPACE_PATH: context.workspace.path
      output_map:
        context.workspace.commit: commit
    contracts:
      writes_context:
      - context.workspace.commit
  cleanup_workspace:
    kind: tool
    config:
      kind: tool
      command: |
        set -eu
        git worktree remove "$SPARK_WORKSPACE_PATH"
      env_map:
        SPARK_WORKSPACE_PATH: context.workspace.path
  done:
    kind: exit
edges:
- from: start
  to: prepare_workspace
- from: prepare_workspace
  to: work
  condition: outcome=success
- from: work
  to: finalize_commit
  condition: outcome=success
- from: finalize_commit
  to: cleanup_workspace
  condition: outcome=success
- from: cleanup_workspace
  to: done
  condition: outcome=success
"#;

const CHILD_FLOW: &str = r#"schema_version: "1"
id: lifecycle_child
title: Lifecycle Child
nodes:
  start:
    kind: start
  produce:
    kind: tool
    config:
      kind: tool
      command: printf 'produced by child' > produced.txt
  done:
    kind: exit
edges:
- from: start
  to: produce
- from: produce
  to: done
  condition: outcome=success
"#;

#[test]
fn visible_worktree_lifecycle_commits_child_work_on_branch_and_removes_worktree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let flow_dir = settings.flows_dir.join("lifecycle");
    fs::create_dir_all(flow_dir.join("workers")).expect("flow dirs");
    fs::write(flow_dir.join("parent.yaml"), PARENT_FLOW).expect("write parent flow");
    fs::write(flow_dir.join("workers/child.yaml"), CHILD_FLOW).expect("write child flow");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::write(repo.join("README.md"), "lifecycle\n").expect("seed repo");
    git_in(&repo, &["init"]);
    git_in(&repo, &["config", "user.email", "contracts@example.com"]);
    git_in(&repo, &["config", "user.name", "Contracts"]);
    git_in(&repo, &["add", "--all"]);
    git_in(&repo, &["commit", "-m", "initial"]);
    // A dirty file in the source checkout must never reach the branch.
    fs::write(repo.join("dirty.txt"), "uncommitted\n").expect("dirty file");

    let service = AttractorApiService::new(settings.clone());
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-lifecycle".to_string()),
        flow_name: Some("lifecycle/parent.yaml".to_string()),
        working_directory: repo.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    assert_eq!(
        response.body["terminal_status"],
        json!("completed"),
        "{:?}",
        response.body
    );

    let branch = "spark/lifecycle/run-lifecycle";
    let produced = git_in(&repo, &["show", &format!("{branch}:produced.txt")]);
    assert_eq!(produced, "produced by child");
    let branch_files = git_in(&repo, &["ls-tree", "--name-only", branch]);
    assert!(
        !branch_files.contains("dirty.txt"),
        "dirty source-checkout files must not reach the branch: {branch_files}"
    );
    let subject = git_in(&repo, &["log", "-1", "--format=%s", branch]);
    assert_eq!(subject, "lifecycle change");
    assert!(
        !branch_files.contains(".spark"),
        "the worktree directory must never be committed: {branch_files}"
    );

    let worktree = repo.join(".spark/checkouts/run-lifecycle");
    assert!(
        !worktree.exists(),
        "successful runs must remove their worktree"
    );
    assert!(
        !repo.join(".git/spark").exists(),
        "worktrees must not be created inside .git"
    );
    let status = git_in(&repo, &["status", "--porcelain"]);
    assert!(
        !status.contains(".spark"),
        "the .spark directory must be invisible to git status: {status}"
    );
    assert!(
        repo.join("dirty.txt").exists(),
        "the source checkout must be left untouched"
    );
    let source_head = git_in(&repo, &["rev-parse", "HEAD"]);
    let base_commit = git_in(&repo, &["rev-parse", &format!("{branch}~1")]);
    assert_eq!(
        source_head, base_commit,
        "the branch must start from the committed source state"
    );
}

const RUNS_ROOT_FLOW: &str = r#"schema_version: "1"
id: runs_root_probe
title: Runs Root Probe
nodes:
  start:
    kind: start
  probe:
    kind: tool
    config:
      kind: tool
      command: printf '%s' "$SPARK_RUNS_ROOT"
      env_map:
        SPARK_RUNS_ROOT: internal.runs_dir
  done:
    kind: exit
edges:
- from: start
  to: probe
- from: probe
  to: done
  condition: outcome=success
"#;

#[test]
fn tool_nodes_can_bind_the_runtime_runs_root_from_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");

    let service = AttractorApiService::new(settings.clone());
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-runs-root".to_string()),
        flow_content: Some(RUNS_ROOT_FLOW.to_string()),
        working_directory: project.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(response.body["terminal_status"], json!("completed"));

    let store = attractor_runtime::RunStore::for_settings(&settings);
    let bundle = store
        .read_run_bundle("run-runs-root")
        .expect("read bundle")
        .expect("bundle exists");
    let result = store
        .read_result(&bundle.paths)
        .expect("read result")
        .expect("result exists");
    let probed = std::path::PathBuf::from(result.body_markdown.trim());
    assert_eq!(
        probed, settings.runs_dir,
        "internal.runs_dir must resolve to the runtime's runs root"
    );
    assert!(
        bundle.paths.root.starts_with(&probed),
        "the probed root must actually contain this run"
    );
}
