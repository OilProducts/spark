use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::context::ContextMap;
use crate::error::{AttractorCoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Success,
    Retry,
    Fail,
    PartialSuccess,
    Skipped,
}

impl OutcomeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Retry => "retry",
            Self::Fail => "fail",
            Self::PartialSuccess => "partial_success",
            Self::Skipped => "skipped",
        }
    }
}

impl std::fmt::Display for OutcomeStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for OutcomeStatus {
    type Err = AttractorCoreError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim() {
            "success" => Ok(Self::Success),
            "retry" => Ok(Self::Retry),
            "fail" => Ok(Self::Fail),
            "partial_success" => Ok(Self::PartialSuccess),
            "skipped" => Ok(Self::Skipped),
            other => Err(AttractorCoreError::InvalidIdentifier {
                kind: "outcome status",
                value: other.to_string(),
                reason: "expected success, retry, fail, partial_success, or skipped".to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Business,
    Contract,
    Runtime,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Business => "business",
            Self::Contract => "contract",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub status: OutcomeStatus,
    #[serde(default)]
    pub preferred_label: String,
    #[serde(default)]
    pub suggested_next_ids: Vec<String>,
    #[serde(default)]
    pub context_updates: ContextMap,
    #[serde(default)]
    pub failure_reason: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,
    #[serde(default)]
    pub raw_response_text: String,
}

impl Outcome {
    pub fn new(status: OutcomeStatus) -> Self {
        Self {
            status,
            preferred_label: String::new(),
            suggested_next_ids: Vec::new(),
            context_updates: ContextMap::new(),
            failure_reason: String::new(),
            notes: String::new(),
            retryable: None,
            failure_kind: None,
            raw_response_text: String::new(),
        }
    }

    pub fn from_contract_parts(
        status: impl AsRef<str>,
        preferred_label: impl Into<String>,
        suggested_next_ids: impl IntoIterator<Item = impl Into<String>>,
        context_updates: ContextMap,
    ) -> Result<Self> {
        Ok(Self {
            status: OutcomeStatus::from_str(status.as_ref())?,
            preferred_label: preferred_label.into(),
            suggested_next_ids: suggested_next_ids.into_iter().map(Into::into).collect(),
            context_updates,
            ..Self::new(OutcomeStatus::Success)
        })
    }

    pub fn to_payload(&self) -> OutcomePayload {
        OutcomePayload {
            status: self.status,
            preferred_label: self.preferred_label.clone(),
            suggested_next_ids: self.suggested_next_ids.clone(),
            context_updates: self.context_updates.clone(),
            notes: self.notes.clone(),
            failure_reason: self.failure_reason.clone(),
            failure_kind: self.failure_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomePayload {
    pub status: OutcomeStatus,
    pub preferred_label: String,
    pub suggested_next_ids: Vec<String>,
    pub context_updates: ContextMap,
    pub notes: String,
    pub failure_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,
}
