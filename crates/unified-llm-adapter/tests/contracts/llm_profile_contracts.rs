use std::collections::BTreeMap;
use std::net::TcpListener;

use serde_json::json;
use unified_llm_adapter::{
    build_openai_compatible_chat_request, get_llm_profile, load_llm_profiles, public_llm_profiles,
    public_llm_profiles_with_env, AdapterErrorKind, Client, Message, Request,
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

    let whitespace_env = BTreeMap::from([("LOCAL_LLM_API_KEY".to_string(), "   ".to_string())]);
    let profiles = public_llm_profiles_with_env(temp.path(), &whitespace_env).unwrap();
    assert_eq!(profiles[0]["configured"], json!(false));
    assert_eq!(profiles[1]["configured"], json!(true));
}

#[test]
fn profile_backed_openai_compatible_config_uses_endpoint_and_key_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        temp.path().join("llm-profiles.toml"),
        r#"
[profiles.local]
provider = "openai_compatible"
base_url = "http://localhost:4000/responses"
models = ["local-small", "local-large"]
api_key_env = "LOCAL_LLM_API_KEY"
default_model = "local-large"

[profiles.no_key]
provider = "openai_compatible"
base_url = "http://localhost:5000/v1"
models = ["no-key"]
"#,
    )
    .expect("write profiles");
    let profiles = load_llm_profiles(temp.path()).expect("profiles");
    let local = profiles.get("local").expect("local profile");
    let no_key = profiles.get("no_key").expect("no-key profile");

    let missing_env = BTreeMap::<String, String>::new();
    let error = local
        .openai_compatible_request_config_with_env(&missing_env)
        .expect_err("missing key");
    assert_eq!(
        error.to_string(),
        "LLM profile 'local' requires environment variable LOCAL_LLM_API_KEY to be non-empty."
    );

    let whitespace_env = BTreeMap::from([("LOCAL_LLM_API_KEY".to_string(), "   ".to_string())]);
    assert_eq!(
        local
            .openai_compatible_request_config_with_env(&whitespace_env)
            .unwrap_err()
            .to_string(),
        "LLM profile 'local' requires environment variable LOCAL_LLM_API_KEY to be non-empty."
    );

    let env = BTreeMap::from([(
        "LOCAL_LLM_API_KEY".to_string(),
        " profile-secret ".to_string(),
    )]);
    let config = local
        .openai_compatible_request_config_with_env(&env)
        .expect("profile config");
    assert_eq!(config.api_key.as_deref(), Some("profile-secret"));
    assert!(config.require_api_key);
    let prepared = build_openai_compatible_chat_request(
        "openai_compatible",
        &Request {
            model: "local-large".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        config,
    )
    .expect("prepared request");
    assert_eq!(prepared.url, "http://localhost:4000/v1/chat/completions");
    assert_eq!(prepared.headers["Authorization"], "Bearer profile-secret");

    let no_key_config = no_key
        .openai_compatible_request_config_with_env(&missing_env)
        .expect("local no-key profile");
    assert_eq!(no_key_config.api_key, None);
    assert!(!no_key_config.require_api_key);
}

#[test]
fn client_profile_routes_normalize_to_openai_compatible_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_url = unused_base_url();
    std::fs::write(
        temp.path().join("llm-profiles.toml"),
        format!(
            r#"
[profiles.local]
provider = "openai_compatible"
base_url = "{base_url}"
models = ["local-large"]
default_model = "local-large"
"#
        ),
    )
    .expect("write profiles");
    let env = BTreeMap::<String, String>::new();
    let client =
        Client::from_env_map_and_profiles(&env, temp.path(), Some("LOCAL")).expect("client");

    assert_eq!(client.default_provider(), Some("local"));
    let profile = client.require_llm_profile("LOCAL").expect("profile route");
    assert_eq!(profile.id, "local");
    assert_eq!(profile.provider, "openai_compatible");
    assert_eq!(profile.default_model.as_deref(), Some("local-large"));
    assert_eq!(
        client.routed_provider_for_selector("LOCAL").as_deref(),
        Some("openai_compatible")
    );

    let error = client
        .complete(Request {
            model: "local-large".to_string(),
            messages: vec![Message::user("hello")],
            provider: Some("LOCAL".to_string()),
            ..Request::default()
        })
        .expect_err("local transport should attempt HTTP");
    assert_eq!(error.kind, AdapterErrorKind::Network);
    assert_eq!(error.provider.as_deref(), Some("openai_compatible"));
    assert!(!error.message.contains("no HTTP transport is configured"));
}

#[test]
fn client_profile_routes_return_clear_missing_key_errors_when_selected() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        temp.path().join("llm-profiles.toml"),
        r#"
[profiles.local]
provider = "openai_compatible"
base_url = "http://localhost:4000/v1"
models = ["local-large"]
api_key_env = "LOCAL_LLM_API_KEY"
default_model = "local-large"
"#,
    )
    .expect("write profiles");
    let env = BTreeMap::<String, String>::new();
    let client = Client::from_env_map_and_profiles(&env, temp.path(), None).expect("client");

    let error = client
        .complete(Request {
            model: "local-large".to_string(),
            messages: vec![Message::user("hello")],
            provider: Some("local".to_string()),
            ..Request::default()
        })
        .expect_err("missing profile key");

    assert_eq!(error.kind, AdapterErrorKind::Configuration);
    assert_eq!(
        error.message,
        "LLM profile 'local' requires environment variable LOCAL_LLM_API_KEY to be non-empty."
    );
    assert!(!error.message.contains("localhost"));
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
    assert_profile_error(
        r#"[profiles.bad]
base_url = "http://localhost"
models = ["one"]
"#,
        "LLM profile 'bad' provider is required.",
    );
    assert_profile_error(
        r#"[profiles.bad]
provider = "openai_compatible"
models = ["one"]
"#,
        "LLM profile 'bad' base_url is required.",
    );
    assert_profile_error(
        r#"[profiles.bad]
provider = "openai_compatible"
base_url = "http://localhost"
models = ["one", "  "]
"#,
        "LLM profile 'bad' model is required.",
    );
    assert_profile_error(
        r#"[profiles.bad]
provider = "openai_compatible"
base_url = "http://localhost"
models = ["one"]
headers = { "X-Debug" = "true" }
"#,
        "LLM profile 'bad' has unsupported key 'headers'; supported keys: api_key_env, base_url, default_model, label, models, provider.",
    );
    assert_profile_error(
        r#"[profiles.""]
provider = "openai_compatible"
base_url = "http://localhost"
models = ["one"]
"#,
        "profile id \"\" is required.",
    );
    assert_profile_error(
        r#"[profiles]
bad = "not a table"
"#,
        "LLM profile 'bad' must be a table.",
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
    let error = public_llm_profiles(temp.path()).unwrap_err();
    assert_eq!(error.to_string(), expected);

    let adapter_error: unified_llm_adapter::AdapterError = error.into();
    assert_eq!(adapter_error.kind, AdapterErrorKind::Configuration);
    assert_eq!(adapter_error.message, expected);
    assert!(!adapter_error.retryable);
}

fn unused_base_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("unused local listener");
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    url
}
