use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, Serializer};
use serde_json::json;
use spark_storage::{
    append_jsonl, read_json, read_jsonl, read_toml, write_json_atomic, write_toml_atomic,
    AppendLogRepository, DocumentRepository, JsonLinesOptions, JsonRepository, JsonWriteOptions,
    JsonlRepository, KnownFields, StorageError, StorageFormat, TomlRepository, UnknownFieldPolicy,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DemoDocument {
    schema_version: u8,
    name: String,
    enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JournalRecord {
    sequence: u64,
    label: String,
}

struct FailingSerialize;

impl Serialize for FailingSerialize {
    fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(serde::ser::Error::custom(
            "intentional serialization failure",
        ))
    }
}

struct DemoKnownFields;

impl KnownFields for DemoKnownFields {
    fn known_fields() -> &'static [&'static str] {
        &["enabled", "name", "schema_version"]
    }
}

#[test]
fn json_helpers_round_trip_typed_records_and_generic_values() {
    let temp = tempfile::tempdir().expect("tempdir");
    let document_path = temp.path().join("nested/demo.json");
    let value_path = temp.path().join("value.json");
    let document = DemoDocument {
        schema_version: 1,
        name: "demo".to_string(),
        enabled: true,
    };

    write_json_atomic(&document_path, &document, JsonWriteOptions::default()).expect("write json");
    let loaded: DemoDocument = read_json(&document_path).expect("read typed json");
    assert_eq!(loaded, document);

    let value = json!({
        "schema_version": 1,
        "name": "generic",
        "items": [{"id": "first"}]
    });
    write_json_atomic(&value_path, &value, JsonWriteOptions::default()).expect("write value json");
    let loaded: serde_json::Value = read_json(&value_path).expect("read value json");
    assert_eq!(loaded, value);
}

#[test]
fn toml_helpers_round_trip_typed_records_and_generic_values() {
    let temp = tempfile::tempdir().expect("tempdir");
    let document_path = temp.path().join("nested/demo.toml");
    let value_path = temp.path().join("value.toml");
    let document = DemoDocument {
        schema_version: 1,
        name: "demo".to_string(),
        enabled: true,
    };

    write_toml_atomic(&document_path, &document).expect("write toml");
    let loaded: DemoDocument = read_toml(&document_path).expect("read typed toml");
    assert_eq!(loaded, document);

    let mut table = toml::map::Map::new();
    table.insert("schema_version".to_string(), toml::Value::Integer(1));
    table.insert(
        "name".to_string(),
        toml::Value::String("generic".to_string()),
    );
    table.insert("enabled".to_string(), toml::Value::Boolean(true));
    let value = toml::Value::Table(table);
    write_toml_atomic(&value_path, &value).expect("write value toml");
    let loaded: toml::Value = read_toml(&value_path).expect("read value toml");
    assert_eq!(loaded, value);
}

#[test]
fn jsonl_helpers_preserve_append_order_and_parse_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let journal_path = temp.path().join("events/journal.jsonl");
    let first = JournalRecord {
        sequence: 1,
        label: "first".to_string(),
    };
    let second = JournalRecord {
        sequence: 2,
        label: "second".to_string(),
    };

    append_jsonl(&journal_path, &first).expect("append first");
    append_jsonl(&journal_path, &second).expect("append second");
    let records: Vec<JournalRecord> =
        read_jsonl(&journal_path, JsonLinesOptions::strict()).expect("read jsonl");
    assert_eq!(records, vec![first.clone(), second.clone()]);

    std::fs::write(
        &journal_path,
        "{\"sequence\":1,\"label\":\"first\"}\n\n{\"sequence\":2,\"label\":\"second\"}\n",
    )
    .expect("write blank line fixture");
    let tolerant: Vec<JournalRecord> =
        read_jsonl(&journal_path, JsonLinesOptions::allow_blank_lines())
            .expect("read blank-line tolerant jsonl");
    assert_eq!(tolerant, vec![first, second]);

    let error =
        read_jsonl::<JournalRecord>(&journal_path, JsonLinesOptions::strict()).expect_err("strict");
    assert!(matches!(error, StorageError::JsonlLine { line: 2, .. }));

    std::fs::write(
        &journal_path,
        "{\"sequence\":1,\"label\":\"first\"}\nnot-json\n",
    )
    .expect("write invalid line");
    let error = read_jsonl::<JournalRecord>(&journal_path, JsonLinesOptions::strict())
        .expect_err("invalid");
    assert!(matches!(error, StorageError::JsonlLine { line: 2, .. }));
}

