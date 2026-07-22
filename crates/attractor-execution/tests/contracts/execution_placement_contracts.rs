#[test]
fn profile_metadata_declares_extra_container_mounts() {
    use attractor_execution::ExecutionProfile;
    let mut profile = ExecutionProfile::implementation_native();
    profile.metadata.insert(
        "container.mounts".to_string(),
        serde_json::json!(["/home/user/.codex:/home/user/.codex:ro"]),
    );
    let mounts = attractor_execution::profile_mounts_for_test(&profile).expect("valid mounts");
    assert_eq!(
        mounts,
        vec!["/home/user/.codex:/home/user/.codex:ro".to_string()]
    );

    profile.metadata.insert(
        "container.mounts".to_string(),
        serde_json::json!(["not-a-mount"]),
    );
    assert!(attractor_execution::profile_mounts_for_test(&profile).is_err());
}
