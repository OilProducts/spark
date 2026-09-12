use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use attractor_api::{AttractorApiService, ExecutionLockSpec, PipelineStartRequest};
use attractor_core::RunExecutionLock;
use attractor_runtime::RunStore;
use serde_json::json;
use spark_common::settings::SparkSettings;

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
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

fn git_in(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=contracts@example.com",
            "-c",
            "user.name=Contracts",
        ])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git_repo(dir: &Path) {
    fs::create_dir_all(dir).expect("create repo dir");
    fs::write(dir.join("README.md"), "contracts\n").expect("seed repo");
    git_in(dir, &["init"]);
    git_in(dir, &["add", "--all"]);
    git_in(dir, &["commit", "-m", "initial"]);
}

fn tool_flow(id: &str, command: &str) -> String {
    format!(
        r#"schema_version: "1"
id: {id}
title: {id}
nodes:
  start:
    kind: start
  work:
    kind: tool
    config:
      kind: tool
      command: '{command}'
  done:
    kind: exit
edges:
  - from: start
    to: work
  - from: work
    to: done
"#
    )
}

fn wait_for_release_file_command(marker: &str) -> String {
    format!(
        "for i in $(seq 1 200); do if [ -f {marker} ]; then exit 0; fi; sleep 0.05; done; exit 1"
    )
}

fn project_lock(key: &str) -> ExecutionLockSpec {
    ExecutionLockSpec {
        scope: "project".to_string(),
        key: key.to_string(),
        conflict_policy: "queue".to_string(),
    }
}

fn start_locked_run(
    service: &AttractorApiService,
    run_id: &str,
    working_directory: &Path,
    flow: String,
    lock: Option<ExecutionLockSpec>,
) {
    let response = service.start_pipeline(PipelineStartRequest {
        run_id: Some(run_id.to_string()),
        flow_content: Some(flow),
        working_directory: working_directory.to_string_lossy().to_string(),
        execution_lock: lock,
        ..PipelineStartRequest::default()
    });
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    assert_eq!(
        response.body["status"],
        json!("started"),
        "{:?}",
        response.body
    );
}

fn read_lock(settings: &SparkSettings, run_id: &str) -> Option<RunExecutionLock> {
    RunStore::for_settings(settings)
        .read_run_bundle(run_id)
        .expect("read run bundle")
        .and_then(|bundle| bundle.record)
        .and_then(|record| record.execution_lock)
}

fn read_status(settings: &SparkSettings, run_id: &str) -> String {
    RunStore::for_settings(settings)
        .read_run_bundle(run_id)
        .expect("read run bundle")
        .and_then(|bundle| bundle.record)
        .map(|record| record.status)
        .unwrap_or_default()
}

fn wait_until(deadline_message: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for: {deadline_message}");
}

#[test]
fn project_scope_lock_queues_conflicting_runs_and_promotes_fifo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project = temp.path().join("repo");
    init_git_repo(&project);
    let service = AttractorApiService::new(settings.clone());

    start_locked_run(
        &service,
        "run-lock-a",
        &project,
        tool_flow("holder", &wait_for_release_file_command("release-a")),
        Some(project_lock("integration")),
    );
    wait_until("first run holds the lock", || {
        read_lock(&settings, "run-lock-a")
            .map(|lock| lock.state == "holding")
            .unwrap_or(false)
    });

    start_locked_run(
        &service,
        "run-lock-b",
        &project,
        tool_flow("waiter", "printf done"),
        Some(project_lock("integration")),
    );
    wait_until("second run queues behind the first", || {
        read_lock(&settings, "run-lock-b")
            .map(|lock| lock.state == "queued" && lock.queue_position == Some(1))
            .unwrap_or(false)
    });
    assert_eq!(read_status(&settings, "run-lock-b"), "queued");

    let lock_a = read_lock(&settings, "run-lock-a").expect("lock a");
    let lock_b = read_lock(&settings, "run-lock-b").expect("lock b");
    assert_eq!(lock_a.identity, lock_b.identity);
    assert!(!lock_a.identity.is_empty());

    fs::write(project.join("release-a"), "go").expect("release first run");
    wait_until("both runs complete and release their locks", || {
        read_status(&settings, "run-lock-a") == "completed"
            && read_status(&settings, "run-lock-b") == "completed"
            && read_lock(&settings, "run-lock-a").is_some_and(|lock| lock.state == "released")
            && read_lock(&settings, "run-lock-b").is_some_and(|lock| lock.state == "released")
    });
    assert_eq!(
        read_lock(&settings, "run-lock-a").expect("lock a").state,
        "released"
    );
    assert_eq!(
        read_lock(&settings, "run-lock-b").expect("lock b").state,
        "released"
    );
}

