use std::fs;
use std::path::Path;
use std::process::Command;

use spark_assets::{flows, frontend, guides, icons, models, resources, ResourceSource};
use spark_common::settings::SparkSettings;

const STARTER_FLOW_NAMES: &[&str] = &[
    "examples/human-review-loop",
    "examples/implement-review-loop",
    "examples/parallel-review",
    "examples/simple-linear",
    "examples/supervision/implementation-worker",
    "examples/supervision/supervised-implementation",
    "math-research/explore-conjecture",
    "math-research/formalize-result",
    "math-research/prove-refute",
    "software-development/audit-codebase",
    "software-development/design-change",
    "software-development/implement-change",
    "software-development/investigate-bug",
    "software-development/merge-change",
    "software-development/review-change",
    "software-development/run-retrospective",
    "software-development/spec-implementation/implement-milestone",
    "software-development/spec-implementation/implement-spec",
    "software-development/workers/implement-task",
    "software-development/workers/resolve-merge-conflicts",
];

#[test]
fn frontend_resource_order_prefers_explicit_ui_then_source_then_packaged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let explicit_ui = temp.path().join("explicit-ui");
    let explicit_assets = explicit_ui.join("assets");
    fs::create_dir_all(&explicit_assets).expect("explicit assets");
    fs::write(explicit_ui.join("index.html"), "<main>explicit</main>").expect("explicit index");
    fs::write(explicit_assets.join("app.js"), "explicit();").expect("explicit js");

    let project_root = temp.path().join("source-root");
    let source_dist = project_root.join("frontend/dist");
    let source_assets = source_dist.join("assets");
    fs::create_dir_all(&source_assets).expect("source assets");
    fs::write(source_dist.join("index.html"), "<main>source</main>").expect("source index");
    fs::write(source_assets.join("app.js"), "source();").expect("source js");

    let mut settings = settings(temp.path());
    settings.project_root = project_root.clone();
    settings.ui_dir = Some(explicit_ui.clone());

    let explicit_index = frontend::load_index(&settings).expect("explicit index");
    assert_eq!(explicit_index.source(), ResourceSource::ExplicitUiDir);
    assert_eq!(
        explicit_index.filesystem_path().map(Path::to_path_buf),
        Some(explicit_ui.join("index.html"))
    );
    assert_eq!(
        explicit_index.text().expect("utf-8"),
        "<main>explicit</main>"
    );
    assert_eq!(
        frontend::load_asset(&settings, "assets/app.js")
            .expect("asset lookup")
            .expect("explicit asset")
            .text()
            .expect("utf-8"),
        "explicit();"
    );

    settings.ui_dir = None;
    let source_index = frontend::load_index(&settings).expect("source index");
    assert_eq!(source_index.source(), ResourceSource::SourceTree);
    assert_eq!(source_index.text().expect("utf-8"), "<main>source</main>");
    assert_eq!(
        frontend::load_asset(&settings, "assets/app.js")
            .expect("asset lookup")
            .expect("source asset")
            .text()
            .expect("utf-8"),
        "source();"
    );

    settings.project_root = temp.path().join("missing-source-root");
    let packaged_index = frontend::load_index(&settings).expect("packaged index");
    assert_eq!(packaged_index.source(), ResourceSource::Packaged);
    assert!(packaged_index
        .text()
        .expect("utf-8")
        .contains("<div id=\"root\"></div>"));
    assert_eq!(
        frontend::load_asset(&settings, "assets/spark-app-icon.png")
            .expect("asset lookup")
            .expect("packaged icon")
            .source(),
        ResourceSource::Packaged
    );
    assert!(frontend::packaged_asset_names()
        .iter()
        .any(|name| name == "index.html"));
}

#[test]
fn frontend_asset_lookup_rejects_unsafe_paths_and_symlink_escapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui_root = temp.path().join("ui");
    let assets = ui_root.join("assets");
    fs::create_dir_all(&assets).expect("assets");
    fs::write(ui_root.join("index.html"), "<!doctype html>").expect("index");
    fs::write(temp.path().join("secret.txt"), "secret").expect("secret");
    #[cfg(unix)]
    std::os::unix::fs::symlink(temp.path().join("secret.txt"), assets.join("escape.txt"))
        .expect("symlink");

    let mut settings = settings(temp.path());
    settings.ui_dir = Some(ui_root);

    for unsafe_path in [
        "",
        "../Cargo.toml",
        "assets/../Cargo.toml",
        "/tmp/secret",
        ".",
    ] {
        assert!(
            frontend::load_asset(&settings, unsafe_path).is_err(),
            "{unsafe_path}"
        );
    }

    #[cfg(unix)]
    assert!(frontend::load_asset(&settings, "assets/escape.txt").is_err());
}

