use std::fs;
use std::path::Path;

use spark_assets::{flows, frontend, guides, icons, models, resources, ResourceSource};
use spark_common::settings::SparkSettings;

const STARTER_FLOW_NAMES: &[&str] = &[
    "examples/human-review-loop.yaml",
    "examples/implement-review-loop.yaml",
    "examples/parallel-review.yaml",
    "examples/simple-linear.yaml",
    "examples/supervision/implementation-worker.yaml",
    "examples/supervision/supervised-implementation.yaml",
    "math-research/explore-conjecture.yaml",
    "math-research/formalize-result.yaml",
    "software-development/audit-codebase.yaml",
    "software-development/design-change.yaml",
    "software-development/implement-change.yaml",
    "software-development/integrate-ready-branches.yaml",
    "software-development/investigate-bug.yaml",
    "software-development/merge-change.yaml",
    "software-development/review-change.yaml",
    "software-development/run-retrospective.yaml",
    "software-development/spec-implementation/implement-milestone.yaml",
    "software-development/spec-implementation/implement-spec.yaml",
    "software-development/workers/implement-task.yaml",
    "software-development/workers/resolve-merge-conflicts.yaml",
];

#[test]
fn frontend_resource_order_prefers_explicit_ui_then_source_then_packaged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let explicit_ui = temp.path().join("explicit-ui");
    let explicit_assets = explicit_ui.join("assets");
    fs::create_dir_all(&explicit_assets).expect("explicit assets");
    fs::write(explicit_ui.join("index.html"), "<main>explicit</main>").expect("explicit index");
    fs::write(explicit_assets.join("app.js"), "explicit();").expect("explicit js");

    let project_root = temp.path().join("source-root");
    let source_dist = project_root.join("frontend/dist");
    let source_assets = source_dist.join("assets");
    fs::create_dir_all(&source_assets).expect("source assets");
    fs::write(source_dist.join("index.html"), "<main>source</main>").expect("source index");
    fs::write(source_assets.join("app.js"), "source();").expect("source js");

    let mut settings = settings(temp.path());
    settings.project_root = project_root.clone();
    settings.ui_dir = Some(explicit_ui.clone());

    let explicit_index = frontend::load_index(&settings).expect("explicit index");
    assert_eq!(explicit_index.source(), ResourceSource::ExplicitUiDir);
    assert_eq!(
        explicit_index.filesystem_path().map(Path::to_path_buf),
        Some(explicit_ui.join("index.html"))
    );
    assert_eq!(
        explicit_index.text().expect("utf-8"),
        "<main>explicit</main>"
    );
    assert_eq!(
        frontend::load_asset(&settings, "assets/app.js")
            .expect("asset lookup")
            .expect("explicit asset")
            .text()
            .expect("utf-8"),
        "explicit();"
    );

    settings.ui_dir = None;
    let source_index = frontend::load_index(&settings).expect("source index");
    assert_eq!(source_index.source(), ResourceSource::SourceTree);
    assert_eq!(source_index.text().expect("utf-8"), "<main>source</main>");
    assert_eq!(
        frontend::load_asset(&settings, "assets/app.js")
            .expect("asset lookup")
            .expect("source asset")
            .text()
            .expect("utf-8"),
        "source();"
    );

    settings.project_root = temp.path().join("missing-source-root");
    let packaged_index = frontend::load_index(&settings).expect("packaged index");
    assert_eq!(packaged_index.source(), ResourceSource::Packaged);
    assert!(packaged_index
        .text()
        .expect("utf-8")
        .contains("<div id=\"root\"></div>"));
    assert_eq!(
        frontend::load_asset(&settings, "assets/spark-app-icon.png")
            .expect("asset lookup")
            .expect("packaged icon")
            .source(),
        ResourceSource::Packaged
    );
    assert!(frontend::packaged_asset_names()
        .iter()
        .any(|name| name == "index.html"));
}

#[test]
fn frontend_asset_lookup_rejects_unsafe_paths_and_symlink_escapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui_root = temp.path().join("ui");
    let assets = ui_root.join("assets");
    fs::create_dir_all(&assets).expect("assets");
    fs::write(ui_root.join("index.html"), "<!doctype html>").expect("index");
    fs::write(temp.path().join("secret.txt"), "secret").expect("secret");
    #[cfg(unix)]
    std::os::unix::fs::symlink(temp.path().join("secret.txt"), assets.join("escape.txt"))
        .expect("symlink");

    let mut settings = settings(temp.path());
    settings.ui_dir = Some(ui_root);

    for unsafe_path in [
        "",
        "../Cargo.toml",
        "assets/../Cargo.toml",
        "/tmp/secret",
        ".",
    ] {
        assert!(
            frontend::load_asset(&settings, unsafe_path).is_err(),
            "{unsafe_path}"
        );
    }

    #[cfg(unix)]
    assert!(frontend::load_asset(&settings, "assets/escape.txt").is_err());
}

