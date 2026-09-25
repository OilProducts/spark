#[path = "contracts/conversation_repository_contracts.rs"]
mod conversation_repository_contracts;
#[path = "contracts/flow_catalog_contracts.rs"]
mod flow_catalog_contracts;
#[path = "contracts/mission_contracts.rs"]
mod mission_contracts;
#[path = "contracts/project_registry_contracts.rs"]
mod project_registry_contracts;
#[path = "contracts/review_artifact_contracts.rs"]
mod review_artifact_contracts;
#[path = "contracts/run_orchestration_contracts.rs"]
mod run_orchestration_contracts;
#[path = "contracts/trigger_source_activation_contracts.rs"]
mod trigger_source_activation_contracts;
#[path = "contracts/workflow_log_contracts.rs"]
mod workflow_log_contracts;

fn write_conversation_snapshot(data_dir: &std::path::Path, snapshot: &serde_json::Value) {
    use spark_storage::conversation::{record_from_snapshot, ArtifactCollection};
    use spark_storage::{ActivityRepository, ConversationHandleRepository, ProjectRegistry};

    let mut record = record_from_snapshot(snapshot).expect("conversation snapshot");
    let project = ProjectRegistry::new(data_dir)
        .ensure_project_paths(&record.meta.project_path)
        .expect("project paths");
    let root = project.conversations_dir.join(&record.meta.conversation_id);
    std::fs::create_dir_all(root.join("artifacts")).expect("conversation directories");
    let activity = ActivityRepository::new(&root);
    let mut revision = 1;
    let record_count = (record.transcript.turns.len() + record.transcript.segments.len()) as i64;
    let published_revision = record.meta.revision.max(record_count);
    let mut journal_revision = published_revision - record_count;
    for turn in &record.transcript.turns {
        journal_revision += 1;
        let event = activity
            .append_event(
                serde_json::json!({"type": "turn_upsert", "revision": journal_revision}),
                &record.meta.updated_at,
            )
            .expect("turn event");
        activity
            .append_transcript(&spark_storage::TranscriptRecord::TurnUpsert {
                revision,
                committed_at: event.committed_at,
                source_event_sequence: event.sequence,
                turn: turn.clone(),
            })
            .expect("turn transcript");
        revision += 1;
    }
    for segment in &record.transcript.segments {
        journal_revision += 1;
        let event = activity
            .append_event(
                serde_json::json!({"type": "segment_upsert", "revision": journal_revision}),
                &record.meta.updated_at,
            )
            .expect("segment event");
        activity
            .append_transcript(&spark_storage::TranscriptRecord::SegmentUpsert {
                revision,
                committed_at: event.committed_at,
                source_event_sequence: event.sequence,
                segment: segment.clone(),
            })
            .expect("segment transcript");
        revision += 1;
    }
    record.meta.revision = published_revision;
    macro_rules! write {
        ($path:expr, $value:expr) => {{
            let mut bytes = serde_json::to_vec_pretty($value).expect("json");
            bytes.push(b'\n');
            std::fs::write($path, bytes).expect("conversation file");
        }};
    }
    write!(root.join("conversation.json"), &record.meta);
    write!(root.join("event-log.json"), &record.artifacts.event_log);
    for (collection, name) in [
        (
            ArtifactCollection::FlowRunRequests,
            "flow-run-requests.json",
        ),
        (ArtifactCollection::FlowLaunches, "flow-launches.json"),
        (ArtifactCollection::RunRecoveries, "run-recoveries.json"),
        (ArtifactCollection::ProposedPlans, "proposed-plans.json"),
    ] {
        write!(
            root.join("artifacts").join(name),
            record.artifacts.collection(collection)
        );
    }
    if !record.meta.conversation_handle.is_empty() {
        ConversationHandleRepository::new(data_dir)
            .ensure_conversation_handle(
                &record.meta.conversation_id,
                &project.project_id,
                &record.meta.project_path,
                &record.meta.created_at,
                Some(&record.meta.conversation_handle),
            )
            .expect("conversation handle");
    }
}
