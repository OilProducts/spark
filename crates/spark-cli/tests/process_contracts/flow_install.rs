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

fn install_into(flows_dir: &Path, flow: serde_json::Value) -> String {
    let flow = serde_json::from_value(flow).expect("flow definition JSON");
    let flow = attractor_dsl::validated_flow(flow).expect("valid flow");
    let name = format!("tests/{}", flow.id.replace('_', "-"));
    attractor_dsl::write_flow_dir(&flows_dir.join(&name), &flow).expect("write flow");
    name
}

/// Writes a JSON `FlowDefinition` as a flow directory at `dir`, unvalidated,
/// so tests can also write flows the validator rejects.
#[allow(dead_code)]
pub(crate) fn write_flow_json(dir: &Path, flow: serde_json::Value) {
    let flow = serde_json::from_value(flow).expect("flow definition JSON");
    attractor_dsl::write_flow_dir(dir, &flow).expect("write flow");
}
