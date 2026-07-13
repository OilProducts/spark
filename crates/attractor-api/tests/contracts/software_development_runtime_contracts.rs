use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use attractor_api::{AttractorApiService, PipelineStartRequest, RuntimeHandlerRunnerFactory};
use attractor_core::{Outcome, OutcomeStatus};
use attractor_runtime::{RuntimeHandlerRunner, HANDLER_CODERGEN};
use serde_json::json;
use spark_common::settings::SparkSettings;
use std::collections::BTreeMap;

#[test]
fn launcher_policy_prepares_and_cleans_an_isolated_runtime_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "spark@example.invalid"]);
    git(&repo, &["config", "user.name", "Spark Tests"]);
    fs::write(repo.join("task.md"), "ship it\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);

    let service = AttractorApiService::new(settings(temp.path()));
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("runtime-boundary".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(flow("isolated_branch")),
        launch_context: Some(
            [("context.request.artifact_path".into(), json!("task.md"))]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    });

    assert_eq!(response.status_code, 200, "{}", response.body);
    let run_dir = run_dir(&repo, "runtime-boundary");
    assert!(run_dir.join("task.json").is_file());
    assert!(run_dir.join("result.json").is_file());
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["commits"].as_array().unwrap().len(), 1);
    assert_eq!(result["validation"][0]["exit_code"], 0);
    assert_eq!(result["base_divergence"], "0 0");
    let workspace: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("workspace.json")).unwrap()).unwrap();
    assert!(!Path::new(workspace["worktree"].as_str().unwrap()).exists());
    assert!(git(
        &repo,
        &["branch", "--list", "spark/runtime-test/runtime-boundary"]
    )
    .contains("spark/runtime-test/runtime-boundary"));
    assert!(!git(
        &repo,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            "spark/runtime-test/runtime-boundary"
        ]
    )
    .lines()
    .any(|path| path.starts_with(".spark/")));
}

#[test]
fn isolated_launcher_reports_selected_base_ref_drift_without_rebasing() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let base_branch = git(&repo, &["branch", "--show-current"]).trim().to_string();
    let starting_base = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    let source_repo = repo.clone();
    let factory: RuntimeHandlerRunnerFactory = Arc::new(move || {
        let mut runner = RuntimeHandlerRunner::new();
        let source_repo = source_repo.clone();
        runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime| {
            fs::write(runtime.run_workdir.join("implemented.txt"), "managed\n").unwrap();
            fs::write(source_repo.join("base-advanced.txt"), "advanced\n").unwrap();
            git(&source_repo, &["add", "base-advanced.txt"]);
            git(&source_repo, &["commit", "-qm", "advance selected base"]);
            Ok(Outcome::new(OutcomeStatus::Success))
        });
        runner
    });

    let response = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings(temp.path()),
        factory,
    )
    .start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("base-ref-drift".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(agent_flow("implement_change", "isolated_branch")),
        launch_context: Some(
            [
                (
                    "context.request.objective".into(),
                    json!("implement safely"),
                ),
                ("context.request.base_ref".into(), json!(base_branch)),
                (
                    "context.request.validation_command".into(),
                    json!("test -f implemented.txt"),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    });

    assert_eq!(response.status_code, 200, "{}", response.body);
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "base-ref-drift").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["base_commit"], starting_base);
    assert_eq!(result["base_divergence"], "0 1");
    assert_eq!(result["commits"].as_array().unwrap().len(), 1);
    assert_eq!(
        git(
            &repo,
            &[
                "merge-base",
                &starting_base,
                "spark/implement-change/base-ref-drift",
            ],
        )
        .trim(),
        starting_base
    );
}

#[test]
fn read_only_launcher_executes_from_a_detached_committed_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "spark@example.invalid"]);
    git(&repo, &["config", "user.name", "Spark Tests"]);
    fs::write(repo.join("task.md"), "committed\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    fs::write(repo.join("task.md"), "dirty\n").unwrap();

    let service = AttractorApiService::new(settings(temp.path()));
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("read-boundary".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(flow("read_only")),
        launch_context: Some(
            [("context.request.artifact_path".into(), json!("task.md"))]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    });
    assert_eq!(response.status_code, 200, "{}", response.body);
    assert_eq!(fs::read_to_string(repo.join("task.md")).unwrap(), "dirty\n");
    let workspace: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "read-boundary").join("workspace.json")).unwrap(),
    )
    .unwrap();
    assert!(workspace["source_dirty"].as_bool().unwrap());
    assert_eq!(
        workspace["warnings"],
        json!(["Uncommitted source-checkout changes were excluded from the execution workspace."])
    );
    assert_eq!(response.body["warnings"], workspace["warnings"]);
    let task: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "read-boundary").join("task.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(task["objective"], "committed");
    assert!(!Path::new(workspace["worktree"].as_str().unwrap()).exists());
}

