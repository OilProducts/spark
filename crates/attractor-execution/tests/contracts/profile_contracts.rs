use attractor_execution::{
    load_execution_profile_config, public_execution_placement_settings,
    resolve_execution_profile_by_id, ExecutionProfileConfigRoot,
};
use serde_json::json;

#[test]
fn malformed_profile_errors_redact_source_and_error_chain() {
    let temp = tempfile::tempdir().unwrap();
    let source = "[execution_profiles.local.metadata]\ncredential = pasted-secret-credential\n";
    let path = temp.path().join("spark.toml");
    std::fs::write(&path, source).unwrap();
    let settings = ExecutionProfileConfigRoot::new(temp.path());
    let error = load_execution_profile_config(&settings, None, None, None).unwrap_err();
    assert!(error.to_string().contains("near byte"));
    assert!(!format!("{error:?}").contains("pasted-secret-credential"));
    assert!(std::error::Error::source(&error).is_none());
    let response = public_execution_placement_settings(&settings);
    assert_eq!(response["config"]["loaded"], false);
    assert!(!response.to_string().contains("pasted-secret-credential"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn profile_resolution_matches_python_fixture_observations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(
        config_dir.join("spark.toml"),
        r#"
[execution_defaults]
execution_profile_id = "native-fast"

[execution_profiles.native-fast]
label = "Native Fast"
mode = "native"
capabilities = ["filesystem", "subprocess"]

[execution_profiles.container]
label = "Container"
mode = "local_container"
image = "spark-worker:compat"
enabled = false
capabilities = ["filesystem"]
[execution_profiles.container.metadata]
worker = "spark-server worker run-node"
"#,
    )
    .expect("write config");
    let settings = ExecutionProfileConfigRoot::new(&config_dir);

    let default_selection =
        resolve_execution_profile_by_id(&settings, None, None, None).expect("default selection");
    assert_eq!(default_selection.selected_profile_id, "native-fast");
    assert_eq!(default_selection.selection_source, "spark_default");
    assert_eq!(
        default_selection.profile.capabilities,
        ["filesystem", "subprocess"]
    );

    let explicit_selection =
        resolve_execution_profile_by_id(&settings, Some("native-fast"), None, None)
            .expect("explicit selection");
    assert_eq!(explicit_selection.selection_source, "explicit");

    let settings_view = public_execution_placement_settings(&settings);
    assert_eq!(
        settings_view["execution_modes"],
        json!(["native", "local_container"])
    );
    assert_eq!(settings_view["config"]["exists"], json!(true));
    assert_eq!(settings_view["config"]["loaded"], json!(true));
    assert_eq!(
        settings_view["default_execution_profile_id"],
        json!("native-fast")
    );
    assert_eq!(settings_view["profiles"][0]["id"], json!("container"));
    assert_eq!(
        settings_view["profiles"][0]["metadata"],
        json!({"worker": "spark-server worker run-node"})
    );
    assert_eq!(settings_view["profiles"][1]["id"], json!("native-fast"));
}

#[test]
fn missing_config_synthesizes_native_unless_profile_was_selected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    // Core sections alone declare no profiles, and the former document is never read.
    std::fs::write(
        config_dir.join("spark.toml"),
        "[models]\nprovider = 'codex'\n",
    )
    .unwrap();
    std::fs::write(
        config_dir.join("execution-profiles.toml"),
        "[profiles.container]\nlabel = 'Container'\nmode = 'native'\n",
    )
    .unwrap();
    let settings = ExecutionProfileConfigRoot::new(&config_dir);
    assert_eq!(
        public_execution_placement_settings(&settings)["config"]["synthesized_native_default"],
        true
    );

    let selection =
        resolve_execution_profile_by_id(&settings, None, None, None).expect("native default");
    assert_eq!(selection.selected_profile_id, "native");
    assert_eq!(selection.selection_source, "implementation_default");
    assert_eq!(selection.profile.label, "Native");

    let selection = resolve_execution_profile_by_id(&settings, None, Some("native"), None)
        .expect("a project may select the synthesized native profile");
    assert_eq!(selection.selection_source, "project_default");

    let error =
        resolve_execution_profile_by_id(&settings, Some("container"), None, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "execution profile 'container' was selected, but spark.toml has no [execution_profiles]"
    );
}

#[test]
fn invalid_local_container_image_and_field_errors_match_fixture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(
        config_dir.join("spark.toml"),
        r#"
[execution_profiles.container]
label = "Container"
mode = "local_container"
"#,
    )
    .expect("write config");
    let settings = ExecutionProfileConfigRoot::new(&config_dir);

    let error = load_execution_profile_config(&settings, Some("container"), None, None)
        .expect_err("missing image is invalid");
    assert_eq!(
        error.to_string(),
        "invalid execution profile config: execution_profiles.container.image: image is required for local_container profiles"
    );
    assert_eq!(
        error.field_errors[0].field,
        "execution_profiles.container.image"
    );
    assert_eq!(
        error.field_errors[0].profile_id.as_deref(),
        Some("container")
    );

    let settings_view = public_execution_placement_settings(&settings);
    assert_eq!(settings_view["config"]["loaded"], json!(false));
    assert_eq!(settings_view["profiles"], json!([]));
    assert_eq!(
        settings_view["validation_errors"],
        json!([{
            "field": "execution_profiles.container.image",
            "message": "image is required for local_container profiles",
            "profile_id": "container",
        }])
    );
}

#[test]
fn parser_rejects_invalid_mode_enabled_and_capability_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(
        config_dir.join("spark.toml"),
        r#"
[execution_profiles.bad_mode]
label = "Bad"
mode = "remote_worker"

[execution_profiles.bad_enabled]
label = "Bad Enabled"
mode = "native"
enabled = "yes"

[execution_profiles.bad_capabilities]
label = "Bad Capabilities"
mode = "native"
capabilities = ["filesystem", ""]
"#,
    )
    .expect("write config");
    let settings = ExecutionProfileConfigRoot::new(&config_dir);

    let error = load_execution_profile_config(&settings, None, None, None).unwrap_err();
    let fields = error
        .field_errors
        .iter()
        .map(|error| (error.field.as_str(), error.message.as_str()))
        .collect::<Vec<_>>();
    assert!(fields.contains(&(
        "execution_profiles.bad_mode.mode",
        "execution mode must be one of: native, local_container"
    )));
    assert!(fields.contains(&(
        "execution_profiles.bad_enabled.enabled",
        "enabled must be a boolean"
    )));
    assert!(fields.contains(&(
        "execution_profiles.bad_capabilities.capabilities[1]",
        "capability must be a non-empty string"
    )));
}
