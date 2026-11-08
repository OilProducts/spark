//! Sandbox runs of the bundled JSON-mode tool scripts.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use spark_assets::flows;

fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').expect("flow/node")
}

/// A bundled flow's canonical JSON form, assembled like the loader does.
fn bundled_flow(name: &str) -> Value {
    let files = flows::starter_flow_files(name).expect("bundled flow files");
    attractor_dsl::assemble_flow_files(&files)
        .unwrap_or_else(|error| panic!("{name} does not assemble: {}", error.detail()))
        .to_canonical_json_value()
}

/// The tool's command run with `sh -c` in `dir`, each env_map variable set
/// from `context` (an unmapped variable is a test bug).
fn run_tool(path: &str, dir: &Path, context: &[(&str, &str)]) -> Output {
    let (flow, node) = split(path);
    let config = &bundled_flow(flow)["nodes"][node]["config"];
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(config["command"].as_str().expect("tool command"))
        .current_dir(dir);
    for (variable, key) in config["env_map"].as_object().into_iter().flatten() {
        let value = context
            .iter()
            .find(|(k, _)| *k == key.as_str().unwrap())
            .unwrap_or_else(|| panic!("{path}: no context value for {key}"))
            .1;
        command.env(variable, value);
    }
    command.output().expect("spawn tool")
}

/// The single JSON object a JSON-mode tool printed at exit 0; its decision
/// is one of the node's declared routes.
fn decision_output(path: &str, output: &Output) -> Value {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{path}: {:?} {stderr}",
        output.status
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{path}: stdout is not one JSON value: {error}"));
    let (flow, node) = split(path);
    let routes = &bundled_flow(flow)["nodes"][node]["routes"];
    let decision = value["decision"].as_str().expect("decision field");
    assert!(
        routes.get(decision).is_some(),
        "{path}: undeclared {decision}"
    );
    value
}

/// A nonzero exit with no domain decision on stdout: an execution error.
fn assert_execution_error(path: &str, output: &Output) {
    assert!(!output.status.success(), "{path}: expected a nonzero exit");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("decision"),
        "{path}: an execution error printed a decision"
    );
}

fn have(tools: &[&str], test: &str) -> bool {
    for tool in tools {
        let found = Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .output()
            .is_ok_and(|output| output.status.success());
        if !found {
            eprintln!("skipping {test}: {tool} is not installed");
            return false;
        }
    }
    true
}

#[test]
fn repair_budget_allows_one_repair_then_exhausts() {
    const TOOL: &str = "math-research/research-program/repair_budget";
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::create_dir(dir.join(".math-research")).unwrap();
    std::fs::write(dir.join(".math-research/current-action"), "A0001\n").unwrap();
    let first = decision_output(TOOL, &run_tool(TOOL, dir, &[]));
    assert_eq!(first, json!({"decision": "allowed"}));
    assert_eq!(
        std::fs::read_to_string(dir.join(".math-research/repair-used")).unwrap(),
        "A0001\n"
    );
    let second = decision_output(TOOL, &run_tool(TOOL, dir, &[]));
    assert_eq!(second, json!({"decision": "exhausted"}));

    // A marker that is not a readable file is an operating failure, not exhaustion.
    std::fs::remove_file(dir.join(".math-research/repair-used")).unwrap();
    std::fs::create_dir(dir.join(".math-research/repair-used")).unwrap();
    assert_execution_error(TOOL, &run_tool(TOOL, dir, &[]));

    for action in ["bogus\n", ""] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".math-research")).unwrap();
        std::fs::write(temp.path().join(".math-research/current-action"), action).unwrap();
        assert_execution_error(TOOL, &run_tool(TOOL, temp.path(), &[]));
        assert!(!temp.path().join(".math-research/repair-used").exists());
    }
}

