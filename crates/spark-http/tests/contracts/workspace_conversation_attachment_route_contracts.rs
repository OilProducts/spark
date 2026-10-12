use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use serde_json::{json, Value};
use spark_agent_adapter::{AgentTurnOutput, HistoryTurn, AGENT_ATTACHMENTS_METADATA_KEY};
use spark_common::settings::SparkSettings;
use spark_http::build_app_with_agent_turn_backend;
use spark_storage::conversation::{ConversationMutation, TranscriptTurn};
use spark_storage::{ConversationRepository, StorageError};
use tower::ServiceExt;

use super::workspace_conversation_turn_route_contracts::{
    request_json, settings, wait_for_conversation_snapshot, ScriptedAgentTurnBackend,
};

const PROJECT: &str = "/projects/attachments";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake-png";

fn answers(count: usize) -> ScriptedAgentTurnBackend {
    ScriptedAgentTurnBackend::new(
        (0..count)
            .map(|index| AgentTurnOutput {
                final_assistant_text: Some(format!("Answer {index}.")),
                ..AgentTurnOutput::default()
            })
            .collect(),
    )
}

fn attachments_url(conversation_id: &str) -> String {
    format!("/workspace/api/conversations/{conversation_id}/attachments")
}

fn attachment_url(conversation_id: &str, attachment_id: &str) -> String {
    format!(
        "{}/{attachment_id}?project_path={}",
        attachments_url(conversation_id),
        urlencode(PROJECT)
    )
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

async fn request_raw(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Vec<u8>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::from(body))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, headers, bytes.to_vec())
}

async fn upload(
    app: &axum::Router,
    conversation_id: &str,
    filename: &str,
    bytes: Vec<u8>,
) -> (StatusCode, Value) {
    let uri = format!(
        "{}?project_path={}&filename={}",
        attachments_url(conversation_id),
        urlencode(PROJECT),
        urlencode(filename)
    );
    let (status, _, body) = request_raw(app.clone(), "POST", &uri, bytes).await;
    (status, serde_json::from_slice(&body).expect("json"))
}

async fn send(app: &axum::Router, conversation_id: &str, body: Value) -> (StatusCode, Value) {
    let (status, value, _) = request_json(
        app.clone(),
        "POST",
        &format!("/workspace/api/conversations/{conversation_id}/turns"),
        Some(body),
    )
    .await;
    (status, value)
}

async fn wait_for_answers(settings: &SparkSettings, conversation_id: &str, count: usize) {
    wait_for_conversation_snapshot(settings, conversation_id, PROJECT, |snapshot| {
        snapshot["turns"]
            .as_array()
            .expect("turns")
            .iter()
            .filter(|turn| turn["role"] == "assistant" && turn["status"] == "complete")
            .count()
            == count
    })
    .await;
}

