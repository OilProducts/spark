use std::fs;
use std::path::Path;

use attractor_api::{AttractorApiService, PipelineStartRequest};
use serde_json::json;
use spark_common::settings::SparkSettings;

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
        runs_dir: None,
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

fn parent_flow() -> serde_json::Value {
    json!({
        "schema_version": "2",
        "id": "lifecycle_parent",
        "title": "Lifecycle Parent",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {
                        "to": "prepare_workspace",
                        "description": "Enter the first workflow step."
                    }
                }
            },
            "prepare_workspace": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "set -eu\nbranch=\"spark/lifecycle/${SPARK_RUN_ID}\"\ncheckout_root=\"${SPARK_PROJECT_WORK_DIR}/checkouts\"\nworktree=\"${checkout_root}/${SPARK_RUN_ID}\"\nmkdir -p \"$checkout_root\"\ngit worktree add -b \"$branch\" \"$worktree\" HEAD >/dev/null\nprintf '{\"path\":\"%s\",\"branch\":\"%s\"}' \"$worktree\" \"$branch\"\n",
                    "env_map": {"SPARK_RUN_ID": "internal.run_id"},
                    "output_map": {
                        "context.workspace.path": "path",
                        "context.workspace.branch": "branch"
                    },
                    "exit_codes": {"0": "done"}
                },
                "contracts": {
                    "writes_context": ["context.workspace.path", "context.workspace.branch"]
                },
                "routes": {"done": {"to": "work", "description": "The step chose done."}}
            },
            "work": {
                "kind": "subflow",
                "config": {
                    "kind": "subflow",
                    "flow_ref": "lifecycle-child",
                    "results": {"completed": "done"}
                },
                "manager": {"child_workdir_from": "context.workspace.path"},
                "routes": {
                    "done": {"to": "finalize_commit", "description": "The step chose done."}
                }
            },
            "finalize_commit": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "set -eu\ncd \"$SPARK_WORKSPACE_PATH\"\ngit add --all\ngit commit -m \"lifecycle change\" >/dev/null\nprintf '{\"commit\":\"%s\"}' \"$(git rev-parse HEAD)\"\n",
                    "env_map": {"SPARK_WORKSPACE_PATH": "context.workspace.path"},
                    "output_map": {"context.workspace.commit": "commit"},
                    "exit_codes": {"0": "done"}
                },
                "contracts": {"writes_context": ["context.workspace.commit"]},
                "routes": {
                    "done": {"to": "cleanup_workspace", "description": "The step chose done."}
                }
            },
            "cleanup_workspace": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "set -eu\ngit worktree remove \"$SPARK_WORKSPACE_PATH\"\n",
                    "env_map": {"SPARK_WORKSPACE_PATH": "context.workspace.path"},
                    "exit_codes": {"0": "done"}
                },
                "routes": {"done": {"to": "done", "description": "The step chose done."}}
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

fn child_flow() -> serde_json::Value {
    json!({
        "schema_version": "2",
        "id": "lifecycle_child",
        "title": "Lifecycle Child",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "produce", "description": "Enter the first workflow step."}
                }
            },
            "produce": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "printf 'produced by child' > produced.txt\nprintf '%s' \"$SPARK_PROJECT_WORK_DIR\" > work_dir.txt",
                    "exit_codes": {"0": "done"}
                },
                "routes": {"done": {"to": "done", "description": "The step chose done."}}
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

#[test]
fn visible_worktree_lifecycle_commits_child_work_on_branch_and_removes_worktree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    // The parent's flow_ref names the child installed beside it.
    let parent = crate::install_flow(&settings, parent_flow());
    assert_eq!(
        crate::install_flow(&settings, child_flow()),
        "tests/lifecycle-child"
    );

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
        flow_name: Some(parent),
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
    // The child runs in the worktree but keeps the root project's work area.
    let work = spark_common::project::project_work_dir(&settings.data_dir, &repo.to_string_lossy())
        .unwrap();
    assert_eq!(
        git_in(&repo, &["show", &format!("{branch}:work_dir.txt")]),
        work.to_string_lossy()
    );
    let subject = git_in(&repo, &["log", "-1", "--format=%s", branch]);
    assert_eq!(subject, "lifecycle change");
    assert!(
        !branch_files.contains(".spark"),
        "the worktree directory must never be committed: {branch_files}"
    );

    assert!(
        !work.join("checkouts/run-lifecycle").exists(),
        "successful runs must remove their worktree"
    );
    assert!(work.join("checkouts").is_dir());
    assert!(
        !repo.join(".git/spark").exists(),
        "worktrees must not be created inside .git"
    );
    assert!(
        !repo.join(".spark").exists(),
        "working files stay out of the source checkout"
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

fn runs_root_flow() -> serde_json::Value {
    json!({
        "schema_version": "2",
        "id": "runs_root_probe",
        "title": "Runs Root Probe",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "probe", "description": "Enter the first workflow step."}
                }
            },
            "probe": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "printf '%s\\n%s' \"$SPARK_RUNS_ROOT\" \"$SPARK_PROJECT_WORK_DIR\"",
                    "env_map": {"SPARK_RUNS_ROOT": "internal.runs_dir"},
                    "exit_codes": {"0": "done"}
                },
                "routes": {"done": {"to": "done", "description": "The step chose done."}}
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

/// Runs default to the owning project's `G/runs`, or a configured external
/// root's `<root>/<project-id>`; tools see the runs root and `G/work`, which
/// the launch creates.
#[test]
fn tool_nodes_see_the_runtime_runs_root_and_project_work_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let default = settings(temp.path());
    let external = SparkSettings {
        runs_dir: Some(temp.path().join("external-runs")),
        ..default.clone()
    };
    fs::create_dir_all(&default.config_dir).expect("config dir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    let project_id = spark_common::project::build_project_id(project.to_string_lossy()).unwrap();
    let storage = default.data_dir.join("projects").join(&project_id);
    let flow = crate::install_flow(&default, runs_root_flow());

    for (settings, runs_root, run_root) in [
        (
            &default,
            default.data_dir.join("projects"),
            storage.join("runs/run-default"),
        ),
        (
            &external,
            temp.path().join("external-runs"),
            temp.path()
                .join("external-runs")
                .join(&project_id)
                .join("run-external"),
        ),
    ] {
        let run_id = run_root.file_name().unwrap().to_string_lossy().to_string();
        let service = AttractorApiService::new(settings.clone());
        let response = service.start_pipeline(PipelineStartRequest {
            wait: Some(true),
            run_id: Some(run_id.clone()),
            flow_name: Some(flow.clone()),
            working_directory: project.to_string_lossy().to_string(),
            ..PipelineStartRequest::default()
        });
        assert_eq!(response.body["terminal_status"], json!("completed"));

        let store = attractor_runtime::RunStore::for_settings(settings);
        let bundle = store
            .read_run_bundle(&run_id)
            .expect("read bundle")
            .expect("bundle exists");
        assert_eq!(bundle.paths.root, run_root);
        let result = store
            .read_result(&bundle.paths)
            .expect("read result")
            .expect("result exists");
        let probed: Vec<_> = result.body_markdown.trim().lines().collect();
        assert_eq!(
            probed,
            [
                runs_root.to_string_lossy().as_ref(),
                storage.join("work").to_string_lossy().as_ref()
            ],
            "internal.runs_dir and SPARK_PROJECT_WORK_DIR come from the runtime"
        );
        assert!(storage.join("work").is_dir());
    }
}
