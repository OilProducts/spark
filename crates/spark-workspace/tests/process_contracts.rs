#[path = "process_contracts/conversation_event_normalization_contracts.rs"]
mod conversation_event_normalization_contracts;
#[path = "contracts/flow_install.rs"]
mod flow_install;
#[path = "process_contracts/native_live_turn_contracts.rs"]
mod native_live_turn_contracts;
#[path = "process_contracts/test_support.rs"]
mod test_support;
#[path = "process_contracts/utility_contracts.rs"]
mod utility_contracts;
#[allow(unused_imports)]
pub(crate) use flow_install::{install_flow, install_named_flow, install_org_flow};

/// One conversation store per test home, shared by every service a test
/// builds, the way one app shares its store.
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

#[allow(dead_code)]
fn conversations_with_factory(
    settings: spark_common::settings::SparkSettings,
    factory: attractor_api::RuntimeHandlerRunnerFactory,
) -> spark_workspace::WorkspaceConversationService {
    let store = shared_store(&settings.data_dir);
    spark_workspace::WorkspaceConversationService::new_with_runtime_handler_runner_factory(
        settings, store, factory,
    )
}

#[allow(dead_code)]
fn conversations_with_factory_and_backend(
    settings: spark_common::settings::SparkSettings,
    factory: attractor_api::RuntimeHandlerRunnerFactory,
    backend: std::sync::Arc<dyn spark_agent_adapter::AgentTurnBackend>,
) -> spark_workspace::WorkspaceConversationService {
    let store = shared_store(&settings.data_dir);
    spark_workspace::WorkspaceConversationService::new_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings, store, factory, backend,
    )
}

#[allow(dead_code)]
fn missions(
    settings: spark_common::settings::SparkSettings,
) -> spark_workspace::missions::WorkspaceMissionService {
    let store = shared_store(&settings.data_dir);
    spark_workspace::missions::WorkspaceMissionService::new(settings, store)
}

#[allow(dead_code)]
fn roles(
    settings: spark_common::settings::SparkSettings,
) -> spark_workspace::roles::WorkspaceRoleService {
    let store = shared_store(&settings.data_dir);
    spark_workspace::roles::WorkspaceRoleService::new(settings, store)
}

#[allow(dead_code)]
fn triggers(
    settings: spark_common::settings::SparkSettings,
) -> spark_workspace::WorkspaceTriggerService {
    let store = shared_store(&settings.data_dir);
    spark_workspace::WorkspaceTriggerService::new(settings, store)
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

/// The wire rows of the `conversation.changes` envelope a live client
/// behind `since` receives.
#[allow(dead_code)]
fn live_change_rows(
    settings: &spark_common::settings::SparkSettings,
    conversation_id: &str,
    project_path: &str,
    since: i64,
) -> Vec<serde_json::Value> {
    let service = conversations(settings.clone());
    let generation = service.store().database().generation().to_string();
    let (envelope, _) = spark_workspace::live::conversation_changes_envelope(
        &service,
        conversation_id,
        project_path,
        &generation,
        since,
    )
    .expect("live changes");
    assert_eq!(envelope.event_type, "conversation.changes");
    envelope.payload["rows"].as_array().expect("rows").clone()
}

/// The conversation state a live client behind `since` rebuilds from its
/// `conversation.changes` rows: the conversation row's metadata, plus its
/// turns, segments and artifact collections.
#[allow(dead_code)]
fn live_rows_state(
    settings: &spark_common::settings::SparkSettings,
    conversation_id: &str,
    project_path: &str,
    since: i64,
) -> serde_json::Value {
    let mut state = serde_json::json!({"turns": [], "segments": []});
    for row in live_change_rows(settings, conversation_id, project_path, since) {
        if row["deleted"] == true {
            continue;
        }
        let payload = row["payload"].clone();
        match row["kind"].as_str() {
            Some("conversation") => {
                for (key, value) in payload["metadata"].as_object().into_iter().flatten() {
                    state[key] = value.clone();
                }
                state["settings"] = payload["settings"].clone();
            }
            Some("turn") => state["turns"].as_array_mut().unwrap().push(payload),
            Some("segment") => state["segments"].as_array_mut().unwrap().push(payload),
            Some("artifact") => {
                let collection = row["collection"].as_str().expect("collection");
                if !state[collection].is_array() {
                    state[collection] = serde_json::json!([]);
                }
                state[collection].as_array_mut().unwrap().push(payload);
            }
            _ => {}
        }
    }
    state
}
