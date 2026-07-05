use attractor_core::ContextMap;
use serde_json::{json, Value};

use crate::profile::ExecutionProfile;

pub const EXECUTION_MODE_CONTEXT_KEY: &str = "_attractor.runtime.execution_mode";
pub const EXECUTION_CONTAINER_IMAGE_CONTEXT_KEY: &str =
    "_attractor.runtime.execution_container_image";
pub const EXECUTION_PROFILE_ID_CONTEXT_KEY: &str = "_attractor.runtime.execution_profile_id";
pub const EXECUTION_PROFILE_SELECTION_SOURCE_CONTEXT_KEY: &str =
    "_attractor.runtime.execution_profile_selection_source";
pub const EXECUTION_PROFILE_CAPABILITIES_CONTEXT_KEY: &str =
    "_attractor.runtime.execution_profile_capabilities";

pub fn seed_execution_profile_context(
    context: &mut ContextMap,
    profile: &ExecutionProfile,
    selection_source: Option<&str>,
) {
    let capabilities = json!(profile.capabilities);
    insert_execution_context(
        context,
        profile.mode.as_str(),
        Some(profile.id.as_str()),
        profile.image.as_deref(),
        Some(capabilities),
        selection_source,
    );
}

pub(crate) fn insert_execution_context(
    context: &mut ContextMap,
    execution_mode: &str,
    execution_profile_id: Option<&str>,
    execution_container_image: Option<&str>,
    execution_profile_capabilities: Option<Value>,
    selection_source: Option<&str>,
) {
    let profile_id = execution_profile_id.unwrap_or_default();
    let image = execution_container_image.unwrap_or_default();
    let capabilities = execution_profile_capabilities.unwrap_or_else(|| json!([]));

    for (key, value) in [
        ("execution_mode", json!(execution_mode)),
        ("execution_profile_id", json!(profile_id)),
        ("execution_container_image", json!(image)),
        ("execution_profile_capabilities", capabilities.clone()),
        (EXECUTION_MODE_CONTEXT_KEY, json!(execution_mode)),
        (EXECUTION_PROFILE_ID_CONTEXT_KEY, json!(profile_id)),
        (EXECUTION_CONTAINER_IMAGE_CONTEXT_KEY, json!(image)),
        (EXECUTION_PROFILE_CAPABILITIES_CONTEXT_KEY, capabilities),
        (
            EXECUTION_PROFILE_SELECTION_SOURCE_CONTEXT_KEY,
            json!(selection_source.unwrap_or_default()),
        ),
    ] {
        context.insert(key.to_string(), value);
    }
}
