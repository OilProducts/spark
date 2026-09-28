//! Utility inference: one small model call Spark's own code makes for
//! housekeeping, on the workspace utility model. Each caller supplies its
//! prompt as a code constant.

use std::collections::BTreeMap;
use std::time::Duration;

use spark_common::settings::SparkSettings;

use crate::conversations::{capture_execution_settings, EnvironmentAgentTurnBackend};
use crate::{WorkspaceError, WorkspaceResult};

pub const UTILITY_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs one utility call and returns the model's text. `Ok(None)` means no
/// utility model is set, so utility inference is off; there is no fallback to
/// the chat model. Provider failures and timeouts are errors.
pub fn utility_complete(
    settings: &SparkSettings,
    instructions: &str,
    input: &str,
) -> WorkspaceResult<Option<String>> {
    let Some(group) = crate::settings::workspace_utility_model_settings(settings)? else {
        return Ok(None);
    };
    let (execution_settings, profile) = capture_execution_settings(settings, &group)?;
    let metadata = BTreeMap::from([("spark.execution.settings".to_string(), execution_settings)]);
    let client = EnvironmentAgentTurnBackend::new(settings.config_dir.clone())
        .client_for_execution(&metadata, &std::env::vars().collect())
        .map_err(|error| WorkspaceError::ServiceUnavailable(error.message))?;
    let call = spark_agent_adapter::UtilityCall {
        provider: group
            .provider
            .clone()
            .or_else(|| profile.as_ref().map(|profile| profile.provider.clone())),
        model: group.model.clone().or_else(|| {
            profile
                .as_ref()
                .and_then(|profile| profile.default_model.clone())
        }),
        llm_profile: group.llm_profile.clone(),
        reasoning_effort: group.reasoning_effort.clone(),
        instructions: instructions.to_string(),
        input: input.to_string(),
        metadata,
        timeout: UTILITY_TIMEOUT,
    };
    spark_agent_adapter::run_utility_call(&client, &call)
        .map(Some)
        .map_err(|error| {
            WorkspaceError::ServiceUnavailable(format!("Utility call failed: {error}"))
        })
}
