use spark_common::settings::ClientPreferences;
use spark_storage::settings::{
    client_settings_path, import_client_preferences, load_or_create_client_identity,
    read_settings_document,
};

#[test]
fn import_is_revision_checked_backed_up_idempotent_and_client_scoped() {
    let root = tempfile::tempdir().unwrap();
    let first = client_settings_path(root.path(), "browser-first").unwrap();
    std::fs::create_dir_all(first.parent().unwrap()).unwrap();
    let original = "[preferences]\neditor_sidebar_width=444\n[preferences.flow_node_positions.flow.a]\nx=8.0\ny=9.0\n";
    std::fs::write(&first, original).unwrap();
    let incoming: ClientPreferences = serde_json::from_value(serde_json::json!({"editor_sidebar_width": 300, "editor_mode": "raw", "flow_node_positions": {"flow": {"a": {"x": 1, "y": 2}}, "other": {"b": {"x": 3, "y": 4}}}})).unwrap();
    assert!(import_client_preferences(&first, "absent", &incoming).is_err());
    let revision = read_settings_document(&first).unwrap().revision;
    let migrated = import_client_preferences(&first, &revision, &incoming).unwrap();
    let stored: ClientPreferences = migrated.section(&first, "preferences").unwrap().unwrap();
    assert_eq!(stored.editor_sidebar_width, Some(444));
    assert_eq!(
        stored.flow_node_positions.as_ref().unwrap()["flow"]["a"].x,
        8.0
    );
    assert_eq!(
        stored.flow_node_positions.as_ref().unwrap()["other"]["b"].x,
        3.0
    );
    assert_eq!(
        std::fs::read_to_string(first.with_file_name("browser-first.toml.browser-import-v0.bak"))
            .unwrap(),
        original
    );
    assert_eq!(
        import_client_preferences(&first, "stale", &ClientPreferences::default())
            .unwrap()
            .revision,
        migrated.revision
    );
    let second = client_settings_path(root.path(), "browser-second").unwrap();
    assert_eq!(read_settings_document(&second).unwrap().revision, "absent");
    let identity = root.path().join("desktop-client-id");
    let id = load_or_create_client_identity(&identity).unwrap();
    let desktop = client_settings_path(root.path(), &id).unwrap();
    let saved = import_client_preferences(&desktop, "absent", &incoming).unwrap();
    // Platform identity is home-owned and has no port/origin input.
    let restarted_id = load_or_create_client_identity(&identity).unwrap();
    assert_eq!(
        read_settings_document(&client_settings_path(root.path(), &restarted_id).unwrap())
            .unwrap()
            .revision,
        saved.revision
    );
}