#[test]
fn read_only_launcher_preserves_authored_findings_and_adds_lifecycle_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let run_dir = run_dir(&repo, "read-result");
    fs::create_dir_all(&run_dir).unwrap();
    let authored = json!({"outcome":"success","findings":[{"summary":"substantive"}]});
    fs::write(
        run_dir.join("result.json"),
        serde_json::to_vec(&authored).unwrap(),
    )
    .unwrap();

    let response =
        AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some("read-result".into()),
            working_directory: repo.to_string_lossy().into_owned(),
            flow_content: Some(flow("read_only")),
            launch_context: Some(
                [("context.request.objective".into(), json!("inspect"))]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        });
    assert_eq!(response.status_code, 200, "{}", response.body);
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("result.json")).unwrap()).unwrap();
    assert_eq!(persisted["findings"], authored["findings"]);
    assert_eq!(persisted["outcome"], "success");
    assert_eq!(
        persisted["base_commit"],
        git(&repo, &["rev-parse", "HEAD"]).trim()
    );
    assert!(persisted["branch"].is_null());
    assert!(persisted["worktree"].as_str().is_some());
    assert_eq!(persisted["commits"], json!([]));
    assert_eq!(persisted["validation"], json!([]));
    assert!(persisted["summary"].as_str().is_some());
    assert_eq!(persisted["base_divergence"], "0 0");
}

#[test]
fn read_only_launcher_repairs_malformed_authored_result() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let run_dir = run_dir(&repo, "malformed-read-result");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(run_dir.join("result.json"), b"not json").unwrap();

    let response =
        AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some("malformed-read-result".into()),
            working_directory: repo.to_string_lossy().into_owned(),
            flow_content: Some(flow("read_only")),
            launch_context: Some(
                [("context.request.objective".into(), json!("inspect"))]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        });

    assert_eq!(response.status_code, 200, "{}", response.body);
    let repaired: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("result.json")).unwrap()).unwrap();
    assert_eq!(repaired["outcome"], "success");
    assert!(repaired["base_commit"].as_str().is_some());
    assert_eq!(repaired["commits"], json!([]));
    assert_eq!(repaired["validation"], json!([]));
    assert!(repaired["summary"].as_str().is_some());
    assert_eq!(repaired["base_divergence"], "0 0");
}

#[test]
fn review_launcher_with_optional_fixes_uses_one_runtime_managed_commit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let response =
        AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some("review-fixes".into()),
            working_directory: repo.to_string_lossy().into_owned(),
            flow_content: Some(flow_with_id("review_change", "read_only")),
            launch_context: Some(
                [
                    ("context.request.objective".into(), json!("review and fix")),
                    (
                        "context.request.git_result".into(),
                        json!({"apply_fixes": true}),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        });
    assert_eq!(response.status_code, 200, "{}", response.body);
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "review-fixes").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["commits"].as_array().unwrap().len(), 1);
    assert_eq!(result["branch"], "spark/review-change/review-fixes");
    assert!(!Path::new(result["worktree"].as_str().unwrap()).exists());
}

