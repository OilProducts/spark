use std::fs;
use std::path::Path;

use attractor_api::{AttractorApiService, PipelineStartRequest};
use attractor_runtime::RunStore;
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
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

/// Tool-only replica of implement-change's validate fix loop: a deterministic
/// gate whose failure routes through a bounded counter and a diagnosis stage
/// back to the implementer, with the counter failing the run once the fix
/// budget is spent.
fn fix_loop_flow(gate_command: &str) -> serde_json::Value {
    json!({
        "schema_version": "1",
        "id": "validate_fix_loop",
        "title": "Validate Fix Loop",
        "nodes": {
            "start": {"kind": "start"},
            "prepare": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "printf '{\"validation_attempts\":0}'",
                    "output_map": {
                        "context.validation.attempts": "validation_attempts"
                    }
                },
                "contracts": {"writes_context": ["context.validation.attempts"]}
            },
            "implement": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "echo attempt >> attempts.txt"
                }
            },
            "validate": {
                "kind": "tool",
                "config": {"kind": "tool", "command": gate_command}
            },
            "count_validate_attempt": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "set -eu\nattempts=$((SPARK_VALIDATION_ATTEMPTS + 1))\nif [ \"$attempts\" -gt 3 ]; then\n  echo \"validation gate failed ${attempts} times; fix budget (3 attempts) exhausted\" >&2\n  exit 1\nfi\nprintf '{\"attempts\":%s}' \"$attempts\"\n",
                    "env_map": {
                        "SPARK_VALIDATION_ATTEMPTS": "context.validation.attempts"
                    },
                    "output_map": {"context.validation.attempts": "attempts"}
                },
                "contracts": {"writes_context": ["context.validation.attempts"]}
            },
            "diagnose_validation": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "printf '{\"summary\":\"gate red\"}'",
                    "output_map": {"context.review.summary": "summary"}
                },
                "contracts": {"writes_context": ["context.review.summary"]}
            },
            "done": {"kind": "exit"}
        },
        "edges": [
            {"from": "start", "to": "prepare"},
            {
                "from": "prepare",
                "to": "implement",
                "condition": "outcome=success"
            },
            {
                "from": "implement",
                "to": "validate",
                "condition": "outcome=success"
            },
            {
                "from": "validate",
                "to": "done",
                "condition": "outcome=success"
            },
            {
                "from": "validate",
                "to": "count_validate_attempt",
                "condition": "outcome=fail"
            },
            {
                "from": "count_validate_attempt",
                "to": "diagnose_validation",
                "condition": "outcome=success"
            },
            {
                "from": "diagnose_validation",
                "to": "implement",
                "condition": "outcome=success"
            }
        ]
    })
}

fn run_fix_loop_flow(gate_command: &str, run_id: &str) -> (serde_json::Value, SparkSettings) {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");

    let service = AttractorApiService::new(settings.clone());
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some(run_id.to_string()),
        flow_name: Some(crate::install_flow(&settings, fix_loop_flow(gate_command))),
        working_directory: project.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    // Keep the tempdir alive through assertions by leaking it into the path
    // set; contract tempdirs are process-scoped.
    std::mem::forget(temp);
    (response.body, settings)
}

#[test]
fn validate_failure_routes_through_diagnosis_back_to_implement() {
    // The gate fails until the implementer has run twice.
    let (body, settings) = run_fix_loop_flow(
        "test \"$(wc -l < attempts.txt)\" -ge 2",
        "run-validate-fix-loop",
    );
    assert_eq!(body["terminal_status"], json!("completed"), "{body:?}");

    let store = RunStore::for_settings(&settings);
    let bundle = store
        .read_run_bundle("run-validate-fix-loop")
        .expect("bundle")
        .expect("bundle exists");
    let checkpoint = store
        .read_checkpoint(&bundle.paths)
        .expect("checkpoint")
        .expect("checkpoint exists");
    assert_eq!(
        checkpoint.context.get("context.validation.attempts"),
        Some(&json!(1)),
        "exactly one gate failure should be counted",
    );
    assert_eq!(
        checkpoint.context.get("context.review.summary"),
        Some(&json!("gate red")),
        "diagnosis must land in the review feedback the implementer reads",
    );
}

#[test]
fn validate_fix_budget_exhaustion_fails_the_run() {
    let (body, settings) = run_fix_loop_flow("exit 1", "run-validate-budget");
    assert_eq!(body["terminal_status"], json!("failed"), "{body:?}");

    let store = RunStore::for_settings(&settings);
    let bundle = store
        .read_run_bundle("run-validate-budget")
        .expect("bundle")
        .expect("bundle exists");
    let record = bundle.record.expect("record");
    assert!(
        record
            .last_error
            .contains("fix budget (3 attempts) exhausted"),
        "budget exhaustion must be the recorded failure: {}",
        record.last_error,
    );
    let checkpoint = store
        .read_checkpoint(&bundle.paths)
        .expect("checkpoint")
        .expect("checkpoint exists");
    assert_eq!(
        checkpoint.context.get("context.validation.attempts"),
        Some(&json!(3)),
        "the counter stops at the budget",
    );
}

/// Every packaged software-development flow must survive the real DSL
/// pipeline (parse, transforms, validation) without error diagnostics.
#[test]
fn packaged_software_development_flows_validate_cleanly() {
    let flows = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spark-assets/assets/flows/software-development");
    let mut seen = 0;
    let mut stack = vec![flows.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read flows dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            }
        }
        if !dir.join("flow.md").is_file() {
            continue;
        }
        let name = dir
            .strip_prefix(flows.parent().expect("flows root"))
            .expect("relative")
            .to_str()
            .expect("utf8");
        // The packaged assets are laid out like the bundled library.
        let roots = attractor_dsl::FlowRoots::new(flows.join("../.."), None);
        let preview = attractor_api::preview_with_roots(
            attractor_api::PreviewRequest {
                flow_name: Some(name.to_string()),
                ..Default::default()
            },
            &roots,
        );
        assert_eq!(
            (preview.status_code, &preview.body["status"]),
            (200, &json!("ok")),
            "{name} failed validation: {:?}",
            preview.body,
        );
        seen += 1;
    }
    assert!(seen >= 10, "expected the packaged flow library, saw {seen}");
}
