use std::fs;
use std::path::Path;

use spark_common::project::{definitions_dir, state_dir};
use spark_workspace::initialize_project;

fn seed_version(definitions: &Path) -> String {
    fs::read_to_string(definitions.join("spec/README.md"))
        .expect("spec")
        .lines()
        .find_map(|line| line.strip_prefix("version: "))
        .expect("seed version")
        .to_string()
}

#[test]
fn initialize_project_installs_the_default_definitions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let setup = initialize_project(temp.path()).expect("initialize");
    let definitions = definitions_dir(temp.path());
    assert_eq!(setup.definitions_dir, definitions);
    assert!(!setup.already_initialized);
    assert!(setup.skipped.is_empty(), "{:?}", setup.skipped);

    for asset in spark_assets::flows::starter_flow_assets().expect("flows") {
        let installed = fs::read_to_string(definitions.join("flows").join(&asset.name))
            .unwrap_or_else(|_| panic!("flow file {}", asset.name));
        assert_eq!(installed, asset.content);
    }
    assert!(definitions
        .join("flows/organization/review-change/flow.md")
        .is_file());
    for (file, _) in spark_assets::roles::assets() {
        assert!(definitions.join("roles").join(&file).is_file(), "{file}");
    }
    assert!(definitions.join("playbooks/bug-report.md").is_file());
    assert!(!definitions.join("playbooks/agent-org-setup.md").exists());
    assert!(!definitions.join("playbook.md").exists());
    assert!(!definitions.join("triggers").exists());
    for path in [
        "spec/README.md",
        "profiles/software/spec.md",
        "profiles/operations/spec.md",
        "profiles/external-effects/spec.md",
        "standards/README.md",
        "templates/README.md",
        "decisions/0001-vendor-established-standards.md",
    ] {
        assert!(definitions.join(path).is_file(), "{path}");
    }
    assert!(setup.created.contains(&"README.md".to_string()));
    assert!(!temp.path().join(".spark/state").exists());
    assert!(!temp.path().join(".spark/.gitignore").exists());

    let seed = seed_version(&definitions);
    let spark = env!("CARGO_PKG_VERSION");
    for record in ["README.md", "CHANGELOG.md"] {
        let text = fs::read_to_string(definitions.join(record)).expect("record");
        assert!(!text.contains("{{"), "{text}");
        assert!(text.contains(&format!("Spark {spark}")), "{text}");
        assert!(text.contains(&format!("seed {seed}")), "{text}");
        assert!(
            text.contains("external-effects, operations, software"),
            "{text}"
        );
    }
    let marker = fs::read_to_string(definitions.join(".initialized")).expect("marker");
    assert!(marker.contains(spark) && marker.contains(&seed), "{marker}");
}

#[test]
fn initialize_project_keeps_custom_files_and_retries_a_partial_copy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let definitions = definitions_dir(temp.path());
    fs::create_dir_all(definitions.join("roles")).expect("roles");
    fs::write(definitions.join("roles/coordinator.md"), "custom").expect("custom role");
    fs::write(definitions.join("README.md"), "custom readme").expect("custom readme");

    let setup = initialize_project(temp.path()).expect("initialize");
    assert_eq!(
        setup.skipped,
        vec!["README.md".to_string(), "roles/coordinator.md".to_string()]
    );
    assert_eq!(
        fs::read_to_string(definitions.join("roles/coordinator.md")).unwrap(),
        "custom"
    );
    assert_eq!(
        fs::read_to_string(definitions.join("README.md")).unwrap(),
        "custom readme"
    );

    // An interrupted copy leaves no marker; the next run fills in what is missing.
    fs::remove_file(definitions.join(".initialized")).expect("marker");
    fs::remove_file(definitions.join("spec/README.md")).expect("spec");
    let retry = initialize_project(temp.path()).expect("retry");
    assert_eq!(retry.created, vec!["spec/README.md".to_string()]);
    assert!(definitions.join("spec/README.md").is_file());
    assert!(definitions.join(".initialized").is_file());
}

#[test]
fn initialize_project_installs_nothing_once_initialized() {
    let temp = tempfile::tempdir().expect("tempdir");
    let definitions = definitions_dir(temp.path());
    initialize_project(temp.path()).expect("initialize");
    fs::remove_file(definitions.join("roles/coordinator.md")).expect("remove role");
    fs::remove_dir_all(definitions.join("flows")).expect("remove flows");
    let state = state_dir(temp.path()).expect("state");
    fs::remove_dir_all(&state).expect("clear state");

    let again = initialize_project(temp.path()).expect("again");
    assert!(again.already_initialized);
    assert!(again.created.is_empty() && again.skipped.is_empty());
    assert!(!definitions.join("roles/coordinator.md").exists());
    assert!(!definitions.join("flows").exists());

    // The marker alone, e.g. left by an earlier Spark, also stops setup.
    let other = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(definitions_dir(other.path())).expect("definitions");
    fs::write(definitions_dir(other.path()).join(".initialized"), "").expect("marker");
    assert!(
        initialize_project(other.path())
            .expect("marked")
            .already_initialized
    );
    assert_eq!(
        fs::read_dir(definitions_dir(other.path())).unwrap().count(),
        1
    );
}

#[test]
fn initialize_project_rejects_a_missing_or_file_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let missing = temp.path().join("missing");
    assert!(initialize_project(&missing).is_err());
    assert!(!missing.exists());
    let file = temp.path().join("file");
    fs::write(&file, "").expect("file");
    assert!(initialize_project(&file).is_err());
}

#[test]
fn state_dir_ignores_all_working_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = state_dir(temp.path()).expect("state");
    assert_eq!(state, temp.path().join(".spark/state"));
    assert_eq!(fs::read_to_string(state.join(".gitignore")).unwrap(), "*\n");
    fs::write(state.join(".gitignore"), "custom\n").expect("custom ignore");
    state_dir(temp.path()).expect("again");
    assert_eq!(
        fs::read_to_string(state.join(".gitignore")).unwrap(),
        "custom\n"
    );
    assert!(!temp.path().join(".spark/.gitignore").exists());
}