#[tokio::test]
async fn attachments_upload_send_download_and_restore_into_history() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = answers(2);
    let requests = backend.requests();
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(backend));

    // A client path is display metadata only; it never becomes a storage path.
    let (status, image) = upload(&app, "chat-a", "../../evil/shot.png", PNG.to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{image}");
    assert_eq!(image["filename"], "shot.png");
    assert_eq!(image["media_type"], "image/png");
    assert_eq!(image["size_bytes"], PNG.len());
    assert!(image["id"].as_str().expect("id").starts_with("att-"));
    assert!(!temp.path().join("spark-home/workspace/evil").exists());
    let (_, notes) = upload(&app, "chat-a", "notes.md", b"# Notes".to_vec()).await;
    assert_eq!(notes["media_type"], "text/plain");

    // Attachment-only message.
    let (status, snapshot) = send(
        &app,
        "chat-a",
        json!({"project_path": PROJECT, "message": "", "provider": "codex",
            "attachment_ids": [image["id"], notes["id"]]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{snapshot}");
    let user_turn = &snapshot["turns"][0];
    assert_eq!(user_turn["content"], "");
    assert_eq!(
        user_turn["attachments"],
        json!([
            {"id": image["id"], "filename": "shot.png", "media_type": "image/png", "size_bytes": PNG.len()},
            {"id": notes["id"], "filename": "notes.md", "media_type": "text/plain", "size_bytes": 7},
        ])
    );
    wait_for_answers(&settings, "chat-a", 1).await;

    let first = requests.lock().expect("requests")[0].clone();
    let sent = first.metadata[AGENT_ATTACHMENTS_METADATA_KEY]
        .as_array()
        .expect("attachments")
        .clone();
    assert_eq!(sent.len(), 2);
    let image_path = sent[0]["path"].as_str().expect("path");
    assert!(image_path.starts_with(temp.path().join("spark-home").to_str().unwrap()));
    assert!(image_path.ends_with("/content.png"));
    assert_eq!(std::fs::read(image_path).expect("stored bytes"), PNG);

    let (status, headers, bytes) = request_raw(
        app.clone(),
        "GET",
        &attachment_url("chat-a", image["id"].as_str().unwrap()),
        Vec::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(
        headers["content-disposition"],
        "inline; filename*=UTF-8''shot.png"
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    let (_, headers, _) = request_raw(
        app.clone(),
        "GET",
        &attachment_url("chat-a", notes["id"].as_str().unwrap()),
        Vec::new(),
    )
    .await;
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename*=UTF-8''notes.md"
    );

    // A sent attachment stays until the conversation is deleted.
    let (status, _, _) = request_raw(
        app.clone(),
        "DELETE",
        &attachment_url("chat-a", image["id"].as_str().unwrap()),
        Vec::new(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A later text-only turn restores the attachments from storage into history.
    let (status, _) = send(
        &app,
        "chat-a",
        json!({"project_path": PROJECT, "message": "Follow up", "provider": "claude-code"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    wait_for_answers(&settings, "chat-a", 2).await;
    let second = requests.lock().expect("requests")[1].clone();
    assert!(!second.metadata.contains_key(AGENT_ATTACHMENTS_METADATA_KEY));
    let HistoryTurn::User(restored) = &second.history[0] else {
        panic!("first history turn is the user's");
    };
    assert_eq!(restored.attachments.len(), 2);
    assert_eq!(restored.attachments[0].path, image_path);
}

#[tokio::test]
async fn attachment_limits_scope_and_provider_support_are_enforced_on_the_server() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(answers(1)));

    let (status, body) = upload(&app, "chat-b", "huge.bin", vec![0; 20 * 1024 * 1024 + 1]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        "Attachments are limited to 20 MiB per file."
    );

    let (status, body) = send(
        &app,
        "chat-b",
        json!({"project_path": PROJECT, "message": "  "}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Message is required.");

    let ids = (0..11)
        .map(|index| format!("att-{index}"))
        .collect::<Vec<_>>();
    let (status, body) = send(
        &app,
        "chat-b",
        json!({"project_path": PROJECT, "message": "x", "attachment_ids": ids}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        "A message can carry at most 10 attachments."
    );

    let mut large = Vec::new();
    for index in 0..3 {
        let (status, attachment) = upload(
            &app,
            "chat-b",
            &format!("part{index}.bin"),
            vec![0; 17 * 1024 * 1024],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        large.push(attachment["id"].clone());
    }
    let (status, body) = send(
        &app,
        "chat-b",
        json!({"project_path": PROJECT, "message": "x", "provider": "codex", "attachment_ids": large}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        "Attachments are limited to 50 MiB per message."
    );

    // Another conversation's upload cannot be referenced.
    let (_, foreign) = upload(&app, "chat-other", "shot.png", PNG.to_vec()).await;
    let (status, body) = send(
        &app,
        "chat-b",
        json!({"project_path": PROJECT, "message": "x", "provider": "codex", "attachment_ids": [foreign["id"]]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["detail"]
        .as_str()
        .unwrap()
        .contains("is not available in this conversation"));

    let mut bmp = b"BM".to_vec();
    bmp.extend_from_slice(&30u32.to_le_bytes());
    bmp.resize(30, 0);
    let (_, bitmap) = upload(&app, "chat-b", "old.bmp", bmp).await;
    assert_eq!(bitmap["media_type"], "image/bmp");
    let (status, body) = send(
        &app,
        "chat-b",
        json!({"project_path": PROJECT, "message": "x", "provider": "codex", "attachment_ids": [bitmap["id"]]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        "Codex cannot take image/bmp images (old.bmp). Use PNG, JPEG, GIF or WebP."
    );

    // An unsupported provider is reported; the upload survives for a retry.
    let (_, image) = upload(&app, "chat-b", "shot.png", PNG.to_vec()).await;
    let (status, body) = send(
        &app,
        "chat-b",
        json!({"project_path": PROJECT, "message": "Look", "provider": "openai", "model": "gpt-5", "attachment_ids": [image["id"]]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        "Attachments need Codex or Claude Code; the openai provider cannot receive shot.png."
    );
    assert!(ConversationRepository::new(&settings.data_dir)
        .read_snapshot("chat-b", Some(PROJECT))
        .expect("read")
        .is_none());
    let (status, snapshot) = send(
        &app,
        "chat-b",
        json!({"project_path": PROJECT, "message": "Look", "provider": "codex", "attachment_ids": [image["id"]]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["turns"][0]["attachments"][0]["id"], image["id"]);
    wait_for_answers(&settings, "chat-b", 1).await;
}

#[tokio::test]
async fn draft_removal_abandoned_cleanup_and_conversation_deletion_remove_uploads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(answers(1)));

    let (_, draft) = upload(&app, "chat-c", "draft.txt", b"draft".to_vec()).await;
    let draft_url = attachment_url("chat-c", draft["id"].as_str().unwrap());
    let (status, _, _) = request_raw(app.clone(), "DELETE", &draft_url, Vec::new()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request_raw(app.clone(), "GET", &draft_url, Vec::new()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, sent) = upload(&app, "chat-c", "sent.png", PNG.to_vec()).await;
    let (_, recent) = upload(&app, "chat-c", "recent.txt", b"retry me".to_vec()).await;
    send(
        &app,
        "chat-c",
        json!({"project_path": PROJECT, "message": "Here", "provider": "codex", "attachment_ids": [sent["id"]]}),
    )
    .await;
    wait_for_answers(&settings, "chat-c", 1).await;
    // Loading the conversation keeps unsent uploads younger than 24 hours.
    let (status, _, _) = request_json(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/conversations/chat-c?project_path={}",
            urlencode(PROJECT)
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let recent_url = attachment_url("chat-c", recent["id"].as_str().unwrap());
    assert_eq!(
        request_raw(app.clone(), "GET", &recent_url, Vec::new())
            .await
            .0,
        StatusCode::OK
    );

    // Past the window, unreferenced uploads and interrupted leftovers go;
    // referenced ones stay.
    let repository = ConversationRepository::new(&settings.data_dir);
    let leftover = repository
        .project_paths(PROJECT)
        .expect("project")
        .conversations_dir
        .join("chat-c/attachments/att-interrupted");
    std::fs::create_dir_all(&leftover).expect("leftover");
    let mut removed = repository
        .prune_attachments(
            "chat-c",
            PROJECT,
            time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        )
        .expect("prune");
    removed.sort();
    let mut expected = vec![
        "att-interrupted".to_string(),
        recent["id"].as_str().unwrap().to_string(),
    ];
    expected.sort();
    assert_eq!(removed, expected);
    let sent_url = attachment_url("chat-c", sent["id"].as_str().unwrap());
    assert_eq!(
        request_raw(app.clone(), "GET", &sent_url, Vec::new())
            .await
            .0,
        StatusCode::OK
    );

    let (status, _, _) = request_json(
        app.clone(),
        "DELETE",
        &format!(
            "/workspace/api/conversations/chat-c?project_path={}",
            urlencode(PROJECT)
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        request_raw(app.clone(), "GET", &sent_url, Vec::new())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert!(!leftover.parent().unwrap().exists());
}

fn attachment_dir(settings: &SparkSettings, conversation_id: &str, attachment: &Value) -> PathBuf {
    ConversationRepository::new(&settings.data_dir)
        .project_paths(PROJECT)
        .expect("project")
        .conversations_dir
        .join(conversation_id)
        .join("attachments")
        .join(attachment["id"].as_str().expect("id"))
}

fn content_path(settings: &SparkSettings, conversation_id: &str, attachment: &Value) -> PathBuf {
    std::fs::read_dir(attachment_dir(settings, conversation_id, attachment))
        .expect("attachment dir")
        .map(|entry| entry.expect("entry").path())
        .find(|path| path.file_name().unwrap() != "attachment.json")
        .expect("content file")
}

fn user_turn_mutation(attachment: &Value) -> ConversationMutation {
    let turn: TranscriptTurn = serde_json::from_value(json!({
        "id": "turn-race", "role": "user", "content": "", "status": "complete",
        "kind": "message", "attachments": [{"id": attachment["id"]}],
    }))
    .expect("turn");
    ConversationMutation::TurnUpserted { turn }
}

fn turn_count(settings: &SparkSettings, conversation_id: &str) -> usize {
    ConversationRepository::new(&settings.data_dir)
        .read_snapshot(conversation_id, Some(PROJECT))
        .expect("read")
        .map_or(0, |snapshot| {
            snapshot["turns"].as_array().expect("turns").len()
        })
}

#[tokio::test]
async fn uploads_with_missing_or_damaged_content_are_rejected_before_the_turn_persists() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = answers(1);
    let requests = backend.requests();
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(backend));

    let (_, removed) = upload(&app, "chat-d", "shot.png", PNG.to_vec()).await;
    std::fs::remove_file(content_path(&settings, "chat-d", &removed)).expect("remove bytes");
    let (_, damaged) = upload(&app, "chat-d", "notes.txt", b"notes".to_vec()).await;
    std::fs::write(content_path(&settings, "chat-d", &damaged), b"no").expect("truncate");

    for attachment in [&removed, &damaged] {
        let (status, body) = send(
            &app,
            "chat-d",
            json!({"project_path": PROJECT, "message": "Look", "provider": "codex",
                "attachment_ids": [attachment["id"]]}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["detail"]
            .as_str()
            .unwrap()
            .contains("is not available in this conversation"));
        let url = attachment_url("chat-d", attachment["id"].as_str().unwrap());
        assert_eq!(
            request_raw(app.clone(), "GET", &url, Vec::new()).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(turn_count(&settings, "chat-d"), 0);
    assert!(requests.lock().expect("requests").is_empty());
}

#[tokio::test]
async fn restored_history_with_a_missing_attachment_fails_after_restart_and_provider_switch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    // Scripted answers are numbered per backend, so the restart shares one.
    let backend = Arc::new(answers(2));
    let requests = backend.requests();
    let app = build_app_with_agent_turn_backend(settings.clone(), backend.clone());
    let (_, image) = upload(&app, "chat-e", "shot.png", PNG.to_vec()).await;
    let (_, notes) = upload(&app, "chat-e", "notes.txt", b"notes".to_vec()).await;
    let (status, _) = send(
        &app,
        "chat-e",
        json!({"project_path": PROJECT, "message": "Here", "provider": "codex",
            "attachment_ids": [image["id"], notes["id"]]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    wait_for_answers(&settings, "chat-e", 1).await;

    // A restarted server rebuilds history from storage for either provider.
    let app = build_app_with_agent_turn_backend(settings.clone(), backend);
    let image_bytes = content_path(&settings, "chat-e", &image);
    let notes_record = attachment_dir(&settings, "chat-e", &notes).join("attachment.json");
    let record = std::fs::read(&notes_record).expect("record");

    std::fs::remove_file(&image_bytes).expect("remove bytes");
    let (status, body) = send(
        &app,
        "chat-e",
        json!({"project_path": PROJECT, "message": "Again", "provider": "claude-code"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["detail"],
        format!(
            "Attachment {} from an earlier message is missing or unreadable, so the conversation history cannot be sent.",
            image["id"].as_str().unwrap()
        )
    );
    std::fs::write(&image_bytes, PNG).expect("restore bytes");

    std::fs::remove_file(&notes_record).expect("remove record");
    let (status, body) = send(
        &app,
        "chat-e",
        json!({"project_path": PROJECT, "message": "Again", "provider": "codex"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["detail"]
        .as_str()
        .unwrap()
        .contains(notes["id"].as_str().unwrap()));
    assert_eq!(turn_count(&settings, "chat-e"), 2);
    assert_eq!(requests.lock().expect("requests").len(), 1);

    std::fs::write(&notes_record, record).expect("restore record");
    let (status, _) = send(
        &app,
        "chat-e",
        json!({"project_path": PROJECT, "message": "Again", "provider": "claude-code"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    wait_for_answers(&settings, "chat-e", 2).await;
    let HistoryTurn::User(restored) = &requests.lock().expect("requests")[1].history[0] else {
        panic!("first history turn is the user's");
    };
    assert_eq!(restored.attachments.len(), 2);
}

/// Deletion, cleanup and turn commits serialize on the conversation lock and
/// re-check references under it, so in either order a referenced upload
/// survives and a deleted one is never referenced.
#[tokio::test]
async fn referenced_uploads_survive_deletion_and_cleanup_racing_a_send() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(answers(1)));
    let repository = ConversationRepository::new(&settings.data_dir);
    let past_window = || time::OffsetDateTime::now_utc() + time::Duration::hours(25);

    // Delete or cleanup first: the send's commit is rejected.
    let (_, deleted) = upload(&app, "chat-f", "a.txt", b"a".to_vec()).await;
    let (_, pruned) = upload(&app, "chat-f", "b.txt", b"b".to_vec()).await;
    let url = attachment_url("chat-f", deleted["id"].as_str().unwrap());
    assert_eq!(
        request_raw(app.clone(), "DELETE", &url, Vec::new()).await.0,
        StatusCode::OK
    );
    repository
        .prune_attachments("chat-f", PROJECT, past_window())
        .expect("prune");
    for attachment in [&deleted, &pruned] {
        let error = repository
            .commit_conversation("chat-f", PROJECT, 0, vec![user_turn_mutation(attachment)])
            .expect_err("stale reference");
        assert!(
            matches!(error, StorageError::ConversationCommitRejected { .. }),
            "{error}"
        );
    }
    assert_eq!(turn_count(&settings, "chat-f"), 0);

    // Send first: deletion conflicts and cleanup keeps the upload.
    let (_, sent) = upload(&app, "chat-f", "c.txt", b"c".to_vec()).await;
    repository
        .commit_conversation("chat-f", PROJECT, 0, vec![user_turn_mutation(&sent)])
        .expect("commit");
    let url = attachment_url("chat-f", sent["id"].as_str().unwrap());
    assert_eq!(
        request_raw(app.clone(), "DELETE", &url, Vec::new()).await.0,
        StatusCode::CONFLICT
    );
    assert!(repository
        .prune_attachments("chat-f", PROJECT, past_window())
        .expect("prune")
        .is_empty());
    assert_eq!(
        request_raw(app.clone(), "GET", &url, Vec::new()).await.0,
        StatusCode::OK
    );

    // Neither deletion nor cleanup proceeds while a commit holds the lock.
    let (_, draft) = upload(&app, "chat-f", "d.txt", b"d".to_vec()).await;
    let lock = std::fs::File::open(
        repository
            .project_paths(PROJECT)
            .expect("project")
            .conversations_dir
            .join("chat-f/.commit.lock"),
    )
    .expect("lock file");
    lock.lock().expect("hold conversation lock");
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let blocked = {
        let repository = repository.clone();
        let draft_id = draft["id"].as_str().unwrap().to_string();
        std::thread::spawn(move || {
            let pruned = repository.prune_attachments("chat-f", PROJECT, past_window());
            let deleted = repository.delete_unreferenced_attachment("chat-f", PROJECT, &draft_id);
            done_tx.send(()).expect("signal");
            (pruned.expect("prune"), deleted.expect("delete"))
        })
    };
    assert!(done_rx.recv_timeout(Duration::from_millis(300)).is_err());
    assert!(attachment_dir(&settings, "chat-f", &draft).exists());
    lock.unlock().expect("release");
    let (pruned_ids, deleted_draft) = blocked.join().expect("thread");
    assert_eq!(pruned_ids, vec![draft["id"].as_str().unwrap().to_string()]);
    assert!(!deleted_draft);
}
