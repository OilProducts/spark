use std::path::Path;

use attractor_core::RunRecord;
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::json;
use spark_common::settings::SparkSettings;
use spark_workspace::live::run_segment_envelopes_after;

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
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

fn adapter_event(sequence_hint: u64, node_id: &str, text: &str) -> attractor_core::RawRuntimeEvent {
    serde_json::from_value(json!({
        "type": "CodergenAdapter",
        "run_id": "run-envelopes",
        "emitted_at": format!("2026-07-08T12:00:{:02}.000000000Z", sequence_hint),
        "adapter_event_type": "rust_agent_session_event",
        "node_id": node_id,
        "payload": {"turn_stream_event": {
            "kind": "content_completed",
            "channel": "assistant",
            "content_delta": text,
            "message": text,
            "source": {"backend": "rust_unified_llm_adapter"},
        }},
    }))
    .expect("raw event")
}

#[test]
fn segment_envelopes_filter_by_combined_journal_cursor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    std::fs::create_dir_all(&project_path).expect("project");
    let store = RunStore::for_settings(&settings);
    let mut record = RunRecord::new("run-envelopes", project_path.to_string_lossy().to_string());
    record.status = "running".to_string();
    let paths = store
        .create_run(CreateRunRequest {
            record,
            ..CreateRunRequest::default()
        })
        .expect("run");

    store
        .append_event(&paths, adapter_event(1, "first", "First answer."))
        .expect("first event");
    let first_batch =
        run_segment_envelopes_after(&settings, "run-envelopes", 0).expect("envelopes");
    assert_eq!(first_batch.len(), 1);
    let first_sequence = first_batch[0].cursor.as_ref().expect("cursor").value as u64;
    assert_eq!(
        first_batch[0].payload["segment"]["content"],
        "First answer."
    );

    store
        .append_event(&paths, adapter_event(2, "second", "Second answer."))
        .expect("second event");

    // After the first segment's cursor: only the newly touched segment.
    let delta =
        run_segment_envelopes_after(&settings, "run-envelopes", first_sequence).expect("envelopes");
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0].payload["segment"]["node_id"], "second");
    assert_eq!(delta[0].event_type, "run.segment_upsert");

    // Past the newest sequence: nothing to publish.
    let newest = delta[0].cursor.as_ref().expect("cursor").value as u64;
    let empty = run_segment_envelopes_after(&settings, "run-envelopes", newest).expect("envelopes");
    assert!(empty.is_empty());
}
