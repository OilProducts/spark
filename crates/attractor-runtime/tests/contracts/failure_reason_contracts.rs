use attractor_core::{FlowDefinition, LaunchContext, RunRecord};
use attractor_runtime::{
    ExecuteRunRequest, ExecutionStart, PipelineExecutor, RunStore, RuntimeHandlerRunner,
};
use serde_json::Value;

const FLOW: &str = r#"schema_version: "1"
id: failing
title: Failing
nodes:
  start: {kind: start}
  review:
    kind: agent_task
    config: {kind: agent_task, prompt: Review the change.}
    contracts: {response: status_envelope}
  done: {kind: exit}
edges:
- {from: start, to: review}
- {from: review, to: done}
"#;

/// An agent that follows the structured-response contract it is given.
struct ContractFollowingAgent;

impl spark_agent_adapter::CodergenBackend for ContractFollowingAgent {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let asks_for_reason = request
            .prompt
            .contains(r#"Whenever "outcome" is "fail", set "failure_reason" to one sentence"#);
        let response = if asks_for_reason {
            r#"{"outcome":"fail","failure_reason":"The change lowercases effort strings."}"#
        } else {
            r#"{"outcome":"fail"}"#
        };
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text(response.to_string()),
            events: Vec::new(),
            usage: None,
        })
    }
}

#[test]
fn failed_agent_visit_carries_the_agents_reason() {
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let runner = RuntimeHandlerRunner::new()
        .with_codergen_backend_factory(|| Box::new(ContractFollowingAgent));
    PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: RunRecord::new("run-fail", project.to_string_lossy()),
            flow: FlowDefinition::from_yaml_str(FLOW).unwrap().normalize(),
            flow_source: None,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .unwrap();

    let reason = "The change lowercases effort strings.";
    let paths = store
        .run_root(&project.to_string_lossy(), "run-fail")
        .unwrap();
    let failed = store
        .read_raw_events(&paths)
        .unwrap()
        .into_iter()
        .find(|event| event.event_type == "StageFailed")
        .expect("StageFailed event");
    assert_eq!(failed.payload["error"], Value::from(reason));
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
    assert_eq!(status["outcome"], "fail");
    assert_eq!(status["failure_reason"], reason);
}