#[test]
fn starter_flow_inventory_is_packaged_sorted_and_utf8() {
    let names = flows::starter_flow_names().expect("starter flow names");
    assert_eq!(
        names,
        STARTER_FLOW_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    );

    let assets = flows::starter_flow_assets().expect("starter flow assets");
    assert_eq!(assets.len(), STARTER_FLOW_NAMES.len());
    for asset in assets {
        assert!(asset.name.ends_with(".yaml"));
        assert!(asset.content.contains("schema_version:"), "{}", asset.name);
        let flow = attractor_dsl::parse_flow_definition(&asset.content).unwrap_or_else(|error| {
            panic!("{} does not satisfy the flow schema: {error}", asset.name)
        });
        let software_metadata = flow.metadata.get("software_development");
        if asset.name.starts_with("software-development/workers/")
            || asset
                .name
                .contains("/spec-implementation/implement-milestone")
        {
            assert_ne!(
                software_metadata
                    .and_then(|value| value.get("launcher"))
                    .and_then(serde_json::Value::as_bool),
                Some(true),
                "{} must not be requestable",
                asset.name
            );
        }
    }

    for unsafe_name in [
        "../bad.yaml",
        "/tmp/bad.yaml",
        "examples",
        "examples/bad.txt",
    ] {
        assert!(
            flows::load_starter_flow(unsafe_name).is_err(),
            "{unsafe_name}"
        );
    }
}

#[test]
fn guide_model_icon_and_provider_template_resources_are_available() {
    assert_eq!(
        guides::guide_names(),
        vec![
            "flow-definition-authoring.md".to_string(),
            "spark-operations.md".to_string()
        ]
    );
    assert!(guides::flow_definition_authoring_guide()
        .expect("flow authoring")
        .text()
        .expect("guide utf-8")
        .contains("FlowDefinition YAML"));
    assert!(guides::spark_operations_guide()
        .expect("operations")
        .text()
        .expect("guide utf-8")
        .contains("Spark"));

    let catalog = models::model_catalog_resource();
    assert_eq!(catalog.source(), ResourceSource::Packaged);
    let parsed: serde_json::Value =
        serde_json::from_str(models::model_catalog_json()).expect("model catalog json");
    let catalog_entries = parsed.as_array().expect("model catalog array");
    assert!(catalog_entries.len() > 5);
    assert!(catalog_entries.iter().any(|entry| {
        entry["id"] == "gpt-5.2-codex"
            && entry["aliases"]
                .as_array()
                .expect("aliases")
                .iter()
                .any(|alias| alias == "codex")
    }));
    assert!(catalog_entries
        .iter()
        .any(|entry| entry["provider"] == "gemini" && entry["id"] == "gemini-3.1-pro-preview"));

    assert!(icons::favicon().expect("favicon").bytes().len() > 1000);
    assert_eq!(
        icons::root_image_names(),
        vec![
            "spark-app-icon-dark.png".to_string(),
            "spark-symbol-light.png".to_string(),
            "spark-wordmark-dark-orange-symbol.png".to_string(),
            "spark-wordmark-light-mono-symbol.png".to_string(),
            "spark-wordmark-light-orange-symbol.png".to_string(),
        ]
    );
    assert!(
        icons::load_root_image("spark-wordmark-light-orange-symbol.png")
            .expect("root image")
            .bytes()
            .len()
            > 1000
    );

    let template = resources::provider_env_template();
    let text = template.text().expect("provider template utf-8");
    for key in [
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_ORG_ID",
        "OPENAI_PROJECT_ID",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "GEMINI_API_KEY",
        "GEMINI_BASE_URL",
        "GOOGLE_API_KEY",
        "OPENROUTER_API_KEY",
        "OPENROUTER_BASE_URL",
        "OPENROUTER_HTTP_REFERER",
        "OPENROUTER_TITLE",
        "LITELLM_BASE_URL",
        "LITELLM_API_KEY",
        "OPENAI_COMPATIBLE_BASE_URL",
        "OPENAI_COMPATIBLE_API_KEY",
    ] {
        assert!(text.contains(key), "{key}");
    }
    assert!(!text.contains("your_key_here"));
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
