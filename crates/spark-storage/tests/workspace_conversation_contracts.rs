use std::fs;

use serde_json::{json, Value};
use spark_storage::{
    ConversationHandleRepository, ConversationRepository, ProjectRegistry,
    CONVERSATION_HANDLE_PATTERN, CONVERSATION_STATE_SCHEMA_VERSION,
    UNSUPPORTED_CONVERSATION_STATE_SCHEMA, UNSUPPORTED_CONVERSATION_STATE_SEGMENTS,
};

#[test]
fn conversation_handle_index_preserves_immutable_handles_and_tolerates_malformed_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let handles = ConversationHandleRepository::new(&home);

    let handle = handles
        .ensure_conversation_handle(
            "conversation-a",
            "project-a",
            "/projects/a",
            "2026-01-01T00:00:00Z",
            Some(" Calm-River "),
        )
        .expect("ensure preferred");
    assert_eq!(handle, "calm-river");

    let retained = handles
        .ensure_conversation_handle(
            "conversation-a",
            "project-a",
            "/projects/a",
            "2026-01-01T00:00:01Z",
            Some("brisk-bank"),
        )
        .expect("retain existing");
    assert_eq!(retained, "calm-river");

    let generated = handles
        .ensure_conversation_handle(
            "conversation-b",
            "project-a",
            "/projects/a",
            "2026-01-01T00:00:02Z",
            None,
        )
        .expect("generate");
    assert_eq!(generated.split('-').count(), 2);

    let mut payload = handles.load().expect("load");
    payload["handles"]["malformed"] = json!("keep me");
    handles.write(&payload).expect("write malformed");
    handles
        .remove_project_conversation_handles("project-a")
        .expect("remove project handles");
    let payload = handles.load().expect("reload");
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["pattern"], CONVERSATION_HANDLE_PATTERN);
    assert!(payload["handles"].get("calm-river").is_none());
    assert!(payload["handles"].get(generated).is_none());
    assert_eq!(payload["handles"]["malformed"], json!("keep me"));
}