#[test]
fn round_gate_records_rounds_until_the_budget_is_exhausted() {
    const TOOL: &str = "math-research/prove-refute/round_gate";
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    // Defaults: round 0 of 3.
    for (round, decision) in [(1, "more"), (2, "more"), (3, "exhausted")] {
        let output = decision_output(TOOL, &run_tool(TOOL, dir, &[]));
        assert_eq!(output, json!({"decision": decision}), "round {round}");
        assert_eq!(
            std::fs::read_to_string(dir.join(".mathlab/round")).unwrap(),
            format!("{round}\n")
        );
    }
    std::fs::write(dir.join(".mathlab/round"), "0\n").unwrap();
    std::fs::write(dir.join(".mathlab/max-rounds"), "1\n").unwrap();
    assert_eq!(
        decision_output(TOOL, &run_tool(TOOL, dir, &[]))["decision"],
        "exhausted"
    );

    for (round, max) in [("x\n", "3\n"), ("1\n", "0\n"), ("-1\n", "3\n")] {
        std::fs::write(dir.join(".mathlab/round"), round).unwrap();
        std::fs::write(dir.join(".mathlab/max-rounds"), max).unwrap();
        assert_execution_error(TOOL, &run_tool(TOOL, dir, &[]));
        assert_eq!(
            std::fs::read_to_string(dir.join(".mathlab/round")).unwrap(),
            round
        );
    }
}

#[test]
fn research_verify_rejects_missing_evidence_and_passes_unverifiable_claims() {
    const TOOL: &str = "math-research/research-program/verify";
    if !have(&["bash", "jq"], "research_verify") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let evidence = dir.join("evidence/A0007");
    std::fs::create_dir_all(dir.join(".math-research")).unwrap();
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(dir.join(".math-research/current-action"), "A0007\n").unwrap();
    let verify = || decision_output(TOOL, &run_tool(TOOL, dir, &[]))["decision"].clone();

    assert_eq!(verify(), "rejected");
    assert!(std::fs::read_to_string(evidence.join("verification.log"))
        .unwrap()
        .contains("claim.json is missing"));
    std::fs::write(evidence.join("claim.json"), "[]").unwrap();
    assert_eq!(verify(), "rejected");

    std::fs::write(evidence.join("claim.json"), "{}").unwrap();
    assert_eq!(verify(), "passed");
    assert!(evidence.join(".not-machine-checkable").exists());

    std::fs::write(evidence.join("verify.sh"), "echo checking; exit 1\n").unwrap();
    assert_eq!(verify(), "rejected");
    assert!(!evidence.join(".not-machine-checkable").exists());
    assert!(std::fs::read_to_string(evidence.join("verification.log"))
        .unwrap()
        .contains("checking"));
    std::fs::write(evidence.join("verify.sh"), "exit 0\n").unwrap();
    assert_eq!(verify(), "passed");
    assert!(evidence.join(".verified").exists());

    std::fs::write(dir.join(".math-research/current-action"), "nope\n").unwrap();
    assert_execution_error(TOOL, &run_tool(TOOL, dir, &[]));
}

#[test]
fn implement_change_validate_reports_the_wrapped_command_status_as_data() {
    const TOOL: &str = "software-development/implement-change/validate";
    if !have(&["jq"], "implement_change_validate") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().to_str().unwrap();
    let run = |command: &str| {
        run_tool(
            TOOL,
            temp.path(),
            &[
                ("context.task.validation_command", command),
                ("context.workspace.path", workspace),
            ],
        )
    };
    let passed = decision_output(TOOL, &run("true"));
    assert_eq!(
        passed,
        json!({"decision": "passed", "command_exit_code": 0, "command_output": ""})
    );

    let rejected = decision_output(TOOL, &run("echo 'quote \" and\ttab'; exit 3"));
    assert_eq!(rejected["decision"], "rejected");
    assert_eq!(rejected["command_exit_code"], 3);
    assert_eq!(rejected["command_output"], "quote \" and\ttab\n");

    // Every output_map source is present in the decision object.
    let config =
        &bundled_flow("software-development/implement-change")["nodes"]["validate"]["config"];
    for source in config["output_map"].as_object().unwrap().values() {
        assert!(rejected.get(source.as_str().unwrap()).is_some(), "{source}");
    }

    // A missing workspace is the wrapper's own failure, not a rejection.
    let missing = temp.path().join("missing");
    let output = run_tool(
        TOOL,
        temp.path(),
        &[
            ("context.task.validation_command", "true"),
            ("context.workspace.path", missing.to_str().unwrap()),
        ],
    );
    assert_execution_error(TOOL, &output);
}