#[test]
fn linked_worktrees_of_one_repository_share_lock_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project = temp.path().join("repo");
    init_git_repo(&project);
    let worktree = temp.path().join("repo-worktrees/run-1");
    fs::create_dir_all(worktree.parent().expect("worktree parent")).expect("worktree root");
    git_in(
        &project,
        &[
            "worktree",
            "add",
            "-b",
            "spark/run-1",
            worktree.to_str().expect("worktree utf8"),
        ],
    );
    let service = AttractorApiService::new(settings.clone());

    start_locked_run(
        &service,
        "run-wt-a",
        &project,
        tool_flow("holder", &wait_for_release_file_command("release-wt")),
        Some(project_lock("integration")),
    );
    wait_until("checkout run holds the lock", || {
        read_lock(&settings, "run-wt-a")
            .map(|lock| lock.state == "holding")
            .unwrap_or(false)
    });

    start_locked_run(
        &service,
        "run-wt-b",
        &worktree,
        tool_flow("waiter", "printf done"),
        Some(project_lock("integration")),
    );
    wait_until("worktree run queues behind checkout run", || {
        read_lock(&settings, "run-wt-b")
            .map(|lock| lock.state == "queued")
            .unwrap_or(false)
    });
    assert_eq!(
        read_lock(&settings, "run-wt-a").expect("lock a").identity,
        read_lock(&settings, "run-wt-b").expect("lock b").identity,
        "linked worktrees must resolve to one repository lock identity"
    );

    fs::write(project.join("release-wt"), "go").expect("release holder");
    wait_until("both runs complete after release", || {
        read_status(&settings, "run-wt-a") == "completed"
            && read_status(&settings, "run-wt-b") == "completed"
    });
}

#[test]
fn different_lock_keys_do_not_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project = temp.path().join("repo");
    init_git_repo(&project);
    let service = AttractorApiService::new(settings.clone());

    start_locked_run(
        &service,
        "run-key-a",
        &project,
        tool_flow("holder", &wait_for_release_file_command("release-key")),
        Some(project_lock("integration")),
    );
    wait_until("holder acquires its lock", || {
        read_lock(&settings, "run-key-a")
            .map(|lock| lock.state == "holding")
            .unwrap_or(false)
    });

    start_locked_run(
        &service,
        "run-key-b",
        &project,
        tool_flow("independent", "printf done"),
        Some(project_lock("other-key")),
    );
    wait_until("independent key completes while holder still runs", || {
        read_status(&settings, "run-key-b") == "completed"
    });
    assert_eq!(
        read_lock(&settings, "run-key-a").expect("lock a").state,
        "holding",
        "the holder must still be running on its own key"
    );

    fs::write(project.join("release-key"), "go").expect("release holder");
    wait_until("holder completes", || {
        read_status(&settings, "run-key-a") == "completed"
    });
}

#[test]
fn all_math_catalog_flows_serialize_per_project_while_other_projects_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project_a = temp.path().join("math-a");
    let project_b = temp.path().join("math-b");
    init_git_repo(&project_a);
    init_git_repo(&project_b);
    let service = AttractorApiService::new(settings.clone());
    let math_lock = || Some(project_lock("math-research"));

    start_locked_run(
        &service,
        "math-explore-a",
        &project_a,
        tool_flow(
            "math-research-explore-conjecture",
            &wait_for_release_file_command("release-math"),
        ),
        math_lock(),
    );
    wait_until("explore holds project A math lock", || {
        read_lock(&settings, "math-explore-a").is_some_and(|lock| lock.state == "holding")
    });
    start_locked_run(
        &service,
        "math-formalize-a",
        &project_a,
        tool_flow("math-research-formalize-result", "printf formalized"),
        math_lock(),
    );
    start_locked_run(
        &service,
        "math-prove-a",
        &project_a,
        tool_flow("math-research-prove-refute", "printf proved"),
        math_lock(),
    );
    wait_until("the other two math flows queue in project A", || {
        ["math-formalize-a", "math-prove-a"]
            .iter()
            .all(|run_id| read_lock(&settings, run_id).is_some_and(|lock| lock.state == "queued"))
    });

    start_locked_run(
        &service,
        "math-prove-b",
        &project_b,
        tool_flow(
            "math-research-prove-refute-other-project",
            "printf independent",
        ),
        math_lock(),
    );
    wait_until("project B runs while project A remains locked", || {
        read_status(&settings, "math-prove-b") == "completed"
    });
    assert_eq!(
        read_lock(&settings, "math-explore-a")
            .expect("holder")
            .state,
        "holding"
    );

    fs::write(project_a.join("release-math"), "go").expect("release math holder");
    wait_until("all project A math flows complete serially", || {
        ["math-explore-a", "math-formalize-a", "math-prove-a"]
            .iter()
            .all(|run_id| read_status(&settings, run_id) == "completed")
    });
}