#[test]
fn atomic_document_writes_preserve_destination_when_serialization_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let json_path = temp.path().join("state.json");
    let toml_path = temp.path().join("state.toml");
    std::fs::write(&json_path, "{\"original\":true}\n").expect("seed json");
    std::fs::write(&toml_path, "original = true\n").expect("seed toml");

    let json_error = write_json_atomic(&json_path, &FailingSerialize, JsonWriteOptions::default())
        .expect_err("json serialization");
    assert!(matches!(json_error, StorageError::JsonWrite { .. }));
    assert_eq!(
        std::fs::read_to_string(&json_path).expect("json after failure"),
        "{\"original\":true}\n"
    );

    let toml_error =
        write_toml_atomic(&toml_path, &FailingSerialize).expect_err("toml serialization");
    assert!(matches!(toml_error, StorageError::TomlWrite { .. }));
    assert_eq!(
        std::fs::read_to_string(&toml_path).expect("toml after failure"),
        "original = true\n"
    );
}

#[test]
fn append_helpers_preserve_existing_records_when_serialization_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let journal_path = temp.path().join("journal.jsonl");
    let existing = JournalRecord {
        sequence: 1,
        label: "existing".to_string(),
    };
    append_jsonl(&journal_path, &existing).expect("append existing");

    let error = append_jsonl(&journal_path, &FailingSerialize).expect_err("append failure");
    assert!(matches!(error, StorageError::JsonWrite { .. }));

    let records: Vec<JournalRecord> =
        read_jsonl(&journal_path, JsonLinesOptions::strict()).expect("read after failure");
    assert_eq!(records, vec![existing]);
}

#[test]
fn unknown_field_policies_allow_deny_and_collect_top_level_fields() {
    let json_value = json!({
        "schema_version": 1,
        "name": "demo",
        "enabled": true,
        "z_extra": true,
        "a_extra": false
    });

    let allowed = spark_storage::validate_json_object_fields::<DemoKnownFields>(
        "demo.json",
        &json_value,
        UnknownFieldPolicy::Allow,
    )
    .expect("allow unknown fields");
    assert!(allowed.is_empty());

    let collected = spark_storage::validate_json_object_fields::<DemoKnownFields>(
        "demo.json",
        &json_value,
        UnknownFieldPolicy::Collect,
    )
    .expect("collect unknown fields");
    assert_eq!(collected.unknown_fields(), ["a_extra", "z_extra"]);

    let denied = spark_storage::validate_json_object_fields::<DemoKnownFields>(
        "demo.json",
        &json_value,
        UnknownFieldPolicy::Deny,
    )
    .expect_err("deny unknown fields");
    assert!(matches!(
        denied,
        StorageError::UnknownFields { fields, .. } if fields == vec!["a_extra".to_string(), "z_extra".to_string()]
    ));

    let toml_value: toml::Value = r#"
schema_version = 1
name = "demo"
enabled = true
z_extra = true
a_extra = false
"#
    .parse()
    .expect("toml value");
    let collected = spark_storage::validate_toml_table_fields::<DemoKnownFields>(
        "demo.toml",
        &toml_value,
        UnknownFieldPolicy::Collect,
    )
    .expect("collect toml unknown fields");
    assert_eq!(collected.unknown_fields(), ["a_extra", "z_extra"]);
}

