use attractor_core::{FailureKind, FlowDefinition, FlowNode, Outcome, OutcomeStatus};
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackoffConfig {
    pub initial_delay_ms: u64,
    pub backoff_factor: f64,
    pub max_delay_ms: u64,
    pub jitter: bool,
}

impl BackoffConfig {
    pub fn delay_for_attempt(self, attempt: u64) -> f64 {
        let delay = self.base_delay_for_attempt(attempt);
        if self.jitter {
            delay * rand::thread_rng().gen_range(0.5..=1.5)
        } else {
            delay
        }
    }

    pub fn delay_for_attempt_with_jitter_factor(self, attempt: u64, jitter_factor: f64) -> f64 {
        let delay = self.base_delay_for_attempt(attempt);
        if self.jitter {
            delay * jitter_factor
        } else {
            delay
        }
    }

    fn base_delay_for_attempt(self, attempt: u64) -> f64 {
        let normalized_attempt = attempt.max(1);
        let delay = (self.initial_delay_ms as f64)
            * self
                .backoff_factor
                .powi((normalized_attempt.saturating_sub(1)) as i32);
        delay.min(self.max_delay_ms as f64)
    }
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 200,
            backoff_factor: 2.0,
            max_delay_ms: 60_000,
            jitter: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u64,
    pub backoff: BackoffConfig,
}

impl RetryPolicy {
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            backoff: BackoffConfig {
                initial_delay_ms: 0,
                backoff_factor: 1.0,
                max_delay_ms: 0,
                jitter: false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPreset {
    None,
    Standard,
    Aggressive,
    Linear,
    Patient,
}

impl RetryPreset {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "standard" => Some(Self::Standard),
            "aggressive" => Some(Self::Aggressive),
            "linear" => Some(Self::Linear),
            "patient" => Some(Self::Patient),
            _ => None,
        }
    }

    pub fn policy(self) -> RetryPolicy {
        match self {
            Self::None => RetryPolicy::none(),
            Self::Standard => RetryPolicy {
                max_attempts: 5,
                backoff: BackoffConfig::default(),
            },
            Self::Aggressive => RetryPolicy {
                max_attempts: 5,
                backoff: BackoffConfig {
                    initial_delay_ms: 500,
                    ..BackoffConfig::default()
                },
            },
            Self::Linear => RetryPolicy {
                max_attempts: 3,
                backoff: BackoffConfig {
                    initial_delay_ms: 500,
                    backoff_factor: 1.0,
                    max_delay_ms: 60_000,
                    jitter: false,
                },
            },
            Self::Patient => RetryPolicy {
                max_attempts: 3,
                backoff: BackoffConfig {
                    initial_delay_ms: 2_000,
                    backoff_factor: 3.0,
                    max_delay_ms: 60_000,
                    jitter: true,
                },
            },
        }
    }
}

pub fn retry_policy_for_node(flow: &FlowDefinition, node_id: &str) -> RetryPolicy {
    let Some(node) = flow.nodes.get(node_id) else {
        return RetryPolicy::none();
    };
    if let Some(preset) = node
        .retry
        .as_ref()
        .and_then(|retry| retry.policy.as_deref())
        .and_then(|value| RetryPreset::parse(value).map(RetryPreset::policy))
    {
        return preset;
    }

    let max_retries = max_retries_for_node(flow, node);
    RetryPolicy {
        max_attempts: max_retries.saturating_add(1).max(1),
        backoff: BackoffConfig::default(),
    }
}

pub fn max_retries_for_node(flow: &FlowDefinition, node: &FlowNode) -> u64 {
    node.retry
        .as_ref()
        .and_then(|retry| retry.max_retries)
        .or(flow.defaults.max_retries)
        .unwrap_or(0)
}

pub fn should_retry_outcome(outcome: &Outcome) -> bool {
    match outcome.status {
        OutcomeStatus::Retry => true,
        OutcomeStatus::Fail => {
            if matches!(
                outcome.failure_kind,
                Some(FailureKind::Business | FailureKind::Contract)
            ) {
                return false;
            }
            outcome.retryable.unwrap_or(true)
        }
        _ => false,
    }
}

pub fn should_retry_attempt(outcome: &Outcome, retries_so_far: u64, policy: &RetryPolicy) -> bool {
    retries_so_far.saturating_add(1) < policy.max_attempts && should_retry_outcome(outcome)
}