#[test]
fn merge_launcher_integrates_only_after_its_execution_path_completes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let target = git(&repo, &["branch", "--show-current"]);
    git(&repo, &["checkout", "-qb", "spark/source/launcher"]);
    fs::write(repo.join("feature.txt"), "ready\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "feature"]);
    git(&repo, &["checkout", "-q", target.trim()]);

    let response =
        AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some("merge-launcher".into()),
            working_directory: repo.to_string_lossy().into_owned(),
            flow_content: Some(flow_with_id("merge_change", "repository_integration")),
            launch_context: Some(
                [
                    ("context.request.objective".into(), json!("merge")),
                    (
                        "context.request.validation_command".into(),
                        json!("test -f feature.txt"),
                    ),
                    (
                        "context.request.git_result".into(),
                        json!({"source_ref":"spark/source/launcher"}),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        });
    assert_eq!(response.status_code, 200, "{}", response.body);
    assert!(repo.join("feature.txt").is_file());
    assert_eq!(
        git(&repo, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
}

#[test]
fn merge_launcher_discovers_and_records_validation_when_command_is_omitted() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    fs::write(
        repo.join("package.json"),
        r#"{"scripts":{"test":"test -f feature.txt","build":"test -f feature.txt"}}"#,
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "validation gate"]);
    let target = git(&repo, &["branch", "--show-current"]);
    git(&repo, &["checkout", "-qb", "spark/source/discovered"]);
    fs::write(repo.join("feature.txt"), "ready\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "feature"]);
    git(&repo, &["checkout", "-q", target.trim()]);

    let response =
        AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some("merge-discovery".into()),
            working_directory: repo.to_string_lossy().into_owned(),
            flow_content: Some(flow_with_id("merge_change", "repository_integration")),
            launch_context: Some(
                [
                    ("context.request.objective".into(), json!("merge")),
                    (
                        "context.request.git_result".into(),
                        json!({"source_ref":"spark/source/discovered"}),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        });
    assert_eq!(response.status_code, 200, "{}", response.body);
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "merge-discovery").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        result["validation"][0]["command"],
        "npm test && npm run build"
    );
    assert_eq!(result["validation"][0]["exit_code"], 0);
}

#[test]
fn mutating_launcher_discovers_the_repository_gate_when_command_is_omitted() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    fs::write(
        repo.join("package.json"),
        r#"{"scripts":{"test":"true","build":"true"}}"#,
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "validation gate"]);

    let response =
        AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some("update-dependencies-discovery".into()),
            working_directory: repo.to_string_lossy().into_owned(),
            flow_content: Some(flow_with_id("update_dependencies", "isolated_branch")),
            launch_context: Some(
                [(
                    "context.request.objective".into(),
                    json!("update dependencies"),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        });
    assert_eq!(response.status_code, 200, "{}", response.body);
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "update-dependencies-discovery").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        result["validation"][0]["command"],
        "npm test && npm run build"
    );
    assert_eq!(result["validation"][0]["exit_code"], 0);
}

#[test]
fn implement_spec_program_accepts_its_legacy_spec_path_at_the_launcher_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let response =
        AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some("legacy-spec".into()),
            working_directory: repo.to_string_lossy().into_owned(),
            flow_content: Some(flow_with_id("implement_spec_program", "isolated_branch")),
            launch_context: Some(
                [("context.request.spec_path".into(), json!("task.md"))]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        });
    assert_eq!(response.status_code, 200, "{}", response.body);
    let task: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir(&repo, "legacy-spec").join("task.json")).unwrap())
            .unwrap();
    assert_eq!(task["artifact_path"], "task.md");
    assert_eq!(task["objective"], "ship it");
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "legacy-spec").join("result.json")).unwrap(),
    )
    .unwrap();
    assert!(result["commits"].as_array().unwrap().is_empty());
}

#[test]
fn validation_failure_marks_the_real_run_failed_and_preserves_the_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let service = AttractorApiService::new(settings(temp.path()));
    let _response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("validation-failure".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(flow("isolated_branch")),
        launch_context: Some(
            [
                ("context.request.objective".into(), json!("ship it")),
                ("context.request.validation_command".into(), json!("false")),
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    });
    assert_eq!(
        service.get_pipeline("validation-failure").body["status"],
        "failed"
    );
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "validation-failure").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["outcome"], "fail");
    assert!(Path::new(result["worktree"].as_str().unwrap()).exists());
    assert!(result["commits"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn commit_failure_marks_the_real_run_failed_and_preserves_the_worktree() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let hooks = temp.path().join("hooks");
    fs::create_dir(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).unwrap();
    git(
        &repo,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );

    let service = AttractorApiService::new(settings(temp.path()));
    let _response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("commit-failure".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(flow("isolated_branch")),
        launch_context: Some(
            [("context.request.objective".into(), json!("ship it"))]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    });
    assert_eq!(
        service.get_pipeline("commit-failure").body["status"],
        "failed"
    );
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "commit-failure").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["outcome"], "fail");
    assert!(Path::new(result["worktree"].as_str().unwrap()).exists());
}

