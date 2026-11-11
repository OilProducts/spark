use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::ProjectRegistry;
use tower::ServiceExt;

#[tokio::test]
async fn conversation_routes_return_snapshot_tool_output_settings_and_delete_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/http-app";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    write_state(
        &project.conversations_dir,
        "conversation-http",
        json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 1,
            "conversation_id": "conversation-http",
            "conversation_handle": "amber-anchor",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "HTTP thread",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [
                {"id": "turn-a", "role": "assistant", "content": "", "timestamp": "2026-01-01T00:00:01Z", "status": "complete", "kind": "message"}
            ],
            "flow_run_requests": [
                {"id": "request-http", "created_at": "2026-01-01T00:00:01Z", "updated_at": "2026-01-01T00:00:01Z", "flow_name": "flow.dot", "summary": "Run", "project_path": project_path, "conversation_id": "conversation-http", "source_turn_id": "turn-a"}
            ],
            "segments": [
                {
                    "id": "segment-tool",
                    "turn_id": "turn-a",
                    "order": 1,
                    "kind": "tool_call",
                    "role": "assistant",
                    "status": "complete",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "content": "",
                    "source": {},
                    "tool_call": {
                        "id": "tool-a",
                        "kind": "command_execution",
                        "status": "completed",
                        "title": "Command",
                        "output": "full output",
                        "file_paths": []
                    }
                },
                {
                    "id": "segment-present", "turn_id": "turn-a", "order": 2,
                    "kind": "tool_call", "role": "assistant", "status": "complete",
                    "content": "", "source": {},
                    "tool_call": {"id": "tool-b", "output": "PREVIEW", "output_truncated": true,
                        "output_size": 14, "output_artifact": "tool-output/present.txt"}
                },
                {
                    "id": "segment-missing", "turn_id": "turn-a", "order": 3,
                    "kind": "tool_call", "role": "assistant", "status": "complete",
                    "content": "", "source": {},
                    "tool_call": {"id": "tool-c", "output": "ONLY PREVIEW", "output_truncated": true,
                        "output_size": 100000, "output_artifact": "tool-output/missing.txt"}
                }
            ]
        }),
    );
    let app = crate::store_app_default(settings.clone());

    let snapshot = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-http?project_path=/projects/http-app",
        None,
    )
    .await;
    assert_eq!(snapshot.0, StatusCode::OK);
    assert_eq!(snapshot.1["conversation_id"], "conversation-http");
    assert_eq!(
        crate::conversation_payload(&snapshot.1)["effective"]["llm_profile"],
        Value::Null
    );
    // The artifact of a page turn travels with the page.
    assert!(snapshot.1["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["kind"] == "artifact"
            && row["collection"] == "flow_run_requests"
            && row["id"] == "request-http"
            && row["turn_id"] == "turn-a"));
    assert_eq!(snapshot.1["turn_ids"], json!(["turn-a"]));
    assert_eq!(snapshot.1["older_cursor"], Value::Null);
    let revision = crate::conversation_revision(&snapshot.1);

    let output = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-http/tool-output/segment-tool?project_path=/projects/http-app",
        None,
    )
    .await;
    assert_eq!(output.0, StatusCode::OK);
    assert_eq!(
        output.1,
        json!({"output": "full output", "output_size": 11})
    );
    // Externalized output comes from its blob; a missing blob is reported,
    // never its preview passed off as the full output.
    let blobs = project
        .conversations_dir
        .join("conversation-http/tool-output");
    std::fs::create_dir_all(&blobs).expect("tool output dir");
    std::fs::write(blobs.join("present.txt"), "present output").expect("blob");
    let output_uri = |segment: &str| {
        format!("/workspace/api/conversations/conversation-http/tool-output/{segment}?project_path=/projects/http-app")
    };
    let present = request_json(app.clone(), "GET", &output_uri("segment-present"), None).await;
    assert_eq!(
        present.1,
        json!({"output": "present output", "output_size": 14})
    );
    let missing = request_json(app.clone(), "GET", &output_uri("segment-missing"), None).await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND, "{}", missing.1);
    assert!(!missing.1.to_string().contains("ONLY PREVIEW"));

    for payload in [
        json!({"project_path": "/projects/http-app", "chat_mode": "plan"}),
        json!({"project_path": "/projects/http-app", "chat_mode": "plan", "expected_revision": "0"}),
    ] {
        let expected = if payload.get("expected_revision").is_some() {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        };
        let rejected = request_json(
            app.clone(),
            "PUT",
            "/workspace/api/conversations/conversation-http/settings",
            Some(payload),
        )
        .await;
        assert_eq!(rejected.0, expected);
        let unchanged = request_json(
            app.clone(),
            "GET",
            "/workspace/api/conversations/conversation-http?project_path=/projects/http-app",
            None,
        )
        .await;
        assert_eq!(unchanged.1, snapshot.1);
    }

    let settings_response = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/conversation-http/settings",
        Some(json!({"project_path": "/projects/http-app", "chat_mode": "plan", "expected_revision": revision})),
    )
    .await;
    assert_eq!(settings_response.0, StatusCode::OK);
    // A MutationAck: the conversation row and the mode-change turn, at one new revision.
    let ack = &settings_response.1;
    let changes = &ack["changes"];
    assert_eq!(
        crate::conversation_payload(changes)["metadata"]["chat_mode"],
        "plan"
    );
    assert_eq!(
        crate::row_payloads(changes, "turn")[0]["kind"],
        "mode_change"
    );
    assert_eq!(changes["rows"].as_array().unwrap().len(), 2);
    assert!(ack["revision"].as_str().unwrap().parse::<u64>().unwrap() > revision.parse().unwrap());
    assert_eq!(changes["watermark"], ack["revision"]);
    assert_eq!(
        crate::conversation_payload(changes)["settings"]["models"]["revision"],
        ack["revision"]
    );

    let rejected = request_json(app.clone(), "PUT",
        "/workspace/api/conversations/conversation-http/settings",
        Some(json!({"project_path": "/projects/http-app", "chat_mode": "chat", "expected_revision": revision}))).await;
    assert_eq!(rejected.0, StatusCode::CONFLICT);
    let unchanged = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-http?project_path=/projects/http-app",
        None,
    )
    .await;
    assert_eq!(crate::conversation_revision(&unchanged.1), ack["revision"]);

    // An unchanged setting allocates no revision: the current row at its revision.
    let noop = request_json(app.clone(), "PUT",
        "/workspace/api/conversations/conversation-http/settings",
        Some(json!({"project_path": "/projects/http-app", "chat_mode": "plan", "expected_revision": ack["revision"]}))).await;
    assert_eq!(noop.0, StatusCode::OK, "{}", noop.1);
    assert_eq!(noop.1["revision"], ack["revision"]);
    assert_eq!(
        noop.1["changes"]["from_revision"],
        noop.1["changes"]["watermark"]
    );
    assert_eq!(noop.1["changes"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(noop.1["changes"]["rows"][0]["revision"], ack["revision"]);

    let deleted = request_json(
        app,
        "DELETE",
        "/workspace/api/conversations/conversation-http?project_path=/projects/http-app",
        None,
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert_eq!(
        deleted.1,
        json!({
            "status": "deleted",
            "conversation_id": "conversation-http",
            "project_path": "/projects/http-app"
        })
    );
    assert!(!project.conversations_dir.join("conversation-http").exists());
}

#[tokio::test]
async fn empty_thread_created_through_settings_persists_until_deleted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/http-empty";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    let app = crate::store_app_default(settings.clone());

    let created = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/conversation-empty/settings",
        Some(json!({"project_path": project_path, "expected_revision": "0"})),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    let created_rows = created.1["changes"]["rows"].as_array().unwrap();
    assert_eq!(created_rows.len(), 1);
    assert_eq!(created_rows[0]["id"], "conversation-empty");
    assert_eq!(created_rows[0]["revision"], created.1["revision"]);

    // A fresh app reads only what was persisted, like a browser reload.
    let reloaded_app = crate::store_app_default(settings.clone());
    let listed = request_json(
        reloaded_app.clone(),
        "GET",
        "/workspace/api/projects/conversations?project_path=/projects/http-empty",
        None,
    )
    .await;
    assert_eq!(listed.0, StatusCode::OK);
    // A SummarySnapshot of the project's own threads; no started roles, no role rows.
    assert_eq!(listed.1["project_path"], "/projects/http-empty");
    let listed = crate::summary_entries(&listed.1);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["conversation_id"], "conversation-empty");
    assert_eq!(listed[0]["revision"], created.1["revision"]);
    let reloaded = request_json(
        reloaded_app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-empty?project_path=/projects/http-empty",
        None,
    )
    .await;
    assert_eq!(reloaded.0, StatusCode::OK);
    assert_eq!(
        crate::conversation_revision(&reloaded.1),
        created.1["revision"]
    );
    assert_eq!(reloaded.1["turn_ids"], json!([]));

    let deleted = request_json(
        reloaded_app.clone(),
        "DELETE",
        "/workspace/api/conversations/conversation-empty?project_path=/projects/http-empty",
        None,
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert!(!project
        .conversations_dir
        .join("conversation-empty")
        .exists());
    let listed = request_json(
        reloaded_app,
        "GET",
        "/workspace/api/projects/conversations?project_path=/projects/http-empty",
        None,
    )
    .await;
    assert!(crate::summary_entries(&listed.1).is_empty());
    // A fresh open of a deleted conversation is not found.
    let reopened = request_json(
        crate::store_app_default(settings.clone()),
        "GET",
        "/workspace/api/conversations/conversation-empty?project_path=/projects/http-empty",
        None,
    )
    .await;
    assert_eq!(reopened.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn project_conversation_list_allocates_missing_summary_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/http-list";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    write_state(
        &project.conversations_dir,
        "conversation-no-handle",
        json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 1,
            "conversation_id": "conversation-no-handle",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "Needs handle",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [],
            "segments": []
        }),
    );
    let app = crate::store_app_default(settings.clone());

    let listed = request_json(
        app,
        "GET",
        "/workspace/api/projects/conversations?project_path=/projects/http-list",
        None,
    )
    .await;

    assert_eq!(listed.0, StatusCode::OK);
    let handle = crate::summary_entries(&listed.1)[0]["handle"]
        .as_str()
        .expect("handle")
        .to_string();
    let handle = handle.as_str();
    assert_eq!(handle.split('-').count(), 2);
    let stored = crate::shared_store(&settings.data_dir)
        .database()
        .read(|read| read.conversation("conversation-no-handle"))
        .expect("rows")
        .expect("conversation");
    assert_eq!(stored.handle, handle);
}

