use spark_common::settings::{
    resolve_model_settings, resolve_settings_with_persisted, ModelSettings, ModelSettingsSource,
    RuntimeSettings, SettingsOverrides,
};
use std::collections::BTreeMap;

fn selection(provider: &str, model: Option<&str>) -> ModelSettings {
    ModelSettings {
        provider: Some(provider.into()),
        llm_profile: None,
        model: model.map(Into::into),
        reasoning_effort: None,
    }
}

#[test]
fn model_groups_inherit_without_filling_omitted_override_fields() {
    let workspace = selection("openai", Some("workspace-model"));
    let project = selection("anthropic", None);
    let conversation = selection("openai", Some("conversation-model"));
    assert_eq!(
        resolve_model_settings(&workspace, Some(&project), Some(&conversation)).unwrap(),
        (&conversation, ModelSettingsSource::Conversation)
    );
    assert_eq!(
        resolve_model_settings(&workspace, Some(&project), None).unwrap(),
        (&project, ModelSettingsSource::Project)
    );
    assert_eq!(
        resolve_model_settings(&workspace, None, None).unwrap(),
        (&workspace, ModelSettingsSource::Workspace)
    );
    let mut invalid = project.clone();
    invalid.llm_profile = Some("profile".into());
    assert!(resolve_model_settings(&workspace, Some(&invalid), None).is_err());
    invalid.provider = None;
    assert!(invalid.validate().is_ok());
    invalid.model = Some(" ".into());
    assert!(invalid.validate().is_err());
}

#[test]
fn runtime_choices_keep_cli_environment_persisted_default_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let overrides = SettingsOverrides {
        data_dir: Some(root.join("home")),
        ..Default::default()
    };
    let persisted = RuntimeSettings {
        flows_dir: Some(root.join("persisted-flows")),
        runs_dir: Some(root.join("persisted-runs")),
        ..Default::default()
    };
    let mut env = BTreeMap::new();
    let settings = resolve_settings_with_persisted(&overrides, &env, &persisted).unwrap();
    assert_eq!(settings.flows_dir, persisted.flows_dir.clone().unwrap());
    assert_eq!(settings.runs_dir, persisted.runs_dir.unwrap());
    assert_eq!(settings.config_dir, root.join("home/config"));
    env.insert(
        "SPARK_FLOWS_DIR".into(),
        root.join("env-flows").to_string_lossy().into_owned(),
    );
    let settings =
        resolve_settings_with_persisted(&overrides, &env, &RuntimeSettings::default()).unwrap();
    assert_eq!(settings.flows_dir, root.join("env-flows"));
    let overrides = SettingsOverrides {
        flows_dir: Some(root.join("cli-flows")),
        ..overrides
    };
    assert_eq!(
        resolve_settings_with_persisted(&overrides, &env, &RuntimeSettings::default())
            .unwrap()
            .flows_dir,
        root.join("cli-flows")
    );
}

#[test]
fn invalid_persisted_runtime_paths_are_rejected_even_when_overridden() {
    let overrides = SettingsOverrides {
        flows_dir: Some("/override".into()),
        ..Default::default()
    };
    let env = BTreeMap::new();
    let empty = RuntimeSettings {
        flows_dir: Some("".into()),
        ..Default::default()
    };
    assert!(resolve_settings_with_persisted(&overrides, &env, &empty).is_err());
    let relative = RuntimeSettings {
        project_roots: vec!["relative/project".into()],
        ..Default::default()
    };
    assert!(resolve_settings_with_persisted(&overrides, &env, &relative).is_err());
}
