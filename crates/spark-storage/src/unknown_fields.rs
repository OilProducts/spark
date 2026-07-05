use std::collections::BTreeSet;
use std::path::Path;

use crate::error::{Result, StorageError};

/// Unknown top-level field behavior for compatibility-oriented readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownFieldPolicy {
    Allow,
    Deny,
    Collect,
}

/// Static top-level field list for owner schema records.
pub trait KnownFields {
    fn known_fields() -> &'static [&'static str];
}

/// Deterministic report of additive top-level fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnknownFieldReport {
    unknown_fields: Vec<String>,
}

impl UnknownFieldReport {
    pub fn new(unknown_fields: Vec<String>) -> Self {
        Self { unknown_fields }
    }

    pub fn unknown_fields(&self) -> &[String] {
        &self.unknown_fields
    }

    pub fn into_unknown_fields(self) -> Vec<String> {
        self.unknown_fields
    }

    pub fn is_empty(&self) -> bool {
        self.unknown_fields.is_empty()
    }
}

pub fn validate_json_object_fields<K: KnownFields>(
    path: impl AsRef<Path>,
    value: &serde_json::Value,
    policy: UnknownFieldPolicy,
) -> Result<UnknownFieldReport> {
    validate_json_object_fields_for(path, value, K::known_fields(), policy)
}

pub fn validate_json_object_fields_for(
    path: impl AsRef<Path>,
    value: &serde_json::Value,
    known_fields: &[&str],
    policy: UnknownFieldPolicy,
) -> Result<UnknownFieldReport> {
    let path = path.as_ref();
    let object = value
        .as_object()
        .ok_or_else(|| StorageError::InvalidDocumentShape {
            path: path.to_path_buf(),
            format: "JSON",
            expected: "object",
        })?;
    let actual_fields = object.keys().map(String::as_str);
    apply_policy(path, collect_unknown(actual_fields, known_fields), policy)
}

pub fn validate_toml_table_fields<K: KnownFields>(
    path: impl AsRef<Path>,
    value: &toml::Value,
    policy: UnknownFieldPolicy,
) -> Result<UnknownFieldReport> {
    validate_toml_table_fields_for(path, value, K::known_fields(), policy)
}

pub fn validate_toml_table_fields_for(
    path: impl AsRef<Path>,
    value: &toml::Value,
    known_fields: &[&str],
    policy: UnknownFieldPolicy,
) -> Result<UnknownFieldReport> {
    let path = path.as_ref();
    let table = value
        .as_table()
        .ok_or_else(|| StorageError::InvalidDocumentShape {
            path: path.to_path_buf(),
            format: "TOML",
            expected: "table",
        })?;
    let actual_fields = table.keys().map(String::as_str);
    apply_policy(path, collect_unknown(actual_fields, known_fields), policy)
}

fn collect_unknown<'a>(
    actual_fields: impl Iterator<Item = &'a str>,
    known_fields: &[&str],
) -> Vec<String> {
    let known = known_fields.iter().copied().collect::<BTreeSet<_>>();
    actual_fields
        .filter(|field| !known.contains(field))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn apply_policy(
    path: &Path,
    unknown_fields: Vec<String>,
    policy: UnknownFieldPolicy,
) -> Result<UnknownFieldReport> {
    match policy {
        UnknownFieldPolicy::Allow => Ok(UnknownFieldReport::default()),
        UnknownFieldPolicy::Collect => Ok(UnknownFieldReport::new(unknown_fields)),
        UnknownFieldPolicy::Deny if unknown_fields.is_empty() => Ok(UnknownFieldReport::default()),
        UnknownFieldPolicy::Deny => Err(StorageError::UnknownFields {
            path: path.to_path_buf(),
            fields: unknown_fields,
        }),
    }
}