#[test]
fn starter_flow_inventory_is_packaged_sorted_and_assembles() {
    let names = flows::starter_flow_names()
        .expect("starter flow names")
        .into_iter()
        .filter(|name| name != "math-research/research-program")
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        STARTER_FLOW_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    );

    for name in &names {
        let files = flows::starter_flow_files(name).expect("flow files");
        assert!(files.keys().all(|file| file.ends_with(".md")), "{name}");
        let flow = bundled_flow(name);
        let software_metadata = flow.metadata.get("software_development");
        if name.starts_with("software-development/workers/")
            || name.contains("/spec-implementation/implement-milestone")
        {
            assert_ne!(
                software_metadata
                    .and_then(|value| value.get("launcher"))
                    .and_then(serde_json::Value::as_bool),
                Some(true),
                "{name} must not be requestable"
            );
        }
    }
    // Every bundled file belongs to a flow directory.
    for asset in flows::starter_flow_assets().expect("starter flow assets") {
        let (flow, _) = asset.name.rsplit_once('/').expect("nested file");
        assert!(
            flow == "math-research/research-program" || STARTER_FLOW_NAMES.contains(&flow),
            "{}",
            asset.name
        );
    }

    for unsafe_name in ["../bad", "/tmp/bad", "examples", "examples/bad.txt"] {
        assert!(
            flows::starter_flow_files(unsafe_name).is_err(),
            "{unsafe_name}"
        );
    }
}

#[test]
fn bundled_flows_round_trip_through_the_directory_writer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let roles_dir = temp.path().join("roles");
    fs::create_dir_all(&roles_dir).expect("roles dir");
    for (name, text) in spark_assets::roles::assets() {
        fs::write(roles_dir.join(name), text).expect("role");
    }
    for name in flows::starter_flow_names().expect("names") {
        let flow = bundled_flow(&name);
        // Bundled files are exactly what the shared writer renders.
        assert_eq!(
            attractor_dsl::render_flow_files(&flow).expect("render"),
            flows::starter_flow_files(&name).expect("files"),
            "{name}"
        );
        // Converting the single-document YAML form and loading it again is lossless.
        let yaml = attractor_dsl::flow_yaml(&flow).expect("yaml");
        let converted = temp.path().join("flows").join(&name);
        attractor_dsl::write_flow_dir(
            &converted,
            &attractor_dsl::parse_flow_definition(&yaml).expect("parse"),
        )
        .expect("write");
        assert_eq!(
            attractor_dsl::read_flow_dir(&converted).expect("load"),
            flow,
            "{name}"
        );
        attractor_dsl::resolve_flow_roles(&flow, &roles_dir)
            .unwrap_or_else(|error| panic!("{name}: {}", error.detail()));
    }
}

/// A bundled flow in its authored form, assembled like the loader does.
fn bundled_flow(name: &str) -> attractor_dsl::FlowDefinition {
    let files = flows::starter_flow_files(name).expect("bundled flow files");
    attractor_dsl::assemble_flow_files(&files)
        .unwrap_or_else(|error| panic!("{name} does not assemble: {}", error.detail()))
}

#[test]
fn implement_milestone_reviews_before_expensive_validation_and_blocks_invalid_state() {
    let flow = bundled_flow("software-development/spec-implementation/implement-milestone");
    for (from, to, condition) in [
        (
            "validate_active_item_state",
            "review_current",
            "outcome=success",
        ),
        ("review_current", "prepare_validation", "outcome=success"),
        ("prepare_validation", "validate_repo", ""),
        (
            "assess_validation",
            "gate_item_completion",
            "outcome=success",
        ),
        (
            "prepare_milestone_state",
            "blocked_exit",
            "outcome=fail && preferred_label=Blocked",
        ),
        (
            "prepare_milestone_state",
            "extract_items",
            "outcome=success",
        ),
    ] {
        assert!(flow
            .edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.condition == condition));
    }
    assert!(!flow
        .edges
        .iter()
        .any(|edge| edge.from == "validate_item_plan" && edge.to == "extract_items"));
}

