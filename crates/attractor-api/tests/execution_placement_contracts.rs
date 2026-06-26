use attractor_api::{execution_placement_settings, handle_attractor_request};
use serde_json::json;
use spark_common::settings::SparkSettings;

#[test]
fn execution_placement_settings_exposes_profile_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[defaults]
execution_profile_id = "native-fast"

[profiles.native-fast]
label = "Native Fast"
mode = "native"
capabilities = ["filesystem"]
"#,
    )
    .expect("write profiles");

    let response = execution_placement_settings(&settings);

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["loaded"], json!(true));
    assert_eq!(
        response.body["default_execution_profile_id"],
        json!("native-fast")
    );
    assert_eq!(response.body["profiles"][0]["id"], json!("native-fast"));
}

#[test]
fn execution_placement_settings_route_uses_mounted_attractor_metadata_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[profiles.native-fast]
label = "Native Fast"
mode = "native"
"#,
    )
    .expect("write profiles");

    let response = handle_attractor_request(
        "GET",
        "/attractor/api/execution-placement-settings",
        "",
        settings,
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["loaded"], json!(true));
    assert_eq!(response.body["profiles"][0]["id"], json!("native-fast"));
}

#[test]
fn missing_config_synthesizes_native_default_for_public_settings_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());

    let response = handle_attractor_request(
        "GET",
        "/attractor/api/execution-placement-settings",
        "",
        settings,
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["exists"], json!(false));
    assert_eq!(response.body["config"]["loaded"], json!(true));
    assert_eq!(
        response.body["config"]["synthesized_native_default"],
        json!(true)
    );
    assert_eq!(response.body["default_execution_profile_id"], json!(null));
    assert_eq!(
        response.body["profiles"],
        json!([{
            "id": "native",
            "label": "Native",
            "mode": "native",
            "enabled": true,
            "image": null,
            "capabilities": {},
            "metadata": {},
        }])
    );
}

#[test]
fn execution_placement_settings_reports_invalid_config_as_validation_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        r#"
[profiles.container]
label = "Container"
mode = "local_container"
"#,
    )
    .expect("write profiles");

    let response = execution_placement_settings(&settings);

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["config"]["loaded"], json!(false));
    assert_eq!(
        response.body["validation_errors"][0],
        json!({
            "field": "profiles.container.image",
            "message": "image is required for local_container profiles",
            "profile_id": "container"
        })
    );
}

fn settings(root: &std::path::Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("project"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("spark-home/flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
