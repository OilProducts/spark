//! Tests install their flows the way Spark reads them: into the bundled
//! library, or into a project's org repo, which shadows it.

use std::path::Path;

/// Installs a JSON `FlowDefinition` as a bundled flow directory and returns
/// its name.
#[allow(dead_code)]
pub(crate) fn install_flow(
    settings: &spark_common::settings::SparkSettings,
    flow: serde_json::Value,
) -> String {
    install_into(&settings.bundled_dir().join("flows"), flow)
}

/// Installs a JSON `FlowDefinition` into `project`'s org repo and returns
/// its name.
#[allow(dead_code)]
pub(crate) fn install_org_flow(project: &Path, flow: serde_json::Value) -> String {
    install_into(&spark_common::project::org_dir(project).join("flows"), flow)
}

/// Installs a JSON `FlowDefinition` as the bundled flow `name`, for tests
/// whose flow names are load-bearing.
#[allow(dead_code)]
pub(crate) fn install_named_flow(
    settings: &spark_common::settings::SparkSettings,
    name: &str,
    flow: serde_json::Value,
) {
    write_into(&settings.bundled_dir().join("flows").join(name), flow);
}

fn install_into(flows_dir: &Path, flow: serde_json::Value) -> String {
    let id = flow["id"].as_str().expect("flow id").replace('_', "-");
    let name = format!("tests/{id}");
    write_into(&flows_dir.join(&name), flow);
    name
}

fn write_into(dir: &Path, flow: serde_json::Value) {
    let flow = serde_json::from_value(flow).expect("flow definition JSON");
    let flow = attractor_dsl::validated_flow(flow).expect("valid flow");
    attractor_dsl::write_flow_dir(dir, &flow).expect("write flow");
}
