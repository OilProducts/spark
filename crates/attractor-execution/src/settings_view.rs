use serde_json::{json, Value};

use crate::errors::ExecutionProfileConfigError;
use crate::modes::EXECUTION_MODES;
use crate::profile::{
    load_execution_profile_config, ExecutionProfile, ExecutionProfileSettings,
    EXECUTION_PROFILES_FILENAME,
};

pub fn public_execution_placement_settings(settings: &impl ExecutionProfileSettings) -> Value {
    let config_path = settings.config_dir().join(EXECUTION_PROFILES_FILENAME);
    let (loaded, profiles, default_execution_profile_id, synthesized_native_default, errors) =
        match load_execution_profile_config(settings, None, None, None) {
            Ok(graph) => {
                let profiles = graph
                    .profiles
                    .values()
                    .map(|profile| serialize_profile(profile, graph.synthesized_native_default))
                    .collect::<Vec<_>>();
                (
                    true,
                    profiles,
                    graph
                        .default_execution_profile_id
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                    graph.synthesized_native_default,
                    Vec::new(),
                )
            }
            Err(error) => (
                false,
                Vec::new(),
                Value::Null,
                false,
                serialize_config_errors(error),
            ),
        };

    json!({
        "execution_modes": EXECUTION_MODES,
        "config": {
            "filename": EXECUTION_PROFILES_FILENAME,
            "path": config_path.to_string_lossy(),
            "exists": config_path.exists(),
            "loaded": loaded,
            "synthesized_native_default": synthesized_native_default,
        },
        "default_execution_profile_id": default_execution_profile_id,
        "profiles": profiles,
        "validation_errors": errors,
    })
}

fn serialize_profile(profile: &ExecutionProfile, synthesized_native_default: bool) -> Value {
    let capabilities = if synthesized_native_default
        && profile.id == crate::profile::IMPLEMENTATION_NATIVE_PROFILE_ID
        && profile.capabilities.is_empty()
    {
        json!({})
    } else {
        json!(profile.capabilities)
    };
    json!({
        "id": profile.id,
        "label": profile.label,
        "mode": profile.mode.as_str(),
        "enabled": profile.enabled,
        "image": profile.image,
        "capabilities": capabilities,
        "metadata": profile.metadata,
    })
}

fn serialize_config_errors(error: ExecutionProfileConfigError) -> Vec<Value> {
    if error.field_errors.is_empty() {
        return vec![json!({
            "field": null,
            "message": error.message,
            "profile_id": null,
        })];
    }
    error
        .field_errors
        .into_iter()
        .map(|error| {
            json!({
                "field": error.field,
                "message": error.message,
                "profile_id": error.profile_id,
            })
        })
        .collect()
}
