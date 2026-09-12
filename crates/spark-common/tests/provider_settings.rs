use spark_common::provider_settings::{ProviderConnection, ProviderConnections};
use std::collections::BTreeMap;

#[test]
fn resolved_connections_preserve_environment_precedence_and_only_capture_references() {
    let stored = ProviderConnections(BTreeMap::from([(
        "openai".into(),
        ProviderConnection {
            base_url: Some("https://stored.example/v1".into()),
            api_key_env: Some("TEAM_SECRET".into()),
            organization: Some("stored-org".into()),
            ..Default::default()
        },
    )]));
    let env = BTreeMap::from([
        (
            "OPENAI_BASE_URL".into(),
            "https://environment.example/v1".into(),
        ),
        (
            "OPENAI_API_KEY".into(),
            "NEVER_SERIALIZE_STANDARD_SECRET".into(),
        ),
        ("TEAM_SECRET".into(), "NEVER_SERIALIZE_TEAM_SECRET".into()),
    ]);
    let snapshot = stored.resolve(&env).unwrap();
    assert_eq!(
        snapshot.0["openai"].api_key_env.as_deref(),
        Some("OPENAI_API_KEY")
    );
    assert_eq!(
        snapshot.0["openai"].base_url.as_deref(),
        Some("https://environment.example/v1")
    );
    assert!(snapshot.credential_status(&env)["openai"]);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("NEVER_SERIALIZE"));
    let mut later = env.clone();
    later.insert("OPENAI_BASE_URL".into(), "https://later.example/v1".into());
    assert_eq!(
        snapshot.execution_environment(&later)["OPENAI_BASE_URL"],
        "https://environment.example/v1"
    );
    assert_eq!(
        stored.resolve(&later).unwrap().0["openai"]
            .base_url
            .as_deref(),
        Some("https://later.example/v1")
    );
    later.remove("OPENAI_API_KEY");
    assert_eq!(
        stored
            .resolve(&later)
            .unwrap()
            .execution_environment(&later)["OPENAI_API_KEY"],
        "NEVER_SERIALIZE_TEAM_SECRET"
    );
    for url in [
        "https://user:SECRET@example.com",
        "https://example.com?key=SECRET",
        "file:///SECRET",
    ] {
        let invalid = ProviderConnections(BTreeMap::from([(
            "openai".into(),
            ProviderConnection {
                base_url: Some(url.into()),
                ..Default::default()
            },
        )]));
        assert!(!invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("SECRET"));
    }
}