#[test]
fn repository_adapters_use_public_codecs_and_expose_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let document = DemoDocument {
        schema_version: 1,
        name: "repo".to_string(),
        enabled: true,
    };

    let json_repo = JsonRepository::<DemoDocument>::new(temp.path().join("state/demo.json"))
        .expect("json repo");
    assert_eq!(json_repo.format(), StorageFormat::Json);
    assert_eq!(json_repo.path(), temp.path().join("state/demo.json"));
    assert_eq!(json_repo.read_optional().expect("missing optional"), None);
    json_repo.write(&document).expect("write json repo");
    assert_eq!(json_repo.read().expect("read json repo"), document);

    let toml_repo = TomlRepository::<DemoDocument>::new(temp.path().join("state/demo.toml"))
        .expect("toml repo");
    assert_eq!(toml_repo.format(), StorageFormat::Toml);
    toml_repo.write(&document).expect("write toml repo");
    assert_eq!(toml_repo.read().expect("read toml repo"), document);

    let jsonl_repo = JsonlRepository::<JournalRecord>::new(temp.path().join("state/journal.jsonl"))
        .expect("jsonl repo")
        .with_read_options(JsonLinesOptions::allow_blank_lines());
    assert_eq!(jsonl_repo.format(), StorageFormat::Jsonl);
    jsonl_repo
        .append(&JournalRecord {
            sequence: 1,
            label: "first".to_string(),
        })
        .expect("append repo");
    std::fs::write(
        jsonl_repo.path(),
        "{\"sequence\":1,\"label\":\"first\"}\n\n{\"sequence\":2,\"label\":\"second\"}\n",
    )
    .expect("insert blank line");
    let records = jsonl_repo.read_all().expect("read repo jsonl");
    assert_eq!(records.len(), 2);
    assert_eq!(StorageFormat::Json.extension(), "json");
}

#[test]
fn m0_filesystem_fixture_samples_load_as_generic_values_and_rewrite_through_codecs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture_dir = repo_root().join("tests/compat/fixtures/filesystem");

    let layout: serde_json::Value =
        read_json(fixture_dir.join("project-conversation-layout.json")).expect("layout fixture");
    let handles = layout["durable_state"]["after"]["conversation_handles"]["data"].clone();
    assert!(handles.is_object());
    let handles_path = temp.path().join("conversation-handles.json");
    write_json_atomic(&handles_path, &handles, JsonWriteOptions::default())
        .expect("rewrite handles");
    let loaded_handles: serde_json::Value = read_json(&handles_path).expect("read handles");
    assert_eq!(loaded_handles["schema_version"], 1);

    let project_data = layout["durable_state"]["after"]["project"]["data"].clone();
    let project_toml = toml::Value::try_from(project_data).expect("project toml value");
    let project_path = temp.path().join("project.toml");
    write_toml_atomic(&project_path, &project_toml).expect("rewrite project");
    let loaded_project: toml::Value = read_toml(&project_path).expect("read project");
    assert!(loaded_project.get("project_id").is_some());

    let run_request: serde_json::Value =
        read_json(fixture_dir.join("convo-run-request-state.json")).expect("run request fixture");
    let request_state = run_request["durable_state"]["after"]["flow_run_requests"]["data"].clone();
    let journal_path = temp.path().join("requests.jsonl");
    append_jsonl(&journal_path, &request_state).expect("append request state");
    let loaded_requests: Vec<serde_json::Value> =
        read_jsonl(&journal_path, JsonLinesOptions::strict()).expect("read request state");
    assert_eq!(loaded_requests, vec![request_state]);

    let trigger: serde_json::Value =
        read_json(fixture_dir.join("trigger-create-state.json")).expect("trigger fixture");
    assert_eq!(
        trigger["durable_state"]["after"]["trigger_definition"]["format"],
        "toml"
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate under workspace root")
        .to_path_buf()
}
