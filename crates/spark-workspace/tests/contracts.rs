#[path = "contracts/conversation_repository_contracts.rs"]
mod conversation_repository_contracts;
#[path = "contracts/history_index_contracts.rs"]
mod history_index_contracts;
#[path = "contracts/mission_contracts.rs"]
mod mission_contracts;
#[path = "contracts/project_registry_contracts.rs"]
mod project_registry_contracts;
#[path = "contracts/project_setup_contracts.rs"]
mod project_setup_contracts;
#[path = "contracts/review_artifact_contracts.rs"]
mod review_artifact_contracts;
#[path = "contracts/role_contracts.rs"]
mod role_contracts;
#[path = "contracts/run_orchestration_contracts.rs"]
mod run_orchestration_contracts;
#[path = "contracts/trigger_source_activation_contracts.rs"]
mod trigger_source_activation_contracts;
#[path = "contracts/workflow_log_contracts.rs"]
mod workflow_log_contracts;

/// Seeds one conversation's rows in the home's shared store, exactly as given.
fn write_conversation_snapshot(data_dir: &std::path::Path, snapshot: &serde_json::Value) {
    let mut record =
        spark_storage::conversation::record_from_snapshot(snapshot).expect("conversation snapshot");
    // Each stored turn and segment accounts for at least one revision.
    let entries = (record.transcript.turns.len() + record.transcript.segments.len()) as i64;
    record.meta.revision = record.meta.revision.max(entries);
    spark_storage::ConversationRepository::new(shared_store(data_dir))
        .seed_record(&record)
        .expect("seed conversation");
}
#[path = "contracts/flow_install.rs"]
mod flow_install;
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
