#![forbid(unsafe_code)]

//! Execution profile and local-container worker contracts for Attractor.

pub mod container;
pub mod context;
pub mod errors;
pub mod metadata;
pub mod modes;
pub mod profile;
pub mod protocol;
pub mod settings_view;
pub mod worker;

pub use container::{
    profile_mounts_for_test, CommandResult, CommandSpec, ContainerCommandRunner,
    ContainerizedNodeExecutor, SystemCommandRunner,
};
pub use context::{
    seed_execution_profile_context, EXECUTION_CONTAINER_IMAGE_CONTEXT_KEY,
    EXECUTION_MODE_CONTEXT_KEY, EXECUTION_PROFILE_CAPABILITIES_CONTEXT_KEY,
    EXECUTION_PROFILE_ID_CONTEXT_KEY, EXECUTION_PROFILE_SELECTION_SOURCE_CONTEXT_KEY,
};
pub use errors::{
    ExecutionLaunchError, ExecutionProfileConfigError, ExecutionProfileFieldError,
    ExecutionProfileSelectionError, ExecutionProtocolError,
};
pub use metadata::{
    apply_launch_metadata_to_record, build_launch_metadata, ExecutionLaunchMetadata,
};
pub use modes::{
    normalize_execution_mode, ExecutionMode, EXECUTION_MODES, EXECUTION_MODE_LOCAL_CONTAINER,
    EXECUTION_MODE_NATIVE,
};
pub use profile::{
    load_execution_profile_config, resolve_execution_profile_by_id, ExecutionProfile,
    ExecutionProfileConfigRoot, ExecutionProfileGraph, ExecutionProfileSelection,
    ExecutionProfileSettings, EXECUTION_PROFILES_FILENAME, IMPLEMENTATION_NATIVE_PROFILE_ID,
};
pub use protocol::{
    outcome_from_payload, outcome_to_payload, ChildInterventionRequestFrame,
    ChildInterventionResultFrame, ChildRunRequestFrame, ChildRunResultFrame,
    ChildStatusRequestFrame, ChildStatusResultFrame, EventFrame, HumanGateAnswerFrame,
    HumanGateRequestFrame, ResultFrame, RunRootMetadata, WorkerFrame, WorkerNodeRequest,
};
pub use settings_view::public_execution_placement_settings;
pub use worker::{run_worker_node_from_reader_writer, run_worker_node_stdio};
