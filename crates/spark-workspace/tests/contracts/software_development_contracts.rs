use spark_workspace::software_development::{
    cleanup_read_only, cleanup_success, finish_merge_change, merge_change, normalize_task,
    prepare_isolated_workspace, prepare_merge_change, prepare_read_only_workspace, record_result,
    repository_identity, RepositoryLock, RunResult, SoftwareDevelopmentError, TaskInput,
};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repository() -> TempDir {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    git(
        temp.path(),
        &["config", "user.email", "spark@example.invalid"],
    );
    git(temp.path(), &["config", "user.name", "Spark Tests"]);
    fs::write(temp.path().join("README.md"), "initial\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "initial"]);
    temp
}

#[test]
fn inline_and_artifact_inputs_normalize_equivalently() {
    let repo = repository();
    fs::write(repo.path().join("task.md"), "  ship the feature\n").unwrap();
    let inline = normalize_task(
        repo.path(),
        TaskInput {
            objective: Some(" ship the feature ".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let artifact = normalize_task(
        repo.path(),
        TaskInput {
            artifact_path: Some("task.md".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(inline.objective, artifact.objective);
    assert_eq!(inline.base_ref, artifact.base_ref);
    assert!(normalize_task(repo.path(), TaskInput::default()).is_err());
    assert!(normalize_task(
        repo.path(),
        TaskInput {
            objective: Some("one".into()),
            artifact_path: Some("task.md".into()),
            ..Default::default()
        }
    )
    .is_err());
    assert!(normalize_task(
        repo.path(),
        TaskInput {
            artifact_path: Some("../task.md".into()),
            ..Default::default()
        }
    )
    .is_err());
}

#[test]
fn artifact_inputs_are_normalized_from_the_selected_committed_workspace() {
    let repo = repository();
    fs::write(repo.path().join("task.md"), "committed objective\n").unwrap();
    git(repo.path(), &["add", "task.md"]);
    git(repo.path(), &["commit", "-qm", "task"]);
    fs::write(repo.path().join("task.md"), "dirty objective\n").unwrap();

    let workspace =
        prepare_isolated_workspace(repo.path(), "implement-change", "artifact-run", None).unwrap();
    let task = normalize_task(
        &workspace.worktree,
        TaskInput {
            artifact_path: Some("task.md".into()),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(task.objective, "committed objective");
    assert_eq!(
        fs::read_to_string(repo.path().join("task.md")).unwrap(),
        "dirty objective\n"
    );
}

#[test]
fn isolated_runs_use_committed_state_and_preserve_dirty_source() {
    let repo = repository();
    fs::write(repo.path().join("README.md"), "developer changes\n").unwrap();
    fs::write(repo.path().join("untracked.txt"), "private\n").unwrap();
    let first =
        prepare_isolated_workspace(repo.path(), "implement-change", "run-one", None).unwrap();
    let second =
        prepare_isolated_workspace(repo.path(), "implement-change", "run-two", None).unwrap();
    assert_ne!(first.branch, second.branch);
    assert_ne!(first.worktree, second.worktree);
    assert_eq!(
        fs::read_to_string(first.worktree.join("README.md")).unwrap(),
        "initial\n"
    );
    assert!(!first.worktree.join("untracked.txt").exists());
    assert_eq!(
        fs::read_to_string(repo.path().join("README.md")).unwrap(),
        "developer changes\n"
    );
    assert!(first.source_dirty);
    cleanup_success(repo.path(), &first).unwrap();
    assert!(!first.worktree.exists());
    assert!(second.worktree.exists());
}

#[cfg(unix)]
#[test]
fn read_only_workspace_never_changes_external_symlink_target_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let repo = repository();
    let external = tempfile::NamedTempFile::new().unwrap();
    symlink(external.path(), repo.path().join("external-link")).unwrap();
    git(repo.path(), &["add", "external-link"]);
    git(repo.path(), &["commit", "-qm", "add symlink"]);
    let before = fs::metadata(external.path()).unwrap().permissions().mode();

    let workspace =
        prepare_read_only_workspace(repo.path(), "explore-codebase", "read-link", None).unwrap();
    assert_eq!(
        fs::metadata(external.path()).unwrap().permissions().mode(),
        before
    );
    cleanup_read_only(repo.path(), &workspace).unwrap();
    assert_eq!(
        fs::metadata(external.path()).unwrap().permissions().mode(),
        before
    );
}

#[test]
fn linked_worktrees_share_identity_and_integration_lock() {
    let repo = repository();
    let linked = repo.path().join("linked");
    git(
        repo.path(),
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
    );
    assert_eq!(
        repository_identity(repo.path()).unwrap().common_dir,
        repository_identity(&linked).unwrap().common_dir
    );
    let lock = RepositoryLock::acquire(repo.path()).unwrap();
    assert!(matches!(
        RepositoryLock::acquire(&linked),
        Err(SoftwareDevelopmentError::Locked)
    ));
    drop(lock);
    RepositoryLock::acquire(&linked).unwrap();

    let from_main =
        prepare_isolated_workspace(repo.path(), "implement-change", "main-run", None).unwrap();
    let from_linked =
        prepare_isolated_workspace(&linked, "implement-change", "linked-run", None).unwrap();
    assert_eq!(from_main.run_dir.parent(), from_linked.run_dir.parent());
}

#[test]
fn result_state_is_durable_and_failure_workspace_is_preserved() {
    let repo = repository();
    let workspace =
        prepare_isolated_workspace(repo.path(), "repair-validation", "failed-run", None).unwrap();
    record_result(
        &workspace.run_dir,
        &RunResult {
            outcome: "fail".into(),
            branch: Some(workspace.branch.clone()),
            base_commit: workspace.base_commit.clone(),
            worktree: Some(workspace.worktree.clone()),
            commits: vec![],
            validation: vec![],
            summary: "validation failed".into(),
            base_divergence: Some("target advanced by one commit".into()),
        },
    )
    .unwrap();
    let result: RunResult =
        serde_json::from_slice(&fs::read(workspace.run_dir.join("result.json")).unwrap()).unwrap();
    assert_eq!(result.outcome, "fail");
    assert!(workspace.worktree.exists());
}

#[test]
fn merge_uses_an_explicit_validated_commit_and_rejects_dirty_targets() {
    let repo = repository();
    let target_branch = git(repo.path(), &["branch", "--show-current"]);
    git(repo.path(), &["checkout", "-qb", "spark/source/run"]);
    fs::write(repo.path().join("feature.txt"), "shipped\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "feature"]);
    git(repo.path(), &["checkout", "-q", &target_branch]);

    let result = merge_change(
        repo.path(),
        "spark/source/run",
        "merge-ok",
        "test -f feature.txt",
    )
    .unwrap();
    assert_eq!(result.validation[0].exit_code, Some(0));
    assert_eq!(
        git(repo.path(), &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );

    fs::write(repo.path().join("dirty.txt"), "mine\n").unwrap();
    assert!(matches!(
        merge_change(repo.path(), "spark/source/run", "merge-dirty", "true"),
        Err(SoftwareDevelopmentError::DirtyTarget)
    ));
}

#[test]
fn failed_merge_validation_preserves_integration_workspace() {
    let repo = repository();
    let target_branch = git(repo.path(), &["branch", "--show-current"]);
    git(repo.path(), &["checkout", "-qb", "spark/source/failure"]);
    fs::write(repo.path().join("feature.txt"), "candidate\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "candidate"]);
    git(repo.path(), &["checkout", "-q", &target_branch]);
    let common = repository_identity(repo.path()).unwrap().common_dir;

    assert!(merge_change(repo.path(), "spark/source/failure", "merge-fail", "false").is_err());
    assert!(common.join("spark/worktrees/merge-fail").exists());
    assert_eq!(
        git(repo.path(), &["rev-parse", "HEAD"]),
        git(repo.path(), &["rev-parse", &target_branch])
    );
    let result = repo
        .path()
        .join(".spark/software-development/runs/merge-fail/result.json");
    let result: serde_json::Value = serde_json::from_slice(&fs::read(result).unwrap()).unwrap();
    assert_eq!(result["outcome"], "fail");
    assert_eq!(result["validation"][0]["exit_code"], 1);
}

#[test]
fn invalid_merge_inputs_do_not_change_the_target_or_create_runtime_state() {
    let repo = repository();
    let before = git(repo.path(), &["rev-parse", "HEAD"]);
    assert!(merge_change(repo.path(), "missing", "invalid-merge", "true").is_err());
    assert_eq!(git(repo.path(), &["rev-parse", "HEAD"]), before);
    assert!(!repo
        .path()
        .join(".spark/software-development/runs/invalid-merge")
        .exists());
}

#[test]
fn successful_merge_removes_the_source_worktree_and_records_durable_evidence() {
    let repo = repository();
    let source_root = tempfile::tempdir().unwrap();
    let source = source_root.path().join("source-worktree");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-qb",
            "spark/source/worktree",
            source.to_str().unwrap(),
        ],
    );
    fs::write(source.join("feature.txt"), "candidate\n").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-qm", "candidate"]);

    merge_change(
        repo.path(),
        "spark/source/worktree",
        "merge-cleanup",
        "test -f feature.txt",
    )
    .unwrap();

    assert!(!source.exists());
    let result = repository_identity(repo.path())
        .unwrap()
        .common_dir
        .parent()
        .unwrap()
        .join(".spark/software-development/runs/merge-cleanup/result.json");
    assert!(result.is_file());
    let value: serde_json::Value = serde_json::from_slice(&fs::read(result).unwrap()).unwrap();
    assert_eq!(value["validation"][0]["exit_code"], 0);
}

#[test]
fn merge_stops_when_the_target_moves_during_validation() {
    let repo = repository();
    let target_branch = git(repo.path(), &["branch", "--show-current"]);
    git(repo.path(), &["checkout", "-qb", "spark/source/movement"]);
    fs::write(repo.path().join("feature.txt"), "candidate\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "candidate"]);
    git(repo.path(), &["checkout", "-q", &target_branch]);
    let command = format!(
        "git -C '{}' commit --allow-empty -m moved >/dev/null",
        repo.path().display()
    );

    assert!(matches!(
        merge_change(
            repo.path(),
            "spark/source/movement",
            "merge-moved",
            &command
        ),
        Err(SoftwareDevelopmentError::TargetMoved { .. })
    ));
    assert!(repository_identity(repo.path())
        .unwrap()
        .common_dir
        .join("spark/worktrees/merge-moved")
        .exists());
}

#[test]
fn merge_session_keeps_the_lock_while_a_bounded_conflict_is_resolved() {
    let repo = repository();
    let target_branch = git(repo.path(), &["branch", "--show-current"]);
    git(repo.path(), &["checkout", "-qb", "spark/source/conflict"]);
    fs::write(repo.path().join("README.md"), "source\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "source"]);
    git(repo.path(), &["checkout", "-q", &target_branch]);
    fs::write(repo.path().join("README.md"), "target\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "target"]);

    let session =
        prepare_merge_change(repo.path(), "spark/source/conflict", "merge-conflict").unwrap();
    assert!(matches!(
        RepositoryLock::acquire(repo.path()),
        Err(SoftwareDevelopmentError::Locked)
    ));
    fs::write(
        session.workspace.worktree.join("README.md"),
        "target and source\n",
    )
    .unwrap();
    let result = finish_merge_change(session, "grep -q 'target and source' README.md").unwrap();
    assert_eq!(result.validation[0].exit_code, Some(0));
    assert_eq!(
        git(repo.path(), &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
    RepositoryLock::acquire(repo.path()).unwrap();
}
