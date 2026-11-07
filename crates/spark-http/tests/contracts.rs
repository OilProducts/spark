#[path = "../../test-fixtures/conversation_seed.rs"]
mod conversation_seed;
#[path = "contracts/isolated_home.rs"]
mod isolated_home;
#[path = "../../test-fixtures/isolated_spark_env.rs"]
mod isolated_spark_env;
pub(crate) use isolated_home::isolate_home;
#[path = "contracts/deprecated_route_error_contracts.rs"]
mod deprecated_route_error_contracts;
#[path = "contracts/product_shell_contracts.rs"]
mod product_shell_contracts;
#[path = "contracts/workspace_conversation_attachment_route_contracts.rs"]
mod workspace_conversation_attachment_route_contracts;
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
    seed_conversation(data_dir, snapshot);
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
#[path = "contracts/flow_install.rs"]
mod flow_install;
#[allow(unused_imports)]
pub(crate) use flow_install::{install_flow, install_org_flow};

/// One conversation store per test home, shared by the app and any service
/// a test builds next to it, the way one app shares its store.
#[allow(dead_code)]
fn shared_store(
    data_dir: impl AsRef<std::path::Path>,
) -> std::sync::Arc<spark_storage::ConversationStore> {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static STORES: OnceLock<Mutex<HashMap<PathBuf, Weak<spark_storage::ConversationStore>>>> =
        OnceLock::new();
    let data_dir = data_dir.as_ref();
    let mut stores = STORES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(store) = stores.get(data_dir).and_then(Weak::upgrade) {
        return store;
    }
    let store = spark_storage::ConversationStore::open(data_dir).expect("conversation store");
    stores.insert(data_dir.to_path_buf(), Arc::downgrade(&store));
    store
}

/// An app on the home's shared store.
#[allow(dead_code)]
fn store_app(
    settings: spark_common::settings::SparkSettings,
    backend: std::sync::Arc<dyn spark_agent_adapter::AgentTurnBackend>,
) -> axum::Router {
    let store = shared_store(&settings.data_dir);
    spark_http::build_app_with_conversation_store(settings, store, backend)
        .expect("app on the home's store")
}

#[allow(dead_code)]
fn store_app_default(settings: spark_common::settings::SparkSettings) -> axum::Router {
    let backend =
        spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&settings);
    store_app(settings, backend)
}

#[allow(dead_code)]
fn conversations(
    settings: spark_common::settings::SparkSettings,
) -> spark_workspace::WorkspaceConversationService {
    let store = shared_store(&settings.data_dir);
    spark_workspace::WorkspaceConversationService::new(settings, store)
}

#[allow(dead_code)]
fn conversations_with_backend(
    settings: spark_common::settings::SparkSettings,
    backend: std::sync::Arc<dyn spark_agent_adapter::AgentTurnBackend>,
) -> spark_workspace::WorkspaceConversationService {
    let store = shared_store(&settings.data_dir);
    spark_workspace::WorkspaceConversationService::new_with_agent_turn_backend(
        settings, store, backend,
    )
}

/// Seeds one conversation's rows in the home's shared store, exactly as given.
#[allow(dead_code)]
fn seed_conversation(data_dir: &std::path::Path, snapshot: &serde_json::Value) {
    conversation_seed::seed_conversation_snapshot(&shared_store(data_dir), snapshot);
}

/// The conversation's committed turn and segment rows as upsert payloads,
/// read through the store's revision catch-up from the beginning.
#[allow(dead_code)]
fn stored_upserts(
    settings: &spark_common::settings::SparkSettings,
    conversation_id: &str,
) -> Vec<serde_json::Value> {
    use spark_storage::conversation::{CatchUp, ChangeScope, EntityKind};
    let store = shared_store(&settings.data_dir);
    let generation = store.database().generation().to_string();
    let CatchUp::Batch(mut batch) = store
        .database()
        .read(|read| read.changes(ChangeScope::Conversation(conversation_id), &generation, 0))
        .expect("catch-up")
    else {
        panic!("catch-up reset");
    };
    batch.rows.sort_by_key(|row| row.revision);
    batch
        .rows
        .into_iter()
        .filter(|row| !row.deleted)
        .filter_map(|row| match row.kind {
            EntityKind::Turn => Some(serde_json::json!({
                "type": "turn_upsert", "revision": row.revision, "turn": row.payload,
            })),
            EntityKind::Segment => Some(serde_json::json!({
                "type": "segment_upsert", "revision": row.revision, "segment": row.payload,
            })),
            _ => None,
        })
        .collect()
}

