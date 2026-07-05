use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use attractor_core::{
    validate_context_updates_against_contract, ContextMap, ContextWriteContract, FailureKind,
    Outcome, OutcomeStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const STRUCTURED_OUTCOME_KEYS: &[&str] = &[
    "outcome",
    "preferred_label",
    "suggested_next_ids",
    "context_updates",
    "notes",
    "failure_reason",
    "retryable",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlainTextParseResult {
    pub raw_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeledOutcomeParseResult {
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredContractViolation {
    pub response_contract: String,
    pub raw_text: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_contract: Option<ContextWriteContract>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredTextOutcome {
    PlainText(PlainTextParseResult),
    ModeledOutcome(ModeledOutcomeParseResult),
    ContractViolation(StructuredContractViolation),
    InvalidOutcome(Outcome),
}

pub fn build_status_envelope_prompt_appendix(
    write_contract: Option<&ContextWriteContract>,
) -> String {
    [
        "Structured response contract:".to_string(),
        "- Return ONLY a JSON object.".to_string(),
        "- Required top-level key: \"outcome\" with one of \"success\", \"fail\", \"partial_success\", or \"retry\".".to_string(),
        "- Optional top-level keys: \"preferred_label\", \"suggested_next_ids\", \"context_updates\", \"notes\", \"failure_reason\", and \"retryable\".".to_string(),
        "- Use \"preferred_label\" for routing.".to_string(),
        "- \"suggested_next_ids\" must be a list of strings.".to_string(),
        "- \"context_updates\" must be a JSON object.".to_string(),
        "- Do not emit any other top-level keys.".to_string(),
        build_status_envelope_context_updates_contract_text(write_contract),
        "- If no routing or context updates are needed, prefer: {\"outcome\":\"success\"}".to_string(),
    ]
    .join("\n")
}

pub fn build_status_envelope_context_updates_contract_text(
    write_contract: Option<&ContextWriteContract>,
) -> String {
    let allowed_keys = write_contract
        .map(|contract| contract.allowed_keys.as_slice())
        .unwrap_or(&[]);
    let mut lines = vec!["Node-specific \"context_updates\" rules:".to_string()];
    if allowed_keys.is_empty() {
        lines.push("- This node must not emit \"context_updates\".".to_string());
        return lines.join("\n");
    }

    lines.extend([
        "- This node may include \"context_updates\" only when needed.".to_string(),
        format!(
            "- Allowed \"context_updates\" keys for this node, and no others: {}.",
            format_allowed_keys(allowed_keys)
        ),
        "- Inside \"context_updates\", emit a flat key/value map using the literal keys above."
            .to_string(),
        "- Use JSON null to clear a previously set context key when this node needs to end or reset live state.".to_string(),
    ]);
    if let Some(dotted_key) = allowed_keys.iter().find(|key| key.contains('.')) {
        lines.extend([
            format!("- Keys with dots stay literal keys, for example \"{dotted_key}\"."),
            format!(
                "- Do not nest objects inside \"context_updates\" for dotted keys. Use {} not {}.",
                flat_context_updates_example(dotted_key),
                nested_context_updates_example(dotted_key),
            ),
        ]);
    }
    lines.join("\n")
}

pub fn status_envelope_allowed_keys(write_contract: Option<&ContextWriteContract>) -> String {
    format_allowed_keys(
        write_contract
            .map(|contract| contract.allowed_keys.as_slice())
            .unwrap_or(&[]),
    )
}

pub fn coerce_structured_text_outcome(
    text: &str,
    response_contract: &str,
) -> StructuredTextOutcome {
    let raw_text = text.trim().to_string();
    let (candidate, envelope_error) =
        extract_structured_outcome_payload(text, has_response_contract(response_contract));
    if let Some(reason) = envelope_error {
        return contract_violation_or_invalid_outcome(&raw_text, reason, response_contract);
    }
    let Some(candidate) = candidate else {
        return StructuredTextOutcome::PlainText(PlainTextParseResult { raw_text });
    };

    let preferred_label = match optional_string(&candidate, "preferred_label", "") {
        Ok(value) => value,
        Err(reason) => {
            return contract_violation_or_invalid_outcome(&raw_text, reason, response_contract);
        }
    };
    let suggested_next_ids = match string_list(&candidate, "suggested_next_ids") {
        Ok(value) => value,
        Err(reason) => {
            return contract_violation_or_invalid_outcome(&raw_text, reason, response_contract);
        }
    };
    let context_updates = match context_updates(&candidate) {
        Ok(value) => value,
        Err(reason) => {
            return contract_violation_or_invalid_outcome(&raw_text, reason, response_contract);
        }
    };
    let notes = match optional_string(&candidate, "notes", "") {
        Ok(value) => value,
        Err(reason) => {
            return contract_violation_or_invalid_outcome(&raw_text, reason, response_contract);
        }
    };
    let failure_reason = match optional_string(&candidate, "failure_reason", "") {
        Ok(value) => value,
        Err(reason) => {
            return contract_violation_or_invalid_outcome(&raw_text, reason, response_contract);
        }
    };
    let retryable = match optional_bool(&candidate, "retryable") {
        Ok(value) => value,
        Err(reason) => {
            return contract_violation_or_invalid_outcome(&raw_text, reason, response_contract);
        }
    };

    let outcome_name = candidate
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let status = match OutcomeStatus::from_str(&outcome_name) {
        Ok(OutcomeStatus::Skipped) => {
            return contract_violation_or_invalid_outcome(
                &raw_text,
                "invalid structured status envelope: unsupported outcome status 'skipped'"
                    .to_string(),
                response_contract,
            );
        }
        Ok(status) => status,
        Err(_) => {
            return contract_violation_or_invalid_outcome(
                &raw_text,
                format!(
                    "invalid structured status envelope: unsupported outcome status '{}'",
                    if outcome_name.is_empty() {
                        "<empty>"
                    } else {
                        outcome_name.as_str()
                    }
                ),
                response_contract,
            );
        }
    };

    StructuredTextOutcome::ModeledOutcome(ModeledOutcomeParseResult {
        outcome: Outcome {
            status,
            preferred_label,
            suggested_next_ids,
            context_updates,
            notes,
            failure_reason,
            retryable,
            failure_kind: (has_response_contract(response_contract)
                && status == OutcomeStatus::Fail)
                .then_some(FailureKind::Business),
            raw_response_text: raw_text,
        },
    })
}

pub fn validate_write_contract_violation(
    outcome: &Outcome,
    write_contract: Option<&ContextWriteContract>,
    response_contract: &str,
    raw_text: &str,
) -> Option<StructuredContractViolation> {
    if !has_response_contract(response_contract) {
        return None;
    }
    let write_contract = write_contract?;
    let violation =
        validate_context_updates_against_contract(&outcome.context_updates, write_contract)?;
    Some(StructuredContractViolation {
        response_contract: response_contract.to_string(),
        raw_text: raw_text.trim().to_string(),
        reason: violation.format_reason(None),
        write_contract: Some(write_contract.clone()),
    })
}

pub fn build_contract_repair_prompt(violation: &StructuredContractViolation) -> String {
    let mut lines = vec![
        format!(
            "Your previous final answer violated the {} response contract.",
            violation.response_contract
        ),
        format!("Validation error: {}", violation.reason),
        String::new(),
        "Re-emit only a corrected final answer for the same decision.".to_string(),
        "Do not do new repository work.".to_string(),
        "Do not run commands.".to_string(),
        "Do not change the substantive decision, routing label, or context updates except as required to satisfy the response contract.".to_string(),
    ];
    let allowed_keys = violation
        .write_contract
        .as_ref()
        .map(|contract| contract.allowed_keys.as_slice())
        .unwrap_or(&[]);
    if allowed_keys.is_empty() {
        lines.push("Re-emit the same decision with no \"context_updates\".".to_string());
    } else {
        lines.push(format!(
            "Re-emit the same decision using only these \"context_updates\" keys when needed: {}.",
            format_allowed_keys(allowed_keys),
        ));
    }
    lines.extend([
        build_status_envelope_context_updates_contract_text(violation.write_contract.as_ref()),
        String::new(),
        "Previous invalid final answer:".to_string(),
        violation.raw_text.clone(),
    ]);
    lines.join("\n")
}

pub fn contract_failure_outcome(violation: &StructuredContractViolation) -> Outcome {
    Outcome {
        status: OutcomeStatus::Fail,
        notes: violation.raw_text.clone(),
        failure_reason: violation.reason.clone(),
        failure_kind: Some(FailureKind::Contract),
        raw_response_text: violation.raw_text.clone(),
        ..Outcome::new(OutcomeStatus::Fail)
    }
}

pub fn extract_structured_outcome_payload(
    text: &str,
    require_contract: bool,
) -> (Option<BTreeMap<String, Value>>, Option<String>) {
    let stripped = text.trim();
    if stripped.is_empty() {
        return if require_contract {
            (
                None,
                Some("invalid structured status envelope: empty response".to_string()),
            )
        } else {
            (None, None)
        };
    }

    let mut candidates = vec![stripped.to_string()];
    if stripped.starts_with("```") && stripped.ends_with("```") {
        let lines = stripped.lines().collect::<Vec<_>>();
        if lines.len() >= 3 {
            let inner = lines[1..lines.len() - 1].join("\n").trim().to_string();
            if !inner.is_empty() && !candidates.contains(&inner) {
                candidates.push(inner);
            }
        }
    }

    let mut validation_errors = Vec::new();
    for candidate in candidates {
        let payload: Value = match serde_json::from_str(&candidate) {
            Ok(value) => value,
            Err(source) => {
                if require_contract {
                    validation_errors.push(format!(
                        "invalid structured status envelope: invalid JSON: {source}"
                    ));
                }
                continue;
            }
        };
        let Some(object) = payload.as_object() else {
            if require_contract {
                validation_errors
                    .push("invalid structured status envelope: expected a JSON object".to_string());
            }
            continue;
        };
        if !object.contains_key("outcome") {
            if require_contract {
                validation_errors.push(
                    "invalid structured status envelope: missing required top-level key \"outcome\""
                        .to_string(),
                );
            }
            continue;
        }
        let allowed: BTreeSet<_> = STRUCTURED_OUTCOME_KEYS.iter().copied().collect();
        let unexpected = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unexpected.is_empty() {
            return (
                None,
                Some(format!(
                    "invalid structured status envelope: unexpected top-level keys {}",
                    unexpected.join(", ")
                )),
            );
        }
        return (
            Some(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            None,
        );
    }

    if require_contract {
        return (None, validation_errors.pop());
    }
    (None, None)
}

fn has_response_contract(response_contract: &str) -> bool {
    !response_contract.trim().is_empty()
}

fn contract_violation_or_invalid_outcome(
    text: &str,
    reason: String,
    response_contract: &str,
) -> StructuredTextOutcome {
    if has_response_contract(response_contract) {
        StructuredTextOutcome::ContractViolation(StructuredContractViolation {
            response_contract: response_contract.to_string(),
            raw_text: text.trim().to_string(),
            reason,
            write_contract: None,
        })
    } else {
        StructuredTextOutcome::InvalidOutcome(Outcome {
            status: OutcomeStatus::Fail,
            notes: text.trim().to_string(),
            failure_reason: reason,
            raw_response_text: text.trim().to_string(),
            ..Outcome::new(OutcomeStatus::Fail)
        })
    }
}

fn optional_string(
    payload: &BTreeMap<String, Value>,
    key: &str,
    default: &str,
) -> Result<String, String> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(default.to_string()),
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(format!(
            "invalid structured status envelope: {key} must be a string"
        )),
    }
}

fn optional_bool(payload: &BTreeMap<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(format!(
            "invalid structured status envelope: {key} must be a boolean"
        )),
    }
}

fn string_list(payload: &BTreeMap<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_string).ok_or_else(|| {
                    "invalid structured status envelope: suggested_next_ids must be a list of strings"
                        .to_string()
                })
            })
            .collect(),
        _ => Err(
            "invalid structured status envelope: suggested_next_ids must be a list of strings"
                .to_string(),
        ),
    }
}

fn context_updates(payload: &BTreeMap<String, Value>) -> Result<ContextMap, String> {
    match payload.get("context_updates") {
        None | Some(Value::Null) => Ok(ContextMap::new()),
        Some(Value::Object(items)) => Ok(items
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()),
        _ => {
            Err("invalid structured status envelope: context_updates must be an object".to_string())
        }
    }
}

fn format_allowed_keys(allowed_keys: &[String]) -> String {
    if allowed_keys.is_empty() {
        "<none>".to_string()
    } else {
        allowed_keys
            .iter()
            .map(|key| format!("\"{key}\""))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn flat_context_updates_example(key: &str) -> String {
    json!({"context_updates": {key: "..."}}).to_string()
}

fn nested_context_updates_example(key: &str) -> String {
    let mut nested = json!("...");
    for segment in key.split('.').rev() {
        nested = json!({segment: nested});
    }
    json!({"context_updates": nested}).to_string()
}
