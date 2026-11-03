use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::context::ContextMap;
use crate::error::{AttractorCoreError, Result};
use crate::flow_definition::ExitResult;

/// How one node invocation ended. A completed invocation carries the
/// decision its node declared; an execution error never carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Completed,
    Error,
}

impl OutcomeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Error => "error",
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
            "completed" => Ok(Self::Completed),
            "error" => Ok(Self::Error),
            other => Err(AttractorCoreError::InvalidIdentifier {
                kind: "outcome status",
                value: other.to_string(),
                reason: "expected completed or error".to_string(),
            }),
        }
    }
}

/// An execution error's class. `Runtime` (transport, provider, spawn,
/// timeout and other infrastructure failures) and `Contract` (a response or
/// context write that broke its contract after repair) are tolerable inside
/// a fork that opts in; `Routing` (no matching route, an undeclared
/// decision, an exhausted visit limit) never is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Contract,
    Runtime,
    Routing,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Runtime => "runtime",
            Self::Routing => "routing",
        }
    }

    /// Whether a fork declaring `tolerate_branch_execution_errors` may keep
    /// a branch ending with this error.
    pub fn is_tolerable(self) -> bool {
        !matches!(self, Self::Routing)
    }
}

/// A failure exit reached inside a parallel branch: the owning run ends
/// with this exit's declared result instead of entering the join.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchTerminal {
    pub exit_id: String,
    pub result: ExitResult,
    pub branch: String,
    pub branch_index: usize,
    /// The node and decision whose route reached the exit.
    pub node: String,
    pub decision: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub status: OutcomeStatus,
    /// The declared decision of a completed invocation; empty for an error.
    #[serde(default)]
    pub decision: String,
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
    /// Set by a fork whose branch reached a failure exit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<BranchTerminal>,
}

impl Outcome {
    pub fn new(status: OutcomeStatus) -> Self {
        Self {
            status,
            decision: String::new(),
            context_updates: ContextMap::new(),
            failure_reason: String::new(),
            notes: String::new(),
            retryable: None,
            failure_kind: None,
            raw_response_text: String::new(),
            terminal: None,
        }
    }

    /// A completed invocation choosing `decision`.
    pub fn completed(decision: impl Into<String>) -> Self {
        Self {
            decision: decision.into(),
            ..Self::new(OutcomeStatus::Completed)
        }
    }

    /// An execution error of `kind`.
    pub fn error(kind: FailureKind, reason: impl Into<String>, retryable: Option<bool>) -> Self {
        Self {
            failure_reason: reason.into(),
            retryable,
            failure_kind: Some(kind),
            ..Self::new(OutcomeStatus::Error)
        }
    }

    pub fn is_error(&self) -> bool {
        self.status == OutcomeStatus::Error
    }

    /// The error kind, defaulting an unclassified error to `Runtime`.
    pub fn error_kind(&self) -> Option<FailureKind> {
        self.is_error()
            .then(|| self.failure_kind.unwrap_or(FailureKind::Runtime))
    }
}