/// The payloads of one row kind in a Page's `rows`, or a Batch's (an ack's
/// `changes`), in row order; tombstones are left out.
#[allow(dead_code)]
fn row_payloads<'a>(rows_owner: &'a serde_json::Value, kind: &str) -> Vec<&'a serde_json::Value> {
    rows_owner["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == kind && row["deleted"] == false)
        .map(|row| &row["payload"])
        .collect()
}

/// The conversation row's payload (metadata and summary) in `rows_owner`.
#[allow(dead_code)]
fn conversation_payload(rows_owner: &serde_json::Value) -> serde_json::Value {
    row_payloads(rows_owner, "conversation")
        .first()
        .map(|payload| (*payload).clone())
        .unwrap_or(serde_json::Value::Null)
}

/// The conversation row's revision, as the decimal string the wire carries.
#[allow(dead_code)]
fn conversation_revision(rows_owner: &serde_json::Value) -> String {
    rows_owner["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["kind"] == "conversation")
        .and_then(|row| row["revision"].as_str())
        .expect("conversation row revision")
        .to_string()
}

/// One conversation's full committed state, read from the home's shared
/// store: the explicit history projection, for assertions only.
#[allow(dead_code)]
fn stored_snapshot(data_dir: &std::path::Path, conversation_id: &str) -> serde_json::Value {
    spark_storage::ConversationRepository::new(shared_store(data_dir))
        .read_snapshot(conversation_id, None)
        .expect("read snapshot")
        .expect("stored conversation")
}

/// A SummarySnapshot's live, listed conversation rows as thread-list entries.
#[allow(dead_code)]
fn summary_entries(snapshot: &serde_json::Value) -> Vec<serde_json::Value> {
    snapshot["rows"]
        .as_array()
        .expect("summary rows")
        .iter()
        .filter(|row| row["deleted"] == false)
        .map(|row| {
            let mut entry = row["payload"].clone();
            entry["conversation_id"] = row["id"].clone();
            entry["revision"] = row["revision"].clone();
            entry
        })
        .collect()
}

/// A conversation's newest window read through the route, as the fields
/// tests inspect: `revision` and `handle` from its conversation row,
/// `settings`, `turns` in position order and `segments` in turn order.
#[allow(dead_code)]
async fn conversation_view(
    app: axum::Router,
    conversation_id: &str,
    project_path: &str,
) -> serde_json::Value {
    use tower::ServiceExt;
    let uri = format!(
        "/workspace/api/conversations/{conversation_id}?project_path={}&limit=100",
        encode_query(project_path)
    );
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let page: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let conversation = conversation_payload(&page);
    let position = |row: &serde_json::Value| {
        row["position"]
            .as_str()
            .and_then(|position| position.parse::<i64>().ok())
            .unwrap_or_default()
    };
    let mut turns: Vec<&serde_json::Value> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == "turn")
        .collect();
    turns.sort_by_key(|row| position(row));
    let mut view = serde_json::json!({
        "conversation_id": conversation_id,
        "revision": conversation_revision(&page),
        "conversation_handle": conversation["handle"],
        "project_path": conversation["project_path"],
        "title": conversation["title"],
        "settings": conversation["settings"],
        "turns": turns.iter().map(|row| row["payload"].clone()).collect::<Vec<_>>(),
        "segments": row_payloads(&page, "segment"),
    });
    for collection in [
        "flow_run_requests",
        "flow_launches",
        "run_recoveries",
        "proposed_plans",
        "event_log",
    ] {
        let mut artifacts: Vec<&serde_json::Value> = page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["kind"] == "artifact" && row["collection"] == collection)
            .collect();
        artifacts.sort_by_key(|row| position(row));
        view[collection] = artifacts.iter().map(|row| row["payload"].clone()).collect();
    }
    view
}
