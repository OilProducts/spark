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
/// gate whose failure routes through a diagnosis stage back to the
/// implementer. The diagnosis's visit limit ends the loop once the fix budget
/// is spent: the gate's Fix route then has no eligible target.
fn fix_loop_flow(gate_command: &str) -> serde_json::Value {
    json!({
        "schema_version": "1",
        "id": "validate_fix_loop",
        "title": "Validate Fix Loop",
        "nodes": {
            "start": {"kind": "start"},
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
            "diagnose_validation": {
                "kind": "tool",
                "config": {
                    "kind": "tool",
                    "command": "printf '{\"summary\":\"diagnosed: %s\"}' \"$SPARK_GATE_OUTPUT\"",
                    "env_map": {"SPARK_GATE_OUTPUT": "context.tool.output"},
                    "output_map": {"context.review.summary": "summary"}
                },
                "contracts": {"writes_context": ["context.review.summary"]},
                "runtime": {"max_visits": 3}
            },
            "done": {"kind": "exit"}
        },
        "edges": [
            {"from": "start", "to": "implement"},
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
                "to": "diagnose_validation",
                "label": "Fix",
                "condition": "outcome=fail"
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

fn final_checkpoint(settings: &SparkSettings, run_id: &str) -> attractor_core::CheckpointState {
    let store = RunStore::for_settings(settings);
    let bundle = store
        .read_run_bundle(run_id)
        .expect("bundle")
        .expect("bundle exists");
    store
        .read_checkpoint(&bundle.paths)
        .expect("checkpoint")
        .expect("checkpoint exists")
}

const GATE_LOG: &str = "echo \"gate red after $(wc -l < attempts.txt) attempts\"";

#[test]
fn validate_failure_routes_through_diagnosis_back_to_implement() {
    // The gate fails until the implementer has run twice.
    let (body, settings) = run_fix_loop_flow(
        &format!("{GATE_LOG}\ntest \"$(wc -l < attempts.txt)\" -ge 2"),
        "run-validate-fix-loop",
    );
    assert_eq!(body["terminal_status"], json!("completed"), "{body:?}");

    let checkpoint = final_checkpoint(&settings, "run-validate-fix-loop");
    assert_eq!(checkpoint.visit_counts["diagnose_validation"], 1);
    assert_eq!(
        checkpoint.context.get("context.review.summary"),
        Some(&json!("diagnosed: gate red after 1 attempts")),
        "the diagnosis reads the gate's own failure output",
    );
}

#[test]
fn validate_fix_budget_exhaustion_fails_the_run() {
    let (body, settings) = run_fix_loop_flow(&format!("{GATE_LOG}\nexit 1"), "run-validate-budget");
    assert_eq!(body["terminal_status"], json!("failed"), "{body:?}");

    let record = RunStore::for_settings(&settings)
        .read_run_bundle("run-validate-budget")
        .expect("bundle")
        .expect("bundle exists")
        .record
        .expect("record");
    assert_eq!(
        record.last_error,
        "visit_limit_reached: diagnose_validation used all 3 visits; no other route from validate",
    );
    // Three fixes after the first attempt; the fourth gate failure ends it.
    let checkpoint = final_checkpoint(&settings, "run-validate-budget");
    assert_eq!(checkpoint.visit_counts["implement"], 4);
    assert_eq!(
        checkpoint.context.get("context.review.summary"),
        Some(&json!("diagnosed: gate red after 3 attempts")),
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

/// Tool-only replica of implement-change's review loop: the child `work`
/// node contests a finding through its write contract, the parent's subflow
/// `output_map` carries it to `evaluate`, and `implement`'s visit limit ends
/// a loop that never converges.
#[test]
fn contest_reaches_evaluate_through_output_map_and_visit_limit_ends_the_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    let child = crate::install_flow(
        &settings,
        json!({
            "schema_version": "1",
            "id": "contest_child",
            "title": "Contest Child",
            "nodes": {
                "start": {"kind": "start"},
                "work": {
                    "kind": "tool",
                    "config": {
                        "kind": "tool",
                        "command": "echo attempt >> \"$SPARK_PROJECT/attempts.txt\"\nprintf '{\"contested\":\"finding 1: see src/lib.rs:%s\"}' \"$(wc -l < \"$SPARK_PROJECT/attempts.txt\")\"",
                        "env_map": {"SPARK_PROJECT": "context.project"},
                        "output_map": {"context.review.contested": "contested"}
                    },
                    "contracts": {"writes_context": ["context.review.contested"]}
                },
                "done": {"kind": "exit"}
            },
            "edges": [
                {"from": "start", "to": "work"},
                {"from": "work", "to": "done", "condition": "outcome=success"}
            ]
        }),
    );
    assert_eq!(child, "tests/contest-child");
    let parent = crate::install_flow(
        &settings,
        json!({
            "schema_version": "1",
            "id": "contest_parent",
            "title": "Contest Parent",
            "nodes": {
                "start": {"kind": "start"},
                "implement": {
                    "kind": "subflow",
                    "config": {
                        "kind": "subflow",
                        "flow_ref": "contest-child",
                        "input_map": {"context.project": "context.project"},
                        "output_map": {"context.review.contested": "context.review.contested"}
                    },
                    "contracts": {"writes_context": ["context.review.contested"]},
                    "runtime": {"max_visits": 5}
                },
                "evaluate": {
                    "kind": "tool",
                    "config": {
                        "kind": "tool",
                        "command": "echo \"$SPARK_CONTESTED\" >> \"$SPARK_PROJECT/evaluate.txt\"\nprintf '{\"required_changes\":\"uphold finding 1 (review %s)\"}' \"$(wc -l < \"$SPARK_PROJECT/evaluate.txt\")\"",
                        "env_map": {
                            "SPARK_CONTESTED": "context.review.contested",
                            "SPARK_PROJECT": "context.project"
                        },
                        "output_map": {"context.review.required_changes": "required_changes"}
                    },
                    "contracts": {
                        "reads_context": ["context.review.contested"],
                        "writes_context": ["context.review.required_changes"]
                    }
                },
                "done": {"kind": "exit"}
            },
            "edges": [
                {"from": "start", "to": "implement"},
                {"from": "implement", "to": "evaluate", "condition": "outcome=success"},
                // The reviewer always upholds, so the loop never converges.
                {"from": "evaluate", "to": "implement", "condition": "outcome=success"},
                {"from": "evaluate", "to": "done", "condition": "outcome=fail"}
            ]
        }),
    );

    let service = AttractorApiService::new(settings.clone());
    let response = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-contest-loop".to_string()),
        flow_name: Some(parent),
        working_directory: project.to_string_lossy().to_string(),
        launch_context: Some(
            [(
                "context.project".to_string(),
                json!(project.to_string_lossy()),
            )]
            .into_iter()
            .collect(),
        ),
        ..PipelineStartRequest::default()
    });
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    assert_eq!(
        response.body["terminal_status"],
        json!("failed"),
        "{:?}",
        response.body
    );

    let reviewed = fs::read_to_string(project.join("evaluate.txt")).expect("evaluate ran");
    assert_eq!(
        reviewed.lines().collect::<Vec<_>>(),
        (1..=5)
            .map(|n| format!("finding 1: see src/lib.rs:{n}"))
            .collect::<Vec<_>>(),
        "each evaluate must see the contest its preceding child work wrote",
    );

    let store = RunStore::for_settings(&settings);
    let bundle = store
        .read_run_bundle("run-contest-loop")
        .expect("bundle")
        .expect("bundle exists");
    let record = bundle.record.expect("record");
    assert_eq!(
        record.last_error,
        "visit_limit_reached: implement used all 5 visits; no other route from evaluate",
    );
    let checkpoint = store
        .read_checkpoint(&bundle.paths)
        .expect("checkpoint")
        .expect("checkpoint exists");
    assert_eq!(
        checkpoint.context.get("context.review.required_changes"),
        Some(&json!("uphold finding 1 (review 5)")),
        "the last required changes stay as the open items",
    );
}
