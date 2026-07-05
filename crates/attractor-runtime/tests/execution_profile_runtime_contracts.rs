use attractor_core::{LaunchContext, Outcome, OutcomeStatus, RunRecord};
use attractor_dsl::parse_dot;
use attractor_runtime::{ExecuteRunRequest, PipelineExecutor, RunStore, RuntimeHandlerRunner};
use serde_json::json;
use spark_storage::read_json;

#[test]
fn runtime_seeds_execution_metadata_into_context_checkpoint_and_run_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_path = temp.path().join("Project");
    std::fs::create_dir_all(&project_path).expect("project");
    let store = RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"));
    let graph = parse_dot(
        r#"
        digraph G {
          start [shape=Mdiamond]
          done [shape=Msquare]
          start -> done
        }
        "#,
    )
    .expect("dot parses");
    let mut record = RunRecord::new("run-execution-profile", project_path.to_string_lossy());
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    record.execution_mode = "local_container".to_string();
    record.execution_profile_id = Some("container".to_string());
    record.execution_container_image = Some("spark-worker:compat".to_string());
    record.execution_profile_capabilities = Some(json!(["filesystem"]));

    let mut executor = PipelineExecutor::new(RuntimeHandlerRunner::new());
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record,
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("pipeline");

    assert_eq!(result.status, "completed");
    assert_eq!(result.context["execution_mode"], json!("local_container"));
    assert_eq!(
        result.context["_attractor.runtime.execution_profile_id"],
        json!("container")
    );
    assert_eq!(
        result.context["_attractor.runtime.execution_container_image"],
        json!("spark-worker:compat")
    );
    assert_eq!(
        result.context["_attractor.runtime.execution_profile_capabilities"],
        json!(["filesystem"])
    );

    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-execution-profile")
        .expect("run root");
    let run_json: serde_json::Value = read_json(paths.run_json()).expect("run json");
    assert_eq!(run_json["execution_mode"], json!("local_container"));
    assert_eq!(run_json["execution_profile_id"], json!("container"));
    assert_eq!(
        run_json["execution_container_image"],
        json!("spark-worker:compat")
    );
    let checkpoint = store
        .read_checkpoint(&paths)
        .expect("checkpoint read")
        .expect("checkpoint");
    assert_eq!(
        checkpoint.context["_attractor.runtime.execution_profile_id"],
        json!("container")
    );
}

#[test]
fn pipeline_persists_node_executor_cleanup_error_without_failing_successful_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_path = temp.path().join("Project Cleanup");
    std::fs::create_dir_all(&project_path).expect("project");
    let store = RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"));
    let graph = parse_dot(
        r#"
        digraph G {
          work [shape=Mdiamond]
          done [shape=Msquare]
          work -> done
        }
        "#,
    )
    .expect("dot parses");
    let mut record = RunRecord::new("run-cleanup-error", project_path.to_string_lossy());
    record.started_at = "2026-06-23T10:00:00Z".to_string();

    let mut executor = PipelineExecutor::new(CleanupExecutor {
        cleanup_error: Some("cleanup denied".to_string()),
    });
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record,
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("pipeline");

    assert_eq!(result.status, "completed");
    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-cleanup-error")
        .expect("run root");
    let run_json: serde_json::Value = read_json(paths.run_json()).expect("run json");
    assert_eq!(run_json["cleanup_error"], json!("cleanup denied"));
    let events = store.read_raw_events(&paths).expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "cleanup_error"
            && event.payload["message"] == json!("cleanup denied")));
}

struct CleanupExecutor {
    cleanup_error: Option<String>,
}

impl attractor_runtime::NodeExecutor for CleanupExecutor {
    fn execute(
        &mut self,
        _request: attractor_runtime::NodeExecutionRequest,
    ) -> Result<Outcome, attractor_runtime::RuntimeNodeError> {
        Ok(Outcome::new(OutcomeStatus::Success))
    }

    fn take_cleanup_error(&mut self) -> Option<String> {
        self.cleanup_error.take()
    }
}
