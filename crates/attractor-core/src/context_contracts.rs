use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::context::ContextMap;
use crate::graph::{attr_string, DotAttribute};

const ALLOWED_CONTEXT_UPDATE_PREFIXES: &[&str] = &[
    "context.",
    "graph.",
    "internal.",
    "parallel.",
    "stack.",
    "human.gate.",
    "work.",
    "_attractor.",
];

const CONTEXT_READ_SHORTHAND_PREFIXES: &[&str] =
    &["request.", "review.", "planflow.", "milestone.", "item."];

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

pub fn normalize_context_update_key(key: &str) -> String {
    normalize_context_key(key)
}

pub fn normalize_context_read_key(key: &str) -> String {
    let normalized = key.trim();
    if normalized.is_empty() || !normalized.contains('.') {
        return normalized.to_string();
    }
    if invalid_context_key_reason(normalized).is_some() {
        return normalized.to_string();
    }
    if starts_with_any(normalized, ALLOWED_CONTEXT_UPDATE_PREFIXES) {
        return normalized.to_string();
    }
    if starts_with_any(normalized, CONTEXT_READ_SHORTHAND_PREFIXES) {
        return format!("context.{normalized}");
    }
    normalized.to_string()
}

pub fn parse_context_write_contract(raw: Option<&str>) -> ContextWriteContract {
    let parsed = parse_context_key_contract(
        raw,
        normalize_context_update_key,
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
        normalize_context_read_key,
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

pub fn normalize_context_updates(updates: &ContextMap) -> ContextMap {
    updates
        .iter()
        .map(|(key, value)| (normalize_context_update_key(key), value.clone()))
        .collect()
}

pub fn validate_context_updates_against_contract(
    updates: &ContextMap,
    contract: &ContextWriteContract,
) -> Option<ContextWriteContractViolation> {
    validate_context_updates_against_contract_with_exemptions(updates, contract, &[], &[])
}

pub fn validate_context_updates_against_contract_with_exemptions(
    updates: &ContextMap,
    contract: &ContextWriteContract,
    exempt_keys: &[&str],
    exempt_prefixes: &[&str],
) -> Option<ContextWriteContractViolation> {
    let mut normalized_authored_keys = BTreeSet::new();
    let mut invalid_authored_keys = BTreeSet::new();

    for raw_key in updates.keys() {
        let raw_key_text = raw_key.trim();
        let normalized_key = normalize_context_update_key(raw_key_text);
        if is_exempt_context_update_key(&normalized_key, exempt_keys, exempt_prefixes) {
            continue;
        }
        if invalid_context_key_reason(raw_key_text).is_some() {
            invalid_authored_keys.insert(raw_key_text.to_string());
            continue;
        }
        normalized_authored_keys.insert(normalized_key);
    }

    if normalized_authored_keys.is_empty() && invalid_authored_keys.is_empty() {
        return None;
    }

    let allowed: BTreeSet<_> = contract.allowed_keys.iter().cloned().collect();
    let mut offending_keys: Vec<String> = normalized_authored_keys
        .iter()
        .filter(|key| !allowed.contains(*key))
        .cloned()
        .collect();
    let invalid_keys: Vec<String> = invalid_authored_keys.into_iter().collect();

    if offending_keys.is_empty() && invalid_keys.is_empty() && contract.parse_error.is_empty() {
        return None;
    }
    if !contract.parse_error.is_empty() && offending_keys.is_empty() && invalid_keys.is_empty() {
        offending_keys = normalized_authored_keys.into_iter().collect();
    }

    Some(ContextWriteContractViolation {
        offending_keys,
        allowed_keys: contract.allowed_keys.clone(),
        parse_error: contract.parse_error.clone(),
        invalid_keys,
    })
}

fn normalize_context_key(key: &str) -> String {
    let normalized = key.trim();
    if normalized.is_empty() || !normalized.contains('.') {
        return normalized.to_string();
    }
    if invalid_context_key_reason(normalized).is_some() {
        return normalized.to_string();
    }
    if starts_with_any(normalized, ALLOWED_CONTEXT_UPDATE_PREFIXES) {
        return normalized.to_string();
    }
    format!("context.{normalized}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedContextKeyContract {
    keys: Vec<String>,
    parse_error: String,
}

fn parse_context_key_contract(
    raw: Option<&str>,
    normalize_key: fn(&str) -> String,
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

    let mut normalized_keys = BTreeSet::new();
    for item in items {
        let Some(text) = item.as_str() else {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: "expected a JSON array of strings".to_string(),
            };
        };
        let normalized_item = text.trim();
        if normalized_item.is_empty() {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: empty_key_error.to_string(),
            };
        }
        if let Some(reason) = invalid_context_key_reason(normalized_item) {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: format!("invalid {invalid_key_label} '{normalized_item}': {reason}"),
            };
        }
        let normalized_item = normalize_key(normalized_item);
        if normalized_item.is_empty() {
            return ParsedContextKeyContract {
                keys: Vec::new(),
                parse_error: empty_key_error.to_string(),
            };
        }
        normalized_keys.insert(normalized_item);
    }

    ParsedContextKeyContract {
        keys: normalized_keys.into_iter().collect(),
        parse_error: String::new(),
    }
}

fn invalid_context_key_reason(key: &str) -> Option<&'static str> {
    let normalized = key.trim();
    if normalized.is_empty() {
        return Some("keys must be non-empty");
    }
    if normalized.contains('/') || normalized.contains('\\') {
        return Some("path separators are not allowed");
    }
    if normalized.starts_with('.') || normalized.ends_with('.') || normalized.contains("..") {
        return Some("empty dotted segments are not allowed");
    }
    if !normalized
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

fn starts_with_any(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

fn is_exempt_context_update_key(key: &str, exempt_keys: &[&str], exempt_prefixes: &[&str]) -> bool {
    exempt_keys.iter().any(|candidate| key == *candidate)
        || exempt_prefixes.iter().any(|prefix| key.starts_with(prefix))
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.join(", ")
    }
}