#[test]
fn analysis_flows_end_with_their_result_and_investigate_bug_verifies_its_plan() {
    for name in [
        "software-development/audit-codebase",
        "software-development/design-change",
        "software-development/investigate-bug",
        "software-development/run-retrospective",
    ] {
        let flow = bundled_flow(name);
        assert!(
            !flow.nodes.values().any(|node| {
                let kind = serde_json::to_value(&node.kind).unwrap();
                kind == "human_gate" || kind == "subflow"
            }),
            "{name} must end with its result"
        );
    }
    let flow = bundled_flow("software-development/investigate-bug");
    for (from, to, condition) in [
        ("propose_plan", "verify_plan", "outcome=success"),
        ("verify_plan", "done", "outcome=success"),
        ("verify_plan", "propose_plan", "outcome=fail"),
    ] {
        assert!(flow
            .edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.condition == condition));
    }
}

#[test]
fn merge_change_skips_validation_only_for_successful_conflict_free_merges() {
    let flow = bundled_flow("software-development/merge-change");
    let routes = flow
        .edges
        .iter()
        .filter(|edge| edge.from == "prepare_integration")
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 2);
    for (to, condition) in [
        (
            "finish_merge",
            "outcome=success && context.integration.conflict_count=0",
        ),
        (
            "resolve_conflicts",
            "outcome=success && context.integration.conflict_count!=0",
        ),
    ] {
        assert!(routes
            .iter()
            .any(|edge| edge.to == to && edge.condition == condition));
    }
    assert!(flow
        .edges
        .iter()
        .any(|edge| edge.from == "resolve_conflicts"
            && edge.to == "validate"
            && edge.condition == "outcome=success"));
    assert!(flow.edges.iter().any(|edge| edge.from == "validate"
        && edge.to == "finish_merge"
        && edge.condition == "outcome=success"));
    let prepare = serde_json::to_value(&flow.nodes["prepare_integration"].config).unwrap();
    let finish = serde_json::to_value(&flow.nodes["finish_merge"].config).unwrap();
    for scenario in ["clean", "conflict", "unrelated", "already-integrated"] {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        seed_change_repository(repo);
        run_git(repo, &["branch", "-M", "target"]);
        run_git(repo, &["config", "user.name", "Contracts"]);
        run_git(repo, &["config", "user.email", "contracts@example.com"]);
        if scenario == "unrelated" {
            run_git(repo, &["checkout", "--orphan", "source"]);
            run_git(repo, &["rm", "-f", "seed.txt"]);
        } else {
            run_git(repo, &["checkout", "-b", "source"]);
        }
        if scenario != "already-integrated" {
            fs::write(repo.join("seed.txt"), "source").unwrap();
            run_git(repo, &["add", "seed.txt"]);
            run_git(repo, &["commit", "-m", "source"]);
        }
        run_git(repo, &["checkout", "target"]);
        if scenario == "conflict" {
            fs::write(repo.join("seed.txt"), "target").unwrap();
            run_git(repo, &["add", "seed.txt"]);
            run_git(repo, &["commit", "-m", "target"]);
        }
        let output = Command::new("sh")
            .arg("-ec")
            .arg(prepare["command"].as_str().unwrap())
            .current_dir(repo)
            .env("SPARK_SOURCE_REF", "source")
            .env("SPARK_RUN_ID", "merge-check")
            .output()
            .unwrap();
        if matches!(scenario, "unrelated" | "already-integrated") {
            assert!(
                !output.status.success(),
                "{scenario} must not reach finish_merge"
            );
            continue;
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            result["conflict_count"],
            if scenario == "clean" { 0 } else { 1 }
        );
        if scenario == "clean" {
            let output = Command::new("sh")
                .arg("-ec")
                .arg(finish["command"].as_str().unwrap())
                .current_dir(repo)
                .env("SPARK_INTEGRATION_PATH", result["path"].as_str().unwrap())
                .env(
                    "SPARK_TARGET_COMMIT",
                    result["target_commit"].as_str().unwrap(),
                )
                .env("SPARK_SOURCE_REF", "source")
                .env("SPARK_RUN_ID", "merge-check")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(fs::read_to_string(repo.join("seed.txt")).unwrap(), "source");
            let parents = Command::new("git")
                .args(["show", "-s", "--format=%P", "HEAD"])
                .current_dir(repo)
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&parents.stdout)
                    .split_whitespace()
                    .count(),
                2
            );
            assert!(!repo.join(".spark/integrations/merge-check").exists());
        }
    }
}

