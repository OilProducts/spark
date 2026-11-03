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

/// The `conversation.snapshot` envelope a client behind `revision` receives.
#[allow(dead_code)]
fn live_snapshot_state(
    settings: &spark_common::settings::SparkSettings,
    conversation_id: &str,
    project_path: &str,
    revision: i64,
) -> serde_json::Value {
    let envelopes = spark_workspace::live::conversation_envelopes_after(
        settings,
        &shared_store(&settings.data_dir),
        conversation_id,
        project_path,
        revision,
    )
    .expect("live envelopes");
    assert_eq!(envelopes.len(), 1, "one snapshot replaces journal replay");
    assert_eq!(envelopes[0].event_type, "conversation.snapshot");
    envelopes[0].payload["state"].clone()
}

/// Live envelopes for the stored turn and segment rows, followed by the
/// snapshot envelope a client behind them receives.
#[allow(dead_code)]
fn stored_live_envelopes(
    settings: &spark_common::settings::SparkSettings,
    conversation_id: &str,
    project_path: &str,
) -> Vec<spark_workspace::LiveEnvelope> {
    let mut envelopes: Vec<_> = stored_upserts(settings, conversation_id)
        .into_iter()
        .map(|payload| {
            let revision = payload["revision"].as_i64().unwrap_or(0);
            spark_workspace::live::conversation_event_envelope(
                conversation_id,
                project_path,
                payload,
                revision,
            )
        })
        .collect();
    envelopes.extend(
        spark_workspace::live::conversation_envelopes_after(
            settings,
            &shared_store(&settings.data_dir),
            conversation_id,
            project_path,
            0,
        )
        .expect("live envelopes"),
    );
    envelopes
}
