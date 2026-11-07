#[path = "contracts/conversation_repository_contracts.rs"]
mod conversation_repository_contracts;
#[path = "../../test-fixtures/conversation_seed.rs"]
mod conversation_seed;
#[path = "contracts/history_index_contracts.rs"]
mod history_index_contracts;
#[path = "contracts/inbox_contracts.rs"]
mod inbox_contracts;
#[path = "../../test-fixtures/isolated_spark_env.rs"]
mod isolated_spark_env;
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
    conversation_seed::seed_conversation_snapshot(&shared_store(data_dir), snapshot);
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

/// The server's terminal publish hook: hands a run's state to its owner
/// conversation; whether that inbox changed.
#[allow(dead_code)]
fn deliver_run_events(settings: &spark_common::settings::SparkSettings, run_id: &str) -> bool {
    spark_workspace::inbox::deliver_run_events(settings, &shared_store(&settings.data_dir), run_id)
        .unwrap()
}

/// Server startup: settle orphaned turns, then recover every inbox.
#[allow(dead_code)]
fn recover(settings: &spark_common::settings::SparkSettings) {
    conversations(settings.clone())
        .settle_orphaned_turns()
        .unwrap();
    spark_workspace::inbox::recover(settings, &shared_store(&settings.data_dir)).unwrap();
}

/// Seeds a root run record owned by `conversation_id`, in its roster, with
/// the pending result `create_run` writes.
#[allow(dead_code)]
fn owned_run(
    settings: &spark_common::settings::SparkSettings,
    project: &str,
    conversation_id: &str,
    run_id: &str,
    status: &str,
) {
    spark_storage::ConversationRepository::new(shared_store(&settings.data_dir))
        .register_inbox_run(conversation_id, run_id, "work/seeded", "Seeded work")
        .unwrap();
    let mut record = attractor_core::RunRecord::new(run_id, project);
    record.status = status.into();
    record.launch_context = Some(
        [(
            "context.spark_owner".to_string(),
            serde_json::json!({ "conversation_id": conversation_id }),
        )]
        .into(),
    );
    let store = attractor_runtime::RunStore::for_settings(settings);
    let paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record,
            ..Default::default()
        })
        .unwrap();
    store
        .write_result(&paths, &attractor_core::RunResult::pending(run_id, status))
        .unwrap();
}

/// An inbox batch prompt's rendered events, without its header and markers.
#[allow(dead_code)]
fn batch_events(prompt: &str) -> Vec<String> {
    prompt
        .split("\n\n")
        .filter(|part| !part.starts_with("[Inbox: "))
        .map(|part| {
            part.strip_prefix("[event ")
                .and_then(|rest| rest.split_once("]\n"))
                .map_or(part, |(_, body)| body)
                .to_string()
        })
        .collect()
}

/// The owner a run was launched with, or null for an unowned run.
#[allow(dead_code)]
fn run_owner(settings: &spark_common::settings::SparkSettings, run_id: &str) -> serde_json::Value {
    let store = attractor_runtime::RunStore::for_settings(settings);
    let paths = store.find_run_root(run_id).unwrap().unwrap();
    store
        .read_run_record(&paths)
        .unwrap()
        .unwrap()
        .launch_context
        .and_then(|context| context.get("context.spark_owner").cloned())
        .unwrap_or_default()
}

/// Asserts `conversation_id` owns `run_id` and lists it in its roster.
#[allow(dead_code)]
fn assert_owned(
    settings: &spark_common::settings::SparkSettings,
    run_id: &str,
    conversation_id: &str,
) {
    assert_eq!(
        run_owner(settings, run_id),
        serde_json::json!({ "conversation_id": conversation_id })
    );
    assert!(
        spark_storage::ConversationRepository::new(shared_store(&settings.data_dir))
            .inbox_runs(conversation_id)
            .unwrap()
            .iter()
            .any(|run| run.run_id == run_id),
        "{run_id} is not in the roster of {conversation_id}"
    );
}