pub fn coerce_retry_exhausted_outcome(
    flow: &FlowDefinition,
    node_id: &str,
    outcome: &Outcome,
    retries_so_far: u64,
    max_retries: u64,
) -> Outcome {
    if !matches!(outcome.status, OutcomeStatus::Retry | OutcomeStatus::Fail) {
        return outcome.clone();
    }
    if retries_so_far < max_retries {
        return outcome.clone();
    }

    let allow_partial = flow
        .nodes
        .get(node_id)
        .is_some_and(|node| crate::flow_runtime::node_attr_bool(node, "allow_partial", false));
    if !allow_partial {
        if outcome.status == OutcomeStatus::Fail {
            return outcome.clone();
        }
        return Outcome {
            status: OutcomeStatus::Fail,
            preferred_label: outcome.preferred_label.clone(),
            suggested_next_ids: outcome.suggested_next_ids.clone(),
            context_updates: outcome.context_updates.clone(),
            failure_reason: "max retries exceeded".to_string(),
            notes: outcome.notes.clone(),
            raw_response_text: outcome.raw_response_text.clone(),
            ..Outcome::new(OutcomeStatus::Fail)
        };
    }

    if outcome.status == OutcomeStatus::Fail && !should_retry_outcome(outcome) {
        return outcome.clone();
    }

    Outcome {
        status: OutcomeStatus::PartialSuccess,
        preferred_label: outcome.preferred_label.clone(),
        suggested_next_ids: outcome.suggested_next_ids.clone(),
        context_updates: outcome.context_updates.clone(),
        notes: if outcome.notes.trim().is_empty() {
            "retries exhausted, partial accepted".to_string()
        } else {
            outcome.notes.clone()
        },
        raw_response_text: outcome.raw_response_text.clone(),
        ..Outcome::new(OutcomeStatus::PartialSuccess)
    }
}

pub const EXECUTION_BASES_KEY: &str = "internal.execution_attempt_bases";
pub const RETRY_REQUEST_KEY: &str = "internal.retry_request";

/// Artifact identity is independent of the configured automatic-retry allowance.
pub fn execution_attempt(
    context: &attractor_core::ContextMap,
    node: &str,
    automatic_retries: u64,
) -> crate::error::Result<u64> {
    let bases: std::collections::BTreeMap<String, u64> = context
        .get(EXECUTION_BASES_KEY)
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(
            |error| crate::error::RuntimeStorageError::InvalidRuntimeGraph {
                reason: format!("Invalid execution attempt bases: {error}"),
            },
        )?
        .unwrap_or_default();
    bases
        .get(node)
        .copied()
        .unwrap_or(0)
        .checked_add(automatic_retries)
        .ok_or_else(|| crate::error::RuntimeStorageError::InvalidRuntimeGraph {
            reason: "Execution attempt overflow".to_string(),
        })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetryTarget {
    pub run_id: String,
    pub node_id: String,
    pub stage_index: u64,
    pub child: Option<Box<RetryTarget>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetryRequest {
    pub id: String,
    pub target: RetryTarget,
    pub preparing: bool,
}

pub fn saved_retry_request(
    context: &attractor_core::ContextMap,
) -> crate::error::Result<Option<RetryRequest>> {
    context
        .get(RETRY_REQUEST_KEY)
        .map(|value| {
            let request: RetryRequest = serde_json::from_value(value.clone()).map_err(|error| {
                crate::error::RuntimeStorageError::InvalidRuntimeGraph {
                    reason: format!("Invalid retry request: {error}"),
                }
            })?;
            let mut target = Some(&request.target);
            let mut seen = std::collections::BTreeSet::new();
            while let Some(current) = target {
                if request.id.is_empty()
                    || current.run_id.is_empty()
                    || current.node_id.is_empty()
                    || !seen.insert(&current.run_id)
                {
                    return Err(crate::error::RuntimeStorageError::InvalidRuntimeGraph {
                        reason: "Invalid empty or repeated retry request target".to_string(),
                    });
                }
                target = current.child.as_deref();
            }
            Ok(request)
        })
        .transpose()
}

/// Authorization is restricted to the prepared node invocation, never later work.
pub fn retry_authorizes_checkpoint(
    context: &attractor_core::ContextMap,
    run_id: &str,
    node: &str,
    stage: u64,
) -> bool {
    saved_retry_request(context)
        .ok()
        .flatten()
        .is_some_and(|r| {
            r.target.run_id == run_id && r.target.node_id == node && r.target.stage_index == stage
        })
}
