//! Runs start only from approved live files, so tests install their flows
//! the way a human would: write the directory, then approve its files.

/// Logs `human_save` for every file in a live flow directory.
#[allow(dead_code)]
pub(crate) fn approve_flow(
    settings: &spark_common::settings::SparkSettings,
    dir: &std::path::Path,
) {
    attractor_dsl::accept(
        &attractor_dsl::TrustStore::for_settings(settings),
        dir,
        "human:test",
    )
    .expect("approve flow");
}

/// Installs flow YAML as an approved live flow directory and returns its name.
#[allow(dead_code)]
pub(crate) fn install_flow(
    settings: &spark_common::settings::SparkSettings,
    content: impl AsRef<str>,
) -> String {
    let flow = attractor_dsl::parse_flow_definition(content.as_ref()).expect("valid flow");
    let name = format!("tests/{}", flow.id.replace('_', "-"));
    let dir = settings.flows_dir.join(&name);
    attractor_dsl::write_flow_dir(&dir, &flow).expect("write flow");
    approve_flow(settings, &dir);
    name
}