#[test]
fn research_program_separates_correctness_from_substantive_progress() {
    let flow = bundled_flow("math-research/research-program");
    let prompt = |node_id: &str| {
        serde_json::to_value(&flow.nodes[node_id].config).expect("serialize node config")["prompt"]
            .as_str()
            .expect("agent prompt")
            .to_string()
    };

    let research = prompt("research");
    assert!(research.contains("substantive progress"));
    assert!(research.contains("intended frontier change"));
    assert!(research.contains("Changing a finite bound may be substantive"));

    let assess = prompt("assess");
    assert!(assess.contains("correctness separately from substantive progress"));
    assert!(assess.contains("Do not reject finite-bound work categorically"));
    assert!(assess.contains("substantive_progress as a boolean"));
    assert!(assess.contains("substantive_progress_reason"));
    assert!(assess.contains("frontier_change"));

    let record = prompt("record");
    assert!(record.contains("substantive progress separately from claim correctness"));
    assert!(record.contains("prohibit another action of the same kind"));
}

#[test]
fn math_commit_tools_propagate_git_commit_failures() {
    for flow_name in [
        "math-research/explore-conjecture",
        "math-research/formalize-result",
        "math-research/prove-refute",
    ] {
        let flow = bundled_flow(flow_name);
        let commands = flow
            .nodes
            .values()
            .filter(|node| node.label.starts_with("Commit"))
            .filter_map(|node| serde_json::to_value(&node.config).ok())
            .filter_map(|config| config["command"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        assert_eq!(commands.len(), 1, "{flow_name}");

        for (case, command) in commands.iter().enumerate() {
            for commit_succeeds in [true, false] {
                let temp = tempfile::tempdir().expect("tempdir");
                let repo = temp.path();
                run_git(repo, &["init"]);
                fs::write(repo.join("seed.txt"), "seed").expect("seed");
                run_git(repo, &["add", "--all"]);
                run_git(repo, &["commit", "-m", "initial"]);
                fs::write(repo.join("changed.txt"), format!("{flow_name}-{case}")).expect("change");
                if !commit_succeeds {
                    fs::create_dir_all(repo.join(".git/hooks")).expect("hooks");
                    let hook = repo.join(".git/hooks/pre-commit");
                    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("hook");
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
                            .expect("hook mode");
                    }
                }

                let status = Command::new("sh")
                    .args(["-c", command])
                    .current_dir(repo)
                    .env("GIT_AUTHOR_NAME", "Contracts")
                    .env("GIT_AUTHOR_EMAIL", "contracts@example.com")
                    .env("GIT_COMMITTER_NAME", "Contracts")
                    .env("GIT_COMMITTER_EMAIL", "contracts@example.com")
                    .status()
                    .expect("commit tool");
                assert_eq!(status.success(), commit_succeeds, "{flow_name} case {case}");
            }
        }
    }
}

fn prepare_change_workspace(repo: &Path, run: &str, artifact: &str) -> std::process::Output {
    let flow = bundled_flow("software-development/implement-change");
    let normalize = serde_json::to_value(&flow.nodes["normalize_task"]).unwrap();
    assert!(normalize["contracts"]["writes_context"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("context.task.artifact_path")));
    let config = serde_json::to_value(&flow.nodes["prepare_workspace"].config).unwrap();
    let context = serde_json::json!({
        "internal.run_id": run,
        "context.task.base_ref": "HEAD",
        "context.task.artifact_path": artifact,
    });
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(config["command"].as_str().unwrap())
        .current_dir(repo);
    for (name, key) in config["env_map"].as_object().unwrap() {
        command.env(
            name,
            context[key.as_str().unwrap()]
                .as_str()
                .expect("mapped value"),
        );
    }
    command.output().expect("prepare workspace")
}

fn seed_change_repository(repo: &Path) {
    run_git(repo, &["init"]);
    fs::write(repo.join("seed.txt"), "seed").unwrap();
    run_git(repo, &["add", "seed.txt"]);
    run_git(repo, &["commit", "-m", "initial"]);
}

#[test]
fn change_workspace_carries_only_the_request_and_preserves_retry_content() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path();
    seed_change_repository(repo);
    let artifact = "changes/request with spaces.md";
    fs::create_dir(repo.join("changes")).unwrap();
    fs::write(repo.join(artifact), "approved request\n").unwrap();
    fs::write(repo.join("unrelated.txt"), "unrelated").unwrap();
    let prepare = || prepare_change_workspace(repo, "request-handoff", artifact);
    let output = prepare();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["source_dirty"], true);
    let checkout = Path::new(result["path"].as_str().unwrap());
    assert_eq!(
        fs::read(checkout.join(artifact)).unwrap(),
        b"approved request\n"
    );
    assert!(!checkout.join("unrelated.txt").exists());
    assert!(prepare().status.success());
    fs::write(checkout.join(artifact), "retained checkout request").unwrap();
    let conflict = prepare();
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("conflicts"));
    assert_eq!(
        fs::read(checkout.join(artifact)).unwrap(),
        b"retained checkout request"
    );
    assert_eq!(
        fs::read(repo.join(artifact)).unwrap(),
        b"approved request\n"
    );
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(status.stdout).unwrap(),
        "?? changes/\n?? unrelated.txt\n"
    );

    run_git(repo, &["add", artifact]);
    run_git(repo, &["commit", "-m", "approved request"]);
    assert!(
        prepare_change_workspace(repo, "committed-request", artifact)
            .status
            .success()
    );
    assert!(prepare_change_workspace(repo, "inline-objective", "")
        .status
        .success());
}

