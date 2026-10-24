//! Tests install their flows the way Spark reads them: into the bundled
//! library, or into a project's org repo, which shadows it.

use std::path::Path;

/// Installs flow YAML as a bundled flow directory and returns its name.
#[allow(dead_code)]
pub(crate) fn install_flow(
    settings: &spark_common::settings::SparkSettings,
    content: impl AsRef<str>,
) -> String {
    install_into(&settings.bundled_dir().join("flows"), content.as_ref())
}

/// Installs flow YAML into `project`'s org repo and returns its name.
#[allow(dead_code)]
pub(crate) fn install_org_flow(project: &Path, content: impl AsRef<str>) -> String {
    install_into(
        &spark_common::project::org_dir(project).join("flows"),
        content.as_ref(),
    )
}

fn install_into(flows_dir: &Path, content: &str) -> String {
    let flow = attractor_dsl::parse_flow_definition(content).expect("valid flow");
    let name = format!("tests/{}", flow.id.replace('_', "-"));
    attractor_dsl::write_flow_dir(&flows_dir.join(&name), &flow).expect("write flow");
    name
}
