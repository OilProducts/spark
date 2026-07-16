use std::fs;
use std::path::Path;

use attractor_api::{AttractorApiService, PipelineStartRequest};
use attractor_runtime::RunStore;
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

/// Tool-only replica of implement-change's validate fix loop: a deterministic
/// gate whose failure routes through a bounded counter and a diagnosis stage
/// back to the implementer, with the counter failing the run once the fix
/// budget is spent.
fn fix_loop_flow(gate_command: &str) -> String {
    format!(
        concat!(
            "schema_version: '1'\n",
            "id: validate_fix_loop\n",
            "title: Validate Fix Loop\n",
            "nodes:\n",
            "  start:\n",
            "    kind: start\n",
            "  prepare:\n",
            "    kind: tool\n",
            "    config:\n",
            "      kind: tool\n",
            "      command: printf '{{\"validation_attempts\":0}}'\n",
            "      output_map:\n",
            "        context.validation.attempts: validation_attempts\n",
            "    contracts:\n",
            "      writes_context:\n",
            "      - context.validation.attempts\n",
            "  implement:\n",
            "    kind: tool\n",
            "    config:\n",
            "      kind: tool\n",
            "      command: echo attempt >> attempts.txt\n",
            "  validate:\n",
            "    kind: tool\n",
            "    config:\n",
            "      kind: tool\n",
            "      command: {gate}\n",
            "  count_validate_attempt:\n",
            "    kind: tool\n",
            "    config:\n",
            "      kind: tool\n",
            "      command: |\n",
            "        set -eu\n",
            "        attempts=$((SPARK_VALIDATION_ATTEMPTS + 1))\n",
            "        if [ \"$attempts\" -gt 3 ]; then\n",
            "          echo \"validation gate failed ${{attempts}} times; fix budget (3 attempts) exhausted\" >&2\n",
            "          exit 1\n",
            "        fi\n",
            "        printf '{{\"attempts\":%s}}' \"$attempts\"\n",
            "      env_map:\n",
            "        SPARK_VALIDATION_ATTEMPTS: context.validation.attempts\n",
            "      output_map:\n",
            "        context.validation.attempts: attempts\n",
            "    contracts:\n",
            "      writes_context:\n",
            "      - context.validation.attempts\n",
            "  diagnose_validation:\n",
            "    kind: tool\n",
            "    config:\n",
            "      kind: tool\n",
            "      command: printf '{{\"summary\":\"gate red\"}}'\n",
            "      output_map:\n",
            "        context.review.summary: summary\n",
            "    contracts:\n",
            "      writes_context:\n",
            "      - context.review.summary\n",
            "  done:\n",
            "    kind: exit\n",
            "edges:\n",
            "- from: start\n",
            "  to: prepare\n",
            "- from: prepare\n",
            "  to: implement\n",
            "  condition: outcome=success\n",
            "- from: implement\n",
            "  to: validate\n",
            "  condition: outcome=success\n",
            "- from: validate\n",
            "  to: done\n",
            "  condition: outcome=success\n",
            "- from: validate\n",
            "  to: count_validate_attempt\n",
            "  condition: outcome=fail\n",
            "- from: count_validate_attempt\n",
            "  to: diagnose_validation\n",
            "  condition: outcome=success\n",
            "- from: diagnose_validation\n",
            "  to: implement\n",
            "  condition: outcome=success\n",
        ),
        gate = gate_command,
    )
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
        flow_content: Some(fix_loop_flow(gate_command)),
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
    let mut stack = vec![flows];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read flows dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                continue;
            }
            let source = fs::read_to_string(&path).expect("read flow");
            let preview = attractor_api::preview_named_flow_source(
                path.parent().expect("parent"),
                path.file_name().expect("name").to_str().expect("utf8"),
                &source,
            );
            assert_eq!(
                preview.status_code,
                200,
                "{} failed validation: {:?}",
                path.display(),
                preview.body,
            );
            seen += 1;
        }
    }
    assert!(seen >= 10, "expected the packaged flow library, saw {seen}");
}
