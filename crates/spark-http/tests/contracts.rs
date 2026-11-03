#[path = "contracts/isolated_home.rs"]
mod isolated_home;
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
    let mut record =
        spark_storage::conversation::record_from_snapshot(snapshot).expect("conversation snapshot");
    // Each stored turn and segment accounts for at least one revision.
    let entries = (record.transcript.turns.len() + record.transcript.segments.len()) as i64;
    record.meta.revision = record.meta.revision.max(entries);
    spark_storage::ConversationRepository::new(shared_store(data_dir))
        .seed_record(&record)
        .expect("seed conversation");
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
