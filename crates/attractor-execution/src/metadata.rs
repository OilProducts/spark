use attractor_core::{ContextMap, RunRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::insert_execution_context;
use crate::modes::ExecutionMode;
use crate::profile::ExecutionProfileSelection;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionLaunchMetadata {
    pub execution_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_container_image: Option<String>,
    #[serde(default)]
    pub execution_profile_capabilities: Value,
    #[serde(default)]
    pub execution_profile_selection_source: String,
}

impl ExecutionLaunchMetadata {
    pub fn as_context_updates(&self) -> ContextMap {
        let mut context = ContextMap::new();
        insert_execution_context(
            &mut context,
            &self.execution_mode,
            self.execution_profile_id.as_deref(),
            self.execution_container_image.as_deref(),
            Some(self.execution_profile_capabilities.clone()),
            Some(&self.execution_profile_selection_source),
        );
        context
    }
}

pub fn build_launch_metadata(selection: &ExecutionProfileSelection) -> ExecutionLaunchMetadata {
    let profile = &selection.profile;
    ExecutionLaunchMetadata {
        execution_mode: profile.mode.as_str().to_string(),
        execution_profile_id: Some(profile.id.clone()),
        execution_container_image: (profile.mode == ExecutionMode::LocalContainer)
            .then(|| profile.image.clone())
            .flatten(),
        execution_profile_capabilities: json!(profile.capabilities),
        execution_profile_selection_source: selection.selection_source.clone(),
    }
}

pub fn apply_launch_metadata_to_record(record: &mut RunRecord, metadata: &ExecutionLaunchMetadata) {
    record.execution_mode = metadata.execution_mode.clone();
    record.execution_profile_id = metadata.execution_profile_id.clone();
    record.execution_container_image = metadata.execution_container_image.clone();
    record.execution_profile_capabilities = Some(metadata.execution_profile_capabilities.clone());
}
