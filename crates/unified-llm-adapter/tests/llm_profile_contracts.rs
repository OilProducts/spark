use std::collections::BTreeMap;

use serde_json::json;
use unified_llm_adapter::{
    get_llm_profile, load_llm_profiles, public_llm_profiles, public_llm_profiles_with_env,
};

#[test]
fn missing_config_returns_empty_public_profiles() {
    let temp = tempfile::tempdir().expect("tempdir");

    assert!(load_llm_profiles(temp.path()).unwrap().is_empty());
    assert_eq!(
        public_llm_profiles(temp.path()).unwrap(),
        Vec::<serde_json::Value>::new()
    );
}

#[test]
fn valid_profile_public_metadata_and_env_configured_flags_match_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        temp.path().join("llm-profiles.toml"),
        r#"
[profiles.local]
provider = "openai_compatible"
base_url = "http://localhost:4000/v1"
models = ["local-small", "local-large"]
label = "Local"
api_key_env = "LOCAL_LLM_API_KEY"
default_model = "local-large"

[profiles.no_key]
provider = "openai_compatible"
base_url = "http://localhost:5000/v1"
models = ["no-key"]
"#,
    )
    .expect("write profiles");
    let env = BTreeMap::from([("LOCAL_LLM_API_KEY".to_string(), "secret".to_string())]);

    let profiles = public_llm_profiles_with_env(temp.path(), &env).unwrap();

    assert_eq!(
        profiles,
        vec![
            json!({
                "id": "local",
                "label": "Local",
                "provider": "openai_compatible",
                "models": ["local-small", "local-large"],
                "default_model": "local-large",
                "configured": true,
            }),
            json!({
                "id": "no_key",
                "label": null,
                "provider": "openai_compatible",
                "models": ["no-key"],
                "default_model": null,
                "configured": true,
            }),
        ]
    );

    let empty_env = BTreeMap::<String, String>::new();
    let profiles = public_llm_profiles_with_env(temp.path(), &empty_env).unwrap();
    assert_eq!(profiles[0]["configured"], json!(false));
    assert_eq!(profiles[1]["configured"], json!(true));
}

#[test]
fn profile_parser_reports_compatible_errors() {
    assert_profile_error(
        r#"[profiles.bad]
provider = "anthropic"
base_url = "http://localhost"
models = ["claude"]
"#,
        "LLM profile 'bad' has unsupported provider 'anthropic'; supported providers: openai_compatible.",
    );
    assert_profile_error(
        r#"[profiles.bad]
provider = "openai_compatible"
base_url = "http://localhost"
"#,
        "LLM profile 'bad' models must be a non-empty list.",
    );
    assert_profile_error(
        r#"[profiles.bad]
provider = "openai_compatible"
base_url = "http://localhost"
models = []
"#,
        "LLM profile 'bad' must declare at least one model.",
    );
    assert_profile_error(
        r#"[profiles.bad]
provider = "openai_compatible"
base_url = "http://localhost"
models = ["one"]
default_model = "two"
"#,
        "LLM profile 'bad' default_model 'two' is not listed in models.",
    );
}

#[test]
fn missing_profile_and_malformed_toml_errors_match_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        temp.path().join("llm-profiles.toml"),
        r#"[profiles.good]
provider = "openai_compatible"
base_url = "http://localhost"
models = ["one"]
"#,
    )
    .expect("write profiles");

    assert_eq!(
        get_llm_profile(temp.path(), "missing")
            .unwrap_err()
            .to_string(),
        "LLM profile 'missing' was not found."
    );

    let malformed = tempfile::tempdir().expect("tempdir");
    std::fs::write(malformed.path().join("llm-profiles.toml"), "[profiles.")
        .expect("write malformed");
    assert!(public_llm_profiles(malformed.path())
        .unwrap_err()
        .to_string()
        .starts_with("Invalid LLM profile config: "));
}

#[test]
fn profiles_key_must_be_a_table_when_present() {
    assert_profile_error(
        r#"profiles = "invalid"
"#,
        "LLM profile config must contain a [profiles] table.",
    );
}

fn assert_profile_error(config: &str, expected: &str) {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("llm-profiles.toml"), config).expect("write profiles");
    assert_eq!(
        public_llm_profiles(temp.path()).unwrap_err().to_string(),
        expected
    );
}