#[test]
fn conversation_repository_reads_sidecars_writes_split_state_and_deletes_all_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = "/projects/conversation-app";
    let registry = ProjectRegistry::new(&home);
    let project = registry
        .ensure_project_paths(project_path)
        .expect("project paths");
    let repo = ConversationRepository::new(&home);
    let state_path = project
        .conversations_dir
        .join("conversation-a")
        .join("state.json");
    fs::create_dir_all(state_path.parent().expect("state parent")).expect("state parent");
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": CONVERSATION_STATE_SCHEMA_VERSION,
            "revision": 2,
            "conversation_id": "conversation-a",
            "conversation_handle": "amber-anchor",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "Stored thread",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:02Z",
            "turns": [],
            "segments": [],
            "event_log": [{"message": "embedded", "timestamp": "2026-01-01T00:00:00Z"}],
            "flow_run_requests": [{"id": "embedded"}]
        }))
        .expect("json"),
    )
    .expect("state");
    fs::write(
        project.flow_run_requests_dir.join("conversation-a.json"),
        serde_json::to_string_pretty(&json!({
            "conversation_id": "conversation-a",
            "project_id": project.project_id,
            "project_path": project_path,
            "event_log": [{"message": "sidecar", "timestamp": "2026-01-01T00:00:01Z"}],
            "flow_run_requests": [{"id": "request-a"}]
        }))
        .expect("json"),
    )
    .expect("run requests");
    fs::write(
        project.flow_launches_dir.join("conversation-a.json"),
        serde_json::to_string_pretty(&json!({
            "conversation_id": "conversation-a",
            "project_id": project.project_id,
            "project_path": project_path,
            "flow_launches": [{"id": "launch-a"}],
            "run_recoveries": [{"id": "recovery-a"}]
        }))
        .expect("json"),
    )
    .expect("launches");
    fs::write(
        project.proposed_plans_dir.join("conversation-a.json"),
        serde_json::to_string_pretty(&json!({
            "conversation_id": "conversation-a",
            "project_id": project.project_id,
            "project_path": project_path,
            "proposed_plans": [{"id": "plan-a"}]
        }))
        .expect("json"),
    )
    .expect("plans");

    let mut snapshot = repo
        .read_snapshot("conversation-a", Some(project_path))
        .expect("read")
        .expect("snapshot");
    assert_eq!(snapshot["event_log"][0]["message"], "sidecar");
    assert_eq!(snapshot["flow_run_requests"][0]["id"], "request-a");
    assert_eq!(snapshot["flow_launches"][0]["id"], "launch-a");
    assert_eq!(snapshot["run_recoveries"][0]["id"], "recovery-a");
    assert_eq!(snapshot["proposed_plans"][0]["id"], "plan-a");
    assert_eq!(snapshot["llm_profile"], Value::Null);

    snapshot["revision"] = json!(3);
    snapshot["event_log"] = json!([{"message": "written", "timestamp": "2026-01-01T00:00:03Z"}]);
    snapshot["flow_run_requests"] = json!([{"id": "request-b"}]);
    snapshot["flow_launches"] = json!([{"id": "launch-b"}]);
    snapshot["run_recoveries"] = json!([{"id": "recovery-b"}]);
    snapshot["proposed_plans"] = json!([{"id": "plan-b"}]);
    repo.write_snapshot(&snapshot).expect("write split");

    let core: Value =
        serde_json::from_str(&fs::read_to_string(&state_path).expect("state")).expect("json");
    assert!(core.get("flow_run_requests").is_none());
    assert_eq!(core["revision"], 3);
    assert_eq!(core["llm_profile"], Value::Null);
    let requests: Value = serde_json::from_str(
        &fs::read_to_string(project.flow_run_requests_dir.join("conversation-a.json"))
            .expect("requests"),
    )
    .expect("json");
    assert_eq!(requests["flow_run_requests"][0]["id"], "request-b");

    repo.append_raw_rpc_log("conversation-a", project_path, "outbound", "hello")
        .expect("raw log");
    assert_eq!(
        repo.read_raw_rpc_log("conversation-a", project_path)
            .expect("read raw")[0]
            .line,
        "hello"
    );
    repo.append_conversation_event(
        "conversation-a",
        project_path,
        &json!({"type": "later", "revision": 5}),
    )
    .expect("event");
    repo.append_conversation_event(
        "conversation-a",
        project_path,
        &json!({"type": "", "revision": 6}),
    )
    .expect("skip event");
    repo.append_conversation_event(
        "conversation-a",
        project_path,
        &json!({"type": "nested", "state": {"revision": 4}}),
    )
    .expect("nested event");
    let events = repo
        .read_conversation_events_after("conversation-a", project_path, 3)
        .expect("read events");
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().expect("type"))
            .collect::<Vec<_>>(),
        vec!["nested", "later"]
    );

    repo.handle_repository()
        .ensure_conversation_handle(
            "conversation-a",
            &project.project_id,
            project_path,
            "2026-01-01T00:00:00Z",
            Some("amber-anchor"),
        )
        .expect("handle");
    repo.delete_conversation("conversation-a", project_path)
        .expect("delete");
    assert!(!state_path.parent().expect("state parent").exists());
    assert!(!project
        .flow_run_requests_dir
        .join("conversation-a.json")
        .exists());
    assert!(repo
        .handle_repository()
        .find_conversation_by_handle("amber-anchor")
        .expect("find")
        .is_none());
}

#[test]
fn conversation_repository_rejects_unsupported_historical_state_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = "/projects/conversation-app";
    let project = ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project");
    let state_path = project
        .conversations_dir
        .join("conversation-old")
        .join("state.json");
    fs::create_dir_all(state_path.parent().expect("parent")).expect("parent");
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": 4,
            "revision": 1,
            "segments": []
        }))
        .expect("json"),
    )
    .expect("state");

    let error = ConversationRepository::new(&home)
        .read_snapshot("conversation-old", Some(project_path))
        .expect_err("unsupported schema");
    assert_eq!(error.to_string(), UNSUPPORTED_CONVERSATION_STATE_SCHEMA);

    fs::write(
        &state_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": CONVERSATION_STATE_SCHEMA_VERSION,
            "revision": 1
        }))
        .expect("json"),
    )
    .expect("state");
    let error = ConversationRepository::new(&home)
        .read_snapshot("conversation-old", Some(project_path))
        .expect_err("missing segments");
    assert_eq!(error.to_string(), UNSUPPORTED_CONVERSATION_STATE_SEGMENTS);
}