#[tokio::test]
async fn conversation_routes_return_json_errors_for_bad_inputs_and_unsupported_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = crate::store_app_default(settings);

    let unknown = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/missing?project_path=/projects/http-errors",
        None,
    )
    .await;
    assert_eq!(unknown.0, StatusCode::NOT_FOUND);
    assert_eq!(
        unknown.1,
        json!({"detail": "Unknown conversation: missing"})
    );

    let malformed = request_text(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/conversation-old/settings",
        "{not-json",
        Some("application/json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    assert!(malformed.2.starts_with("application/json"));
    let malformed_body: Value = serde_json::from_str(&malformed.1).expect("json");
    assert!(malformed_body["detail"]
        .as_str()
        .expect("detail")
        .contains("Failed to parse"));

    let missing_delete_query = request_json(
        app,
        "DELETE",
        "/workspace/api/conversations/conversation-old",
        None,
    )
    .await;
    assert_eq!(missing_delete_query.0, StatusCode::BAD_REQUEST);
    assert!(missing_delete_query.1["detail"]
        .as_str()
        .expect("detail")
        .contains("project_path"));
}

async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    let request = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        builder
            .body(Body::from(body.to_string()))
            .expect("request body")
    } else {
        builder.body(Body::empty()).expect("request body")
    };
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice::<Value>(&bytes).expect("json");
    (status, value, content_type)
}

