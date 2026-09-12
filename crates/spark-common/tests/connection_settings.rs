use spark_common::settings::ConnectionSettings;
use std::collections::BTreeMap;

#[test]
fn connection_precedence_and_secret_redaction() {
    let stored = ConnectionSettings {
        server_host: Some("localhost".into()),
        server_port: Some(8123),
        client_api_base_url: Some("https://spark.example".into()),
    };
    let empty = BTreeMap::new();
    assert_eq!(stored.resolve(&empty, None, None).unwrap(), stored);
    let env = BTreeMap::from([
        ("SPARK_HOST".into(), "127.0.0.2".into()),
        ("SPARK_PORT".into(), "9123".into()),
        (
            "SPARK_API_BASE_URL".into(),
            "https://environment.example".into(),
        ),
    ]);
    let resolved = stored.resolve(&env, Some("127.0.0.3"), Some(0)).unwrap();
    assert_eq!(resolved.server_host.as_deref(), Some("127.0.0.3"));
    assert_eq!(resolved.server_port, Some(0));
    assert_eq!(
        resolved.client_api_base_url.as_deref(),
        Some("https://environment.example")
    );
    assert_eq!(
        stored.resolve(&env, None, None).unwrap().server_port,
        Some(9123)
    );
    let malformed_env = BTreeMap::from([("SPARK_PORT".into(), "invalid".into())]);
    assert!(stored.resolve(&malformed_env, None, None).is_err());
    assert!(stored.resolve(&malformed_env, None, Some(8000)).is_ok());
    for value in [
        "https://name:SECRET@example.com",
        "https://example.com?key=SECRET",
        "https://example.com#SECRET",
        "file:///SECRET",
    ] {
        let invalid = ConnectionSettings {
            client_api_base_url: Some(value.into()),
            ..Default::default()
        };
        let error = invalid.validate().unwrap_err().to_string();
        assert!(!error.contains("SECRET"));
    }
}
