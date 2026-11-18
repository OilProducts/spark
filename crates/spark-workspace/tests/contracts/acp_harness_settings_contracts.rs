use std::fs;

use serde_json::{json, Value};
use spark_common::settings::{resolve_settings_with_env, SettingsOverrides};
use spark_storage::settings::read_settings_document;
use spark_workspace::settings::{update_workspace_settings, workspace_settings};

#[test]
fn acp_harnesses_save_one_at_a_time_keep_hidden_values_and_a_misconfigured_one_stops_no_other() {
    use spark_common::agent_settings::HIDDEN_ENV_VALUE as HIDDEN;
    let temp = tempfile::tempdir().unwrap();
    let settings = resolve_settings_with_env(
        &SettingsOverrides {
            data_dir: Some(temp.path().join("home")),
            ..Default::default()
        },
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    let path = settings.config_dir.join("spark.toml");
    let store = super::shared_store(&settings.data_dir);
    let save = |section: &str, value: Value| {
        let revision = read_settings_document(&path).unwrap().revision;
        update_workspace_settings(
            &settings,
            &store,
            serde_json::from_value(
                json!({"section": section, "expected_revision": revision, "value": value}),
            )
            .unwrap(),
        )
    };
    let stored = |id: &str| -> Value {
        json!(read_settings_document(&path).unwrap().values["agents"]["acp_harnesses"][id])
    };
    let agent = json!({"name": "Agent", "executable": "/bin/agent", "args": ["--acp"],
        "env": {"AGENT_TOKEN": "token-sentinel"},
        "system_instructions": {"env": {"AGENT_PROMPT": {"text": "prompt-sentinel {{instructions}}"}}}});
    save("acp_harness", json!({"id": "agent", "registration": agent})).unwrap();
    let view = workspace_settings(&settings).unwrap();
    assert!(!view.to_string().contains("token-sentinel"));
    assert!(!view.to_string().contains("prompt-sentinel"));
    assert!(view["agents"]["stored"].get("acp_harnesses").is_none());
    let shown = &view["agents"]["acp_harnesses"]["agent"];
    assert!(shown["error"].is_null());
    assert_eq!(shown["registration"]["env"]["AGENT_TOKEN"], HIDDEN);
    assert_eq!(
        shown["registration"]["system_instructions"]["env"]["AGENT_PROMPT"],
        json!({"text": HIDDEN})
    );

    // Saved back as shown, edited or alongside other agents settings, the
    // hidden values survive unchanged.
    let mut edited = shown["registration"].clone();
    edited["name"] = json!("Renamed");
    save(
        "acp_harness",
        json!({"id": "agent", "registration": edited}),
    )
    .unwrap();
    let mut limits = view["agents"]["stored"].clone();
    limits["max_turns"] = json!(5);
    save("agents", limits).unwrap();
    let kept = stored("agent");
    assert_eq!(kept["name"], "Renamed");
    assert_eq!(kept["env"]["AGENT_TOKEN"], "token-sentinel");
    assert_eq!(
        kept["system_instructions"]["env"]["AGENT_PROMPT"],
        json!({"text": "prompt-sentinel {{instructions}}"})
    );
    let mut unknown = edited.clone();
    unknown["env"]["OTHER"] = json!(HIDDEN);
    assert!(save(
        "acp_harness",
        json!({"id": "agent", "registration": unknown})
    )
    .is_err());
    assert!(save("agents", json!({"acp_harnesses": {"other": agent}})).is_err());
    assert_eq!(stored("agent"), kept);

    // Hand-edited registrations Spark cannot use are set apart, with why,
    // and can be fixed or removed in place.
    let text = fs::read_to_string(&path).unwrap()
        + "\n[agents.acp_harnesses.reserved]\nname = 'Reserved'\nexecutable = 'agent'\n\
           [agents.acp_harnesses.reserved.env]\nSPARK_HOME = 'reserved-sentinel'\n\
           [agents.acp_harnesses.typo]\nname = 'Typo'\nexectuable = 'agent'\n\
           [agents.acp_harnesses.wrongtype]\nname = 'Wrong Type'\nexecutable = 'agent'\nargs = '--token args-sentinel'\n";
    fs::write(&path, text).unwrap();
    let configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert!(configuration.agents.acp_harnesses.contains_key("agent"));
    let misconfigured = &configuration.misconfigured_acp_harnesses;
    assert!(misconfigured["reserved"].error.contains("SPARK_HOME"));
    assert!(misconfigured["typo"]
        .error
        .contains("unknown field `exectuable`"));
    assert_eq!(misconfigured["typo"].name, "Typo");
    assert_eq!(
        misconfigured["wrongtype"].error,
        "`args` has the wrong type: expected a sequence, found string."
    );
    let view = workspace_settings(&settings).unwrap();
    assert!(!view.to_string().contains("reserved-sentinel"));
    assert_eq!(view["agents"]["validation_errors"], json!([]));
    let harnesses = &view["agents"]["acp_harnesses"];
    assert!(harnesses["reserved"]["error"]
        .as_str()
        .unwrap()
        .contains("SPARK_HOME"));
    let mut fixed = harnesses["typo"]["registration"].clone();
    fixed.as_object_mut().unwrap().remove("exectuable");
    fixed["executable"] = json!("agent");
    save("acp_harness", json!({"id": "typo", "registration": fixed})).unwrap();
    save(
        "acp_harness",
        json!({"id": "reserved", "registration": null}),
    )
    .unwrap();
    let view = workspace_settings(&settings).unwrap();
    let harnesses = view["agents"]["acp_harnesses"].as_object().unwrap();
    assert_eq!(
        harnesses.keys().collect::<Vec<_>>(),
        ["agent", "typo", "wrongtype"],
        "{harnesses:?}"
    );
    assert!(harnesses["typo"]["error"].is_null());
    assert_eq!(stored("agent"), kept);

    // An agents section that can't be read still shows each registration's
    // problem, and its repair resets only what can't be read.
    let mut limits = view["agents"]["stored"].clone();
    limits["max_tool_rounds_per_input"] = json!(7);
    save("agents", limits).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("max_turns = 5"), "{text}");
    fs::write(&path, text.replace("max_turns = 5", "max_turns = 'nine'")).unwrap();
    let view = workspace_settings(&settings).unwrap();
    assert!(view["agents"]["validation_errors"][0]
        .as_str()
        .unwrap()
        .contains("max_turns"));
    assert!(view["agents"]["acp_harnesses"]["wrongtype"]["error"]
        .as_str()
        .unwrap()
        .contains("`args`"));
    let repair = view["agents"]["repair_defaults"].clone();
    assert_eq!(repair["max_turns"], 0);
    assert_eq!(repair["max_tool_rounds_per_input"], 7);
    save("agents", repair).unwrap();
    assert_eq!(stored("agent"), kept);
    assert_eq!(
        workspace_settings(&settings).unwrap()["agents"]["validation_errors"],
        json!([])
    );
}
