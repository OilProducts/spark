use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::context::ContextMap;
use crate::graph::{attr_string, DotAttribute};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextWriteContract {
    #[serde(default)]
    pub allowed_keys: Vec<String>,
    #[serde(default)]
    pub parse_error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextReadContract {
    #[serde(default)]
    pub declared_keys: Vec<String>,
    #[serde(default)]
    pub parse_error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextWriteContractViolation {
    pub offending_keys: Vec<String>,
    pub allowed_keys: Vec<String>,
    #[serde(default)]
    pub parse_error: String,
    #[serde(default)]
    pub invalid_keys: Vec<String>,
}

impl ContextWriteContractViolation {
    pub fn format_reason(&self, node_id: Option<&str>) -> String {
        let node_prefix = node_id
            .map(|id| format!(" for node '{id}'"))
            .unwrap_or_default();
        let offending = join_or_none(&self.offending_keys);
        let declared = join_or_none(&self.allowed_keys);
        let invalid = join_or_none(&self.invalid_keys);
        let mut parts = Vec::new();
        if !self.invalid_keys.is_empty() {
            parts.push(format!(
                "invalid context_updates keys{node_prefix}: {invalid}"
            ));
            parts.push(
                "context_updates keys must be bare identifiers or dot-separated identifiers"
                    .to_string(),
            );
        }
        parts.push(format!(
            "undeclared context_updates keys{node_prefix}: {offending}"
        ));
        parts.push(format!(
            "declared spark.writes_context allowlist: {declared}"
        ));
        if !self.parse_error.is_empty() {
            parts.push(format!(
                "spark.writes_context parse error: {}",
                self.parse_error
            ));
        }
        parts.join("; ")
    }
}

pub fn parse_context_write_contract(raw: Option<&str>) -> ContextWriteContract {
    let parsed = parse_context_key_contract(
        raw,
        "expected non-empty context update keys",
        "context update key",
    );
    ContextWriteContract {
        allowed_keys: parsed.keys,
        parse_error: parsed.parse_error,
    }
}

pub fn parse_context_read_contract(raw: Option<&str>) -> ContextReadContract {
    let parsed = parse_context_key_contract(
        raw,
        "expected non-empty context read keys",
        "context read key",
    );
    ContextReadContract {
        declared_keys: parsed.keys,
        parse_error: parsed.parse_error,
    }
}

pub fn resolve_context_write_contract(
    attrs: &BTreeMap<String, DotAttribute>,
) -> ContextWriteContract {
    let raw_value = attrs
        .get("spark.writes_context")
        .map(|_| attr_string(attrs, "spark.writes_context"));
    parse_context_write_contract(raw_value.as_deref())
}

pub fn resolve_context_read_contract(
    attrs: &BTreeMap<String, DotAttribute>,
) -> ContextReadContract {
    let raw_value = attrs
        .get("spark.reads_context")
        .map(|_| attr_string(attrs, "spark.reads_context"));
    parse_context_read_contract(raw_value.as_deref())
}

pub fn validate_context_updates_against_contract(
    updates: &ContextMap,
    contract: &ContextWriteContract,
) -> Option<ContextWriteContractViolation> {
    validate_context_updates_against_contract_with_exemptions(updates, contract, &[])
}

/// Compares update keys literally with the contract; `exempt_keys` are the
/// Spark keys the node's kind emits itself.
pub fn validate_context_updates_against_contract_with_exemptions(
    updates: &ContextMap,
    contract: &ContextWriteContract,
    exempt_keys: &[&str],
) -> Option<ContextWriteContractViolation> {
    let mut authored_keys = BTreeSet::new();
    let mut invalid_authored_keys = BTreeSet::new();

    for key in updates.keys() {
        if exempt_keys.contains(&key.as_str()) {
            continue;
        }
        if invalid_context_key_reason(key).is_some() {
            invalid_authored_keys.insert(key.clone());
            continue;
        }
        authored_keys.insert(key.clone());
    }

    if authored_keys.is_empty() && invalid_authored_keys.is_empty() {
        return None;
    }

    let allowed: BTreeSet<_> = contract.allowed_keys.iter().cloned().collect();
    let mut offending_keys: Vec<String> = authored_keys
        .iter()
        .filter(|key| !allowed.contains(*key))
        .cloned()
        .collect();
    let invalid_keys: Vec<String> = invalid_authored_keys.into_iter().collect();

    if offending_keys.is_empty() && invalid_keys.is_empty() && contract.parse_error.is_empty() {
        return None;
    }
    if !contract.parse_error.is_empty() && offending_keys.is_empty() && invalid_keys.is_empty() {
        offending_keys = authored_keys.into_iter().collect();
    }

    Some(ContextWriteContractViolation {
        offending_keys,
        allowed_keys: contract.allowed_keys.clone(),
        parse_error: contract.parse_error.clone(),
        invalid_keys,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedContextKeyContract {
    keys: Vec<String>,
    parse_error: String,
}

fn parse_context_key_contract(
    raw: Option<&str>,
    empty_key_error: &'static str,
    invalid_key_label: &'static str,
) -> ParsedContextKeyContract {
    let normalized_raw = raw.unwrap_or("").trim();
    if normalized_raw.is_empty() {
        return ParsedContextKeyContract::default();
    }

    let parsed: serde_json::Value = match serde_json::from_str(normalized_raw) {
        Ok(value) => value,
        Err(source) => {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: format!(
                    "expected a JSON array of strings: {} at line {} column {}",
                    source,
                    source.line(),
                    source.column()
                ),
            };
        }
    };

    let Some(items) = parsed.as_array() else {
        return ParsedContextKeyContract {
            keys: Vec::new(),
            parse_error: "expected a JSON array of strings".to_string(),
        };
    };

    let mut keys = BTreeSet::new();
    for item in items {
        let Some(key) = item.as_str() else {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: "expected a JSON array of strings".to_string(),
            };
        };
        if key.is_empty() {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: empty_key_error.to_string(),
            };
        }
        if let Some(reason) = invalid_context_key_reason(key) {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: format!("invalid {invalid_key_label} '{key}': {reason}"),
            };
        }
        keys.insert(key.to_string());
    }

    ParsedContextKeyContract {
        keys: keys.into_iter().collect(),
        parse_error: String::new(),
    }
}

/// Why `key` is not a literal context key; keys are never trimmed.
pub fn invalid_context_key_reason(key: &str) -> Option<&'static str> {
    if key.is_empty() {
        return Some("keys must be non-empty");
    }
    if key.chars().any(char::is_whitespace) {
        return Some("keys must not contain whitespace");
    }
    if key.contains('/') || key.contains('\\') {
        return Some("path separators are not allowed");
    }
    if key.starts_with('.') || key.ends_with('.') || key.contains("..") {
        return Some("empty dotted segments are not allowed");
    }
    if !key
        .split('.')
        .all(|segment| segment.chars().all(is_context_key_char))
    {
        return Some("keys must be bare identifiers or dot-separated identifiers");
    }
    None
}

fn is_context_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.join(", ")
    }
}