#[test]
fn injected_agent_reads_runtime_paths_and_persists_a_substantive_read_only_result() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let factory: RuntimeHandlerRunnerFactory = Arc::new(|| {
        let mut runner = RuntimeHandlerRunner::new();
        runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime| {
            let task_path = runtime.context["internal.software_development.task_path"]
                .as_str()
                .unwrap();
            let result_path = runtime.context["internal.software_development.result_path"]
                .as_str()
                .unwrap();
            let run_dir = runtime.context["internal.software_development.run_dir"]
                .as_str()
                .unwrap();
            assert_eq!(Path::new(task_path).parent().unwrap(), Path::new(run_dir));
            assert_eq!(Path::new(result_path).parent().unwrap(), Path::new(run_dir));
            assert_eq!(
                runtime.run_workdir,
                Path::new(
                    runtime.context["internal.software_development.workspace"]["worktree"]
                        .as_str()
                        .unwrap()
                )
            );
            let task: serde_json::Value =
                serde_json::from_slice(&fs::read(task_path).unwrap()).unwrap();
            fs::write(
                result_path,
                serde_json::to_vec(&json!({
                    "outcome": "success",
                    "summary": "inspected committed task",
                    "findings": [{"evidence": task["objective"], "priority": "high"}],
                    "agent_cwd": runtime.run_workdir,
                }))
                .unwrap(),
            )
            .unwrap();
            Ok(Outcome::new(OutcomeStatus::Success))
        });
        runner
    });
    let response = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings(temp.path()),
        factory,
    )
    .start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("agent-read-result".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(agent_flow("explore_codebase", "read_only")),
        launch_context: Some(
            [("context.request.objective".into(), json!("trace startup"))]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    });
    assert_eq!(response.status_code, 200, "{}", response.body);
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "agent-read-result").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["findings"][0]["evidence"], "trace startup");
    assert_eq!(result["outcome"], "success");
    assert!(result["base_commit"].as_str().is_some());
    assert_eq!(result["commits"], json!([]));
    assert_eq!(result["validation"], json!([]));
    assert_eq!(result["base_divergence"], "0 0");
    assert!(!Path::new(result["agent_cwd"].as_str().unwrap()).exists());
}

#[test]
fn injected_agent_mutates_only_the_managed_workspace_and_success_cleans_it() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let factory: RuntimeHandlerRunnerFactory = Arc::new(|| {
        let mut runner = RuntimeHandlerRunner::new();
        runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime| {
            let task_path = runtime.context["internal.software_development.task_path"]
                .as_str()
                .unwrap();
            let task: serde_json::Value =
                serde_json::from_slice(&fs::read(task_path).unwrap()).unwrap();
            assert_eq!(task["objective"], "implement boundary");
            fs::write(runtime.run_workdir.join("implemented.txt"), "managed\n").unwrap();
            Ok(Outcome::new(OutcomeStatus::Success))
        });
        runner
    });
    let response = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings(temp.path()),
        factory,
    )
    .start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("agent-mutation".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(agent_flow("implement_change", "isolated_branch")),
        launch_context: Some(
            [
                (
                    "context.request.objective".into(),
                    json!("implement boundary"),
                ),
                (
                    "context.request.validation_command".into(),
                    json!("test -f implemented.txt"),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    });
    assert_eq!(response.status_code, 200, "{}", response.body);
    assert!(!repo.join("implemented.txt").exists());
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "agent-mutation").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["outcome"], "success");
    assert!(!Path::new(result["worktree"].as_str().unwrap()).exists());
    assert!(git(
        &repo,
        &[
            "show",
            "spark/implement-change/agent-mutation:implemented.txt"
        ]
    )
    .contains("managed"));
}

#[test]
fn bounded_launcher_consolidates_multiple_agent_commits_into_one_validated_commit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let factory: RuntimeHandlerRunnerFactory = Arc::new(|| {
        let mut runner = RuntimeHandlerRunner::new();
        runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime| {
            fs::write(runtime.run_workdir.join("one.txt"), "one\n").unwrap();
            git(&runtime.run_workdir, &["add", "one.txt"]);
            git(&runtime.run_workdir, &["commit", "-qm", "agent one"]);
            fs::write(runtime.run_workdir.join("two.txt"), "two\n").unwrap();
            git(&runtime.run_workdir, &["add", "two.txt"]);
            git(&runtime.run_workdir, &["commit", "-qm", "agent two"]);
            Ok(Outcome::new(OutcomeStatus::Success))
        });
        runner
    });
    let response = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings(temp.path()),
        factory,
    )
    .start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("agent-multiple-commits".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(agent_flow("implement_change", "isolated_branch")),
        launch_context: Some(
            [
                ("context.request.objective".into(), json!("commit twice")),
                (
                    "context.request.validation_command".into(),
                    json!("test -f one.txt && test -f two.txt"),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    });
    assert_eq!(response.status_code, 200, "{}", response.body);
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "agent-multiple-commits").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["outcome"], "success");
    assert_eq!(result["commits"].as_array().unwrap().len(), 1);
    assert_eq!(
        git(
            &repo,
            &[
                "rev-list",
                "--count",
                "HEAD..spark/implement-change/agent-multiple-commits",
            ],
        )
        .trim(),
        "1"
    );
}

