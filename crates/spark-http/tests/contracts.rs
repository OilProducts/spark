#[path = "contracts/deprecated_route_error_contracts.rs"]
mod deprecated_route_error_contracts;
#[path = "contracts/product_shell_contracts.rs"]
mod product_shell_contracts;
#[path = "contracts/workspace_conversation_route_contracts.rs"]
mod workspace_conversation_route_contracts;
#[path = "contracts/workspace_conversation_turn_route_contracts.rs"]
mod workspace_conversation_turn_route_contracts;
#[path = "contracts/workspace_flow_route_contracts.rs"]
mod workspace_flow_route_contracts;
#[path = "contracts/workspace_review_route_contracts.rs"]
mod workspace_review_route_contracts;
#[path = "contracts/workspace_route_contracts.rs"]
mod workspace_route_contracts;
#[path = "contracts/workspace_run_route_contracts.rs"]
mod workspace_run_route_contracts;
#[path = "contracts/workspace_trigger_route_contracts.rs"]
mod workspace_trigger_route_contracts;

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
            std::fs::write($path, serde_json::to_vec_pretty($value).expect("json"))
                .expect("conversation file");
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

/// Percent-encodes a query value, so paths with spaces make valid URIs.
fn encode_query(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
