use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

pub fn workspace_settings(settings: &SparkSettings) -> Value {
    let execution_placement = attractor_api::execution_placement_settings(settings);
    json!({
        "execution_placement": execution_placement.body,
    })
}
