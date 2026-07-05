use std::path::Path;

use attractor_api::AttractorApiService;
use serde_json::json;
use spark_common::settings::SparkSettings;

#[test]
fn llm_profiles_route_returns_empty_list_when_config_absent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.list_llm_profiles();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body, json!({"profiles": []}));
}

#[test]
fn llm_profiles_route_exposes_public_profile_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("llm-profiles.toml"),
        r#"
[profiles.local]
provider = "openai_compatible"
base_url = "http://localhost:4000/v1"
models = ["local-small", "local-large"]
label = "Local"
default_model = "local-large"

[profiles.needs_key]
provider = "openai_compatible"
base_url = "http://localhost:5000/v1"
models = ["keyed"]
api_key_env = "SPARK_TEST_ABSENT_LLM_PROFILE_KEY"
"#,
    )
    .expect("write profiles");
    let service = AttractorApiService::new(settings);

    let response = service.list_llm_profiles();

    assert_eq!(response.status_code, 200);
    assert_eq!(
        response.body,
        json!({
            "profiles": [
                {
                    "id": "local",
                    "label": "Local",
                    "provider": "openai_compatible",
                    "models": ["local-small", "local-large"],
                    "default_model": "local-large",
                    "configured": true,
                },
                {
                    "id": "needs_key",
                    "label": null,
                    "provider": "openai_compatible",
                    "models": ["keyed"],
                    "default_model": null,
                    "configured": false,
                },
            ]
        })
    );
}

#[test]
fn llm_profiles_route_maps_config_errors_to_json_400() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    std::fs::write(
        settings.config_dir.join("llm-profiles.toml"),
        r#"[profiles.bad]
provider = "anthropic"
base_url = "http://localhost"
models = ["claude"]
"#,
    )
    .expect("write profiles");
    let service = AttractorApiService::new(settings);

    let response = service.list_llm_profiles();

    assert_eq!(response.status_code, 400);
    assert_eq!(
        response.body,
        json!({"detail": "LLM profile 'bad' has unsupported provider 'anthropic'; supported providers: openai_compatible."})
    );
}

fn settings(root: &Path) -> SparkSettings {
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
