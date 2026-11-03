use attractor_core::{FlowDefinition, LaunchContext, RunRecord};
use attractor_runtime::{
    ExecuteRunRequest, ExecutionStart, PipelineExecutor, RunStore, RuntimeHandlerRunner,
};
use serde_json::{json, Value};

fn flow() -> FlowDefinition {
    serde_json::from_value(json!({
        "schema_version": "2",
        "id": "failing",
        "title": "Failing",
        "nodes": {
            "start": {
                "kind": "start",
                "routes": { "next": { "to": "review", "description": "Review the change." } }
            },
            "review": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Review the change."
                },
                "contracts": { "response": "decision_envelope" },
                "routes": {
                    "approve": { "to": "done", "description": "The change is correct." },
                    "reject": { "to": "rejected", "description": "The change has a defect; name it in notes." }
                }
            },
            "done": {
                "kind": "exit",
                "config": { "kind": "exit", "result": { "name": "completed", "outcome": "success" } }
            },
            "rejected": {
                "kind": "exit",
                "config": { "kind": "exit", "result": { "name": "rejected", "outcome": "failure" } }
            }
        }
    }))
    .unwrap()
}

/// An agent that follows the decision-envelope contract it is given.
struct ContractFollowingAgent;

impl spark_agent_adapter::CodergenBackend for ContractFollowingAgent {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let asks_for_evidence = request
            .prompt
            .contains(r#"Put the evidence for your decision, including any blocker, in "notes"."#);
        let response = if asks_for_evidence {
            r#"{"decision":"reject","notes":"The change lowercases effort strings."}"#
        } else {
            r#"{"decision":"reject"}"#
        };
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text(response.to_string()),
            events: Vec::new(),
            usage: None,
            resume_metadata: Default::default(),
        })
    }
}

/// A negative judgment is a completed decision, not a failed visit: the
/// agent's evidence travels in its notes to the reached failure exit's result.
#[test]
fn rejecting_agent_visit_carries_the_agents_evidence_to_the_run_result() {
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let runner = RuntimeHandlerRunner::new()
        .with_codergen_backend_factory(|| Box::new(ContractFollowingAgent));
    let result = PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: RunRecord::new("run-fail", project.to_string_lossy()),
            flow: flow().normalize(),
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .unwrap();

    let reason = "The change lowercases effort strings.";
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(result.current_node, "rejected");
    let paths = store
        .run_root(&project.to_string_lossy(), "run-fail")
        .unwrap();
    let record = store.read_run_record(&paths).unwrap().unwrap();
    assert_eq!(record.status, "completed");
    assert_eq!(record.outcome.as_deref(), Some("failure"));
    assert_eq!(record.outcome_reason_code.as_deref(), Some("rejected"));
    assert_eq!(
        record.outcome_reason_message.as_deref(),
        Some(format!("review chose reject: {reason}").as_str())
    );
    let events = store.read_raw_events(&paths).unwrap();
    let completed = events
        .iter()
        .find(|event| event.event_type == "StageCompleted" && event.payload["node_id"] == "review")
        .expect("StageCompleted event");
    assert_eq!(completed.payload["decision"], "reject");
    assert_eq!(completed.payload["notes"], Value::from(reason));
    assert!(!events.iter().any(|event| event.event_type == "StageFailed"));
    let status: Value = serde_json::from_str(
        &std::fs::read_to_string(
            store
                .node_execution_root(&paths, "review", 1, 0)
                .unwrap()
                .join("status.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(status["status"], "completed");
    assert_eq!(status["decision"], "reject");
    assert_eq!(status["notes"], reason);
    assert!(status.get("outcome").is_none(), "{status}");
}
