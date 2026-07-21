use std::collections::BTreeMap;

use attractor_core::{CheckpointState, RawRuntimeEvent, RunRecord};
use attractor_runtime::{
    combined_journal_window, combined_run_journal_entries, log_event, CreateRunRequest,
    RunStore,
};

fn store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"))
}

fn record(run_id: &str, project_path: &str, parent: Option<&str>) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path);
    record.flow_name = format!("{run_id}-flow");
    record.started_at = "2026-07-21T10:00:00Z".to_string();
    record.parent_run_id = parent.map(str::to_string);
    record
}

fn checkpoint() -> CheckpointState {
    CheckpointState {
        timestamp: "2026-07-21T10:00:00Z".to_string(),
        current_node: "start".to_string(),
        completed_nodes: Vec::new(),
        context: BTreeMap::new(),
        retry_counts: Default::default(),
        logs: Vec::new(),
    }
}

fn stamped_log(run_id: &str, message: &str, emitted_at: &str) -> RawRuntimeEvent {
    let mut event = log_event(run_id, message);
    event.emitted_at = emitted_at.to_string();
    event
}

fn assert_window_matches_cold(store: &RunStore, run_id: &str) {
    let cold = combined_run_journal_entries(store, run_id)
        .expect("cold rebuild")
        .expect("run exists");
    let window = combined_journal_window(store, run_id, 0)
        .expect("cached window")
        .expect("run exists");
    assert!(window.complete, "cursor 0 must be inside the retained ring");
    assert_eq!(
        window.latest_sequence,
        cold.iter().map(|entry| entry.sequence).max().unwrap_or(0)
    );
    assert_eq!(window.entries_after.len(), cold.len());
    for (cached, cold) in window.entries_after.iter().zip(cold.iter()) {
        assert_eq!(cached.sequence, cold.sequence);
        assert_eq!(cached.id, cold.id);
        assert_eq!(cached.emitted_at, cold.emitted_at);
        assert_eq!(cached.summary, cold.summary);
        assert_eq!(cached.source_scope, cold.source_scope);
        assert_eq!(cached.payload, cold.payload);
    }
}

#[test]
fn cached_windows_match_cold_rebuilds_across_incremental_appends() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("project dir");
    let parent_paths = store
        .create_run(CreateRunRequest {
            record: record("run-cache-parent", &project.to_string_lossy(), None),
            checkpoint: Some(checkpoint()),
            ..CreateRunRequest::default()
        })
        .expect("parent run");

    assert_window_matches_cold(&store, "run-cache-parent");

    // Incremental append on the parent: only the delta is parsed, and the
    // window keeps matching the cold rebuild.
    store
        .append_event(
            &parent_paths,
            stamped_log("run-cache-parent", "[work] step one", "2026-07-21T10:00:05Z"),
        )
        .expect("append");
    assert_window_matches_cold(&store, "run-cache-parent");

    // A child appears (its record announces the parent) with later events.
    let child_paths = store
        .create_run(CreateRunRequest {
            record: record(
                "run-cache-child",
                &project.to_string_lossy(),
                Some("run-cache-parent"),
            ),
            checkpoint: Some(checkpoint()),
            ..CreateRunRequest::default()
        })
        .expect("child run");
    store
        .append_event(
            &child_paths,
            stamped_log("run-cache-child", "[child] hello", "2026-07-21T10:00:10Z"),
        )
        .expect("child append");
    assert_window_matches_cold(&store, "run-cache-parent");

    // Interleaved appends on both sources still converge.
    store
        .append_event(
            &parent_paths,
            stamped_log("run-cache-parent", "[work] step two", "2026-07-21T10:00:15Z"),
        )
        .expect("append");
    store
        .append_event(
            &child_paths,
            stamped_log("run-cache-child", "[child] again", "2026-07-21T10:00:20Z"),
        )
        .expect("child append");
    assert_window_matches_cold(&store, "run-cache-parent");

    // Cursor windows return exactly the delta.
    let full = combined_journal_window(&store, "run-cache-parent", 0)
        .expect("window")
        .expect("run exists");
    let after = full.latest_sequence - 2;
    let delta = combined_journal_window(&store, "run-cache-parent", after)
        .expect("window")
        .expect("run exists");
    assert!(delta.complete);
    assert_eq!(delta.entries_after.len(), 2);
    assert_eq!(delta.entries_after[0].sequence, after + 1);
    assert_eq!(delta.entries_after[1].sequence, after + 2);
}

#[test]
fn out_of_order_child_append_rebuilds_instead_of_renumbering_silently() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("project dir");
    let parent_paths = store
        .create_run(CreateRunRequest {
            record: record("run-cache-ooo", &project.to_string_lossy(), None),
            checkpoint: Some(checkpoint()),
            ..CreateRunRequest::default()
        })
        .expect("parent run");
    store
        .append_event(
            &parent_paths,
            stamped_log("run-cache-ooo", "[work] late parent", "2026-07-21T11:00:00Z"),
        )
        .expect("append");
    // Prime the cache.
    assert_window_matches_cold(&store, "run-cache-ooo");

    // A child whose first event carries an EARLIER clock than entries the
    // cache already stamped: extending in place would renumber history, so
    // the window must come from a rebuild — and still match cold exactly.
    let child_paths = store
        .create_run(CreateRunRequest {
            record: record(
                "run-cache-ooo-child",
                &project.to_string_lossy(),
                Some("run-cache-ooo"),
            ),
            checkpoint: Some(checkpoint()),
            ..CreateRunRequest::default()
        })
        .expect("child run");
    store
        .append_event(
            &child_paths,
            stamped_log(
                "run-cache-ooo-child",
                "[child] from the past",
                "2026-07-21T09:00:00Z",
            ),
        )
        .expect("child append");
    assert_window_matches_cold(&store, "run-cache-ooo");
}

#[test]
fn run_meta_survives_a_corrupt_event_log() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("project dir");
    let paths = store
        .create_run(CreateRunRequest {
            record: record("run-meta-corrupt", &project.to_string_lossy(), None),
            checkpoint: Some(checkpoint()),
            ..CreateRunRequest::default()
        })
        .expect("run");
    std::fs::write(paths.events_jsonl(), b"{ this is not json\n").expect("corrupt log");

    // Metadata reads never touch the event log, so they keep working where
    // a bundle read fails.
    let meta = store
        .read_run_meta("run-meta-corrupt")
        .expect("meta read")
        .expect("meta exists");
    assert_eq!(
        meta.record.expect("record").run_id,
        "run-meta-corrupt".to_string()
    );
    assert_eq!(store.list_run_records().expect("listing").len(), 1);
    assert!(store.read_run_bundle("run-meta-corrupt").is_err());
}

#[test]
fn childless_combined_journal_is_resequenced_densely() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store(&temp);
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("project dir");
    store
        .create_run(CreateRunRequest {
            record: record("run-cache-dense", &project.to_string_lossy(), None),
            checkpoint: Some(checkpoint()),
            ..CreateRunRequest::default()
        })
        .expect("run");
    let entries = combined_run_journal_entries(&store, "run-cache-dense")
        .expect("entries")
        .expect("run exists");
    let sequences: Vec<u64> = entries.iter().map(|entry| entry.sequence).collect();
    assert_eq!(sequences, (1..=entries.len() as u64).collect::<Vec<_>>());
    assert!(entries
        .iter()
        .enumerate()
        .all(|(index, entry)| entry.id == format!("journal-{}", index + 1)));
}