#[test]
fn injected_agent_failure_preserves_its_workspace_and_durable_diagnosis() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    let factory: RuntimeHandlerRunnerFactory = Arc::new(|| {
        let mut runner = RuntimeHandlerRunner::new();
        runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime| {
            let result_path = runtime.context["internal.software_development.result_path"]
                .as_str()
                .unwrap();
            fs::write(
                result_path,
                serde_json::to_vec(&json!({
                    "outcome": "fail",
                    "summary": "reproduction failed at the injected boundary",
                    "worktree": runtime.run_workdir,
                }))
                .unwrap(),
            )
            .unwrap();
            Ok(Outcome::new(OutcomeStatus::Fail))
        });
        runner
    });
    let _response = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings(temp.path()),
        factory,
    )
    .start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("agent-failure".into()),
        working_directory: repo.to_string_lossy().into_owned(),
        flow_content: Some(agent_flow("plan_change", "read_only")),
        launch_context: Some(
            [("context.request.objective".into(), json!("plan safely"))]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    });
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(run_dir(&repo, "agent-failure").join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["outcome"], "fail");
    assert!(result["base_commit"].as_str().is_some());
    assert!(result["branch"].is_null());
    assert_eq!(result["commits"], json!([]));
    assert_eq!(result["validation"], json!([]));
    assert_eq!(
        result["summary"],
        "reproduction failed at the injected boundary"
    );
    assert_eq!(result["base_divergence"], "0 0");
    assert!(Path::new(result["worktree"].as_str().unwrap()).exists());
}

#[test]
fn invalid_task_cardinality_is_rejected_before_workspace_creation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = initialized_repository(temp.path());
    for (run_id, context) in [
        ("neither-input", BTreeMap::new()),
        (
            "both-input",
            [
                ("context.request.objective".into(), json!("one")),
                ("context.request.artifact_path".into(), json!("task.md")),
            ]
            .into_iter()
            .collect(),
        ),
    ] {
        let response =
            AttractorApiService::new(settings(temp.path())).start_pipeline(PipelineStartRequest {
                wait: Some(true),
                run_id: Some(run_id.into()),
                working_directory: repo.to_string_lossy().into_owned(),
                flow_content: Some(flow("isolated_branch")),
                launch_context: Some(context),
                ..Default::default()
            });
        assert_eq!(
            response.body["status"], "validation_error",
            "{}",
            response.body
        );
        assert!(!run_dir(&repo, run_id).exists());
    }
}

fn initialized_repository(root: &Path) -> std::path::PathBuf {
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "spark@example.invalid"]);
    git(&repo, &["config", "user.name", "Spark Tests"]);
    fs::write(repo.join("task.md"), "ship it\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    repo
}

fn run_dir(repo: &Path, run_id: &str) -> std::path::PathBuf {
    let common = git(repo, &["rev-parse", "--git-common-dir"]);
    let common = repo.join(common.trim());
    common
        .parent()
        .unwrap()
        .join(".spark/software-development/runs")
        .join(run_id)
}

fn flow(policy: &str) -> String {
    flow_with_id("runtime_test", policy)
}

fn flow_with_id(id: &str, policy: &str) -> String {
    format!(
        r#"schema_version: '1'
id: {id}
metadata:
  software_development:
    launcher: true
    workspace_policy: {policy}
nodes:
  start:
    kind: start
  done:
    kind: exit
edges:
  - from: start
    to: done
"#
    )
}

fn agent_flow(id: &str, policy: &str) -> String {
    format!(
        r#"schema_version: '1'
id: {id}
metadata:
  software_development:
    launcher: true
    workspace_policy: {policy}
nodes:
  start:
    kind: start
  agent:
    kind: agent_task
    config:
      kind: agent_task
      prompt: use the declared runtime boundary
  done:
    kind: exit
edges:
  - from: start
    to: agent
  - from: agent
    to: done
    condition: outcome=success
"#
    )
}

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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn settings(root: &Path) -> SparkSettings {
    let home = root.join("spark-home");
    SparkSettings {
        project_root: root.join("project"),
        data_dir: home.clone(),
        config_dir: home.join("config"),
        runtime_dir: home.join("runtime"),
        logs_dir: home.join("logs"),
        workspace_dir: home.join("workspace"),
        projects_dir: home.join("workspace/projects"),
        attractor_dir: home.join("attractor"),
        runs_dir: home.join("attractor/runs"),
        flows_dir: home.join("flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