#[test]
fn unsupported_lock_scope_is_a_validation_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let service = AttractorApiService::new(settings.clone());

    let response = service.start_pipeline(PipelineStartRequest {
        run_id: Some("run-bad-scope".to_string()),
        flow_content: Some(tool_flow("bad", "printf done")),
        working_directory: temp.path().join("repo").to_string_lossy().to_string(),
        execution_lock: Some(ExecutionLockSpec {
            scope: "global".to_string(),
            key: "integration".to_string(),
            conflict_policy: "queue".to_string(),
        }),
        ..PipelineStartRequest::default()
    });
    assert_eq!(response.body["status"], json!("validation_error"));
    assert!(
        response.body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("scope"),
        "{:?}",
        response.body
    );
}

#[test]
fn child_inherits_lock_without_releasing_parent_and_direct_retry_queues() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    let project = temp.path().join("repo");
    init_git_repo(&project);
    let child_path = project.join("child.yaml");
    fs::write(
        &child_path,
        tool_flow(
            "child",
            &format!(
                "({}); test -f retry-child",
                wait_for_release_file_command("release-child")
            ),
        ),
    )
    .unwrap();
    let parent_flow = format!(
        r#"schema_version: '1'
id: parent
nodes:
  start: {{ kind: start }}
  child:
    kind: subflow
    runtime: {{ error_policy: continue }}
    config: {{ kind: subflow, flow_ref: child.yaml }}
  wait:
    kind: tool
    config: {{ kind: tool, command: '{}' }}
  done: {{ kind: exit }}
edges:
- {{ from: start, to: child }}
- {{ from: child, to: wait, condition: outcome=fail }}
- {{ from: wait, to: done }}
"#,
        wait_for_release_file_command("release-parent")
    );
    let service = AttractorApiService::new(settings.clone());
    let store = RunStore::for_settings(&settings);
    start_locked_run(
        &service,
        "parent",
        &project,
        parent_flow,
        Some(project_lock("integration")),
    );
    let mut child_id = String::new();
    wait_until("child inherits the parent lock", || {
        let children = store.list_child_run_bundles("parent").unwrap();
        let Some(child) = children.first().and_then(|b| b.record.as_ref()) else {
            return false;
        };
        child_id = child.run_id.clone();
        child
            .execution_lock
            .as_ref()
            .is_some_and(|lock| lock.state == "inherited" && lock.queue_position.is_none())
    });
    let identity = read_lock(&settings, "parent").unwrap().identity;
    assert_eq!(read_lock(&settings, &child_id).unwrap().identity, identity);
    start_locked_run(
        &service,
        "competitor",
        &project,
        tool_flow(
            "competitor",
            &wait_for_release_file_command("release-competitor"),
        ),
        Some(project_lock("integration")),
    );
    wait_until("competitor queues", || {
        read_status(&settings, "competitor") == "queued"
    });
    fs::write(project.join("release-child"), "go").unwrap();
    wait_until("child fails", || {
        read_status(&settings, &child_id) == "failed"
    });
    assert_eq!(read_lock(&settings, &child_id).unwrap().state, "released");
    assert_eq!(read_lock(&settings, "parent").unwrap().state, "holding");
    assert_eq!(read_status(&settings, "competitor"), "queued");
    fs::write(project.join("release-parent"), "go").unwrap();
    wait_until("parent releases and competitor takes lock", || {
        read_lock(&settings, "parent").is_some_and(|l| l.state == "released")
            && read_lock(&settings, "competitor").is_some_and(|l| l.state == "holding")
    });
    fs::write(project.join("retry-child"), "go").unwrap();
    wait_until("parent executor exits and child retry is accepted", || {
        let response = service.retry_pipeline_route(&child_id);
        if response.status_code == 200 {
            return true;
        }
        assert_eq!(
            response.body["detail"], "Retry conflicts with an active executor in the run tree",
            "{:?}",
            response.body
        );
        false
    });
    wait_until("independent child retry queues for the same lock", || {
        read_lock(&settings, &child_id)
            .is_some_and(|l| l.state == "queued" && l.identity == identity)
    });
    fs::write(project.join("release-competitor"), "go").unwrap();
    wait_until("retry completes and releases", || {
        read_status(&settings, &child_id) == "completed"
            && read_lock(&settings, &child_id).is_some_and(|l| l.state == "released")
    });
}