#[test]
fn change_workspace_rejects_missing_and_escaping_request_paths() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("project");
    fs::create_dir(&repo).unwrap();
    seed_change_repository(&repo);
    for artifact in [
        "missing.md",
        "../outside.md",
        "/tmp/outside.md",
        "changes/../../outside.md",
        ".git/config",
    ] {
        let output = prepare_change_workspace(&repo, "invalid-request", artifact);
        assert!(!output.status.success(), "{artifact}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("task artifact:"));
        assert!(!repo.join(".spark/checkouts/invalid-request").exists());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(temp.path(), repo.join("linked")).unwrap();
        fs::write(temp.path().join("outside.md"), "outside").unwrap();
        assert!(
            !prepare_change_workspace(&repo, "source-link", "linked/outside.md")
                .status
                .success()
        );
    }
}

#[cfg(unix)]
#[test]
fn change_workspace_rejects_destination_symlinks_without_writing_outside() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("project");
    fs::create_dir(&repo).unwrap();
    seed_change_repository(&repo);
    assert!(prepare_change_workspace(&repo, "destination-link", "")
        .status
        .success());
    fs::create_dir(repo.join("changes")).unwrap();
    fs::write(repo.join("changes/request.md"), "approved request").unwrap();
    let checkout = repo.join(".spark/checkouts/destination-link");
    std::os::unix::fs::symlink(temp.path(), checkout.join("changes")).unwrap();
    let output = prepare_change_workspace(&repo, "destination-link", "changes/request.md");
    assert!(!output.status.success());
    assert!(!temp.path().join("request.md").exists());
    fs::remove_file(checkout.join("changes")).unwrap();
    fs::create_dir(checkout.join("changes")).unwrap();
    std::os::unix::fs::symlink(
        temp.path().join("absent.md"),
        checkout.join("changes/request.md"),
    )
    .unwrap();
    assert!(
        !prepare_change_workspace(&repo, "destination-link", "changes/request.md")
            .status
            .success()
    );
    assert!(!temp.path().join("absent.md").exists());
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=Contracts",
            "-c",
            "user.email=contracts@example.com",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn guide_model_icon_and_provider_template_resources_are_available() {
    assert_eq!(
        guides::guide_names(),
        vec![
            "flow-definition-authoring.md".to_string(),
            "spark-operations.md".to_string()
        ]
    );
    assert!(guides::flow_definition_authoring_guide()
        .expect("flow authoring")
        .text()
        .expect("guide utf-8")
        .contains("type: Flow"));
    assert!(guides::spark_operations_guide()
        .expect("operations")
        .text()
        .expect("guide utf-8")
        .contains("Spark"));

    let catalog = models::model_catalog_resource();
    assert_eq!(catalog.source(), ResourceSource::Packaged);
    let parsed: serde_json::Value =
        serde_json::from_str(models::model_catalog_json()).expect("model catalog json");
    let catalog_entries = parsed["models"].as_array().expect("model catalog array");
    assert!(catalog_entries.len() > 5);
    assert!(catalog_entries.iter().any(|entry| {
        entry["id"] == "gpt-5.2-codex"
            && entry["aliases"]
                .as_array()
                .expect("aliases")
                .iter()
                .any(|alias| alias == "codex")
    }));
    assert!(catalog_entries
        .iter()
        .any(|entry| entry["provider"] == "gemini" && entry["id"] == "gemini-3.1-pro-preview"));

    assert!(icons::favicon().expect("favicon").bytes().len() > 1000);
    assert_eq!(
        icons::root_image_names(),
        vec![
            "spark-app-icon-dark.png".to_string(),
            "spark-symbol-light.png".to_string(),
            "spark-wordmark-dark-orange-symbol.png".to_string(),
            "spark-wordmark-light-mono-symbol.png".to_string(),
            "spark-wordmark-light-orange-symbol.png".to_string(),
        ]
    );
    assert!(
        icons::load_root_image("spark-wordmark-light-orange-symbol.png")
            .expect("root image")
            .bytes()
            .len()
            > 1000
    );

    let template = resources::provider_env_template();
    let text = template.text().expect("provider template utf-8");
    for key in [
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_ORG_ID",
        "OPENAI_PROJECT_ID",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "GEMINI_API_KEY",
        "GEMINI_BASE_URL",
        "GOOGLE_API_KEY",
        "OPENROUTER_API_KEY",
        "OPENROUTER_BASE_URL",
        "OPENROUTER_HTTP_REFERER",
        "OPENROUTER_TITLE",
        "LITELLM_BASE_URL",
        "LITELLM_API_KEY",
        "OPENAI_COMPATIBLE_BASE_URL",
        "OPENAI_COMPATIBLE_API_KEY",
    ] {
        assert!(text.contains(key), "{key}");
    }
    assert!(!text.contains("your_key_here"));
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

#[test]
fn locally_edited_flows_lists_only_edits_that_seeding_keeps() {
    use spark_common::approvals::{
        append_approvals, approval_entries, sha256_hex as hash, EVENT_HUMAN_SAVE, EVENT_INSTALLED,
    };
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path();
    let log = dir.join("config/approvals.jsonl");
    let assets = flows::starter_flow_assets().expect("bundled flows");
    let [same, stale, edited, unknown, ..] = &assets[..] else {
        panic!("at least four bundled flows")
    };
    let write = |name: &str, content: &[u8]| {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    };
    write(&same.name, same.content.as_bytes());
    write(&stale.name, b"older packaged version\n");
    write(&edited.name, b"edited: true\n");
    write(&unknown.name, b"installed before the log existed\n");
    let installed = |name: &String, bytes: &[u8]| (format!("flows/{name}"), hash(bytes));
    append_approvals(
        &log,
        &approval_entries(
            EVENT_INSTALLED,
            "spark/test",
            [
                installed(&same.name, b"anything"),
                installed(&stale.name, b"older packaged version\n"),
                installed(&edited.name, edited.content.as_bytes()),
            ],
        ),
    )
    .unwrap();
    // Only `installed` entries mark what seeding put there.
    append_approvals(
        &log,
        &approval_entries(
            EVENT_HUMAN_SAVE,
            "human:test",
            [installed(&edited.name, b"edited: true\n")],
        ),
    )
    .unwrap();

    // Matching the bundled copy, or the last installed version (updated on the
    // next start), is not an edit; missing files are reinstalled. Edits are
    // reported per flow directory.
    let flow_of = |name: &str| name.rsplit_once('/').unwrap().0.to_string();
    assert_eq!(flow_of(&edited.name), flow_of(&unknown.name));
    assert_eq!(
        flows::locally_edited_flows(dir, &log),
        vec![flow_of(&edited.name)]
    );
}