async fn request_text(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, String, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let request = builder
        .body(Body::from(body.to_string()))
        .expect("request body");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("utf-8"),
        content_type,
    )
}

fn write_state(conversations_dir: &Path, conversation_id: &str, payload: Value) {
    assert_eq!(payload["conversation_id"], conversation_id);
    crate::write_conversation_snapshot(
        conversations_dir
            .ancestors()
            .nth(3)
            .expect("data directory"),
        &payload,
    );
}

fn settings(root: &Path) -> SparkSettings {
    crate::isolate_home();
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        runs_dir: None,
        ui_dir: None,
        project_roots: Vec::<PathBuf>::new(),
    }
}

fn history_turn(id: &str, status: &str, content: &str) -> Value {
    json!({"id": id, "role": "user", "content": content, "timestamp": "2026-01-01T00:00:00Z",
        "status": status, "kind": "message"})
}

fn seed_history(
    settings: &SparkSettings,
    project_path: &str,
    conversation_id: &str,
    turns: Vec<Value>,
) {
    ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    crate::write_conversation_snapshot(
        &settings.data_dir,
        &json!({
            "schema_version": 5, "settings_schema_version": 1, "revision": 1,
            "conversation_id": conversation_id, "project_path": project_path,
            "chat_mode": "chat", "provider": "codex", "model": null, "reasoning_effort": null,
            "title": "Paged", "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "turns": turns, "segments": []
        }),
    );
}

