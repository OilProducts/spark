use attractor_core::ContextMap;
use attractor_dsl::parse_dot;
use attractor_runtime::{codergen_events_for_journal, RuntimeCodergen};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn runtime_codergen_wrapper_writes_stage_artifacts_and_maps_events() {
    let graph = parse_dot(
        r#"
        digraph G {
          graph [goal="Ship"];
          task [shape=box, prompt="Plan for $goal"];
        }
        "#,
    )
    .expect("dot parses");
    let logs_root = tempdir().unwrap();
    let mut codergen = RuntimeCodergen::simulation(graph, Some(logs_root.path().to_path_buf()));

    let execution = codergen
        .execute(
            "task",
            ContextMap::from([("graph.goal".to_string(), json!("docs"))]),
        )
        .expect("codergen executes");

    assert_eq!(execution.outcome.status.as_str(), "success");
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/prompt.md"))
            .unwrap()
            .trim(),
        "Plan for docs"
    );
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/response.md"))
            .unwrap()
            .trim(),
        "[Simulated] Response for stage: task"
    );

    let events = codergen_events_for_journal("run-1", "task", &execution);
    assert_eq!(events[0].event_type, "CodergenAdapter");
    assert_eq!(events[0].payload["node_id"], json!("task"));
    assert_eq!(
        events[0].payload["adapter_event_type"],
        json!("codergen_backend_request_started")
    );
}