/// Windows page newest first by keyset; in-flight turns are pinned; a turn
/// moved after a cursor opened resets it, and newest-end inserts do not.
#[tokio::test]
async fn conversation_pages_keyset_pin_in_flight_turns_and_reset_when_a_turn_moves() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/paged";
    // Built first: startup recovery fails messages a previous process left queued.
    let app = crate::store_app_default(settings.clone());
    seed_history(
        &settings,
        project_path,
        "conversation-paged",
        vec![
            history_turn("t0", "complete", "zero"),
            history_turn("t1", "queued", "one"),
            history_turn("t2", "complete", "two"),
            history_turn("t3", "complete", "three"),
            history_turn("t4", "complete", "four"),
        ],
    );
    let page = |query: String| {
        let app = app.clone();
        async move {
            request_json(
                app,
                "GET",
                &format!("/workspace/api/conversations/conversation-paged?project_path=/projects/paged{query}"),
                None,
            )
            .await
        }
    };
    let newest = page("&limit=2".into()).await;
    assert_eq!(newest.0, StatusCode::OK);
    assert_eq!(newest.1["turn_ids"], json!(["t3", "t4"]));
    assert_eq!(newest.1["pinned_turn_ids"], json!(["t1"]));
    let cursor = newest.1["older_cursor"]
        .as_str()
        .expect("older cursor")
        .to_string();
    let older = page(format!("&limit=2&before={cursor}")).await;
    assert_eq!(older.1["turn_ids"], json!(["t1", "t2"]));
    // An older page is not a live catch-up: same watermark contract, no pins twice.
    assert_eq!(older.1["pinned_turn_ids"], json!([]));
    let oldest = page(format!(
        "&limit=2&before={}",
        older.1["older_cursor"].as_str().unwrap()
    ))
    .await;
    assert_eq!(oldest.1["turn_ids"], json!(["t0"]));
    assert_eq!(oldest.1["older_cursor"], Value::Null);

    // A newest-end insert keeps the cursor valid.
    let repository =
        spark_storage::ConversationRepository::new(crate::shared_store(&settings.data_dir));
    repository
        .append_system_turn("conversation-paged", project_path, "mode_change", "plan")
        .unwrap();
    let appended = repository
        .read_snapshot("conversation-paged", None)
        .unwrap()
        .unwrap()["turns"][5]["id"]
        .clone();
    assert_eq!(
        page(format!("&limit=2&before={cursor}")).await.1["turn_ids"],
        json!(["t1", "t2"])
    );
    // Rewriting the queued turn moves it to the end: the cursor resets.
    repository
        .fail_queued_message("conversation-paged", project_path, "t1", "not sent")
        .unwrap();
    let reset = page(format!("&limit=2&before={cursor}")).await;
    assert_eq!(reset.0, StatusCode::CONFLICT, "{}", reset.1);
    assert_eq!(reset.1["code"], "window_reset");
    assert_eq!(reset.1["reason"], "order_changed");
    assert_eq!(reset.1["generation"], newest.1["generation"]);
    let reopened = page("&limit=2".into()).await;
    assert_eq!(reopened.1["turn_ids"], json!([appended, "t1"]));

    // Limits and cursors are validated; another generation's cursor resets.
    for bad in [
        "&limit=0",
        "&limit=101",
        "&limit=many",
        "&before=%%%",
        "&before=e30",
    ] {
        assert_eq!(page(bad.into()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    let foreign = spark_workspace::live::encode_page_cursor(
        "another-generation",
        "conversation-paged",
        &spark_storage::conversation::PageCursor {
            opening_revision: 1,
            before_position: 3,
            before_turn_id: "t3".into(),
        },
    );
    let stale = page(format!("&before={foreign}")).await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    assert_eq!(stale.1["reason"], "generation_changed");
    let other_conversation = spark_workspace::live::encode_page_cursor(
        newest.1["generation"].as_str().unwrap(),
        "conversation-other",
        &spark_storage::conversation::PageCursor {
            opening_revision: 1,
            before_position: 3,
            before_turn_id: "t3".into(),
        },
    );
    assert_eq!(
        page(format!("&before={other_conversation}")).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        page(format!("&before={}", "A".repeat(2049))).await.0,
        StatusCode::BAD_REQUEST
    );
}

/// The changes route: rows after a cursor, a reset for another generation
/// or a cursor ahead, 400 for malformed revisions, and a deleted
/// conversation's tombstone for an old cursor.
#[tokio::test]
async fn conversation_changes_route_answers_cursors_with_rows_resets_and_tombstones() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/changes";
    seed_history(
        &settings,
        project_path,
        "conversation-changes",
        vec![history_turn("t0", "complete", "zero")],
    );
    let app = crate::store_app_default(settings.clone());
    let window = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-changes?project_path=/projects/changes",
        None,
    )
    .await;
    let generation = window.1["generation"].as_str().unwrap().to_string();
    let changes = |query: String| {
        let app = app.clone();
        async move {
            request_json(app, "GET", &format!("/workspace/api/conversations/conversation-changes/changes?project_path=/projects/changes{query}"), None).await
        }
    };
    let all = changes(format!("&generation={generation}&since=0")).await;
    assert_eq!(all.0, StatusCode::OK);
    assert_eq!(all.1["watermark"], window.1["watermark"]);
    assert!(crate::row_payloads(&all.1, "turn")
        .iter()
        .any(|turn| turn["id"] == "t0"));
    let none = changes(format!(
        "&generation={generation}&since={}",
        window.1["watermark"].as_str().unwrap()
    ))
    .await;
    assert_eq!(none.1["rows"], json!([]));
    for (query, reason) in [
        (
            "&generation=elsewhere&since=0".to_string(),
            "generation_changed",
        ),
        (
            format!("&generation={generation}&since=999999"),
            "cursor_ahead",
        ),
    ] {
        let reset = changes(query).await;
        assert_eq!(reset.0, StatusCode::CONFLICT);
        assert_eq!(
            (reset.1["code"].as_str(), reset.1["reason"].as_str()),
            (Some("resync_required"), Some(reason))
        );
    }
    for bad in [
        "&generation=g&since=-1",
        "&generation=g&since=1.5",
        "&since=0",
    ] {
        assert_eq!(
            changes(bad.into()).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    let deleted = request_json(
        app.clone(),
        "DELETE",
        "/workspace/api/conversations/conversation-changes?project_path=/projects/changes",
        None,
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    let after = changes(format!(
        "&generation={generation}&since={}",
        window.1["watermark"].as_str().unwrap()
    ))
    .await;
    assert_eq!(after.0, StatusCode::OK);
    let tombstones: Vec<&Value> = after.1["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["deleted"] == true)
        .collect();
    assert!(tombstones
        .iter()
        .any(|row| row["kind"] == "conversation" && row["id"] == "conversation-changes"));
    assert!(tombstones
        .iter()
        .any(|row| row["kind"] == "turn" && row["id"] == "t0"));
    assert!(tombstones.iter().all(|row| row["payload"].is_null()));
}

/// A send, its turn's progress and a settings update carry only the rows
/// they change: their size does not grow with the conversation's history.
#[tokio::test]
async fn acceptance_progress_and_settings_results_stay_bounded_by_their_changes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/bounded";
    let filler = "history ".repeat(128);
    seed_history(
        &settings,
        project_path,
        "conversation-bounded",
        (0..300)
            .map(|index| history_turn(&format!("old-{index}"), "complete", &filler))
            .collect(),
    );
    let backend =
        crate::workspace_conversation_turn_route_contracts::ScriptedAgentTurnBackend::new(vec![
            spark_agent_adapter::AgentTurnOutput {
                final_assistant_text: Some("Bounded answer.".to_string()),
                ..Default::default()
            },
        ]);
    let app = crate::store_app(settings.clone(), std::sync::Arc::new(backend));
    let history_bytes = crate::stored_snapshot(&settings.data_dir, "conversation-bounded")
        .to_string()
        .len();
    assert!(history_bytes > 300 * 1024);
    let sent = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-bounded/turns",
        Some(json!({"project_path": project_path, "message": "Short", "provider": "codex"})),
    )
    .await;
    assert_eq!(sent.0, StatusCode::OK, "{}", sent.1);
    let rows = sent.1["changes"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(sent.1.to_string().len() < 16 * 1024);
    // The completion's commits change only this turn's rows and the summary.
    let generation = sent.1["generation"].as_str().unwrap();
    let mut progress = Value::Null;
    for _ in 0..200 {
        progress = request_json(
            app.clone(),
            "GET",
            &format!(
                "/workspace/api/conversations/conversation-bounded/changes?project_path={project_path}&generation={generation}&since={}",
                sent.1["revision"].as_str().unwrap()
            ),
            None,
        )
        .await
        .1;
        if crate::row_payloads(&progress, "turn")
            .iter()
            .any(|turn| turn["status"] == "complete" && turn["content"] == "Bounded answer.")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let progress_rows = progress["rows"].as_array().unwrap();
    assert!(
        progress_rows
            .iter()
            .all(|row| !row["id"].as_str().unwrap().starts_with("old-")),
        "{progress}"
    );
    assert!(progress.to_string().len() < 16 * 1024);
    let saved = request_json(
        app,
        "PUT",
        "/workspace/api/conversations/conversation-bounded/settings",
        Some(json!({"project_path": project_path, "chat_mode": "chat",
            "expected_revision": crate::conversation_payload(&progress)["settings"]["models"]["revision"]})),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    assert_eq!(saved.1["changes"]["rows"].as_array().unwrap().len(), 1);
    assert!(saved.1.to_string().len() < 8 * 1024);
}
